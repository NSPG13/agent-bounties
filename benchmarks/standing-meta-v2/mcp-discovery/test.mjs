import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-discovery.mjs");

if (!existsSync(checker)) {
  console.error(`missing child implementation: ${checker}`);
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  api_base: "https://api.bountyboard.global",
  mcp_tools: "https://mcp.bountyboard.global/tools",
  required_tools: [
    "route_blocked_goal",
    "prepare_agent_to_earn",
    "agent_native_claim",
    "prepare_standing_meta_v2_child",
  ],
};

const invalidProtocolErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "protocol_asset_mismatch",
  "protocol_token_mismatch",
  "protocol_inactive",
  "api_endpoint_mismatch",
  "mcp_endpoint_mismatch",
  "standing_meta_endpoint_mismatch",
  "required_tool_missing:route_blocked_goal",
  "required_tool_missing:prepare_agent_to_earn",
  "required_tool_missing:agent_native_claim",
  "required_tool_missing:prepare_standing_meta_v2_child",
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
    name: "missing required tool",
    args: [join(benchmarkRoot, "fixtures", "missing-tool.json")],
    status: 1,
    output: {
      ready: false,
      errors: ["required_tool_missing:prepare_standing_meta_v2_child"],
    },
  },
  {
    name: "wrong protocol",
    args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")],
    status: 1,
    output: { ready: false, errors: invalidProtocolErrors },
  },
  {
    name: "valid discovery manifest",
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
      `${testCase.name}: expected exit ${testCase.status}, received ${result.status}; stdout=${JSON.stringify(result.stdout)} stderr=${JSON.stringify(result.stderr)}`,
    );
  }
  if (result.stderr !== "") {
    throw new Error(`${testCase.name}: stderr must be empty: ${JSON.stringify(result.stderr)}`);
  }
  const expected = `${JSON.stringify(testCase.output)}\n`;
  if (result.stdout !== expected) {
    throw new Error(
      `${testCase.name}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(result.stdout)}`,
    );
  }
}

console.log(`mcp_discovery_benchmark=passed cases=${cases.length}`);
