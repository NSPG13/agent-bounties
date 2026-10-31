//! Read-only evidence for a separate operator authorization. Never signs or sends.
use super::*;
mod diagnostics;
mod pacing;
use diagnostics::{checked, Phase};

#[derive(Debug, Clone)]
pub struct TimeoutRevalidationPolicy {
    pub factory_runtime_code_hash: String,
    pub implementation_contract: String,
    pub implementation_runtime_code_hash: String,
    pub max_gas: u64,
    pub max_fee_per_gas_wei: u128,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutRevalidationObservation {
    pub observed_at: chrono::DateTime<Utc>,
    pub common_safe_block: BaseBlockIdentity,
    /// SHA256 of normalized origin only; no credential, URL path or query.
    pub provider_origin_hashes: [String; 2],
    pub latest_blocks: [BaseBlockIdentity; 2],
    pub estimated_gas: [u64; 2],
    pub maximum_fee_wei: u128,
    pub transaction_hash: String,
    pub nonce: u64,
}

fn unavailable() -> ChainBaseError {
    invalid("timeout revalidation unavailable or inconsistent")
}
fn clock() -> Result<u64, ChainBaseError> {
    u64::try_from(Utc::now().timestamp()).map_err(|_| unavailable())
}
fn fresh(block: &BaseBlockIdentity, seconds: u64) -> Result<(), ChainBaseError> {
    let now = clock()?;
    if block.timestamp > now || now - block.timestamp > seconds {
        return Err(unavailable());
    }
    Ok(())
}
fn origins(rpcs: [&str; 2]) -> Result<[String; 2], ChainBaseError> {
    let urls = rpcs.map(reqwest::Url::parse);
    let [a, b] = urls;
    let (a, b) = (a.map_err(|_| unavailable())?, b.map_err(|_| unavailable())?);
    for url in [&a, &b] {
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(unavailable());
        }
    }
    // Different paths/credentials/ports on the same host are not independent routes.
    if a.host_str().map(|h| h.trim_end_matches('.'))
        == b.host_str().map(|h| h.trim_end_matches('.'))
        || a.origin() == b.origin()
    {
        return Err(unavailable());
    }
    Ok([a, b].map(|u| hex::encode(Sha256::digest(u.origin().ascii_serialization().as_bytes()))))
}
async fn read<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    method: &str,
    params: Value,
) -> Result<Value, ChainBaseError> {
    // This allowlist excludes signing, broadcasting and unknown methods even with a custom transport.
    if !matches!(
        method,
        "eth_chainId"
            | "eth_getBlockByNumber"
            | "eth_call"
            | "eth_getCode"
            | "eth_getTransactionCount"
            | "eth_getTransactionByHash"
            | "eth_getTransactionReceipt"
            | "eth_estimateGas"
            | "eth_getBalance"
    ) {
        return Err(unavailable());
    }
    let result = t
        .post_json_value(
            rpc,
            &json!({"jsonrpc":"2.0","id":721,"method":method,"params":params}),
        )
        .await
        .map_err(diagnostics::sanitized)?;
    rpc_result(result, 721, method).map_err(diagnostics::sanitized)
}
// At most four independent reads per provider; no background tasks survive cancellation.
async fn batch<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    requests: Vec<(&str, Value)>,
) -> Result<Vec<Value>, ChainBaseError> {
    let mut output = Vec::with_capacity(requests.len());
    for chunk in requests.chunks(4) {
        let one = |i: usize| async move {
            match chunk.get(i) {
                Some((method, params)) => read(t, rpc, method, params.clone()).await.map(Some),
                None => Ok(None),
            }
        };
        let (a, b, c, d) = tokio::try_join!(one(0), one(1), one(2), one(3))?;
        output.extend([a, b, c, d].into_iter().flatten());
    }
    Ok(output)
}
fn q(value: &Value) -> Result<u128, ChainBaseError> {
    let s = value.as_str().ok_or_else(unavailable)?;
    let digits = s.strip_prefix("0x").ok_or_else(unavailable)?;
    if digits.is_empty()
        || digits.len() > 32
        || !digits.bytes().all(|c| c.is_ascii_hexdigit())
        || (digits.len() > 1 && digits.starts_with('0'))
    {
        return Err(unavailable());
    }
    u128::from_str_radix(digits, 16).map_err(|_| unavailable())
}
fn block(value: &Value) -> Result<BaseBlockIdentity, ChainBaseError> {
    Ok(BaseBlockIdentity {
        number: u64::try_from(q(&value["number"])?).map_err(|_| unavailable())?,
        hash: normalize_hash(value["hash"].as_str().ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?,
        timestamp: u64::try_from(q(&value["timestamp"])?).map_err(|_| unavailable())?,
    })
}
async fn word<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    to: &str,
    data: &str,
    tag: &str,
) -> Result<String, ChainBaseError> {
    let v = read(t, rpc, "eth_call", json!([{"to":to,"data":data},tag])).await?;
    let w = v.as_str().ok_or_else(unavailable)?;
    parse_bytes32(w).map_err(|_| unavailable())?;
    Ok(w.to_ascii_lowercase())
}
async fn scope_at<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    s: &TimeoutRelayScope,
    tag: &str,
    time: u64,
) -> Result<(), ChainBaseError> {
    let sig = if s.action == "expire_claim" {
        "claimExpiresAt()"
    } else {
        "verificationExpiresAt()"
    };
    let canonical = format!(
        "{}{:0>64}",
        selector("isCanonicalBounty(address)"),
        &s.bounty_contract[2..]
    );
    let mut requests = Vec::new();
    for name in [
        "status()",
        "round()",
        "factory()",
        "bountyId()",
        "termsHash()",
        sig,
    ] {
        requests.push((
            "eth_call",
            json!([{"to":s.bounty_contract,"data":selector(name)},tag]),
        ));
    }
    requests.push((
        "eth_call",
        json!([{"to":s.factory_contract,"data":canonical},tag]),
    ));
    let values = batch(t, rpc, requests).await?;
    let expected = [
        format!("0x{:064x}", if s.action == "expire_claim" { 2 } else { 3 }),
        format!("0x{:064x}", s.round),
        format!("0x{:0>64}", &s.factory_contract[2..]),
        s.bounty_id.clone(),
        s.terms_hash.clone(),
    ];
    for (value, expected) in values.iter().take(5).zip(expected) {
        if value != &expected {
            return Err(unavailable());
        }
    }
    let deadline =
        quantity(values[5].as_str().ok_or_else(unavailable)?).map_err(|_| unavailable())?;
    if deadline == 0
        || time <= deadline
        || quantity(values[6].as_str().ok_or_else(unavailable)?).map_err(|_| unavailable())? != 1
    {
        return Err(unavailable());
    }
    Ok(())
}
pub(super) async fn identity<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    s: &TimeoutRelayScope,
    p: &TimeoutRevalidationPolicy,
    tag: &str,
) -> Result<(), ChainBaseError> {
    let expected_clone = format!(
        "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
        &p.implementation_contract[2..]
    );
    let values = batch(
        t,
        rpc,
        vec![
            ("eth_getCode", json!([s.factory_contract, tag])),
            ("eth_getCode", json!([p.implementation_contract, tag])),
            ("eth_getCode", json!([s.bounty_contract, tag])),
            (
                "eth_call",
                json!([{"to":s.factory_contract,"data":selector("implementation()")},tag]),
            ),
        ],
    )
    .await?;
    for (value, expected) in values.iter().take(2).zip([
        &p.factory_runtime_code_hash,
        &p.implementation_runtime_code_hash,
    ]) {
        let bytes = parse_alloy_bytes(value.as_str().ok_or_else(unavailable)?)
            .map_err(|_| unavailable())?;
        if bytes.is_empty() || format!("{:#x}", alloy::primitives::keccak256(bytes)) != *expected {
            return Err(unavailable());
        }
    }
    if values[2] != expected_clone
        || values[3] != format!("0x{:0>64}", &p.implementation_contract[2..])
    {
        return Err(unavailable());
    }
    Ok(())
}
async fn absent_and_nonce<T: JsonRpcTransport + ?Sized>(
    t: &T,
    rpc: &str,
    saved: &PreparedBaseRelayTransaction,
    sender: &str,
) -> Result<(), ChainBaseError> {
    let values = batch(
        t,
        rpc,
        vec![
            (
                "eth_getTransactionByHash",
                json!([saved.transaction.tx_hash]),
            ),
            (
                "eth_getTransactionReceipt",
                json!([saved.transaction.tx_hash]),
            ),
            ("eth_getTransactionCount", json!([sender, "latest"])),
            ("eth_getTransactionCount", json!([sender, "pending"])),
        ],
    )
    .await?;
    if !values[0].is_null()
        || !values[1].is_null()
        || q(&values[2])? != u128::from(saved.nonce)
        || q(&values[3])? != u128::from(saved.nonce)
    {
        return Err(unavailable());
    }
    Ok(())
}

