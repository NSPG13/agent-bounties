"use strict";
// Use the pinned SDK's real Ethereum/Universal provider chain-ID behavior with
// a synthetic session and inert signing client. No live relay, RPC or wallet.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const { webcrypto } = require("node:crypto");
const { UniversalProvider } = require("../tools/phone-wallet/node_modules/@walletconnect/universal-provider");
const { EthereumProvider } = require("../tools/phone-wallet/node_modules/@walletconnect/ethereum-provider");
const createPhoneWallet = require("../site/phone-wallet.js");
const postingSessionLibrary = require("../site/posting-session.js");
const { createPostingJournal } = require("../site/marketplace-workflow.js");
const address = "0x" + "12".repeat(20), other = "0x" + "34".repeat(20);
const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
const start = source.indexOf("  async function fundApprovedBounty()"), end = source.indexOf("  function configureSpeech", start);
assert.ok(start >= 0 && end > start);
const fundingFunction = source.slice(start, end) + "; fundApprovedBounty";
const batchFunction = source.slice(source.indexOf("  async function sendWalletCalls("), source.indexOf("  function contractTerms("));
const walletFunctions = source.slice(source.indexOf("  function signatureParts("), source.indexOf("  async function sendWalletCalls("));
const bindingFunctions = source.slice(source.indexOf("  async function prepareWalletRequest("), source.indexOf("  async function watchUsdcAsset("));
const atomicError = { code: -32602, message: "Invalid params\n\n0 > atomicRequired - Expected a value of type `boolean`, but received: `undefined`" };
const rejection = { code: 4001, message: "MetaMask Tx Signature: User denied transaction signature." };
const syntheticSignature = "0x" + "12".repeat(64) + "1b", transactionHash = "0x" + "ab".repeat(32);

