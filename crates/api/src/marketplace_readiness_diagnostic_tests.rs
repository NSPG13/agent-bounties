//! Exercise the real database/head waits and the actual emitted diagnostic.
//! Each child has an isolated stderr stream; no production logging hook is added.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const FIXTURE: &str = "AGENT_BOUNTIES_READINESS_DIAGNOSTIC_FIXTURE";
const TEST: &str = "marketplace_readiness::diagnostic_tests::observation_timeout_stages_postgres";

#[tokio::test]
#[ignore = "requires the isolated AGENT_BOUNTIES_TEST_DATABASE_URL fixture"]
async fn observation_timeout_stages_postgres() {
    if let Ok(case) = std::env::var(FIXTURE) {
        run_case(&case).await;
        return;
    }
    for case in [
        "snapshot_exact",
        "snapshot_shared",
        "snapshot_readiness",
        "heads_exact",
        "heads_shared",
        "heads_readiness",
        "mixed_exact",
        "mixed_readiness",
    ] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([TEST, "--exact", "--ignored", "--nocapture"])
            .env(FIXTURE, case)
            .env("BASE_RPC_READ_WIRE_INTERVAL_MS", "0")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case}: fixture exceeded its outer safety deadline");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let events: Vec<Value> = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| event["event"] == "service_failure_stage")
            .collect();
        let expected = if case.starts_with("snapshot_") {
            "marketplace_snapshot_timed_out"
        } else {
            "marketplace_heads_timed_out"
        };
        assert_eq!(events.len(), 1, "{case}: {events:?}");
        assert_eq!(events[0]["stage"], expected, "{case}");
        assert!(!events[0].to_string().contains("private-fixture"));
        println!("{case}: {expected}; unavailable within the unchanged 12-second bound");
    }
}

async fn run_case(case: &str) {
    let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&database_url).unwrap();
    assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    assert_eq!(parsed.username(), "agent_bounties_test");
    assert_eq!(parsed.path(), "/agent_bounties_test");
    assert_eq!(
        std::env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
        Ok("1")
    );
    // One actual SQLx connection lets the fixture exhaust database capacity
    // without changing production query code or inventing a database error.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(60))
        .connect(&database_url)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let factory = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let now = Utc::now();
    let heartbeat = BaseIndexerHeartbeat {
        network: "base-mainnet".into(),
        escrow_contract: factory.clone(),
        status: "success".into(),
        started_at: now,
        completed_at: Some(now),
        latest_block: Some(1000),
        confirmed_to_block: Some(998),
        from_block: Some(990),
        to_block: Some(998),
        fetched_logs: 0,
        persisted_cursor_block: Some(998),
        skipped_reason: None,
        error_message: None,
        updated_at: now,
    };
    store
        .upsert_base_log_cursor("base-mainnet", &factory, 998, None)
        .await
        .unwrap();
    store
        .upsert_base_indexer_heartbeat(&heartbeat)
        .await
        .unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let app = Router::new().route(
        "/private-fixture",
        post(move || {
            requests.fetch_add(1, Ordering::SeqCst);
            async {
                // The request reaches a real local HTTP server, which never
                // replies. Only the unchanged caller deadline can cancel it.
                std::future::pending::<()>().await;
                StatusCode::OK
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}/private-fixture", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused",
        store,
    );
    Arc::get_mut(&mut state)
        .unwrap()
        .marketplace_head_rpc
        .base_mainnet = Some(rpc_url.clone());
    let mut blocked_database = if case.starts_with("snapshot_") || case.starts_with("mixed_") {
        Some(pool.acquire().await.unwrap())
    } else {
        None
    };
    let started = std::time::Instant::now();
    let release_database = if case.starts_with("mixed_") {
        let connection = blocked_database.take().unwrap();
        Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(6)).await;
            drop(connection);
        }))
    } else {
        None
    };
    match case {
        "snapshot_exact" | "heads_exact" | "mixed_exact" => {
            assert!(matches!(
                require_current_reader(
                    &state,
                    "base-mainnet",
                    AUTONOMOUS_PROTOCOL_VERSION,
                    &factory
                )
                .await,
                Err(StatusCode::SERVICE_UNAVAILABLE)
            ));
        }
        "snapshot_shared" => {
            let snapshot = capture_reader(
                &state,
                "base-mainnet",
                AUTONOMOUS_PROTOCOL_VERSION,
                Some(factory.clone()),
            )
            .await;
            assert!(snapshot.heartbeat.is_none());
            assert!(snapshot.cursor.is_none());
            let mut observation = finish_projection_observation(
                "base-mainnet",
                Some((&rpc_url, 8453)),
                vec![snapshot],
            )
            .await;
            assert!(observation
                .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, &factory)
                .is_err());
        }
        "heads_shared" => {
            let snapshot = ReaderSnapshot {
                protocol: AUTONOMOUS_PROTOCOL_VERSION,
                factory: Some(factory.clone()),
                heartbeat: Some(heartbeat),
                cursor: Some(998),
            };
            let mut observation = finish_projection_observation(
                "base-mainnet",
                Some((&rpc_url, 8453)),
                vec![snapshot],
            )
            .await;
            assert!(observation
                .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, &factory)
                .is_err());
        }
        "snapshot_readiness" | "heads_readiness" | "mixed_readiness" => {
            let response = readiness(State(state)).await;
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(result["ready"], false);
            assert!(result["safe_block"].is_null());
            assert!(result["latest_block"].is_null());
            assert!(result["readers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|reader| reader["ready"] == false));
            assert!(!String::from_utf8_lossy(&bytes).contains("private-fixture"));
        }
        _ => panic!("unknown fixture case"),
    }
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_secs(12), "{case}: {elapsed:?}");
    // A six-second database wait must consume the original budget. Restarting
    // the timer for head reads would take eighteen seconds and fail this check.
    assert!(elapsed < Duration::from_secs(16), "{case}: {elapsed:?}");
    assert_eq!(
        observed.load(Ordering::SeqCst),
        usize::from(!case.starts_with("snapshot_")),
        "database wait must not start RPC; stalled RPC must not retry"
    );
    if let Some(release) = release_database {
        release.await.unwrap();
    }
    drop(blocked_database);
    // The canceled DB acquisition leaves the one connection available.
    let connection = tokio::time::timeout(Duration::from_secs(1), pool.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(connection);
    server.abort();
}
