use super::*;
#[test]
fn disabled_and_non_allowlisted_operations_fail_closed() {
    let id = Uuid::new_v4();
    let allowed = id.to_string();
    assert!(gate(None, Some(&allowed), id).is_err());
    assert!(gate(Some("1"), Some(&allowed), id).is_err());
    assert!(gate(Some("true"), None, id).is_err());
    assert!(gate(Some("true"), Some(&allowed), Uuid::new_v4()).is_err());
    assert!(gate(Some("true"), Some(&allowed), id).is_ok());
}
#[test]
fn reviewed_pins_cannot_be_replaced_by_configuration() {
    let limits = X402HostedRelayerConfig::default();
    assert!(reviewed_policy(
        "base-mainnet",
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9",
        &limits
    )
    .is_ok());
    assert!(reviewed_policy(
        "base-mainnet",
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "0x0000000000000000000000000000000000000000",
        &limits
    )
    .is_err());
    assert!(reviewed_policy(
        "base-sepolia",
        "0x9601a40b35ad6843846732c6cb73c4c82f9ba850",
        "0xe70b9d541a176307e50f308aa370a1661eabfd99",
        &limits
    )
    .is_err());
}

use axum::{body::Body, http::Request as HttpRequest};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

#[tokio::test]
async fn missing_and_wrong_operator_are_rejected_before_configuration_or_store() {
    let saved = timeout_relay::test_saved();
    let body = json!({"operation_id":saved.scope.operation_id().unwrap(),"scope":saved.scope,"expected_transaction_hash":saved.prepared.transaction.tx_hash});
    for configured in [None, Some("fixture-operator".to_string())] {
        let mut state = crate::tests::test_state(BountyNetwork::default());
        Arc::make_mut(&mut state).operator_api_token = configured;
        let app = Router::new()
            .route("/operator-revalidate", post(handle))
            .with_state(state);
        for bearer in [None, Some("Bearer wrong"), Some("Bearer wallet-session")] {
            let mut request = HttpRequest::builder()
                .method("POST")
                .uri("/operator-revalidate")
                .header("content-type", "application/json");
            if let Some(bearer) = bearer {
                request = request.header("authorization", bearer);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }
}

struct FakeIo {
    fail_check: usize,
    final_fee_increases: bool,
    lost_broadcast_response: bool,
    checks: AtomicUsize,
    quotes: AtomicUsize,
    broadcasts: AtomicUsize,
}
impl FakeIo {
    fn new(fail_check: usize, final_fee_increases: bool, lost_broadcast_response: bool) -> Self {
        Self {
            fail_check,
            final_fee_increases,
            lost_broadcast_response,
            checks: AtomicUsize::new(0),
            quotes: AtomicUsize::new(0),
            broadcasts: AtomicUsize::new(0),
        }
    }
}
impl RecoveryIo for FakeIo {
    async fn check(
        &self,
        _config: &Config,
        saved: &db::SavedTimeoutRelay,
        _seconds: u64,
    ) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
        let call = self.checks.fetch_add(1, Ordering::SeqCst) + 1;
        if call == self.fail_check {
            return Err(ChainBaseError::RelayerProvider(
                "fixture chain unavailable".into(),
            ));
        }
        let block = chain_base::BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "aa".repeat(32)),
            timestamp: Utc::now().timestamp() as u64,
        };
        Ok(TimeoutRevalidationObservation {
            observed_at: Utc::now(),
            common_safe_block: block.clone(),
            provider_origin_hashes: ["a".repeat(64), "b".repeat(64)],
            latest_blocks: [block.clone(), block],
            estimated_gas: [21_000, 21_000],
            maximum_fee_wei: 75_000 + u128::from(call == 2 && self.final_fee_increases),
            transaction_hash: saved.prepared.transaction.tx_hash.clone(),
            nonce: saved.prepared.nonce,
        })
    }
    async fn quote(&self, chain_id: u64, maximum_fee_wei: u128) -> Result<u64, ChainBaseError> {
        assert_eq!(chain_id, 8453);
        assert_eq!(maximum_fee_wei, 75_000);
        self.quotes.fetch_add(1, Ordering::SeqCst);
        Ok(100)
    }
    async fn broadcast(
        &self,
        _config: &Config,
        saved: &db::SavedTimeoutRelay,
        _seconds: u64,
    ) -> Result<(), ChainBaseError> {
        assert!(saved.send_attempted);
        assert!(!saved.marker_commit_unknown);
        self.broadcasts.fetch_add(1, Ordering::SeqCst);
        if self.lost_broadcast_response {
            Err(ChainBaseError::RelayerProvider(
                "fixture lost response".into(),
            ))
        } else {
            Ok(())
        }
    }
}

