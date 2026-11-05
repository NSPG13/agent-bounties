use super::*;
mod fixture {
    include!("../../../service-runtime/tests/fixtures/bounty_discovery.rs");
}

async fn remote_feed(state: SharedState, network: &str) -> Value {
    remote_feed_with_claimable(state, network, None).await
}

async fn remote_feed_with_claimable(
    state: SharedState,
    network: &str,
    claimable_only: Option<bool>,
) -> Value {
    let response = chatgpt_app::mcp_post(
        State(state),
        HeaderMap::new(),
        Json(json!({
            "jsonrpc":"2.0", "id":1, "method":"tools/call",
            "params":{"name":"list_autonomous_bounties","arguments":{"network":network,"claimable_only":claimable_only}}
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
}

#[tokio::test]
async fn discovery_errors_are_not_empty_successes() {
    for network in ["base", "unknown"] {
        let feed = list_autonomous_bounties(
            State(test_state()),
            Json(AutonomousBountyFeedArgs {
                network: Some(network.to_string()),
                ..Default::default()
            }),
        )
        .await
        .0;
        assert!(feed["error"].is_string(), "{feed}");
        let events = list_autonomous_bounty_events(
            State(test_state()),
            Json(ListAutonomousBountyEventsArgs {
                network: Some(network.to_string()),
                ..Default::default()
            }),
        )
        .await
        .0;
        assert!(events["error"].is_string(), "{events}");
        assert_eq!(remote_feed(test_state(), network).await["isError"], true);
    }
    let events = list_autonomous_bounty_events(
        State(test_state()),
        Json(ListAutonomousBountyEventsArgs {
            bounty_id: Some(format!("0x{}", "ab".repeat(20))),
            ..Default::default()
        }),
    )
    .await
    .0;
    assert!(events["error"].is_string(), "{events}");
    assert!(events.to_string().contains("contract address"));
}

#[tokio::test]
#[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn stored_bounties_and_events_survive_aliases_postgres() {
    const CHILD_MODE: &str = "AGENT_BOUNTIES_MCP_DISCOVERY_FIXTURE_CHILD";
    let mode = std::env::var(CHILD_MODE).ok();
    if mode.is_none() {
        // Upstream configuration is process-wide. Give each child test its own
        // loopback URL instead of mutating this test process's environment.
        let checked_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = checked_requests.clone();
        let rpc = Router::new().route(
            "/:availability/v1/base/autonomous-bounties/feed",
            axum::routing::get(
                move |axum::extract::Path(availability): axum::extract::Path<String>,
                      axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >,
                      headers: HeaderMap| {
                    let observed = observed.clone();
                    async move {
                        assert!(headers.get("authorization").is_none());
                        assert!(headers.get("cookie").is_none());
                        assert_eq!(
                            query.get("network").map(String::as_str),
                            Some("base-mainnet")
                        );
                        assert_eq!(
                            query.get("claimable_only").map(String::as_str),
                            Some("true")
                        );
                        assert!(matches!(availability.as_str(), "available" | "unavailable"));
                        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if availability == "available" {
                            // Transport fixture only: real missing-terms eligibility
                            // is exercised in the API's PostgreSQL regression.
                            (StatusCode::OK, Json(json!([])))
                        } else {
                            (
                                StatusCode::SERVICE_UNAVAILABLE,
                                Json(json!({"error":"reader_unavailable"})),
                            )
                        }
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
        for availability in ["available", "unavailable"] {
            let upstream = format!("http://{address}/{availability}");
            let output = tokio::task::spawn_blocking(move || {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "tests::bounty_discovery_regressions::stored_bounties_and_events_survive_aliases_postgres", "--ignored", "--test-threads=1"])
                    .env(CHILD_MODE, availability)
                    .env("API_UPSTREAM_BASE_URL", upstream)
                    .output().unwrap()
            }).await.unwrap();
            assert!(
                output.status.success(),
                "{availability}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        server.abort();
        // Five network aliases, two transports, two upstream outcomes.
        assert_eq!(
            checked_requests.load(std::sync::atomic::Ordering::Relaxed),
            20
        );
        return;
    }
    let mode = mode.unwrap();
    assert!(matches!(mode.as_str(), "available" | "unavailable"));
    let upstream = url::Url::parse(&std::env::var("API_UPSTREAM_BASE_URL").unwrap()).unwrap();
    assert_eq!(upstream.scheme(), "http");
    assert_eq!(upstream.host_str(), Some("127.0.0.1"));
    assert!(upstream.port().is_some());
    assert_eq!(upstream.path(), format!("/{mode}"));
    let store = PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let (id, _) = fixture::seed(&store, "base-mainnet").await;
    let mut state = test_state();
    Arc::get_mut(&mut state).unwrap().store = Some(store);
    for network in ["base-mainnet", "base", "mainnet", "8453", "BASE"] {
        let feed = list_autonomous_bounties(
            State(state.clone()),
            Json(AutonomousBountyFeedArgs {
                network: Some(network.to_string()),
                ..Default::default()
            }),
        )
        .await
        .0;
        assert!(feed.get("error").is_none(), "{feed}");
        assert!(feed["content"][0]["json"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["bounty_id"] == id));
        let ready = list_autonomous_bounties(
            State(state.clone()),
            Json(AutonomousBountyFeedArgs {
                network: Some(network.to_string()),
                claimable_only: Some(true),
            }),
        )
        .await
        .0;
        let remote_ready = remote_feed_with_claimable(state.clone(), network, Some(true)).await;
        if mode == "available" {
            assert!(ready.get("error").is_none(), "{ready}");
            assert_eq!(ready["content"][0]["json"], json!([]));
            assert_ne!(remote_ready["isError"], true, "{remote_ready}");
            assert_eq!(remote_ready["structuredContent"]["items"], json!([]));
        } else {
            assert!(ready["error"].is_string(), "{ready}");
            assert!(ready.get("content").is_none(), "{ready}");
            assert_eq!(remote_ready["isError"], true, "{remote_ready}");
            assert_eq!(ready["error_code"], "upstream_unavailable", "{ready}");
            assert_eq!(
                remote_ready["structuredContent"],
                json!({"error_code":"upstream_unavailable"}),
                "{remote_ready}"
            );
            assert!(remote_ready["structuredContent"].get("items").is_none());
        }
        let remote = remote_feed(state.clone(), network).await;
        assert_ne!(remote["isError"], true, "{remote}");
        assert!(remote.to_string().contains(&id), "{remote}");
        let events = list_autonomous_bounty_events(
            State(state.clone()),
            Json(ListAutonomousBountyEventsArgs {
                network: Some(network.to_string()),
                bounty_id: Some(id.clone()),
            }),
        )
        .await
        .0;
        assert_eq!(events["content"][0]["json"].as_array().unwrap().len(), 6);
    }
    for network in ["sepolia", "base-sepolia", "84532"] {
        let events = list_autonomous_bounty_events(
            State(state.clone()),
            Json(ListAutonomousBountyEventsArgs {
                network: Some(network.to_string()),
                bounty_id: Some(id.clone()),
            }),
        )
        .await
        .0;
        assert_eq!(events["content"][0]["json"], json!([]));
    }
}
