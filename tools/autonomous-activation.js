(() => {
  "use strict";

  const BASE_CHAIN_ID = "0x2105";
  const BUNDLE_URL = "/deployments/base-mainnet-activation.json";
  const state = { bundle: null, account: null, inspected: false, factoryVerified: false };
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
    const response = await fetch(BUNDLE_URL, { cache: "no-store" });
    if (!response.ok) throw new Error("The checked-in activation bundle is unavailable.");
    const bundle = await response.json();
    if (
      bundle.schema_version !== "agent-bounties/autonomous-activation-bundle-v1"
      || bundle.network !== "base-mainnet"
      || bundle.chain_id !== 8453
      || bundle.deployment.to !== null
      || bundle.deployment.value_wei !== 0
      || bundle.creation_batch.total_initial_funding !== "4000000"
      || bundle.bounties.length !== 4
      || bundle.creation_batch.wallet_calls.length !== 5
    ) {
      throw new Error("Activation bundle violates the capped autonomous-v1 contract.");
    }
    state.bundle = bundle;
    document.querySelector("[data-factory]").textContent = bundle.deployment.expected_factory;
    document.querySelector("[data-implementation]").textContent = bundle.deployment.expected_implementation;
    document.querySelector("[data-creator]").textContent = bundle.deployment.from;
    document.querySelector("[data-bounties]").textContent = bundle.bounties.map((item) => `#${item.issue}`).join(", ");
    document.querySelector("[data-funding]").textContent = `${Number(bundle.creation_batch.total_initial_funding) / 1_000_000} USDC`;
    return bundle;
  }

  async function wallet(method, params = []) {
    if (!window.ethereum) throw new Error("Open this page in a browser with MetaMask or another EIP-1193 wallet.");
    return window.ethereum.request({ method, params });
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
    state.factoryVerified = true;
    byId("activate").disabled = false;
    byId("verify").disabled = false;
    return true;
  }

  async function inspect() {
    const target = byId("inspect-output");
    try {
      const account = await connect();
      const deployment = state.bundle.deployment;
      const nonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "latest"]), 16);
      const eth = uintResult(await wallet("eth_getBalance", [account, "latest"]));
      const usdc = await tokenBalance(account);
      const factoryExists = await verifyFactory();
      if (!factoryExists && nonce !== deployment.deployer_nonce) {
        throw new Error(`Nonce drift: bundle requires ${deployment.deployer_nonce}, wallet is ${nonce}. Regenerate the bundle before signing.`);
      }
      if (!factoryExists && usdc < BigInt(state.bundle.creation_batch.total_initial_funding)) {
        throw new Error(`Wallet has ${Number(usdc) / 1_000_000} USDC; 4 USDC is required.`);
      }
      let estimatedGas = 0n;
      if (!factoryExists) {
        estimatedGas = uintResult(await wallet("eth_estimateGas", [{ from: account, data: deployment.data, value: "0x0" }]));
      }
      state.inspected = true;
      byId("deploy").disabled = factoryExists;
      write(target, [
        `Account: ${account}`,
        `Chain: Base mainnet (${BASE_CHAIN_ID})`,
        `Nonce: ${nonce}`,
        `ETH: ${(Number(eth) / 1e18).toFixed(6)}`,
        `USDC: ${(Number(usdc) / 1_000_000).toFixed(6)}`,
        factoryExists ? "Factory: deployed and configuration verified" : `Factory: empty predicted address; estimated deployment gas ${estimatedGas}`,
      ], "success");
    } catch (error) {
      state.inspected = false;
      byId("deploy").disabled = true;
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
      if (!state.inspected || state.factoryVerified) return;
      write(target, ["Wallet confirmation requested for the exact factory deployment.", `Expected: ${state.bundle.deployment.expected_factory}`]);
      const hash = await wallet("eth_sendTransaction", [{
        from: state.account,
        data: state.bundle.deployment.data,
        value: "0x0",
      }]);
      const receipt = await waitReceipt(hash);
      if (String(receipt.contractAddress).toLowerCase() !== state.bundle.deployment.expected_factory.toLowerCase()) {
        throw new Error(`Receipt contract mismatch: ${receipt.contractAddress}`);
      }
      if (!(await verifyFactory())) throw new Error("Factory receipt succeeded but code is unavailable.");
      byId("deploy").disabled = true;
      write(target, ["Factory deployment confirmed and configuration verified.", `Transaction: https://base.blockscout.com/tx/${hash}`], "success");
    } catch (error) {
      write(target, error.message || String(error), "error");
    }
  }

  async function verifyActivation(timeoutMilliseconds = 0) {
    const deadline = Date.now() + timeoutMilliseconds;
    do {
      try {
        if (!(await verifyFactory())) throw new Error("Canonical factory is not deployed.");
        const results = [];
        for (const bounty of state.bundle.bounties) {
          const contract = bounty.predicted_bounty_contract;
          const canonical = uintResult(await call(state.bundle.deployment.expected_factory, `0xdb021126${addressWord(contract)}`));
          const bountyId = (await call(contract, "0xc17bd75e")).toLowerCase();
          const funded = uintResult(await call(contract, "0x820a5f50"));
          const target = uintResult(await call(contract, "0x953b8fb8"));
          const status = uintResult(await call(contract, "0x200d2ed2"));
          const balance = await tokenBalance(contract);
          if (canonical !== 1n || bountyId !== bounty.bounty_id.toLowerCase() || funded !== 1_000_000n || target !== 1_000_000n || balance !== 1_000_000n || status !== 1n) {
            throw new Error(`Issue #${bounty.issue} is not yet canonical, fully funded, and claimable.`);
          }
          results.push(`#${bounty.issue}: ${contract} | 1 USDC | claimable`);
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
      await connect();
      if (!(await verifyFactory())) throw new Error("Deploy and verify the factory first.");
      write(target, "Wallet confirmation requested for one exact five-call batch.");
      await wallet("wallet_sendCalls", [{
        version: "2.0.0",
        chainId: BASE_CHAIN_ID,
        from: state.account,
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
      await connect();
      for (const transaction of state.bundle.creation_batch.wallet_calls) {
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
    byId("inspect").addEventListener("click", inspect);
    byId("deploy").addEventListener("click", deploy);
    byId("activate").addEventListener("click", activateBatch);
    byId("sequential").addEventListener("click", activateSequential);
    byId("verify").addEventListener("click", () => showVerifiedActivation());
  }

  document.addEventListener("DOMContentLoaded", initialize);
})();