async function fixture({ adapted = true, change = null, uncertain = false, batch = false, legacy = false, pending = false,
  accountCode = batch ? "0x6001600055" : "0x", approveAuthorization = false, wrapped = false, rejectTransaction = false,
  mutateAt = null, mutation = "draft", syncFailure = false } = {}) {
  const storage = new Map(), signing = [], statuses = [], network = [], checkpoints = [], publications = [];
  const store = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  store.setItem("agent-bounties-phone-connected-v1", "ab-phone-12345678-1234-1234-1234-123456789012");
  const namespace = { chains: ["eip155:8453"], accounts: [`eip155:8453:${address}`], methods: ["eth_signTypedData_v4", "wallet_sendCalls", "eth_sendTransaction"], events: [], rpcMap: { "eip155:8453": "http://127.0.0.1:1" } };
  const universal = new UniversalProvider({ logger: "silent", disableProviderPing: true });
  universal.client = {
    core: { projectId: "0".repeat(32), storage: { getItem: async key => storage.get(key), setItem: async (key, value) => storage.set(key, value) } },
    request: async request => {
      signing.push(request);
      if (uncertain) throw new Error("Fixture wallet response was lost");
      if (request.request.method === "eth_sendTransaction") {
        if (rejectTransaction) throw new Error(JSON.stringify(rejection));
        return transactionHash;
      }
      if (wrapped) throw new Error(JSON.stringify(rejection));
      if (batch) {
        assert.equal(request.request.method, "wallet_sendCalls");
        if (typeof request.request.params[0].atomicRequired !== "boolean") throw Object.assign(new Error(atomicError.message), atomicError);
        return { id: "synthetic-batch-id" };
      }
      if (approveAuthorization) return syntheticSignature;
      throw Object.assign(new Error("Fixture user rejected the wallet request"), { code: 4001 });
    },
  };
  universal.namespaces = { eip155: namespace };
  universal.session = { topic: "synthetic-session", expiry: Date.now() / 1000 + 3600, namespaces: { eip155: namespace } };
  universal.createProviders();
  // Make accidental HTTP reads fail immediately, even if a fixture regresses.
  universal.rpcProviders.eip155.httpProviders[8453].request = async request => {
    if (request.method === "eth_getCode") { assert.deepEqual(Array.from(request.params), [address, "latest"]); return accountCode; }
    if (request.method === "eth_getTransactionReceipt") { assert.equal(request.params[0], transactionHash); return { status: "0x1" }; }
    network.push(request); throw new Error("Unexpected RPC call in offline fixture");
  };
  const sdk = new EthereumProvider(); sdk.signer = universal; sdk.chainId = 8453; sdk.accounts = [address];
  const win = {
    document: null, crypto: webcrypto, localStorage: store, sessionStorage: store, location: new URL("https://agentbounties.app/post.html"),
    agentBountiesPhoneWalletConfig: { projectId: "0".repeat(32) },
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    addEventListener() {}, dispatchEvent() {}, setTimeout, clearTimeout,
    AgentBountiesLegal: { requireAcceptance: async () => { if (mutateAt === "legal") mutate(); return { durable: true }; } },
  };
  win.location.assign = () => {};
  const phone = createPhoneWallet(win, { loadVendor: async () => ({ createProvider: async () => sdk }) });
  await phone.restore();
  const provider = adapted ? phone.provider : sdk;
  const journal = createPostingJournal(win);
  const state = { approved: true, provider, account: address, balances: { usdc: 1000000n, required: 1000000n, eth: 1n }, draft: { title: "Synthetic review" }, fundingUsdc: 1 };
  let journey = { id: "12345678-1234-1234-1234-123456789012", role: "post", draft: state.draft };
  function mutate() {
    if (mutation === "draft") journey = { ...journey, draft: { title: "Other device revised this draft" } };
    if (mutation === "account") state.account = other;
    if (mutation === "provider") state.provider = { request: async () => { throw new Error("Replacement provider must not receive a request"); } };
    if (mutation === "approval") state.approved = false;
  }
  win.AgentBountiesPostingSession = postingSessionLibrary;
  win.AgentBountiesWorkflow = { createClient: () => ({ load: () => journey }) };
  win.AgentBountiesFundingReadiness = { estimateFees: async () => { if (mutateAt === "fees") mutate(); return { estimatedTotalWei: 1n }; } };
  const authorization = { domain: { chainId: 8453 }, primaryType: "ReceiveWithAuthorization", message: {
    value: "1000000", to: "0x" + "90".repeat(20), validBefore: String(Math.floor(Date.now() / 1000) + 3600),
  } };
  const calls = [{ to: other, data: "0x010203" }, { to: address, data: "0x040506" }];
  const plan = { bounty_id: "0x" + "56".repeat(32), predicted_bounty_contract: "0x" + "78".repeat(20), eip3009_authorization: authorization, wallet_calls: calls };
  const postingSession = {
    refresh: async () => { if (mutateAt === "refresh") mutate(); },
    flush: async (options = {}) => {
      checkpoints.push({ required: options.requireServer === true, phase: journal.load()?.phase });
      if (mutateAt === "sync" && options.requireServer) mutate();
      if (syncFailure && options.requireServer) throw new Error("Account sync unavailable; nothing dispatched");
    },
    approved: async () => state.approved,
    reconcile: async () => ({ creation_confirmed: !pending, funding_confirmed: !pending, claimable: !pending, public_inventory_verified: !pending }),
  };
  const run = vm.runInNewContext(bindingFunctions + walletFunctions + (legacy ? batchFunction.replace("atomicRequired:false,", "") : batchFunction) + fundingFunction, {
    postingBusy: false, postingBinding: null, postingJournal: journal, postingSession, state, window: win, ui: { form: { querySelectorAll: () => [] }, fundNow: {}, badge: {} }, document: { querySelector: () => null },
    track() {}, setPaymentStatus: value => statuses.push(value), refreshWalletReadiness: async () => {},
    updatePostingCost() {}, updatePostingTracker() {},
    loadProtocol: async () => ({ api_base_url: "https://api.agentbounties.app", factory: "0x" + "90".repeat(20), chain_id_hex: "0x2105" }),
    currentRewardSplit: () => ({ solver: "900000", verifier: "100000", total: "1000000" }),
    contractTerms: () => ({}), termsDocument: () => ({}), createPayload: () => ({}), validateCreationPlan() {}, formatUsdc: String,
    requestJson: async (url, options) => {
      publications.push(url);
      if (url.endsWith("/terms")) return {};
      if (url.endsWith("/authorized-creation-plan")) {
        assert.equal(JSON.parse(options.body).signature.v, 27);
        return { relay_transaction: { to: "0x" + "90".repeat(20), data: "0xaabbcc", value_wei: 0 } };
      }
      assert.ok(url.endsWith("/creation-plan"));
      if (change === "account") { sdk.accounts = [other]; universal.session.namespaces.eip155.accounts = [`eip155:8453:${other}`]; }
      if (change === "chain") { sdk.chainId = 1; universal.rpcProviders.eip155.chainId = 1; }
      return plan;
    },
    pollCreation: async () => pending ? null : [{ kind: "canonical_bounty_created" }, { kind: "funding_added" }, { kind: "bounty_became_claimable" }],
    fetchFeedItem: async () => ({ terms_valid: true, verification_ready: true }),
  });
  return { sdk, provider, run, signing, statuses, network, journal, authorization, calls, checkpoints, publications };
}

