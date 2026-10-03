import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-wallet-ux-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-wallet-ux.mjs"), implementation);
  return root;
}

function run(root) {
  return spawnSync(process.execPath, [runner, root], {
    encoding: "utf8",
    timeout: 10_000,
    windowsHide: true,
  });
}

const knownGood = String.raw`import { readFileSync } from "node:fs";
const fail = (status, error) => { console.log(JSON.stringify({ready:false,errors:[error]})); process.exit(status); };
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let m;
try { m = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (m === null || Array.isArray(m) || typeof m !== "object") fail(2, "manifest_root_object_required");
const errors = [];
if (m.schema !== "https://agentbounties.org/schemas/wallet-ux-manifest.v2.json") errors.push("schema_mismatch");
const connectors = Array.isArray(m.connectors) ? m.connectors : [];
for (const c of ["injected-eip1193","walletconnect"]) { if (!connectors.includes(c)) errors.push("connector_missing:" + c); }
if (m.binding !== "agent-bounties/bounded-wallet-relay-v1") errors.push("binding_mismatch");
if (m.signing_flow !== "EIP-1193") errors.push("signing_flow_mismatch");
if (m.bond_preview !== true) errors.push("bond_preview_missing");
if (m.error_guidance !== true) errors.push("error_guidance_missing");
if (m.network !== "base-mainnet") errors.push("network_mismatch");
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,connectors:["injected-eip1193","walletconnect"],binding:"agent-bounties/bounded-wallet-relay-v1",signing_flow:"EIP-1193",bond_preview:true,error_guidance:true,network:"base-mainnet"}));
`;

// Test known-good implementation
const good = source("known-good", knownGood);
const goodResult = run(good);
console.log(`self-test known-good: ${goodResult.stdout?.trim() || "FAIL"}`);

// Test missing file (no checker script)
const empty = source("empty", "");
const emptyResult = run(empty);
console.log(`self-test empty: exit=${emptyResult.status}`);

// Cleanup
rmSync(temporary, { recursive: true, force: true });

const allPassed = goodResult.status === 0 && emptyResult.status !== 0;
console.log(`\nSelf-test ${allPassed ? "PASSED" : "FAILED"}`);
process.exit(allPassed ? 0 : 1);
