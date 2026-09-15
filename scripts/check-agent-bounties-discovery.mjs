import { readFileSync } from "node:fs";

/**
 * Write failure payload to standard output and exit with status code.
 * @param {number} status
 * @param {string[]} errors
 */
function fail(status, errors) {
  process.stdout.write(JSON.stringify({ ready: false, errors }) + "\n");
  process.exit(status);
}

/**
 * Validate Agent Bounties discovery manifest file against canonical requirements.
 */
function main() {
  if (process.argv.length !== 3) {
    fail(2, ["manifest_path_required"]);
  }

  const manifestPath = process.argv[2];
  let text = "";
  try {
    text = readFileSync(manifestPath, "utf8");
  } catch {
    fail(2, ["manifest_unreadable"]);
  }

  let manifest = null;
  try {
    manifest = JSON.parse(text);
  } catch {
    fail(2, ["manifest_invalid_json"]);
  }

  if (manifest === null || Array.isArray(manifest) || typeof manifest !== "object") {
    fail(2, ["manifest_root_object_required"]);
  }

  const requiredTools = [
    "route_blocked_goal",
    "prepare_agent_to_earn",
    "agent_native_claim",
    "prepare_standing_meta_v2_child",
  ];

  const protocol = (manifest.protocol && typeof manifest.protocol === "object") ? manifest.protocol : {};
  const endpoints = (manifest.endpoints && typeof manifest.endpoints === "object") ? manifest.endpoints : {};
  const tools = Array.isArray(manifest.agent_tools) ? manifest.agent_tools : [];

  const errors = [];
  if (manifest.schema !== "https://agentbounties.app/schemas/discovery-manifest.v2.json") {
    errors.push("schema_mismatch");
  }
  if (protocol.network !== "base-mainnet") {
    errors.push("protocol_network_mismatch");
  }
  if (protocol.chain_id !== 8453) {
    errors.push("protocol_chain_id_mismatch");
  }
  if (protocol.asset !== "USDC") {
    errors.push("protocol_asset_mismatch");
  }
  if (String(protocol.token ?? "").toLowerCase() !== "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913") {
    errors.push("protocol_token_mismatch");
  }
  if (protocol.deployment_status !== "active") {
    errors.push("protocol_inactive");
  }
  if (endpoints.api_base !== "https://api.agentbounties.app") {
    errors.push("api_endpoint_mismatch");
  }
  if (endpoints.mcp_tools !== "https://mcp.agentbounties.app/tools") {
    errors.push("mcp_endpoint_mismatch");
  }
  if (endpoints.autonomous_standing_meta_v2_child_preparation !== "https://api.agentbounties.app/v1/base/autonomous-bounties/standing-meta-v2-child-preparation") {
    errors.push("standing_meta_endpoint_mismatch");
  }

  for (const tool of requiredTools) {
    if (!tools.includes(tool)) {
      errors.push("required_tool_missing:" + tool);
    }
  }

  if (errors.length > 0) {
    fail(1, errors);
  }

  const successPayload = {
    ready: true,
    network: "base-mainnet",
    asset: "USDC",
    api_base: "https://api.agentbounties.app",
    mcp_tools: "https://mcp.agentbounties.app/tools",
    required_tools: requiredTools,
  };
  process.stdout.write(JSON.stringify(successPayload) + "\n");
  process.exit(0);
}

main();
