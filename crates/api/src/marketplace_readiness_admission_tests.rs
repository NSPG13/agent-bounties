//! Real loopback HTTP and process-local pacing. The baseline uses the same
//! head-validation code with the original default two-second admission policy.
use super::*;
use chain_base::{JsonRpcTransport, ReqwestJsonRpcTransport};
use std::sync::Mutex as StdMutex;
use tokio::time::Instant;

const FIXTURE: &str = "AGENT_BOUNTIES_HEAD_ADMISSION_FIXTURE";
const TEST: &str =
    "marketplace_readiness::admission_tests::head_admission_uses_original_observation_deadline";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn head_admission_uses_original_observation_deadline() {
    if let Ok(case) = std::env::var(FIXTURE) {
        assert_eq!(
            std::env::var("BASE_RPC_READ_WIRE_INTERVAL_MS").unwrap(),
            "250"
        );
        match case.as_str() {
            "matrix" => matrix().await,
            "burst" => latest_number_burst().await,
            "deadline" => expired_and_slow_head_deadlines().await,
            _ => panic!("unknown local fixture"),
        }
        return;
    }
    // The production setting is read once. Isolated child processes avoid
    // changing the environment of other tests or inventing a public test hook.
    for case in ["matrix", "burst", "deadline"] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([TEST, "--exact", "--nocapture"])
            .env(FIXTURE, case)
            .env("BASE_RPC_READ_WIRE_INTERVAL_MS", "250")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(150) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("{case}: fixture exceeded its outer safety deadline");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{case}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if case == "burst" {
            let events: Vec<Value> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| event["event"] == "service_failure_stage")
                .collect();
            assert_eq!(events.len(), 1, "{events:?}");
            assert_eq!(events[0]["stage"], "head_latest_number_read");
            assert_eq!(events[0]["failure_class"], "local_pacing_timeout");
        }
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }
}

