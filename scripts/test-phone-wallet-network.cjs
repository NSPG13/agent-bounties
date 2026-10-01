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
const fundingReadinessWindow = {};
vm.runInNewContext(fs.readFileSync(require.resolve("../site/funding-readiness.js"), "utf8"), { window: fundingReadinessWindow });
const { validateFundingAuthorization } = fundingReadinessWindow.AgentBountiesFundingReadiness;
const address = "0x" + "12".repeat(20), other = "0x" + "34".repeat(20);
const nativeUsdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
const start = source.indexOf("  async function fundApprovedBounty()"), end = source.indexOf("  function showAuthorizationRecovery", start);
assert.ok(start >= 0 && end > start);
const fundingFunction = source.slice(start, end) + "; fundApprovedBounty";
const batchFunction = source.slice(source.indexOf("  async function sendWalletCalls("), source.indexOf("  function contractTerms("));
const walletFunctions = source.slice(source.indexOf("  function signatureParts("), source.indexOf("  async function sendWalletCalls("));
const bindingFunctions = source.slice(source.indexOf("  async function prepareWalletRequest("), source.indexOf("  async function watchUsdcAsset("));
const legalFunctions = source.slice(source.indexOf("  const LEGAL_RECEIPT_KEY"), source.indexOf("  async function refreshWalletReadiness("));
const finishFunction = source.slice(source.indexOf("  async function finishPosting("), source.indexOf("  async function fundApprovedBounty("));
const atomicError = { code: -32602, message: "Invalid params\n\n0 > atomicRequired - Expected a value of type `boolean`, but received: `undefined`" };
const rejection = { code: 4001, message: "MetaMask Tx Signature: User denied transaction signature." };
const syntheticSignature = "0x" + "12".repeat(64) + "1b", transactionHash = "0x" + "ab".repeat(32);

