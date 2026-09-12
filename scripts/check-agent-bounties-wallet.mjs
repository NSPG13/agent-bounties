import { readFileSync } from "node:fs";

/**
 * Write failure payload to standard output and terminate the process.
 * @param {number} status
 * @param {string[]} errors
 */
function fail(status, errors) {
  process.stdout.write(JSON.stringify({ ready: false, errors }) + "\n");
  process.exit(status);
}

/**
 * Validate Agent Bounties wallet UX manifest file against canonical requirements.
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

  const requiredCapabilities = [
    "check_wallet_readiness",
    "agent_native_claim",
    "prepare_work_recovery",
    "open_action_review",
  ];

  const protocol = (manifest.protocol && typeof manifest.protocol === "object") ? manifest.protocol : {};
  const endpoints = (manifest.endpoints && typeof manifest.endpoints === "object") ? manifest.endpoints : {};
  const capabilities = Array.isArray(manifest.required_capabilities) ? manifest.required_capabilities : [];

  const errors = [];
  if (manifest.schema !== "https://agentbounties.app/schemas/wallet-ux-manifest.v1.json") {
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
    errors.push("api_base_mismatch");
  }
  if (endpoints.agent_wallet_readiness !== "https://api.agentbounties.app/v1/base/agent-wallet/readiness") {
    errors.push("wallet_readiness_endpoint_mismatch");
  }

  for (const cap of requiredCapabilities) {
    if (!capabilities.includes(cap)) {
      errors.push("required_capability_missing:" + cap);
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
    agent_wallet_readiness: "https://api.agentbounties.app/v1/base/agent-wallet/readiness",
    required_capabilities: requiredCapabilities,
  };
  process.stdout.write(JSON.stringify(successPayload) + "\n");
  process.exit(0);
}

main();
