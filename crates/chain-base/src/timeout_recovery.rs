//! Exact, permissionless timeout scope. Read checks never authorize a replay.
mod revalidation;
use super::*;
use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::Decodable2718,
};
pub use revalidation::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeoutRelayScope {
    pub network: String,
    pub bounty_contract: String,
    pub action: String,
    pub round: u64,
    pub factory_contract: String,
    pub bounty_id: String,
    pub terms_hash: String,
}
impl TimeoutRelayScope {
    pub fn validate(&self) -> Result<(), ChainBaseError> {
        if !matches!(self.network.as_str(), "base-mainnet" | "base-sepolia")
            || self.round == 0
            || !matches!(self.action.as_str(), "expire_claim" | "expire_submission")
            || normalize_address(&self.bounty_contract)? != self.bounty_contract
            || normalize_address(&self.factory_contract)? != self.factory_contract
            || normalize_hash(&self.bounty_id)? != self.bounty_id
            || normalize_hash(&self.terms_hash)? != self.terms_hash
        {
            return Err(invalid("invalid timeout scope"));
        }
        Ok(())
    }
    pub fn operation_id(&self) -> Result<Uuid, ChainBaseError> {
        self.validate()?;
        // Terms are checked in the durable scope, not folded into a key that
        // could allocate another transaction when supplied terms change.
        let digest = Sha256::digest(
            format!(
                "agent-bounties/timeout-v1\0{}\0{}\0{}\0{}",
                self.network, self.bounty_contract, self.action, self.round
            )
            .as_bytes(),
        );
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(bytes))
    }
    pub fn calldata(&self) -> Result<&'static str, ChainBaseError> {
        match self.action.as_str() {
            "expire_claim" => Ok("0x1257d2c8"),
            "expire_submission" => Ok("0xf9251ec7"),
            _ => Err(invalid("unknown timeout action")),
        }
    }
}
fn invalid(message: &str) -> ChainBaseError {
    ChainBaseError::InvalidRelayIntent(message.into())
}
fn selector(signature: &str) -> String {
    format!("0x{}", hex::encode(&keccak_word(signature.as_bytes())[..4]))
}
fn quantity(word: &str) -> Result<u64, ChainBaseError> {
    u64::try_from(U256::from_be_slice(&parse_bytes32(word)?))
        .map_err(|_| invalid("timeout quantity overflow"))
}

