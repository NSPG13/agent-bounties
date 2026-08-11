(() => {
  "use strict";

  const CHAIN_ID = "0x2105";
  const WALLET = "0x1eaa1c68772cf76bc5f4e4174766076e33ace662";
  const USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const V3 = "0x8e3d799d3d2cf52112e5be4ce48f105379462077";
  const KEEPER = "0xc26a630e85134ed30968735c8e7de4576cfa5dbc";
  const ZERO_HASH = `0x${"00".repeat(32)}`;
  const REQUIRED_BALANCE = 8_040_000n;
  const TARGET = 2_010_000n;
  const MIGRATION_SECONDS = 7_200n;
  const SELECTORS = Object.freeze({
    owner: "0x8da5cb5b",
    policy: "0x0505c8c9",
    policyVersion: "0x58355ead",
    lifetimeSpent: "0xb80762dd",
    periodSpent: "0x81497000",
    configurePolicy: "0x27d3543c",
    balanceOf: "0x70a08231",
  });

  const state = {
    providers: [],
    provider: null,
    account: null,
    plan: null,
    busy: false,
  };
  const announced = [];
  const byId = (id) => document.getElementById(id);
  const form = () => byId("v3-migration-form");
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function output(lines, tone = "") {
    const element = byId("v3-migration-output");
    element.textContent = Array.isArray(lines) ? lines.join("\n") : lines;
    element.dataset.tone = tone;
  }

  function status(text, tone = "") {
    const element = byId("migration-status");
    element.textContent = text;
    element.dataset.tone = tone;
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

  function addressWord(address) {
    const normalized = String(address || "").toLowerCase();
    if (!/^0x[0-9a-f]{40}$/.test(normalized)) throw new Error("Invalid EVM address.");
    return strip0x(normalized).padStart(64, "0");
  }

  function uintWord(value) {
    const number = BigInt(value);
    if (number < 0n || number >= (1n << 256n)) throw new Error("Integer outside uint256.");
    return number.toString(16).padStart(64, "0");
  }

  function bytes32Word(value) {
    const raw = strip0x(value);
    if (!/^[0-9a-f]{64}$/.test(raw)) throw new Error("Invalid bytes32 value.");
    return raw;
  }

  function resultAddress(value) {
    const raw = strip0x(value);
    if (raw.length < 64) throw new Error("Invalid address result.");
    return `0x${raw.slice(-40)}`;
  }

  function splitWords(value, count) {
    const raw = strip0x(value);
    if (raw.length !== count * 64) throw new Error(`Expected ${count} ABI words.`);
    return Array.from({ length: count }, (_, index) => raw.slice(index * 64, (index + 1) * 64));
  }

  async function call(to, data) {
    return request("eth_call", [{ to, data }, "latest"]);
  }

  async function latestTimestamp() {
    const block = await request("eth_getBlockByNumber", ["latest", false]);
    if (!block || !block.timestamp) throw new Error("Latest Base block is unavailable.");
    return BigInt(block.timestamp);
  }

  async function balance() {
    return BigInt(await call(USDC, `${SELECTORS.balanceOf}${addressWord(WALLET)}`));
  }

  async function inspect() {
    const [ownerRaw, policyRaw, versionRaw, lifetimeRaw, periodRaw, walletBalance, v3Code, now] = await Promise.all([
      call(WALLET, SELECTORS.owner),
      call(WALLET, SELECTORS.policy),
      call(WALLET, SELECTORS.policyVersion),
      call(WALLET, SELECTORS.lifetimeSpent),
      call(WALLET, SELECTORS.periodSpent),
      balance(),
      request("eth_getCode", [V3, "latest"]),
      latestTimestamp(),
    ]);
    const owner = resultAddress(ownerRaw);
    const words = splitWords(policyRaw, 13);
    return {
      owner,
      policyRaw: `0x${words.join("")}`,
      words,
      version: BigInt(versionRaw),
      lifetimeSpent: BigInt(lifetimeRaw),
      periodSpent: BigInt(periodRaw),
      balance: walletBalance,
      v3Ready: String(v3Code).toLowerCase() !== "0x" && String(v3Code).toLowerCase() !== "0x0",
      now,
    };
  }

  async function connect() {
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

  async function review() {
    if (!state.account) throw new Error("Connect the owner wallet first.");
    const observed = await inspect();
    if (observed.owner !== state.account) throw new Error("Connected account is not the bounded-wallet owner.");
    if (!observed.v3Ready) throw new Error("The V3 verifier is not deployed on Base yet.");
    if (observed.balance < REQUIRED_BALANCE) throw new Error("The bounded wallet has less than the required 8.04 USDC.");

    const maxLifetime = BigInt(`0x${observed.words[6]}`);
    if (maxLifetime - observed.lifetimeSpent < REQUIRED_BALANCE) {
      throw new Error("The remaining lifetime gross-spend budget is below 8.04 USDC.");
    }
    const currentExpiry = BigInt(`0x${observed.words[2]}`);
    const nextExpiry = currentExpiry < observed.now + MIGRATION_SECONDS
      ? currentExpiry
      : observed.now + MIGRATION_SECONDS;
    if (nextExpiry <= observed.now + 300n) throw new Error("The existing policy expires too soon for a safe migration.");

    const next = [
      addressWord(KEEPER),
      uintWord(observed.now > 0n ? observed.now - 1n : 0n),
      uintWord(nextExpiry),
      uintWord(MIGRATION_SECONDS),
      uintWord(TARGET),
      uintWord(REQUIRED_BALANCE),
      observed.words[6],
      uintWord(TARGET),
      uintWord(1n),
      uintWord(1n),
      addressWord(V3),
      bytes32Word(ZERO_HASH),
      bytes32Word(ZERO_HASH),
    ];
    const nextPolicy = `0x${next.join("")}`;
    const data = `${SELECTORS.configurePolicy}${next.join("")}`;
    await request("eth_call", [{ from: state.account, to: WALLET, data, value: "0x0" }, "latest"]);
    state.plan = { observed, nextPolicy, data, nextExpiry };
    output([
      `Current wallet balance: ${Number(observed.balance) / 1_000_000} USDC`,
      `Current lifetime spend: ${Number(observed.lifetimeSpent) / 1_000_000} USDC`,
      `Current period spend: ${Number(observed.periodSpent) / 1_000_000} USDC`,
      `Current / next policy version: ${observed.version} / ${observed.version + 1n}`,
      `Temporary delegate: ${KEEPER}`,
      `V3 verifier: ${V3}`,
      "Allowed action bitmap: create only",
      "Allowed verification mode: deterministic only",
      "Per-action / bounty-target cap: 2.01 USDC",
      "Period cap: 8.04 USDC",
      `Temporary expiry timestamp: ${nextExpiry}`,
      "Wallet balance and lifetime cap are unchanged by this policy transaction.",
    ], "pending");
    updateButtons();
  }

  async function approve(event) {
    event.preventDefault();
    if (state.busy) return;
    if (!state.plan) throw new Error("Review the exact live policy change first.");
    if (!form().elements.reviewed.checked) throw new Error("Confirm the exact temporary authority first.");
    state.busy = true;
    updateButtons();
    try {
      const before = await inspect();
      if (before.owner !== state.account
        || before.policyRaw !== state.plan.observed.policyRaw
        || before.version !== state.plan.observed.version
        || before.lifetimeSpent !== state.plan.observed.lifetimeSpent
        || before.periodSpent !== state.plan.observed.periodSpent
        || before.balance !== state.plan.observed.balance) {
        throw new Error("Bounded-wallet state changed after review. Review again.");
      }
      output([
        "Confirm one zero-value owner transaction.",
        `Bounded wallet: ${WALLET}`,
        `Temporary keeper delegate: ${KEEPER}`,
        `V3 verifier: ${V3}`,
        "No USDC or ETH transfer is requested by this policy update.",
      ], "pending");
      const hash = await request("eth_sendTransaction", [{
        from: state.account,
        to: WALLET,
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
        || after.periodSpent !== 0n
        || after.balance !== state.plan.observed.balance) {
        throw new Error("Confirmed transaction did not produce the exact reviewed migration policy.");
      }
      status("V3 migration policy active", "success");
      output([
        `Policy transaction confirmed: ${hash}`,
        `Wallet balance remains: ${Number(after.balance) / 1_000_000} USDC`,
        "The create-only migration executor may now create the four canonical V3 parents.",
        "No bounty is funded until those separate canonical transactions are confirmed.",
      ], "success");
      state.plan = null;
      form().elements.reviewed.checked = false;
    } finally {
      state.busy = false;
      updateButtons();
    }
  }

  function updateButtons() {
    byId("connect-v3-owner").disabled = state.busy;
    byId("review-v3-policy").disabled = state.busy || !state.account;
    byId("approve-v3-policy").disabled = state.busy || !state.plan || !form().elements.reviewed.checked;
  }

  function handle(fn) {
    return async (event) => {
      try {
        await fn(event);
      } catch (error) {
        output(error.message || String(error), "error");
        status("Action required", "error");
        state.plan = null;
        state.busy = false;
        updateButtons();
      }
    };
  }

  document.addEventListener("DOMContentLoaded", async () => {
    await discoverProviders();
    byId("connect-v3-owner").addEventListener("click", handle(connect));
    byId("review-v3-policy").addEventListener("click", handle(review));
    form().addEventListener("submit", handle(approve));
    form().elements.reviewed.addEventListener("change", updateButtons);
    updateButtons();
  });
})();
