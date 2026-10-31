//! Fault injection through the real transport, without a public test hook or
//! mutable process configuration shared with other tests.
use super::*;
use chain_base::{JsonRpcTransport, ReqwestJsonRpcTransport};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const FIXTURE: &str = "AGENT_BOUNTIES_TRANSPORT_DIAGNOSTIC_FIXTURE";
const TEST: &str = "http_observability::diagnostic_tests::actual_transport_failure_classes";

#[tokio::test]
async fn actual_transport_failure_classes() {
    if let Ok(case) = std::env::var(FIXTURE) {
        match case.as_str() {
            "paced" => paced_and_network_failures().await,
            "invalid" => invalid_local_configuration().await,
            _ => panic!("unknown diagnostic fixture"),
        }
        return;
    }
    for (case, setting) in [("paced", "1000"), ("invalid", "private-marker")] {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([TEST, "--exact", "--nocapture"])
            .env(FIXTURE, case)
            .env("BASE_RPC_READ_WIRE_INTERVAL_MS", setting)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while child.try_wait().unwrap().is_none() {
            if started.elapsed() > Duration::from_secs(15) {
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
    }
}

struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(respond: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/private-marker", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let (offset, length) = loop {
                    let size = stream.read(&mut buffer).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&buffer[..size]);
                    assert!(bytes.len() < 64 * 1024);
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
                    let size = stream.read(&mut buffer).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&buffer[..size]);
                }
                let request: Value =
                    serde_json::from_slice(&bytes[offset..offset + length]).unwrap();
                assert_eq!(request["method"], "eth_chainId");
                observed.fetch_add(1, Ordering::SeqCst);
                if !respond {
                    std::future::pending::<()>().await;
                }
                let body =
                    json!({"jsonrpc":"2.0", "id":request["id"], "result":"0x2105"}).to_string();
                let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                stream.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
}

fn request() -> Value {
    json!({"jsonrpc":"2.0", "id":1, "method":"eth_chainId", "params":[]})
}

fn checked_class(error: &chain_base::ChainBaseError) -> FailureClass {
    let class = chain_failure_class(error);
    let event = classified_failure_event(FailureStage::HeadChainIdRead, class, "local");
    assert!(!event.to_string().contains("private-marker"));
    assert!(!event.to_string().contains("127.0.0.1"));
    class
}

async fn paced_and_network_failures() {
    let server = Server::start(true).await;
    let transport = ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(5)).unwrap();
    let mut tasks = Vec::new();
    let started = Instant::now();
    // Exceed the actual 64-waiter cap. Every error must remain local; no
    // fabricated HTTP 429 and no dispatch or background retry after denial.
    for _ in 0..72 {
        let transport = transport.clone();
        let url = server.url.clone();
        tasks.push(tokio::spawn(async move {
            transport.post_json_value(&url, &request()).await
        }));
    }
    let (mut success, mut waits, mut capacity) = (0, 0, 0);
    for task in tasks {
        match task.await.unwrap() {
            Ok(value) => {
                assert_eq!(value["result"], "0x2105");
                success += 1;
            }
            Err(error) => match checked_class(&error) {
                FailureClass::LocalPacingTimeout => waits += 1,
                FailureClass::LocalPacingCapacity => capacity += 1,
                other => panic!("unexpected local failure: {other:?}"),
            },
        }
    }
    assert!(waits > 0 && capacity > 0);
    assert_eq!(success + waits + capacity, 72);
    assert_eq!(server.requests.load(Ordering::SeqCst), success);
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(4));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(server.requests.load(Ordering::SeqCst), success);

    let stalled = Server::start(false).await;
    let short = ReqwestJsonRpcTransport::without_redirects(Duration::from_millis(200)).unwrap();
    let error = short
        .post_json_value(&stalled.url, &request())
        .await
        .unwrap_err();
    assert_eq!(checked_class(&error), FailureClass::TransportTimeout);
    assert_eq!(stalled.requests.load(Ordering::SeqCst), 1);

    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/private-marker", closed.local_addr().unwrap());
    drop(closed);
    let error = short.post_json_value(&url, &request()).await.unwrap_err();
    assert_eq!(checked_class(&error), FailureClass::ConnectionFailed);
}

async fn invalid_local_configuration() {
    let server = Server::start(true).await;
    let transport = ReqwestJsonRpcTransport::default();
    let error = transport
        .post_json_value(&server.url, &request())
        .await
        .unwrap_err();
    assert_eq!(
        checked_class(&error),
        FailureClass::LocalPacingConfiguration
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}
