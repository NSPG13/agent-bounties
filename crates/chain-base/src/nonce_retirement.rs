//! Bounded replacement of an obsolete, never-attempted timeout envelope.
//! All checks are read-only. Signing is separate and requires a journaled owner approval.
use super::*;
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::Decodable2718,
};
use timeout_recovery::{validate_timeout_envelope, TimeoutRelayScope, TimeoutRevalidationPolicy};

pub mod retained_dispatch;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NonceRetirementPlan {
    pub operation_id: Uuid,
    pub original_tx_hash: String,
    pub original_envelope_sha256: String,
    pub sender: String,
    pub chain_id: u64,
    pub nonce: u64,
    pub gas_limit: u64,
    pub max_fee_per_gas_wei: u128,
    pub max_priority_fee_per_gas_wei: u128,
    pub maximum_cost_wei: u128,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
impl NonceRetirementPlan {
    pub fn digest(&self) -> Result<String, ChainBaseError> {
        self.validate()?;
        let mut bytes = b"agent-bounties/nonce-retirement-v1\0".to_vec();
        bytes.extend(serde_json::to_vec(self).map_err(|_| denied())?);
        Ok(format!("0x{}", hex::encode(keccak_word(&bytes))))
    }
    pub fn validate(&self) -> Result<(), ChainBaseError> {
        if self.chain_id != 8453
            || self.operation_id.is_nil()
            || normalize_address(&self.sender)? != self.sender
            || normalize_hash(&self.original_tx_hash)? != self.original_tx_hash
            || self.original_envelope_sha256.len() != 64
            || !self
                .original_envelope_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || self.gas_limit != 25_200
            || self.max_fee_per_gas_wei == 0
            || self.max_priority_fee_per_gas_wei > self.max_fee_per_gas_wei
            || self.maximum_cost_wei
                <= u128::from(self.gas_limit)
                    .checked_mul(self.max_fee_per_gas_wei)
                    .ok_or_else(denied)?
            || self.expires_at <= self.created_at
            || self.expires_at - self.created_at > chrono::Duration::minutes(15)
        {
            return Err(denied());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceRetirementObservation {
    pub observed_at: DateTime<Utc>,
    pub safe_block: BaseBlockIdentity,
    pub finalized_block: BaseBlockIdentity,
    pub provider_hashes: [String; 2],
    pub maximum_cost_wei: u128,
}
fn denied() -> ChainBaseError {
    ChainBaseError::InvalidRelayIntent(
        "nonce retirement evidence unavailable or inconsistent".into(),
    )
}
fn require(b: bool) -> Result<(), ChainBaseError> {
    if b {
        Ok(())
    } else {
        Err(denied())
    }
}
fn q(v: &Value) -> Result<u128, ChainBaseError> {
    let s = v
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .ok_or_else(denied)?;
    require(
        !s.is_empty()
            && s.len() <= 32
            && (s.len() == 1 || !s.starts_with('0'))
            && s.bytes().all(|c| c.is_ascii_hexdigit()),
    )?;
    u128::from_str_radix(s, 16).map_err(|_| denied())
}
fn block(v: &Value) -> Result<BaseBlockIdentity, ChainBaseError> {
    Ok(BaseBlockIdentity {
        number: u64::try_from(q(&v["number"])?).map_err(|_| denied())?,
        hash: normalize_hash(v["hash"].as_str().ok_or_else(denied)?)?,
        timestamp: u64::try_from(q(&v["timestamp"])?).map_err(|_| denied())?,
    })
}
fn fresh(b: &BaseBlockIdentity, now: u64, age: u64) -> Result<(), ChainBaseError> {
    require(b.timestamp <= now && now - b.timestamp <= age)
}
async fn read<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, ChainBaseError> {
    require(matches!(
        method,
        "eth_chainId"
            | "eth_getBlockByNumber"
            | "eth_getTransactionCount"
            | "eth_getTransactionByHash"
            | "eth_getTransactionReceipt"
            | "eth_getCode"
            | "eth_getBalance"
            | "eth_call"
            | "eth_estimateGas"
            | "eth_gasPrice"
    ))?;
    let value = t
        .post_json_value(
            url,
            &json!({"jsonrpc":"2.0","id":7781,"method":method,"params":params}),
        )
        .await
        .map_err(|_| {
            ChainBaseError::RelayerProvider("nonce retirement provider read unavailable".into())
        })?;
    rpc_result(value, 7781, method).map_err(|_| denied())
}
fn providers(urls: [&str; 2]) -> Result<[String; 2], ChainBaseError> {
    let a = reqwest::Url::parse(urls[0]).map_err(|_| denied())?;
    let b = reqwest::Url::parse(urls[1]).map_err(|_| denied())?;
    require(
        a.scheme() == "https"
            && b.scheme() == "https"
            && a.host_str().is_some()
            && b.host_str().is_some()
            && a.host_str() != b.host_str(),
    )?;
    Ok([
        hex::encode(Sha256::digest(urls[0].as_bytes())),
        hex::encode(Sha256::digest(urls[1].as_bytes())),
    ])
}
fn bind(
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
) -> Result<(), ChainBaseError> {
    plan.validate()?;
    validate_timeout_envelope(
        saved,
        scope,
        &plan.sender,
        policy.max_gas,
        policy.max_fee_per_gas_wei,
    )?;
    require(
        scope.network == "base-mainnet"
            && scope.operation_id()? == plan.operation_id
            && plan.original_tx_hash == saved.transaction.tx_hash
            && plan.nonce == saved.nonce
            && plan.original_envelope_sha256
                == hex::encode(Sha256::digest(parse_alloy_bytes(
                    &saved.signed_transaction,
                )?))
            && plan.gas_limit <= policy.max_gas
            && plan.max_fee_per_gas_wei <= policy.max_fee_per_gas_wei,
    )
}
fn self_call(plan: &NonceRetirementPlan) -> Value {
    json!({"from":plan.sender,"to":plan.sender,"value":"0x0","data":"0x","nonce":hex_quantity(plan.nonce)})
}

/// Quote is unsigned. Approval must name the resulting digest, maximum cost and expiry.
pub async fn quote_nonce_retirement(
    urls: [&str; 2],
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    seconds: u64,
) -> Result<(NonceRetirementPlan, NonceRetirementObservation), ChainBaseError> {
    let t = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(3))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(seconds.min(90)),
        quote_with_transport(urls, saved, scope, policy, Utc::now(), &t),
    )
    .await
    .map_err(|_| denied())?
}
pub async fn quote_with_transport<T: JsonRpcTransport + ?Sized>(
    urls: [&str; 2],
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    now: DateTime<Utc>,
    t: &T,
) -> Result<(NonceRetirementPlan, NonceRetirementObservation), ChainBaseError> {
    providers(urls)?;
    let started = std::time::Instant::now();
    let mut fee = saved
        .transaction
        .max_fee_per_gas_wei
        .checked_mul(120)
        .and_then(|v| v.checked_add(99))
        .map(|v| v / 100)
        .ok_or_else(denied)?;
    let mut tip = saved
        .transaction
        .max_priority_fee_per_gas_wei
        .checked_mul(120)
        .and_then(|v| v.checked_add(99))
        .map(|v| v / 100)
        .ok_or_else(denied)?;
    for url in urls {
        let price = q(&read(t, url, "eth_gasPrice", json!([])).await?)?;
        let latest = read(t, url, "eth_getBlockByNumber", json!(["latest", false])).await?;
        fresh(
            &block(&latest)?,
            now.timestamp() as u64 + started.elapsed().as_secs(),
            90,
        )?;
        fee = fee.max(price.checked_mul(2).ok_or_else(denied)?);
        tip = tip.max(price.saturating_sub(q(&latest["baseFeePerGas"])?));
    }
    let mut plan = NonceRetirementPlan {
        operation_id: scope.operation_id()?,
        original_tx_hash: saved.transaction.tx_hash.clone(),
        original_envelope_sha256: hex::encode(Sha256::digest(parse_alloy_bytes(
            &saved.signed_transaction,
        )?)),
        sender: saved.transaction.relayer.clone(),
        chain_id: 8453,
        nonce: saved.nonce,
        gas_limit: 25_200,
        max_fee_per_gas_wei: fee,
        max_priority_fee_per_gas_wei: tip,
        maximum_cost_wei: u128::MAX,
        created_at: now,
        expires_at: now + chrono::Duration::minutes(15),
    };
    bind(&plan, saved, scope, policy)?;
    let observation = check_with_transport(
        urls,
        &plan,
        saved,
        scope,
        policy,
        now + chrono::Duration::from_std(started.elapsed()).map_err(|_| denied())?,
        t,
    )
    .await?;
    plan.maximum_cost_wei = observation
        .maximum_cost_wei
        .checked_mul(2)
        .ok_or_else(denied)?;
    plan.validate()?;
    Ok((plan, observation))
}
/// Repeat immediately before signing and once more before the durable send marker.
pub async fn check_nonce_retirement(
    urls: [&str; 2],
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    seconds: u64,
) -> Result<NonceRetirementObservation, ChainBaseError> {
    let t = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(3))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(seconds.min(90)),
        check_with_transport(urls, plan, saved, scope, policy, Utc::now(), &t),
    )
    .await
    .map_err(|_| denied())?
}
pub async fn check_with_transport<T: JsonRpcTransport + ?Sized>(
    urls: [&str; 2],
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    now: DateTime<Utc>,
    t: &T,
) -> Result<NonceRetirementObservation, ChainBaseError> {
    require(now >= plan.created_at && now < plan.expires_at)?;
    check_evidence_with_transport(urls, plan, saved, scope, policy, now, plan.expires_at, t).await
}

