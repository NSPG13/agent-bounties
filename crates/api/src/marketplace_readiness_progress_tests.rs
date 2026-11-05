use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn snapshot(cursor: u64) -> ReaderSnapshot {
    let now = Utc::now();
    ReaderSnapshot {
        protocol: AUTONOMOUS_PROTOCOL_VERSION,
        factory: Some("fixture".into()),
        heartbeat: Some(BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: "fixture".into(),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(1300),
            confirmed_to_block: Some(cursor),
            from_block: Some(cursor),
            to_block: Some(cursor),
            fetched_logs: 0,
            persisted_cursor_block: Some(cursor),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        }),
        cursor: Some(cursor),
    }
}

fn heads() -> (u64, i64, u64) {
    (1200, Utc::now().timestamp(), 1300)
}

#[tokio::test]
async fn progress_resamples_sampling_race_without_replacing_original_heads() {
    let initial = snapshot(1000);
    let heads = heads();
    let calls = AtomicUsize::new(0);
    let updated = wait_for_reader_progress(
        initial,
        heads,
        tokio::time::Instant::now() + Duration::from_secs(12),
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(snapshot(1200))
        },
    )
    .await;
    let boundary = projection_boundary(
        updated.protocol,
        updated.heartbeat.as_ref(),
        updated.cursor,
        Some((heads.0, heads.1)),
        Some(heads.2),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(boundary.block_number, 1200);
    assert_eq!(boundary.safe_block_number, heads.0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(boundary.expires_at <= DateTime::<Utc>::from_timestamp(heads.1 + 300, 0).unwrap());
}

#[tokio::test]
async fn progress_waits_for_real_durable_progress_with_spaced_db_checks() {
    let calls = AtomicUsize::new(0);
    let started = tokio::time::Instant::now();
    let result = wait_for_reader_progress(
        snapshot(1000),
        heads(),
        started + Duration::from_secs(3),
        || {
            let count = calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(snapshot(if count == 0 { 1000 } else { 1200 }))
        },
    )
    .await;
    assert_eq!(result.cursor, Some(1200));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(started.elapsed() >= Duration::from_secs(1));
}

#[tokio::test]
async fn progress_never_renews_the_deadline_already_spent_on_heads() {
    let original_start = tokio::time::Instant::now() - Duration::from_secs(8);
    let deadline = original_start + Duration::from_secs(12);
    let started = tokio::time::Instant::now();
    let calls = AtomicUsize::new(0);
    let heads = heads();
    let result = wait_for_reader_progress(snapshot(1000), heads, deadline, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            snapshot(1200)
        }
    })
    .await;
    assert_eq!(result.cursor, Some(1000));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_millis(4500));
    assert!(projection_boundary(
        result.protocol,
        result.heartbeat.as_ref(),
        result.cursor,
        Some((heads.0, heads.1)),
        Some(heads.2),
        Utc::now()
    )
    .is_err());
}

#[tokio::test]
async fn progress_does_not_poll_failed_stale_or_inconsistent_siblings() {
    let mut failed = snapshot(1000);
    failed.heartbeat.as_mut().unwrap().status = "failed".into();
    let mut stale = snapshot(1000);
    stale.heartbeat.as_mut().unwrap().completed_at =
        Some(Utc::now() - chrono::Duration::seconds(91));
    let mut inconsistent = snapshot(1000);
    inconsistent
        .heartbeat
        .as_mut()
        .unwrap()
        .persisted_cursor_block = Some(1001);
    for initial in [failed, stale, inconsistent, snapshot(1200)] {
        wait_for_reader_progress(
            initial,
            heads(),
            tokio::time::Instant::now() + Duration::from_secs(12),
            || async {
                panic!("only a healthy reader behind the original safe head can be re-observed")
            },
        )
        .await;
    }
}

