import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const task = process.argv[2];
const sourceRoot = resolve(process.argv[3] ?? "/workspace");
const scripts = {
  "claim-readiness": "next-agent-claim-readiness.mjs",
};

if (!Object.hasOwn(scripts, task)) {
  console.error(`unknown task: ${task ?? "missing"}`);
  process.exit(1);
}

const implementation = join(sourceRoot, "scripts", scripts[task]);
if (!existsSync(implementation)) {
  console.error(`missing implementation: ${implementation}`);
  process.exit(1);
}

const temporary = mkdtempSync(join(tmpdir(), `agent-bounties-${task}-`));
const address = (digit) => `0x${digit.repeat(40)}`;
const hash = (digit) => `0x${digit.repeat(64)}`;

function fixture(name, value, raw = false) {
  const path = join(temporary, name);
  writeFileSync(path, raw ? value : `${JSON.stringify(value)}\n`);
  return path;
}

function invoke(args) {
  return spawnSync(process.execPath, [implementation, ...args], {
    encoding: "utf8",
    timeout: 5_000,
    windowsHide: true,
  });
}

function expectRun(name, args, status, output) {
  const result = invoke(args);
  if (result.error) throw new Error(`${name}: ${result.error.message}`);
  if (result.status !== status) {
    throw new Error(
      `${name}: expected exit ${status}, received ${result.status}; stdout=${JSON.stringify(result.stdout)} stderr=${JSON.stringify(result.stderr)}`,
    );
  }
  if (result.stderr !== "") {
    throw new Error(`${name}: stderr must be empty: ${JSON.stringify(result.stderr)}`);
  }
  const expected = `${JSON.stringify(output)}\n`;
  if (result.stdout !== expected) {
    throw new Error(
      `${name}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(result.stdout)}`,
    );
  }
}

