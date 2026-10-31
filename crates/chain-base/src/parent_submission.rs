//! Read-only parent eligibility checks. These never approve or reject child work.
use super::*;

fn invalid(code: &str) -> ChainBaseError {
    ChainBaseError::InvalidSubmissionPreparation(format!(
        "{code}: parent eligibility is not established; child terms and payments are unchanged"
    ))
}
fn data(signature: &str, words: &[String]) -> String {
    let selector = Keccak256::digest(signature.as_bytes());
    format!(
        "0x{}{}",
        hex::encode(&selector[..4]),
        words
            .iter()
            .map(|s| s.trim_start_matches("0x"))
            .collect::<String>()
    )
}
fn address_word(address: &str) -> Result<String, ChainBaseError> {
    Ok(format!(
        "0x{:0>64}",
        normalize_address(address)?.trim_start_matches("0x")
    ))
}
fn address(word: &str) -> Result<String, ChainBaseError> {
    let bytes = parse_bytes32(word)?;
    if bytes[..12] != [0; 12] {
        return Err(invalid("parent_noncanonical_address"));
    }
    Ok(format!("0x{}", hex::encode(&bytes[12..])))
}
fn quantity(word: &str) -> Result<u128, ChainBaseError> {
    u128::try_from(U256::from_be_slice(&parse_bytes32(word)?))
        .map_err(|_| invalid("parent_quantity_overflow"))
}
const ZERO: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

