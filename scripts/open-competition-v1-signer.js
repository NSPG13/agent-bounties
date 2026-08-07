(() => {
  "use strict";

  const EXPECTED_SCHEMA = "agent-bounties/open-competition-v1-deployment-bundle-v1";
  const EXPECTED_PROTOCOL = "agent-bounties/open-competition-v1";
  const EXPECTED_CHAIN = "0x14a34";
  const EXPECTED_ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const READ_RPC_URLS = [
    "https://sepolia.base.org",
    "https://base-sepolia-rpc.publicnode.com",
  ];
  const $ = (id) => document.getElementById(id);
  let bundle;
  let account;
  let results;
  let provider;
  let providerLabel;
  let nextActionIndex = 0;
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

  async function wallet(method, params = []) {
    const selected = provider || selectProvider();
    try {
      return await selected.request({ method, params });
    } catch (error) {
      const code = error && error.code !== undefined ? ` (${error.code})` : "";
      const message = error && error.message ? error.message : String(error);
      throw new Error(`${method} failed${code}: ${message}`);
    }
  }

  async function readRpc(method, params = []) {
    const failures = [];
    for (const url of READ_RPC_URLS) {
      try {
        const response = await fetch(url, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
        });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const payload = await response.json();
        if (payload.error) throw new Error(`${payload.error.code}: ${payload.error.message}`);
        return payload.result;
      } catch (error) {
        failures.push(`${url}: ${error.message}`);
      }
    }
    throw new Error(`${method} failed on every read-only Base Sepolia RPC: ${failures.join("; ")}`);
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

  function loadBundle(value, message) {
    bundle = validateBundle(value);
    renderBundle();
    note(message);
  }

  async function loadLocalBundle() {
    if (!['127.0.0.1', 'localhost'].includes(window.location.hostname)) return;
    const response = await fetch(
      '/target/open-competition-v1/base-sepolia-deployment-bundle.json',
      { cache: 'no-store' },
    );
    if (!response.ok) throw new Error(`Local frozen bundle returned HTTP ${response.status}.`);
    loadBundle(
      await response.json(),
      'Local frozen bundle loaded and validated. Connect the known Brave MetaMask admin account.',
    );
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
    await maybeReady();
  }

  async function maybeReady() {
    if (!bundle || account !== EXPECTED_ADMIN) return;
    const pendingNonce = Number.parseInt(await readRpc("eth_getTransactionCount", [account, "pending"]), 16);
    const frozenNonce = bundle.actions[0].from_nonce;
    nextActionIndex = pendingNonce - frozenNonce;
    if (nextActionIndex < 0 || nextActionIndex > bundle.actions.length) {
      return fail(`Pending nonce ${pendingNonce} is outside the frozen deployment nonce range ${frozenNonce}-${frozenNonce + bundle.actions.length}.`);
    }
    for (const [index, action] of bundle.actions.entries()) {
      const runtime = String(await readRpc("eth_getCode", [action.expected_contract, "latest"])).toLowerCase();
      if (index < nextActionIndex) {
        if (runtime !== action.expected_runtime_code.toLowerCase()) {
          return fail(`Nonce advanced past ${action.name}, but ${action.expected_contract} does not contain its exact frozen runtime.`);
        }
        if (action.expected_implementation) {
          const implementationRuntime = String(await readRpc(
            "eth_getCode",
            [action.expected_implementation, "latest"],
          )).toLowerCase();
          if (implementationRuntime !== action.expected_implementation_runtime_code.toLowerCase()) {
            return fail("Previously deployed factory implementation runtime bytecode mismatch.");
          }
        }
      } else if (runtime !== "0x") {
        return fail(`Predicted address ${action.expected_contract} is already occupied before its frozen nonce.`);
      }
    }
    if (nextActionIndex === bundle.actions.length) {
      results = makeResults();
      note("Both frozen deployments already exist with exact runtime matches. No transaction is required.");
      $("download").disabled = false;
      return;
    }
    $("reviewed").disabled = false;
    const resumed = nextActionIndex === 0
      ? ""
      : `${nextActionIndex} earlier frozen action already has an exact runtime match. `;
    note(`${resumed}Bundle, account, chain, pending nonce, and remaining predicted addresses match. Review the remaining exact actions before deployment.`);
  }

  async function waitForReceipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const receipt = await readRpc("eth_getTransactionReceipt", [hash]);
      if (receipt) return receipt;
      await new Promise((resolve) => setTimeout(resolve, 2000));
    }
    throw new Error(`Timed out waiting for receipt ${hash}.`);
  }

  function makeResults() {
    const value = {
      schema_version: "agent-bounties/open-competition-v1-deployment-receipts-v1",
      protocol_version: EXPECTED_PROTOCOL,
      network: "base-sepolia",
      chain_id: 84532,
      source_commit: bundle.source_commit,
      deployer: account,
      actions: [],
      evidence_boundary: "Confirmed deployment receipts and exact runtime matches are deployment evidence only. They are not rehearsal, activation, settlement, or payment evidence.",
    };
    if (nextActionIndex > 0) {
      value.preexisting_actions = bundle.actions.slice(0, nextActionIndex).map((action) => ({
        name: action.name,
        contract_address: action.expected_contract,
        runtime_code_hash: action.runtime_code_hash,
        runtime_matches: true,
        evidence_note: "Exact runtime was confirmed before this signer session; obtain the canonical transaction receipt separately.",
      }));
    }
    return value;
  }

  async function execute() {
    $("execute").disabled = true;
    $("reviewed").disabled = true;
    results = makeResults();
    for (const action of bundle.actions.slice(nextActionIndex)) {
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
      const runtime = String(await readRpc("eth_getCode", [action.expected_contract, receipt.blockNumber])).toLowerCase();
      if (runtime !== action.expected_runtime_code.toLowerCase()) {
        throw new Error(`${action.name} runtime bytecode mismatch.`);
      }
      if (action.expected_implementation) {
        const implementationRuntime = String(await readRpc(
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
      loadBundle(
        JSON.parse(await event.target.files[0].text()),
        "Frozen bundle loaded. Connect the known Brave MetaMask admin account.",
      );
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
  loadLocalBundle().catch((error) => fail(error.message));
})();