test("the old batch reproduces the wallet's exact atomicRequired schema rejection through the pinned SDK", async () => {
  const env = await fixture({ batch: true, legacy: true }); await env.run();
  assert.equal(env.signing.length, 1); assert.equal(env.statuses.at(-1), atomicError.message);
  assert.equal(env.journal.load().wallet_error.code, -32602);
  assert.equal(env.journal.load().transactions.length, 0);
  await env.run(); assert.equal(env.signing.length, 1, "no automatic replay or transaction fallback");
});

for (const pending of [false, true]) test(`a smart-wallet batch passes the real SDK with canonical funding ${pending ? "pending" : "confirmed"}`, async () => {
  const env = await fixture({ batch: true, pending }); await env.run();
  assert.equal(env.signing.length, 1); assert.equal(env.network.length, 0);
  const wire = env.signing[0]; assert.equal(wire.chainId, "eip155:8453");
  assert.deepEqual(JSON.parse(JSON.stringify(wire.request)), { method: "wallet_sendCalls", params: [{ version: "2.0.0", atomicRequired: false,
    chainId: "0x2105", from: address, calls: env.calls.map(call => ({ ...call, value: "0x0" })) }] });
  assert.equal(env.journal.load().transactions[0].id, "synthetic-batch-id");
  assert.equal(env.journal.load().phase, pending ? "batch_submitted" : "funding_confirmed");
  await env.run(); assert.equal(env.signing.length, 1, "neither pending nor confirmed funding is repeated");
});

test("a lost smart-wallet batch reply stays recorded and cannot dispatch a second request", async () => {
  const env = await fixture({ batch: true, uncertain: true }); await env.run();
  assert.match(env.statuses.at(-1), /response was lost/); assert.equal(env.journal.load().phase, "sending");
  assert.equal(env.journal.load().wallet_error, undefined);
  await env.run(); assert.equal(env.signing.length, 1); assert.equal(env.network.length, 0);
});

