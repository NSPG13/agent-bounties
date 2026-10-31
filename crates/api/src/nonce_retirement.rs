//! Private, default-off, owner-approved retirement of one retained timeout nonce.
use super::*;
use chain_base::nonce_retirement::*;
use chain_base::PreparedBaseRelayTransaction;
use timeout_relay::TimeoutLeaseBudget;

mod retained_dispatch;

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Prepare {
        operation_id: Uuid,
        expected_transaction_hash: String,
    },
    Execute {
        operation_id: Uuid,
        expected_transaction_hash: String,
        plan_id: Uuid,
        approved_digest: String,
    },
    Reconcile {
        operation_id: Uuid,
        expected_transaction_hash: String,
    },
    PrepareRetainedDispatch {
        operation_id: Uuid,
        expected_transaction_hash: String,
        expected_replacement_hash: String,
    },
    ExecuteRetainedDispatch {
        operation_id: Uuid,
        expected_transaction_hash: String,
        expected_replacement_hash: String,
        dispatch_id: Uuid,
        approved_digest: String,
    },
}
impl Request {
    fn identity(&self) -> (Uuid, &str) {
        match self {
            Self::Prepare {
                operation_id,
                expected_transaction_hash,
            }
            | Self::Execute {
                operation_id,
                expected_transaction_hash,
                ..
            }
            | Self::Reconcile {
                operation_id,
                expected_transaction_hash,
            }
            | Self::PrepareRetainedDispatch {
                operation_id,
                expected_transaction_hash,
                ..
            }
            | Self::ExecuteRetainedDispatch {
                operation_id,
                expected_transaction_hash,
                ..
            } => (*operation_id, expected_transaction_hash),
        }
    }
}
struct Config {
    primary: String,
    shadow: String,
    policy: chain_base::timeout_recovery::TimeoutRevalidationPolicy,
}
fn gate(
    enabled: Option<&str>,
    allowed: Option<&str>,
    id: Uuid,
    normal_relay_enabled: bool,
    old_recovery_enabled: bool,
) -> Result<(), StatusCode> {
    if enabled != Some("true")
        || allowed.and_then(|v| v.parse::<Uuid>().ok()) != Some(id)
        || normal_relay_enabled
        || old_recovery_enabled
    {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(())
}
fn config(state: &SharedState, id: Uuid) -> Result<Config, StatusCode> {
    gate(
        env::var("BASE_NONCE_RETIREMENT_ENABLED").ok().as_deref(),
        env::var("BASE_NONCE_RETIREMENT_OPERATION_ID")
            .ok()
            .as_deref(),
        id,
        state.x402_relayer.enabled,
        env_flag("BASE_TIMEOUT_OPERATOR_REVALIDATION_ENABLED"),
    )?;
    let (_, primary) = timeout_rpc_config(&state.base_rpc_urls, "base-mainnet")?;
    let shadow = env::var("BASE_MAINNET_TIMEOUT_REVALIDATION_SHADOW_RPC_URL")
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let planner = configured_autonomous_planner("base-mainnet")?;
    let policy = timeout_revalidation::reviewed_policy(
        "base-mainnet",
        &planner.factory_contract,
        &planner.implementation_contract,
        &state.x402_relayer,
    )?;
    Ok(Config {
        primary,
        shadow,
        policy,
    })
}
trait RetirementIo: Sync {
    fn check(
        &self,
        c: &Config,
        p: &NonceRetirementPlan,
        s: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<NonceRetirementObservation, ChainBaseError>> + Send;
    fn quote(
        &self,
        maximum: u128,
    ) -> impl std::future::Future<Output = Result<u64, ChainBaseError>> + Send;
    fn absent(
        &self,
        c: &Config,
        s: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<(), ChainBaseError>> + Send;
    fn send(
        &self,
        c: &Config,
        s: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<(), ChainBaseError>> + Send;
}
struct LiveIo;
impl RetirementIo for LiveIo {
    async fn check(
        &self,
        c: &Config,
        p: &NonceRetirementPlan,
        s: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        check_nonce_retirement(
            [&c.primary, &c.shadow],
            p,
            &s.prepared,
            &s.scope,
            &c.policy,
            seconds,
        )
        .await
    }
    async fn quote(&self, maximum: u128) -> Result<u64, ChainBaseError> {
        db::quote_gas_budget_micro_usdc(8453, maximum).await
    }
    async fn absent(
        &self,
        c: &Config,
        s: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        require_retirement_absent([&c.primary, &c.shadow], s, seconds).await
    }
    async fn send(
        &self,
        c: &Config,
        s: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        let transport =
            chain_base::ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(seconds))?;
        let result = chain_base::broadcast_signed_transaction_with_transport(
            &c.primary,
            &s.signed_transaction,
            7782,
            &transport,
        )
        .await?;
        if result.result != s.transaction.tx_hash {
            return Err(ChainBaseError::InvalidRelayIntent(
                "retirement submission requires reconciliation".into(),
            ));
        }
        Ok(())
    }
}
fn status(saved: &db::SavedNonceRetirement) -> Response {
    Json(serde_json::json!({"operation_id":saved.plan.plan.operation_id,"plan_id":saved.plan.id,"digest":saved.plan.digest,"transaction_hash":saved.prepared.transaction.tx_hash,"send_attempted":saved.send_attempted,"status":if saved.confirmed{"finalized"}else{"reconciliation_required"},"sender_fenced":!saved.confirmed})).into_response()
}
pub(super) async fn handle(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Response, StatusCode> {
    require_operator(&state, &headers)?;
    let (id, hash) = request.identity();
    let hash = hash.to_string();
    let c = config(&state, id)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let old = store
        .timeout_relay(id, "base-mainnet")
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if old.send_attempted || old.prepared.transaction.tx_hash != hash {
        return Err(StatusCode::CONFLICT);
    }
    if let Some(mut saved) = store
        .nonce_retirement(id)
        .await
        .map_err(map_x402_db_error)?
    {
        if matches!(
            &request,
            Request::PrepareRetainedDispatch { .. } | Request::ExecuteRetainedDispatch { .. }
        ) && !saved.send_attempted
            && !saved.confirmed
        {
            return retained_dispatch::handle(&state, &c, &old, &saved, request).await;
        }
        // Existing prepare/execute/reconcile stay read-only after durable signing.
        // Any attempted retained dispatch is also permanently reconciliation-only.
        if !saved.confirmed {
            if let Ok(observation) = confirm_nonce_retirement(
                [&c.primary, &c.shadow],
                &saved.plan.plan,
                &saved.prepared,
                state.x402_relayer.rpc_timeout_seconds,
            )
            .await
            {
                store
                    .confirm_nonce_retirement(&saved, &observation)
                    .await
                    .map_err(map_x402_db_error)?;
                saved.confirmed = true;
            }
        }
        return Ok(status(&saved));
    }
    if matches!(&request, Request::Reconcile { .. }) {
        return Ok(
            Json(serde_json::json!({"operation_id":id,"status":"unsigned"})).into_response(),
        );
    }
    if matches!(
        &request,
        Request::PrepareRetainedDispatch { .. } | Request::ExecuteRetainedDispatch { .. }
    ) {
        return Err(StatusCode::CONFLICT);
    }
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if !relayer.has_gas_budget() || relayer.address() != old.prepared.transaction.relayer {
        return Err(StatusCode::CONFLICT);
    }
    let budget = TimeoutLeaseBudget::new(
        tokio::time::Instant::now(),
        state.x402_relayer.lease_seconds,
        state.x402_relayer.rpc_timeout_seconds,
    )?;
    let stage = budget.stage(3)?;
    let lease = timeout_lease_result(
        tokio::time::timeout_at(
            stage.deadline,
            store.acquire_nonce_retirement_lease(id, state.x402_relayer.lease_seconds),
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    )?;
    let result=async {
        match request {
            Request::Prepare{..}=>{
                let stage=budget.stage(1)?;
                let (plan,observation)=timeout_execution_result(tokio::time::timeout_at(stage.deadline,quote_nonce_retirement([&c.primary,&c.shadow],&old.prepared,&old.scope,&c.policy,stage.seconds)).await)?;
                let stage=budget.stage(1)?;
                let saved=tokio::time::timeout_at(stage.deadline,store.save_nonce_retirement_plan(&old.scope,lease,&plan,&observation)).await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.map_err(map_x402_db_error)?;
                Ok(Json(serde_json::json!({"status":"awaiting_owner_approval","plan":saved,"sender_fenced":true,"transaction":{"chain_id":8453,"from":plan.sender,"to":plan.sender,"value":"0","data":"0x","nonce":plan.nonce},"maximum_cost_wei":plan.maximum_cost_wei.to_string()})).into_response())
            },
            Request::Execute{plan_id,approved_digest,..}=>{
                let plan=store.nonce_retirement_plan(plan_id).await.map_err(map_x402_db_error)?.ok_or(StatusCode::NOT_FOUND)?;
                if plan.plan.operation_id!=id || plan.plan.original_tx_hash!=hash || approved_digest!=plan.digest{return Err(StatusCode::CONFLICT);}
                let outcome=first_send(&state,&c,&old,&plan,&approved_digest,lease,budget,&LiveIo).await;
                if let Some(saved)=store.nonce_retirement(id).await.map_err(map_x402_db_error)?{return Ok(status(&saved));}
                outcome?;Err(StatusCode::CONFLICT)
            },Request::Reconcile{..}|Request::PrepareRetainedDispatch{..}|Request::ExecuteRetainedDispatch{..}=>unreachable!(),
        }
    }.await;
    // Release failure grants no additional lifetime or execution attempt.
    let _ = store
        .release_x402_relayer_lease("base-mainnet", lease)
        .await;
    result
}
#[allow(clippy::too_many_arguments)]
async fn first_send(
    state: &SharedState,
    c: &Config,
    old: &db::SavedTimeoutRelay,
    plan: &db::SavedNonceRetirementPlan,
    approved_digest: &str,
    lease: Uuid,
    budget: TimeoutLeaseBudget,
    io: &impl RetirementIo,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let stage = budget.stage(3)?;
    let first = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.check(c, &plan.plan, old, stage.seconds)).await,
    )?;
    let stage = budget.stage(3)?;
    let amount = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.quote(plan.plan.maximum_cost_wei)).await,
    )?;
    let stage = budget.stage(3)?;
    tokio::time::timeout_at(
        stage.deadline,
        store.approve_nonce_retirement(
            &old.scope,
            plan,
            approved_digest,
            lease,
            &format!("configured-operator:{}", deployment_revision()),
            &first,
            amount,
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    let stage = budget.stage(3)?;
    let prepared = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, relayer.sign_nonce_retirement(&plan.plan)).await,
    )?;
    // If this commit is uncertain, do not dispatch or sign another envelope.
    tokio::time::timeout_at(
        stage.deadline,
        store.save_nonce_retirement_envelope(&old.scope, plan, lease, &prepared),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    let saved = db::SavedNonceRetirement {
        plan: plan.clone(),
        prepared,
        send_attempted: false,
        confirmed: false,
    };
    let stage = budget.stage(2)?;
    let final_check = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.check(c, &plan.plan, old, stage.seconds)).await,
    )?;
    let stage = budget.stage(2)?;
    timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.absent(c, &saved.prepared, stage.seconds)).await,
    )?;
    let stage = budget.stage(1)?;
    let consumed = tokio::time::timeout_at(
        stage.deadline,
        store.consume_nonce_retirement_send(&old.scope, &saved, lease, stage.seconds, &final_check),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    if !consumed || !stage.may_start_at(tokio::time::Instant::now()) {
        return Err(StatusCode::CONFLICT);
    }
    // One attempt only. Even a lost or rejected response is reconciliation-only.
    let _ = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.send(c, &saved.prepared, stage.seconds)).await,
    );
    Ok(())
}
#[cfg(test)]
mod tests;

#[cfg(test)]
mod local_rehearsal;