/// Fresh latest state plus sender occupancy, before preparation and again before
/// the first send. The old contract call has no round argument; never resubmit it.
pub async fn verify_timeout_scope(
    rpc: &str,
    scope: &TimeoutRelayScope,
    sender: &str,
    expected_nonce: Option<u64>,
) -> Result<u64, ChainBaseError> {
    verify_timeout_scope_with_transport(
        rpc,
        scope,
        sender,
        expected_nonce,
        &ReqwestJsonRpcTransport::default(),
    )
    .await
}
pub async fn verify_timeout_scope_with_transport<T: JsonRpcTransport + ?Sized>(
    rpc: &str,
    scope: &TimeoutRelayScope,
    sender: &str,
    expected_nonce: Option<u64>,
    transport: &T,
) -> Result<u64, ChainBaseError> {
    scope.validate()?;
    let chain = base_network_descriptor(&scope.network)?.chain_id;
    let observed_chain = rpc_result(
        transport
            .post_json_value(
                rpc,
                &json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
            )
            .await?,
        1,
        "eth_chainId",
    )?;
    if observed_chain != format!("0x{chain:x}") {
        return Err(invalid("timeout chain mismatch"));
    }
    let latest = fetch_block_number_with_transport(rpc, 2, transport).await?;
    let block_tag = format!("0x{latest:x}");
    let block = fetch_block_identity_with_transport(rpc, &block_tag, 3, transport).await?;
    let now =
        u64::try_from(Utc::now().timestamp()).map_err(|_| invalid("timeout clock unavailable"))?;
    if block.number != latest || block.timestamp > now || now - block.timestamp > 90 {
        return Err(invalid("timeout head unavailable"));
    }
    let mut words = BTreeMap::new();
    for (index, signature) in [
        "status()",
        "round()",
        "factory()",
        "bountyId()",
        "termsHash()",
        "claimExpiresAt()",
        "verificationExpiresAt()",
    ]
    .iter()
    .enumerate()
    {
        words.insert(
            *signature,
            fetch_contract_word(
                rpc,
                &scope.bounty_contract,
                &selector(signature),
                &block_tag,
                10 + index as u64,
                transport,
            )
            .await?,
        );
    }
    let expected_status = if scope.action == "expire_claim" { 2 } else { 3 };
    let deadline = quantity(
        &words[if scope.action == "expire_claim" {
            "claimExpiresAt()"
        } else {
            "verificationExpiresAt()"
        }],
    )?;
    let factory_word = format!("0x{:0>64}", scope.factory_contract.trim_start_matches("0x"));
    if quantity(&words["status()"])? != expected_status
        || quantity(&words["round()"])? != scope.round
        || words["factory()"] != factory_word
        || words["bountyId()"] != scope.bounty_id
        || words["termsHash()"] != scope.terms_hash
        || deadline == 0
        || block.timestamp <= deadline
    {
        return Err(invalid(
            "timeout state, round, commitment or deadline changed",
        ));
    }
    let canonical_call = format!(
        "{}{:0>64}",
        selector("isCanonicalBounty(address)"),
        scope.bounty_contract.trim_start_matches("0x")
    );
    if quantity(
        &fetch_contract_word(
            rpc,
            &scope.factory_contract,
            &canonical_call,
            &block_tag,
            18,
            transport,
        )
        .await?,
    )? != 1
    {
        return Err(invalid("timeout bounty is not canonical"));
    }
    let sender = normalize_address(sender)?;
    let mut nonces = Vec::new();
    for (id, tag) in [(19, "latest"), (20, "pending")] {
        let value = rpc_result(transport.post_json_value(rpc, &json!({"jsonrpc":"2.0","id":id,"method":"eth_getTransactionCount","params":[sender,tag]})).await?, id, "eth_getTransactionCount")?;
        nonces.push(parse_rpc_quantity(
            value
                .as_str()
                .ok_or_else(|| invalid("timeout nonce unavailable"))?,
        )?);
    }
    if nonces[0] != nonces[1] || expected_nonce.is_some_and(|nonce| nonce != nonces[0]) {
        return Err(invalid("timeout sender has pending or changed nonce"));
    }
    let checked = fetch_block_identity_with_transport(rpc, &block_tag, 21, transport).await?;
    let finished =
        u64::try_from(Utc::now().timestamp()).map_err(|_| invalid("timeout clock unavailable"))?;
    if finished < block.timestamp || finished - block.timestamp > 90 || checked != block {
        return Err(invalid("timeout observation changed"));
    }
    Ok(nonces[0])
}

