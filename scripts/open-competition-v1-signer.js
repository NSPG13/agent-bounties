(() => {
  "use strict";

  const EXPECTED_SCHEMA = "agent-bounties/open-competition-v1-deployment-bundle-v1";
  const EXPECTED_PROTOCOL = "agent-bounties/open-competition-v1";
  const EXPECTED_CHAIN = "0x14a34";
  const EXPECTED_ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const $ = (id) => document.getElementById(id);
  let bundle;
  let account;
  let results;
  let provider;
  let providerLabel;
  const announcedProviders = [];
  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));

  function isProvider(candidate) {
    return Boolean(candidate && typeof candidate.request === "function");
  }

  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    if (item.provider.isMetaMask && !item.provider.isBraveWallet) return "MetaMask";
    if (item.provider.isBraveWallet) return "Brave Wallet";
    return "Injected wallet";
  }

  function rememberProvider(event) {
    const detail = event && event.detail;
    if (!detail || !isProvider(detail.provider)) return;
    if (!announcedProviders.some((item) => item.provider === detail.provider)) {
      announcedProviders.push(detail);
    }
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(300);
    const candidates = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const candidate of injected) {
      if (isProvider(candidate) && !candidates.some((item) => item.provider === candidate)) {
        candidates.push({ provider: candidate, info: {} });
      }
    }
    const providers = candidates.filter((item) => isProvider(item.provider));
    const selector = $("wallet-provider");
    selector.replaceChildren();
    if (providers.length === 0) {
      const option = document.createElement("option");
      option.textContent = "No injected wallet detected";
      selector.append(option);
      selector.disabled = true;
      throw new Error("No browser wallet is exposed. Unlock MetaMask and reload this page.");
    }
    providers.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      option.dataset.rdns = item.info && item.info.rdns ? item.info.rdns : "";
      selector.append(option);
    });
    const preferred = providers.findIndex((item) => (
      String(item.info && item.info.rdns).toLowerCase() === "io.metamask"
      || (item.provider.isMetaMask && !item.provider.isBraveWallet)
    ));
    selector.value = String(preferred >= 0 ? preferred : 0);
    selector.disabled = false;
    selector._providers = providers;
    return providers;
  }

  function selectProvider() {
    const selector = $("wallet-provider");
    const item = selector._providers && selector._providers[Number.parseInt(selector.value, 10)];
    if (!item) throw new Error("Select an available MetaMask provider.");
    provider = item.provider;
    providerLabel = providerName(item);
    return provider;
  }

  function wallet(method, params = []) {
    const selected = provider || selectProvider();
    return selected.request({ method, params });
  }

  function fail(message) {
    $("status").className = "status bad";
    $("status").textContent = message;
    $("reviewed").disabled = true;
    $("reviewed").checked = false;
    $("execute").disabled = true;
  }

  function note(message) {
    $("status").className = "status";
    $("status").textContent = message;
  }

  function exactHex(value, bytes, label) {
    if (typeof value !== "string" || !new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value)) {
      throw new Error(`${label} is not ${bytes}-byte hex.`);
    }
    return value.toLowerCase();
  }

  function validateBundle(value) {
    if (!value || value.schema_version !== EXPECTED_SCHEMA || value.protocol_version !== EXPECTED_PROTOCOL) {
      throw new Error("Unsupported deployment bundle schema or protocol.");
    }
    if (value.network !== "base-sepolia" || Number(value.chain_id) !== 84532) {
      throw new Error("Bundle is not pinned to Base Sepolia.");
    }
    if (String(value.deployer).toLowerCase() !== EXPECTED_ADMIN) {
      throw new Error("Bundle deployer is not the frozen admin wallet.");
    }
    if (!Array.isArray(value.actions) || value.actions.length !== 2) {
      throw new Error("Bundle must contain exactly two deployments.");
    }
    value.actions.forEach((action, index) => {
      if (action.to !== null || action.value_wei !== 0 || action.from_nonce !== value.preflight_block.deployer_nonce + index) {
        throw new Error(`Action ${index + 1} has unexpected target, value, or nonce.`);
      }
      exactHex(action.expected_contract, 20, `action ${index + 1} contract`);
      exactHex(action.runtime_code_hash, 32, `action ${index + 1} runtime hash`);
      if (!/^0x[0-9a-fA-F]+$/.test(action.data) || action.data.length < 10) {
        throw new Error(`Action ${index + 1} has invalid deployment calldata.`);
      }
      if (!/^0x[0-9a-fA-F]+$/.test(action.expected_runtime_code)) {
        throw new Error(`Action ${index + 1} has invalid expected runtime.`);
      }
    });
    return value;
  }

  function renderBundle() {
    const facts = $("facts");
    facts.hidden = false;
    facts.innerHTML = [
      ["Source commit", bundle.source_commit],
      ["Admin", bundle.deployer],
      ["Pinned block", `${bundle.preflight_block.number} · ${bundle.preflight_block.hash}`],
      ["Pinned nonce", bundle.preflight_block.deployer_nonce],
      ["Test USDC", bundle.preflight_block.deployer_usdc_base_units],
      ["Deployment state", bundle.deployment_state],
    ].map(([label, value]) => `<div class="fact"><span>${label}</span><strong>${String(value)}</strong></div>`).join("");
    $("actions").innerHTML = bundle.actions.map((action, index) => `
      <article class="action">
        <h3>${index + 1}. ${action.name}</h3>
        <dl>
          <dt>Nonce</dt><dd>${action.from_nonce}</dd>
          <dt>Predicted</dt><dd><code>${action.expected_contract}</code></dd>
          <dt>Runtime hash</dt><dd><code>${action.runtime_code_hash}</code></dd>
          <dt>Runtime bytes</dt><dd>${action.runtime_code_bytes}</dd>
          ${action.expected_implementation ? `<dt>Implementation</dt><dd><code>${action.expected_implementation}</code></dd>` : ""}
        </dl>
        <details><summary>Exact deployment calldata (${(action.data.length - 2) / 2} bytes)</summary><code>${action.data}</code></details>
      </article>`).join("");
    maybeReady();
  }

  async function connect() {
    if (!bundle) throw new Error("Load and inspect the frozen bundle before connecting a wallet.");
    if (!$("wallet-provider")._providers) await discoverProviders();
    selectProvider();
    let chain = await wallet("eth_chainId");
    if (chain.toLowerCase() !== EXPECTED_CHAIN) {
      try {
        await wallet("wallet_switchEthereumChain", [{ chainId: EXPECTED_CHAIN }]);
      } catch (error) {
        if (!error || error.code !== 4902) throw error;
        await wallet("wallet_addEthereumChain", [{
          chainId: EXPECTED_CHAIN,
          chainName: "Base Sepolia",
          nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
          rpcUrls: ["https://sepolia.base.org"],
          blockExplorerUrls: ["https://sepolia.basescan.org"],
        }]);
      }
      chain = await wallet("eth_chainId");
    }
    if (chain.toLowerCase() !== EXPECTED_CHAIN) throw new Error("Wallet did not switch to Base Sepolia.");
    const accounts = await wallet("eth_requestAccounts");
    account = String(accounts[0] || "").toLowerCase();
    if (account !== EXPECTED_ADMIN) throw new Error(`Connected account ${account || "(none)"} is not the frozen admin.`);
    note(`Connected ${providerLabel} to frozen admin ${account} on Base Sepolia.`);
    maybeReady();
  }

  async function maybeReady() {
    if (!bundle || account !== EXPECTED_ADMIN) return;
    const pendingNonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "pending"]), 16);
    if (pendingNonce !== bundle.actions[0].from_nonce) {
      return fail(`Pending nonce ${pendingNonce} does not match frozen nonce ${bundle.actions[0].from_nonce}. Regenerate the bundle.`);
    }
    for (const action of bundle.actions) {
      const occupied = await wallet("eth_getCode", [action.expected_contract, "latest"]);
      if (occupied !== "0x") return fail(`Predicted address ${action.expected_contract} is already occupied.`);
    }
    $("reviewed").disabled = false;
    note("Bundle, account, chain, pending nonce, and predicted addresses match. Review exact actions before deployment.");
  }

  async function waitForReceipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const receipt = await wallet("eth_getTransactionReceipt", [hash]);
      if (receipt) return receipt;
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
    throw new Error(`Timed out waiting for receipt ${hash}.`);
  }

  async function execute() {
    $("execute").disabled = true;
    $("reviewed").disabled = true;
    results = {
      schema_version: "agent-bounties/open-competition-v1-deployment-receipts-v1",
      protocol_version: EXPECTED_PROTOCOL,
      network: "base-sepolia",
      chain_id: 84532,
      source_commit: bundle.source_commit,
      deployer: account,
      actions: [],
      evidence_boundary: "Confirmed deployment receipts and exact runtime matches are deployment evidence only. They are not rehearsal, activation, settlement, or payment evidence.",
    };
    for (const action of bundle.actions) {
      note(`Requesting ${providerLabel} signature for ${action.name}…`);
      const hash = await wallet("eth_sendTransaction", [{
        from: account,
        data: action.data,
        value: "0x0",
        nonce: `0x${action.from_nonce.toString(16)}`,
      }]);
      const receipt = await waitForReceipt(hash);
      if (Number.parseInt(receipt.status, 16) !== 1) throw new Error(`${action.name} reverted.`);
      if (String(receipt.contractAddress).toLowerCase() !== action.expected_contract) {
        throw new Error(`${action.name} deployed at an unexpected address.`);
      }
      const runtime = String(await wallet("eth_getCode", [action.expected_contract, receipt.blockNumber])).toLowerCase();
      if (runtime !== action.expected_runtime_code.toLowerCase()) {
        throw new Error(`${action.name} runtime bytecode mismatch.`);
      }
      if (action.expected_implementation) {
        const implementationRuntime = String(await wallet(
          "eth_getCode",
          [action.expected_implementation, receipt.blockNumber],
        )).toLowerCase();
        if (implementationRuntime !== action.expected_implementation_runtime_code.toLowerCase()) {
          throw new Error("Factory implementation runtime bytecode mismatch.");
        }
      }
      results.actions.push({
        name: action.name,
        transaction_hash: hash,
        block_number: Number.parseInt(receipt.blockNumber, 16),
        block_hash: receipt.blockHash,
        contract_address: receipt.contractAddress,
        gas_used: Number.parseInt(receipt.gasUsed, 16),
        runtime_code_hash: action.runtime_code_hash,
        runtime_matches: true,
        implementation_address: action.expected_implementation || null,
        implementation_runtime_code_hash: action.implementation_runtime_code_hash || null,
      });
    }
    note("Both frozen deployments are confirmed with exact runtime matches. Download receipts; rehearsal is still required.");
    $("download").disabled = false;
  }

  function download() {
    const blob = new Blob([`${JSON.stringify(results, null, 2)}\n`], { type: "application/json" });
    const anchor = document.createElement("a");
    anchor.href = URL.createObjectURL(blob);
    anchor.download = `open-competition-v1-base-sepolia-deployment-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(anchor.href);
  }

  $("bundle").addEventListener("change", async (event) => {
    try {
      bundle = validateBundle(JSON.parse(await event.target.files[0].text()));
      renderBundle();
      note("Frozen bundle loaded. Connect the known Brave MetaMask admin account.");
    } catch (error) { fail(error.message); }
  });
  $("connect").addEventListener("click", () => connect().catch((error) => fail(error.message)));
  $("wallet-provider").addEventListener("change", () => {
    provider = null;
    providerLabel = null;
    account = null;
    $("reviewed").disabled = true;
    $("reviewed").checked = false;
    $("execute").disabled = true;
    note("Wallet provider changed. Connect it to re-run every account and chain check.");
  });
  $("reviewed").addEventListener("change", () => {
    $("execute").disabled = !$("reviewed").checked || !bundle || account !== EXPECTED_ADMIN;
  });
  $("execute").addEventListener("click", () => execute().catch((error) => fail(error.message)));
  $("download").addEventListener("click", download);
  discoverProviders().catch((error) => fail(error.message));
})();
