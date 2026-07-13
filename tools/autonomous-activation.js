(() => {
  "use strict";

  const BASE_CHAIN_ID = "0x2105";
  const BUNDLE_URL = "/deployments/canonical-child-seeds-base-mainnet.json";
  const VERIFIER_BUNDLE_URL = "/deployments/canonical-child-verifier-base-mainnet-deployment.json";
  const VERIFIER_MODULE = "0x40adac5a1d00a725f77682f8940b893eaed31ecf";
  const ACCEPTANCE_CRITERIA_HASH = "0x005f591a8549549698e7c028b78ddc84076e0996ef07e19dd543ebdb12cb4553";
  const EXPECTED_ISSUES = [217, 218, 219, 220];
  const state = { bundle: null, account: null, provider: null, providers: [], pendingBounties: [], inspected: false };
  const announcedProviders = [];
  const byId = (id) => document.getElementById(id);
  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

  function write(target, value, tone = "") {
    target.textContent = Array.isArray(value) ? value.join("\n") : value;
    target.dataset.tone = tone;
  }

  function requireLocalOrigin() {
    if (!new Set(["127.0.0.1", "localhost"]).has(location.hostname)) {
      throw new Error("Activation console must be served from localhost.");
    }
  }

  async function loadBundle() {
    requireLocalOrigin();
    const [response, verifierResponse] = await Promise.all([
      fetch(BUNDLE_URL, { cache: "no-store" }),
      fetch(VERIFIER_BUNDLE_URL, { cache: "no-store" }),
    ]);
    if (!response.ok || !verifierResponse.ok) throw new Error("A checked-in activation artifact is unavailable.");
    const bundle = await response.json();
    const verifierBundle = await verifierResponse.json();
    if (
      bundle.schema_version !== "agent-bounties/autonomous-activation-bundle-v1"
      || bundle.network !== "base-mainnet"
      || bundle.chain_id !== 8453
      || bundle.manifest_canonical_json_keccak256 !== "0x67e77578481d2ef5015643c880a3810b018681ccb2f1d7571d4b573601095a74"
      || bundle.creation_batch.total_initial_funding !== "4000000"
      || bundle.bounties.length !== 4
      || bundle.creation_batch.wallet_calls.length !== 5
      || bundle.bounties.some((item, index) => item.issue !== EXPECTED_ISSUES[index])
      || verifierBundle.schema_version !== "agent-bounties/canonical-child-verifier-deployment-v1"
      || verifierBundle.deployment.expected_contract !== VERIFIER_MODULE
      || verifierBundle.acceptance_criteria_hash !== ACCEPTANCE_CRITERIA_HASH
    ) {
      throw new Error("Activation artifacts violate the locked canonical-child-v1 contract.");
    }
    state.bundle = bundle;
    state.verifierBundle = verifierBundle;
    document.querySelector("[data-factory]").textContent = bundle.deployment.expected_factory;
    document.querySelector("[data-implementation]").textContent = bundle.deployment.expected_implementation;
    document.querySelector("[data-creator]").textContent = bundle.deployment.from;
    document.querySelector("[data-bounties]").textContent = bundle.bounties.map((item) => `#${item.issue}`).join(", ");
    document.querySelector("[data-funding]").textContent = `${Number(bundle.creation_batch.total_initial_funding) / 1_000_000} USDC`;
    return bundle;
  }

  function isWalletProvider(provider) {
    return Boolean(provider && typeof provider.request === "function");
  }

  function providerName(provider, info = {}) {
    if (info.name) return info.name;
    if (provider.isMetaMask) return "MetaMask";
    if (provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (provider.isBraveWallet) return "Brave Wallet";
    return "Injected wallet";
  }

  function rememberProvider(event) {
    const detail = event && event.detail;
    if (!detail || !detail.provider || announcedProviders.some((item) => item.provider === detail.provider)) return;
    announcedProviders.push(detail);
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(500);
    const candidates = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const provider of injected) {
      if (isWalletProvider(provider) && !candidates.some((item) => item.provider === provider)) {
        candidates.push({ provider, info: {} });
      }
    }
    state.providers = candidates.filter((item) => isWalletProvider(item.provider));
    const selector = byId("wallet-provider");
    selector.replaceChildren(...state.providers.map((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item.provider, item.info);
      return option;
    }));
    selector.disabled = state.providers.length === 0;
    if (state.providers.length === 0) {
      throw new Error("No EIP-1193 wallet is exposed to this page. Unlock a browser wallet and reload.");
    }
  }

  function selectedProvider() {
    const item = state.providers[Number.parseInt(byId("wallet-provider").value, 10)];
    if (!item) throw new Error("Select an available wallet provider.");
    state.provider = item.provider;
    return item;
  }

  async function wallet(method, params = []) {
    const provider = state.provider || selectedProvider().provider;
    return provider.request({ method, params });
  }

  async function connect() {
    const accounts = await wallet("eth_requestAccounts");
    if (!accounts || !accounts[0]) throw new Error("The wallet returned no account.");
    const account = accounts[0].toLowerCase();
    if (account !== state.bundle.deployment.from.toLowerCase()) {
      throw new Error(`Select the committed creator wallet ${state.bundle.deployment.from}.`);
    }
    if ((await wallet("eth_chainId")).toLowerCase() !== BASE_CHAIN_ID) {
      await wallet("wallet_switchEthereumChain", [{ chainId: BASE_CHAIN_ID }]);
    }
    state.account = account;
    return account;
  }

  function addressWord(address) {
    return address.toLowerCase().replace(/^0x/, "").padStart(64, "0");
  }

  function uintResult(value) {
    return BigInt(value || "0x0");
  }

  function addressResult(value) {
    return `0x${String(value).replace(/^0x/, "").slice(-40)}`.toLowerCase();
  }

  async function call(to, data) {
    return wallet("eth_call", [{ to, data }, "latest"]);
  }

  async function tokenBalance(address) {
    return uintResult(await call(state.bundle.deployment.settlement_token, `0x70a08231${addressWord(address)}`));
  }

  async function verifyFactory() {
    const deployment = state.bundle.deployment;
    const code = await wallet("eth_getCode", [deployment.expected_factory, "latest"]);
    if (!code || code === "0x") return false;
    const implementation = addressResult(await call(deployment.expected_factory, "0x5c60da1b"));
    const token = addressResult(await call(deployment.expected_factory, "0x7b9e618d"));
    if (implementation !== deployment.expected_implementation.toLowerCase()) {
      throw new Error(`Factory implementation mismatch: ${implementation}`);
    }
    if (token !== deployment.settlement_token.toLowerCase()) {
      throw new Error(`Factory settlement token mismatch: ${token}`);
    }
    return true;
  }

  async function verifyVerifierModule() {
    const deployment = state.verifierBundle.deployment;
    const code = (await wallet("eth_getCode", [VERIFIER_MODULE, "latest"])).toLowerCase();
    if (!code || code === "0x") throw new Error("Deploy the canonical child verifier before funding bounties.");
    if (code !== deployment.expected_runtime_code) throw new Error("Canonical child verifier runtime bytecode mismatch.");
    const factory = addressResult(await call(VERIFIER_MODULE, "0x044f3e72"));
    const token = addressResult(await call(VERIFIER_MODULE, "0x7b9e618d"));
    const criteria = (await call(VERIFIER_MODULE, "0x77de6ca7")).toLowerCase();
    if (
      factory !== state.bundle.deployment.expected_factory.toLowerCase()
      || token !== state.bundle.deployment.settlement_token.toLowerCase()
      || criteria !== ACCEPTANCE_CRITERIA_HASH
    ) {
      throw new Error("Canonical child verifier immutable configuration mismatch.");
    }
  }

  async function bountyIsActivated(bounty) {
    const contract = bounty.predicted_bounty_contract;
    const code = await wallet("eth_getCode", [contract, "latest"]);
    if (!code || code === "0x") return false;
    const canonical = uintResult(await call(state.bundle.deployment.expected_factory, `0xdb021126${addressWord(contract)}`));
    const bountyId = (await call(contract, "0xc17bd75e")).toLowerCase();
    const funded = uintResult(await call(contract, "0x820a5f50"));
    const target = uintResult(await call(contract, "0x953b8fb8"));
    const status = uintResult(await call(contract, "0x200d2ed2"));
    const balance = await tokenBalance(contract);
    const verifier = addressResult(await call(contract, "0x41506fc1"));
    const criteria = (await call(contract, "0x8a2b02be")).toLowerCase();
    const terms = (await call(contract, "0xb311d9fd")).toLowerCase();
    if (
      canonical !== 1n
      || bountyId !== bounty.bounty_id.toLowerCase()
      || funded !== 1_000_000n
      || target !== 1_000_000n
      || balance !== 1_000_000n
      || status !== 1n
      || verifier !== VERIFIER_MODULE
      || criteria !== ACCEPTANCE_CRITERIA_HASH
      || terms !== bounty.commitments.terms_hash.toLowerCase()
    ) {
      throw new Error(`Issue #${bounty.issue} exists but fails the locked canonical funding contract.`);
    }
    return true;
  }

  async function inspect() {
    const target = byId("inspect-output");
    try {
      const account = await connect();
      const nonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "latest"]), 16);
      const eth = uintResult(await wallet("eth_getBalance", [account, "latest"]));
      const usdc = await tokenBalance(account);
      const factoryExists = await verifyFactory();
      if (!factoryExists) throw new Error("The attested canonical factory is unavailable.");
      await verifyVerifierModule();
      const pendingBounties = [];
      for (const bounty of state.bundle.bounties) {
        if (!(await bountyIsActivated(bounty))) pendingBounties.push(bounty);
      }
      const requiredFunding = BigInt(pendingBounties.length) * 1_000_000n;
      if (usdc < requiredFunding) {
        throw new Error(`Wallet has ${Number(usdc) / 1_000_000} USDC; ${Number(requiredFunding) / 1_000_000} USDC is required for the remaining bounties.`);
      }
      state.pendingBounties = pendingBounties;
      state.inspected = true;
      byId("activate").disabled = pendingBounties.length !== state.bundle.bounties.length;
      byId("sequential").hidden = pendingBounties.length === 0 || pendingBounties.length === state.bundle.bounties.length;
      byId("sequential").disabled = pendingBounties.length === 0;
      byId("verify").disabled = false;
      write(target, [
        `Wallet provider: ${providerName(state.provider)}`,
        `Account: ${account}`,
        `Chain: Base mainnet (${BASE_CHAIN_ID})`,
        `Nonce: ${nonce}`,
        `ETH: ${(Number(eth) / 1e18).toFixed(6)}`,
        `USDC: ${(Number(usdc) / 1_000_000).toFixed(6)}`,
        "Factory: deployed and immutable configuration verified",
        "Verifier: deployed and byte-for-byte verified",
        pendingBounties.length === 0
          ? "Bounties: all four are deployed; verify canonical state"
          : `Bounties: ${pendingBounties.length} of 4 remain; ${Number(requiredFunding) / 1_000_000} USDC required`,
      ], "success");
    } catch (error) {
      state.inspected = false;
      state.pendingBounties = [];
      byId("activate").disabled = true;
      byId("sequential").disabled = true;
      byId("sequential").hidden = true;
      byId("verify").disabled = true;
      write(target, error.message || String(error), "error");
    }
  }

  async function waitReceipt(transactionHash, timeoutMilliseconds = 180_000) {
    const deadline = Date.now() + timeoutMilliseconds;
    while (Date.now() < deadline) {
      const receipt = await wallet("eth_getTransactionReceipt", [transactionHash]);
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Transaction reverted: ${transactionHash}`);
        return receipt;
      }
      await sleep(1_500);
    }
    throw new Error(`Transaction confirmation timed out: ${transactionHash}`);
  }

  async function verifyActivation(timeoutMilliseconds = 0) {
    const deadline = Date.now() + timeoutMilliseconds;
    do {
      try {
        if (!(await verifyFactory())) throw new Error("Canonical factory is not deployed.");
        await verifyVerifierModule();
        const results = [];
        for (const bounty of state.bundle.bounties) {
          if (!(await bountyIsActivated(bounty))) {
            throw new Error(`Issue #${bounty.issue} is not yet canonical, fully funded, and claimable.`);
          }
          results.push(`#${bounty.issue}: ${bounty.predicted_bounty_contract} | 1 USDC | claimable`);
        }
        return results;
      } catch (error) {
        if (Date.now() >= deadline) throw error;
        await sleep(2_000);
      }
    } while (true);
  }

  async function showVerifiedActivation(timeoutMilliseconds = 0) {
    const target = byId("activate-output");
    try {
      const results = await verifyActivation(timeoutMilliseconds);
      write(target, ["Canonical activation verified from chain state.", ...results, "Indexer reconciliation is still required before hosted funded/claimable language."], "success");
    } catch (error) {
      write(target, error.message || String(error), "error");
    }
  }

  async function activateBatch() {
    const target = byId("activate-output");
    try {
      await inspect();
      if (!state.inspected) return;
      if (state.pendingBounties.length !== state.bundle.bounties.length) {
        throw new Error("Atomic activation is available only before any seed bounty exists. Use the bounded sequential recovery path.");
      }
      write(target, "Wallet confirmation requested for one exact five-call batch.");
      await wallet("wallet_sendCalls", [{
        version: "2.0.0",
        chainId: BASE_CHAIN_ID,
        from: state.account,
        atomicRequired: true,
        calls: state.bundle.creation_batch.wallet_calls.map((item) => ({ to: item.to, data: item.data, value: "0x0" })),
      }]);
      await showVerifiedActivation(180_000);
    } catch (error) {
      byId("sequential").hidden = false;
      byId("sequential").disabled = false;
      write(target, [`Wallet batch was not completed: ${error.message || String(error)}`, "Use the explicit sequential fallback only if the wallet does not support EIP-5792 batching."], "error");
    }
  }

  async function activateSequential() {
    const target = byId("activate-output");
    byId("sequential").disabled = true;
    try {
      await inspect();
      if (!state.inspected || state.pendingBounties.length === 0) return;
      const approvalTemplate = state.bundle.creation_batch.wallet_calls[0];
      const remainingFunding = BigInt(state.pendingBounties.length) * 1_000_000n;
      const approvalData = `${approvalTemplate.data.slice(0, -64)}${remainingFunding.toString(16).padStart(64, "0")}`;
      const transactions = [{ ...approvalTemplate, data: approvalData }, ...state.pendingBounties.map((bounty) => {
        const index = state.bundle.bounties.findIndex((item) => item.issue === bounty.issue);
        return state.bundle.creation_batch.wallet_calls[index + 1];
      })];
      for (const transaction of transactions) {
        write(target, `Wallet confirmation requested: ${transaction.function}`);
        const hash = await wallet("eth_sendTransaction", [{ from: state.account, to: transaction.to, data: transaction.data, value: "0x0" }]);
        await waitReceipt(hash);
      }
      await showVerifiedActivation(90_000);
    } catch (error) {
      byId("sequential").disabled = false;
      write(target, error.message || String(error), "error");
    }
  }

  async function initialize() {
    try {
      await loadBundle();
    } catch (error) {
      write(byId("inspect-output"), error.message || String(error), "error");
      byId("inspect").disabled = true;
      return;
    }
    try {
      await discoverProviders();
    } catch (error) {
      write(byId("inspect-output"), error.message || String(error), "error");
    }
    byId("wallet-provider").addEventListener("change", () => {
      state.provider = null;
      state.account = null;
      state.pendingBounties = [];
      state.inspected = false;
      byId("activate").disabled = true;
      byId("sequential").disabled = true;
      byId("sequential").hidden = true;
      byId("verify").disabled = true;
    });
    byId("inspect").addEventListener("click", inspect);
    byId("activate").addEventListener("click", activateBatch);
    byId("sequential").addEventListener("click", activateSequential);
    byId("verify").addEventListener("click", () => showVerifiedActivation());
  }

  document.addEventListener("DOMContentLoaded", initialize);
})();