#[tokio::test]
async fn progress_stops_on_quarantine_failure_and_regression_without_retry() {
    for mode in [
        "quarantine",
        "failed",
        "regressed",
        "completed_cursor_regressed",
        "identity",
    ] {
        let calls = AtomicUsize::new(0);
        let result = wait_for_reader_progress(
            snapshot(1000),
            heads(),
            tokio::time::Instant::now() + Duration::from_secs(12),
            || {
                calls.fetch_add(1, Ordering::SeqCst);
                let mut updated = snapshot(1200);
                match mode {
                    "quarantine" => {
                        updated.heartbeat = None;
                        updated.cursor = None;
                    }
                    "failed" => updated.heartbeat.as_mut().unwrap().status = "failed".into(),
                    "regressed" => updated.cursor = Some(999),
                    "completed_cursor_regressed" => {
                        updated.heartbeat.as_mut().unwrap().persisted_cursor_block = Some(999)
                    }
                    _ => updated.factory = Some("other".into()),
                }
                std::future::ready(updated)
            },
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_ne!(
            reader_status(
                result.protocol,
                result.heartbeat.as_ref(),
                result.cursor,
                Some(1200),
                Some(1300),
                Utc::now()
            )["ready"],
            true
        );
    }
}

#[tokio::test]
async fn progress_no_change_stays_unavailable_and_does_not_spin() {
    let calls = AtomicUsize::new(0);
    let heads = heads();
    let result = wait_for_reader_progress(
        snapshot(1000),
        heads,
        tokio::time::Instant::now() + Duration::from_millis(1500),
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(snapshot(1000))
        },
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(only_reader_lag(&result, heads));
}

#[test]
fn progress_cannot_renew_a_299_second_old_head_after_eight_seconds() {
    let now = Utc::now();
    let original = Some((1200, now.timestamp() - 299, 1300));
    assert!(current_observed_heads(original, now).is_some());
    let after = now + chrono::Duration::seconds(8);
    assert!(current_observed_heads(original, after).is_none());
    let updated = snapshot(1200);
    assert!(projection_boundary(
        updated.protocol,
        updated.heartbeat.as_ref(),
        updated.cursor,
        original.map(|(safe, time, _)| (safe, time)),
        Some(1300),
        after
    )
    .is_err());
}

#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn progress_shared_observation_uses_four_rpc_calls_with_real_db_progress_postgres() {
    let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let url = url::Url::parse(&database_url).unwrap();
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(
        std::env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
        Ok("1")
    );
    let store = PostgresStore::connect(&database_url).await.unwrap();
    store.migrate().await.unwrap();
    let factory = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let failed_factory = format!("0x{:040x}", Uuid::new_v4().as_u128());
    for (factory, failed) in [(&factory, false), (&failed_factory, true)] {
        let mut heartbeat = snapshot(1000).heartbeat.unwrap();
        heartbeat.escrow_contract = factory.clone();
        if failed {
            heartbeat.status = "failed".into();
        }
        store
            .upsert_base_log_cursor("base-mainnet", factory, 1000, None)
            .await
            .unwrap();
        store
            .upsert_base_indexer_heartbeat(&heartbeat)
            .await
            .unwrap();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let updated_store = store.clone();
    let updated_factories = [factory.clone(), failed_factory.clone()];
    let app = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let calls = calls.clone();
            let store = updated_store.clone();
            let factories = updated_factories.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                if request["method"] == "eth_getBlockByNumber" && request["params"][0] != "safe" {
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        for factory in factories {
                            // Model the real write order: cursor commits before the
                            // successful heartbeat. Both must catch up before ready.
                            store
                                .upsert_base_log_cursor("base-mainnet", &factory, 1200, None)
                                .await
                                .unwrap();
                            let mut heartbeat = snapshot(1200).heartbeat.unwrap();
                            heartbeat.escrow_contract = factory;
                            store
                                .upsert_base_indexer_heartbeat(&heartbeat)
                                .await
                                .unwrap();
                        }
                    });
                }
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!("0x514"),
                    "eth_getBlockByNumber" => json!({
                        "number": if request["params"][0] == "safe" { "0x4b0" } else { "0x514" },
                        "hash": format!("0x{}", "ab".repeat(32)),
                        "timestamp": format!("0x{:x}", Utc::now().timestamp()),
                    }),
                    _ => panic!("only four read-only head calls are allowed"),
                };
                Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused",
        store.clone(),
    );
    Arc::get_mut(&mut state)
        .unwrap()
        .marketplace_head_rpc
        .base_mainnet = Some(rpc_url);
    let mut observation = observe_resolved_projection_readers(
        &state,
        "base-mainnet",
        Some(factory.clone()),
        Some(failed_factory.clone()),
        None,
        None,
    )
    .await;
    let boundary = observation
        .take("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION, &factory)
        .unwrap();
    assert_eq!(boundary.block_number, 1200);
    assert_eq!(boundary.safe_block_number, 1200);
    assert_eq!(
        counted.load(Ordering::SeqCst),
        4,
        "progress must never trigger another head read"
    );
    assert!(
        observation
            .take(
                "base-mainnet",
                OPEN_COMPETITION_PROTOCOL_VERSION,
                &failed_factory
            )
            .is_err(),
        "a failed sibling cannot be retried to green within this request"
    );
    assert_eq!(
        store
            .get_base_indexer_heartbeat("base-mainnet", &failed_factory)
            .await
            .unwrap()
            .unwrap()
            .status,
        "success",
        "the failed sibling really changed while the healthy reader caught up"
    );
    server.abort();
}

