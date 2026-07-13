const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const { webcrypto } = require("crypto");

const repoRoot = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(repoRoot, "site", "autonomous.js"), "utf8");
const protocol = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "site", "protocol.json"), "utf8"),
);

new vm.Script(source, { filename: "site/autonomous.js" });

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
  "/v1/base/autonomous-bounties/authorized-contribution-plan",
  "/v1/base/autonomous-bounties/claim-plan",
  "/v1/base/autonomous-bounties/authorized-claim-plan",
  "/v1/base/autonomous-bounties/submission-plan",
  "FundingAdded",
  "BountyClaimed",
  "submission_added",
  "BountySettled",
  "default_verification",
  "configurePostVerification",
]) {
  assert(source.includes(required), `autonomous wallet flow missing ${required}`);
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
]) {
  assert(!source.includes(retired), `autonomous wallet flow contains retired behavior: ${retired}`);
}

assert.strictEqual(protocol.protocol_version, "agent-bounties/autonomous-v1");
assert.strictEqual(protocol.network, "base-mainnet");
assert.strictEqual(protocol.chain_id, 8453);
assert.strictEqual(protocol.status, "active");
assert.strictEqual(protocol.factory, "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9");
assert.strictEqual(protocol.implementation, "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9");
assert.strictEqual(protocol.default_verification.mode, "deterministic_module");
assert.strictEqual(protocol.default_verification.module_id, "leading_zero_work_v1");
assert.strictEqual(protocol.default_verification.verifier_reward_recipient, "creator_wallet");
assert.strictEqual(protocol.default_verification.threshold, 1);

const postHtml = fs.readFileSync(path.join(repoRoot, "site", "post.html"), "utf8");
assert(
  postHtml.indexOf('value="deterministic_module"') < postHtml.indexOf('value="signed_quorum"'),
  "public posting must default to deterministic verification",
);
assert(postHtml.includes("Verifier wallet quorum (advanced)"));

for (const page of ["index.html", "post.html", "funding.html", "earn.html", "operator.html"]) {
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
  const elements = {
    verificationMode: {
      value: "deterministic_module",
      addEventListener(name, handler) {
        controlListeners[name] = handler;
      },
    },
    verifierModule: { value: "", readOnly: false, disabled: false },
    verifierRewardRecipient: { value: "", disabled: false },
    verifiers: { value: "", disabled: false },
    threshold: { value: "8", readOnly: false },
  };
  const form = {
    id: "autonomous-post-form",
    elements,
    addEventListener(name, handler) {
      formListeners[name] = handler;
    },
    querySelector() {
      return null;
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
  new vm.Script(source, { filename: "site/autonomous.js" }).runInContext(context);
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

  elements.verificationMode.value = "signed_quorum";
  controlListeners.change();
  assert.strictEqual(elements.verifierModule.disabled, true);
  assert.strictEqual(elements.verifierRewardRecipient.disabled, true);
  assert.strictEqual(elements.verifiers.disabled, false);
  assert.strictEqual(elements.threshold.readOnly, false);

  elements.verificationMode.value = "deterministic_module";
  controlListeners.change();
  assert.strictEqual(elements.verifierModule.disabled, false);
  assert.strictEqual(elements.verifiers.disabled, true);
  assert.strictEqual(elements.threshold.value, "1");
}

testDeterministicPostingDefaults()
  .then(() => console.log("autonomous wallet flow contract passed"))
  .catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