struct Fixture {
    store: PostgresStore,
    pool: sqlx::PgPool,
    state: SharedState,
    lease: Uuid,
    saved: db::SavedTimeoutRelay,
    config: Config,
}
async fn fixture() -> Fixture {
    use alloy::{
        eips::Encodable2718,
        network::{EthereumWallet, TransactionBuilder},
        primitives::{Address, Bytes, U256},
        rpc::types::TransactionRequest,
        signers::local::PrivateKeySigner,
    };
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("timeout_api_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let mut saved = timeout_relay::test_saved();
    saved.send_attempted = false;
    saved.scope.bounty_contract = format!("0x{:040x}", Uuid::new_v4().as_u128());
    saved.scope.bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let signer = PrivateKeySigner::random();
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("0x{}", hex::encode(signer.to_bytes())))
            .unwrap();
    let envelope = TransactionRequest::default()
        .with_to(saved.scope.bounty_contract.parse::<Address>().unwrap())
        .with_value(U256::ZERO)
        .with_input(saved.scope.calldata().unwrap().parse::<Bytes>().unwrap())
        .with_nonce(saved.prepared.nonce)
        .with_chain_id(8453)
        .with_gas_limit(25_000)
        .with_max_fee_per_gas(3)
        .with_max_priority_fee_per_gas(1)
        .build(&EthereumWallet::from(signer))
        .await
        .unwrap();
    saved.prepared.signed_transaction = format!("0x{}", hex::encode(envelope.encoded_2718()));
    saved.prepared.transaction.tx_hash = format!("{:#x}", envelope.tx_hash());
    saved.prepared.transaction.relayer = relayer.address();
    let lease = store
        .acquire_x402_relayer_lease(&saved.scope.network, 180)
        .await
        .unwrap()
        .unwrap();
    store
        .save_timeout_relay(
            &saved.scope,
            lease,
            &saved.prepared,
            store.timeout_budget_day().await.unwrap(),
        )
        .await
        .unwrap();
    store.upsert_autonomous_bounty_event(&saved.scope.network,&AutonomousBountyEvent {id:Uuid::new_v4(),log_key:Uuid::new_v4().to_string(),tx_hash:format!("0x{}","bb".repeat(32)),block_number:1,log_index:0,contract_address:saved.scope.factory_contract.clone(),bounty_id:saved.scope.bounty_id.clone(),kind:AutonomousBountyEventKind::CanonicalBountyCreated,data:json!({"bounty_contract":saved.scope.bounty_contract,"terms_hash":saved.scope.terms_hash}),occurred_at:Utc::now()}).await.unwrap();
    sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp()-interval '1 day' WHERE operation_id=$1").bind(saved.scope.operation_id().unwrap()).execute(&pool).await.unwrap();
    let state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "fixture-operator",
        store.clone(),
    );
    let config = Config {
        primary_rpc: "https://rpc-primary.invalid".into(),
        shadow_rpc: "https://rpc-shadow.invalid".into(),
        policy: TimeoutRevalidationPolicy {
            factory_runtime_code_hash: format!("0x{}", "11".repeat(32)),
            implementation_contract: format!("0x{}", "22".repeat(20)),
            implementation_runtime_code_hash: format!("0x{}", "33".repeat(32)),
            max_gas: 25_000,
            max_fee_per_gas_wei: 3,
        },
    };
    Fixture {
        store,
        pool,
        state,
        lease,
        saved,
        config,
    }
}
fn lease_budget() -> TimeoutLeaseBudget {
    TimeoutLeaseBudget::new(tokio::time::Instant::now(), 90, 15).unwrap()
}
async fn send(
    f: &Fixture,
    saved: &mut db::SavedTimeoutRelay,
    io: &FakeIo,
) -> Result<Uuid, StatusCode> {
    first_send(&f.state, &f.config, f.lease, lease_budget(), saved, io).await
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn timeout_revalidation_api_persists_exact_one_send_across_failures_and_retries() {
    let f = fixture().await;
    let op = f.saved.scope.operation_id().unwrap();
    let mut saved = f.saved.clone();
    let before:(DateTime<Utc>,String,serde_json::Value)=sqlx::query_as("SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1").bind(op).fetch_one(&f.pool).await.unwrap();
    let first_fail = FakeIo::new(1, false, false);
    assert!(send(&f, &mut saved, &first_fail).await.is_err());
    assert_eq!(first_fail.quotes.load(Ordering::SeqCst), 0);
    assert_eq!(first_fail.broadcasts.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        0
    );
    let final_fail = FakeIo::new(2, false, false);
    assert!(send(&f, &mut saved, &final_fail).await.is_err());
    assert_eq!(final_fail.broadcasts.load(Ordering::SeqCst), 0);
    assert!(!saved.send_attempted);
    assert_eq!(
        f.store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        100
    );
    let increased_fee = FakeIo::new(0, true, false);
    assert!(send(&f, &mut saved, &increased_fee).await.is_err());
    assert_eq!(increased_fee.broadcasts.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        100,
        "same-day revalidation cannot consume another full allowance"
    );
    assert!(
        !f.store
            .claim_timeout_relay_send(&saved.scope, f.lease, 1)
            .await
            .unwrap(),
        "operator authorization never opens ordinary stale sends"
    );
    let lost_response = FakeIo::new(0, false, true);
    let authorization = send(&f, &mut saved, &lost_response).await.unwrap();
    assert!(saved.send_attempted);
    assert_eq!(lost_response.broadcasts.load(Ordering::SeqCst), 1);
    let record = f
        .store
        .timeout_relay(op, &saved.scope.network)
        .await
        .unwrap()
        .unwrap();
    assert!(record.send_attempted);
    let event:(String,serde_json::Value,i64)=sqlx::query_as("SELECT status,observations,required_budget_micro_usdc FROM timeout_revalidation_events WHERE authorization_id=$1").bind(authorization).fetch_one(&f.pool).await.unwrap();
    assert_eq!(event.0, "consumed");
    assert_eq!(
        event.1["transaction_hash"],
        saved.prepared.transaction.tx_hash
    );
    assert_eq!(event.2, 100);
    let retry = FakeIo::new(0, false, false);
    let mut stale = f.saved.clone();
    assert!(send(&f, &mut stale, &retry).await.is_err());
    assert!(stale.send_attempted);
    assert_eq!(retry.checks.load(Ordering::SeqCst), 0);
    assert_eq!(retry.broadcasts.load(Ordering::SeqCst), 0);
    let after:(DateTime<Utc>,String,serde_json::Value)=sqlx::query_as("SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1").bind(op).fetch_one(&f.pool).await.unwrap();
    assert_eq!(after, before);
    f.store
        .release_x402_relayer_lease(&saved.scope.network, f.lease)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn timeout_revalidation_api_expired_lease_never_marks_or_broadcasts() {
    let f = fixture().await;
    let mut saved = f.saved.clone();
    sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE network=$1").bind(&saved.scope.network).execute(&f.pool).await.unwrap();
    let io = FakeIo::new(0, false, false);
    assert!(send(&f, &mut saved, &io).await.is_err());
    assert_eq!(io.broadcasts.load(Ordering::SeqCst), 0);
    assert!(
        !f.store
            .timeout_relay(saved.scope.operation_id().unwrap(), &saved.scope.network)
            .await
            .unwrap()
            .unwrap()
            .send_attempted
    );
    assert_eq!(
        f.store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        0
    );
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn timeout_revalidation_api_uncertain_marker_timeout_never_dispatches() {
    let f = fixture().await;
    let mut saved = f.saved.clone();
    // A test-only trigger in this fixture's isolated schema delays the real
    // marker UPDATE beyond the API stage. No mock replaces the production CAS.
    sqlx::raw_sql("CREATE FUNCTION delay_timeout_marker() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE TRIGGER delay_timeout_marker AFTER UPDATE ON relay_transaction_outbox FOR EACH ROW WHEN (OLD.send_attempts=0 AND NEW.send_attempts=1) EXECUTE FUNCTION delay_timeout_marker();")
        .execute(&f.pool).await.unwrap();
    let io = FakeIo::new(0, false, false);
    let budget = TimeoutLeaseBudget::new(tokio::time::Instant::now(), 90, 1).unwrap();
    assert!(
        first_send(&f.state, &f.config, f.lease, budget, &mut saved, &io)
            .await
            .is_err()
    );
    assert!(saved.marker_commit_unknown);
    assert_eq!(io.broadcasts.load(Ordering::SeqCst), 0);
    // Do not infer a committed marker from a timeout, or infer a broadcast from
    // a marker. The current request never dispatches in either database outcome.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let current = f
        .store
        .timeout_relay(saved.scope.operation_id().unwrap(), &saved.scope.network)
        .await
        .unwrap()
        .unwrap();
    if current.send_attempted {
        let retry = FakeIo::new(0, false, false);
        let mut stale = f.saved.clone();
        assert!(send(&f, &mut stale, &retry).await.is_err());
        assert_eq!(retry.broadcasts.load(Ordering::SeqCst), 0);
    }
    assert_eq!(io.broadcasts.load(Ordering::SeqCst), 0);
    f.store
        .release_x402_relayer_lease(&saved.scope.network, f.lease)
        .await
        .unwrap();
}

#[tokio::test]
async fn transport_unavailability_keeps_private_failure_class_and_prepared_reconciliation_contract()
{
    use chain_base::JsonRpcTransport;
    // A disposable loopback peer closes connections without an HTTP response,
    // without an external provider, stored credential, signer or transaction.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc = format!("http://{}", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let transport =
        chain_base::ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(1)).unwrap();
    let error = transport
        .post_json_value(
            &rpc,
            &json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ChainBaseError::RpcTransport(_)));
    assert!(!error.to_string().contains(&rpc));
    let event = service_runtime::http_observability::classified_failure_event(
        FailureStage::TimeoutRelayExecutionFailed,
        chain_failure_class(&error),
        "local",
    );
    assert_eq!(event["failure_class"], "transport");
    assert!(!event.to_string().contains(&rpc));
    assert_eq!(
        timeout_execution_result::<()>(Ok(Err(error))),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );
    // A genuine policy error still has its existing 422 mapping.
    assert_eq!(
        timeout_execution_result::<()>(Ok(Err(ChainBaseError::InvalidRelayIntent(
            "fixture policy mismatch".into()
        )))),
        Err(StatusCode::UNPROCESSABLE_ENTITY)
    );

    let state = crate::tests::test_state(BountyNetwork::default());
    let mut saved = timeout_relay::test_saved();
    saved.send_attempted = false;
    let response = timeout_relay::reconcile_with_observation(
        &state,
        &rpc,
        AutonomousTimeoutAction::ExpireClaim,
        &saved,
        true,
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, private"
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 16384)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["status"], "prepared");
    assert_eq!(body["send_attempted"], false);
    assert_eq!(body["confirmed"], false);
    assert_eq!(body["transaction_hash"], saved.prepared.transaction.tx_hash);
    assert!(
        body.get("blocked_reason").is_none(),
        "do not invent a new response field for private diagnostics"
    );
    assert!(!body.to_string().contains(&rpc));
    assert!(!saved.send_attempted);
    peer.abort();
}
