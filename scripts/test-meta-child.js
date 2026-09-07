"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs"), vm = require("node:vm");
const helper = require("../site/meta-child.js");
const composer = require("../site/bounty-composer-v2.js");
// Captured from the existing Rust pure planner test with its routed-V3 parent.
// The browser checks the independent backend output below. These are
// fake test participants and a test runner, never a publishable work package.
const fixture = require("./fixtures/meta-child-plan.json");
const env = { window: {}, TextEncoder, Date: class extends Date { static now() { return Date.parse("2026-09-06T00:00:00Z"); } } };
vm.runInNewContext(fs.readFileSync(require.resolve("../site/evm.js"), "utf8"), env);
vm.runInNewContext(fs.readFileSync(require.resolve("../site/meta-child.js"), "utf8"), env);
const evm = env.window.AgentBountiesEvm, fixedHelper = env.window.AgentBountiesMetaChild;
const split = { solver: 990000n, verifier: 10000n, total: 1000000n };
function inputFor(plan = fixture) {
  const doc = plan.terms.document, terms = doc.contract_terms;
  return { network: "base-mainnet", parent_bounty_contract: plan.parent_bounty_contract, parent_solver: plan.parent_solver,
    intended_child_solver: plan.intended_child_solver, title: doc.title, goal: doc.goal, acceptance_criteria: doc.acceptance_criteria,
    benchmark_source: doc.benchmark.source, runner_manifest: doc.benchmark.runner_manifest, evidence_schema: doc.evidence_schema,
    creation_nonce: terms.creation_nonce, claim_window_seconds: terms.claim_window_seconds, verification_window_seconds: terms.verification_window_seconds,
    funding_deadline: terms.funding_deadline, source_url: doc.source_url, discovery_source: doc.discovery_source };
}
function parentFixture() {
  const context = { parent_bounty_contract: fixture.parent_bounty_contract, intended_child_solver: fixture.intended_child_solver };
  const item = { opportunity_id: "canonical:base-mainnet:" + context.parent_bounty_contract, network: "base-mainnet", work_state: "claimable", verification_ready: true, payment_committed: true, source_id: context.parent_bounty_contract, terms_hash: "0x" + "55".repeat(32), title: "Parent" };
  const record = { terms_hash: item.terms_hash, acceptance_criteria_hash: "0xba3b04ab970dfd91f5ccf1b7eda6670b5a38a854bca16dc980ec8362ed2bcaf9", document: {
    contract_terms: { solver_reward: { amount: 2000000 }, creator_wallet: "0x" + "33".repeat(20), funding_deadline: 1791676800 },
    benchmark: { engine: "standing_meta_v3_routed_parent", minimum_child_target: 1000000, minimum_parent_gross_margin: 1000000, required_child_engine: "sandboxed_regression_v1", required_child_verifier_threshold: 2, required_child_verifier_set_hash: helper.SET_HASH, terms_registry: helper.REGISTRY, participant_registry: helper.PARTICIPANTS },
    verification_policy: { module_id: "policy_bound_verifier_router_v1", verifier_module: "0x380c1af742593dd88b6f20387e9ee693a0536731" } } };
  const reads = [];
  const client = { async opportunity(id, fresh) { reads.push([id, fresh]); return item; }, async inspect() { return { terms: record }; } };
  return { context, item, record, client, reads };
}
test("a freshly resolved parent permits exact 1 USDC without lowering the ordinary floor", async () => {
  const f = parentFixture(), parent = await helper.resolve(f.context, f.client);
  assert.equal(f.reads[0][1], true);
  assert.deepEqual(composer.parsePreparedRewardSplit("0.99", "0.01", parent), split);
  assert.equal(composer.parsePreparedRewardSplit("0.98", "0.02", parent).total, 1000000n);
  assert.deepEqual(composer.rewardSplitForTotal(1, null, parent), split);
  assert.throws(() => composer.parsePreparedRewardSplit("0.99", "0.01"), /at least 2 USDC/);
  assert.throws(() => composer.rewardSplitForTotal(1), /at least 2 USDC/);
  for (const [solver, verifier] of [["0.98", "0.01"], ["1.99", "0.01"], ["0.999999", "0.000001"], ["0.989999", "0.010001"], ["0", "1"]]) {
    assert.throws(() => composer.parsePreparedRewardSplit(solver, verifier, parent));
  }
  assert.throws(() => composer.rewardSplitForTotal(2, split, parent), /exactly 1/);
});
test("claimed, unverified and incompatible parents cannot authorize the exception", async () => {
  for (const mutate of [f => f.item.work_state = "claimed", f => f.item.verification_ready = false, f => f.item.payment_committed = false,
    f => f.item.network = "base-sepolia", f => f.record.terms_hash = "changed", f => f.record.document.benchmark.minimum_child_target = 2000000,
    f => f.record.document.benchmark.required_child_verifier_threshold = 1, f => f.record.document.benchmark.terms_registry = fixture.parent_solver,
    f => f.record.document.verification_policy.verifier_module = fixture.parent_solver, f => f.record.document.benchmark = null]) {
    const f = parentFixture(); mutate(f); await assert.rejects(helper.resolve(f.context, f.client));
  }
});
test("funding request preserves exact amount and work and requires independent participants", async () => {
  const f = parentFixture(), parent = await helper.resolve(f.context, f.client), draft = fixture.terms.document;
  const request = helper.request(draft, parent, fixture.parent_solver, split, 3, fixture.child_create.creation_nonce);
  assert.equal(request.verifier_reward.amount, 10000);
  assert.equal(request.claim_window_seconds, 259200);
  assert.deepEqual(request.benchmark_source, draft.benchmark.source);
  assert.equal(request.funding_deadline, parent.terms.contract_terms.funding_deadline);
  assert.throws(() => helper.request(draft, parent, parent.terms.contract_terms.creator_wallet, split, 3, request.creation_nonce), /parent creator/);
  for (const solver of [null, fixture.parent_solver]) assert.throws(() => helper.request(draft, { ...parent, intended_child_solver: solver }, fixture.parent_solver, split, 3, request.creation_nonce), /different intended child solver/);
});
test("browser verifies the Rust planner's exact terms and three ordered wallet calls", () => {
  assert.equal(fixedHelper.validatePlan(fixture, inputFor(), split, evm), fixture);
});
test("altered funding, chain, quorum, preimages and wallet calls fail before signing", () => {
  for (const mutate of [
    p => p.network.chain_id = 1, p => p.parent_solver = p.intended_child_solver, p => p.task_verifier_threshold = 1,
    p => p.terms.document.contract_terms.solver_reward.amount++, p => p.terms.document.contract_terms.funding_deadline++,
    p => p.terms.document.acceptance_criteria.push("weakened"), p => p.terms.document.benchmark.runner_manifest.command = ["false"],
    p => p.terms.terms_hash = "0x" + "ff".repeat(32), p => p.canonical_terms_hex += "00",
    p => p.pre_claim_wallet_calls.reverse(), p => p.pre_claim_wallet_calls[0].to = p.parent_solver,
    p => p.pre_claim_wallet_calls[1].data = p.pre_claim_wallet_calls[1].data.slice(0, -64) + "f".repeat(64),
    p => p.pre_claim_wallet_calls[2].from = p.intended_child_solver, p => p.pre_claim_wallet_calls[2].value_wei = "1",
    p => p.child_creation.network.chain_id = 1,
  ]) {
    const changed = structuredClone(fixture); mutate(changed);
    assert.throws(() => fixedHelper.validatePlan(changed, inputFor(), split, evm));
  }
});

