use chain_base::{BaseTransactionRelayer, ChainBaseError, EvmTransactionIntent, RelayGasBudget};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct DenyBudget(Mutex<Vec<u128>>);
#[async_trait::async_trait]
impl RelayGasBudget for DenyBudget {
    async fn reserve(&self, chain: u64, cost: u128) -> Result<(), ChainBaseError> {
        assert_eq!(chain, 8453);
        self.0.lock().unwrap().push(cost);
        Err(ChainBaseError::RelayerProvider(
            "daily budget exhausted".into(),
        ))
    }
}

async fn rehearsal(valid_oracle: bool, prepare_only: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let methods = Arc::new(Mutex::new(Vec::new()));
    let observed = methods.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let request: serde_json::Value = loop {
                let mut chunk = [0u8; 4096];
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
                    continue;
                };
                let header = std::str::from_utf8(&bytes[..end]).unwrap();
                let length = header
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                    .unwrap()
                    .1
                    .trim()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
                }
            };
            let method = request["method"].as_str().unwrap();
            observed.lock().unwrap().push(method.to_string());
            assert!(
                !method.starts_with("eth_send"),
                "a denied budget must never reach broadcast"
            );
            let oracle = request["params"][0]["to"].as_str().is_some_and(|to| {
                to.eq_ignore_ascii_case("0x420000000000000000000000000000000000000F")
            });
            let result = match method {
                "eth_chainId" => serde_json::json!("0x2105"),
                "eth_call" if oracle && valid_oracle => serde_json::json!(format!("0x{:064x}", 10)),
                "eth_call" => serde_json::json!("0x"),
                "eth_estimateGas" => serde_json::json!("0x5208"),
                "eth_feeHistory" => {
                    serde_json::json!({"oldestBlock":"0x1","baseFeePerGas":["0x1","0x1"],"gasUsedRatio":[0.5],"reward":[["0x1"]]})
                }
                "eth_getTransactionCount" => serde_json::json!("0x0"),
                "eth_getBalance" => serde_json::json!("0x1000000000000000000"),
                other => panic!("unexpected method {other}"),
            };
            let body =
                serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
    });
    let budget = Arc::new(DenyBudget(Mutex::new(Vec::new())));
    // Public fixture key. Every RPC request stays on the isolated local listener.
    let relayer = BaseTransactionRelayer::from_private_key(&format!("0x{}", "11".repeat(32)))
        .unwrap()
        .with_gas_budget(budget.clone());
    let intent = EvmTransactionIntent {
        from: Some(relayer.address()),
        to: relayer.address(),
        value_wei: 0,
        data: "0x12345678".into(),
        function: "fixture()".into(),
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        if prepare_only {
            relayer
                .prepare_signed_transaction(&url, 8453, &intent, 100_000, 100_000_000_000)
                .await
                .map(|_| ())
        } else {
            relayer
                .simulate_and_broadcast(&url, 8453, &intent, 100_000, 100_000_000_000)
                .await
                .map(|_| ())
        }
    })
    .await
    .unwrap();
    server.abort();
    let error = result.unwrap_err().to_string();
    if valid_oracle {
        assert!(error.contains("daily budget exhausted"));
        let reserved = budget.0.lock().unwrap();
        assert_eq!(reserved.len(), 1);
        // 120% of 21,000 gas at max fee 3 wei plus 2x each 10-wei oracle fee.
        assert_eq!(reserved[0], 75_640);
        assert_eq!(
            methods
                .lock()
                .unwrap()
                .iter()
                .filter(|method| *method == "eth_call")
                .count(),
            3
        );
    } else {
        assert!(error.contains("fee oracle returned invalid data"));
        assert!(budget.0.lock().unwrap().is_empty());
    }
    assert!(!methods
        .lock()
        .unwrap()
        .iter()
        .any(|method| method.starts_with("eth_send")));
}

#[tokio::test]
async fn exhausted_budget_blocks_broadcast_after_all_base_fees_are_quoted() {
    rehearsal(true, false).await;
}
#[tokio::test]
async fn unavailable_l1_fee_quote_is_not_zero_and_never_broadcasts() {
    rehearsal(false, false).await;
}

#[tokio::test]
async fn exhausted_budget_never_returns_a_signed_envelope() {
    rehearsal(true, true).await;
}
#[tokio::test]
async fn unavailable_l1_quote_never_returns_a_signed_envelope() {
    rehearsal(false, true).await;
}
