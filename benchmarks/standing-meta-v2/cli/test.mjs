import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-cli.mjs");

if (!existsSync(checker)) {
  console.error();
  process.exit(1);
}

const ready = {
  ready: true,
  network: "base-mainnet",
  asset: "USDC",
  api_base: "https://api.agentbounties.app",
  cli_entry: "npx agent-bounties",
  required_commands: ["register", "claim", "status", "wallet", "bounties"],
};

const invalidProtocolErrors = [
  "schema_mismatch",
  "protocol_network_mismatch",
  "protocol_chain_id_mismatch",
  "protocol_asset_mismatch",
  "protocol_token_mismatch",
  "protocol_inactive",
  "api_endpoint_mismatch",
  "cli_entry_mismatch",
  "required_command_missing:register",
  "required_command_missing:claim",
  "required_command_missing:status",
  "required_command_missing:wallet",
  "required_command_missing:bounties",
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
