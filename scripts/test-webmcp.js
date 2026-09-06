"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const { webcrypto } = require("node:crypto");
const flow = require("../site/marketplace-workflow.js");
const market = require("../site/marketplace.js");
const { validateCalls, start: startParticipant } = require("../site/participate.js");
const contract = "0x" + "11".repeat(20), wallet = "0x" + "22".repeat(20);
const intentId = "10000000-0000-4000-8000-000000000001";
const amount = (n) => ({ amount: String(n), unit: "base_units", decimals: 6, currency: "USDC" });
function item(extra = {}) {
  return { opportunity_id: `canonical:base-mainnet:${contract}`, source_id: contract, network: "base-mainnet", source_type: "canonical_base", source_status: "claimable", work_state: "claimable", payment_state: "escrowed", payment_committed: true, verification_ready: true, reward: amount(2000000), bond: amount(10000), funded_amount: amount(2010000), funding_target: amount(2010000), terms_hash: "0x" + "aa".repeat(32), title: "Test work", goal: "Deliver a tested result", public_url: "https://github.com/example/incomplete", ...extra };
}
function environment(path = "/", entries = [item()]) {
  const storage = new Map(), tools = new Map(), elements = new Map(), requests = [], navigations = [], listeners = new Map();
  const el = () => ({ hidden: false, disabled: false, dataset: {}, textContent: "", listeners: new Map(), addEventListener(name, callback) { this.listeners.set(name, callback); }, setAttribute() {}, removeAttribute() {}, replaceChildren() {}, append() {} });
  const document = { title: "Test", readyState: "complete", hidden: false,
    modelContext: { registerTool(tool) { tools.set(tool.name, tool); } }, querySelector(s) { return elements.get(s) || null; }, getElementById(id) { return elements.get(`#${id}`); }, querySelectorAll() { return []; }, addEventListener() {}, createElement: el };
  const location = new URL(`https://agentbounties.app${path}`); location.assign = (url) => navigations.push(url);
  const window = { document, location, crypto: webcrypto, console, AgentBountiesWorkflow: flow,
    sessionStorage: { getItem: (key) => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: (key) => storage.delete(key) },
    localStorage: { getItem: () => null }, addEventListener(name, fn) { listeners.set(name, fn); }, dispatchEvent() {},
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options?.detail; } }, Event: class {},
    setTimeout(fn) { fn(); }, setInterval() {},
    async fetch(url, options) {
      requests.push({ url, ...options });
      const payload = url.includes("/opportunities?") ? { schema_version: "agent-bounties/opportunity-projection-v1", generated_at: "2026-09-05T23:00:00Z", items: entries }
        : { intent_id: intentId, bounty_contract: contract, status: "review_required", network: "base-mainnet", action: "solve", details: {}, expected_canonical_events: ["bounty_claimed"] };
      if (options?.method === "POST" && url.endsWith("/action-intents")) Object.assign(payload, JSON.parse(options.body));
      return { ok: true, json: async () => payload };
    },
  };
  function register() {
    vm.runInNewContext(fs.readFileSync(require.resolve("../site/webmcp.js"), "utf8"), { window, document, URL, URLSearchParams, console, AbortController, CustomEvent: window.CustomEvent, setTimeout, TextEncoder });
  }
  return { window, document, elements, tools, storage, requests, navigations, register, el };
}
test("homepage advertises usable tools and one plain-language next step", async () => {
  const env = environment(); env.register();
  const context = env.tools.get("agent_bounties_get_page_context").execute();
  assert.ok(context.available_tools.includes("agent_bounties_start_journey"));
  assert.ok(!context.available_tools.includes("agent_bounties_get_bounty_review"));
  assert.ok(!context.available_tools.includes("agent_bounties_get_competition_manifest"));
  assert.match(context.guidance.autonomy, /without asking permission again/);
  assert.match(context.guidance.consent, /Never click their consent/);
});
test("unsupported browser preserves ordinary navigation without registering tools", () => {
  const env = environment(); delete env.document.modelContext;
  env.elements.set("[data-agent-guidance]", env.el()); env.register();
  assert.equal(env.tools.size, 0);
  assert.match(env.elements.get("[data-agent-guidance]").textContent, /WebMCP support/);
});
test("UI and WebMCP share funding and verification readiness", async () => {
  const invalid = [item({ verification_ready: false }), item({ funded_amount: amount(100) }), item({ payment_committed: false }), item({ network: "base-sepolia" }), item({ source_id: "not-an-address" })];
  const env = environment("/", [...invalid, item()]); env.register();
  const result = await env.tools.get("agent_bounties_list_ready_work").execute({ search: "test" });
  assert.equal(result.total_ready, 1);
  for (const entry of invalid) assert.equal(market.isReadyToEarn(entry), false);
  assert.equal(env.requests[0].cache, "no-store");
});
test("ready work opens a first-party workspace instead of a GitHub mirror", async () => {
  const env = environment(); env.register();
  const result = await env.tools.get("agent_bounties_open_opportunity").execute({ opportunity_id: item().opportunity_id });
  assert.match(result.url, /^https:\/\/agentbounties.app\/participate.html\?bountyContract=/);
  assert.equal(env.navigations[0], result.url);
});
test("scoring-closed items are never counted as actionable now", async () => {
  const closed = item({ evidence_requirements: { scoring_window: { starts_at: "2025-01-01", ends_at: "2025-01-02" } } });
  assert.equal(market.filterItems([closed], "", "now", Date.now()).length, 0);
  const env = environment("/", [closed]); env.register();
  assert.equal((await env.tools.get("agent_bounties_list_ready_work").execute({ timing: "now" })).matched, 0);
});
test("journeys survive navigation and routine updates preserve the request", () => {
  const env = environment(); const client = flow.createClient(env.window);
  const started = client.start({ role: "earn", goal: "Find documentation work" });
  const next = flow.createClient(env.window).start({ role: "earn", preferences: "Small tasks" });
  assert.equal(next.id, started.id); assert.equal(next.goal, started.goal);
});
test("a dropped preparation response reuses its durable key across client reloads", async () => {
  const env = environment(); const original = env.window.fetch; let fail = true;
  env.window.fetch = async (url, options) => {
    const response = await original(url, options);
    if (url.endsWith("/action-intents") && fail) { fail = false; throw new Error("network lost"); }
    return response;
  };
  const input = { action: "solve", opportunity_id: item().opportunity_id };
  await assert.rejects(flow.createClient(env.window).prepareAction(input), /network lost/);
  const result = await flow.createClient(env.window).prepareAction(input);
  const writes = env.requests.filter((r) => r.url.endsWith("/action-intents"));
  assert.equal(writes.length, 2);
  assert.equal(JSON.parse(writes[0].body).idempotency_key, JSON.parse(writes[1].body).idempotency_key);
  assert.equal(result.user_confirmation_required, true);
  await flow.createClient(env.window).prepareAction(input);
  assert.equal(env.requests.filter((r) => r.url.endsWith("/action-intents")).length, 2);
});
test("changed submission evidence creates a different review instead of borrowing approval", async () => {
  const env = environment(), client = flow.createClient(env.window);
  const input = { action: "complete", opportunity_id: item().opportunity_id, artifact_reference: "https://example.com/result", evidence: { passed: 1 } };
  await client.prepareAction(input); await client.prepareAction({ ...input, evidence: { passed: 2 } });
  const writes = env.requests.filter((r) => r.url.endsWith("/action-intents"));
  assert.notEqual(JSON.parse(writes[0].body).idempotency_key, JSON.parse(writes[1].body).idempotency_key);
});
test("evidence preparation rejects credential fields before transmission", async () => {
  const env = environment(), client = flow.createClient(env.window);
  await assert.rejects(client.prepareAction({ action: "complete", opportunity_id: item().opportunity_id, artifact_reference: "https://example.com/result", evidence: { private_key: "test" } }), /credentials/);
  assert.equal(env.requests.filter((r) => r.method === "POST").length, 0);
});
test("paid language requires canonical settlement evidence", async () => {
  const env = environment();
  let response = { status: "pending_confirmation", paid: true, transaction_hash: "0xabc" };
  env.window.fetch = async () => ({ ok: true, json: async () => response });
  assert.equal((await flow.createClient(env.window).progress(intentId)).paid, false);
  response = { ...response, status: "confirmed", canonical_event_id: intentId, confirmed_block: 12, canonical_event_kind: "submission_added" };
  assert.equal((await flow.createClient(env.window).progress(intentId)).paid, false);
  response.canonical_event_kind = "bounty_settled";
  assert.equal((await flow.createClient(env.window).progress(intentId)).paid, true);
});
test("unavailable and closed competitions cannot start child bounties", async () => {
  const env = environment("/competition.html"); env.register();
  env.elements.set("[data-competition-app]", { dataset: { state: "unavailable" } });
  await assert.rejects(env.tools.get("agent_bounties_start_competition_child_bounty").execute(), /unavailable/);
  env.elements.get("[data-competition-app]").dataset.state = "ready";
  env.elements.set("[data-machine-request]", { textContent: JSON.stringify({ phase: "ended", competition_contract: contract }) });
  await assert.rejects(env.tools.get("agent_bounties_start_competition_child_bounty").execute(), /closed/);
  assert.equal(env.navigations.length, 0);
});
test("child handoff requires and preserves the exact parent", async () => {
  const env = environment("/competition.html"); env.register();
  env.elements.set("[data-competition-app]", { dataset: { state: "ready" } });
  env.elements.set("[data-machine-request]", { textContent: JSON.stringify({ phase: "now", competition_contract: contract }) });
  let href = "/#post-a-bounty";
  env.elements.set("[data-child-post-started][href]", { getAttribute: () => href });
  await assert.rejects(env.tools.get("agent_bounties_start_competition_child_bounty").execute(), /lost its competition/);
  href = `/post.html?parentCompetition=${contract}&network=base-mainnet`;
  await env.tools.get("agent_bounties_start_competition_child_bounty").execute();
  assert.equal(flow.createClient(env.window).load().parent_competition.competition_contract, contract);
});
test("funding review never approves an unapproved card", async () => {
  const env = environment("/post.html"); env.register();
  let clicks = 0;
  env.elements.set("[data-approve-card]", { dataset: { approved: "false" } });
  env.elements.set("[data-open-funding]", { disabled: false, click() { clicks++; } });
  await assert.rejects(env.tools.get("agent_bounties_open_funding_review").execute(), /explicitly approve/);
  assert.equal(clicks, 0);
});
test("exact transaction validation rejects changed destinations, unlimited approval and foreign accounts", () => {
  const env = environment();
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/evm.js"), "utf8"), { window: env.window, TextEncoder, crypto: webcrypto });
  const evm = env.window.AgentBountiesEvm;
  const selector = (s) => evm.keccak256Hex(evm.textHex(s)).slice(0, 10);
  const approve = { from: wallet, to: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", value_wei: 0, data: selector("approve(address,uint256)") + evm.addressWord(contract) + evm.uint256Word(10000) };
  const claim = { from: wallet, to: contract, value_wei: 0, data: selector("claim()") };
  const bound = { action: "solve", contract, wallet, amount: 10000 };
  assert.equal(validateCalls([approve, claim], bound, evm).length, 2);
  assert.throws(() => validateCalls([{ ...approve, to: contract }, claim], bound, evm), /differs/);
  assert.throws(() => validateCalls([{ ...approve, data: approve.data.slice(0, -64) + "f".repeat(64) }, claim], bound, evm), /differs/);
  assert.throws(() => validateCalls([{ ...claim, from: contract }], bound, evm), /differs/);
  assert.throws(() => validateCalls([{ ...claim, value_wei: 1 }], bound, evm), /differs/);
});

async function participantFixture(action = "solve") {
  const env = environment(`/participate.html?bountyContract=${contract}&network=base-mainnet&intent=${intentId}`);
  const source = fs.readFileSync(require.resolve("../site/participate.js"), "utf8");
  for (const match of source.matchAll(/\[data-[a-z-]+\]/g)) env.elements.set(match[0], env.el());
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/evm.js"), "utf8"), { window: env.window, TextEncoder, crypto: webcrypto });
  const evm = env.window.AgentBountiesEvm, sent = [], observations = [], publications = [];
  const selector = (s) => evm.keccak256Hex(evm.textHex(s)).slice(0, 10);
  const hash = "0x" + "ab".repeat(32), hash2 = "0x" + "cd".repeat(32), bountyId = "test-bounty";
  const details = { artifact_reference: "https://example.com/result", evidence: { tests_passed: true } };
  const submission = { network: { chain_id: 8453 }, bounty_contract: contract, solver: wallet, bounty_id: bountyId, round: 2,
    submission_hash: hash, evidence_hash: hash2, evidence_publication: { network: "base-mainnet", bounty_contract: contract, solver_wallet: wallet, bounty_id: bountyId, round: 2, ...details } };
  const calls = action === "complete" ? [{ from: wallet, to: contract, value_wei: 0, data: selector("submit(bytes32,bytes32)") + evm.bytes32Word(hash) + evm.bytes32Word(hash2) }]
    : [{ from: wallet, to: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", value_wei: 0, data: selector("approve(address,uint256)") + evm.addressWord(contract) + evm.uint256Word(10000) }, { from: wallet, to: contract, value_wei: 0, data: action === "fund" ? selector("fund(uint256)") + evm.uint256Word(10000) : selector("claim()") }];
  const state = { mined: true, account: wallet, dropObservation: false, dropWallet: false,
    intent: { intent_id: intentId, network: "base-mainnet", bounty_contract: contract, action, amount_base_units: action === "fund" ? 10000 : null, status: "review_required", details, actor_wallet: wallet },
    feed: { bounty_id: bountyId, bounty_contract: contract, terms_valid: true, validation_errors: [], status: action === "complete" ? "claimed" : "claimable", verification_ready: true, claim_bond: "10000", solver_reward: "2000000", funded_amount: "2010000", target_amount: "2010000", terms: { document: { title: "Fixture", goal: "Test workflow", acceptance_criteria: ["Pass the check"] } }, events: [] } };
  env.window.ethereum = { async request({ method, params }) {
    if (method === "eth_accounts" || method === "eth_requestAccounts") return [state.account];
    if (method === "eth_chainId") return "0x2105";
    if (method === "eth_getTransactionReceipt") return state.mined ? { status: "0x1" } : null;
    if (method === "eth_sendTransaction") { sent.push(params[0]); if (state.dropWallet) throw new Error("wallet response lost"); return sent.length === 1 ? hash : hash2; }
    throw new Error(`Unexpected wallet method ${method}`);
  } };
  env.window.AgentBountiesLegal = { async requireAcceptance() { return { durable: true }; } };
  env.window.fetch = async (url, options) => {
    let payload;
    if (url.includes("/feed?")) payload = state.feed ? [state.feed] : [];
    else if (url.includes("/verification-jobs?")) payload = [];
    else if (url.endsWith("/observations")) {
      observations.push(JSON.parse(options.body));
      if (state.dropObservation) { state.dropObservation = false; throw new Error("observation response lost"); }
      state.intent.transaction_hash = observations.at(-1).transaction_hash; state.intent.status = "pending_confirmation";
      payload = state.intent;
    } else if (url.endsWith("/claim-plan")) payload = { network: { chain_id: 8453 }, bounty_contract: contract, claim_bond: "10000", wallet_calls: calls };
    else if (url.endsWith("/contribution-plan")) {
      const input = JSON.parse(options.body); assert.deepEqual(input, { network: "base-mainnet", contribution: { bounty_contract: contract, contributor: wallet, amount: { amount: 10000, currency: "usdc" } } });
      payload = { network: { chain_id: 8453 }, wallet_calls: calls };
    } else if (url.endsWith("/submission-preparation")) payload = submission;
    else if (url.endsWith("/submission-plan")) payload = calls[0];
    else if (url.endsWith("/submission-evidence")) { publications.push(JSON.parse(options.body)); payload = { status: "published" }; }
    else if (url.endsWith(`/action-intents/${intentId}`)) payload = state.intent;
    else throw new Error(`Unexpected HTTP request ${url}`);
    return { ok: true, json: async () => JSON.parse(JSON.stringify(payload)) };
  };
  await startParticipant(env.window, env.document);
  const click = (selector, trusted = true) => env.elements.get(selector).listeners.get("click")({ isTrusted: trusted });
  return { ...env, state, sent, observations, publications, submission, click, reload: () => startParticipant(env.window, env.document) };
}

test("one human confirmation completes ordered claim calls and retries cannot repeat payment", async () => {
  const env = await participantFixture();
  await env.click("[data-wallet-connect]", false);
  await env.click("[data-wallet-confirm]", false);
  assert.equal(env.sent.length, 0);
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 2); assert.equal(env.observations.length, 1);
  await env.click("[data-wallet-confirm]"); await env.reload(); await env.click("[data-wallet-connect]");
  assert.equal(env.sent.length, 2);
});
test("pending approval survives reload and resumes without a second approval transaction", async () => {
  const env = await participantFixture(); env.state.mined = false;
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 1);
  await env.reload(); await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 1);
  env.state.mined = true; await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 2);
});
test("changed accounts and uncertain wallet responses stop further signing", async () => {
  const env = await participantFixture();
  await env.click("[data-wallet-connect]"); env.state.account = contract; await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 0);
  env.state.account = wallet; env.state.dropWallet = true; await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 1);
  await env.reload(); await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 1);
});
test("lost observation recovers with the same hash and never resends the submission", async () => {
  const env = await participantFixture("complete"); env.state.dropObservation = true;
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 1);
  await env.reload(); assert.equal(env.observations.length, 2);
  assert.deepEqual(env.observations[0], env.observations[1]);
  await env.click("[data-wallet-confirm]"); assert.equal(env.sent.length, 1);
});
test("confirmed publication reuses frozen consent and payment requires the same solver, round and hashes", async () => {
  const env = await participantFixture("complete");
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  await assert.rejects(env.window.AgentBountiesParticipation.publishEvidence(), /Wait for the exact/);
  Object.assign(env.state.intent, { status: "confirmed", canonical_event_id: intentId, canonical_event_kind: "submission_added", confirmed_block: 8 });
  await env.window.AgentBountiesParticipation.publishEvidence(); await env.window.AgentBountiesParticipation.publishEvidence();
  assert.equal(env.publications.length, 1); assert.deepEqual(env.publications[0], env.submission.evidence_publication);
  const event = { id: "paid", kind: "bounty_settled", block_number: 9, contract_address: contract, bounty_id: "test-bounty", data: { solver: wallet, round: 2, submission_hash: env.submission.submission_hash, evidence_hash: env.submission.evidence_hash, solver_payout: 2010000 } };
  env.state.feed.events = [{ ...event, data: { ...event.data, round: 1 } }];
  assert.equal((await env.window.AgentBountiesParticipation.refresh()).paid, false);
  env.state.feed.events = [{ ...event, data: { ...event.data, solver: contract } }];
  assert.equal((await env.window.AgentBountiesParticipation.refresh()).paid, false);
  env.state.feed.events = [event]; assert.equal((await env.window.AgentBountiesParticipation.refresh()).paid, true);
});
test("altered evidence returned by preparation cannot reach a wallet", async () => {
  const env = await participantFixture("complete"); env.submission.evidence_publication.evidence = { tests_passed: false };
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]"); assert.equal(env.sent.length, 0);
});
test("posting recovery survives navigation and cannot discard a live authorization or batch", () => {
  const env = environment(), plan = { predicted_bounty_contract: contract, bounty_id: "posting" };
  let journal = flow.createPostingJournal(env.window); journal.prepare(plan); journal.checkpoint("signing");
  journal.reject({ code: 4001 }); assert.equal(journal.load(), null);
  journal.prepare(plan); journal.checkpoint("authorized"); journal.checkpoint("sending"); journal.reject({ code: 4001 });
  journal = flow.createPostingJournal(env.window); assert.equal(journal.load().authorizationIssued, true);
  assert.throws(() => journal.prepare(plan), /already recorded/);
  assert.throws(() => flow.createClient(env.window).start({ role: "post", new_task: true }), /existing posting/);
  journal.checkpoint("funding_confirmed"); flow.createClient(env.window).start({ role: "post", new_task: true });
  assert.equal(journal.load(), null);
  journal.prepare(plan); journal.checkpoint("batch_submitted", { id: "pending-wallet-batch" }); journal.reject({ code: 4001 });
  assert.equal(journal.load().transactions.length, 1);
});
test("an expired unbroadcast review renews but a pending wallet step cannot", async () => {
  const env = environment(), original = env.window.fetch; let expired = false;
  env.window.fetch = async (url, options) => {
    const response = await original(url, options), payload = await response.json();
    if (expired && url.endsWith(`/action-intents/${intentId}`)) payload.status = "expired";
    return { ok: true, json: async () => payload };
  };
  const input = { action: "solve", opportunity_id: item().opportunity_id }, client = flow.createClient(env.window);
  await client.prepareAction(input); expired = true;
  env.storage.set(`agent-bounties.wallet-step.v1:${intentId}`, JSON.stringify({ sending: true }));
  await assert.rejects(client.prepareAction(input), /expired after a wallet step/);
  assert.equal(env.requests.filter((r) => r.method === "POST").length, 1);
  env.storage.delete(`agent-bounties.wallet-step.v1:${intentId}`);
  await client.prepareAction(input);
  const writes = env.requests.filter((r) => r.method === "POST"); assert.equal(writes.length, 2);
  assert.notEqual(JSON.parse(writes[0].body).idempotency_key, JSON.parse(writes[1].body).idempotency_key);
});
test("reading a staged journey returns review instead of resetting consent by restaging", async () => {
  const env = environment("/post.html"), client = flow.createClient(env.window);
  client.save({ ...client.start({ role: "post", goal: "Draft a task" }), draft: { title: "Preserve me" } });
  client.start({ role: "post", preferences: "Keep it small" }); env.register();
  const result = env.tools.get("agent_bounties_get_journey").execute();
  assert.equal(result.journey.draft.title, "Preserve me");
  assert.equal(result.next_action.tool, "agent_bounties_get_bounty_review");
});
test("an already open funding review is idempotent", async () => {
  const env = environment("/post.html"); env.register();
  env.elements.set("[data-approve-card]", { dataset: { approved: "true" } });
  env.elements.set("#funding-dialog", { open: true });
  assert.equal((await env.tools.get("agent_bounties_open_funding_review").execute()).status, "wallet_review_opened");
});
test("posting batch fallback occurs only for explicit unsupported-method errors", async () => {
  const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
  const fn = source.slice(source.indexOf("  async function sendWalletCalls("), source.indexOf("  function contractTerms("));
  for (const code of [-32601, 4200, 4001, -32000, undefined]) {
    const env = environment(), journal = flow.createPostingJournal(env.window); journal.prepare({ predicted_bounty_contract: contract, bounty_id: "batch" });
    let sends = 0;
    const send = vm.runInNewContext(`${fn}; sendWalletCalls`, { postingJournal: journal,
      state: { account: wallet, provider: { request: async () => { const error = new Error("wallet error"); error.code = code; throw error; } } },
      sendTransaction: async () => { sends++; return "hash"; }, waitReceipt: async () => ({ status: "0x1" }) });
    const calls = [{ to: contract, data: "0x" }, { to: contract, data: "0x01" }], protocol = { chain_id_hex: "0x2105" };
    if ([-32601, 4200].includes(code)) { await send(calls, protocol); assert.equal(sends, 2); }
    else { await assert.rejects(send(calls, protocol), /wallet error/); assert.equal(sends, 0); assert.equal(journal.load().phase, "sending"); }
  }
});
test("tracking verification never asks for a new wallet approval", async () => {
  const env = environment("/", [item({ source_status: "submitted", work_state: "in_progress" })]);
  const result = await flow.createClient(env.window).prepareAction({ action: "verify", opportunity_id: item().opportunity_id });
  assert.equal(result.user_confirmation_required, false);
  assert.equal(result.status, "verification_tracking");
  assert.equal(env.requests.filter((request) => request.method === "POST").length, 0);
});
test("unavailable retry storage blocks action preparation before any external write", async () => {
  const env = environment(); env.window.sessionStorage.setItem = () => { throw new Error("quota"); };
  await assert.rejects(flow.createClient(env.window).prepareAction({ action: "solve", opportunity_id: item().opportunity_id }), /retry checkpoint/);
  assert.equal(env.requests.filter((request) => request.method === "POST").length, 0);
});
test("a contribution uses the exact reviewed amount and recorded transaction", async () => {
  const env = await participantFixture("fund");
  await env.click("[data-wallet-connect]"); await env.click("[data-wallet-confirm]");
  assert.equal(env.sent.length, 2); assert.equal(env.observations.length, 1);
  const last = env.sent[1]; assert.equal(last.to, contract);
  assert.equal(BigInt(`0x${last.data.slice(-64)}`), 10000n);
  await env.reload(); await env.click("[data-wallet-confirm]"); assert.equal(env.sent.length, 2);
});
test("reordering identical evidence fields reuses the existing submission review", async () => {
  const env = environment(), original = env.window.fetch; let savedIntent;
  env.window.fetch = async (url, options) => {
    if (url.endsWith(`/action-intents/${intentId}`) && savedIntent) return { ok: true, json: async () => savedIntent };
    const response = await original(url, options), payload = await response.json();
    if (url.endsWith("/action-intents")) savedIntent = payload;
    return { ok: true, json: async () => payload };
  };
  const client = flow.createClient(env.window), input = { action: "complete", opportunity_id: item().opportunity_id, artifact_reference: "https://example.com/result" };
  await client.prepareAction({ ...input, evidence: { checks: { passed: 1, failed: 0 }, result: "ok" } });
  await client.prepareAction({ ...input, evidence: { result: "ok", checks: { failed: 0, passed: 1 } } });
  assert.equal(env.requests.filter((r) => r.method === "POST").length, 1);
});
