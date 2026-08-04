//! Deterministic MCP / API tool-registry drift test.
//!
//! Compares the committed `tool-registry.json` fixture with the set of
//! required read-only discovery and readiness tools and fails closed when a
//! required tool is missing or renamed.
//!
//! Runnable fully offline from the committed fixture; emits one concise
//! failure summary.

use std::collections::HashSet;

/// The required read-only discovery and readiness tools that MUST be present
/// in the tool registry.
const REQUIRED_TOOLS: &[&str] = &[
    "list_autonomous_bounties",
    "list_opportunities",
    "prepare_agent_to_earn",
    "prepare_bounty_post",
];

/// The committed tool-registry fixture content (embedded at compile-time so
/// the test runs offline with zero I/O).
const TOOL_REGISTRY_JSON: &str =
    include_str!("../fixtures/tool-registry.json");

/// Parse the `schema_version` and `tools` array from the registry JSON.
fn parse_registry(json_str: &str) -> Result<(String, Vec<String>), String> {
    let value: serde_json::Value =
        serde_json::from_str(json_str).map_err(|e| format!("failed to parse registry JSON: {e}"))?;

    let schema_version = value
        .get("schema_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "missing or non-string 'schema_version' field in registry".to_string())?;

    let tools = value
        .get("tools")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing or non-array 'tools' field in registry".to_string())?;

    let parsed_tools = tools
        .iter()
        .map(|v| {
            v.as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| "tool entry is not a string".to_string())
        })
        .collect();

    Ok((schema_version, parsed_tools))
}

/// Checks the registry for missing, duplicate, extra, or renamed required tools, and verifies schema version.
/// Returns Ok(()) on success or a concise failure summary.
fn check_tool_registry_drift(json_str: &str) -> Result<(), String> {
    let (schema_version, tool_names) = parse_registry(json_str)?;

    if schema_version != "agent-bounties/mcp-tool-registry-v1" {
        return Err(format!("Schema version drift detected: {}", schema_version));
    }

    // Check for duplicates.
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for name in &tool_names {
        if !seen.insert(name.as_str()) {
            duplicates.push(name.as_str());
        }
    }

    let tool_set: HashSet<&str> = tool_names.iter().map(|s| s.as_str()).collect();

    let mut missing = Vec::new();
    for required in REQUIRED_TOOLS {
        if !tool_set.contains(required) {
            missing.push(*required);
        }
    }

    let mut extra = Vec::new();
    for tool in &tool_set {
        if !REQUIRED_TOOLS.contains(tool) {
            extra.push(*tool);
        }
    }

    if missing.is_empty() && duplicates.is_empty() && extra.is_empty() {
        return Ok(());
    }

    let mut summary = String::from("Tool registry drift detected:");
    if !missing.is_empty() {
        summary.push_str(&format!("\n  Missing tools: {:?}", missing));
    }
    if !duplicates.is_empty() {
        summary.push_str(&format!("\n  Duplicate tools: {:?}", duplicates));
    }
    if !extra.is_empty() {
        summary.push_str(&format!("\n  Extra tools: {:?}", extra));
    }
    Err(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_fixture_contains_all_required_tools() {
        if let Err(summary) = check_tool_registry_drift(TOOL_REGISTRY_JSON) {
            panic!("{summary}");
        }
    }

    #[test]
    fn detects_missing_tool() {
        // Registry with one required tool removed.
        let json = r#"{
            "schema_version": "agent-bounties/mcp-tool-registry-v1",
            "tools": [
                "list_autonomous_bounties",
                "list_opportunities",
                "prepare_bounty_post"
            ]
        }"#;
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("prepare_agent_to_earn"),
            "error should name the missing tool, got: {err}"
        );
    }

    #[test]
    fn detects_renamed_tool() {
        // `list_opportunities` renamed to `list_opps`.
        let json = r#"{
            "schema_version": "agent-bounties/mcp-tool-registry-v1",
            "tools": [
                "list_autonomous_bounties",
                "list_opps",
                "prepare_agent_to_earn",
                "prepare_bounty_post"
            ]
        }"#;
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("list_opportunities"),
            "error should name the original tool, got: {err}"
        );
    }

    #[test]
    fn detects_duplicate_tool() {
        let json = r#"{
            "schema_version": "agent-bounties/mcp-tool-registry-v1",
            "tools": [
                "list_autonomous_bounties",
                "list_autonomous_bounties",
                "list_opportunities",
                "prepare_agent_to_earn",
                "prepare_bounty_post"
            ]
        }"#;
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Duplicate"),
            "error should mention duplicates, got: {err}"
        );
    }

    #[test]
    fn rejects_invalid_json() {
        let result = check_tool_registry_drift("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn rejects_missing_tools_field() {
        let result = check_tool_registry_drift(r#"{"schema_version": "v1"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn fixture_distinguishes_mcp_transport_from_json_inventory() {
        // The fixture is the JSON tool *inventory* (a static list of tool names).
        // The MCP *transport* endpoint is `/mcp` (Streamable HTTP / JSON-RPC).
        // The inventory endpoint is `/tools` (plain JSON).
        // This test ensures the fixture itself does not contain transport metadata
        // (i.e. no JSON-RPC method or endpoint fields), confirming it is purely
        // the tool inventory.
        let value: serde_json::Value = serde_json::from_str(TOOL_REGISTRY_JSON).unwrap();
        assert!(
            value.get("jsonrpc").is_none(),
            "fixture must not contain JSON-RPC transport fields"
        );
        assert!(
            value.get("method").is_none(),
            "fixture must not contain JSON-RPC method field"
        );
        // It MUST have `schema_version` to identify it as inventory.
        assert!(
            value.get("schema_version").is_some(),
            "fixture must have schema_version identifying it as inventory"
        );
    }

    #[test]
    fn existing_public_tool_names_are_unchanged() {
        let (_, tool_names) = parse_registry(TOOL_REGISTRY_JSON).unwrap();
        for required in REQUIRED_TOOLS {
            assert!(
                tool_names.contains(&required.to_string()),
                "required tool '{required}' not found in committed fixture"
            );
        }
    }

    #[test]
    fn parses_and_rejects_missing_fixture() {
        let json = include_str!("../../../fixtures/registry/missing.json");
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing tools:"));
    }

    #[test]
    fn parses_and_rejects_renamed_fixture() {
        let json = include_str!("../../../fixtures/registry/renamed.json");
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Missing tools:"));
        assert!(err.contains("Extra tools:"));
    }

    #[test]
    fn parses_and_rejects_extra_fixture() {
        let json = include_str!("../../../fixtures/registry/extra.json");
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Extra tools:"));
    }

    #[test]
    fn parses_and_rejects_docs_drift_fixture() {
        let json = include_str!("../../../fixtures/registry/docs-drift.json");
        let result = check_tool_registry_drift(json);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Schema version drift detected"));
    }
}
