use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};

fn routes(state: SharedState) -> Router {
    Router::new()
        .route("/v1/agents", post(register_agent))
        .route("/v1/capabilities", post(register_capability))
        .route("/v1/bounties/pooled", post(open_pooled_bounty))
        .with_state(state)
}

async fn post_json(app: &Router, path: &str, body: Value, token: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn pooled_request(title: &str) -> OpenPooledBountyRequest {
    OpenPooledBountyRequest {
        bounty_id: None,
        idempotency_key: None,
        title: title.into(),
        template_slug: "write-docs-for-area".into(),
        target_amount_minor: 1000,
        currency: "usdc".into(),
        funding_mode: FundingMode::Simulated,
        privacy: PrivacyLevel::Private,
        funding_targets: vec![],
    }
}

#[tokio::test]
async fn unavailable_store_cannot_leave_an_operator_pooled_ghost() {
    let state = operator_test_state(test_state(BountyNetwork::default()));
    let result = open_pooled_bounty(
        State(state.clone()),
        operator_test_headers(),
        Json(pooled_request("Local fixture")),
    )
    .await;
    assert_eq!(result.unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(state.network.lock().unwrap().bounties.is_empty());
}

#[tokio::test]
async fn operator_credential_cannot_bind_an_unverified_payout_wallet() {
    let state = operator_test_state(test_state(BountyNetwork::default()));
    let result = register_agent(
        State(state.clone()),
        operator_test_headers(),
        Json(RegisterAgentRequest {
            handle: "operator-cannot-bind-wallet".into(),
            payout_wallet: Some("0x1111111111111111111111111111111111111111".into()),
        }),
    )
    .await;
    assert_eq!(result.unwrap_err(), StatusCode::FORBIDDEN);
    assert!(state.network.lock().unwrap().agents.is_empty());
}

#[tokio::test]
#[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
async fn separate_api_instances_hydrate_owned_capabilities_and_replay_current_pooled_state() {
    let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let wallet = format!(
        "0x{}",
        &service_runtime::auth::hash_secret(&Uuid::new_v4().to_string())[..40]
    );
    let now = Utc::now();
    let challenge = db::auth::WalletChallenge {
        id: Uuid::new_v4(),
        domain: "localhost".into(),
        chain_id: 8453,
        address: wallet.clone(),
        message: "Local fixture; cryptographic exchange is covered separately".into(),
        created_at: now,
        expires_at: now + ChronoDuration::minutes(5),
    };
    store.create_wallet_challenge(&challenge).await.unwrap();
    let token = format!(
        "abws_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    store
        .consume_wallet_challenge_and_create_session(
            challenge.id,
            &service_runtime::auth::hash_secret(&token),
            None,
        )
        .await
        .unwrap();
    let first = test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "local-operator",
        store.clone(),
    );
    let second = test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "local-operator",
        PostgresStore::connect(&url).await.unwrap(),
    );
    let first_api = routes(first.clone());
    let second_api = routes(second.clone());
    let handle = format!("local-owned-{}", Uuid::new_v4());
    let registration = json!({"handle":handle,"payout_wallet":wallet});
    let (status, result) = post_json(&first_api, "/v1/agents", registration.clone(), &token).await;
    assert_eq!(status, StatusCode::OK);
    let mut agent: Agent = serde_json::from_value(result).unwrap();
    assert!(second.network.lock().unwrap().agents.is_empty());
    let capability = json!({"agent_id":agent.id,"class":"Coding","template_slugs":["write-docs-for-area"],"min_price_minor":1,"max_price_minor":10,"currency":"usdc","latency_seconds":60,"supported_verifiers":["JsonSchema"]});
    let (status, capability_result) =
        post_json(&second_api, "/v1/capabilities", capability.clone(), &token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an empty second cache must read the durable owned agent"
    );
    assert_eq!(second.network.lock().unwrap().agents.len(), 1);
    assert_eq!(second.network.lock().unwrap().capabilities.len(), 1);
    let principal = format!("wallet:8453:{wallet}");
    assert!(store
        .resource_is_authorized(
            "capability",
            capability_result["id"].as_str().unwrap(),
            &principal,
            "write"
        )
        .await
        .unwrap());
    let mut invalid = capability;
    invalid["max_price_minor"] = json!(0);
    assert_eq!(
        post_json(&second_api, "/v1/capabilities", invalid, &token)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(second.network.lock().unwrap().capabilities.len(), 1);

    agent.status = domain::AgentStatus::Suspended;
    store.upsert_agent(&agent).await.unwrap();
    let (status, replay) = post_json(&first_api, "/v1/agents", registration, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay["status"], "Suspended");
    assert!(matches!(
        first.network.lock().unwrap().agents[&agent.id].status,
        domain::AgentStatus::Suspended
    ));

    let pooled = serde_json::to_value(pooled_request("One local pooled publication")).unwrap();
    let (status, created) =
        post_json(&first_api, "/v1/bounties/pooled", pooled.clone(), &token).await;
    assert_eq!(status, StatusCode::OK);
    let mut bounty: domain::Bounty = serde_json::from_value(created).unwrap();
    bounty.status = BountyStatus::Claimable;
    store.upsert_bounty(&bounty).await.unwrap();
    let (status, replay) =
        post_json(&second_api, "/v1/bounties/pooled", pooled.clone(), &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay["id"], bounty.id.to_string());
    assert_eq!(replay["status"], "Claimable");
    assert_eq!(
        second.network.lock().unwrap().bounties[&bounty.id].status,
        BountyStatus::Claimable
    );
    for index in 0..2 {
        let body =
            serde_json::to_value(pooled_request(&format!("Additional local fixture {index}")))
                .unwrap();
        assert_eq!(
            post_json(&first_api, "/v1/bounties/pooled", body, &token)
                .await
                .0,
            StatusCode::OK
        );
    }
    let denied =
        serde_json::to_value(pooled_request("Fourth publication must be refused")).unwrap();
    assert_eq!(
        post_json(&second_api, "/v1/bounties/pooled", denied, &token)
            .await
            .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        second.network.lock().unwrap().bounties.len(),
        1,
        "quota rejection leaves no cached bounty"
    );
    assert_eq!(
        post_json(&second_api, "/v1/bounties/pooled", pooled, &token)
            .await
            .0,
        StatusCode::OK,
        "a replay is free after quota exhaustion"
    );
}
