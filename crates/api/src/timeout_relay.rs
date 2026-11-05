//! Permissionless lifecycle recovery, with a durable one-send boundary.
use super::*;
use chain_base::timeout_recovery::{
    validate_timeout_envelope, verify_timeout_scope, TimeoutRelayScope,
};

// The database send marker requires two seconds beyond the send timeout.
// Keep another whole second for rounding/dispatch; every stage also consumes
// the same monotonic deadline captured BEFORE lease acquisition. Database
// waits therefore reduce the budget instead of silently extending the lease.
const TIMEOUT_LEASE_MARGIN_SECONDS: u64 = 3;

#[derive(Clone, Copy)]
pub(super) struct TimeoutLeaseBudget {
    deadline: tokio::time::Instant,
    rpc_timeout_seconds: u64,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TimeoutStage {
    pub(super) deadline: tokio::time::Instant,
    pub(super) seconds: u64,
}

impl TimeoutStage {
    pub(super) fn may_start_at(&self, now: tokio::time::Instant) -> bool {
        now < self.deadline
    }
}

fn lease_budget_exhausted() -> StatusCode {
    record_failure_stage(
        FailureStage::TimeoutRelayExecutionTimedOut,
        &deployment_revision(),
    );
    StatusCode::SERVICE_UNAVAILABLE
}

impl TimeoutLeaseBudget {
    pub(super) fn new(
        started: tokio::time::Instant,
        lease_seconds: u64,
        rpc_timeout_seconds: u64,
    ) -> Result<Self, StatusCode> {
        Ok(Self {
            deadline: started
                .checked_add(Duration::from_secs(lease_seconds))
                .ok_or_else(lease_budget_exhausted)?,
            rpc_timeout_seconds,
        })
    }

    pub(super) fn stage(&self, remaining_stages: u32) -> Result<TimeoutStage, StatusCode> {
        self.stage_at(tokio::time::Instant::now(), remaining_stages)
    }

