import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-agent-discovery-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-agent-discovery.mjs"), implementation);
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
if (m.schema !== "https://agentbounties.org/schemas/agent-discovery-manifest.v2.json") errors.push("schema_mismatch");
if (m.feed_url !== "https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet") errors.push("feed_url_mismatch");
if (m.network !== "base-mainnet") errors.push("network_mismatch");
const reqFields = ["bounty_id","platform","title","status"];
const fields = Array.isArray(m.discovery_fields) ? m.discovery_fields : [];
for (const f of reqFields) { if (!fields.includes(f)) errors.push("discovery_field_missing:" + f); }
if (m.refresh_interval_seconds !== 300) errors.push("refresh_interval_mismatch");
const indexed = Array.isArray(m.indexed) ? m.indexed : [];
if (!indexed.includes("claimable")) errors.push("indexed_missing:claimable");
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,feed_url:"https://api.agentbounties.app/v1/base/autonomous-bounties/feed?network=base-mainnet",network:"base-mainnet",refresh_interval_seconds:300,discovery_fields:reqFields,indexed:["claimable","claimed","settled"]}));
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
