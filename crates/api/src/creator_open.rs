//! Disabled-by-default adapter for the separately versioned creator-open release.
use super::*;
use chain_base::creator_open::CreatorOpenTerms;
use chain_base::creator_open_service as service;
use serde_json::json;
use service::{CreatorOpenRequest, CreatorOpenSignatures};

fn preflight_error(error: chain_base::ChainBaseError, invalid_status: StatusCode) -> StatusCode {
    use chain_base::ChainBaseError;
    match error {
        ChainBaseError::RpcTransport(_)
        | ChainBaseError::RpcHttpStatus(_)
        | ChainBaseError::RpcRetryAfter { .. }
        | ChainBaseError::RpcProviderError { .. }
        | ChainBaseError::InvalidRpcResponse(_)
        | ChainBaseError::InvalidRpcQuantity(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => invalid_status,
    }
}

pub(super) async fn live_block(
    rpc: &str,
) -> Result<chain_base::BaseBlockIdentity, chain_base::ChainBaseError> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let number = chain_base::fetch_block_number(rpc, 1).await?;
        chain_base::fetch_exact_block_identity(rpc, number, 2).await
    })
    .await
    .map_err(|_| {
        chain_base::ChainBaseError::RpcTransport("creator-open preflight timed out".into())
    })?
}
fn release(network: &str) -> Result<service::Release, StatusCode> {
    let key = match network {
        "base-sepolia" => "CREATOR_OPEN_SEPOLIA_RELEASE_JSON",
        "base-mainnet" => "CREATOR_OPEN_MAINNET_RELEASE_JSON",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let value = env::var(key).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let r: service::Release =
        serde_json::from_str(&value).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    r.validate().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if r.network != network {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(r)
}
fn enabled(state: &SharedState, network: &str) -> bool {
    env_flag("ENABLE_CREATOR_OPEN_SPONSORSHIP")
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
#[utoipa::path(get, path="/v1/base/creator-open/capabilities", responses((status=200,description="Configured versioned creator-open capabilities; release is disabled by default"),(status=503,description="Unavailable; preserve the original request")))]
pub(super) async fn capabilities(State(state): State<SharedState>) -> Response {
    let budget = match &state.store {
        Some(store) => store.gas_sponsor_budget_status().await.ok(),
        None => None,
    };
    let capacity = budget.as_ref().is_some_and(|v| v.remaining_micro_usdc > 0);
    ([(header::CACHE_CONTROL,"no-store")],Json(json!({"protocol":chain_base::creator_open::PROTOCOL,"networks":(["base-sepolia","base-mainnet"].map(|network|json!({"network":network,"available":enabled(&state,network) && capacity,"release":release(network).ok()}))),"actions":["create","submit","review","increase","cancel","expire","refund_bond","refund_principal"],"customer_gas_wei":"0","daily_budget":budget,"requires_release_manifest":true,"maximum_authorization_seconds":7200,"work_deadline_maximum_days":366,"import_endpoint":"/v1/base/creator-open/import","prepare_endpoint":"/v1/base/creator-open/prepare","relay_endpoint":"/v1/base/creator-open/relay","status_endpoint":"/v1/base/creator-open/status","disclosure":chain_base::creator_open::DISCLOSURE,"activation":"Requires exact-bytecode rehearsal and release approval. Existing autonomous-v1 contracts are unchanged."}))).into_response()
}
/// Public terms import never asks for a wallet or authorizes a transaction.
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatorOpenImportRequest {
    original_terms: CreatorOpenTerms,
    #[schema(value_type=String)]
    user_salt: alloy::primitives::B256,
    #[schema(value_type=String)]
    creation_transaction_hash: alloy::primitives::B256,
}
#[utoipa::path(post, path="/v1/base/creator-open/import", request_body=CreatorOpenImportRequest, responses((status=202,description="Canonical creation imported; safe index catch-up remains"),(status=409,description="No matching safe creation proof"),(status=503,description="Release or canonical chain evidence unavailable")))]
pub(super) async fn import(
    State(state): State<SharedState>,
    Json(input): Json<CreatorOpenImportRequest>,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(Duration::from_secs(25), import_inner(state, input))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}
async fn import_inner(
    state: SharedState,
    input: CreatorOpenImportRequest,
) -> Result<Response, StatusCode> {
    let network = input.original_terms.network.clone();
    let release = release(&network)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if input.creation_transaction_hash.is_zero() {
        return Err(StatusCode::BAD_REQUEST);
    }
    // This synthetic identity is permanently expired. It carries no signing
    // authority and never enters the sponsored action/request quota tables.
    let request = service::Request {
        expected_solver_reward_micro_usdc: input.original_terms.solver_reward_micro_usdc,
        original_terms: input.original_terms,
        user_salt: input.user_salt,
        expected_revision: 1,
        valid_before: 1,
        public_evidence: None,
        action: service::Action::Create {
            token_nonce: input.creation_transaction_hash,
        },
    };
    let p = service::prepare(&release, &request, &service::Signatures::default())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(&network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let head = live_block(&rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    service::verify_release(
        &chain_base::ReqwestJsonRpcTransport::default(),
        &rpc,
        &release,
        &head,
        Utc::now().timestamp() as u64,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let safe = chain_base::fetch_safe_block_identity(&rpc, 20)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if safe.number > head.number
        || safe.timestamp > head.timestamp
        || head.timestamp.saturating_sub(safe.timestamp) > 3600
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let hash = format!("{:#x}", input.creation_transaction_hash);
    let receipt = chain_base::fetch_transaction_receipt(&rpc, &hash, 21)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .result
        .ok_or(StatusCode::CONFLICT)?;
    let number = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .ok_or(StatusCode::CONFLICT)?;
    if !receipt.transaction_hash.eq_ignore_ascii_case(&hash)
        || receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? != Some(true)
        || number > safe.number
        || number < release.deployment_block
    {
        return Err(StatusCode::CONFLICT);
    }
    let block = fetch_exact_block_identity(&rpc, number, 22)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    if receipt.block_hash.as_deref() != Some(block.hash.as_str()) {
        return Err(StatusCode::CONFLICT);
    }
    let mut logs = receipt
        .logs_to_evm_logs()
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    for log in &mut logs {
        if log.block_number != number || !log.tx_hash.eq_ignore_ascii_case(&hash) {
            return Err(StatusCode::BAD_GATEWAY);
        }
        log.occurred_at = chrono::DateTime::from_timestamp(
            i64::try_from(block.timestamp).map_err(|_| StatusCode::BAD_GATEWAY)?,
            0,
        );
    }
    // Project the entire successful receipt: create-and-close in the same
    // transaction is still a real creation, and must never be called open.
    chain_base::creator_open::project(
        &request.original_terms,
        release.factory,
        release.implementation,
        request.user_salt,
        block.timestamp,
        &logs,
    )
    .map_err(|_| StatusCode::CONFLICT)?;
    if fetch_exact_block_identity(&rpc, safe.number, 23)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .hash
        != safe.hash
        || fetch_exact_block_identity(&rpc, number, 24)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .hash
            != block.hash
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    store
        .save_creator_open_external_creation(
            &network,
            &format!("{:#x}", p.bounty_contract),
            &hash,
            &request,
        )
        .await
        .map_err(map_x402_db_error)?;
    Ok((StatusCode::ACCEPTED,[(header::CACHE_CONTROL,"no-store")],Json(json!({"protocol":chain_base::creator_open::PROTOCOL,"network":network,"bounty_contract":p.bounty_contract,"creation_transaction_hash":hash,"creation_verified_block":block,"imported":true,"index_status":"pending_canonical_scan","next_step":"Read the bounties endpoint after the safe index completes. Import acceptance is not current state or proof of solver payment."}))).into_response())
}
#[utoipa::path(post, path="/v1/base/creator-open/prepare", request_body = CreatorOpenRequest, responses((status=200,description="Prepare exact public terms and short-lived signature requirements"),(status=503,description="Unavailable; preserve the original request")))]
pub(super) async fn prepare(
    State(state): State<SharedState>,
    Json(request): Json<service::Request>,
) -> Result<Response, StatusCode> {
    let network = request.original_terms.network.as_str();
    if !enabled(&state, network)
        || state
            .store
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
            .gas_sponsor_budget_status()
            .await
            .map_err(map_x402_db_error)?
            .remaining_micro_usdc
            == 0
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let release = release(network)?;
    let p = service::prepare(&release, &request, &service::Signatures::default())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .check_sponsored_action_capacity(
            network,
            &key(p.approval_hash),
            &format!("{:#x}", p.quota_wallet),
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let block = live_block(&rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::time::timeout(
        Duration::from_secs(12),
        service::verify_fresh_state(
            &chain_base::ReqwestJsonRpcTransport::default(),
            &rpc,
            &release,
            &p,
            &block,
            Utc::now().timestamp() as u64,
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(|error| preflight_error(error, StatusCode::CONFLICT))?;
    let balance = service::principal_balance(
        &chain_base::ReqwestJsonRpcTransport::default(),
        &rpc,
        &p,
        &block,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store, private")],
        Json(json!({"prepared":p,"verified_block":block,"approval_required":true,"wallet_balance_micro_usdc":balance.map(|v|v.to_string())})),
    )
        .into_response())
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatorOpenRelayRequest {
    request: CreatorOpenRequest,
    #[schema(value_type=String)]
    approval_hash: alloy::primitives::B256,
    signatures: CreatorOpenSignatures,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatorOpenStatusRequest {
    network: String,
    #[schema(value_type=String)]
    approval_hash: alloy::primitives::B256,
}
fn key(hash: alloy::primitives::B256) -> String {
    format!("creator-open:{hash:#x}")
}
async fn reconcile(
    state: &SharedState,
    release: &service::Release,
    p: &service::Prepared,
    op: &db::SponsoredAction,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(
        Duration::from_secs(15),
        reconcile_inner(state, release, p, op),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}
async fn reconcile_inner(
    state: &SharedState,
    release: &service::Release,
    p: &service::Prepared,
    op: &db::SponsoredAction,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let evidence =
        if let Some(receipt) = sponsored_verdict::confirmed_receipt(state, &p.network, op).await? {
            let (_, rpc) = state
                .base_rpc_urls
                .resolve(&p.network)
                .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            let block = fetch_exact_block_identity(
                &rpc,
                receipt
                    .block_number()
                    .map_err(|_| StatusCode::BAD_GATEWAY)?
                    .ok_or(StatusCode::BAD_GATEWAY)?,
                7,
            )
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
            service::receipt_outcome(release, p, &receipt, &block)
                .map_err(|_| StatusCode::BAD_GATEWAY)?
        } else {
            None
        };
    if evidence.is_some() {
        store
            .finish_sponsored_action(op.id, None, "confirmed", None, None)
            .await
            .map_err(map_x402_db_error)?;
    }
    let current = store
        .sponsored_action(op.id)
        .await
        .map_err(map_x402_db_error)?;
    let code = if evidence.is_some() {
        StatusCode::OK
    } else if current.status == "failed" {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::ACCEPTED
    };
    Ok((code,[(header::CACHE_CONTROL,"no-store, private")],Json(json!({"protocol":chain_base::creator_open::PROTOCOL,"network":p.network,"bounty_contract":p.bounty_contract,"approval_hash":p.approval_hash,"confirmed":evidence.is_some(),"solver_paid":evidence.as_ref().is_some_and(|v|v["solver_paid"]==true),"customer_gas_wei":"0","operation":current,"evidence":evidence,"next_step":"Keep this exact request until its canonical outcome is known. A transaction hash is not payment."}))).into_response())
}
#[utoipa::path(post, path="/v1/base/creator-open/status", request_body = CreatorOpenStatusRequest, responses((status=200,description="Reconcile a previously recorded action without signatures"),(status=503,description="Unavailable; preserve the original request")))]
pub(super) async fn status(
    State(state): State<SharedState>,
    Json(request): Json<CreatorOpenStatusRequest>,
) -> Result<Response, StatusCode> {
    let release = release(&request.network)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = key(request.approval_hash);
    let r = store
        .creator_open_request(&request.network, &key)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let p = service::prepare(&release, &r, &service::Signatures::default())
        .map_err(|_| StatusCode::CONFLICT)?;
    if p.approval_hash != request.approval_hash {
        return Err(StatusCode::CONFLICT);
    }
    let op = store
        .find_sponsored_action(&request.network, &key)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    reconcile(&state, &release, &p, &op).await
}
#[utoipa::path(post, path="/v1/base/creator-open/relay", request_body = CreatorOpenRelayRequest, responses((status=200,description="Sponsor an exact signed creator-open action; never customer-paid fallback"),(status=503,description="Unavailable; preserve the original request")))]
pub(super) async fn relay(
    State(state): State<SharedState>,
    Json(input): Json<CreatorOpenRelayRequest>,
) -> Result<Response, StatusCode> {
    let network = input.request.original_terms.network.as_str();
    let release = release(network)?;
    let p = service::prepare(&release, &input.request, &input.signatures)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if input.approval_hash != p.approval_hash {
        return Err(StatusCode::CONFLICT);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let key = key(p.approval_hash);
    // Reconciliation is available after sponsorship is disabled or authority expires.
    if let Some(op) = store
        .find_sponsored_action(network, &key)
        .await
        .map_err(map_x402_db_error)?
        .filter(|op| op.transaction_hash.is_some())
    {
        let op = if enabled(&state, network) {
            sponsored_verdict::retry_saved(&state, network, op, input.request.valid_before).await?
        } else {
            op
        };
        return reconcile(&state, &release, &p, &op).await;
    }
    if !enabled(&state, network) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let from = state
        .x402_relayer
        .address()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let transport = chain_base::ReqwestJsonRpcTransport::default();
    let block = live_block(&rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    tokio::time::timeout(Duration::from_secs(15), async {
        service::verify_fresh_state(
            &transport,
            &rpc,
            &release,
            &p,
            &block,
            Utc::now().timestamp() as u64,
        )
        .await
        .map_err(|error| preflight_error(error, StatusCode::CONFLICT))?;
        service::verify_signatures(&transport, &rpc, &p, &input.signatures, &block)
            .await
            .map_err(|error| preflight_error(error, StatusCode::UNAUTHORIZED))?;
        service::simulate(&transport, &rpc, &p, &from, &block)
            .await
            .map_err(|error| preflight_error(error, StatusCode::UNPROCESSABLE_ENTITY))
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
    let op = store
        .reserve_sponsored_action(
            network,
            &key,
            &format!("{:#x}", p.quota_wallet),
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    store
        .save_creator_open_request(
            network,
            &key,
            &format!("{:#x}", p.bounty_contract),
            op.id,
            &input.request,
        )
        .await
        .map_err(map_x402_db_error)?;
    let mut intent = p.transaction.clone();
    intent.from = Some(from);
    let op = sponsored_verdict::broadcast(&state, network, op, &intent, 1_500_000).await?;
    reconcile(&state, &release, &p, &op).await
}

#[cfg(test)]
#[path = "creator_open_rehearsal.rs"]
mod rehearsal;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeedQuery {
    network: String,
    bounty_contract: Option<alloy::primitives::Address>,
    entry: Option<u64>,
}
#[utoipa::path(get, path="/v1/base/creator-open/bounties", params(("network"=String,Query,description="base-mainnet or base-sepolia"),("bounty_contract"=Option<String>,Query,description="Canonical creator-open contract"),("entry"=Option<u64>,Query,description="Exact historical entry number")), responses((status=200,description="Receipt-verified safe creator-open snapshots; unavailable records omitted"),(status=503,description="Unavailable; preserve the original request")))]
pub(super) async fn feed(
    State(state): State<SharedState>,
    Query(query): Query<FeedQuery>,
) -> Result<Response, StatusCode> {
    tokio::time::timeout(Duration::from_secs(20), feed_inner(state, query))
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}
async fn feed_inner(state: SharedState, query: FeedQuery) -> Result<Response, StatusCode> {
    let release = release(&query.network)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc) = state
        .base_rpc_urls
        .resolve(&query.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let head = live_block(&rpc)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    service::verify_release(
        &chain_base::ReqwestJsonRpcTransport::default(),
        &rpc,
        &release,
        &head,
        Utc::now().timestamp() as u64,
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let contract = query.bounty_contract.map(|a| format!("{a:#x}"));
    let indexes = store
        .creator_open_index_batch(&query.network, contract.as_deref())
        .await
        .map_err(map_x402_db_error)?;
    let mut items = Vec::new();
    let mut unavailable = 0;
    let mut verified = std::collections::BTreeMap::new();
    for item in indexes {
        let Some(block) = &item.checkpoint else {
            unavailable += 1;
            continue;
        };
        let current = Utc::now()
            .signed_duration_since(item.checked_at)
            .num_seconds();
        if !item.available || !(0..=120).contains(&current) {
            unavailable += 1;
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = verified.entry(block.number) {
            entry.insert(
                fetch_exact_block_identity(&rpc, block.number, 4)
                    .await
                    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
                    .hash,
            );
        }
        if verified[&block.number] != block.hash {
            unavailable += 1;
            store
                .invalidate_creator_open_index(&item, true)
                .await
                .map_err(map_x402_db_error)?;
            continue;
        }
        let Some(p) = item.projection else {
            unavailable += 1;
            continue;
        };
        let mut terms = item.request.original_terms.clone();
        terms.solver_reward_micro_usdc =
            u64::try_from(p.solver_reward).map_err(|_| StatusCode::BAD_GATEWAY)?;
        let c = terms
            .config(terms.submission_deadline - 1)
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        if format!("{:#x}", c.termsHash) != p.terms_hash {
            return Err(StatusCode::BAD_GATEWAY);
        }
        let entries = if let Some(entry) = query.entry {
            p.entries
                .iter()
                .filter(|(id, _)| **id == entry)
                .collect::<Vec<_>>()
        } else {
            p.entries
                .iter()
                .rev()
                .filter(|(_, e)| e.status == "pending" || e.status == "winner")
                .take(65)
                .collect()
        };
        let evidence_hashes = entries
            .iter()
            .map(|(_, e)| e.evidence_hash.clone())
            .collect::<Vec<_>>();
        let evidence = store
            .creator_open_public_evidence(&query.network, &p.contract, &evidence_hashes)
            .await
            .map_err(map_x402_db_error)?;
        items.push(json!({"protocol":p.protocol,"network":query.network,"bounty_contract":p.contract,"original_terms":item.request.original_terms,"user_salt":item.request.user_salt,"terms":terms,"terms_hash":p.terms_hash,"revision":p.revision,"status":p.status,"funded":p.status!="unfunded","solver_paid":p.settlement_transaction.is_some(),"settlement_transaction":p.settlement_transaction,"winner":p.winner,"principal_micro_usdc":p.principal.to_string(),"outstanding_bonds_micro_usdc":p.outstanding_bonds.to_string(),"entry_count":p.entries.len(),"entries":entries.into_iter().map(|(id,e)|json!({"id":id,"entry":e,"public_evidence":evidence.get(&e.evidence_hash)})).collect::<Vec<_>>(),"entry_view":"Pending and winning entries; pass entry to read a historical entry.","verified_safe_block":block,"checked_at":item.checked_at,"customer_gas_wei":"0"}));
    }
    if contract.is_some() && items.is_empty() {
        return Err(if unavailable > 0 {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NOT_FOUND
        });
    }
    Ok(([(header::CACHE_CONTROL,"no-store")],Json(json!({"protocol":chain_base::creator_open::PROTOCOL,"network":query.network,"items":items,"unavailable_count":unavailable,"evidence_boundary":"Only canonical receipt-verified OpenBountySettled events establish solver payment for this protocol. Unavailable snapshots are omitted, not treated as unfunded."}))).into_response())
}

#[cfg(test)]
mod preflight_tests {
    use super::*;
    use chain_base::ChainBaseError;

    #[test]
    fn provider_failures_preserve_authority_instead_of_reporting_customer_conflicts() {
        for invalid_status in [
            StatusCode::CONFLICT,
            StatusCode::UNAUTHORIZED,
            StatusCode::UNPROCESSABLE_ENTITY,
        ] {
            for failure in [
                ChainBaseError::RpcTransport("timeout".into()),
                ChainBaseError::RpcHttpStatus(429),
                ChainBaseError::RpcRetryAfter {
                    status: 429,
                    retry_after_seconds: 7200,
                },
                ChainBaseError::RpcRetryAfter {
                    status: 503,
                    retry_after_seconds: 7200,
                },
                ChainBaseError::RpcProviderError {
                    code: -32000,
                    message: "header unavailable".into(),
                },
                ChainBaseError::InvalidRpcResponse("missing result".into()),
                ChainBaseError::InvalidRpcQuantity("malformed block".into()),
            ] {
                assert_eq!(
                    preflight_error(failure, invalid_status),
                    StatusCode::SERVICE_UNAVAILABLE
                );
            }
            assert_eq!(
                preflight_error(
                    ChainBaseError::InvalidTermsDocument(
                        "stale revision or invalid signature".into()
                    ),
                    invalid_status
                ),
                invalid_status
            );
        }
    }
}