struct Reader<'a, T: JsonRpcTransport + ?Sized> {
    rpc: &'a str,
    transport: &'a T,
    block: Value,
}
impl<T: JsonRpcTransport + ?Sized> Reader<'_, T> {
    async fn read(
        &self,
        to: &str,
        signature: &str,
        args: &[String],
        count: usize,
    ) -> Result<Vec<String>, ChainBaseError> {
        let result = rpc_result(self.transport.post_json_value(self.rpc, &json!({"jsonrpc":"2.0","id":1,"method":"eth_call","params":[{"to":to,"data":data(signature,args)},self.block]})).await?, 1, "eth_call")?;
        let value = result
            .as_str()
            .ok_or_else(|| invalid("parent_missing_chain_result"))?;
        if !value.starts_with("0x")
            || value.len() != 2 + count * 64
            || !value.as_bytes()[2..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(invalid("parent_malformed_chain_result"));
        }
        (0..count)
            .map(|i| normalize_hash(&format!("0x{}", &value[2 + i * 64..2 + (i + 1) * 64])))
            .collect()
    }
    async fn fields(
        &self,
        to: &str,
        signatures: &[&str],
    ) -> Result<BTreeMap<String, String>, ChainBaseError> {
        // Fixed, bounded read set. Never execute caller-supplied RPC methods.
        let mut out = BTreeMap::new();
        for signature in signatures {
            out.insert(
                (*signature).to_string(),
                self.read(to, signature, &[], 1).await?.remove(0),
            );
        }
        Ok(out)
    }
}
#[derive(Clone)]
struct Snapshot {
    parent: BTreeMap<String, String>,
    child: BTreeMap<String, String>,
    terms: Vec<String>,
    parent_participant: Vec<String>,
    child_participant: Vec<String>,
    routed: bool,
}
fn validate(
    s: &Snapshot,
    solver: &str,
    parent_id: &str,
    terms_hash: &str,
    policy_hash: &str,
    round: u64,
    now: u64,
) -> Result<(), ChainBaseError> {
    let p = |key: &str| {
        s.parent
            .get(key)
            .ok_or_else(|| invalid("parent_missing_field"))
    };
    let c = |key: &str| {
        s.child
            .get(key)
            .ok_or_else(|| invalid("child_missing_field"))
    };
    let require = |ok, code| if ok { Ok(()) } else { Err(invalid(code)) };
    require(
        quantity(p("status()")?)? == 2
            && quantity(p("round()")?)? == u128::from(round)
            && address(p("solver()")?)? == solver
            && p("bountyId()")? == parent_id
            && p("termsHash()")? == terms_hash
            && p("policyHash()")? == policy_hash,
        "parent_claim_changed",
    )?;
    let cutoff = quantity(p("claimExpiresAt()")?)?
        .checked_sub(quantity(p("claimWindowSeconds()")?)?)
        .ok_or_else(|| invalid("parent_claim_timing_invalid"))?;
    require(
        cutoff > 0 && quantity(p("claimExpiresAt()")?)? > u128::from(now),
        "parent_claim_expired",
    )?;
    require(
        quantity(p("verificationMode()")?)? == 0
            && quantity(p("threshold()")?)? == 1
            && address(p("verifierModule()")?)?
                == if s.routed {
                    BASE_MAINNET_STANDING_META_V3_ROUTER
                } else {
                    BASE_MAINNET_STANDING_META_V2_VERIFIER
                }
            && p("acceptanceCriteriaHash()")?
                == if s.routed {
                    BASE_MAINNET_STANDING_META_V3_ACCEPTANCE_CRITERIA_HASH
                } else {
                    BASE_MAINNET_STANDING_META_V2_ACCEPTANCE_CRITERIA_HASH
                },
        "parent_profile_changed",
    )?;
    for f in ["factory()", "settlementToken()"] {
        require(p(f)? == c(f)?, "child_rail_mismatch")?;
    }
    require(
        address(p("factory()")?)? == BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY
            && address(p("settlementToken()")?)?
                .eq_ignore_ascii_case(BASE_MAINNET_USDC_TOKEN_ADDRESS),
        "parent_rail_mismatch",
    )?;
    require(
        address(c("creator()")?)? == solver
            && address(c("solver()")?)? != solver
            && c("solver()")? != ZERO,
        "child_participant_mismatch",
    )?;
    let reward = quantity(p("solverReward()")?)?;
    let target = quantity(c("targetAmount()")?)?;
    require(
        if s.routed {
            target >= 1_000_000 && reward >= 2_000_000 && target <= reward - 1_000_000
        } else {
            target >= reward
        },
        "child_target_mismatch",
    )?;
    require(
        quantity(c("verificationMode()")?)? == 1
            && c("verifierModule()")? == ZERO
            && c("verifierSetHash()")? == BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH
            && quantity(c("threshold()")?)? == 2,
        "child_quorum_mismatch",
    )?;
    require(
        quantity(c("status()")?)? == 4
            && quantity(c("fundedAmount()")?)? == 0
            && quantity(c("activeClaimBond()")?)? == 0
            && c("submissionHash()")? != ZERO
            && c("evidenceHash()")? != ZERO,
        "child_settlement_required",
    )?;
    let t = &s.terms;
    require(
        t.len() == 11
            && address(&t[0])? == solver
            && quantity(&t[1])? > 0
            && quantity(&t[1])? < cutoff
            && quantity(&t[2])? == u128::from(round)
            && quantity(&t[3])? == 2
            && quantity(&t[4])? > 0
            && t[5] == parent_id,
        "child_terms_timing_or_parent_mismatch",
    )?;
    for (i, field) in [
        (6, "policyHash()"),
        (7, "acceptanceCriteriaHash()"),
        (8, "benchmarkHash()"),
        (9, "evidenceSchemaHash()"),
        (10, "verifierSetHash()"),
    ] {
        require(t[i] == *c(field)?, "child_terms_commitment_mismatch")?;
    }
    for participant in [&s.parent_participant, &s.child_participant] {
        require(
            participant.len() == 3 && participant[0] != ZERO && quantity(&participant[2])? == 1,
            "parent_registration_cutoff_not_met",
        )?;
    }
    require(
        s.parent_participant[0] != s.child_participant[0],
        "parent_distinct_participant_required",
    )
}

pub fn requires_parent_submission_preflight(item: &AutonomousBountyFeedItem) -> bool {
    matches!(
        item.terms
            .as_ref()
            .and_then(|t| t.document.benchmark.get("engine"))
            .and_then(Value::as_str),
        Some("standing_meta_v2_parent" | "standing_meta_v3_routed_parent")
    )
}

