/* Account wallet selection. Discovery never requests accounts or signatures. */
"use strict";
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory;
  else root.AgentBountiesWalletLink = factory(root);
})(typeof window === "object" ? window : globalThis, function createWalletLink(win) {
  const doc = win.document;
  const discovered = new Map();
  let chooser, list, status, createButton, pending, bundlePromise, brand = null, continuation;
  const cancelled = () => Object.assign(new Error("Wallet selection cancelled."), { code: 4001 });

  function remember(detail) {
    if (!detail?.provider || typeof detail.provider.request !== "function") return;
    const name = String(detail.info?.name || "Browser wallet").trim().slice(0, 64);
    discovered.set(detail.provider, { provider: detail.provider, label: name || "Browser wallet", kind: "browser", rdns: String(detail.info?.rdns || "") });
    if (chooser?.open) render();
  }
  win.addEventListener("eip6963:announceProvider", (event) => remember(event.detail));
  win.dispatchEvent(new win.Event("eip6963:requestProvider"));

  function choices() {
    const phone = win.AgentBountiesPhoneWallet;
    const embedded = win.AgentBountiesCoinbaseEmbeddedWallet?.provider;
    const result = [...discovered.values()].filter((item) => item.provider !== phone?.provider && item.provider !== embedded);
    const injected = Array.isArray(win.ethereum?.providers) && win.ethereum.providers.length
      ? win.ethereum.providers : [win.ethereum];
    for (const provider of injected) {
      if (!provider || provider === phone?.provider || provider === embedded || typeof provider.request !== "function" || result.some((item) => item.provider === provider)) continue;
      result.push({ provider, kind: "browser", label: provider.isCoinbaseWallet ? "Base App / Coinbase Wallet" : provider.isMetaMask ? "MetaMask" : "Browser wallet" });
    }
    if (phone?.state().available) result.push({ provider: phone.provider, kind: "phone", label: "Use a phone wallet" });
    return result;
  }

  function embeddedConfigured() {
    return Boolean(win.AgentBountiesWalletConfig?.providers?.coinbaseEmbedded?.enabled);
  }

  async function loadEmbedded() {
    if (!embeddedConfigured()) throw new Error("Coinbase embedded wallet is not configured on this site yet.");
    if (win.AgentBountiesCoinbaseEmbeddedWallet?.enabled) {
      await win.AgentBountiesCoinbaseEmbeddedWallet.checkReadiness?.();
      return win.AgentBountiesCoinbaseEmbeddedWallet.provider;
    }
    if (!bundlePromise) {
      bundlePromise = new Promise((resolve, reject) => {
        let failed = false;
        const stylesheet = doc.createElement("link");
        stylesheet.rel = "stylesheet";
        stylesheet.href = new URL("vendor/coinbase-embedded-wallet.bundle.css?v=4", doc.baseURI).href;
        const script = doc.createElement("script");
        script.src = new URL("vendor/coinbase-embedded-wallet.bundle.js?v=4", doc.baseURI).href;
        script.async = true;
        const fail = () => { failed = true; win.clearTimeout(timer); script.remove(); stylesheet.remove(); reject(new Error("Coinbase embedded wallet could not load. Check your connection and try again, or choose another wallet below.")); };
        const timer = win.setTimeout(fail, 12000);
        stylesheet.onerror = fail;
        stylesheet.onload = () => { if (!failed) doc.head.append(script); };
        script.onerror = fail;
        script.onload = () => {
          if (failed) return;
          win.clearTimeout(timer);
          const adapter = win.AgentBountiesCoinbaseEmbeddedWallet;
          if (!adapter?.enabled || typeof adapter.provider?.request !== "function") fail();
          else resolve(adapter.provider);
        };
        doc.head.append(stylesheet);
      }).catch((error) => { bundlePromise = null; throw error; });
    }
    const provider = await bundlePromise;
    await win.AgentBountiesCoinbaseEmbeddedWallet?.checkReadiness?.();
    return provider;
  }

  function finish(choice) {
    const request = pending;
    if (!request) return;
    pending = null;
    chooser.close();
    if (choice) request.resolve(choice);
    else request.reject(cancelled());
  }

  function option(label, description, select) {
    const button = doc.createElement("button");
    button.type = "button";
    button.className = "wallet-choice";
    const title = doc.createElement("strong");
    title.textContent = label;
    const detail = doc.createElement("span");
    detail.textContent = description;
    button.append(title, detail);
    button.addEventListener("click", select);
    return button;
  }

  function brandOf(choice) {
    // Coinbase may also expose isMetaMask for compatibility. Prefer its own identity.
    if (choice.rdns === "com.coinbase.wallet" || choice.provider.isCoinbaseWallet) return "coinbase";
    if (choice.rdns === "io.metamask") return "metamask";
    if (choice.rdns || choice.provider.isBraveWallet || choice.provider.isTrust || choice.provider.isTrustWallet) return "other";
    if (choice.provider.isMetaMask) return "metamask";
    return "other";
  }
  function chooseBrand(value) {
    brand = value; status.textContent = ""; render();
    chooser.querySelector("#wallet-link-title").focus();
  }
  function note(message) {
    const p = doc.createElement("p"); p.textContent = message; list.append(p);
  }
  async function copyContinuation() {
    const request = pending;
    status.textContent = "Saving your place…";
    try {
      if (!continuation) throw new Error("Open agentbounties.app in your wallet app. Sign in with the same account.");
      const url = await continuation();
      if (pending !== request) return;
      await win.navigator.clipboard.writeText(url);
      if (pending === request) status.textContent = "Link copied. Paste it in your wallet app’s browser. Sign in with the same account.";
    } catch (error) { if (pending === request) status.textContent = error.message; }
  }
  function render() {
    if (!list) return;
    list.replaceChildren();
    const names = { coinbase: "Coinbase", metamask: "MetaMask", moonpay: "MoonPay", other: "Other wallets" };
    chooser.querySelector("#wallet-link-title").textContent = brand ? names[brand] : "Choose your wallet";
    chooser.querySelector("[data-wallet-intro]").textContent = brand ? "" : "Use one wallet to add money and approve your bounty.";
    chooser.querySelector("[data-wallet-back]").hidden = !brand;
    chooser.querySelector("[data-wallet-create]").hidden = brand !== "coinbase";
    if (!brand) {
      for (const [id, title, description] of [
        ["coinbase", "Coinbase", "Use your Base app or Coinbase account wallet."],
        ["metamask", "MetaMask", "Use your phone app or browser extension."],
        ["moonpay", "MoonPay", "Full bounty payments are not available here yet."],
      ]) list.append(option(title, description, () => chooseBrand(id)));
      const more = doc.createElement("details"), summary = doc.createElement("summary");
      summary.textContent = "Other wallets";
      const extra = doc.createElement("div");
      for (const choice of choices().filter(item => item.kind === "phone" || brandOf(item) === "other"))
        extra.append(option(choice.label, choice.kind === "phone" ? "Pair a supported phone wallet." : "Connect this wallet.", () => finish(choice)));
      more.append(summary, extra); list.append(more);
      return;
    }
    if (brand === "moonpay") {
      note("MoonPay can hold, buy and send USDC. Approving a bounty from its wallet is not supported here yet.");
      note("Do not buy more for this bounty in MoonPay. Your existing money stays in your MoonPay wallet.");
      const help = doc.createElement("a"); help.href = "https://support.moonpay.com/en/articles/383215-managing-your-wallets";
      help.target = "_blank"; help.rel = "noopener noreferrer"; help.textContent = "MoonPay wallet help"; list.append(help);
      return;
    }
    const matches = choices().filter(item => item.kind !== "phone" && brandOf(item) === brand);
    for (const choice of matches) list.append(option("Connect " + names[brand], "Approve the connection in this wallet.", () => finish(choice)));
    if (brand === "coinbase") {
      if (!matches.length) {
        note("Open this bounty in the Base app’s browser. Coinbase Wallet is now the Base app.");
        list.append(option("Copy bounty link", "Paste it in the Base app’s browser.", copyContinuation));
      }
      note("Keep the same address when buying USDC on Base. Return here to approve your bounty.");
      // Email-created wallets are distinct from an existing Base app wallet.
      createButton.disabled = !embeddedConfigured();
      if (!embeddedConfigured()) status.textContent = "Email wallet sign-in is unavailable here.";
    } else if (brand === "metamask") {
      if (!matches.length) {
        const phone = choices().find(item => item.kind === "phone");
        if (phone) list.append(option("Connect MetaMask on my phone", "Open MetaMask on this phone, or scan from another device.", () => finish(phone)));
        else note("Open this bounty in MetaMask’s browser.");
      }
      note("Buy USDC on Base in MetaMask. Return here to approve your bounty with the same wallet.");
    }
  }

  function mount() {
    if (chooser) return;
    chooser = doc.createElement("dialog");
    chooser.className = "wallet-link-dialog";
    chooser.setAttribute("aria-labelledby", "wallet-link-title");
    chooser.innerHTML = '<header><h2 id="wallet-link-title" tabindex="-1">Choose your wallet</h2><button type="button" class="wallet-link-close" aria-label="Close wallet chooser">×</button></header><p data-wallet-intro></p><div data-wallet-choices></div><details data-wallet-create><summary>Use an email wallet instead</summary><p>This may be a different wallet from your Base app. Use your usual email or social sign-in to recover it.</p><div data-wallet-create-button></div></details><p class="wallet-link-status" role="status" aria-live="polite"></p><button type="button" class="wallet-choice" data-wallet-back hidden>Back to wallet choices</button><p class="wallet-link-note">You approve every payment in your wallet.</p>';
    list = chooser.querySelector("[data-wallet-choices]");
    status = chooser.querySelector("[role=status]");
    createButton = option("Use Coinbase with email", "Sign in with your usual email or social account. This can recover your wallet.", async () => {
      const request = pending;
      createButton.disabled = true;
      status.textContent = "Opening Coinbase sign-in…";
      try {
        const provider = await loadEmbedded();
        if (pending === request) finish({ provider, kind: "embedded", label: "Coinbase embedded wallet", capabilities: provider.agentBountiesCapabilities });
      } catch (error) {
        if (pending === request) status.textContent = error.message;
      } finally {
        if (pending === request) createButton.disabled = false;
      }
    });
    createButton.classList.add("wallet-choice-create");
    chooser.querySelector("[data-wallet-create-button]").append(createButton);
    chooser.querySelector("[data-wallet-back]").addEventListener("click", () => chooseBrand(null));
    chooser.querySelector(".wallet-link-close").addEventListener("click", () => finish());
    chooser.addEventListener("cancel", (event) => { event.preventDefault(); finish(); });
    chooser.addEventListener("close", () => { if (pending) finish(); });
    doc.body.append(chooser);
  }

  function select(options = {}) {
    if (pending) return pending.promise;
    mount();
    brand = null; continuation = options.prepareContinuation;
    status.textContent = "";
    createButton.disabled = !embeddedConfigured();
    render();
    const request = {};
    request.promise = new Promise((resolve, reject) => { request.resolve = resolve; request.reject = reject; });
    pending = request;
    chooser.showModal();
    chooser.querySelector("#wallet-link-title").focus();
    win.dispatchEvent(new win.Event("eip6963:requestProvider"));
    return request.promise;
  }

  // This tab-scoped intent only resumes the UI; it is never ownership evidence.
  const intentKey = "agentbounties:pending-embedded-account-link";
  function clearPending() {
    try { win.sessionStorage.removeItem(intentKey); } catch (_) { /* Storage may be disabled. */ }
  }
  function beginPending(userId, expectedAddress = null) {
    const address = /^0x[0-9a-f]{40}$/.test(String(expectedAddress || "").toLowerCase()) ? expectedAddress.toLowerCase() : null;
    try { win.sessionStorage.setItem(intentKey, JSON.stringify({ userId: String(userId), startedAt: Date.now(), expectedAddress: address })); } catch (_) { /* Same-page linking still works. */ }
  }
  function pendingIntent(userId) {
    try {
      const intent = JSON.parse(win.sessionStorage.getItem(intentKey));
      if (intent && intent.userId === String(userId) && Date.now() - intent.startedAt >= 0 && Date.now() - intent.startedAt < 30 * 60 * 1000
        && (intent.expectedAddress == null || /^0x[0-9a-f]{40}$/.test(intent.expectedAddress))) return intent;
    } catch (_) { /* Ignore expired or malformed intent. */ }
    clearPending();
    return null;
  }
  function hasPending(userId) { return Boolean(pendingIntent(userId)); }
  function pendingAddress(userId) { return pendingIntent(userId)?.expectedAddress || null; }

  // This adapter policy is known before loading/authenticating the SDK. Creation
  // needs direct transactions; supported sponsored relays remain separate.
  const embeddedCapabilities = Object.freeze({ directTransactions: false, postingTransactions: true, reviewedPostingOnly: true, chainIds: Object.freeze([8453]), transactionPolicy: "reviewed-base-posting" });
  return Object.freeze({ select, choices, loadEmbedded, embeddedCapabilities, beginPending, hasPending, pendingAddress, clearPending, cancel: () => finish() });
});
