use super::*;

pub(crate) const ENGINE: &str = "creator_review_v1";
/// The paying customer reviews; a dedicated platform key signs only the recorded verdict.
pub const CUSTOMER_REVIEW_ENGINE: &str = "customer_review_v1";

/// Platform keys allowed to sign customer-review verdicts, pinned per network by reviewed code.
/// An empty list fails closed: no customer-review terms validate until a signer is pinned here.
pub const BASE_MAINNET_CUSTOMER_REVIEW_VERIFIERS: &[&str] =
    &["0x5e760f69412d5bf40085cd8cd40ee20985bf1e0e"];
// Dedicated isolated Sepolia rehearsal signer; never authorized on Base mainnet.
pub const BASE_SEPOLIA_CUSTOMER_REVIEW_VERIFIERS: &[&str] =
    &["0x3d62809d8127c3ed9d60c52e2bca020f7377c5e1"];
/// Publicly known Anvil account #9. Test builds only; never a production signer.
#[cfg(any(test, feature = "testing-customer-review-signer"))]
pub const TEST_CUSTOMER_REVIEW_VERIFIER: &str = "0xa0ee7a142d267c1f36714e4a8f75612f20a79720";

pub fn customer_review_verifiers(network: &str) -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut verifiers = match network {
        "base-mainnet" => BASE_MAINNET_CUSTOMER_REVIEW_VERIFIERS.to_vec(),
        "base-sepolia" => BASE_SEPOLIA_CUSTOMER_REVIEW_VERIFIERS.to_vec(),
        _ => Vec::new(),
    };
    #[cfg(any(test, feature = "testing-customer-review-signer"))]
    verifiers.push(TEST_CUSTOMER_REVIEW_VERIFIER);
    verifiers
}

/// True for either human-review engine; both settle through one exact signed verdict.
pub(crate) fn is_engine(engine: &Value) -> bool {
    engine == ENGINE || engine == CUSTOMER_REVIEW_ENGINE
}

