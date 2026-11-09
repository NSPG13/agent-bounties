use super::*;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

fn block(number: u64, timestamp: u64) -> BaseBlockIdentity {
    BaseBlockIdentity {
        number,
        timestamp,
        hash: format!("0x{}", "aa".repeat(32)),
    }
}

fn lag() -> anyhow::Error {
    CreatorSafeHeadUnavailable::BehindCheckpoint {
        checkpoint_block: 100,
        provider_safe_block: 99,
    }
    .into()
}

#[test]
fn inconsistent_samples_wait_without_authorizing_ingestion() {
    let head = block(100, 10_000);
    for safe in [block(101, 10_000), block(99, 10_001), block(99, 6_399)] {
        let error = require_usable_safe_head(&head, &safe).unwrap_err();
        assert!(indexer_error_is_retryable(&error));
        assert!(safe_head_observation(&error).is_some());
        assert_eq!(indexer_provider_delay(&error), None);
    }
    require_usable_safe_head(&head, &block(100, 10_000)).unwrap();
    require_usable_safe_head(&head, &block(99, 6_400)).unwrap();
    let text_only = anyhow!("creator-open provider safe head unavailable");
    assert!(!indexer_error_is_retryable(&text_only));
}

#[test]
fn creator_safe_lag_backoff_preserves_the_ordinary_failure_budget() {
    let policy = IndexerRecoveryPolicy::default();
    let mut state = IndexerRecoveryState::default();
    let transient = anyhow::Error::new(ChainBaseError::RpcTransport("offline fixture".into()));
    for _ in 1..policy.exit_after_consecutive_failures {
        assert!(matches!(
            state.on_error(&policy, &transient),
            IndexerRecoveryDecision::RetryFromPersistedCursor { .. }
        ));
    }
    for n in 1_u32..=100 {
        match state.on_error(&policy, &lag()) {
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                backoff_seconds, ..
            } => {
                assert_eq!(
                    backoff_seconds,
                    5_u64.saturating_mul(1_u64 << (n - 1).min(63)).min(120)
                );
            }
            other => panic!("safe lag must not restart sibling readers: {other:?}"),
        }
    }
    assert!(matches!(
        state.on_error(&policy, &transient),
        IndexerRecoveryDecision::ExitForSupervisorRestart { .. }
    ));
    state.on_success();
    assert!(matches!(
        state.on_error(&policy, &transient),
        IndexerRecoveryDecision::RetryFromPersistedCursor {
            consecutive_failures: 1,
            ..
        }
    ));
    let fatal = lag().context(IndexerOperationalError {
        message: "checkpoint persistence conflict".into(),
        retryable: false,
        provider_delay_seconds: None,
    });
    assert!(matches!(
        state.on_error(&policy, &fatal),
        IndexerRecoveryDecision::HaltForOperatorInvestigation { .. }
    ));
}

#[test]
fn creator_recovery_report_retains_bounded_redacted_cause_and_fixed_observation() {
    let decision = IndexerRecoveryDecision::HaltForOperatorInvestigation {
        consecutive_failures: 1,
    };
    let error = anyhow!(
        "RPC failed at https://private.invalid/path?api_key=NEVER_SHOW {}",
        "é".repeat(800)
    );
    let report = creator_open_indexer_recovery_report("base-sepolia", &error, decision);
    let message = report["error"].as_str().unwrap();
    assert!(message.starts_with("RPC failed at [redacted-url]"));
    assert!(!report.to_string().contains("NEVER_SHOW"));
    assert_eq!(message.chars().count(), 512);
    assert!(report["safe_head_observation"].is_null());
    let report = creator_open_indexer_recovery_report("base-sepolia", &lag(), decision);
    assert_eq!(
        report["safe_head_observation"],
        json!({"kind":"behind_checkpoint","checkpoint_block":100,"provider_safe_block":99})
    );
    assert_eq!(
        report["decision"]["action"],
        "halt_for_operator_investigation"
    );
}

