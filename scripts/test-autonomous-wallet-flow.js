const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const { webcrypto } = require("crypto");

const repoRoot = path.resolve(__dirname, "..");
const evmSource = fs.readFileSync(path.join(repoRoot, "site", "evm.js"), "utf8");
const source = fs.readFileSync(path.join(repoRoot, "site", "autonomous.js"), "utf8");
const x402Source = fs.readFileSync(path.join(repoRoot, "site", "x402-browser.js"), "utf8");
const registrySource = fs.readFileSync(path.join(repoRoot, "site", "wallet-adapter-registry.js"), "utf8");
const embeddedAdapterSource = fs.readFileSync(
  path.join(repoRoot, "tools", "coinbase-embedded-wallet", "src", "index.js"),
  "utf8",
);
const protocol = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "site", "protocol.json"), "utf8"),
);

new vm.Script(evmSource, { filename: "site/evm.js" });
new vm.Script(source, { filename: "site/autonomous.js" });
new vm.Script(x402Source, { filename: "site/x402-browser.js" });
new vm.Script(registrySource, { filename: "site/wallet-adapter-registry.js" });

for (const required of [
  "eth_signTypedData_v4",
  "wallet_sendCalls",
  "eip6963:requestProvider",
  "data-wallet-provider",
  "create_bounty",
  "eip3009_authorization",
  "/v1/base/autonomous-bounties/creation-plan",
  "/v1/base/autonomous-bounties/authorized-creation-plan",
  "/v1/base/autonomous-bounties/contribution-plan",
  "window.AgentBountiesX402",
  "x402-hosted-relay",
  "Your wallet signs exact Base USDC authorization",
  "/v1/base/autonomous-bounties/claims",
  "request_bond_sponsorship",
  "wallet_signature",
  "canonical_event_id",
  "/v1/base/autonomous-bounties/submission-plan",
  "FundingAdded",
  "BountyClaimed",
  "submission_added",
  "BountySettled",
  "default_verification",
  "configurePostVerification",
  "legacy-recovery-form",
  "recoverLegacyBounties",
  "expectedCloneRuntime",
  "0x884834e884d6e93462655a2820140ad03e6747bc",
  "0x786be3f994365fcd417a1b502a83300ea87d9b34",
  "0x481dfc6f45d43b89dfcc1a84fd6d9b5f73a6a0b9",
  "0x3195aebfc39a069bf1a4420951d0babc99b2b612",
  "0xea8a1af0",
  "0x110f8874",
]) {
  assert(source.includes(required), `autonomous wallet flow missing ${required}`);
}

for (const required of [
  "agent-bounty-fund",
  "payment-required",
  "payment-signature",
  "payment-response",
  "eth_signTypedData_v4",
  "FundingAdded",
  "/v1/x402/base/bounties/",
  "/v1/x402/base/relays/",
]) {
  assert(x402Source.includes(required), `x402 browser flow missing ${required}`);
}
assert(!x402Source.includes("eth_sendTransaction"), "x402 browser flow must not request a user gas transaction");

for (const required of [
  "register",
  "capabilitiesFor",
  "eip6963:announceProvider",
]) {
  assert(registrySource.includes(required), `wallet adapter registry missing ${required}`);
}

for (const required of [
  "directTransactions: false",
  "gasSponsoredOnSupportedRelays: true",
  "authMethodLinking: true",
  "Link another sign-in method",
]) {
  assert(embeddedAdapterSource.includes(required), `Coinbase embedded adapter missing ${required}`);
}

for (const retired of [
  "import wallet",
  "seed phrase",
  "private key",
  "createEscrow",
  "EscrowReleased",
  "/v1/base/funding-plan",
  "/v1/base/release-plan",
  "settlement signer",
  "/v1/base/autonomous-bounties/authorized-contribution-plan",
]) {
  assert(!source.includes(retired), `autonomous wallet flow contains retired behavior: ${retired}`);
}

