(() => {
  "use strict";

  const CHAIN_ID = "0x2105";
  const UINT64_MAX = (1n << 64n) - 1n;
  const REQUIRED_FUNDING = 8_040_000n;
  const ZERO_HASH = `0x${"00".repeat(32)}`;
  const CONFIG = Object.freeze({
    wallet: "0x1eaa1c68772cf76bc5f4e4174766076e33ace662",
    owner: "0x884834e884d6e93462655a2820140ad03e6747bc",
    keeper: "0xc26a630e85134ed30968735c8e7de4576cfa5dbc",
    router: "0x380c1af742593dd88b6f20387e9ee693a0536731",
    factory: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
    usdc: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    activationDelaySeconds: 604800n,
    expectedCurrentPolicy: Object.freeze({
      periodSeconds: 86400n,
      maxPerAction: 5_000_000n,
      maxPerPeriod: 10_000_000n,
      maxLifetime: 89_000_000n,
      maxBountyTarget: 5_000_000n,
      allowedActions: 15n,
      allowedModes: 1n,
      signedHash: ZERO_HASH,
      aiHash: ZERO_HASH,
    }),
  });
  const state = {
    providers: [],
    provider: null,
    account: null,
    deployment: null,
    plan: null,
    busy: false,
  };
  const announced = [];
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const byId = (id) => document.getElementById(id);
  const form = () => byId("durable-wallet-policy-form");

  const KECCAK_RATE_BYTES = 136;
  const KECCAK_ROTATIONS = Object.freeze([
    0, 1, 62, 28, 27,
    36, 44, 6, 55, 20,
    3, 10, 43, 25, 39,
    41, 45, 15, 21, 8,
    18, 2, 61, 56, 14,
  ]);
  const KECCAK_ROUND_CONSTANTS = Object.freeze([
    0x0000000000000001n, 0x0000000000008082n, 0x800000000000808an,
    0x8000000080008000n, 0x000000000000808bn, 0x0000000080000001n,
    0x8000000080008081n, 0x8000000000008009n, 0x000000000000008an,
    0x0000000000000088n, 0x0000000080008009n, 0x000000008000000an,
    0x000000008000808bn, 0x800000000000008bn, 0x8000000000008089n,
    0x8000000000008003n, 0x8000000000008002n, 0x8000000000000080n,
    0x000000000000800an, 0x800000008000000an, 0x8000000080008081n,
    0x8000000000008080n, 0x0000000080000001n, 0x8000000080008008n,
  ]);

  function rotateLeft64(value, bits) {
    if (bits === 0) return value;
    const shift = BigInt(bits);
    return ((value << shift) | (value >> (64n - shift))) & ((1n << 64n) - 1n);
  }

  function keccakPermutation(words) {
    for (const roundConstant of KECCAK_ROUND_CONSTANTS) {
      const column = Array(5).fill(0n);
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) column[x] ^= words[x + (5 * y)];
      }
      const delta = column.map((_, x) => (
        column[(x + 4) % 5] ^ rotateLeft64(column[(x + 1) % 5], 1)
      ));
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) words[x + (5 * y)] ^= delta[x];
      }
      const rotated = Array(25).fill(0n);
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) {
          rotated[y + (5 * ((2 * x + 3 * y) % 5))] = rotateLeft64(
            words[x + (5 * y)], KECCAK_ROTATIONS[x + (5 * y)],
          );
        }
      }
      for (let x = 0; x < 5; x += 1) {
        for (let y = 0; y < 5; y += 1) {
          words[x + (5 * y)] = rotated[x + (5 * y)]
            ^ ((~rotated[((x + 1) % 5) + (5 * y)])
              & rotated[((x + 2) % 5) + (5 * y)]);
        }
      }
      words[0] ^= roundConstant;
    }
  }

  function absorbKeccakBlock(words, bytes) {
    for (let index = 0; index < KECCAK_RATE_BYTES; index += 1) {
      words[Math.floor(index / 8)] ^= BigInt(bytes[index]) << BigInt((index % 8) * 8);
    }
    keccakPermutation(words);
  }

  function keccak256Hex(value) {
    const input = String(value || "");
    if (!/^0x(?:[0-9a-fA-F]{2})*$/.test(input)) throw new Error("Keccak input must be hex bytes.");
    const bytes = new Uint8Array((input.length - 2) / 2);
    for (let index = 0; index < bytes.length; index += 1) {
      bytes[index] = Number.parseInt(input.slice(2 + (index * 2), 4 + (index * 2)), 16);
    }
    const words = Array(25).fill(0n);
    let offset = 0;
    while (offset + KECCAK_RATE_BYTES <= bytes.length) {
      absorbKeccakBlock(words, bytes.subarray(offset, offset + KECCAK_RATE_BYTES));
      offset += KECCAK_RATE_BYTES;
    }
    const finalBlock = new Uint8Array(KECCAK_RATE_BYTES);
    finalBlock.set(bytes.subarray(offset));
    finalBlock[bytes.length - offset] ^= 0x01;
    finalBlock[KECCAK_RATE_BYTES - 1] ^= 0x80;
    absorbKeccakBlock(words, finalBlock);
    let digest = "";
    for (let index = 0; index < 32; index += 1) {
      const byte = Number((words[Math.floor(index / 8)] >> BigInt((index % 8) * 8)) & 0xffn);
      digest += byte.toString(16).padStart(2, "0");
    }
    return `0x${digest}`;
  }

  function textHex(value) {
    return `0x${Array.from(new TextEncoder().encode(value), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function selector(signature) {
    return keccak256Hex(textHex(signature)).slice(0, 10);
  }

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
    return `0x${splitWords(value, 1)[0].slice(-40)}`;
  }

  function resultBool(value) {
    return BigInt(value) === 1n;
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

  async function call(to, signature, args = "") {
    return request("eth_call", [{ to, data: `${selector(signature)}${args}` }, "latest"]);
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
    state.deployment = null;
    state.plan = null;
    status(`Connected ${state.account.slice(0, 8)}… Review the exact Base state.`, "pending");
    updateButtons();
  }

  async function discoverDeployment() {
    if (state.deployment) return state.deployment;
    const latestHex = await request("eth_blockNumber");
    const latest = BigInt(latestHex);
    const from = latest > 100_000n ? latest - 100_000n : 0n;
    const topic = keccak256Hex(textHex("PolicyBootstrapped(bytes32,address,bytes32)"));
    const logs = await request("eth_getLogs", [{
      address: CONFIG.router,
      fromBlock: `0x${from.toString(16)}`,
      toBlock: "latest",
      topics: [topic],
    }]);
    if (!Array.isArray(logs) || logs.length !== 1) {
      throw new Error("Expected exactly one durable router bootstrap event on Base.");
    }
    const log = logs[0];
    if (!Array.isArray(log.topics) || log.topics.length !== 3) {
      throw new Error("Durable router bootstrap event has the wrong shape.");
    }
    const policyHash = requiredBytes32(log.topics[1], "Routed policy hash");
    const adapter = requiredAddress(`0x${strip0x(log.topics[2]).slice(-40)}`, "Routed adapter");
    const eventRuntimeHash = requiredBytes32(log.data, "Adapter runtime code hash");
    const [routerCode, adapterCode] = await Promise.all([
      request("eth_getCode", [CONFIG.router, "latest"]),
      request("eth_getCode", [adapter, "latest"]),
    ]);
    if ([routerCode, adapterCode].some((code) => code === "0x" || code === "0x0")) {
      throw new Error("Durable router or routed adapter bytecode is missing on Base.");
    }
    const adapterRuntimeHash = keccak256Hex(adapterCode);
    if (adapterRuntimeHash !== eventRuntimeHash) {
      throw new Error("Routed adapter runtime hash differs from the bootstrap event.");
    }
    const policyArg = bytes32Word(policyHash);
    const [
      factoryRaw, registrarRaw, guardianRaw, delayRaw, bootstrapRaw, activeRaw, recordRaw,
      adapterRouterRaw, adapterPolicyRaw, adapterFactoryRaw, acceptanceRaw, childFloorRaw, marginFloorRaw,
    ] = await Promise.all([
      call(CONFIG.router, "canonicalFactory()"),
      call(CONFIG.router, "registrar()"),
      call(CONFIG.router, "guardian()"),
      call(CONFIG.router, "activationDelay()"),
      call(CONFIG.router, "bootstrapUsed()"),
      call(CONFIG.router, "isPolicyActive(bytes32)", policyArg),
      call(CONFIG.router, "policies(bytes32)", policyArg),
      call(adapter, "verifierRouter()"),
      call(adapter, "committedPolicyHash()"),
      call(adapter, "canonicalFactory()"),
      call(adapter, "ACCEPTANCE_CRITERIA_HASH()"),
      call(adapter, "MINIMUM_CHILD_TARGET()"),
      call(adapter, "MINIMUM_PARENT_GROSS_MARGIN()"),
    ]);
    const record = splitWords(recordRaw, 6);
    const deployment = {
      policyHash,
      adapter,
      adapterRuntimeHash,
      acceptanceHash: requiredBytes32(acceptanceRaw, "Acceptance criteria hash"),
      bootstrapTransaction: requiredBytes32(log.transactionHash, "Bootstrap transaction"),
      bootstrapBlock: BigInt(log.blockNumber),
      factory: resultAddress(factoryRaw),
      registrar: resultAddress(registrarRaw),
      guardian: resultAddress(guardianRaw),
      activationDelay: BigInt(delayRaw),
      bootstrapUsed: resultBool(bootstrapRaw),
      active: resultBool(activeRaw),
      record: {
        adapter: `0x${record[0].slice(-40)}`,
        runtimeCodeHash: `0x${record[1]}`,
        activatedAt: BigInt(`0x${record[4]}`),
        vetoed: BigInt(`0x${record[5]}`) === 1n,
      },
      adapterRouter: resultAddress(adapterRouterRaw),
      adapterPolicy: requiredBytes32(adapterPolicyRaw, "Adapter policy hash"),
      adapterFactory: resultAddress(adapterFactoryRaw),
      childFloor: BigInt(childFloorRaw),
      marginFloor: BigInt(marginFloorRaw),
    };
    if (deployment.factory !== CONFIG.factory || deployment.registrar !== CONFIG.keeper
      || deployment.guardian !== CONFIG.owner || deployment.activationDelay !== CONFIG.activationDelaySeconds
      || !deployment.bootstrapUsed || !deployment.active || deployment.record.vetoed
      || deployment.record.activatedAt === 0n || deployment.record.adapter !== adapter
      || deployment.record.runtimeCodeHash !== adapterRuntimeHash
      || deployment.adapterRouter !== CONFIG.router || deployment.adapterPolicy !== policyHash
      || deployment.adapterFactory !== CONFIG.factory || deployment.childFloor !== 1_000_000n
      || deployment.marginFloor !== 1_000_000n) {
      throw new Error("Durable router or routed policy immutable state failed validation.");
    }
    state.deployment = deployment;
    return deployment;
  }

  async function inspect() {
    const deployment = await discoverDeployment();
    const [
      walletCode, ownerRaw, policyRaw, versionRaw, lifetimeRaw, periodRaw, periodBucketRaw, balanceRaw,
    ] = await Promise.all([
      request("eth_getCode", [CONFIG.wallet, "latest"]),
      call(CONFIG.wallet, "owner()"),
      call(CONFIG.wallet, "policy()"),
      call(CONFIG.wallet, "policyVersion()"),
      call(CONFIG.wallet, "lifetimeSpent()"),
      call(CONFIG.wallet, "periodSpent()"),
      call(CONFIG.wallet, "periodBucket()"),
      call(CONFIG.usdc, "balanceOf(address)", addressWord(CONFIG.wallet)),
    ]);
    if (walletCode === "0x" || walletCode === "0x0") throw new Error("Bounded wallet bytecode is missing.");
    const words = splitWords(policyRaw, 13);
    return {
      deployment,
      owner: resultAddress(ownerRaw),
      words,
      policyRaw: `0x${words.join("")}`,
      version: BigInt(versionRaw),
      lifetimeSpent: BigInt(lifetimeRaw),
      periodSpent: BigInt(periodRaw),
      periodBucket: BigInt(periodBucketRaw),
      balance: BigInt(balanceRaw),
    };
  }

  function validateCurrentPolicy(observed) {
    if (observed.owner !== CONFIG.owner) throw new Error("Bounded-wallet owner changed.");
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
    for (const [key, expected] of Object.entries(CONFIG.expectedCurrentPolicy)) {
      if (values[key] !== expected) throw new Error(`Current wallet ${key} changed; review the new state separately.`);
    }
    const now = BigInt(Math.floor(Date.now() / 1000));
    const currentBucket = now / values.periodSeconds;
    const effectivePeriodSpent = observed.periodBucket === currentBucket ? observed.periodSpent : 0n;
    const alreadyDurable = values.delegate === CONFIG.keeper
      && values.deterministicVerifier === CONFIG.router && values.validUntil === UINT64_MAX;
    if (!alreadyDurable && effectivePeriodSpent !== 0n) {
      throw new Error("Current 24-hour spend counter is nonzero. Policy update is blocked to avoid resetting today's cap.");
    }
    if (observed.balance < REQUIRED_FUNDING) throw new Error("Bounded-wallet balance is below 8.04 USDC.");
    if (observed.lifetimeSpent + REQUIRED_FUNDING > values.maxLifetime) {
      throw new Error("Remaining lifetime budget is below the 8.04 USDC replacement requirement.");
    }
    return { ...values, effectivePeriodSpent, alreadyDurable };
  }

  async function review() {
    if (!state.account) throw new Error("Connect the owner wallet first.");
    status("Reading the router event, adapter bytecode, and wallet policy directly from Base…", "pending");
    const observed = await inspect();
    if (observed.owner !== state.account) throw new Error("Connected account is not the bounded-wallet owner.");
    const current = validateCurrentPolicy(observed);
    if (current.alreadyDurable) {
      status("Durable wallet policy is already active.", "success");
      output([
        `Policy version: ${observed.version}`,
        `Delegate: ${CONFIG.keeper}`,
        `Stable verifier router: ${CONFIG.router}`,
        `Active routed policy: ${observed.deployment.policyHash}`,
        `Wallet balance: ${(Number(observed.balance) / 1_000_000).toFixed(6)} USDC`,
        "No owner transaction remains.",
      ], "success");
      state.plan = null;
      form().elements.reviewed.disabled = true;
      updateButtons();
      return;
    }
    const next = [
      addressWord(CONFIG.keeper), observed.words[1], uintWord(UINT64_MAX), observed.words[3],
      observed.words[4], observed.words[5], observed.words[6], observed.words[7],
      observed.words[8], observed.words[9], addressWord(CONFIG.router), observed.words[11], observed.words[12],
    ];
    const nextPolicy = `0x${next.join("")}`;
    const configureSelector = selector(
      "configurePolicy((address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32))",
    );
    const data = `${configureSelector}${next.join("")}`;
    await request("eth_call", [{ from: state.account, to: CONFIG.wallet, data, value: "0x0" }, "latest"]);
    state.plan = { observed, nextPolicy, data };
    output([
      `Router bootstrap transaction: ${observed.deployment.bootstrapTransaction}`,
      `Active routed policy: ${observed.deployment.policyHash}`,
      `Routed adapter: ${observed.deployment.adapter}`,
      `Adapter runtime hash: ${observed.deployment.adapterRuntimeHash}`,
      `Current / next policy version: ${observed.version} / ${observed.version + 1n}`,
      `Wallet balance: ${(Number(observed.balance) / 1_000_000).toFixed(6)} USDC`,
      `Lifetime spent / cap: ${(Number(observed.lifetimeSpent) / 1_000_000).toFixed(6)} / ${(Number(current.maxLifetime) / 1_000_000).toFixed(6)} USDC`,
      `Delegate: ${current.delegate} → ${CONFIG.keeper}`,
      `Deterministic verifier: ${current.deterministicVerifier} → ${CONFIG.router}`,
      `Expiry: ${current.validUntil} → ${UINT64_MAX}`,
      `Per action: ${(Number(current.maxPerAction) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Per 24 hours: ${(Number(current.maxPerPeriod) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Lifetime cap: ${(Number(current.maxLifetime) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Bounty target cap: ${(Number(current.maxBountyTarget) / 1_000_000).toFixed(2)} USDC (unchanged)`,
      `Action bitmap: ${current.allowedActions} (unchanged)`,
      `Verification-mode bitmap: ${current.allowedModes} (unchanged)`,
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
      state.deployment = null;
      const before = await inspect();
      const current = validateCurrentPolicy(before);
      if (current.alreadyDurable) throw new Error("Durable policy became active already; review again.");
      if (before.owner !== state.account || before.policyRaw !== state.plan.observed.policyRaw
        || before.version !== state.plan.observed.version || before.lifetimeSpent !== state.plan.observed.lifetimeSpent
        || before.periodSpent !== state.plan.observed.periodSpent || before.periodBucket !== state.plan.observed.periodBucket
        || before.balance !== state.plan.observed.balance
        || before.deployment.policyHash !== state.plan.observed.deployment.policyHash
        || before.deployment.adapterRuntimeHash !== state.plan.observed.deployment.adapterRuntimeHash) {
        throw new Error("Base state changed after review. Review again.");
      }
      status("Confirm one zero-value owner transaction in your wallet.", "pending");
      const hash = await request("eth_sendTransaction", [{
        from: state.account, to: CONFIG.wallet, data: state.plan.data, value: "0x0",
      }]);
      const started = Date.now();
      let receipt = null;
      while (Date.now() - started < 180_000) {
        receipt = await request("eth_getTransactionReceipt", [hash]);
        if (receipt) break;
        await sleep(1_500);
      }
      if (!receipt || receipt.status !== "0x1") throw new Error(`Policy transaction failed or timed out: ${hash}`);
      state.deployment = null;
      const after = await inspect();
      const post = validateCurrentPolicy(after);
      if (!post.alreadyDurable || after.policyRaw !== state.plan.nextPolicy
        || after.version !== state.plan.observed.version + 1n
        || after.lifetimeSpent !== state.plan.observed.lifetimeSpent
        || after.balance !== state.plan.observed.balance || after.periodSpent !== 0n) {
        throw new Error("Confirmed transaction did not produce the exact reviewed durable policy.");
      }
      status("Durable autonomous wallet policy confirmed.", "success");
      output([
        `Policy transaction: ${hash}`,
        `Policy version: ${after.version}`,
        `Delegate: ${CONFIG.keeper}`,
        `Stable verifier router: ${CONFIG.router}`,
        `Active routed policy: ${after.deployment.policyHash}`,
        `Wallet balance remains ${(Number(after.balance) / 1_000_000).toFixed(6)} USDC`,
        "The hourly control loop can now create and fund the four routed parents without another owner signature.",
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
    document.querySelector("[data-connect-owner]").disabled = state.busy || state.providers.length === 0;
    document.querySelector("[data-review-policy]").disabled = state.busy || !state.account;
    document.querySelector("[data-approve-policy]").disabled = state.busy || !state.plan || !form().elements.reviewed.checked;
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
    status("Connect the owner wallet. The page will derive and verify all deployment state directly from Base.", "pending");
    updateButtons();
  });
})();
