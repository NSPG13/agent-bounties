use super::*;
use async_trait::async_trait;
use db::managed_service_orders::{ManagedOrder, ManagedOrderStatus};
use managed_orders::{funding::FundingPlan, ManagedQuote};
use std::{collections::VecDeque, sync::Mutex};
use uuid::Uuid;

struct Fake(Mutex<VecDeque<Value>>);
#[async_trait]
impl JsonRpcTransport for Fake {
    async fn post_json_value(
        &self,
        _: &str,
        request: &Value,
    ) -> std::result::Result<Value, chain_base::ChainBaseError> {
        assert!(matches!(
            request["method"].as_str().unwrap(),
            "eth_chainId"
                | "eth_getCode"
                | "eth_call"
                | "eth_getBlockByNumber"
                | "eth_getTransactionReceipt"
        ));
        let v = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected RPC read");
        Ok(json!({"jsonrpc":"2.0","id":1,"result":v}))
    }
}
fn word(s: &str) -> String {
    format!("0x{:0>64}", s.trim_start_matches("0x"))
}
fn hash(n: u64) -> String {
    format!("0x{n:064x}")
}
fn topic(s: &str) -> String {
    format!("0x{}", hex::encode(keccak256(s.as_bytes())))
}
fn log(address: &str, topics: Vec<String>, words: Vec<String>, block: u64, index: u64) -> Value {
    json!({"address":address,"topics":topics,"data":format!("0x{}",words.iter().map(|s|s.trim_start_matches("0x")).collect::<String>()),
    "transactionHash":hash(block),"blockNumber":format!("0x{block:x}"),"blockHash":hash(block+1000),"logIndex":format!("0x{index:x}"),"removed":false})
}
fn fixture() -> (ManagedRecoveryTarget, Vec<String>, Vec<Value>) {
    let v: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let q: ManagedQuote = serde_json::from_value(v["quote"].clone()).unwrap();
    let mut input = q.input().clone();
    let (factory, implementation, _) = pin(&input.bounty.network).unwrap();
    input.bounty.factory = factory.into();
    let q = ManagedQuote::new(input).unwrap();
    let b = &q.input().bounty;
    let p = FundingPlan {
        order_id: Uuid::new_v4(),
        network: b.network.clone(),
        factory: b.factory.clone(),
        funding_wallet: b.funding_wallet.clone(),
        predicted_bounty: b.predicted_bounty.clone(),
        creation_nonce: b.creation_nonce.clone(),
        funding_deadline: b.funding_deadline,
        target_micro_usdc: q.bounty_target_micro_usdc(),
        calldata: "0x1234".into(),
        create_call_keccak256: b.create_call_keccak256.clone(),
    };
    let bid = hash(99);
    let n = p.target_micro_usdc;
    let token = base_network_descriptor(&p.network)
        .unwrap()
        .native_usdc_token_address
        .to_ascii_lowercase();
    let funding = vec![
        log(
            factory,
            vec![
                topic("CanonicalBountyCreated(bytes32,address,address,bytes32,bytes32,bytes32)"),
                bid.clone(),
                word(&p.predicted_bounty),
                word(&p.funding_wallet),
            ],
            vec![hash(0), hash(0), p.creation_nonce.clone()],
            100,
            0,
        ),
        log(
            &p.predicted_bounty,
            vec![
                topic("FundingAdded(bytes32,address,uint256,uint256,uint256)"),
                bid.clone(),
                word(&p.funding_wallet),
            ],
            vec![hash(n), hash(n), hash(n)],
            100,
            1,
        ),
    ];
    let recovery = vec![
        log(
            &p.predicted_bounty,
            vec![topic("BountyCancelled(bytes32,uint256)"), bid.clone()],
            vec![hash(0)],
            200,
            0,
        ),
        log(
            &p.predicted_bounty,
            vec![
                topic("RefundWithdrawn(bytes32,address,uint256,uint256,uint256)"),
                bid,
                word(&p.funding_wallet),
            ],
            vec![hash(n), hash(0), hash(n)],
            200,
            1,
        ),
        log(
            &token,
            vec![
                TRANSFER_TOPIC.into(),
                word(&p.predicted_bounty),
                word(&p.funding_wallet),
            ],
            vec![hash(n)],
            200,
            2,
        ),
    ];
    let header = |n: u64| json!({"number":format!("0x{n:x}"),"hash":hash(n+1000)});
    let receipt = |n: u64, logs: Vec<Value>| json!({"status":"0x1","transactionHash":hash(n),"blockNumber":format!("0x{n:x}"),"blockHash":hash(n+1000),"logs":logs});
    let responses = vec![
        json!(84532u64)
            .as_u64()
            .map(|n| json!(format!("0x{n:x}")))
            .unwrap(),
        header(202),
        json!("0x1234"),
        json!("0x5678"),
        json!(format!(
            "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
            implementation.trim_start_matches("0x")
        )),
        json!(hash(1)),
        json!(word(&token)),
        json!(hash(5)),
        json!(hash(0)),
        json!(hash(0)),
        json!(hash(0)),
        receipt(100, funding),
        header(100),
        receipt(200, recovery),
        header(200),
        header(202),
    ];
    let t = ManagedRecoveryTarget {
        order: ManagedOrder {
            order_id: p.order_id,
            quote_digest: q.digest().unwrap(),
            quote: q,
            accepted_at: 0,
            status: ManagedOrderStatus::Funded,
        },
        funding_operation_id: Uuid::new_v4(),
        funding_plan: p,
        funding_tx: hash(100),
        funded_block: 100,
    };
    (t, vec![hash(100), hash(200)], responses)
}
async fn check(
    t: &ManagedRecoveryTarget,
    h: &[String],
    responses: Vec<Value>,
) -> Result<RecoveredPrincipal> {
    qualify_with(
        &Fake(Mutex::new(responses.into())),
        "fixture",
        t,
        h,
        &digest(&json!("0x1234")).unwrap(),
        &digest(&json!("0x5678")).unwrap(),
    )
    .await
}
#[tokio::test]
async fn exact_canonical_receipts_and_native_transfer_qualify_without_writes() {
    let (t, h, v) = fixture();
    let proof = check(&t, &h, v).await.unwrap();
    assert_eq!(proof.order_id(), t.order.order_id);
}
#[tokio::test]
async fn wrong_chain_code_clone_token_finality_receipt_and_safe_reorg_fail_closed() {
    let (t, h, v) = fixture();
    for (index, bad) in [
        (0, json!("0x1")),
        (2, json!("0x00")),
        (3, json!("0x00")),
        (4, json!("0x00")),
        (5, json!(hash(0))),
        (6, json!(hash(1))),
        (7, json!(hash(4))),
        (8, json!(hash(1))),
        (9, json!(hash(1))),
        (10, json!(hash(1))),
    ] {
        let mut changed = v.clone();
        changed[index] = bad;
        assert!(
            check(&t, &h, changed).await.is_err(),
            "RPC response {index}"
        );
    }
    for (index, key, bad) in [
        (1, "number", json!("0xc9")),
        (11, "status", json!("0x0")),
        (11, "transactionHash", json!(hash(7))),
        (12, "hash", json!(hash(7))),
        (13, "blockHash", json!(hash(7))),
        (15, "hash", json!(hash(7))),
    ] {
        let mut changed = v.clone();
        changed[index][key] = bad;
        assert!(
            check(&t, &h, changed).await.is_err(),
            "RPC response {index}/{key}"
        );
    }
    for key in ["removed", "transactionHash", "blockHash", "blockNumber"] {
        let mut changed = v.clone();
        changed[13]["logs"][1][key] = if key == "removed" {
            json!(true)
        } else {
            json!(hash(7))
        };
        assert!(check(&t, &h, changed).await.is_err(), "log {key}");
    }
    let mut changed = v.clone();
    changed[13]["logs"][2]["address"] = json!(t.funding_plan.factory);
    assert!(check(&t, &h, changed).await.is_err());
    let mut changed = v;
    changed[13]["logs"][2]["data"] = json!(hash(1));
    assert!(check(&t, &h, changed).await.is_err());
}
