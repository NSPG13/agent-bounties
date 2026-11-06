use super::*;
use sqlx::PgPool;

async fn fixture() -> (PostgresStore, PostgresStore, PgPool, PgPool, String) {
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").expect("isolated database");
    db::test_support::assert_isolated_security_database(&url);
    let admin = PgPool::connect(&url).await.unwrap();
    let schema = format!("checkpoint_contention_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = PgPool::connect_with(options.clone()).await.unwrap();
    let first = PostgresStore::from_pool(pool.clone());
    first.migrate().await.unwrap();
    let second = PostgresStore::from_pool(PgPool::connect_with(options).await.unwrap());
    (first, second, pool, admin, schema)
}

const FACTORY: &str = "0x1111111111111111111111111111111111111111";
const NETWORK: &str = "base-sepolia";
fn block(number: u64, byte: &str) -> chain_base::BaseBlockIdentity {
    chain_base::BaseBlockIdentity {
        number,
        hash: format!("0x{}", byte.repeat(32)),
        timestamp: 1,
    }
}
async fn durable_pair(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT json_build_object('cursor',(SELECT row_to_json(c) FROM base_log_cursors c WHERE network=$1 AND escrow_contract=$2),'checkpoint',(SELECT row_to_json(k) FROM autonomous_canonical_checkpoints k WHERE network=$1 AND factory_contract=$2))")
        .bind(NETWORK).bind(FACTORY).fetch_one(pool).await.unwrap()
}
async fn cleanup(
    first: PostgresStore,
    second: PostgresStore,
    pool: PgPool,
    admin: PgPool,
    schema: String,
) {
    drop(first);
    drop(second);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn exact_same_checkpoint_cas_race_retries_without_mutation() {
    let (first, second, pool, admin, schema) = fixture().await;
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            None,
            None,
            &block(100, "aa"),
            None,
        )
        .await
        .unwrap();
    // Both pollers read this state before either scan completes.
    let expected = second
        .autonomous_canonical_checkpoint(NETWORK, FACTORY)
        .await
        .unwrap()
        .unwrap();
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            Some(100),
            Some(&expected),
            &block(101, "bb"),
            Some("winner"),
        )
        .await
        .unwrap();
    let before = durable_pair(&pool).await;
    let error = anyhow::Error::new(
        second
            .commit_autonomous_canonical_checkpoint(
                NETWORK,
                FACTORY,
                Some(100),
                Some(&expected),
                &block(101, "bb"),
                Some("loser-must-not-write"),
            )
            .await
            .unwrap_err(),
    );
    assert_eq!(
        durable_pair(&pool).await,
        before,
        "loser cannot alter history, cursor, hash or timestamps"
    );
    assert!(
        indexer_error_is_retryable(&error),
        "already committed identical checkpoint must restart a fresh bounded poll: {error}"
    );
    assert!(matches!(
        IndexerRecoveryPolicy::default().decision_for_error(1, &error),
        IndexerRecoveryDecision::RetryFromPersistedCursor { .. }
    ));
    assert!(
        matches!(
            IndexerRecoveryPolicy::default().decision_for_error(8, &error),
            IndexerRecoveryDecision::ExitForSupervisorRestart { .. }
        ),
        "contention does not remove the existing retry budget"
    );
    assert!(indexer_provider_delay(&error).is_none());
    cleanup(first, second, pool, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn checkpoint_contention_keeps_integrity_and_bootstrap_failures_terminal() {
    let (first, second, pool, admin, schema) = fixture().await;
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            None,
            None,
            &block(100, "aa"),
            None,
        )
        .await
        .unwrap();
    let expected = second
        .autonomous_canonical_checkpoint(NETWORK, FACTORY)
        .await
        .unwrap()
        .unwrap();
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            Some(100),
            Some(&expected),
            &block(101, "bb"),
            None,
        )
        .await
        .unwrap();
    for proposed in [block(101, "cc"), block(102, "cc"), block(99, "aa")] {
        let before = durable_pair(&pool).await;
        let error = anyhow::Error::new(
            second
                .commit_autonomous_canonical_checkpoint(
                    NETWORK,
                    FACTORY,
                    Some(100),
                    Some(&expected),
                    &proposed,
                    None,
                )
                .await
                .unwrap_err(),
        );
        assert!(
            !indexer_error_is_retryable(&error),
            "mismatch must still halt: {error}"
        );
        assert!(matches!(
            IndexerRecoveryPolicy::default().decision_for_error(1, &error),
            IndexerRecoveryDecision::HaltForOperatorInvestigation { .. }
        ));
        assert_eq!(durable_pair(&pool).await, before);
    }
    // Initial/bootstrap evidence must not inherit the concurrent-poller exception.
    let error = anyhow::Error::new(
        second
            .commit_autonomous_bootstrap_checkpoint(NETWORK, FACTORY, 101, &block(101, "bb"), &[])
            .await
            .unwrap_err(),
    );
    assert!(!indexer_error_is_retryable(&error));
    sqlx::query("UPDATE base_log_cursors SET last_scanned_block=102 WHERE network=$1 AND escrow_contract=$2").bind(NETWORK).bind(FACTORY).execute(&pool).await.unwrap();
    let error = anyhow::Error::new(
        second
            .commit_autonomous_canonical_checkpoint(
                NETWORK,
                FACTORY,
                Some(100),
                Some(&expected),
                &block(101, "bb"),
                None,
            )
            .await
            .unwrap_err(),
    );
    assert!(
        !indexer_error_is_retryable(&error),
        "detached cursor remains terminal"
    );
    sqlx::query("UPDATE base_log_cursors SET last_scanned_block=101 WHERE network=$1 AND escrow_contract=$2").bind(NETWORK).bind(FACTORY).execute(&pool).await.unwrap();
    first
        .invalidate_autonomous_canonical_checkpoint(NETWORK, FACTORY)
        .await
        .unwrap();
    let before = durable_pair(&pool).await;
    let error = anyhow::Error::new(
        second
            .commit_autonomous_canonical_checkpoint(
                NETWORK,
                FACTORY,
                Some(100),
                Some(&expected),
                &block(101, "bb"),
                None,
            )
            .await
            .unwrap_err(),
    );
    assert!(
        !indexer_error_is_retryable(&error),
        "quarantine remains terminal"
    );
    assert_eq!(durable_pair(&pool).await, before);
    cleanup(first, second, pool, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn checkpoint_position_is_one_snapshot_while_another_poller_advances() {
    let (first, second, pool, admin, schema) = fixture().await;
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            None,
            None,
            &block(100, "aa"),
            None,
        )
        .await
        .unwrap();
    let mut writer = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE autonomous_canonical_checkpoints IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *writer)
        .await
        .unwrap();
    let reading_store = second.clone();
    let reading = tokio::spawn(async move {
        reading_store
            .autonomous_indexer_position(NETWORK, FACTORY)
            .await
    });
    let mut blocked = false;
    for _ in 0..100 {
        blocked = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND wait_event='relation' AND query LIKE 'SELECT block_number,block_hash,invalidated FROM autonomous_canonical_checkpoints%')")
            .fetch_one(&pool).await.unwrap();
        if blocked {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        blocked,
        "reader must reach checkpoint after taking its cursor snapshot"
    );
    sqlx::query("UPDATE autonomous_canonical_checkpoints SET block_number=101,block_hash=$1 WHERE network=$2 AND factory_contract=$3").bind(block(101,"bb").hash).bind(NETWORK).bind(FACTORY).execute(&mut *writer).await.unwrap();
    sqlx::query("UPDATE base_log_cursors SET last_scanned_block=101 WHERE network=$1 AND escrow_contract=$2").bind(NETWORK).bind(FACTORY).execute(&mut *writer).await.unwrap();
    writer.commit().await.unwrap();
    let old = reading.await.unwrap().unwrap();
    assert_eq!(old.cursor.unwrap().last_scanned_block, 100);
    assert_eq!(
        old.checkpoint.unwrap().block_number,
        100,
        "new checkpoint cannot mix with old cursor"
    );
    let current = second
        .autonomous_indexer_position(NETWORK, FACTORY)
        .await
        .unwrap();
    assert_eq!(current.cursor.unwrap().last_scanned_block, 101);
    assert_eq!(current.checkpoint.unwrap().block_number, 101);
    cleanup(first, second, pool, admin, schema).await;
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn poll_captures_position_before_headers_and_revalidates_next_poll() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let (first, second, pool, admin, schema) = fixture().await;
    first
        .commit_autonomous_canonical_checkpoint(
            NETWORK,
            FACTORY,
            None,
            None,
            &block(100, "aa"),
            None,
        )
        .await
        .unwrap();
    let expected = first
        .autonomous_canonical_checkpoint(NETWORK, FACTORY)
        .await
        .unwrap()
        .unwrap();
    let server_store = first.clone();
    let rounds = Arc::new(AtomicUsize::new(0));
    let observed = rounds.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..6 {
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
            let result = match request["method"].as_str().unwrap() {
                "eth_blockNumber" => {
                    if observed.fetch_add(1, Ordering::SeqCst) == 0 {
                        // Another instance commits after this poll captured its position.
                        server_store
                            .commit_autonomous_canonical_checkpoint(
                                NETWORK,
                                FACTORY,
                                Some(100),
                                Some(&expected),
                                &block(101, "bb"),
                                None,
                            )
                            .await
                            .unwrap();
                    }
                    serde_json::json!("0x66")
                }
                "eth_getBlockByNumber" => {
                    let later = observed.load(Ordering::SeqCst) > 1;
                    let tag = request["params"][0].as_str().unwrap();
                    let number = if later { 101 } else { 100 };
                    assert!(tag == "safe" || tag == if later { "0x65" } else { "0x64" });
                    // A real wrong hash on the next poll still quarantines history.
                    let hash = if later && tag != "safe" {
                        "cc"
                    } else if later {
                        "bb"
                    } else {
                        "aa"
                    };
                    serde_json::json!({"number":format!("0x{number:x}"),"hash":block(number,hash).hash,"timestamp":"0x1"})
                }
                other => panic!("unexpected RPC {other}"),
            };
            let body =
                serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
            reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        }
    });
    let config = AutonomousIndexerConfig {
        network: NETWORK.into(),
        rpc_url: format!("http://{address}"),
        bootstrap_archive_rpc_url: None,
        factory_contract: FACTORY.into(),
        start_block: None,
        poll_seconds: 5,
        confirmations: 0,
        max_blocks_per_query: 100,
        request_id: 123,
        public_base_url: "http://localhost".into(),
    };
    let report = poll_autonomous_indexer_once(&second, &config)
        .await
        .unwrap();
    assert_eq!(report.persisted_cursor_block, Some(100));
    assert_eq!(
        report.skipped_reason.as_deref(),
        Some("no confirmed blocks are ready to scan")
    );
    let actual = second
        .autonomous_canonical_checkpoint(NETWORK, FACTORY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(actual.block_number, 101);
    assert!(!actual.invalidated);
    let error = poll_autonomous_indexer_once(&second, &config)
        .await
        .unwrap_err();
    assert!(
        !indexer_error_is_retryable(&error),
        "fresh exact-header mismatch must still halt"
    );
    assert!(
        second
            .autonomous_canonical_checkpoint(NETWORK, FACTORY)
            .await
            .unwrap()
            .unwrap()
            .invalidated
    );
    assert_eq!(rounds.load(Ordering::SeqCst), 2);
    server.await.unwrap();
    cleanup(first, second, pool, admin, schema).await;
}
