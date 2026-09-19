import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-wallet.mjs");

if (!existsSync(checker)) {
  console.error(`missing child implementation: ${checker}`);
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  api_base: "https://api.agentbounties.app",
  agent_wallet_readiness: "https://api.agentbounties.app/v1/base/agent-wallet/readiness",
  required_capabilities: [
    "check_wallet_readiness",
    "agent_native_claim",
    "prepare_work_recovery",
    "open_action_review",
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
  "wallet_readiness_endpoint_mismatch",
  "required_capability_missing:check_wallet_readiness",
  "required_capability_missing:agent_native_claim",
  "required_capability_missing:prepare_work_recovery",
  "required_capability_missing:open_action_review",
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
    name: "missing required capability",
    args: [join(benchmarkRoot, "fixtures", "missing-capability.json")],
    status: 1,
    output: {
      ready: false,
      errors: ["required_capability_missing:open_action_review"],
    },
  },
  {
    name: "wrong protocol and capabilities",
    args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")],
    status: 1,
    output: { ready: false, errors: invalidProtocolErrors },
  },
  {
    name: "valid wallet UX manifest",
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

console.log("wallet_ux_benchmark=passed");
