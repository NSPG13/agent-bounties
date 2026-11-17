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
const termsPublication = require("../site/terms-publication.js");
const termsFixture = require("./fixtures/terms-publication-readback.json").cases[0];
const { createPostingJournal } = require("../site/marketplace-workflow.js");
const fundingReadinessWindow = {};
vm.runInNewContext(fs.readFileSync(require.resolve("../site/funding-readiness.js"), "utf8"), { window: fundingReadinessWindow });
const { describeWalletCalls, formatUnits } = fundingReadinessWindow.AgentBountiesFundingReadiness;
const address = "0x" + "12".repeat(20), other = "0x" + "34".repeat(20);
const nativeUsdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const source = fs.readFileSync(require.resolve("../site/bounty-composer-v2.js"), "utf8");
const start = source.indexOf("  async function fundApprovedBounty()"), end = source.indexOf("  function showAuthorizationRecovery", start);
assert.ok(start >= 0 && end > start);
const fundingFunction = source.slice(start, end) + "; fundApprovedBounty";
const publicationStart = source.indexOf("  async function publicationRequest("), publicationEnd = source.indexOf("  function draftContext(");
assert.ok(publicationStart >= 0 && publicationEnd > publicationStart);
const publicationFunction = source.slice(publicationStart, publicationEnd);
const batchFunction = source.slice(source.indexOf("  async function sendWalletCalls("), source.indexOf("  function contractTerms("));
const walletFunctions = source.slice(source.indexOf("  function signatureParts("), source.indexOf("  async function sendWalletCalls("));
const bindingFunctions = source.slice(source.indexOf("  const V2_CREATION_PENDING"), source.indexOf("  async function watchUsdcAsset("));
const legalFunctions = source.slice(source.indexOf("  const LEGAL_RECEIPT_KEY"), source.indexOf("  async function refreshWalletReadiness("));
const finishFunction = source.slice(source.indexOf("  async function finishPosting("), source.indexOf("  async function fundApprovedBounty("));
const rejection = { code: 4001, message: "MetaMask Tx Signature: User denied transaction signature." };
const hashes = ["0x" + "a1".repeat(32), "0x" + "c2".repeat(32), "0x" + "a3".repeat(32), "0x" + "c4".repeat(32)];
const protocol = JSON.parse(fs.readFileSync(require.resolve("../site/protocol-v2.json"), "utf8"));

