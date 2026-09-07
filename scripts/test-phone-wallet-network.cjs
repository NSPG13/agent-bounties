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
const { createPostingJournal } = require("../site/marketplace-workflow.js");
const address = "0x" + "12".repeat(20), other = "0x" + "34".repeat(20);
const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
const start = source.indexOf("  async function fundApprovedBounty()"), end = source.indexOf("  function configureSpeech", start);
assert.ok(start >= 0 && end > start);
const fundingFunction = source.slice(start, end) + "; fundApprovedBounty";

async function fixture({ adapted = true, change = null, uncertain = false } = {}) {
  const storage = new Map(), signing = [], statuses = [], network = [];
  const store = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  store.setItem("agent-bounties-phone-connected-v1", "ab-phone-12345678-1234-1234-1234-123456789012");
  const namespace = { chains: ["eip155:8453"], accounts: [`eip155:8453:${address}`], methods: ["eth_signTypedData_v4"], events: [], rpcMap: { "eip155:8453": "http://127.0.0.1:1" } };
  const universal = new UniversalProvider({ logger: "silent", disableProviderPing: true });
  universal.client = {
    core: { projectId: "0".repeat(32), storage: { getItem: async key => storage.get(key), setItem: async (key, value) => storage.set(key, value) } },
    request: async request => {
      signing.push(request);
      if (uncertain) throw new Error("Fixture wallet response was lost");
      throw Object.assign(new Error("Fixture user rejected the wallet request"), { code: 4001 });
    },
  };
  universal.namespaces = { eip155: namespace };
  universal.session = { topic: "synthetic-session", expiry: Date.now() / 1000 + 3600, namespaces: { eip155: namespace } };
  universal.createProviders();
  // Make accidental HTTP reads fail immediately, even if a fixture regresses.
  universal.rpcProviders.eip155.httpProviders[8453].request = async request => { network.push(request); throw new Error("Unexpected RPC call in offline fixture"); };
  const sdk = new EthereumProvider(); sdk.signer = universal; sdk.chainId = 8453; sdk.accounts = [address];
  const win = {
    document: null, crypto: webcrypto, localStorage: store, sessionStorage: store, location: new URL("https://agentbounties.app/post.html"),
    agentBountiesPhoneWalletConfig: { projectId: "0".repeat(32) },
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    addEventListener() {}, dispatchEvent() {}, setTimeout, clearTimeout,
    AgentBountiesLegal: { requireAcceptance: async () => ({ durable: true }) },
  };
  const phone = createPhoneWallet(win, { loadVendor: async () => ({ createProvider: async () => sdk }) });
  await phone.restore();
  const provider = adapted ? phone.provider : sdk;
  const journal = createPostingJournal(win);
  const state = { approved: true, provider, account: address, balances: { usdc: 1000000n, required: 1000000n, eth: 1n }, draft: { title: "Synthetic review" }, fundingUsdc: 1 };
  const authorization = { domain: { chainId: 8453 }, primaryType: "ReceiveWithAuthorization", message: { value: "1000000" } };
  const plan = { bounty_id: "0x" + "56".repeat(32), predicted_bounty_contract: "0x" + "78".repeat(20), eip3009_authorization: authorization };
  const run = vm.runInNewContext(fundingFunction, {
    postingBusy: false, postingJournal: journal, state, window: win, ui: { form: { querySelectorAll: () => [] }, fundNow: {} },
    track() {}, setPaymentStatus: value => statuses.push(value), refreshWalletReadiness: async () => {},
    loadProtocol: async () => ({ api_base_url: "https://api.agentbounties.app", factory: "0x" + "90".repeat(20) }),
    currentRewardSplit: () => ({ solver: "900000", verifier: "100000", total: "1000000" }),
    contractTerms: () => ({}), termsDocument: () => ({}), createPayload: () => ({}), validateCreationPlan() {}, formatUsdc: String,
    requestJson: async url => {
      if (url.endsWith("/terms")) return {};
      assert.ok(url.endsWith("/creation-plan"));
      if (change === "account") { sdk.accounts = [other]; universal.session.namespaces.eip155.accounts = [`eip155:8453:${other}`]; }
      if (change === "chain") { sdk.chainId = 1; universal.rpcProviders.eip155.chainId = 1; }
      return plan;
    },
    isContractAccount: async () => false,
    sendTransaction: async () => { throw new Error("No transaction may follow the rejected fixture signature"); },
  });
  return { sdk, provider, run, signing, statuses, network, journal, authorization };
}

test("the real pinned SDK reproduces the false network-change stop before adaptation", async () => {
  const env = await fixture({ adapted: false });
  assert.equal(await env.sdk.request({ method: "eth_chainId" }), 8453);
  await env.run();
  assert.match(env.statuses.at(-1), /wallet or network changed/);
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
  assert.match(env.statuses.at(-1), /Fixture user rejected/);
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
