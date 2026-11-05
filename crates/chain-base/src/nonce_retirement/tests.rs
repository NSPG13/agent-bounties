use super::*;
use std::sync::Mutex;
async fn fixture() -> (
    BaseTransactionRelayer,
    PreparedBaseRelayTransaction,
    TimeoutRelayScope,
    TimeoutRevalidationPolicy,
) {
    let signer = alloy::signers::local::PrivateKeySigner::random();
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("0x{}", hex::encode(signer.to_bytes())))
            .unwrap();
    let scope = TimeoutRelayScope {
        network: "base-mainnet".into(),
        bounty_contract: format!("0x{}", "33".repeat(20)),
        action: "expire_claim".into(),
        round: 3,
        factory_contract: format!("0x{}", "11".repeat(20)),
        bounty_id: format!("0x{}", "44".repeat(32)),
        terms_hash: format!("0x{}", "55".repeat(32)),
    };
    let envelope = TransactionRequest::default()
        .with_to(parse_alloy_address(&scope.bounty_contract).unwrap())
        .with_value(U256::ZERO)
        .with_input(parse_alloy_bytes(scope.calldata().unwrap()).unwrap())
        .with_nonce(77)
        .with_chain_id(8453)
        .with_gas_limit(25_000)
        .with_max_fee_per_gas(3)
        .with_max_priority_fee_per_gas(1)
        .build(&EthereumWallet::from(signer))
        .await
        .unwrap();
    let saved = PreparedBaseRelayTransaction {
        nonce: 77,
        signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
        transaction: BaseRelayedTransaction {
            relayer: relayer.address(),
            tx_hash: format!("{:#x}", envelope.tx_hash()),
            estimated_gas: 21_000,
            gas_limit: 25_000,
            max_fee_per_gas_wei: 3,
            max_priority_fee_per_gas_wei: 1,
            estimated_max_cost_wei: 75_000,
        },
    };
    let policy = TimeoutRevalidationPolicy {
        factory_runtime_code_hash: format!("0x{}", hex::encode(keccak_word(&[1]))),
        implementation_contract: format!("0x{}", "22".repeat(20)),
        implementation_runtime_code_hash: format!("0x{}", hex::encode(keccak_word(&[2]))),
        max_gas: 30_000,
        max_fee_per_gas_wei: 1_000_000_000,
    };
    (relayer, saved, scope, policy)
}
struct Fake<'a> {
    scope: &'a TimeoutRelayScope,
    policy: &'a TimeoutRevalidationPolicy,
    saved: &'a PreparedBaseRelayTransaction,
    fault: &'a str,
    now: DateTime<Utc>,
    replacement: Option<&'a PreparedBaseRelayTransaction>,
    reads: Mutex<Vec<String>>,
}
impl Fake<'_> {
    fn result(&self, url: &str, method: &str, p: &Value) -> Value {
        self.reads.lock().unwrap().push(method.into());
        let bad = self.fault;
        let shadow = url.contains("shadow");
        match method {
            "eth_chainId" => json!(if bad == "wrong_chain" && shadow {
                "0x1"
            } else {
                "0x2105"
            }),
            "eth_gasPrice" => json!("0x3"),
            "eth_getBlockByNumber" => {
                let tag = p[0].as_str().unwrap();
                let mut n = match tag {
                    "latest" => 120,
                    "safe" => 110,
                    "finalized" => 100,
                    _ => u64::from_str_radix(&tag[2..], 16).unwrap(),
                };
                if bad == "wrong_height" && tag.starts_with("0x") {
                    n += 1;
                }
                let ts = if bad == "future" {
                    self.now.timestamp() + 60
                } else if bad == "stale" {
                    self.now.timestamp() - 2000
                } else {
                    self.now.timestamp() - i64::try_from(120u64.saturating_sub(n)).unwrap()
                };
                let hash = format!("0x{:064x}", if bad == "fork" && shadow { n + 1 } else { n });
                json!({"number":hex_quantity(n),"hash":hash,"timestamp":format!("0x{ts:x}"),"baseFeePerGas":if bad=="fee_overflow"{"0xffffffffffffffffffffffffffffffff"}else{"0x1"}})
            }
            "eth_getTransactionCount" => {
                let n = if self.replacement.is_some() {
                    if bad == "nonce_order" && p[1] == "pending" {
                        77
                    } else {
                        78
                    }
                } else if bad == "nonce_changed" && shadow {
                    78
                } else {
                    77
                };
                json!(hex_quantity(n))
            }
            "eth_getCode" => {
                let addr = p[0].as_str().unwrap();
                if addr == self.saved.transaction.relayer {
                    return json!(if bad == "delegated" {
                        "0xef01000000000000000000000000000000000000000001"
                    } else {
                        "0x"
                    });
                }
                if addr == self.scope.factory_contract {
                    json!(if bad == "factory_code" {
                        "0x03"
                    } else {
                        "0x01"
                    })
                } else if addr == self.policy.implementation_contract {
                    json!("0x02")
                } else {
                    json!(format!(
                        "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
                        &self.policy.implementation_contract[2..]
                    ))
                }
            }
            "eth_getBalance" => json!(if bad == "low_balance" {
                "0x0"
            } else {
                "0xffffffffffffffff"
            }),
            "eth_estimateGas" => json!(if bad == "gas_changed" {
                "0x5209"
            } else {
                "0x5208"
            }),
            "eth_call" => {
                if p[0]["to"] == self.saved.transaction.relayer {
                    return json!(if bad == "nonempty_call" { "0x01" } else { "0x" });
                }
                let data = p[0]["data"].as_str().unwrap();
                let selector =
                    |sig: &str| format!("0x{}", hex::encode(&keccak_word(sig.as_bytes())[..4]));
                let value = if data == selector("implementation()") {
                    format!("0x{:0>64}", &self.policy.implementation_contract[2..])
                } else if data == selector("factory()") {
                    format!("0x{:0>64}", &self.scope.factory_contract[2..])
                } else if data == selector("bountyId()") {
                    self.scope.bounty_id.clone()
                } else if data == selector("termsHash()") {
                    self.scope.terms_hash.clone()
                } else if data == selector("round()") {
                    format!("0x{:064x}", if bad == "lower_round" { 2 } else { 3 })
                } else if data == selector("status()") {
                    format!("0x{:064x}", if bad == "still_active" { 2 } else { 1 })
                } else {
                    format!("0x{:064x}", 10)
                };
                json!(value)
            }
            "eth_getTransactionByHash" | "eth_getTransactionReceipt" => {
                if p[0] == self.saved.transaction.tx_hash {
                    return if bad == "original_visible" {
                        json!({"hash":self.saved.transaction.tx_hash})
                    } else {
                        Value::Null
                    };
                }
                if (bad == "replacement_tx_primary" && !shadow
                    || bad == "replacement_tx_shadow" && shadow)
                    && method == "eth_getTransactionByHash"
                    || (bad == "replacement_receipt_primary" && !shadow
                        || bad == "replacement_receipt_shadow" && shadow)
                        && method == "eth_getTransactionReceipt"
                {
                    return json!({"hash":p[0]});
                }
                let Some(r) = self.replacement else {
                    return Value::Null;
                };
                let number = if bad == "not_finalized" { 101 } else { 90 };
                let block_hash = format!("0x{number:064x}");
                if method == "eth_getTransactionByHash" {
                    json!({"hash":r.transaction.tx_hash,"from":r.transaction.relayer,"to":r.transaction.relayer,"type":"0x2","chainId":"0x2105","nonce":hex_quantity(r.nonce),"value":"0x0","input":"0x","accessList":[],"gas":hex_quantity(r.transaction.gas_limit),"maxFeePerGas":format!("0x{:x}",r.transaction.max_fee_per_gas_wei),"maxPriorityFeePerGas":format!("0x{:x}",r.transaction.max_priority_fee_per_gas_wei),"blockNumber":hex_quantity(number),"blockHash":block_hash,"transactionIndex":"0x0"})
                } else {
                    json!({"transactionHash":r.transaction.tx_hash,"from":r.transaction.relayer,"to":r.transaction.relayer,"status":if bad=="receipt_failed"{"0x0"}else{"0x1"},"gasUsed":"0x5208","logs":[],"contractAddress":null,"effectiveGasPrice":"0x3","blockNumber":hex_quantity(number),"blockHash":if bad=="receipt_fork"{format!("0x{}","99".repeat(32))}else{block_hash},"transactionIndex":"0x0"})
                }
            }
            _ => panic!("unexpected RPC {method}"),
        }
    }
    fn response(&self, url: &str, v: &Value) -> Value {
        if let Some(batch) = v.as_array() {
            return Value::Array(batch.iter().map(|v| self.response(url, v)).collect());
        }
        json!({"jsonrpc":"2.0","id":v["id"],"result":self.result(url,v["method"].as_str().unwrap(),&v["params"])})
    }
}
#[async_trait::async_trait]
impl JsonRpcTransport for Fake<'_> {
    async fn post_json_value(&self, url: &str, p: &Value) -> Result<Value, ChainBaseError> {
        if self.fault == "transport" {
            return Err(denied());
        }
        Ok(self.response(url, p))
    }
}
const URLS: [&str; 2] = ["https://primary.invalid", "https://shadow.invalid"];
#[tokio::test]
async fn retirement_read_checks_reject_hostile_and_stale_evidence() {
    let (_, saved, scope, policy) = fixture().await;
    let now = Utc::now();
    let fake = Fake {
        scope: &scope,
        policy: &policy,
        saved: &saved,
        fault: "",
        now,
        replacement: None,
        reads: Mutex::new(vec![]),
    };
    let (plan, _) = quote_with_transport(URLS, &saved, &scope, &policy, now, &fake)
        .await
        .unwrap();
    assert!(plan.maximum_cost_wei > 25_200 * plan.max_fee_per_gas_wei);
    assert_eq!(plan.nonce, 77);
    for fault in [
        "wrong_chain",
        "wrong_height",
        "future",
        "stale",
        "fork",
        "fee_overflow",
        "nonce_changed",
        "delegated",
        "factory_code",
        "low_balance",
        "gas_changed",
        "nonempty_call",
        "lower_round",
        "still_active",
        "original_visible",
        "transport",
    ] {
        let bad = Fake {
            fault,
            ..Fake {
                scope: &scope,
                policy: &policy,
                saved: &saved,
                fault: "",
                now,
                replacement: None,
                reads: Mutex::new(vec![]),
            }
        };
        assert!(
            check_with_transport(URLS, &plan, &saved, &scope, &policy, now, &bad)
                .await
                .is_err(),
            "{fault}"
        );
    }
    assert!(check_with_transport(
        [URLS[0], URLS[0]],
        &plan,
        &saved,
        &scope,
        &policy,
        now,
        &fake
    )
    .await
    .is_err());
    assert!(
        check_with_transport(URLS, &plan, &saved, &scope, &policy, plan.expires_at, &fake)
            .await
            .is_err()
    );
    assert!(fake
        .reads
        .lock()
        .unwrap()
        .iter()
        .all(|m| m != "eth_sendRawTransaction"));
}
#[tokio::test]
async fn retirement_exact_envelope_and_finality() {
    let (relayer, saved, scope, policy) = fixture().await;
    let now = Utc::now();
    let fake = Fake {
        scope: &scope,
        policy: &policy,
        saved: &saved,
        fault: "",
        now,
        replacement: None,
        reads: Mutex::new(vec![]),
    };
    let (plan, _) = quote_with_transport(URLS, &saved, &scope, &policy, now, &fake)
        .await
        .unwrap();
    let replacement = relayer.sign_nonce_retirement(&plan).await.unwrap();
    validate_nonce_retirement_envelope(&plan, &replacement).unwrap();
    for field in ["nonce", "cost", "gas", "hash", "sender", "raw"] {
        let mut bad = replacement.clone();
        match field {
            "nonce" => bad.nonce += 1,
            "cost" => bad.transaction.estimated_max_cost_wei += 1,
            "gas" => bad.transaction.estimated_gas += 1,
            "hash" => bad.transaction.tx_hash = saved.transaction.tx_hash.clone(),
            "sender" => bad.transaction.relayer = scope.bounty_contract.clone(),
            _ => bad.signed_transaction = saved.signed_transaction.clone(),
        };
        assert!(validate_nonce_retirement_envelope(&plan, &bad).is_err());
    }
    for fault in [
        "",
        "nonce_order",
        "not_finalized",
        "receipt_failed",
        "receipt_fork",
        "fork",
        "wrong_height",
        "delegated",
    ] {
        let fake = Fake {
            scope: &scope,
            policy: &policy,
            saved: &saved,
            fault,
            now,
            replacement: Some(&replacement),
            reads: Mutex::new(vec![]),
        };
        let result = confirm_with_transport(URLS, &plan, &replacement, now, &fake).await;
        if fault.is_empty() {
            assert_eq!(result.unwrap().finalized_nonce, 78);
        } else {
            assert!(result.is_err(), "{fault}");
        }
    }
}