struct Fixture {
    url: String,
    seen: Arc<StdMutex<Vec<(Instant, u64)>>>,
    safe_received: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn start(latency: Duration) -> Self {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let safe_received = Arc::new(tokio::sync::Notify::new());
        let records = seen.clone();
        let safe = safe_received.clone();
        let app = Router::new().route(
            "/",
            post(move |Json(request): Json<Value>| {
                let records = records.clone();
                let safe = safe.clone();
                async move {
                    // Every request is scalar. HTTP and method counts are equal.
                    assert!(request.is_object());
                    let id = request["id"].as_u64().unwrap();
                    records.lock().unwrap().push((Instant::now(), id));
                    if id == 93 {
                        safe.notify_one();
                    }
                    let result = match request["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x2105"),
                        "eth_blockNumber" => json!("0x3e8"),
                        "eth_getBlockByNumber" => {
                            let number = if request["params"][0] == "safe" {
                                "0x3e6"
                            } else {
                                assert_eq!(request["params"], json!(["0x3e8", false]));
                                "0x3e8"
                            };
                            json!({"number":number,"hash":format!("0x{}","ab".repeat(32)),
                            "timestamp":format!("0x{:x}",Utc::now().timestamp())})
                        }
                        _ => panic!("unexpected method in read-only fixture"),
                    };
                    tokio::time::sleep(latency).await;
                    Json(json!({"jsonrpc":"2.0","id":id,"result":result}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            seen,
            safe_received,
            task,
        }
    }
}

async fn observe(url: &str, fixed: bool, deadline: Instant) -> Option<(u64, i64, u64)> {
    tokio::time::timeout_at(deadline, async {
        if fixed {
            observed_heads_for_chain(url, 8453, deadline).await
        } else {
            observed_heads_with_transport(url, 8453, &ReqwestJsonRpcTransport::default()).await
        }
    })
    .await
    .ok()
    .flatten()
}

async fn matrix_case(clients: usize, latency_ms: u64, fixed: bool) -> usize {
    let fixture = Fixture::start(Duration::from_millis(latency_ms)).await;
    // TLS/client setup is synchronous and is not the admission mechanism being
    // compared. Give both policies identically prepared, independent clients.
    let transports: Vec<_> = (0..clients)
        .map(|_| ReqwestJsonRpcTransport::default())
        .collect();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(12);
    let mut jobs = Vec::new();
    for transport in transports {
        let url = fixture.url.clone();
        let transport = if fixed {
            transport.with_read_observation_deadline(deadline)
        } else {
            transport
        };
        jobs.push(tokio::spawn(async move {
            tokio::time::timeout_at(
                deadline,
                observed_heads_with_transport(&url, 8453, &transport),
            )
            .await
            .ok()
            .flatten()
        }));
    }
    let mut passed = 0;
    for job in jobs {
        if let Some((safe, _, latest)) = job.await.unwrap() {
            assert_eq!((safe, latest), (998, 1000));
            passed += 1;
        }
    }
    let seen = fixture.seen.lock().unwrap();
    assert!(started.elapsed() < Duration::from_secs(12));
    assert!(seen.iter().all(|(_, id)| (92..=95).contains(id)));
    assert!(seen.len() <= clients * 4, "no retry or duplicate head read");
    // Server receive times may bunch after executor/network stalls. Admission
    // spacing is independently asserted from the pacer's own reservation clock.
    let min_receive_gap = seen
        .windows(2)
        .map(|pair| pair[1].0.duration_since(pair[0].0).as_millis())
        .min();
    if fixed {
        assert_eq!(passed, clients);
        assert_eq!(seen.len(), clients * 4);
        for id in 92..=95 {
            assert_eq!(
                seen.iter().filter(|(_, seen_id)| *seen_id == id).count(),
                clients
            );
        }
    }
    println!("matrix fixed={fixed} clients={clients} latency_ms={latency_ms} passed={passed} HTTP={} RPC_methods={} elapsed_ms={} min_receive_gap_ms={min_receive_gap:?}", seen.len(), seen.len(), started.elapsed().as_millis());
    passed
}

async fn matrix() {
    let parallel = Arc::new(tokio::sync::Semaphore::new(2));
    for latency in [50, 150, 300] {
        // Independent endpoints can run concurrently; capacity is per endpoint.
        let mut jobs = Vec::new();
        for clients in [1, 3, 4, 8, 9] {
            let parallel = parallel.clone();
            jobs.push(tokio::spawn(async move {
                let _permit = parallel.acquire_owned().await.unwrap();
                let old = matrix_case(clients, latency, false).await;
                let new = matrix_case(clients, latency, true).await;
                assert!(new >= old, "admission acceptance must not regress");
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
    }
}

async fn latest_number_burst() {
    for fixed in [false, true] {
        let fixture = Fixture::start(Duration::from_millis(150)).await;
        let head = ReqwestJsonRpcTransport::default();
        let siblings: Vec<_> = (100..108)
            .map(|id| (id, ReqwestJsonRpcTransport::default()))
            .collect();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(12);
        let head = if fixed {
            head.with_read_observation_deadline(deadline)
        } else {
            head
        };
        let observation = async {
            tokio::time::timeout_at(
                deadline,
                observed_heads_with_transport(&fixture.url, 8453, &head),
            )
            .await
            .ok()
            .flatten()
        };
        let burst =
            async {
                fixture.safe_received.notified().await;
                // The safe read is already on the wire. Eight sibling scalar reads
                // join before its response, so the next head read waits >2 seconds.
                tokio::time::sleep(Duration::from_millis(50)).await;
                let mut jobs = Vec::new();
                for (id, transport) in siblings {
                    let url = fixture.url.clone();
                    jobs.push(tokio::spawn(async move {
                        transport.post_json_value(&url,
                        &json!({"jsonrpc":"2.0","id":id,"method":"eth_chainId","params":[]}))
                        .await.unwrap();
                    }));
                }
                for job in jobs {
                    job.await.unwrap();
                }
            };
        let (result, ()) = tokio::join!(observation, burst);
        assert_eq!(result.is_some(), fixed);
        let seen = fixture.seen.lock().unwrap();
        let count = if fixed { 12 } else { 10 };
        assert_eq!(seen.len(), count);
        assert_eq!(seen.iter().filter(|(_, id)| *id >= 100).count(), 8);
        assert_eq!(seen.iter().any(|(_, id)| *id == 94), fixed);
        assert_eq!(seen.iter().any(|(_, id)| *id == 95), fixed);
        println!(
            "burst fixed={fixed} passed={} HTTP={} RPC_methods={} elapsed_ms={}",
            result.is_some(),
            count,
            count,
            started.elapsed().as_millis()
        );
    }
}

async fn expired_and_slow_head_deadlines() {
    let fixture = Fixture::start(Duration::from_secs(30)).await;
    assert!(observe(&fixture.url, true, Instant::now()).await.is_none());
    assert!(fixture.seen.lock().unwrap().is_empty());
    let started = Instant::now();
    let deadline = started + Duration::from_millis(300);
    assert!(observe(&fixture.url, true, deadline).await.is_none());
    assert!(started.elapsed() < Duration::from_millis(600));
    assert_eq!(fixture.seen.lock().unwrap().len(), 1);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        fixture.seen.lock().unwrap().len(),
        1,
        "no detached late RPC or retry"
    );
}
