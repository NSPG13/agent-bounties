import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-mcp-interop-"));

function source(name, implementation) {
  const root = join(temporary, name);
  const scripts = join(root, "scripts");
  mkdirSync(scripts, { recursive: true });
  writeFileSync(join(scripts, "check-agent-bounties-mcp-interop.mjs"), implementation);
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
if (m.schema !== "https://agentbounties.org/schemas/mcp-interop-manifest.v2.json") errors.push("schema_mismatch");
const requiredProtocols = ["stdio","sse","streamable_http"];
const protocols = Array.isArray(m.transport_protocols) ? m.transport_protocols : [];
for (const p of requiredProtocols) { if (!protocols.includes(p)) errors.push("transport_protocol_missing:" + p); }
const validAuth = ["oauth2","api_key","none"];
const auth = Array.isArray(m.auth_methods) ? m.auth_methods : [];
if (auth.length === 0 || !auth.every(a => validAuth.includes(a))) errors.push("auth_method_invalid");
const requiredCaps = ["tools_list","resources_read","prompts_get","sampling_createMessage","roots_list"];
const caps = Array.isArray(m.required_capabilities) ? m.required_capabilities : [];
for (const c of requiredCaps) { if (!caps.includes(c)) errors.push("capability_missing:" + c); }
if (m.protocol_version !== "2024-11-05") errors.push("protocol_version_mismatch");
if (m.max_tools_per_server !== 256) errors.push("max_tools_mismatch");
if (m.heartbeat_interval !== 30) errors.push("heartbeat_interval_mismatch");
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,transport_protocols:requiredProtocols,auth_methods:validAuth,required_capabilities:requiredCaps,protocol_version:"2024-11-05",max_tools_per_server:256,heartbeat_interval:30}));
`;

// Test known-good
const good = source("known-good", knownGood);
const goodResult = run(good);
console.log(`self-test known-good: ${goodResult.stdout?.trim() || "FAIL"}`);

// Test empty
const empty = source("empty", "");
const emptyResult = run(empty);
console.log(`self-test empty: exit=${emptyResult.status}`);

rmSync(temporary, { recursive: true, force: true });
const allPassed = goodResult.status === 0 && emptyResult.status !== 0;
console.log(`\nSelf-test ${allPassed ? "PASSED" : "FAILED"}`);
process.exit(allPassed ? 0 : 1);
