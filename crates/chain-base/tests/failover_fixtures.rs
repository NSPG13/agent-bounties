//! Offline failover fixtures for `FailoverJsonRpcTransport`.
//!
//! These tests use a scripted in-memory transport so no live RPC endpoint is
//! contacted. They prove bounded retry, endpoint advance on retriable failures,
//! wrong-chain rejection, execution-error preservation, and endpoint exhaustion.

use async_trait::async_trait;
use chain_base::{
    BaseRpcUrlConfig, ChainBaseError, FailoverJsonRpcTransport, FailoverRetryConfig,
    JsonRpcTransport,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const BASE_MAINNET_CHAIN_ID: u64 = 8453;
const CHAIN_ID_HEX: &str = "0x2105"; // 8453

/// A scripted fake transport that records `(endpoint, method)` calls and returns
/// responses from a caller-provided closure.
struct FakeTransport {
    handler: Arc<dyn Fn(&str, &Value) -> Result<Value, ChainBaseError> + Send + Sync>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl FakeTransport {
    fn new(
        handler: impl Fn(&str, &Value) -> Result<Value, ChainBaseError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            handler: Arc::new(handler),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn method_of(request: &Value) -> String {
        request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    }
}

#[async_trait]
impl JsonRpcTransport for FakeTransport {
    async fn post_json_value(
        &self,
        rpc_url: &str,
        request: &Value,
    ) -> Result<Value, ChainBaseError> {
        self.calls
            .lock()
            .unwrap()
            .push((rpc_url.to_string(), Self::method_of(request)));
        (self.handler)(rpc_url, request)
    }
}

/// Wraps a handler so `eth_chainId` always answers with the Base mainnet chain ID.
fn chain_aware(
    respond: impl Fn(&str, &Value) -> Result<Value, ChainBaseError> + Send + Sync + 'static,
) -> impl Fn(&str, &Value) -> Result<Value, ChainBaseError> + Send + Sync + 'static {
    move |endpoint, request| {
        if FakeTransport::method_of(request) == "eth_chainId" {
            return Ok(json!({"jsonrpc": "2.0", "id": 0, "result": CHAIN_ID_HEX}));
        }
        respond(endpoint, request)
    }
}

fn retry_zero() -> FailoverRetryConfig {
    FailoverRetryConfig {
        max_retries: 2,
        base_backoff_ms: 0,
        max_backoff_ms: 0,
    }
}

#[test]
fn failover_retry_config_default_values() {
    let retry = FailoverRetryConfig::default();
    assert_eq!(retry.max_retries, 3);
    assert_eq!(retry.base_backoff_ms, 200);
    assert_eq!(retry.max_backoff_ms, 5_000);
}

#[test]
fn deterministic_backoff_sequence() {
    let retry = FailoverRetryConfig::default();
    assert_eq!(retry.deterministic_backoff_ms(0), 200);
    assert_eq!(retry.deterministic_backoff_ms(1), 400);
    assert_eq!(retry.deterministic_backoff_ms(2), 800);
    assert_eq!(retry.deterministic_backoff_ms(3), 1600);
    assert_eq!(retry.deterministic_backoff_ms(4), 3200);
    assert_eq!(retry.deterministic_backoff_ms(5), 5000); // capped
    assert_eq!(retry.deterministic_backoff_ms(10), 5000); // still capped
}

#[test]
fn retry_config_custom_max_backoff() {
    let retry = FailoverRetryConfig {
        max_retries: 5,
        base_backoff_ms: 100,
        max_backoff_ms: 1_000,
    };
    assert_eq!(retry.deterministic_backoff_ms(0), 100);
    assert_eq!(retry.deterministic_backoff_ms(3), 800);
    assert_eq!(retry.deterministic_backoff_ms(4), 1000); // capped
}

#[test]
fn is_retriable_http_status_classification() {
    assert!(FailoverJsonRpcTransport::is_retriable_http_status(429));
    assert!(FailoverJsonRpcTransport::is_retriable_http_status(500));
    assert!(FailoverJsonRpcTransport::is_retriable_http_status(502));
    assert!(FailoverJsonRpcTransport::is_retriable_http_status(503));
    assert!(FailoverJsonRpcTransport::is_retriable_http_status(504));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(400));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(401));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(403));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(404));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(200));
    assert!(!FailoverJsonRpcTransport::is_retriable_http_status(301));
}

#[test]
fn base_rpc_url_config_with_multiple_endpoints() {
    let config = BaseRpcUrlConfig {
        base_sepolia: vec![],
        base_mainnet: vec![
            "https://mainnet.infura.io/v3/key".to_string(),
            "https://base.llamarpc.com".to_string(),
            "https://base-mainnet.g.alchemy.com/v2/key".to_string(),
        ],
    };
    let (_network, urls) = config.resolve_endpoint_list("base-mainnet").unwrap();
    assert_eq!(urls.len(), 3);
    assert_eq!(urls[0], "https://mainnet.infura.io/v3/key");
    assert_eq!(urls[1], "https://base.llamarpc.com");
    assert_eq!(urls[2], "https://base-mainnet.g.alchemy.com/v2/key");
}