for (const accountCode of ["0x", "0xef0100" + "34".repeat(20)]) {
  for (const pending of [false, true]) test(`EOA authorization completes without a batch for ${accountCode === "0x" ? "ordinary" : "delegated"} account; canonical pending=${pending}`, async () => {
    const env = await fixture({ accountCode, approveAuthorization: true, pending }); await env.run();
    assert.deepEqual(env.signing.map(call => call.request.method), ["eth_signTypedData_v4", "eth_sendTransaction"]);
    assert.deepEqual(Array.from(env.signing[0].request.params), [address, JSON.stringify(env.authorization)]);
    assert.deepEqual(JSON.parse(JSON.stringify(env.signing[1].request.params)), [{ from: address, to: "0x" + "90".repeat(20), data: "0xaabbcc", value: "0x0" }]);
    assert.equal(env.journal.load().authorizationIssued, true); assert.equal(env.journal.load().transactions[0], transactionHash);
    assert.equal(env.journal.load().phase, pending ? "submitted" : "funding_confirmed");
    await env.run(); assert.equal(env.signing.length, 2); assert.equal(env.network.length, 0);
  });
}

test("wrapped MetaMask rejection ends one request and reopens preparation without automatic retries", async () => {
  for (const batch of [false, true]) {
    const env = await fixture({ batch, wrapped: true }); await env.run();
    assert.equal(env.signing.length, 1); assert.equal(env.journal.load(), null);
    assert.match(env.statuses.at(-1), /wallet did not approve/); assert.equal(env.network.length, 0);
  }
});

test("a wrapped rejection after a USDC signature preserves the live authorization and prevents a second signing request", async () => {
  const env = await fixture({ accountCode: "0xef0100" + "34".repeat(20), approveAuthorization: true, rejectTransaction: true }); await env.run();
  assert.equal(env.signing.length, 2); assert.equal(env.journal.load().authorizationIssued, true);
  assert.equal(env.journal.load().phase, "sending"); assert.equal(env.journal.load().wallet_method, "eth_sendTransaction");
  await env.run(); assert.equal(env.signing.length, 2);
});

test("invalid account code cannot request a signature and other contracts cannot borrow EOA authorization", async () => {
  for (const accountCode of [null, undefined, true, "", "0xnothex", "0xef010"]) {
    const env = await fixture({ accountCode: accountCode === undefined ? {} : accountCode }); await env.run();
    assert.match(env.statuses.at(-1), /account type could not be checked/); assert.equal(env.signing.length, 0);
  }
  for (const accountCode of ["0xef0100", "0xef0100" + "34".repeat(21), "0xef0101" + "34".repeat(20), "0x6001600055"]) {
    const env = await fixture({ batch: true, accountCode }); await env.run();
    assert.equal(env.signing[0].request.method, "wallet_sendCalls");
  }
});

test("the real pinned SDK reproduces the false network-change stop before adaptation", async () => {
  const env = await fixture({ adapted: false });
  assert.equal(await env.sdk.request({ method: "eth_chainId" }), 8453);
  await env.run();
  assert.match(env.statuses.at(-1), /network changed/);
  assert.equal(env.signing.length, 0); assert.equal(env.journal.load(), null);
});

test("the unchanged posting guard reaches one signature request through the adapted SDK", async () => {
  const env = await fixture();
  assert.equal(await env.provider.request({ method: "eth_chainId" }), "0x2105");
  await env.run();
  assert.equal(env.signing.length, 1);
  assert.equal(env.signing[0].chainId, "eip155:8453");
  assert.equal(env.signing[0].request.method, "eth_signTypedData_v4");
  assert.deepEqual(Array.from(env.signing[0].request.params), [address, JSON.stringify(env.authorization)]);
  assert.match(env.statuses.at(-1), /wallet did not approve/);
  assert.equal(env.network.length, 0);
  assert.equal(env.journal.load(), null, "An explicit rejection authorizes no payment and leaves the review available");
});

test("an uncertain signature reply retains the existing journal and cannot dispatch again", async () => {
  const env = await fixture({ uncertain: true }); await env.run();
  assert.match(env.statuses.at(-1), /Fixture wallet response was lost/);
  assert.ok(env.journal.load());
  await env.run(); assert.equal(env.signing.length, 1);
});