pub async fn preflight_parent_submission(
    rpc: &str,
    network: &str,
    item: &AutonomousBountyFeedItem,
    solver: &str,
    evidence: &Value,
    now: u64,
) -> Result<Value, ChainBaseError> {
    tokio::time::timeout(
        std::time::Duration::from_secs(25),
        preflight_with_transport(
            rpc,
            network,
            item,
            solver,
            evidence,
            now,
            &ReqwestJsonRpcTransport::default(),
        ),
    )
    .await
    .map_err(|_| invalid("parent_eligibility_check_unavailable"))?
}
async fn preflight_with_transport<T: JsonRpcTransport + ?Sized>(
    rpc: &str,
    network: &str,
    item: &AutonomousBountyFeedItem,
    solver: &str,
    evidence: &Value,
    now: u64,
    transport: &T,
) -> Result<Value, ChainBaseError> {
    if network != "base-mainnet" || !requires_parent_submission_preflight(item) {
        return Err(invalid("parent_profile_unsupported"));
    }
    let child = normalize_address(
        evidence
            .get("child_bounty_contract")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("child_bounty_contract_required"))?,
    )?;
    let parent = normalize_address(&item.bounty_contract)?;
    let solver = normalize_address(solver)?;
    if child == parent || child == address(ZERO)? {
        return Err(invalid("child_address_invalid"));
    }
    let chain = rpc_result(
        transport
            .post_json_value(
                rpc,
                &json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}),
            )
            .await?,
        1,
        "eth_chainId",
    )?;
    if chain.as_str().map(parse_rpc_quantity).transpose()? != Some(8453) {
        return Err(invalid("parent_wrong_chain"));
    }
    let block = fetch_block_identity_with_transport(rpc, "safe", 2, transport).await?;
    if block.timestamp > now.saturating_add(30) || now.saturating_sub(block.timestamp) > 2700 {
        return Err(invalid("parent_stale_safe_block"));
    }
    let reader = Reader {
        rpc,
        transport,
        block: json!({"blockHash":block.hash,"requireCanonical":true}),
    };
    for bounty in [&parent, &child] {
        if quantity(
            &reader
                .read(
                    BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY,
                    "isCanonicalBounty(address)",
                    &[address_word(bounty)?],
                    1,
                )
                .await?[0],
        )? != 1
        {
            return Err(invalid("parent_or_child_not_canonical"));
        }
    }
    let p = reader
        .fields(
            &parent,
            &[
                "bountyId()",
                "factory()",
                "settlementToken()",
                "solverReward()",
                "termsHash()",
                "policyHash()",
                "acceptanceCriteriaHash()",
                "verificationMode()",
                "verifierModule()",
                "threshold()",
                "status()",
                "round()",
                "solver()",
                "claimExpiresAt()",
                "claimWindowSeconds()",
            ],
        )
        .await?;
    let c = reader
        .fields(
            &child,
            &[
                "creator()",
                "factory()",
                "settlementToken()",
                "targetAmount()",
                "termsHash()",
                "policyHash()",
                "acceptanceCriteriaHash()",
                "benchmarkHash()",
                "evidenceSchemaHash()",
                "verifierSetHash()",
                "verificationMode()",
                "verifierModule()",
                "threshold()",
                "status()",
                "fundedAmount()",
                "activeClaimBond()",
                "solver()",
                "submissionHash()",
                "evidenceHash()",
            ],
        )
        .await?;
    let cutoff = quantity(&p["claimExpiresAt()"])?
        .checked_sub(quantity(&p["claimWindowSeconds()"])?)
        .ok_or_else(|| invalid("parent_claim_timing_invalid"))?;
    let cutoff_word = format!("0x{cutoff:064x}");
    let terms = reader
        .read(
            BASE_MAINNET_STANDING_META_V2_TERMS_REGISTRY,
            "commitment(bytes32)",
            &[c["termsHash()"].clone()],
            11,
        )
        .await?;
    let pp = reader
        .read(
            BASE_MAINNET_STANDING_META_V2_PARTICIPANT_REGISTRY,
            "eligibleAt(address,uint64)",
            &[address_word(&solver)?, cutoff_word.clone()],
            3,
        )
        .await?;
    let cp = reader
        .read(
            BASE_MAINNET_STANDING_META_V2_PARTICIPANT_REGISTRY,
            "eligibleAt(address,uint64)",
            &[c["solver()"].clone(), cutoff_word],
            3,
        )
        .await?;
    let terms_record = item
        .terms
        .as_ref()
        .ok_or_else(|| invalid("parent_terms_missing"))?;
    let round = item
        .events
        .iter()
        .rev()
        .find(|e| e.kind == AutonomousBountyEventKind::BountyClaimed)
        .and_then(|e| e.data.get("round"))
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid("parent_claim_missing"))?;
    let snapshot = Snapshot {
        parent: p,
        child: c,
        terms,
        parent_participant: pp,
        child_participant: cp,
        routed: terms_record.document.benchmark["engine"] == "standing_meta_v3_routed_parent",
    };
    validate(
        &snapshot,
        &solver,
        &item.bounty_id,
        &item.terms_hash,
        &terms_record.policy_hash,
        round,
        now,
    )?;
    if snapshot.routed
        && quantity(
            &reader
                .read(
                    BASE_MAINNET_STANDING_META_V3_ROUTER,
                    "isPolicyActive(bytes32)",
                    std::slice::from_ref(&terms_record.policy_hash),
                    1,
                )
                .await?[0],
        )? != 1
    {
        return Err(invalid("parent_routing_unavailable"));
    }
    let recheck =
        fetch_block_identity_with_transport(rpc, &hex_quantity(block.number), 3, transport).await?;
    if recheck.hash != block.hash {
        return Err(invalid("parent_safe_block_changed"));
    }
    Ok(
        json!({"schema":"agent-bounties/parent-submission-preflight-v1","parent_bounty_contract":parent,"child_bounty_contract":child,"parent_round":round,"parent_claim_cutoff":cutoff.to_string(),"parent_submission_deadline":quantity(&snapshot.parent["claimExpiresAt()"] )?.to_string(),"safe_block_number":block.number,"safe_block_hash":block.hash,"checked_at":now,"claim":"The parent eligibility rules matched canonical state at this safe block. This is not a verdict, payment or new requirement for the child solver."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn word(n: u128) -> String {
        format!("0x{n:064x}")
    }
    fn solver() -> String {
        format!("0x{}", "11".repeat(20))
    }
    fn snapshot(routed: bool) -> Snapshot {
        let parent = [
            ("status()", word(2)),
            ("round()", word(3)),
            ("solver()", address_word(&solver()).unwrap()),
            ("bountyId()", word(10)),
            ("termsHash()", word(11)),
            ("policyHash()", word(12)),
            ("claimExpiresAt()", word(1000)),
            ("claimWindowSeconds()", word(900)),
            ("verificationMode()", word(0)),
            ("threshold()", word(1)),
            (
                "verifierModule()",
                address_word(if routed {
                    BASE_MAINNET_STANDING_META_V3_ROUTER
                } else {
                    BASE_MAINNET_STANDING_META_V2_VERIFIER
                })
                .unwrap(),
            ),
            (
                "acceptanceCriteriaHash()",
                if routed {
                    BASE_MAINNET_STANDING_META_V3_ACCEPTANCE_CRITERIA_HASH
                } else {
                    BASE_MAINNET_STANDING_META_V2_ACCEPTANCE_CRITERIA_HASH
                }
                .to_string(),
            ),
            (
                "factory()",
                address_word(BASE_MAINNET_AUTONOMOUS_BOUNTY_FACTORY).unwrap(),
            ),
            (
                "settlementToken()",
                address_word(BASE_MAINNET_USDC_TOKEN_ADDRESS).unwrap(),
            ),
            ("solverReward()", word(3_000_000)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect::<BTreeMap<_, _>>();
        let child = [
            ("creator()", address_word(&solver()).unwrap()),
            ("solver()", word(0x222)),
            ("factory()", parent["factory()"].clone()),
            ("settlementToken()", parent["settlementToken()"].clone()),
            (
                "targetAmount()",
                word(if routed { 1_000_000 } else { 3_000_000 }),
            ),
            ("verificationMode()", word(1)),
            ("verifierModule()", word(0)),
            (
                "verifierSetHash()",
                BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH.to_string(),
            ),
            ("threshold()", word(2)),
            ("status()", word(4)),
            ("fundedAmount()", word(0)),
            ("activeClaimBond()", word(0)),
            ("submissionHash()", word(20)),
            ("evidenceHash()", word(21)),
            ("termsHash()", word(22)),
            ("policyHash()", word(23)),
            ("acceptanceCriteriaHash()", word(24)),
            ("benchmarkHash()", word(25)),
            ("evidenceSchemaHash()", word(26)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect::<BTreeMap<_, _>>();
        let terms = vec![
            address_word(&solver()).unwrap(),
            word(99),
            word(3),
            word(2),
            word(100),
            word(10),
            word(23),
            word(24),
            word(25),
            word(26),
            BASE_MAINNET_STANDING_META_V2_VERIFIER_SET_HASH.to_string(),
        ];
        Snapshot {
            parent,
            child,
            terms,
            parent_participant: vec![word(31), word(33), word(1)],
            child_participant: vec![word(32), word(34), word(1)],
            routed,
        }
    }
    fn check(s: &Snapshot, now: u64) -> Result<(), ChainBaseError> {
        validate(s, &solver(), &word(10), &word(11), &word(12), 3, now)
    }
    #[test]
    fn accepts_original_v2_and_routed_v3_rules_and_exact_claim_expiry() {
        for routed in [false, true] {
            let s = snapshot(routed);
            assert!(check(&s, 999).is_ok(), "{:?}", check(&s, 999));
            assert!(check(&s, 1000)
                .unwrap_err()
                .to_string()
                .contains("parent_claim_expired"));
        }
    }
    #[test]
    fn parent_rules_reject_late_terms_late_registration_and_shared_identity() {
        let baseline = snapshot(true);
        for timestamp in [0, 100, 101] {
            let mut s = baseline.clone();
            s.terms[1] = word(timestamp);
            assert!(check(&s, 500).is_err());
        }
        for i in 0..2 {
            let mut s = baseline.clone();
            if i == 0 {
                s.parent_participant[2] = word(0)
            } else {
                s.child_participant[2] = word(0)
            };
            assert!(check(&s, 500)
                .unwrap_err()
                .to_string()
                .contains("registration_cutoff"));
        }
        let mut s = baseline.clone();
        s.child_participant[0] = s.parent_participant[0].clone();
        assert!(check(&s, 500)
            .unwrap_err()
            .to_string()
            .contains("distinct_participant"));
        let mut s = baseline;
        s.child_participant[0] = word(0);
        assert!(check(&s, 500).is_err());
    }
    #[test]
    fn rejects_unsettled_child_quorum_changes_and_wrong_parent_commitments() {
        for (field, bad) in [
            ("status()", word(3)),
            ("fundedAmount()", word(1)),
            ("activeClaimBond()", word(1)),
            ("submissionHash()", word(0)),
            ("evidenceHash()", word(0)),
            ("threshold()", word(1)),
            ("verifierSetHash()", word(1)),
            ("creator()", word(1)),
            ("solver()", word(0)),
            ("targetAmount()", word(2_000_001)),
            ("factory()", word(1)),
            ("settlementToken()", word(1)),
        ] {
            let mut s = snapshot(true);
            s.child.insert(field.into(), bad);
            assert!(check(&s, 500).is_err(), "{field}");
        }
        for index in 0..11 {
            let mut s = snapshot(true);
            s.terms[index] = word(0);
            assert!(check(&s, 500).is_err(), "terms[{index}]");
        }
        for field in [
            "status()",
            "round()",
            "solver()",
            "bountyId()",
            "termsHash()",
            "policyHash()",
            "verifierModule()",
            "acceptanceCriteriaHash()",
        ] {
            let mut s = snapshot(true);
            s.parent.insert(field.into(), word(0));
            assert!(check(&s, 500).is_err(), "{field}");
        }
    }
    struct Transport {
        result: String,
    }
    #[async_trait::async_trait]
    impl JsonRpcTransport for Transport {
        async fn post_json_value(&self, _: &str, request: &Value) -> Result<Value, ChainBaseError> {
            assert_eq!(request["method"], "eth_call");
            assert_eq!(
                request["params"][1],
                json!({"blockHash":word(7),"requireCanonical":true})
            );
            Ok(json!({"jsonrpc":"2.0","id":1,"result":self.result}))
        }
    }
    #[tokio::test]
    async fn reads_exact_canonical_block_and_rejects_malformed_abi_without_panicking() {
        for result in [
            format!("0x{}", "é".repeat(32)),
            "0x".into(),
            format!("{}00", word(1)),
        ] {
            let transport = Transport { result };
            let reader = Reader {
                rpc: "https://rpc.invalid",
                transport: &transport,
                block: json!({"blockHash":word(7),"requireCanonical":true}),
            };
            assert!(reader.read(&solver(), "status()", &[], 1).await.is_err());
        }
        let transport = Transport { result: word(4) };
        let reader = Reader {
            rpc: "https://rpc.invalid",
            transport: &transport,
            block: json!({"blockHash":word(7),"requireCanonical":true}),
        };
        assert_eq!(
            reader.read(&solver(), "status()", &[], 1).await.unwrap(),
            vec![word(4)]
        );
    }
}
