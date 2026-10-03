import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-gmv-forward-month-20260824.mjs");

if (!existsSync(checker)) {
  console.error(`missing child implementation: ${checker}`);
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  competition: "0x8c990ddf5360c00ee0b2090000e3a3a6f90a6a9d",
  bounty_id: "0x54684252bfcece3e9258af16f337391498b1fefd9cbb7690abe149ad5bee60c6",
  gmv_base_units: 900000,
  status: "eligible",
};

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
    name: "self-dealing disallowed",
    args: [join(benchmarkRoot, "fixtures", "self-dealing.json")],
    status: 1,
    output: { ready: false, errors: ["self_dealing_disallowed"] },
  },
  {
    name: "outside scoring window",
    args: [join(benchmarkRoot, "fixtures", "outside-window.json")],
    status: 1,
    output: { ready: false, errors: ["settlement_outside_scoring_window"] },
  },
  {
    name: "excluded wallet disallowed",
    args: [join(benchmarkRoot, "fixtures", "excluded-wallet.json")],
    status: 1,
    output: { ready: false, errors: ["excluded_wallet_disallowed"] },
  },
  {
    name: "excluded contract disallowed",
    args: [join(benchmarkRoot, "fixtures", "excluded-contract.json")],
    status: 1,
    output: { ready: false, errors: ["excluded_contract_disallowed"] },
  },
  {
    name: "valid settlement attribution",
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
      `${testCase.name}: expected stdout ${JSON.stringify(expected)}, received ${JSON.stringify(result.stdout)}`,
    );
  }
}

console.log(`gmv_forward_month_20260824_benchmark=passed cases=${cases.length}`);
