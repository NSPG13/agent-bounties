(() => {
  "use strict";

  const EXPECTED_SCHEMA = "agent-bounties/open-competition-entrant-wallet-deployment-v1";
  const EXPECTED_FUNDING_SCHEMA = "agent-bounties/open-competition-entrant-wallet-rehearsal-funding-v1";
  const EXPECTED_CHAIN = "0x14a34";
  const EXPECTED_ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc";
  const EXPECTED_DEPLOYER = "0x4e59b44847b379578588920ca78fbf26c0b4956c";
  const EXPECTED_USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e";
  const MAX_FUNDING_ETH_WEI = 500000000000000n;
  const MAX_FUNDING_USDC = 400000n;
  const READ_RPC_URLS = ["https://sepolia.base.org", "https://base-sepolia-rpc.publicnode.com"];
  const $ = (id) => document.getElementById(id);
  const { keccak256Hex } = window.AgentBountiesEvm;
  const announcedProviders = [];
  const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  let bundle;
  let funding;
  let provider;
  let providerLabel;
  let account;
  let result;
  let fundingResult;

  function isProvider(candidate) {
    return Boolean(candidate && typeof candidate.request === "function");
  }

  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    if (item.provider.isMetaMask && !item.provider.isBraveWallet) return "MetaMask";
    if (item.provider.isBraveWallet) return "Brave Wallet";
    return "Injected wallet";
  }

  window.addEventListener("eip6963:announceProvider", (event) => {
    const detail = event && event.detail;
    if (detail && isProvider(detail.provider) && !announcedProviders.some((item) => item.provider === detail.provider)) {
      announcedProviders.push(detail);
    }
  });

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
    if (!providers.length) throw new Error("No injected browser wallet detected. Unlock MetaMask and reload.");
    providers.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item);
      selector.append(option);
    });
    const preferred = providers.findIndex((item) => (
      String(item.info && item.info.rdns).toLowerCase() === "io.metamask"
      || (item.provider.isMetaMask && !item.provider.isBraveWallet)
    ));
    selector.value = String(preferred >= 0 ? preferred : 0);
    selector.disabled = false;
    selector._providers = providers;
  }

  function selectProvider() {
    const selector = $("wallet-provider");
    const selected = selector._providers && selector._providers[Number.parseInt(selector.value, 10)];
    if (!selected) throw new Error("Select an available MetaMask provider.");
    provider = selected.provider;
    providerLabel = providerName(selected);
  }

  async function wallet(method, params = []) {
    if (!provider) selectProvider();
    try {
      return await provider.request({ method, params });
    } catch (error) {
      const code = error && error.code !== undefined ? ` (${error.code})` : "";
      throw new Error(`${method} failed${code}: ${error && error.message ? error.message : String(error)}`);
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

  function exactHex(value, bytes, label) {
    if (typeof value !== "string" || !new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value)) {
      throw new Error(`${label} is not ${bytes}-byte hex.`);
    }
    return value.toLowerCase();
  }

  function validateBundle(value) {
    if (!value || value.schema_version !== EXPECTED_SCHEMA || value.network !== "base-sepolia" || Number(value.chain_id) !== 84532) {
      throw new Error("Unsupported bundle schema, network, or chain.");
    }
    if (value.contract_source_dirty !== false || value.deployment_state !== "source_only_not_ready_to_earn") {
      throw new Error("Bundle is dirty or has an unsafe deployment state.");
    }
    if (exactHex(value.deterministic_deployer.address, 20, "deterministic deployer") !== EXPECTED_DEPLOYER) {
      throw new Error("Unexpected deterministic deployer.");
    }
    exactHex(value.deterministic_deployer.runtime_code_hash, 32, "deployer runtime hash");
    exactHex(value.entrant_wallet_factory.address, 20, "factory address");
    exactHex(value.entrant_wallet_factory.implementation, 20, "implementation address");
    exactHex(value.entrant_wallet_factory.runtime_code_hash, 32, "factory runtime hash");
    exactHex(value.entrant_wallet_factory.implementation_runtime_code_hash, 32, "implementation runtime hash");
    if (!/^0x[0-9a-fA-F]+$/.test(value.entrant_wallet_factory.deployment_transaction)) {
      throw new Error("Deployment transaction calldata is invalid.");
    }
    if (Object.values(value.activation_gates || {}).some(Boolean)) {
      throw new Error("A source-only deployment bundle cannot contain enabled activation gates.");
    }
    return value;
  }

  function validateFunding(value) {
    if (!value || value.schema_version !== EXPECTED_FUNDING_SCHEMA || value.network !== "base-sepolia"
        || Number(value.chain_id) !== 84532 || value.from.toLowerCase() !== EXPECTED_ADMIN) {
      throw new Error("Unsupported funding schema, network, chain, or sender.");
    }
    exactHex(value.recipient, 20, "ephemeral keeper");
    if (exactHex(value.usdc_token, 20, "test USDC") !== EXPECTED_USDC) throw new Error("Unexpected test USDC token.");
    if (BigInt(value.eth_wei) !== MAX_FUNDING_ETH_WEI || BigInt(value.usdc_base_units) !== MAX_FUNDING_USDC) {
      throw new Error("Funding values differ from the bounded rehearsal request.");
    }
    if (Number(value.maximum_transactions) !== 2) throw new Error("Funding request exceeds the two-call limit.");
    return value;
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

  function fundFail(message) {
    $("fund-status").className = "status bad";
    $("fund-status").textContent = message;
    $("fund-reviewed").disabled = true;
    $("fund-reviewed").checked = false;
    $("fund-execute").disabled = true;
  }

  function fundNote(message) {
    $("fund-status").className = "status";
    $("fund-status").textContent = message;
  }

  function renderBundle() {
    $("facts").hidden = false;
    $("facts").innerHTML = [
      ["Source contracts tree", bundle.contract_source_revision],
      ["Chain", "Base Sepolia · 84532"],
      ["Required signer", EXPECTED_ADMIN],
      ["Value", "0 ETH"],
      ["Deployment state", bundle.deployment_state],
      ["All activation gates", "false"],
    ].map(([label, value]) => `<div class="fact"><span>${label}</span><strong>${value}</strong></div>`).join("");
    const action = bundle.entrant_wallet_factory;
    $("action").innerHTML = `<article class="action"><dl>
      <dt>Recipient</dt><dd><code>${bundle.deterministic_deployer.address}</code></dd>
      <dt>Predicted factory</dt><dd><code>${action.address}</code></dd>
      <dt>Factory runtime hash</dt><dd><code>${action.runtime_code_hash}</code></dd>
      <dt>Predicted implementation</dt><dd><code>${action.implementation}</code></dd>
      <dt>Implementation runtime hash</dt><dd><code>${action.implementation_runtime_code_hash}</code></dd>
      <dt>Existing competition factory</dt><dd><code>${bundle.canonical.competition_factory}</code></dd>
      <dt>Existing test USDC</dt><dd><code>${bundle.canonical.settlement_token}</code></dd>
      </dl><details><summary>Exact CREATE2 calldata (${(action.deployment_transaction.length - 2) / 2} bytes)</summary><code>${action.deployment_transaction}</code></details></article>`;
  }

  async function loadBundle() {
    const response = await fetch("/target/open-competition-entrant-wallet/base-sepolia-deployment-regenerated.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`Local frozen bundle returned HTTP ${response.status}.`);
    bundle = validateBundle(await response.json());
    renderBundle();
    note("Frozen source-only bundle loaded. Connect the Brave MetaMask admin account.");
  }

  function renderFunding() {
    $("fund-facts").hidden = false;
    $("fund-facts").innerHTML = [
      ["Required signer", funding.from],
      ["Chain", "Base Sepolia · 84532"],
      ["Ephemeral keeper", funding.recipient],
      ["Native funding", `${funding.eth_wei} wei · 0.0005 test ETH`],
      ["Token", funding.usdc_token],
      ["Token funding", `${funding.usdc_base_units} base units · 0.4 test USDC`],
      ["Atomic calls", "2"],
      ["Public activation", "disabled"],
    ].map(([label, value]) => `<div class="fact"><span>${label}</span><strong>${value}</strong></div>`).join("");
  }

  async function loadFunding() {
    const response = await fetch("/target/open-competition-entrant-wallet/base-sepolia-rehearsal-funding.json", { cache: "no-store" });
    if (!response.ok) throw new Error(`Local funding request returned HTTP ${response.status}.`);
    funding = validateFunding(await response.json());
    renderFunding();
    fundNote("Bounded request loaded. Connect the Brave MetaMask admin account to run balance and atomicity checks.");
    if (account === EXPECTED_ADMIN) await preflightFunding();
  }

  function uint256Hex(value) {
    return BigInt(value).toString(16).padStart(64, "0");
  }

  function addressWord(value) {
    return exactHex(value, 20, "address").slice(2).padStart(64, "0");
  }

  function transferCalldata(recipient, amount) {
    return `0xa9059cbb${addressWord(recipient)}${uint256Hex(amount)}`;
  }

  function balanceOfCalldata(address) {
    return `0x70a08231${addressWord(address)}`;
  }

  async function preflightFunding() {
    if (!funding || account !== EXPECTED_ADMIN) return;
    const [adminEthHex, adminUsdcHex, keeperEthHex, keeperUsdcHex] = await Promise.all([
      readRpc("eth_getBalance", [EXPECTED_ADMIN, "latest"]),
      readRpc("eth_call", [{ to: EXPECTED_USDC, data: balanceOfCalldata(EXPECTED_ADMIN) }, "latest"]),
      readRpc("eth_getBalance", [funding.recipient, "latest"]),
      readRpc("eth_call", [{ to: EXPECTED_USDC, data: balanceOfCalldata(funding.recipient) }, "latest"]),
    ]);
    const adminEth = BigInt(adminEthHex);
    const adminUsdc = BigInt(adminUsdcHex);
    const keeperEth = BigInt(keeperEthHex);
    const keeperUsdc = BigInt(keeperUsdcHex);
    if (keeperEth >= BigInt(funding.eth_wei) && keeperUsdc >= BigInt(funding.usdc_base_units)) {
      fundNote("The ephemeral keeper is already funded to both exact minimums. No transaction is required.");
      return;
    }
    if (keeperEth !== 0n || keeperUsdc !== 0n) throw new Error("Ephemeral keeper is only partially funded; stop for manual reconciliation.");
    if (adminEth <= BigInt(funding.eth_wei) || adminUsdc < BigInt(funding.usdc_base_units)) {
      throw new Error("Admin balance is below the bounded testnet funding requirement plus gas.");
    }
    const capabilities = await wallet("wallet_getCapabilities", [account, [EXPECTED_CHAIN]]);
    const atomic = capabilities && Object.entries(capabilities).find(
      ([chain]) => chain.toLowerCase() === EXPECTED_CHAIN,
    )?.[1];
    const status = atomic && atomic.atomic && atomic.atomic.status;
    if (status !== "supported" && status !== "ready") throw new Error("MetaMask does not report atomic call support on Base Sepolia.");
    $("fund-reviewed").disabled = false;
    fundNote(`Balances and atomic batching passed. Admin has ${adminEth} wei and ${adminUsdc} test-USDC base units; keeper is empty.`);
  }

  async function preflight() {
    const deployerCode = String(await readRpc("eth_getCode", [bundle.deterministic_deployer.address, "latest"])).toLowerCase();
    if (keccak256Hex(deployerCode) !== bundle.deterministic_deployer.runtime_code_hash.toLowerCase()) {
      throw new Error("Canonical deterministic deployer runtime hash mismatch.");
    }
    const action = bundle.entrant_wallet_factory;
    const factoryCode = String(await readRpc("eth_getCode", [action.address, "latest"])).toLowerCase();
    const implementationCode = String(await readRpc("eth_getCode", [action.implementation, "latest"])).toLowerCase();
    if (factoryCode !== "0x" || implementationCode !== "0x") {
      if (keccak256Hex(factoryCode) !== action.runtime_code_hash.toLowerCase()
          || keccak256Hex(implementationCode) !== action.implementation_runtime_code_hash.toLowerCase()) {
        throw new Error("A predicted address is occupied by non-frozen runtime bytecode.");
      }
      result = makeResult(null, null, null, true);
      note("The factory and implementation already exist with exact frozen runtime hashes. No transaction is required.");
      $("download").disabled = false;
      return;
    }
    $("reviewed").disabled = false;
    note("Account, chain, deployer code, empty predicted addresses, and frozen bytecode all match. Review before opening MetaMask.");
  }

  async function connect() {
    if (!bundle) throw new Error("Wait for the frozen bundle to load.");
    selectProvider();
    let chain = await wallet("eth_chainId");
    if (chain.toLowerCase() !== EXPECTED_CHAIN) {
      await wallet("wallet_switchEthereumChain", [{ chainId: EXPECTED_CHAIN }]);
      chain = await wallet("eth_chainId");
    }
    if (chain.toLowerCase() !== EXPECTED_CHAIN) throw new Error("Wallet did not switch to Base Sepolia.");
    const accounts = await wallet("eth_requestAccounts");
    account = String(accounts[0] || "").toLowerCase();
    if (account !== EXPECTED_ADMIN) throw new Error(`Connected account ${account || "(none)"} is not the required admin.`);
    note(`Connected ${providerLabel} to ${account} on Base Sepolia. Running read-only bytecode checks.`);
    await preflight();
    await preflightFunding();
  }

  async function waitForReceipt(hash) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const receipt = await readRpc("eth_getTransactionReceipt", [hash]);
      if (receipt) return receipt;
      await sleep(2000);
    }
    throw new Error(`Timed out waiting for receipt ${hash}.`);
  }

  function makeResult(hash, receipt, blockHash, preexisting = false) {
    return {
      schema_version: "agent-bounties/open-competition-entrant-wallet-deployment-receipt-v1",
      network: "base-sepolia",
      chain_id: 84532,
      contract_source_revision: bundle.contract_source_revision,
      signer: account,
      transaction_hash: hash,
      block_number: receipt ? Number.parseInt(receipt.blockNumber, 16) : null,
      block_hash: blockHash,
      gas_used: receipt ? Number.parseInt(receipt.gasUsed, 16) : null,
      deterministic_deployer: bundle.deterministic_deployer.address,
      entrant_wallet_factory: bundle.entrant_wallet_factory.address,
      entrant_wallet_factory_runtime_hash: bundle.entrant_wallet_factory.runtime_code_hash,
      entrant_wallet_implementation: bundle.entrant_wallet_factory.implementation,
      entrant_wallet_implementation_runtime_hash: bundle.entrant_wallet_factory.implementation_runtime_code_hash,
      runtime_matches: true,
      preexisting,
      activation_gates: bundle.activation_gates,
      evidence_boundary: "Confirmed deployment and exact runtime matches are deployment evidence only, not rehearsal, activation, settlement, relay availability, gas sponsorship, or payment evidence.",
    };
  }

  async function execute() {
    $("execute").disabled = true;
    $("reviewed").disabled = true;
    note(`Requesting ${providerLabel} confirmation for the exact zero-value CREATE2 deployment transaction…`);
    const hash = await wallet("eth_sendTransaction", [{
      from: account,
      to: bundle.deterministic_deployer.address,
      data: bundle.entrant_wallet_factory.deployment_transaction,
      value: "0x0",
    }]);
    const receipt = await waitForReceipt(hash);
    if (Number.parseInt(receipt.status, 16) !== 1) throw new Error("Entrant wallet factory deployment reverted.");
    const action = bundle.entrant_wallet_factory;
    const factoryCode = String(await readRpc("eth_getCode", [action.address, receipt.blockNumber])).toLowerCase();
    const implementationCode = String(await readRpc("eth_getCode", [action.implementation, receipt.blockNumber])).toLowerCase();
    if (keccak256Hex(factoryCode) !== action.runtime_code_hash.toLowerCase()
        || keccak256Hex(implementationCode) !== action.implementation_runtime_code_hash.toLowerCase()) {
      throw new Error("Post-deployment runtime hash mismatch.");
    }
    result = makeResult(hash, receipt, receipt.blockHash);
    note("Deployment confirmed with exact factory and implementation runtime hashes. Download the receipt; rehearsal remains disabled until the scenarios pass.");
    $("download").disabled = false;
  }

  async function waitForCallsStatus(id) {
    for (let attempt = 0; attempt < 180; attempt += 1) {
      const status = await wallet("wallet_getCallsStatus", [id]);
      if (Number(status.status) === 200) return status;
      if (Number(status.status) >= 400) throw new Error(`Atomic funding batch failed with status ${status.status}.`);
      await sleep(2000);
    }
    throw new Error(`Timed out waiting for atomic funding batch ${id}.`);
  }

  async function executeFunding() {
    $("fund-execute").disabled = true;
    $("fund-reviewed").disabled = true;
    fundNote(`Requesting ${providerLabel} confirmation for one atomic two-call testnet funding batch…`);
    const response = await wallet("wallet_sendCalls", [{
      version: "2.0.0",
      from: account,
      chainId: EXPECTED_CHAIN,
      atomicRequired: true,
      calls: [
        { to: funding.recipient, value: `0x${BigInt(funding.eth_wei).toString(16)}` },
        { to: funding.usdc_token, value: "0x0", data: transferCalldata(funding.recipient, funding.usdc_base_units) },
      ],
    }]);
    const id = typeof response === "string" ? response : response && response.id;
    if (!id) throw new Error("MetaMask returned no atomic call-batch identifier.");
    const status = await waitForCallsStatus(id);
    const receipts = Array.isArray(status.receipts) ? status.receipts : [];
    if (!receipts.length || receipts.some((receipt) => Number.parseInt(receipt.status, 16) !== 1)) {
      throw new Error("Atomic funding status did not contain successful receipts.");
    }
    const [keeperEthHex, keeperUsdcHex] = await Promise.all([
      readRpc("eth_getBalance", [funding.recipient, "latest"]),
      readRpc("eth_call", [{ to: EXPECTED_USDC, data: balanceOfCalldata(funding.recipient) }, "latest"]),
    ]);
    if (BigInt(keeperEthHex) < BigInt(funding.eth_wei) || BigInt(keeperUsdcHex) < BigInt(funding.usdc_base_units)) {
      throw new Error("Confirmed funding receipts did not produce the requested keeper balances.");
    }
    fundingResult = {
      schema_version: "agent-bounties/open-competition-entrant-wallet-rehearsal-funding-receipt-v1",
      network: "base-sepolia",
      chain_id: 84532,
      admin: account,
      keeper: funding.recipient,
      eth_wei: funding.eth_wei,
      usdc_token: funding.usdc_token,
      usdc_base_units: funding.usdc_base_units,
      call_batch_id: id,
      atomic: status.atomic === true,
      transactions: receipts.map((receipt) => ({
        transaction_hash: receipt.transactionHash,
        block_number: Number.parseInt(receipt.blockNumber, 16),
        block_hash: receipt.blockHash,
        gas_used: Number.parseInt(receipt.gasUsed, 16),
      })),
      public_activation_enabled: false,
      evidence_boundary: "Testnet actor funding only; not bounty funding, settlement, payment, relay availability, gas sponsorship, or activation evidence.",
    };
    fundNote("Atomic testnet funding confirmed and balances reconciled. Download the receipt, then run the scenarios.");
    $("fund-download").disabled = false;
  }

  function download() {
    const blob = new Blob([`${JSON.stringify(result, null, 2)}\n`], { type: "application/json" });
    const anchor = document.createElement("a");
    anchor.href = URL.createObjectURL(blob);
    anchor.download = `open-competition-entrant-wallet-base-sepolia-deployment-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(anchor.href);
  }

  function downloadFunding() {
    const blob = new Blob([`${JSON.stringify(fundingResult, null, 2)}\n`], { type: "application/json" });
    const anchor = document.createElement("a");
    anchor.href = URL.createObjectURL(blob);
    anchor.download = `open-competition-entrant-wallet-base-sepolia-funding-${Date.now()}.json`;
    anchor.click();
    URL.revokeObjectURL(anchor.href);
  }

  $("connect").addEventListener("click", () => connect().catch((error) => fail(error.message)));
  $("wallet-provider").addEventListener("change", () => {
    provider = null;
    account = null;
    $("reviewed").disabled = true;
    $("reviewed").checked = false;
    $("execute").disabled = true;
    note("Wallet provider changed. Connect it to re-run every check.");
  });
  $("reviewed").addEventListener("change", () => {
    $("execute").disabled = !$("reviewed").checked || account !== EXPECTED_ADMIN;
  });
  $("fund-reviewed").addEventListener("change", () => {
    $("fund-execute").disabled = !$("fund-reviewed").checked || account !== EXPECTED_ADMIN;
  });
  $("execute").addEventListener("click", () => execute().catch((error) => fail(error.message)));
  $("download").addEventListener("click", download);
  $("fund-execute").addEventListener("click", () => executeFunding().catch((error) => fundFail(error.message)));
  $("fund-download").addEventListener("click", downloadFunding);
  discoverProviders().catch((error) => fail(error.message));
  loadBundle().catch((error) => fail(error.message));
  loadFunding().catch((error) => fundFail(error.message));
})();
