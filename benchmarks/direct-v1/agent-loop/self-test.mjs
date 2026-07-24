import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const runner = join(benchmarkRoot, "test.mjs");
const temporary = mkdtempSync(join(tmpdir(), "agent-bounties-direct-benchmark-"));
const sourceRoot = join(temporary, "source");
const scriptsRoot = join(sourceRoot, "scripts");
mkdirSync(scriptsRoot, { recursive: true });

const claimImplementation = `
import { readFileSync } from "node:fs";
const emit = (value, status = 0) => { console.log(JSON.stringify(value)); process.exit(status); };
if (process.argv.length !== 3) emit({ok:false,errors:["claim_response_path_required"]}, 2);
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { emit({ok:false,errors:["claim_response_unreadable"]}, 2); }
let value;
try { value = JSON.parse(text); } catch { emit({ok:false,errors:["claim_response_invalid_json"]}, 2); }
if (!value || Array.isArray(value) || typeof value !== "object") emit({ok:false,errors:["claim_response_object_required"]}, 2);
if (value.schema_version === "agent-bounties/claim-problem-v1") {
  if (value.state !== "failed" || typeof value.error !== "string" || !value.error || typeof value.failed_transition !== "string" || !value.failed_transition || typeof value.next_action !== "string" || !value.next_action) emit({ok:false,errors:["claim_problem_invalid"]}, 1);
  emit({ok:true,state:"failed",action:"follow_error_next_action",may_sign:false,may_start_work:false,error:value.error,failed_transition:value.failed_transition});
}
if (value.schema_version !== "agent-bounties/agent-native-claim-v1") emit({ok:false,errors:["claim_response_schema_unsupported"]}, 1);
const candidate = value.candidate;
if (!candidate || Array.isArray(candidate) || typeof candidate !== "object" || typeof candidate.status !== "string") emit({ok:false,errors:["claim_candidate_invalid"]}, 1);
const state = candidate.status;
if (state === "waitlisted") emit({ok:true,state,action:"poll_same_idempotency_key",may_sign:false,may_start_work:false});
if (state === "authorization_ready") {
  const request = value.wallet_request;
  const next = value.next_request;
  const params = request?.params;
  const solver = String(candidate.solver_wallet ?? "").toLowerCase();
  const valid = request?.method === "eth_signTypedData_v4" && Array.isArray(params) && params.length === 2 && String(params[0]).toLowerCase() === solver && /^0x[0-9a-f]{40}$/.test(solver) && typeof params[1] === "string" && params[1].length > 0 && next && !Array.isArray(next) && typeof next === "object" && typeof next.url === "string" && next.method === "POST" && next.body && typeof next.body.idempotency_key === "string";
  if (!valid) emit({ok:false,errors:["authorization_request_invalid"]}, 1);
  emit({ok:true,state,action:"sign_wallet_request_and_replay",may_sign:true,may_start_work:false});
}
if (state === "relaying") emit({ok:true,state,action:"replay_same_signed_request",may_sign:false,may_start_work:false});
if (state === "claimed") {
  const event = value.canonical_event_id;
  if (typeof event !== "string" || !event || candidate.canonical_event_id !== event) emit({ok:false,errors:["canonical_claim_evidence_invalid"]}, 1);
  emit({ok:true,state,action:"start_work",may_sign:false,may_start_work:true,canonical_event_id:event});
}
emit({ok:false,errors:["claim_state_unsupported:" + state]}, 1);
`;