#[tokio::test]
async fn retained_dispatch_requires_new_exact_authority_and_all_chain_checks() {
    use retained_dispatch::{check_retained_dispatch_with_transport, RetainedNonceDispatch};
    let (relayer, original, scope, policy) = fixture().await;
    let now = Utc::now();
    let fake = Fake {
        scope: &scope,
        policy: &policy,
        saved: &original,
        fault: "",
        now,
        replacement: None,
        reads: Mutex::new(vec![]),
    };
    let (mut historical, _) = quote_with_transport(URLS, &original, &scope, &policy, now, &fake)
        .await
        .unwrap();
    let replacement = relayer.sign_nonce_retirement(&historical).await.unwrap();
    // Synthetic historical provenance; timestamps are not transaction fields.
    historical.created_at -= chrono::Duration::minutes(20);
    historical.expires_at -= chrono::Duration::minutes(20);
    let before = historical.clone();
    let plan_id = Uuid::new_v4();
    assert!(
        check_with_transport(URLS, &historical, &original, &scope, &policy, now, &fake)
            .await
            .is_err()
    );
    assert!(relayer.sign_nonce_retirement(&historical).await.is_err());
    let proposal =
        RetainedNonceDispatch::propose(URLS, plan_id, &historical, &replacement, now).unwrap();
    for fault in [
        "",
        "wrong_chain",
        "wrong_height",
        "future",
        "stale",
        "fork",
        "fee_overflow",
        "nonce_changed",
        "delegated",
        "factory_code",
        "low_balance",
        "gas_changed",
        "nonempty_call",
        "lower_round",
        "still_active",
        "original_visible",
        "transport",
        "replacement_tx_primary",
        "replacement_tx_shadow",
        "replacement_receipt_primary",
        "replacement_receipt_shadow",
    ] {
        let fake = Fake {
            scope: &scope,
            policy: &policy,
            saved: &original,
            fault,
            now,
            replacement: None,
            reads: Mutex::new(vec![]),
        };
        let result = check_retained_dispatch_with_transport(
            URLS,
            &proposal,
            plan_id,
            &historical,
            &original,
            &replacement,
            &scope,
            &policy,
            now,
            &fake,
        )
        .await;
        assert_eq!(result.is_ok(), fault.is_empty(), "{fault}");
        assert!(fake
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|m| m != "eth_sendRawTransaction"));
    }
    for fault in [
        "operation",
        "plan",
        "digest",
        "hash",
        "sha",
        "cost",
        "providers",
        "expired",
        "future",
    ] {
        let mut bad = proposal.clone();
        match fault {
            "operation" => bad.operation_id = Uuid::new_v4(),
            "plan" => bad.retirement_plan_id = Uuid::new_v4(),
            "digest" => bad.retirement_digest = format!("0x{}", "ab".repeat(32)),
            "hash" => bad.transaction_hash = original.transaction.tx_hash.clone(),
            "sha" => bad.envelope_sha256 = "ab".repeat(32),
            "cost" => bad.maximum_cost_wei += 1,
            "providers" => bad.provider_hashes.swap(0, 1),
            "expired" => {
                bad.created_at -= chrono::Duration::minutes(16);
                bad.expires_at -= chrono::Duration::minutes(16);
            }
            "future" => {
                bad.created_at += chrono::Duration::seconds(1);
                bad.expires_at += chrono::Duration::seconds(1);
            }
            _ => unreachable!(),
        }
        assert!(
            check_retained_dispatch_with_transport(
                URLS,
                &bad,
                plan_id,
                &historical,
                &original,
                &replacement,
                &scope,
                &policy,
                now,
                &fake
            )
            .await
            .is_err(),
            "{fault}"
        );
    }
    assert_eq!(
        historical, before,
        "historical approval time is never refreshed"
    );
}
