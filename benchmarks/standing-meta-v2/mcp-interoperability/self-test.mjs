import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-mcp-interoperability-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-mcp-interop.mjs"), implementation);
  return root;
}

function run(root) {
  return spawnSync(process.execPath, [runner, root], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
}

const knownGood = `import { readFileSync } from "node:fs";
const fail = (status, error) => { console.log(JSON.stringify({ready:false,errors:[error]})); process.exit(status); };
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let manifest;
try { manifest = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (manifest === null || Array.isArray(manifest) || typeof manifest !== "object") fail(2, "manifest_root_object_required");
const errors = [];
if (manifest.schema !== "https://agentbounties.org/schemas/mcp-interop-manifest.v2.json") errors.push("schema_mismatch");
if (manifest.network !== "base-mainnet") errors.push("protocol_network_mismatch");
if (manifest.chain_id !== 8453) errors.push("protocol_chain_id_mismatch");
if (manifest.asset !== "USDC") errors.push("protocol_asset_mismatch");
if (String(manifest.token || "").toLowerCase() !== "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913") errors.push("protocol_token_mismatch");
if (manifest.deployment_status !== "active") errors.push("protocol_deployment_status_mismatch");
if (manifest.api_base !== "https://api.agentbounties.app") errors.push("protocol_api_base_mismatch");
if (JSON.stringify(manifest.supported_transports) !== '["stdio", "sse", "streamable-http"]') errors.push("protocol_supported_transports_mismatch");
if (manifest.mcp_version !== "2024-11-05") errors.push("protocol_mcp_version_mismatch");
if (JSON.stringify(manifest.required_tools) !== '["bounty_search", "bounty_claim", "wallet_status", "settlement_verify"]') errors.push("protocol_required_tools_mismatch");
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({"ready": true, "schema": "https://agentbounties.org/schemas/mcp-interop-manifest.v2.json", "network": "base-mainnet", "chain_id": 8453, "asset": "USDC", "token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "deployment_status": "active", "api_base": "https://api.agentbounties.app", "supported_transports": ["stdio", "sse", "streamable-http"], "mcp_version": "2024-11-05", "required_tools": ["bounty_search", "bounty_claim", "wallet_status", "settlement_verify"]}));`;

const alwaysReady = `console.log(JSON.stringify({"ready": true, "schema": "https://agentbounties.org/schemas/mcp-interop-manifest.v2.json", "network": "base-mainnet", "chain_id": 8453, "asset": "USDC", "token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "deployment_status": "active", "api_base": "https://api.agentbounties.app", "supported_transports": ["stdio", "sse", "streamable-http"], "mcp_version": "2024-11-05", "required_tools": ["bounty_search", "bounty_claim", "wallet_status", "settlement_verify"]}));`;

try {
  const good = run(source("known-good", knownGood));
  if (good.status !== 0) {
    throw new Error("known-good fixture failed: " + good.stdout + good.stderr);
  }
  const bad = run(source("known-bad", alwaysReady));
  if (bad.status === 0) {
    throw new Error("known-bad fixture unexpectedly passed");
  }
  const missing = run(join(temporary, "missing"));
  if (missing.status === 0) {
    throw new Error("missing implementation unexpectedly passed");
  }
  console.log("mcp-interoperability_benchmark_self_test=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
