use super::*;

pub(crate) const ENGINE: &str = "creator_review_v1";

// A supported human review path, not an assertion that an automated service is live.
pub(crate) fn validate(document: &AutonomousBountyTermsDocument) -> Result<(), ChainBaseError> {
    let policy = &document.verification_policy;
    let benchmark = &document.benchmark;
    let contract = &document.contract_terms;
    let schema = &document.evidence_schema;
    let creator = contract["creator_wallet"].as_str().unwrap_or_default();
    let cutoff = benchmark["delivery_deadline"].as_u64().unwrap_or(0);
    let signers = policy["verifiers"].as_array();
    let exact_creator = signers.is_some_and(|values| {
        values.len() == 1
            && values[0]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(creator))
    });
    let evidence = schema["required"].as_array().is_some_and(|required| {
        ["artifact_url", "artifact_sha256"]
            .iter()
            .all(|field| required.contains(&json!(field)))
    });
    if benchmark["engine"] != ENGINE
        || policy["engine"] != ENGINE
        || policy["mechanism"] != "signed_quorum"
        || policy["threshold"] != 1
        || normalize_evm_address(creator).is_err()
        || !exact_creator
        || benchmark["reviewer"] != "creator"
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
            "creator review must bind the creator as sole signer, all published criteria, an exact delivery/funding cutoff, disclosure and public artifact evidence".to_string()));
    }
    Ok(())
}

pub(crate) fn submission_on_time(
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
}
