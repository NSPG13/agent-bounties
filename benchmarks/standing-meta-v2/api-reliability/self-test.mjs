import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-api-reliability-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-api-reliability.mjs"), implementation);
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
if (m.schema !== "https://agentbounties.org/schemas/api-reliability-manifest.v2.json") errors.push("schema_mismatch");
const net = m.network || {};
if (net.protocol !== "https") errors.push("protocol_mismatch");
if (!(net.endpoint_base_url || "").match(/^https:\/\/api\.agentbounties\.app\/v[0-9]+\/.*/)) errors.push("endpoint_pattern_mismatch");
const validStatuses = [200,201,204];
for (const s of (net.expected_statuses || [])) { if (!validStatuses.includes(s)) errors.push("status_code_unknown"); }
const pol = m.policy || {};
if (pol.timeout !== 5) errors.push("timeout_mismatch");
if (pol.retry_strategy !== "exponential_backoff") errors.push("retry_strategy_mismatch");
if (pol.max_retries !== 3) errors.push("max_retries_mismatch");
if (pol.health_check_interval !== 30) errors.push("health_check_interval_mismatch");
const requiredMetrics = ["latency_p95","error_rate","uptime_percentage","throughput_rps"];
const metrics = Array.isArray(m.required_metrics) ? m.required_metrics : [];
for (const r of requiredMetrics) { if (!metrics.includes(r)) errors.push("required_metric_missing:" + r); }
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,protocol:"https",expected_statuses:[200,201,204],timeout:5,retry_strategy:"exponential_backoff",max_retries:3,health_check_interval:30,required_metrics:requiredMetrics}));
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
