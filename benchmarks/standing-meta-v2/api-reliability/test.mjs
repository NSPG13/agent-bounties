import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-api-reliability.mjs");

if (!existsSync(checker)) {
  console.error("missing:check-agent-bounties-api-reliability.mjs");
  process.exit(1);
}

const ready = {"ready": true, "schema": "https://agentbounties.org/schemas/api-reliability-manifest.v2.json", "network": "base-mainnet", "chain_id": 8453, "asset": "USDC", "token": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", "deployment_status": "active", "api_base": "https://api.agentbounties.app", "health_endpoint": "/api/health", "expected_status": 200, "max_latency_ms": 3000, "required_endpoints": ["/api/bounties", "/api/claim", "/api/settlement", "/api/agent/status"]};

const invalidProtocolErrors = ["schema_mismatch", "protocol_network_mismatch", "protocol_chain_id_mismatch", "protocol_asset_mismatch", "protocol_token_mismatch", "protocol_deployment_status_mismatch", "protocol_api_base_mismatch", "protocol_health_endpoint_mismatch", "protocol_expected_status_mismatch", "protocol_max_latency_ms_mismatch", "protocol_required_endpoints_mismatch"];

const cases = [
  {
    name: "missing argument",
    args: [],
    status: 2,
    output: { ready: false, errors: ["manifest_path_required"] },
  },
  {
    name: "unreadable manifest",
    args: [join(benchmarkRoot, "fixtures", "absent.json")],
    status: 2,
    output: { ready: false, errors: ["manifest_unreadable"] },
  },
  {
    name: "malformed JSON",
    args: [join(benchmarkRoot, "fixtures", "malformed.json")],
    status: 2,
    output: { ready: false, errors: ["manifest_invalid_json"] },
  },
  {
    name: "non-object root",
    args: [join(benchmarkRoot, "fixtures", "not-an-object.json")],
    status: 2,
    output: { ready: false, errors: ["manifest_root_object_required"] },
  },
  {
    name: "missing required field",
    args: [join(benchmarkRoot, "fixtures", "missing-field.json")],
    status: 1,
    output: { ready: false, errors: invalidProtocolErrors.slice(0, 1) },
  },
  {
    name: "wrong protocol",
    args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")],
    status: 1,
    output: { ready: false, errors: invalidProtocolErrors },
  },
  {
    name: "valid manifest",
    args: [join(benchmarkRoot, "fixtures", "valid.json")],
    status: 0,
    output: ready,
  },
];

for (const testCase of cases) {
  const result = spawnSync(process.execPath, [checker, ...testCase.args], {
    encoding: "utf8",
    timeout: 5_000,
    windowsHide: true,
  });
  if (result.error) {
    throw new Error(`${testCase.name}: spawn error ${result.error.message}`);
  }
  if (result.status !== testCase.status) {
    throw new Error(
      `${testCase.name}: expected status ${testCase.status} got ${result.status}`
    );
  }
  if (result.stderr !== "") {
    throw new Error(`${testCase.name}: unexpected stderr: ${result.stderr}`);
  }
  const expected = JSON.stringify(testCase.output);
  if (result.stdout.trim() !== expected) {
    throw new Error(
      `${testCase.name}: expected ${expected} got ${result.stdout.trim()}`
    );
  }
}

console.log("api-reliability_benchmark=passed");
