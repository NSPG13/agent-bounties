use serde_json::Value;

mod status_postgres_fixture {
    include!("../../../scripts/fixtures/postgres-status-support.rs");
}

#[tokio::test]
#[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn bounty_status_reads_scoped_postgres_after_cross_process_funding() {
    if status_postgres_fixture::run_writer_if_requested().await {
        return;
    }
    let store = status_postgres_fixture::store().await;
    let (network, bounties) = status_postgres_fixture::seed(&store).await;
    let (selected, other) = (bounties[0].id, bounties[1].id);
    let state = test_state_with_operator_token_and_store(network, "test-only", store);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/v1/bounties/:id", get(bounty_status))
        .with_state(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let url = format!("{base}/v1/bounties/{selected}");
    let before: Value = client
        .get(&url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before["bounty"]["status"], "Unfunded");
    assert_eq!(before["funding_summary"]["claimable"], false);
    status_postgres_fixture::write_in_separate_process(
        "tests::bounty_status_reads_scoped_postgres_after_cross_process_funding",
        &[selected, other],
    )
    .await;
    assert_eq!(
        state.network.lock().unwrap().bounties[&selected].status,
        BountyStatus::Unfunded
    );
    assert!(state
        .network
        .lock()
        .unwrap()
        .funding_contributions
        .is_empty());
    let current: Value = client
        .get(&url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    status_postgres_fixture::assert_funded(&current, selected, other);
    assert_eq!(
        client
            .get(format!("{base}/v1/bounties/{}", Uuid::new_v4()))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    server.abort();
    let restarted = test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "test-only",
        status_postgres_fixture::store().await,
    );
    let fresh = bounty_status(State(restarted), Path(selected), HeaderMap::new())
        .await
        .unwrap()
        .0;
    status_postgres_fixture::assert_funded(&serde_json::to_value(fresh).unwrap(), selected, other);
}