/// Validate the signed bytes, not merely the saved metadata, before their first
/// submission. This does not authorize rebroadcasting any prior send attempt.
pub fn validate_timeout_envelope(
    saved: &PreparedBaseRelayTransaction,
    scope: &TimeoutRelayScope,
    sender: &str,
    max_gas: u64,
    max_fee: u128,
) -> Result<(), ChainBaseError> {
    scope.validate()?;
    let raw = parse_alloy_bytes(&saved.signed_transaction)?;
    let mut bytes = raw.as_ref();
    let envelope = TxEnvelope::decode_2718(&mut bytes)
        .map_err(|_| invalid("invalid saved timeout envelope"))?;
    if !matches!(&envelope, TxEnvelope::Eip1559(tx) if tx.tx().access_list.0.is_empty()) {
        return Err(invalid("timeout envelope must be plain EIP-1559"));
    }
    let signer = envelope
        .signature()
        .recover_address_from_prehash(&envelope.signature_hash())
        .map_err(|_| invalid("invalid saved timeout signer"))?;
    if !bytes.is_empty()
        || envelope.chain_id() != Some(base_network_descriptor(&scope.network)?.chain_id)
        || envelope.to() != Some(parse_alloy_address(&scope.bounty_contract)?)
        || envelope.value() != U256::ZERO
        || envelope.input().as_ref() != parse_alloy_bytes(scope.calldata()?)?.as_ref()
        || envelope.nonce() != saved.nonce
        || signer != parse_alloy_address(sender)?
        || !saved.transaction.relayer.eq_ignore_ascii_case(sender)
        || format!("{:#x}", alloy::primitives::keccak256(&raw)) != saved.transaction.tx_hash
        || envelope.gas_limit() != saved.transaction.gas_limit
        || envelope.gas_limit() > max_gas
        || envelope.max_fee_per_gas() != saved.transaction.max_fee_per_gas_wei
        || envelope.max_fee_per_gas() > max_fee
        || envelope.max_priority_fee_per_gas()
            != Some(saved.transaction.max_priority_fee_per_gas_wei)
    {
        return Err(invalid(
            "saved timeout envelope does not match its exact scope",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeoutReceiptObservation {
    Pending,
    Confirmed { block: u64, event_id: String },
    Reverted { block: u64 },
}
/// A receipt must match the recorded transaction, immutable source, canonical
/// block, action and round. A later lifecycle state cannot rewrite this result.
pub async fn observe_timeout_receipt<T: JsonRpcTransport + ?Sized>(
    rpc: &str,
    scope: &TimeoutRelayScope,
    hash: &str,
    confirmations: u64,
    transport: &T,
) -> Result<TimeoutReceiptObservation, ChainBaseError> {
    scope.validate()?;
    let Some(receipt) = fetch_transaction_receipt_with_transport(rpc, hash, 31, transport)
        .await?
        .result
    else {
        return Ok(TimeoutReceiptObservation::Pending);
    };
    let chain = rpc_result(
        transport
            .post_json_value(
                rpc,
                &json!({"jsonrpc":"2.0","id":32,"method":"eth_chainId","params":[]}),
            )
            .await?,
        32,
        "eth_chainId",
    )?;
    if chain != format!("0x{:x}", base_network_descriptor(&scope.network)?.chain_id)
        || !receipt.transaction_hash.eq_ignore_ascii_case(hash)
    {
        return Err(invalid("timeout receipt chain or hash mismatch"));
    }
    let Some(number) = receipt.block_number()? else {
        return Ok(TimeoutReceiptObservation::Pending);
    };
    let latest = fetch_block_number_with_transport(rpc, 33, transport).await?;
    if number > latest || latest - number + 1 < confirmations.max(1) {
        return Ok(TimeoutReceiptObservation::Pending);
    }
    let tag = format!("0x{number:x}");
    let block = fetch_block_identity_with_transport(rpc, &tag, 34, transport).await?;
    if block.number != number
        || !receipt
            .block_hash
            .as_ref()
            .is_some_and(|h| h.eq_ignore_ascii_case(&block.hash))
    {
        return Ok(TimeoutReceiptObservation::Pending);
    }
    // These identity fields are immutable, checked at the receipt's exact block.
    for (index, (signature, expected)) in [
        (
            "factory()",
            format!("0x{:0>64}", scope.factory_contract.trim_start_matches("0x")),
        ),
        ("bountyId()", scope.bounty_id.clone()),
        ("termsHash()", scope.terms_hash.clone()),
    ]
    .into_iter()
    .enumerate()
    {
        if fetch_contract_word(
            rpc,
            &scope.bounty_contract,
            &selector(signature),
            &tag,
            35 + index as u64,
            transport,
        )
        .await?
            != expected
        {
            return Err(invalid("timeout receipt source mismatch"));
        }
    }
    let canonical_call = format!(
        "{}{:0>64}",
        selector("isCanonicalBounty(address)"),
        scope.bounty_contract.trim_start_matches("0x")
    );
    if quantity(
        &fetch_contract_word(
            rpc,
            &scope.factory_contract,
            &canonical_call,
            &tag,
            38,
            transport,
        )
        .await?,
    )? != 1
    {
        return Err(invalid("timeout receipt factory mismatch"));
    }
    let outcome = match receipt.succeeded()? {
        None => TimeoutReceiptObservation::Pending,
        Some(false) => TimeoutReceiptObservation::Reverted { block: number },
        Some(true) => {
            let logs = receipt.logs_to_evm_logs()?;
            if logs
                .iter()
                .any(|log| log.block_number != number || !log.tx_hash.eq_ignore_ascii_case(hash))
            {
                return Err(invalid("timeout receipt log coordinates mismatch"));
            }
            let kind = if scope.action == "expire_claim" {
                AutonomousBountyEventKind::ClaimExpired
            } else {
                AutonomousBountyEventKind::SubmissionExpired
            };
            let events = decode_autonomous_bounty_logs(logs)?;
            let matching: Vec<_> = events
                .into_iter()
                .filter(|event| {
                    event.kind == kind
                        && event.contract_address == scope.bounty_contract
                        && event.bounty_id == scope.bounty_id
                        && event.data["round"].as_u64() == Some(scope.round)
                })
                .collect();
            if matching.len() != 1 {
                return Err(invalid("timeout receipt action or round mismatch"));
            }
            TimeoutReceiptObservation::Confirmed {
                block: number,
                event_id: matching[0].log_key.clone(),
            }
        }
    };
    if fetch_block_identity_with_transport(rpc, &tag, 39, transport).await? != block {
        return Err(invalid("timeout receipt canonical block changed"));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    fn scope() -> TimeoutRelayScope {
        TimeoutRelayScope {
            network: "base-mainnet".into(),
            bounty_contract: format!("0x{}", "11".repeat(20)),
            action: "expire_claim".into(),
            round: 7,
            factory_contract: format!("0x{}", "22".repeat(20)),
            bounty_id: format!("0x{}", "33".repeat(32)),
            terms_hash: format!("0x{}", "44".repeat(32)),
        }
    }
    fn word(n: u64) -> String {
        format!("0x{n:064x}")
    }
    struct Transport {
        replacements: BTreeMap<u64, Value>,
        seen: Mutex<Vec<Value>>,
        now: u64,
    }
    impl Transport {
        fn new() -> Self {
            Self {
                replacements: BTreeMap::new(),
                seen: Mutex::new(vec![]),
                now: Utc::now().timestamp() as u64,
            }
        }
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for Transport {
        async fn post_json_value(&self, _: &str, request: &Value) -> Result<Value, ChainBaseError> {
            self.seen.lock().unwrap().push(request.clone());
            let id = request["id"].as_u64().unwrap();
            let s = scope();
            let result = if let Some(value) = self.replacements.get(&id) {
                value.clone()
            } else {
                match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!("0x64"),
                    "eth_getBlockByNumber" => {
                        json!({"number":"0x64","hash":word(100),"timestamp":format!("0x{:x}",self.now)})
                    }
                    "eth_getTransactionCount" => json!("0x9"),
                    "eth_call" => {
                        assert_eq!(request["params"][1], "0x64");
                        json!(match id {
                            10 => word(2),
                            11 => word(7),
                            12 | 35 =>
                                format!("0x{:0>64}", s.factory_contract.trim_start_matches("0x")),
                            13 | 36 => s.bounty_id,
                            14 | 37 => s.terms_hash,
                            15 | 16 => word(self.now - 1),
                            18 | 38 => word(1),
                            _ => panic!("unexpected eth_call {id}"),
                        })
                    }
                    "eth_getTransactionReceipt" => {
                        json!({"transactionHash":word(55),"blockNumber":"0x64","blockHash":word(100),"status":"0x1","logs":[{
                            "address":s.bounty_contract,"topics":[event_topic("ClaimExpired(bytes32,uint64,address,uint256,uint256)"),s.bounty_id,word(7),word(99)],"data":format!("{}{}",word(1),&word(2)[2..]),"transactionHash":word(55),"blockNumber":"0x64","logIndex":"0x0"
                        }]})
                    }
                    method => panic!("unexpected method {method}"),
                }
            };
            Ok(json!({"jsonrpc":"2.0","id":id,"result":result}))
        }
    }
    #[test]
    fn timeout_identity_is_exact_and_domain_separated_without_terms_aliases() {
        let s = scope();
        let key = s.operation_id().unwrap();
        for change in 0..4 {
            let mut other = s.clone();
            match change {
                0 => other.network = "base-sepolia".into(),
                1 => other.bounty_contract = format!("0x{}", "55".repeat(20)),
                2 => other.action = "expire_submission".into(),
                _ => other.round += 1,
            };
            assert_ne!(other.operation_id().unwrap(), key);
        }
        let mut other = s.clone();
        other.terms_hash = word(8);
        assert_eq!(other.operation_id().unwrap(), key);
        other.round = 0;
        assert!(other.operation_id().is_err());
        other = s.clone();
        other.network = "base".into();
        assert!(other.operation_id().is_err());
    }
    #[tokio::test]
    async fn timeout_preflight_checks_chain_current_round_deadline_source_and_nonce() {
        let s = scope();
        let t = Transport::new();
        assert_eq!(
            verify_timeout_scope_with_transport("local", &s, &s.factory_contract, Some(9), &t)
                .await
                .unwrap(),
            9
        );
        let now = t.now;
        for (id, value) in [
            (1, json!("0x1")),
            (10, json!(word(1))),
            (11, json!(word(8))),
            (12, json!(word(1))),
            (13, json!(word(1))),
            (14, json!(word(1))),
            (15, json!(word(now))),
            (18, json!(word(0))),
            (19, json!("0x8")),
            (20, json!("0xa")),
            (
                3,
                json!({"number":"0x64","hash":word(100),"timestamp":format!("0x{:x}",now-91)}),
            ),
            (
                21,
                json!({"number":"0x64","hash":word(999),"timestamp":format!("0x{now:x}")}),
            ),
        ] {
            let mut t = Transport::new();
            // Every mutation must use the original block clock, including deadline equality.
            t.now = now;
            t.replacements.insert(id, value);
            assert!(
                verify_timeout_scope_with_transport("local", &s, &s.factory_contract, Some(9), &t)
                    .await
                    .is_err(),
                "accepted altered field {id}"
            );
            assert!(t
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request["method"] != "eth_sendRawTransaction"));
        }
        assert!(verify_timeout_scope_with_transport(
            "local",
            &s,
            &s.factory_contract,
            Some(8),
            &Transport::new()
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn timeout_receipt_matches_exact_hash_source_action_round_and_canonical_block() {
        let s = scope();
        let t = Transport::new();
        assert!(matches!(
            observe_timeout_receipt("local", &s, &word(55), 1, &t)
                .await
                .unwrap(),
            TimeoutReceiptObservation::Confirmed { block: 100, .. }
        ));
        let original = t
            .post_json_value(
                "local",
                &json!({"id":31,"method":"eth_getTransactionReceipt"}),
            )
            .await
            .unwrap()["result"]
            .clone();
        for change in 0..3 {
            let mut receipt = original.clone();
            match change {
                0 => receipt["logs"][0]["transactionHash"] = json!(word(123)),
                1 => receipt["logs"][0]["blockNumber"] = json!("0x63"),
                _ => {
                    let duplicate = receipt["logs"][0].clone();
                    receipt["logs"].as_array_mut().unwrap().push(duplicate);
                }
            }
            let mut t = Transport::new();
            t.replacements.insert(31, receipt);
            assert!(observe_timeout_receipt("local", &s, &word(55), 1, &t)
                .await
                .is_err());
        }
        let mut reorg = Transport::new();
        reorg.replacements.insert(
            39,
            json!({"number":"0x64","hash":word(101),"timestamp":format!("0x{:x}",reorg.now)}),
        );
        assert!(observe_timeout_receipt("local", &s, &word(55), 1, &reorg)
            .await
            .is_err());
        let mut wrong = s.clone();
        wrong.round += 1;
        assert!(
            observe_timeout_receipt("local", &wrong, &word(55), 1, &Transport::new())
                .await
                .is_err()
        );
        wrong = s.clone();
        wrong.action = "expire_submission".into();
        assert!(
            observe_timeout_receipt("local", &wrong, &word(55), 1, &Transport::new())
                .await
                .is_err()
        );
        assert!(
            observe_timeout_receipt("local", &s, &word(56), 1, &Transport::new())
                .await
                .is_err()
        );
        assert_eq!(
            observe_timeout_receipt("local", &s, &word(55), 2, &Transport::new())
                .await
                .unwrap(),
            TimeoutReceiptObservation::Pending
        );
        for id in [32, 35, 36, 37, 38, 39] {
            let mut t = Transport::new();
            t.replacements.insert(id, json!(word(999)));
            assert!(observe_timeout_receipt("local", &s, &word(55), 1, &t)
                .await
                .is_err());
        }
        let mut t = Transport::new();
        t.replacements.insert(31, Value::Null);
        assert_eq!(
            observe_timeout_receipt("local", &s, &word(55), 1, &t)
                .await
                .unwrap(),
            TimeoutReceiptObservation::Pending
        );
        let mut t = Transport::new();
        t.replacements.insert(31,json!({"transactionHash":word(55),"blockNumber":"0x64","blockHash":word(100),"status":"0x0","logs":[]}));
        assert_eq!(
            observe_timeout_receipt("local", &s, &word(55), 1, &t)
                .await
                .unwrap(),
            TimeoutReceiptObservation::Reverted { block: 100 }
        );
    }
    #[tokio::test]
    async fn timeout_signed_envelope_is_checked_against_bytes_not_metadata() {
        let s = scope();
        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
                .parse()
                .unwrap();
        let envelope = TransactionRequest::default()
            .with_to(s.bounty_contract.parse::<Address>().unwrap())
            .with_value(U256::ZERO)
            .with_input(parse_alloy_bytes(s.calldata().unwrap()).unwrap())
            .with_nonce(9)
            .with_chain_id(8453)
            .with_gas_limit(25000)
            .with_max_fee_per_gas(3)
            .with_max_priority_fee_per_gas(1)
            .build(&EthereumWallet::from(signer.clone()))
            .await
            .unwrap();
        let p = PreparedBaseRelayTransaction {
            nonce: 9,
            signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
            transaction: BaseRelayedTransaction {
                relayer: format!("{:#x}", signer.address()),
                tx_hash: format!("{:#x}", envelope.tx_hash()),
                estimated_gas: 21000,
                gas_limit: 25000,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75000,
            },
        };
        validate_timeout_envelope(&p, &s, &p.transaction.relayer, 25000, 3).unwrap();
        for change in 0..6 {
            let mut bad = p.clone();
            match change {
                0 => bad.nonce += 1,
                1 => bad.transaction.tx_hash = word(1),
                2 => bad.transaction.relayer = s.bounty_contract.clone(),
                3 => bad.transaction.gas_limit += 1,
                4 => bad.transaction.max_fee_per_gas_wei += 1,
                _ => bad.signed_transaction.push_str("00"),
            };
            assert!(validate_timeout_envelope(&bad, &s, &p.transaction.relayer, 25000, 3).is_err());
        }
        let mut wrong = s.clone();
        wrong.action = "expire_submission".into();
        assert!(validate_timeout_envelope(&p, &wrong, &p.transaction.relayer, 25000, 3).is_err());
        wrong = s.clone();
        wrong.network = "base-sepolia".into();
        assert!(validate_timeout_envelope(&p, &wrong, &p.transaction.relayer, 25000, 3).is_err());
        let mut request = TransactionRequest::default()
            .with_to(s.bounty_contract.parse::<Address>().unwrap())
            .with_value(U256::ZERO)
            .with_input(parse_alloy_bytes(s.calldata().unwrap()).unwrap())
            .with_nonce(9)
            .with_chain_id(8453)
            .with_gas_limit(25000)
            .with_max_fee_per_gas(3)
            .with_max_priority_fee_per_gas(1);
        request.access_list = Some(
            serde_json::from_value(json!([{"address":s.factory_contract,"storageKeys":[]}]))
                .unwrap(),
        );
        let extra = request
            .build(&EthereumWallet::from(signer.clone()))
            .await
            .unwrap();
        let mut changed = p.clone();
        changed.signed_transaction = format!("0x{}", hex::encode(extra.encoded_2718()));
        changed.transaction.tx_hash = format!("{:#x}", extra.tx_hash());
        assert!(validate_timeout_envelope(&changed, &s, &p.transaction.relayer, 25000, 3).is_err());
        assert!(validate_timeout_envelope(&p, &s, &p.transaction.relayer, 24999, 3).is_err());
        assert!(validate_timeout_envelope(&p, &s, &p.transaction.relayer, 25000, 2).is_err());
    }
}

// Reuse the exact runtime/clone binding for obsolete-round nonce retirement.
pub(crate) async fn retirement_identity<T: JsonRpcTransport + ?Sized>(
    t: &T,
    url: &str,
    scope: &TimeoutRelayScope,
    policy: &TimeoutRevalidationPolicy,
    tag: &str,
) -> Result<(), ChainBaseError> {
    revalidation::identity(t, url, scope, policy, tag).await
}