async function fixture({ adapted = true, change = null, uncertain = false, batch = false, legacy = false, pending = false,
  accountCode = batch ? "0x6001600055" : "0x", approveAuthorization = false, wrapped = false, rejectTransaction = false,
  mutateAt = null, mutation = "draft", syncFailure = false, authorizedMismatch = null, batchReply = { id: "synthetic-batch-id" }, authorizationTamper = null, relayFault = null, gasSponsored = true, walletSignature = syntheticSignature } = {}) {
  const storage = new Map(), signing = [], statuses = [], network = [], checkpoints = [], publications = [], relays = [];
  let saved = null, broadcast = false, faulted = false, synchronized = false;
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
        return batchReply;
      }
      if (approveAuthorization) return walletSignature;
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
    AgentBountiesLegal: {
      loadPolicy: async () => ({ source: "hosted", supported_actions: ["post_bounty"], terms_version: "test-terms", privacy_version: "test-privacy", statement_hash: "test-statement" }),
      requireAcceptance: async () => { if (mutateAt === "legal") mutate(); return { durable: true, action: "post_bounty", wallet_address: address, terms_version: "test-terms", privacy_version: "test-privacy", statement_hash: "test-statement" }; },
    },
  };
  win.location.assign = () => {};
  const phone = createPhoneWallet(win, { loadVendor: async () => ({ createProvider: async () => sdk }) });
  await phone.restore();
  const provider = adapted ? phone.provider : sdk;
  const journal = createPostingJournal(win);
  const state = { approved: true, provider, account: address, gasSponsored, balances: { usdc: 1000000n, required: 1000000n, eth: 0n }, draft: { title: "Synthetic review" }, fundingUsdc: 1 };
  let journey = { id: "12345678-1234-1234-1234-123456789012", role: "post", draft: state.draft };
  function mutate() {
    if (mutation === "draft") journey = { ...journey, draft: { title: "Other device revised this draft" } };
    if (mutation === "account") state.account = other;
    if (mutation === "provider") state.provider = { request: async () => { throw new Error("Replacement provider must not receive a request"); } };
    if (mutation === "approval") state.approved = false;
  }
  win.AgentBountiesPostingSession = postingSessionLibrary;
  win.AgentBountiesWorkflow = { createClient: () => ({ load: () => journey }) };
  // These deliberately synthetic calls exercise SDK dispatch and recovery.
  // Real calldata descriptions/amount decoding have their own readiness tests.
  win.AgentBountiesFundingReadiness = {
    describeWalletCalls: () => ({ summary: "Synthetic exact wallet request review", transferUsdcUnits: 1000000n }),
    estimateFees: async () => { if (mutateAt === "fees") mutate(); return { estimatedTotalWei: 1n }; },
    validateFundingAuthorization,
  };
  const creationNonce = "0x" + "ef".repeat(32);
  const fundingDeadline = Math.floor(Date.now() / 1000) + (authorizationTamper === "expired" ? -1 : 3600);
  const authorization = {
    types: {
      EIP712Domain: [{ name: "name", type: "string" }, { name: "version", type: "string" }, { name: "chainId", type: "uint256" }, { name: "verifyingContract", type: "address" }],
      TransferWithAuthorization: [{ name: "from", type: "address" }, { name: "to", type: "address" }, { name: "value", type: "uint256" }, { name: "validAfter", type: "uint256" }, { name: "validBefore", type: "uint256" }, { name: "nonce", type: "bytes32" }],
    },
    domain: { name: "USD Coin", version: "2", chainId: 8453, verifyingContract: nativeUsdc },
    primaryType: "TransferWithAuthorization",
    message: { from: address, value: "1000000", to: "0x" + "78".repeat(20), validAfter: "0", validBefore: String(fundingDeadline), nonce: creationNonce },
  };
  if (authorizationTamper === "sender") authorization.message.from = other;
  if (authorizationTamper === "recipient") authorization.message.to = other;
  if (authorizationTamper === "amount") authorization.message.value = "2000000";
  if (authorizationTamper === "nonce") authorization.message.nonce = "0x" + "cd".repeat(32);
  if (authorizationTamper === "deadline") authorization.message.validBefore = String(fundingDeadline + 1);
  if (authorizationTamper === "domain") authorization.domain.verifyingContract = other;
  if (authorizationTamper === "types") authorization.types.TransferWithAuthorization[2].type = "uint128";
  const calls = [{ to: other, data: "0x010203" }, { to: address, data: "0x040506" }];
  const plan = { bounty_id: "0x" + "56".repeat(32), predicted_bounty_contract: "0x" + "78".repeat(20), eip3009_authorization: authorization, wallet_calls: calls };
  const postingSession = {
    canContinue: () => Boolean(saved && ["authorized","sending"].includes(journal.load()?.phase) && !journal.load()?.transactions?.length),
    saveContinuation: async request => { saved = structuredClone(request); journal.authorizeContinuation("0x" + "56".repeat(32)); if (mutateAt === "authorization_save") mutate(); await postingSession.flush({requireServer:true}); },
    loadContinuation: async () => { if (!synchronized) throw new Error("Account sync unavailable; saved authority is not durably confirmed"); return saved; },
    snapshot: () => ({ operation_id: journey.id }),
    refresh: async () => { if (mutateAt === "refresh") mutate(); },
    flush: async (options = {}) => {
      checkpoints.push({ required: options.requireServer === true, phase: journal.load()?.phase });
      if ((mutateAt === "sync" || (mutateAt === "signing_sync" && journal.load()?.phase === "signing")) && options.requireServer) mutate();
      if (options.requireServer && (syncFailure === true || (syncFailure && syncFailure === journal.load()?.phase))) throw new Error("Account sync unavailable; no further wallet request dispatched");
      if (options.requireServer && journal.load()?.continuation_hash) synchronized = true;
      for (const identifier of journal.load()?.transactions || []) assert.equal(typeof identifier, "string", "durable recovery accepts batch identifiers, not response objects");
    },
    approved: async () => state.approved,
    reconcile: async () => ({ creation_confirmed: broadcast && !pending, funding_confirmed: broadcast && !pending, claimable: broadcast && !pending, public_inventory_verified: broadcast && !pending }),
  };
  const run = vm.runInNewContext(legalFunctions + bindingFunctions + walletFunctions + (legacy ? batchFunction.replace("atomicRequired:false,", "") : batchFunction) + finishFunction + fundingFunction, {
    postingBusy: false, postingBinding: null, postingJournal: journal, postingSession, state, window: win, ui: { form: { querySelectorAll: () => [] }, fundNow: {}, badge: {} }, document: { querySelector: () => null },
    sessionStorage: store,
    fetch: async (url, options) => {
      assert.ok(url.endsWith("/creation-relay"));
      assert.equal(checkpoints.at(-1).phase,"sending"); assert.equal(checkpoints.at(-1).required,true);
      assert.ok(saved); relays.push(JSON.parse(options.body));
      if (relayFault === "lost_before" && !faulted) { faulted=true; throw new Error("relay response lost"); }
      if ([429,503].includes(relayFault)) return {status:relayFault,json:async()=>({})};
      broadcast=true;
      if (relayFault === "lost_after" && !faulted) { faulted=true; throw new Error("relay response lost"); }
      return {status:202,json:async()=>({schema:"agent-bounties/sponsored-creation-v1",customer_gas_wei:"0",relay:{transaction:transactionHash,network:relayFault==="network"?"base-sepolia":"base-mainnet",bountyContract:relayFault==="bounty"?other:plan.predicted_bounty_contract}})};
    },
    track() {}, renderFundingGuide() {}, setPaymentStatus: value => statuses.push(value), refreshWalletReadiness: async () => {},
    updatePostingCost() {}, updatePostingTracker() {},
    loadProtocol: async () => ({ api_base_url: "https://api.agentbounties.app", factory: "0x" + "90".repeat(20), chain_id_hex: "0x2105", native_usdc: nativeUsdc }),
    currentRewardSplit: () => ({ solver: "900000", verifier: "100000", total: "1000000" }),
    contractTerms: () => ({}), termsDocument: () => ({}), createPayload: () => ({ creator: address, creation_nonce: creationNonce, funding_deadline: fundingDeadline }), validateCreationPlan() {}, formatUsdc: String,
    requestJson: async (url, options) => {
      publications.push(url);
      if (url.endsWith("/terms")) return {};
      if (url.endsWith("/authorized-creation-plan")) {
        assert.equal(JSON.parse(options.body).signature.v, 27);
        return {
          bounty_id: authorizedMismatch === "bounty_id" ? "0x" + "ab".repeat(32) : plan.bounty_id,
          predicted_bounty_contract: authorizedMismatch === "bounty_contract" ? other : plan.predicted_bounty_contract,
          network: { chain_id: authorizedMismatch === "network" ? 1 : 8453 },
          relay_transaction: { to: "0x" + "90".repeat(20), data: "0xaabbcc", value_wei: 0 },
        };
      }
      assert.ok(url.endsWith("/creation-plan"));
      if (change === "account") { sdk.accounts = [other]; universal.session.namespaces.eip155.accounts = [`eip155:8453:${other}`]; }
      if (change === "chain") { sdk.chainId = 1; universal.rpcProviders.eip155.chainId = 1; }
      return plan;
    },
    pollCreation: async () => pending || !broadcast ? null : [{ kind: "canonical_bounty_created" }, { kind: "funding_added" }, { kind: "bounty_became_claimable" }],
    fetchFeedItem: async () => ({ terms_valid: true, verification_ready: true }),
  });
  return { sdk, provider, run, signing, statuses, network, journal, authorization, calls, checkpoints, publications, relays, saved: () => saved };
}


