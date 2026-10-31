use super::*;
use crate::{JsonRpcTransport, ReqwestJsonRpcTransport};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<(Instant, Value)>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(statuses: Vec<(u16, Duration, Option<u64>)>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let records = seen.clone();
        let task = tokio::spawn(async move {
            let mut index = 0;
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let (offset, length) = loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse::<usize>()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < offset + length {
                    let n = stream.read(&mut buffer).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let request: Value =
                    serde_json::from_slice(&bytes[offset..offset + length]).unwrap();
                records
                    .lock()
                    .unwrap()
                    .push((Instant::now(), request.clone()));
                let (status, delay, retry) =
                    statuses
                        .get(index)
                        .copied()
                        .unwrap_or((200, Duration::ZERO, None));
                index += 1;
                tokio::time::sleep(delay).await;
                let response = |r: &Value| json!({"jsonrpc":"2.0","id":r["id"],"result":"0x2105"});
                let body = request
                    .as_array()
                    .map_or_else(
                        || response(&request),
                        |rs| Value::Array(rs.iter().map(response).collect()),
                    )
                    .to_string();
                let retry = retry.map_or(String::new(), |n| format!("Retry-After: {n}\r\n"));
                let reply = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\n{retry}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        Self { url, seen, task }
    }
    async fn received(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(3), async {
            while self.seen.lock().unwrap().len() < count {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
}
fn transport(pacer: Pacer) -> ReqwestJsonRpcTransport {
    ReqwestJsonRpcTransport {
        client: reqwest::Client::new(),
        read_pacing: pacer,
    }
}
fn read(id: u64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"eth_chainId","params":[]})
}
fn enabled(ms: u64) -> Pacer {
    Pacer::parse(Some(&ms.to_string()))
}
fn interval_assert(server: &Server, count: usize, minimum_ms: u64) {
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), count);
    for pair in seen.windows(2) {
        assert!(
            pair[1].0.duration_since(pair[0].0) >= Duration::from_millis(minimum_ms),
            "arrival delta {:?}; records {:?}",
            pair[1].0.duration_since(pair[0].0),
            *seen
        );
    }
}

#[test]
fn configuration_is_explicit_bounded_and_does_not_retain_values() {
    for value in [None, Some("0")] {
        assert!(matches!(Pacer::parse(value), Pacer::Disabled));
    }
    for value in ["10", "100", "1000"] {
        assert!(matches!(Pacer::parse(Some(value)), Pacer::Enabled { .. }));
    }
    for value in [
        "",
        "1",
        "9",
        "1001",
        "-1",
        " 100",
        "1.5",
        "secret-marker",
        "9999999999999999999999999",
    ] {
        let config = Pacer::parse(Some(value));
        assert!(matches!(config, Pacer::Invalid));
        assert!(!format!("{config:?}").contains(value) || value.is_empty());
    }
    let a = Pacer::process();
    let b = Pacer::process();
    if let (Pacer::Enabled { registry: a, .. }, Pacer::Enabled { registry: b, .. }) = (a, b) {
        assert!(Arc::ptr_eq(&a, &b));
    }
}

#[tokio::test]
async fn actual_wire_sibling_transports_share_spacing_across_recovery_passes() {
    let server = Server::start(vec![]).await;
    let pacer = enabled(60);
    let first = transport(pacer.clone());
    let sibling = transport(pacer.clone());
    let final_check = transport(pacer);
    // Warm HTTP/client setup before measuring steady-state arrivals. The guard
    // bounds local starts, not variable first-connection or network latency.
    for (index, client) in [&first, &sibling, &final_check].into_iter().enumerate() {
        client
            .post_json_value(&server.url, &read(100 + index as u64))
            .await
            .unwrap();
    }
    server.seen.lock().unwrap().clear();
    let job = async {
        for id in 0..4 {
            first.post_json_value(&server.url, &read(id)).await.unwrap();
        }
        // Reconstructing the transport/final check does not reset the endpoint gate.
        for id in 4..8 {
            final_check
                .post_json_value(&server.url, &read(id))
                .await
                .unwrap();
        }
    };
    let other = async {
        for id in 8..12 {
            sibling
                .post_json_value(&server.url, &read(id))
                .await
                .unwrap();
        }
    };
    tokio::join!(job, other);
    interval_assert(&server, 12, 45);
}

#[tokio::test]
async fn actual_wire_separate_process_registries_do_not_share_capacity() {
    let server = Server::start(vec![]).await;
    transport(enabled(500))
        .post_json_value(&server.url, &read(1))
        .await
        .unwrap();
    transport(enabled(500))
        .post_json_value(&server.url, &read(2))
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].0.duration_since(seen[0].0) < Duration::from_millis(400));
}

