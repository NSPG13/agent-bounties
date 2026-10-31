use super::*;
use crate::{
    prepare_standing_meta_v2_child, publish_autonomous_bounty_terms, tests::test_state,
    PublishAutonomousBountyTermsRequest,
};
use app::BountyNetwork;
use axum::{
    body::{to_bytes, Body},
    http::Request,
    routing::post,
    Router,
};
use chrono::{Duration, Utc};
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

fn request() -> PublishAutonomousBountyTermsRequest {
    let document =
        serde_json::from_str(include_str!("../../../../bounties/autonomous-v1/244.json")).unwrap();
    PublishAutonomousBountyTermsRequest {
        creator_wallet: "0x884834E884d6E93462655A2820140aD03E6747bC".into(),
        document,
    }
}
fn child() -> Value {
    let registry: Value = serde_json::from_str(include_str!(
        "../../../verifier-sdk/regression-profiles-v1.json"
    ))
    .unwrap();
    json!({"parent_bounty_contract":"0x3333333333333333333333333333333333333333", "parent_solver":"0x884834E884d6E93462655A2820140aD03E6747bC", "intended_child_solver":"0x2222222222222222222222222222222222222222", "title":"Test exact bounded fixture", "goal":"Isolated test", "acceptance_criteria":["Test criterion"], "benchmark_source":registry["profiles"][0]["sources"][0],"runner_manifest":registry["profiles"][0]["runner_manifest"],"funding_deadline":2000000000})
}
fn app(state: SharedState) -> Router {
    Router::new()
        .route("/terms", post(publish_autonomous_bounty_terms))
        .route("/child", post(prepare_standing_meta_v2_child))
        .with_state(state)
}
async fn send(app: Router, path: &str, body: &Value, headers: &HeaderMap) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    req.headers_mut().unwrap().extend(headers.clone());
    let response = app
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({"unexpected":String::from_utf8_lossy(&bytes)}));
    (status, body)
}
#[tokio::test]
async fn publication_routes_reject_anonymous_operator_and_duplicate_headers_before_storage() {
    let mut state = test_state(BountyNetwork::default());
    Arc::get_mut(&mut state).unwrap().operator_api_token = Some("fixture-operator".into());
    for (path, body) in [
        ("/terms", serde_json::to_value(request()).unwrap()),
        ("/child", child()),
    ] {
        let (status, body) = send(app(state.clone()), path, &body, &HeaderMap::new()).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        assert_eq!(body["error_code"], "publication_authentication_required");
        let input = if path == "/terms" {
            serde_json::to_value(request()).unwrap()
        } else {
            child()
        };
        for name in ["authorization", "x-operator-token"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                name,
                if name == "authorization" {
                    "Bearer fixture-operator"
                } else {
                    "fixture-operator"
                }
                .parse()
                .unwrap(),
            );
            headers.insert("idempotency-key", "stable".parse().unwrap());
            let (status, body) = send(app(state.clone()), path, &input, &headers).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
            assert_eq!(body["error_code"], "publication_wallet_authority_required");
        }
        for name in service_runtime::terms_publication::SINGULAR_HEADERS {
            let mut headers = HeaderMap::new();
            headers.append(*name, "one".parse().unwrap());
            headers.append(*name, "two".parse().unwrap());
            let (status, body) = send(app(state.clone()), path, &input, &headers).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {name} {body}");
            assert!(body.get("calls").is_none());
        }
    }
}

