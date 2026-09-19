import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-api.mjs");

if (!existsSync(checker)) {
  console.error(`missing child implementation: ${checker}`);
  process.exit(1);
}

const ready = {
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
};

const invalidProtocolErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "protocol_asset_mismatch",
  "protocol_token_mismatch",
  "protocol_inactive",
  "api_base_mismatch",
  "required_route_missing:/v1/base/autonomous-bounties/feed",
  "required_route_missing:/v1/base/autonomous-bounties/inventory-summary",
  "required_route_missing:/v1/base/autonomous-bounties/leaderboard",
  "required_route_missing:/api-docs/openapi.json",
];

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
    name: "missing required route",
    args: [join(benchmarkRoot, "fixtures", "missing-endpoint.json")],
    status: 1,
    output: {
      ready: false,
      errors: ["required_route_missing:/api-docs/openapi.json"],
    },
  },
  {
    name: "wrong route and protocol",
    args: [join(benchmarkRoot, "fixtures", "wrong-route.json")],
    status: 1,
    output: { ready: false, errors: invalidProtocolErrors },
  },
  {
    name: "valid API discovery manifest",
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
    throw new Error(`${testCase.name}: ${result.error.message}`);
  }
  if (result.status !== testCase.status) {
    throw new Error(
      `${testCase.name}: expected status ${testCase.status}, received ${result.status}\nstderr: ${result.stderr}\nstdout: ${result.stdout}`,
    );
  }
  if (result.stderr !== "") {
    throw new Error(
      `${testCase.name}: expected empty stderr, received ${JSON.stringify(result.stderr)}`,
    );
  }
  let payload;
  try {
    payload = JSON.parse(result.stdout.trim());
  } catch (error) {
    throw new Error(
      `${testCase.name}: invalid JSON output: ${JSON.stringify(result.stdout)} (${error.message})`,
    );
  }
  if (JSON.stringify(payload) !== JSON.stringify(testCase.output)) {
    throw new Error(
      `${testCase.name}: output mismatch\nexpected: ${JSON.stringify(testCase.output)}\nreceived: ${JSON.stringify(payload)}`,
    );
  }
}

console.log("api_discovery_benchmark=passed");
