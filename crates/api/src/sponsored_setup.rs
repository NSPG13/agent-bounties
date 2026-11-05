//! Gas-only setup on immutable, explicitly pinned wallet/identity factories.
use super::*;
use chain_base::{sponsored_setup as service, ReqwestJsonRpcTransport};
use serde_json::json;
use service::SponsoredSetupRequest;

fn release(network: &str) -> Result<service::Release, StatusCode> {
    let key = match network {
        "base-mainnet" => "SPONSORED_SETUP_MAINNET_RELEASE_JSON",
        "base-sepolia" => "SPONSORED_SETUP_SEPOLIA_RELEASE_JSON",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let r: service::Release =
        serde_json::from_str(&env::var(key).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    r.validate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if r.network != network {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(r)
}
fn enabled(state: &SharedState, network: &str) -> bool {
    env_flag("ENABLE_SPONSORED_SETUP")
        && state.store.is_some()
        && state.x402_relayer.enabled
        && state
            .x402_relayer
            .relayer
            .as_ref()
            .is_some_and(|r| r.has_gas_budget())
        && state.base_rpc_urls.resolve(network).is_ok()
        && release(network).is_ok()
}
#[utoipa::path(get,path="/v1/base/sponsored-setup/capabilities",responses((status=200,description="Wallet deployment and participant-registration sponsorship readiness")))]
pub(super) async fn capabilities(State(state): State<SharedState>) -> Response {
    let budget = match &state.store {
        Some(s) => s.gas_sponsor_budget_status().await.ok(),
        None => None,
    };
    let capacity = budget.as_ref().is_some_and(|b| b.remaining_micro_usdc > 0);
    ([(header::CACHE_CONTROL,"no-store")],Json(json!({"schema":service::SCHEMA,"networks":(["base-mainnet","base-sepolia"].map(|network|json!({"network":network,"available":capacity && enabled(&state,network),"release":release(network).ok()}))),"actions":["wallet_deploy","participant_register"],"customer_gas_wei":"0","initial_wallet_funding":"0","daily_budget":budget,"review_url":"https://agentizable.ai/setup.html","requires_owner_signature":true,"maximum_authorization_seconds":7200,"evidence_boundary":"Setup state is not bounty funding or solver payment. An identity attestation must come from the registry's immutable attester; the sponsor cannot create one."}))).into_response()
}
async fn inspect(
    state: &SharedState,
    r: &service::Release,
    p: &service::Prepared,
) -> Result<(String, chain_base::BaseBlockIdentity, bool), StatusCode> {
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(&r.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let block = creator_open::live_block(&rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    service::verify_release(
        &ReqwestJsonRpcTransport::default(),
        &rpc,
        r,
        &block,
        Utc::now().timestamp() as u64,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let complete = service::complete(&ReqwestJsonRpcTransport::default(), &rpc, p, &block)
        .await
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok((rpc, block, complete))
}
#[utoipa::path(post,path="/v1/base/sponsored-setup/prepare",request_body=SponsoredSetupRequest,responses((status=200,description="Exact empty-wallet or attested-identity setup review; no signature or send"),(status=503,description="Sponsorship unavailable")))]
pub(super) async fn prepare(
    State(state): State<SharedState>,
    Json(request): Json<service::Request>,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(Duration::from_secs(30),async {
        let r=release(&request.network)?; let p=service::prepare(&r,&request).map_err(|_|StatusCode::BAD_REQUEST)?;
        let (rpc,block,complete)=inspect(&state,&r,&p).await?;
        if !complete {
            if !enabled(&state,&request.network){return Err(StatusCode::SERVICE_UNAVAILABLE);}
            service::verify_fresh(&ReqwestJsonRpcTransport::default(),&rpc,&p,&block,Utc::now().timestamp() as u64).await.map_err(|_|StatusCode::CONFLICT)?;
            let from=state.x402_relayer.address().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
            service::simulate(&ReqwestJsonRpcTransport::default(),&rpc,&p,&from,&block).await.map_err(|_|StatusCode::UNPROCESSABLE_ENTITY)?;
        }
        Ok(([(header::CACHE_CONTROL,"no-store, private")],Json(json!({"prepared":p,"setup_complete":complete,"observed_block":block,"customer_gas_wei":"0","funding_confirmed":false,"solver_paid":false}))).into_response())
    }).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct SponsoredSetupSignedRequest {
    pub request: SponsoredSetupRequest,
    pub owner_signature: String,
}
fn response(
    p: &service::Prepared,
    complete: bool,
    block: &chain_base::BaseBlockIdentity,
    op: Option<db::SponsoredAction>,
) -> Response {
    (if complete{StatusCode::OK}else{StatusCode::ACCEPTED},[(header::CACHE_CONTROL,"no-store, private")],Json(json!({"schema":service::SCHEMA,"request_digest":p.digest,"network":p.request.network,"predicted_wallet":p.predicted_wallet,"setup_complete":complete,"observed_block":block,"operation":op,"customer_gas_wei":"0","funding_confirmed":false,"solver_paid":false,"next_step":"Reconcile this exact request. Setup never funds a wallet, accepts a bounty or proves payment."}))).into_response()
}
async fn reconcile(
    state: &SharedState,
    r: &service::Release,
    p: &service::Prepared,
) -> Result<Response, StatusCode> {
    let (_, block, complete) = inspect(state, r, p).await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = format!("setup:{:#x}", p.digest);
    let mut op = store
        .find_sponsored_action(&r.network, &key)
        .await
        .map_err(map_x402_db_error)?;
    if let Some(item) = op.as_ref() {
        // A successful setup state alone does not identify this relay's receipt.
        // Canonical receipt success is required before closing its journal.
        if item.status == "broadcast"
            && sponsored_verdict::confirmed_receipt(state, &r.network, item)
                .await?
                .is_some()
            && complete
        {
            op = Some(
                store
                    .finish_sponsored_action(item.id, None, "confirmed", None, None)
                    .await
                    .map_err(map_x402_db_error)?,
            );
        } else {
            op = Some(
                store
                    .sponsored_action(item.id)
                    .await
                    .map_err(map_x402_db_error)?,
            );
        }
    }
    Ok(response(p, complete, &block, op))
}
#[utoipa::path(post,path="/v1/base/sponsored-setup/status",request_body=SponsoredSetupRequest,responses((status=200,description="Fresh canonical setup state; never sends"),(status=202,description="Setup not yet observed")))]
pub(super) async fn status(
    State(state): State<SharedState>,
    Json(request): Json<service::Request>,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let r = release(&request.network)?;
        let p = service::prepare(&r, &request).map_err(|_| StatusCode::BAD_REQUEST)?;
        reconcile(&state, &r, &p).await
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}
#[utoipa::path(post,path="/v1/base/sponsored-setup/relay",request_body=SponsoredSetupSignedRequest,responses((status=200,description="Setup canonically observed"),(status=202,description="Exact setup request saved or broadcast"),(status=401,description="Owner approval rejected"),(status=429,description="Shared sponsorship quota exhausted"),(status=503,description="Sponsorship unavailable; no paid fallback")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(input): Json<SponsoredSetupSignedRequest>,
) -> Result<Response, StatusCode> {
    // The sender lease/outbox own broadcast timeouts. Never cancel a send by an
    // outer timeout after side effects could have started.
    let r = release(&input.request.network)?;
    let mut p = service::prepare(&r, &input.request).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (rpc, block, complete) =
        tokio::time::timeout(Duration::from_secs(30), inspect(&state, &r, &p))
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
    service::verify_authority(
        &ReqwestJsonRpcTransport::default(),
        &rpc,
        &p,
        &input.owner_signature,
        &block,
    )
    .await
    .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = format!("setup:{:#x}", p.digest);
    if complete {
        return reconcile(&state, &r, &p).await;
    }
    if let Some(op) = store
        .find_sponsored_action(&r.network, &key)
        .await
        .map_err(map_x402_db_error)?
        .filter(|o| o.transaction_hash.is_some())
    {
        if enabled(&state, &r.network) {
            let _ = sponsored_verdict::retry_saved(&state, &r.network, op, p.request.valid_before)
                .await;
        }
        return reconcile(&state, &r, &p).await;
    }
    if !enabled(&state, &r.network) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    service::verify_fresh(
        &ReqwestJsonRpcTransport::default(),
        &rpc,
        &p,
        &block,
        Utc::now().timestamp() as u64,
    )
    .await
    .map_err(|_| StatusCode::CONFLICT)?;
    let from = state
        .x402_relayer
        .address()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    service::simulate(&ReqwestJsonRpcTransport::default(), &rpc, &p, &from, &block)
        .await
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let op = store
        .reserve_sponsored_action(
            &r.network,
            &key,
            &normalize_evm_address(&p.request.owner).map_err(|_| StatusCode::BAD_REQUEST)?,
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    p.transaction.from = Some(from);
    sponsored_verdict::broadcast(&state, &r.network, op, &p.transaction, 1_500_000).await?;
    reconcile(&state, &r, &p).await
}

#[cfg(test)]
#[path = "sponsored_setup_rehearsal.rs"]
mod rehearsal;
