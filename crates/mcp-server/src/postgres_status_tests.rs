mod status_postgres_fixture {
    include!("../../../scripts/fixtures/postgres-status-support.rs");
}

#[tokio::test]
#[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding() {
    if status_postgres_fixture::run_writer_if_requested().await {
        return;
    }
    let store = status_postgres_fixture::store().await;
    let (network, bounties) = status_postgres_fixture::seed(&store).await;
    let (selected, other) = (bounties[0].id, bounties[1].id);
    let mut state = test_state_with_network(network);
    Arc::get_mut(&mut state).unwrap().store = Some(store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/tools/get_bounty_status", post(get_bounty_status))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("{base}/tools/get_bounty_status");
    let before: Value = client
        .post(&url)
        .json(&json!({"bounty_id":selected}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["content"][0]["json"]["bounty"]["status"], "Unfunded");
    status_postgres_fixture::write_in_separate_process(
        "tests::mcp_bounty_status_reads_scoped_postgres_after_cross_process_funding",
        &[selected, other],
    )
    .await;
    assert_eq!(
        state.network.lock().unwrap().bounties[&selected].status,
        domain::BountyStatus::Unfunded
    );
    assert!(state
        .network
        .lock()
        .unwrap()
        .funding_contributions
        .is_empty());
    let current: Value = client
        .post(&url)
        .json(&json!({"bounty_id":selected}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(current.get("error").is_none(), "{current}");
    status_postgres_fixture::assert_funded(&current["content"][0]["json"], selected, other);
    let absent: Value = client
        .post(&url)
        .json(&json!({"bounty_id":Uuid::new_v4()}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(absent["error"].as_str().unwrap().contains("not found"));
    assert!(absent.get("content").is_none());
    server.abort();
    let mut restarted = test_state();
    Arc::get_mut(&mut restarted).unwrap().store = Some(status_postgres_fixture::store().await);
    let fresh = get_bounty_status(
        State(restarted),
        HeaderMap::new(),
        Json(BountyIdArgs {
            bounty_id: selected,
        }),
    )
    .await
    .0;
    status_postgres_fixture::assert_funded(&fresh["content"][0]["json"], selected, other);
}
