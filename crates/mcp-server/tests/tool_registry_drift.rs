use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn mcp_tools_match_discovery_manifest() {
    let root = workspace_root();

    let mcp_registry: Value = serde_json::from_slice(
        &fs::read(root.join("crates/mcp-server/fixtures/tool-registry.json")).unwrap(),
    )
    .unwrap();
    let mcp_tools: BTreeSet<&str> = mcp_registry["tools"]
        .as_array()
        .expect("MCP tool-registry fixture: tools must be an array")
        .iter()
        .map(|v| v.as_str().expect("MCP tool name must be a string"))
        .collect();

    let manifest_tools: Vec<String> = serde_json::from_slice(
        &fs::read(root.join("crates/mcp-server/fixtures/agent-manifest-tools.json")).unwrap(),
    )
    .unwrap();
    let manifest_tool_set: BTreeSet<&str> =
        manifest_tools.iter().map(|s| s.as_str()).collect();

    let required_tools = [
        "list_autonomous_bounties",
        "list_opportunities",
        "prepare_agent_to_earn",
        "prepare_bounty_post",
    ];

    let mut failures: Vec<String> = Vec::new();
    for tool in &required_tools {
        if !mcp_tools.contains(tool) {
            failures.push(format!(
                "[MCP endpoint /tools]  missing required tool: {tool}"
            ));
        }
        if !manifest_tool_set.contains(tool) {
            failures.push(format!(
                "[API manifest /.well-known/agent-bounties.json]  missing required tool: {tool}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Tool-registry drift detected ({} failure(s)):\n{}",
        failures.len(),
        failures.join("\n"),
    );
}