async function fixture({ adapted = true, change = null, pending = false, rejectAt = null, lostAt = null, revertAt = null,
  mutateAt = null, mutation = "draft", syncFailure = false, planTamper = null, quoteTamper = null, eth = 1n, allowance = 0n,
  missingPublicationHelper = false, publicationFault = null } = {}) {
  const storage = new Map(), signing = [], statuses = [], network = [], checkpoints = [], publications = [], fetches = [];
  let broadcast = false, sends = 0, sendingSaves = 0;
  const store = { getItem: key => storage.get(key) || null, setItem: (key, value) => storage.set(key, value), removeItem: key => storage.delete(key) };
  store.setItem("agent-bounties-phone-connected-v1", "ab-phone-12345678-1234-1234-1234-123456789012");
  const namespace = { chains: ["eip155:8453"], accounts: [`eip155:8453:${address}`], methods: ["eth_signTypedData_v4", "wallet_sendCalls", "eth_sendTransaction"], events: [], rpcMap: { "eip155:8453": "http://127.0.0.1:1" } };
  const universal = new UniversalProvider({ logger: "silent", disableProviderPing: true });
  const context = { allowance };
  universal.client = {
    core: { projectId: "0".repeat(32), storage: { getItem: async key => storage.get(key), setItem: async (key, value) => storage.set(key, value) } },
    request: async request => {
      signing.push(request);
      if (request.request.method !== "eth_sendTransaction") throw new Error("Only reviewed v2 transactions may reach the phone wallet");
      const index = ++sends, plan = context.plan;
      if (index === rejectAt) throw new Error(JSON.stringify(rejection));
      if (index === lostAt) throw new Error("Fixture wallet response was lost");
      // A creation sent after the allowance is the only call that moves USDC.
      if (plan && String(request.request.params[0].data).toLowerCase() === plan.wallet_calls[1].data && index !== revertAt) broadcast = true;
      return hashes[index - 1];
    },
  };
  universal.namespaces = { eip155: namespace };
  universal.session = { topic: "synthetic-session", expiry: Date.now() / 1000 + 3600, namespaces: { eip155: namespace } };
  universal.createProviders();
  // Make accidental HTTP reads fail immediately, even if a fixture regresses.
  universal.rpcProviders.eip155.httpProviders[8453].request = async request => {
    if (request.method === "eth_getTransactionReceipt") {
      const index = hashes.indexOf(request.params[0]);
      assert.ok(index >= 0 && index < sends, "receipts are read only for returned hashes");
      return { status: index + 1 === revertAt ? "0x0" : "0x1" };
    }
    if (request.method === "eth_call" && request.params[0].data.startsWith("0xdd62ed3e")) {
      assert.equal(request.params[0].to, protocol.native_usdc);
      assert.equal(request.params[0].data, `0xdd62ed3e${address.slice(2).padStart(64, "0")}${protocol.factory.slice(2).padStart(64, "0")}`);
      return `0x${context.allowance.toString(16).padStart(64, "0")}`;
    }
    network.push(request); throw new Error("Unexpected RPC call in offline fixture");
  };
  const sdk = new EthereumProvider(); sdk.signer = universal; sdk.chainId = 8453; sdk.accounts = [address];
  const win = {
    document: null, crypto: webcrypto, localStorage: store, sessionStorage: store, location: new URL("https://agentizable.ai/post.html"),
    agentBountiesPhoneWalletConfig: { projectId: "0".repeat(32) },
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    addEventListener() {}, dispatchEvent() {}, setTimeout, clearTimeout,
    AgentBountiesLegal: {
      loadPolicy: async () => ({ source: "hosted", supported_actions: ["post_bounty"], terms_version: "test-terms", privacy_version: "test-privacy", statement_hash: "test-statement" }),
      requireAcceptance: async () => { if (mutateAt === "legal") mutate(); return { durable: true, action: "post_bounty", wallet_address: address, terms_version: "test-terms", privacy_version: "test-privacy", statement_hash: "test-statement" }; },
    },
  };
  win.location.assign = () => {};
  for (const name of ["evm.js", "posting-v2-plan.js"]) vm.runInNewContext(fs.readFileSync(require.resolve(`../site/${name}`), "utf8"), { window: win, TextEncoder });
  if (!missingPublicationHelper) win.AgentBountiesTermsPublication = termsPublication;
  const phone = createPhoneWallet(win, { loadVendor: async () => ({ createProvider: async () => sdk }) });
  await phone.restore();
  const provider = adapted ? phone.provider : sdk;
  const journal = createPostingJournal(win);
  const amounts = win.AgentBountiesPostingV2Plan.economics(2_000_000n, 40_000n, protocol);
  const state = { approved: true, provider, providerKind: "phone", linkedWallets: [{ address }], accountSession: { user: { id: "fixture-account" } }, taskWindowDays: 7, account: address,
    balances: { usdc: amounts.total, required: amounts.total, eth }, draft: { title: "Synthetic review" }, fundingUsdc: 2.04 };
  let journey = { id: "12345678-1234-1234-1234-123456789012", role: "post", draft: state.draft };
  function mutate() {
    if (mutation === "draft") journey = { ...journey, draft: { title: "Other device revised this draft" } };
    if (mutation === "account") state.account = other;
    if (mutation === "provider") state.provider = { request: async () => { throw new Error("Replacement provider must not receive a request"); } };
    if (mutation === "approval") state.approved = false;
  }
  win.AgentBountiesPostingSession = postingSessionLibrary;
  win.AgentBountiesWorkflow = { createClient: () => ({ load: () => journey }) };
  // Real call decoding binds the v2 selector, amount and recipient; fees are synthetic.
  win.AgentBountiesFundingReadiness = { describeWalletCalls, formatUnits,
    estimateFees: async () => { if (mutateAt === "fees") mutate(); return { estimatedTotalWei: 1n }; } };
  const creationNonce = "0x" + "ef".repeat(32);
  const fundingDeadline = Math.floor(Date.now() / 1000) + 3600;
  // Use the Rust-builder-verified record shape, not a request echo. Only this
  // fixture's creator, nonce, deadline and v2 economics vary; optional fields
  // retain their canonical server representation. The real helper checks all hashes.
  const usdc = amount => ({ amount: Number(amount), currency: "usdc" });
  const committed = { ...structuredClone(termsFixture.request.document.contract_terms), protocol_version: protocol.protocol_version,
    creator_wallet: address, creation_nonce: creationNonce, funding_deadline: fundingDeadline,
    solver_reward: usdc(amounts.solver), verifier_reward: usdc(amounts.verifier), claim_bond: usdc(amounts.verifier), initial_funding: usdc(amounts.total),
    platform_fee_bps: protocol.platform_fee_bps, platform_fee: usdc(amounts.fee), platform_fee_recipient: protocol.platform_fee_recipient };
  const publicationDocument = { ...structuredClone(termsFixture.request.document), contract_terms: committed };
  const publishedDocument = { ...structuredClone(termsFixture.record.document), contract_terms: structuredClone(committed) };
  const publishedRecord = { ...structuredClone(termsFixture.record), creator_wallet: address, document: publishedDocument };
  for (const [field,value] of [["terms_hash",publishedDocument],["policy_hash",publishedDocument.verification_policy],
    ["acceptance_criteria_hash",publishedDocument.acceptance_criteria],["benchmark_hash",publishedDocument.benchmark],["evidence_schema_hash",publishedDocument.evidence_schema]])
    publishedRecord[field] = win.AgentBountiesEvm.keccak256Hex(win.AgentBountiesEvm.textHex(termsPublication.canonical(value)));
  const createPayload = (terms, terms_) => ({ creator: address, solver_reward: terms_.solver_reward, verifier_reward: terms_.verifier_reward,
    terms_hash: terms.terms_hash, policy_hash: terms.policy_hash, acceptance_criteria_hash: terms.acceptance_criteria_hash, benchmark_hash: terms.benchmark_hash,
    evidence_schema_hash: terms.evidence_schema_hash, funding_deadline: terms_.funding_deadline, claim_window_seconds: terms_.claim_window_seconds,
    verification_window_seconds: terms_.verification_window_seconds, verification_mode: "signed_quorum", verifier_module: null, verifier_reward_recipient: null,
    verifiers: [address], threshold: 1, initial_funding: terms_.initial_funding, creation_nonce: terms_.creation_nonce });
  function quote(create) {
    const value = { protocol_version: protocol.protocol_version, solver_reward: String(amounts.solver), verifier_reward: String(amounts.verifier), claim_bond: String(amounts.verifier),
      platform_fee_bps: protocol.platform_fee_bps, platform_fee: String(amounts.fee), platform_fee_recipient: protocol.platform_fee_recipient, target_amount: String(amounts.total) };
    assert.equal(create.initial_funding.amount, Number(amounts.total));
    return value;
  }
  function v2Plan(create) {
    const encoded = win.AgentBountiesPostingV2Plan.encode(create, protocol);
    return { protocol_version: protocol.protocol_version, network: { chain_id: 8453, native_usdc_token_address: protocol.native_usdc },
      factory_contract: protocol.factory, implementation_contract: protocol.implementation, bounty_id: encoded.bountyId, predicted_bounty_contract: encoded.bounty, quote: quote(create),
      wallet_calls: [{ from: address, to: protocol.native_usdc, value_wei: 0, data: "0x095ea7b3" + protocol.factory.slice(2).padStart(64, "0") + amounts.total.toString(16).padStart(64, "0") },
        { from: address, to: protocol.factory, value_wei: 0, data: encoded.data }],
      // Advertising a signature must not route new creation through the retired V1 relay.
      eip3009_authorization: { primaryType: "ReceiveWithAuthorization" } };
  }
  const postingSession = {
    canContinue: () => false,
    loadContinuation: async () => { throw new Error("No V1 continuation exists in this fixture"); },
    snapshot: () => ({ operation_id: journey.id }),
    refresh: async () => { if (mutateAt === "refresh") mutate(); },
    flush: async (options = {}) => {
      checkpoints.push({ required: options.requireServer === true, phase: journal.load()?.phase });
      if (mutateAt === "sending_sync" && journal.load()?.phase === "sending" && options.requireServer) mutate();
      const creationSave = options.requireServer && journal.load()?.phase === "sending" && ++sendingSaves === 2;
      if (options.requireServer && (syncFailure === true || (syncFailure && syncFailure === journal.load()?.phase) || (syncFailure === "creation_sending" && creationSave)
        || (syncFailure === "creation_submitted" && journal.load()?.phase === "submitted" && journal.load().transactions.length === 2))) throw new Error("Account sync unavailable; no further wallet request dispatched");
      for (const identifier of journal.load()?.transactions || []) assert.equal(typeof identifier, "string", "durable recovery accepts transaction identifiers, not response objects");
    },
    approved: async () => state.approved,
    reconcile: async () => { const done = (broadcast || context.creationLanded === true) && !pending;
      return { creation_confirmed: done, funding_confirmed: done, claimable: done, public_inventory_verified: done }; },
  };
  const run = vm.runInNewContext(legalFunctions + bindingFunctions + publicationFunction + walletFunctions + batchFunction + finishFunction + fundingFunction, {
    postingBusy: false, postingBinding: null, postingJournal: journal, postingSession, state, window: win, ui: { form: { querySelectorAll: () => [] }, fundNow: {}, badge: {} },
    document: { querySelector: selector => selector === "[data-v2-cost-consent]" ? { checked: true } : null },
    sessionStorage: store, addressWord: value => String(value).toLowerCase().replace(/^0x/, "").padStart(64, "0"),
    fetch: async url => { fetches.push(url); throw new Error("No direct fetch, relay or sponsor may be reached"); },
    track() {}, renderFundingGuide() {}, setPaymentStatus: value => statuses.push(value),
    refreshWalletReadiness: async () => { state.v2CostConsentBinding = run.legalBinding(); },
    updatePostingCost() {}, updatePostingTracker() {},
    loadProtocol: async () => structuredClone(protocol),
    currentRewardSplit: () => ({ solver: String(amounts.solver), verifier: String(amounts.verifier), total: String(amounts.solver + amounts.verifier) }),
    contractTerms: () => structuredClone(committed), termsDocument: () => structuredClone(publicationDocument), createPayload,
    validateCreationPlan() {}, formatUsdc: String,
    requestJson: async (url, options) => {
      publications.push(url);
      assert.doesNotMatch(url, /creation-relay|authorized-creation-plan|autonomous-bounties\/creation-plan/, "retired V1 creation must not be requested");
      if (url.endsWith("/terms")) {
        assert.equal(options.method, "POST");
        assert.match(options.headers["Idempotency-Key"], /^publication-[0-9a-f-]{36}$/);
        assert.deepEqual(JSON.parse(options.body), { creator_wallet: address, document: publicationDocument });
        // Synthetic website-account authority is linked to this exact wallet.
        // The happy path must not request a separate ownership signature.
        assert.deepEqual(state.linkedWallets, [{ address }]);
        assert.equal(options.headers.Authorization, undefined);
        if (["authentication", "authority"].includes(publicationFault)) throw Object.assign(new Error("Publication authority required"), {
          status: publicationFault === "authentication" ? 401 : 403,
          body: { error_code: publicationFault === "authentication" ? "publication_authentication_required" : "publication_wallet_authority_required" },
        });
        const result = structuredClone(publishedRecord);
        if (publicationFault === "hash") result.terms_hash = "0x" + "00".repeat(32);
        if (publicationFault === "content") result.document.goal = "Substituted goal";
        return result;
      }
      const body = JSON.parse(options.body);
      if (url.endsWith("/v2/quote")) {
        const value = quote({ initial_funding: usdc(amounts.total) });
        if (quoteTamper === "fee") value.platform_fee = String(amounts.fee + 1n);
        if (quoteTamper === "recipient") value.platform_fee_recipient = other;
        return value;
      }
      assert.ok(url.endsWith("/v2/creation-plan"));
      if (change === "account") { sdk.accounts = [other]; universal.session.namespaces.eip155.accounts = [`eip155:8453:${other}`]; }
      if (change === "chain") { sdk.chainId = 1; universal.rpcProviders.eip155.chainId = 1; }
      const plan = v2Plan(body.create);
      if (planTamper === "calldata") plan.wallet_calls[1].data = plan.wallet_calls[1].data.slice(0, -1) + "f";
      if (planTamper === "allowance") plan.wallet_calls[0].data = plan.wallet_calls[0].data.slice(0, -2) + "ff";
      if (planTamper === "factory") plan.factory_contract = other;
      if (planTamper === "bounty") plan.predicted_bounty_contract = other;
      context.plan = v2Plan(body.create);
      return plan;
    },
    pollCreation: async () => pending || !(broadcast || context.creationLanded) ? null : [{ kind: "canonical_bounty_created" }, { kind: "funding_added" }, { kind: "bounty_became_claimable" }],
    fetchFeedItem: async () => ({ terms_valid: true, verification_ready: true }),
  });
  run.legalBinding = vm.runInNewContext(legalFunctions + "; legalBinding", { state, postingSession, window: win, sessionStorage: store });
  const methods = () => signing.map(item => item.request.method);
  const sentData = () => signing.map(item => String(item.request.params[0].data).toLowerCase());
  return { sdk, provider, run, store, signing, methods, sentData, statuses, network, journal, checkpoints, publications, fetches, context, amounts, setAllowance: value => { context.allowance = value; } };
}

