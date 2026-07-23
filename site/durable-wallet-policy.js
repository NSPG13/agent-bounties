(() => {
  "use strict";

  const CONFIG_URL = "durable-verifier-router.json";
  const CHAIN_ID = "0x2105";
  const UINT64_MAX = (1n << 64n) - 1n;
  const REQUIRED_FUNDING = 8_040_000n;
  const ZERO_HASH = `0x${"00".repeat(32)}`;
  const state = {
    config: null,
    providers: [],
    provider: null,
    account: null,
    plan: null,
    busy: false,
  };
  const announced = [];
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const byId = (id) => document.getElementById(id);
  const form = () => byId("durable-wallet-policy-form");

  function output(value, tone = "") {
    const element = byId("durable-policy-review");
    element.textContent = Array.isArray(value) ? value.join("\n") : value;
    element.dataset.tone = tone;
  }

  function status(value, tone = "") {
    const element = byId("durable-policy-status");
    element.textContent = value;
    element.dataset.tone = tone;
  }

  function requiredAddress(value, label) {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(normalized)) throw new Error(`${label} is not an EVM address.`);
    return normalized;
  }

  function requiredBytes32(value, label) {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{64}$/.test(normalized)) throw new Error(`${label} is not bytes32.`);
    return normalized;
  }

  function requiredSelector(value, label) {
    const normalized = String(value || "").toLowerCase();
    if (!/^0x[0-9a-f]{8}$/.test(normalized)) throw new Error(`${label} is not a selector.`);
    return normalized;
  }

  async function loadConfig() {
    const response = await fetch(CONFIG_URL, { cache: "no-store" });
    if (!response.ok) throw new Error("Confirmed router deployment configuration is not published yet.");
    const config = await response.json();
    if (config.schema !== "agent-bounties/durable-verifier-router-public-v1" || config.status !== "active") {
      throw new Error("Durable router configuration is not active.");
    }
    config.wallet = requiredAddress(config.wallet, "Bounded wallet");
    config.owner = requiredAddress(config.owner, "Owner");
    config.keeper = requiredAddress(config.keeper, "Keeper");
    config.router = requiredAddress(config.router, "Router");
    config.adapter = requiredAddress(config.adapter, "Routed adapter");
    config.factory = requiredAddress(config.factory, "Canonical factory");
    config.usdc = requiredAddress(config.usdc, "Native USDC");
    config.policy_hash = requiredBytes32(config.policy_hash, "Active routed policy hash");
    config.adapter_runtime_code_hash = requiredBytes32(
      config.adapter_runtime_code_hash, "Adapter runtime code hash",
    );
    const selectors = config.selectors || {};
    for (const key of [
      "wallet_owner", "wallet_policy", "wallet_policy_version", "wallet_lifetime_spent",
      "wallet_period_spent", "wallet_configure_policy", "balance_of", "router_factory",
      "router_registrar", "router_guardian", "router_activation_delay", "router_bootstrap_used",
      "router_policy_active", "router_policy_record",
    ]) selectors[key] = requiredSelector(selectors[key], key);
    config.selectors = selectors;
    state.config = config;
    status("Confirmed router configuration loaded. Connect the owner wallet.", "pending");
    updateButtons();
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
    if (!detail || !detail.provider || typeof detail.provider.request !== "function") return;
    if (!announced.some((item) => item.provider === detail.provider)) announced.push(detail);
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(250);
    const candidates = [...announced];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const provider of injected) {
      if (provider && typeof provider.request === "function"
        && !candidates.some((item) => item.provider === provider)) {
        candidates.push({ provider, info: {} });
      }
    }
    state.providers = candidates;
    const select = document.querySelector("[data-wallet-provider]");
    select.replaceChildren();
    if (!candidates.length) {
      const option = document.createElement("option");
      option.textContent = "No browser wallet detected";
      select.append(option);
      select.disabled = true;
      return;
    }
    candidates.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      select.append(option);
    });
    select.disabled = false;
  }

  function selectProvider() {
    const index = Number.parseInt(document.querySelector("[data-wallet-provider]").value, 10);
    const selected = state.providers[index];
    if (!selected) throw new Error("Unlock a browser wallet, reload, and select it.");
    state.provider = selected.provider;
    return state.provider;
  }

  async function request(method, params = []) {
    const provider = state.provider || selectProvider();
    if (method === "eth_sendTransaction") {
      if (!window.AgentBountiesLegal) throw new Error("The legal agreement could not be loaded.");
      await window.AgentBountiesLegal.requireAcceptance({
        action: "update_agent_policy",
        walletAddress: state.account,
        scope: form(),
      });
    }
    return provider.request({ method, params });
  }

  function strip0x(value) {
    return String(value).replace(/^0x/, "").toLowerCase();
  }

  function addressWord(value) {
    return strip0x(requiredAddress(value, "Address")).padStart(64, "0");
  }

  function bytes32Word(value) {
    return strip0x(requiredBytes32(value, "bytes32"));
  }

  function uintWord(value) {
    const number = BigInt(value);
    if (number < 0n || number >= (1n << 256n)) throw new Error("Integer outside uint256.");
    return number.toString(16).padStart(64, "0");
  }

  function splitWords(value, count) {
    const raw = strip0x(value);
    if (raw.length !== count * 64) throw new Error(`Expected ${count} ABI words.`);
    return Array.from({ length: count }, (_, index) => raw.slice(index * 64, (index + 1) * 64));
  }

  function resultAddress(value) {
    const words = splitWords(value, 1);
    return `0x${words[0].slice(-40)}`;
  }

  function resultBool(value) {
    return BigInt(value) === 1n;
  }

  async function call(to, data) {
    return request("eth_call", [{ to, data }, "latest"]);
  }

  async function connect() {
    if (!state.config) throw new Error("Confirmed deployment configuration is not loaded.");
    const provider = selectProvider();
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    if (!Array.isArray(accounts) || !accounts[0]) throw new Error("The wallet did not return an account.");
    const chain = await provider.request({ method: "eth_chainId" });
    if (String(chain).toLowerCase() !== CHAIN_ID) {
      await provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: CHAIN_ID }] });
    }
    state.account = String(accounts[0]).toLowerCase();
    state.plan = null;
    status(`Connected ${state.account.slice(0, 8)}…`, "pending");
    updateButtons();
  }

  async function inspect() {
    const config = state.config;
    if (!config) throw new Error("Router configuration is missing.");
    const s = config.selectors;
    const policyArg = bytes32Word(config.policy_hash);
    const [
      walletCode, routerCode, adapterCode, ownerRaw, policyRaw, versionRaw, lifetimeRaw,
      periodRaw, balanceRaw, routerFactoryRaw, registrarRaw, guardianRaw, delayRaw,
      bootstrapRaw, activeRaw, recordRaw,
    ] = await Promise.all([
      request("eth_getCode", [config.wallet, "latest"]),
      request("eth_getCode", [config.router, "latest"]),
      request("eth_getCode", [config.adapter, "latest"]),
      call(config.wallet, s.wallet_owner),
      call(config.wallet, s.wallet_policy),
      call(config.wallet, s.wallet_policy_version),
      call(config.wallet, s.wallet_lifetime_spent),
      call(config.wallet, s.wallet_period_spent),
      call(config.usdc, `${s.balance_of}${addressWord(config.wallet)}`),
      call(config.router, s.router_factory),
      call(config.router, s.router_registrar),
      call(config.router, s.router_guardian),
      call(config.router, s.router_activation_delay),
      call(config.router, s.router_bootstrap_used),
      call(config.router, `${s.router_policy_active}${policyArg}`),
      call(config.router, `${s.router_policy_record}${policyArg}`),
    ]);
    const words = splitWords(policyRaw, 13);
    const record = splitWords(recordRaw, 6);
    return {
      walletCode: String(walletCode).toLowerCase(),
      routerCode: String(routerCode).toLowerCase(),
      adapterCode: String(adapterCode).toLowerCase(),
      owner: resultAddress(ownerRaw),
      words,
      policyRaw: `0x${words.join("")}`,
      version: BigInt(versionRaw),
      lifetimeSpent: BigInt(lifetimeRaw),
      periodSpent: BigInt(periodRaw),
      balance: BigInt(balanceRaw),
      routerFactory: resultAddress(routerFactoryRaw),
      registrar: resultAddress(registrarRaw),
      guardian: resultAddress(guardianRaw),
      activationDelay: BigInt(delayRaw),
      bootstrapUsed: resultBool(bootstrapRaw),
      active: resultBool(activeRaw),
      record: {
        adapter: `0x${record[0].slice(-40)}`,
        runtimeCodeHash: `0x${record[1]}`,
        proposedAt: BigInt(`0x${record[2]}`),
        activateAfter: BigInt(`0x${record[3]}`),
        activatedAt: BigInt(`0x${record[4]}`),
        vetoed: BigInt(`0x${record[5]}`) === 1n,
      },
    };
  }

  function validateInfrastructure(observed) {
    const config = state.config;
    if ([observed.walletCode, observed.routerCode, observed.adapterCode].some((code) => code === "0x" || code === "0x0")) {
      throw new Error("Wallet, router, or routed adapter bytecode is missing on Base.");
    }
    if (observed.owner !== config.owner) throw new Error("Bounded-wallet owner does not match the deployment record.");
    if (observed.routerFactory !== config.factory) throw new Error("Router factory binding does not match.");
    if (observed.registrar !== config.keeper) throw new Error("Router registrar does not match the protected keeper.");
    if (observed.guardian !== config.owner) throw new Error("Router guardian does not match the owner.");
    if (observed.activationDelay !== BigInt(config.activation_delay_seconds)) {
      throw new Error("Router activation delay does not match the deployment record.");
    }
    if (!observed.bootstrapUsed || !observed.active || observed.record.vetoed || observed.record.activatedAt === 0n) {
      throw new Error("The initial routed policy is not active.");
    }
    if (observed.record.adapter !== config.adapter
      || observed.record.runtimeCodeHash !== config.adapter_runtime_code_hash) {
      throw new Error("Active router policy does not match the attested adapter.");
    }
  }

  function validateCurrentPolicy(observed) {
    const config = state.config;
    const p = config.expected_current_policy;
    const values = {
      delegate: `0x${observed.words[0].slice(-40)}`,
      validAfter: BigInt(`0x${observed.words[1]}`),
      validUntil: BigInt(`0x${observed.words[2]}`),
      periodSeconds: BigInt(`0x${observed.words[3]}`),
      maxPerAction: BigInt(`0x${observed.words[4]}`),
      maxPerPeriod: BigInt(`0x${observed.words[5]}`),
      maxLifetime: BigInt(`0x${observed.words[6]}`),
      maxBountyTarget: BigInt(`0x${observed.words[7]}`),
      allowedActions: BigInt(`0x${observed.words[8]}`),
      allowedModes: BigInt(`0x${observed.words[9]}`),
      deterministicVerifier: `0x${observed.words[10].slice(-40)}`,
      signedHash: `0x${observed.words[11]}`,
      aiHash: `0x${observed.words[12]}`,
    };
    const exact = {
      periodSeconds: BigInt(p.period_seconds),
      maxPerAction: BigInt(p.max_per_action),
      maxPerPeriod: BigInt(p.max_per_period),
      maxLifetime: BigInt(p.max_lifetime_spend),
      maxBountyTarget: BigInt(p.max_bounty_target),
      allowedActions: BigInt(p.allowed_actions),
      allowedModes: BigInt(p.allowed_verification_modes),
      signedHash: requiredBytes32(p.signed_quorum_hash, "Expected signed quorum hash"),
      aiHash: requiredBytes32(p.ai_quorum_hash, "Expected AI quorum hash"),
    };
    for (const [key, expected] of Object.entries(exact)) {
      if (values[key] !== expected) throw new Error(`Current wallet ${key} changed; review the new state separately.`);
    }
    if (observed.periodSpent !== 0n) {
      throw new Error("Current 24-hour spend counter is nonzero. The policy update is blocked to avoid resetting today's cap.");
    }
    if (observed.balance < REQUIRED_FUNDING) throw new Error("Bounded-wallet balance is below 8.04 USDC.");
    if (observed.lifetimeSpent + REQUIRED_FUNDING > values.maxLifetime) {
      throw new Error("Remaining lifetime budget is below the 8.04 USDC replacement requirement.");
    }
    return values;
  }

  async function review() {
    if (!state.account) throw new Error("Connect the owner wallet first.");
    const observed = await inspect();
    validateInfrastructure(observed);
    if (observed.owner !== state.account) throw new Error("Connected account is not the bounded-wallet owner.");
    const current = validateCurrentPolicy(observed);

    const next = [
      addressWord(state.config.keeper),
      observed.words[1],
      uintWord(UINT64_MAX),
      observed.words[3],
      observed.words[4],
      observed.words[5],
      observed.words[6],
      observed.words[7],
      observed.words[8],
      observed.words[9],
      addressWord(state.config.router),
      observed.words[11],
      observed.words[12],
    ];
    const nextPolicy = `0x${next.join("")}`;
    const data = `${state.config.selectors.wallet_configure_policy}${next.join("")}`;

    if (observed.policyRaw === nextPolicy) {
      status("Durable wallet policy is already active.", "success");
      output([
        `Policy version: ${observed.version}`,
        `Delegate: ${state.config.keeper}`,
        `Stable verifier router: ${state.config.router}`,
        `Wallet balance: ${(Number(observed.balance) / 1_000_000).toFixed(6)} USDC`,
        "No owner transaction remains.",
      ], "success");
      state.plan = null;
      form().elements.reviewed.disabled = true;
      updateButtons();
      return;
    }

    await request("eth_call", [{ from: state.account, to: state.config.wallet, data, value: "0x0" }, "latest"]);
    state.plan = { observed, nextPolicy, data };
    output([
      `Current / next policy version: ${observed.version} / ${observed.version + 1n}`,
      `Wallet balance: ${(Number(observed.balance) / 1_000_000).toFixed(6)} USDC`,
      `Lifetime spent / cap: ${(Number(observed.lifetimeSpent) / 1_000_000).toFixed(6)} / ${(Number(current.maxLifetime) / 1_000_000).toFixed(6)} USDC`,
      `Delegate: ${current.delegate} → ${state.config.keeper}`,
      `Deterministic verifier: ${current.deterministicVerifier} → ${state.config.router}`,
      `Expiry: ${current.validUntil} → ${UINT64_MAX}`,
      `Per action: ${(Number(current.maxPerAction) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Per 24 hours: ${(Number(current.maxPerPeriod) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Lifetime cap: ${(Number(current.maxLifetime) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Bounty target cap: ${(Number(current.maxBountyTarget) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Action bitmap: ${current.allowedActions} (unchanged)`,
      `Verification-mode bitmap: ${current.allowedModes} (unchanged)`,
      `Active routed policy: ${state.config.policy_hash}`,
      `Routed adapter: ${state.config.adapter}`,
      "Transaction value: 0 ETH; token transfer: none.",
    ], "pending");
    form().elements.reviewed.disabled = false;
    updateButtons();
  }

  async function approve(event) {
    event.preventDefault();
    if (state.busy) return;
    if (!state.plan) throw new Error("Review the exact live policy change first.");
    if (!form().elements.reviewed.checked) throw new Error("Confirm the exact policy first.");
    state.busy = true;
    updateButtons();
    try {
      const before = await inspect();
      validateInfrastructure(before);
      validateCurrentPolicy(before);
      if (before.owner !== state.account
        || before.policyRaw !== state.plan.observed.policyRaw
        || before.version !== state.plan.observed.version
        || before.lifetimeSpent !== state.plan.observed.lifetimeSpent
        || before.periodSpent !== state.plan.observed.periodSpent
        || before.balance !== state.plan.observed.balance) {
        throw new Error("Base wallet state changed after review. Review again.");
      }
      status("Confirm one zero-value owner transaction in your wallet.", "pending");
      const hash = await request("eth_sendTransaction", [{
        from: state.account,
        to: state.config.wallet,
        data: state.plan.data,
        value: "0x0",
      }]);
      const started = Date.now();
      let receipt = null;
      while (Date.now() - started < 180_000) {
        receipt = await request("eth_getTransactionReceipt", [hash]);
        if (receipt) break;
        await sleep(1_500);
      }
      if (!receipt || receipt.status !== "0x1") throw new Error(`Policy transaction failed or timed out: ${hash}`);
      const after = await inspect();
      if (after.policyRaw !== state.plan.nextPolicy
        || after.version !== state.plan.observed.version + 1n
        || after.lifetimeSpent !== state.plan.observed.lifetimeSpent
        || after.balance !== state.plan.observed.balance
        || after.periodSpent !== 0n) {
        throw new Error("Confirmed transaction did not produce the exact reviewed durable policy.");
      }
      status("Durable autonomous wallet policy confirmed.", "success");
      output([
        `Policy transaction: ${hash}`,
        `Policy version: ${after.version}`,
        `Delegate: ${state.config.keeper}`,
        `Stable verifier router: ${state.config.router}`,
        `Wallet balance remains ${(Number(after.balance) / 1_000_000).toFixed(6)} USDC`,
        "The repository control loop can now create and fund the four routed parents without another owner signature.",
      ], "success");
      state.plan = null;
      form().elements.reviewed.checked = false;
      form().elements.reviewed.disabled = true;
    } finally {
      state.busy = false;
      updateButtons();
    }
  }

  function updateButtons() {
    const connectButton = document.querySelector("[data-connect-owner]");
    const reviewButton = document.querySelector("[data-review-policy]");
    const approveButton = document.querySelector("[data-approve-policy]");
    connectButton.disabled = state.busy || !state.config || state.providers.length === 0;
    reviewButton.disabled = state.busy || !state.account || !state.config;
    approveButton.disabled = state.busy || !state.plan || !form().elements.reviewed.checked;
  }

  function guard(action) {
    return async (event) => {
      try {
        await action(event);
      } catch (error) {
        status(error.message || String(error), "error");
        output(error.message || String(error), "error");
        state.plan = null;
        form().elements.reviewed.checked = false;
        form().elements.reviewed.disabled = true;
        updateButtons();
      }
    };
  }

  document.addEventListener("DOMContentLoaded", async () => {
    document.querySelector("[data-connect-owner]").addEventListener("click", guard(connect));
    document.querySelector("[data-review-policy]").addEventListener("click", guard(review));
    form().addEventListener("submit", guard(approve));
    form().elements.reviewed.addEventListener("change", updateButtons);
    await discoverProviders();
    try {
      await loadConfig();
    } catch (error) {
      status(error.message || String(error), "error");
    }
    updateButtons();
  });
})();
