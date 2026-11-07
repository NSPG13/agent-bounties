use super::*;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

struct Fixture {
    admin: sqlx::PgPool,
    pool: sqlx::PgPool,
    schema: String,
    store: PostgresStore,
    config: AutonomousIndexerConfig,
    safe: Arc<AtomicU64>,
    changed_hash: Arc<AtomicBool>,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("safe_lag_{}", uuid::Uuid::new_v4().simple());
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
        let safe = Arc::new(AtomicU64::new(99));
        let changed_hash = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (safe_read, changed, observed) = (safe.clone(), changed_hash.clone(), calls.clone());
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                observed.lock().unwrap().push(request.clone());
                let result = match request["method"].as_str().unwrap() {
                    "eth_blockNumber" => serde_json::json!("0x66"),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let number = if tag == "safe" {
                            safe_read.load(Ordering::SeqCst)
                        } else {
                            u64::from_str_radix(tag.strip_prefix("0x").unwrap(), 16).unwrap()
                        };
                        let hash = if number == 100 && changed.load(Ordering::SeqCst) {
                            "bb"
                        } else {
                            "aa"
                        };
                        serde_json::json!({"number":format!("0x{number:x}"),"hash":format!("0x{}",hash.repeat(32)),"timestamp":"0xa"})
                    }
                    other => panic!("unexpected RPC method {other}"),
                };
                let body = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    .to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        let factory = format!("0x{}", "61".repeat(20));
        let config = AutonomousIndexerConfig {
            network: "base-sepolia".into(),
            rpc_url: format!("http://{address}"),
            bootstrap_archive_rpc_url: None,
            factory_contract: factory.clone(),
            start_block: None,
            poll_seconds: 15,
            confirmations: 2,
            max_blocks_per_query: 1000,
            request_id: 123,
            public_base_url: "http://localhost".into(),
        };
        let event = AutonomousBountyEvent {
            id: uuid::Uuid::new_v4(),
            log_key: "safe-lag-creation".into(),
            tx_hash: format!("0x{}", "62".repeat(32)),
            block_number: 50,
            log_index: 0,
            contract_address: factory.clone(),
            bounty_id: format!("0x{}", "63".repeat(32)),
            kind: AutonomousBountyEventKind::CanonicalBountyCreated,
            data: serde_json::json!({"bounty_contract":format!("0x{}","64".repeat(20))}),
            occurred_at: Utc::now(),
        };
        store
            .upsert_autonomous_bounty_event(&config.network, &event)
            .await
            .unwrap();
        store
            .commit_autonomous_canonical_checkpoint(
                &config.network,
                &factory,
                None,
                None,
                &chain_base::BaseBlockIdentity {
                    number: 100,
                    hash: format!("0x{}", "aa".repeat(32)),
                    timestamp: 10,
                },
                None,
            )
            .await
            .unwrap();
        Self {
            admin,
            pool,
            schema,
            store,
            config,
            safe,
            changed_hash,
            calls,
            server,
        }
    }
    async fn history(&self) -> serde_json::Value {
        sqlx::query_scalar(
            "SELECT jsonb_agg(to_jsonb(e) ORDER BY log_key) FROM autonomous_bounty_events e",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
    async fn checkpoint(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT to_jsonb(c) FROM autonomous_canonical_checkpoints c")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn close(self) {
        self.server.abort();
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn provider_safe_lag_preserves_history_and_recovers_only_after_exact_hash_revalidation() {
    let f = Fixture::new().await;
    let history = f.history().await;
    let checkpoint = f.checkpoint().await;
    let cursor = f
        .store
        .get_base_log_cursor(&f.config.network, &f.config.factory_contract)
        .await
        .unwrap();
    for safe in [99, 98] {
        f.safe.store(safe, Ordering::SeqCst);
        let error = poll_autonomous_indexer_once_with_heartbeat(&f.store, &f.config)
            .await
            .unwrap_err();
        assert!(indexer_error_is_retryable(&error));
        assert_eq!(
            serde_json::to_value(indexer_safe_head_observation(&error)).unwrap(),
            serde_json::json!({"checkpoint_block":100,"provider_safe_block":safe})
        );
        assert_eq!(
            f.history().await,
            history,
            "no timestamp backfill during provider lag"
        );
        assert_eq!(
            f.checkpoint().await,
            checkpoint,
            "no invalidation or timestamp rewrite"
        );
        assert_eq!(
            f.store
                .get_base_log_cursor(&f.config.network, &f.config.factory_contract)
                .await
                .unwrap(),
            cursor
        );
        let heartbeat = f
            .store
            .get_base_indexer_heartbeat(&f.config.network, &f.config.factory_contract)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(heartbeat.status, "failed");
        assert!(heartbeat.error_message.is_some());
        assert!(f
            .store
            .list_autonomous_bounty_events(&f.config.network)
            .await
            .is_err());
        assert!(f
            .store
            .list_verified_autonomous_bounty_events(&f.config.network)
            .await
            .is_err());
        assert!(f
            .store
            .require_autonomous_canonical_checkpoint(
                &f.config.network,
                &f.config.factory_contract,
                50
            )
            .await
            .is_err());
        // Internal archival evidence remains readable for investigation.
        assert_eq!(
            f.store
                .list_autonomous_bounty_events_for_factory(
                    &f.config.network,
                    &f.config.factory_contract,
                    false
                )
                .await
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(
        f.calls.lock().unwrap().len(),
        4,
        "only latest and safe per failed poll; no exact, logs or backfill"
    );
    // Failure does not age into success or leak raw certified history.
    sqlx::query("UPDATE base_indexer_heartbeats SET completed_at=now()-interval '1 day'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f
        .store
        .list_autonomous_bounty_events(&f.config.network)
        .await
        .is_err());
    f.safe.store(100, Ordering::SeqCst);
    let report = poll_autonomous_indexer_once_with_heartbeat(&f.store, &f.config)
        .await
        .unwrap();
    assert_eq!(
        report.skipped_reason.as_deref(),
        Some("no confirmed blocks are ready to scan")
    );
    assert_eq!(report.persisted_cursor_block, Some(100));
    assert_eq!(f.checkpoint().await, checkpoint);
    assert_eq!(
        f.store
            .get_base_log_cursor(&f.config.network, &f.config.factory_contract)
            .await
            .unwrap(),
        cursor
    );
    let calls = f.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 8);
    assert_eq!(
        calls[6]["params"][0], "0x64",
        "exact checkpoint checked before backfill"
    );
    assert_eq!(calls[7]["params"][0], "0x32");
    let heartbeat = f
        .store
        .get_base_indexer_heartbeat(&f.config.network, &f.config.factory_contract)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(heartbeat.status, "skipped");
    assert!(heartbeat.error_message.is_none());
    assert_eq!(
        f.store
            .list_autonomous_bounty_events(&f.config.network)
            .await
            .unwrap()
            .len(),
        1
    );
    f.store
        .require_autonomous_canonical_checkpoint(&f.config.network, &f.config.factory_contract, 50)
        .await
        .unwrap();
    f.close().await;
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn provider_safe_lag_cannot_hide_real_hash_mismatch_or_clear_existing_quarantine() {
    let f = Fixture::new().await;
    let history = f.history().await;
    let checkpoint = f.checkpoint().await;
    f.changed_hash.store(true, Ordering::SeqCst);
    let error = poll_autonomous_indexer_once_with_heartbeat(&f.store, &f.config)
        .await
        .unwrap_err();
    assert!(indexer_error_is_retryable(&error));
    assert_eq!(f.checkpoint().await, checkpoint);
    assert_eq!(
        f.calls.lock().unwrap().len(),
        2,
        "behind safe cannot establish canonical success even if a later exact header would differ"
    );
    f.safe.store(100, Ordering::SeqCst);
    let error = poll_autonomous_indexer_once_with_heartbeat(&f.store, &f.config)
        .await
        .unwrap_err();
    assert!(!indexer_error_is_retryable(&error));
    assert!(error.to_string().contains("chain reorganization detected"));
    assert!(indexer_safe_head_observation(&error).is_none());
    assert_eq!(f.checkpoint().await["invalidated"], true);
    assert_eq!(f.calls.lock().unwrap().len(), 5);
    assert_eq!(f.history().await, history);
    f.changed_hash.store(false, Ordering::SeqCst);
    let quarantined = f.checkpoint().await;
    let error = poll_autonomous_indexer_once_with_heartbeat(&f.store, &f.config)
        .await
        .unwrap_err();
    assert!(!indexer_error_is_retryable(&error));
    assert!(error.to_string().contains("invalid or detached"));
    assert_eq!(
        f.calls.lock().unwrap().len(),
        7,
        "quarantine never rechecks or clears its stored hash"
    );
    assert_eq!(f.checkpoint().await, quarantined);
    assert_eq!(f.history().await, history);
    f.close().await;
}

#[test]
fn typed_safe_lag_uses_existing_bounded_recovery_and_never_raw_text_authority() {
    let error = anyhow::Error::new(AutonomousSafeHeadBehind {
        checkpoint_block: 100,
        provider_safe_block: 99,
    });
    let recovery = IndexerRecoveryPolicy {
        initial_backoff_seconds: 5,
        max_backoff_seconds: 120,
        exit_after_consecutive_failures: 8,
    };
    assert_eq!(
        recovery.decision_for_error(1, &error),
        IndexerRecoveryDecision::RetryFromPersistedCursor {
            consecutive_failures: 1,
            backoff_seconds: 5
        }
    );
    for count in [8, 100, u32::MAX] {
        assert_eq!(
            recovery.decision_for_error(count, &error),
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                consecutive_failures: count,
                backoff_seconds: 120
            }
        );
    }
    assert!(!indexer_error_is_retryable(&anyhow!(error.to_string())));
    assert!(indexer_provider_delay(&error).is_none());
    let outer = error.context(IndexerOperationalError {
        message: "failed to persist failure heartbeat".into(),
        retryable: false,
        provider_delay_seconds: None,
    });
    assert!(
        indexer_safe_head_observation(&outer).is_some(),
        "retain diagnostics without granting retry"
    );
    assert_eq!(
        recovery.decision_for_error(1, &outer),
        IndexerRecoveryDecision::HaltForOperatorInvestigation {
            consecutive_failures: 1
        }
    );
}

#[test]
fn safe_head_holds_preserve_the_ordinary_failure_budget_and_success_reset() {
    let policy = IndexerRecoveryPolicy::default();
    let lag = anyhow::Error::new(AutonomousSafeHeadBehind {
        checkpoint_block: 100,
        provider_safe_block: 99,
    });
    let transport = anyhow::Error::new(ChainBaseError::RpcHttpStatus(503));
    let mut state = IndexerRecoveryState::default();
    for count in 1..8 {
        assert!(
            matches!(state.on_error(&policy,&transport),IndexerRecoveryDecision::RetryFromPersistedCursor { consecutive_failures,.. } if consecutive_failures==count)
        );
    }
    for count in 1..=20 {
        assert!(
            matches!(state.on_error(&policy,&lag),IndexerRecoveryDecision::RetryFromPersistedCursor { consecutive_failures,backoff_seconds } if consecutive_failures==count && (5..=120).contains(&backoff_seconds))
        );
    }
    assert_eq!(
        state.on_error(&policy, &transport),
        IndexerRecoveryDecision::ExitForSupervisorRestart {
            consecutive_failures: 8
        }
    );
    state.on_success();
    assert_eq!(
        state.on_error(&policy, &lag),
        IndexerRecoveryDecision::RetryFromPersistedCursor {
            consecutive_failures: 1,
            backoff_seconds: 5
        }
    );
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn historical_availability_is_factory_scoped_and_does_not_invent_freshness() {
    let f = Fixture::new().await;
    // Certified historical rows without a heartbeat retain archival access.
    assert_eq!(
        f.store
            .list_autonomous_bounty_events(&f.config.network)
            .await
            .unwrap()
            .len(),
        1
    );
    f.store
        .require_autonomous_canonical_checkpoint(&f.config.network, &f.config.factory_contract, 50)
        .await
        .unwrap();
    let other = format!("0x{}", "65".repeat(20));
    f.store
        .commit_autonomous_canonical_checkpoint(
            &f.config.network,
            &other,
            None,
            None,
            &chain_base::BaseBlockIdentity {
                number: 100,
                hash: format!("0x{}", "aa".repeat(32)),
                timestamp: 10,
            },
            None,
        )
        .await
        .unwrap();
    sqlx::query("INSERT INTO base_indexer_heartbeats(network,escrow_contract,status,started_at,completed_at,error_message) VALUES($1,$2,'failed',now(),now(),'unavailable')")
        .bind(&f.config.network).bind(&other).execute(&f.pool).await.unwrap();
    // Network-wide completeness cannot be inferred, but a scoped unrelated
    // factory is not suppressed by the other reader's availability failure.
    assert!(matches!(
        f.store
            .list_autonomous_bounty_events(&f.config.network)
            .await,
        Err(DbError::AutonomousReaderUnavailable)
    ));
    f.store
        .require_autonomous_canonical_checkpoint(&f.config.network, &f.config.factory_contract, 50)
        .await
        .unwrap();
    sqlx::query("UPDATE base_indexer_heartbeats SET status='success',error_message=NULL,completed_at=now()-interval '1 day'")
        .execute(&f.pool).await.unwrap();
    assert_eq!(
        f.store
            .list_autonomous_bounty_events(&f.config.network)
            .await
            .unwrap()
            .len(),
        1
    );
    for status in ["success", "skipped"] {
        sqlx::query("INSERT INTO base_indexer_heartbeats(network,escrow_contract,status,started_at,completed_at,error_message) VALUES($1,$2,$3,now(),now(),'unavailable') ON CONFLICT(network,escrow_contract) DO UPDATE SET status=EXCLUDED.status,error_message=EXCLUDED.error_message")
            .bind(&f.config.network).bind(&f.config.factory_contract).bind(status).execute(&f.pool).await.unwrap();
        assert!(matches!(
            f.store
                .list_autonomous_bounty_events(&f.config.network)
                .await,
            Err(DbError::AutonomousReaderUnavailable)
        ));
        assert!(matches!(
            f.store
                .require_autonomous_canonical_checkpoint(
                    &f.config.network,
                    &f.config.factory_contract,
                    50
                )
                .await,
            Err(DbError::AutonomousReaderUnavailable)
        ));
    }
    sqlx::query(
        "UPDATE base_indexer_heartbeats SET error_message=NULL,completed_at=now()-interval '1 day'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        f.store
            .list_autonomous_bounty_events(&f.config.network)
            .await
            .unwrap()
            .len(),
        1
    );
    f.store
        .require_autonomous_canonical_checkpoint(&f.config.network, &f.config.factory_contract, 50)
        .await
        .unwrap();
    f.close().await;
}
