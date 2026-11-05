//! Explicit initial submission of retained bytes. There is no signer call here.
use super::*;
use chain_base::nonce_retirement::retained_dispatch::{
    check_retained_dispatch, RetainedNonceDispatch,
};

pub(super) trait DispatchIo: Sync {
    fn check(
        &self,
        c: &Config,
        p: &RetainedNonceDispatch,
        saved: &db::SavedNonceRetirement,
        old: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<NonceRetirementObservation, ChainBaseError>> + Send;
    fn quote(
        &self,
        maximum: u128,
    ) -> impl std::future::Future<Output = Result<u64, ChainBaseError>> + Send;
    fn send(
        &self,
        c: &Config,
        saved: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> impl std::future::Future<Output = Result<(), ChainBaseError>> + Send;
}
struct LiveDispatch;
impl DispatchIo for LiveDispatch {
    async fn check(
        &self,
        c: &Config,
        p: &RetainedNonceDispatch,
        saved: &db::SavedNonceRetirement,
        old: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        check_retained_dispatch(
            [&c.primary, &c.shadow],
            p,
            saved.plan.id,
            &saved.plan.plan,
            &old.prepared,
            &saved.prepared,
            &old.scope,
            &c.policy,
            seconds,
        )
        .await
    }
    async fn quote(&self, maximum: u128) -> Result<u64, ChainBaseError> {
        db::quote_gas_budget_micro_usdc(8453, maximum).await
    }
    async fn send(
        &self,
        c: &Config,
        saved: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        LiveIo.send(c, saved, seconds).await
    }
}

fn enabled(value: Option<&str>) -> Result<(), StatusCode> {
    if value == Some("true") {
        Ok(())
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

pub(super) async fn handle(
    state: &SharedState,
    c: &Config,
    old: &db::SavedTimeoutRelay,
    saved: &db::SavedNonceRetirement,
    request: Request,
) -> Result<Response, StatusCode> {
    enabled(
        env::var("BASE_RETAINED_NONCE_DISPATCH_ENABLED")
            .ok()
            .as_deref(),
    )?;
    let expected = match &request {
        Request::PrepareRetainedDispatch {
            expected_replacement_hash,
            ..
        }
        | Request::ExecuteRetainedDispatch {
            expected_replacement_hash,
            ..
        } => expected_replacement_hash,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    if expected != &saved.prepared.transaction.tx_hash || saved.send_attempted || saved.confirmed {
        return Err(StatusCode::CONFLICT);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let budget = TimeoutLeaseBudget::new(
        tokio::time::Instant::now(),
        state.x402_relayer.lease_seconds,
        state.x402_relayer.rpc_timeout_seconds,
    )?;
    let stage = budget.stage(3)?;
    let lease = timeout_lease_result(
        tokio::time::timeout_at(
            stage.deadline,
            store.acquire_nonce_retirement_lease(
                saved.plan.plan.operation_id,
                state.x402_relayer.lease_seconds,
            ),
        )
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
    )?;
    let result=async {
        match request {
            Request::PrepareRetainedDispatch { .. } => {
                let proposal=RetainedNonceDispatch::propose([&c.primary,&c.shadow],saved.plan.id,
                    &saved.plan.plan,&saved.prepared,Utc::now()).map_err(|_|StatusCode::CONFLICT)?;
                let stage=budget.stage(1)?;
                let observation=timeout_execution_result(tokio::time::timeout_at(stage.deadline,
                    LiveDispatch.check(c,&proposal,saved,old,stage.seconds)).await)?;
                let stage=budget.stage(1)?;
                let proposal=tokio::time::timeout_at(stage.deadline,
                    store.save_retained_nonce_dispatch(&old.scope,saved,lease,&proposal,&observation))
                    .await.map_err(|_|StatusCode::SERVICE_UNAVAILABLE)?.map_err(map_x402_db_error)?;
                Ok(Json(serde_json::json!({"status":"awaiting_owner_approval",
                    "authority":"initial_dispatch_saved_envelope","dispatch":proposal,
                    "transaction_hash":saved.prepared.transaction.tx_hash,"sender_fenced":true,
                    "transaction":{"chain_id":8453,"from":saved.plan.plan.sender,"to":saved.plan.plan.sender,
                        "value":"0","data":"0x","nonce":saved.plan.plan.nonce,
                        "gas_limit":saved.plan.plan.gas_limit,
                        "max_fee_per_gas_wei":saved.plan.plan.max_fee_per_gas_wei.to_string(),
                        "max_priority_fee_per_gas_wei":saved.plan.plan.max_priority_fee_per_gas_wei.to_string()},
                    "maximum_cost_wei":saved.plan.plan.maximum_cost_wei.to_string()})).into_response())
            },
            Request::ExecuteRetainedDispatch { dispatch_id,approved_digest,.. } => {
                let dispatch=store.retained_nonce_dispatch(dispatch_id).await.map_err(map_x402_db_error)?
                    .ok_or(StatusCode::NOT_FOUND)?;
                dispatch.proposal.bind(saved.plan.id,&saved.plan.plan,&saved.prepared)
                    .map_err(|_|StatusCode::CONFLICT)?;
                if dispatch.digest != approved_digest { return Err(StatusCode::CONFLICT); }
                let result=initial_dispatch(state,c,old,saved,&dispatch,&approved_digest,lease,budget,&LiveDispatch).await;
                // Preserve the reason when no marker was committed. Never retry here.
                let latest=store.nonce_retirement(saved.plan.plan.operation_id).await.map_err(map_x402_db_error)?
                    .ok_or(StatusCode::CONFLICT)?;
                if latest.send_attempted || latest.confirmed { return Ok(status(&latest)); }
                result?;
                Err(StatusCode::CONFLICT)
            },
            _ => Err(StatusCode::BAD_REQUEST),
        }
    }.await;
    let _ = store
        .release_x402_relayer_lease("base-mainnet", lease)
        .await;
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn initial_dispatch(
    state: &SharedState,
    c: &Config,
    old: &db::SavedTimeoutRelay,
    saved: &db::SavedNonceRetirement,
    dispatch: &db::SavedRetainedNonceDispatch,
    approved_digest: &str,
    lease: Uuid,
    budget: TimeoutLeaseBudget,
    io: &impl DispatchIo,
) -> Result<(), StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    if saved.send_attempted || saved.confirmed || approved_digest != dispatch.digest {
        return Err(StatusCode::CONFLICT);
    }
    dispatch
        .proposal
        .bind(saved.plan.id, &saved.plan.plan, &saved.prepared)
        .map_err(|_| StatusCode::CONFLICT)?;
    let stage = budget.stage(3)?;
    let amount = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.quote(dispatch.proposal.maximum_cost_wei)).await,
    )?;
    // One complete fresh check immediately before the latch; no signing in between.
    let stage = budget.stage(2)?;
    let observation = timeout_execution_result(
        tokio::time::timeout_at(
            stage.deadline,
            io.check(c, &dispatch.proposal, saved, old, stage.seconds),
        )
        .await,
    )?;
    let stage = budget.stage(1)?;
    tokio::time::timeout_at(
        stage.deadline,
        store.consume_retained_nonce_dispatch(
            &old.scope,
            saved,
            dispatch,
            approved_digest,
            lease,
            &format!("configured-operator:{}", deployment_revision()),
            stage.seconds,
            &observation,
            amount,
        ),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .map_err(map_x402_db_error)?;
    if !stage.may_start_at(tokio::time::Instant::now()) {
        return Err(StatusCode::CONFLICT);
    }
    // Any provider result is terminal for dispatch authority, including 429/timeouts.
    let _ = timeout_execution_result(
        tokio::time::timeout_at(stage.deadline, io.send(c, &saved.prepared, stage.seconds)).await,
    );
    Ok(())
}

#[cfg(test)]
mod tests;