test("pinned SDK network normalization reaches one exact signature and no wallet-paid transaction", async () => {
  const old=await fixture({adapted:false});assert.equal(await old.sdk.request({method:"eth_chainId"}),8453);await old.run();assert.equal(old.signing.length,0);assert.match(old.statuses.at(-1),/network changed/);
  const env=await fixture();assert.equal(await env.provider.request({method:"eth_chainId"}),"0x2105");await env.run();assert.equal(env.signing.length,1);assert.equal(env.signing[0].request.method,"eth_signTypedData_v4");assert.match(env.statuses.at(-1),/Wallet request cancelled/);assert.equal(env.journal.load(),null);assert.equal(env.relays.length,0);
});
for (const accountCode of ["0x","0xef0100"+"34".repeat(20),"0x6001600055"]) for (const pending of [false,true]) {
  test(`zero-ETH sponsored SDK creation, code=${accountCode}, pending=${pending}`, async()=>{
    const env=await fixture({accountCode,pending,approveAuthorization:true});await env.run();
    assert.deepEqual(env.signing.map(x=>x.request.method),["eth_signTypedData_v4"]);
    assert.deepEqual(Array.from(env.signing[0].request.params),[address,JSON.stringify(env.authorization)]);
    assert.equal(env.relays.length,1);assert.equal(env.relays[0].signature.v,27);
    assert.equal(env.saved().signature,syntheticSignature);assert.equal(env.journal.load().transactions[0],transactionHash);
    assert.equal(env.journal.load().phase,pending?"submitted":"funding_confirmed");
    await env.run();assert.equal(env.signing.length,1);assert.equal(env.relays.length,1);assert.equal(env.network.length,0);
  });
}
for (const relayFault of ["lost_before","lost_after"]) test(`lost ${relayFault} keeps one authorization and reconciles safely`,async()=>{
  const env=await fixture({approveAuthorization:true,relayFault});await env.run();
  assert.match(env.statuses.at(-1),/relay response lost/);assert.equal(env.journal.load().authorizationIssued,true);assert.equal(env.journal.load().transactions.length,0);
  await env.run();assert.equal(env.signing.length,1);assert.equal(env.journal.load().phase,"funding_confirmed");
  assert.equal(env.relays.length,relayFault==="lost_before"?2:1);if(env.relays.length===2)assert.deepEqual(env.relays[0],env.relays[1]);
});
for (const relayFault of [429,503,"network","bounty"]) test(`unavailable or foreign relay ${relayFault} cannot trigger customer gas`,async()=>{
 const env=await fixture({approveAuthorization:true,relayFault,pending:true});await env.run();
 assert.equal(env.journal.load().transactions.length,0);assert.equal(env.journal.load().authorizationIssued,true);
 await env.run();assert.equal(env.signing.length,1);assert.deepEqual(env.relays[0],env.relays[1]);assert.equal(env.network.length,0);
});
for (const authorizationTamper of ["sender","recipient","amount","nonce","deadline","expired","domain","types"]) test(`exact funding authority rejects ${authorizationTamper} before signing`,async()=>{
 const env=await fixture({approveAuthorization:true,authorizationTamper});await env.run();assert.match(env.statuses.at(-1),/USDC authorization does not match/);assert.equal(env.signing.length,0);assert.equal(env.relays.length,0);
});
for (const change of ["account","chain"]) test(`SDK ${change} change stops before signing`,async()=>{
 const env=await fixture({change,approveAuthorization:true});await env.run();assert.match(env.statuses.at(-1),/wallet or network changed/);assert.equal(env.signing.length,0);assert.equal(env.relays.length,0);
});
for (const mutateAt of ["refresh","legal","signing_sync"]) for (const mutation of ["draft","account","provider","approval"]) test(`changed ${mutation} during ${mutateAt} cannot borrow approval`,async()=>{
 const env=await fixture({mutateAt,mutation,approveAuthorization:true});await env.run();assert.equal(env.signing.length,0);assert.equal(env.relays.length,0);assert.match(env.statuses.at(-1),/approved draft or wallet changed/);
});
for (const syncFailure of [true,"signing","authorized","sending"]) test(`durable sync failure ${syncFailure} prevents gas or principal submission`,async()=>{
 const env=await fixture({syncFailure,approveAuthorization:true});await env.run();assert.equal(env.relays.length,0);assert.match(env.statuses.at(-1),/sync unavailable/);
 assert.equal(env.signing.length,["authorized","sending"].includes(syncFailure)?1:0);
 if(env.signing.length){assert.equal(env.journal.load().authorizationIssued,true);await env.run();assert.equal(env.signing.length,1);assert.equal(env.relays.length,0);}
});
for(const mutation of ["draft","account","provider","approval"]) test(`changed ${mutation} after signature is saved cannot send it`,async()=>{
 const env=await fixture({mutateAt:"authorization_save",mutation,approveAuthorization:true});await env.run();assert.equal(env.signing.length,1);assert.equal(env.relays.length,0);assert.equal(env.journal.load().authorizationIssued,true);
});
test("lost signature reply remains locked and sponsor outage prevents signing",async()=>{
 const env=await fixture({uncertain:true});await env.run();await env.run();assert.equal(env.signing.length,1);assert.equal(env.relays.length,0);assert.ok(env.journal.load());
 const unavailable=await fixture({gasSponsored:false,approveAuthorization:true});await unavailable.run();assert.equal(unavailable.signing.length,0);assert.equal(unavailable.relays.length,0);
});
test("a wider legacy creation signature is preserved without truncation or a paid fallback",async()=>{
 const signature="0x7f"+syntheticSignature.slice(2);const env=await fixture({approveAuthorization:true,walletSignature:signature});await env.run();
 assert.equal(env.saved().signature,signature);assert.equal(env.signing.length,1);assert.equal(env.relays.length,0);assert.match(env.statuses.at(-1),/exact 65-byte/);await env.run();assert.equal(env.signing.length,1);
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
