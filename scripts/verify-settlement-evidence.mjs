#!/usr/bin/env node
import { readFileSync } from "node:fs";

const HEX_ADDR_RE = /^0x[0-9a-f]{40}$/i;
const HEX_HASH_RE = /^0x[0-9a-f]{64}$/i;

function out(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function fail(code, errors) {
  out({ ok: false, errors });
  process.exit(code);
}

const [itemPath, expectedContract, expectedSolver] = process.argv.slice(2);

if (!itemPath || !expectedContract || !expectedSolver) {
  fail(2, ["item_path_contract_and_solver_required"]);
}

if (!HEX_ADDR_RE.test(expectedContract) || !HEX_ADDR_RE.test(expectedSolver)) {
  fail(2, ["expected_address_invalid"]);
}

let item;
try {
  item = JSON.parse(readFileSync(itemPath, "utf8"));
} catch {
  fail(2, ["settlement_item_object_required"]);
}

if (Array.isArray(item) || item === null || typeof item !== "object") {
  fail(2, ["settlement_item_object_required"]);
}

const events = Array.isArray(item.events) ? item.events : [];
const ecLower = expectedContract.toLowerCase();
const esLower = expectedSolver.toLowerCase();

const candidates = events.filter((e) => {
  if (e.kind !== "bounty_settled") return false;
  if (typeof e.contract_address !== "string" || e.contract_address.toLowerCase() !== ecLower) return false;
  if (e.bounty_id !== item.bounty_id) return false;
  if (typeof e.log_index !== "number" || e.log_index < 0) return false;
  if (!e.data || typeof e.data.round !== "number" || e.data.round <= 0) return false;
  return true;
});

if (candidates.length === 0) {
  fail(1, ["exact_settlement_event_required"]);
}

const solverMatches = candidates.filter(
  (e) => typeof e.data.solver === "string" && e.data.solver.toLowerCase() === esLower
);

if (solverMatches.length === 0) {
  fail(1, ["settlement_solver_mismatch"]);
}

if (solverMatches.length !== 1) {
  fail(1, ["exact_settlement_event_required"]);
}

const m = solverMatches[0];
const d = m.data;

const solverReward = d.solver_reward;
const claimBondReturned = d.claim_bond_returned;
const verifierReward = d.verifier_reward;
const timeoutBondBonus = d.timeout_bond_bonus;

if (
  typeof solverReward !== "number" || solverReward < 0 ||
  typeof claimBondReturned !== "number" || claimBondReturned < 0 ||
  typeof verifierReward !== "number" || verifierReward < 0 ||
  typeof timeoutBondBonus !== "number" || timeoutBondBonus < 0
) {
  fail(1, ["settlement_amount_mismatch"]);
}

const solverPayout = solverReward + claimBondReturned + timeoutBondBonus;

// Validate against the solver_payout in the event data
if (typeof d.solver_payout === "number" && solverPayout !== d.solver_payout) {
  fail(1, ["settlement_amount_mismatch"]);
}

// Also validate against item.amount if present
if (typeof item.amount === "number" && solverPayout !== item.amount) {
  fail(1, ["settlement_amount_mismatch"]);
}

if (
  !HEX_HASH_RE.test(d.submission_hash) ||
  !HEX_HASH_RE.test(d.evidence_hash) ||
  !HEX_HASH_RE.test(d.policy_hash) ||
  !HEX_HASH_RE.test(d.verification_hash)
) {
  fail(1, ["settlement_commitment_invalid"]);
}

out({
  ok: true,
  paid: true,
  bounty_contract: m.contract_address,
  bounty_id: item.bounty_id,
  solver: d.solver,
  solver_payout: String(solverPayout),
  verifier_reward: String(verifierReward),
  transaction_hash: m.tx_hash,
  log_index: m.log_index,
});
process.exit(0);