const selectionImplementation = `
import { readFileSync } from "node:fs";
const emit = (value, status = 0) => { console.log(JSON.stringify(value)); process.exit(status); };
const address = /^0x[0-9a-f]{40}$/;
const hash = /^0x[0-9a-f]{64}$/;
const uint = /^(0|[1-9][0-9]*)$/;
if (process.argv.length !== 4) emit({ok:false,errors:["feed_path_and_solver_required"]}, 2);
const solver = process.argv[3].toLowerCase();
if (!address.test(solver)) emit({ok:false,errors:["solver_wallet_invalid"]}, 2);
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { emit({ok:false,errors:["feed_unreadable"]}, 2); }
let feed;
try { feed = JSON.parse(text); } catch { emit({ok:false,errors:["feed_invalid_json"]}, 2); }
if (!Array.isArray(feed)) emit({ok:false,errors:["feed_array_required"]}, 2);
const normalized = [];
for (let index = 0; index < feed.length; index += 1) {
  const item = feed[index];
  const values = ["solver_reward","verifier_reward","claim_bond","target_amount","funded_amount"];
  const valid = item && !Array.isArray(item) && typeof item === "object" && hash.test(String(item.bounty_id ?? "").toLowerCase()) && address.test(String(item.bounty_contract ?? "").toLowerCase()) && address.test(String(item.creator ?? "").toLowerCase()) && hash.test(String(item.terms_hash ?? "").toLowerCase()) && typeof item.status === "string" && values.every((key) => typeof item[key] === "string" && uint.test(item[key])) && typeof item.terms_valid === "boolean" && typeof item.verification_ready === "boolean" && Array.isArray(item.validation_errors);
  if (!valid) emit({ok:false,errors:["feed_item_invalid:" + index]}, 2);
  normalized.push({...item,bounty_id:item.bounty_id.toLowerCase(),bounty_contract:item.bounty_contract.toLowerCase(),creator:item.creator.toLowerCase(),terms_hash:item.terms_hash.toLowerCase()});
}
const eligible = normalized.filter((item) => item.status === "claimable" && item.terms_valid && item.verification_ready && item.validation_errors.length === 0 && BigInt(item.solver_reward) > 0n && BigInt(item.claim_bond) > 0n && BigInt(item.funded_amount) >= BigInt(item.target_amount) && item.creator !== solver);
eligible.sort((left, right) => {
  const reward = BigInt(right.solver_reward) - BigInt(left.solver_reward);
  if (reward !== 0n) return reward > 0n ? 1 : -1;
  const bond = BigInt(left.claim_bond) - BigInt(right.claim_bond);
  if (bond !== 0n) return bond > 0n ? 1 : -1;
  return left.bounty_id.localeCompare(right.bounty_id);
});
if (eligible.length === 0) emit({ok:false,errors:["no_safe_claimable_bounty"]}, 1);
const item = eligible[0];
emit({ok:true,bounty_contract:item.bounty_contract,bounty_id:item.bounty_id,solver_reward:item.solver_reward,claim_bond:item.claim_bond,terms_hash:item.terms_hash,next_action:"agent_native_claim",request_bond_sponsorship:true});
`;