assert.strictEqual(protocol.protocol_version, "agent-bounties/autonomous-v1");
assert.strictEqual(protocol.network, "base-mainnet");
assert.strictEqual(protocol.chain_id, 8453);
assert.strictEqual(protocol.status, "active");
assert.strictEqual(protocol.factory, "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9");
assert.strictEqual(protocol.implementation, "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9");
assert.strictEqual(protocol.default_verification.mode, "signed_quorum");
assert.strictEqual(protocol.default_verification.product_label, "single_verifier");
assert.strictEqual(protocol.default_verification.verifiers.length, 1);
assert.strictEqual(protocol.default_verification.threshold, 1);
assert.strictEqual(protocol.deterministic_modules.leading_zero_work_v1.usage, "protocol_canary_only");
assert.strictEqual(
  protocol.deterministic_modules.leading_zero_work_v1.benchmark.engine,
  "leading_zero_work_v1",
);
assert.strictEqual(protocol.deterministic_modules.leading_zero_work_v1.benchmark.difficulty_bits, 16);
assert.strictEqual(
  protocol.deterministic_modules.leading_zero_work_v1.benchmark.verifier_module,
  protocol.deterministic_modules.leading_zero_work_v1.contract,
);

const postHtml = fs.readFileSync(path.join(repoRoot, "site", "post.html"), "utf8");
assert(
  postHtml.indexOf('value="signed_quorum" selected') >= 0,
  "public posting must default to one signed verifier",
);
assert(postHtml.includes("One automatic verifier"));
assert(postHtml.includes("Exact on-chain verifier"));
assert(postHtml.includes("does not evaluate my task or acceptance criteria"));
assert(postHtml.includes('name="demoVerifierAccepted" type="checkbox"'));
assert(!postHtml.includes('{"engine":"github_ci"'));
assert(postHtml.includes('name="solverReward" type="number" min="0.01" step="0.01" value="2.00"'));
assert(postHtml.includes('name="verifierReward" type="number" min="0.01" step="0.01" value="0.01"'));

const earnHtml = fs.readFileSync(path.join(repoRoot, "site", "earn.html"), "utf8");
assert(earnHtml.includes("Sign once. Start after BountyClaimed"));
assert(earnHtml.includes("Gas is sponsored for this funding action"));
assert(earnHtml.includes('data-wallet-requires="direct-transactions"'));
assert(earnHtml.includes('src="wallet-adapter-registry.js?v=1"'));
assert(earnHtml.includes('src="x402-browser.js?v=1"'));
assert(
  earnHtml.indexOf('src="x402-browser.js?v=1"') < earnHtml.indexOf('src="autonomous.js"'),
  "the sponsored x402 client must load before the autonomous funding flow",
);
assert(source.includes('params.get("bountyContract")'));
assert(source.includes("Sign once to claim"));
assert(source.includes("Sponsored refundable bond"));
assert(!source.includes("/v1/base/autonomous-bounties/authorized-claim-plan"));
const fundingStart = source.indexOf("async function fundBounty(event)");
const submissionStart = source.indexOf("async function submitBounty(event)", fundingStart);
assert(fundingStart >= 0 && submissionStart > fundingStart, "funding function boundaries are missing");
const fundingBody = source.slice(fundingStart, submissionStart);
assert(
  !fundingBody.includes("/v1/base/autonomous-bounties/authorized-contribution-plan"),
  "EOA funding must not call the retired user-gas authorized-contribution planner",
);
assert(
  !fundingBody.includes("sendTransaction(authorized.relay_transaction"),
  "EOA funding must not broadcast a relayer transaction from the user's wallet",
);

for (const page of ["index.html", "post.html", "funding.html", "earn.html", "operator.html", "recovery.html"]) {
  const html = fs.readFileSync(path.join(repoRoot, "site", page), "utf8");
  assert(html.includes("autonomous.js"), `${page} does not load autonomous.js`);
  assert(!html.includes("main.js"), `${page} loads the retired browser bundle`);
}

for (const page of ["post.html", "funding.html", "earn.html"]) {
  const html = fs.readFileSync(path.join(repoRoot, "site", page), "utf8");
  assert(html.includes("data-wallet-provider"), `${page} does not offer explicit wallet-provider selection`);
  assert(html.includes("Connect wallet"), `${page} does not use connect-wallet onboarding`);
}

