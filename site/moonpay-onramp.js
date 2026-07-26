(() => {
  "use strict";

  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  const BASE_CHAIN_ID = "0x2105";
  const BALANCE_OF_SELECTOR = "70a08231";
  const CHECKOUT_HOSTS = new Set(["buy.moonpay.com", "buy-sandbox.moonpay.com"]);
  const announcedProviders = [];
  const state = {
    protocol: null,
    providers: [],
    provider: null,
    account: null,
    requiredUsdc: 0n,
    usdcBalance: null,
    ethBalance: null,
  };

  const select = (selector) => document.querySelector(selector);
  const selectAll = (selector) => [...document.querySelectorAll(selector)];

  function setOutput(selector, message, tone = "") {
    const element = select(selector);
    if (!element) return;
    element.textContent = Array.isArray(message) ? message.join("\n") : message;
    element.dataset.tone = tone;
  }

  function providerName(provider, info = {}) {
    if (info.name) return info.name;
    if (provider.isMetaMask) return "MetaMask";
    if (provider.isCoinbaseWallet) return "Coinbase Wallet";
    if (provider.isBraveWallet) return "Brave Wallet";
    return "Browser wallet";
  }

  function validProvider(provider) {
    return Boolean(provider && typeof provider.request === "function");
  }

  function rememberProvider(event) {
    const detail = event?.detail;
    if (!detail || !validProvider(detail.provider)) return;
    if (!announcedProviders.some((item) => item.provider === detail.provider)) {
      announcedProviders.push(detail);
    }
  }

  window.addEventListener("eip6963:announceProvider", rememberProvider);

  async function discoverProviders() {
    window.dispatchEvent(new Event("eip6963:requestProvider"));
    await new Promise((resolve) => setTimeout(resolve, 250));
    const candidates = [...announcedProviders];
    const injected = window.ethereum && Array.isArray(window.ethereum.providers)
      ? window.ethereum.providers
      : (window.ethereum ? [window.ethereum] : []);
    for (const provider of injected) {
      if (validProvider(provider) && !candidates.some((item) => item.provider === provider)) {
        candidates.push({ provider, info: {} });
      }
    }
    state.providers = candidates;
    const selector = select("[data-wallet-provider]");
    selector.textContent = "";
    if (!candidates.length) {
      const option = document.createElement("option");
      option.textContent = "No browser wallet detected";
      selector.append(option);
      selector.disabled = true;
      return;
    }
    candidates.forEach((item, index) => {
      const option = document.createElement("option");
      option.value = String(index);
      option.textContent = providerName(item.provider, item.info);
      selector.append(option);
    });
    selector.disabled = false;
  }

  function selectedProvider() {
    const index = Number.parseInt(select("[data-wallet-provider]").value, 10);
    const candidate = state.providers[index];
    if (!candidate) throw new Error("Unlock a browser wallet, reload, and select it here.");
    state.provider = candidate.provider;
    return candidate.provider;
  }

  async function loadProtocol() {
    if (state.protocol) return state.protocol;
    const response = await fetch("protocol.json", { cache: "no-store" });
    if (!response.ok) throw new Error("Protocol configuration is unavailable.");
    const protocol = await response.json();
    if (
      protocol.status !== "active"
      || protocol.network !== "base-mainnet"
      || protocol.chain_id !== 8453
      || !ADDRESS.test(protocol.native_usdc || "")
      || !/^https:\/\//.test(protocol.mcp_base_url || "")
    ) {
      throw new Error("The active Base protocol configuration could not be verified.");
    }
    state.protocol = protocol;
    return protocol;
  }

  async function switchToBase(provider) {
    try {
      await provider.request({
        method: "wallet_switchEthereumChain",
        params: [{ chainId: BASE_CHAIN_ID }],
      });
    } catch (error) {
      if (Number(error?.code) !== 4902) throw error;
      await provider.request({
        method: "wallet_addEthereumChain",
        params: [{
          chainId: BASE_CHAIN_ID,
          chainName: "Base",
          nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
          rpcUrls: ["https://mainnet.base.org"],
          blockExplorerUrls: ["https://basescan.org"],
        }],
      });
    }
    const chainId = await provider.request({ method: "eth_chainId" });
    if (String(chainId).toLowerCase() !== BASE_CHAIN_ID) {
      throw new Error("Switch the connected wallet to Base mainnet before continuing.");
    }
  }

  async function connectWallet() {
    const protocol = await loadProtocol();
    const provider = selectedProvider();
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    const account = String(accounts?.[0] || "");
    if (!ADDRESS.test(account)) throw new Error("The wallet did not return a valid EVM address.");
    await switchToBase(provider);
    state.account = account.toLowerCase();
    select("[data-wallet-address]").textContent = state.account;
    select("[data-refresh-balance]").disabled = false;
    select("[data-start-moonpay]").disabled = !select("[data-onramp-ack]").checked;
    setOutput("[data-wallet-output]", [
      `Connected: ${state.account}`,
      `Network: ${protocol.network}`,
      "No transaction or signature was requested.",
    ], "success");
    await refreshBalances();
  }

  function paddedAddress(address) {
    return address.slice(2).toLowerCase().padStart(64, "0");
  }

  async function refreshBalances() {
    if (!state.provider || !state.account) throw new Error("Connect the destination wallet first.");
    const protocol = await loadProtocol();
    await switchToBase(state.provider);
    const [ethHex, usdcHex] = await Promise.all([
      state.provider.request({ method: "eth_getBalance", params: [state.account, "latest"] }),
      state.provider.request({
        method: "eth_call",
        params: [{
          to: protocol.native_usdc,
          data: `0x${BALANCE_OF_SELECTOR}${paddedAddress(state.account)}`,
        }, "latest"],
      }),
    ]);
    state.ethBalance = BigInt(ethHex || "0x0");
    state.usdcBalance = BigInt(usdcHex || "0x0");
    select("[data-eth-balance]").textContent = `${formatUnits(state.ethBalance, 18, 6)} ETH`;
    select("[data-usdc-balance]").textContent = `${formatUnits(state.usdcBalance, 6, 6)} USDC`;
    renderBalanceGuidance();
  }

  function formatUnits(value, decimals, maximumFractionDigits) {
    const negative = value < 0n;
    const absolute = negative ? -value : value;
    const scale = 10n ** BigInt(decimals);
    const whole = absolute / scale;
    const remainder = absolute % scale;
    const fraction = remainder.toString().padStart(decimals, "0")
      .slice(0, maximumFractionDigits)
      .replace(/0+$/, "");
    return `${negative ? "-" : ""}${whole}${fraction ? `.${fraction}` : ""}`;
  }

  function parseUsdc(value) {
    const trimmed = String(value || "").trim();
    if (!/^\d+(?:\.\d{1,6})?$/.test(trimmed)) return 0n;
    const [whole, fraction = ""] = trimmed.split(".");
    return BigInt(whole) * 1_000_000n + BigInt(fraction.padEnd(6, "0"));
  }

  function renderBalanceGuidance() {
    const guidance = select("[data-balance-guidance]");
    if (state.usdcBalance === null || state.ethBalance === null) {
      guidance.textContent = "Base ETH may be required for wallet transaction gas unless the final wallet path sponsors it.";
      return;
    }
    const enoughUsdc = state.usdcBalance >= state.requiredUsdc;
    const hasGas = state.ethBalance > 0n;
    const messages = [
      enoughUsdc
        ? "This wallet already holds at least the planned USDC contribution."
        : `USDC shortfall: ${formatUnits(state.requiredUsdc - state.usdcBalance, 6, 6)} USDC.`,
      hasGas
        ? "This wallet has some Base ETH for gas. The wallet still decides the actual fee."
        : "No Base ETH is visible. The existing funding path may require gas unless the selected wallet sponsors it; MoonPay can also buy Base ETH.",
    ];
    guidance.textContent = messages.join(" ");
    guidance.dataset.tone = enoughUsdc && hasGas ? "success" : "pending";
  }

  function safeReturnUrl() {
    const value = new URLSearchParams(location.search).get("return");
    if (value) {
      try {
        const parsed = new URL(value);
        if (
          parsed.origin === location.origin
          && ["/earn.html", "/funding.html"].includes(parsed.pathname)
        ) {
          return parsed;
        }
      } catch (_error) {
        // Use the bounded fallback below.
      }
    }
    return new URL("earn.html#fund-bounty-panel", location.href);
  }

  function checkoutReturnUrl() {
    const url = new URL(location.href);
    for (const key of ["transactionId", "transactionStatus", "status", "transaction_id"]) {
      url.searchParams.delete(key);
    }
    url.hash = "";
    return url;
  }

  function renderContext() {
    const params = new URLSearchParams(location.search);
    const bountyContract = params.get("bountyContract") || "";
    if (!ADDRESS.test(bountyContract)) {
      throw new Error("This MoonPay handoff is missing a valid bounty contract.");
    }
    state.requiredUsdc = parseUsdc(params.get("amount"));
    if (state.requiredUsdc <= 0n) {
      throw new Error("This MoonPay handoff is missing a valid planned USDC amount.");
    }
    select("[data-bounty-contract]").textContent = bountyContract.toLowerCase();
    select("[data-required-usdc]").textContent = `${formatUnits(state.requiredUsdc, 6, 6)} USDC`;
    const suggestedUsd = Math.max(20, Math.ceil(Number(state.requiredUsdc) / 1_000_000 * 1.08 * 100) / 100);
    select("[data-fiat-amount]").value = suggestedUsd.toFixed(2);
    for (const link of selectAll("[data-return-link]")) link.href = safeReturnUrl().href;
    renderReturnStatus();
  }

  function renderAssetHelp() {
    const asset = select("[data-onramp-asset]").value;
    const help = select("[data-asset-help]");
    const button = select("[data-start-moonpay]");
    if (asset === "eth") {
      help.textContent = "Buy Base ETH into the same wallet for transaction gas. This still does not fund the bounty.";
      button.textContent = "Continue to MoonPay for Base ETH";
      if (Number(select("[data-fiat-amount]").value) > 100) {
        select("[data-fiat-amount]").value = "20.00";
      }
    } else {
      help.textContent = "Buy Base USDC into your wallet, then return to approve the contribution.";
      button.textContent = "Continue to MoonPay for Base USDC";
    }
  }

  function renderReturnStatus() {
    const params = new URLSearchParams(location.search);
    const transactionId = params.get("transactionId") || params.get("transaction_id");
    const status = params.get("transactionStatus") || params.get("status");
    if (!transactionId && !status) return;
    const container = select("[data-return-status]");
    container.hidden = false;
    select("[data-return-status-copy]").textContent = [
      status ? `MoonPay redirect status: ${status}.` : "MoonPay returned without a status value.",
      transactionId ? `MoonPay transaction reference: ${transactionId}.` : "No transaction reference was supplied.",
    ].join(" ");
  }

  async function requestCheckout() {
    if (!state.account || !state.provider) throw new Error("Connect the destination wallet first.");
    if (!select("[data-onramp-ack]").checked) {
      throw new Error("Acknowledge that the purchase and bounty funding are separate actions.");
    }
    const amount = String(select("[data-fiat-amount]").value || "").trim();
    if (!/^\d+(?:\.\d{1,2})?$/.test(amount) || Number(amount) <= 0) {
      throw new Error("Enter a positive USD amount with at most two decimal places.");
    }
    const params = new URLSearchParams(location.search);
    const bountyContract = params.get("bountyContract");
    if (!ADDRESS.test(bountyContract || "")) throw new Error("The bounty contract is invalid.");
    const intent = params.get("intent");
    if (intent && !UUID.test(intent)) throw new Error("The ChatGPT action intent is invalid.");

    const protocol = await loadProtocol();
    await switchToBase(state.provider);
    const endpoint = `${protocol.mcp_base_url.replace(/\/$/, "")}/v1/onramps/moonpay/checkout`;
    setOutput("[data-onramp-output]", [
      "Creating a device-bound MoonPay checkout URL...",
      "No bounty transaction is being signed.",
    ], "pending");
    const response = await fetch(endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        wallet_address: state.account,
        base_currency_amount: amount,
        base_currency_code: "usd",
        asset: select("[data-onramp-asset]").value,
        return_url: checkoutReturnUrl().href,
        intent_id: intent || null,
        bounty_contract: bountyContract,
      }),
      cache: "no-store",
      credentials: "omit",
    });
    const body = await response.json().catch(() => null);
    if (!response.ok) {
      throw new Error(body?.error || body?.message || `MoonPay checkout creation failed (${response.status}).`);
    }
    validateCheckoutPlan(body, bountyContract);
    sessionStorage.setItem("agent-bounties:moonpay:last-external-transaction", body.external_transaction_id);
    setOutput("[data-onramp-output]", [
      body.environment === "sandbox"
        ? "Opening MoonPay sandbox. It validates the checkout flow but will not top up Base mainnet."
        : "Opening MoonPay. Review the final quote, fees, eligibility, asset, network, and wallet before approval.",
      body.evidence_boundary,
    ], "pending");
    location.assign(body.checkout_url);
  }

  function validateCheckoutPlan(body, bountyContract) {
    if (
      !body
      || body.schema_version !== "agent-bounties/moonpay-onramp-checkout-v1"
      || body.provider !== "moonpay"
      || body.destination_wallet?.toLowerCase() !== state.account
      || body.bounty_contract?.toLowerCase() !== bountyContract.toLowerCase()
      || body.bounty_funded !== false
      || body.canonical_funding_event !== null
    ) {
      throw new Error("The MoonPay checkout response did not preserve the reviewed wallet and bounty boundary.");
    }
    const checkout = new URL(body.checkout_url);
    if (
      checkout.protocol !== "https:"
      || !CHECKOUT_HOSTS.has(checkout.hostname)
      || !checkout.searchParams.get("signature")
      || checkout.searchParams.get("walletAddress")?.toLowerCase() !== state.account
    ) {
      throw new Error("The MoonPay checkout URL is not an approved signed MoonPay destination.");
    }
  }

  async function run(action) {
    try {
      await action();
    } catch (error) {
      setOutput("[data-onramp-output]", error.message || String(error), "error");
      if (action === connectWallet || action === refreshBalances) {
        setOutput("[data-wallet-output]", error.message || String(error), "error");
      }
    }
  }

  function wireEvents() {
    select("[data-connect-wallet]").addEventListener("click", () => run(connectWallet));
    select("[data-refresh-balance]").addEventListener("click", () => run(refreshBalances));
    select("[data-start-moonpay]").addEventListener("click", () => run(requestCheckout));
    select("[data-onramp-asset]").addEventListener("change", renderAssetHelp);
    select("[data-onramp-ack]").addEventListener("change", (event) => {
      select("[data-start-moonpay]").disabled = !(event.currentTarget.checked && state.account);
    });
    select("[data-wallet-provider]").addEventListener("change", () => {
      state.provider = null;
      state.account = null;
      select("[data-start-moonpay]").disabled = true;
      select("[data-refresh-balance]").disabled = true;
      select("[data-wallet-address]").textContent = "Not connected";
      select("[data-usdc-balance]").textContent = "—";
      select("[data-eth-balance]").textContent = "—";
    });
  }

  async function initialize() {
    try {
      renderContext();
      renderAssetHelp();
      wireEvents();
      await discoverProviders();
    } catch (error) {
      setOutput("[data-onramp-output]", error.message || String(error), "error");
      select("[data-start-moonpay]").disabled = true;
    }
  }

  initialize();
})();
