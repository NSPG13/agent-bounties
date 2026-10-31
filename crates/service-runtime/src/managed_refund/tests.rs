use super::*;
use crate::managed_checkout::tests::Fixture;
use managed_orders::ManagedQuote;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};
struct Fake {
    responses: Mutex<VecDeque<Result<Value>>>,
    calls: Mutex<Vec<Operation>>,
}
impl Fake {
    fn new(responses: Vec<Result<Value>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            calls: Mutex::new(vec![]),
        }
    }
}
#[async_trait]
impl Transport for Fake {
    async fn send(&self, op: Operation) -> Result<Value> {
        self.calls.lock().unwrap().push(op);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected refund HTTP request")
    }
}
fn fixture() -> ManagedRefundPlan {
    let v: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let q: ManagedQuote = serde_json::from_value(v["quote"].clone()).unwrap();
    ManagedRefundPlan::new(
        &q,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "pi_fixture".into(),
        q.input().expires_at + 1,
    )
    .unwrap()
}
fn refund(p: &ManagedRefundPlan) -> Value {
    json!({"id":"re_fixture","object":"refund","charge":"ch_fixture",
    "payment_intent":p.payment_intent_id(),"amount":p.amount_minor(),"currency":"mxn","status":"succeeded","created":p.created_at(),
    "metadata":p.request(p.created_at(),p.merchant()).unwrap().body["metadata"]})
}
fn page(data: Vec<Value>, more: bool) -> Value {
    json!({"object":"list","data":data,"has_more":more})
}
fn account(p: &ManagedRefundPlan) -> Value {
    json!({"object":"account","id":p.merchant().stripe_account_id,"charges_enabled":false,"payouts_enabled":false})
}

#[tokio::test]
async fn discovery_requires_complete_exact_original_payment_history() {
    let p = fixture();
    let t = Fake::new(vec![Ok(page(vec![], false))]);
    assert_eq!(discover(&t, &p).await.unwrap(), None);
    let t = Fake::new(vec![Ok(page(vec![refund(&p)], false))]);
    assert_eq!(discover(&t, &p).await.unwrap(), Some("re_fixture".into()));
    let mut other = refund(&p);
    other["metadata"] = json!({});
    let t = Fake::new(vec![Ok(page(vec![other], false))]);
    assert!(matches!(
        discover(&t, &p).await,
        Err(ManagedRefundExecutionError::ReconciliationRequired)
    ));
    let t = Fake::new(vec![Ok(page(vec![], true))]);
    assert!(matches!(
        discover(&t, &p).await,
        Err(ManagedRefundExecutionError::Response)
    ));
    let mut other = refund(&p);
    other["id"] = json!("re_second");
    let t = Fake::new(vec![Ok(page(vec![refund(&p), other], false))]);
    assert!(matches!(
        discover(&t, &p).await,
        Err(ManagedRefundExecutionError::ReconciliationRequired)
    ));
    let mut wrong = refund(&p);
    wrong["payment_intent"] = json!("pi_another");
    let t = Fake::new(vec![Ok(page(vec![wrong], false))]);
    assert!(matches!(
        discover(&t, &p).await,
        Err(ManagedRefundExecutionError::Response)
    ));
}

#[test]
fn restricted_keys_are_mode_checked_without_exposing_them() {
    let merchant = RefundMerchant {
        stripe_account_id: "acct_fixture".into(),
        livemode: false,
    };
    let client = ManagedRefundClient::new("rk_test_fixture", merchant.clone()).unwrap();
    assert_eq!(client.transport.base, "https://api.stripe.com");
    assert!(client.transport.authorization.is_sensitive());
    assert!(ManagedRefundClient::new("rk_live_fixture", merchant).is_err());
}