#[tokio::test]
async fn actual_wire_waiter_rechecks_new_negative_hold_before_dispatch() {
    let server = Server::start(vec![(429, Duration::from_millis(80), Some(120))]).await;
    let pacer = enabled(250);
    let a = transport(pacer.clone());
    let b = transport(pacer);
    let first_request = read(1);
    let first = a.post_json_value(&server.url, &first_request);
    let second = async {
        server.received(1).await;
        b.post_json_value(&server.url, &read(2)).await
    };
    let (one, two) = tokio::join!(first, second);
    assert!(matches!(
        one,
        Err(ChainBaseError::RpcRetryAfter {
            status: 429,
            retry_after_seconds: 120
        })
    ));
    assert!(matches!(
        two,
        Err(ChainBaseError::RpcRetryAfter {
            status: 429,
            retry_after_seconds: 119..=120
        })
    ));
    assert_eq!(
        server.seen.lock().unwrap().len(),
        1,
        "waiting sibling must never reach wire after new hold"
    );
}

#[tokio::test]
async fn actual_wire_read_retries_are_paced_and_keep_existing_attempt_bound() {
    let server = Server::start(vec![(503, Duration::ZERO, None); 3]).await;
    let result = transport(enabled(40))
        .post_json_value(&server.url, &read(1))
        .await;
    assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(503))));
    interval_assert(&server, 3, 900); // Existing1s/2s retry waits retained.
}

#[tokio::test]
async fn actual_wire_mutations_mixed_batches_and_unknown_methods_bypass_invalid_read_config() {
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"eth_sendRawTransaction","params":["0x00"]}),
        json!({"jsonrpc":"2.0","id":1,"method":"unknown_write","params":[]}),
    ] {
        let server = Server::start(vec![(503, Duration::ZERO, None)]).await;
        let result = transport(Pacer::Invalid)
            .post_json_value(&server.url, &request)
            .await;
        assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(503))));
        assert_eq!(server.seen.lock().unwrap().len(), 1);
    }
    let server = Server::start(vec![(503, Duration::ZERO, None)]).await;
    let result = transport(Pacer::Invalid)
        .post_json_values(
            &server.url,
            &[
                read(1),
                json!({"jsonrpc":"2.0","id":2,"method":"eth_sendRawTransaction","params":["0x00"]}),
            ],
        )
        .await;
    assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(503))));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
    let server = Server::start(vec![]).await;
    let error = transport(Pacer::Invalid)
        .post_json_value(&server.url, &read(1))
        .await
        .unwrap_err();
    assert!(matches!(error, ChainBaseError::RpcTransport(_)));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn actual_wire_mutation_is_not_delayed_by_a_waiting_read() {
    let server = Server::start(vec![]).await;
    let pacer = enabled(500);
    let t = transport(pacer.clone());
    t.post_json_value(&server.url, &read(1)).await.unwrap();
    let other = transport(pacer);
    let pending_request = read(2);
    let pending = other.post_json_value(&server.url, &pending_request);
    let mutation = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.post_json_value(
            &server.url,
            &json!({"jsonrpc":"2.0","id":3,"method":"eth_sendRawTransaction","params":["0x00"]}),
        )
        .await
        .unwrap();
    };
    let (result, ()) = tokio::join!(pending, mutation);
    result.unwrap();
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1].1["id"], 3);
    assert_eq!(seen[2].1["id"], 2);
    assert!(seen[1].0.duration_since(seen[0].0) < Duration::from_millis(400));
}

