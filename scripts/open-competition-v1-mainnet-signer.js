(() => {
  "use strict";

  const EXPECTED_SCHEMA = "agent-bounties/open-competition-v1-mainnet-bundle-v1";
  const EXPECTED_PROTOCOL = "agent-bounties/open-competition-v1";
  const EXPECTED_SOURCE_COMMIT = "bc9b3cc9f9f95a87df671be2d13199ac9d06ebcf";
  const EXPECTED_CHAIN = "0x2105";
  const EXPECTED_ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const EXPECTED_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const EXPECTED_VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e";
  const EXPECTED_DIFFICULTY = 16n;
  const MIN_ETH_WEI = 100_000_000_000_000n;
  const READ_RPC_URLS = ["https://mainnet.base.org", "https://base-rpc.publicnode.com"];
  const $ = (id) => document.getElementById(id);
  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  const announcedProviders = [];
  let bundle;
  let provider;
  let providerLabel;
  let account;
  let result;

  function isProvider(candidate) { return Boolean(candidate && typeof candidate.request === "function"); }
  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    if (item.provider.isMetaMask && !item.provider.isBraveWallet) return "MetaMask";
    return item.provider.isBraveWallet ? "Brave Wallet" : "Injected wallet";
  }
  function rememberProvider(event) {
    const detail = event && event.detail;
    if (detail && isProvider(detail.provider) && !announcedProviders.some((item) => item.provider === detail.provider)) announcedProviders.push(detail);
  }
  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await sleep(300);
    const candidates = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers) ? window.ethereum.providers : (window.ethereum ? [window.ethereum] : []);
    for (const candidate of injected) if (isProvider(candidate) && !candidates.some((item) => item.provider === candidate)) candidates.push({ provider: candidate, info: {} });
    const choices = candidates.filter((item) => isProvider(item.provider));
    const selector = $("wallet-provider");
    selector.replaceChildren();
    if (!choices.length) throw new Error("No injected wallet detected. Unlock MetaMask and reload.");
    choices.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      selector.append(option);
    });
    const preferred = choices.findIndex((item) => String(item.info && item.info.rdns).toLowerCase() === "io.metamask" || (item.provider.isMetaMask && !item.provider.isBraveWallet));
    selector.value = String(preferred >= 0 ? preferred : 0);
    selector._providers = choices;
    selector.disabled = false;
  }

  function selectProvider() {
    const selector = $("wallet-provider");
    const item = selector._providers && selector._providers[Number.parseInt(selector.value, 10)];
    if (!item) throw new Error("Select an available MetaMask provider.");
    provider = item.provider;
    providerLabel = providerName(item);
  }
  async function wallet(method, params = []) {
    if (!provider) selectProvider();
    try { return await provider.request({ method, params }); }
    catch (error) {
      const code = error && error.code !== undefined ? ` (${error.code})` : "";
      throw new Error(`${method} failed${code}: ${error && error.message ? error.message : String(error)}`);
    }
  }
  async function readRpc(method, params = []) {
    const failures = [];
    for (const url of READ_RPC_URLS) {
      try {
        const response = await fetch(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const value = await response.json();
        if (value.error) throw new Error(`${value.error.code}: ${value.error.message}`);
        return value.result;
      } catch (error) { failures.push(`${url}: ${error.message}`); }
    }
    throw new Error(`${method} failed on every read-only Base RPC: ${failures.join("; ")}`);
  }
  function fail(message) {
    $("status").className = "status bad";
    $("status").textContent = message;
    $("reviewed").disabled = true;
    $("reviewed").checked = false;
    $("execute").disabled = true;
  }
  function note(message) { $("status").className = "status"; $("status").textContent = message; }
  function exactHex(value, bytes, label) {
    if (typeof value !== "string" || !new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value)) throw new Error(`${label} is not ${bytes}-byte hex.`);
    return value.toLowerCase();
  }
  function addressWord(value) { return `${"0".repeat(24)}${exactHex(value, 20, "address").slice(2)}`; }

  function validateBundle(value) {
    if (!value || value.schema_version !== EXPECTED_SCHEMA || value.protocol_version !== EXPECTED_PROTOCOL) throw new Error("Unsupported mainnet bundle schema or protocol.");
    if (value.network !== "base-mainnet" || Number(value.chain_id) !== 8453) throw new Error("Bundle is not pinned to Base mainnet.");
    if (value.source_commit !== EXPECTED_SOURCE_COMMIT) throw new Error("Bundle does not use the frozen source commit.");
    if (String(value.deployer).toLowerCase() !== EXPECTED_ADMIN || String(value.settlement_token).toLowerCase() !== EXPECTED_USDC) throw new Error("Unexpected deployer or settlement token.");
    if (value.deployment_state !== "sepolia_rehearsed_not_ready_to_earn") throw new Error("Unexpected pre-deployment release state.");
    if (value.activation.public_creation_enabled || value.activation.public_commitments_enabled || value.activation.public_inventory_eligible) throw new Error("Bundle must keep every public activation gate off.");
    if (value.hidden_canary.total_admin_usdc_budget_base_units !== 1_200_000 || value.hidden_canary.solver_reward_usdc_base_units !== 1_000_000 || value.hidden_canary.verifier_reward_usdc_base_units !== 100_000 || value.hidden_canary.entry_bond_usdc_base_units !== 100_000 || value.hidden_canary.max_entries !== 4 || value.hidden_canary.creator_may_compete !== false || value.hidden_canary.inventory_visibility !== "hidden") throw new Error("Hidden canary bounds do not match the release plan.");
    if (String(value.verifier_profile.verifier_address).toLowerCase() !== EXPECTED_VERIFIER || BigInt(value.verifier_profile.difficulty_bits) !== EXPECTED_DIFFICULTY || value.verifier_profile.public_inventory_eligible !== false) throw new Error("Verifier profile is not the pinned canary-only profile.");
    if (!Array.isArray(value.actions) || value.actions.length !== 1) throw new Error("Mainnet bundle must contain exactly one factory deployment.");
    const action = value.actions[0];
    if (action.name !== "deploy_open_competition_factory_v1" || action.to !== null || action.value_wei !== 0 || action.from_nonce !== value.preflight_block.deployer_nonce) throw new Error("Factory action target, value, or nonce is invalid.");
    exactHex(action.expected_contract, 20, "factory");
    exactHex(action.expected_implementation, 20, "implementation");
    exactHex(action.runtime_code_hash, 32, "factory runtime hash");
    exactHex(action.implementation_runtime_code_hash, 32, "implementation runtime hash");
    if (!/^0x[0-9a-fA-F]+$/.test(action.data) || !/^0x[0-9a-fA-F]+$/.test(action.expected_runtime_code) || !/^0x[0-9a-fA-F]+$/.test(action.expected_implementation_runtime_code)) throw new Error("Factory bytecode fields are malformed.");
    return value;
  }

  function renderBundle() {
    const action = bundle.actions[0];
    $("facts").hidden = false;
    $("facts").innerHTML = [
      ["Source commit", bundle.source_commit], ["Admin", bundle.deployer],
      ["Pinned safe block", `${bundle.preflight_block.number} · ${bundle.preflight_block.hash}`],
      ["Pinned nonce", bundle.preflight_block.deployer_nonce], ["Pinned ETH wei", bundle.preflight_block.deployer_eth_wei],
      ["Pinned USDC", bundle.preflight_block.deployer_usdc_base_units], ["Public activation", "OFF"],
    ].map(([label, value]) => `<div class="fact"><span>${label}</span><strong>${String(value)}</strong></div>`).join("");
    $("actions").innerHTML = `<article class="action"><h3>Frozen factory deployment</h3><dl><dt>Nonce</dt><dd>${action.from_nonce}</dd><dt>Predicted factory</dt><dd><code>${action.expected_contract}</code></dd><dt>Predicted implementation</dt><dd><code>${action.expected_implementation}</code></dd><dt>Factory runtime hash</dt><dd><code>${action.runtime_code_hash}</code></dd><dt>Implementation hash</dt><dd><code>${action.implementation_runtime_code_hash}</code></dd></dl><details><summary>Exact deployment calldata (${(action.data.length - 2) / 2} bytes)</summary><code>${action.data}</code></details></article>`;
  }
  function loadBundle(value, message) { bundle = validateBundle(value); renderBundle(); note(message); }
  async function loadLocalBundle() {
    if (!["127.0.0.1", "localhost"].includes(window.location.hostname)) return;
    const response = await fetch("/target/open-competition-v1/base-mainnet-deployment-canary-bundle.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`Local audited mainnet bundle returned HTTP ${response.status}.`);
    loadBundle(await response.json(), "Audited local mainnet bundle loaded. Connect the known Brave MetaMask admin.");
  }

  async function connect() {
    if (!bundle) throw new Error("Load and inspect the audited mainnet bundle before connecting a wallet.");
    if (!$("wallet-provider")._providers) await discoverProviders();
    selectProvider();
    let chain = String(await wallet("eth_chainId")).toLowerCase();
    if (chain !== EXPECTED_CHAIN) {
      await wallet("wallet_switchEthereumChain", [{ chainId: EXPECTED_CHAIN }]);
      chain = String(await wallet("eth_chainId")).toLowerCase();
    }
    if (chain !== EXPECTED_CHAIN) throw new Error("Wallet did not switch to Base mainnet.");
    const accounts = await wallet("eth_requestAccounts");
    account = String(accounts[0] || "").toLowerCase();
    if (account !== EXPECTED_ADMIN) throw new Error(`Connected account ${account || "(none)"} is not the frozen admin.`);
    await preflight();
  }

  async function preflight() {
    const action = bundle.actions[0];
    const pinnedBlock = await readRpc("eth_getBlockByNumber", [`0x${bundle.preflight_block.number.toString(16)}`, false]);
    if (!pinnedBlock || String(pinnedBlock.hash).toLowerCase() !== String(bundle.preflight_block.hash).toLowerCase()) throw new Error("Pinned Base safe block is no longer canonical.");
    const pendingNonce = Number.parseInt(await readRpc("eth_getTransactionCount", [account, "pending"]), 16);
    const factoryRuntime = String(await readRpc("eth_getCode", [action.expected_contract, "latest"])).toLowerCase();
    const implementationRuntime = String(await readRpc("eth_getCode", [action.expected_implementation, "latest"])).toLowerCase();
    if (pendingNonce === action.from_nonce + 1 && factoryRuntime === action.expected_runtime_code.toLowerCase() && implementationRuntime === action.expected_implementation_runtime_code.toLowerCase()) {
      note("The frozen deployment already exists with exact factory and implementation runtimes. No transaction is required.");
      result = makeResult(null, null);
      $("download").disabled = false;
      return;
    }
    if (pendingNonce !== action.from_nonce) throw new Error(`Pending nonce ${pendingNonce} does not match frozen nonce ${action.from_nonce}.`);
    if (factoryRuntime !== "0x" || implementationRuntime !== "0x") throw new Error("A predicted deployment address is already occupied.");
    const ethBalance = BigInt(await readRpc("eth_getBalance", [account, "pending"]));
    const usdcRaw = await readRpc("eth_call", [{ to: EXPECTED_USDC, data: `0x70a08231${addressWord(account)}` }, "latest"]);
    const usdcBalance = BigInt(usdcRaw);
    if (ethBalance < MIN_ETH_WEI) throw new Error(`Admin ETH ${ethBalance} is below the bounded signing minimum ${MIN_ETH_WEI}.`);
    if (usdcBalance < BigInt(bundle.hidden_canary.total_admin_usdc_budget_base_units)) throw new Error(`Admin USDC ${usdcBalance} is below the exact canary budget.`);
    const verifierRuntime = String(await readRpc("eth_getCode", [EXPECTED_VERIFIER, "latest"])).toLowerCase();
    if (AgentBountiesEvm.keccak256Hex(verifierRuntime).toLowerCase() !== String(bundle.verifier_profile.runtime_code_hash).toLowerCase()) throw new Error("Pinned verifier runtime hash mismatch.");
    const difficulty = BigInt(await readRpc("eth_call", [{ to: EXPECTED_VERIFIER, data: "0x249379ad" }, "latest"]));
    if (difficulty !== EXPECTED_DIFFICULTY) throw new Error("Pinned verifier configuration mismatch.");
    $("reviewed").disabled = false;
    note(`Signing-time preflight passed for ${providerLabel}: nonce ${pendingNonce}, ${ethBalance} wei, ${usdcBalance} USDC base units. Public activation remains off.`);
  }

  async function waitForReceipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const receipt = await readRpc("eth_getTransactionReceipt", [hash]);
      if (receipt) return receipt;
      await sleep(2000);
    }
    throw new Error(`Timed out waiting for receipt ${hash}.`);
  }
  function makeResult(hash, receipt) {
    const action = bundle.actions[0];
    return {
      schema_version: "agent-bounties/open-competition-v1-mainnet-deployment-receipt-v1",
      protocol_version: EXPECTED_PROTOCOL,
      network: "base-mainnet",
      chain_id: 8453,
      source_commit: bundle.source_commit,
      deployer: account,
      factory: action.expected_contract,
      implementation: action.expected_implementation,
      transaction_hash: hash,
      block_number: receipt ? Number.parseInt(receipt.blockNumber, 16) : null,
      block_hash: receipt ? receipt.blockHash : null,
      gas_used: receipt ? Number.parseInt(receipt.gasUsed, 16) : null,
      factory_runtime_code_hash: action.runtime_code_hash,
      implementation_runtime_code_hash: action.implementation_runtime_code_hash,
      runtime_matches: true,
      deployment_state: "mainnet_canary_not_ready_to_earn",
      public_creation_enabled: false,
      public_commitments_enabled: false,
      public_inventory_eligible: false,
      evidence_boundary: "Canonical deployment receipt and exact runtime matches are deployment evidence only, not bounty creation, settlement, payment, or activation evidence.",
    };
  }
  async function execute() {
    $("execute").disabled = true;
    $("reviewed").disabled = true;
    const action = bundle.actions[0];
    note("Requesting the exact frozen Base mainnet factory deployment signature...");
    const hash = await wallet("eth_sendTransaction", [{ from: account, data: action.data, value: "0x0", nonce: `0x${action.from_nonce.toString(16)}` }]);
    const receipt = await waitForReceipt(hash);
    if (Number.parseInt(receipt.status, 16) !== 1 || String(receipt.contractAddress).toLowerCase() !== action.expected_contract) throw new Error("Factory deployment receipt mismatch or revert.");
    const factoryRuntime = String(await readRpc("eth_getCode", [action.expected_contract, receipt.blockNumber])).toLowerCase();
    const implementationRuntime = String(await readRpc("eth_getCode", [action.expected_implementation, receipt.blockNumber])).toLowerCase();
    if (factoryRuntime !== action.expected_runtime_code.toLowerCase() || implementationRuntime !== action.expected_implementation_runtime_code.toLowerCase()) throw new Error("Deployed runtime bytecode mismatch.");
    result = makeResult(hash, receipt);
    note("Frozen factory deployed with exact runtimes. Public creation and inventory remain off; the hidden canary is next.");
    $("download").disabled = false;
  }
  function download() {
    const blob = new Blob([`${JSON.stringify(result, null, 2)}\n`], { type: "application/json" });
    const anchor = document.createElement("a");
    anchor.href = URL.createObjectURL(blob);
    anchor.download = `open-competition-v1-base-mainnet-deployment-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(anchor.href);
  }

  $("bundle").addEventListener("change", async (event) => {
    try { loadBundle(JSON.parse(await event.target.files[0].text()), "Audited mainnet bundle loaded. Connect the known admin wallet."); }
    catch (error) { fail(error.message); }
  });
  $("connect").addEventListener("click", () => connect().catch((error) => fail(error.message)));
  $("wallet-provider").addEventListener("change", () => { provider = null; providerLabel = null; account = null; fail("Wallet provider changed. Reconnect to rerun all checks."); });
  $("reviewed").addEventListener("change", () => { $("execute").disabled = !$("reviewed").checked || account !== EXPECTED_ADMIN || !bundle; });
  $("execute").addEventListener("click", () => execute().catch((error) => fail(error.message)));
  $("download").addEventListener("click", download);
  discoverProviders().catch((error) => fail(error.message));
  loadLocalBundle().catch((error) => fail(error.message));
})();
