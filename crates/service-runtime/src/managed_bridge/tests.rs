use super::*;
use serde_json::json;
use std::{collections::VecDeque, sync::Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn fixture(payout: bool) -> (BridgeTransferPlan, Value, BridgeExecutionPolicy) {
    let v: Value = serde_json::from_str(if payout {
        include_str!("../../../managed-orders/tests/fixtures/bridge-payout.json")
    } else {
        include_str!("../../../managed-orders/tests/fixtures/bridge-conversion.json")
    })
    .unwrap();
    let plan = BridgeTransferPlan::restore(v["plan"].clone()).unwrap();
    let policy = BridgeExecutionPolicy {
        company: plan.company().clone(),
        maximum_conversion_minor: 225000,
        maximum_payout_micro_usdc: 102000000,
        maximum_payout_minor: 200000,
    };
    (plan, v["response"].clone(), policy)
}
struct Scripted {
    responses: Mutex<VecDeque<Result<Value>>>,
    operations: Mutex<Vec<Operation>>,
}
impl Scripted {
    fn new(responses: Vec<Result<Value>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            operations: Mutex::new(vec![]),
        }
    }
}
#[async_trait]
impl Transport for Scripted {
    async fn send(&self, op: Operation) -> Result<Value> {
        self.operations.lock().unwrap().push(op);
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected external request")
    }
}
fn customer(p: &BridgeTransferPlan) -> Value {
    json!({"id":p.company().customer_id,"status":"active"})
}
fn external(p: &BridgeTransferPlan) -> Value {
    let BridgeRoute::SupplierMxnPayout {
        external_account_id,
        ..
    } = p.route()
    else {
        panic!("payout fixture required")
    };
    json!({"id":external_account_id,"customer_id":p.company().customer_id,"currency":"mxn",
        "account_type":"clabe","active":true,"account_verification":{"clabe":{"matched":true}}})
}

#[tokio::test]
async fn conversion_create_uses_company_identity_original_key_and_exact_request() {
    let (p, response, policy) = fixture(false);
    let t = Scripted::new(vec![Ok(customer(&p)), Ok(response)]);
    let o = create(&t, &policy, &p, p.created_at(), true).await.unwrap();
    assert!(!o.output_matches_commitment());
    let calls = t.operations.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(matches!(calls[0],Operation::Customer(id) if id==p.company().customer_id));
    let Operation::Create { operation, body } = &calls[1] else {
        panic!("create required")
    };
    assert_eq!(*operation, p.operation_id());
    assert_eq!(*body, p.request(p.created_at()).unwrap());
}

#[tokio::test]
async fn payout_checks_active_matching_verified_recipient_before_create() {
    let (p, response, policy) = fixture(true);
    let t = Scripted::new(vec![Ok(customer(&p)), Ok(external(&p)), Ok(response)]);
    create(&t, &policy, &p, p.created_at(), true).await.unwrap();
    assert_eq!(t.operations.lock().unwrap().len(), 3);
    for (pointer, value) in [
        ("/id", json!(Uuid::new_v4())),
        ("/customer_id", json!(Uuid::new_v4())),
        ("/currency", json!("usd")),
        ("/account_type", json!("iban")),
        ("/active", json!(false)),
        ("/account_verification/clabe/matched", Value::Null),
    ] {
        let mut account = external(&p);
        *account.pointer_mut(pointer).unwrap() = value;
        let t = Scripted::new(vec![Ok(customer(&p)), Ok(account)]);
        assert!(matches!(
            create(&t, &policy, &p, p.created_at(), true).await,
            Err(BridgeExecutionError::Ineligible)
        ));
        assert_eq!(t.operations.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn changed_verified_beneficiary_snapshot_requires_reapproval() {
    let (p, _, policy) = fixture(true);
    let mut changed = external(&p);
    // Verification remains true and the same ID/customer remain bound. A changed snapshot
    // still cannot silently redirect an approved supplier payment.
    changed["clabe"] = json!({"last_4":"9999"});
    let t = Scripted::new(vec![Ok(customer(&p)), Ok(changed)]);
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), true).await,
        Err(BridgeExecutionError::Ineligible)
    ));
    assert_eq!(t.operations.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn uncertain_create_can_only_retry_the_same_body_and_key_inside_the_window() {
    let (p, response, policy) = fixture(false);
    let t = Scripted::new(vec![
        Ok(customer(&p)),
        Err(BridgeExecutionError::Transport),
        Ok(customer(&p)),
        Ok(response),
    ]);
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), true).await,
        Err(BridgeExecutionError::Transport)
    ));
    create(&t, &policy, &p, p.created_at() + 60, true)
        .await
        .unwrap();
    let calls = t.operations.lock().unwrap();
    let (
        Operation::Create {
            operation: a,
            body: ab,
        },
        Operation::Create {
            operation: b,
            body: bb,
        },
    ) = (&calls[1], &calls[3])
    else {
        panic!("replay required")
    };
    assert_eq!(a, b);
    assert_eq!(ab, bb);
}

#[tokio::test]
async fn containment_limits_environment_and_expiry_prevent_any_external_call() {
    let (p, _, mut policy) = fixture(false);
    let t = Scripted::new(vec![]);
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), false).await,
        Err(BridgeExecutionError::Disabled)
    ));
    assert!(matches!(
        create(&t, &policy, &p, p.created_at() - 1, true).await,
        Err(BridgeExecutionError::Contract(BridgeError::Expired))
    ));
    assert!(matches!(
        create(&t, &policy, &p, p.created_at() + 23 * 3600 - 29, true).await,
        Err(BridgeExecutionError::Contract(BridgeError::Expired))
    ));
    policy.maximum_conversion_minor = 224999;
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), true).await,
        Err(BridgeExecutionError::Limit)
    ));
    policy.company.environment = BridgeEnvironment::Live;
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), true).await,
        Err(BridgeExecutionError::Configuration)
    ));
    assert!(t.operations.lock().unwrap().is_empty());
}

