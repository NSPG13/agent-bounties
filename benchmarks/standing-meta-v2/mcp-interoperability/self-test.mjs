import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-mcp-interoperability-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-mcp-interoperability.mjs"), implementation);
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
if (m.schema !== "https://agentbounties.org/schemas/mcp-interoperability-manifest.v2.json") errors.push("schema_mismatch");
if (m.protocol_version !== "2025-03-26") errors.push("protocol_version_mismatch");
if (m.transport !== "streamable-http") errors.push("transport_mismatch");
const reqTools = ["get_bounty_feed","prepare_bounty_action","get_bounty_action_status"];
const tools = Array.isArray(m.required_tools) ? m.required_tools : [];
for (const t of reqTools) { if (!tools.includes(t)) errors.push("required_tool_missing:" + t); }
const caps = Array.isArray(m.capabilities) ? m.capabilities : [];
if (!caps.includes("list_changed")) errors.push("capability_missing:list_changed");
const compat = Array.isArray(m.compatibility) ? m.compatibility : [];
for (const c of ["python-sdk","typescript-sdk"]) { if (!compat.includes(c)) errors.push("compatibility_entry_missing:" + c); }
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,protocol_version:"2025-03-26",transport:"streamable-http",required_tools:reqTools,capabilities:["list_changed","notifications"],compatibility:["python-sdk","typescript-sdk"]}));
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
