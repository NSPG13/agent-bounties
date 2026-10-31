use super::super::tests::{fixture, observation, original, Fixture};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fake {
    fault: &'static str,
    sends: AtomicUsize,
    expected_bytes: String,
}

#[test]
fn retained_dispatch_evidence_deadline_is_a_service_failure_not_policy_rejection() {
    let error = ChainBaseError::RpcTransport("retained dispatch evidence deadline elapsed".into());
    let event = service_runtime::http_observability::classified_failure_event(
        FailureStage::TimeoutRelayExecutionFailed,
        chain_failure_class(&error),
        "local",
    );
    assert_eq!(event["failure_class"], "transport");
    assert!(!event.to_string().contains("retained dispatch evidence"));
    assert_eq!(
        timeout_execution_result::<()>(Ok(Err(error))),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );
    assert_eq!(
        timeout_execution_result::<()>(Ok(Err(ChainBaseError::InvalidRelayIntent(
            "fixture policy mismatch".into(),
        )))),
        Err(StatusCode::UNPROCESSABLE_ENTITY)
    );
}

impl DispatchIo for Fake {
    async fn quote(&self, _: u128) -> Result<u64, ChainBaseError> {
        Ok(125)
    }
    async fn check(
        &self,
        _: &Config,
        p: &RetainedNonceDispatch,
        s: &db::SavedNonceRetirement,
        _: &db::SavedTimeoutRelay,
        _: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        if self.fault == "429" {
            return Err(ChainBaseError::RelayerProvider("fixture HTTP 429".into()));
        }
        let mut o = observation(&s.plan.plan);
        o.provider_hashes = p.provider_hashes.clone();
        if self.fault == "provider" {
            o.provider_hashes.swap(0, 1);
        }
        if self.fault == "stale" {
            o.observed_at -= chrono::Duration::seconds(31);
        }
        if self.fault == "future" {
            o.observed_at += chrono::Duration::seconds(31);
        }
        Ok(o)
    }
    async fn send(
        &self,
        _: &Config,
        s: &PreparedBaseRelayTransaction,
        _: u64,
    ) -> Result<(), ChainBaseError> {
        assert_eq!(s.signed_transaction, self.expected_bytes);
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.fault == "lost_reply" {
            Err(ChainBaseError::RelayerProvider("fixture lost reply".into()))
        } else {
            Ok(())
        }
    }
}

// Only disposable isolated schemas: seed the historical post-save/no-marker
// boundary without waiting for real-world approval expiry or altering any row.
async fn retained_fixture(
    previous_day: bool,
) -> (
    Fixture,
    db::SavedNonceRetirement,
    db::SavedRetainedNonceDispatch,
) {
    let mut f = fixture().await;
    let replacement = f
        .state
        .x402_relayer
        .relayer
        .as_ref()
        .unwrap()
        .sign_nonce_retirement(&f.plan.plan)
        .await
        .unwrap();
    let day = f.store.timeout_budget_day().await.unwrap()
        - chrono::Duration::days(i64::from(previous_day));
    let at = Utc::now() - chrono::Duration::minutes(5);
    sqlx::query("INSERT INTO nonce_retirement_approvals(plan_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at) VALUES($1,$2,$3,'isolated-historical-fixture',$4,$5,$6,100,$7,$8)")
        .bind(f.plan.id).bind(f.plan.plan.operation_id).bind(&f.plan.digest).bind(f.lease)
        .bind(serde_json::to_value(observation(&f.plan.plan)).unwrap()).bind(day).bind(at)
        .bind(at+chrono::Duration::seconds(120)).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES('platform',$1,100,1000000)")
        .bind(day).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO nonce_retirement_envelopes(operation_id,plan_id,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,$4,$5)")
        .bind(f.plan.plan.operation_id).bind(f.plan.id).bind(&replacement.transaction.tx_hash)
        .bind(&replacement.signed_transaction).bind(serde_json::to_value(&replacement.transaction).unwrap())
        .execute(&f.pool).await.unwrap();
    // The candidate dispatch receives no signer, proving no runtime signing dependency.
    Arc::make_mut(&mut f.state).x402_relayer.relayer = None;
    let saved = f
        .store
        .nonce_retirement(f.plan.plan.operation_id)
        .await
        .unwrap()
        .unwrap();
    let proposal = RetainedNonceDispatch::propose(
        [&f.config.primary, &f.config.shadow],
        f.plan.id,
        &f.plan.plan,
        &saved.prepared,
        Utc::now(),
    )
    .unwrap();
    let mut o = observation(&f.plan.plan);
    o.provider_hashes = proposal.provider_hashes.clone();
    let proposal = f
        .store
        .save_retained_nonce_dispatch(&f.saved.scope, &saved, f.lease, &proposal, &o)
        .await
        .unwrap();
    (f, saved, proposal)
}

