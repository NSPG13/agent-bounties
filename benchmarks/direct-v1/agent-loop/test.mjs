import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const benchmarkRoot = dirname(fileURLToPath(import.meta.url));
const task = process.argv[2];
const sourceRoot = resolve(process.argv[3] ?? "/workspace");
const scripts = {
  "claim-next-action": "next-agent-claim-action.mjs",
  "select-funded-bounty": "select-funded-bounty.mjs",
  "verify-settlement-evidence": "verify-settlement-evidence.mjs",
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

function claimCases() {
  const solver = address("1");
  const eventId = "018f47a0-1a2b-7c3d-8e4f-123456789abc";
  const base = {
    schema_version: "agent-bounties/agent-native-claim-v1",
    candidate: { status: "waitlisted", solver_wallet: solver, canonical_event_id: null },
    canonical_event_id: null,
    wallet_request: null,
    next_request: null,
  };
  expectRun("missing claim path", [], 2, { ok: false, errors: ["claim_response_path_required"] });
  expectRun("unreadable claim", [join(temporary, "absent.json")], 2, { ok: false, errors: ["claim_response_unreadable"] });
  expectRun("invalid claim JSON", [fixture("claim-invalid.json", "{", true)], 2, { ok: false, errors: ["claim_response_invalid_json"] });
  expectRun("claim root", [fixture("claim-root.json", [])], 2, { ok: false, errors: ["claim_response_object_required"] });
  expectRun("waitlisted", [fixture("claim-waitlisted.json", base)], 0, {
    ok: true,
    state: "waitlisted",
    action: "poll_same_idempotency_key",
    may_sign: false,
    may_start_work: false,
  });

  const authorization = structuredClone(base);
  authorization.candidate.status = "authorization_ready";
  authorization.wallet_request = {
    method: "eth_signTypedData_v4",
    params: [solver.toUpperCase(), JSON.stringify({ domain: { chainId: 8453 } })],
  };
  authorization.next_request = {
    method: "POST",
    url: "https://api.agentbounties.app/v1/base/autonomous-bounties/claims",
    body: { idempotency_key: "benchmark-claim-1" },
  };
  expectRun("authorization ready", [fixture("claim-authorization.json", authorization)], 0, {
    ok: true,
    state: "authorization_ready",
    action: "sign_wallet_request_and_replay",
    may_sign: true,
    may_start_work: false,
  });
  authorization.wallet_request.params[0] = address("2");
  expectRun("unsafe authorization", [fixture("claim-unsafe.json", authorization)], 1, {
    ok: false,
    errors: ["authorization_request_invalid"],
  });

  const relaying = structuredClone(base);
  relaying.candidate.status = "relaying";
  expectRun("relaying", [fixture("claim-relaying.json", relaying)], 0, {
    ok: true,
    state: "relaying",
    action: "replay_same_signed_request",
    may_sign: false,
    may_start_work: false,
  });

  const claimed = structuredClone(base);
  claimed.candidate.status = "claimed";
  claimed.candidate.canonical_event_id = eventId;
  claimed.canonical_event_id = eventId;
  expectRun("claimed", [fixture("claim-claimed.json", claimed)], 0, {
    ok: true,
    state: "claimed",
    action: "start_work",
    may_sign: false,
    may_start_work: true,
    canonical_event_id: eventId,
  });
  claimed.candidate.canonical_event_id = "different-event";
  expectRun("unconfirmed claim", [fixture("claim-unconfirmed.json", claimed)], 1, {
    ok: false,
    errors: ["canonical_claim_evidence_invalid"],
  });

  expectRun("claim problem", [fixture("claim-problem.json", {
    schema_version: "agent-bounties/claim-problem-v1",
    state: "failed",
    failed_transition: "confirm_claim",
    error: "claim_event_mismatch",
    next_action: "Do not start work; report the transaction hash.",
  })], 0, {
    ok: true,
    state: "failed",
    action: "follow_error_next_action",
    may_sign: false,
    may_start_work: false,
    error: "claim_event_mismatch",
    failed_transition: "confirm_claim",
  });
  expectRun("unknown claim state", [fixture("claim-unknown.json", {
    ...base,
    candidate: { ...base.candidate, status: "exclusive" },
  })], 1, { ok: false, errors: ["claim_state_unsupported:exclusive"] });
}

function bounty(overrides = {}) {
  return {
    bounty_id: hash("a"),
    bounty_contract: address("a"),
    creator: address("f"),
    status: "claimable",
    solver_reward: "2000000",
    verifier_reward: "200000",
    claim_bond: "200000",
    target_amount: "2200000",
    funded_amount: "2200000",
    terms_hash: hash("b"),
    terms_valid: true,
    verification_ready: true,
    validation_errors: [],
    ...overrides,
  };
}

function selectionCases() {
  const solver = address("1");
  expectRun("missing feed args", [], 2, { ok: false, errors: ["feed_path_and_solver_required"] });
  expectRun("invalid solver", [fixture("feed-empty.json", []), "invalid"], 2, { ok: false, errors: ["solver_wallet_invalid"] });
  expectRun("invalid feed JSON", [fixture("feed-invalid.json", "{", true), solver], 2, { ok: false, errors: ["feed_invalid_json"] });
  expectRun("feed root", [fixture("feed-root.json", {}), solver], 2, { ok: false, errors: ["feed_array_required"] });
  expectRun("malformed feed item", [fixture("feed-malformed.json", [bounty({ solver_reward: "2.0" })]), solver], 2, { ok: false, errors: ["feed_item_invalid:0"] });
  expectRun("no safe bounty", [fixture("feed-none.json", [bounty({ funded_amount: "1" })]), solver], 1, { ok: false, errors: ["no_safe_claimable_bounty"] });

  const selected = bounty({
    bounty_id: hash("3"),
    bounty_contract: address("3"),
    solver_reward: "3000000",
    claim_bond: "100000",
    target_amount: "3100000",
    funded_amount: "3100000",
    terms_hash: hash("4"),
  });
  const feed = [
    bounty({ bounty_id: hash("1"), bounty_contract: address("1"), solver_reward: "2500000" }),
    bounty({ bounty_id: hash("2"), bounty_contract: address("2"), solver_reward: "3000000", claim_bond: "200000", target_amount: "3200000", funded_amount: "3200000" }),
    selected,
    bounty({ bounty_id: hash("5"), bounty_contract: address("5"), creator: solver, solver_reward: "5000000", target_amount: "5200000", funded_amount: "5200000" }),
    bounty({ bounty_id: hash("6"), bounty_contract: address("6"), solver_reward: "6000000", target_amount: "6200000", funded_amount: "6200000", verification_ready: false }),
  ];
  expectRun("rank safe bounty", [fixture("feed-ranked.json", feed), solver.toUpperCase()], 0, {
    ok: true,
    bounty_contract: address("3"),
    bounty_id: hash("3"),
    solver_reward: "3000000",
    claim_bond: "100000",
    terms_hash: hash("4"),
    next_action: "agent_native_claim",
    request_bond_sponsorship: true,
  });

  const tie = [
    bounty({ bounty_id: hash("c"), bounty_contract: address("c") }),
    bounty({ bounty_id: hash("b"), bounty_contract: address("b") }),
  ];
  expectRun("stable tie", [fixture("feed-tie.json", tie), solver], 0, {
    ok: true,
    bounty_contract: address("b"),
    bounty_id: hash("b"),
    solver_reward: "2000000",
    claim_bond: "200000",
    terms_hash: hash("b"),
    next_action: "agent_native_claim",
    request_bond_sponsorship: true,
  });
}

function settledItem(overrides = {}) {
  const contract = address("7");
  const solver = address("8");
  const item = {
    bounty_id: hash("7"),
    bounty_contract: contract,
    status: "settled",
    solver_reward: "2000000",
    verifier_reward: "200000",
    claim_bond: "200000",
    events: [{
      tx_hash: hash("9"),
      log_index: 12,
      contract_address: contract,
      bounty_id: hash("7"),
      kind: "bounty_settled",
      data: {
        round: 1,
        solver,
        solver_reward: 2000000,
        claim_bond_returned: 200000,
        timeout_bond_bonus: 100000,
        solver_payout: 2300000,
        verifier_reward: 200000,
        submission_hash: hash("a"),
        evidence_hash: hash("b"),
        policy_hash: hash("c"),
        verification_hash: hash("d"),
      },
    }],
  };
  return { contract, solver, item: { ...item, ...overrides } };
}

function settlementCases() {
  const value = settledItem();
  expectRun("missing settlement args", [], 2, { ok: false, errors: ["item_path_contract_and_solver_required"] });
  expectRun("invalid expected address", [fixture("settlement-empty.json", {}), "bad", value.solver], 2, { ok: false, errors: ["expected_address_invalid"] });
  expectRun("settlement item root", [fixture("settlement-root.json", []), value.contract, value.solver], 2, { ok: false, errors: ["settlement_item_object_required"] });
  expectRun("valid settlement", [fixture("settlement-valid.json", value.item), value.contract.toUpperCase(), value.solver.toUpperCase()], 0, {
    ok: true,
    paid: true,
    bounty_contract: value.contract,
    bounty_id: hash("7"),
    solver: value.solver,
    solver_payout: "2300000",
    verifier_reward: "200000",
    transaction_hash: hash("9"),
    log_index: 12,
  });

  const noEvent = { ...value.item, events: [] };
  expectRun("missing settlement", [fixture("settlement-missing.json", noEvent), value.contract, value.solver], 1, { ok: false, errors: ["exact_settlement_event_required"] });
  const duplicate = structuredClone(value.item);
  duplicate.events.push(structuredClone(duplicate.events[0]));
  expectRun("duplicate settlement", [fixture("settlement-duplicate.json", duplicate), value.contract, value.solver], 1, { ok: false, errors: ["exact_settlement_event_required"] });
  const wrongSolver = structuredClone(value.item);
  wrongSolver.events[0].data.solver = address("6");
  expectRun("wrong solver", [fixture("settlement-solver.json", wrongSolver), value.contract, value.solver], 1, { ok: false, errors: ["settlement_solver_mismatch"] });
  const wrongPayout = structuredClone(value.item);
  wrongPayout.events[0].data.solver_payout += 1;
  expectRun("wrong payout", [fixture("settlement-payout.json", wrongPayout), value.contract, value.solver], 1, { ok: false, errors: ["settlement_amount_mismatch"] });
  const wrongHash = structuredClone(value.item);
  wrongHash.events[0].data.evidence_hash = "0x00";
  expectRun("wrong commitment", [fixture("settlement-hash.json", wrongHash), value.contract, value.solver], 1, { ok: false, errors: ["settlement_commitment_invalid"] });
}

try {
  if (task === "claim-next-action") claimCases();
  if (task === "select-funded-bounty") selectionCases();
  if (task === "verify-settlement-evidence") settlementCases();
  console.log(`direct_agent_loop_benchmark=passed task=${task}`);
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