async fn session(store: &db::PostgresStore, address: &str, chain_id: u64) -> HeaderMap {
    let token = format!(
        "abws_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    let now = Utc::now();
    let id = Uuid::new_v4();
    store
        .create_wallet_challenge(&db::auth::WalletChallenge {
            id,
            domain: "localhost".into(),
            chain_id,
            address: address.to_ascii_lowercase(),
            message: "isolated test fixture; not an external proof".into(),
            created_at: now,
            expires_at: now + Duration::minutes(5),
        })
        .await
        .unwrap();
    store
        .consume_wallet_challenge_and_create_session(id, &auth::hash_secret(&token), None)
        .await
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
    headers.insert(
        "idempotency-key",
        "original-publication-key".parse().unwrap(),
    );
    headers
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn publication_http_wallet_ownership_and_exact_historical_replay() {
    let url =
        std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").expect("isolated PostgreSQL required");
    db::test_support::assert_isolated_security_database(&url);
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("publication_api_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    let store = db::PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let mut state = test_state(BountyNetwork::default());
    Arc::get_mut(&mut state).unwrap().store = Some(store.clone());
    let mut request = request();
    // Historical lookups must normalize the original eligibility input too:
    // serde defaults alone do not lowercase, sort or deduplicate wallet lists.
    request.document.agent_eligibility = Some(domain::AgentEligibilityPolicy {
        wallet_allowlist: vec![
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB".into(),
            "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
        ],
        ..Default::default()
    });
    let body = serde_json::to_value(&request).unwrap();
    let headers = session(&store, &request.creator_wallet, 8453).await;
    // Store a valid original result at its historical creation time, using the
    // same authenticated identity/fingerprint and atomic write as the handler.
    let deadline = request.document.contract_terms["funding_deadline"]
        .as_i64()
        .unwrap();
    let original = chain_base::build_autonomous_bounty_terms_record(
        &request.creator_wallet,
        request.document.clone(),
        chrono::DateTime::from_timestamp(deadline - 60, 0).unwrap(),
    )
    .unwrap();
    let context = PublicationContext::authenticate(
        &state,
        &headers,
        TermsPublicationOperation::Terms,
        "base-mainnet",
        &request.creator_wallet,
        body.clone(),
    )
    .await
    .unwrap();
    let saved = context.publish(&state, &original).await.unwrap();
    assert!(
        crate::autonomous_terms_record(request.clone()).is_err(),
        "new admission must now reject the elapsed cutoff"
    );
    let (status, result) = send(app(state.clone()), "/terms", &body, &headers).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result, serde_json::to_value(&saved).unwrap());
    let mut new_key = headers.clone();
    new_key.insert("idempotency-key", "different-key".parse().unwrap());
    let (status, result) = send(app(state.clone()), "/terms", &body, &new_key).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(result["error_code"], "publication_already_exists");
    assert_eq!(
        result["next_action_url"],
        format!("/v1/base/autonomous-bounties/terms/{}", saved.terms_hash)
    );
    let mut canonical_request = body.clone();
    canonical_request["document"] = serde_json::to_value(&saved.document).unwrap();
    let (status, result) = send(app(state.clone()), "/terms", &canonical_request, &new_key).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(result["error_code"], "publication_already_exists");
    let (status, result) = send(app(state.clone()), "/terms", &canonical_request, &headers).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(result["error_code"], "publication_idempotency_conflict");
    let mut changed = body.clone();
    changed["document"]["goal"] = "changed exact content".into();
    let (status, result) = send(app(state.clone()), "/terms", &changed, &headers).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(result["error_code"], "publication_idempotency_conflict");
    for (wallet, chain) in [
        ("0x2222222222222222222222222222222222222222", 8453),
        (&*request.creator_wallet, 84532),
    ] {
        let wrong = session(&store, wallet, chain).await;
        let (status, result) = send(app(state.clone()), "/terms", &body, &wrong).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
        let (status, result) = send(app(state.clone()), "/child", &child(), &wrong).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    }
    // A valid owner cannot return a child plan without a currently available parent.
    let (status, result) = send(app(state.clone()), "/child", &child(), &headers).await;
    assert_ne!(status, StatusCode::OK);
    assert!(result.get("pre_claim_wallet_calls").is_none());
    // A fresh, valid document is admitted through the actual authenticated route.
    let mut fresh = body.clone();
    fresh["document"]["contract_terms"]["funding_deadline"] = json!(Utc::now().timestamp() + 86400);
    fresh["document"]["contract_terms"]["network"] = "Base".into();
    let mut fresh_headers = headers.clone();
    fresh_headers.insert("idempotency-key", "new-valid-publication".parse().unwrap());
    let (status, fresh_result) = send(app(state.clone()), "/terms", &fresh, &fresh_headers).await;
    assert_eq!(status, StatusCode::OK, "{fresh_result}");
    let (status, repeated) = send(app(state.clone()), "/terms", &fresh, &fresh_headers).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(repeated, fresh_result);
    for expired in [false, true] {
        let revoked = session(&store, &request.creator_wallet, 8453).await;
        let token = revoked["authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        if expired {
            sqlx::query("UPDATE security_wallet_sessions SET created_at=now()-interval '2 minutes',expires_at=now()-interval '1 minute' WHERE token_hash=$1").bind(auth::hash_secret(token)).execute(&pool).await.unwrap();
        } else {
            store
                .revoke_auth_session(&auth::hash_secret(token))
                .await
                .unwrap();
        }
        let (status, result) = send(app(state.clone()), "/terms", &body, &revoked).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{result}");
    }
    // Browser account cookies retain the verified-link + allowed-origin path.
    let (site, account, mut cookie) =
        crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "publication-owner");
    Arc::get_mut(&mut state).unwrap().security_auth =
        security_auth::SecurityAuthState::for_managed_payment_test(site);
    cookie.insert(
        "idempotency-key",
        "account-valid-publication".parse().unwrap(),
    );
    let mut account_body = fresh.clone();
    account_body["document"]["title"] = "A separately published account-owned fixture".into();
    let (status, result) = send(app(state.clone()), "/terms", &account_body, &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "unlinked account: {result}");
    store
        .upsert_site_auth_account(
            &account,
            "google",
            "publication-owner",
            "Fixture",
            "fixture@example.invalid",
            "",
        )
        .await
        .unwrap();
    store
        .link_site_auth_wallet(&account, &request.creator_wallet, 8453)
        .await
        .unwrap();
    for origin in [None, Some("https://attacker.invalid")] {
        let mut wrong_origin = cookie.clone();
        wrong_origin.remove("origin");
        if let Some(origin) = origin {
            wrong_origin.insert("origin", origin.parse().unwrap());
        }
        let (status, result) =
            send(app(state.clone()), "/terms", &account_body, &wrong_origin).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{result}");
    }
    let (status, result) = send(app(state.clone()), "/terms", &account_body, &cookie).await;
    assert_eq!(status, StatusCode::OK, "verified account: {result}");
    let (_, _, mut unlinked) =
        crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "different-account");
    unlinked.insert("idempotency-key", "different-account-key".parse().unwrap());
    let (status, result) = send(app(state.clone()), "/terms", &account_body, &unlinked).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "unlinked other account: {result}"
    );
    let mut invalid_bearer = cookie.clone();
    invalid_bearer.insert("authorization", "Bearer bad".parse().unwrap());
    let (status, result) = send(app(state.clone()), "/terms", &account_body, &invalid_bearer).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "invalid bearer may not fall back to cookie: {result}"
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM security_quota_reservations WHERE action='publication'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 3);
    drop(state);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
