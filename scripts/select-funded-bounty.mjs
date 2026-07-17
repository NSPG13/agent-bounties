#!/usr/bin/env node
import { readFileSync } from "node:fs";

const VALID_ADDRESS_RE = /^0x[0-9a-f]{40}$/i;
const VALID_BOUNTY_ID_RE = /^0x[0-9a-f]{64}$/i;

function err(code, errors) {
  process.stdout.write(JSON.stringify({ ok: false, errors }) + "\n");
  process.exit(code);
}

function ok(entry) {
  const out = {
    ok: true,
    bounty_contract: entry.bounty_contract.toLowerCase(),
    bounty_id: entry.bounty_id.toLowerCase(),
    solver_reward: entry.solver_reward,
    claim_bond: entry.claim_bond,
    terms_hash: entry.terms_hash,
    next_action: "agent_native_claim",
    request_bond_sponsorship: true,
  };
  process.stdout.write(JSON.stringify(out) + "\n");
  process.exit(0);
}

function isValidInt(v) {
  if (typeof v !== "string") return false;
  if (!/^-?\d+$/.test(v)) return false;
  if (/^-?0\d/.test(v) && v.length > 2) return false;
  return true;
}

function isMalformed(item) {
  if (typeof item !== "object" || item === null || Array.isArray(item)) return true;
  if (typeof item.status !== "string") return true;
  if (!isValidInt(item.solver_reward)) return true;
  if (!isValidInt(item.claim_bond)) return true;
  if (!isValidInt(item.target_amount)) return true;
  if (!isValidInt(item.funded_amount)) return true;
  if (!VALID_BOUNTY_ID_RE.test(item.bounty_id)) return true;
  if (!VALID_ADDRESS_RE.test(item.bounty_contract)) return true;
  if (!VALID_ADDRESS_RE.test(item.creator)) return true;
  return false;
}

function isEligible(item, solver) {
  if (item.status !== "claimable") return false;
  if (BigInt(item.funded_amount) < BigInt(item.target_amount)) return false;
  if (item.terms_valid !== true) return false;
  if (item.verification_ready !== true) return false;
  if (item.validation_errors && item.validation_errors.length > 0) return false;
  if (BigInt(item.solver_reward) <= 0n) return false;
  if (BigInt(item.claim_bond) <= 0n) return false;
  if (item.creator.toLowerCase() === solver.toLowerCase()) return false;
  return true;
}

const args = process.argv.slice(2);
if (args.length !== 2) {
  err(2, ["feed_path_and_solver_required"]);
}

const [feedPath, solver] = args;

if (!VALID_ADDRESS_RE.test(solver)) {
  err(2, ["solver_wallet_invalid"]);
}

let raw;
try {
  raw = readFileSync(feedPath, "utf8");
} catch {
  err(2, ["feed_invalid_json"]);
}

let feed;
try {
  feed = JSON.parse(raw);
} catch {
  err(2, ["feed_invalid_json"]);
}

if (!Array.isArray(feed)) {
  err(2, ["feed_array_required"]);
}

const malformed = [];
for (let i = 0; i < feed.length; i++) {
  if (isMalformed(feed[i])) {
    malformed.push(`feed_item_invalid:${i}`);
  }
}
if (malformed.length > 0) {
  err(2, malformed);
}

const eligible = feed.filter((item) => isEligible(item, solver));

if (eligible.length === 0) {
  err(1, ["no_safe_claimable_bounty"]);
}

eligible.sort((a, b) => {
  const rA = BigInt(a.solver_reward);
  const rB = BigInt(b.solver_reward);
  if (rA > rB) return -1;
  if (rA < rB) return 1;
  const bA = BigInt(a.claim_bond);
  const bB = BigInt(b.claim_bond);
  if (bA < bB) return -1;
  if (bA > bB) return 1;
  return a.bounty_id < b.bounty_id ? -1 : a.bounty_id > b.bounty_id ? 1 : 0;
});

ok(eligible[0]);
