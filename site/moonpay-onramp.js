(() => {
  "use strict";

  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
  const BASE_CHAIN_ID = "0x2105";
  const CHECKOUT_HOSTS = new Set(["buy.moonpay.com", "buy-sandbox.moonpay.com"]);
  const ATTEMPT_STORAGE = "agent-bounties:onramp-attempts:v1";
  const TOPUP_WINDOW = "agent-bounties-wallet-topup";
  const announcedProviders = [];
  const state = {
    protocol: null,
    providers: [],
    provider: null,
    account: null,
    bountyContract: "",
    requiredUsdc: 0n,
    usdcBalance: null,
    ethBalance: null,
    observedBalanceState: null,
    operationId: "",
    balanceRead: 0,
    checkoutBusy: false,
    balanceBusy: false,
    providerTab: null,
    connection: null,
    observedAt: null,
    blockNumber: null,
    balanceError: null,
    baselineUsdc: null,
    receivedUsdc: 0n,
    refreshPromise: null,
    connectionRead: 0,
    boundProviders: new WeakSet(),
  };

  const select = (selector) => document.querySelector(selector);
  const selectAll = (selector) => [...document.querySelectorAll(selector)];

  function track(eventName) {
    window.agentBountiesAnalytics?.track(eventName);
  }

  function setOutput(selector, message, tone = "") {
    const element = select(selector);
    if (!element) return;
    element.textContent = Array.isArray(message) ? message.join("\n") : message;
    element.dataset.tone = tone;
    if (tone === "error" || tone === "pending") window.AgentBountiesTopupGuide?.feedback(Array.isArray(message) ? message.join(" ") : message, tone);
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
      track("wallet_missing_detected");
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
    if (state.checkoutBusy) throw new Error("Wait for the current checkout request before changing wallets.");
    const protocol = await loadProtocol();
    const provider = selectedProvider();
    const accounts = await provider.request({ method: "eth_requestAccounts" });
    const account = String(accounts?.[0] || "");
    if (!ADDRESS.test(account)) throw new Error("The wallet did not return a valid EVM address.");
    await switchToBase(provider);
    if (state.account !== account.toLowerCase()) select("[data-onramp-ack]").checked = false;
    state.account = account.toLowerCase();
    state.baselineUsdc = null;
    resetBalanceDisplay();
    select("[data-wallet-address]").textContent = state.account;
    select("[data-refresh-balance]").disabled = false;
    renderPurchaseRecovery();
    setOutput("[data-wallet-output]", [
      `Connected: ${state.account}`,
      `Network: ${protocol.network}`,
      "No transaction or signature was requested.",
    ], "success");
    track("wallet_connected");
    void refreshConnection();
    await refreshBalances();
  }

  async function usePublicAddress() {
    if (state.checkoutBusy) throw new Error("Wait for the current checkout request before changing wallets.");
    const account = String(select("[data-wallet-address-input]").value || "").trim();
    if (!ADDRESS.test(account)) {
      throw new Error("Enter one valid public EVM address. Never enter a recovery phrase or private key.");
    }
    state.provider = null;
    if (state.account !== account.toLowerCase()) select("[data-onramp-ack]").checked = false;
    state.account = account.toLowerCase();
    state.baselineUsdc = null;
    resetBalanceDisplay();
    select("[data-wallet-address]").textContent = state.account;
    select("[data-refresh-balance]").disabled = false;
    renderPurchaseRecovery();
    setOutput("[data-wallet-output]", [
      `Public destination: ${state.account}`,
      "Network: base-mainnet",
      "This page can verify balances and receive-address context, but it cannot sign for this wallet.",
    ], "success");
    track("wallet_connected");
    void refreshConnection();
    await refreshBalances();
  }

  function resetBalanceDisplay() {
    state.balanceRead += 1;
    state.usdcBalance = null;
    state.ethBalance = null;
    state.observedAt = null;
    state.receivedUsdc = 0n;
    select("[data-usdc-balance]").textContent = "—";
    select("[data-eth-balance]").textContent = "—";
    select("[data-usdc-shortfall]").textContent = "Check wallet balance";
    select("[data-topup-step-status]").textContent = "Checking the selected wallet";
    select("[data-balance-observed]").textContent = "Balances have not been checked for this wallet.";
    select("[data-quote-guidance]").textContent = "Check the selected wallet's balance before choosing a purchase amount. The provider confirms fees and purchase minimums.";
    renderBalanceGuidance();
  }

  function balanceFresh() {
    return !state.balanceError && state.observedAt && Date.now() - Date.parse(state.observedAt) < 30000;
  }

  function refreshBalances() {
    if (state.refreshPromise) return state.refreshPromise;
    if (!state.account) return Promise.reject(new Error("Choose your wallet first."));
    state.refreshPromise = readBalances().finally(() => { state.refreshPromise = null; });
    return state.refreshPromise;
  }

  async function readBalances() {
    state.balanceBusy = true; notifyGuide();
    const wallet = state.account;
    const read = ++state.balanceRead;
    try {
      const protocol = await loadProtocol();
      const balances = await window.AgentBountiesFundingReadiness.readBalances({ wallet, usdcAddress: protocol.native_usdc });
      if (read !== state.balanceRead || wallet !== state.account) return;
      state.ethBalance = balances.eth;
      state.usdcBalance = balances.usdc;
      state.observedAt = balances.observedAt;
      state.blockNumber = balances.blockNumber;
      state.balanceError = null;
      const baseline = currentAttempt()?.baselineUsdc;
      if (state.baselineUsdc === null) state.baselineUsdc = /^\d+$/.test(baseline || "") ? BigInt(baseline) : balances.usdc;
      state.receivedUsdc = balances.usdc > state.baselineUsdc ? balances.usdc - state.baselineUsdc : 0n;
      select("[data-eth-balance]").textContent = `${formatUnits(state.ethBalance, 18, 6)} ETH`;
      select("[data-usdc-balance]").textContent = `${formatUnits(state.usdcBalance, 6, 6)} USDC`;
      select("[data-balance-observed]").textContent = `Checked on Base at ${new Date(balances.observedAt).toLocaleTimeString()}. Updates automatically.`;
      renderBalanceGuidance();
    } catch (error) {
      if (read === state.balanceRead && wallet === state.account) {
        state.balanceError = error.message || "Balance check failed.";
        state.observedAt = null;
        state.usdcBalance = null; state.ethBalance = null;
        select("[data-usdc-balance]").textContent = "Unavailable";
        select("[data-eth-balance]").textContent = "Unavailable";
        select("[data-usdc-shortfall]").textContent = "Check wallet balance";
      }
      throw error;
    } finally {
      state.balanceBusy = false;
      notifyGuide();
      // A destination change during a read must not leave the new wallet unchecked.
      if (wallet !== state.account && state.account) setTimeout(() => void checkAutomatically(), 0);
    }
  }

  async function checkAutomatically() {
    if (!state.account || document.hidden) return;
    try { await refreshBalances(); } catch (_) { /* Visible retry state; never create another order. */ }
  }

  function boundedRead(action, timeoutMs = 4000) {
    let timer;
    return Promise.race([Promise.resolve().then(action), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("Wallet connection check timed out.")), timeoutMs);
    })]).finally(() => clearTimeout(timer));
  }

  function bindProvider(provider) {
    if (!provider?.on || state.boundProviders.has(provider)) return;
    state.boundProviders.add(provider);
    for (const event of ["accountsChanged", "chainChanged", "disconnect"]) provider.on(event, () => {
      state.connection = null; notifyGuide();
      void refreshConnection(); void checkAutomatically();
    });
  }

  async function refreshConnection() {
    const version = ++state.connectionRead, wallet = state.account;
    const phone = window.AgentBountiesPhoneWallet;
    const candidates = state.provider ? [state.provider] : state.providers.map(item => item.provider);
    if (phone?.provider && !candidates.includes(phone.provider)) candidates.unshift(phone.provider);
    const results = await Promise.all(candidates.map(async provider => {
      try {
        bindProvider(provider);
        if (provider === phone?.provider) {
          await boundedRead(() => phone.restore());
          return phone.state();
        }
        const [accounts, chain] = await boundedRead(() => Promise.all([
          provider.request({ method: "eth_accounts" }), provider.request({ method: "eth_chainId" }),
        ]));
        const address = accounts?.find(value => value.toLowerCase() === wallet) || accounts?.[0];
        return { connected: ADDRESS.test(address || ""), address, chain_id: String(chain).toLowerCase() };
      } catch (_) { return null; }
    }));
    if (version !== state.connectionRead || wallet !== state.account) return;
    state.connection = results.find(value => value?.connected && value.address?.toLowerCase() === wallet)
      || results.find(value => value?.connected) || null;
    notifyGuide();
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
      guidance.textContent = state.bountyContract
        ? "This existing-bounty flow may use gas sponsorship; verify the final wallet request before signing."
        : "New-bounty creation is not gas-sponsored. Add a small amount of Base ETH to the same wallet as the planned USDC.";
      return;
    }
    const enoughUsdc = state.usdcBalance >= state.requiredUsdc;
    const hasGas = state.ethBalance > 0n;
    const messages = [
      enoughUsdc
        ? "This wallet already holds at least the planned USDC contribution."
        : `USDC shortfall: ${formatUnits(state.requiredUsdc - state.usdcBalance, 6, 6)} USDC.`,
      state.bountyContract
        ? "Existing-bounty funding may use gas sponsorship; the final wallet path determines whether ETH is needed."
        : (hasGas
          ? "Base ETH is available. Whether it covers gas remains unknown until the exact transaction is estimated at bounty review."
          : "No Base ETH is visible. New-bounty creation cannot proceed until the same wallet has a small Base ETH balance; choose Base ETH above to review a separate purchase."),
    ];
    guidance.textContent = messages.join(" ");
    guidance.dataset.tone = enoughUsdc ? "success" : "pending";
    const shortfall = window.AgentBountiesFundingReadiness.shortfall(state.requiredUsdc, state.usdcBalance);
    select("[data-usdc-shortfall]").textContent = `${formatUnits(shortfall, 6, 6)} USDC`;
    select("[data-quote-guidance]").textContent = shortfall === 0n
      ? "No USDC purchase is needed for this contribution. Return to the bounty to check gas and review funding."
      : `Request enough to receive at least ${formatUnits(shortfall, 6, 6)} USDC on Base after provider fees. The provider confirms its USD quote and minimum; this page does not assume a USDC/USD exchange rate.`;
    select("[data-topup-step-status]").textContent = enoughUsdc ? "Required USDC visible in wallet" : "More USDC needed";
    const observed = enoughUsdc ? "funded" : "unfunded";
    if (state.observedBalanceState !== observed) {
      state.observedBalanceState = observed;
      track(enoughUsdc ? "wallet_funded_observed" : "wallet_unfunded_detected");
    }
  }

  function safeReturnUrl() {
    const value = new URLSearchParams(location.search).get("return");
    if (value) {
      try {
        const parsed = new URL(value);
        if (
          parsed.origin === location.origin
          && ["/", "/index.html", "/post.html", "/onramp.html", "/participate.html", "/competition.html"].includes(parsed.pathname)
        ) {
          return parsed;
        }
      } catch (_error) {
        // Use the bounded fallback below.
      }
    }
    return new URL("post.html", location.href);
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
    if (bountyContract && !ADDRESS.test(bountyContract)) {
      throw new Error("This on-ramp handoff contains an invalid bounty contract.");
    }
    state.bountyContract = bountyContract.toLowerCase();
    state.requiredUsdc = parseUsdc(params.get("amount"));
    if (state.requiredUsdc <= 0n) {
      throw new Error("This on-ramp handoff is missing a valid planned USDC amount.");
    }
    select("[data-bounty-contract]").textContent = bountyContract
      ? bountyContract.toLowerCase()
      : "New bounty not created yet";
    select("[data-required-usdc]").textContent = `${formatUnits(state.requiredUsdc, 6, 6)} USDC`;
    state.operationId = params.get("operation_id") || params.get("operation") || params.get("posting_operation_id") || params.get("journey") || params.get("intent") || "";
    if (state.operationId && !/^[a-zA-Z0-9_-]{1,128}$/.test(state.operationId)) throw new Error("The posting operation is invalid.");
    select("[data-fiat-amount]").value = "";
    select("[data-partner-options]").hidden = !bountyContract;
    for (const link of selectAll("[data-return-link]")) link.href = safeReturnUrl().href;
    renderReturnStatus();
    track("onramp_viewed");
  }

  function renderAssetHelp() {
    const asset = select("[data-onramp-asset]").value;
    const help = select("[data-asset-help]");
    const button = select("[data-start-moonpay]");
    if (asset === "eth") {
      help.textContent = "Buy Base ETH into the same wallet for transaction gas. This still does not fund the bounty.";
      button.textContent = "Continue to MoonPay for Base ETH";
      select("[data-onramp-ack-copy]").textContent = "I understand that this purchases Base ETH into my wallet and does not yet fund the bounty.";
    } else {
      help.textContent = "Buy Base USDC into your wallet, then return to approve the contribution.";
      button.textContent = "Continue to MoonPay for Base USDC";
      select("[data-onramp-ack-copy]").textContent = "I understand that this purchases Base USDC into my wallet and does not yet fund the bounty.";
    }
    renderPurchaseRecovery();
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
    track("onramp_returned");
  }

  async function requestCheckout() {
    if (!state.account) throw new Error("Connect a wallet or enter its public address first.");
    if (!select("[data-onramp-ack]").checked) {
      throw new Error("Acknowledge that the purchase and bounty funding are separate actions.");
    }
    if (state.checkoutBusy || currentAttempt()) throw new Error("A purchase may already be in progress. Resume or resolve that purchase below before starting another.");
    if (!state.bountyContract) { openDirectCheckout(); return; }
    const amount = String(select("[data-fiat-amount]").value || "").trim();
    if (!/^\d+(?:\.\d{1,2})?$/.test(amount) || Number(amount) <= 0) {
      throw new Error("Enter a positive USD amount with at most two decimal places.");
    }
    const params = new URLSearchParams(location.search);
    const bountyContract = params.get("bountyContract");
    if (bountyContract && !ADDRESS.test(bountyContract)) throw new Error("The bounty contract is invalid.");
    const intent = params.get("intent");
    if (intent && !UUID.test(intent)) throw new Error("The ChatGPT action intent is invalid.");

    const protocol = await loadProtocol();
    if (state.provider) await switchToBase(state.provider);
    if (!bountyContract) {
      openDirectCheckout();
      return;
    }
    if (state.checkoutBusy || currentAttempt()) throw new Error("A purchase may already be in progress. Resume the existing purchase.");
    saveAttempt({ status: "requesting" });
    state.checkoutBusy = true;
    renderPurchaseRecovery();
    track("onramp_moonpay_started");
    const endpoint = `${protocol.mcp_base_url.replace(/\/$/, "")}/v1/onramps/moonpay/checkout`;
    setOutput("[data-onramp-output]", [
      "Creating a device-bound MoonPay checkout URL...",
      "No bounty transaction is being signed.",
    ], "pending");
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 15000);
    try {
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
      signal: controller.signal,
    });
    const body = await response.json().catch(() => null);
    if (!response.ok) {
      clearAttempt();
      throw new Error(body?.error || body?.message || `MoonPay checkout creation failed (${response.status}).`);
    }
    validateCheckoutPlan(body, bountyContract);
    saveAttempt({ status: "opened", reference: body.external_transaction_id });
    setOutput("[data-onramp-output]", [
      body.environment === "sandbox"
        ? "Opening MoonPay sandbox. It validates the checkout flow but will not top up Base mainnet."
        : "Opening MoonPay. Review the final quote, fees, eligibility, asset, network, and wallet before approval.",
      body.evidence_boundary,
    ], "pending");
    location.assign(body.checkout_url);
    } finally {
      clearTimeout(timer);
      state.checkoutBusy = false;
      renderPurchaseRecovery();
    }
  }

  function attemptKey() {
    return `${state.account || ""}:${select("[data-onramp-asset]").value}`;
  }

  function attempts() {
    try {
      const saved = JSON.parse(localStorage.getItem(ATTEMPT_STORAGE) || "{}");
      return saved && typeof saved === "object" && !Array.isArray(saved) ? saved : {};
    } catch (_error) { return {}; }
  }

  function currentAttemptKey() {
    const saved = attempts();
    if (saved[attemptKey()]) return attemptKey();
    return state.account ? [state.account + ":usdc", state.account + ":eth"].find(key => saved[key]) : null;
  }
  function currentAttempt() { return attempts()[currentAttemptKey()] || null; }

  function saveAttempt({ status, reference = "", provider = "moonpay" }) {
    const saved = attempts();
    saved[attemptKey()] = { status, provider, baselineUsdc: state.usdcBalance?.toString() ?? null, operation: state.operationId, startedAt: new Date().toISOString(), reference: /^[a-zA-Z0-9_-]{1,128}$/.test(reference) ? reference : "" };
    // Keep only recovery metadata. Signed checkout URLs and credentials never enter storage.
    localStorage.setItem(ATTEMPT_STORAGE, JSON.stringify(saved));
  }

  function clearAttempt() {
    const saved = attempts();
    delete saved[currentAttemptKey()];
    localStorage.setItem(ATTEMPT_STORAGE, JSON.stringify(saved));
  }

  function renderPurchaseRecovery() {
    const attempt = currentAttempt();
    select("[data-purchase-recovery]").hidden = !attempt;
    select("[data-start-moonpay]").disabled = Boolean(state.checkoutBusy || attempt || !state.account || !select("[data-onramp-ack]").checked);
    for (const selector of ["[data-onramp-asset]", "[data-wallet-provider]", "[data-connect-wallet]", "[data-use-wallet-address]", "[data-fiat-amount]", "[data-clear-purchase]"]) {
      const element = select(selector);
      element.disabled = state.checkoutBusy || (selector === "[data-wallet-provider]" && !state.providers.length);
    }
    select("[data-purchase-recovery-copy]").textContent = attempt
      ? `${attempt.provider === "metamask" ? "MetaMask" : "MoonPay"} was opened for this wallet. Purchase status is unverified.`
      : "";
    window.dispatchEvent(new Event("agent-bounties:onramp-state"));
    notifyGuide();
  }

  function canOpenPurchase() {
    return Boolean(state.account && balanceFresh() && !state.checkoutBusy && !currentAttempt());
  }

  function openDirectCheckout(provider = "moonpay") {
    if (!["moonpay", "metamask"].includes(provider)) throw new Error("Choose MoonPay or MetaMask.");
    if (!state.account) throw new Error("Choose the destination wallet first.");
    if (state.checkoutBusy || currentAttempt()) throw new Error("Resume or resolve the existing purchase before opening another checkout.");
    if (!balanceFresh()) throw new Error("Check your balance before opening a purchase.");
    const asset = select("[data-onramp-asset]").value;
    const destination = provider === "metamask" ? "https://portfolio.metamask.io/"
      : asset === "eth" ? "https://www.moonpay.com/buy/eth" : "https://www.moonpay.com/buy/usdc";
    const tab = window.open("about:blank", TOPUP_WINDOW);
    if (!tab) throw new Error("Allow the checkout tab, then try again. No purchase was opened.");
    tab.opener = null;
    state.providerTab = tab;
    try {
      saveAttempt({ status: "opened", provider });
      tab.location.replace(destination);
    } catch (error) { tab.close(); throw error; }
    renderPurchaseRecovery();
    track(provider === "metamask" ? "onramp_metamask_started" : "onramp_moonpay_started");
  }

  function topupStatus() {
    const fresh = Boolean(balanceFresh());
    const shortfall = fresh ? window.AgentBountiesFundingReadiness.shortfall(state.requiredUsdc, state.usdcBalance) : null;
    return { operation_id: state.operationId, wallet_address: state.account,
      wallet_connected: Boolean(state.connection?.connected && state.connection.address?.toLowerCase() === state.account),
      wallet_chain_id: state.connection?.chain_id || null, balance_network: "base-mainnet",
      required_usdc: formatUnits(state.requiredUsdc, 6, 6), usdc_balance: fresh ? formatUnits(state.usdcBalance, 6, 6) : null,
      usdc_shortfall: shortfall === null ? null : formatUnits(shortfall, 6, 6),
      eth_balance: fresh ? formatUnits(state.ethBalance, 18, 18) : null,
      received_usdc_since_first_check: fresh ? formatUnits(state.receivedUsdc, 6, 6) : null,
      observed_at: state.observedAt, block_number: state.blockNumber, balance_error: state.balanceError,
      ready_for_bounty_review: fresh && shortfall === 0n && (Boolean(state.bountyContract) || state.ethBalance > 0n),
      provider_order_status: currentAttempt() ? "unverified" : "not_recorded", purchase_opened: Boolean(currentAttempt()),
      bounty_funded: false, payment_authorized: false, return_url: safeReturnUrl().href, poll_after_ms: 5000,
      next_action: fresh && shortfall === 0n ? "Return to the saved bounty review to check fees and approve funding." : currentAttempt() ? "Wait for the existing purchase; do not buy again." : "Choose wallet or card. The person confirms any purchase with the provider." };
  }

  function notifyGuide() {
    window.AgentBountiesTopupGuide?.render({ account: state.account, required: state.requiredUsdc,
      usdc: state.usdcBalance, eth: state.ethBalance, busy: state.balanceBusy || state.checkoutBusy,
      fresh: Boolean(balanceFresh()), error: state.balanceError, connection: state.connection, received: state.receivedUsdc,
      pending: Boolean(currentAttempt()), existingBounty: Boolean(state.bountyContract) });
    window.dispatchEvent(new Event("agent-bounties:onramp-state"));
  }
  window.AgentBountiesOnramp = Object.freeze({ hasPendingPurchase: () => Boolean(currentAttempt()), canOpenPurchase, openDirectCheckout, status: topupStatus });

  function registerTopupTools() {
    if (window.agentBountiesAnalyticsConfig?.webMcpEnabled === false) return;
    try { if (new URLSearchParams(location.search).get("webmcp") === "off" || localStorage.getItem("agent-bounties.webmcp.disabled.v1") === "true") return; } catch (_) { /* Optional preference storage. */ }
    const registry = document.modelContext;
    if (!registry?.registerTool) return;
    const lifecycle = new AbortController();
    const empty = { type: "object", properties: {}, additionalProperties: false };
    const validate = (input, keys = []) => {
      if (!input || typeof input !== "object" || Array.isArray(input) || Object.keys(input).some(key => !keys.includes(key))) throw new Error("Invalid top-up input.");
    };
    const register = tool => {
      try { Promise.resolve(registry.registerTool(tool, { signal: lifecycle.signal })).catch(() => console.warn("Top-up WebMCP registration unavailable.")); }
      catch (_) { console.warn("Top-up WebMCP registration unavailable; use the page controls."); }
    };
    register({ name: "agent_bounties_get_topup_status", title: "Check money in my wallet",
      description: "Refresh public Base balances and the existing wallet session, then return the exact shortfall, saved review URL and uncertain purchase state. Never opens checkout, asks for a signature, clears an order or declares bounty funding. A smaller purchase is enough if the wallet covers the bounty total. Poll at the returned interval; do not repeat a pending purchase.",
      inputSchema: empty, annotations: { readOnlyHint: true, untrustedContentHint: false },
      async execute(input) { validate(input); await Promise.all([refreshConnection(), refreshBalances().catch(() => {})]); return topupStatus(); } });
    register({ name: "agent_bounties_prepare_topup", title: "Prepare the add-money screen",
      description: "Show wallet-app or card purchase choices for the saved destination. Select the missing asset and check balances for the person. No checkout is opened and no purchase, account, legal or wallet confirmation is accepted. The person chooses the amount and confirms any purchase with the provider. An existing uncertain order stays protected.",
      inputSchema: { type: "object", properties: { method: { type: "string", enum: ["wallet", "card"] } }, required: ["method"], additionalProperties: false },
      annotations: { readOnlyHint: false, untrustedContentHint: false },
      async execute(input) { validate(input, ["method"]); if (!["wallet", "card"].includes(input.method)) throw new Error("Choose wallet or card."); await refreshBalances().catch(() => {}); return { ...topupStatus(), ...window.AgentBountiesTopupGuide.show(input.method) }; } });
    window.addEventListener("pagehide", event => { if (!event.persisted) lifecycle.abort(); });
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
    window.AgentBountiesTopupGuide?.feedback("Working on it…", "pending");
    try {
      await action();
      window.AgentBountiesTopupGuide?.feedback("", "");
    } catch (error) {
      const message = error.name === "AbortError" ? "Checkout request timed out. Check the existing purchase before retrying; its status remains unverified." : error.message || String(error);
      setOutput("[data-onramp-output]", message, "error");
      if (action === connectWallet || action === usePublicAddress || action === refreshBalances) {
        setOutput("[data-wallet-output]", error.message || String(error), "error");
      }
    }
  }

  function wireEvents() {
    select("[data-connect-wallet]").addEventListener("click", () => run(connectWallet));
    select("[data-use-wallet-address]").addEventListener("click", () => run(usePublicAddress));
    select("[data-refresh-balance]").addEventListener("click", () => run(refreshBalances));
    select("[data-start-moonpay]").addEventListener("click", () => run(requestCheckout));
    select("[data-onramp-asset]").addEventListener("change", () => {
      select("[data-onramp-ack]").checked = false;
      select("[data-purchase-resolved]").checked = false;
      renderAssetHelp();
    });
    select("[data-onramp-ack]").addEventListener("change", renderPurchaseRecovery);
    select("[data-resume-checkout]").addEventListener("click", () => {
      if (state.providerTab && !state.providerTab.closed) state.providerTab.focus();
      else setOutput("[data-onramp-output]", "Use the purchase tab or your confirmation email. Check the wallet address and Base network on the receipt.", "pending");
    });
    select("[data-clear-purchase]").addEventListener("click", () => run(async () => {
      if (!select("[data-purchase-resolved]").checked) throw new Error("Confirm the provider shows the earlier purchase completed or cancelled, with no pending payment.");
      clearAttempt();
      select("[data-purchase-resolved]").checked = false;
      select("[data-onramp-ack]").checked = false;
      renderPurchaseRecovery();
      await refreshBalances();
    }));
    window.addEventListener("storage", renderPurchaseRecovery);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) { void checkAutomatically(); void refreshConnection(); }
    });
    window.addEventListener("focus", () => { void checkAutomatically(); void refreshConnection(); });
    window.addEventListener("pageshow", () => { void checkAutomatically(); void refreshConnection(); });
    window.addEventListener("agent-bounties:phone-wallet-state", event => {
      if (event.detail?.connected || !state.connection?.connected || state.connection?.connection_route === "external_wallet_walletconnect") {
        state.connection = event.detail; notifyGuide();
      }
    });
    setInterval(() => { void checkAutomatically(); }, 5000);
    setInterval(() => { if (!document.hidden) void refreshConnection(); }, 15000);
    select("[data-wallet-provider]").addEventListener("change", () => {
      state.provider = null;
      state.account = null;
      select("[data-onramp-ack]").checked = false;
      resetBalanceDisplay();
      select("[data-start-moonpay]").disabled = true;
      select("[data-refresh-balance]").disabled = true;
      select("[data-wallet-address]").textContent = "Not connected";
      select("[data-usdc-balance]").textContent = "—";
      select("[data-eth-balance]").textContent = "—";
      select("[data-usdc-shortfall]").textContent = "Check wallet balance";
      renderPurchaseRecovery();
    });
    for (const link of selectAll("[data-onramp-provider]")) {
      link.addEventListener("click", event => {
        const provider = link.dataset.onrampProvider;
        if (provider === "metamask") {
          event.preventDefault();
          try { openDirectCheckout("metamask"); } catch (error) { window.AgentBountiesTopupGuide?.feedback(error.message, "error"); }
          return;
        }
        if (provider === "moonpay") track("onramp_moonpay_started");
        if (provider === "metamask") track("onramp_metamask_started");
        if (provider === "coinbase") track("onramp_coinbase_started");
      });
    }
  }

  async function initialize() {
    try {
      renderContext();
      if (new URLSearchParams(location.search).get("asset") === "eth") select("[data-onramp-asset]").value = "eth";
      renderAssetHelp();
      wireEvents();
      await discoverProviders();
      const wallet = new URLSearchParams(location.search).get("wallet");
      if (wallet && ADDRESS.test(wallet)) {
        select("[data-wallet-address-input]").value = wallet;
        await usePublicAddress();
      }
    } catch (error) {
      setOutput("[data-onramp-output]", error.message || String(error), "error");
      select("[data-start-moonpay]").disabled = true;
    } finally { notifyGuide(); registerTopupTools(); }
  }

  initialize();
})();
