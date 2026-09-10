"use strict";
const { test } = require("node:test"), assert = require("node:assert/strict"), vm = require("node:vm"), fs = require("node:fs");
const review = require("../site/creator-review.js"), workspace = require("../site/creator-review-workspace.js");
const contract = "0x" + "22".repeat(20), creator = "0x" + "11".repeat(20), solver = "0x" + "33".repeat(20);
const hash = (byte) => "0x" + byte.repeat(32);
function environment(options = {}) {
  const elements = new Map(), records = new Map(), requests = [];
  const element = () => ({ hidden: true, disabled: true, textContent: "", children: [], handlers: {}, addEventListener(name, fn) { this.handlers[name] = fn; }, replaceChildren() { this.children = []; }, append(child) { this.children.push(child); } });
  const root = element(); root.querySelector = (name) => { if (!elements.has(name)) elements.set(name, element()); return elements.get(name); };
  const win = { location: { search: `?bountyContract=${contract}` }, document: { querySelector: () => root, createElement: element }, sessionStorage: { getItem: (key) => records.get(key) || null, setItem: (key, value) => records.set(key, value), removeItem: (key) => records.delete(key) } };
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/evm.js"), "utf8"), { window: win, TextEncoder, BigInt, Uint8Array });
  const now = Math.floor(Date.now() / 1000), terms = { terms_hash: hash("aa"), policy_hash: hash("bb"), document: { acceptance_criteria: ["Files open", "Dimensions match"], contract_terms: { verification_window_seconds: 3600 }, benchmark: { engine: review.ENGINE, delivery_deadline: now + 1000 } } };
  const job = { bounty_contract: contract, bounty_id: hash("cc"), round: 1, solver_wallet: solver, threshold: 1, eligible_verifiers: [creator], terms, verifier_reward: "2000000", current_solver_payout: "20000000", verification_expires_at: now + 3600, submission_evidence: { artifact_hash: hash("dd"), evidence_hash: hash("ee") } };
  const submitted = { kind: "submission_added", id: "submission", contract_address: contract, bounty_id: job.bounty_id, block_number: 1, log_index: 0, data: { round: 1, solver, submission_hash: hash("dd"), evidence_hash: hash("ee") } };
  const item = { status: "submitted", bounty_contract: contract, bounty_id: job.bounty_id, creator, terms_valid: true, terms_hash: terms.terms_hash, terms, events: [submitted] };
  win.AgentBountiesWorkflow = { createClient: () => ({ async request(path, body) {
    if (path.includes("/feed?")) return [item];
    if (path.includes("/verification-jobs?")) return [job];
    if (path.endsWith("verification-attestation-plan")) { const plan = workspace.typedData(body.attestation); if (options.alteredSignature) plan.message.verifier = solver; return plan; }
    if (path.endsWith("attestation-settlement-plan")) return { from: creator, to: options.alteredTransaction ? solver : contract, value_wei: "0", data: workspace.settlementData(body.attestations[0], win.AgentBountiesEvm) };
    throw new Error(`Unexpected endpoint ${path}`);
  } }) };
  let rejected = false;
  win.ethereum = { async request({ method }) {
    requests.push(method);
    if (method === "eth_accounts" || method === "eth_requestAccounts") return [options.foreignWallet ? solver : creator];
    if (method === "eth_chainId") return "0x2105";
    if (method === "eth_signTypedData_v4") return "0x" + "ab".repeat(65);
    if (method === "eth_sendTransaction") {
      if (options.rejectOnce && !rejected) { rejected = true; throw Object.assign(new Error("Rejected"), { code: 4001 }); }
      if (options.lostResponse) throw new Error("Connection lost after sending");
      return hash("ff");
    }
    throw new Error(method);
  } };
  workspace.start(win);
  const checks = terms.document.acceptance_criteria.map((criterion) => ({ criterion, passed: true, reason: "Verified in the supplied artifact" }));
  return { win, item, job, checks, requests, records, elements, stage: () => win.AgentBountiesCreatorReviewWorkspace.stage({ checks }), click: () => elements.get("button").handlers.click({ isTrusted: true }), refresh: () => win.AgentBountiesCreatorReviewWorkspace.refresh() };
}
test("creator review is explicit, binds a calendar deadline, and cannot replace meta verification", () => {
  const draft = { review_mode: "creator", delivery_deadline: new Date(Date.now() + 7 * 86400000).toISOString() };
  const prepared = review.prepare(draft);
  assert.equal(prepared.delivery_deadline, draft.delivery_deadline);
  assert.equal(review.deadline("2099-09-13T18:00:00-06:00"), "2099-09-13T18:00:00-06:00");
  assert.equal(review.ready(prepared.benchmark, prepared.evidence_schema), true);
  assert.equal(review.prepare({ benchmark: null }).benchmark, null);
  assert.throws(() => review.prepare({ ...draft, meta_child: {} }), /meta/);
  assert.throws(() => review.prepare({ ...draft, benchmark: { engine: "sandboxed_regression_v1" } }), /replace/);
  assert.throws(() => review.prepare({ ...draft, delivery_deadline: "Sunday 6pm" }), /timestamp/);
  assert.throws(() => review.prepare({ ...draft, delivery_deadline: "2000-01-01T00:00:00Z" }), /future/);
});
test("the assessment must cover exact criteria and cannot pass a late submission", () => {
  const env = environment();
  assert.equal(workspace.assessment(env.job, { checks: env.checks }).passed, true);
  assert.throws(() => workspace.assessment(env.job, { checks: [...env.checks].reverse() }), /match/);
  env.job.verification_expires_at += 1001;
  assert.equal(workspace.assessment(env.job, { checks: env.checks }).passed, false);
});
test("staging cannot sign and changed signature or transaction scopes cannot reach settlement", async () => {
  for (const option of ["alteredSignature", "alteredTransaction", "foreignWallet"]) {
    const env = environment({ [option]: true }); await env.stage(); assert.equal(env.requests.length, 0);
    await env.click(); assert.equal(env.requests.includes("eth_sendTransaction"), false);
    if (option !== "alteredTransaction") assert.equal(env.requests.includes("eth_signTypedData_v4"), false);
  }
});
test("a lost send response is not payment and cannot send again", async () => {
  const env = environment({ lostResponse: true }); await env.stage(); await env.click(); await env.click();
  assert.equal(env.requests.filter((method) => method === "eth_sendTransaction").length, 1);
  assert.equal((await env.refresh()).paid, false);
  await assert.rejects(env.stage(), /recorded verdict/);
  const event = { ...env.item.events[0], kind: "bounty_settled", id: "settled", block_number: 2, data: { ...env.item.events[0].data, round: 2 } };
  env.item.events.push(event); assert.equal((await env.refresh()).paid, false);
  event.data.round = 1; assert.equal((await env.refresh()).paid, true);
});
test("explicitly rejected transaction resumes the same signed verdict", async () => {
  const env = environment({ rejectOnce: true }); await env.stage(); await env.click(); await env.click();
  assert.equal(env.requests.filter((method) => method === "eth_signTypedData_v4").length, 1);
  assert.equal(env.requests.filter((method) => method === "eth_sendTransaction").length, 2);
  assert.equal((await env.refresh()).status, "pending_confirmation");
});

