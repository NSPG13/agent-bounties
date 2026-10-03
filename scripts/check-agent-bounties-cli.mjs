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
 * Validate Agent Bounties CLI manifest file against canonical requirements.
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

  const requiredCommands = [
    "claim-next-action",
    "select-funded-bounty",
    "verify-settlement-evidence",
    "prepare-child-bounty",
  ];

  const protocol = (manifest.protocol && typeof manifest.protocol === "object") ? manifest.protocol : {};
  const endpoints = (manifest.endpoints && typeof manifest.endpoints === "object") ? manifest.endpoints : {};
  const commands = Array.isArray(manifest.required_commands) ? manifest.required_commands : [];

  const errors = [];
  if (manifest.schema !== "https://agentbounties.app/schemas/cli-manifest.v1.json") {
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
  if (endpoints.cli_home !== "https://agentbounties.app/cli") {
    errors.push("cli_home_mismatch");
  }

  for (const cmd of requiredCommands) {
    if (!commands.includes(cmd)) {
      errors.push("required_command_missing:" + cmd);
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
    cli_home: "https://agentbounties.app/cli",
    required_commands: requiredCommands,
  };
  process.stdout.write(JSON.stringify(successPayload) + "\n");
  process.exit(0);
}

main();