async function testDeterministicPostingDefaults() {
  const documentListeners = {};
  const formListeners = {};
  const controlListeners = {};
  const scope = { textContent: "" };
  const elements = {
    verificationMode: {
      value: "deterministic_module",
      addEventListener(name, handler) {
        controlListeners[name] = handler;
      },
    },
    demoVerifierAccepted: { checked: false, disabled: false },
    verifierModule: { value: "", readOnly: false, disabled: false },
    verifierRewardRecipient: { value: "", disabled: false },
    verifiers: { value: "", disabled: false },
    threshold: { value: "8", readOnly: false },
    benchmark: { value: '{"engine":"tampered"}', readOnly: false },
    title: { value: "Canary" },
    goal: { value: "Complete the exact work proof." },
    acceptance: { value: "The scope-bound proof passes." },
    evidenceSchema: { value: '{"type":"object","additionalProperties":true}' },
    aiProvider: { value: "" },
    aiModel: { value: "" },
    aiModelVersion: { value: "" },
    systemPrompt: { value: "" },
    rubric: { value: "" },
    decodingParameters: { value: '{"temperature":0,"seed":0}' },
    sourceUrl: { value: "" },
    discoverySource: { value: "" },
  };
  const form = {
    id: "autonomous-post-form",
    elements,
    addEventListener(name, handler) {
      formListeners[name] = handler;
    },
    querySelector(selector) {
      return selector === "[data-verifier-scope]" ? scope : null;
    },
  };
  const document = {
    addEventListener(name, handler) {
      documentListeners[name] = handler;
    },
    getElementById(id) {
      return id === "autonomous-post-form" ? form : null;
    },
    querySelectorAll() {
      return [];
    },
  };
  const window = {
    addEventListener() {},
    dispatchEvent() {},
    ethereum: null,
  };
  const context = vm.createContext({
    console,
    crypto: webcrypto,
    document,
    window,
    Event,
    URLSearchParams,
    location: { search: "" },
    fetch: async () => ({ ok: true, json: async () => protocol }),
    setTimeout: (callback) => {
      callback();
      return 1;
    },
  });
  const instrumentedSource = source.replace(
    /\}\)\(\);\s*$/,
    "globalThis.__agentBountiesTest = { termsDocument, validateHostedClaimHandoff }; })();",
  );
  new vm.Script(evmSource, { filename: "site/evm.js" }).runInContext(context);
  new vm.Script(instrumentedSource, { filename: "site/autonomous.js" }).runInContext(context);
  await documentListeners.DOMContentLoaded();

  assert.strictEqual(
    elements.verifierModule.value,
    protocol.deterministic_modules.leading_zero_work_v1.contract,
  );
  assert.strictEqual(elements.verifierModule.readOnly, true);
  assert.strictEqual(elements.verifierModule.disabled, false);
  assert.strictEqual(elements.verifiers.disabled, true);
  assert.strictEqual(elements.threshold.value, "1");
  assert.strictEqual(elements.threshold.readOnly, true);
  assert.strictEqual(elements.benchmark.readOnly, true);
  assert.strictEqual(elements.demoVerifierAccepted.disabled, false);
  assert.deepStrictEqual(
    JSON.parse(elements.benchmark.value),
    protocol.deterministic_modules.leading_zero_work_v1.benchmark,
  );
  assert.strictEqual(
    scope.textContent,
    protocol.deterministic_modules.leading_zero_work_v1.scope_notice,
  );

  elements.verificationMode.value = "signed_quorum";
  controlListeners.change();
  assert.strictEqual(elements.verifierModule.disabled, true);
  assert.strictEqual(elements.verifierRewardRecipient.disabled, true);
  assert.strictEqual(elements.verifiers.disabled, false);
  assert.strictEqual(elements.threshold.readOnly, false);
  assert.strictEqual(elements.benchmark.readOnly, false);
  assert.strictEqual(elements.demoVerifierAccepted.disabled, true);

  elements.benchmark.value = '{"engine":"github_ci"}';
  elements.verificationMode.value = "deterministic_module";
  controlListeners.change();
  assert.strictEqual(elements.verifierModule.disabled, false);
  assert.strictEqual(elements.verifiers.disabled, true);
  assert.strictEqual(elements.threshold.value, "1");
  assert.strictEqual(elements.benchmark.readOnly, true);
  assert.strictEqual(elements.demoVerifierAccepted.disabled, false);
  assert.deepStrictEqual(
    JSON.parse(elements.benchmark.value),
    protocol.deterministic_modules.leading_zero_work_v1.benchmark,
  );

  elements.benchmark.value = '{"engine":"github_ci"}';
  elements.verifierRewardRecipient.value = "0x1111111111111111111111111111111111111111";
  const terms = context.__agentBountiesTest.termsDocument(
    form,
    { verifier_reward: { amount: 10_000, currency: "usdc" } },
    protocol,
  );
  assert.deepStrictEqual(
    JSON.parse(JSON.stringify(terms.benchmark)),
    protocol.deterministic_modules.leading_zero_work_v1.benchmark,
  );
  assert.strictEqual(terms.verification_policy.module_id, "leading_zero_work_v1");
  assert.strictEqual(terms.verification_policy.settlement_scope, "protocol_canary_only");

  const account = "0x2222222222222222222222222222222222222222";
  const bountyContract = "0x1111111111111111111111111111111111111111";
  const requestBody = {
    idempotency_key: "github-claim-comment:test",
    network: "base-mainnet",
    bounty_contract: bountyContract,
    solver_wallet: account,
    request_bond_sponsorship: true,
    source: "github-claim",
  };
  const typedData = {
    types: {
      EIP712Domain: [
        { name: "name", type: "string" },
        { name: "version", type: "string" },
        { name: "chainId", type: "uint256" },
        { name: "verifyingContract", type: "address" },
      ],
      TransferWithAuthorization: [
        { name: "from", type: "address" },
        { name: "to", type: "address" },
        { name: "value", type: "uint256" },
        { name: "validAfter", type: "uint256" },
        { name: "validBefore", type: "uint256" },
        { name: "nonce", type: "bytes32" },
      ],
    },
    domain: {
      name: "USD Coin",
      version: "2",
      chainId: 8453,
      verifyingContract: protocol.native_usdc,
    },
    primaryType: "TransferWithAuthorization",
    message: {
      from: account,
      to: bountyContract,
      value: "10000",
      validAfter: "0",
      validBefore: "1800000000",
      nonce: `0x${"33".repeat(32)}`,
    },
  };
  const handoff = {
    schema_version: "agent-bounties/agent-native-claim-v1",
    candidate: {
      status: "authorization_ready",
      bounty_contract: bountyContract,
      solver_wallet: account,
    },
    wallet_request: {
      method: "eth_signTypedData_v4",
      params: [account, JSON.stringify(typedData)],
    },
    next_request: {
      method: "POST",
      url: "https://api.agentbounties.app/v1/base/autonomous-bounties/claims",
      body: requestBody,
    },
  };
  const selected = { bounty_contract: bountyContract, claim_bond: "10000" };
  const walletRequest = context.__agentBountiesTest.validateHostedClaimHandoff(
    handoff,
    requestBody,
    selected,
    account,
    protocol,
    "https://api.agentbounties.app",
  );
  assert.strictEqual(walletRequest.method, "eth_signTypedData_v4");

  const tampered = JSON.parse(JSON.stringify(handoff));
  const tamperedTypedData = JSON.parse(tampered.wallet_request.params[1]);
  tamperedTypedData.message.to = "0x3333333333333333333333333333333333333333";
  tampered.wallet_request.params[1] = JSON.stringify(tamperedTypedData);
  assert.throws(
    () => context.__agentBountiesTest.validateHostedClaimHandoff(
      tampered,
      requestBody,
      selected,
      account,
      protocol,
      "https://api.agentbounties.app",
    ),
    /differs from the selected Base USDC bond/,
  );
}

testDeterministicPostingDefaults()
  .then(() => console.log("autonomous wallet flow contract passed"))
  .catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
