import { readFileSync } from "node:fs";

const emit = (value, status = 0) => {
  console.log(JSON.stringify(value));
  process.exit(status);
};

const address = /^0x[0-9a-f]{40}$/;
const hash = /^0x[0-9a-f]{64}$/;
const uint = /^(0|[1-9][0-9]*)$/;
const paymentKeywords = /\b(plan(\s+to\s+)?pay|transaction\s+hash|proof\s+of\s+payment|hosted\s+row)\b/i;

// Validate and parse claim-readiness response
if (process.argv.length !== 3) emit({ ok: false, errors: ["readiness_path_required"] }, 2);

let text;
try {
  text = readFileSync(process.argv[2], "utf8");
} catch {
  emit({ ok: false, errors: ["readiness_unreadable"] }, 2);
}

let value;
try {
  value = JSON.parse(text);
} catch {
  emit({ ok: false, errors: ["readiness_invalid_json"] }, 2);
}

if (!value || Array.isArray(value) || typeof value !== "object") {
  emit({ ok: false, errors: ["readiness_object_required"] }, 2);
}

if (value.schema_version !== "agent-bounties/claim-readiness-v1") {
  emit({ ok: false, errors: ["readiness_schema_unsupported"] }, 1);
}

// Validate required fields
const required = ["bounty_contract", "bounty_id", "solver_wallet", "status", "solver_reward", "claim_bond", "next_action"];
for (const field of required) {
  if (typeof value[field] !== "string" || !value[field]) {
    const key = field.replace(/([A-Z])/g, "_$1").toLowerCase();
    emit({ ok: false, errors: [`${key}_required`] }, 1);
  }
}

// Validate address and hash formats
if (!address.test(String(value.bounty_contract ?? "").toLowerCase())) {
  emit({ ok: false, errors: ["bounty_contract_invalid"] }, 1);
}
if (!hash.test(String(value.bounty_id ?? "").toLowerCase())) {
  emit({ ok: false, errors: ["bounty_id_invalid"] }, 1);
}
if (!address.test(String(value.solver_wallet ?? "").toLowerCase())) {
  emit({ ok: false, errors: ["solver_wallet_invalid"] }, 1);
}

// Validate numeric fields — allow negative for cash margin and external spend
const intOrUint = /^-?(0|[1-9][0-9]*)$/;
const numericFields = ["solver_reward", "claim_bond", "verifier_reward", "refundable_bond"];
const signedFields = ["gross_cash_margin", "external_spend"];
for (const field of numericFields) {
  if (value[field] !== undefined && value[field] !== null && !uint.test(String(value[field]))) {
    emit({ ok: false, errors: [`${field}_invalid`] }, 1);
  }
}
for (const field of signedFields) {
  if (value[field] !== undefined && value[field] !== null && !intOrUint.test(String(value[field]))) {
    emit({ ok: false, errors: [`${field}_invalid`] }, 1);
  }
}

// Check if next_action describes payment (transaction hash, plan to pay, proof of payment, hosted row)
if (paymentKeywords.test(value.next_action)) {
  emit({ ok: false, errors: ["next_action_describes_payment"] }, 1);
}

// Determine profit outlook
// Blocked statuses override any numerical calculation
let profit_outlook;
const blockedStatuses = ["recovery_reserved", "non_creator", "blocked"];
if (blockedStatuses.includes(value.status)) {
  profit_outlook = "blocked";
} else {
  const reward = BigInt(value.solver_reward || "0");
  const verifierCost = BigInt(value.verifier_reward || "0");
  const externalSpend = BigInt(value.external_spend || "0");
  // Bond is refundable, so it's not a cost — only verifier fee and external spend reduce net profit
  const totalCost = verifierCost + externalSpend;
  if (reward > totalCost) {
    profit_outlook = "profitable";
  } else if (reward === totalCost) {
    profit_outlook = "break_even";
  } else {
    profit_outlook = "unprofitable";
  }
}

// Build canonical response
const result = {
  ok: true,
  status: value.status,
  can_claim: value.can_claim === true,
  can_start_work: value.can_start_work === true,
  solver_reward: value.solver_reward,
  gross_cash_margin: value.gross_cash_margin || "0",
  refundable_bond: value.refundable_bond || "0",
  external_spend: value.external_spend || "0",
  next_action: value.next_action,
  profit_outlook,
};

emit(result, 0);