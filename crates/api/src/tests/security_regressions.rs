use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};

fn routes(state: SharedState) -> Router {
    Router::new()
        .route("/v1/agents", post(register_agent))
        .route(
            "/v1/bounties/:id/participants",
            post(secure_operations::grant_participant),
        )
        .route(
            "/v1/bounties/:id/participants/revoke",
            post(secure_operations::revoke_participant),
        )
        .route("/v1/capabilities", post(register_capability))
        .route("/v1/bounties/:id/claim", post(claim_bounty))
        .route("/v1/bounties/:id/submit", post(submit_result))
        .route("/v1/bounties/:id", get(bounty_status))
        .route("/v1/bounties/claimable", get(list_claimable_bounties))
        .route("/v1/agents/:id/paid-status", get(agent_paid_status))
        .route("/public/proofs/:id", get(public_proof_page))
        .route("/v1/unfunded-bounties", post(publish_unfunded_bounty))
        .route(
            "/v1/stripe/live/funding-intents/:id/checkout-session",
            post(execute_stripe_funding_intent_checkout),
        )
        .route(
            "/v1/base/autonomous-bounties/claims",
            post(agent_native_claim),
        )
        .layer(middleware::from_fn(privacy_cache::protect_private_reads))
        .with_state(state)
}
async fn request(
    app: &Router,
    path: &str,
    body: Option<Value>,
    bearer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut request =
        Request::builder()
            .uri(path)
            .method(if body.is_some() { "POST" } else { "GET" });
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let request = request
        .header(header::CONTENT_TYPE, "application/json")
        .body(
            body.map(|v| Body::from(v.to_string()))
                .unwrap_or_else(Body::empty),
        )
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = String::from_utf8(
        to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    (status, headers, body)
}

#[tokio::test]
async fn anonymous_mutations_are_rejected_before_work_or_database_access() {
    let state = test_state(BountyNetwork::default());
    let app = routes(state.clone());
    let id = Uuid::new_v4();
    let bodies = vec![
        (
            "/v1/agents".into(),
            json!({"handle":"anonymous-fixture","payout_wallet":null}),
        ),
        (
            format!("/v1/bounties/{id}/claim"),
            json!({"bounty_id":id,"solver_agent_id":id}),
        ),
        (
            format!("/v1/bounties/{id}/submit"),
            json!({"bounty_id":id,"solver_agent_id":id,"artifact_uri":"https://example.com/fixture","artifact_body":"fixture"}),
        ),
        (
            "/v1/unfunded-bounties".into(),
            json!({"title":"User-prepared fixture","goal":"Write a concise guide","acceptance_criteria":["Contains tested instructions"],"idempotency_key":"fixture"}),
        ),
        (
            "/v1/base/autonomous-bounties/claims".into(),
            json!({"idempotency_key":"fixture","network":"base-mainnet","bounty_contract":"0x1111111111111111111111111111111111111111","solver_wallet":"0x2222222222222222222222222222222222222222"}),
        ),
    ];
    for (path, body) in bodies {
        let (status, _, _) = request(&app, &path, Some(body), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
    assert!(state.network.lock().unwrap().agents.is_empty());
}

#[tokio::test]
async fn private_funding_checkout_is_hidden_before_stripe_execution() {
    let mut network = BountyNetwork::default();
    let bounty = network
        .open_pooled_bounty(OpenPooledBountyRequest {
            bounty_id: None,
            idempotency_key: None,
            title: "Private funding fixture".into(),
            template_slug: "small-code-change".into(),
            target_amount_minor: 5000,
            currency: "usd".into(),
            funding_mode: domain::FundingMode::StripeFiatLedger,
            privacy: PrivacyLevel::Private,
            funding_targets: vec![],
        })
        .unwrap();
    let intent = network
        .create_funding_intent(
            CreateFundingIntentRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(Uuid::new_v4()),
                amount_minor: 5000,
                currency: "usd".into(),
                rail: domain::PaymentRail::StripeFiat,
                external_reference: Some("private-funding-reference".into()),
                stripe_success_url: None,
                stripe_cancel_url: None,
            },
            "http://127.0.0.1:8080",
        )
        .unwrap()
        .intent;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let app = routes(
        test_state_with_stripe_public_checkout_and_payment_method_configuration(
            network,
            upstream,
            "pmc_fixture",
        ),
    );
    for id in [intent.id, Uuid::new_v4()] {
        let (status, headers, body) = request(
            &app,
            &format!("/v1/stripe/live/funding-intents/{id}/checkout-session"),
            Some(json!({})),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(headers[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store"));
        assert!(!body.contains("private-funding-reference"));
        assert!(!body.contains(&bounty.id.to_string()));
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn historical_private_parent_is_hidden_in_api_proofs_payouts_and_caches() {
    let (mut network, bounty, proof) = completed_simulated_bounty().await;
    let solver = network
        .settlements
        .values()
        .flat_map(|s| &s.payout_intents)
        .next()
        .unwrap()
        .recipient_agent_id;
    network.bounties.get_mut(&bounty.id).unwrap().privacy = PrivacyLevel::Private;
    // Existing child metadata is deliberately left public, as in historical records.
    let app = routes(test_state(network));
    for path in [
        format!("/v1/bounties/{}", bounty.id),
        format!("/public/proofs/{}", proof.id),
    ] {
        let (status, _, body) = request(&app, &path, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!body.contains(&bounty.id.to_string()));
    }
    let (status, headers, body) = request(
        &app,
        &format!("/v1/agents/{solver}/paid-status"),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("no-store"));
    let paid: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(paid["payouts"], json!([]));
    assert_eq!(paid["totals"], json!([]));
    assert!(!body.contains(&bounty.id.to_string()));
    assert!(!body.contains(&proof.id.to_string()));
    let (_, _, feed) = request(&app, "/v1/bounties/claimable", None, None).await;
    assert!(!feed.contains(&bounty.id.to_string()));
}

pub(super) async fn fixture_wallet_session(store: &PostgresStore, wallet: &str) -> String {
    let now = Utc::now();
    let challenge = db::auth::WalletChallenge {
        id: Uuid::new_v4(),
        domain: "localhost".into(),
        chain_id: 8453,
        address: wallet.into(),
        message: "fixture: cryptographic exchange tested separately".into(),
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
    token
}

#[tokio::test]
#[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
async fn wallet_sessions_scope_registration_and_agent_changes_through_api() {
    let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let wallet_value = format!("0x{}11111111", Uuid::new_v4().simple());
    let wallet = wallet_value.as_str();
    let other_value = format!("0x{}22222222", Uuid::new_v4().simple());
    let other = other_value.as_str();
    let token = fixture_wallet_session(&store, wallet).await;
    let other_token = fixture_wallet_session(&store, other).await;
    let state = test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "fixture-operator",
        store.clone(),
    );
    let app = routes(state.clone());
    let handle = format!("fixture-{}", Uuid::new_v4());
    let body = json!({"handle":handle,"payout_wallet":wallet});
    let (status, _, registered) =
        request(&app, "/v1/agents", Some(body.clone()), Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    let agent: Agent = serde_json::from_str(&registered).unwrap();
    let (status, _, replay) = request(&app, "/v1/agents", Some(body), Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(registered, replay);
    let (status, _, _) = request(
        &app,
        "/v1/agents",
        Some(json!({"handle":handle,"payout_wallet":other})),
        Some(&other_token),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _, _) = request(
        &app,
        "/v1/agents",
        Some(json!({"handle":format!("invalid-{handle}"),"payout_wallet":"bad-wallet"})),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = request(
        &app,
        "/v1/agents",
        Some(json!({"handle":format!("wrong-{handle}"),"payout_wallet":other})),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let capability = serde_json::to_value(RegisterCapabilityRequest {
        agent_id: agent.id,
        class: CapabilityClass::Coding,
        template_slugs: vec!["write-docs-for-area".into()],
        min_price_minor: 1,
        max_price_minor: 10,
        currency: "usdc".into(),
        latency_seconds: 60,
        supported_verifiers: vec![VerifierKind::JsonSchema],
    })
    .unwrap();
    let (status, _, _) = request(
        &app,
        "/v1/capabilities",
        Some(capability),
        Some(&other_token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(state.network.lock().unwrap().agents.len(), 1);
    store
        .revoke_auth_session(&service_runtime::auth::hash_secret(&token))
        .await
        .unwrap();
    let (status, _, _) = request(
        &app,
        "/v1/agents",
        Some(json!({"handle":"revoked","payout_wallet":null})),
        Some(&token),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
async fn private_owner_and_explicit_participant_access_through_api() {
    let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let wallet = format!("0x{}11111111", Uuid::new_v4().simple());
    let participant = format!("0x{}22222222", Uuid::new_v4().simple());
    let token = fixture_wallet_session(&store, &wallet).await;
    let participant_token = fixture_wallet_session(&store, &participant).await;
    let bounty = domain::Bounty::new(
        "Private fixture",
        "write-docs-for-area",
        Money::new(100, "usdc").unwrap(),
        domain::FundingMode::Simulated,
        PrivacyLevel::Private,
    );
    store.upsert_bounty(&bounty).await.unwrap();
    let owner = format!("wallet:8453:{wallet}");
    let grantee = format!("wallet:8453:{participant}");
    store
        .claim_resource_ownership("bounty", &bounty.id.to_string(), &owner)
        .await
        .unwrap();
    let app = routes(test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "fixture-operator",
        store.clone(),
    ));
    let path = format!("/v1/bounties/{}", bounty.id);
    let (status, _, _) = request(&app, &path, None, Some(&token)).await;
    assert_eq!(status, StatusCode::OK);
    for credential in [None, Some(participant_token.as_str())] {
        assert_eq!(
            request(&app, &path, None, credential).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let grant = json!({"principal_key":grantee,"expires_at":Utc::now()+ChronoDuration::hours(1)});
    assert_eq!(
        request(
            &app,
            &format!("{path}/participants"),
            Some(grant.clone()),
            Some(&participant_token)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            &format!("{path}/participants"),
            Some(grant),
            Some(&token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (status, headers, body) = request(&app, &path, None, Some(&participant_token)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("Private fixture"));
    assert!(headers[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("no-store"));
    assert_eq!(
        request(&app, &path, None, None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            &format!("{path}/participants/revoke"),
            Some(json!({"principal_key":grantee})),
            Some(&token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, &path, None, Some(&participant_token)).await.0,
        StatusCode::NOT_FOUND
    );
}
