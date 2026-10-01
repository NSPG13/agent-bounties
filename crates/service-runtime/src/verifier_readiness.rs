//! Temporary fail-closed operational containment, independent of shared relay enablement.
//! Replace this hold only with reviewed, authenticated release-bound health checkpoints.
use chain_base::AutonomousBountyFeedItem;
use domain::AutonomousBountyTermsRecord;

pub const UNAVAILABLE: &str = "verification_runner_unavailable: Verification unavailable. The approved runner and required signers have no current health checkpoints. Existing funds, evidence and recovery remain available.";

pub fn needs_hosted_regression(terms: &AutonomousBountyTermsRecord) -> bool {
    let benchmark = &terms.document.benchmark;
    benchmark["engine"] == "sandboxed_regression_v1"
        || benchmark["required_child_engine"] == "sandboxed_regression_v1"
        || terms.document.verification_policy["engine"] == "sandboxed_regression_v1"
}

pub fn require_available(terms: &AutonomousBountyTermsRecord) -> Result<(), &'static str> {
    if needs_hosted_regression(terms) {
        Err(UNAVAILABLE)
    } else {
        Ok(())
    }
}

pub fn apply(feed: &mut [AutonomousBountyFeedItem]) {
    for item in feed {
        if item.verification_ready && item.terms.as_ref().is_some_and(needs_hosted_regression) {
            item.verification_ready = false;
            item.verification_readiness_reason = UNAVAILABLE.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn terms() -> AutonomousBountyTermsRecord {
        serde_json::from_value(serde_json::json!({
            "terms_hash":"hash", "policy_hash":"policy", "acceptance_criteria_hash":"criteria", "creator_wallet":"wallet", "benchmark_hash":"benchmark", "evidence_schema_hash":"evidence",
            "document": {"schema_version":"agent-bounties/autonomous-v1", "title":"test", "goal":"test", "contract_terms":{}, "acceptance_criteria":[], "benchmark":{"engine":"sandboxed_regression_v1"}, "verification_policy":{}, "evidence_schema":{}}, "created_at":"2026-09-30T00:00:00Z"
        })).expect("fixture")
    }
    #[test]
    fn shared_relay_enablement_cannot_approve_paused_verification() {
        let mut terms = terms();
        assert_eq!(require_available(&terms), Err(UNAVAILABLE));
        terms.document.benchmark = serde_json::json!({"engine":"standing_meta_v3_routed_parent","required_child_engine":"sandboxed_regression_v1"});
        assert_eq!(require_available(&terms), Err(UNAVAILABLE));
        terms.document.benchmark = serde_json::json!({"engine":"creator_open_v1"});
        assert!(require_available(&terms).is_ok());
    }
    #[test]
    fn public_hold_preserves_records_and_does_not_mutate_raw_job_input() {
        let item: AutonomousBountyFeedItem = serde_json::from_value(serde_json::json!({
            "bounty_id":"id", "bounty_contract":"contract", "creator":"creator", "status":"submitted", "solver_reward":"990000", "verifier_reward":"10000", "claim_bond":"10000", "timeout_bond_pool":"0", "target_amount":"1000000", "funded_amount":"1000000", "terms_hash":"hash", "terms":terms(), "terms_valid":true, "verification_mode":"signed_quorum", "verifier_module":null, "verification_ready":true, "verification_readiness_reason":"profile approved", "validation_errors":[], "events":[]
        })).unwrap();
        let original = serde_json::to_value(&item).unwrap();
        let mut feed = vec![item.clone()];
        apply(&mut feed);
        assert!(!feed[0].verification_ready);
        assert_eq!(feed[0].verification_readiness_reason, UNAVAILABLE);
        let mut after = serde_json::to_value(&feed[0]).unwrap();
        after["verification_ready"] = original["verification_ready"].clone();
        after["verification_readiness_reason"] = original["verification_readiness_reason"].clone();
        assert_eq!(after, original);
        assert!(item.verification_ready, "raw job input stays discoverable");
        feed[0].verification_readiness_reason = "verification_profile_unapproved".into();
        apply(&mut feed);
        assert_eq!(
            feed[0].verification_readiness_reason,
            "verification_profile_unapproved"
        );
    }
}