test("saved brief edits reach the shared journey and invalidate an older funding proposal", () => {
  const flow = require("../site/marketplace-workflow.js"), records = new Map(), listeners = new Map(), elements = new Map();
  const element = () => ({ value: "", textContent: "", open: false, handlers: {}, setCustomValidity(value) { this.validationMessage = value; }, addEventListener(name, fn) { this.handlers[name] = fn; } });
  for (const name of ["#bounty-composer-form", "#bounty-composer-input", "[data-brief-budget]", "[data-brief-deadline]", "[data-brief-status]", "[data-brief-timezone]", "[data-ai-options]", "[data-export-draft]"]) elements.set(name, element());
  const document = { activeElement: null, documentElement: { dataset: {} }, querySelector: (name) => elements.get(name), addEventListener() {} };
  let invalidations = 0;
  const win = { AgentBountiesWorkflow: flow, AgentBountiesPostingBrief: require("../site/posting-brief.js"), AgentBountiesComposer: { invalidate() { invalidations++; } }, document, setInterval() {},
    crypto: require("node:crypto").webcrypto, location: new URL("https://agentbounties.app/post.html"),
    sessionStorage: { getItem: (key) => records.get(key) || null, setItem: (key, value) => records.set(key, value) },
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    addEventListener(name, fn) { listeners.set(name, fn); }, dispatchEvent(event) { listeners.get(event.type)?.(event); } };
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/posting-workspace.js"), "utf8"), { window: win, document, Intl, Date, URL, Blob });
  const goal = elements.get("#bounty-composer-input"), budget = elements.get("[data-brief-budget]"), deadline = elements.get("[data-brief-deadline]");
  goal.value = "Make a dimensioned CAD model"; document.activeElement = goal; goal.handlers.input();
  const client = flow.createClient(win); assert.equal(client.load().goal, goal.value);
  document.activeElement = budget; budget.value = "20"; budget.handlers.input();
  const cutoff = new Date(Date.now() + 7 * 86400000).toISOString();
  client.save({ ...client.load(), draft: { goal: goal.value, solver_reward_usdc: "18", verifier_reward_usdc: "2" }, brief: { ...client.load().brief, deadline_at: cutoff } });
  assert.notEqual(deadline.value, "", "AI staging updates the deadline even while another field has focus");
  budget.value = "25"; budget.handlers.input();
  assert.equal(client.load().brief.budget_usdc, "25"); assert.equal(client.load().draft_stale, true); assert.equal(invalidations, 1);
  assert.ok(client.load().brief.deadline_at, "editing the budget must not erase the deadline");
  const timezone = elements.get("[data-brief-timezone]");
  document.activeElement = timezone; timezone.value = "America/Mexico_City"; timezone.handlers.input();
  document.activeElement = deadline; deadline.value = "2026-12-10T21:00"; deadline.handlers.input();
  const newCutoff = "2026-12-11T21:00:23.456-06:00";
  document.activeElement = null;
  client.save({ ...client.load(), draft_stale: false, brief: { ...client.load().brief, deadline_at: newCutoff }, draft: { goal: goal.value, solver_reward_usdc: "23", verifier_reward_usdc: "2", delivery_deadline: newCutoff } });
  assert.equal(deadline.value, "2026-12-11T21:00", "restaging an exact deadline replaces old display fields");
  const before = invalidations;
  document.activeElement = budget; budget.value = "25.00"; budget.handlers.input();
  assert.equal(invalidations, before, "equivalent amount formatting preserves the proposal approval");
  assert.equal(Date.parse(client.load().brief.deadline_at), Date.parse(newCutoff));
  document.activeElement = timezone; timezone.value = "CST"; timezone.handlers.input();
  assert.equal(client.load().brief.deadline_at, null, "ambiguous zones cannot stage a guessed deadline");
  assert.match(deadline.validationMessage, /ambiguous/);
});