#[tokio::test]
async fn progress_requires_the_completed_heartbeat_after_the_cursor_commit() {
    let mut committed_only = snapshot(1000);
    committed_only.cursor = Some(1200);
    let result = wait_for_reader_progress(
        committed_only,
        heads(),
        tokio::time::Instant::now() + Duration::from_secs(12),
        || std::future::ready(snapshot(1200)),
    )
    .await;
    assert_eq!(result.heartbeat.unwrap().persisted_cursor_block, Some(1200));
}

#[test]
fn projection_snapshot_preserves_the_frozen_boundary_and_rejects_regression() {
    let original = snapshot(1200);
    let boundary = projection_boundary(
        original.protocol,
        original.heartbeat.as_ref(),
        original.cursor,
        Some((1180, Utc::now().timestamp())),
        Some(1300),
        Utc::now(),
    )
    .unwrap();
    let mut fresh = db::ProjectionReaderSnapshot {
        heartbeat: original.heartbeat.clone(),
        cursor: Some(db::BaseLogScanCursor {
            network: "base-mainnet".into(),
            escrow_contract: "fixture".into(),
            last_scanned_block: 1200,
            last_log_key: None,
            updated_at: Utc::now(),
        }),
    };
    assert!(boundary.validate_snapshot(&fresh).is_ok());
    fresh.cursor.as_mut().unwrap().last_scanned_block = 1199;
    fresh.heartbeat.as_mut().unwrap().persisted_cursor_block = Some(1199);
    assert!(
        boundary.validate_snapshot(&fresh).is_err(),
        "still above the safe target is insufficient after cursor regression"
    );
    fresh.cursor.as_mut().unwrap().last_scanned_block = 1200;
    fresh.heartbeat = original.heartbeat.clone();
    fresh.heartbeat.as_mut().unwrap().persisted_cursor_block = Some(1199);
    assert!(
        boundary.validate_snapshot(&fresh).is_err(),
        "completed-heartbeat progress cannot regress behind a stable durable cursor"
    );
    fresh.heartbeat = original.heartbeat.clone();
    fresh.heartbeat.as_mut().unwrap().completed_at =
        Some(boundary.completed_at - chrono::Duration::seconds(1));
    assert!(boundary.validate_snapshot(&fresh).is_err());
    fresh.heartbeat = original.heartbeat;
    fresh.heartbeat.as_mut().unwrap().status = "failed".into();
    assert!(boundary.validate_snapshot(&fresh).is_err());
    assert_eq!(boundary.block_number, 1180);
    assert_eq!(boundary.safe_block_number, 1180);
}
