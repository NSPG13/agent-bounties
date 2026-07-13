(() => {
  "use strict";

  const BASE_CHAIN_ID = "0x2105";
  const FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
  const USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const ACCEPTANCE_CRITERIA_HASH = "0xa103c2c907f96e03a2f2b0e6b2209e0a3ca53686f7e9f79d89d7bfa1f8e314de";
  const BUNDLE_URL = "/deployments/canonical-child-verifier-base-mainnet-deployment.json";
  const state = { bundle: null, account: null, provider: null, providers: [], inspected: false, deployed: false };
  const announcedProviders = [];
  const byId = (id) => document.getElementById(id);
  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

  function write(target, value, tone = "") {
    target.textContent = Array.isArray(value) ? value.join("\n") : value;
    target.dataset.tone = tone;
  }

  function requireLocalOrigin() {
    if (!new Set(["127.0.0.1", "localhost"]).has(location.hostname)) {
      throw new Error("Deployment console must be served from localhost.");
    }
  }

  async function loadBundle() {
    requireLocalOrigin();
    const response = await fetch(BUNDLE_URL, { cache: "no-store" });
    if (!response.ok) throw new Error("The checked-in verifier deployment bundle is unavailable.");
    const bundle = await response.json();
    const deployment = bundle.deployment || {};
    if (
      bundle.schema_version !== "agent-bounties/canonical-child-verifier-deployment-v1"
      || bundle.protocol_version !== "agent-bounties/canonical-child-v1"
      || bundle.network !== "base-mainnet"
      || bundle.chain_id !== 8453
      || bundle.canonical_factory !== FACTORY
      || bundle.settlement_token !== USDC
      || bundle.acceptance_criteria_hash !== ACCEPTANCE_CRITERIA_HASH
      || deployment.to !== null
      || deployment.value_wei !== 0
      || !/^0x[0-9a-f]{40}$/.test(deployment.from)
      || !/^0x[0-9a-f]{40}$/.test(deployment.expected_contract)
      || !/^0x[0-9a-f]+$/.test(deployment.data)
      || !/^0x[0-9a-f]+$/.test(deployment.expected_runtime_code)
    ) {
      throw new Error("Deployment bundle violates the canonical-child-v1 contract.");
    }
    state.bundle = bundle;
    document.querySelector("[data-source-commit]").textContent = bundle.source_commit;
    document.querySelector("[data-factory]").textContent = bundle.canonical_factory;
    document.querySelector("[data-contract]").textContent = deployment.expected_contract;
    document.querySelector("[data-deployer]").textContent = deployment.from;
    document.querySelector("[data-runtime-hash]").textContent = deployment.runtime_code_hash;
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
      if (provider && typeof provider.request === "function" && !candidates.some((item) => item.provider === provider)) {
        candidates.push({ provider, info: {} });
      }
    }
    state.providers = candidates.filter((item) => item.provider && typeof item.provider.request === "function");
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
    if (account !== state.bundle.deployment.from) {
      throw new Error(`Select the committed deployer wallet ${state.bundle.deployment.from}.`);
    }
    if ((await wallet("eth_chainId")).toLowerCase() !== BASE_CHAIN_ID) {
      await wallet("wallet_switchEthereumChain", [{ chainId: BASE_CHAIN_ID }]);
    }
    state.account = account;
    return account;
  }

  function addressResult(value) {
    return `0x${String(value).replace(/^0x/, "").slice(-40)}`.toLowerCase();
  }

  async function call(to, data) {
    return wallet("eth_call", [{ to, data }, "latest"]);
  }

  async function verifyFactory() {
    const code = await wallet("eth_getCode", [FACTORY, "latest"]);
    if (!code || code === "0x") throw new Error("Canonical factory code is unavailable.");
    const token = addressResult(await call(FACTORY, "0x7b9e618d"));
    if (token !== USDC) throw new Error(`Canonical factory token mismatch: ${token}`);
  }

  async function verifyModule() {
    const deployment = state.bundle.deployment;
    const code = (await wallet("eth_getCode", [deployment.expected_contract, "latest"])).toLowerCase();
    if (!code || code === "0x") return false;
    if (code !== deployment.expected_runtime_code) throw new Error("Deployed module runtime bytecode mismatch.");
    const factory = addressResult(await call(deployment.expected_contract, "0x044f3e72"));
    const token = addressResult(await call(deployment.expected_contract, "0x7b9e618d"));
    const criteria = (await call(deployment.expected_contract, "0x77de6ca7")).toLowerCase();
    if (factory !== FACTORY || token !== USDC || criteria !== ACCEPTANCE_CRITERIA_HASH) {
      throw new Error(`Immutable module configuration mismatch: factory=${factory} token=${token} criteria=${criteria}`);
    }
    state.deployed = true;
    byId("verify").disabled = false;
    return true;
  }

  async function inspect() {
    const target = byId("inspect-output");
    try {
      const account = await connect();
      await verifyFactory();
      const deployment = state.bundle.deployment;
      const nonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "latest"]), 16);
      const eth = BigInt(await wallet("eth_getBalance", [account, "latest"]));
      const deployed = await verifyModule();
      let estimatedGas = 0n;
      if (!deployed) {
        if (nonce !== deployment.deployer_nonce) {
          throw new Error(`Nonce drift: bundle requires ${deployment.deployer_nonce}, wallet is ${nonce}. Regenerate the bundle before signing.`);
        }
        estimatedGas = BigInt(await wallet("eth_estimateGas", [{ from: account, data: deployment.data, value: "0x0" }]));
      }
      state.inspected = true;
      byId("deploy").disabled = deployed;
      byId("verify").disabled = !deployed;
      write(target, [
        `Wallet provider: ${providerName(state.provider)}`,
        `Account: ${account}`,
        `Chain: Base mainnet (${BASE_CHAIN_ID})`,
        `Nonce: ${nonce}`,
        `ETH: ${(Number(eth) / 1e18).toFixed(6)}`,
        deployed ? "Module: deployed; runtime and immutable getters verified" : `Target: empty; estimated deployment gas ${estimatedGas}`,
      ], "success");
    } catch (error) {
      state.inspected = false;
      byId("deploy").disabled = true;
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

  async function deploy() {
    const target = byId("deploy-output");
    try {
      await inspect();
      if (!state.inspected || state.deployed) return;
      const deployment = state.bundle.deployment;
      write(target, ["Wallet confirmation requested for the exact zero-value contract creation.", `Expected module: ${deployment.expected_contract}`]);
      const hash = await wallet("eth_sendTransaction", [{ from: state.account, data: deployment.data, value: "0x0" }]);
      const receipt = await waitReceipt(hash);
      if (String(receipt.contractAddress).toLowerCase() !== deployment.expected_contract) {
        throw new Error(`Receipt contract mismatch: ${receipt.contractAddress}`);
      }
      if (!(await verifyModule())) throw new Error("Receipt succeeded but module code is unavailable.");
      byId("deploy").disabled = true;
      write(target, ["Module deployment confirmed; runtime and immutable getters verified.", `Transaction: https://base.blockscout.com/tx/${hash}`], "success");
    } catch (error) {
      write(target, error.message || String(error), "error");
    }
  }

  async function showVerification() {
    const target = byId("deploy-output");
    try {
      await connect();
      if (!(await verifyModule())) throw new Error("Expected module is not deployed.");
      write(target, ["Canonical child verifier is deployed and byte-for-byte verified.", `Module: ${state.bundle.deployment.expected_contract}`, state.bundle.evidence_boundary], "success");
    } catch (error) {
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
      state.inspected = false;
      state.deployed = false;
      byId("deploy").disabled = true;
      byId("verify").disabled = true;
    });
    byId("inspect").addEventListener("click", inspect);
    byId("deploy").addEventListener("click", deploy);
    byId("verify").addEventListener("click", showVerification);
  }

  initialize();
})();
