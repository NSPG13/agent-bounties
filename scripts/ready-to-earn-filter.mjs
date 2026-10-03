#!/usr/bin/env node
/**
 * Ready-to-Earn Inventory Filter
 * 
 * Filters a canonical bounty feed to show only bounties that are ready
 * for an agent to discover and earn from. Excludes bounties that are
 * blocked by verification status, recovery, invalid terms, or terminal
 * lifecycle states.
 * 
 * Bounty: NSPG13/agent-bounties #683 — 2 USDC
 */
import { readFileSync, existsSync } from "node:fs";

const emit = (value, status = 0) => {
  console.log(JSON.stringify(value));
  process.exit(status);
};

// ── Helpers ──────────────────────────────────────────────────────

const address = /^0x[0-9a-fA-F]{40}$/;
const hash = /^0x[0-9a-fA-F]{64}$/;
const requiredTermFields = [
  "protocol_version",
  "creator_wallet",
  "network",
  "settlement_token",
  "solver_reward",
  "claim_bond",
  "funding_deadline",
  "claim_window_seconds",
  "verification_window_seconds",
  "creation_nonce",
];

// ── Exclusion predicates ─────────────────────────────────────────

/**
 * A bounty is "term-invalid" if its contract_terms object is missing any
 * required field or has an unparseable reward/bond amount.
 */
function hasInvalidTerms(bounty) {
  const terms = bounty.contract_terms;
  if (!terms || typeof terms !== "object") return true;
  for (const field of requiredTermFields) {
    if (terms[field] === undefined || terms[field] === null) return true;
  }
  // Amounts must be non-negative integers
  const amountFields = ["solver_reward", "verifier_reward", "claim_bond", "initial_funding"];
  for (const af of amountFields) {
    const val = terms[af];
    if (val && typeof val === "object") {
      if (typeof val.amount !== "number" || val.amount < 0) return true;
    }
  }
  return false;
}

/**
 * Terminal statuses — the bounty lifecycle has ended and no further
 * agent action can change the outcome.
 */
const terminalStatuses = new Set([
  "settled",
  "expired",
  "cancelled",
  "refunded",
  "completed",
  "failed",
  "voided",
]);

function isTerminal(bounty) {
  return terminalStatuses.has(String(bounty.status ?? "").toLowerCase());
}

/**
 * Recovery-reserved — the bounty is locked for recovery or reserved
 * for a specific purpose.
 */
const recoveryStatuses = new Set([
  "recovery_reserved",
  "recovery-pending",
  "reserved",
]);

function isRecoveryReserved(bounty) {
  return recoveryStatuses.has(String(bounty.status ?? "").toLowerCase());
}

/**
 * Not verification-ready — the bounty is funded but not yet claimable
 * because verification conditions haven't been met.
 */
function isVerificationNotReady(bounty) {
  return bounty.verification_ready === false;
}

// ── Main filter ──────────────────────────────────────────────────

function filterReadyToEarn(bounties) {
  if (!Array.isArray(bounties)) {
    return { ok: false, errors: ["feed_must_be_array"] };
  }

  const ready = [];
  const excluded = [];

  for (const bounty of bounties) {
    const reasons = [];

    if (hasInvalidTerms(bounty)) reasons.push("invalid_terms");
    if (isTerminal(bounty)) reasons.push("terminal_status");
    if (isRecoveryReserved(bounty)) reasons.push("recovery_reserved");
    if (isVerificationNotReady(bounty)) reasons.push("verification_not_ready");

    if (reasons.length > 0) {
      excluded.push({
        bounty_id: bounty.bounty_id ?? bounty.id ?? null,
        bounty_contract: bounty.bounty_contract ?? bounty.contract ?? null,
        title: bounty.title ?? null,
        exclusion_reasons: reasons,
      });
    } else {
      ready.push(bounty);
    }
  }

  return {
    ok: true,
    ready_count: ready.length,
    excluded_count: excluded.length,
    ready: ready.map((b) => ({
      bounty_id: b.bounty_id ?? b.id ?? null,
      bounty_contract: b.bounty_contract ?? b.contract ?? null,
      title: b.title ?? null,
      status: b.status ?? null,
      verification_ready: b.verification_ready ?? null,
    })),
    excluded,
  };
}

// ── CLI entry point ──────────────────────────────────────────────

if (process.argv.length !== 3) {
  emit({ ok: false, errors: ["feed_path_required"] }, 2);
}

const feedPath = process.argv[2];
if (!existsSync(feedPath)) {
  emit({ ok: false, errors: ["feed_not_found"] }, 2);
}

let raw;
try {
  raw = readFileSync(feedPath, "utf8");
} catch {
  emit({ ok: false, errors: ["feed_unreadable"] }, 2);
}

let feed;
try {
  feed = JSON.parse(raw);
} catch {
  emit({ ok: false, errors: ["feed_invalid_json"] }, 2);
}

const result = filterReadyToEarn(feed);
emit(result, result.ok ? 0 : 2);