const pendingPhase = "v2_creation_pending";
const sessionStorageSet = (env, value) => env.store.setItem("agent-bounties.posting-operation.v1", JSON.stringify(value));
test("missing publication helper stops before publication or any wallet request", async () => {
  const env = await fixture({ missingPublicationHelper: true });
  await env.run();
  assert.match(env.statuses.at(-1), /Publication checks could not load/);
  assert.equal(env.publications.length, 0); assert.equal(env.signing.length, 0); assert.equal(env.fetches.length, 0);
});
for (const publicationFault of ["authentication", "authority", "hash", "content"]) test(`publication ${publicationFault} stops before any wallet request`, async () => {
  const env = await fixture({ publicationFault });
  await env.run();
  assert.match(env.statuses.at(-1), publicationFault === "hash" ? /terms_hash.*does not match/ : publicationFault === "content" ? /differ from/ : /Verify this wallet/);
  assert.equal(env.publications.length, 1); assert.ok(env.publications[0].endsWith("/terms"));
  assert.equal(env.signing.length, 0); assert.equal(env.network.length, 0); assert.equal(env.journal.load(), null);
});

test("pinned SDK network normalization reaches the exact allowance and a cancelled first request leaves nothing recorded", async () => {
  const old=await fixture({adapted:false});assert.equal(await old.sdk.request({method:"eth_chainId"}),8453);await old.run();assert.equal(old.signing.length,0);assert.match(old.statuses.at(-1),/network changed/);
  const env=await fixture({rejectAt:1});assert.equal(await env.provider.request({method:"eth_chainId"}),"0x2105");await env.run();
  assert.deepEqual(env.methods(),["eth_sendTransaction"]);assert.match(env.statuses.at(-1),/Wallet request cancelled/);assert.equal(env.journal.load(),null);assert.equal(env.fetches.length,0);
});
for (const pending of [false,true]) test(`v2 creation sends exactly the validated allowance then creation from the creator's wallet, pending=${pending}`, async()=>{
  const env=await fixture({pending});await env.run();
  assert.deepEqual(env.methods(),["eth_sendTransaction","eth_sendTransaction"]);
  assert.deepEqual(env.sentData(),env.context.plan.wallet_calls.map(call=>call.data));
  for (const [index,item] of env.signing.entries()) {
    const tx=item.request.params[0];assert.equal(tx.from.toLowerCase(),address);assert.equal(tx.to.toLowerCase(),env.context.plan.wallet_calls[index].to);assert.equal(tx.value,"0x0");
  }
  assert.equal(env.journal.load().bounty_id,env.context.plan.bounty_id);assert.deepEqual(env.journal.load().transactions,hashes.slice(0,2));
  assert.equal(env.journal.load().phase,pending?"submitted":"funding_confirmed");
  assert.deepEqual(env.publications.map(url=>url.replace(/^.*\/v1\/base\/autonomous-bounties/,"")),["/terms","/v2/quote","/v2/creation-plan"]);
  await env.run();assert.equal(env.signing.length,2);assert.equal(env.network.length,0);assert.equal(env.fetches.length,0);
});
for (const [label,options] of [["rejected",{rejectAt:2}],["reverted",{revertAt:2}],["blocked by a failed pre-dispatch save",{syncFailure:"creation_sending"}]]) test(`creation ${label} after the allowance is resumable without moving USDC`, async()=>{
  const env=await fixture(options);await env.run();
  assert.equal(env.journal.load().phase,pendingPhase);assert.match(env.statuses.at(-1),/No USDC has moved.*Finish posting/s);
  assert.equal(env.signing.length,options.syncFailure?1:2);
  env.setAllowance(env.amounts.total);await env.run();
  assert.equal(env.sentData().at(-1),env.context.plan.wallet_calls[1].data,"only the same creation is requested");
  assert.equal(env.signing.length,options.syncFailure?2:3);
  assert.equal(env.journal.load().phase,"funding_confirmed");
});
test("a resumed creation re-approves only when the recorded allowance is not on Base", async()=>{
  const env=await fixture({rejectAt:2});await env.run();assert.equal(env.journal.load().phase,pendingPhase);
  await env.run();
  assert.deepEqual(env.sentData().slice(2),env.context.plan.wallet_calls.map(call=>call.data));
  assert.equal(env.journal.load().phase,"funding_confirmed");assert.equal(env.signing.length,4);
});
test("an uncertain allowance reply is resumable; an uncertain creation reply stays locked", async()=>{
  const allowance=await fixture({lostAt:1});await allowance.run();assert.equal(allowance.journal.load().phase,pendingPhase);
  await allowance.run();assert.equal(allowance.journal.load().phase,"funding_confirmed");assert.equal(allowance.signing.length,3);
  const creation=await fixture({lostAt:2});await creation.run();assert.equal(creation.journal.load().phase,"sending");assert.match(creation.statuses.at(-1),/response was lost/);
  await creation.run();assert.equal(creation.signing.length,2,"an uncertain creation is never requested again");assert.match(creation.statuses.at(-1),/already recorded/);
});
test("canonical creation outranks a resumable record and is reconciled without another wallet request", async()=>{
  const env=await fixture({rejectAt:2});await env.run();assert.equal(env.journal.load().phase,pendingPhase);
  env.context.creationLanded=true;const before=env.publications.length;
  await env.run();
  assert.equal(env.signing.length,2);assert.equal(env.publications.length,before,"no terms, quote or plan is requested again");
  assert.equal(env.journal.load().phase,"funding_confirmed");
});
test("a creation hash recorded before a failed save stays locked instead of resumable", async()=>{
  const env=await fixture({syncFailure:"creation_submitted"});await env.run();
  assert.equal(env.journal.load().phase,"submitted");assert.deepEqual(env.journal.load().transactions,hashes.slice(0,2));assert.match(env.statuses.at(-1),/sync unavailable/);
  await env.run();assert.equal(env.signing.length,2);assert.match(env.statuses.at(-1),/already recorded/);
});
test("a resumable record for another bounty cannot authorize this creation", async()=>{
  const env=await fixture({rejectAt:2});await env.run();assert.equal(env.journal.load().phase,pendingPhase);
  const recorded=env.journal.load();sessionStorageSet(env,{...recorded,bounty_contract:other});
  env.setAllowance(env.amounts.total);await env.run();
  assert.match(env.statuses.at(-1),/no longer matches the reviewed bounty/);assert.equal(env.signing.length,2);
});
test("finishing from a tab without the reserved publication stops before publishing new terms", async()=>{
  const env=await fixture({rejectAt:2});await env.run();assert.equal(env.journal.load().phase,pendingPhase);
  env.store.removeItem("agent-bounties:terms-publication:v1:direct");const before=env.publications.length;
  await env.run();
  assert.match(env.statuses.at(-1),/browser tab where you approved/);assert.equal(env.publications.length,before);assert.equal(env.signing.length,2);
});
for (const planTamper of ["calldata","allowance","factory","bounty"]) test(`changed v2 plan ${planTamper} stops before any wallet request`,async()=>{
 const env=await fixture({planTamper});await env.run();assert.match(env.statuses.at(-1),/V2 plan differs/);assert.equal(env.signing.length,0);assert.equal(env.journal.load(),null);
});
for (const quoteTamper of ["fee","recipient"]) test(`changed v2 quote ${quoteTamper} stops before the creation plan`,async()=>{
 const env=await fixture({quoteTamper});await env.run();assert.match(env.statuses.at(-1),/V2 plan differs/);assert.equal(env.signing.length,0);assert.ok(!env.publications.some(url=>url.endsWith("/v2/creation-plan")));
});
test("a wallet without Base ETH stops before publication", async()=>{
 const env=await fixture({eth:0n});await env.run();assert.match(env.statuses.at(-1),/not ready to fund/);assert.equal(env.publications.length,0);assert.equal(env.signing.length,0);
});
for (const change of ["account","chain"]) test(`SDK ${change} change stops before any wallet request`,async()=>{
 const env=await fixture({change});await env.run();assert.match(env.statuses.at(-1),/wallet or network changed/);assert.equal(env.signing.length,0);
});
for (const mutateAt of ["refresh","legal","fees","sending_sync"]) for (const mutation of ["draft","account","provider","approval"]) test(`changed ${mutation} during ${mutateAt} cannot borrow approval`,async()=>{
 const env=await fixture({mutateAt,mutation});await env.run();assert.equal(env.signing.length,0);assert.match(env.statuses.at(-1),/approved draft or wallet changed|signing account or Base network changed/);
 assert.equal(env.journal.load(),null,"a request the wallet never received leaves no recorded operation");
});
for (const syncFailure of [true,"prepared","sending"]) test(`durable sync failure ${syncFailure} prevents any wallet request`,async()=>{
 const env=await fixture({syncFailure});await env.run();assert.equal(env.signing.length,0);assert.match(env.statuses.at(-1),/sync unavailable/);
 assert.equal(env.journal.load(),null);
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
