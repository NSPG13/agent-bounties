use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
    middleware,
};
use hmac::{Hmac, Mac};
use managed_orders::{FiatCurrency, PriceBasis, ServiceTerms};
use sha2::Sha256;
use std::sync::Arc;
use tower::ServiceExt;

fn app(state: SharedState) -> Router {
    router()
        .route("/v1/stripe/checkout-webhooks", post(stripe_webhook))
        .layer(middleware::from_fn(
            crate::privacy_cache::protect_private_reads,
        ))
        .with_state(state)
}
async fn request(
    state: SharedState,
    method: &str,
    path: &str,
    headers: HeaderMap,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    *builder.headers_mut().unwrap() = headers;
    builder = builder.header("content-type", "application/json");
    let response = app(state)
        .oneshot(
            builder
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "no-store, private"
    );
    let code = response.status();
    let bytes = to_bytes(response.into_body(), 128 * 1024).await.unwrap();
    (code, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}
fn operator() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        "x-operator-token",
        "managed-fixture-operator".parse().unwrap(),
    );
    h
}
fn policy() -> CheckoutPolicy {
    CheckoutPolicy {
        stripe_account_id: "acct_managed_route_fixture".into(),
        livemode: false,
        currency: FiatCurrency::Mxn,
        minimum_minor: 1000,
        maximum_minor: 1_000_000,
        payment_method_configuration: None,
        adaptive_pricing: false,
    }
}
pub(super) fn input(buyer: &str) -> QuoteInput {
    let now = chrono::Utc::now().timestamp();
    QuoteInput {
        quote_id: Uuid::new_v4(),
        buyer_principal: format!("account:{buyer}"),
        stripe_account_id: policy().stripe_account_id,
        livemode: false,
        issued_at: now - 1,
        expires_at: now + 7200,
        terms: ServiceTerms {
            deliverable: "Fixture deliverable".into(),
            acceptance_criteria: "Fixture criteria".into(),
            delivery_deadline: now + 86400,
            terms_version: "fixture-v1".into(),
            remedies: "Fixture remedy".into(),
        },
        currency: FiatCurrency::Mxn,
        price_basis: PriceBasis {
            reference: "fixture-offer".into(),
            minor_units_per_usdc_numerator: 2000,
            minor_units_per_usdc_denominator: 1,
            valid_until: now + 7200,
        },
        solver_reward_micro_usdc: 100_000_000,
        verifier_reserve_micro_usdc: 5_000_000,
        additional_costs: vec![],
        bounty: managed_orders_bounty(now + 86400),
    }
}
struct Fixture {
    state: SharedState,
    store: db::PostgresStore,
    pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    schema: String,
    buyer: String,
    headers: HeaderMap,
    other: HeaderMap,
}
impl Fixture {
    async fn new() -> Self {
        let url =
            std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").expect("isolated database required");
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("managed_api_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let scoped = schema.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(8)
            .after_connect(move |connection, _| {
                let sql = format!("SET search_path TO {scoped}");
                Box::pin(async move {
                    sqlx::query(&sql).execute(connection).await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await
            .unwrap();
        let store = db::PostgresStore::from_pool(pool.clone());
        store.migrate().await.unwrap();
        let (site, buyer, headers) =
            crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "owner");
        let (_, _, other) =
            crate::site_auth::managed_payment_account_fixture(Some(store.clone()), "other");
        let mut state = crate::tests::test_state(app::BountyNetwork::default());
        let s = Arc::make_mut(&mut state);
        s.store = Some(store.clone());
        s.security_auth = security_auth::SecurityAuthState::for_managed_payment_test(site);
        s.operator_api_token = Some("managed-fixture-operator".into());
        s.managed_payments = ManagedPaymentsConfig {
            recovered_refunds_enabled: false,
            refunds_enabled: false,
            enabled: true,
            public_checkout_enabled: false,
            policy: Some(policy()),
            maximum_usd_minor: None,
            canary_account_id: Some(buyer.clone()),
            bounty: None,
            review_signer: None,
        };
        s.stripe_webhook_secret = Some(b"whsec_managed_route_fixture".to_vec());
        s.stripe_webhook_livemode = Some(false);
        Self {
            state,
            store,
            pool,
            admin,
            schema,
            buyer,
            headers,
            other,
        }
    }
    async fn issued(&self) -> (Uuid, String) {
        let quote = input(&self.buyer);
        let id = quote.quote_id;
        let (code, response) = request(
            self.state.clone(),
            "POST",
            "/v1/managed-service/quotes",
            operator(),
            serde_json::to_value(quote).unwrap(),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        (id, response["quote_digest"].as_str().unwrap().into())
    }
    async fn accepted(&self) -> (Uuid, Uuid, String) {
        let (quote, digest) = self.issued().await;
        let (code, response) = request(
            self.state.clone(),
            "POST",
            &format!("/v1/managed-service/quotes/{quote}/accept"),
            self.headers.clone(),
            json!({"quote_digest":digest,"idempotency_key":"fixture-accept"}),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        (
            Uuid::parse_str(response["order"]["order_id"].as_str().unwrap()).unwrap(),
            quote,
            digest,
        )
    }
    async fn count(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "managed_service_orders"
                | "managed_stripe_checkout_operations"
                | "managed_stripe_event_inbox"
                | "managed_stripe_paid_receipts"
        ));
        sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

#[tokio::test]
async fn managed_routes_require_account_or_operator_auth_before_body_parsing() {
    let state = crate::tests::test_state(app::BountyNetwork::default());
    let id = Uuid::new_v4();
    for (method, path) in [
        ("POST", "/v1/managed-service/quotes".into()),
        ("GET", format!("/v1/managed-service/quotes/{id}")),
        ("POST", format!("/v1/managed-service/quotes/{id}/accept")),
        ("GET", format!("/v1/managed-service/orders/{id}")),
        ("POST", format!("/v1/managed-service/orders/{id}/checkout")),
        ("POST", format!("/v1/managed-service/orders/{id}/refund")),
    ] {
        assert_eq!(
            request(
                state.clone(),
                method,
                &path,
                HeaderMap::new(),
                json!({"malicious":"payload"})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
}
#[test]
fn managed_launch_policy_requires_configuration_and_an_explicit_audience() {
    let state = crate::tests::test_state(app::BountyNetwork::default());
    let buyer = format!("account:{}", "a".repeat(64));
    let mut cfg = ManagedPaymentsConfig::default();
    assert!(!cfg.intake_allowed(&state, &buyer));
    cfg.enabled = true;
    cfg.policy = Some(policy());
    assert!(!cfg.intake_allowed(&state, &buyer));
    cfg.canary_account_id = Some("a".repeat(64));
    assert!(cfg.intake_allowed(&state, &buyer));
    assert!(!cfg.intake_allowed(&state, &format!("account:{}", "b".repeat(64))));
    cfg.enabled = false;
    assert!(!cfg.intake_allowed(&state, &buyer));
    let mut public_state = state;
    Arc::make_mut(&mut public_state).stripe_public_checkout_enabled = true;
    assert!(!cfg.intake_allowed(&public_state, &buyer));
    cfg.enabled = true;
    cfg.canary_account_id = None;
    assert!(cfg.intake_allowed(&public_state, &buyer));
}
#[tokio::test]
async fn managed_public_audience_does_not_open_legacy_checkout() {
    let mut state = crate::tests::test_state(app::BountyNetwork::default());
    let cfg = &mut Arc::make_mut(&mut state).managed_payments;
    cfg.enabled = true;
    cfg.policy = Some(policy());
    cfg.public_checkout_enabled = true;
    let buyer = format!("account:{}", "b".repeat(64));
    assert!(state.managed_payments.intake_allowed(&state, &buyer));
    assert!(!state.stripe_public_checkout_enabled);
    assert_eq!(
        crate::execute_stripe_funding_intent_checkout(
            State(state.clone()),
            Path(Uuid::new_v4()),
            HeaderMap::new()
        )
        .await
        .unwrap_err(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    Arc::make_mut(&mut state).managed_payments.enabled = false;
    assert!(!state.managed_payments.intake_allowed(&state, &buyer));
    Arc::make_mut(&mut state).managed_payments.enabled = true;
    Arc::make_mut(&mut state).managed_payments.policy = None;
    assert!(!state.managed_payments.intake_allowed(&state, &buyer));
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_quote_order_ownership_fee_and_disabled_checkout_are_enforced() {
    let f = Fixture::new().await;
    let (quote, digest) = f.issued().await;
    let path = format!("/v1/managed-service/quotes/{quote}");
    assert_eq!(
        request(f.state.clone(), "GET", &path, f.other.clone(), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (_, view) = request(
        f.state.clone(),
        "GET",
        &path,
        f.headers.clone(),
        Value::Null,
    )
    .await;
    assert_eq!(
        view["quote"]["input"]["solver_reward_micro_usdc"],
        100_000_000u64
    );
    assert_eq!(view["quote"]["platform_fee_micro_usdc"], 7_500_000u64);
    assert_eq!(view["quote"]["total_minor"], 225000u64);
    let accept = format!("{path}/accept");
    let body = json!({"quote_digest":digest,"idempotency_key":"owner-key"});
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &accept,
            f.other.clone(),
            body.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (code, accepted) = request(
        f.state.clone(),
        "POST",
        &accept,
        f.headers.clone(),
        body.clone(),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let order = accepted["order"]["order_id"].as_str().unwrap();
    assert_eq!(
        request(
            f.state.clone(),
            "GET",
            &format!("/v1/managed-service/orders/{order}"),
            f.other.clone(),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for headers in [f.other.clone(), f.headers.clone()] {
        let (code, _) = request(
            f.state.clone(),
            "POST",
            &format!("/v1/managed-service/orders/{order}/checkout"),
            headers,
            json!({}),
        )
        .await;
        assert!(matches!(
            code,
            StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE
        ));
    }
    assert_eq!(f.count("managed_service_orders").await, 1);
    assert_eq!(f.count("managed_stripe_checkout_operations").await, 0);
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_writes_reject_csrf_ambiguous_credentials_and_buyer_price_overrides() {
    let f = Fixture::new().await;
    let (quote, digest) = f.issued().await;
    let path = format!("/v1/managed-service/quotes/{quote}/accept");
    let body = json!({"quote_digest":digest,"idempotency_key":"fixture-key"});
    let mut missing = f.headers.clone();
    missing.remove("origin");
    let mut wrong = f.headers.clone();
    wrong.insert("origin", "https://attacker.invalid".parse().unwrap());
    let mut repeated = f.headers.clone();
    repeated.append("cookie", f.headers["cookie"].clone());
    let mut cookie = f.headers.clone();
    cookie.insert(
        "cookie",
        format!(
            "{}; {}",
            f.headers["cookie"].to_str().unwrap(),
            f.headers["cookie"].to_str().unwrap()
        )
        .parse()
        .unwrap(),
    );
    let mut credentials = f.headers.clone();
    credentials.insert("authorization", "Bearer invalid".parse().unwrap());
    for headers in [missing, wrong, repeated, cookie, credentials, operator()] {
        assert_eq!(
            request(f.state.clone(), "POST", &path, headers, body.clone())
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let mut extra = body.clone();
    extra["amount_minor"] = 1.into();
    assert_eq!(
        request(f.state.clone(), "POST", &path, f.headers.clone(), extra)
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/managed-service/quotes",
            f.headers.clone(),
            serde_json::to_value(input(&f.buyer)).unwrap()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(f.count("managed_service_orders").await, 0);
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_acceptance_replays_during_containment_and_rejects_new_or_changed_orders() {
    let mut f = Fixture::new().await;
    let (order, quote, digest) = f.accepted().await;
    let (new, new_digest) = f.issued().await;
    Arc::make_mut(&mut f.state).managed_payments.enabled = false;
    let path = format!("/v1/managed-service/quotes/{quote}/accept");
    let (code, replay) = request(
        f.state.clone(),
        "POST",
        &path,
        f.headers.clone(),
        json!({"quote_digest":digest,"idempotency_key":"fixture-accept"}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(replay["order"]["order_id"], order.to_string());
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &path,
            f.headers.clone(),
            json!({"quote_digest":digest,"idempotency_key":"changed-key"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &format!("/v1/managed-service/quotes/{new}/accept"),
            f.headers.clone(),
            json!({"quote_digest":new_digest,"idempotency_key":"new-key"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(f.count("managed_service_orders").await, 1);
    f.cleanup().await;
}
fn webhook(plan: &ManagedCheckoutPlan) -> Value {
    json!({"id":"evt_managed_route","object":"event","type":"checkout.session.completed","livemode":false,"created":plan.created_at(),"data":{"object":{
        "id":"cs_test_managed_route","object":"checkout.session","mode":"payment","livemode":false,"status":"complete","payment_status":"paid",
        "metadata":plan.request(plan.created_at(),plan.policy()).unwrap().body["metadata"],"client_reference_id":plan.order_id(),"amount_total":225000,"currency":"mxn","payment_intent":"pi_managed_route",
        "expires_at":plan.expires_at(),"url":null}}})
}
fn signed(value: &Value) -> HeaderMap {
    let body = serde_json::to_vec(value).unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"whsec_managed_route_fixture").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&body);
    let mut headers = HeaderMap::new();
    headers.insert(
        "stripe-signature",
        format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()))
            .parse()
            .unwrap(),
    );
    headers
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_signed_webhook_dispatch_persists_early_events_and_never_uses_legacy_credit() {
    let mut f = Fixture::new().await;
    let (order, _, _) = f.accepted().await;
    let plan = f
        .store
        .prepare_managed_checkout(&format!("account:{}", f.buyer), order, policy())
        .await
        .unwrap();
    let event = webhook(&plan);
    Arc::make_mut(&mut f.state).managed_payments.enabled = false;
    let (code, result) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&event),
        event.clone(),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(result["disposition"], "pending_binding");
    let lease = f
        .store
        .claim_managed_checkout(order)
        .await
        .unwrap()
        .unwrap();
    f.store
        .complete_managed_checkout(
            &lease,
            &plan.session_binding(&event["data"]["object"]).unwrap(),
        )
        .await
        .unwrap();
    let (code, result) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&event),
        event,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(result["disposition"], "duplicate");
    let (_, view) = request(
        f.state.clone(),
        "GET",
        &format!("/v1/managed-service/orders/{order}"),
        f.headers.clone(),
        Value::Null,
    )
    .await;
    assert_eq!(view["order"]["status"], "awaiting_bank_settlement");
    let (code, checkout) = request(
        f.state.clone(),
        "POST",
        &format!("/v1/managed-service/orders/{order}/checkout"),
        f.headers.clone(),
        json!({}),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(checkout["status"], "payment_received");
    assert_eq!(checkout["order_status"], "awaiting_bank_settlement");
    assert!(checkout.get("checkout").is_none());
    assert_eq!(f.count("managed_stripe_paid_receipts").await, 1);
    assert!(f.store.list_payment_events().await.unwrap().is_empty());
    assert!(f.state.network.lock().unwrap().ledger.entries().is_empty());
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_webhook_rejects_unsigned_wrong_account_mode_amount_and_duplicate_signature() {
    let f = Fixture::new().await;
    let (order, _, _) = f.accepted().await;
    let plan = f
        .store
        .prepare_managed_checkout(&format!("account:{}", f.buyer), order, policy())
        .await
        .unwrap();
    let valid = webhook(&plan);
    let mut wrong_account = valid.clone();
    wrong_account["data"]["object"]["metadata"]["merchant_account_id"] = "acct_other".into();
    let mut wrong_mode = valid.clone();
    wrong_mode["livemode"] = true.into();
    let mut wrong_amount = valid.clone();
    wrong_amount["data"]["object"]["amount_total"] = 1.into();
    let mut wrong_currency = valid.clone();
    wrong_currency["data"]["object"]["currency"] = "usd".into();
    let mut connected_account = valid.clone();
    connected_account["account"] = "acct_other".into();
    let mut inner_mode = valid.clone();
    inner_mode["data"]["object"]["livemode"] = true.into();
    for body in [
        wrong_account,
        wrong_mode,
        wrong_amount,
        wrong_currency,
        connected_account,
        inner_mode,
    ] {
        assert!(request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&body),
            body
        )
        .await
        .0
        .is_client_error());
    }
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            HeaderMap::new(),
            valid.clone()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut tampered = valid.clone();
    tampered["data"]["object"]["amount_total"] = 2.into();
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&valid),
            tampered
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let mut unsigned_test_state = f.state.clone();
    let state = Arc::make_mut(&mut unsigned_test_state);
    state.allow_unsigned_stripe_webhooks = true;
    state.stripe_webhook_secret = None;
    assert!(!request(
        unsigned_test_state,
        "POST",
        "/v1/stripe/checkout-webhooks",
        HeaderMap::new(),
        valid.clone()
    )
    .await
    .0
    .is_success());
    let mut repeated = signed(&valid);
    repeated.append("stripe-signature", repeated["stripe-signature"].clone());
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            repeated,
            valid
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(f.count("managed_stripe_event_inbox").await, 0);
    assert_eq!(f.count("managed_stripe_paid_receipts").await, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_canary_and_request_quota_are_enforced_on_the_account() {
    let mut f = Fixture::new().await;
    let (quote, _) = f.issued().await;
    // A different verified account is not in the private canary audience.
    let (_, other_id, _) =
        crate::site_auth::managed_payment_account_fixture(Some(f.store.clone()), "other");
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/managed-service/quotes",
            operator(),
            serde_json::to_value(input(&other_id)).unwrap()
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    Arc::make_mut(&mut f.state).managed_payments.enabled = false;
    let path = format!("/v1/managed-service/quotes/{quote}");
    for _ in 0..60 {
        assert_eq!(
            request(
                f.state.clone(),
                "GET",
                &path,
                f.headers.clone(),
                Value::Null
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    let retention: i64 = sqlx::query_scalar("SELECT max(extract(epoch FROM (expires_at-created_at)))::bigint FROM security_quota_reservations WHERE action='managed_read'")
        .fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        retention, 300,
        "request-only quotas must not retain a week of reads"
    );
    // Reusing the same resource is still charged to the request quota during containment.
    assert_eq!(
        request(
            f.state.clone(),
            "GET",
            &path,
            f.headers.clone(),
            Value::Null
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another account is neither throttled by this buyer nor given their quote.
    assert_eq!(
        request(f.state.clone(), "GET", &path, f.other.clone(), Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.count("managed_service_orders").await, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_policy_drift_and_missing_credentials_cannot_start_a_payment_operation() {
    let mut f = Fixture::new().await;
    let (quote, digest) = f.issued().await;
    let path = format!("/v1/managed-service/quotes/{quote}/accept");
    let body = json!({"quote_digest":digest,"idempotency_key":"policy-drift"});
    let mut changed = policy();
    changed.stripe_account_id = "acct_changed".into();
    Arc::make_mut(&mut f.state).managed_payments.policy = Some(changed);
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &path,
            f.headers.clone(),
            body.clone()
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(f.count("managed_service_orders").await, 0);
    Arc::make_mut(&mut f.state).managed_payments.policy = Some(policy());
    let (code, response) = request(f.state.clone(), "POST", &path, f.headers.clone(), body).await;
    assert_eq!(code, StatusCode::OK);
    let id = response["order"]["order_id"].as_str().unwrap();
    Arc::make_mut(&mut f.state).stripe_live_execution_enabled = true;
    for credential in [None, Some("rk_live_wrong_mode".to_owned())] {
        Arc::make_mut(&mut f.state).stripe_secret_key = credential;
        assert_eq!(
            request(
                f.state.clone(),
                "POST",
                &format!("/v1/managed-service/orders/{id}/checkout"),
                f.headers.clone(),
                json!({})
            )
            .await
            .0,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(f.count("managed_stripe_checkout_operations").await, 0);
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_usd_limit_blocks_new_value_but_preserves_committed_acceptance_and_reads() {
    let mut f = Fixture::new().await;
    let (order, accepted_quote, accepted_digest) = f.accepted().await;
    let (unaccepted_quote, unaccepted_digest) = f.issued().await;
    let state = Arc::make_mut(&mut f.state);
    state.managed_payments.maximum_usd_minor = Some(600);
    state.stripe_live_execution_enabled = true;
    state.stripe_secret_key = Some("sk_test_usd_limit_fixture".into());

    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/managed-service/quotes",
            operator(),
            serde_json::to_value(input(&f.buyer)).unwrap()
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &format!("/v1/managed-service/quotes/{unaccepted_quote}/accept"),
            f.headers.clone(),
            json!({"quote_digest":unaccepted_digest,"idempotency_key":"new-after-cap"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, replay) = request(
        f.state.clone(),
        "POST",
        &format!("/v1/managed-service/quotes/{accepted_quote}/accept"),
        f.headers.clone(),
        json!({"quote_digest":accepted_digest,"idempotency_key":"fixture-accept"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replay["order"]["order_id"], order.to_string());
    assert_eq!(
        request(
            f.state.clone(),
            "GET",
            &format!("/v1/managed-service/orders/{order}"),
            f.headers.clone(),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &format!("/v1/managed-service/orders/{order}/checkout"),
            f.headers.clone(),
            json!({})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(f.count("managed_stripe_checkout_operations").await, 0);

    let mut small = input(&f.buyer);
    small.solver_reward_micro_usdc = 1_000_000;
    small.verifier_reserve_micro_usdc = 100_000;
    let id = small.quote_id;
    let (status, offer) = request(
        f.state.clone(),
        "POST",
        "/v1/managed-service/quotes",
        operator(),
        serde_json::to_value(small).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &format!("/v1/managed-service/quotes/{id}/accept"),
            f.headers.clone(),
            json!({"quote_digest":offer["quote_digest"],"idempotency_key":"small-after-cap"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(f.count("managed_service_orders").await, 2);
    f.cleanup().await;
}

async fn paid(f: &Fixture) -> Uuid {
    let (order, _, _) = f.accepted().await;
    let plan = f
        .store
        .prepare_managed_checkout(&format!("account:{}", f.buyer), order, policy())
        .await
        .unwrap();
    let event = webhook(&plan);
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&event),
            event.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    let lease = f
        .store
        .claim_managed_checkout(order)
        .await
        .unwrap()
        .unwrap();
    f.store
        .complete_managed_checkout(
            &lease,
            &plan.session_binding(&event["data"]["object"]).unwrap(),
        )
        .await
        .unwrap();
    order
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_refund_requires_owner_origin_and_empty_body_but_survives_charge_containment() {
    let mut f = Fixture::new().await;
    let order = paid(&f).await;
    let path = format!("/v1/managed-service/orders/{order}/refund");
    Arc::make_mut(&mut f.state).managed_payments.enabled = false;
    for (headers, expected) in [
        (f.other.clone(), StatusCode::NOT_FOUND),
        (operator(), StatusCode::UNAUTHORIZED),
    ] {
        assert_eq!(
            request(f.state.clone(), "POST", &path, headers, json!({}))
                .await
                .0,
            expected
        );
    }
    let mut no_origin = f.headers.clone();
    no_origin.remove("origin");
    assert_eq!(
        request(f.state.clone(), "POST", &path, no_origin, json!({}))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            &path,
            f.headers.clone(),
            json!({"amount":1})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    for _ in 0..2 {
        let (code, result) =
            request(f.state.clone(), "POST", &path, f.headers.clone(), json!({})).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(result["order"]["status"], "refund_requested");
        assert_eq!(result["order"]["quote"]["total_minor"], 225000);
        assert_eq!(result["refund_execution"], "deferred");
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_refund_operations")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let bank: String = sqlx::query_scalar("SELECT state FROM managed_stripe_bank_actions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(bank, "refund_reserved");
    // Delayed successful payment delivery must never reopen the reserved funding action.
    let plan = f
        .store
        .get_managed_checkout_plan(&format!("account:{}", f.buyer), order)
        .await
        .unwrap()
        .unwrap();
    let mut late = webhook(&plan);
    late["id"] = "evt_late_payment_after_refund".into();
    late["type"] = "checkout.session.async_payment_succeeded".into();
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&late),
            late
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, checkout) = request(
        f.state.clone(),
        "POST",
        &format!("/v1/managed-service/orders/{order}/checkout"),
        f.headers.clone(),
        json!({}),
    )
    .await;
    assert_eq!(checkout["order_status"], "refund_requested");
    assert!(checkout.get("checkout").is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn managed_refund_dispatch_requires_native_signature_and_binding_before_sent() {
    let mut f = Fixture::new().await;
    let order = paid(&f).await;
    f.store
        .request_managed_refund(&format!("account:{}", f.buyer), order)
        .await
        .unwrap();
    let plan = f
        .store
        .get_managed_refund_plan(&format!("account:{}", f.buyer), order)
        .await
        .unwrap()
        .unwrap();
    let refund = json!({"id":"re_api_fixture","object":"refund","charge":"ch_api_fixture","payment_intent":plan.payment_intent_id(),"amount":plan.amount_minor(),"currency":"mxn","status":"succeeded","created":plan.created_at(),"metadata":plan.request(plan.created_at(),plan.merchant()).unwrap().body["metadata"]});
    let event = json!({"id":"evt_refund_api_fixture","object":"event","type":"refund.updated","livemode":false,"created":plan.created_at(),"data":{"object":refund}});
    Arc::make_mut(&mut f.state).managed_payments.enabled = false;
    assert_ne!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            HeaderMap::new(),
            event.clone()
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut wrong = event.clone();
    wrong["livemode"] = true.into();
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&wrong),
            wrong
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (code, body) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&event),
        event.clone(),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["disposition"], "pending_binding");
    let read = format!("/v1/managed-service/orders/{order}");
    assert_eq!(
        request(
            f.state.clone(),
            "GET",
            &read,
            f.headers.clone(),
            Value::Null
        )
        .await
        .1["order"]["status"],
        "refund_requested"
    );
    let lease = f.store.claim_managed_refund(order).await.unwrap().unwrap();
    f.store
        .complete_managed_refund(&lease, &plan.binding(&event["data"]["object"]).unwrap())
        .await
        .unwrap();
    assert_eq!(
        request(
            f.state.clone(),
            "GET",
            &read,
            f.headers.clone(),
            Value::Null
        )
        .await
        .1["order"]["status"],
        "refund_sent"
    );
    assert_eq!(
        request(
            f.state.clone(),
            "POST",
            "/v1/stripe/checkout-webhooks",
            signed(&event),
            event
        )
        .await
        .1["disposition"],
        "duplicate"
    );
    assert!(f.store.list_payment_events().await.unwrap().is_empty());
    assert!(f.state.network.lock().unwrap().ledger.entries().is_empty());
    f.cleanup().await;
}

fn managed_orders_bounty(funding_deadline: i64) -> managed_orders::BountyCommitment {
    managed_orders::BountyCommitment::test_fixture(false, funding_deadline)
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn signed_card_dispute_stops_funding_and_unrelated_disputes_keep_legacy_handling() {
    let f = Fixture::new().await;
    let (order, _, _) = f.accepted().await;
    let plan = f
        .store
        .prepare_managed_checkout(&format!("account:{}", f.buyer), order, policy())
        .await
        .unwrap();
    let paid = webhook(&plan);
    let lease = f
        .store
        .claim_managed_checkout(order)
        .await
        .unwrap()
        .unwrap();
    f.store
        .complete_managed_checkout(
            &lease,
            &plan.session_binding(&paid["data"]["object"]).unwrap(),
        )
        .await
        .unwrap();
    let (code, _) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&paid),
        paid,
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    let dispute = |intent: &str, amount: u64| {
        json!({"id":"evt_managed_dispute","object":"event","type":"charge.dispute.created","livemode":false,
            "created":plan.created_at()+60,"data":{"object":{"id":"dp_managed_route","object":"dispute","amount":amount,
            "charge":"ch_managed_route","currency":"mxn","payment_intent":intent,"reason":"fraudulent",
            "status":"needs_response","livemode":false}}})
    };
    let over = dispute("pi_managed_route", 225_001);
    let (code, _) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&over),
        over,
    )
    .await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(!f.store.managed_order_disputed(order).await.unwrap());
    let event = dispute("pi_managed_route", 225_000);
    let (code, result) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&event),
        event,
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{result}");
    assert_eq!(result["managed_order_id"], order.to_string());
    assert_eq!(result["disposition"], "dispute_recorded_funding_stopped");
    assert!(f.store.managed_order_disputed(order).await.unwrap());
    let unrelated = dispute("pi_not_managed", 1_000);
    let (code, result) = request(
        f.state.clone(),
        "POST",
        "/v1/stripe/checkout-webhooks",
        signed(&unrelated),
        unrelated,
    )
    .await;
    assert!(result.get("managed_order_id").is_none(), "{code} {result}");
    assert!(f.state.network.lock().unwrap().ledger.entries().is_empty());
    f.cleanup().await;
}
