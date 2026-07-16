(() => {
  "use strict";

  const CHAIN_ID = "0x14a34";
  const USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e";
  const BUNDLE_URL = "/deployments/base-sepolia-sponsor-activation.json";
  const EXPLORER = "https://sepolia.basescan.org/tx/";
  const ACTIVATION = Object.freeze({
    sourceCommit: "1c23462386782ed9f6ac5a5bd4695bfc336bdce7",
    deployer: "0x884834e884d6e93462655a2820140ad03e6747bc",
    grantSigner: "0x52bbc33facb5bd3d31125c168047543f423ee034",
    factory: "0x9601a40b35ad6843846732c6cb73c4c82f9ba850",
    implementation: "0xe70b9d541a176307e50f308aa370a1661eabfd99",
    verifier: "0x7231f1312448fa60078fb56cdb6e2c392bd1269b",
    sponsor: "0xa1e2e93530114f7fe64c251556b8de13dad7d157",
    factoryCreationHash: "0x2c911d92c9580a1c7a86e1d48173f54e11224ab84435ca6c6213c4a66b35d1e5",
    factoryRuntimeHash: "0x7e07f933a77423a9183f6bbf3eb897c4e7b73399c95056b6142bdeb6be95d171",
    implementationRuntimeHash: "0xc36fcba5176b2cd8b57a9fd0cbf931177dc8b36cf8367c1568ccebe5f03be3f6",
    verifierCreationHash: "0x3ea68aa44bae30c12db1ff78b1df7941201a169add9bd8fc01e588bbc72beb3b",
    verifierRuntimeHash: "0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3",
    sponsorCreationHash: "0xee2c4631dffdfa40566b2e98cd4111f405c72ba461cb4aeca76f67cbbaa72efe",
    sponsorRuntimeHash: "0x09c5ecb7be48d2235ead4d4c4a9d11a83722f5b52dbdd58096ba09e185259a1b",
  });
  const state = { bundle: null, provider: null, providers: [], account: null, status: {} };
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

  function isAddress(value) {
    return /^0x[0-9a-f]{40}$/.test(value);
  }

  function validDeployment(value) {
    return value && value.to === null && value.value_wei === 0
      && Number.isInteger(value.from_nonce) && isAddress(value.expected_contract)
      && /^0x[0-9a-f]+$/.test(value.data)
      && /^0x[0-9a-f]+$/.test(value.expected_runtime_code);
  }

  function exactFundingData(recipient, amount) {
    return `0xa9059cbb${recipient.replace(/^0x/, "").padStart(64, "0")}${amount.toString(16).padStart(64, "0")}`;
  }

  async function loadBundle() {
    requireLocalOrigin();
    const response = await fetch(BUNDLE_URL, { cache: "no-store" });
    if (!response.ok) throw new Error("The checked-in Base Sepolia activation bundle is unavailable.");
    const bundle = await response.json();
    if (
      bundle.schema_version !== "agent-bounties/base-sepolia-sponsor-activation-v1"
      || bundle.protocol_version !== "agent-bounties/autonomous-v1"
      || bundle.network !== "base-sepolia"
      || bundle.chain_id !== 84532
      || bundle.settlement_token !== USDC
      || bundle.source_commit !== ACTIVATION.sourceCommit
      || bundle.deployer !== ACTIVATION.deployer
      || bundle.grant_signer !== ACTIVATION.grantSigner
      || !validDeployment(bundle.factory)
      || !validDeployment(bundle.verifier)
      || !validDeployment(bundle.sponsor)
      || bundle.factory.from_nonce !== 1
      || bundle.factory.expected_contract !== ACTIVATION.factory
      || bundle.factory.expected_implementation !== ACTIVATION.implementation
      || bundle.factory.creation_code_hash !== ACTIVATION.factoryCreationHash
      || bundle.factory.runtime_code_hash !== ACTIVATION.factoryRuntimeHash
      || bundle.factory.implementation_runtime_code_hash !== ACTIVATION.implementationRuntimeHash
      || bundle.factory.from_nonce + 1 !== bundle.verifier.from_nonce
      || bundle.verifier.expected_contract !== ACTIVATION.verifier
      || bundle.verifier.creation_code_hash !== ACTIVATION.verifierCreationHash
      || bundle.verifier.runtime_code_hash !== ACTIVATION.verifierRuntimeHash
      || bundle.verifier.from_nonce + 1 !== bundle.sponsor.from_nonce
      || bundle.sponsor.expected_contract !== ACTIVATION.sponsor
      || bundle.sponsor.creation_code_hash !== ACTIVATION.sponsorCreationHash
      || bundle.sponsor.runtime_code_hash !== ACTIVATION.sponsorRuntimeHash
      || bundle.verifier.difficulty_bits !== 16
      || bundle.sponsor.max_bond_base_units !== 100000
      || bundle.sponsor.max_network_per_day_base_units !== 1000000
      || bundle.sponsor.max_lifetime_per_solver_base_units !== 100000
      || bundle.sponsor_funding.to !== USDC
      || bundle.sponsor_funding.amount_base_units !== 100000
      || bundle.sponsor_funding.recipient !== bundle.sponsor.expected_contract
      || bundle.sponsor_funding.data !== exactFundingData(ACTIVATION.sponsor, 100000)
    ) throw new Error("Activation bundle violates the locked Base Sepolia contract.");
    state.bundle = bundle;
    for (const [attribute, value] of Object.entries({
      "source-commit": bundle.source_commit,
      deployer: bundle.deployer,
      factory: bundle.factory.expected_contract,
      implementation: bundle.factory.expected_implementation,
      verifier: bundle.verifier.expected_contract,
      sponsor: bundle.sponsor.expected_contract,
      "grant-signer": bundle.grant_signer,
    })) document.querySelector(`[data-${attribute}]`).textContent = value;
  }

  function providerName(item) {
    if (item.info && item.info.name) return item.info.name;
    if (item.provider.isMetaMask) return "MetaMask";
    if (item.provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (item.provider.isBraveWallet) return "Brave Wallet";
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
      ? window.ethereum.providers : (window.ethereum ? [window.ethereum] : []);
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
      option.textContent = providerName(item);
      return option;
    }));
    selector.disabled = state.providers.length === 0;
    if (state.providers.length === 0) throw new Error("No injected wallet is exposed. Unlock a browser wallet and reload.");
  }

  function selectedProvider() {
    const item = state.providers[Number.parseInt(byId("wallet-provider").value, 10)];
    if (!item) throw new Error("Select an available wallet provider.");
    state.provider = item.provider;
    return item;
  }

  function wallet(method, params = []) {
    const provider = state.provider || selectedProvider().provider;
    return provider.request({ method, params });
  }

  async function connect() {
    const accounts = await wallet("eth_requestAccounts");
    if (!accounts || !accounts[0]) throw new Error("The wallet returned no account.");
    const account = accounts[0].toLowerCase();
    if (account !== state.bundle.deployer) throw new Error(`Select the committed deployer ${state.bundle.deployer}.`);
    if ((await wallet("eth_chainId")).toLowerCase() !== CHAIN_ID) {
      await wallet("wallet_switchEthereumChain", [{ chainId: CHAIN_ID }]);
    }
    if ((await wallet("eth_chainId")).toLowerCase() !== CHAIN_ID) {
      throw new Error("Wallet did not switch to Base Sepolia.");
    }
    state.account = account;
    return account;
  }

  async function verifyBundleHashes() {
    const commitments = [
      [state.bundle.factory.data, ACTIVATION.factoryCreationHash, "factory creation code"],
      [state.bundle.factory.expected_runtime_code, ACTIVATION.factoryRuntimeHash, "factory runtime"],
      [state.bundle.factory.expected_implementation_runtime_code, ACTIVATION.implementationRuntimeHash, "implementation runtime"],
      [state.bundle.verifier.data, ACTIVATION.verifierCreationHash, "verifier creation code"],
      [state.bundle.verifier.expected_runtime_code, ACTIVATION.verifierRuntimeHash, "verifier runtime"],
      [state.bundle.sponsor.data, ACTIVATION.sponsorCreationHash, "sponsor creation code"],
      [state.bundle.sponsor.expected_runtime_code, ACTIVATION.sponsorRuntimeHash, "sponsor runtime"],
    ];
    for (const [data, expected, label] of commitments) {
      const observed = String(await wallet("web3_sha3", [data])).toLowerCase();
      if (observed !== expected) throw new Error(`Committed ${label} hash mismatch.`);
    }
  }

  function addressResult(value) {
    return `0x${String(value).replace(/^0x/, "").slice(-40)}`.toLowerCase();
  }

  function uintResult(value) {
    return BigInt(value);
  }

  function call(to, data) {
    return wallet("eth_call", [{ to, data }, "latest"]);
  }

  async function exactCode(deployment) {
    const code = (await wallet("eth_getCode", [deployment.expected_contract, "latest"])).toLowerCase();
    if (code === "0x") return false;
    if (code !== deployment.expected_runtime_code) throw new Error(`${deployment.name} runtime bytecode mismatch.`);
    return true;
  }

  async function verifyFactory() {
    if (!(await exactCode(state.bundle.factory))) return false;
    const factory = state.bundle.factory;
    const token = addressResult(await call(factory.expected_contract, "0x7b9e618d"));
    const implementation = addressResult(await call(factory.expected_contract, "0x5c60da1b"));
    const implementationCode = (await wallet("eth_getCode", [implementation, "latest"])).toLowerCase();
    if (token !== USDC || implementation !== factory.expected_implementation
      || implementationCode !== factory.expected_implementation_runtime_code) {
      throw new Error("Factory token or implementation mismatch.");
    }
    return true;
  }

  async function verifyVerifier() {
    if (!(await exactCode(state.bundle.verifier))) return false;
    if (uintResult(await call(state.bundle.verifier.expected_contract, "0x249379ad")) !== 16n) {
      throw new Error("Verifier difficulty mismatch.");
    }
    return true;
  }

  async function verifySponsor() {
    if (!(await exactCode(state.bundle.sponsor))) return false;
    const sponsor = state.bundle.sponsor.expected_contract;
    const checks = [
      ["0x7b9e618d", USDC],
      ["0x044f3e72", state.bundle.factory.expected_contract],
      ["0xf7c37ccd", state.bundle.grant_signer],
      ["0x8da5cb5b", state.bundle.deployer],
    ];
    for (const [selector, expected] of checks) {
      if (addressResult(await call(sponsor, selector)) !== expected) throw new Error("Sponsor address configuration mismatch.");
    }
    const caps = [
      ["0x890371b2", 100000n],
      ["0x26735b26", 1000000n],
      ["0x4ee9423d", 100000n],
    ];
    for (const [selector, expected] of caps) {
      if (uintResult(await call(sponsor, selector)) !== expected) throw new Error("Sponsor cap mismatch.");
    }
    return true;
  }

  async function tokenBalance(address) {
    return uintResult(await call(USDC, `0x70a08231${address.replace(/^0x/, "").padStart(64, "0")}`));
  }

  function setButtons() {
    const status = state.status;
    byId("deploy-factory").disabled = !status.inspected || status.factory;
    byId("deploy-verifier").disabled = !status.inspected || !status.factory || status.verifier;
    byId("deploy-sponsor").disabled = !status.inspected || !status.factory || !status.verifier || status.sponsor;
    byId("fund-sponsor").disabled = !status.inspected || !status.sponsor || status.funded;
  }

  async function inspect() {
    const target = byId("inspect-output");
    try {
      const account = await connect();
      await verifyBundleHashes();
      const status = {
        factory: await verifyFactory(),
        verifier: await verifyVerifier(),
        sponsor: await verifySponsor(),
      };
      if ((!status.factory && (status.verifier || status.sponsor)) || (!status.verifier && status.sponsor)) {
        throw new Error("Component deployment order is inconsistent; stop and investigate.");
      }
      const nonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "latest"]), 16);
      const pendingNonce = Number.parseInt(await wallet("eth_getTransactionCount", [account, "pending"]), 16);
      if (pendingNonce !== nonce) {
        throw new Error(`Wallet has pending transactions (latest ${nonce}, pending ${pendingNonce}); wait or clear them before activation.`);
      }
      const next = !status.factory ? state.bundle.factory
        : (!status.verifier ? state.bundle.verifier : (!status.sponsor ? state.bundle.sponsor : null));
      if (next && nonce !== next.from_nonce) {
        throw new Error(`Nonce drift: ${next.name} requires ${next.from_nonce}, wallet is ${nonce}. Regenerate before signing.`);
      }
      status.sponsorBalance = await tokenBalance(state.bundle.sponsor.expected_contract);
      const exactSeed = BigInt(state.bundle.sponsor_funding.amount_base_units);
      if (status.sponsorBalance !== 0n && status.sponsorBalance !== exactSeed) {
        throw new Error(`Sponsor balance drift: expected 0 or ${exactSeed}, got ${status.sponsorBalance}. Stop and investigate.`);
      }
      status.funded = status.sponsorBalance === exactSeed;
      status.inspected = true;
      const eth = BigInt(await wallet("eth_getBalance", [account, "latest"]));
      const usdc = await tokenBalance(account);
      let estimate = null;
      if (next) estimate = BigInt(await wallet("eth_estimateGas", [{ from: account, data: next.data, value: "0x0" }]));
      else if (!status.funded) estimate = BigInt(await wallet("eth_estimateGas", [{ from: account, to: USDC, data: state.bundle.sponsor_funding.data, value: "0x0" }]));
      state.status = status;
      setButtons();
      write(target, [
        `Wallet provider: ${providerName(state.providers[Number.parseInt(byId("wallet-provider").value, 10)])}`,
        `Account: ${account}`,
        `Chain: Base Sepolia (${CHAIN_ID})`,
        `Nonce: ${nonce}`,
        `ETH: ${(Number(eth) / 1e18).toFixed(8)}`,
        `Test USDC: ${(Number(usdc) / 1e6).toFixed(2)}`,
        `Factory: ${status.factory ? "verified" : "missing"}`,
        `Verifier: ${status.verifier ? "verified" : "missing"}`,
        `Sponsor: ${status.sponsor ? "verified" : "missing"}`,
        `Sponsor balance: ${(Number(status.sponsorBalance) / 1e6).toFixed(2)} test USDC`,
        estimate === null ? "All activation actions are confirmed." : `Next live gas estimate: ${estimate}`,
      ], "success");
      return status;
    } catch (error) {
      state.status = {};
      for (const id of ["deploy-factory", "deploy-verifier", "deploy-sponsor", "fund-sponsor"]) byId(id).disabled = true;
      write(target, error.message || String(error), "error");
      throw error;
    }
  }

  async function waitReceipt(transactionHash, timeoutMilliseconds = 180000) {
    const deadline = Date.now() + timeoutMilliseconds;
    while (Date.now() < deadline) {
      const receipt = await wallet("eth_getTransactionReceipt", [transactionHash]);
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Transaction reverted: ${transactionHash}`);
        return receipt;
      }
      await sleep(1500);
    }
    throw new Error(`Transaction confirmation timed out: ${transactionHash}`);
  }

  async function deployComponent(key, outputId) {
    const target = byId(outputId);
    try {
      await inspect();
      if (state.status[key]) return;
      const deployment = state.bundle[key];
      write(target, [`Wallet confirmation requested for ${deployment.name}.`, `Expected contract: ${deployment.expected_contract}`]);
      const hash = await wallet("eth_sendTransaction", [{
        from: state.account,
        data: deployment.data,
        value: "0x0",
        nonce: `0x${deployment.from_nonce.toString(16)}`,
      }]);
      const receipt = await waitReceipt(hash);
      if (String(receipt.contractAddress).toLowerCase() !== deployment.expected_contract) {
        throw new Error(`Receipt contract mismatch: ${receipt.contractAddress}`);
      }
      await inspect();
      if (!state.status[key]) throw new Error("Receipt succeeded but exact runtime verification failed.");
      write(target, [`${deployment.name} confirmed and verified.`, `${EXPLORER}${hash}`], "success");
    } catch (error) {
      write(target, error.message || String(error), "error");
    }
  }

  async function fundSponsor() {
    const target = byId("funding-output");
    try {
      await inspect();
      if (state.status.funded) return;
      write(target, ["Wallet confirmation requested for exactly 0.10 test USDC.", `Recipient: ${state.bundle.sponsor.expected_contract}`]);
      const hash = await wallet("eth_sendTransaction", [{
        from: state.account,
        to: USDC,
        data: state.bundle.sponsor_funding.data,
        value: "0x0",
      }]);
      await waitReceipt(hash);
      await inspect();
      if (!state.status.funded) throw new Error("Transfer confirmed but sponsor balance is not the exact committed seed.");
      write(target, ["Sponsor seed confirmed by canonical token balance.", `${EXPLORER}${hash}`, state.bundle.evidence_boundary], "success");
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
      state.status = {};
      setButtons();
    });
    byId("inspect").addEventListener("click", () => inspect().catch(() => {}));
    byId("deploy-factory").addEventListener("click", () => deployComponent("factory", "factory-output"));
    byId("deploy-verifier").addEventListener("click", () => deployComponent("verifier", "verifier-output"));
    byId("deploy-sponsor").addEventListener("click", () => deployComponent("sponsor", "sponsor-output"));
    byId("fund-sponsor").addEventListener("click", fundSponsor);
  }

  initialize();
})();