async fn prepared(f: &Fixture) -> ManagedRefundPlan {
    let checkout = f.prepared(0).await;
    let order_id = checkout.order_id();
    // Seed a previously reconciled payment for worker-only fixtures. The database suite
    // separately proves this receipt can only be produced by native signed Checkout events.
    let session = format!("cs_test_{}", order_id.simple());
    let pi = format!("pi_{}", order_id.simple());
    sqlx::query("UPDATE managed_stripe_checkout_operations SET session_id=$2,payment_intent_id=$3 WHERE order_id=$1")
        .bind(order_id).bind(&session).bind(&pi).execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE managed_service_actions SET state='completed' WHERE order_id=$1")
        .bind(order_id)
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO managed_stripe_paid_receipts(order_id,stripe_account_id,livemode,payment_intent_id,session_id,quote_digest,amount_minor,currency)
        VALUES($1,$2,false,$3,$4,$5,225000,'mxn')")
        .bind(order_id).bind(&checkout.policy().stripe_account_id).bind(&pi).bind(&session).bind(checkout.quote_digest()).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO managed_stripe_bank_actions(order_id,action,state) VALUES($1,'reconcile_bank_cash','held')").bind(order_id).execute(&f.pool).await.unwrap();
    f.store
        .request_managed_refund("account:runtime_fixture", order_id)
        .await
        .unwrap();
    f.store
        .get_managed_refund_plan("account:runtime_fixture", order_id)
        .await
        .unwrap()
        .unwrap()
}
async fn due(f: &Fixture) {
    sqlx::query(
        "UPDATE managed_refund_execution SET next_attempt_at=clock_timestamp()-interval '1 second'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn execution_persists_attempt_and_does_not_treat_api_success_as_refund_sent() {
    let f = Fixture::new().await;
    let p = prepared(&f).await;
    let t = Fake::new(vec![
        Ok(account(&p)),
        Ok(page(vec![], false)),
        Ok(refund(&p)),
    ]);
    assert_eq!(
        run(
            &f.store,
            &t,
            p.merchant(),
            p.order_id(),
            RefundKind::Full,
            true
        )
        .await
        .unwrap(),
        ManagedRefundExecution::Bound {
            refund_id: "re_fixture".into()
        }
    );
    let order = f
        .store
        .get_managed_service_order("account:runtime_fixture", p.order_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        order.status,
        db::managed_service_orders::ManagedOrderStatus::RefundPending
    );
    let attempts: i64 = sqlx::query_scalar("SELECT create_attempts FROM managed_refund_execution")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    {
        let calls = t.calls.lock().unwrap();
        let Operation::Create(r) = &calls[2] else {
            panic!("create missing")
        };
        assert_eq!(
            r.body,
            p.request(p.created_at(), p.merchant()).unwrap().body
        );
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn lost_create_response_recovers_by_list_and_get_without_second_post() {
    let f = Fixture::new().await;
    let p = prepared(&f).await;
    let t = Fake::new(vec![
        Ok(account(&p)),
        Ok(page(vec![], false)),
        Err(ManagedRefundExecutionError::Transport),
    ]);
    assert!(run(
        &f.store,
        &t,
        p.merchant(),
        p.order_id(),
        RefundKind::Full,
        true
    )
    .await
    .is_err());
    due(&f).await;
    let t = Fake::new(vec![
        Ok(account(&p)),
        Ok(page(vec![refund(&p)], false)),
        Ok(refund(&p)),
    ]);
    run(
        &f.store,
        &t,
        p.merchant(),
        p.order_id(),
        RefundKind::Full,
        false,
    )
    .await
    .unwrap();
    assert!(!t
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|c| matches!(c, Operation::Create(_))));
    let attempts: i64 = sqlx::query_scalar("SELECT create_attempts FROM managed_refund_execution")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    due(&f).await;
    let t = Fake::new(vec![Ok(account(&p)), Ok(refund(&p))]);
    run(
        &f.store,
        &t,
        p.merchant(),
        p.order_id(),
        RefundKind::Full,
        false,
    )
    .await
    .unwrap();
    assert_eq!(t.calls.lock().unwrap().len(), 2);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn merchant_mismatch_containment_and_external_refunds_cannot_create() {
    let f = Fixture::new().await;
    let p = prepared(&f).await;
    let t = Fake::new(vec![Ok(json!({"object":"account","id":"acct_wrong"}))]);
    assert!(matches!(
        run(
            &f.store,
            &t,
            p.merchant(),
            p.order_id(),
            RefundKind::Full,
            true
        )
        .await,
        Err(ManagedRefundExecutionError::Configuration)
    ));
    due(&f).await;
    let t = Fake::new(vec![Ok(account(&p)), Ok(page(vec![], false))]);
    assert!(matches!(
        run(
            &f.store,
            &t,
            p.merchant(),
            p.order_id(),
            RefundKind::Full,
            false
        )
        .await,
        Err(ManagedRefundExecutionError::ReconciliationRequired)
    ));
    due(&f).await;
    let mut external = refund(&p);
    external["metadata"] = json!({});
    let t = Fake::new(vec![Ok(account(&p)), Ok(page(vec![external], false))]);
    assert!(matches!(
        run(
            &f.store,
            &t,
            p.merchant(),
            p.order_id(),
            RefundKind::Full,
            true
        )
        .await,
        Err(ManagedRefundExecutionError::ReconciliationRequired)
    ));
    let attempts: i64 = sqlx::query_scalar("SELECT create_attempts FROM managed_refund_execution")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 0);
    f.cleanup().await;
}

async fn http_reply(op: Operation, reply: Vec<u8>, delay: Duration) -> (Result<Value>, String) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 8192];
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|s| s.strip_prefix("content-length:"))
                    .map(|s| s.trim().parse().unwrap())
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        std::thread::sleep(delay);
        let _ = socket.write_all(&reply);
        String::from_utf8(bytes).unwrap()
    });
    // Synthetic loopback endpoint only; production client keeps the fixed TLS origin.
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
    let result = transport.send(op).await;
    (result, server.join().unwrap())
}

