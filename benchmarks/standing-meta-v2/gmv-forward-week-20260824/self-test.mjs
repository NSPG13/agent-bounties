import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-gmv-benchmark-"));

/**
 * Write an implementation script into a temporary workspace.
 * @param {string} name Workspace folder name
 * @param {string} implementation Source code to place in the checker script
 * @returns {string} Root path of temporary workspace
 */
function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-gmv-forward-20260824.mjs"), implementation);
  return root;
}

/**
 * Run the benchmark test runner against the target workspace.
 * @param {string} root Target workspace directory
 * @returns {import("node:child_process").SpawnSyncReturns<string>} Spawn result
 */
function run(root) {
  return spawnSync(process.execPath, [runner, root], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
}

const originalCheckerPath = join(
  benchmarkRoot,
  "..",
  "..",
  "..",
  "scripts",
  "check-agent-bounties-gmv-forward-20260824.mjs",
);
const knownGood = readFileSync(originalCheckerPath, "utf8");

const alwaysReady = `
console.log(JSON.stringify({ready:true,network:"base-mainnet",competition:"0x2f1d2b24105596b153e473032256569fe544a44f",bounty_id:"0x4ffb4c5f75f2816b1d58c97ae440184bb5e75cbc158dc639b66a7e50e5d81417",gmv_base_units:900000,status:"eligible"}));
`;

try {
  const good = run(source("known-good", knownGood));
  if (good.status !== 0) {
    throw new Error(`known-good fixture failed: ${good.stdout}${good.stderr}`);
  }
  const bad = run(source("known-bad", alwaysReady));
  if (bad.status === 0) {
    throw new Error("known-bad fixture unexpectedly passed");
  }
  const missing = run(join(temporary, "missing"));
  if (missing.status === 0) {
    throw new Error("missing implementation unexpectedly passed");
  }
  console.log("gmv_forward_week_20260824_benchmark_self_test=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
