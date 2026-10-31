use super::*;
use axum::{body::Body, http::Request as HttpRequest};
use chain_base::{timeout_recovery::TimeoutRevalidationPolicy, BaseBlockIdentity};
use serde_json::json;
use sha2::Digest;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;
pub(super) fn observation(p: &NonceRetirementPlan) -> NonceRetirementObservation {
    NonceRetirementObservation {
        observed_at: Utc::now(),
        safe_block: BaseBlockIdentity {
            number: 110,
            hash: format!("0x{}", "11".repeat(32)),
            timestamp: Utc::now().timestamp() as u64,
        },
        finalized_block: BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "22".repeat(32)),
            timestamp: Utc::now().timestamp() as u64,
        },
        provider_hashes: ["a".repeat(64), "b".repeat(64)],
        maximum_cost_wei: p.maximum_cost_wei,
    }
}
#[test]
fn closed_gates_and_strict_body() {
    let id = Uuid::new_v4();
    let s = id.to_string();
    assert!(gate(None, Some(&s), id, false, false).is_err());
    assert!(gate(Some("true"), Some(&s), id, true, false).is_err());
    assert!(gate(Some("true"), Some(&s), id, false, true).is_err());
    assert!(gate(Some("true"), Some(&s), Uuid::new_v4(), false, false).is_err());
    assert!(gate(Some("true"), Some(&s), id, false, false).is_ok());
    assert!(serde_json::from_value::<Request>(json!({"action":"execute","operation_id":id,"expected_transaction_hash":"x","plan_id":id,"approved_digest":"d","nonce":78})).is_err());
}
#[tokio::test]
async fn operator_required_before_lookup_or_rpc() {
    for token in [None, Some("fixture-operator".into())] {
        let mut state = crate::tests::test_state(BountyNetwork::default());
        Arc::make_mut(&mut state).operator_api_token = token;
        let app = Router::new()
            .route("/retire", post(handle))
            .with_state(state);
        for auth in ["Bearer wrong", "Bearer wallet-session"] {
            let body = json!({"action":"prepare","operation_id":Uuid::new_v4(),"expected_transaction_hash":"x"});
            let r = app
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .method("POST")
                        .uri("/retire")
                        .header("content-type", "application/json")
                        .header("authorization", auth)
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        }
    }
}
struct FakeIo {
    checks: AtomicUsize,
    sends: AtomicUsize,
    fail_check: usize,
    absent: bool,
    lost_reply: bool,
}
impl FakeIo {
    fn new(fail_check: usize, absent: bool, lost_reply: bool) -> Self {
        Self {
            checks: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            fail_check,
            absent,
            lost_reply,
        }
    }
}
fn fail() -> ChainBaseError {
    ChainBaseError::RelayerProvider("fixture unavailable".into())
}
impl RetirementIo for FakeIo {
    async fn check(
        &self,
        _: &Config,
        p: &NonceRetirementPlan,
        _: &db::SavedTimeoutRelay,
        _: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        if self.checks.fetch_add(1, Ordering::SeqCst) + 1 == self.fail_check {
            Err(fail())
        } else {
            Ok(observation(p))
        }
    }
    async fn quote(&self, _: u128) -> Result<u64, ChainBaseError> {
        Ok(100)
    }
    async fn absent(
        &self,
        _: &Config,
        _: &PreparedBaseRelayTransaction,
        _: u64,
    ) -> Result<(), ChainBaseError> {
        if self.absent {
            Ok(())
        } else {
            Err(fail())
        }
    }
    async fn send(
        &self,
        _: &Config,
        _: &PreparedBaseRelayTransaction,
        _: u64,
    ) -> Result<(), ChainBaseError> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.lost_reply {
            Err(fail())
        } else {
            Ok(())
        }
    }
}
pub(super) struct Fixture {
    pub(super) store: PostgresStore,
    pub(super) pool: sqlx::PgPool,
    pub(super) state: SharedState,
    pub(super) lease: Uuid,
    pub(super) saved: db::SavedTimeoutRelay,
    pub(super) config: Config,
    pub(super) plan: db::SavedNonceRetirementPlan,
}
pub(super) async fn fixture() -> Fixture {
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
    let schema = format!("retirement_api_{}", Uuid::new_v4().simple());
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
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "fixture-operator",
        store.clone(),
    );
    Arc::make_mut(&mut state).x402_relayer.relayer = Some(Arc::new(
        relayer.with_gas_budget(store.gas_sponsor_budget()),
    ));
    let now = Utc::now();
    let plan = NonceRetirementPlan {
        operation_id: saved.scope.operation_id().unwrap(),
        original_tx_hash: saved.prepared.transaction.tx_hash.clone(),
        original_envelope_sha256: hex::encode(sha2::Sha256::digest(
            hex::decode(&saved.prepared.signed_transaction[2..]).unwrap(),
        )),
        sender: saved.prepared.transaction.relayer.clone(),
        chain_id: 8453,
        nonce: saved.prepared.nonce,
        gas_limit: 25_200,
        max_fee_per_gas_wei: 4,
        max_priority_fee_per_gas_wei: 2,
        maximum_cost_wei: 200_000,
        created_at: now,
        expires_at: now + chrono::Duration::minutes(15),
    };
    let observation = observation(&plan);
    let plan = store
        .save_nonce_retirement_plan(&saved.scope, lease, &plan, &observation)
        .await
        .unwrap();
    let config = Config {
        primary: "https://rpc-primary.invalid".into(),
        shadow: "https://rpc-shadow.invalid".into(),
        policy: TimeoutRevalidationPolicy {
            factory_runtime_code_hash: format!("0x{}", "11".repeat(32)),
            implementation_contract: format!("0x{}", "22".repeat(20)),
            implementation_runtime_code_hash: format!("0x{}", "33".repeat(32)),
            max_gas: 30_000,
            max_fee_per_gas_wei: 10,
        },
    };
    Fixture {
        store,
        pool,
        state,
        lease,
        saved,
        config,
        plan,
    }
}
async fn execute(f: &Fixture, io: &impl RetirementIo) -> Result<(), StatusCode> {
    first_send(
        &f.state,
        &f.config,
        &f.saved,
        &f.plan,
        &f.plan.digest,
        f.lease,
        TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
        io,
    )
    .await
}
pub(super) async fn original(
    f: &Fixture,
) -> (String, serde_json::Value, chrono::DateTime<Utc>, i32) {
    sqlx::query_as("SELECT signed_transaction,transaction_metadata,created_at,send_attempts FROM relay_transaction_outbox WHERE operation_id=$1").bind(f.plan.plan.operation_id).fetch_one(&f.pool).await.unwrap()
}
#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retirement_faults_and_one_send() {
    for (failure, absent, lost, expected) in [
        (1, true, false, 0),
        (2, true, false, 0),
        (0, false, false, 0),
        (0, true, true, 1),
        (0, true, false, 1),
    ] {
        let f = fixture().await;
        let before = original(&f).await;
        f.store.migrate().await.unwrap();
        let io = FakeIo::new(failure, absent, lost);
        let _ = execute(&f, &io).await;
        assert_eq!(io.sends.load(Ordering::SeqCst), expected);
        if failure != 1 {
            let _ = execute(&f, &io).await;
            assert_eq!(
                io.sends.load(Ordering::SeqCst),
                expected,
                "repeat cannot resend"
            );
        }
        assert_eq!(original(&f).await, before);
        assert!(
            f.store
                .gas_sponsor_budget()
                .ensure_sender_nonce_available(8453, &f.plan.plan.sender, f.plan.plan.nonce + 1)
                .await
                .is_err(),
            "pending next nonce remains fenced"
        );
        assert!(f
            .store
            .acquire_saved_relayer_lease("base-mainnet", f.plan.plan.operation_id, 45)
            .await
            .unwrap()
            .is_none());
        let saved = f
            .store
            .nonce_retirement(f.plan.plan.operation_id)
            .await
            .unwrap();
        if failure != 1 {
            let saved = saved.unwrap();
            assert_eq!(saved.send_attempted, expected == 1);
            let body = axum::body::to_bytes(status(&saved).into_body(), 16384)
                .await
                .unwrap();
            assert!(!String::from_utf8_lossy(&body).contains(&saved.prepared.signed_transaction));
        }
    }
}
#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retirement_concurrency_budget_and_finality() {
    let f = fixture().await;
    let before = original(&f).await;
    let io = FakeIo::new(0, true, false);
    let (a, b) = tokio::join!(execute(&f, &io), execute(&f, &io));
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    let saved = f
        .store
        .nonce_retirement(f.plan.plan.operation_id)
        .await
        .unwrap()
        .unwrap();
    let mut proof = NonceRetirementConfirmation {
        observed_at: Utc::now(),
        transaction_hash: saved.prepared.transaction.tx_hash.clone(),
        included_block: observation(&f.plan.plan).finalized_block.clone(),
        finalized_block: observation(&f.plan.plan).finalized_block,
        finalized_nonce: f.plan.plan.nonce,
        provider_hashes: ["a".repeat(64), "b".repeat(64)],
    };
    assert!(f
        .store
        .confirm_nonce_retirement(&saved, &proof)
        .await
        .is_err());
    proof.finalized_nonce += 1;
    proof.transaction_hash = f.saved.prepared.transaction.tx_hash.clone();
    assert!(f
        .store
        .confirm_nonce_retirement(&saved, &proof)
        .await
        .is_err());
    proof.transaction_hash = saved.prepared.transaction.tx_hash.clone();
    f.store
        .confirm_nonce_retirement(&saved, &proof)
        .await
        .unwrap();
    f.store
        .confirm_nonce_retirement(&saved, &proof)
        .await
        .unwrap();
    assert!(f
        .store
        .gas_sponsor_budget()
        .ensure_sender_nonce_available(8453, &f.plan.plan.sender, f.plan.plan.nonce + 1)
        .await
        .is_ok());
    assert!(f
        .store
        .gas_sponsor_budget()
        .ensure_sender_nonce_available(8453, &f.plan.plan.sender, f.plan.plan.nonce)
        .await
        .is_err());
    assert_eq!(original(&f).await, before);
    // Even a direct legacy marker update cannot reactivate the obsolete intent.
    let updated =
        sqlx::query("UPDATE relay_transaction_outbox SET send_attempts=1 WHERE operation_id=$1")
            .bind(f.plan.plan.operation_id)
            .execute(&f.pool)
            .await
            .unwrap();
    assert_eq!(updated.rows_affected(), 0);
    // Normal outbox upserts remain usable after the fence closes.
    let mut ordinary = saved.prepared.clone();
    ordinary.nonce += 1;
    ordinary.transaction.tx_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let id = Uuid::new_v4();
    f.store
        .save_relay_outbox(id, "base-mainnet", &ordinary)
        .await
        .unwrap();
    f.store
        .save_relay_outbox(id, "base-mainnet", &ordinary)
        .await
        .unwrap();
    assert!(f.store.claim_relay_outbox_send(id).await.unwrap());
    let reserved: i64 = sqlx::query_scalar(
        "SELECT reserved_micro_usdc FROM gas_sponsorship_daily_budget WHERE budget_key='platform'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(reserved, 100);
    // Expired lease, exhausted budget, stale observation and wrong digest cannot sign.
    for fault in ["lease", "budget", "stale", "digest"] {
        let f = fixture().await;
        let mut o = observation(&f.plan.plan);
        let mut digest = f.plan.digest.clone();
        match fault {
            "lease" => {
                sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()-interval '1 second'").execute(&f.pool).await.unwrap();
            }
            "budget" => {
                sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES('platform',(clock_timestamp() AT TIME ZONE 'UTC')::date,1000000,1000000)").execute(&f.pool).await.unwrap();
            }
            "stale" => o.observed_at -= chrono::Duration::seconds(31),
            "digest" => digest = "wrong".into(),
            _ => unreachable!(),
        }
        assert!(f
            .store
            .approve_nonce_retirement(
                &f.saved.scope,
                &f.plan,
                &digest,
                f.lease,
                "fixture",
                &o,
                100
            )
            .await
            .is_err());
        assert!(f
            .store
            .nonce_retirement(f.plan.plan.operation_id)
            .await
            .unwrap()
            .is_none());
    }
}
#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retirement_lease_contention_and_delayed_commit() {
    let f = fixture().await;
    // Simulate recovery holding the lease while a legacy sender already owns its
    // outbox row. The trigger must fail closed without a lock-order deadlock.
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM x402_relayer_leases FOR UPDATE")
        .execute(&mut *tx)
        .await
        .unwrap();
    let update = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("UPDATE relay_transaction_outbox SET send_attempts=1 WHERE operation_id=$1")
            .bind(f.plan.plan.operation_id)
            .execute(&f.pool),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(update.rows_affected(), 0);
    tx.rollback().await.unwrap();
    // A committed marker whose response arrives too late never starts a send.
    sqlx::raw_sql("CREATE FUNCTION delay_retirement_send() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE TRIGGER delay_retirement_send AFTER INSERT ON nonce_retirement_send_events FOR EACH ROW EXECUTE FUNCTION delay_retirement_send();").execute(&f.pool).await.unwrap();
    let io = FakeIo::new(0, true, false);
    let result = first_send(
        &f.state,
        &f.config,
        &f.saved,
        &f.plan,
        &f.plan.digest,
        f.lease,
        TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 1).unwrap(),
        &io,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(io.sends.load(Ordering::SeqCst), 0);
    assert!(
        f.store
            .nonce_retirement(f.plan.plan.operation_id)
            .await
            .unwrap()
            .is_some(),
        "fixture must reach durable signing before the injected send-marker delay: {result:?}"
    );
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let _ = execute(&f, &io).await;
    assert_eq!(io.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retirement_migration_rollback_and_immutable_records() {
    let f = fixture().await;
    let changed = sqlx::query("UPDATE nonce_retirement_plans SET nonce=nonce+1 WHERE id=$1")
        .bind(f.plan.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(changed.rows_affected(), 0);
    let changed = sqlx::query("DELETE FROM nonce_retirement_plans WHERE id=$1")
        .bind(f.plan.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(changed.rows_affected(), 0);
    // Inject a DDL collision in this isolated schema. A failed whole migration
    // must rollback before the pooled session releases its advisory lock.
    sqlx::raw_sql("DROP VIEW nonce_retirement_fences; CREATE TABLE nonce_retirement_fences(fixture_collision INT);").execute(&f.pool).await.unwrap();
    assert!(f.store.migrate().await.is_err());
    use sqlx::Connection;
    // A distinct physical session is essential: advisory locks are reentrant.
    let mut other = sqlx::PgConnection::connect_with(f.pool.connect_options().as_ref())
        .await
        .unwrap();
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(4270265017)")
        .fetch_one(&mut other)
        .await
        .unwrap();
    assert!(got, "migration failure leaked session advisory lock");
    sqlx::query("SELECT pg_advisory_unlock(4270265017)")
        .execute(&mut other)
        .await
        .unwrap();
    drop(other);
    sqlx::query("DROP TABLE nonce_retirement_fences")
        .execute(&f.pool)
        .await
        .unwrap();
    f.store.migrate().await.unwrap();
}
#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retirement_previous_day_and_competing_journal_fail_closed() {
    let f = fixture().await;
    let o = observation(&f.plan.plan);
    sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES('platform',(clock_timestamp() AT TIME ZONE 'UTC')::date-1,100,1000000)").execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO nonce_retirement_approvals(plan_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at) VALUES($1,$2,$3,'fixture',$4,$5,(clock_timestamp() AT TIME ZONE 'UTC')::date-1,100,clock_timestamp(),clock_timestamp()+interval '60 seconds')").bind(f.plan.id).bind(f.plan.plan.operation_id).bind(&f.plan.digest).bind(f.lease).bind(json!({"fixture":"previous_day"})).execute(&f.pool).await.unwrap();
    let prepared = f
        .state
        .x402_relayer
        .relayer
        .as_ref()
        .unwrap()
        .sign_nonce_retirement(&f.plan.plan)
        .await
        .unwrap();
    assert!(
        f.store
            .save_nonce_retirement_envelope(&f.saved.scope, &f.plan, f.lease, &prepared)
            .await
            .is_err(),
        "prior-day budget cannot admit a signature"
    );
    // Simulate bytes saved before midnight; neither current balance nor an old
    // budget receipt permits another day's first send.
    sqlx::query("INSERT INTO nonce_retirement_envelopes(operation_id,plan_id,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,$4,$5)").bind(f.plan.plan.operation_id).bind(f.plan.id).bind(&prepared.transaction.tx_hash).bind(&prepared.signed_transaction).bind(serde_json::to_value(&prepared.transaction).unwrap()).execute(&f.pool).await.unwrap();
    let saved = f
        .store
        .nonce_retirement(f.plan.plan.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(f
        .store
        .consume_nonce_retirement_send(&f.saved.scope, &saved, f.lease, 1, &o)
        .await
        .is_err());
    let f = fixture().await;
    let key = Uuid::new_v4().to_string();
    f.store
        .reserve_sponsored_action("base-mainnet", &key, &key, u32::MAX, u32::MAX)
        .await
        .unwrap();
    let io = FakeIo::new(0, true, false);
    assert!(execute(&f, &io).await.is_err());
    assert_eq!(io.sends.load(Ordering::SeqCst), 0);
    assert!(f
        .store
        .nonce_retirement(f.plan.plan.operation_id)
        .await
        .unwrap()
        .is_none());
}
