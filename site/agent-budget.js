(function () {
  "use strict";

  const state = {
    manifest: null,
    providers: [],
    provider: null,
    account: null,
    infrastructureChecked: false,
    factoryReady: false,
    plan: null,
    rotationPlan: null,
    busy: false,
    legalAction: null,
  };
  const announcedProviders = [];
  const CHAIN_ID = "0x2105";
  const ZERO_HASH = `0x${"00".repeat(32)}`;
  const EXPECTED = Object.freeze({
    sourceRevision: "dc05b4e01474f09f02bb1bbb69651e4ce4deb338",
    bountyFactory: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
    settlementToken: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    deterministicVerifier: "0xe573cb4f471d38b5bf10ce82237251ac902c9867",
    signedQuorumVerifierSetHash: "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd",
    deterministicDeployer: "0x4e59b44847b379578588920ca78fbf26c0b4956c",
    deterministicDeployerHash: "0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989",
    walletFactory: "0x3840936351049aed639780a16845e6094c1f17f6",
    implementation: "0x40d3e16082cf71ece0129ca3044e1b8233e29db8",
    factoryRuntimeHash: "0x243e248a890daf57cb14cee262bc7bb70b8822c65a014a8bf1c39653bc30aa52",
    implementationRuntimeHash: "0x7fb59d5add3ac348ac3d7e6a5aa6b22ad542a6e6093a1ceb8d535f747ed536df",
    cloneRuntimeHash: "0xc663bed9b4097e22e5a18c0ecb662561bf45df1829e6412cdd0d8568d05ca1b6",
  });
  const OBSOLETE_DETERMINISTIC_VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e";
  const SELECTORS = Object.freeze({
    predictWallet: "0x240fa116",
    createAndFund: "0x86f357d0",
    createWithAuthorization: "0x9b2065e0",
    isFactoryWallet: "0xf48f2346",
    implementation: "0x5c60da1b",
    bountyFactory: "0xb8f75c0b",
    settlementToken: "0x7b9e618d",
    owner: "0x8da5cb5b",
    policy: "0x0505c8c9",
    policyVersion: "0x58355ead",
    lifetimeSpent: "0xb80762dd",
    periodSpent: "0x81497000",
    configurePolicy: "0x27d3543c",
    revokePolicy: "0x9eba3667",
    balanceOf: "0x70a08231",
    allowance: "0xdd62ed3e",
    approve: "0x095ea7b3",
  });
  const CLONE_PREFIX = "3d602d80600a3d3981f3" + "363d3d373d3d3d363d73";
  const CLONE_SUFFIX = "5af43d82803e903d91602b57fd5bf3";

  const byId = (id) => document.getElementById(id);
  const form = () => byId("agent-budget-form");
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function output(lines, tone = "") {
    const element = byId("agent-budget-output");
    element.textContent = Array.isArray(lines) ? lines.join("\n") : lines;
    element.dataset.tone = tone;
  }

  function status(message, tone = "") {
    const element = byId("budget-status");
    element.textContent = message;
    element.dataset.tone = tone;
  }

  function isProvider(provider) {
    return Boolean(provider && typeof provider.request === "function");
  }

  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    if (item.provider.isMetaMask) return "MetaMask";
    if (item.provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (item.provider.isBraveWallet) return "Brave Wallet";
    return "Browser wallet";
  }

  function rememberProvider(event) {
    const detail = event && event.detail;
    if (!detail || !isProvider(detail.provider)) return;
    if (!announcedProviders.some((item) => item.provider === detail.provider)) announcedProviders.push(detail);
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(250);
    const providers = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const provider of injected) {
      if (isProvider(provider) && !providers.some((item) => item.provider === provider)) {
        providers.push({ provider, info: {} });
      }
    }
    state.providers = providers;
    const selector = document.querySelector("[data-wallet-provider]");
    selector.replaceChildren();
    if (providers.length === 0) {
      const option = document.createElement("option");
      option.textContent = "No browser wallet detected";
      selector.append(option);
      selector.disabled = true;
      return;
    }
    providers.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      selector.append(option);
    });
    selector.disabled = false;
  }

  function selectProvider() {
    const index = Number.parseInt(document.querySelector("[data-wallet-provider]").value, 10);
    const item = state.providers[index];
    if (!item) throw new Error("Unlock a browser wallet, reload, and select it.");
    state.provider = item.provider;
    return item.provider;
  }

  async function request(method, params = []) {
    const provider = state.provider || selectProvider();
    if (["eth_signTypedData_v4", "eth_sendTransaction", "wallet_sendCalls"].includes(method)) {
      if (!state.legalAction || !window.AgentBountiesLegal) {
        throw new Error("Review and accept the legal agreement before this wallet action.");
      }
      await window.AgentBountiesLegal.requireAcceptance({
        action: state.legalAction,
        walletAddress: state.account,
        scope: form(),
      });
    }
    return provider.request({ method, params });
  }

  async function acceptLegalAction(action) {
    if (!window.AgentBountiesLegal) {
      throw new Error("The legal agreement could not be loaded. Reload before using the wallet.");
    }
    state.legalAction = action;
    return window.AgentBountiesLegal.requireAcceptance({
      action,
      walletAddress: state.account,
      scope: form(),
    });
  }

  async function loadManifest() {
    if (state.manifest) return state.manifest;
    const urls = [
      "bounded-agent-wallet-base-mainnet.json",
      "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/deployments/bounded-agent-wallet-base-mainnet.json",
    ];
    let manifest = null;
    for (const url of urls) {
      try {
        const response = await fetch(url, { cache: "no-store" });
        if (response.ok) {
          manifest = await response.json();
          break;
        }
      } catch (_error) {
        // Try the source-controlled fallback.
      }
    }
    if (!manifest || manifest.schema !== "agent-bounties/bounded-agent-wallet-deployment-v1") {
      throw new Error("The reviewed bounded-wallet deployment manifest is unavailable.");
    }
    if (manifest.chain_id !== 8453 || manifest.network !== "base-mainnet") {
      throw new Error("The bounded-wallet manifest is not pinned to Base mainnet.");
    }
    if (manifest.contract_source_dirty !== false
      || !/^[0-9a-f]{40}$/i.test(String(manifest.contract_source_revision || ""))) {
      throw new Error("The bounded-wallet manifest is stale or was generated from uncommitted contract source.");
    }
    if (manifest.contract_source_revision_kind !== "git-tree") {
      throw new Error("The bounded-wallet manifest does not use a content-addressed source tree.");
    }
    const pinned = [
      [manifest.contract_source_revision, EXPECTED.sourceRevision, "source revision"],
      [manifest.canonical && manifest.canonical.bounty_factory, EXPECTED.bountyFactory, "bounty factory"],
      [manifest.canonical && manifest.canonical.settlement_token, EXPECTED.settlementToken, "settlement token"],
      [manifest.canonical && manifest.canonical.deterministic_verifier, EXPECTED.deterministicVerifier, "verifier"],
      [manifest.canonical && manifest.canonical.signed_quorum_verifier_set_hash, EXPECTED.signedQuorumVerifierSetHash, "signed quorum"],
      [manifest.deterministic_deployer && manifest.deterministic_deployer.address, EXPECTED.deterministicDeployer, "deployer"],
      [manifest.deterministic_deployer && manifest.deterministic_deployer.runtime_code_hash, EXPECTED.deterministicDeployerHash, "deployer runtime"],
      [manifest.wallet_factory && manifest.wallet_factory.address, EXPECTED.walletFactory, "wallet factory"],
      [manifest.wallet_factory && manifest.wallet_factory.implementation, EXPECTED.implementation, "implementation"],
      [manifest.wallet_factory && manifest.wallet_factory.runtime_code_hash, EXPECTED.factoryRuntimeHash, "factory runtime"],
      [manifest.wallet_factory && manifest.wallet_factory.implementation_runtime_code_hash, EXPECTED.implementationRuntimeHash, "implementation runtime"],
      [manifest.wallet_factory && manifest.wallet_factory.clone_runtime_code_hash, EXPECTED.cloneRuntimeHash, "clone runtime"],
    ];
    for (const [actual, expected, label] of pinned) {
      if (String(actual || "").toLowerCase() !== expected) {
        throw new Error(`The bounded-wallet manifest ${label} differs from this reviewed activation page.`);
      }
    }
    state.manifest = manifest;
    return manifest;
  }

  function requiredAddress(value, label) {
    const normalized = String(value || "").trim().toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(normalized) || normalized === `0x${"00".repeat(20)}`) {
      throw new Error(`${label} must be a non-zero EVM address.`);
    }
    return normalized;
  }

  function strip0x(value) {
    return String(value).replace(/^0x/, "").toLowerCase();
  }

  function uintWord(value) {
    const number = BigInt(value);
    if (number < 0n || number >= (1n << 256n)) throw new Error("Integer is outside uint256.");
    return number.toString(16).padStart(64, "0");
  }

  function addressWord(value) {
    return strip0x(requiredAddress(value, "Address")).padStart(64, "0");
  }

  function bytes32Word(value) {
    const normalized = strip0x(value);
    if (!/^[0-9a-f]{64}$/.test(normalized)) throw new Error("Expected a 32-byte value.");
    return normalized;
  }

  function resultAddress(value) {
    const raw = strip0x(value);
    if (raw.length < 64) throw new Error("Wallet returned an invalid address result.");
    return requiredAddress(`0x${raw.slice(-40)}`, "Returned address");
  }

  function resultUint(value) {
    return BigInt(value);
  }

  function randomBytes32() {
    const value = new Uint8Array(32);
    crypto.getRandomValues(value);
    return `0x${Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function usdcUnits(value, label) {
    const match = String(value).trim().match(/^(\d+)(?:\.(\d{1,6}))?$/);
    if (!match) throw new Error(`${label} must have at most six decimal places.`);
    const units = BigInt(match[1]) * 1_000_000n + BigInt((match[2] || "").padEnd(6, "0"));
    if (units <= 0n) throw new Error(`${label} must be positive.`);
    return units;
  }

  function snapshot() {
    const data = new FormData(form());
    return JSON.stringify({
      delegate: data.get("delegate"),
      initialFunding: data.get("initialFunding"),
      maxPerAction: data.get("maxPerAction"),
      maxPerPeriod: data.get("maxPerPeriod"),
      maxLifetime: data.get("maxLifetime"),
      maxBountyTarget: data.get("maxBountyTarget"),
      expiryDays: data.get("expiryDays"),
    });
  }

  function policyWords(policy) {
    return [
      addressWord(policy.delegate),
      uintWord(policy.validAfter),
      uintWord(policy.validUntil),
      uintWord(policy.periodSeconds),
      uintWord(policy.maxPerAction),
      uintWord(policy.maxPerPeriod),
      uintWord(policy.maxLifetimeSpend),
      uintWord(policy.maxBountyTarget),
      uintWord(policy.allowedActions),
      uintWord(policy.allowedVerificationModes),
      addressWord(policy.deterministicVerifierModule),
      bytes32Word(policy.signedQuorumVerifierSetHash),
      bytes32Word(policy.aiJudgeVerifierSetHash),
    ];
  }

  function exactPolicyWords(value) {
    const raw = strip0x(value);
    if (!/^[0-9a-f]{832}$/.test(raw)) throw new Error("Bounded wallet returned invalid policy encoding.");
    return Array.from({ length: 13 }, (_, index) => raw.slice(index * 64, (index + 1) * 64));
  }

  function wordAddress(word, label) {
    if (!/^0{24}[0-9a-f]{40}$/.test(word)) throw new Error(`${label} has noncanonical address encoding.`);
    return requiredAddress(`0x${word.slice(24)}`, label);
  }

  function wordUint(word) {
    return BigInt(`0x${word}`);
  }

  async function hash(value) {
    const result = await request("web3_sha3", [value]);
    if (!/^0x[0-9a-fA-F]{64}$/.test(result || "")) throw new Error("Wallet provider cannot hash bytecode.");
    return result.toLowerCase();
  }

  async function call(to, data) {
    return request("eth_call", [{ to, data }, "latest"]);
  }

  async function latestTimestamp() {
    const block = await request("eth_getBlockByNumber", ["latest", false]);
    const timestamp = block && typeof block.timestamp === "string" ? Number.parseInt(block.timestamp, 16) : NaN;
    if (!Number.isSafeInteger(timestamp) || timestamp <= 0) throw new Error("Wallet provider returned an invalid Base block.");
    return timestamp;
  }

  async function codeHash(address) {
    const code = String(await request("eth_getCode", [address, "latest"])).toLowerCase();
    return { code, hash: code === "0x" || code === "0x0" ? null : await hash(code) };
  }

  async function switchToBase() {
    const current = String(await request("eth_chainId")).toLowerCase();
    if (current === CHAIN_ID) return;
    try {
      await request("wallet_switchEthereumChain", [{ chainId: CHAIN_ID }]);
    } catch (error) {
      if (!error || error.code !== 4902) throw error;
      await request("wallet_addEthereumChain", [{
        chainId: CHAIN_ID,
        chainName: "Base",
        nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
        rpcUrls: ["https://mainnet.base.org"],
        blockExplorerUrls: ["https://basescan.org"],
      }]);
    }
  }

  async function connect() {
    selectProvider();
    await loadManifest();
    const accounts = await request("eth_requestAccounts");
    if (!accounts || !accounts[0]) throw new Error("The wallet returned no account.");
    await switchToBase();
    state.account = requiredAddress(accounts[0], "Owner account");
    state.plan = null;
    state.rotationPlan = null;
    state.infrastructureChecked = false;
    status(`Connected ${state.account.slice(0, 8)}...${state.account.slice(-4)}`, "success");
    byId("inspect-budget-factory").disabled = false;
    byId("review-agent-budget").disabled = false;
    restoreKnownWallet();
    updateButtons();
    output("Connected on Base. Inspect infrastructure, then review the exact policy-bound wallet.", "success");
  }

  async function ensureConnectedOwner() {
    await switchToBase();
    const accounts = await request("eth_accounts");
    const active = requiredAddress(accounts && accounts[0], "Active owner account");
    if (active !== state.account
      || (state.plan && state.plan.account !== active)
      || (state.rotationPlan && state.rotationPlan.account !== active)) {
      throw new Error("The active wallet account changed. Reconnect and review the policy again.");
    }
    return active;
  }

  async function inspectInfrastructure() {
    const manifest = await loadManifest();
    if (!state.account) throw new Error("Connect the owner wallet first.");
    await switchToBase();
    const deployer = await codeHash(manifest.deterministic_deployer.address);
    if (deployer.hash !== manifest.deterministic_deployer.runtime_code_hash) {
      throw new Error("The deterministic deployer bytecode does not match the reviewed manifest.");
    }
    for (const address of [
      manifest.canonical.bounty_factory,
      manifest.canonical.settlement_token,
      manifest.canonical.deterministic_verifier,
    ]) {
      const observed = await codeHash(address);
      if (!observed.hash) throw new Error(`Required canonical contract is unavailable: ${address}`);
    }
    const factory = await codeHash(manifest.wallet_factory.address);
    if (!factory.hash) {
      state.factoryReady = false;
      state.infrastructureChecked = true;
      output([
        "Canonical bounty contracts and deterministic deployer verified.",
        `Bounded wallet factory is not deployed yet: ${manifest.wallet_factory.address}`,
        "Activation will first request the manifest's exact zero-value deployment transaction.",
      ], "pending");
      updateButtons();
      return false;
    }
    if (factory.hash !== manifest.wallet_factory.runtime_code_hash) {
      throw new Error("Bounded wallet factory runtime does not match the reviewed manifest.");
    }
    const implementation = await codeHash(manifest.wallet_factory.implementation);
    if (implementation.hash !== manifest.wallet_factory.implementation_runtime_code_hash) {
      throw new Error("Bounded wallet implementation runtime does not match the reviewed manifest.");
    }
    const observedImplementation = resultAddress(await call(manifest.wallet_factory.address, SELECTORS.implementation));
    const observedFactory = resultAddress(await call(manifest.wallet_factory.address, SELECTORS.bountyFactory));
    const observedToken = resultAddress(await call(manifest.wallet_factory.address, SELECTORS.settlementToken));
    if (observedImplementation !== manifest.wallet_factory.implementation
      || observedFactory !== manifest.canonical.bounty_factory
      || observedToken !== manifest.canonical.settlement_token) {
      throw new Error("Bounded wallet factory immutable bindings do not match the manifest.");
    }
    state.factoryReady = true;
    state.infrastructureChecked = true;
    output(`Reviewed bounded wallet factory verified: ${manifest.wallet_factory.address}`, "success");
    updateButtons();
    return true;
  }

  function buildPolicy(chainTimestamp) {
    const values = new FormData(form());
    const delegate = requiredAddress(values.get("delegate"), "Agent delegate");
    if (delegate === state.account) throw new Error("Use a dedicated delegate address, not the owner wallet.");
    const initialFunding = usdcUnits(values.get("initialFunding"), "Initial funding");
    const maxPerAction = usdcUnits(values.get("maxPerAction"), "Per-action cap");
    const maxPerPeriod = usdcUnits(values.get("maxPerPeriod"), "Period cap");
    const maxLifetimeSpend = usdcUnits(values.get("maxLifetime"), "Lifetime cap");
    const maxBountyTarget = usdcUnits(values.get("maxBountyTarget"), "Bounty target cap");
    const expiryDays = Number.parseInt(values.get("expiryDays"), 10);
    if (!Number.isInteger(expiryDays) || expiryDays < 1 || expiryDays > 30) {
      throw new Error("Expiry must be between 1 and 30 days.");
    }
    if (initialFunding > maxLifetimeSpend) throw new Error("Initial funding exceeds lifetime authority.");
    if (maxPerAction > maxPerPeriod || maxPerPeriod > maxLifetimeSpend) {
      throw new Error("Caps must satisfy per action <= per 24 hours <= lifetime.");
    }
    return {
      initialFunding,
      policy: {
        delegate,
        validAfter: BigInt(chainTimestamp),
        validUntil: BigInt(chainTimestamp + expiryDays * 86_400),
        periodSeconds: 86_400n,
        maxPerAction,
        maxPerPeriod,
        maxLifetimeSpend,
        maxBountyTarget,
        allowedActions: 15n,
        allowedVerificationModes: 3n,
        deterministicVerifierModule: state.manifest.canonical.deterministic_verifier,
        signedQuorumVerifierSetHash: state.manifest.canonical.signed_quorum_verifier_set_hash,
        aiJudgeVerifierSetHash: ZERO_HASH,
      },
    };
  }

  async function deriveWallet(policy, userSalt) {
    const manifest = state.manifest;
    const encodedPolicy = `0x${policyWords(policy).join("")}`;
    const policyHash = await hash(encodedPolicy);
    const effectiveSalt = await hash(`0x${addressWord(state.account)}${bytes32Word(userSalt)}${bytes32Word(policyHash)}`);
    const cloneInitCode = `0x${CLONE_PREFIX}${strip0x(manifest.wallet_factory.implementation)}${CLONE_SUFFIX}`;
    const cloneInitCodeHash = await hash(cloneInitCode);
    const create2Hash = await hash(
      `0xff${strip0x(manifest.wallet_factory.address)}${strip0x(effectiveSalt)}${strip0x(cloneInitCodeHash)}`,
    );
    return { wallet: `0x${strip0x(create2Hash).slice(-40)}`, policyHash, effectiveSalt, encodedPolicy };
  }

  async function reviewPlan() {
    if (!state.account) throw new Error("Connect the owner wallet first.");
    if (!state.infrastructureChecked) throw new Error("Inspect infrastructure first.");
    await ensureConnectedOwner();
    const { initialFunding, policy } = buildPolicy(await latestTimestamp());
    const userSalt = randomBytes32();
    const derived = await deriveWallet(policy, userSalt);
    const balanceData = `${SELECTORS.balanceOf}${addressWord(state.account)}`;
    const ownerBalance = resultUint(await call(state.manifest.canonical.settlement_token, balanceData));
    if (ownerBalance < initialFunding) {
      throw new Error(`Owner has ${Number(ownerBalance) / 1_000_000} USDC; ${Number(initialFunding) / 1_000_000} is required.`);
    }
    if (state.factoryReady) {
      const predicted = resultAddress(await call(
        state.manifest.wallet_factory.address,
        `${SELECTORS.predictWallet}${addressWord(state.account)}${policyWords(policy).join("")}${bytes32Word(userSalt)}`,
      ));
      if (predicted !== derived.wallet) throw new Error("Local and on-chain wallet predictions differ.");
    }
    const ownerCode = String(await request("eth_getCode", [state.account, "latest"])).toLowerCase();
    const ownerIsContract = ownerCode !== "0x" && ownerCode !== "0x0";
    state.plan = {
      account: state.account,
      snapshot: snapshot(),
      initialFunding,
      policy,
      policyWords: policyWords(policy),
      userSalt,
      ownerIsContract,
      ...derived,
    };
    form().elements.existingWallet.value = derived.wallet;
    output([
      `Predicted policy-bound wallet: ${derived.wallet}`,
      `Delegate: ${policy.delegate}`,
      `Initial / lifetime: ${Number(initialFunding) / 1_000_000} / ${Number(policy.maxLifetimeSpend) / 1_000_000} USDC`,
      `Per action / 24 hours: ${Number(policy.maxPerAction) / 1_000_000} / ${Number(policy.maxPerPeriod) / 1_000_000} USDC`,
      `Policy hash: ${derived.policyHash}`,
      ownerIsContract
        ? "Smart-account activation requires one exact USDC approval and one factory transaction; future in-policy actions do not require the owner."
        : "EOA activation requires one funding authorization and one gas transaction; future in-policy actions do not require the owner.",
    ], "pending");
    updateButtons();
  }

  async function sendTransaction(to, data) {
    return request("eth_sendTransaction", [{ from: state.account, to, data, value: "0x0" }]);
  }

  async function waitReceipt(hashValue, timeoutMs = 180_000) {
    const started = Date.now();
    while (Date.now() - started < timeoutMs) {
      const receipt = await request("eth_getTransactionReceipt", [hashValue]);
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Transaction reverted: ${hashValue}`);
        return receipt;
      }
      await sleep(1_500);
    }
    throw new Error(`Transaction confirmation timed out: ${hashValue}`);
  }

  async function ownerFactoryAllowance() {
    const data = `${SELECTORS.allowance}${addressWord(state.account)}${addressWord(state.manifest.wallet_factory.address)}`;
    return resultUint(await call(state.manifest.canonical.settlement_token, data));
  }

  async function activateWithAllowance(plan) {
    const factory = state.manifest.wallet_factory.address;
    const token = state.manifest.canonical.settlement_token;
    let allowance = await ownerFactoryAllowance();
    let approvalHash = null;
    if (allowance !== plan.initialFunding) {
      output([
        `Confirm an exact ${Number(plan.initialFunding) / 1_000_000} USDC approval to the reviewed factory.`,
        `Factory: ${factory}`,
        "This approval cannot create, fund, or settle a bounty by itself.",
      ], "pending");
      await ensureConnectedOwner();
      const approvalData = `${SELECTORS.approve}${addressWord(factory)}${uintWord(plan.initialFunding)}`;
      approvalHash = await sendTransaction(token, approvalData);
      await waitReceipt(approvalHash);
      allowance = await ownerFactoryAllowance();
      if (allowance !== plan.initialFunding) throw new Error("Confirmed USDC allowance differs from the reviewed amount.");
    }
    const activationData = `${SELECTORS.createAndFund}`
      + `${plan.policyWords.join("")}${bytes32Word(plan.userSalt)}${uintWord(plan.initialFunding)}`;
    output([
      "Approval confirmed. Confirm the zero-value factory transaction that deploys and funds only the reviewed wallet.",
      `Wallet: ${plan.wallet}`,
      `Policy hash: ${plan.policyHash}`,
    ], "pending");
    await ensureConnectedOwner();
    const transactionHash = await sendTransaction(factory, activationData);
    await waitReceipt(transactionHash);
    const remainingAllowance = await ownerFactoryAllowance();
    if (remainingAllowance !== 0n) throw new Error("Factory allowance was not fully consumed by activation.");
    return { transactionHash, approvalHash };
  }

  async function deployFactory() {
    const manifest = state.manifest;
    output("Confirm the exact zero-value bounded-wallet factory deployment in your wallet.", "pending");
    const txHash = await sendTransaction(
      manifest.deterministic_deployer.address,
      manifest.wallet_factory.deployment_transaction,
    );
    await waitReceipt(txHash);
    state.infrastructureChecked = false;
    const ready = await inspectInfrastructure();
    if (!ready) throw new Error("Factory deployment confirmed but reviewed bytecode is unavailable.");
  }

  function signatureParts(signature) {
    const raw = strip0x(signature);
    if (!/^[0-9a-f]{130}$/.test(raw)) throw new Error("Wallet returned an invalid 65-byte signature.");
    let v = Number.parseInt(raw.slice(128, 130), 16);
    if (v < 27) v += 27;
    if (v !== 27 && v !== 28) throw new Error("Wallet returned an unsupported recovery id.");
    return { r: `0x${raw.slice(0, 64)}`, s: `0x${raw.slice(64, 128)}`, v };
  }

  async function inspectActivatedWallet(plan) {
    const manifest = state.manifest;
    const walletCode = await codeHash(plan.wallet);
    if (walletCode.hash !== manifest.wallet_factory.clone_runtime_code_hash) {
      throw new Error("Activated wallet runtime does not match the reviewed clone.");
    }
    const registered = resultUint(await call(
      manifest.wallet_factory.address,
      `${SELECTORS.isFactoryWallet}${addressWord(plan.wallet)}`,
    ));
    if (registered !== 1n) throw new Error("Activated wallet is not registered by the reviewed factory.");
    const owner = resultAddress(await call(plan.wallet, SELECTORS.owner));
    if (owner !== state.account) throw new Error("Activated wallet owner does not match the connected wallet.");
    const observedPolicy = String(await call(plan.wallet, SELECTORS.policy)).toLowerCase();
    if (observedPolicy !== plan.encodedPolicy.toLowerCase()) throw new Error("Activated wallet policy differs from review.");
    const balance = resultUint(await call(
      manifest.canonical.settlement_token,
      `${SELECTORS.balanceOf}${addressWord(plan.wallet)}`,
    ));
    if (balance < plan.initialFunding) throw new Error("Activated wallet did not receive the authorized USDC.");
    return balance;
  }

  async function activate(event) {
    event.preventDefault();
    if (state.busy) return;
    if (!state.plan) throw new Error("Review the exact wallet before activation.");
    if (state.plan.snapshot !== snapshot()) {
      state.plan = null;
      updateButtons();
      throw new Error("Policy fields changed. Review the exact wallet again.");
    }
    if (!form().elements.reviewed.checked) throw new Error("Confirm that you reviewed the policy.");
    state.busy = true;
    updateButtons();
    try {
      await ensureConnectedOwner();
      await acceptLegalAction("activate_agent_budget");
      if (!state.factoryReady) await deployFactory();
      const predicted = resultAddress(await call(
        state.manifest.wallet_factory.address,
        `${SELECTORS.predictWallet}${addressWord(state.account)}${state.plan.policyWords.join("")}${bytes32Word(state.plan.userSalt)}`,
      ));
      if (predicted !== state.plan.wallet) throw new Error("On-chain prediction changed after deployment.");
      const ownerCode = String(await request("eth_getCode", [state.account, "latest"])).toLowerCase();
      const ownerIsContract = ownerCode !== "0x" && ownerCode !== "0x0";
      if (ownerIsContract !== state.plan.ownerIsContract) throw new Error("Owner account type changed. Review the exact wallet again.");
      let txHash;
      let approvalHash = null;
      if (ownerIsContract) {
        ({ transactionHash: txHash, approvalHash } = await activateWithAllowance(state.plan));
      } else {
        const now = await latestTimestamp();
        const validAfter = Math.max(0, now - 1);
        const validBefore = now + 1_800;
        const nonce = randomBytes32();
        const typedData = {
          types: window.AgentBountiesEvm.transferWithAuthorizationTypes(),
          primaryType: "TransferWithAuthorization",
          domain: {
            name: "USD Coin",
            version: "2",
            chainId: 8453,
            verifyingContract: state.manifest.canonical.settlement_token,
          },
          message: {
            from: state.account,
            to: state.plan.wallet,
            value: state.plan.initialFunding.toString(),
            validAfter: String(validAfter),
            validBefore: String(validBefore),
            nonce,
          },
        };
        output("Review one USDC authorization. It is bound to the exact policy-derived wallet and amount.", "pending");
        const signature = await request("eth_signTypedData_v4", [state.account, JSON.stringify(typedData)]);
        const parts = signatureParts(signature);
        const activationData = `${SELECTORS.createWithAuthorization}`
          + `${addressWord(state.account)}${state.plan.policyWords.join("")}${bytes32Word(state.plan.userSalt)}`
          + `${uintWord(state.plan.initialFunding)}${uintWord(validAfter)}${uintWord(validBefore)}${bytes32Word(nonce)}`
          + `${uintWord(parts.v)}${bytes32Word(parts.r)}${bytes32Word(parts.s)}`;
        output("Authorization signed. Confirm the zero-value gas transaction that atomically deploys and funds the wallet.", "pending");
        await ensureConnectedOwner();
        txHash = await sendTransaction(state.manifest.wallet_factory.address, activationData);
        await waitReceipt(txHash);
      }
      const balance = await inspectActivatedWallet(state.plan);
      form().elements.existingWallet.value = state.plan.wallet;
      localStorage.setItem("agent-bounties-bounded-wallet", JSON.stringify({ owner: state.account, wallet: state.plan.wallet }));
      byId("revoke-agent-budget").disabled = false;
      status("Agent budget active", "success");
      output([
        `Bounded wallet active: ${state.plan.wallet}`,
        `Confirmed wallet balance: ${Number(balance) / 1_000_000} USDC`,
        ...(approvalHash ? [`Exact approval transaction: ${approvalHash}`] : []),
        `Activation transaction: ${txHash}`,
        "The delegate may now execute in-policy canonical actions without another owner prompt. Revoke here at any time.",
      ], "success");
    } finally {
      state.busy = false;
      updateButtons();
    }
  }

  function restoreKnownWallet() {
    try {
      const known = JSON.parse(localStorage.getItem("agent-bounties-bounded-wallet") || "null");
      if (known && requiredAddress(known.owner, "Stored owner") === state.account) {
        form().elements.existingWallet.value = requiredAddress(known.wallet, "Stored wallet");
        byId("revoke-agent-budget").disabled = false;
      }
    } catch (_error) {
      localStorage.removeItem("agent-bounties-bounded-wallet");
    }
  }

  async function inspectExistingWallet(wallet) {
    const runtime = await codeHash(wallet);
    if (runtime.hash !== state.manifest.wallet_factory.clone_runtime_code_hash) {
      throw new Error("Existing wallet runtime does not match the reviewed bounded-wallet clone.");
    }
    const registered = resultUint(await call(
      state.manifest.wallet_factory.address,
      `${SELECTORS.isFactoryWallet}${addressWord(wallet)}`,
    ));
    if (registered !== 1n) throw new Error("Existing wallet is not registered by the reviewed factory.");
    const owner = resultAddress(await call(wallet, SELECTORS.owner));
    if (owner !== state.account) throw new Error("Connected account is not this bounded wallet's owner.");
    const policyEncoded = String(await call(wallet, SELECTORS.policy)).toLowerCase();
    const policy = exactPolicyWords(policyEncoded);
    const version = resultUint(await call(wallet, SELECTORS.policyVersion));
    const lifetimeSpent = resultUint(await call(wallet, SELECTORS.lifetimeSpent));
    const periodSpent = resultUint(await call(wallet, SELECTORS.periodSpent));
    const balance = resultUint(await call(
      state.manifest.canonical.settlement_token,
      `${SELECTORS.balanceOf}${addressWord(wallet)}`,
    ));
    return { policyEncoded, policy, version, lifetimeSpent, periodSpent, balance };
  }

  function rotationSnapshot() {
    return JSON.stringify({
      wallet: form().elements.existingWallet.value,
      delegate: form().elements.delegate.value,
    });
  }

  async function reviewRotation() {
    if (!state.account) throw new Error("Connect the owner wallet first.");
    if (!state.infrastructureChecked || !state.factoryReady) throw new Error("Inspect infrastructure first.");
    await ensureConnectedOwner();
    const wallet = requiredAddress(form().elements.existingWallet.value, "Bounded wallet");
    const delegate = requiredAddress(form().elements.delegate.value, "Replacement delegate");
    if (delegate === state.account) throw new Error("Use a dedicated delegate address, not the owner wallet.");
    const observed = await inspectExistingWallet(wallet);
    const currentDelegate = wordAddress(observed.policy[0], "Current delegate");
    const currentVerifier = wordAddress(observed.policy[10], "Deterministic verifier");
    const currentModes = wordUint(observed.policy[9]);
    const currentSignedQuorum = `0x${observed.policy[11]}`;
    const currentAiQuorum = `0x${observed.policy[12]}`;
    const nextVerifier = state.manifest.canonical.deterministic_verifier;
    if (currentVerifier !== nextVerifier && currentVerifier !== OBSOLETE_DETERMINISTIC_VERIFIER) {
      throw new Error("Existing policy uses an unknown verifier; this page will not replace it.");
    }
    if (![1n, 3n].includes(currentModes)
      || (currentModes === 1n && currentSignedQuorum !== ZERO_HASH)
      || (currentModes === 3n && currentSignedQuorum !== EXPECTED.signedQuorumVerifierSetHash)
      || currentAiQuorum !== ZERO_HASH) {
      throw new Error("Existing policy uses unknown verification authority; this page will not replace it.");
    }
    const verifierChanged = currentVerifier !== nextVerifier;
    const signedQuorumChanged = currentModes !== 3n
      || currentSignedQuorum !== EXPECTED.signedQuorumVerifierSetHash;
    if (delegate === currentDelegate && !verifierChanged && !signedQuorumChanged) {
      throw new Error("The replacement delegate and reviewed verifier policies are already active.");
    }
    const now = BigInt(await latestTimestamp());
    if (wordUint(observed.policy[2]) <= now) throw new Error("Existing policy is expired; create a new reviewed policy instead.");
    const nextPolicy = [...observed.policy];
    nextPolicy[0] = addressWord(delegate);
    nextPolicy[9] = uintWord(3n);
    nextPolicy[10] = addressWord(nextVerifier);
    nextPolicy[11] = bytes32Word(EXPECTED.signedQuorumVerifierSetHash);
    const nextEncoded = `0x${nextPolicy.join("")}`;
    const currentHash = await hash(observed.policyEncoded);
    const nextHash = await hash(nextEncoded);
    state.rotationPlan = {
      account: state.account,
      wallet,
      delegate,
      currentDelegate,
      currentVerifier,
      nextVerifier,
      verifierChanged,
      signedQuorumChanged,
      currentPolicy: observed.policyEncoded,
      currentHash,
      nextPolicy: nextEncoded,
      nextHash,
      currentVersion: observed.version,
      lifetimeSpent: observed.lifetimeSpent,
      periodSpent: observed.periodSpent,
      balance: observed.balance,
      snapshot: rotationSnapshot(),
      data: `${SELECTORS.configurePolicy}${nextPolicy.join("")}`,
    };
    output([
      `Bounded wallet: ${wallet}`,
      `Current delegate: ${currentDelegate}`,
      `Replacement delegate: ${delegate}`,
      `Current verifier: ${currentVerifier}`,
      `Reviewed verifier: ${nextVerifier}`,
      `Reviewed regression verifier set: ${EXPECTED.signedQuorumVerifierSetHash}`,
      `Current / next policy version: ${observed.version} / ${observed.version + 1n}`,
      `Current / next policy hash: ${currentHash} / ${nextHash}`,
      `Wallet balance remains ${Number(observed.balance) / 1_000_000} USDC.`,
      "The next policy permits only the existing deterministic module and the exact two-wallet sandboxed-regression quorum. AI-judge authority remains disabled.",
      `Lifetime spend remains ${Number(observed.lifetimeSpent) / 1_000_000} USDC. The deployed wallet starts a fresh policy-period counter; current period spend is ${Number(observed.periodSpent) / 1_000_000} USDC.`,
      "All financial caps, actions, expiry, owner, and wallet balance remain unchanged.",
      "The transaction transfers no USDC or ETH.",
    ], "pending");
    updateButtons();
  }

  async function rotateDelegate() {
    if (!state.rotationPlan) throw new Error("Review the exact delegate rotation first.");
    if (!form().elements.rotationReviewed.checked) throw new Error("Confirm the exact reviewed policy change first.");
    if (rotationSnapshot() !== state.rotationPlan.snapshot) {
      state.rotationPlan = null;
      throw new Error("Wallet or delegate changed. Review the rotation again.");
    }
    state.busy = true;
    updateButtons();
    try {
      await ensureConnectedOwner();
      await acceptLegalAction("update_agent_policy");
      const before = await inspectExistingWallet(state.rotationPlan.wallet);
      if (before.policyEncoded !== state.rotationPlan.currentPolicy
        || before.version !== state.rotationPlan.currentVersion
        || before.lifetimeSpent !== state.rotationPlan.lifetimeSpent
        || before.periodSpent !== state.rotationPlan.periodSpent
        || before.balance !== state.rotationPlan.balance) {
        throw new Error("Bounded wallet state changed after review. Review the rotation again.");
      }
      await request("eth_call", [{
        from: state.account,
        to: state.rotationPlan.wallet,
        data: state.rotationPlan.data,
        value: "0x0",
      }, "latest"]);
      output([
        "Confirm one zero-value owner transaction.",
        `Wallet: ${state.rotationPlan.wallet}`,
        `Replacement delegate: ${state.rotationPlan.delegate}`,
        `Reviewed verifier: ${state.rotationPlan.nextVerifier}`,
        `Reviewed regression verifier set: ${EXPECTED.signedQuorumVerifierSetHash}`,
        `Next policy hash: ${state.rotationPlan.nextHash}`,
        "The policy transaction starts a fresh policy-period spend counter; lifetime spend is preserved.",
        "No token or native-ETH transfer is requested.",
      ], "pending");
      const transactionHash = await sendTransaction(state.rotationPlan.wallet, state.rotationPlan.data);
      await waitReceipt(transactionHash);
      const after = await inspectExistingWallet(state.rotationPlan.wallet);
      if (after.policyEncoded !== state.rotationPlan.nextPolicy
        || after.version !== state.rotationPlan.currentVersion + 1n
        || after.lifetimeSpent !== state.rotationPlan.lifetimeSpent
        || after.periodSpent !== 0n
        || after.balance !== state.rotationPlan.balance) {
        throw new Error("Confirmed rotation did not produce the exact reviewed wallet state.");
      }
      status("Agent policy upgraded", "success");
      output([
        `Policy update confirmed: ${transactionHash}`,
        `Active delegate: ${state.rotationPlan.delegate}`,
        `Active verifier: ${state.rotationPlan.nextVerifier}`,
        `Active regression verifier set: ${EXPECTED.signedQuorumVerifierSetHash}`,
        `Policy hash: ${state.rotationPlan.nextHash}`,
        `Wallet balance: ${Number(after.balance) / 1_000_000} USDC`,
        "Policy-period spend counter: 0 USDC; lifetime spend is unchanged.",
        "The agent can now sign capped actions for a separate gas sponsor; the wallet needs no ETH.",
      ], "success");
      state.rotationPlan = null;
      form().elements.rotationReviewed.checked = false;
    } finally {
      state.busy = false;
      updateButtons();
    }
  }

  async function revoke() {
    if (state.busy) return;
    const wallet = requiredAddress(form().elements.existingWallet.value, "Bounded wallet");
    if (!state.account) throw new Error("Connect the owner wallet first.");
    state.busy = true;
    updateButtons();
    try {
      await acceptLegalAction("revoke_agent_policy");
      const owner = resultAddress(await call(wallet, SELECTORS.owner));
      if (owner !== state.account) throw new Error("Connected account is not this bounded wallet's owner.");
      output("Confirm one owner transaction to revoke all new delegate actions.", "pending");
      const txHash = await sendTransaction(wallet, SELECTORS.revokePolicy);
      await waitReceipt(txHash);
      status("Agent authority revoked", "pending");
      output(`Revocation confirmed: ${txHash}`, "success");
    } finally {
      state.busy = false;
      updateButtons();
    }
  }

  function updateButtons() {
    const reviewed = Boolean(form().elements.reviewed.checked);
    const rotationReviewed = Boolean(form().elements.rotationReviewed.checked);
    byId("connect-budget-wallet").disabled = state.busy;
    byId("inspect-budget-factory").disabled = state.busy || !state.account;
    byId("review-agent-budget").disabled = state.busy || !state.account || !state.infrastructureChecked;
    byId("activate-agent-budget").disabled = state.busy || !state.plan || !reviewed;
    const existing = String(form().elements.existingWallet.value || "").trim();
    const delegate = String(form().elements.delegate.value || "").trim();
    byId("review-delegate-rotation").disabled = state.busy || !state.account || !state.factoryReady
      || !/^0x[0-9a-fA-F]{40}$/.test(existing) || !/^0x[0-9a-fA-F]{40}$/.test(delegate);
    byId("rotate-agent-delegate").disabled = state.busy || !state.rotationPlan || !rotationReviewed;
    byId("revoke-agent-budget").disabled = state.busy || !state.account || !/^0x[0-9a-fA-F]{40}$/.test(existing);
  }

  function invalidatePlan(event) {
    if (event.target.name === "reviewed" || event.target.name === "rotationReviewed") {
      updateButtons();
      return;
    }
    if (event.target.name === "existingWallet" || event.target.name === "delegate") state.rotationPlan = null;
    if (event.target.name === "existingWallet") {
      updateButtons();
      return;
    }
    state.plan = null;
    updateButtons();
  }

  async function handle(action) {
    if (state.busy) return;
    try {
      await action();
    } catch (error) {
      status("Action stopped", "pending");
      output(error && error.message ? error.message : String(error), "error");
      state.busy = false;
      updateButtons();
    }
  }

  async function initialize() {
    form().addEventListener("input", invalidatePlan);
    form().addEventListener("submit", (event) => handle(() => activate(event)));
    byId("connect-budget-wallet").addEventListener("click", () => handle(connect));
    byId("inspect-budget-factory").addEventListener("click", () => handle(inspectInfrastructure));
    byId("review-agent-budget").addEventListener("click", () => handle(reviewPlan));
    byId("review-delegate-rotation").addEventListener("click", () => handle(reviewRotation));
    byId("rotate-agent-delegate").addEventListener("click", () => handle(rotateDelegate));
    byId("revoke-agent-budget").addEventListener("click", () => handle(revoke));
    document.querySelector("[data-wallet-provider]").addEventListener("change", () => {
      state.provider = null;
      state.account = null;
      state.plan = null;
      state.rotationPlan = null;
      state.infrastructureChecked = false;
      state.factoryReady = false;
      status("Not connected");
      updateButtons();
    });
    try {
      await loadManifest();
      await discoverProviders();
      status("Ready for owner connection", "pending");
    } catch (error) {
      status("Activation unavailable", "pending");
      output(error.message || String(error), "error");
    }
    updateButtons();
  }

  document.addEventListener("DOMContentLoaded", initialize);
})();