#[test]
fn base_rpc_url_config_empty_endpoints() {
    let config = BaseRpcUrlConfig {
        base_sepolia: vec![],
        base_mainnet: vec![],
    };
    assert!(config.resolve("base-mainnet").is_err());
    assert!(config.resolve_endpoint_list("base-mainnet").is_err());
}

#[tokio::test]
async fn failover_429_retries_then_advances_to_next_endpoint() {
    let fake = FakeTransport::new(chain_aware(|endpoint, _request| {
        if endpoint == "https://a.example" {
            Err(ChainBaseError::RpcHttpStatus(429))
        } else {
            Ok(json!({"jsonrpc": "2.0", "id": 1, "result": "0x1b4"}))
        }
    }));
    let calls = fake.calls.clone();
    let transport = FailoverJsonRpcTransport::with_transport(
        fake,
        vec![
            "https://a.example".to_string(),
            "https://b.example".to_string(),
        ],
        BASE_MAINNET_CHAIN_ID,
        retry_zero(),
    );

    let result = transport
        .post_json_value(
            "",
            &json!({"jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []}),
        )
        .await
        .unwrap();
    assert_eq!(result["result"], "0x1b4");

    let calls = calls.lock().unwrap();
    // Endpoint A: 1 chainId + (max_retries=2 => 3 attempts) of the real request.
    let a = calls
        .iter()
        .filter(|(e, m)| e == "https://a.example" && m == "eth_blockNumber")
        .count();
    let b = calls
        .iter()
        .filter(|(e, m)| e == "https://b.example" && m == "eth_blockNumber")
        .count();
    assert_eq!(a, 3);
    assert_eq!(b, 1);
}

#[tokio::test]
async fn failover_skips_wrong_chain_endpoint() {
    let fake = FakeTransport::new(|endpoint, request| {
        if FakeTransport::method_of(request) == "eth_chainId" {
            let result = if endpoint == "https://wrong.example" {
                "0x1"
            } else {
                CHAIN_ID_HEX
            };
            return Ok(json!({"jsonrpc": "2.0", "id": 0, "result": result}));
        }
        Ok(json!({"jsonrpc": "2.0", "id": 1, "result": "0x2a"}))
    });
    let calls = fake.calls.clone();
    let transport = FailoverJsonRpcTransport::with_transport(
        fake,
        vec![
            "https://wrong.example".to_string(),
            "https://good.example".to_string(),
        ],
        BASE_MAINNET_CHAIN_ID,
        retry_zero(),
    );

    let result = transport
        .post_json_value(
            "",
            &json!({"jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []}),
        )
        .await
        .unwrap();
    assert_eq!(result["result"], "0x2a");

    let calls = calls.lock().unwrap();
    // Wrong-chain endpoint is only ever probed for chain ID, never for the real request.
    assert_eq!(
        calls
            .iter()
            .filter(|(e, m)| e == "https://wrong.example" && m == "eth_blockNumber")
            .count(),
        0
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(e, m)| e == "https://good.example" && m == "eth_blockNumber")
            .count(),
        1
    );
}

#[tokio::test]
async fn failover_preserves_execution_error_without_retry() {
    let fake = FakeTransport::new(chain_aware(|_endpoint, _request| {
        Err(ChainBaseError::RpcProviderError {
            code: -32000,
            message: "execution reverted".to_string(),
        })
    }));
    let calls = fake.calls.clone();
    let transport = FailoverJsonRpcTransport::with_transport(
        fake,
        vec![
            "https://a.example".to_string(),
            "https://b.example".to_string(),
        ],
        BASE_MAINNET_CHAIN_ID,
        retry_zero(),
    );

    let err = transport
        .post_json_value(
            "",
            &json!({"jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": []}),
        )
        .await
        .unwrap_err();
    match err {
        ChainBaseError::RpcProviderError { code, message } => {
            assert_eq!(code, -32000);
            assert_eq!(message, "execution reverted");
        }
        other => panic!("expected RpcProviderError, got {other:?}"),
    }

    let calls = calls.lock().unwrap();
    // No retry and no failover: the real request hit endpoint A exactly once.
    assert_eq!(
        calls
            .iter()
            .filter(|(e, m)| e == "https://a.example" && m == "eth_call")
            .count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|(e, _)| e == "https://b.example")
            .count(),
        0
    );
}

#[tokio::test]
async fn failover_exhausts_all_endpoints() {
    let fake = FakeTransport::new(chain_aware(|_endpoint, _request| {
        Err(ChainBaseError::RpcHttpStatus(503))
    }));
    let calls = fake.calls.clone();
    let transport = FailoverJsonRpcTransport::with_transport(
        fake,
        vec![
            "https://a.example".to_string(),
            "https://b.example".to_string(),
        ],
        BASE_MAINNET_CHAIN_ID,
        retry_zero(),
    );

    let err = transport
        .post_json_value(
            "",
            &json!({"jsonrpc": "2.0", "id": 1, "method": "eth_blockNumber", "params": []}),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ChainBaseError::RpcHttpStatus(503)));

    let calls = calls.lock().unwrap();
    // Each endpoint: 1 chainId + 3 attempts = 4 calls; two endpoints => 6 real requests, 2 chain IDs.
    assert_eq!(
        calls.iter().filter(|(_, m)| m == "eth_blockNumber").count(),
        6
    );
    assert_eq!(calls.iter().filter(|(_, m)| m == "eth_chainId").count(), 2);
}
