use super::*;
use managed_orders::{FiatCurrency, ManagedQuote, PriceBasis, QuoteInput, ServiceTerms};
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};

fn quote_at(now: i64) -> ManagedQuote {
    ManagedQuote::new(QuoteInput {
        quote_id: Uuid::new_v4(),
        buyer_principal: "account:runtime_fixture".into(),
        stripe_account_id: "acct_runtime_fixture".into(),
        livemode: false,
        issued_at: now - 1,
        expires_at: now + 7200,
        terms: ServiceTerms {
            deliverable: "Synthetic service".into(),
            acceptance_criteria: "Synthetic criteria".into(),
            delivery_deadline: now + 86400,
            terms_version: "fixture-v1".into(),
            remedies: "Fixture refund terms".into(),
        },
        currency: FiatCurrency::Mxn,
        price_basis: PriceBasis {
            reference: "fixture-price".into(),
            minor_units_per_usdc_numerator: 2000,
            minor_units_per_usdc_denominator: 1,
            valid_until: now + 7200,
        },
        solver_reward_micro_usdc: 100_000_000,
        verifier_reserve_micro_usdc: 5_000_000,
        additional_costs: vec![],
        bounty: managed_orders_bounty(now + 86400),
    })
    .unwrap()
}
fn policy() -> CheckoutPolicy {
    CheckoutPolicy {
        stripe_account_id: "acct_runtime_fixture".into(),
        livemode: false,
        currency: FiatCurrency::Mxn,
        minimum_minor: 1000,
        maximum_minor: 1_000_000,
        payment_method_configuration: None,
        adaptive_pricing: false,
    }
}
fn plan() -> ManagedCheckoutPlan {
    let now = chrono::Utc::now().timestamp();
    ManagedCheckoutPlan::new(
        &quote_at(now),
        Uuid::new_v4(),
        Uuid::new_v4(),
        policy(),
        now,
    )
    .unwrap()
}
fn session(plan: &ManagedCheckoutPlan, id: &str) -> Value {
    json!({"id":id,"object":"checkout.session","mode":"payment","livemode":false,
        "metadata":plan.request(plan.created_at(),plan.policy()).unwrap().body["metadata"],
        "client_reference_id":plan.order_id(),"amount_total":225000,"currency":"mxn",
        "status":"open","payment_status":"unpaid","payment_intent":null,"expires_at":plan.expires_at(),
        "url":format!("https://checkout.stripe.com/c/pay/{id}")})
}
fn merchant() -> Value {
    json!({"object":"account","id":"acct_runtime_fixture","charges_enabled":true,"payouts_enabled":true})
}
fn page(data: Vec<Value>, more: bool) -> Value {
    json!({"object":"list","data":data,"has_more":more})
}
#[derive(Debug)]
struct Call {
    kind: &'static str,
    key: Option<String>,
    body: Option<Value>,
    after: Option<String>,
}
struct Fake {
    replies: Mutex<VecDeque<Result<Value>>>,
    calls: Mutex<Vec<Call>>,
}
impl Fake {
    fn new(replies: Vec<Result<Value>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(vec![]),
        }
    }
    fn kinds(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().iter().map(|c| c.kind).collect()
    }
}
#[async_trait]
impl Transport for Fake {
    async fn send(&self, op: Operation) -> Result<Value> {
        let (kind, key, body, after) = match op {
            Operation::Account => ("account", None, None, None),
            Operation::Retrieve(_) => ("retrieve", None, None, None),
            Operation::List { after, .. } => ("list", None, None, after),
            Operation::Create(r) => ("create", Some(r.idempotency_key), Some(r.body), None),
        };
        self.calls.lock().unwrap().push(Call {
            kind,
            key,
            body,
            after,
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected provider call")
    }
}

#[test]
fn production_client_accepts_restricted_keys_but_rejects_mode_and_merchant_mismatch() {
    assert!(ManagedStripeClient::new("rk_test_synthetic", policy()).is_ok());
    assert!(ManagedStripeClient::new("rk_live_synthetic", policy()).is_err());
    assert!(ManagedStripeClient::new("unknown_synthetic", policy()).is_err());
    let mut wrong = policy();
    wrong.stripe_account_id = "acct_bad/path".into();
    assert!(ManagedStripeClient::new("rk_test_synthetic", wrong).is_err());
    let client = ManagedStripeClient::new("rk_test_synthetic", policy()).unwrap();
    assert_eq!(client.transport.base, "https://api.stripe.com");
    assert!(client.transport.authorization.is_sensitive());
}

#[tokio::test]
async fn discovery_finishes_pagination_before_selecting_exact_operation() {
    let plan = plan();
    let f = Fake::new(vec![
        Ok(page(vec![json!({"id":"cs_test_other"})], true)),
        Ok(page(vec![session(&plan, "cs_test_found")], false)),
    ]);
    assert_eq!(
        discover(&f, &plan).await.unwrap(),
        Some("cs_test_found".into())
    );
    assert_eq!(f.kinds(), vec!["list", "list"]);
    assert_eq!(
        f.calls.lock().unwrap()[1].after.as_deref(),
        Some("cs_test_other")
    );
}

#[tokio::test]
async fn discovery_rejects_ambiguity_truncation_duplicate_cursor_and_bad_binding() {
    let plan = plan();
    let f = Fake::new(vec![Ok(page(
        vec![session(&plan, "cs_test_one"), session(&plan, "cs_test_two")],
        false,
    ))]);
    assert!(matches!(
        discover(&f, &plan).await,
        Err(ManagedCheckoutExecutionError::ReconciliationRequired)
    ));
    let f = Fake::new(
        (0..5)
            .map(|n| Ok(page(vec![json!({"id":format!("cs_test_{n}")})], true)))
            .collect(),
    );
    assert!(matches!(
        discover(&f, &plan).await,
        Err(ManagedCheckoutExecutionError::ReconciliationRequired)
    ));
    assert_eq!(f.kinds().len(), 5);
    let f = Fake::new(vec![
        Ok(page(vec![json!({"id":"cs_test_repeat"})], true)),
        Ok(page(vec![json!({"id":"cs_test_repeat"})], false)),
    ]);
    assert!(matches!(
        discover(&f, &plan).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
    let mut bad = session(&plan, "cs_test_bad");
    bad["amount_total"] = 1.into();
    let f = Fake::new(vec![Ok(page(vec![bad], false))]);
    assert!(matches!(
        discover(&f, &plan).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
}

async fn http_reply(reply: Vec<u8>, delay: Duration) -> Result<Value> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buf = [0; 8192];
        let _ = socket.read(&mut buf);
        std::thread::sleep(delay);
        let _ = socket.write_all(&reply);
    });
    // Test-only loopback origin, retaining production redirect and body handling.
    let transport = StripeTransport {
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(200))
            .build()
            .unwrap(),
        authorization: HeaderValue::from_static("Bearer rk_test_synthetic"),
        base: format!("http://{addr}"),
    };
    let result = transport.send(Operation::Account).await;
    server.join().unwrap();
    result
}

#[tokio::test]
async fn http_transport_refuses_redirects_and_does_not_expose_error_bodies() {
    let response=b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/credential-trap\r\nContent-Length: 0\r\n\r\n".to_vec();
    assert!(matches!(
        http_reply(response, Duration::ZERO).await,
        Err(ManagedCheckoutExecutionError::Http(302))
    ));
    let secret = "synthetic-sensitive-provider-body";
    let error = http_reply(
        format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n{secret}",
            secret.len()
        )
        .into_bytes(),
        Duration::ZERO,
    )
    .await
    .unwrap_err();
    assert!(!format!("{error:?} {error}").contains(secret));
}

#[tokio::test]
async fn http_transport_bounds_declared_streaming_and_slow_responses() {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        MAX_RESPONSE_BYTES + 1
    )
    .into_bytes();
    assert!(matches!(
        http_reply(response, Duration::ZERO).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
    let mut response = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    response.extend(vec![b' '; MAX_RESPONSE_BYTES + 1]);
    assert!(matches!(
        http_reply(response, Duration::ZERO).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
    assert!(matches!(
        http_reply(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            Duration::from_millis(350)
        )
        .await,
        Err(ManagedCheckoutExecutionError::Transport)
    ));
    assert!(matches!(
        http_reply(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad".to_vec(),
            Duration::ZERO
        )
        .await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
}

pub(crate) struct Fixture {
    pub(crate) store: PostgresStore,
    pub(crate) pool: sqlx::PgPool,
    admin: sqlx::PgPool,
    schema: String,
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("isolated test database required");
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("managed_runtime_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let scoped = schema.clone();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
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
        let store = PostgresStore::from_pool(pool.clone());
        store.migrate().await.unwrap();
        Self {
            store,
            pool,
            admin,
            schema,
        }
    }
    pub(crate) async fn prepared(&self, age: i64) -> ManagedCheckoutPlan {
        self.prepared_mode(age, false).await
    }
    async fn prepared_mode(&self, age: i64, livemode: bool) -> ManagedCheckoutPlan {
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&self.pool)
                .await
                .unwrap();
        let mut input = quote_at(now - age).input().clone();
        input.livemode = livemode;
        input.bounty =
            managed_orders::BountyCommitment::test_fixture(livemode, input.bounty.funding_deadline);
        let quote = ManagedQuote::new(input).unwrap();
        let mut checkout_policy = policy();
        checkout_policy.livemode = livemode;
        self.store.put_managed_service_quote(&quote).await.unwrap();
        let order = self
            .store
            .accept_managed_service_quote(
                &quote.input().buyer_principal,
                quote.input().quote_id,
                &quote.digest().unwrap(),
                &Uuid::new_v4().to_string(),
            )
            .await
            .unwrap();
        let plan = ManagedCheckoutPlan::new(
            &quote,
            order.order_id,
            Uuid::new_v4(),
            checkout_policy.clone(),
            now - age,
        )
        .unwrap();
        // Historical fixture simulates a committed operation before a process crash.
        sqlx::query("UPDATE managed_service_orders SET accepted_at=$2 WHERE order_id=$1")
            .bind(order.order_id)
            .bind(now - age)
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO managed_stripe_checkout_operations(order_id,operation_id,stripe_account_id,livemode,plan,plan_digest) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(order.order_id).bind(plan.operation_id()).bind(&checkout_policy.stripe_account_id).bind(livemode).bind(serde_json::to_value(&plan).unwrap()).bind(plan.digest()).execute(&self.pool).await.unwrap();
        plan
    }
    async fn due(&self) {
        sqlx::query("UPDATE managed_stripe_checkout_execution SET next_attempt_at=clock_timestamp()-interval '1 second'").execute(&self.pool).await.unwrap();
    }
    async fn counts(&self) -> (i64, i64) {
        let attempts:i64=sqlx::query_scalar("SELECT COALESCE(sum(create_attempts),0)::bigint FROM managed_stripe_checkout_execution").fetch_one(&self.pool).await.unwrap();
        let paid: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_stripe_paid_receipts")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        (attempts, paid)
    }
    pub(crate) async fn cleanup(self) {
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn worker_creates_once_and_response_alone_does_not_claim_payment() {
    let f = Fixture::new().await;
    let plan = f.prepared(0).await;
    let provider = Fake::new(vec![Ok(merchant()), Ok(session(&plan, "cs_test_new"))]);
    assert!(matches!(
        run(&f.store, &policy(), &provider, plan.order_id(), true)
            .await
            .unwrap(),
        ManagedCheckoutExecution::Bound { .. }
    ));
    assert_eq!(f.counts().await, (1, 0));
    assert_eq!(
        run(&f.store, &policy(), &provider, plan.order_id(), true)
            .await
            .unwrap(),
        ManagedCheckoutExecution::Busy
    );
    f.due().await;
    let recovery = Fake::new(vec![Ok(merchant()), Ok(session(&plan, "cs_test_new"))]);
    run(&f.store, &policy(), &recovery, plan.order_id(), false)
        .await
        .unwrap();
    assert_eq!(recovery.kinds(), vec!["account", "retrieve"]);
    assert_eq!(f.counts().await, (1, 0));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn worker_lost_create_response_recovers_with_intake_disabled_without_another_post() {
    let f = Fixture::new().await;
    let plan = f.prepared(0).await;
    let provider = Fake::new(vec![
        Ok(merchant()),
        Err(ManagedCheckoutExecutionError::Transport),
    ]);
    assert!(matches!(
        run(&f.store, &policy(), &provider, plan.order_id(), true).await,
        Err(ManagedCheckoutExecutionError::Transport)
    ));
    f.due().await;
    let response = session(&plan, "cs_test_recovered");
    let mut disabled = merchant();
    disabled["charges_enabled"] = false.into();
    disabled["payouts_enabled"] = false.into();
    let recovery = Fake::new(vec![
        Ok(disabled),
        Ok(page(vec![response.clone()], false)),
        Ok(response),
    ]);
    run(&f.store, &policy(), &recovery, plan.order_id(), false)
        .await
        .unwrap();
    assert_eq!(recovery.kinds(), vec!["account", "list", "retrieve"]);
    assert_eq!(f.counts().await, (1, 0));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn worker_absent_readback_replays_exact_request_only_within_existing_lifetime() {
    let f = Fixture::new().await;
    let plan = f.prepared(0).await;
    let first = Fake::new(vec![
        Ok(merchant()),
        Err(ManagedCheckoutExecutionError::Transport),
    ]);
    assert!(run(&f.store, &policy(), &first, plan.order_id(), true)
        .await
        .is_err());
    f.due().await;
    let second = Fake::new(vec![
        Ok(merchant()),
        Ok(page(vec![], false)),
        Ok(session(&plan, "cs_test_replayed")),
    ]);
    run(&f.store, &policy(), &second, plan.order_id(), true)
        .await
        .unwrap();
    assert_eq!(
        first.calls.lock().unwrap()[1].key,
        second.calls.lock().unwrap()[2].key
    );
    assert_eq!(
        first.calls.lock().unwrap()[1].body,
        second.calls.lock().unwrap()[2].body
    );
    assert_eq!(f.counts().await, (2, 0));
    let expired = f.prepared(4000).await;
    let old = Fake::new(vec![Ok(merchant())]);
    assert!(matches!(
        run(&f.store, &policy(), &old, expired.order_id(), true).await,
        Err(ManagedCheckoutExecutionError::ReconciliationRequired)
    ));
    assert_eq!(old.kinds(), vec!["account"]);
    assert_eq!(f.counts().await, (2, 0));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn worker_wrong_merchant_permissions_and_disabled_intake_never_create() {
    let f = Fixture::new().await;
    let plan = f.prepared(0).await;
    let mut wrong = merchant();
    wrong["id"] = "acct_wrong".into();
    for reply in [Ok(wrong), Err(ManagedCheckoutExecutionError::Http(403))] {
        let provider = Fake::new(vec![reply, Ok(session(&plan, "cs_test_must_not_create"))]);
        assert!(run(&f.store, &policy(), &provider, plan.order_id(), true)
            .await
            .is_err());
        assert_eq!(provider.kinds(), vec!["account"]);
        f.due().await;
    }
    let provider = Fake::new(vec![
        Ok(merchant()),
        Ok(session(&plan, "cs_test_intake_disabled")),
    ]);
    assert!(matches!(
        run(&f.store, &policy(), &provider, plan.order_id(), false).await,
        Err(ManagedCheckoutExecutionError::ReconciliationRequired)
    ));
    assert_eq!(f.counts().await, (0, 0));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn sandbox_checkout_accepts_inactive_capabilities_but_live_creation_requires_both() {
    let f = Fixture::new().await;
    let sandbox = f.prepared(0).await;
    let mut inactive = merchant();
    inactive["charges_enabled"] = false.into();
    inactive["payouts_enabled"] = false.into();
    let provider = Fake::new(vec![
        Ok(inactive),
        Ok(session(&sandbox, "cs_test_inactive")),
    ]);
    assert!(matches!(
        run(&f.store, &policy(), &provider, sandbox.order_id(), true)
            .await
            .unwrap(),
        ManagedCheckoutExecution::Bound { .. }
    ));
    assert_eq!(provider.kinds(), vec!["account", "create"]);
    assert_eq!(f.counts().await, (1, 0));

    let live = f.prepared_mode(0, true).await;
    for field in ["charges_enabled", "payouts_enabled"] {
        for value in [json!(false), Value::Null, json!("true")] {
            let mut account = merchant();
            account[field] = value;
            let provider = Fake::new(vec![Ok(account)]);
            assert!(matches!(
                run(&f.store, live.policy(), &provider, live.order_id(), true).await,
                Err(ManagedCheckoutExecutionError::Configuration)
            ));
            assert_eq!(provider.kinds(), vec!["account"]);
            assert_eq!(f.counts().await, (1, 0));
            f.due().await;
        }
    }
    let mut live_response = session(&live, "cs_live_ready");
    live_response["livemode"] = true.into();
    let ready = Fake::new(vec![Ok(merchant()), Ok(live_response)]);
    assert!(matches!(
        run(&f.store, live.policy(), &ready, live.order_id(), true)
            .await
            .unwrap(),
        ManagedCheckoutExecution::Bound { .. }
    ));
    assert_eq!(ready.kinds(), vec!["account", "create"]);
    assert_eq!(f.counts().await, (2, 0));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn worker_bad_create_response_remains_unbound_and_recovery_cannot_switch_session() {
    let f = Fixture::new().await;
    let plan = f.prepared(0).await;
    let mut bad = session(&plan, "cs_test_invalid");
    bad["livemode"] = true.into();
    let provider = Fake::new(vec![Ok(merchant()), Ok(bad)]);
    assert!(matches!(
        run(&f.store, &policy(), &provider, plan.order_id(), true).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
    f.due().await;
    let recovery = Fake::new(vec![
        Ok(merchant()),
        Ok(page(vec![session(&plan, "cs_test_candidate")], false)),
        Ok(session(&plan, "cs_test_other")),
    ]);
    assert!(matches!(
        run(&f.store, &policy(), &recovery, plan.order_id(), false).await,
        Err(ManagedCheckoutExecutionError::Response)
    ));
    let bound: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM managed_stripe_checkout_operations WHERE order_id=$1",
    )
    .bind(plan.order_id())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(bound, None);
    assert_eq!(f.counts().await, (1, 0));
    f.cleanup().await;
}

fn managed_orders_bounty(funding_deadline: i64) -> managed_orders::BountyCommitment {
    managed_orders::BountyCommitment::test_fixture(false, funding_deadline)
}