struct Fixture {
    admin: sqlx::PgPool,
    pool: sqlx::PgPool,
    schema: String,
    store: PostgresStore,
    release: service::Release,
    bounty: String,
    rpc: String,
    changed_hash: Arc<AtomicBool>,
    calls: Arc<Mutex<Vec<Value>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("creator_lag_{}", uuid::Uuid::new_v4().simple());
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
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../site/fixtures/creator-open-preparations.json"
        ))
        .unwrap();
        let release: service::Release = serde_json::from_value(vectors["release"].clone()).unwrap();
        let request: service::Request =
            serde_json::from_value(vectors["plans"][0]["request"].clone()).unwrap();
        let prepared =
            service::prepare(&release, &request, &service::Signatures::default()).unwrap();
        let bounty = format!("{:#x}", prepared.bounty_contract);
        store
            .save_creator_open_external_creation(
                "base-sepolia",
                &bounty,
                &format!("0x{}", "11".repeat(32)),
                &request,
            )
            .await
            .unwrap();
        // Store valid but deliberately distinguishable retained evidence. The
        // lag branch must not parse, replace or delete it.
        let checkpoint = serde_json::to_value(block(100, 10_000)).unwrap();
        let logs = json!([{"address":bounty,"topics":[],"data":"0x1234",
            "tx_hash":format!("0x{}", "11".repeat(32)),"block_number":99,
            "log_index":4,"occurred_at":"1970-01-01T02:46:39Z"}]);
        let projection = json!({"protocol":protocol::PROTOCOL,"contract":bounty,
            "bounty_id":request.original_terms.bounty_id,"creator":request.original_terms.creator,
            "terms_hash":format!("0x{}", "22".repeat(32)),"revision":1,
            "solver_reward":20_000_000,"review_reward":1_000_000,
            "submission_deadline":2_000_000_000_u64,"review_window":3600,"max_pending":8,
            "status":"open","principal":21_000_000,"outstanding_bonds":0,
            "entries":{},"winner":null,"settlement_transaction":null});
        sqlx::query("UPDATE creator_open_indexes SET checkpoint=$1, logs=$3, projection=$4, available=true WHERE bounty_contract=$2")
            .bind(checkpoint).bind(&bounty).bind(logs).bind(projection).execute(&pool).await.unwrap();
        let changed_hash = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let rpc = format!("http://{}", listener.local_addr().unwrap());
        let (changed, observed) = (changed_hash.clone(), calls.clone());
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
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                }
                assert!(length < 8192);
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                observed.lock().unwrap().push(request.clone());
                let result = match request["method"].as_str().unwrap() {
                    "eth_getBlockByNumber" => {
                        json!({"number":"0x64","timestamp":"0x2710","hash":format!("0x{}", if changed.load(Ordering::SeqCst) { "bb" } else { "aa" }.repeat(32))})
                    }
                    // Stop after proving the exact checkpoint was re-read. A
                    // malformed receipt is an integrity failure, never success.
                    "eth_getTransactionReceipt" => json!({"malformed_fixture_receipt":true}),
                    other => panic!("unexpected RPC call: {other}"),
                };
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        Self {
            admin,
            pool,
            schema,
            store,
            release,
            bounty,
            rpc,
            changed_hash,
            calls,
            server,
        }
    }
    async fn item(&self) -> db::CreatorOpenIndex {
        self.store
            .creator_open_index_batch("base-sepolia", Some(&self.bounty))
            .await
            .unwrap()
            .remove(0)
    }
    async fn row(&self) -> Value {
        sqlx::query_scalar(
            "SELECT to_jsonb(i) FROM creator_open_indexes i WHERE bounty_contract=$1",
        )
        .bind(&self.bounty)
        .fetch_one(&self.pool)
        .await
        .unwrap()
    }
    async fn close(self) {
        self.server.abort();
        let _ = self.server.await;
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
async fn creator_safe_lag_preserves_checkpoint_then_revalidates_exact_hash_postgres() {
    let f = Fixture::new().await;
    let original = f.row().await;
    for safe in [99, 98] {
        let item = f.item().await;
        let error =
            poll_item_with_failure_fence(&f.store, &f.release, &f.rpc, &block(safe, 9_999), &item)
                .await
                .unwrap_err();
        assert!(indexer_error_is_retryable(&error));
        assert!(
            f.calls.lock().unwrap().is_empty(),
            "lag must not query an unsafe checkpoint"
        );
        let row = f.row().await;
        for key in [
            "checkpoint",
            "logs",
            "projection",
            "request_hash",
            "external_creation_id",
        ] {
            assert_eq!(row[key], original[key], "preserve {key}");
        }
        assert_eq!(row["available"], false);
        assert_eq!(row["version"], item.version + 1);
    }
    let item = f.item().await;
    let error =
        poll_item_with_failure_fence(&f.store, &f.release, &f.rpc, &block(100, 10_000), &item)
            .await
            .unwrap_err();
    assert!(
        !indexer_error_is_retryable(&error),
        "malformed receipt remains fatal"
    );
    let calls = f.calls.lock().unwrap().clone();
    assert_eq!(calls[0]["method"], "eth_getBlockByNumber");
    assert_eq!(calls[0]["params"][0], "0x64");
    assert_eq!(calls[1]["method"], "eth_getTransactionReceipt");
    assert_eq!(f.row().await["checkpoint"], original["checkpoint"]);
    f.changed_hash.store(true, Ordering::SeqCst);
    f.calls.lock().unwrap().clear();
    let item = f.item().await;
    assert!(!poll_item_with_failure_fence(
        &f.store,
        &f.release,
        &f.rpc,
        &block(100, 10_000),
        &item
    )
    .await
    .unwrap());
    let row = f.row().await;
    assert!(row["checkpoint"].is_null());
    assert!(row["projection"].is_null());
    assert_eq!(row["logs"], json!([]));
    assert_eq!(row["available"], false);
    assert_eq!(
        f.calls.lock().unwrap().len(),
        1,
        "mismatch stops before receipt/scan"
    );
    f.close().await;
}
