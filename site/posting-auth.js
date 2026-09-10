(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.AgentBountiesPostingAuth = api;
  if (root?.document) api.bind(root);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";

  const INTENT_KEY = "agentbounties:post-auth-return:v1";
  const RECEIPT_KEY = "agentbounties:post-auth-receipt:v1";
  const MAX_TARGET_LENGTH = 65_536;
  const INTENT_MAX_AGE_MS = 4 * 60 * 60 * 1000;
  const RECEIPT_MAX_AGE_MS = 5 * 60 * 1000;
  const WALLET_KINDS = new Set(["browser", "phone", "embedded", "linked"]);

  function safePostTarget(win, candidate) {
    if (!win?.location?.origin || typeof candidate !== "string" || !candidate || candidate.length > MAX_TARGET_LENGTH) return null;
    try {
      const target = new URL(candidate, win.location.href);
      if (target.origin !== win.location.origin || target.pathname !== "/post.html"
        || target.username || target.password || target.href.length > MAX_TARGET_LENGTH) return null;
      return target.href;
    } catch (_) {
      return null;
    }
  }

  function read(win, key) {
    try { return JSON.parse(win.sessionStorage.getItem(key)); }
    catch (_) { return null; }
  }

  function remove(win, key) {
    try { win.sessionStorage.removeItem(key); }
    catch (_) { /* A storage-disabled browser can still use the current page. */ }
  }

  function remember(win, candidate = win?.location?.href, now = Date.now()) {
    const target = safePostTarget(win, candidate);
    if (!target) return null;
    const intent = { schema: "agent-bounties/post-auth-return-v1", target, started_at: now };
    try { win.sessionStorage.setItem(INTENT_KEY, JSON.stringify(intent)); }
    catch (_) { return null; }
    return target;
  }

  function pending(win, now = Date.now()) {
    const intent = read(win, INTENT_KEY);
    const age = Number(intent?.started_at) <= now ? now - Number(intent?.started_at) : -1;
    const target = intent?.schema === "agent-bounties/post-auth-return-v1"
      && Number.isFinite(age) && age >= 0 && age < INTENT_MAX_AGE_MS
      ? safePostTarget(win, intent.target) : null;
    if (!target) remove(win, INTENT_KEY);
    return target;
  }

  function loginUrl(win) {
    const target = new URL("/", win.location.origin);
    target.searchParams.set("postReturn", "1");
    target.hash = "login";
    return target.href;
  }

  function begin(win, candidate = win?.location?.href) {
    const target = remember(win, candidate);
    if (!target) throw new Error("This browser could not preserve the prepared bounty. Keep this tab open and retry.");
    win.location.assign(loginUrl(win));
    return target;
  }

  function complete(win, details = {}, now = Date.now()) {
    const target = pending(win, now);
    if (!target) return false;
    const rawAddress = String(details.address || "").toLowerCase();
    const address = /^0x[0-9a-f]{40}$/.test(rawAddress) ? rawAddress : null;
    const kind = WALLET_KINDS.has(details.walletKind) ? details.walletKind : "linked";
    try {
      win.sessionStorage.setItem(RECEIPT_KEY, JSON.stringify({
        schema: "agent-bounties/post-auth-receipt-v1",
        completed_at: now,
        wallet_kind: kind,
        wallet_address: address,
      }));
    } catch (_) { /* The destination still verifies the server session. */ }
    remove(win, INTENT_KEY);
    if (typeof win.location.replace === "function") win.location.replace(target);
    else win.location.assign(target);
    return true;
  }

  function consumeReceipt(win, now = Date.now()) {
    const value = read(win, RECEIPT_KEY);
    remove(win, RECEIPT_KEY);
    const age = Number(value?.completed_at) <= now ? now - Number(value?.completed_at) : -1;
    if (value?.schema !== "agent-bounties/post-auth-receipt-v1"
      || !Number.isFinite(age) || age < 0 || age >= RECEIPT_MAX_AGE_MS
      || !WALLET_KINDS.has(value.wallet_kind)
      || (value.wallet_address !== null && !/^0x[0-9a-f]{40}$/.test(value.wallet_address))) return null;
    return value;
  }

  function bind(win) {
    for (const control of win.document.querySelectorAll("[data-post-auth-start]")) {
      control.addEventListener("click", (event) => {
        if (control.dataset.authenticated === "true") { remember(win); return; }
        event.preventDefault();
        try { begin(win); }
        catch (error) {
          const status = win.document.querySelector("[data-composer-status]");
          if (status) { status.textContent = error.message; status.dataset.tone = "error"; }
        }
      });
    }
  }

  return Object.freeze({
    INTENT_KEY,
    RECEIPT_KEY,
    safePostTarget,
    remember,
    pending,
    loginUrl,
    begin,
    complete,
    consumeReceipt,
    bind,
  });
});
