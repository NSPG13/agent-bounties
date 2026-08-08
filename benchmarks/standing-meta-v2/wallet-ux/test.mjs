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
  api_base: "https://api.agentbounties.app",
  wallet_provider: "bounded-wallet",
  required_features: ["gasless_claim", "sponsored_creation", "bond_refund", "wallet_relay"],
};

const invalidProtocolErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "protocol_asset_mismatch",
  "protocol_token_mismatch",
  "protocol_inactive",
  "wallet_provider_mismatch",
  "api_endpoint_mismatch",
  "required_feature_missing:gasless_claim",
  "required_feature_missing:sponsored_creation",
  "required_feature_missing:bond_refund",
  "required_feature_missing:wallet_relay",
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
    throw new Error();
  }
  if (result.status !== testCase.status) {
    throw new Error(
      ,
    );
  }
  if (result.stderr !== "") {
    throw new Error();
  }
  const expected = ;
  if (result.stdout !== expected) {
    throw new Error(
      ,
    );
  }
}

console.log();