test("the confirmed funding branch uses the bounded child plan and retries a lost preparation with the same nonce", async () => {
  const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
  const start = source.indexOf("  async function fundApprovedBounty()"), end = source.indexOf("  function configureSpeech", start);
  const storage = new Map(), requests = [], sent = [], consents = [];
  const win = { sessionStorage: { getItem: key => storage.get(key) || null, setItem: (key, val) => storage.set(key, val), removeItem: key => storage.delete(key) },
    AgentBountiesLegal: { requireAcceptance: async () => { consents.push("human commitment"); return { durable: true }; } },
    AgentBountiesWorkflow: { createClient: () => ({}) }, AgentBountiesEvm: evm };
  const journal = require("../site/marketplace-workflow.js").createPostingJournal(win);
  const parent = { terms_hash: "fixed-parent" };
  const fields = [{ disabled: false }, { disabled: false }];
  const state = { approved: true, provider: { request: async ({ method }) => method === "eth_accounts" ? [fixture.parent_solver] : "0x2105" }, account: fixture.parent_solver,
    balances: { usdc: 1000000n, required: 1000000n, eth: 1n }, draft: fixture.terms.document, metaParent: parent, fundingUsdc: 1, taskWindowDays: 3 };
  let fail = true, nonces = 0;
  const run = vm.runInNewContext(source.slice(start, end) + "; fundApprovedBounty", {
    postingBusy: false, postingJournal: journal, state, window: win, ui: { form: { querySelectorAll: () => fields }, fundNow: {}, badge: {} },
    track() {}, setPaymentStatus() {}, refreshWalletReadiness: async () => {}, loadProtocol: async () => ({ api_base_url: "https://api.agentbounties.app", factory: fixture.child_creation.factory_contract, chain_id: 8453 }),
    currentRewardSplit: () => split, randomBytes32: () => nonces++ ? "0x" + "77".repeat(32) : fixture.child_create.creation_nonce,
    metaChild: { ...helper, resolve: async () => parent, request: (_draft, _parent, _wallet, _split, _days, nonce) => ({ ...inputFor(), creation_nonce: nonce }), validatePlan: fixedHelper.validatePlan },
    requestJson: async (url, options) => {
      assert.equal(consents.length > 0, true); assert.match(url, /standing-meta-v2-child-preparation$/);
      requests.push(JSON.parse(options.body)); if (fail) { fail = false; throw new Error("lost response"); }
      return { ...fixture, hosted_terms_published: true };
    },
    validateCreationPlan() {}, formatUsdc: v => Number(v).toFixed(2),
    sendWalletCalls: async calls => { sent.push(calls); journal.checkpoint("batch_submitted", { id: "test-only" }); },
    pollCreation: async () => true, fetchFeedItem: async () => ({ verification_ready: true }),
  });
  await run(); assert.equal(sent.length, 0); assert.equal(journal.load(), null); assert.ok(fields.every(field => !field.disabled));
  await run(); assert.equal(sent.length, 1); assert.deepEqual(sent[0], fixture.pre_claim_wallet_calls);
  assert.equal(requests[0].creation_nonce, requests[1].creation_nonce);
  assert.equal(journal.load().phase, "funding_confirmed");
  assert.ok(fields.every(field => field.disabled));
  await run(); assert.equal(sent.length, 1);
});
