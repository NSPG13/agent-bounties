import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-gmv-benchmark-"));

/**
 * Create a temporary workspace with the specified checker implementation.
 * @param {string} name
 * @param {string} implementation
 * @returns {string}
 */
function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-gmv-20260907.mjs"), implementation);
  return root;
}

/**
 * Execute the benchmark runner against a workspace root.
 * @param {string} root
 * @returns {import("node:child_process").SpawnSyncReturns<string>}
 */
function run(root) {
  return spawnSync(process.execPath, [runner, root], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
}

const originalCheckerPath = join(benchmarkRoot, "..", "..", "..", "scripts", "check-agent-bounties-gmv-20260907.mjs");
const knownGood = readFileSync(originalCheckerPath, "utf8");

const alwaysReady = `
console.log(JSON.stringify({ready:true,network:"base-mainnet",competition:"0x81f0dd1f7da5f53ab6317e27131f9af45392b84c",bounty_id:"0x6f4939b0e12c8dd5d84efbfec0c7933bcdfcac9fb012c1b63014c6a6be7cad04",gmv_base_units:900000,status:"eligible"}));
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
  console.log("gmv_fortnight_20260907_benchmark_self_test=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