#[tokio::test]
async fn actual_wire_cancellation_does_not_leave_reserved_future_slots() {
    let server = Server::start(vec![]).await;
    let p = enabled(300);
    let t = transport(p.clone());
    t.post_json_value(&server.url, &read(1)).await.unwrap();
    let canceled = tokio::time::timeout(
        Duration::from_millis(20),
        t.post_json_value(&server.url, &read(2)),
    )
    .await;
    assert!(canceled.is_err());
    transport(p)
        .post_json_value(&server.url, &read(3))
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].1["id"], 3);
    assert!(seen[1].0.duration_since(seen[0].0) < Duration::from_millis(550));
}

#[tokio::test]
async fn actual_wire_batch_is_one_attempt_not_a_logical_read_or_compute_unit_budget() {
    let server = Server::start(vec![]).await;
    let t = transport(enabled(100));
    t.post_json_value(&server.url, &read(100)).await.unwrap();
    server.seen.lock().unwrap().clear();
    t.post_json_values(&server.url, &[read(1), read(2), read(3)])
        .await
        .unwrap();
    t.post_json_value(&server.url, &read(4)).await.unwrap();
    interval_assert(&server, 2, 80);
    assert_eq!(
        server.seen.lock().unwrap()[0].1.as_array().unwrap().len(),
        3
    );
}

#[tokio::test]
async fn pacing_capacity_cancellation_and_deadlines_fail_closed_without_background_work() {
    let p = enabled(1000);
    p.wait("fixture").await.unwrap();
    let Pacer::Enabled { registry, .. } = &p else {
        panic!()
    };
    let key: Key = Sha256::digest(b"fixture").into();
    let entry = registry.lock().unwrap().entries[&key].clone();
    let permits = entry
        .callers
        .clone()
        .try_acquire_many_owned(MAX_WAITERS as u32)
        .unwrap();
    assert!(p
        .wait("fixture")
        .await
        .unwrap_err()
        .to_string()
        .contains("caller capacity"));
    drop(permits);
    let lock = entry.next.lock().await;
    assert!(p
        .wait("fixture")
        .await
        .unwrap_err()
        .to_string()
        .contains("wait deadline"));
    drop(lock);
    assert_eq!(entry.callers.available_permits(), MAX_WAITERS);
    p.wait("fixture").await.unwrap();
    let mut held = Vec::new();
    {
        let mut state = registry.lock().unwrap();
        state.entries.clear();
        for n in 0..MAX_ENDPOINTS {
            let entry = Arc::new(Entry {
                next: Arc::new(AsyncMutex::new(Instant::now() + Duration::from_secs(5))),
                callers: Arc::new(Semaphore::new(MAX_WAITERS)),
            });
            state
                .entries
                .insert(Sha256::digest(n.to_le_bytes()).into(), entry.clone());
            held.push(entry);
        }
    }
    assert!(p
        .wait("overflow")
        .await
        .unwrap_err()
        .to_string()
        .contains("endpoint capacity"));
    assert_eq!(registry.lock().unwrap().entries.len(), MAX_ENDPOINTS);
    drop(held);
    // Idle elapsed entries may be reclaimed; active/recent entries above were not.
    for value in registry.lock().unwrap().entries.values() {
        *value.next.try_lock().unwrap() = Instant::now();
    }
    p.wait("new-endpoint").await.unwrap();
    assert_eq!(registry.lock().unwrap().entries.len(), 1);
}

#[tokio::test]
async fn independent_endpoints_and_disabled_mode_do_not_share_delay() {
    let p = enabled(1000);
    p.wait("first").await.unwrap();
    tokio::time::timeout(Duration::from_millis(50), p.wait("second"))
        .await
        .unwrap()
        .unwrap();
    for _ in 0..100 {
        tokio::time::timeout(Duration::from_millis(50), Pacer::Disabled.wait("first"))
            .await
            .unwrap()
            .unwrap();
    }
}
