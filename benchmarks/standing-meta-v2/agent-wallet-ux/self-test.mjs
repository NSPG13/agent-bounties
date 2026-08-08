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

const knownGood = `import { readFileSync } from "node:fs";
const requiredElements = ["balance_display","send_form","receive_qr","tx_history","gas_estimator"];
const supportedTypes = ["browser_extension","mobile_deep_link","web_wallet"];
const fail = (status, error) => { console.log(JSON.stringify({ready:false,errors:[error]})); process.exit(status); };
if (process.argv.length !== 3) fail(2, "manifest_path_required");
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { fail(2, "manifest_unreadable"); }
let manifest;
try { manifest = JSON.parse(text); } catch { fail(2, "manifest_invalid_json"); }
if (manifest === null || Array.isArray(manifest) || typeof manifest !== "object") fail(2, "manifest_root_object_required");
const protocol = manifest.protocol || {};
const ui = manifest.ui || {};
const elements = Array.isArray(ui.required_elements) ? ui.required_elements : [];
const types = Array.isArray(ui.supported_types) ? ui.supported_types : [];
const errors = [];
if (manifest.schema !== "https://agentbounties.org/schemas/wallet-ux-manifest.v2.json") errors.push("schema_mismatch");
if (protocol.network !== "base-mainnet") errors.push("protocol_network_mismatch");
if (protocol.chain_id !== 8453) errors.push("protocol_chain_id_mismatch");
if (protocol.asset !== "USDC") errors.push("protocol_asset_mismatch");
if ((protocol.native_token || "").toLowerCase() !== "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913") errors.push("protocol_token_mismatch");
if (ui.version !== "2.0") errors.push("wallet_ux_version_mismatch");
for (const el of requiredElements) { if (!elements.includes(el)) errors.push("required_element_missing:" + el); }
if (ui.confirmation_timeout !== 30) errors.push("confirmation_timeout_mismatch");
for (const t of types) { if (!supportedTypes.includes(t)) errors.push("supported_type_unknown"); }
if (errors.length > 0) { console.log(JSON.stringify({ready:false,errors})); process.exit(1); }
console.log(JSON.stringify({ready:true,network:"base-mainnet",asset:"USDC",wallet_ux_version:"2.0",required_elements:requiredElements,confirmation_timeout:30,supported_types:supportedTypes}));
`;

// Test known-good
const goodRoot = source("known-good", knownGood);
const good = run(goodRoot);
const goodOk = good.status === 0;
console.log(`known-good: ${goodOk ? "PASS" : "FAIL status=" + good.status}`);

// Test known-bad (broken schema)
const knownBad = knownGood.replace('agentbounties.org/schemas/wallet-ux-manifest.v2.json', 'wrong-schema');
const badRoot = source("known-bad", knownBad);
const bad = run(badRoot);
let badOutput = { ready: false, errors: [] };
try { badOutput = JSON.parse((bad.stdout || "").trim()); } catch {}
const badOk = bad.status === 1 && badOutput.ready === false && Array.isArray(badOutput.errors);
console.log(`known-bad: ${badOk ? "PASS" : "FAIL status=" + bad.status + " output=" + JSON.stringify(badOutput)}`);

// Cleanup
rmSync(temporary, { recursive: true, force: true });

const final = goodOk && badOk;
process.exit(final ? 0 : 1);
