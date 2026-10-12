import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-api-benchmark-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-api.mjs"), implementation);
  return root;
}

function run(root) {
  return spawnSync(process.execPath, [runner, root], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
}

const knownGood = `
import { readFileSync } from "node:fs";
const requiredRoutes = [
  "/v1/base/autonomous-bounties/feed",
  "/v1/base/autonomous-bounties/inventory-summary",
  "/v1/base/autonomous-bounties/leaderboard",
  "/api-docs/openapi.json",
];
const fail = (status, error) => {
  const payload = typeof error === "string" ? [error] : error;
  process.stdout.write(JSON.stringify({ ready: false, errors: payload }) + "\\n");
  process.exit(status);
};
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let manifest;
try { manifest = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (manifest === null || Array.isArray(manifest) || typeof manifest !== "object") fail(2, "manifest_root_object_required");
const protocol = manifest.protocol ?? {};
const endpoints = manifest.endpoints ?? {};
const routes = Array.isArray(manifest.supported_routes) ? manifest.supported_routes : [];
const errors = [];
if (manifest.schema !== "https://agentbounties.app/schemas/api-discovery-manifest.v1.json") errors.push("schema_mismatch");
if (protocol.network !== "base-mainnet") errors.push("protocol_network_mismatch");
if (protocol.chain_id !== 8453) errors.push("protocol_chain_id_mismatch");
if (protocol.asset !== "USDC") errors.push("protocol_asset_mismatch");
if (String(protocol.token ?? "").toLowerCase() !== "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913") errors.push("protocol_token_mismatch");
if (protocol.deployment_status !== "active") errors.push("protocol_inactive");
if (endpoints.api_base !== "https://api.agentbounties.app") errors.push("api_base_mismatch");
for (const route of requiredRoutes) {
  if (!routes.includes(route)) errors.push("required_route_missing:" + route);
}
if (errors.length > 0) fail(1, errors);
process.stdout.write(JSON.stringify({
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  api_base: "https://api.agentbounties.app",
  required_routes: requiredRoutes,
}) + "\\n");
`;

const alwaysReady = `
process.stdout.write(JSON.stringify({
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  api_base: "https://api.agentbounties.app",
  required_routes: [
    "/v1/base/autonomous-bounties/feed",
    "/v1/base/autonomous-bounties/inventory-summary",
    "/v1/base/autonomous-bounties/leaderboard",
    "/api-docs/openapi.json",
  ],
}) + "\\n");
`;

try {
  const good = run(source("known-good", knownGood));
  if (good.status !== 0) {
    throw new Error(`known-good fixture failed: ${good.stdout}${good.stderr}`);
  }
  const bad = run(source("known-bad", alwaysReady));
  if (bad.status === 0) {
    throw new Error("known-bad fixture unexpectedly passed");
  }
  const missing = run(join(temporary, "missing"));
  if (missing.status === 0) {
    throw new Error("missing implementation unexpectedly passed");
  }
  console.log("api_discovery_benchmark_self_test=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
