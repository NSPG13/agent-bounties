/* Optional advertising outcome measurement. No Google tag, wallet data or signing. */
(function (root, factory) {
  if (typeof module === "object" && module.exports) module.exports = factory;
  if (root?.document) root.AgentBountiesAdMeasurement = factory(root);
})(typeof window !== "undefined" ? window : null, function (win) {
  "use strict";
  const VERSION = "google-ads-outcomes-v1";
  const CONSENT = "agent-bounties.ads-consent.v1", STATE = "agent-bounties.ads-acquisition.v1";
  const REVOKE = "agent-bounties.ads-withdrawal.v1", HANDOFFS = "agent-bounties.ads-handoffs.v1";
  const TTL = 90 * 86400000;
  const api = "https://api.agentbounties.app/v1/distribution/";
  const safeRead = key => { try { return JSON.parse(win.localStorage.getItem(key) || "null"); } catch (_) { return null; } };
  const save = (key, value) => { try { if (value === null) win.localStorage.removeItem(key); else win.localStorage.setItem(key, JSON.stringify(value)); return true; } catch (_) { return false; } };
  const consent = () => { const value = safeRead(CONSENT); return value?.version === VERSION && value.expires_at > Date.now() ? value.choice : null; };
  const privacy = () => win.navigator.globalPrivacyControl === true || ["1", "yes"].includes(win.navigator.doNotTrack || win.doNotTrack)
    || new URL(win.location.href).searchParams.get("analytics") === "off" || safeRead("bountyboard.analytics.disabled.v1") === true;
  const validPair = value => value && /^aba1_[0-9a-f]{64}\.[0-9a-f]{64}$/.test(value.acquisition)
    && /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(value.handoff);
  let pending = null, acquisition = safeRead(STATE), flight = Promise.resolve(null), captureFlight = null, notice;
  const recordedStages = new Set();
  if (acquisition?.expires_at <= Date.now()) { acquisition = null; save(STATE, null); save(HANDOFFS, null); }
  const url = new URL(win.location.href);
  const ids = ["gclid", "gbraid", "wbraid"].filter(key => url.searchParams.has(key));
  const campaign = url.searchParams.get("utm_campaign") || url.searchParams.get("gad_campaignid")
    || ({"/bug-fix.html":"bug-fix","/app-testing.html":"app-testing","/api-integration.html":"api-integration"})[url.pathname];
  if (ids.length === 1 && url.searchParams.getAll(ids[0]).length === 1 && /^[a-zA-Z0-9._~-]{10,512}$/.test(url.searchParams.get(ids[0])) && /^[a-zA-Z0-9._-]{1,64}$/.test(campaign || "")) {
    const first = safeRead("bountyboard.analytics.attribution.v1");
    pending = { identifier_kind: ids[0], click_id: url.searchParams.get(ids[0]), campaign,
      first_touch_source: first?.expires_at > Date.now() ? first.source : "google-ads" };
    const original = url.searchParams.get("acquisition");
    if (!acquisition && /^aba1_[0-9a-f]{64}\.[0-9a-f]{64}$/.test(original || "")) acquisition = { acquisition: original, expires_at: Date.now() + TTL };
  }
  // Strip before analytics, shared links, login return URLs or agent prompts can read them.
  if (ids.length) {
    ["gclid", "gbraid", "wbraid"].forEach(key => url.searchParams.delete(key));
    win.history.replaceState(win.history.state, "", url.href);
  }
  function uuidNonce() { const bytes = new Uint8Array(32); win.crypto.getRandomValues(bytes); return Array.from(bytes, b => b.toString(16).padStart(2, "0")).join(""); }
  async function request(path, body) {
    const controller = new win.AbortController(), timeout = win.setTimeout(() => controller.abort(), 3500);
    try {
      const response = await win.fetch(api + path, { method: "POST", headers: { "content-type": "application/json" }, credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal, body: JSON.stringify(body) });
      if (!response.ok) throw new Error("Advertising measurement unavailable");
      return await response.json();
    } finally { win.clearTimeout(timeout); }
  }
  function status(message) {
    if (notice) { const line = notice.querySelector("[data-ad-status]"); if (line) line.textContent = message; }
    win.document.querySelectorAll("[data-ads-withdrawal-status]").forEach(line => { line.textContent = message; });
    win.dispatchEvent?.(new win.CustomEvent("agent-bounties:ad-measurement-status", { detail: { message } }));
  }
  async function retryWithdrawal() {
    const queued = safeRead(REVOKE);
    const values = Array.isArray(queued) ? queued : queued?.acquisition ? [queued.acquisition] : [];
    for (const value of values) {
      try {
        await request("website-consent/revoke", { acquisition: value });
        const remaining = (safeRead(REVOKE) || []).filter(item => item !== value);
        save(REVOKE, remaining.length ? remaining : null);
      } catch (_) { /* Keep this opaque deletion request, never a raw click. */ }
    }
    status(safeRead(REVOKE)?.length ? "Advertising measurement is off here. Server deletion is waiting for a connection; it will retry when you return." : "Advertising measurement is off. Any queued server deletion is complete.");
  }
  function queueWithdrawal(values) {
    const old = safeRead(REVOKE);
    const previous = Array.isArray(old) ? old : old?.acquisition ? [old.acquisition] : [];
    const all = [...new Set([...previous, ...values].filter(Boolean))];
    save(REVOKE, all.length ? all : null);
  }
  async function withdraw() {
    const previous = [acquisition?.acquisition, ...Object.values(safeRead(HANDOFFS) || {}).map(pair => pair.acquisition)];
    save(CONSENT, { version: VERSION, choice: "denied", expires_at: Date.now() + TTL });
    pending = null; acquisition = null; save(STATE, null); save(HANDOFFS, null);
    queueWithdrawal(previous);
    await retryWithdrawal();
  }
  async function captureOnce() {
    if (!pending || consent() !== "granted" || privacy()) return null;
    const data = pending;
    if (!data.nonce) data.nonce = uuidNonce();
    try {
      const result = await request("website-acquisitions", { ...data, acquisition: acquisition?.acquisition || null, consent_version: VERSION, consent_granted: true });
      if (consent() !== "granted" || privacy()) {
        queueWithdrawal([result.acquisition]); await retryWithdrawal(); return null;
      }
      acquisition = { acquisition: result.acquisition, expires_at: Math.min(Date.parse(result.expires_at), Date.now() + TTL) };
      save(STATE, acquisition); pending = null; status("Advertising measurement allowed. You can turn it off in Privacy settings.");
      return acquisition;
    } catch (_) { status("Advertising measurement could not connect. You can still post your bounty."); return null; }
  }
  function capture() {
    if (!captureFlight) captureFlight = captureOnce().finally(() => { captureFlight = null; });
    return captureFlight;
  }
  async function allow() {
    if (privacy()) return null;
    if (!save(CONSENT, { version: VERSION, choice: "granted", expires_at: Date.now() + TTL })) { status("This browser cannot save your choice. Advertising measurement remains off."); return null; }
    flight = capture();
    const result = await flight;
    if (result && notice) notice.hidden = true;
    return result;
  }
  function current(operation) {
    if (privacy() || consent() === "denied") return null;
    const pair = safeRead(HANDOFFS)?.[operation]; return validPair(pair) ? pair : null;
  }
  function restore(operation, pair) {
    if (!validPair(pair) || privacy() || consent() === "denied") return;
    const all = safeRead(HANDOFFS) || {}; all[operation] = { acquisition: pair.acquisition, handoff: pair.handoff };
    save(HANDOFFS, Object.fromEntries(Object.entries(all).slice(-20)));
  }
  async function prepare(operation, stage = "draft") {
    if (!operation || privacy() || consent() === "denied") return null;
    await flight;
    if (pending && consent() === "granted") await capture();
    const pair = current(operation), value = pair?.acquisition || acquisition?.acquisition;
    if (!value) return null;
    const key = `${value}:${operation}:${stage}`;
    if (pair && recordedStages.has(key)) return pair;
    try {
      const result = await request("website-handoffs", { acquisition: value, operation_id: operation, stage });
      if (privacy() || consent() === "denied") return null;
      restore(operation, result); recordedStages.add(key); return current(operation);
    } catch (_) { status("Advertising measurement is unavailable; bounty posting remains available."); return pair; }
  }
  function showChoice() {
    win.document.querySelectorAll("[data-ads-measurement-withdraw]").forEach(button => button.addEventListener("click", withdraw));
    win.document.querySelectorAll("[data-analytics-opt-out]").forEach(button => button.addEventListener("click", withdraw));
    if (!pending || privacy() || consent()) return;
    notice = win.document.createElement("section"); notice.setAttribute("aria-label", "Optional advertising measurement"); notice.className = "ad-measurement-choice";
    const text = win.document.createElement("p"); text.textContent = "Allow advertising measurement? We can tell Google whether this ad visit leads to a funded bounty or verified payment. No wallet address, task content or payment amount is sent. This is separate from Google Analytics and does not enable personalized ads.";
    const yes = win.document.createElement("button"); yes.type = "button"; yes.textContent = "Allow ad measurement"; yes.addEventListener("click", allow);
    const no = win.document.createElement("button"); no.type = "button"; no.textContent = "No thanks"; no.addEventListener("click", () => { void withdraw(); notice.hidden = true; });
    const line = win.document.createElement("p"); line.setAttribute("data-ad-status", ""); line.setAttribute("role", "status");
    const privacyLink = win.document.createElement("a"); privacyLink.href = "/privacy.html"; privacyLink.textContent = "Privacy details";
    notice.append(text, yes, no, privacyLink, line); win.document.body.appendChild(notice);
  }
  if (privacy()) { pending = null; void withdraw(); }
  else { void retryWithdrawal(); if (consent() === "granted") flight = capture(); }
  if (win.document.readyState === "loading") win.document.addEventListener("DOMContentLoaded", showChoice, { once: true }); else showChoice();
  win.addEventListener?.("online", retryWithdrawal);
  win.addEventListener?.("agent-bounties:journey", event => {
    if (event.detail?.role === "post" && !current(event.detail.id)) void prepare(event.detail.id, "draft");
  });
  win.document.addEventListener("click", event => {
    const link = event.target.closest?.("[data-buyer-post]");
    if (link && pending && consent() === "granted") {
      event.preventDefault(); void capture().finally(() => { win.location.href = link.href; });
    }
  });
  return { allow, withdraw, prepare, current, restore, consent };
});
