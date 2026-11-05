//! Explicit operator-only first-send recovery. Never creates a transaction.
use super::*;
use chain_base::timeout_recovery::{
    revalidate_timeout_envelope, TimeoutRelayScope, TimeoutRevalidationObservation,
    TimeoutRevalidationPolicy,
};
use timeout_relay::TimeoutLeaseBudget;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    operation_id: Uuid,
    scope: TimeoutRelayScope,
    expected_transaction_hash: String,
}

struct Config {
    primary_rpc: String,
    shadow_rpc: String,
    policy: TimeoutRevalidationPolicy,
}

trait RecoveryIo: Sync {
    fn check(
        &self,
        config: &Config,
        saved: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<TimeoutRevalidationObservation, ChainBaseError>> + Send;
    fn quote(
        &self,
        chain_id: u64,
        maximum_fee_wei: u128,
    ) -> impl std::future::Future<Output = Result<u64, ChainBaseError>> + Send;
    fn broadcast(
        &self,
        config: &Config,
        saved: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<(), ChainBaseError>> + Send;
}
struct LiveIo;
impl RecoveryIo for LiveIo {
    async fn check(
        &self,
        config: &Config,
        saved: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
        revalidate_timeout_envelope(
            [&config.primary_rpc, &config.shadow_rpc],
            &saved.scope,
            &saved.prepared,
            &saved.prepared.transaction.relayer,
            &config.policy,
            seconds,
        )
        .await
    }
    async fn quote(&self, chain_id: u64, maximum_fee_wei: u128) -> Result<u64, ChainBaseError> {
        db::quote_gas_budget_micro_usdc(chain_id, maximum_fee_wei).await
    }
    async fn broadcast(
        &self,
        config: &Config,
        saved: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        let transport =
            chain_base::ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(seconds))?;
        chain_base::broadcast_signed_transaction_with_transport(
            &config.primary_rpc,
            &saved.prepared.signed_transaction,
            90,
            &transport,
        )
        .await
        .map(|_| ())
    }
}

fn gate(enabled: Option<&str>, allowed: Option<&str>, id: Uuid) -> Result<(), StatusCode> {
    if enabled != Some("true") || allowed.and_then(|value| value.parse::<Uuid>().ok()) != Some(id) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(())
}

pub(super) fn reviewed_policy(
    network: &str,
    factory: &str,
    implementation: &str,
    limits: &X402HostedRelayerConfig,
) -> Result<TimeoutRevalidationPolicy, StatusCode> {
    let (manifest, factory_key, implementation_key) = match network {
        "base-mainnet" => (
            include_str!("../../../deployments/base-mainnet.json"),
            "contract",
            "implementation",
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let manifest: serde_json::Value =
        serde_json::from_str(manifest).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let pins = &manifest["factory"];
    if pins[factory_key] != factory || pins[implementation_key] != implementation {
        return Err(StatusCode::CONFLICT);
    }
    let hash = |key: &str| {
        pins[key]
            .as_str()
            .map(str::to_owned)
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)
    };
    Ok(TimeoutRevalidationPolicy {
        factory_runtime_code_hash: hash("runtime_code_hash")?,
        implementation_contract: implementation.into(),
        implementation_runtime_code_hash: hash("implementation_runtime_code_hash")?,
        max_gas: limits.max_gas,
        max_fee_per_gas_wei: limits.max_fee_per_gas_wei,
    })
}

fn config(state: &SharedState, request: &Request) -> Result<Config, StatusCode> {
    gate(
        std::env::var("BASE_TIMEOUT_OPERATOR_REVALIDATION_ENABLED")
            .ok()
            .as_deref(),
        std::env::var("BASE_TIMEOUT_OPERATOR_REVALIDATION_OPERATION_ID")
            .ok()
            .as_deref(),
        request.operation_id,
    )?;
    let scope = &request.scope;
    scope.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    if scope.operation_id().map_err(|_| StatusCode::BAD_REQUEST)? != request.operation_id {
        return Err(StatusCode::CONFLICT);
    }
    let planner = configured_autonomous_planner(&scope.network)?;
    if planner.factory_contract != scope.factory_contract {
        return Err(StatusCode::CONFLICT);
    }
    let policy = reviewed_policy(
        &scope.network,
        &planner.factory_contract,
        &planner.implementation_contract,
        &state.x402_relayer,
    )?;
    let (_, primary_rpc) = timeout_rpc_config(&state.base_rpc_urls, &scope.network)?;
    let key = match scope.network.as_str() {
        "base-mainnet" => "BASE_MAINNET_TIMEOUT_REVALIDATION_SHADOW_RPC_URL",
        "base-sepolia" => "BASE_SEPOLIA_TIMEOUT_REVALIDATION_SHADOW_RPC_URL",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let shadow_rpc = std::env::var(key).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Config {
        primary_rpc,
        shadow_rpc,
        policy,
    })
}

pub(super) async fn handle(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Response, StatusCode> {
    // Authenticate before looking up an operation, reading configuration or making RPCs.
    require_operator(&state, &headers)?;
    let config = config(&state, &request)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut saved = store
        .timeout_relay(request.operation_id, &request.scope.network)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if saved.scope != request.scope
        || saved.prepared.transaction.tx_hash != request.expected_transaction_hash
    {
        return Err(StatusCode::CONFLICT);
    }
    if !store
        .timeout_publication_matches(&saved.scope)
        .await
        .map_err(map_x402_db_error)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let action = match saved.scope.action.as_str() {
        "expire_claim" => AutonomousTimeoutAction::ExpireClaim,
        "expire_submission" => AutonomousTimeoutAction::ExpireSubmission,
        _ => return Err(StatusCode::CONFLICT),
    };
    if saved.send_attempted {
        return Ok(timeout_relay::reconcile(&state, &config.primary_rpc, action, &saved).await);
    }
    let relayer = timeout_relayer(&state.x402_relayer)?;
    if !relayer.has_gas_budget() || relayer.address() != saved.prepared.transaction.relayer {
        return Err(StatusCode::CONFLICT);
    }
    let budget = TimeoutLeaseBudget::new(
        tokio::time::Instant::now(),
        state.x402_relayer.lease_seconds,
        state.x402_relayer.rpc_timeout_seconds,
    )?;
    let lease_stage = budget.stage(3)?;
    let lease = timeout_lease_result(
        tokio::time::timeout_at(
            lease_stage.deadline,
            store
                .acquire_x402_relayer_lease(&saved.scope.network, state.x402_relayer.lease_seconds),
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    )?;
    let result = first_send(&state, &config, lease, budget, &mut saved, &LiveIo).await;
    // Failure to release does not permit another send. The bounded lease expires.
    if store
        .release_x402_relayer_lease(&saved.scope.network, lease)
        .await
        .is_err()
    {
        record_failure_stage(
            FailureStage::TimeoutLeaseReleaseFailed,
            &deployment_revision(),
        );
    }
    // A provider can find the saved hash despite a zero journal marker.
    // Observe its receipt without changing or resending that journal.
    let mut response = timeout_relay::reconcile_with_observation(
        &state,
        &config.primary_rpc,
        action,
        &saved,
        true,
    )
    .await;
    let status = match result {
        Ok(id) => {
            response.headers_mut().insert(
                "x-timeout-revalidation-id",
                id.to_string()
                    .parse()
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
            );
            "consumed"
        }
        Err(_) if saved.send_attempted || saved.marker_commit_unknown => "reconciliation_required",
        Err(_) => "blocked",
    };
    response.headers_mut().insert(
        "x-timeout-revalidation-status",
        HeaderValue::from_static(status),
    );
    Ok(response)
}

async fn first_send(
    state: &SharedState,
    config: &Config,
    lease: Uuid,
    budget: TimeoutLeaseBudget,
    saved: &mut db::SavedTimeoutRelay,
    io: &impl RecoveryIo,
) -> Result<Uuid, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let stage = budget.stage(3)?;
    let current = tokio::time::timeout_at(
        stage.deadline,
        store.timeout_relay(
            saved
                .scope
                .operation_id()
                .map_err(|_| StatusCode::BAD_REQUEST)?,
            &saved.scope.network,
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?
    .ok_or(StatusCode::NOT_FOUND)?;
    if current.scope != saved.scope
        || current.prepared.signed_transaction != saved.prepared.signed_transaction
        || serde_json::to_value(&current.prepared.transaction).ok()
            != serde_json::to_value(&saved.prepared.transaction).ok()
    {
        return Err(StatusCode::CONFLICT);
    }
    if current.send_attempted {
        *saved = current;
        return Err(StatusCode::CONFLICT);
    }
    let stage = budget.stage(3)?;
    let first = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.check(config, saved, stage.seconds)).await,
    )?;
    let stage = budget.stage(3)?;
    let chain_id = base_network_descriptor(&saved.scope.network)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .chain_id;
    let amount = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.quote(chain_id, first.maximum_fee_wei)).await,
    )?;
    let observations =
        serde_json::to_value(&first).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stage = budget.stage(3)?;
    let authorization = tokio::time::timeout_at(
        stage.deadline,
        store.authorize_timeout_revalidation(&db::TimeoutRevalidationRequest {
            scope: saved.scope.clone(),
            expected_tx_hash: saved.prepared.transaction.tx_hash.clone(),
            lease_token: lease,
            // This deployment has one operator credential, not individual operator identities.
            operator_identity: format!("configured-operator:{}", deployment_revision()),
            observations,
            observed_at: first.observed_at,
            required_budget_micro_usdc: amount,
            validity_seconds: 120,
        }),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    let stage = budget.stage(2)?;
    let final_check = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.check(config, saved, stage.seconds)).await,
    )?;
    if final_check.maximum_fee_wei > first.maximum_fee_wei {
        // No fee mutation or optimistic budget estimate. A later explicit attempt
        // can reserve only the additional budget needed for these same bytes.
        return Err(StatusCode::CONFLICT);
    }
    let observations =
        serde_json::to_value(&final_check).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stage = budget.stage(1)?;
    // An unknown commit outcome is never permission to send. Preserve that
    // uncertainty even when the original row cannot be reloaded during an outage.
    saved.marker_commit_unknown = true;
    let consumed = tokio::time::timeout_at(
        stage.deadline,
        store.consume_timeout_revalidation(
            &saved.scope,
            &saved.prepared.transaction.tx_hash,
            authorization.id,
            lease,
            stage.seconds,
            &observations,
            final_check.observed_at,
            amount,
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    saved.marker_commit_unknown = false;
    if !consumed {
        return Err(StatusCode::CONFLICT);
    }
    saved.send_attempted = true;
    if !stage.may_start_at(tokio::time::Instant::now()) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    // Exactly one dispatch. No retries, signing, fee bump, alternate envelope or
    // fallback provider. A timeout after this point requires reconciliation.
    let _ = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.broadcast(config, saved, stage.seconds)).await,
    );
    Ok(authorization.id)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod local_rehearsal;