async fn dispatch(
    f: &Fixture,
    s: &db::SavedNonceRetirement,
    p: &db::SavedRetainedNonceDispatch,
    lease: Uuid,
    io: &Fake,
) -> Result<(), StatusCode> {
    initial_dispatch(
        &f.state,
        &f.config,
        &f.saved,
        s,
        p,
        &p.digest,
        lease,
        TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
        io,
    )
    .await
}
async fn counts(f: &Fixture) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM nonce_retirement_send_events),(SELECT count(*) FROM retained_nonce_dispatch_authorizations)")
        .fetch_one(&f.pool).await.unwrap()
}

#[test]
fn dispatch_is_separately_disabled_and_strict() {
    for v in [None, Some("false"), Some("1"), Some("TRUE")] {
        assert!(enabled(v).is_err());
    }
    assert!(enabled(Some("true")).is_ok());
    let body = serde_json::json!({"action":"execute_retained_dispatch","operation_id":Uuid::new_v4(),
        "expected_transaction_hash":"old","expected_replacement_hash":"saved","dispatch_id":Uuid::new_v4(),"approved_digest":"d"});
    assert!(serde_json::from_value::<Request>(body.clone()).is_ok());
    for field in [
        "private_key",
        "signed_transaction",
        "nonce",
        "max_fee_per_gas_wei",
        "rpc_url",
    ] {
        let mut bad = body.clone();
        bad[field] = serde_json::json!("override");
        assert!(serde_json::from_value::<Request>(bad).is_err());
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retained_dispatch_faults_preserve_bytes_and_latch() {
    for fault in ["429", "provider", "stale", "future", "lost_reply", ""] {
        let (f, s, p) = retained_fixture(false).await;
        let before = original(&f).await;
        let io = Fake {
            fault,
            sends: AtomicUsize::new(0),
            expected_bytes: s.prepared.signed_transaction.clone(),
        };
        let result = dispatch(&f, &s, &p, f.lease, &io).await;
        let sent = matches!(fault, "" | "lost_reply");
        assert_eq!(result.is_ok(), sent, "{fault}");
        assert_eq!(io.sends.load(Ordering::SeqCst), usize::from(sent));
        assert_eq!(counts(&f).await, (i64::from(sent), i64::from(sent)));
        if sent {
            assert!(dispatch(&f, &s, &p, f.lease, &io).await.is_err());
            assert_eq!(
                io.sends.load(Ordering::SeqCst),
                1,
                "stale caller cannot resend"
            );
        }
        let after = f
            .store
            .nonce_retirement(f.plan.plan.operation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.prepared.signed_transaction,
            s.prepared.signed_transaction
        );
        assert_eq!(after.send_attempted, sent);
        assert_eq!(original(&f).await, before);
        assert!(f
            .store
            .gas_sponsor_budget()
            .ensure_sender_nonce_available(8453, &f.plan.plan.sender, f.plan.plan.nonce + 1)
            .await
            .is_err());
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retained_dispatch_concurrency_and_day_reservations() {
    for previous_day in [false, true] {
        let (f, s, p) = retained_fixture(previous_day).await;
        let io = Fake {
            fault: "",
            sends: AtomicUsize::new(0),
            expected_bytes: s.prepared.signed_transaction.clone(),
        };
        let (a, b) = tokio::join!(
            dispatch(&f, &s, &p, f.lease, &io),
            dispatch(&f, &s, &p, f.lease, &io)
        );
        assert!(a.is_ok() ^ b.is_ok());
        assert_eq!(io.sends.load(Ordering::SeqCst), 1);
        assert_eq!(counts(&f).await, (1, 1));
        let current:i64=sqlx::query_scalar("SELECT reserved_micro_usdc FROM gas_sponsorship_daily_budget WHERE budget_key='platform' AND budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date")
            .fetch_one(&f.pool).await.unwrap();
        assert_eq!(current, 125, "same-day delta, new-day full reservation");
        if previous_day {
            let previous:i64=sqlx::query_scalar("SELECT reserved_micro_usdc FROM gas_sponsorship_daily_budget WHERE budget_key='platform' AND budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date-1")
                .fetch_one(&f.pool).await.unwrap();
            assert_eq!(previous, 100);
        }
        for table in [
            "retained_nonce_dispatch_proposals",
            "retained_nonce_dispatch_authorizations",
        ] {
            assert_eq!(
                sqlx::query(&format!("DELETE FROM {table}"))
                    .execute(&f.pool)
                    .await
                    .unwrap()
                    .rows_affected(),
                0
            );
            assert_eq!(
                sqlx::query(&format!(
                    "UPDATE {table} SET expires_at=expires_at+interval '1 minute'"
                ))
                .execute(&f.pool)
                .await
                .unwrap()
                .rows_affected(),
                0
            );
        }
        f.store.migrate().await.unwrap();
        assert_eq!(counts(&f).await, (1, 1));
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retained_dispatch_rejects_mismatched_authority_and_budget() {
    for fault in [
        "lease",
        "digest",
        "proposal",
        "bytes",
        "budget",
        "prior_marker",
        "expired_lease",
        "insufficient_lease",
        "stored_expired_proposal",
        "competing_journal",
    ] {
        let (f, mut s, mut p) = retained_fixture(false).await;
        let mut lease = f.lease;
        match fault {
            "lease" => lease = Uuid::new_v4(),
            "digest" => p.digest = format!("0x{}", "ab".repeat(32)),
            "proposal" => {
                p.proposal.created_at -= chrono::Duration::minutes(20);
                p.proposal.expires_at -= chrono::Duration::minutes(20);
                p.digest = p.proposal.digest().unwrap();
            }
            "bytes" => s.prepared.signed_transaction = f.saved.prepared.signed_transaction.clone(),
            "budget" => {
                sqlx::query("UPDATE gas_sponsorship_daily_budget SET reserved_micro_usdc=1000000")
                    .execute(&f.pool)
                    .await
                    .unwrap();
            }
            "prior_marker" => {
                sqlx::query("INSERT INTO nonce_retirement_send_events(operation_id,observation) VALUES($1,'{}')").bind(f.plan.plan.operation_id).execute(&f.pool).await.unwrap();
            }
            "expired_lease" => {
                sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()-interval '1 second'")
                    .execute(&f.pool).await.unwrap();
            }
            "insufficient_lease" => {
                sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()+interval '2 seconds'")
                    .execute(&f.pool).await.unwrap();
            }
            "stored_expired_proposal" => {
                // Seed an actual, immutable, expired proposal with an exact digest.
                // A tiny fixture lifetime avoids changing the database clock.
                p.id = Uuid::new_v4();
                p.proposal.created_at = Utc::now();
                p.proposal.expires_at = p.proposal.created_at + chrono::Duration::milliseconds(1);
                p.digest = p.proposal.digest().unwrap();
                sqlx::query("INSERT INTO retained_nonce_dispatch_proposals(id,operation_id,retirement_plan_id,tx_hash,digest,proposal,observation,created_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,'{}',$7,$8)")
                    .bind(p.id).bind(p.proposal.operation_id).bind(p.proposal.retirement_plan_id)
                    .bind(&p.proposal.transaction_hash).bind(&p.digest).bind(serde_json::to_value(&p.proposal).unwrap())
                    .bind(p.proposal.created_at).bind(p.proposal.expires_at).execute(&f.pool).await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
                let stored = f
                    .store
                    .retained_nonce_dispatch(p.id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(stored.digest, p.digest);
                assert_eq!(stored.proposal, p.proposal);
            }
            "competing_journal" => {
                let key = Uuid::new_v4().to_string();
                f.store
                    .reserve_sponsored_action("base-mainnet", &key, &key, u32::MAX, u32::MAX)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let io = Fake {
            fault: "",
            sends: AtomicUsize::new(0),
            expected_bytes: s.prepared.signed_transaction.clone(),
        };
        assert!(dispatch(&f, &s, &p, lease, &io).await.is_err(), "{fault}");
        assert_eq!(io.sends.load(Ordering::SeqCst), 0);
        assert_eq!(counts(&f).await, (i64::from(fault == "prior_marker"), 0));
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retained_dispatch_durable_failures_never_start_transport() {
    for fault in ["authorization_insert", "commit_reply_delay"] {
        let (f, s, p) = retained_fixture(false).await;
        let before = original(&f).await;
        let ddl = if fault == "authorization_insert" {
            "CREATE FUNCTION dispatch_fixture_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'isolated authorization failure'; END $$; CREATE TRIGGER dispatch_fixture_failure BEFORE INSERT ON retained_nonce_dispatch_authorizations FOR EACH ROW EXECUTE FUNCTION dispatch_fixture_failure();"
        } else {
            // Delay COMMIT after both inserts. Cancellation leaves an uncertain result;
            // neither a late commit nor rollback authorizes this caller to send.
            "CREATE FUNCTION dispatch_fixture_failure() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(2); RETURN NEW; END $$; CREATE CONSTRAINT TRIGGER dispatch_fixture_failure AFTER INSERT ON retained_nonce_dispatch_authorizations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION dispatch_fixture_failure();"
        };
        sqlx::raw_sql(ddl).execute(&f.pool).await.unwrap();
        let io = Fake {
            fault: "",
            sends: AtomicUsize::new(0),
            expected_bytes: s.prepared.signed_transaction.clone(),
        };
        let result = initial_dispatch(
            &f.state,
            &f.config,
            &f.saved,
            &s,
            &p,
            &p.digest,
            f.lease,
            TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 1).unwrap(),
            &io,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(io.sends.load(Ordering::SeqCst), 0);
        if fault == "commit_reply_delay" {
            tokio::time::sleep(Duration::from_millis(2200)).await;
        }
        let (markers, authorizations) = counts(&f).await;
        assert_eq!(
            markers, authorizations,
            "latch and authorization are atomic"
        );
        assert!(markers == 0 || markers == 1);
        if fault == "authorization_insert" {
            assert_eq!(markers, 0);
        }
        let reserved: i64 = sqlx::query_scalar("SELECT reserved_micro_usdc FROM gas_sponsorship_daily_budget WHERE budget_key='platform'")
            .fetch_one(&f.pool).await.unwrap();
        assert_eq!(reserved, if markers == 0 { 100 } else { 125 });
        if markers == 1 {
            assert!(dispatch(&f, &s, &p, f.lease, &io).await.is_err());
        }
        assert_eq!(io.sends.load(Ordering::SeqCst), 0);
        assert_eq!(original(&f).await, before);
    }
}

#[tokio::test]
#[ignore = "requires explicitly isolated disposable PostgreSQL"]
async fn retained_dispatch_identity_constraints_and_migration_rollback() {
    let (f, s, p) = retained_fixture(false).await;
    // A proposal cannot be reassociated with another plan/envelope at the SQL boundary.
    let mismatch = sqlx::query("INSERT INTO retained_nonce_dispatch_proposals(id,operation_id,retirement_plan_id,tx_hash,digest,proposal,observation,created_at,expires_at) SELECT $1,operation_id,$2,tx_hash,$3,proposal,observation,created_at,expires_at FROM retained_nonce_dispatch_proposals WHERE id=$4")
        .bind(Uuid::new_v4()).bind(Uuid::new_v4()).bind(format!("0x{}", "ad".repeat(32))).bind(p.id)
        .execute(&f.pool).await;
    assert_eq!(
        mismatch
            .unwrap_err()
            .as_database_error()
            .unwrap()
            .code()
            .as_deref(),
        Some("23503")
    );
    // Exact proposal ID and operation still cannot authorize a different digest.
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO nonce_retirement_send_events(operation_id,observation) VALUES($1,'{}')",
    )
    .bind(p.proposal.operation_id)
    .execute(&mut *tx)
    .await
    .unwrap();
    let mismatch = sqlx::query("INSERT INTO retained_nonce_dispatch_authorizations(proposal_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at) VALUES($1,$2,$3,'fixture',$4,'{}',CURRENT_DATE,100,clock_timestamp(),clock_timestamp()+interval '60 seconds')")
        .bind(p.id).bind(p.proposal.operation_id).bind(format!("0x{}", "ef".repeat(32)))
        .bind(f.lease).execute(&mut *tx).await;
    assert_eq!(
        mismatch
            .unwrap_err()
            .as_database_error()
            .unwrap()
            .code()
            .as_deref(),
        Some("23503")
    );
    tx.rollback().await.unwrap();
    let io = Fake {
        fault: "",
        sends: AtomicUsize::new(0),
        expected_bytes: s.prepared.signed_transaction.clone(),
    };
    dispatch(&f, &s, &p, f.lease, &io).await.unwrap();
    // DDL collision rolls back the complete migration and releases its session lock.
    sqlx::raw_sql("DROP TRIGGER retained_nonce_dispatch_proposals_immutable ON retained_nonce_dispatch_proposals; CREATE VIEW retained_nonce_dispatch_proposals_immutable AS SELECT 1 AS fixture; DROP TABLE retained_nonce_dispatch_authorizations; CREATE VIEW retained_nonce_dispatch_authorizations AS SELECT 1 AS fixture;")
        .execute(&f.pool).await.unwrap();
    assert!(f.store.migrate().await.is_err());
    let trigger_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_trigger WHERE tgrelid='retained_nonce_dispatch_proposals'::regclass AND tgname='retained_nonce_dispatch_proposals_immutable')")
        .fetch_one(&f.pool).await.unwrap();
    assert!(
        !trigger_exists,
        "failed migration must not commit earlier statements"
    );
    use sqlx::Connection;
    let mut other = sqlx::PgConnection::connect_with(f.pool.connect_options().as_ref())
        .await
        .unwrap();
    let got: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(4270265017)")
        .fetch_one(&mut other)
        .await
        .unwrap();
    assert!(got);
    sqlx::query("SELECT pg_advisory_unlock(4270265017)")
        .execute(&mut other)
        .await
        .unwrap();
    sqlx::raw_sql("DROP VIEW retained_nonce_dispatch_authorizations; DROP VIEW retained_nonce_dispatch_proposals_immutable;").execute(&f.pool).await.unwrap();
    f.store.migrate().await.unwrap();
}
