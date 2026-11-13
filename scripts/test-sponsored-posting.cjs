"use strict";
const { test } = require("node:test"), assert = require("node:assert/strict"), fs = require("node:fs"), vm = require("node:vm");
const workflow = require("../site/marketplace-workflow.js");
const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
const helper = source.slice(source.indexOf("  async function relaySignedCreation("), source.indexOf("  async function continueSignedBounty("));
const contract = "0x" + "cd".repeat(20), id = "0x" + "ab".repeat(32), hash = "0x" + "ef".repeat(32);
const saved = { create: { creator: "0x" + "11".repeat(20), initial_funding: { amount: 21_300_000, currency: "usdc" } }, signature: "fixture-exact-signature", authorization_valid_before: 2_000_000_000, bounty_contract: contract, bounty_id: id };
function harness(fetcher) {
  const values = new Map(), order = [], bodies = [];
  const win = { sessionStorage: { getItem: key => values.get(key) || null, setItem: (key, value) => values.set(key, value), removeItem: key => values.delete(key) } };
  const journal = workflow.createPostingJournal(win); journal.prepare({ bounty_id: id, predicted_bounty_contract: contract }); journal.checkpoint("signing"); journal.authorizeContinuation("0x" + "12".repeat(32));
  const state = { funded: false, binding: true };
  const context = vm.createContext({
    postingJournal: journal,
    postingSession: { reconcile: async () => ({ funding_confirmed: state.funded }), flush: async options => { assert.equal(options.requireServer, true); order.push("durable-save"); } },
    assertPostingBinding: async () => { if (!state.binding) throw new Error("account changed"); },
    setPaymentStatus: () => {}, signatureParts: signature => { assert.equal(signature, saved.signature); return { r: "exact-r", s: "exact-s", v: 27 }; },
    fetch: async (url, options) => { assert.equal(order.at(-1), "durable-save"); order.push("relay"); bodies.push(JSON.parse(options.body)); return fetcher(url, options, state); },
  });
  vm.runInContext(helper + "\nglobalThis.relay = relaySignedCreation;", context);
  return { relay: () => context.relay("https://api.example", saved), journal, state, order, bodies };
}
test("lost relay response preserves exactly one signed request for an identical retry", async () => {
  let calls = 0;
  const h = harness(async () => { if (++calls === 1) throw new Error("response lost"); return { status: 202, json: async () => ({ schema: "agent-bounties/sponsored-creation-v1", customer_gas_wei: "0", relay: { network: "base-mainnet", transaction: hash, bountyContract: contract } }) }; });
  await assert.rejects(h.relay(), /response lost/);
  assert.equal(h.journal.load().phase, "sending"); assert.equal(h.journal.load().authorizationIssued, true);
  assert.equal(h.journal.load().transactions.length, 0);
  assert.equal(await h.relay(), hash);
  assert.deepEqual(h.bodies[0], h.bodies[1]);
  assert.equal(h.bodies[0].authorization_valid_before, saved.authorization_valid_before);
  assert.deepEqual(h.bodies[0].create, saved.create);
  assert.equal(h.journal.load().phase, "submitted");
  assert.deepEqual(h.journal.load().transactions, [hash]);
});
test("confirmed funding reconciles without asking the relayer again", async () => {
  const h = harness(async () => { throw new Error("unexpected send"); }); h.state.funded = true;
  assert.equal(await h.relay(), null); assert.deepEqual(h.order, []);
});
test("a reply for another bounty cannot be recorded as this bounty's transaction", async () => {
  const h = harness(async () => ({ status: 202, json: async () => ({ schema: "agent-bounties/sponsored-creation-v1", customer_gas_wei: "0", relay: { network: "base-mainnet", transaction: hash, bountyContract: "0x" + "99".repeat(20) } }) }));
  await assert.rejects(h.relay(), /different bounty/); assert.equal(h.journal.load().transactions.length, 0);
});
test("capacity or unavailable sponsorship preserves the signature and never changes payer", async () => {
  for (const status of [429, 503]) {
    const h = harness(async () => ({ status, json: async () => ({}) }));
    await assert.rejects(h.relay(), /signed request is saved/);
    assert.equal(h.journal.load().phase, "sending"); assert.equal(h.journal.load().wallet_method, "hosted_creation_relay");
  }
});
test("an account switch during the reply cannot overwrite the new account's state", async () => {
  const h = harness(async (_url, _options, state) => { state.binding = false; return { status: 202, json: async () => ({ schema: "agent-bounties/sponsored-creation-v1", customer_gas_wei: "0", relay: { network: "base-mainnet", transaction: hash, bountyContract: contract } }) }; });
  await assert.rejects(h.relay(), /account changed/); assert.equal(h.journal.load().transactions.length, 0);
});