pub async fn revalidate_timeout_envelope(
    rpcs: [&str; 2],
    scope: &TimeoutRelayScope,
    saved: &PreparedBaseRelayTransaction,
    sender: &str,
    policy: &TimeoutRevalidationPolicy,
    rpc_timeout_seconds: u64,
) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
    if rpc_timeout_seconds == 0 {
        return Err(unavailable());
    }
    // No redirect may silently collapse the two independent provider routes.
    let transport = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(
        rpc_timeout_seconds.min(10),
    ))
    .map_err(diagnostics::sanitized)?;
    let transport = pacing::PacedReads::new(&transport, rpcs);
    match tokio::time::timeout(
        std::time::Duration::from_secs(rpc_timeout_seconds.min(60)),
        revalidate_timeout_envelope_with_transport(rpcs, scope, saved, sender, policy, &transport),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            checked(Phase::ReadDeadline, None, async {
                Err(ChainBaseError::RpcTransport(
                    "timeout revalidation read deadline elapsed".into(),
                ))
            })
            .await
        }
    }
}

pub async fn revalidate_timeout_envelope_with_transport<T: JsonRpcTransport + ?Sized>(
    rpcs: [&str; 2],
    scope: &TimeoutRelayScope,
    saved: &PreparedBaseRelayTransaction,
    sender: &str,
    policy: &TimeoutRevalidationPolicy,
    t: &T,
) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
    let provider_origin_hashes = origins(rpcs)?;
    validate_timeout_envelope(
        saved,
        scope,
        sender,
        policy.max_gas,
        policy.max_fee_per_gas_wei,
    )
    .map_err(|_| unavailable())?;
    if normalize_address(&policy.implementation_contract).map_err(|_| unavailable())?
        != policy.implementation_contract
        || normalize_hash(&policy.factory_runtime_code_hash).map_err(|_| unavailable())?
            != policy.factory_runtime_code_hash
        || normalize_hash(&policy.implementation_runtime_code_hash).map_err(|_| unavailable())?
            != policy.implementation_runtime_code_hash
        || saved.transaction.gas_limit == 0
        || saved.transaction.max_fee_per_gas_wei == 0
        || saved.transaction.max_priority_fee_per_gas_wei > saved.transaction.max_fee_per_gas_wei
    {
        return Err(unavailable());
    }
    let mut safe = Vec::new();
    for (index, rpc) in rpcs.into_iter().enumerate() {
        let slot = Some((index + 1) as u8);
        if q(&checked(
            Phase::ChainIdentity,
            slot,
            read(t, rpc, "eth_chainId", json!([])),
        )
        .await?)?
            != u128::from(base_network_descriptor(&scope.network)?.chain_id)
        {
            return Err(unavailable());
        }
        let b = block(
            &checked(
                Phase::SafeHead,
                slot,
                read(t, rpc, "eth_getBlockByNumber", json!(["safe", false])),
            )
            .await?,
        )?;
        fresh(&b, 600)?;
        safe.push(b);
    }
    let common = safe[0].number.min(safe[1].number);
    let tag = format!("0x{common:x}");
    let observe = |provider_index: usize| {
        let rpc = rpcs[provider_index];
        let slot = Some((provider_index + 1) as u8);
        let safe = &safe;
        let tag = &tag;
        async move {
            let canonical = block(
                &checked(
                    Phase::SafeBlock,
                    slot,
                    read(t, rpc, "eth_getBlockByNumber", json!([tag, false])),
                )
                .await?,
            )?;
            if canonical.number != common
                || (safe[provider_index].number == common && safe[provider_index] != canonical)
            {
                return Err(unavailable());
            }
            fresh(&canonical, 600)?;
            checked(
                Phase::SafeScope,
                slot,
                scope_at(t, rpc, scope, tag, canonical.timestamp),
            )
            .await?;
            checked(Phase::SafeCode, slot, identity(t, rpc, scope, policy, tag)).await?;
            let head_value = checked(
                Phase::LatestHead,
                slot,
                read(t, rpc, "eth_getBlockByNumber", json!(["latest", false])),
            )
            .await?;
            let head = block(&head_value)?;
            fresh(&head, 90)?;
            if head.number < common || head.timestamp < canonical.timestamp {
                return Err(unavailable());
            }
            let basefee = q(&head_value["baseFeePerGas"])?;
            if basefee
                .checked_add(saved.transaction.max_priority_fee_per_gas_wei)
                .ok_or_else(unavailable)?
                > saved.transaction.max_fee_per_gas_wei
            {
                return Err(unavailable());
            }
            // Repeat exact identity at latest and scope at pending to reject state changes after safe.
            let latest_tag = format!("0x{:x}", head.number);
            checked(
                Phase::LatestCode,
                slot,
                identity(t, rpc, scope, policy, &latest_tag),
            )
            .await?;
            checked(
                Phase::LatestScope,
                slot,
                scope_at(t, rpc, scope, &latest_tag, head.timestamp),
            )
            .await?;
            checked(
                Phase::PendingScope,
                slot,
                scope_at(t, rpc, scope, "pending", head.timestamp),
            )
            .await?;
            checked(
                Phase::TransactionAbsence,
                slot,
                absent_and_nonce(t, rpc, saved, sender),
            )
            .await?;
            let tx = json!({"from":sender,"to":scope.bounty_contract,"value":"0x0","data":scope.calldata()?,"nonce":format!("0x{:x}",saved.nonce),"gas":format!("0x{:x}",saved.transaction.gas_limit),"maxFeePerGas":format!("0x{:x}",saved.transaction.max_fee_per_gas_wei),"maxPriorityFeePerGas":format!("0x{:x}",saved.transaction.max_priority_fee_per_gas_wei),"type":"0x2","chainId":format!("0x{:x}",base_network_descriptor(&scope.network)?.chain_id)});
            if checked(
                Phase::Simulation,
                slot,
                read(t, rpc, "eth_call", json!([tx, "pending"])),
            )
            .await?
                != "0x"
            {
                return Err(unavailable());
            }
            let estimate = q(&checked(
                Phase::GasEstimate,
                slot,
                read(t, rpc, "eth_estimateGas", json!([tx, "pending"])),
            )
            .await?)?;
            if estimate == 0 || estimate > u128::from(saved.transaction.gas_limit) {
                return Err(unavailable());
            }
            let mut cost = u128::from(saved.transaction.gas_limit)
                .checked_mul(saved.transaction.max_fee_per_gas_wei)
                .ok_or_else(unavailable)?;
            for (sig, arg) in [
                (
                    "getL1FeeUpperBound(uint256)",
                    parse_alloy_bytes(&saved.signed_transaction)
                        .map_err(|_| unavailable())?
                        .len() as u128,
                ),
                (
                    "getOperatorFee(uint256)",
                    u128::from(saved.transaction.gas_limit),
                ),
            ] {
                let data = format!("{}{:064x}", selector(sig), arg);
                let w = checked(
                    Phase::FeeBounds,
                    slot,
                    word(
                        t,
                        rpc,
                        "0x420000000000000000000000000000000000000f",
                        &data,
                        "pending",
                    ),
                )
                .await?;
                let fee = u128::try_from(U256::from_be_slice(
                    &parse_bytes32(&w).map_err(|_| unavailable())?,
                ))
                .map_err(|_| unavailable())?;
                cost = cost
                    .checked_add(fee.checked_mul(2).ok_or_else(unavailable)?)
                    .ok_or_else(unavailable)?;
            }
            if q(&checked(
                Phase::Balance,
                slot,
                read(t, rpc, "eth_getBalance", json!([sender, "pending"])),
            )
            .await?)?
                < cost.max(saved.transaction.estimated_max_cost_wei)
            {
                return Err(unavailable());
            }
            let checked_block = block(
                &checked(
                    Phase::SafeBlockRecheck,
                    slot,
                    read(t, rpc, "eth_getBlockByNumber", json!([tag, false])),
                )
                .await?,
            )?;
            let checked_head = block(
                &checked(
                    Phase::LatestBlockRecheck,
                    slot,
                    read(t, rpc, "eth_getBlockByNumber", json!([latest_tag, false])),
                )
                .await?,
            )?;
            if checked_block != canonical || checked_head != head {
                return Err(unavailable());
            }
            fresh(&head, 90)?;
            checked(
                Phase::FinalTransactionAbsence,
                slot,
                absent_and_nonce(t, rpc, saved, sender),
            )
            .await?;
            Ok::<_, ChainBaseError>((canonical, head, estimate as u64, cost))
        }
    };
    let (a, b) = tokio::try_join!(observe(0), observe(1))?;
    if a.0 != b.0 || (a.1.number == b.1.number && a.1 != b.1) {
        return Err(unavailable());
    }
    fresh(&a.1, 90)?;
    fresh(&b.1, 90)?;
    Ok(TimeoutRevalidationObservation {
        observed_at: Utc::now(),
        common_safe_block: a.0,
        provider_origin_hashes,
        latest_blocks: [a.1, b.1],
        estimated_gas: [a.2, b.2],
        maximum_fee_wei: a.3.max(b.3).max(saved.transaction.estimated_max_cost_wei),
        transaction_hash: saved.transaction.tx_hash.clone(),
        nonce: saved.nonce,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        eips::Encodable2718,
        network::{EthereumWallet, TransactionBuilder},
        rpc::types::TransactionRequest,
        signers::local::PrivateKeySigner,
    };
    use std::sync::Mutex;
    struct Fixture {
        scope: TimeoutRelayScope,
        saved: PreparedBaseRelayTransaction,
        policy: TimeoutRevalidationPolicy,
        now: u64,
        fault: &'static str,
        seen: Mutex<Vec<Value>>,
    }
    fn w(n: u64) -> String {
        format!("0x{n:064x}")
    }
    impl Fixture {
        async fn new(fault: &'static str) -> Self {
            let scope = TimeoutRelayScope {
                network: "base-mainnet".into(),
                bounty_contract: format!("0x{}", "11".repeat(20)),
                factory_contract: format!("0x{}", "22".repeat(20)),
                bounty_id: w(3),
                terms_hash: w(4),
                action: "expire_claim".into(),
                round: 7,
            };
            let signer = PrivateKeySigner::random();
            let sender = format!("{:#x}", signer.address());
            let envelope = TransactionRequest::default()
                .with_to(scope.bounty_contract.parse().unwrap())
                .with_value(U256::ZERO)
                .with_input(parse_alloy_bytes(scope.calldata().unwrap()).unwrap())
                .with_nonce(9)
                .with_chain_id(8453)
                .with_gas_limit(25000)
                .with_max_fee_per_gas(3)
                .with_max_priority_fee_per_gas(1)
                .build(&EthereumWallet::from(signer))
                .await
                .unwrap();
            let saved = PreparedBaseRelayTransaction {
                nonce: 9,
                signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
                transaction: BaseRelayedTransaction {
                    relayer: sender,
                    tx_hash: format!("{:#x}", envelope.tx_hash()),
                    estimated_gas: 20000,
                    gas_limit: 25000,
                    max_fee_per_gas_wei: 3,
                    max_priority_fee_per_gas_wei: 1,
                    estimated_max_cost_wei: 75000,
                },
            };
            let policy = TimeoutRevalidationPolicy {
                factory_runtime_code_hash: format!(
                    "{:#x}",
                    alloy::primitives::keccak256([0x60, 0])
                ),
                implementation_contract: format!("0x{}", "55".repeat(20)),
                implementation_runtime_code_hash: format!(
                    "{:#x}",
                    alloy::primitives::keccak256([0x60, 1])
                ),
                max_gas: 30000,
                max_fee_per_gas_wei: 5,
            };
            Self {
                scope,
                saved,
                policy,
                now: clock().unwrap(),
                fault,
                seen: Mutex::new(vec![]),
            }
        }
        async fn run(&self) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
            revalidate_timeout_envelope_with_transport(
                [
                    "https://a.example/key-secret",
                    "https://b.example/key-secret",
                ],
                &self.scope,
                &self.saved,
                &self.saved.transaction.relayer,
                &self.policy,
                self,
            )
            .await
        }
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for Fixture {
        async fn post_json_value(&self, rpc: &str, r: &Value) -> Result<Value, ChainBaseError> {
            self.seen.lock().unwrap().push(r.clone());
            let method = r["method"].as_str().unwrap();
            let second = rpc.contains("b.example");
            assert!(!matches!(
                method,
                "eth_sendRawTransaction" | "eth_signTransaction"
            ));
            if self.fault == "provider" && second {
                return Err(ChainBaseError::RpcTransport("secret-url-response".into()));
            }
            if self.fault == "rate_limited" && second {
                return Err(ChainBaseError::RpcHttpStatus(429));
            }
            if self.fault == "retry_after" && second {
                return Err(ChainBaseError::RpcRetryAfter {
                    status: 503,
                    retry_after_seconds: 37,
                });
            }
            if self.fault == "rpc_error" && second {
                return Ok(
                    json!({"jsonrpc":"2.0","id":721,"error":{"code":-32000,"message":"secret"}}),
                );
            }
            let result = match method {
                "eth_chainId" => json!(if self.fault == "chain" && second {
                    "0x1"
                } else {
                    "0x2105"
                }),
                "eth_getBlockByNumber" => {
                    json!({"number":"0x64","hash":w(if (self.fault=="fork"&&second) || (self.fault=="reorg" && self.seen.lock().unwrap().iter().filter(|v|v["method"]=="eth_getBlockByNumber").count()>=5) {101}else{100}),"timestamp":format!("0x{:x}",self.now-if self.fault=="stale" {700}else{0}),"baseFeePerGas":if self.fault=="fee" {"0x3"}else{"0x1"}})
                }
                "eth_getTransactionByHash" | "eth_getTransactionReceipt" => {
                    if (self.fault == "hash" && method == "eth_getTransactionByHash")
                        || (self.fault == "receipt" && method == "eth_getTransactionReceipt")
                    {
                        json!({"hash":self.saved.transaction.tx_hash})
                    } else {
                        Value::Null
                    }
                }
                "eth_getTransactionCount" => {
                    json!(if self.fault == "nonce" && r["params"][1] == "pending" {
                        "0xa"
                    } else {
                        "0x9"
                    })
                }
                "eth_getBalance" => json!(if self.fault == "balance" {
                    "0x1"
                } else {
                    "0xffffff"
                }),
                "eth_estimateGas" => json!(if self.fault == "estimate" {
                    "0x100000"
                } else {
                    "0x4e20"
                }),
                "eth_getCode" => {
                    let to = r["params"][0].as_str().unwrap();
                    if self.fault == "code" && second {
                        json!("0x")
                    } else if to == self.scope.factory_contract {
                        json!("0x6000")
                    } else if to == self.policy.implementation_contract {
                        json!("0x6001")
                    } else {
                        json!(format!(
                            "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
                            &self.policy.implementation_contract[2..]
                        ))
                    }
                }
                "eth_call" => {
                    let data = r["params"][0]["data"].as_str().unwrap();
                    if data == self.scope.calldata().unwrap() {
                        assert_eq!(r["params"][1], "pending");
                        assert_eq!(r["params"][0]["from"], self.saved.transaction.relayer);
                        assert_eq!(r["params"][0]["gas"], "0x61a8");
                        if self.fault == "revert" {
                            return Ok(
                                json!({"jsonrpc":"2.0","id":721,"error":{"code":3,"message":"revert"}}),
                            );
                        }
                        json!(if self.fault == "simulation" {
                            "0x01"
                        } else {
                            "0x"
                        })
                    } else {
                        let x = if data == selector("status()") {
                            w(if self.fault == "status" { 1 } else { 2 })
                        } else if data == selector("round()") {
                            w(if self.fault == "round" && r["params"][1] == "pending" {
                                8
                            } else {
                                7
                            })
                        } else if data == selector("factory()") {
                            if self.fault == "factory" {
                                w(99)
                            } else {
                                format!("0x{:0>64}", &self.scope.factory_contract[2..])
                            }
                        } else if data == selector("bountyId()") {
                            if self.fault == "bounty_id" {
                                w(99)
                            } else {
                                self.scope.bounty_id.clone()
                            }
                        } else if data == selector("termsHash()") {
                            if self.fault == "terms" {
                                w(99)
                            } else {
                                self.scope.terms_hash.clone()
                            }
                        } else if data == selector("implementation()") {
                            if self.fault == "implementation" {
                                w(99)
                            } else {
                                format!("0x{:0>64}", &self.policy.implementation_contract[2..])
                            }
                        } else if data == selector("claimExpiresAt()") {
                            w(if self.fault == "deadline" {
                                self.now
                            } else {
                                self.now - 1
                            })
                        } else if data.starts_with(&selector("isCanonicalBounty(address)")) {
                            w(u64::from(self.fault != "canonical"))
                        } else if data.starts_with(&selector("getL1FeeUpperBound(uint256)"))
                            || data.starts_with(&selector("getOperatorFee(uint256)"))
                        {
                            if self.fault == "oracle" {
                                "0x".into()
                            } else {
                                w(if second { 10 } else { 5 })
                            }
                        } else {
                            panic!("unexpected call {data}")
                        };
                        json!(x)
                    }
                }
                other => panic!("unexpected method {other}"),
            };
            Ok(
                json!({"jsonrpc":"2.0","id":if self.fault=="wrong_id"{999}else{721},"result":result}),
            )
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_accepts_exact_readonly_evidence_and_padded_current_fees() {
        let f = Fixture::new("").await;
        let o = f.run().await.unwrap();
        assert_eq!(o.maximum_fee_wei, 75040);
        assert_eq!(o.estimated_gas, [20000, 20000]);
        assert_eq!(o.nonce, 9);
        assert_eq!(o.common_safe_block.number, 100);
        assert_ne!(o.provider_origin_hashes[0], o.provider_origin_hashes[1]);
        assert!(!serde_json::to_string(&o).unwrap().contains("secret"));
    }
    #[tokio::test]
    async fn timeout_revalidation_all_uncertain_controls_make_zero_broadcasts() {
        for fault in [
            "provider",
            "rpc_error",
            "chain",
            "fork",
            "stale",
            "reorg",
            "fee",
            "hash",
            "receipt",
            "nonce",
            "balance",
            "estimate",
            "code",
            "revert",
            "simulation",
            "status",
            "round",
            "terms",
            "deadline",
            "canonical",
            "oracle",
            "wrong_id",
            "factory",
            "bounty_id",
            "implementation",
        ] {
            let f = Fixture::new(fault).await;
            let error = f.run().await.expect_err(fault);
            assert!(!error.to_string().contains("secret"));
            assert!(f
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|r| r["method"] != "eth_sendRawTransaction"));
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_rejects_alias_routes_and_changed_envelopes_before_reads() {
        let mut f = Fixture::new("").await;
        for routes in [
            ["https://a.example/a", "https://a.example/b"],
            ["https://a.example", "https://a.example:444"],
            ["http://a.example", "https://b.example"],
            ["https://user:secret@a.example", "https://b.example"],
        ] {
            assert!(revalidate_timeout_envelope_with_transport(
                routes,
                &f.scope,
                &f.saved,
                &f.saved.transaction.relayer,
                &f.policy,
                &f
            )
            .await
            .is_err());
        }
        f.saved.transaction.gas_limit += 1;
        assert!(f.run().await.is_err());
        assert!(f.seen.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn timeout_revalidation_preserves_actual_provider_failure_class_without_raw_content() {
        for fault in ["provider", "rpc_error", "rate_limited", "retry_after"] {
            let f = Fixture::new(fault).await;
            let error = f.run().await.unwrap_err();
            assert!(!error.to_string().contains("secret"));
            match fault {
                "provider" => assert!(matches!(error, ChainBaseError::RpcTransport(_))),
                "rpc_error" => assert!(matches!(
                    error,
                    ChainBaseError::RpcProviderError { code: -32000, .. }
                )),
                "rate_limited" => assert!(matches!(error, ChainBaseError::RpcHttpStatus(429))),
                _ => assert!(matches!(
                    error,
                    ChainBaseError::RpcRetryAfter {
                        status: 503,
                        retry_after_seconds: 37
                    }
                )),
            }
            assert!(f
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|r| r["method"] != "eth_sendRawTransaction"));
        }
    }

    // A synthetic budget proves the mechanism, not any live provider's quota.
    // All real checker calls still pass through the existing fixture and pacer.
    struct SharedWindow<'a> {
        fixture: &'a Fixture,
        seen: Mutex<BTreeMap<String, std::collections::VecDeque<tokio::time::Instant>>>,
        admitted: Mutex<usize>,
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for SharedWindow<'_> {
        async fn post_json_value(
            &self,
            rpc: &str,
            request: &Value,
        ) -> Result<Value, ChainBaseError> {
            {
                let mut seen = self.seen.lock().unwrap();
                let window = seen.entry(rpc.into()).or_default();
                let now = tokio::time::Instant::now();
                while window.front().is_some_and(|at| {
                    now.duration_since(*at) >= std::time::Duration::from_millis(80)
                }) {
                    window.pop_front();
                }
                if !window.is_empty() {
                    return Err(ChainBaseError::RpcHttpStatus(429));
                }
                window.push_back(now);
                *self.admitted.lock().unwrap() += 1;
            }
            self.fixture.post_json_value(rpc, request).await
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_independent_pacers_do_not_reserve_aggregate_provider_capacity() {
        let f = Fixture::new("").await;
        let provider = SharedWindow {
            fixture: &f,
            seen: Mutex::new(BTreeMap::new()),
            admitted: Mutex::new(0),
        };
        let routes = ["https://a.example", "https://b.example"];
        let sibling = pacing::PacedReads::new(&provider, routes);
        let recovery = pacing::PacedReads::new(&provider, routes);
        // Sibling consumes the same provider window just before recovery starts.
        read(&sibling, routes[0], "eth_chainId", json!([]))
            .await
            .unwrap();
        let result = revalidate_timeout_envelope_with_transport(
            routes,
            &f.scope,
            &f.saved,
            &f.saved.transaction.relayer,
            &f.policy,
            &recovery,
        )
        .await;
        assert!(matches!(result, Err(ChainBaseError::RpcHttpStatus(429))));
        assert_eq!(*provider.admitted.lock().unwrap(), 1);
    }
    #[tokio::test]
    async fn timeout_revalidation_shared_pacer_preserves_both_checks_and_sibling_reads_inside_lease(
    ) {
        let f = Fixture::new("").await;
        let provider = SharedWindow {
            fixture: &f,
            seen: Mutex::new(BTreeMap::new()),
            admitted: Mutex::new(0),
        };
        let routes = ["https://a.example", "https://b.example"];
        let shared = pacing::PacedReads::new(&provider, routes);
        let recovery = async {
            for _ in 0..2 {
                tokio::time::timeout(
                    std::time::Duration::from_secs(13),
                    revalidate_timeout_envelope_with_transport(
                        routes,
                        &f.scope,
                        &f.saved,
                        &f.saved.transaction.relayer,
                        &f.policy,
                        &shared,
                    ),
                )
                .await
                .unwrap()?;
            }
            Ok::<_, ChainBaseError>(())
        };
        let sibling = async {
            for _ in 0..10 {
                read(&shared, routes[0], "eth_chainId", json!([])).await?;
            }
            Ok::<_, ChainBaseError>(())
        };
        tokio::time::timeout(std::time::Duration::from_secs(42), async {
            tokio::try_join!(recovery, sibling)
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            *provider.admitted.lock().unwrap(),
            202,
            "two complete 96-read checks plus ten sibling reads"
        );
        assert_eq!(f.seen.lock().unwrap().len(), 202);
        assert!(f
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|r| r["method"] != "eth_sendRawTransaction"));
    }
    struct Counted<'a> {
        fixture: &'a Fixture,
        calls: Mutex<Vec<(String, tokio::time::Instant)>>,
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for Counted<'_> {
        async fn post_json_value(&self, rpc: &str, r: &Value) -> Result<Value, ChainBaseError> {
            self.calls
                .lock()
                .unwrap()
                .push((rpc.into(), tokio::time::Instant::now()));
            self.fixture.post_json_value(rpc, r).await
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_paced_complete_check_fits_default_stage_without_omitting_reads() {
        let f = Fixture::new("").await;
        let recorded = Counted {
            fixture: &f,
            calls: Mutex::new(vec![]),
        };
        let endpoints = ["https://a.example", "https://b.example"];
        let paced = pacing::PacedReads::new(&recorded, endpoints);
        tokio::time::timeout(
            std::time::Duration::from_secs(13),
            revalidate_timeout_envelope_with_transport(
                endpoints,
                &f.scope,
                &f.saved,
                &f.saved.transaction.relayer,
                &f.policy,
                &paced,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let calls = recorded.calls.lock().unwrap();
        for endpoint in endpoints {
            let times: Vec<_> = calls
                .iter()
                .filter(|(url, _)| url == endpoint)
                .map(|(_, at)| *at)
                .collect();
            assert_eq!(times.len(), 48, "all independent chain checks must remain");
            assert!(times.windows(2).all(
                |pair| pair[1].duration_since(pair[0]) >= std::time::Duration::from_millis(100)
            ));
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_paced_stage_cancellation_has_no_later_dispatch() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let f = Fixture::new("").await;
        let slow = Slow {
            fixture: &f,
            active: [AtomicUsize::new(0), AtomicUsize::new(0)],
            maximum: [AtomicUsize::new(0), AtomicUsize::new(0)],
        };
        let endpoints = ["https://a.example", "https://b.example"];
        let paced = pacing::PacedReads::new(&slow, endpoints);
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(1),
            revalidate_timeout_envelope_with_transport(
                endpoints,
                &f.scope,
                &f.saved,
                &f.saved.transaction.relayer,
                &f.policy,
                &paced
            )
        )
        .await
        .is_err());
        assert!(slow.maximum[0].load(SeqCst) > 0);
        assert_eq!(slow.active.each_ref().map(|v| v.load(SeqCst)), [0, 0]);
        let completed = f.seen.lock().unwrap().len();
        tokio::time::sleep(std::time::Duration::from_millis(110)).await;
        assert_eq!(f.seen.lock().unwrap().len(), completed);
    }

    struct Slow<'a> {
        fixture: &'a Fixture,
        active: [std::sync::atomic::AtomicUsize; 2],
        maximum: [std::sync::atomic::AtomicUsize; 2],
    }
    struct InFlight<'a>(&'a std::sync::atomic::AtomicUsize);
    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for Slow<'_> {
        async fn post_json_value(&self, rpc: &str, r: &Value) -> Result<Value, ChainBaseError> {
            use std::sync::atomic::Ordering::SeqCst;
            let i = usize::from(rpc.contains("b.example"));
            let active = self.active[i].fetch_add(1, SeqCst) + 1;
            let _guard = InFlight(&self.active[i]);
            self.maximum[i].fetch_max(active, SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            self.fixture.post_json_value(rpc, r).await
        }
    }
    #[tokio::test]
    async fn timeout_revalidation_parallel_reads_are_bounded_and_cancel_without_background_work() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let f = Fixture::new("").await;
        let t = Slow {
            fixture: &f,
            active: [AtomicUsize::new(0), AtomicUsize::new(0)],
            maximum: [AtomicUsize::new(0), AtomicUsize::new(0)],
        };
        revalidate_timeout_envelope_with_transport(
            ["https://a.example", "https://b.example"],
            &f.scope,
            &f.saved,
            &f.saved.transaction.relayer,
            &f.policy,
            &t,
        )
        .await
        .unwrap();
        assert_eq!(t.maximum.each_ref().map(|v| v.load(SeqCst)), [4, 4]);
        assert_eq!(t.active.each_ref().map(|v| v.load(SeqCst)), [0, 0]);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(1),
            revalidate_timeout_envelope_with_transport(
                ["https://a.example", "https://b.example"],
                &f.scope,
                &f.saved,
                &f.saved.transaction.relayer,
                &f.policy,
                &t,
            ),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(t.active.each_ref().map(|v| v.load(SeqCst)), [0, 0]);
    }
}
