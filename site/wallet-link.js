/* Account wallet selection. Discovery never requests accounts or signatures. */
"use strict";
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory;
  else root.AgentBountiesWalletLink = factory(root);
})(typeof window === "object" ? window : globalThis, function createWalletLink(win) {
  const doc = win.document;
  const discovered = new Map();
  let chooser, list, status, createButton, pending, bundlePromise;
  const cancelled = () => Object.assign(new Error("Wallet selection cancelled."), { code: 4001 });

  function remember(detail) {
    if (!detail?.provider || typeof detail.provider.request !== "function") return;
    const name = String(detail.info?.name || "Browser wallet").trim().slice(0, 64);
    discovered.set(detail.provider, { provider: detail.provider, label: name || "Browser wallet", kind: "browser" });
    if (chooser?.open) render();
  }
  win.addEventListener("eip6963:announceProvider", (event) => remember(event.detail));
  win.dispatchEvent(new win.Event("eip6963:requestProvider"));

  function choices() {
    const phone = win.AgentBountiesPhoneWallet;
    const result = [...discovered.values()].filter((item) => item.provider !== phone?.provider);
    const injected = Array.isArray(win.ethereum?.providers) && win.ethereum.providers.length
      ? win.ethereum.providers : [win.ethereum];
    for (const provider of injected) {
      if (!provider || provider === phone?.provider || typeof provider.request !== "function" || result.some((item) => item.provider === provider)) continue;
      result.push({ provider, kind: "browser", label: provider.isMetaMask ? "MetaMask" : provider.isCoinbaseWallet ? "Coinbase Wallet" : "Browser wallet" });
    }
    if (phone?.state().available) result.push({ provider: phone.provider, kind: "phone", label: "Use a phone wallet" });
    return result;
  }

  function embeddedConfigured() {
    return Boolean(win.AgentBountiesWalletConfig?.providers?.coinbaseEmbedded?.enabled);
  }

  function loadEmbedded() {
    if (!embeddedConfigured()) return Promise.reject(new Error("Wallet creation is not configured on this site yet."));
    if (win.AgentBountiesCoinbaseEmbeddedWallet?.enabled) return Promise.resolve(win.AgentBountiesCoinbaseEmbeddedWallet.provider);
    if (!bundlePromise) {
      bundlePromise = new Promise((resolve, reject) => {
        let failed = false;
        const stylesheet = doc.createElement("link");
        stylesheet.rel = "stylesheet";
        stylesheet.href = new URL("vendor/coinbase-embedded-wallet.bundle.css?v=2", doc.baseURI).href;
        const script = doc.createElement("script");
        script.src = new URL("vendor/coinbase-embedded-wallet.bundle.js?v=2", doc.baseURI).href;
        script.async = true;
        const fail = () => { failed = true; win.clearTimeout(timer); script.remove(); stylesheet.remove(); reject(new Error("Wallet creation could not load. Check your connection and try again.")); };
        const timer = win.setTimeout(fail, 20000);
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
    return bundlePromise;
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

  function render() {
    if (!list) return;
    list.replaceChildren();
    for (const choice of choices()) {
      list.append(option(choice.label, choice.kind === "phone" ? "Scan a QR code with your wallet app." : "Choose an address in this wallet.", () => finish(choice)));
    }
    if (!list.childElementCount) {
      const empty = doc.createElement("p");
      empty.textContent = "No browser wallet detected. You can create a wallet above.";
      list.append(empty);
    }
  }

  function mount() {
    if (chooser) return;
    chooser = doc.createElement("dialog");
    chooser.className = "wallet-link-dialog";
    chooser.setAttribute("aria-labelledby", "wallet-link-title");
    chooser.innerHTML = '<header><div><p class="wallet-link-eyebrow">Linked wallets</p><h2 id="wallet-link-title">Choose a wallet</h2></div><button type="button" class="wallet-link-close" aria-label="Close wallet chooser">×</button></header><p>Create a wallet or connect one you already use.</p><div data-wallet-create></div><h3>I already have a wallet</h3><div data-wallet-choices></div><p class="wallet-link-status" role="status"></p><p class="wallet-link-note">Linking proves ownership only. It cannot move funds or approve payments.</p>';
    list = chooser.querySelector("[data-wallet-choices]");
    status = chooser.querySelector("[role=status]");
    createButton = option("I don’t have a wallet — create one", "Use email or social sign-in. No extension or recovery phrase needed. Powered by Coinbase.", async () => {
      const request = pending;
      createButton.disabled = true;
      status.textContent = "Opening wallet creation…";
      try {
        const provider = await loadEmbedded();
        if (pending === request) finish({ provider, kind: "embedded", label: "Embedded wallet" });
      } catch (error) {
        if (pending === request) status.textContent = error.message;
      } finally {
        if (pending === request) createButton.disabled = false;
      }
    });
    createButton.classList.add("wallet-choice-create");
    chooser.querySelector("[data-wallet-create]").append(createButton);
    chooser.querySelector(".wallet-link-close").addEventListener("click", () => finish());
    chooser.addEventListener("cancel", (event) => { event.preventDefault(); finish(); });
    chooser.addEventListener("close", () => { if (pending) finish(); });
    doc.body.append(chooser);
  }

  function select() {
    if (pending) return pending.promise;
    mount();
    status.textContent = embeddedConfigured() ? "" : "Wallet creation is not configured on this site yet.";
    createButton.disabled = !embeddedConfigured();
    render();
    const request = {};
    request.promise = new Promise((resolve, reject) => { request.resolve = resolve; request.reject = reject; });
    pending = request;
    chooser.showModal();
    createButton.focus();
    win.dispatchEvent(new win.Event("eip6963:requestProvider"));
    return request.promise;
  }

  // This tab-scoped intent only resumes the UI; it is never ownership evidence.
  const intentKey = "agentbounties:pending-embedded-account-link";
  function clearPending() {
    try { win.sessionStorage.removeItem(intentKey); } catch (_) { /* Storage may be disabled. */ }
  }
  function beginPending(userId) {
    try { win.sessionStorage.setItem(intentKey, JSON.stringify({ userId: String(userId), startedAt: Date.now() })); } catch (_) { /* Same-page linking still works. */ }
  }
  function hasPending(userId) {
    try {
      const intent = JSON.parse(win.sessionStorage.getItem(intentKey));
      if (intent && intent.userId === String(userId) && Date.now() - intent.startedAt >= 0 && Date.now() - intent.startedAt < 30 * 60 * 1000) return true;
    } catch (_) { /* Ignore expired or malformed intent. */ }
    clearPending();
    return false;
  }

  return Object.freeze({ select, choices, loadEmbedded, beginPending, hasPending, clearPending, cancel: () => finish() });
});