// Temporal authority is supplied only by the ordinary live plan or the
// independently validated retained-dispatch proposal. Historical plan fields
// and every chain predicate remain unchanged in either case.
#[allow(clippy::too_many_arguments)]
async fn check_evidence_with_transport<T: JsonRpcTransport + ?Sized>(
    urls: [&str; 2],
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    now: DateTime<Utc>,
    authority_expires_at: DateTime<Utc>,
    t: &T,
) -> Result<NonceRetirementObservation, ChainBaseError> {
    bind(plan, saved, scope, policy)?;
    require(now.timestamp() >= 0 && now < authority_expires_at)?;
    let hashes = providers(urls)?;
    let started = std::time::Instant::now();
    let clock = || now.timestamp() as u64 + started.elapsed().as_secs();
    let mut heads = Vec::new();
    for url in urls {
        require(q(&read(t, url, "eth_chainId", json!([])).await?)? == 8453)?;
        let safe = block(&read(t, url, "eth_getBlockByNumber", json!(["safe", false])).await?)?;
        let finalized =
            block(&read(t, url, "eth_getBlockByNumber", json!(["finalized", false])).await?)?;
        fresh(&safe, clock(), 600)?;
        fresh(&finalized, clock(), 1800)?;
        require(finalized.number <= safe.number)?;
        heads.push((safe, finalized));
    }
    let safe_tag = hex_quantity(heads.iter().map(|h| h.0.number).min().ok_or_else(denied)?);
    let final_tag = hex_quantity(heads.iter().map(|h| h.1.number).min().ok_or_else(denied)?);
    let inspect = async |url| {
        let safe = block(&read(t, url, "eth_getBlockByNumber", json!([safe_tag, false])).await?)?;
        let finalized =
            block(&read(t, url, "eth_getBlockByNumber", json!([final_tag, false])).await?)?;
        fresh(&safe, clock(), 600)?;
        fresh(&finalized, clock(), 1800)?;
        let latest = read(t, url, "eth_getBlockByNumber", json!(["latest", false])).await?;
        fresh(&block(&latest)?, clock(), 90)?;
        require(
            q(&latest["baseFeePerGas"])?
                .checked_add(plan.max_priority_fee_per_gas_wei)
                .ok_or_else(denied)?
                <= plan.max_fee_per_gas_wei,
        )?;
        require(
            safe.number == q(&json!(safe_tag))? as u64
                && finalized.number == q(&json!(final_tag))? as u64
                && finalized.number <= safe.number
                && safe.number <= block(&latest)?.number,
        )?;
        for tag in [safe_tag.as_str(), final_tag.as_str(), "latest", "pending"] {
            require(
                q(&read(t, url, "eth_getTransactionCount", json!([plan.sender, tag])).await?)?
                    == u128::from(plan.nonce),
            )?;
            require(read(t, url, "eth_getCode", json!([plan.sender, tag])).await? == json!("0x"))?;
        }
        require(
            read(
                t,
                url,
                "eth_getTransactionByHash",
                json!([plan.original_tx_hash]),
            )
            .await?
            .is_null(),
        )?;
        require(
            read(
                t,
                url,
                "eth_getTransactionReceipt",
                json!([plan.original_tx_hash]),
            )
            .await?
            .is_null(),
        )?;
        // Retirement never invokes the old calldata. Its exact old round must already be obsolete.
        for tag in [final_tag.as_str(), "pending"] {
            timeout_recovery::retirement_identity(t, url, scope, policy, tag).await?;
            let call = |sig: &str| json!([{"to":scope.bounty_contract,"data":format!("0x{}",hex::encode(&keccak_word(sig.as_bytes())[..4]))},tag]);
            for (sig, expected) in [
                (
                    "factory()",
                    format!("0x{:0>64}", &scope.factory_contract[2..]),
                ),
                ("bountyId()", scope.bounty_id.clone()),
                ("termsHash()", scope.terms_hash.clone()),
            ] {
                require(read(t, url, "eth_call", call(sig)).await? == json!(expected))?;
            }
            let round = parse_bytes32(
                read(t, url, "eth_call", call("round()"))
                    .await?
                    .as_str()
                    .ok_or_else(denied)?,
            )?;
            let status = parse_bytes32(
                read(t, url, "eth_call", call("status()"))
                    .await?
                    .as_str()
                    .ok_or_else(denied)?,
            )?;
            let round = U256::from_be_slice(&round);
            let status = U256::from_be_slice(&status);
            let active = if scope.action == "expire_claim" {
                U256::from(2)
            } else {
                U256::from(3)
            };
            require(
                round >= U256::from(scope.round)
                    && (round != U256::from(scope.round) || status != active),
            )?;
        }
        require(
            read(t, url, "eth_call", json!([self_call(plan), "pending"])).await? == json!("0x"),
        )?;
        require(
            q(&read(
                t,
                url,
                "eth_estimateGas",
                json!([self_call(plan), "pending"]),
            )
            .await?)?
                == 21_000,
        )?;
        let mut cost = u128::from(plan.gas_limit)
            .checked_mul(plan.max_fee_per_gas_wei)
            .ok_or_else(denied)?;
        for (signature, arg) in [
            ("getL1FeeUpperBound(uint256)", 350u128),
            ("getOperatorFee(uint256)", u128::from(plan.gas_limit)),
        ] {
            let mut data = keccak_word(signature.as_bytes())[..4].to_vec();
            data.extend_from_slice(&encode_uint256(arg)?);
            let v=read(t,url,"eth_call",json!([{"to":"0x420000000000000000000000000000000000000f","data":format!("0x{}",hex::encode(data))},"pending"])).await?;
            let fee = u128::try_from(U256::from_be_slice(&parse_bytes32(
                v.as_str().ok_or_else(denied)?,
            )?))
            .map_err(|_| denied())?;
            cost = cost
                .checked_add(fee.checked_mul(2).ok_or_else(denied)?)
                .ok_or_else(denied)?;
        }
        require(cost <= plan.maximum_cost_wei)?;
        require(
            q(&read(t, url, "eth_getBalance", json!([plan.sender, "pending"])).await?)? >= cost,
        )?;
        require(
            block(&read(t, url, "eth_getBlockByNumber", json!([safe_tag, false])).await?)? == safe,
        )?;
        require(
            block(&read(t, url, "eth_getBlockByNumber", json!([final_tag, false])).await?)?
                == finalized,
        )?;
        for tag in ["latest", "pending"] {
            require(
                q(&read(t, url, "eth_getTransactionCount", json!([plan.sender, tag])).await?)?
                    == u128::from(plan.nonce),
            )?;
        }
        Ok::<_, ChainBaseError>((safe, finalized, cost))
    };
    let (a, b) = tokio::try_join!(inspect(urls[0]), inspect(urls[1]))?;
    require(a.0 == b.0 && a.1 == b.1 && clock() < authority_expires_at.timestamp() as u64)?;
    Ok(NonceRetirementObservation {
        observed_at: Utc::now(),
        safe_block: a.0,
        finalized_block: a.1,
        provider_hashes: hashes,
        maximum_cost_wei: a.2.max(b.2),
    })
}
impl BaseTransactionRelayer {
    /// Offline, exact-plan signing only. The private caller must hold the shared lease,
    /// reserve the current-day budget and durably record owner approval first.
    pub async fn sign_nonce_retirement(
        &self,
        plan: &NonceRetirementPlan,
    ) -> Result<PreparedBaseRelayTransaction, ChainBaseError> {
        plan.validate()?;
        require(
            self.address() == plan.sender
                && Utc::now() < plan.expires_at
                && Utc::now() >= plan.created_at,
        )?;
        let request = TransactionRequest::default()
            .with_from(self.signer.address())
            .with_to(self.signer.address())
            .with_value(U256::ZERO)
            .with_input(Bytes::new())
            .with_nonce(plan.nonce)
            .with_chain_id(plan.chain_id)
            .with_gas_limit(plan.gas_limit)
            .with_max_fee_per_gas(plan.max_fee_per_gas_wei)
            .with_max_priority_fee_per_gas(plan.max_priority_fee_per_gas_wei);
        let envelope = request
            .build(&EthereumWallet::from(self.signer.clone()))
            .await
            .map_err(|_| denied())?;
        let prepared = PreparedBaseRelayTransaction {
            nonce: plan.nonce,
            signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
            transaction: BaseRelayedTransaction {
                relayer: plan.sender.clone(),
                tx_hash: format!("{:#x}", envelope.tx_hash()),
                estimated_gas: 21_000,
                gas_limit: plan.gas_limit,
                max_fee_per_gas_wei: plan.max_fee_per_gas_wei,
                max_priority_fee_per_gas_wei: plan.max_priority_fee_per_gas_wei,
                estimated_max_cost_wei: plan.maximum_cost_wei,
            },
        };
        validate_nonce_retirement_envelope(plan, &prepared)?;
        Ok(prepared)
    }
}
pub fn validate_nonce_retirement_envelope(
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
) -> Result<(), ChainBaseError> {
    plan.validate()?;
    let raw = parse_alloy_bytes(&saved.signed_transaction)?;
    let mut bytes = raw.as_ref();
    let e = TxEnvelope::decode_2718(&mut bytes).map_err(|_| denied())?;
    require(matches!(&e,TxEnvelope::Eip1559(tx) if tx.tx().access_list.0.is_empty()))?;
    let sender = e
        .signature()
        .recover_address_from_prehash(&e.signature_hash())
        .map_err(|_| denied())?;
    require(
        bytes.is_empty()
            && sender == parse_alloy_address(&plan.sender)?
            && e.chain_id() == Some(plan.chain_id)
            && e.nonce() == plan.nonce
            && e.to() == Some(sender)
            && e.value() == U256::ZERO
            && e.input().is_empty()
            && e.gas_limit() == plan.gas_limit
            && e.max_fee_per_gas() == plan.max_fee_per_gas_wei
            && e.max_priority_fee_per_gas() == Some(plan.max_priority_fee_per_gas_wei)
            && saved.nonce == plan.nonce
            && saved.transaction.relayer == plan.sender
            && saved.transaction.tx_hash == format!("{:#x}", e.tx_hash())
            && saved.transaction.estimated_gas == 21_000
            && saved.transaction.gas_limit == plan.gas_limit
            && saved.transaction.max_fee_per_gas_wei == plan.max_fee_per_gas_wei
            && saved.transaction.max_priority_fee_per_gas_wei == plan.max_priority_fee_per_gas_wei
            && saved.transaction.estimated_max_cost_wei == plan.maximum_cost_wei,
    )
}

