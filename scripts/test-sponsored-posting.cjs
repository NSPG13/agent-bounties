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
  const calls = [];
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
    setPaymentStatus: message => { throw new Error(message); }, renderFundingGuide: () => {},
  });
  vm.runInContext(continuation + "\nglobalThis.continuePosting = continueSignedBounty;", context);
  await context.continuePosting();
  assert.deepEqual(calls, ["reconcile", "same-authorization", "confirmation"]);
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
  assert.match(node("[data-funding-instruction]").textContent, /Do not add money/);
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
