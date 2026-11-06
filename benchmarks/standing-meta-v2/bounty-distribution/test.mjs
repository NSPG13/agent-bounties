import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const sourceRoot = resolve(process.argv[2] ?? "/workspace");
const checker = join(sourceRoot, "scripts", "check-agent-bounties-bounty-distribution.mjs");

if (!existsSync(checker)) {
  console.error();
  process.exit(1);
}

const ready = {"ready": true, "channels": ["github-issues", "rss", "api-feed"], "claim_instruction": "/claim #ISSUE wallet: 0xYourBaseWallet", "payment_evidence": "BountySettled", "reward_token": "USDC", "network": "base-mainnet"};

const invalidErrors = ["schema_mismatch", "channel_missing:github-issues", "channel_missing:rss", "channel_missing:api-feed", "claim_instruction_missing:/claim", "payment_evidence_mismatch", "reward_token_mismatch", "network_mismatch"];

const cases = [
  { name: "missing argument", args: [], status: 2, output: { ready: false, errors: ["manifest_path_required"] } },
  { name: "unreadable manifest", args: [join(benchmarkRoot, "fixtures", "absent.json")], status: 2, output: { ready: false, errors: ["manifest_unreadable"] } },
  { name: "malformed JSON", args: [join(benchmarkRoot, "fixtures", "malformed.json")], status: 2, output: { ready: false, errors: ["manifest_invalid_json"] } },
  { name: "non-object root", args: [join(benchmarkRoot, "fixtures", "not-an-object.json")], status: 2, output: { ready: false, errors: ["manifest_root_object_required"] } },
  { name: "missing required field", args: [join(benchmarkRoot, "fixtures", "missing-field.json")], status: 1, output: { ready: false, errors: invalidErrors } },
  { name: "wrong channel scheme", args: [join(benchmarkRoot, "fixtures", "wrong-protocol.json")], status: 1, output: { ready: false, errors: invalidErrors } },
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
