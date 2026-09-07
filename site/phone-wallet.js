/* Shared WalletConnect transport. Signing remains in the existing human reviews. */
"use strict";
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory;
  else root.AgentBountiesPhoneWallet = factory(root);
})(typeof window === "object" ? window : globalThis, function createPhoneWallet(win, dependencies = {}) {
  const BASE = 8453, CHAIN = "0x2105", ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const PROJECT = /^[a-f0-9]{32}$/i;
  const MARKER = "agent-bounties-phone-connected-v1";
  const optionalMethods = ["eth_sendTransaction", "personal_sign", "eth_signTypedData_v4", "wallet_switchEthereumChain", "wallet_sendCalls", "wallet_getCallsStatus", "wallet_getCapabilities", "wallet_watchAsset"];
  const reads = new Set(["eth_chainId", "eth_accounts", "eth_call", "eth_getBalance", "eth_getCode", "eth_getTransactionReceipt", "eth_blockNumber", "eth_getBlockByNumber", "eth_getLogs", "eth_estimateGas", "eth_gasPrice"]);
  const listeners = new Map();
  const doc = win.document;
  let sdk, sdkPromise, vendorPromise, attempt, generation = 0, activePrefix, accounts = [], chain = CHAIN;
  let phase = "disconnected", message = "Scan with your phone wallet. Approve the connection on your phone.", dialog, qr, statusNode, retry, disconnectButton, launcher;
  const projectId = String(win.agentBountiesPhoneWalletConfig?.projectId || "");
  const configured = PROJECT.test(projectId);
  function error(code, message) { return Object.assign(new Error(message), { code }); }
  function chainIdHex(value) {
    // WalletConnect 2.24 returns a number for eth_chainId. EIP-1193 consumers
    // expect a hexadecimal string; preserve the actual chain, never assume Base.
    const valid = typeof value === "number" ? Number.isSafeInteger(value) : typeof value === "string" && /^(?:0x[0-9a-f]+|[0-9]+)$/i.test(value);
    if (!valid || BigInt(value) <= 0n) throw error(4901, "Your phone wallet returned an invalid network. Reconnect it before continuing.");
    return `0x${BigInt(value).toString(16)}`;
  }
  function emit(name, value) { for (const listener of listeners.get(name) || []) listener(value); }
  function marker(value) { try { if (value) win.localStorage.setItem(MARKER, activePrefix); else win.localStorage.removeItem(MARKER); } catch (_) { /* Session remains usable in this tab. */ } }
  function remembered() { try { const value = win.localStorage.getItem(MARKER); return /^ab-phone-[a-f0-9-]{36}$/.test(value || "") ? value : null; } catch (_) { return null; } }
  function baseNamespaces(provider) { return Object.entries(provider?.session?.namespaces || {}).filter(([key]) => key === "eip155" || key === `eip155:${BASE}`).map(([, value]) => value); }
  function liveAccounts(provider) {
    if (!provider?.session || !Number.isFinite(Number(provider.session.expiry)) || Number(provider.session.expiry) <= Date.now() / 1000) return [];
    const approved = baseNamespaces(provider).flatMap((value) => value.accounts || []);
    return (provider.accounts || []).filter((address) => ADDRESS.test(address) && approved.some((entry) => entry.toLowerCase() === `eip155:${BASE}:${address.toLowerCase()}`));
  }
  function state() {
    if (accounts.length && !liveAccounts(sdk).length) { accounts = []; phase = "disconnected"; message = "Your phone session ended. Reconnect when ready; your draft is saved."; marker(false); }
    return { available: configured, status: phase, connected: accounts.length > 0 && phase === "connected", address: accounts[0] || null,
      chain_id: accounts.length ? chain : null, review_open: Boolean(dialog?.open), message,
      next_action: !configured ? "Phone pairing is unavailable here. Continue preparing the saved journey or use a browser wallet." : phase === "connecting" ? "Wait for the QR code to appear; no additional permission is needed to prepare it." : phase === "pairing" ? "Scan the QR code with your wallet app and approve the connection on your phone." : phase === "connected" ? "Continue the prepared review. Approve each signature or transaction on your phone." : "Open phone-wallet pairing.",
      connection_approval_required: phase !== "connected", payment_authorized: false };
  }
  function render() {
    const snapshot = state();
    if (statusNode) statusNode.textContent = message;
    if (launcher) launcher.textContent = phase === "connected" ? `Phone wallet ${accounts[0].slice(0, 6)}…${accounts[0].slice(-4)}` : "Connect phone wallet";
    if (retry) { retry.hidden = !configured || phase === "connected"; retry.disabled = Boolean(attempt); retry.textContent = phase === "pairing" || phase === "connecting" ? "Waiting for your phone…" : "Show a new QR code"; }
    if (disconnectButton) disconnectButton.hidden = !accounts.length;
    win.dispatchEvent(new win.CustomEvent("agent-bounties:phone-wallet-state", { detail: snapshot }));
  }
  function clearQr() { if (qr) { qr.hidden = true; qr.removeAttribute("src"); } }
  function setState(next, text) { phase = next; message = text; render(); }
  function clearConnection(text = "Phone wallet disconnected. Your draft and progress are saved.") {
    accounts = []; marker(false); clearQr(); setState("disconnected", text); emit("accountsChanged", []); emit("disconnect", { code: 4900, message: "Phone wallet disconnected." });
  }
  async function vendor() {
    if (!vendorPromise) vendorPromise = (dependencies.loadVendor ? dependencies.loadVendor() : import("./vendor/phone-wallet.bundle.js?v=2")).catch(() => { vendorPromise = null; throw error(4900, "Phone pairing could not load. Check your connection and try again; your draft is saved."); });
    return vendorPromise;
  }
  async function initialize() {
    if (!configured) throw error(4900, "Phone-wallet pairing is not configured on this site yet. You can continue preparing your work or use a browser wallet.");
    if (!sdkPromise) {
      const version = generation, prefix = remembered() || `ab-phone-${win.crypto.randomUUID()}`;
      sdkPromise = (async () => {
        const library = await vendor();
        const provider = await library.createProvider({ projectId, chains: [BASE], optionalChains: [BASE], optionalMethods, showQrModal: false, telemetryEnabled: false, logger: "silent",
          rpcMap: { [BASE]: "https://mainnet.base.org" }, customStoragePrefix: prefix,
          metadata: { name: "Agent Bounties", description: "Review marketplace work. Approve each signature and payment on your phone.", url: win.location.origin, icons: [new URL("/favicon.svg", win.location.origin).href] } });
        if (version === generation) { sdk = provider; activePrefix = prefix; }
        provider.on("display_uri", async (uri) => {
          const current = attempt;
          if (sdk !== provider || !current || current.cancelled) return;
          try {
            const params = new URLSearchParams(String(uri).split("?")[1] || "");
            if (!/^wc:[a-f0-9]{64}@2\?/i.test(uri) || !/^[a-f0-9]{64}$/i.test(params.get("symKey") || "") || String(uri).length > 2048) throw new Error("Invalid pairing code");
            const image = await library.qrDataUrl(uri);
            if (attempt !== current || current.cancelled) return;
            qr.src = image; qr.hidden = false;
            setState("pairing", "1. Open your phone wallet’s scanner. 2. Scan this code. 3. Check agentbounties.app and approve the connection. This does not authorize a payment.");
          } catch (_) { cancel("Phone pairing could not create a QR code. Try again.", "error"); }
        });
        provider.on("accountsChanged", () => {
          if (sdk !== provider || attempt) return; // A session is usable only after connect() resolves.
          accounts = liveAccounts(provider);
          if (!accounts.length) clearConnection();
          else { chain = `0x${Number(provider.chainId).toString(16)}`; marker(true); setState("connected", "Your phone wallet is connected. Review the selected address before continuing."); emit("accountsChanged", accounts.slice()); }
        });
        provider.on("chainChanged", (value) => { if (sdk === provider) { chain = String(value); render(); emit("chainChanged", value); } });
        provider.on("disconnect", () => { if (sdk !== provider) return; if (attempt) cancel("Your phone wallet ended the connection. Show a new QR code to reconnect."); clearConnection(); });
        return provider;
      })().catch(() => { if (version === generation) sdkPromise = null; throw error(4900, "Phone pairing could not reach the wallet relay. Check your connection and try again; your draft is saved."); });
    }
    return sdkPromise;
  }
  function buildDialog() {
    if (dialog) { if (!dialog.open) dialog.showModal(); return; }
    const node = (tag, text, className) => { const element = doc.createElement(tag); if (text) element.textContent = text; if (className) element.className = className; return element; };
    dialog = node("dialog", "", "ab-phone-dialog"); dialog.setAttribute("aria-labelledby", "ab-phone-title");
    const close = node("button", "Close", "ab-phone-close"); close.type = "button";
    close.addEventListener("click", () => { cancel(); dialog.close(); });
    const title = node("h2", "Connect your phone wallet"); title.id = "ab-phone-title";
    statusNode = node("p", message); statusNode.setAttribute("role", "status"); statusNode.setAttribute("aria-live", "polite");
    qr = node("img", "", "ab-phone-qr"); qr.alt = "Scan this QR code with your phone wallet’s scanner"; qr.hidden = true; qr.width = 290; qr.height = 290;
    const note = node("p", "Use a WalletConnect-compatible wallet that supports Base, such as MetaMask. Keep this page open. Never share the QR code or enter a recovery phrase here.", "ab-phone-note");
    const actions = node("div", "", "ab-phone-actions");
    retry = node("button", "Show a new QR code"); retry.type = "button"; retry.addEventListener("click", () => { void begin().catch(() => {}); });
    disconnectButton = node("button", "Disconnect phone wallet"); disconnectButton.type = "button"; disconnectButton.hidden = true;
    disconnectButton.addEventListener("click", () => { void disconnect(); });
    actions.append(retry, disconnectButton); dialog.append(close, title, statusNode, qr, note, actions); doc.body.append(dialog);
    dialog.addEventListener("cancel", () => cancel()); dialog.addEventListener("close", () => { clearQr(); render(); });
    dialog.showModal(); render();
  }
  function cancel(text = "Pairing cancelled. Your draft and progress are saved.", next = "cancelled") {
    if (!attempt) return;
    const current = attempt; current.cancelled = true; attempt = null; win.clearTimeout(current.timer); clearQr();
    current.reject(error(4001, text));
    const abandoned = sdk; generation++; sdk = null; sdkPromise = null;
    void abandoned?.signer.cleanupPendingPairings({ deletePairings: true }).catch(() => {});
    // The SDK may still receive a late phone approval. The attempt identity below
    // rejects and disconnects it. Each retry gets isolated SDK storage, so a
    // delayed approval cannot disconnect or replace the new phone session.
    setState(next, text);
  }
  async function restore() {
    if (phase === "disconnecting" || (phase === "error" && accounts.length)) return [];
    if (!configured || !remembered()) return [];
    const provider = await initialize();
    if (attempt || provider !== sdk) return [];
    accounts = liveAccounts(provider);
    if (accounts.length) { chain = `0x${Number(provider.chainId).toString(16)}`; setState("connected", "Your phone wallet is connected. Approvals stay on your phone."); }
    else marker(false);
    return accounts.slice();
  }
  function begin() {
    buildDialog();
    if (phase === "disconnecting") return Promise.reject(error(4900, "Wait for the phone wallet to disconnect."));
    if (state().connected) return Promise.resolve(accounts.slice());
    if (!configured) { setState("unavailable", "Phone-wallet pairing is not configured here yet. Continue preparing your work or use a browser wallet."); return Promise.reject(error(4900, message)); }
    if (attempt) return attempt.promise;
    const current = { cancelled: false };
    current.promise = new Promise((resolve, reject) => { current.resolve = resolve; current.reject = reject; });
    attempt = current; clearQr(); setState("connecting", "Preparing a secure QR code…");
    current.timer = win.setTimeout(() => { if (attempt === current) cancel("This QR code expired. Show a new code when your phone is ready.", "expired"); }, 300000);
    void (async () => {
      try {
        const provider = await initialize();
        if (current.cancelled) { await provider.signer.cleanupPendingPairings({ deletePairings: true }); return; }
        if (!liveAccounts(provider).length) await provider.connect();
        if (current.cancelled || attempt !== current) { await provider.disconnect().catch(() => {}); return; }
        accounts = liveAccounts(provider);
        if (!accounts.length) { await provider.disconnect().catch(() => {}); throw error(4901, "This wallet did not approve a Base account. Connect a wallet that supports Base."); }
        chain = `0x${Number(provider.chainId).toString(16)}`; attempt = null; win.clearTimeout(current.timer); clearQr(); marker(true);
        setState("connected", "Phone wallet connected. Return to your prepared review; approve each signature or payment on your phone.");
        emit("connect", { chainId: chain }); emit("accountsChanged", accounts.slice()); current.resolve(accounts.slice());
        if (dialog.open) dialog.close();
      } catch (caught) {
        if (attempt !== current) return;
        attempt = null; win.clearTimeout(current.timer); clearQr();
        // EthereumProvider 2.24 wraps connect rejections in Error(message),
        // dropping the original code. Match its exact consent-rejection texts
        // only here; transaction errors below are never reclassified.
        const rejectionText = String(caught?.message || "").trim().toLowerCase().replace(/\.$/, "");
        const rejected = [4001, 5000, 5001, 5002, 5003].includes(Number(caught?.code)) ||
          ["user rejected", "user rejected chains", "user rejected methods", "user rejected events", "user rejected the request"].includes(rejectionText);
        const failure = error(rejected ? 4001 : 4900, rejected ? "Connection declined on your phone. Your draft is saved; reconnect when ready." : "Phone pairing could not connect. Check your wallet and network, then show a new QR code. Your draft is saved.");
        setState(rejected ? "cancelled" : "error", failure.message); current.reject(failure);
      }
    })();
    return current.promise;
  }
  async function openReview() { buildDialog(); void begin().catch(() => {}); return state(); }
  async function disconnect() {
    cancel();
    setState("disconnecting", "Disconnecting your phone wallet…");
    try { if (sdk?.session) await sdk.disconnect(); clearConnection(); }
    catch (_) { setState("error", "The relay could not confirm disconnection. Disconnect Agent Bounties inside your phone wallet, then retry here."); }
    return state();
  }
  const provider = Object.freeze({
    on(name, listener) { if (!listeners.has(name)) listeners.set(name, new Set()); listeners.get(name).add(listener); return provider; },
    removeListener(name, listener) { listeners.get(name)?.delete(listener); return provider; },
    async request(request) {
      if (request.method === "eth_requestAccounts") { const restored = await restore(); return restored.length ? restored : begin(); }
      if (request.method === "eth_accounts") return (accounts.length ? liveAccounts(sdk) : await restore()).slice();
      if (!sdk || !liveAccounts(sdk).length || phase !== "connected") throw error(4900, "Connect your phone wallet before continuing this review.");
      if (!reads.has(request.method) && !optionalMethods.includes(request.method)) throw error(4200, "This phone connection does not support that wallet method.");
      if (!reads.has(request.method)) {
        const approved = baseNamespaces(sdk).flatMap((value) => value.methods || []);
        if (!approved.includes(request.method)) throw error(4200, "This phone wallet did not approve that method. Use another supported wallet or the available transaction flow.");
      }
      if (request.method === "wallet_switchEthereumChain" && request.params?.[0]?.chainId !== CHAIN) throw error(4901, "This connection is for Base only.");
      if (Number(sdk.chainId) !== BASE && !["eth_chainId", "wallet_switchEthereumChain"].includes(request.method)) throw error(4901, "Switch your phone wallet to Base before continuing.");
      if (["eth_sendTransaction", "wallet_sendCalls", "personal_sign", "eth_signTypedData_v4"].includes(request.method)) {
        const from = request.method === "personal_sign" ? request.params?.[1] : request.method === "eth_signTypedData_v4" ? request.params?.[0] : request.params?.[0]?.from;
        if (!ADDRESS.test(from || "") || String(from).toLowerCase() !== liveAccounts(sdk)[0]?.toLowerCase()) throw error(4100, "The selected phone account changed. Reconnect and review the exact request before signing.");
      }
      // Preserve rejection, unsupported-method, and uncertain-response errors.
      // The existing payment journals decide whether a retry is permissible.
      try {
        const result = await sdk.request(request);
        return request.method === "eth_chainId" ? chainIdHex(result) : result;
      }
      catch (failure) {
        // Safe read failures can explain recovery without exposing browser
        // internals. Financial errors stay intact for the payment journal.
        if (reads.has(request.method) && failure?.name === "InvalidStateError") throw error(4900, "Your browser closed the phone-wallet connection. Refresh this page and reconnect; your draft is saved. Check any pending wallet request before trying it again.");
        throw failure;
      }
    },
  });
  const announce = () => { if (configured) win.dispatchEvent(new win.CustomEvent("eip6963:announceProvider", { detail: Object.freeze({
    info: Object.freeze({ uuid: "c1ae1723-39a9-4b06-843c-c4c3ad0967a6", name: "Phone wallet (QR)", rdns: "app.agentbounties.phone", icon: "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 40 40'><rect x='10' y='3' width='20' height='34' rx='4' fill='%23174d43'/><path d='M16 31h8' stroke='white' stroke-width='2'/></svg>" }), provider,
  }) })); };
  win.addEventListener("eip6963:requestProvider", announce); announce();
  if (doc?.body) {
    launcher = doc.createElement("button"); launcher.type = "button"; launcher.className = "ab-phone-launcher"; launcher.hidden = !configured;
    launcher.addEventListener("click", () => { void openReview(); }); doc.body.append(launcher); render();
  }
  win.addEventListener("pagehide", () => { cancel(); clearQr(); });
  return Object.freeze({ provider, state, openReview, restore, disconnect });
});
