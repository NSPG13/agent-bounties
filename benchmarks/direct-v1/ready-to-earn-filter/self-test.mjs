import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-ready-to-earn-filter-"));
const sourceRoot = join(temporary, "source");
const scriptsRoot = join(sourceRoot, "scripts");
mkdirSync(scriptsRoot, { recursive: true });

const filterImplementation = `
import { readFileSync, existsSync } from "node:fs";
const emit = (value, status = 0) => { console.log(JSON.stringify(value)); process.exit(status); };
const address = /^0x[0-9a-fA-F]{40}$/;
const hash = /^0x[0-9a-fA-F]{64}$/;
const requiredTermFields = ["protocol_version","creator_wallet","network","settlement_token","solver_reward","claim_bond","funding_deadline","claim_window_seconds","verification_window_seconds","creation_nonce"];
const terminalStatuses = new Set(["settled","expired","cancelled","refunded","completed","failed","voided"]);
const recoveryStatuses = new Set(["recovery_reserved","recovery-pending","reserved"]);
function hasInvalidTerms(b) { const t = b.contract_terms; if (!t||typeof t!=="object") return true; for (const f of requiredTermFields) { if (t[f]===undefined||t[f]===null) return true; } const amt = ["solver_reward","verifier_reward","claim_bond","initial_funding"]; for (const a of amt) { const v = t[a]; if (v&&typeof v==="object"&&(typeof v.amount!=="number"||v.amount<0)) return true; } return false; }
function isTerminal(b) { return terminalStatuses.has(String(b.status??"").toLowerCase()); }
function isRecoveryReserved(b) { return recoveryStatuses.has(String(b.status??"").toLowerCase()); }
function isVerificationNotReady(b) { return b.verification_ready===false; }
function filterReadyToEarn(bounties) {
  if (!Array.isArray(bounties)) return {ok:false,errors:["feed_must_be_array"]};
  const ready = []; const excluded = [];
  for (const bounty of bounties) {
    const reasons = [];
    if (hasInvalidTerms(bounty)) reasons.push("invalid_terms");
    if (isTerminal(bounty)) reasons.push("terminal_status");
    if (isRecoveryReserved(bounty)) reasons.push("recovery_reserved");
    if (isVerificationNotReady(bounty)) reasons.push("verification_not_ready");
    if (reasons.length>0) {
      excluded.push({bounty_id:bounty.bounty_id??bounty.id??null,bounty_contract:bounty.bounty_contract??bounty.contract??null,title:bounty.title??null,exclusion_reasons:reasons});
    } else {
      ready.push({bounty_id:bounty.bounty_id??bounty.id??null,bounty_contract:bounty.bounty_contract??bounty.contract??null,title:bounty.title??null,status:bounty.status??null,verification_ready:bounty.verification_ready??null});
    }
  }
  return {ok:true,ready_count:ready.length,excluded_count:excluded.length,ready,excluded};
}
if (process.argv.length!==3) emit({ok:false,errors:["feed_path_required"]},2);
const fp = process.argv[2];
if (!existsSync(fp)) emit({ok:false,errors:["feed_not_found"]},2);
let r; try { r = readFileSync(fp,"utf8"); } catch { emit({ok:false,errors:["feed_unreadable"]},2); }
let f; try { f = JSON.parse(r); } catch { emit({ok:false,errors:["feed_invalid_json"]},2); }
const res = filterReadyToEarn(f);
emit(res, res.ok ? 0 : 2);
`;

writeFileSync(join(scriptsRoot, "ready-to-earn-filter.mjs"), filterImplementation);

function run(task, root) {
  return spawnSync(process.execPath, [runner, task, root], {
    encoding: "utf8",
    timeout: 15_000,
    windowsHide: true,
  });
}

try {
  // 1. Known-good implementation must pass
  const good = run("ready-to-earn-filter", sourceRoot);
  if (good.status !== 0) {
    throw new Error(`ready-to-earn-filter known-good failed: ${good.stdout}${good.stderr}`);
  }

  // 2. Always-success implementation must fail (validates the runner catches real bugs)
  writeFileSync(join(scriptsRoot, "ready-to-earn-filter.mjs"), 'console.log(JSON.stringify({ok:true,ready_count:0,excluded_count:0,ready:[],excluded:[]}));\n');
  const bad = run("ready-to-earn-filter", sourceRoot);
  if (bad.status === 0) throw new Error("ready-to-earn-filter always-success implementation passed");
  writeFileSync(join(scriptsRoot, "ready-to-earn-filter.mjs"), filterImplementation);

  // 3. Missing implementation must fail
  const missing = run("ready-to-earn-filter", join(temporary, "missing"));
  if (missing.status === 0) throw new Error("ready-to-earn-filter missing implementation passed");

  console.log("ready_to_earn_filter_benchmark_self_test=passed");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}