test("expired authorization without canonical confirmation never claims nothing was sent", async () => {
  const h = harness(async () => ({ status: 422, json: async () => ({}) }));
  const original = saved.authorization_valid_before;
  try {
    saved.authorization_valid_before = 1;
    await assert.rejects(h.relay(), /expiry alone does not prove an earlier transaction failed/);
    assert.equal(h.journal.load().authorizationIssued, true);
  } finally { saved.authorization_valid_before = original; }
});

test("continuation reconciles an expired request with zero remaining USDC and no fresh capacity", async () => {
  const continuation = source.slice(source.indexOf("  async function continueSignedBounty("), source.indexOf("  async function fundApprovedBounty("));
  const calls = [], statuses = [];
  const prior = { ...saved, authorization_valid_before: 1, legal_acceptance: { durable: true, action: "post_bounty", wallet_address: saved.create.creator, terms_version: "v1", privacy_version: "v1", statement_hash: "h" } };
  const context = vm.createContext({
    postingBusy: false, postingBinding: null,
    state: { approved: true, provider: {}, account: saved.create.creator, draft: {}, balances: { usdc: 0, required: 21.3 }, gasSponsored: false },
    ui: { fundNow: {} },
    window: {
      AgentBountiesPostingSession: { stable: JSON.stringify, envelope: value => value },
      AgentBountiesWorkflow: { createClient: () => ({ load: () => ({}) }) },
      AgentBountiesLegal: { loadPolicy: async () => ({ source: "hosted", supported_actions: ["post_bounty"], terms_version: "v1", privacy_version: "v1", statement_hash: "h" }) },
    },
    postingSession: { refresh: async () => {}, loadContinuation: async () => prior, reconcile: async () => { calls.push("reconcile"); return { funding_confirmed: false }; }, flush: async () => {}, canContinue: () => true },
    assertPostingBinding: async () => {},
    loadProtocol: async () => ({ api_base_url: "https://api.example", factory: "factory", native_usdc: "usdc" }),
    requestJson: async () => ({ bounty_id: id, predicted_bounty_contract: contract }), validateCreationPlan: () => {}, currentRewardSplit: () => ({ total: 21_300_000 }),
    relaySignedCreation: async (_api, request) => { assert.equal(request, prior); calls.push("same-authorization"); return hash; },
    finishPosting: async (_api, _plan, _protocol, transaction) => { assert.equal(transaction, hash); calls.push("confirmation"); },
    setPaymentStatus: message => statuses.push(message), renderFundingGuide: () => {},
  });
  vm.runInContext(continuation + "\nglobalThis.continuePosting = continueSignedBounty;", context);
  await context.continuePosting();
  assert.deepEqual(calls, ["reconcile"]); assert.match(statuses.at(-1), /V1 creation is retired/);
});

