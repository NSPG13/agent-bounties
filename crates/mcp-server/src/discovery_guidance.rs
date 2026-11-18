//! Guidance for failed read-only discovery, shared by HTTP and both MCP eras.

use axum::Json;
use serde_json::{json, Value};

pub(super) const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub(super) fn unavailable(tool: &str) -> Option<Value> {
    if !matches!(
        tool,
        "get_bounty_feed"
            | "render_bounty_feed"
            | "list_opportunities"
            | "list_autonomous_bounties"
            | "get_autonomous_inventory_summary"
    ) {
        return None;
    }
    Some(json!({
        "schema_version": "agent-bounties/discovery-problem-v1",
        "error_code": "upstream_unavailable",
        "message": "Agentizable's work inventory is temporarily unavailable. Work availability is unknown; this does not mean there are no matching tasks.",
        "coverage": "unavailable",
        "degraded": true,
        "retryable": true,
        "retry_policy": {"maximum_retries": 1, "same_arguments": true},
        "next_action": "Retry the same read-only discovery request at most once, keeping all filters unchanged. If it still fails, explain that availability could not be checked and stop until a later check. Do not ask for wallet setup, funds or signatures to fix this outage, start work, or substitute unfunded requests for paid work.",
        "evidence_boundary": "No inventory, funding, claim or payment evidence was obtained."
    }))
}

pub(super) fn attach_problem(mut response: Json<Value>, tool: &str) -> Json<Value> {
    if response.0.get("error_code").and_then(Value::as_str) == Some("upstream_unavailable") {
        if let (Some(object), Some(guidance)) = (response.0.as_object_mut(), unavailable(tool)) {
            object.extend(guidance.as_object().expect("guidance is an object").clone());
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_inventory_never_becomes_empty_inventory() {
        let response = attach_problem(
            Json(json!({"error":"safe diagnostic", "error_code":"upstream_unavailable"})),
            "list_autonomous_bounties",
        )
        .0;
        assert_eq!(response["error"], "safe diagnostic");
        assert_eq!(response["coverage"], "unavailable");
        assert_eq!(response["degraded"], true);
        assert_eq!(response["retry_policy"]["maximum_retries"], 1);
        for field in ["items", "bounties", "item_count", "content"] {
            assert!(response.get(field).is_none(), "invented inventory: {field}");
        }
    }

    #[test]
    fn success_and_argument_errors_are_unchanged() {
        for value in [
            json!({"content":[{"json":{"items":[],"coverage":"complete","degraded":false}}]}),
            json!({"error":"invalid network"}),
        ] {
            assert_eq!(
                attach_problem(Json(value.clone()), "list_autonomous_bounties").0,
                value
            );
        }
    }

    #[test]
    fn retry_guidance_never_applies_to_writes_or_payment_status() {
        for tool in [
            "agent_native_claim",
            "prepare_bounty_post",
            "get_bounty_action_status",
            "fund_bounty_with_x402",
            "publish_unfunded_bounty",
        ] {
            assert!(
                unavailable(tool).is_none(),
                "unsafe retry guidance for {tool}"
            );
        }
    }
}