// A supported human review path, not an assertion that an automated service is live.
pub(crate) fn validate(document: &AutonomousBountyTermsDocument) -> Result<(), ChainBaseError> {
    let policy = &document.verification_policy;
    let benchmark = &document.benchmark;
    let contract = &document.contract_terms;
    let schema = &document.evidence_schema;
    let creator = contract["creator_wallet"].as_str().unwrap_or_default();
    let cutoff = benchmark["delivery_deadline"].as_u64().unwrap_or(0);
    let signer = policy["verifiers"]
        .as_array()
        .filter(|values| values.len() == 1)
        .and_then(|values| values[0].as_str());
    let customer = benchmark["engine"] == CUSTOMER_REVIEW_ENGINE;
    let authorized_signer = signer.is_some_and(|signer| {
        if customer {
            let network = contract["network"].as_str().unwrap_or_default();
            customer_review_verifiers(network)
                .iter()
                .any(|pinned| pinned.eq_ignore_ascii_case(signer))
                && !signer.eq_ignore_ascii_case(creator)
        } else {
            signer.eq_ignore_ascii_case(creator)
        }
    });
    let reviewer = if customer {
        "paying_customer"
    } else {
        "creator"
    };
    let evidence = schema["required"].as_array().is_some_and(|required| {
        ["artifact_url", "artifact_sha256"]
            .iter()
            .all(|field| required.contains(&json!(field)))
    });
    if !is_engine(&benchmark["engine"])
        || policy["engine"] != benchmark["engine"]
        || policy["mechanism"] != "signed_quorum"
        || policy["threshold"] != 1
        || normalize_evm_address(creator).is_err()
        || !authorized_signer
        || benchmark["reviewer"] != reviewer
        || benchmark["acceptance"] != "all_published_criteria"
        || cutoff == 0
        || contract["funding_deadline"].as_u64() != Some(cutoff)
        || document.acceptance_criteria.is_empty()
        || policy["public_disclosure"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
        || schema["type"] != "object"
        || !evidence
        || schema["properties"]["artifact_url"]["type"] != "string"
        || schema["properties"]["artifact_url"]["pattern"] != "^https://"
        || schema["properties"]["artifact_sha256"]["type"] != "string"
        || schema["properties"]["artifact_sha256"]["pattern"] != "^sha256:[0-9a-f]{64}$"
    {
        return Err(ChainBaseError::InvalidVerificationConfiguration(
            "human review must bind its authorized sole signer, all published criteria, an exact delivery/funding cutoff, disclosure and public artifact evidence".to_string()));
    }
    Ok(())
}

pub fn submission_on_time(
    document: &AutonomousBountyTermsDocument,
    verification_expires_at: u64,
) -> bool {
    // AgentBounty._submit emits block.timestamp + the immutable verification window.
    document.contract_terms["verification_window_seconds"]
        .as_u64()
        .and_then(|window| verification_expires_at.checked_sub(window))
        .zip(document.benchmark["delivery_deadline"].as_u64())
        .is_some_and(|(submitted_at, cutoff)| submitted_at <= cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> AutonomousBountyTermsDocument {
        serde_json::from_value(json!({
            "schema_version":"agent-bounties/terms-v1", "contract_terms":{
                "creator_wallet":"0x1111111111111111111111111111111111111111", "funding_deadline":2000, "verification_window_seconds":100},
            "title":"Design deliverable", "goal":"Deliver the specified model", "acceptance_criteria":["All specified files open and meet the dimensioned brief"],
            "benchmark":{"engine":ENGINE,"reviewer":"creator","acceptance":"all_published_criteria","delivery_deadline":2000},
            "verification_policy":{"engine":ENGINE,"mechanism":"signed_quorum","threshold":1,"verifiers":["0x1111111111111111111111111111111111111111"],"public_disclosure":"Creator signs the human review verdict."},
            "evidence_schema":{"type":"object","required":["artifact_url","artifact_sha256"],"properties":{"artifact_url":{"type":"string","pattern":"^https://"},"artifact_sha256":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}}}, "source_url":null,"discovery_source":null
        })).unwrap()
    }
    #[test]
    fn creator_review_binds_authority_and_evidence() {
        let doc = document();
        validate(&doc).unwrap();
        for (field, value) in [
            ("threshold", json!(2)),
            ("engine", json!("sandboxed_regression_v1")),
            (
                "verifiers",
                json!(["0x2222222222222222222222222222222222222222"]),
            ),
        ] {
            let mut invalid = doc.clone();
            invalid.verification_policy[field] = value;
            assert!(validate(&invalid).is_err());
        }
        let mut invalid = doc.clone();
        invalid.contract_terms["funding_deadline"] = json!(2001);
        assert!(validate(&invalid).is_err());
        let mut invalid = doc;
        invalid.evidence_schema["required"] = json!([]);
        assert!(validate(&invalid).is_err());
    }
    #[test]
    fn delivery_cutoff_uses_canonical_submission_not_review_time() {
        let doc = document();
        assert!(submission_on_time(&doc, 2100));
        assert!(!submission_on_time(&doc, 2101));
        assert!(!submission_on_time(&doc, 99));
    }

    #[test]
    fn creator_review_can_prepare_fully_funded_public_creation_with_exact_terms() {
        let now = Utc::now();
        let cutoff = now.timestamp() + 86400;
        let mut doc = document();
        doc.benchmark["delivery_deadline"] = json!(cutoff);
        doc.contract_terms = json!({
            "protocol_version":"agent-bounties/autonomous-v1", "creator_wallet":"0x1111111111111111111111111111111111111111",
            "network":"base-mainnet", "settlement_token":BASE_MAINNET_USDC_TOKEN_ADDRESS,
            "solver_reward":{"amount":18_000_000,"currency":"usdc"}, "verifier_reward":{"amount":2_000_000,"currency":"usdc"},
            "claim_bond":{"amount":2_000_000,"currency":"usdc"}, "initial_funding":{"amount":20_000_000,"currency":"usdc"},
            "funding_deadline":cutoff,"claim_window_seconds":86400,"verification_window_seconds":172800,"creation_nonce":format!("0x{}","12".repeat(32))
        });
        let record = build_autonomous_bounty_terms_record(
            "0x1111111111111111111111111111111111111111",
            doc,
            now,
        )
        .unwrap();
        let create = autonomous_bounty_create_from_terms(&record).unwrap();
        validate_autonomous_creation_for_public_earning("base-mainnet", &create, &record).unwrap();
        assert_eq!(create.verifiers, vec![record.creator_wallet.clone()]);
        assert_eq!(create.threshold, 1);
        let mut altered = create;
        altered.verifiers[0] = "0x2222222222222222222222222222222222222222".to_string();
        assert!(
            validate_autonomous_creation_for_public_earning("base-mainnet", &altered, &record)
                .is_err()
        );
    }

    fn customer_document() -> AutonomousBountyTermsDocument {
        let mut doc = document();
        doc.contract_terms["network"] = json!("base-mainnet");
        doc.benchmark["engine"] = json!(CUSTOMER_REVIEW_ENGINE);
        doc.benchmark["reviewer"] = json!("paying_customer");
        doc.verification_policy["engine"] = json!(CUSTOMER_REVIEW_ENGINE);
        doc.verification_policy["verifiers"] = json!([TEST_CUSTOMER_REVIEW_VERIFIER]);
        doc.verification_policy["public_disclosure"] =
            json!("The paying customer reviews; the platform key signs only that verdict.");
        doc
    }

    #[test]
    fn customer_review_requires_a_pinned_platform_signer_distinct_from_the_creator() {
        let doc = customer_document();
        validate(&doc).unwrap();
        assert!(is_engine(&doc.benchmark["engine"]));

        let mut unpinned = doc.clone();
        unpinned.verification_policy["verifiers"] =
            json!(["0x2222222222222222222222222222222222222222"]);
        assert!(validate(&unpinned).is_err(), "unpinned signer accepted");

        let mut creator_signs = doc.clone();
        creator_signs.contract_terms["creator_wallet"] = json!(TEST_CUSTOMER_REVIEW_VERIFIER);
        assert!(
            validate(&creator_signs).is_err(),
            "creator cannot sign customer review"
        );

        let mut wrong_reviewer = doc.clone();
        wrong_reviewer.benchmark["reviewer"] = json!("creator");
        assert!(validate(&wrong_reviewer).is_err());

        let mut mixed_engines = doc;
        mixed_engines.verification_policy["engine"] = json!(ENGINE);
        assert!(
            validate(&mixed_engines).is_err(),
            "policy and benchmark engines must match"
        );
    }

    #[test]
    fn customer_review_signer_is_pinned_only_for_its_network() {
        let signer = "0x5e760f69412d5bf40085cd8cd40ee20985bf1e0e";
        assert_eq!(BASE_MAINNET_CUSTOMER_REVIEW_VERIFIERS, &[signer]);
        assert_eq!(
            BASE_SEPOLIA_CUSTOMER_REVIEW_VERIFIERS,
            &["0x3d62809d8127c3ed9d60c52e2bca020f7377c5e1"]
        );
        assert_eq!(
            customer_review_verifiers("base-mainnet"),
            vec![signer, TEST_CUSTOMER_REVIEW_VERIFIER]
        );
        let mut doc = customer_document();
        doc.verification_policy["verifiers"] = json!([signer]);
        validate(&doc).unwrap();

        for network in ["base-sepolia", "unknown"] {
            let mut wrong_network = doc.clone();
            wrong_network.contract_terms["network"] = json!(network);
            assert!(validate(&wrong_network).is_err(), "signer crossed networks");
        }

        let mut creator_signs = doc.clone();
        creator_signs.contract_terms["creator_wallet"] = json!(signer);
        assert!(
            validate(&creator_signs).is_err(),
            "creator signed its own review"
        );

        let mut delegate_signs = doc;
        delegate_signs.verification_policy["verifiers"] =
            json!(["0xedcdaa360f31a17be346dd39554e3b82b76121ed"]);
        assert!(
            validate(&delegate_signs).is_err(),
            "funding delegate signed review"
        );
        assert!(!is_engine(&json!("sandboxed_regression_v1")));
    }

    #[test]
    fn sepolia_customer_review_rejects_cross_network_and_funding_roles() {
        let signer = BASE_SEPOLIA_CUSTOMER_REVIEW_VERIFIERS[0];
        let mut doc = customer_document();
        doc.contract_terms["network"] = json!("base-sepolia");
        doc.verification_policy["verifiers"] = json!([signer]);
        validate(&doc).unwrap();
        for network in ["base-mainnet", "unknown"] {
            let mut invalid = doc.clone();
            invalid.contract_terms["network"] = json!(network);
            assert!(
                validate(&invalid).is_err(),
                "Sepolia signer crossed networks"
            );
        }
        for unauthorized in [
            "0xff452d01a41c7fe0e0f9fd8c3bf64e70434effc1",
            "0x28f4b5ad65a1af23e1ff1881c8fb5ee84adb4bea",
            BASE_MAINNET_CUSTOMER_REVIEW_VERIFIERS[0],
        ] {
            let mut invalid = doc.clone();
            invalid.verification_policy["verifiers"] = json!([unauthorized]);
            assert!(
                validate(&invalid).is_err(),
                "wrong role signed Sepolia review"
            );
        }
        doc.contract_terms["creator_wallet"] = json!(signer);
        assert!(
            validate(&doc).is_err(),
            "creator signed its own customer review"
        );
    }
}
