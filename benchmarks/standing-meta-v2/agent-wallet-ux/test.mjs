import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-wallet-ux.mjs");

if (!existsSync(checker)) {
  console.error();
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  wallet_ux_version: "2.0",
  required_elements: ["balance_display", "send_form", "receive_qr", "tx_history", "gas_estimator"],
  confirmation_timeout: 30,
  supported_types: ["browser_extension", "mobile_deep_link", "web_wallet"],
};

const invalidErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "protocol_asset_mismatch",
  "protocol_token_mismatch",
  "wallet_ux_version_mismatch",
  "required_element_missing:balance_display",
  "required_element_missing:send_form",
  "required_element_missing:receive_qr",
  "required_element_missing:tx_history",
  "required_element_missing:gas_estimator",
  "confirmation_timeout_mismatch",
  "supported_type_unknown",
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
    name: "missing required field",
    args: [join(benchmarkRoot, "fixtures", "missing-field.json")],
    status: 1,
    output: { ready: false, errors: invalidErrors },
  },
  {
    name: "wrong protocol",
    args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")],
    status: 1,
    output: { ready: false, errors: invalidErrors },
  },
  {
    name: "valid manifest",
    args: [join(benchmarkRoot, "fixtures", "valid.json")],
    status: 0,
    output: ready,
  },
];

let passed = 0;
let failed = 0;

for (const c of cases) {
  const args = [runner, ...c.args];
  if (c.args.length > 0) args[1] = c.args[0];
  const result = spawnSync(process.execPath, [checker, ...c.args], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });

  const actualStatus = result.status ?? (result.error ? 1 : 0);
  let actualOutput;
  try {
    actualOutput = JSON.parse((result.stdout ?? "").trim());
  } catch {
    actualOutput = null;
  }

  const statusOk = actualStatus === c.status;
  const outputOk =
    c.status !== 0
      ? actualOutput !== null && actualOutput.ready === false && Array.isArray(actualOutput.errors)
      : JSON.stringify(actualOutput) === JSON.stringify(c.output);

  if (statusOk && outputOk) {
    passed++;
  } else {
    failed++;
    const errMsg = result.error ? result.error.message : (result.stderr || "");
    console.error(`FAIL ${c.name}: status=${actualStatus}(exp ${c.status}) output=${JSON.stringify(actualOutput)} err=${errMsg}`);
  }
}

console.log(`${passed}/${cases.length} passed`);
process.exit(failed > 0 ? 1 : 0);
