(() => {
  "use strict";

  const SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-release-bundle-v1";
  const CHAIN = "0x2105";
  const ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const SOURCE_TREE = "cb3476158ee39a8928dba73da6861d5f782792ce";
  const CREATE2 = "0x4e59b44847b379578588920ca78fbf26c0b4956c";
  const EMPTY_HASH = "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470";
  const RPCS = ["https://mainnet.base.org", "https://base-rpc.publicnode.com"];
  const $ = (id) => document.getElementById(id);
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const providers = [];
  let bundle;
  let provider;
  let providerLabel;
  let account;
  let result;

  function exactHex(value, bytes, label) {
    if (typeof value !== "string" || !new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value)) throw new Error(`${label} is malformed.`);
    return value.toLowerCase();
  }
  function fail(message) {
    $("status").className = "status bad";
    $("status").textContent = message;
    $("reviewed").disabled = true;
    $("reviewed").checked = false;
    $("execute").disabled = true;
  }
  function note(message) { $("status").className = "status"; $("status").textContent = message; }
  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    return item.provider.isMetaMask && !item.provider.isBraveWallet ? "MetaMask" : "Injected wallet";
  }
  function rememberProvider(event) {
    const item = event && event.detail;
    if (item && item.provider && typeof item.provider.request === "function" && !providers.some((known) => known.provider === item.provider)) providers.push(item);
  }
  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(300);
    const injected = window.ethereum && Array.isArray(window.ethereum.providers) ? window.ethereum.providers : (window.ethereum ? [window.ethereum] : []);
    for (const candidate of injected) if (candidate && typeof candidate.request === "function" && !providers.some((item) => item.provider === candidate)) providers.push({ provider: candidate, info: {} });
    if (!providers.length) throw new Error("No injected wallet detected. Unlock Brave MetaMask and reload.");
    const selector = $("wallet-provider");
    selector.replaceChildren();
    providers.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      selector.append(option);
    });
    const preferred = providers.findIndex((item) => String(item.info && item.info.rdns).toLowerCase() === "io.metamask" || (item.provider.isMetaMask && !item.provider.isBraveWallet));
    selector.value = String(preferred >= 0 ? preferred : 0);
    selector.disabled = false;
  }
  function selectProvider() {
    const item = providers[Number.parseInt($("wallet-provider").value, 10)];
    if (!item) throw new Error("Select an injected wallet.");
    provider = item.provider;
    providerLabel = providerName(item);
  }
  async function wallet(method, params = []) {
    if (!provider) selectProvider();
    try { return await provider.request({ method, params }); }
    catch (error) { throw new Error(`${method} failed${error && error.code !== undefined ? ` (${error.code})` : ""}: ${error && error.message ? error.message : String(error)}`); }
  }
  async function rpc(method, params = []) {
    const errors = [];
    for (const url of RPCS) {
      try {
        const response = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const body = await response.json();
        if (body.error) throw new Error(body.error.message);
        return body.result;
      } catch (error) { errors.push(`${url}: ${error.message}`); }
    }
    throw new Error(`${method} failed on every read-only Base RPC: ${errors.join("; ")}`);
  }
  function hashRuntime(code) { return AgentBountiesEvm.keccak256Hex(code).toLowerCase(); }

  function validateBundle(value) {
    if (!value || value.schema_version !== SCHEMA || value.network !== "base-mainnet" || Number(value.chain_id) !== 8453) throw new Error("Unsupported release bundle.");
    if (value.contract_source_revision !== SOURCE_TREE || value.admin.toLowerCase() !== ADMIN) throw new Error("Release source tree or admin mismatch.");
    if (value.deployment_state !== "mainnet_canary_not_ready_to_earn") throw new Error("Unsafe deployment state.");
    if (!value.release_evidence.base_sepolia_rehearsal_passed || !value.release_evidence.exact_mainnet_fork_replay_passed || !value.release_evidence.static_analysis_passed || !value.release_evidence.frozen_bytecode) throw new Error("Required release evidence is incomplete.");
    if (value.release_evidence.independent_review !== "timeboxed_and_waived_by_admin") throw new Error("Independent-review disposition mismatch.");
    if (Object.values(value.activation).some(Boolean)) throw new Error("Every hosted and public activation gate must remain false.");
    if (!value.signing_constraints.explicit_wallet_confirmation_required || !value.signing_constraints.single_zero_value_create2_call || value.signing_constraints.existing_bounties_or_contributors_touched) throw new Error("Signing constraints are unsafe.");
    const action = value.action;
    if (action.name !== "deploy_open_competition_entrant_wallet_factory_v1" || action.from.toLowerCase() !== ADMIN || action.to.toLowerCase() !== CREATE2 || Number(action.value_wei) !== 0) throw new Error("Deployment action sender, target, or value mismatch.");
    exactHex(action.expected_factory, 20, "factory");
    exactHex(action.expected_implementation, 20, "implementation");
    exactHex(action.factory_runtime_code_hash, 32, "factory runtime hash");
    exactHex(action.implementation_runtime_code_hash, 32, "implementation runtime hash");
    exactHex(action.clone_runtime_code_hash, 32, "clone runtime hash");
    if (!/^0x[0-9a-fA-F]+$/.test(action.data)) throw new Error("Deployment calldata is malformed.");
    return value;
  }
  function render() {
    const action = bundle.action;
    $("facts").hidden = false;
    $("facts").innerHTML = [
      ["Contract tree", bundle.contract_source_revision],
      ["Admin", bundle.admin],
      ["Pinned safe block", `${bundle.preflight_safe_block.number} · ${bundle.preflight_safe_block.hash}`],
      ["Pinned ETH wei", bundle.preflight_safe_block.admin_eth_wei],
      ["Pinned USDC base units", bundle.preflight_safe_block.admin_usdc_base_units],
      ["Existing 1 USDC canary", bundle.preserved_hidden_canary.settlement_transaction],
      ["Public activation", "OFF"],
    ].map(([label, value]) => `<div class="fact"><span>${label}</span><strong>${String(value)}</strong></div>`).join("");
    $("action").innerHTML = `<article class="action"><h3>Exact CREATE2 deployment</h3><dl><dt>Target</dt><dd><code>${action.to}</code></dd><dt>Value</dt><dd>0 ETH</dd><dt>Predicted factory</dt><dd><code>${action.expected_factory}</code></dd><dt>Implementation</dt><dd><code>${action.expected_implementation}</code></dd><dt>Factory runtime</dt><dd><code>${action.factory_runtime_code_hash}</code></dd><dt>Implementation runtime</dt><dd><code>${action.implementation_runtime_code_hash}</code></dd></dl><details><summary>Exact calldata (${(action.data.length - 2) / 2} bytes)</summary><code>${action.data}</code></details></article>`;
  }
  function loadBundle(value, message) { bundle = validateBundle(value); render(); note(message); }
  async function loadLocal() {
    if (!["127.0.0.1", "localhost"].includes(window.location.hostname)) return;
    const response = await fetch("/target/open-competition-entrant-wallet/base-mainnet-release-bundle.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`Local release bundle returned HTTP ${response.status}.`);
    loadBundle(await response.json(), "Audited release bundle loaded. Connect the known Brave MetaMask admin.");
  }

  async function connect() {
    if (!bundle) throw new Error("Load the audited bundle first.");
    selectProvider();
    let chain = String(await wallet("eth_chainId")).toLowerCase();
    if (chain !== CHAIN) {
      await wallet("wallet_switchEthereumChain", [{ chainId: CHAIN }]);
      chain = String(await wallet("eth_chainId")).toLowerCase();
    }
    if (chain !== CHAIN) throw new Error("Wallet did not switch to Base mainnet.");
    const accounts = await wallet("eth_requestAccounts");
    account = String(accounts[0] || "").toLowerCase();
    if (account !== ADMIN) throw new Error(`Connected account ${account || "(none)"} is not the frozen admin.`);
    await preflight();
  }
  async function preflight() {
    const pinned = bundle.preflight_safe_block;
    const action = bundle.action;
    const block = await rpc("eth_getBlockByNumber", [`0x${Number(pinned.number).toString(16)}`, false]);
    if (!block || String(block.hash).toLowerCase() !== String(pinned.hash).toLowerCase()) throw new Error("Pinned Base safe block is no longer canonical.");
    const dependencyEntries = [
      [bundle.canonical_dependencies.competition_factory, bundle.canonical_dependencies.competition_factory_runtime_code_hash, "competition factory"],
      [bundle.canonical_dependencies.settlement_token, bundle.canonical_dependencies.settlement_token_runtime_code_hash, "native USDC"],
      [bundle.canonical_dependencies.approved_canary_verifier, bundle.canonical_dependencies.approved_canary_verifier_runtime_code_hash, "approved verifier"],
      [bundle.canonical_dependencies.deterministic_deployer, bundle.canonical_dependencies.deterministic_deployer_runtime_code_hash, "CREATE2 deployer"],
    ];
    for (const [address, expected, label] of dependencyEntries) {
      const observed = hashRuntime(String(await rpc("eth_getCode", [address, "latest"])).toLowerCase());
      if (observed !== String(expected).toLowerCase()) throw new Error(`${label} runtime changed.`);
    }
    const factoryCode = String(await rpc("eth_getCode", [action.expected_factory, "latest"])).toLowerCase();
    const implementationCode = String(await rpc("eth_getCode", [action.expected_implementation, "latest"])).toLowerCase();
    if (factoryCode !== "0x" || implementationCode !== "0x") {
      if (hashRuntime(factoryCode) === action.factory_runtime_code_hash.toLowerCase() && hashRuntime(implementationCode) === action.implementation_runtime_code_hash.toLowerCase()) {
        result = await makeResult(null, null);
        $("download").disabled = false;
        note("The exact entrant factory and implementation already exist. No transaction is required; public activation remains off.");
        return;
      }
      throw new Error("A predicted deployment address is occupied by unexpected code.");
    }
    const eth = BigInt(await rpc("eth_getBalance", [account, "latest"]));
    if (eth < BigInt(pinned.minimum_admin_eth_wei)) throw new Error(`Admin ETH ${eth} is below the bounded signing minimum ${pinned.minimum_admin_eth_wei}.`);
    $("reviewed").disabled = false;
    note(`Signing-time preflight passed for ${providerLabel}: ${eth} wei, exact runtimes, vacant addresses, and public activation OFF.`);
  }
  async function receipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const value = await rpc("eth_getTransactionReceipt", [hash]);
      if (value) return value;
      await sleep(2000);
    }
    throw new Error(`Timed out waiting for ${hash}.`);
  }
  async function makeResult(hash, confirmed) {
    return {
      schema_version: "agent-bounties/open-competition-entrant-wallet-mainnet-deployment-receipt-v1",
      network: "base-mainnet",
      chain_id: 8453,
      contract_source_revision: bundle.contract_source_revision,
      admin: ADMIN,
      transaction_hash: hash,
      block_number: confirmed ? Number.parseInt(confirmed.blockNumber, 16) : null,
      block_hash: confirmed ? confirmed.blockHash : null,
      factory: bundle.action.expected_factory,
      implementation: bundle.action.expected_implementation,
      factory_runtime_code_hash: bundle.action.factory_runtime_code_hash,
      implementation_runtime_code_hash: bundle.action.implementation_runtime_code_hash,
      runtime_matches: true,
      public_creation_enabled: false,
      public_commitments_enabled: false,
      public_inventory_enabled: false,
      evidence_boundary: "Deployment and exact runtime evidence only; not hosted relay, gas sponsorship, public activation, bounty settlement, or payment evidence.",
    };
  }
  async function execute() {
    $("execute").disabled = true;
    $("reviewed").disabled = true;
    note("Requesting MetaMask confirmation for the exact zero-value CREATE2 deployment…");
    const action = bundle.action;
    const hash = await wallet("eth_sendTransaction", [{ from: account, to: action.to, value: "0x0", data: action.data }]);
    const confirmed = await receipt(hash);
    if (Number.parseInt(confirmed.status, 16) !== 1) throw new Error("Entrant factory deployment reverted.");
    const factoryCode = String(await rpc("eth_getCode", [action.expected_factory, confirmed.blockNumber])).toLowerCase();
    const implementationCode = String(await rpc("eth_getCode", [action.expected_implementation, confirmed.blockNumber])).toLowerCase();
    if (hashRuntime(factoryCode) !== action.factory_runtime_code_hash.toLowerCase() || hashRuntime(implementationCode) !== action.implementation_runtime_code_hash.toLowerCase()) throw new Error("Post-deployment runtime hash mismatch.");
    result = await makeResult(hash, confirmed);
    $("download").disabled = false;
    note("Frozen entrant factory deployed with exact runtimes. Hosted relay and public activation remain off.");
  }
  function download() {
    const blob = new Blob([`${JSON.stringify(result, null, 2)}\n`], { type: "application/json" });
    const anchor = document.createElement("a");
    anchor.href = URL.createObjectURL(blob);
    anchor.download = `open-competition-entrant-wallet-mainnet-deployment-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(anchor.href);
  }

  $("bundle").addEventListener("change", async (event) => {
    try { loadBundle(JSON.parse(await event.target.files[0].text()), "Release bundle loaded. Connect the known admin."); }
    catch (error) { fail(error.message); }
  });
  $("connect").addEventListener("click", () => connect().catch((error) => fail(error.message)));
  $("wallet-provider").addEventListener("change", () => { provider = null; account = null; fail("Wallet provider changed. Reconnect to rerun all checks."); });
  $("reviewed").addEventListener("change", () => { $("execute").disabled = !$("reviewed").checked || account !== ADMIN; });
  $("execute").addEventListener("click", () => execute().catch((error) => fail(error.message)));
  $("download").addEventListener("click", download);
  discoverProviders().catch((error) => fail(error.message));
  loadLocal().catch((error) => fail(error.message));
})();
