use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
mod fixture {
    include!("../../../service-runtime/tests/fixtures/bounty_discovery.rs");
}

fn routes(state: SharedState) -> Router {
    Router::new()
        .route("/feed", get(autonomous_bounty_feed))
        .route("/events", get(list_autonomous_bounty_events))
        .with_state(state)
}
async fn read(app: &Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn discovery_errors_are_not_empty_successes() {
    let app = routes(test_state(BountyNetwork::default()));
    for path in ["/feed?network=typo", "/events?network=typo"] {
        assert_eq!(read(&app, path).await.0, StatusCode::BAD_REQUEST);
    }
    let (status, error) = read(&app, &format!("/events?bounty_id=0x{}", "ab".repeat(20))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["error_code"], "invalid_bounty_id");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("contract address"));
    for path in [
        "/feed",
        "/events",
        "/feed?network=base",
        "/events?network=base",
    ] {
        assert_eq!(read(&app, path).await.0, StatusCode::SERVICE_UNAVAILABLE);
    }
}

#[tokio::test]
#[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn stored_bounties_and_events_survive_aliases_postgres() {
    let store = PostgresStore::connect(&postgres_test_database_url())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let (id, contract) = fixture::seed(&store, "base-mainnet").await;
    let (test_id, _) = fixture::seed(&store, "base-sepolia").await;
    // Current earning reads need independent local heads and durable reader
    // evidence. Never use a public RPC or ambient network configuration here.
    let rpc = Router::new().route(
        "/",
        post(|Json(request): Json<Value>| async move {
            let result = match request["method"].as_str().unwrap() {
                "eth_chainId" => json!("0x2105"),
                "eth_blockNumber" => json!("0x64"),
                "eth_getBlockByNumber" => {
                    assert!(matches!(
                        request["params"][0].as_str(),
                        Some("safe" | "0x64")
                    ));
                    json!({"number":"0x64", "hash":format!("0x{}", "11".repeat(32)),
                        "timestamp":format!("0x{:x}", Utc::now().timestamp())})
                }
                method => panic!("unexpected fixture RPC method: {method}"),
            };
            Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
    let mut state =
        test_state_with_operator_token_and_store(BountyNetwork::default(), "unused", store.clone());
    let configured = Arc::get_mut(&mut state).unwrap();
    configured.base_rpc_urls.base_mainnet = Some(rpc_url.clone());
    configured.base_rpc_urls.base_sepolia = Some(rpc_url);
    let app = routes(state);
    let factory = service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_FACTORY;
    store
        .upsert_base_log_cursor("base-mainnet", factory, 100, None)
        .await
        .unwrap();
    assert!(store
        .get_base_indexer_heartbeat("base-mainnet", factory)
        .await
        .unwrap()
        .is_none());
    // A current cursor and good heads cannot replace the missing heartbeat.
    for scope in [String::new(), format!("&bounty_contract={contract}")] {
        assert_eq!(
            read(
                &app,
                &format!("/feed?network=base-mainnet&claimable_only=true{scope}")
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
    let now = Utc::now();
    store
        .upsert_base_indexer_heartbeat(&BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: factory.into(),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(100),
            confirmed_to_block: Some(100),
            from_block: Some(1),
            to_block: Some(100),
            fetched_logs: 6,
            persisted_cursor_block: Some(100),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        })
        .await
        .unwrap();
    for network in ["base-mainnet", "base", "mainnet", "8453", "BASE"] {
        for scope in [String::new(), format!("&bounty_contract={contract}")] {
            let (status, feed) = read(&app, &format!("/feed?network={network}{scope}")).await;
            assert_eq!(status, StatusCode::OK);
            assert!(feed
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["bounty_id"] == id));
            assert!(!feed
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["bounty_id"] == test_id));
            let funded = feed
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["bounty_id"] == id)
                .unwrap();
            assert_eq!(funded["funded_amount"], "21000000");
            assert_eq!(funded["terms_valid"], false);
            // Even with a fresh reader, funding alone cannot bypass missing
            // terms or verifier readiness. Cover both broad and scoped aliases.
            let (status, ready) = read(
                &app,
                &format!("/feed?network={network}{scope}&claimable_only=true"),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert!(ready
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v["bounty_id"] != id && v["bounty_id"] != test_id));
        }
        let (status, events) =
            read(&app, &format!("/events?network={network}&bounty_id={id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.as_array().unwrap().len(), 6);
    }
    for network in ["base-sepolia", "sepolia", "84532"] {
        assert_eq!(
            read(&app, &format!("/events?network={network}&bounty_id={id}"))
                .await
                .1,
            json!([])
        );
        // Raw Sepolia history remains readable without a configured earning
        // deployment. Do not turn that unavailable earning read into empty success.
        assert_eq!(
            read(
                &app,
                &format!("/feed?network={network}&claimable_only=true")
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        let (status, events) = read(
            &app,
            &format!("/events?network={network}&bounty_id={test_id}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(events.as_array().unwrap().len(), 6);
    }
    // Unknown but well-formed IDs remain a valid empty query.
    assert_eq!(
        read(&app, &format!("/events?bounty_id=0x{}", "00".repeat(32)))
            .await
            .1,
        json!([])
    );
    server.abort();
}
