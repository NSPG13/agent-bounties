(() => {
  "use strict";

  const BUNDLE_URL = "/deployments/bounded-wallet-base-activation.json";
  const state = { bundle: null, network: null, provider: null, providers: [], account: null, inspected: false };
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

  function addressWord(address) {
    return address.toLowerCase().replace(/^0x/, "").padStart(64, "0");
  }

  function addressResult(value) {
    return `0x${String(value).replace(/^0x/, "").slice(-40)}`.toLowerCase();
  }

  function uintResult(value) {
    return BigInt(value || "0x0");
  }

  function words(value, count) {
    const data = String(value).replace(/^0x/, "");
    if (data.length !== count * 64 || !/^[0-9a-f]+$/i.test(data)) {
      throw new Error(`Expected exactly ${count} ABI words.`);
    }
    return Array.from({ length: count }, (_, index) => `0x${data.slice(index * 64, (index + 1) * 64)}`);
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
    if (state.providers.length === 0) throw new Error("Unlock an injected wallet and reload this page.");
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

  async function call(to, data, block = "latest") {
    return wallet("eth_call", [{ to, data }, block]);
  }

  async function code(address) {
    return String(await wallet("eth_getCode", [address, "latest"])).toLowerCase();
  }

  async function tokenBalance(address) {
    return uintResult(await call(state.network.native_usdc, `0x70a08231${addressWord(address)}`));
  }

  async function tokenAllowance(owner, spender) {
    return uintResult(await call(
      state.network.native_usdc,
      `0xdd62ed3e${addressWord(owner)}${addressWord(spender)}`,
    ));
  }

  async function tokenNonce(owner) {
    return uintResult(await call(state.network.native_usdc, `0x7ecebe00${addressWord(owner)}`));
  }

  async function postActivation(path, body) {
    const response = await fetch(path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    });
    const value = await response.json();
    if (!response.ok) throw new Error(value.error || `Activation relay failed with HTTP ${response.status}.`);
    return value;
  }

  async function connect() {
    const accounts = await wallet("eth_requestAccounts");
    if (!accounts || !accounts[0]) throw new Error("The wallet returned no account.");
    const account = accounts[0].toLowerCase();
    if (account !== state.network.pilot.owner) {
      throw new Error(`Select the committed owner ${state.network.pilot.owner}.`);
    }
    const chainId = state.network.chain_id_hex.toLowerCase();
    if (String(await wallet("eth_chainId")).toLowerCase() !== chainId) {
      try {
        await wallet("wallet_switchEthereumChain", [{ chainId }]);
      } catch (error) {
        if (error && error.code === 4902 && state.network.network === "base-sepolia") {
          await wallet("wallet_addEthereumChain", [{
            chainId,
            chainName: "Base Sepolia",
            nativeCurrency: { name: "Sepolia Ether", symbol: "ETH", decimals: 18 },
            rpcUrls: [state.network.rpc_url],
            blockExplorerUrls: ["https://sepolia.basescan.org"],
          }]);
        } else {
          throw error;
        }
      }
    }
    state.account = account;
    return account;
  }

  async function verifyBountyFactory() {
    if (await code(state.network.bounty_factory) === "0x") return false;
    const token = addressResult(await call(state.network.bounty_factory, "0x7b9e618d"));
    const implementation = addressResult(await call(state.network.bounty_factory, "0x5c60da1b"));
    if (token !== state.network.native_usdc || await code(implementation) === "0x") {
      throw new Error("Bounty factory immutable configuration mismatch.");
    }
    return true;
  }

  async function verifyVerifier() {
    if (await code(state.network.verifier_module) === "0x") return false;
    const difficulty = uintResult(await call(state.network.verifier_module, "0x249379ad"));
    if (difficulty !== BigInt(state.network.verifier_difficulty_bits)) {
      throw new Error("Deterministic verifier difficulty mismatch.");
    }
    return true;
  }

  async function verifyWalletFactory() {
    if (await code(state.network.wallet_factory) === "0x") return false;
    const bountyFactory = addressResult(await call(state.network.wallet_factory, "0xb8f75c0b"));
    const token = addressResult(await call(state.network.wallet_factory, "0x7b9e618d"));
    if (bountyFactory !== state.network.bounty_factory || token !== state.network.native_usdc) {
      throw new Error("Bounded-wallet factory immutable configuration mismatch.");
    }
    return true;
  }

  async function pendingDeployments() {
    const deployer = state.bundle.deterministic_deployer;
    if (await code(deployer.contract) !== deployer.runtime_code) {
      throw new Error("Canonical deterministic deployer bytecode mismatch.");
    }
    const pending = [];
    for (const deployment of state.network.deployments) {
      const observed = await code(deployment.expected_contract);
      if (observed === "0x") {
        pending.push(deployment);
      } else if (observed !== deployment.runtime_code) {
        throw new Error(`${deployment.name} runtime bytecode mismatch.`);
      }
    }
    if (!pending.some((item) => item.name === "AgentBountyFactory")) await verifyBountyFactory();
    if (!pending.some((item) => item.name === "LeadingZeroWorkVerifier")) await verifyVerifier();
    if (!pending.some((item) => item.name === "BoundedAgentWalletFactory")) await verifyWalletFactory();
    return pending;
  }

  async function verifyPolicyWallet(item) {
    if (await code(item.expected_contract) === "0x") return false;
    const registered = uintResult(await call(
      state.network.wallet_factory,
      `0xf48f2346${addressWord(item.expected_contract)}`,
    ));
    const owner = addressResult(await call(item.expected_contract, "0x8da5cb5b"));
    const bountyFactory = addressResult(await call(item.expected_contract, "0xc45a0155"));
    const token = addressResult(await call(item.expected_contract, "0x7b9e618d"));
    const policy = words(await call(item.expected_contract, "0x0505c8c9"), 9);
    const expected = item.policy;
    const checks = [
      addressResult(policy[0]) === item.delegate,
      uintResult(policy[1]) === BigInt(expected.valid_after),
      uintResult(policy[2]) === BigInt(expected.valid_until),
      uintResult(policy[3]) === BigInt(expected.period_seconds),
      uintResult(policy[4]) === BigInt(expected.max_per_action),
      uintResult(policy[5]) === BigInt(expected.max_per_period),
      uintResult(policy[6]) === BigInt(expected.max_lifetime_spend),
      uintResult(policy[7]) === BigInt(expected.allowed_actions),
      uintResult(policy[8]) === BigInt(expected.allowed_verification_modes),
    ];
    if (
      registered !== 1n
      || owner !== item.owner
      || bountyFactory !== state.network.bounty_factory
      || token !== state.network.native_usdc
      || checks.some((check) => !check)
      || uintResult(await call(item.expected_contract, "0x58355ead")) < 1n
      || uintResult(await call(item.expected_contract, "0x63d256ce")) !== 0n
    ) {
      throw new Error(`${item.role} wallet fails canonical policy checks.`);
    }
    return true;
  }

  function renderNetwork() {
    state.network = state.bundle.networks[byId("network").value];
    document.querySelector("[data-owner]").textContent = state.network.pilot.owner;
    document.querySelector("[data-bounty-factory]").textContent = state.network.bounty_factory;
    document.querySelector("[data-wallet-factory]").textContent = state.network.wallet_factory;
    document.querySelector("[data-creator-wallet]").textContent = state.network.pilot.wallets[0].expected_contract;
    document.querySelector("[data-solver-wallet]").textContent = state.network.pilot.wallets[1].expected_contract;
    document.querySelector("[data-relayer]").textContent = state.network.pilot.relayer;
    state.inspected = false;
    for (const id of ["deploy", "wallets", "relayer", "verify"]) byId(id).disabled = true;
  }

  async function inspect() {
    const target = byId("inspect-output");
    try {
      const account = await connect();
      const pending = await pendingDeployments();
      const eth = uintResult(await wallet("eth_getBalance", [account, "latest"]));
      const usdc = await tokenBalance(account);
      const relayerEth = uintResult(await wallet("eth_getBalance", [state.network.pilot.relayer, "latest"]));
      const allowance = await tokenAllowance(account, state.network.wallet_factory);
      const missingWallets = [];
      if (pending.length === 0) {
        for (const item of state.network.pilot.wallets) {
          if (!(await verifyPolicyWallet(item))) missingWallets.push(item);
        }
      }
      const missingFunding = missingWallets.reduce((total, item) => total + BigInt(item.initial_funding), 0n);
      state.inspected = true;
      byId("deploy").disabled = pending.length === 0;
      byId("wallets").disabled = pending.length !== 0 || missingWallets.length === 0 || usdc < missingFunding;
      byId("relayer").disabled = relayerEth >= BigInt(state.network.pilot.relayer_eth_funding_wei);
      byId("verify").disabled = pending.length !== 0;
      write(target, [
        `Provider: ${providerName(state.provider)}`,
        `Account: ${account}`,
        `Network: ${state.network.network} (${state.network.chain_id})`,
        `ETH: ${(Number(eth) / 1e18).toFixed(8)}`,
        `USDC: ${(Number(usdc) / 1e6).toFixed(6)}`,
        `Components pending: ${pending.length}`,
        `Policy wallets pending: ${missingWallets.length}`,
        `Relayer ETH: ${(Number(relayerEth) / 1e18).toFixed(8)}`,
        `Wallet-factory allowance: ${(Number(allowance) / 1e6).toFixed(6)} USDC`,
      ], "success");
    } catch (error) {
      state.inspected = false;
      for (const id of ["deploy", "wallets", "relayer", "verify"]) byId(id).disabled = true;
      write(target, error.message || String(error), "error");
    }
  }

  async function waitReceipt(hash, timeoutMilliseconds = 240_000) {
    const deadline = Date.now() + timeoutMilliseconds;
    while (Date.now() < deadline) {
      const receipt = await wallet("eth_getTransactionReceipt", [hash]);
      if (receipt) {
        if (receipt.status !== "0x1") throw new Error(`Transaction reverted: ${hash}`);
        return receipt;
      }
      await sleep(1_500);
    }
    throw new Error(`Transaction confirmation timed out: ${hash}`);
  }

  async function sendTransaction(transaction, target, label) {
    write(target, `Wallet confirmation requested: ${label}`);
    const hash = await wallet("eth_sendTransaction", [{ from: state.account, ...transaction }]);
    write(target, `Broadcast ${label}: ${hash}\nWaiting for confirmation...`);
    await waitReceipt(hash);
    return hash;
  }

  async function deployComponents() {
    const target = byId("deploy-output");
    byId("deploy").disabled = true;
    try {
      await connect();
      const pending = await pendingDeployments();
      if (pending.length === 0) throw new Error("All canonical components are already deployed.");
      write(target, `Relaying ${pending.length} source-pinned deployment transaction(s)...`);
      const result = await postActivation("/activation/deploy-components", { network: state.network.network });
      if ((await pendingDeployments()).length !== 0) throw new Error("A canonical component remains undeployed.");
      write(target, [
        "All source-pinned canonical components are deployed and configured.",
        ...result.transactions.map((item) => `${item.component}: ${item.hash}`),
      ], "success");
      await inspect();
    } catch (error) {
      byId("deploy").disabled = false;
      write(target, error.message || String(error), "error");
    }
  }

  function signatureParts(signature) {
    const value = String(signature).replace(/^0x/, "");
    if (!/^[0-9a-f]{130}$/i.test(value)) throw new Error("Wallet returned an invalid EIP-712 signature.");
    let v = Number.parseInt(value.slice(128, 130), 16);
    if (v < 27) v += 27;
    if (v !== 27 && v !== 28) throw new Error("Wallet returned an unsupported recovery id.");
    return { r: `0x${value.slice(0, 64)}`, s: `0x${value.slice(64, 128)}`, v };
  }

  function usdcDomain() {
    const domain = state.network.pilot.funding_authorization.domain;
    return {
      name: domain.name,
      version: domain.version,
      chainId: state.network.chain_id,
      verifyingContract: domain.verifying_contract,
    };
  }

  async function signFundingAuthorization(item, validBefore) {
    const authorization = state.network.pilot.funding_authorization;
    const message = {
      from: item.owner,
      to: item.expected_contract,
      value: item.initial_funding,
      validAfter: authorization.valid_after,
      validBefore: String(validBefore),
      nonce: item.funding_authorization_nonce,
    };
    const typedData = {
      types: {
        EIP712Domain: [
          { name: "name", type: "string" },
          { name: "version", type: "string" },
          { name: "chainId", type: "uint256" },
          { name: "verifyingContract", type: "address" },
        ],
        TransferWithAuthorization: [
          { name: "from", type: "address" },
          { name: "to", type: "address" },
          { name: "value", type: "uint256" },
          { name: "validAfter", type: "uint256" },
          { name: "validBefore", type: "uint256" },
          { name: "nonce", type: "bytes32" },
        ],
      },
      domain: usdcDomain(),
      primaryType: "TransferWithAuthorization",
      message,
    };
    const signature = await wallet("eth_signTypedData_v4", [state.account, JSON.stringify(typedData)]);
    return {
      role: item.role,
      valid_after: Number(authorization.valid_after),
      valid_before: validBefore,
      nonce: item.funding_authorization_nonce,
      ...signatureParts(signature),
    };
  }

  async function signAllowanceRevocation(deadline) {
    const nonce = await tokenNonce(state.account);
    const typedData = {
      types: {
        EIP712Domain: [
          { name: "name", type: "string" },
          { name: "version", type: "string" },
          { name: "chainId", type: "uint256" },
          { name: "verifyingContract", type: "address" },
        ],
        Permit: [
          { name: "owner", type: "address" },
          { name: "spender", type: "address" },
          { name: "value", type: "uint256" },
          { name: "nonce", type: "uint256" },
          { name: "deadline", type: "uint256" },
        ],
      },
      domain: usdcDomain(),
      primaryType: "Permit",
      message: {
        owner: state.account,
        spender: state.network.wallet_factory,
        value: "0",
        nonce: String(nonce),
        deadline: String(deadline),
      },
    };
    const signature = await wallet("eth_signTypedData_v4", [state.account, JSON.stringify(typedData)]);
    return { nonce: String(nonce), deadline, ...signatureParts(signature) };
  }

  async function fundWallets() {
    const target = byId("wallet-output");
    byId("wallets").disabled = true;
    try {
      await connect();
      if (!(await verifyWalletFactory())) throw new Error("Wallet factory is not deployed.");
      const missing = [];
      for (const item of state.network.pilot.wallets) {
        if (!(await verifyPolicyWallet(item))) missing.push(item);
      }
      if (missing.length === 0) throw new Error("Both policy wallets already exist.");
      const total = missing.reduce((sum, item) => sum + BigInt(item.initial_funding), 0n);
      if (await tokenBalance(state.account) < total) throw new Error("Owner USDC balance is below required pilot funding.");
      const validity = Math.min(900, Number(state.network.pilot.funding_authorization.max_validity_seconds));
      const validBefore = Math.floor(Date.now() / 1000) + validity;
      const authorizations = [];
      for (const item of missing) {
        write(target, `Signature requested: authorize exactly ${Number(item.initial_funding) / 1e6} USDC to the pinned ${item.role} wallet.`);
        authorizations.push(await signFundingAuthorization(item, validBefore));
      }
      let allowanceRevoke = null;
      if (await tokenAllowance(state.account, state.network.wallet_factory) > 0n) {
        write(target, "Signature requested: revoke the residual wallet-factory USDC allowance to zero.");
        allowanceRevoke = await signAllowanceRevocation(validBefore);
      }
      write(target, "Signatures accepted. The pinned relayer is creating and funding the policy wallets...");
      const result = await postActivation("/activation/fund-policy-wallets", {
        network: state.network.network,
        authorizations,
        allowance_revoke: allowanceRevoke,
      });
      for (const item of state.network.pilot.wallets) {
        if (!(await verifyPolicyWallet(item))) throw new Error(`${item.role} wallet was not created.`);
      }
      write(target, [
        "Both policy wallets are canonical, funded, and registered. Residual factory allowance is zero. The owner paid no transaction gas.",
        ...result.transactions.map((item) => `${item.role}: ${item.hash}`),
      ], "success");
      await inspect();
    } catch (error) {
      write(target, error.message || String(error), "error");
      byId("wallets").disabled = false;
    }
  }

  async function fundRelayer() {
    const target = byId("verify-output");
    byId("relayer").disabled = true;
    try {
      await connect();
      const desired = BigInt(state.network.pilot.relayer_eth_funding_wei);
      const current = uintResult(await wallet("eth_getBalance", [state.network.pilot.relayer, "latest"]));
      if (current >= desired) throw new Error("Relayer already has the pilot gas allocation.");
      const amount = desired - current;
      await sendTransaction(
        { to: state.network.pilot.relayer, data: "0x", value: `0x${amount.toString(16)}` },
        target,
        `fund relayer with ${(Number(amount) / 1e18).toFixed(8)} ETH`,
      );
      write(target, "Relayer gas allocation confirmed.", "success");
      await inspect();
    } catch (error) {
      byId("relayer").disabled = false;
      write(target, error.message || String(error), "error");
    }
  }

  async function verifyActivation() {
    const target = byId("verify-output");
    try {
      await connect();
      if ((await pendingDeployments()).length !== 0) throw new Error("Canonical deployment is incomplete.");
      const lines = [];
      for (const item of state.network.pilot.wallets) {
        if (!(await verifyPolicyWallet(item))) throw new Error(`${item.role} wallet is missing.`);
        const balance = await tokenBalance(item.expected_contract);
        lines.push(`${item.role}: ${item.expected_contract} | ${(Number(balance) / 1e6).toFixed(6)} USDC`);
      }
      const relayerEth = uintResult(await wallet("eth_getBalance", [state.network.pilot.relayer, "latest"]));
      if (relayerEth === 0n) throw new Error("Relayer has no gas.");
      lines.push(`relayer: ${state.network.pilot.relayer} | ${(Number(relayerEth) / 1e18).toFixed(8)} ETH`);
      write(target, ["Activation verified from live chain state.", ...lines, "Safe-block attestation and the autonomous funded loop remain required."], "success");
    } catch (error) {
      write(target, error.message || String(error), "error");
    }
  }

  async function initialize() {
    try {
      requireLocalOrigin();
      const response = await fetch(BUNDLE_URL, { cache: "no-store" });
      if (!response.ok) throw new Error("Activation bundle is unavailable.");
      state.bundle = await response.json();
      if (state.bundle.schema_version !== "agent-bounties/bounded-wallet-activation-v1") {
        throw new Error("Activation bundle schema mismatch.");
      }
      renderNetwork();
      await discoverProviders();
    } catch (error) {
      write(byId("inspect-output"), error.message || String(error), "error");
      byId("inspect").disabled = true;
      return;
    }
    byId("network").addEventListener("change", renderNetwork);
    byId("wallet-provider").addEventListener("change", () => { state.provider = null; renderNetwork(); });
    byId("inspect").addEventListener("click", inspect);
    byId("deploy").addEventListener("click", deployComponents);
    byId("wallets").addEventListener("click", fundWallets);
    byId("relayer").addEventListener("click", fundRelayer);
    byId("verify").addEventListener("click", verifyActivation);
  }

  document.addEventListener("DOMContentLoaded", initialize);
})();
