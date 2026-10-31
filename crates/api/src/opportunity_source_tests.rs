//! Source filters must bound loader work, not just discard the resulting rows.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn legacy_fixture() -> (BountyNetwork, String, String) {
    let mut network = BountyNetwork::default();
    let mut ids = Vec::new();
    for privacy in [PrivacyLevel::Public, PrivacyLevel::Private] {
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: format!("Source filter {privacy:?}"),
                template_slug: "fix-ci-failure".into(),
                amount_minor: 1_000_000,
                currency: "usdc".into(),
                funding_mode: domain::FundingMode::Simulated,
                privacy,
            })
            .unwrap();
        ids.push(bounty.id.to_string());
    }
    (network, ids[0].clone(), ids[1].clone())
}

#[tokio::test]
async fn requested_legacy_source_is_complete_despite_unavailable_unrequested_sources() {
    let (network, public, private) = legacy_fixture();
    let state = crate::tests::test_state(network); // no durable store or chain dependency
    let projection = list_opportunities(
        State(state),
        Query(OpportunityQuery {
            source_type: Some("legacy_bounty".into()),
            view: Some("recent".into()),
            ..Default::default()
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(projection.coverage, OpportunityCoverage::Complete);
    assert!(!projection.degraded);
    assert_eq!(projection.source_statuses.len(), 1);
    assert_eq!(projection.source_statuses[0].source_type, "legacy_bounty");
    assert_eq!(projection.source_statuses[0].item_count, 1);
    assert_eq!(projection.items.len(), 1);
    assert_eq!(projection.items[0].source_id, public);
    assert!(!serde_json::to_string(&projection)
        .unwrap()
        .contains(&private));
}

#[tokio::test]
async fn requested_unavailable_source_does_not_report_unchecked_sources() {
    let state = crate::tests::test_state(BountyNetwork::default());
    let projection = list_opportunities(
        State(state),
        Query(OpportunityQuery {
            source_type: Some("unfunded_offchain".into()),
            ..Default::default()
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(projection.coverage, OpportunityCoverage::Unavailable);
    assert!(projection.degraded);
    assert!(projection.items.is_empty());
    assert_eq!(projection.source_statuses.len(), 1);
    assert_eq!(
        projection.source_statuses[0].source_type,
        "unfunded_offchain"
    );
    assert_eq!(
        projection.source_statuses[0].error.as_deref(),
        Some("durable_store_not_configured")
    );
}

#[tokio::test]
async fn requested_canonical_source_ignores_poisoned_legacy_and_missing_offchain_store() {
    let state = crate::tests::test_state(BountyNetwork::default());
    let legacy = state.network.clone();
    assert!(std::thread::spawn(move || {
        let _guard = legacy.lock().unwrap();
        panic!("poisoned unrequested legacy fixture");
    })
    .join()
    .is_err());
    let projection = list_opportunities(
        State(state),
        Query(OpportunityQuery {
            source_type: Some("canonical_base".into()),
            ..Default::default()
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(projection.source_statuses.len(), 1);
    let canonical = &projection.source_statuses[0];
    assert_eq!(canonical.source_type, "canonical_base");
    assert_eq!(projection.coverage, canonical.coverage);
    assert_eq!(projection.degraded, !canonical.available);
    assert_eq!(projection.coverage, OpportunityCoverage::Unavailable);
    let body = serde_json::to_string(&projection).unwrap();
    assert!(!body.contains("legacy_read_model_unavailable"));
    assert!(!body.contains("unfunded_bounty_store_unavailable"));
}

#[tokio::test]
#[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn noncanonical_http_sources_never_contact_rpc_with_live_reader_rows_postgres() {
    let database_url = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&database_url).unwrap();
    assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    assert_eq!(parsed.username(), "agent_bounties_test");
    assert_eq!(parsed.path(), "/agent_bounties_test");
    assert_eq!(
        env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
        Ok("1")
    );
    assert!(env::var("AGENT_BOUNTIES_TEST_POSTGRES_CONTAINER")
        .unwrap()
        .starts_with("agent-bounties-test-"));
    // Gate launchers run this test alone in its own disposable container. Its
    // real configured factory rows cannot affect any later test or local DB.
    let store = PostgresStore::connect(&database_url).await.unwrap();
    store.migrate().await.unwrap();
    let planner = configured_autonomous_planner("base-mainnet").unwrap();
    let now = Utc::now();
    store
        .upsert_base_log_cursor("base-mainnet", &planner.factory_contract, 998, None)
        .await
        .unwrap();
    store
        .upsert_base_indexer_heartbeat(&BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: planner.factory_contract,
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(1000),
            confirmed_to_block: Some(998),
            from_block: Some(998),
            to_block: Some(998),
            fetched_logs: 0,
            persisted_cursor_block: Some(998),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        })
        .await
        .unwrap();
    let trial_id = Uuid::new_v4();
    store
        .create_or_get_trial_bounty(&NewTrialBounty {
            id: trial_id,
            idempotency_key: format!("source-scope-{trial_id}"),
            request_fingerprint: trial_id.to_string(),
            title: "Source filter durable fixture".into(),
            goal: "Read only the requested source".into(),
            acceptance_criteria: vec!["No unrequested chain reads".into()],
            source_url: None,
            discovery_source: "test".into(),
            status: "open".into(),
            demo_agent_solution: json!({}),
            expires_at: now + chrono::Duration::days(3),
        })
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let rpc = Router::new().route(
        "/",
        post(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { StatusCode::BAD_REQUEST }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let rpc_server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
    let (network, public, private) = legacy_fixture();
    let mut state =
        crate::tests::test_state_with_operator_token_and_store(network, "unused", store);
    Arc::get_mut(&mut state).unwrap().base_rpc_urls.base_mainnet = Some(rpc_url);
    let app = Router::new()
        .route("/opportunities", get(list_opportunities))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    for source in ["legacy_bounty", "unfunded_offchain"] {
        let identity = if source == "legacy_bounty" {
            format!("legacy:{public}")
        } else {
            format!("unfunded:{trial_id}")
        };
        let response = client
            .get(format!(
                "{base}/opportunities?source_type={source}&view=recent&opportunity_id={identity}"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = response.json().await.unwrap();
        assert_eq!(value["coverage"], "complete");
        assert_eq!(value["degraded"], false);
        assert_eq!(value["source_statuses"].as_array().unwrap().len(), 1);
        assert_eq!(value["source_statuses"][0]["source_type"], source);
        assert!(!value.to_string().contains(&private));
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["items"][0]["opportunity_id"], identity);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "unrequested chain read");
    }
    // Positive control: canonical selection still attempts independent fresh heads
    // and reports the mock failure instead of returning unchecked success.
    let value: Value = client
        .get(format!("{base}/opportunities?source_type=canonical_base"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert_eq!(value["coverage"], "unavailable");
    assert_eq!(value["degraded"], true);
    assert!(value["items"].as_array().unwrap().is_empty());
    server.abort();
    rpc_server.abort();
}