/// No send/retry authority: verify the local exact envelope has consumed the
/// nonce in a common finalized block. Expired plans remain reconcilable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceRetirementConfirmation {
    pub observed_at: DateTime<Utc>,
    pub transaction_hash: String,
    pub included_block: BaseBlockIdentity,
    pub finalized_block: BaseBlockIdentity,
    pub finalized_nonce: u64,
    pub provider_hashes: [String; 2],
}
pub async fn confirm_nonce_retirement(
    urls: [&str; 2],
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    seconds: u64,
) -> Result<NonceRetirementConfirmation, ChainBaseError> {
    let t = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(3))?;
    tokio::time::timeout(
        std::time::Duration::from_secs(seconds.min(90)),
        confirm_with_transport(urls, plan, saved, Utc::now(), &t),
    )
    .await
    .map_err(|_| denied())?
}
pub async fn require_retirement_absent(
    urls: [&str; 2],
    saved: &PreparedBaseRelayTransaction,
    seconds: u64,
) -> Result<(), ChainBaseError> {
    providers(urls)?;
    let t = ReqwestJsonRpcTransport::without_redirects(std::time::Duration::from_secs(3))?;
    tokio::time::timeout(std::time::Duration::from_secs(seconds.min(90)), async {
        for url in urls {
            for method in ["eth_getTransactionByHash", "eth_getTransactionReceipt"] {
                require(
                    read(&t, url, method, json!([saved.transaction.tx_hash]))
                        .await?
                        .is_null(),
                )?;
            }
        }
        Ok(())
    })
    .await
    .map_err(|_| denied())?
}
pub async fn confirm_with_transport<T: JsonRpcTransport + ?Sized>(
    urls: [&str; 2],
    plan: &NonceRetirementPlan,
    saved: &PreparedBaseRelayTransaction,
    now: DateTime<Utc>,
    t: &T,
) -> Result<NonceRetirementConfirmation, ChainBaseError> {
    validate_nonce_retirement_envelope(plan, saved)?;
    let hashes = providers(urls)?;
    let started = std::time::Instant::now();
    let clock = || now.timestamp() as u64 + started.elapsed().as_secs();
    let mut heights = Vec::new();
    for url in urls {
        require(q(&read(t, url, "eth_chainId", json!([])).await?)? == 8453)?;
        let head =
            block(&read(t, url, "eth_getBlockByNumber", json!(["finalized", false])).await?)?;
        fresh(&head, clock(), 1800)?;
        heights.push(head.number);
    }
    let height = *heights.iter().min().ok_or_else(denied)?;
    let tag = hex_quantity(height);
    let inspect = async |url| {
        let final_block = block(&read(t, url, "eth_getBlockByNumber", json!([tag, false])).await?)?;
        require(final_block.number == height)?;
        fresh(&final_block, clock(), 1800)?;
        let tx = read(
            t,
            url,
            "eth_getTransactionByHash",
            json!([saved.transaction.tx_hash]),
        )
        .await?;
        let receipt = read(
            t,
            url,
            "eth_getTransactionReceipt",
            json!([saved.transaction.tx_hash]),
        )
        .await?;
        require(
            tx["hash"] == saved.transaction.tx_hash
                && receipt["transactionHash"] == saved.transaction.tx_hash
                && tx["from"] == plan.sender
                && tx["to"] == plan.sender
                && receipt["from"] == plan.sender
                && receipt["to"] == plan.sender
                && q(&tx["type"])? == 2
                && q(&tx["chainId"])? == 8453
                && q(&tx["nonce"])? == u128::from(plan.nonce)
                && q(&tx["value"])? == 0
                && tx["input"] == "0x"
                && tx["accessList"] == json!([])
                && tx.get("authorizationList").is_none_or(Value::is_null)
                && q(&tx["gas"])? == u128::from(plan.gas_limit)
                && q(&tx["maxFeePerGas"])? == plan.max_fee_per_gas_wei
                && q(&tx["maxPriorityFeePerGas"])? == plan.max_priority_fee_per_gas_wei
                && q(&receipt["status"])? == 1
                && q(&receipt["gasUsed"])? == 21_000
                && receipt["logs"] == json!([])
                && receipt["contractAddress"].is_null()
                && q(&receipt["effectiveGasPrice"])? <= plan.max_fee_per_gas_wei
                && tx["blockNumber"] == receipt["blockNumber"]
                && tx["blockHash"] == receipt["blockHash"]
                && tx["transactionIndex"] == receipt["transactionIndex"],
        )?;
        let number = u64::try_from(q(&receipt["blockNumber"])?).map_err(|_| denied())?;
        require(number <= height)?;
        let included = block(
            &read(
                t,
                url,
                "eth_getBlockByNumber",
                json!([hex_quantity(number), false]),
            )
            .await?,
        )?;
        require(
            included.number == number
                && receipt["blockHash"] == included.hash
                && included.timestamp <= final_block.timestamp,
        )?;
        let nonce = u64::try_from(q(&read(
            t,
            url,
            "eth_getTransactionCount",
            json!([plan.sender, tag]),
        )
        .await?)?)
        .map_err(|_| denied())?;
        require(nonce > plan.nonce)?;
        let mut previous_nonce = nonce;
        let mut nonces = vec![nonce];
        for at in [tag.as_str(), "latest", "pending"] {
            require(read(t, url, "eth_getCode", json!([plan.sender, at])).await? == "0x")?;
            let current = u64::try_from(q(&read(
                t,
                url,
                "eth_getTransactionCount",
                json!([plan.sender, at]),
            )
            .await?)?)
            .map_err(|_| denied())?;
            require(current >= previous_nonce && (at != tag.as_str() || current == nonce))?;
            previous_nonce = current;
            nonces.push(current);
        }
        require(
            read(
                t,
                url,
                "eth_getTransactionReceipt",
                json!([plan.original_tx_hash]),
            )
            .await?
            .is_null(),
        )?;
        require(
            block(&read(t, url, "eth_getBlockByNumber", json!([tag, false])).await?)?
                == final_block,
        )?;
        require(
            block(
                &read(
                    t,
                    url,
                    "eth_getBlockByNumber",
                    json!([hex_quantity(number), false]),
                )
                .await?,
            )? == included,
        )?;
        Ok::<_, ChainBaseError>((final_block, included, nonce, nonces))
    };
    let (a, b) = tokio::try_join!(inspect(urls[0]), inspect(urls[1]))?;
    require(a == b)?;
    Ok(NonceRetirementConfirmation {
        observed_at: Utc::now(),
        transaction_hash: saved.transaction.tx_hash.clone(),
        included_block: a.1,
        finalized_block: a.0,
        finalized_nonce: a.2,
        provider_hashes: hashes,
    })
}

#[cfg(test)]
mod tests;