const settlementImplementation = `
import { readFileSync } from "node:fs";
const emit = (value, status = 0) => { console.log(JSON.stringify(value)); process.exit(status); };
const address = /^0x[0-9a-f]{40}$/;
const hash = /^0x[0-9a-f]{64}$/;
const amount = (value) => {
  if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0) return BigInt(value);
  if (typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value)) return BigInt(value);
  return null;
};
if (process.argv.length !== 5) emit({ok:false,errors:["item_path_contract_and_solver_required"]}, 2);
const expectedContract = process.argv[3].toLowerCase();
const expectedSolver = process.argv[4].toLowerCase();
if (!address.test(expectedContract) || !address.test(expectedSolver)) emit({ok:false,errors:["expected_address_invalid"]}, 2);
let text;
try { text = readFileSync(process.argv[2], "utf8"); } catch { emit({ok:false,errors:["settlement_item_unreadable"]}, 2); }
let item;
try { item = JSON.parse(text); } catch { emit({ok:false,errors:["settlement_item_invalid_json"]}, 2); }
if (!item || Array.isArray(item) || typeof item !== "object") emit({ok:false,errors:["settlement_item_object_required"]}, 2);
const contract = String(item.bounty_contract ?? "").toLowerCase();
const bountyId = String(item.bounty_id ?? "").toLowerCase();
if (contract !== expectedContract || !address.test(contract) || !hash.test(bountyId) || !Array.isArray(item.events)) emit({ok:false,errors:["settlement_identity_invalid"]}, 1);
const matches = item.events.filter((event) => event && event.kind === "bounty_settled" && String(event.contract_address ?? "").toLowerCase() === contract);
if (matches.length !== 1) emit({ok:false,errors:["exact_settlement_event_required"]}, 1);
const event = matches[0];
const tx = String(event.tx_hash ?? "").toLowerCase();
if (!hash.test(tx) || event.bounty_id?.toLowerCase() !== bountyId || !Number.isInteger(event.log_index) || event.log_index < 0 || !event.data || Array.isArray(event.data)) emit({ok:false,errors:["settlement_event_identity_invalid"]}, 1);
const data = event.data;
if (String(data.solver ?? "").toLowerCase() !== expectedSolver || !address.test(expectedSolver)) emit({ok:false,errors:["settlement_solver_mismatch"]}, 1);
const round = amount(data.round);
const solverReward = amount(data.solver_reward);
const returnedBond = amount(data.claim_bond_returned);
const timeoutBonus = amount(data.timeout_bond_bonus);
const solverPayout = amount(data.solver_payout);
const verifierReward = amount(data.verifier_reward);
const itemReward = amount(item.solver_reward);
const itemBond = amount(item.claim_bond);
const itemVerifier = amount(item.verifier_reward);
if ([round,solverReward,returnedBond,timeoutBonus,solverPayout,verifierReward,itemReward,itemBond,itemVerifier].some((value) => value === null) || round <= 0n || solverReward !== itemReward || returnedBond !== itemBond || verifierReward !== itemVerifier || solverPayout !== solverReward + returnedBond + timeoutBonus) emit({ok:false,errors:["settlement_amount_mismatch"]}, 1);
for (const key of ["submission_hash","evidence_hash","policy_hash","verification_hash"]) if (!hash.test(String(data[key] ?? "").toLowerCase())) emit({ok:false,errors:["settlement_commitment_invalid"]}, 1);
emit({ok:true,paid:true,bounty_contract:contract,bounty_id:bountyId,solver:expectedSolver,solver_payout:solverPayout.toString(),verifier_reward:verifierReward.toString(),transaction_hash:tx,log_index:event.log_index});
`;

const files = {
  "claim-next-action": ["next-agent-claim-action.mjs", claimImplementation],
  "select-funded-bounty": ["select-funded-bounty.mjs", selectionImplementation],
  "verify-settlement-evidence": ["verify-settlement-evidence.mjs", settlementImplementation],
};

function run(task, root) {
  return spawnSync(process.execPath, [runner, task, root], {
    encoding: "utf8",
    timeout: 15_000,
    windowsHide: true,
  });
}

try {
  for (const [, [name, implementation]] of Object.entries(files)) {
    writeFileSync(join(scriptsRoot, name), implementation);
  }
  for (const task of Object.keys(files)) {
    const good = run(task, sourceRoot);
    if (good.status !== 0) {
      throw new Error(`${task} known-good failed: ${good.stdout}${good.stderr}`);
    }
  }

  for (const [task, [name, implementation]] of Object.entries(files)) {
    writeFileSync(join(scriptsRoot, name), 'console.log(JSON.stringify({ok:true}));\n');
    const bad = run(task, sourceRoot);
    if (bad.status === 0) throw new Error(`${task} always-success implementation passed`);
    writeFileSync(join(scriptsRoot, name), implementation);

    const missing = run(task, join(temporary, "missing"));
    if (missing.status === 0) throw new Error(`${task} missing implementation passed`);
  }
  console.log("direct_agent_loop_benchmark_self_test=passed tasks=3");
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