for (const change of ["account", "chain"]) {
  test(`a real ${change} change still stops the posting flow before any signature`, async () => {
    const env = await fixture({ change }); await env.run();
    assert.match(env.statuses.at(-1), /wallet or network changed/);
    assert.equal(env.signing.length, 0); assert.equal(env.network.length, 0); assert.equal(env.journal.load(), null);
  });
}

test("a refreshed draft cannot borrow the previous device's approval or publish its old plan", async () => {
  const env = await fixture({ mutateAt: "refresh", mutation: "draft", approveAuthorization: true });
  await env.run();
  assert.match(env.statuses.at(-1), /approved draft or wallet changed/);
  assert.equal(env.publications.length, 0); assert.equal(env.signing.length, 0); assert.equal(env.journal.load(), null);
});

test("a change during legal review cannot publish terms under stale approval", async () => {
  const env = await fixture({ mutateAt: "legal", mutation: "draft", approveAuthorization: true }); await env.run();
  assert.match(env.statuses.at(-1), /approved draft or wallet changed/);
  assert.equal(env.publications.length, 0); assert.equal(env.signing.length, 0); assert.equal(env.journal.load(), null);
});

for (const mutation of ["draft", "account", "provider", "approval"]) {
  test(`changing ${mutation} during fee preparation stops dispatch and preserves the recorded attempt`, async () => {
    const env = await fixture({ batch: true, mutateAt: "fees", mutation }); await env.run();
    assert.match(env.statuses.at(-1), /approved draft or wallet changed/);
    assert.equal(env.signing.length, 0); assert.ok(env.journal.load());
    await env.run(); assert.equal(env.signing.length, 0, "the pending operation cannot be replayed");
  });
}

test("a changed approval after durable sync cannot reach the USDC signature", async () => {
  const env = await fixture({ approveAuthorization: true, mutateAt: "sync", mutation: "approval" }); await env.run();
  assert.equal(env.signing.length, 0); assert.match(env.statuses.at(-1), /approved draft or wallet changed/);
  assert.ok(env.checkpoints.some(entry => entry.required && entry.phase === "signing"));
});

test("failed required account sync blocks every wallet request", async () => {
  for (const batch of [false, true]) {
    const env = await fixture({ batch, approveAuthorization: true, syncFailure: true }); await env.run();
    assert.equal(env.signing.length, 0); assert.match(env.statuses.at(-1), /sync unavailable/);
    await env.run(); assert.equal(env.signing.length, 0);
  }
});

test("a wallet change after a USDC authorization stops the transaction and retains that authorization", async () => {
  const env = await fixture({ approveAuthorization: true, mutateAt: "fees", mutation: "account" }); await env.run();
  assert.deepEqual(env.signing.map(entry => entry.request.method), ["eth_signTypedData_v4"]);
  assert.equal(env.journal.load().authorizationIssued, true);
  await env.run(); assert.equal(env.signing.length, 1);
});

test("revoking approval while account reads await blocks the final dispatch boundary", async () => {
  const fn = source.slice(source.indexOf("  async function assertPostingBinding("), source.indexOf("  async function watchUsdcAsset("));
  let resolveAccounts;
  const provider = { request: request => request.method === "eth_accounts" ? new Promise(resolve => { resolveAccounts = resolve; }) : Promise.resolve("0x2105") };
  const state = { provider, account: address, draft: { title: "Reviewed draft" }, approved: true };
  const journey = { id: "12345678-1234-1234-1234-123456789012", role: "post", draft: state.draft };
  const binding = { provider, account: address, draft: state.draft, envelope: postingSessionLibrary.stable(postingSessionLibrary.envelope(journey)) };
  const guard = vm.runInNewContext(`${fn}; assertPostingBinding`, { postingBinding: binding, state, postingSession: { approved: async () => true },
    window: { AgentBountiesPostingSession: postingSessionLibrary, AgentBountiesWorkflow: { createClient: () => ({ load: () => journey }) } } });
  const pending = guard(); await new Promise(setImmediate); state.approved = false; resolveAccounts([address]);
  await assert.rejects(pending, /changed/);
});