test("recovery stays visible instead of requesting a second wallet top-up", () => {
  const nodes = new Map();
  const node = key => {
    if (!nodes.has(key)) nodes.set(key, { open: true, checked: true, dataset: {}, hidden: false, textContent: "", setAttribute() {}, focus() {}, querySelector: node });
    return nodes.get(key);
  };
  const context = vm.createContext({ document: { querySelector: selector => selector.startsWith(".ab-phone-dialog") ? null : node(selector) }, window: { AgentBountiesFundingReadiness: { formatUnits: String } } });
  vm.runInContext(fs.readFileSync(require.resolve("../site/funding-guide.js"), "utf8"), context);
  context.window.AgentBountiesFundingGuide.render({ connected: true, approved: true, recorded: true, continuable: true, conflict: false, gasSponsored: false, balances: { usdc: 0n, required: 21_300_000n }, total: "21.30", solver: "20.30", verifier: "1.00", message: "", tone: "pending" });
  assert.equal(node("#funding-dialog").dataset.fundingView, "review");
  assert.equal(node("[data-fund-now]").disabled, false);
  assert.equal(node("[data-funding-topup] [data-onramp-link]").hidden, true);
  assert.match(node("[data-funding-instruction]").textContent, /Do not pay again/);
});

function revocationHarness(fetcher, walletSignature = "0x" + "12".repeat(65)) {
  const listeners = new Map(), values = new Map(), calls = [], statuses = [];
  const original = { create: { creator: saved.create.creator, creation_nonce: "0x" + "98".repeat(32) } };
  const provider = { request: async request => {
    calls.push(request);
    if (request.method === "eth_accounts") return [original.create.creator];
    if (request.method === "eth_chainId") return "0x2105";
    if (request.method === "eth_signTypedData_v4") return walletSignature;
    throw new Error("unexpected wallet call " + request.method);
  } };
  const context = vm.createContext({
    postingBusy: false, state: { provider, account: original.create.creator }, API: "https://api.example",
    document: { querySelector: selector => ({ addEventListener: (_type, listener) => listeners.set(selector, listener) }) },
    window: { sessionStorage: { getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) } },
    postingSession: { loadContinuation: async () => original },
    loadProtocol: async () => ({ native_usdc: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913" }),
    requestJson: async (url, options) => fetcher(JSON.parse(options.body), url),
    setPaymentStatus: (text, tone) => statuses.push({text, tone}),
  });
  const start = source.indexOf('  document.querySelector("[data-revoke-authorization]")');
  const end = source.indexOf('  document.querySelector("[data-renew-authorization]")', start);
  vm.runInContext(source.slice(start, end), context);
  return { run: (isTrusted = true) => listeners.get("[data-revoke-authorization]")({isTrusted,currentTarget:{disabled:false}}), calls, statuses, original, context };
}
test("revocation persists one exact cancellation before a lost reply and retries without signing or sending ETH", async () => {
  const requests = [];
  const h = revocationHarness(async request => {
    requests.push(request);
    if (requests.length === 1) throw new Error("reply lost");
    return {...request, schema:"agent-bounties/sponsored-usdc-revocation-v1",customer_gas_wei:"0",authorization_cancelled:true};
  });
  await h.run(false); assert.equal(h.calls.length, 0);
  await h.run(); await h.run();
  assert.deepEqual(requests[0], requests[1]);
  assert.equal(h.calls.filter(call => call.method === "eth_signTypedData_v4").length, 1);
  const typed = JSON.parse(h.calls.find(call => call.method === "eth_signTypedData_v4").params[1]);
  assert.equal(typed.primaryType, "CancelAuthorization");
  assert.equal(typed.message.nonce, h.original.create.creation_nonce);
  assert.equal(typed.message.authorizer, h.original.create.creator);
  assert.equal(typed.domain.chainId, 8453);
  assert.match(h.statuses.at(-1).text, /Once it is finalized/);
  assert.equal(h.context.postingBusy, false);
});
test("revocation rejects a foreign response and never announces the original request retired", async () => {
  const h = revocationHarness(async request => ({...request,nonce:"0x"+"ff".repeat(32),schema:"agent-bounties/sponsored-usdc-revocation-v1",customer_gas_wei:"0",authorization_cancelled:true}));
  await h.run();
  assert.equal(h.statuses.at(-1).tone,"error");
  assert.match(h.statuses.at(-1).text,/did not match/);
});

test("legacy creation preserves exact 65-byte signatures and refuses truncation or non-hex payloads", () => {
  const context = vm.createContext({});
  const start = source.indexOf("  function signatureParts(");
  const end = source.indexOf("  async function sendTransaction(", start);
  vm.runInContext(source.slice(start, end) + "\nglobalThis.parts = signatureParts;", context);
  for (const v of ["1b", "1c", "00", "01"]) {
    const parts = context.parts("0x" + "12".repeat(32) + "34".repeat(32) + v);
    assert.equal(parts.r, "0x" + "12".repeat(32));
    assert.equal(parts.s, "0x" + "34".repeat(32));
    assert.equal(parts.v, parseInt(v, 16));
  }
  for (const invalid of ["0x" + "12".repeat(66), "0x" + "gg".repeat(64) + "1b", "0x" + "12".repeat(64) + "02", "0x1234"]) {
    assert.throws(() => context.parts(invalid), /exact 65-byte.*preserve this request/);
  }
});

test("revocation keeps wider contract-wallet signatures exact and enforces a size bound",async()=>{
 const signature="0x7f"+"12".repeat(65);const requests=[];
 const fetcher=async request=>{requests.push(request);return {...request,schema:"agent-bounties/sponsored-usdc-revocation-v1",customer_gas_wei:"0",authorization_cancelled:true};};
 const env=revocationHarness(fetcher,signature);await env.run();await env.run();assert.equal(requests[0].signature,signature);assert.deepEqual(requests[0],requests[1]);
 const invalid=revocationHarness(fetcher,"0x"+"12".repeat(4097));await invalid.run();assert.equal(requests.length,2);assert.match(invalid.statuses.at(-1).text,/bounded revocation/);
});

function policyHarness() {
  const { webcrypto, createHash } = require("node:crypto");
  const statement = "The agreed test policy";
  const initial = { schema_version: "agent-bounties/legal-policy-v1", terms_version: "v1", privacy_version: "v1", statement,
    statement_hash: "sha256:" + createHash("sha256").update(statement).digest("hex"), supported_actions: ["post_bounty"] };
  const state = { policy: initial, fault: null, blockNext: null }, calls = [], win = {};
  vm.runInNewContext(fs.readFileSync(require.resolve("../site/legal-consent.js"), "utf8"), {
    window: win, crypto: webcrypto, TextEncoder,
    document: { readyState: "loading", addEventListener() {} },
    fetch: async (url, options) => {
      calls.push(url); assert.equal(options.cache, "no-store");
      if (url === "protocol.json") return { ok: true, json: async () => ({ api_base_url: "https://api.example" }) };
      assert.equal(url, "https://api.example/v1/legal/policy");
      const policy = structuredClone(state.policy), fault = state.fault, block = state.blockNext; state.blockNext = null;
      if (block) await block;
      return { ok: fault !== "http", status: fault === "http" ? 503 : 200,
        json: async () => fault === "integrity" ? { ...policy, statement_hash: "sha256:" + "00".repeat(32) } : policy };
    },
  });
  return { legal: win.AgentBountiesLegal, state, calls, initial };
}

test("fresh legal reads see policy changes without replacing the display cache", async () => {
  const h = policyHarness(); const display = await h.legal.loadPolicy();
  assert.equal((await h.legal.loadPolicy()).terms_version, "v1"); assert.equal(h.calls.length, 2);
  h.state.policy = { ...h.initial, terms_version: "v2" };
  assert.equal((await h.legal.loadPolicy({ fresh: true })).terms_version, "v2");
  assert.equal(await h.legal.loadPolicy(), display); assert.equal(h.calls.length, 4);
});

test("fresh HTTP and integrity failures never return cached hosted success", async () => {
  const h = policyHarness(), cached = await h.legal.loadPolicy();
  for (const fault of ["http", "integrity"]) {
    h.state.fault = fault;
    assert.equal((await h.legal.loadPolicy({ fresh: true })).source, "bundled");
    assert.equal(await h.legal.loadPolicy(), cached);
  }
  h.state.fault = null;
  assert.equal((await h.legal.loadPolicy({ fresh: true })).source, "hosted");
});

test("concurrent display and fresh legal reads cannot replace each other's policy", async () => {
  const h = policyHarness(); let release;
  h.state.blockNext = new Promise(resolve => { release = resolve; });
  const display = h.legal.loadPolicy(); await new Promise(setImmediate);
  h.state.policy = { ...h.initial, terms_version: "v2" };
  assert.equal((await h.legal.loadPolicy({ fresh: true })).terms_version, "v2");
  release(); assert.equal((await display).terms_version, "v1");
  assert.equal((await h.legal.loadPolicy()).terms_version, "v1"); assert.equal(h.calls.length, 4);
});

async function legalContinuationHarness() {
  const h = policyHarness(); await h.legal.loadPolicy();
  const prior = { ...structuredClone(saved), legal_acceptance: { durable: true, action: "post_bounty", wallet_address: saved.create.creator,
    terms_version: h.initial.terms_version, privacy_version: h.initial.privacy_version, statement_hash: h.initial.statement_hash } };
  const before = JSON.stringify(prior), calls = [], statuses = [], state = { confirmed: false };
  const start = source.indexOf("  async function continueSignedBounty("), end = source.indexOf("  async function fundApprovedBounty(");
  assert.ok(start >= 0 && end > start);
  const context = vm.createContext({
    postingBusy: false, postingBinding: null, state: { approved: true, provider: {}, account: prior.create.creator }, ui: { fundNow: {} },
    window: { AgentBountiesLegal: h.legal, AgentBountiesPostingSession: { stable: JSON.stringify, envelope: x => x }, AgentBountiesWorkflow: { createClient: () => ({ load: () => ({}) }) } },
    postingSession: { refresh: async () => {}, loadContinuation: async () => prior, reconcile: async () => { calls.push("reconcile"); return { funding_confirmed: state.confirmed }; }, flush: async () => {}, canContinue: () => true },
    assertPostingBinding: async () => {}, loadProtocol: async () => ({ api_base_url: "https://api.example", factory: "factory", native_usdc: "usdc" }),
    requestJson: async () => { calls.push("plan"); return { bounty_id: id, predicted_bounty_contract: contract }; }, validateCreationPlan() {}, currentRewardSplit: () => ({ total: 21_300_000 }),
    relaySignedCreation: async (_api, request) => { assert.equal(request, prior); calls.push("same-authorization"); return hash; },
    finishPosting: async () => calls.push("confirmed"), setPaymentStatus: message => statuses.push(message), renderFundingGuide() {},
  });
  vm.runInContext(source.slice(start, end) + "\nglobalThis.continuePosting = continueSignedBounty;", context);
  return { ...h, prior, before, calls, policyCalls: h.calls, statuses, state: h.state, canonical: state, run: () => context.continuePosting() };
}

for (const fault of ["changed", "http", "integrity"]) test(`saved continuation blocks fresh ${fault} policy preserves the retired authorization without replay after policy recovery`, async () => {
  const h = await legalContinuationHarness();
  if (fault === "changed") h.state.policy = { ...h.initial, terms_version: "v2" }; else h.state.fault = fault;
  await h.run(); assert.deepEqual(h.calls, ["reconcile"]); assert.match(h.statuses.at(-1), /V1 creation is retired/);
  assert.equal(JSON.stringify(h.prior), h.before);
  h.state.policy = h.initial; h.state.fault = null; await h.run();
  assert.deepEqual(h.calls, ["reconcile", "reconcile"]);
  assert.equal(JSON.stringify(h.prior), h.before);
});

test("confirmed continuation completes before policy reads even during an outage", async () => {
  const h = await legalContinuationHarness(); h.canonical.confirmed = true; h.state.fault = "http";
  const reads = h.calls.length, policyReads = h.policyCalls.length;
  await h.run(); assert.deepEqual(h.calls.slice(reads), ["reconcile", "confirmed"]);
  assert.equal(h.statuses.length, 0); assert.equal(h.policyCalls.length, policyReads); assert.equal(JSON.stringify(h.prior), h.before);
});
