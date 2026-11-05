//! Explain an existing rejected claim; never change eligibility or expose stored evidence.
use super::{agent_action_error, AgentActionApiError, AutonomousBountyFeedItem, StatusCode};

pub(super) fn explain(status: StatusCode, item: &AutonomousBountyFeedItem) -> AgentActionApiError {
    let (code, message, retryable, action) = classify(item);
    agent_action_error(status, code, message, retryable, action)
}

fn classify(item: &AutonomousBountyFeedItem) -> (&'static str, &'static str, bool, &'static str) {
    if item.status != "claimable" {
        return ("claim_not_available", "This bounty is not open for a new claim.", false,
            "Check the bounty's current status and any published recovery steps. Do not sign or post a new bond.");
    }
    if !item.terms_valid {
        return ("verification_terms_not_ready", "The committed bounty terms could not be validated.", false,
            "Ask the platform to check the original committed terms. Do not replace funded tests or sign a claim while this hold remains.");
    }
    if matches!(
        item.verification_readiness_reason.as_str(),
        chain_base::RECOVERY_RESERVED_VERIFICATION_REASON
            | chain_base::STANDING_META_V2_RECOVERY_REASON
    ) {
        return ("claim_recovery_reserved", "This bounty is reserved for incident recovery.", false,
            "Follow the bounty's published recovery steps. Do not claim, sign, or post a bond while this hold remains.");
    }
    // A permanent profile hold takes precedence over a simultaneous temporary checkpoint outage.
    let has_code = |code: &str| {
        item.verification_readiness_reason.split(':').next() == Some(code)
            || item
                .verification_details
                .as_ref()
                .and_then(|details| details["blocker_codes"].as_array())
                .is_some_and(|codes| codes.iter().any(|value| value.as_str() == Some(code)))
    };
    for code in [
        "verification_profile_unapproved",
        "verification_child_profile_unapproved",
    ] {
        if has_code(code) {
            return (code, "The bounty's exact committed verification profile is not approved.", false,
                "Ask for review of the original committed tests. If they cannot qualify, use the published recovery process. Refreshing or replacing funded tests does not remove this hold.");
        }
    }
    if item.terms.as_ref().is_some_and(|terms| {
        matches!(
            terms.document.benchmark["engine"].as_str(),
            Some("creator_review_v1" | "customer_review_v1")
        ) && terms.document.benchmark["delivery_deadline"]
            .as_u64()
            .is_some_and(|deadline| deadline <= chrono::Utc::now().timestamp().max(0) as u64)
    }) {
        return ("delivery_deadline_passed", "The creator-review submission window is not open.", false,
            "Check the original delivery deadline and published recovery steps. A review deadline does not extend the time to submit work.");
    }
    if item.claim_eligibility.is_some() {
        return ("claim_eligibility_required", "This bounty requires a separate eligibility check.", false,
            "Use the bounty's eligibility flow before requesting a claim. A wallet address alone does not establish eligibility.");
    }
    for code in [
        "verification_checkpoint_failed",
        "verification_checkpoint_stale",
        "verification_checkpoint_missing",
        "verification_checkpoint_store_unavailable",
    ] {
        if has_code(code) {
            return (code, "Verification is temporarily unavailable because a required health check is missing, failed, or stale.", true,
                "Wait until the approved runner and required signers report healthy, then check readiness again. Do not sign or post a bond yet.");
        }
    }
    ("verification_unavailable", "The bounty's verification requirements are not ready.", false,
        "Check the bounty's verification rules and ask the platform to review the blocker. Do not sign or post a bond while verification is unavailable.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item() -> AutonomousBountyFeedItem {
        serde_json::from_value(json!({
            "bounty_id":"fixture", "bounty_contract":"0x1111111111111111111111111111111111111111",
            "creator":"0x2222222222222222222222222222222222222222", "status":"claimable",
            "solver_reward":"20", "verifier_reward":"1", "claim_bond":"1", "timeout_bond_pool":"0",
            "target_amount":"21", "funded_amount":"21", "required_external_spend":"0", "gross_cash_margin":"20",
            "terms_hash":"fixture", "terms":null, "terms_valid":true, "verification_mode":"signed_quorum",
            "verification_ready":false, "verification_readiness_reason":"verification_profile_unapproved: unsafe exact profile",
            "validation_errors":[], "events":[]
        })).unwrap()
    }

    #[test]
    fn profile_hold_is_not_a_refresh_or_checkpoint_retry() {
        let mut item = item();
        item.verification_details = Some(
            json!({"blocker_codes":["verification_checkpoint_stale", "verification_profile_unapproved"]}),
        );
        assert_eq!(
            super::super::require_claimable_autonomous_item(&item),
            Err(StatusCode::CONFLICT)
        );
        let (status, body) = explain(StatusCode::CONFLICT, &item);
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error_code, "verification_profile_unapproved");
        assert!(!body.retryable);
        assert!(body.next_action.contains("original committed tests"));
    }

    #[test]
    fn stale_checkpoints_wait_but_do_not_emit_a_claim_plan() {
        let mut item = item();
        item.verification_readiness_reason = "verification_checkpoint_stale: unavailable".into();
        let (_, body) = explain(StatusCode::CONFLICT, &item);
        assert_eq!(body.error_code, "verification_checkpoint_stale");
        assert!(body.retryable);
        assert!(body.next_action.starts_with("Wait until"));
        let json = serde_json::to_value(&body.0).unwrap();
        assert_eq!(json["schema_version"], "agent-bounties/action-error-v1");
        assert!(json.get("calls").is_none());
        assert!(json.get("eip3009_authorization").is_none());
    }

    #[test]
    fn terms_status_and_recovery_holds_override_checkpoint_advice() {
        let mut item = item();
        item.verification_details =
            Some(json!({"blocker_codes":["verification_checkpoint_stale"]}));
        item.terms_valid = false;
        assert_eq!(classify(&item).0, "verification_terms_not_ready");
        item.terms_valid = true;
        item.status = "settled".into();
        assert_eq!(classify(&item).0, "claim_not_available");
        item.status = "claimable".into();
        item.verification_readiness_reason =
            chain_base::RECOVERY_RESERVED_VERIFICATION_REASON.into();
        assert_eq!(classify(&item).0, "claim_recovery_reserved");
    }

    #[test]
    fn eligibility_and_delivery_cutoff_preserve_the_existing_gate() {
        let mut item = item();
        item.verification_ready = true;
        item.verification_readiness_reason = "ready".into();
        item.claim_eligibility = Some(
            serde_json::from_value(json!({
                "registry":"0x3333333333333333333333333333333333333333", "source":"fixture"
            }))
            .unwrap(),
        );
        assert_eq!(classify(&item).0, "claim_eligibility_required");
        assert!(super::super::require_claimable_autonomous_item(&item).is_err());
        item.claim_eligibility = None;
        let now = chrono::Utc::now();
        item.terms = Some(serde_json::from_value(json!({
            "terms_hash":"fixture", "policy_hash":"fixture", "acceptance_criteria_hash":"fixture",
            "benchmark_hash":"fixture", "evidence_schema_hash":"fixture", "creator_wallet":item.creator,
            "created_at":now, "document":{
                "schema_version":"fixture", "contract_terms":{}, "title":"fixture", "goal":"fixture",
                "acceptance_criteria":[], "benchmark":{"engine":"creator_review_v1", "delivery_deadline":now.timestamp()},
                "evidence_schema":{}, "verification_policy":{}
            }
        })).unwrap());
        assert_eq!(classify(&item).0, "delivery_deadline_passed");
        assert!(super::super::require_claimable_autonomous_item(&item).is_err());
        item.terms.as_mut().unwrap().document.benchmark["delivery_deadline"] =
            json!(now.timestamp() + 3600);
        assert!(super::super::require_claimable_autonomous_item(&item).is_ok());
        // Missing timing remains blocked, but is not falsely described as a passed deadline.
        item.terms.as_mut().unwrap().document.benchmark["delivery_deadline"] =
            serde_json::Value::Null;
        assert!(super::super::require_claimable_autonomous_item(&item).is_err());
        assert_eq!(classify(&item).0, "verification_unavailable");
    }

    #[test]
    fn unknown_stored_text_and_evidence_are_not_reflected() {
        let mut item = item();
        item.verification_readiness_reason =
            "secret fixture content https://private.invalid/secret".into();
        item.validation_errors = vec!["secret evidence".into()];
        item.verification_details =
            Some(json!({"blocker_codes":["secret arbitrary code"], "reason":"secret"}));
        let (_, body) = explain(StatusCode::CONFLICT, &item);
        assert_eq!(body.error_code, "verification_unavailable");
        assert!(!serde_json::to_string(&body.0).unwrap().contains("secret"));
        item.verification_ready = true;
        assert!(super::super::require_claimable_autonomous_item(&item).is_ok());
    }
}