function claimReadinessCases() {
  const solver = address("1");
  const contract = address("a");

  // Helper: base claim-readiness response
  function readiness(overrides = {}) {
    return {
      schema_version: "agent-bounties/claim-readiness-v1",
      bounty_contract: contract,
      bounty_id: hash("b"),
      solver_wallet: solver,
      status: "ready",
      solver_reward: "2000000",
      claim_bond: "200000",
      verifier_reward: "200000",
      gross_cash_margin: "1800000",
      refundable_bond: "200000",
      external_spend: "200000",
      next_action: "Sign the claim authorization, then relay the signed transaction to the autonomous bounty contract.",
      can_claim: true,
      can_start_work: false,
      ...overrides,
    };
  }

  // 1. Missing args
  expectRun("missing args", [], 2, { ok: false, errors: ["readiness_path_required"] });

  // 2. Unreadable file
  expectRun("unreadable readiness", [join(temporary, "absent.json")], 2, { ok: false, errors: ["readiness_unreadable"] });

  // 3. Invalid JSON
  expectRun("invalid JSON", [fixture("readiness-invalid.json", "{", true)], 2, { ok: false, errors: ["readiness_invalid_json"] });

  // 4. Root not object
  expectRun("root not object", [fixture("readiness-root.json", [])], 2, { ok: false, errors: ["readiness_object_required"] });

  // 5. Unsupported schema version
  expectRun("unsupported schema", [fixture("readiness-schema.json", { schema_version: "agent-bounties/unknown-v1" })], 1, {
    ok: false,
    errors: ["readiness_schema_unsupported"],
  });

  // Scenario A: Healthy direct bounty — ready to claim, good margin
  const healthy = readiness({
    status: "ready",
    solver_reward: "2000000",
    claim_bond: "200000",
    verifier_reward: "200000",
    gross_cash_margin: "1800000",
    refundable_bond: "200000",
    external_spend: "200000",
    next_action: "Sign the claim authorization, then relay the signed transaction to the autonomous bounty contract.",
    can_claim: true,
    can_start_work: false,
  });
  expectRun("healthy direct bounty", [fixture("readiness-healthy.json", healthy)], 0, {
    ok: true,
    status: "ready",
    can_claim: true,
    can_start_work: false,
    solver_reward: "2000000",
    gross_cash_margin: "1800000",
    refundable_bond: "200000",
    external_spend: "200000",
    next_action: "Sign the claim authorization, then relay the signed transaction to the autonomous bounty contract.",
    profit_outlook: "profitable",
  });

  // Scenario B: Recovery-reserved bounty — reserved for recovery
  const recovery = readiness({
    status: "recovery_reserved",
    solver_reward: "2000000",
    claim_bond: "200000",
    verifier_reward: "200000",
    gross_cash_margin: "0",
    refundable_bond: "200000",
    external_spend: "200000",
    next_action: "This bounty is reserved for recovery. Do not attempt to claim.",
    can_claim: false,
    can_start_work: false,
  });
  expectRun("recovery-reserved bounty", [fixture("readiness-recovery.json", recovery)], 0, {
    ok: true,
    status: "recovery_reserved",
    can_claim: false,
    can_start_work: false,
    solver_reward: "2000000",
    gross_cash_margin: "0",
    refundable_bond: "200000",
    external_spend: "200000",
    next_action: "This bounty is reserved for recovery. Do not attempt to claim.",
    profit_outlook: "blocked",
  });

  // Scenario C: Unprofitable bounty — negative or zero margin
  const unprofitable = readiness({
    status: "ready",
    solver_reward: "100000",
    claim_bond: "200000",
    verifier_reward: "50000",
    gross_cash_margin: "-150000",
    refundable_bond: "200000",
    external_spend: "250000",
    next_action: "The solver reward does not cover the bond and verifier cost. Consider skipping this bounty.",
    can_claim: false,
    can_start_work: false,
  });
  expectRun("unprofitable bounty", [fixture("readiness-unprofitable.json", unprofitable)], 0, {
    ok: true,
    status: "ready",
    can_claim: false,
    can_start_work: false,
    solver_reward: "100000",
    gross_cash_margin: "-150000",
    refundable_bond: "200000",
    external_spend: "250000",
    next_action: "The solver reward does not cover the bond and verifier cost. Consider skipping this bounty.",
    profit_outlook: "unprofitable",
  });

  // Scenario D: Non-creator failure — only creator can claim
  const nonCreator = readiness({
    status: "non_creator",
    solver_reward: "2000000",
    claim_bond: "200000",
    verifier_reward: "200000",
    gross_cash_margin: "0",
    refundable_bond: "0",
    external_spend: "0",
    next_action: "Only the creator can claim this bounty. Wait for the creator to act or find another bounty.",
    can_claim: false,
    can_start_work: false,
    creator: address("f"),
    solver_wallet: address("1"),
  });
  expectRun("non-creator failure", [fixture("readiness-noncreator.json", nonCreator)], 0, {
    ok: true,
    status: "non_creator",
    can_claim: false,
    can_start_work: false,
    solver_reward: "2000000",
    gross_cash_margin: "0",
    refundable_bond: "0",
    external_spend: "0",
    next_action: "Only the creator can claim this bounty. Wait for the creator to act or find another bounty.",
    profit_outlook: "blocked",
  });

  // 5. Reject payment description in next_action (plan, signature, tx hash, hosted row)
  const paymentImpostor = readiness({
    status: "ready",
    next_action: "This is a plan to pay 2000000 USDC to the solver. The transaction hash is 0x1234.",
    can_claim: true,
  });
  expectRun("rejects payment plan in next_action", [fixture("readiness-payment-plan.json", paymentImpostor)], 1, {
    ok: false,
    errors: ["next_action_describes_payment"],
  });

  const paymentTx = readiness({
    status: "ready",
    next_action: "Payment was sent via transaction hash 0xdeadbeef. Proof of payment is in the hosted row.",
    can_claim: true,
  });
  expectRun("rejects tx hash / hosted row as payment", [fixture("readiness-payment-tx.json", paymentTx)], 1, {
    ok: false,
    errors: ["next_action_describes_payment"],
  });

  // 6. Missing required fields
  const missingReward = readiness({});
  delete missingReward.solver_reward;
  expectRun("missing solver_reward", [fixture("readiness-missing-reward.json", missingReward)], 1, {
    ok: false,
    errors: ["solver_reward_required"],
  });

  const missingBond = readiness({});
  delete missingBond.claim_bond;
  expectRun("missing claim_bond", [fixture("readiness-missing-bond.json", missingBond)], 1, {
    ok: false,
    errors: ["claim_bond_required"],
  });

  // 7. Negative margin is clearly distinguished from guaranteed net profit
  const zeroMargin = readiness({
    solver_reward: "200000",
    claim_bond: "200000",
    verifier_reward: "0",
    gross_cash_margin: "0",
    external_spend: "200000",
    next_action: "The solver reward exactly covers the bond. No net profit after verifier cost.",
    can_claim: false,
    can_start_work: false,
  });
  expectRun("zero margin scenario", [fixture("readiness-zero-margin.json", zeroMargin)], 0, {
    ok: true,
    status: "ready",
    can_claim: false,
    can_start_work: false,
    solver_reward: "200000",
    gross_cash_margin: "0",
    refundable_bond: "200000",
    external_spend: "200000",
    next_action: "The solver reward exactly covers the bond. No net profit after verifier cost.",
    profit_outlook: "break_even",
  });
}

try {
  if (task === "claim-readiness") claimReadinessCases();
  console.log(`direct_claim_readiness_benchmark=passed task=${task}`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}