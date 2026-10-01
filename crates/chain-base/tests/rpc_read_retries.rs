use chain_base::{ChainBaseError, JsonRpcTransport, ReqwestJsonRpcTransport};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exercise(
    requests: Value,
    statuses: &[u16],
) -> (Result<Value, ChainBaseError>, Vec<Value>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let observed = Arc::new(Mutex::new(Vec::new()));
    let capture = observed.clone();
    let statuses = statuses.to_vec();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let request: Value = loop {
                let mut chunk = [0; 4096];
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
                    continue;
                };
                let length: usize = std::str::from_utf8(&bytes[..end])
                    .unwrap()
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .unwrap()
                    .1
                    .trim()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                }
            };
            let index = {
                let mut calls = capture.lock().unwrap();
                calls.push(request.clone());
                calls.len() - 1
            };
            let status = *statuses.get(index).unwrap_or(&200);
            if status == 0 {
                drop(stream);
                continue;
            }
            let reply = |r: &Value| json!({"jsonrpc":"2.0","id":r["id"],"result":"0x2105"});
            let body = if let Some(batch) = request.as_array() {
                Value::Array(batch.iter().map(reply).collect())
            } else {
                reply(&request)
            }
            .to_string();
            let body = match status {
                298 => json!({"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"temporary fixture"}}).to_string(),
                299 => "invalid JSON".to_string(),
                _ => body,
            };
            let status = if status == 298 || status == 299 {
                200
            } else {
                status
            };
            stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let transport = ReqwestJsonRpcTransport::default();
    let result = if let Some(batch) = requests.as_array() {
        transport
            .post_json_values(&url, batch)
            .await
            .map(Value::Array)
    } else {
        transport.post_json_value(&url, &requests).await
    };
    server.abort();
    let seen = observed.lock().unwrap().clone();
    (result, seen)
}

#[tokio::test]
async fn temporary_http_failures_repeat_only_the_exact_read() {
    let request = json!({"jsonrpc":"2.0","id":7,"method":"eth_call","params":[{"to":"0x1111111111111111111111111111111111111111","data":"0x1234"},{"blockHash":"0xabcdef","requireCanonical":true}]});
    let (result, seen) = exercise(request.clone(), &[429, 503, 200]).await;
    assert_eq!(result.unwrap()["result"], "0x2105");
    assert_eq!(seen, vec![request; 3]);
}

#[tokio::test]
async fn read_retry_exhaustion_is_bounded_and_never_changes_request() {
    let request = json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]});
    let (result, seen) = exercise(request.clone(), &[502, 504, 429, 200]).await;
    assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(429))));
    assert_eq!(seen, vec![request; 3]);
}

#[tokio::test]
async fn broadcasts_unknown_methods_and_mixed_batches_are_never_retried() {
    for request in [
        json!({"method":"eth_sendRawTransaction","params":["0x1234"],"id":1}),
        json!({"method":"eth_sendTransaction","params":[],"id":1}),
        json!({"method":"wallet_sendCalls","params":[],"id":1}),
        json!({"method":"eth_unrecognized","params":[],"id":1}),
        json!([{"method":"eth_chainId","id":1},{"method":"eth_sendRawTransaction","params":["0x1234"],"id":2}]),
    ] {
        for status in [429, 502, 503, 504] {
            let (result, seen) = exercise(request.clone(), &[status, 200]).await;
            assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(s)) if s == status));
            assert_eq!(seen, vec![request.clone()]);
        }
    }
}

#[tokio::test]
async fn read_batches_retry_but_permanent_http_errors_do_not() {
    let request = json!([{"method":"eth_getCode","params":["0xabc","safe"],"id":1},{"method":"eth_chainId","params":[],"id":2}]);
    let (result, seen) = exercise(request.clone(), &[429, 200]).await;
    assert_eq!(result.unwrap().as_array().unwrap().len(), 2);
    assert_eq!(seen, vec![request.clone(); 2]);
    for status in [400, 401, 403, 404, 500] {
        let (result, seen) = exercise(request.clone(), &[status, 200]).await;
        assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(s)) if s == status));
        assert_eq!(seen, vec![request.clone()]);
    }
}

#[tokio::test]
async fn ambiguous_connections_and_provider_errors_are_not_replayed() {
    for method in ["eth_getCode", "eth_sendRawTransaction"] {
        for status in [0, 298, 299] {
            let request = json!({"method":method,"params":[],"id":1});
            let (result, seen) = exercise(request.clone(), &[status, 200]).await;
            if status == 298 {
                assert!(result.unwrap().get("error").is_some());
            } else {
                assert!(result.is_err());
            }
            assert_eq!(seen, vec![request]);
        }
    }
}
