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

const ready = {"ready": true, "binary": "agent-bounty-cli", "commands": ["list", "claim", "register", "status"], "exit_codes": [0, 1, 2], "help": true, "node_version": ">=20"};

const invalidErrors = ["schema_mismatch", "binary_mismatch", "command_missing:list", "command_missing:claim", "command_missing:register", "exit_code_mismatch:2", "help_missing", "node_version_mismatch"];

const cases = [
  { name: "missing argument", args: [], status: 2, output: { ready: false, errors: ["manifest_path_required"] } },
  { name: "unreadable manifest", args: [join(benchmarkRoot, "fixtures", "absent.json")], status: 2, output: { ready: false, errors: ["manifest_unreadable"] } },
  { name: "malformed JSON", args: [join(benchmarkRoot, "fixtures", "malformed.json")], status: 2, output: { ready: false, errors: ["manifest_invalid_json"] } },
  { name: "non-object root", args: [join(benchmarkRoot, "fixtures", "not-an-object.json")], status: 2, output: { ready: false, errors: ["manifest_root_object_required"] } },
  { name: "missing required field", args: [join(benchmarkRoot, "fixtures", "missing-field.json")], status: 1, output: { ready: false, errors: invalidErrors } },
  { name: "wrong binary name", args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")], status: 1, output: { ready: false, errors: invalidErrors } },
  { name: "valid manifest", args: [join(benchmarkRoot, "fixtures", "valid.json")], status: 0, output: ready },
];

let passed = 0;
let failed = 0;

for (const c of cases) {
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
  const outputOk = c.status !== 0 ? (actualOutput && actualOutput.ready === false && Array.isArray(actualOutput.errors)) : (actualOutput && actualOutput.ready === true);

  if (statusOk && outputOk) {
    passed++;
  } else {
    failed++;
    console.error(`FAIL ${c.name}: status=${actualStatus}/${c.status} output=${JSON.stringify(actualOutput)}`);
  }
}

console.log(`${passed}/${passed + failed} passed`);
process.exit(failed > 0 ? 1 : 0);