    fn stage_at(
        &self,
        now: tokio::time::Instant,
        remaining_stages: u32,
    ) -> Result<TimeoutStage, StatusCode> {
        let seconds = self
            .deadline
            .checked_duration_since(now)
            .ok_or_else(lease_budget_exhausted)?
            .as_secs()
            .saturating_sub(TIMEOUT_LEASE_MARGIN_SECONDS)
            .checked_div(u64::from(remaining_stages))
            .ok_or_else(lease_budget_exhausted)?
            .min(self.rpc_timeout_seconds);
        if seconds == 0 {
            return Err(lease_budget_exhausted());
        }
        Ok(TimeoutStage {
            deadline: now + Duration::from_secs(seconds),
            seconds,
        })
    }
}

pub(super) async fn handle(
    state: &SharedState,
    request: RelayAutonomousTimeoutRequest,
) -> Result<Response, StatusCode> {
    if request.round == 0 {
        return Err(StatusCode::BAD_REQUEST);
    }
    let descriptor = base_network_descriptor(request.network.as_deref().unwrap_or("base-mainnet"))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let network = match descriptor.chain_id {
        8453 => "base-mainnet",
        84532 => "base-sepolia",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let contract =
        normalize_evm_address(&request.bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let store = state.store.as_ref().ok_or_else(|| {
        record_failure_stage(
            FailureStage::TimeoutDurableStoreNotConfigured,
            &deployment_revision(),
        );
        StatusCode::SERVICE_UNAVAILABLE
    })?;
    let action = match request.action {
        AutonomousTimeoutAction::ExpireClaim => "expire_claim",
        AutonomousTimeoutAction::ExpireSubmission => "expire_submission",
    };
    // Only these four fields define the operation. Fetching an existing result
    // precedes current lifecycle eligibility, so an advanced round cannot hide it.
    let mut scope = TimeoutRelayScope {
        network: network.into(),
        bounty_contract: contract.clone(),
        action: action.into(),
        round: request.round,
        factory_contract: format!("0x{}", "00".repeat(20)),
        bounty_id: format!("0x{}", "00".repeat(32)),
        terms_hash: format!("0x{}", "00".repeat(32)),
    };
    let id = scope.operation_id().map_err(|_| StatusCode::BAD_REQUEST)?;
    let existing = store
        .timeout_relay(id, network)
        .await
        .map_err(map_x402_db_error)?;
    if let Some(saved) = &existing {
        if !store
            .timeout_publication_matches(&saved.scope)
            .await
            .map_err(map_x402_db_error)?
        {
            return Err(StatusCode::NOT_FOUND);
        }
    }
    let planner_result = configured_autonomous_planner(network);
    let rpc_result = timeout_rpc_config(&state.base_rpc_urls, network);
    // Configuration or RPC outages cannot hide an already saved transaction.
    if let Some(saved) = &existing {
        if planner_result.as_ref().map_or(true, |planner| {
            planner.factory_contract != saved.scope.factory_contract
        }) || rpc_result.is_err()
        {
            return Ok(known_response(
                request.action,
                saved,
                Err(StatusCode::SERVICE_UNAVAILABLE),
            ));
        }
    }
    let planner = planner_result?;
    let (_, rpc) = rpc_result?;
    if let Some(saved) = &existing {
        if saved.scope.factory_contract != planner.factory_contract {
            return Err(StatusCode::CONFLICT);
        }
        scope = saved.scope.clone();
        if saved.send_attempted {
            return Ok(reconcile(state, &rpc, request.action, saved).await);
        }
    } else {
        scope.factory_contract = planner.factory_contract.clone();
        let item = indexed_autonomous_bounty(state, network, &contract)
            .await
            .inspect_err(|status| {
                if status.is_server_error() {
                    record_failure_stage(
                        FailureStage::TimeoutIndexedLookup,
                        &deployment_revision(),
                    );
                }
            })?;
        if !item.terms_valid || item.status != request.action.previous_bounty_state() {
            return Err(StatusCode::CONFLICT);
        }
        scope.bounty_id = item.bounty_id;
        scope.terms_hash = item.terms_hash;
        scope.validate().map_err(|_| StatusCode::CONFLICT)?;
    }
    let relayer = match timeout_relayer(&state.x402_relayer) {
        Ok(relayer) if relayer.has_gas_budget() => relayer,
        _ => {
            return match existing.as_ref() {
                Some(saved) => Ok(known_response(
                    request.action,
                    saved,
                    Err(StatusCode::SERVICE_UNAVAILABLE),
                )),
                None => Err(StatusCode::SERVICE_UNAVAILABLE),
            }
        }
    };
    let sender = relayer.address();
    let intent = match request.action {
        AutonomousTimeoutAction::ExpireClaim => planner.plan_expire_claim(&contract, Some(&sender)),
        AutonomousTimeoutAction::ExpireSubmission => {
            planner.plan_expire_submission(&contract, Some(&sender))
        }
    }
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_autonomous_timeout_intent(&intent, &contract, &sender, request.action)?;
    let budget = TimeoutLeaseBudget::new(
        tokio::time::Instant::now(),
        state.x402_relayer.lease_seconds,
        state.x402_relayer.rpc_timeout_seconds,
    )?;
    let lease = match timeout_lease_result(
        store
            .acquire_x402_relayer_lease(network, state.x402_relayer.lease_seconds)
            .await,
    ) {
        Ok(lease) => lease,
        Err(status) => {
            return match existing.as_ref() {
                Some(saved) => Ok(known_response(request.action, saved, Err(status))),
                None => Err(status),
            }
        }
    };
    let result = first_send(
        state,
        &rpc,
        &scope,
        &intent,
        relayer,
        (lease, budget),
        existing.clone(),
    )
    .await;
    if store
        .release_x402_relayer_lease(network, lease)
        .await
        .is_err()
    {
        record_failure_stage(
            FailureStage::TimeoutLeaseReleaseFailed,
            &deployment_revision(),
        );
    }
    // A saved hash is always returned even when preparation of its first send
    // failed. No error after a send marker can authorize another envelope/send.
    let saved = match result {
        Ok(saved) => saved,
        Err(status) => match store.timeout_relay(id, network).await {
            Ok(Some(saved)) => saved,
            _ => match existing {
                Some(saved) => saved,
                None => return Err(status),
            },
        },
    };
    Ok(reconcile(state, &rpc, request.action, &saved).await)
}

async fn first_send(
    state: &SharedState,
    rpc: &str,
    scope: &TimeoutRelayScope,
    intent: &EvmTransactionIntent,
    relayer: &BaseTransactionRelayer,
    lease: (Uuid, TimeoutLeaseBudget),
    existing: Option<db::SavedTimeoutRelay>,
) -> Result<db::SavedTimeoutRelay, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (lease, budget) = lease;
    // Another request may have saved or sent while this request awaited the
    // network lease. Re-read under the lease before allocating any nonce.
    let saved = store
        .timeout_relay(
            scope.operation_id().map_err(|_| StatusCode::BAD_REQUEST)?,
            &scope.network,
        )
        .await
        .map_err(map_x402_db_error)?
        .or(existing);
    if let Some(saved) = saved.as_ref().filter(|saved| saved.send_attempted) {
        return Ok(saved.clone());
    }
    if saved.as_ref().is_some_and(|saved| saved.scope != *scope) {
        return Err(StatusCode::CONFLICT);
    }
    let preflight = budget.stage(if saved.is_some() { 3 } else { 4 })?;
    let nonce = timeout_execution_result(
        tokio::time::timeout_at(
            preflight.deadline,
            verify_timeout_scope(
                rpc,
                scope,
                &relayer.address(),
                saved.as_ref().map(|saved| saved.prepared.nonce),
            ),
        )
        .await,
    )?;
    let prepared = if let Some(saved) = saved {
        saved.prepared
    } else {
        let preparation_day = store
            .timeout_budget_day()
            .await
            .map_err(map_x402_db_error)?;
        let preparation = budget.stage(3)?;
        let prepared = timeout_execution_result(
            tokio::time::timeout_at(
                preparation.deadline,
                relayer.prepare_signed_transaction(
                    rpc,
                    base_network_descriptor(&scope.network)
                        .map_err(|_| StatusCode::BAD_REQUEST)?
                        .chain_id,
                    intent,
                    state.x402_relayer.max_gas,
                    state.x402_relayer.max_fee_per_gas_wei,
                ),
            )
            .await,
        )?;
        if prepared.nonce != nonce {
            return Err(StatusCode::CONFLICT);
        }
        validate_timeout_envelope(
            &prepared,
            scope,
            &relayer.address(),
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        )
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
        // Do not allocate a new durable envelope when the remaining recheck
        // and send cannot each receive a bounded stage. A slow database write
        // can still exhaust this lease; the saved envelope is then retained.
        budget.stage(2)?;
        store
            .save_timeout_relay(scope, lease, &prepared, preparation_day)
            .await
            .map_err(map_x402_db_error)?;
        prepared
    };
    validate_timeout_envelope(
        &prepared,
        scope,
        &relayer.address(),
        state.x402_relayer.max_gas,
        state.x402_relayer.max_fee_per_gas_wei,
    )
    .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;
    let recheck = budget.stage(2)?;
    timeout_execution_result(
        tokio::time::timeout_at(
            recheck.deadline,
            verify_timeout_scope(rpc, scope, &relayer.address(), Some(prepared.nonce)),
        )
        .await,
    )?;
    let mut saved = db::SavedTimeoutRelay {
        scope: scope.clone(),
        prepared,
        send_attempted: false,
        marker_commit_unknown: false,
    };
    let send = budget.stage(1)?;
    match store
        .claim_timeout_relay_send(scope, lease, send.seconds)
        .await
    {
        Ok(true) => saved.send_attempted = true,
        Ok(false) => {
            return store
                .timeout_relay(
                    scope.operation_id().map_err(|_| StatusCode::BAD_REQUEST)?,
                    &scope.network,
                )
                .await
                .map_err(map_x402_db_error)?
                .ok_or(StatusCode::SERVICE_UNAVAILABLE)
        }
        Err(_) => {
            // The marker commit itself is uncertain, but this request has not
            // sent. Retain the hash; a later durable read decides its state.
            saved.marker_commit_unknown = true;
            return Ok(saved);
        }
    }
    // Only this branch can send, once. Even timeout, 429, malformed response,
    // node rejection or process death leave the durable marker in place.
    if !send.may_start_at(tokio::time::Instant::now()) {
        // The marker may already be committed. Preserve uncertainty and never
        // start a late send or clear the marker to obtain another attempt.
        lease_budget_exhausted();
        return Ok(saved);
    }
    let outcome = tokio::time::timeout_at(
        send.deadline,
        chain_base::broadcast_signed_transaction(rpc, &saved.prepared.signed_transaction, 90),
    )
    .await;
    // The envelope/hash remain available even if broadcast or lease cleanup
    // fails. Receipt reconciliation, never the provider's reply, proves success.
    let _ = timeout_execution_result(outcome);
    Ok(saved)
}

pub(super) async fn reconcile(
    state: &SharedState,
    rpc: &str,
    action: AutonomousTimeoutAction,
    saved: &db::SavedTimeoutRelay,
) -> Response {
    reconcile_with_observation(state, rpc, action, saved, false).await
}

pub(super) async fn reconcile_with_observation(
    state: &SharedState,
    rpc: &str,
    action: AutonomousTimeoutAction,
    saved: &db::SavedTimeoutRelay,
    observe_unsent: bool,
) -> Response {
    let scope = &saved.scope;
    let observation = if observe_unsent || saved.send_attempted || saved.marker_commit_unknown {
        match tokio::time::timeout(
            Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
            wait_for_timeout_confirmation(
                rpc,
                scope,
                &saved.prepared.transaction,
                state.x402_relayer.confirmations,
                0,
            ),
        )
        .await
        {
            Ok(result) => timeout_confirmation_result(result),
            Err(_) => timeout_confirmation_result(Err(StatusCode::SERVICE_UNAVAILABLE)),
        }
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    };
    known_response(action, saved, observation)
}
pub(super) fn known_response(
    action: AutonomousTimeoutAction,
    saved: &db::SavedTimeoutRelay,
    observation: Result<TimeoutRelayConfirmation, StatusCode>,
) -> Response {
    let scope = &saved.scope;
    let response = timeout_response_after_broadcast(
        &scope.network,
        scope.bounty_contract.clone(),
        action,
        scope,
        saved,
        observation,
    );
    (
        if response.confirmed {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        [(header::CACHE_CONTROL, "no-store, private")],
        Json(response),
    )
        .into_response()
}

#[cfg(test)]
pub(super) fn test_saved() -> db::SavedTimeoutRelay {
    db::SavedTimeoutRelay {
        scope: TimeoutRelayScope {
            network: "base-mainnet".into(),
            bounty_contract: format!("0x{}", "11".repeat(20)),
            action: "expire_claim".into(),
            round: 1,
            factory_contract: format!("0x{}", "22".repeat(20)),
            bounty_id: format!("0x{}", "33".repeat(32)),
            terms_hash: format!("0x{}", "44".repeat(32)),
        },
        prepared: chain_base::PreparedBaseRelayTransaction {
            nonce: 9,
            signed_transaction: "0x010203".into(),
            transaction: BaseRelayedTransaction {
                relayer: format!("0x{}", "22".repeat(20)),
                tx_hash: format!("0x{}", "ab".repeat(32)),
                estimated_gas: 21000,
                gas_limit: 25000,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75000,
            },
        },
        send_attempted: true,
        marker_commit_unknown: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        eips::Encodable2718,
        network::{EthereumWallet, TransactionBuilder},
        primitives::{Address, Bytes, U256},
        rpc::types::TransactionRequest,
        signers::local::PrivateKeySigner,
    };
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        routing::post,
    };
    use serde_json::{json, Value};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use tower::ServiceExt;
    fn word(n: u64) -> String {
        format!("0x{n:064x}")
    }
    fn selector(sig: &str) -> String {
        format!(
            "0x{}",
            hex::encode(&alloy::primitives::keccak256(sig.as_bytes())[..4])
        )
    }

    // Only the price/reservation boundary is local here: nonce occupancy still
    // uses the real Postgres implementation. No Coinbase/model request occurs.
    struct LocalGasBudget {
        store: PostgresStore,
        reservations: AtomicUsize,
    }

    #[axum::async_trait]
    impl chain_base::RelayGasBudget for LocalGasBudget {
        async fn ensure_sender_nonce_available(
            &self,
            chain_id: u64,
            sender: &str,
            nonce: u64,
        ) -> Result<(), chain_base::ChainBaseError> {
            self.store
                .gas_sponsor_budget()
                .ensure_sender_nonce_available(chain_id, sender, nonce)
                .await
        }

        async fn reserve(
            &self,
            chain_id: u64,
            maximum_fee_wei: u128,
        ) -> Result<(), chain_base::ChainBaseError> {
            assert_eq!(chain_id, 8453);
            assert_eq!(maximum_fee_wei, 72_004); // 24k gas * 3 + two padded oracle fees
            self.reservations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    // Use only the container created by run-with-test-postgres.py, never an
    // arbitrary configured database. The operation-specific trigger delays a
    // successful UPDATE commit; it does not replace the production SQL guard.
    struct MarkerDelay {
        container: String,
        name: String,
        active: bool,
    }

    impl MarkerDelay {
        fn sql(&self, sql: &str) -> bool {
            std::process::Command::new("docker")
                .args([
                    "exec",
                    &self.container,
                    "psql",
                    "-X",
                    "-q",
                    "-U",
                    "agent_bounties_test",
                    "-d",
                    "agent_bounties_test",
                    "-v",
                    "ON_ERROR_STOP=1",
                    "-c",
                    sql,
                ])
                .output()
                .is_ok_and(|out| out.status.success())
        }

        fn install(id: Uuid) -> Self {
            assert_eq!(
                env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
                Ok("1")
            );
            let database =
                url::Url::parse(&env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap()).unwrap();
            assert_eq!(database.host_str(), Some("127.0.0.1"));
            assert_eq!(database.username(), "agent_bounties_test");
            assert_eq!(database.path(), "/agent_bounties_test");
            let container = env::var("AGENT_BOUNTIES_TEST_POSTGRES_CONTAINER").unwrap();
            let suffix = container.strip_prefix("agent-bounties-test-").unwrap();
            assert_eq!(suffix.len(), 12);
            assert!(suffix.bytes().all(|byte| byte.is_ascii_hexdigit()));
            let delay = Self {
                container,
                name: format!("timeout_delay_{}", Uuid::new_v4().simple()),
                active: true,
            };
            assert!(delay.sql(&format!(
                "SET statement_timeout='5s'; CREATE FUNCTION {0}() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE TRIGGER {0} AFTER UPDATE ON relay_transaction_outbox FOR EACH ROW WHEN (NEW.operation_id='{id}'::uuid AND OLD.send_attempts=0 AND NEW.send_attempts=1) EXECUTE FUNCTION {0}();", delay.name
            )));
            delay
        }

        fn remove(&mut self) -> bool {
            let removed = self.sql(&format!(
                "SET statement_timeout='5s'; DROP TRIGGER IF EXISTS {0} ON relay_transaction_outbox; DROP FUNCTION IF EXISTS {0}();", self.name
            ));
            if removed {
                self.active = false;
            }
            removed
        }
    }

    impl Drop for MarkerDelay {
        fn drop(&mut self) {
            if self.active {
                let _ = self.remove();
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn timeout_journal_short_lease_prepares_fresh_envelope_through_all_four_stages() {
        run_fresh_lease_case(false).await;
    }

    #[tokio::test]
    #[ignore = "requires owned Docker fixture from scripts/run-with-test-postgres.py"]
    async fn timeout_journal_delayed_marker_commit_never_starts_expired_send() {
        run_fresh_lease_case(true).await;
    }

    async fn run_fresh_lease_case(delay_marker: bool) {
        let database = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        assert!(database.contains("@127.0.0.1:"));
        let store = PostgresStore::connect(&database).await.unwrap();
        store.migrate().await.unwrap();
        let mut scope = test_saved().scope;
        scope.bounty_contract = format!("0x{:040x}", Uuid::new_v4().as_u128());
        scope.bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
        scope.terms_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
        let id = scope.operation_id().unwrap();
        assert!(store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .is_none());
        let gas_budget = Arc::new(LocalGasBudget {
            store: store.clone(),
            reservations: AtomicUsize::new(0),
        });
        let signer = PrivateKeySigner::random();
        let relayer = BaseTransactionRelayer::from_private_key(&format!(
            "0x{}",
            hex::encode(signer.to_bytes())
        ))
        .unwrap()
        .with_gas_budget(gas_budget.clone());
        let sends = Arc::new(AtomicUsize::new(0));
        let estimates = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = format!("http://{}", listener.local_addr().unwrap());
        let server_scope = scope.clone();
        let server_store = store.clone();
        let send_counter = sends.clone();
        let estimate_counter = estimates.clone();
        let now = Utc::now().timestamp() as u64;
        let server = tokio::spawn(async move {
            let app = Router::new().route("/", post(move |Json(request): Json<Value>| {
                let scope = server_scope.clone(); let store = server_store.clone();
                let sends = send_counter.clone(); let estimates = estimate_counter.clone();
                async move {
                    let result = match request["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x2105"),
                        "eth_blockNumber" => json!("0x64"),
                        "eth_getBlockByNumber" => json!({"number":"0x64","hash":word(100),"timestamp":format!("0x{now:x}")}),
                        "eth_getTransactionCount" => json!("0x9"),
                        "eth_getBalance" => json!("0xde0b6b3a7640000"),
                        "eth_estimateGas" => { estimates.fetch_add(1, Ordering::SeqCst); json!("0x4e20") },
                        "eth_feeHistory" => json!({"oldestBlock":"0x63","baseFeePerGas":["0x1","0x1"],"gasUsedRatio":[0.5],"reward":[["0x1"]]}),
                        "eth_call" => {
                            let data = request["params"][0].get("data")
                                .or_else(|| request["params"][0].get("input"))
                                .and_then(Value::as_str).unwrap();
                            let out = if data == selector("status()") { word(2) }
                            else if data == selector("round()") { word(scope.round) }
                            else if data == selector("factory()") { format!("0x{:0>64}", scope.factory_contract.trim_start_matches("0x")) }
                            else if data == selector("bountyId()") { scope.bounty_id.clone() }
                            else if data == selector("termsHash()") { scope.terms_hash.clone() }
                            else if data.starts_with(&selector("isCanonicalBounty(address)")) { word(1) }
                            else if data == selector("claimExpiresAt()") || data == selector("verificationExpiresAt()") { word(now-1) }
                            else if data == selector("expireClaim()") { "0x".into() }
                            else if data.starts_with(&selector("getL1FeeUpperBound(uint256)")) || data.starts_with(&selector("getOperatorFee(uint256)")) { word(1) }
                            else { panic!("unexpected local simulation") };
                            json!(out)
                        },
                        "eth_sendRawTransaction" => {
                            sends.fetch_add(1, Ordering::SeqCst);
                            let saved = store.timeout_relay(id, &scope.network).await.unwrap().unwrap();
                            assert!(saved.send_attempted, "durable marker must precede the wire request");
                            assert_eq!(request["params"][0], saved.prepared.signed_transaction);
                            json!(saved.prepared.transaction.tx_hash)
                        },
                        "eth_getTransactionReceipt" => Value::Null,
                        _ => panic!("unexpected local RPC"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
                }
            }));
            axum::serve(listener, app).await.unwrap();
        });
        let mut state = crate::tests::test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "unused",
            store.clone(),
        );
        let config = Arc::make_mut(&mut state);
        config.x402_relayer.lease_seconds = 16;
        config.x402_relayer.rpc_timeout_seconds = if delay_marker { 1 } else { 15 };
        config.x402_relayer.max_gas = 25_000;
        config.x402_relayer.max_fee_per_gas_wei = 3;
        let intent = EvmTransactionIntent {
            from: Some(relayer.address()),
            to: scope.bounty_contract.clone(),
            value_wei: 0,
            data: scope.calldata().unwrap().into(),
            function: "expireClaim()".into(),
        };
        let mut delay = delay_marker.then(|| MarkerDelay::install(id));
        let budget = TimeoutLeaseBudget::new(
            tokio::time::Instant::now(),
            16,
            state.x402_relayer.rpc_timeout_seconds,
        )
        .unwrap();
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 16)
            .await
            .unwrap()
            .unwrap();
        let started = tokio::time::Instant::now();
        let result = first_send(
            &state,
            &rpc,
            &scope,
            &intent,
            &relayer,
            (lease, budget),
            None,
        )
        .await
        .unwrap();
        assert!(result.send_attempted);
        assert!(!result.marker_commit_unknown);
        if delay_marker {
            assert!(started.elapsed() >= Duration::from_secs(2));
        }
        assert_eq!(
            estimates.load(Ordering::SeqCst),
            1,
            "actual fresh preparation ran"
        );
        assert_eq!(gas_budget.reservations.load(Ordering::SeqCst), 1);
        let expected_sends = usize::from(!delay_marker);
        assert_eq!(sends.load(Ordering::SeqCst), expected_sends);
        let stored = store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .unwrap();
        assert!(stored.send_attempted);
        assert_eq!(
            stored.prepared.signed_transaction,
            result.prepared.signed_transaction
        );
        assert_eq!(
            stored.prepared.transaction.tx_hash,
            result.prepared.transaction.tx_hash
        );
        assert_eq!(stored.prepared.nonce, 9);
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
        // A new lease/request still cannot send after the marker is committed,
        // including when this process deliberately sent zero wire requests.
        let retry_budget = TimeoutLeaseBudget::new(
            tokio::time::Instant::now(),
            16,
            state.x402_relayer.rpc_timeout_seconds,
        )
        .unwrap();
        let retry_lease = store
            .acquire_x402_relayer_lease(&scope.network, 16)
            .await
            .unwrap()
            .unwrap();
        let retried = first_send(
            &state,
            &rpc,
            &scope,
            &intent,
            &relayer,
            (retry_lease, retry_budget),
            None,
        )
        .await
        .unwrap();
        assert!(retried.send_attempted);
        assert_eq!(
            retried.prepared.signed_transaction,
            stored.prepared.signed_transaction
        );
        assert_eq!(sends.load(Ordering::SeqCst), expected_sends);
        assert_eq!(gas_budget.reservations.load(Ordering::SeqCst), 1);
        assert_eq!(estimates.load(Ordering::SeqCst), 1);
        store
            .release_x402_relayer_lease(&scope.network, retry_lease)
            .await
            .unwrap();
        if let Some(delay) = delay.as_mut() {
            assert!(delay.remove());
        }
        server.abort();
    }

    #[test]
    fn timeout_lease_budget_supports_every_minimum_accepted_rpc_setting() {
        let start = tokio::time::Instant::now();
        for rpc in 1..=30 {
            let lease = 15.max(rpc + 1);
            let budget = TimeoutLeaseBudget::new(start, lease, rpc).unwrap();
            let mut now = start;
            for remaining in (1..=4).rev() {
                let stage = budget.stage_at(now, remaining).unwrap();
                assert!((1..=rpc).contains(&stage.seconds));
                assert!(stage.deadline + Duration::from_secs(2) < budget.deadline);
                // Each stage uses almost its full share; the next stage must
                // still fit even after dispatch/rounding time has elapsed.
                now = stage.deadline + Duration::from_millis(1);
            }
        }
        let short = TimeoutLeaseBudget::new(start, 16, 15).unwrap();
        assert_eq!(short.stage_at(start, 4).unwrap().seconds, 3);
    }

    #[test]
    fn timeout_lease_budget_counts_cumulative_slow_stages_and_database_waits() {
        let start = tokio::time::Instant::now();
        let budget = TimeoutLeaseBudget::new(start, 45, 15).unwrap();
        for (elapsed, remaining, expected) in [(0, 4, 10), (9, 3, 11), (18, 2, 12), (27, 1, 15)] {
            let stage = budget
                .stage_at(start + Duration::from_secs(elapsed), remaining)
                .unwrap();
            assert_eq!(stage.seconds, expected);
            assert!(stage.deadline + Duration::from_secs(2) < budget.deadline);
        }
        // The acquisition/read/save waits do not create a new lease deadline.
        let after_wait = budget.stage_at(start + Duration::from_secs(35), 2).unwrap();
        assert_eq!(after_wait.seconds, 3);
        let send = budget.stage_at(start + Duration::from_secs(40), 1).unwrap();
        assert_eq!(send.seconds, 2);
        assert!(send.may_start_at(start + Duration::from_secs(41)));
        assert!(!send.may_start_at(start + Duration::from_secs(42)));
        assert!(!send.may_start_at(start + Duration::from_secs(46)));
    }

    #[test]
    fn timeout_lease_budget_expires_without_rounding_up_or_resetting() {
        let start = tokio::time::Instant::now();
        let budget = TimeoutLeaseBudget::new(start, 16, 15).unwrap();
        assert_eq!(
            budget
                .stage_at(start + Duration::from_secs(12), 1)
                .unwrap()
                .seconds,
            1
        );
        for elapsed in [
            Duration::from_millis(12001),
            Duration::from_secs(16),
            Duration::from_secs(20),
        ] {
            assert_eq!(
                budget.stage_at(start + elapsed, 1).unwrap_err(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        assert_eq!(
            budget.stage_at(start, 0).unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            TimeoutLeaseBudget::new(start, 16, 0)
                .unwrap()
                .stage_at(start, 4)
                .unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn timeout_requires_explicit_round_and_never_exposes_saved_bytes() {
        let app = Router::new()
            .route("/timeout", post(relay_autonomous_timeout))
            .with_state(crate::tests::test_state(BountyNetwork::default()));
        let response=app.oneshot(Request::builder().method("POST").uri("/timeout").header("content-type","application/json").body(Body::from(r#"{"bounty_contract":"0x1111111111111111111111111111111111111111","action":"expire_claim"}"#)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let saved = test_saved();
        let response = known_response(
            AutonomousTimeoutAction::ExpireClaim,
            &saved,
            Err(StatusCode::SERVICE_UNAVAILABLE),
        );
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(
            value["transaction_hash"],
            saved.prepared.transaction.tx_hash
        );
        assert_eq!(value["round"], 1);
        assert_eq!(value["status"], "reconciliation_required");
        assert_eq!(value["confirmed"], false);
        assert!(!value.to_string().contains("signed_transaction"));
        assert!(!value
            .to_string()
            .contains(&saved.prepared.signed_transaction));
        let mut uncertain = saved.clone();
        uncertain.send_attempted = false;
        uncertain.marker_commit_unknown = true;
        let response = known_response(
            AutonomousTimeoutAction::ExpireClaim,
            &uncertain,
            Err(StatusCode::SERVICE_UNAVAILABLE),
        );
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert!(value["send_attempted"].is_null());
        assert_eq!(value["status"], "reconciliation_required");
    }
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn timeout_journal_lost_send_response_retries_and_later_round_are_reconciliation_only() {
        run_journal_lease_case(90, 1).await;
    }

    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn timeout_journal_short_accepted_lease_sends_once_and_reconciles() {
        run_journal_lease_case(16, 15).await;
    }

    async fn run_journal_lease_case(lease_seconds: u64, rpc_timeout_seconds: u64) {
        let url = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        assert!(url.contains("@127.0.0.1:"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let mut saved = test_saved();
        saved.send_attempted = false;
        saved.scope.bounty_contract = format!("0x{:040x}", Uuid::new_v4().as_u128());
        saved.scope.bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
        saved.scope.terms_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
        saved.scope.factory_contract = configured_autonomous_planner("base-mainnet")
            .unwrap()
            .factory_contract;
        let signer = PrivateKeySigner::random();
        let key = format!("0x{}", hex::encode(signer.to_bytes()));
        let relayer = BaseTransactionRelayer::from_private_key(&key)
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
        let envelope = TransactionRequest::default()
            .with_to(saved.scope.bounty_contract.parse::<Address>().unwrap())
            .with_value(U256::ZERO)
            .with_input(saved.scope.calldata().unwrap().parse::<Bytes>().unwrap())
            .with_nonce(9)
            .with_chain_id(8453)
            .with_gas_limit(25000)
            .with_max_fee_per_gas(3)
            .with_max_priority_fee_per_gas(1)
            .build(&EthereumWallet::from(signer))
            .await
            .unwrap();
        saved.prepared.signed_transaction = format!("0x{}", hex::encode(envelope.encoded_2718()));
        saved.prepared.transaction.tx_hash = format!("{:#x}", envelope.tx_hash());
        saved.prepared.transaction.relayer = relayer.address();
        let sends = Arc::new(AtomicUsize::new(0));
        let round = Arc::new(AtomicU64::new(2));
        let confirmed = Arc::new(AtomicBool::new(false));
        let now = Utc::now().timestamp() as u64;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = format!("http://{}", listener.local_addr().unwrap());
        let fixture = saved.clone();
        let send_counter = sends.clone();
        let observed_round = round.clone();
        let receipt_available = confirmed.clone();
        let server = tokio::spawn(async move {
            let app=Router::new().route("/",post(move |Json(request):Json<Value>| {
            let fixture=fixture.clone();let send_counter=send_counter.clone();let observed_round=observed_round.clone();let receipt_available=receipt_available.clone();
            async move {
                let scope=&fixture.scope;
                let result=match request["method"].as_str().unwrap() {
                    "eth_chainId"=>json!("0x2105"),"eth_blockNumber"=>json!("0x64"),"eth_getBlockByNumber"=>json!({"number":"0x64","hash":word(100),"timestamp":format!("0x{now:x}")}),"eth_getTransactionCount"=>json!("0x9"),
                    "eth_call"=> {let data=request["params"][0]["data"].as_str().unwrap();let out=if data==selector("status()") {word(2)} else if data==selector("round()") {word(observed_round.load(Ordering::SeqCst))} else if data==selector("factory()") {format!("0x{:0>64}",scope.factory_contract.trim_start_matches("0x"))} else if data==selector("bountyId()") {scope.bounty_id.clone()} else if data==selector("termsHash()") {scope.terms_hash.clone()} else if data.starts_with(&selector("isCanonicalBounty(address)")) {word(1)} else if data==selector("claimExpiresAt()")||data==selector("verificationExpiresAt()") {word(now-1)} else {panic!("unexpected local call")};json!(out)},
                    "eth_sendRawTransaction"=> {assert_eq!(request["params"][0],fixture.prepared.signed_transaction);send_counter.fetch_add(1,Ordering::SeqCst);tokio::time::sleep(Duration::from_secs(rpc_timeout_seconds + 2)).await;json!(fixture.prepared.transaction.tx_hash)},
                    "eth_getTransactionReceipt"=>if !receipt_available.load(Ordering::SeqCst) {Value::Null} else {json!({"transactionHash":fixture.prepared.transaction.tx_hash,"blockNumber":"0x64","blockHash":word(100),"status":"0x1","logs":[{"address":scope.bounty_contract,"topics":[format!("{:#x}",alloy::primitives::keccak256("ClaimExpired(bytes32,uint64,address,uint256,uint256)".as_bytes())),scope.bounty_id,word(scope.round),word(99)],"data":format!("{}{}",word(1),&word(2)[2..]),"transactionHash":fixture.prepared.transaction.tx_hash,"blockNumber":"0x64","logIndex":"0x0"}]} )},
                    method=>panic!("unexpected local RPC {method}"),
                };Json(json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            }
        }));
            axum::serve(listener, app).await.unwrap();
        });
        let mut state = crate::tests::test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "unused",
            store.clone(),
        );
        let mutable = Arc::make_mut(&mut state);
        mutable.base_rpc_urls.base_mainnet = Some(rpc.clone());
        mutable.x402_relayer.rpc_timeout_seconds = rpc_timeout_seconds;
        mutable.x402_relayer.lease_seconds = lease_seconds;
        mutable.x402_relayer.confirmations = 1;
        mutable.x402_relayer.max_gas = 25000;
        mutable.x402_relayer.max_fee_per_gas_wei = 3;
        mutable.x402_relayer.enabled = true;
        mutable.x402_relayer.relayer = Some(Arc::new(relayer.clone()));
        let scope = &saved.scope;
        let id = scope.operation_id().unwrap();
        let intent = EvmTransactionIntent {
            from: Some(relayer.address()),
            to: scope.bounty_contract.clone(),
            value_wei: 0,
            data: scope.calldata().unwrap().into(),
            function: "expireClaim()".into(),
        };
        let budget = TimeoutLeaseBudget::new(
            tokio::time::Instant::now(),
            lease_seconds,
            rpc_timeout_seconds,
        )
        .unwrap();
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, lease_seconds)
            .await
            .unwrap()
            .unwrap();
        // Wrong expected round fails before preparing or saving any transaction.
        assert!(first_send(
            &state,
            &rpc,
            scope,
            &intent,
            &relayer,
            (lease, budget),
            None
        )
        .await
        .is_err());
        assert!(store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .is_none());
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        store
            .save_timeout_relay(
                scope,
                lease,
                &saved.prepared,
                store.timeout_budget_day().await.unwrap(),
            )
            .await
            .unwrap();
        assert!(first_send(
            &state,
            &rpc,
            scope,
            &intent,
            &relayer,
            (lease, budget),
            Some(saved.clone())
        )
        .await
        .is_err());
        assert!(
            !store
                .timeout_relay(id, &scope.network)
                .await
                .unwrap()
                .unwrap()
                .send_attempted
        );
        // An outbox row alone is not publication authority. Private/unlinked
        // records cannot disclose even their saved transaction hash.
        let app = Router::new()
            .route("/timeout", post(relay_autonomous_timeout))
            .with_state(state.clone());
        let body = json!({"round":scope.round,"network":scope.network,"bounty_contract":scope.bounty_contract,"action":scope.action});
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/timeout")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .is_empty());
        let mut publication = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: Uuid::new_v4().to_string(),
            tx_hash: word(71),
            block_number: 50,
            log_index: 0,
            contract_address: scope.factory_contract.clone(),
            bounty_id: scope.bounty_id.clone(),
            kind: AutonomousBountyEventKind::CanonicalBountyCreated,
            data: json!({"bounty_contract":scope.bounty_contract,"terms_hash":scope.terms_hash,
                "creator":relayer.address(),"policy_hash":word(0)}),
            occurred_at: Utc::now(),
        };
        store
            .upsert_autonomous_bounty_event(&scope.network, &publication)
            .await
            .unwrap();
        // The shared disposable database is also read by broad-feed fixtures.
        // Preserve the full canonical event shape without publishing valid
        // terms or turning this recovery-only fixture into earning inventory.
        for (index, (kind, data)) in [
            (
                AutonomousBountyEventKind::CanonicalBountyTermsCommitted,
                json!({"acceptance_criteria_hash":word(0),"benchmark_hash":word(0),"evidence_schema_hash":word(0)}),
            ),
            (
                AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured,
                json!({"solver_reward":20_000_000,"verifier_reward":1_000_000,"claim_bond":1_000_000,
                    "target_amount":21_000_000,"initial_funding":0,"funding_deadline":4_000_000_000_u64,
                    "claim_window_seconds":86400,"verification_window_seconds":172800}),
            ),
            (
                AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
                json!({"verification_mode":1,"verifier_module":format!("0x{:040x}",0),
                    "verifier_reward_recipient":relayer.address(),"threshold":1,"verifier_set_hash":word(0)}),
            ),
        ].into_iter().enumerate() {
            let mut event = publication.clone();
            event.id = Uuid::new_v4();
            event.log_key = Uuid::new_v4().to_string();
            event.log_index = index as u64 + 1;
            event.kind = kind;
            event.data = data;
            store.upsert_autonomous_bounty_event(&scope.network, &event).await.unwrap();
        }
        round.store(scope.round, Ordering::SeqCst);
        let expired_budget = TimeoutLeaseBudget::new(
            tokio::time::Instant::now() - Duration::from_secs(lease_seconds + 1),
            lease_seconds,
            rpc_timeout_seconds,
        )
        .unwrap();
        assert_eq!(
            first_send(
                &state,
                &rpc,
                scope,
                &intent,
                &relayer,
                (lease, expired_budget),
                Some(saved.clone()),
            )
            .await
            .err(),
            Some(StatusCode::SERVICE_UNAVAILABLE)
        );
        let unsent = store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .unwrap();
        assert!(!unsent.send_attempted);
        assert_eq!(
            unsent.prepared.signed_transaction,
            saved.prepared.signed_transaction
        );
        assert_eq!(sends.load(Ordering::SeqCst), 0);
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
        // A new lease may send the SAME never-attempted envelope. It cannot
        // discard the nonce fence, replace it, or replay an uncertain send.
        let budget = TimeoutLeaseBudget::new(
            tokio::time::Instant::now(),
            lease_seconds,
            rpc_timeout_seconds,
        )
        .unwrap();
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, lease_seconds)
            .await
            .unwrap()
            .unwrap();
        let (a, b) = tokio::join!(
            first_send(
                &state,
                &rpc,
                scope,
                &intent,
                &relayer,
                (lease, budget),
                Some(saved.clone())
            ),
            first_send(
                &state,
                &rpc,
                scope,
                &intent,
                &relayer,
                (lease, budget),
                Some(saved.clone())
            )
        );
        assert!(a.unwrap().send_attempted && b.unwrap().send_attempted);
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
        // Simulate a later round and even loss of the signing key. Reads must
        // recover the original hash; neither circumstance permits another send.
        round.store(scope.round + 1, Ordering::SeqCst);
        Arc::make_mut(&mut state).x402_relayer.relayer = None;
        for is_confirmed in [false, true] {
            confirmed.store(is_confirmed, Ordering::SeqCst);
            let app = Router::new()
                .route("/timeout", post(relay_autonomous_timeout))
                .with_state(state.clone());
            let body = json!({"round":scope.round,"network":scope.network,"bounty_contract":scope.bounty_contract,"action":scope.action});
            let response = app
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/timeout")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if is_confirmed {
                    StatusCode::OK
                } else {
                    StatusCode::ACCEPTED
                }
            );
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap())
                    .unwrap();
            assert_eq!(
                value["transaction_hash"],
                saved.prepared.transaction.tx_hash
            );
            assert_eq!(value["confirmed"], is_confirmed);
            assert_eq!(value["round"], scope.round);
            assert!(!value.to_string().contains("signed_transaction"));
            assert_eq!(sends.load(Ordering::SeqCst), 1);
        }
        // Known public identity remains reconcilable when RPC configuration is
        // missing, but a changed publication binding fails closed immediately.
        Arc::make_mut(&mut state).base_rpc_urls.base_mainnet = None;
        for changed in [false, true] {
            if changed {
                publication.data["terms_hash"] = json!(word(999));
                store
                    .upsert_autonomous_bounty_event(&scope.network, &publication)
                    .await
                    .unwrap();
            }
            let response = handle(
                &state,
                RelayAutonomousTimeoutRequest {
                    round: scope.round,
                    network: Some(scope.network.clone()),
                    bounty_contract: scope.bounty_contract.clone(),
                    action: AutonomousTimeoutAction::ExpireClaim,
                },
            )
            .await;
            if changed {
                assert_eq!(response.err(), Some(StatusCode::NOT_FOUND));
            } else {
                let response = response.unwrap();
                assert_eq!(response.status(), StatusCode::ACCEPTED);
                let value: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap())
                        .unwrap();
                assert_eq!(
                    value["transaction_hash"],
                    saved.prepared.transaction.tx_hash
                );
                assert_eq!(value["confirmed"], false);
            }
        }
        publication.data["terms_hash"] = json!(scope.terms_hash);
        store
            .upsert_autonomous_bounty_event(&scope.network, &publication)
            .await
            .unwrap();
        let fixture_events = store
            .list_autonomous_bounty_events_for_factory(
                &scope.network,
                &scope.factory_contract,
                false,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|event| event.bounty_id == scope.bounty_id);
        let fixture_feed =
            chain_base::build_autonomous_bounty_feed(fixture_events, [], false).unwrap();
        assert_eq!(fixture_feed.len(), 1);
        assert!(!fixture_feed[0].terms_valid);
        assert_eq!(fixture_feed[0].funded_amount, "0");
        assert_eq!(sends.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
