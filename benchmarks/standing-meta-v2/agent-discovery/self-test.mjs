import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-discovery-benchmark-"));

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

const knownGood = `
import { readFileSync } from "node:fs";
const caps = ["claim_detection","funding_verification","settlement_tracking","child_bounty_creation"];
const fail = (s, e) => { console.log(JSON.stringify({ready:false,errors:[e]})); process.exit(s); };
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let m;
try { m = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (m === null || Array.isArray(m) || typeof m !== "object") fail(2, "manifest_not_object");
if (m.schema !== "https://agentbounties.org/schemas/agent-discovery-manifest.v2.json") fail(1, "schema_mismatch");
if (m.network !== "base-mainnet") fail(1, "protocol_network_mismatch");
if (m.chainId !== 8453) fail(1, "protocol_chain_id_mismatch");
if (m.agent_registration !== "https://api.agentbounties.app/v1/base/agents/register") fail(1, "registration_endpoint_mismatch");
if (m.agent_discovery !== "https://api.agentbounties.app/v1/base/agents/discover") fail(1, "discovery_endpoint_mismatch");
if (m.agent_card_path !== ".well-known/agent-card.json") fail(1, "agent_card_path_mismatch");
if (!Array.isArray(m.required_capabilities)) fail(1, "required_capabilities_not_array");
for (const c of caps) { if (!m.required_capabilities.includes(c)) fail(1, \`required_capability_missing:\${c}\`); }
console.log(JSON.stringify({ready:true,network:"base-mainnet",agent_registration:m.agent_registration,agent_discovery:m.agent_discovery,agent_card_path:m.agent_card_path,required_capabilities:m.required_capabilities}));
process.exit(0);
`;

const wrongProto = `
import { readFileSync } from "node:fs";
const caps = ["claim_detection","funding_verification","settlement_tracking","child_bounty_creation"];
const fail = (s, e) => { console.log(JSON.stringify({ready:false,errors:[e]})); process.exit(s); };
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let m;
try { m = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (m === null || Array.isArray(m) || typeof m !== "object") fail(2, "manifest_not_object");
if (m.schema !== "https://agentbounties.org/schemas/agent-discovery-manifest.v2.json") fail(1, "schema_mismatch");
if (m.network === "wrong-network") fail(1, "protocol_network_mismatch");
for (const c of caps) { if (!m.required_capabilities?.includes(c)) fail(1, \`required_capability_missing:\${c}\`); }
fail(1, "protocol_chain_id_mismatch");
`;

const rootGood = source("good", knownGood);
const resultGood = run(rootGood);
console.log(`Known good: status=${resultGood.status} ${resultGood.status === 0 ? 'PASS' : 'FAIL'}`);

const rootWrong = source("wrong-proto", wrongProto);
const resultWrong = run(rootWrong);
console.log(`Wrong proto: status=${resultWrong.status} ${resultWrong.status !== 0 ? 'PASS (should fail)' : 'FAIL (should not pass)'}`);

rmSync(temporary, { recursive: true, force: true });
process.exit(resultGood.status === 0 ? 0 : 1);