#[tokio::test]
async fn refund_http_wire_preserves_original_payment_form_key_and_pinned_version() {
    let p = fixture();
    let request = p.request(p.created_at(), p.merchant()).unwrap();
    let expected = stripe_form_encode(&request.body);
    let key = request.idempotency_key.clone();
    let (result, wire) = http_reply(
        Operation::Create(request),
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        Duration::ZERO,
    )
    .await;
    assert_eq!(result.unwrap(), json!({}));
    assert!(wire.starts_with("POST /v1/refunds HTTP/1.1\r\n"));
    assert!(wire.contains(&format!("stripe-version: {MANAGED_STRIPE_API_VERSION}\r\n")));
    assert!(wire.contains(&format!("idempotency-key: {key}\r\n")));
    assert!(wire.ends_with(&expected));
    assert!(!wire.contains("stripe-account:"));
    let (result, wire) = http_reply(
        Operation::List {
            payment_intent: "pi_fixture".into(),
            after: Some("re_previous".into()),
        },
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        Duration::ZERO,
    )
    .await;
    result.unwrap();
    assert!(wire.starts_with("GET /v1/refunds?limit=100&payment_intent=pi_fixture&starting_after=re_previous HTTP/1.1\r\n"));
    assert!(!wire.contains("idempotency-key:"));
}

#[tokio::test]
async fn refund_http_never_follows_redirect_or_exposes_provider_error_content() {
    let (result, _) = http_reply(Operation::Account, b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/credential-trap\r\nContent-Length: 0\r\n\r\n".to_vec(), Duration::ZERO).await;
    assert!(matches!(
        result,
        Err(ManagedRefundExecutionError::Http(302))
    ));
    let secret = "sensitive-synthetic-provider-detail";
    let (result, _) = http_reply(
        Operation::Account,
        format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Length: {}\r\n\r\n{secret}",
            secret.len()
        )
        .into_bytes(),
        Duration::ZERO,
    )
    .await;
    let error = result.unwrap_err();
    assert!(matches!(error, ManagedRefundExecutionError::Http(403)));
    assert!(!format!("{error:?} {error}").contains(secret));
}

#[tokio::test]
async fn refund_http_bounds_declared_streaming_slow_and_malformed_responses() {
    let mut stream = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    stream.extend(vec![b' '; MAX_BYTES + 1]);
    for reply in [
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            MAX_BYTES + 1
        )
        .into_bytes(),
        stream,
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nbad".to_vec(),
    ] {
        assert!(matches!(
            http_reply(Operation::Account, reply, Duration::ZERO)
                .await
                .0,
            Err(ManagedRefundExecutionError::Response)
        ));
    }
    assert!(matches!(
        http_reply(
            Operation::Account,
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
            Duration::from_millis(350)
        )
        .await
        .0,
        Err(ManagedRefundExecutionError::Transport)
    ));
}

#[tokio::test]
async fn a_status_read_accepts_only_this_plans_exact_refund() {
    let p = fixture();
    let t = Fake::new(vec![Ok(refund(&p))]);
    assert_eq!(
        observe(&t, &p, "re_fixture").await.unwrap(),
        RefundOutcome::Succeeded
    );
    let mut pending = refund(&p);
    pending["status"] = json!("requires_action");
    let t = Fake::new(vec![Ok(pending)]);
    assert_eq!(
        observe(&t, &p, "re_fixture").await.unwrap(),
        RefundOutcome::RequiresAction
    );
    for change in [
        |v: &mut Value| v["id"] = json!("re_other"),
        |v: &mut Value| v["amount"] = json!(1),
        |v: &mut Value| v["metadata"] = json!({}),
        |v: &mut Value| v["status"] = json!("reversed"),
    ] {
        let mut other = refund(&p);
        change(&mut other);
        let t = Fake::new(vec![Ok(other)]);
        assert!(matches!(
            observe(&t, &p, "re_fixture").await,
            Err(ManagedRefundExecutionError::Response)
        ));
    }
    // A malformed id never reaches Stripe.
    let t = Fake::new(vec![]);
    assert!(observe(&t, &p, "../refunds").await.is_err());
}