#[tokio::test]
async fn disabled_company_cannot_create_but_known_readback_survives_lowered_caps() {
    let (p, response, mut policy) = fixture(false);
    let mut c = customer(&p);
    c["status"] = json!("paused");
    let t = Scripted::new(vec![Ok(c)]);
    assert!(matches!(
        create(&t, &policy, &p, p.created_at(), true).await,
        Err(BridgeExecutionError::Ineligible)
    ));
    assert_eq!(t.operations.lock().unwrap().len(), 1);
    policy.maximum_conversion_minor = 0;
    let id = Uuid::parse_str(response["id"].as_str().unwrap()).unwrap();
    let t = Scripted::new(vec![Ok(response)]);
    retrieve(&t, &policy, &p, id).await.unwrap();
    assert!(matches!(t.operations.lock().unwrap()[0],Operation::Retrieve(i) if i==id));
}

#[test]
fn production_transport_pins_origin_redacts_key_and_rejects_wrong_key_mode() {
    let (_, _, policy) = fixture(false);
    let client =
        ManagedBridgeClient::new("sk-test-synthetic-fixture-only", policy.clone()).unwrap();
    assert_eq!(client.transport.base, "https://api.sandbox.bridge.xyz/v0");
    assert!(client.transport.key.is_sensitive());
    for key in [
        "sk-live-synthetic-fixture-only",
        "sk_test_wrong_provider_key",
        "sk-test-contains\nnewline",
    ] {
        assert!(ManagedBridgeClient::new(key, policy.clone()).is_err());
    }
    let mut live = policy;
    live.company.environment = BridgeEnvironment::Live;
    assert!(ManagedBridgeClient::new("sk-test-synthetic-fixture-only", live.clone()).is_err());
    let client = ManagedBridgeClient::new("sk-live-synthetic-fixture-only", live).unwrap();
    assert_eq!(client.transport.base, "https://api.bridge.xyz/v0");
}

// Loopback transport tests use a private test-only constructor, never production URL overrides.
async fn server(response: String) -> (BridgeTransport, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut all = vec![];
        let mut buf = [0; 2048];
        loop {
            let n = stream.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            all.extend_from_slice(&buf[..n]);
            if let Some(end) = all.windows(4).position(|v| v == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&all[..end]).to_ascii_lowercase();
                let size = header
                    .lines()
                    .find_map(|s| s.strip_prefix("content-length: "))
                    .map(|s| s.parse::<usize>().unwrap())
                    .unwrap_or(0);
                if all.len() >= end + 4 + size {
                    break;
                }
            }
            assert!(all.len() < MAX_RESPONSE_BYTES);
        }
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
        String::from_utf8(all).unwrap()
    });
    (
        BridgeTransport {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            key: HeaderValue::from_static("sk-test-synthetic-fixture-only"),
            base: format!("http://{address}/v0"),
        },
        task,
    )
}
fn ok(body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len())
}

#[tokio::test]
async fn real_http_transport_sends_idempotency_only_on_post() {
    let (p, _, _) = fixture(false);
    let (t, task) = server(ok("{}")).await;
    t.send(Operation::Create {
        operation: p.operation_id(),
        body: p.request(p.created_at()).unwrap(),
    })
    .await
    .unwrap();
    let wire = task.await.unwrap();
    let (headers, body) = wire.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("POST /v0/transfers HTTP/1.1"));
    assert!(headers
        .to_ascii_lowercase()
        .contains(&format!("idempotency-key: {}", p.operation_id())));
    assert!(headers
        .to_ascii_lowercase()
        .contains("api-key: sk-test-synthetic-fixture-only"));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        p.request(p.created_at()).unwrap()
    );
    let (t, task) = server(ok("{}")).await;
    t.send(Operation::Retrieve(p.operation_id())).await.unwrap();
    let wire = task.await.unwrap();
    assert!(wire.starts_with("GET /v0/transfers/"));
    assert!(!wire.to_ascii_lowercase().contains("idempotency-key"));
}

#[tokio::test]
async fn transport_rejects_redirects_oversized_and_malformed_bodies_without_logging_them() {
    for (response,kind) in [
        ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/leak\r\nContent-Length: 0\r\n\r\n".into(),302),
        (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",MAX_RESPONSE_BYTES+1),0),
        (ok("SECRET_PRIVATE_INVALID_JSON"),0),
        ("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n".into(),403),
    ] {
        let (t,task)=server(response).await;let e=t.send(Operation::Customer(Uuid::new_v4())).await.err().unwrap();
        match e {BridgeExecutionError::Http(code)=>assert_eq!(code,kind),BridgeExecutionError::Response=>assert_eq!(kind,0),_=>panic!("unexpected error")}
        assert!(!e.to_string().contains("SECRET_PRIVATE"));task.await.unwrap();
    }
}

#[tokio::test]
async fn chunked_response_limit_applies_without_content_length() {
    let payload = "x".repeat(MAX_RESPONSE_BYTES + 1);
    let wire=format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",payload.len(),payload);
    let (t, task) = server(wire).await;
    assert!(matches!(
        t.send(Operation::Customer(Uuid::new_v4())).await,
        Err(BridgeExecutionError::Response)
    ));
    task.await.unwrap();
}
