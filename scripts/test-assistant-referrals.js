"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const crypto = require("node:crypto");
const assert = require("node:assert/strict");
const { test } = require("node:test");
const home = require("../site/solarpunk-home.js");
const postingPrompt = require("../site/posting-prompt.js");
const source = fs.readFileSync(path.join(__dirname, "../site/analytics.js"), "utf8");

function storage() {
  const values = new Map(), writes = [];
  return { writes, getItem: key => values.get(key) ?? null,
    setItem(key, value) { writes.push(["set", key, String(value)]); values.set(key, String(value)); },
    removeItem(key) { writes.push(["remove", key]); values.delete(key); } };
}

function page(url, options = {}) {
  const events = [];
  const windowListeners = new Map();
  const local = options.local || storage();
  const window = {
    agentBountiesAnalyticsConfig: { googleMeasurementId: options.measurementId || "" },
    location: new URL(url), localStorage: local, sessionStorage: storage(),
    crypto: crypto.webcrypto,
    addEventListener(name, callback) {
      const callbacks = windowListeners.get(name) || [];
      callbacks.push(callback);
      windowListeners.set(name, callbacks);
    },
    fetch(endpoint, request) {
      assert.equal(endpoint, "https://api.agentizable.ai/v1/analytics/events");
      events.push(JSON.parse(request.body));
      return Promise.resolve({ ok: true });
    },
  };
  if (options.noLocalStorage) {
    Object.defineProperty(window, "localStorage", { get() { throw new Error("Storage unavailable"); } });
  }
  vm.runInNewContext(source, {
    window, navigator: options.privacy || {}, URL, URLSearchParams, Date, Uint8Array,
    document: options.document || { referrer: "", addEventListener() {} },
  });
  return { analytics: window.agentBountiesAnalytics, window, local, events,
    emit(name, event = {}) { for (const callback of windowListeners.get(name) || []) callback(event); } };
}

const landing = "https://agentizable.ai/?utm_source=example-source&utm_campaign=example-campaign&secret=never-forward&gclid=not-collected";
const review = "https://agentizable.ai/post.html?from=webmcp";

function privacyControls({ mounted = true, loading = false } = {}) {
  const listeners = new Map();
  const control = { textContent: "Disable analytics on this browser", disabled: true,
    attributes: new Map([["aria-pressed", "false"]]),
    setAttribute(name, value) { this.attributes.set(name, value); } };
  const document = { referrer: "", readyState: loading ? "loading" : "complete",
    querySelector(selector) { return mounted && selector === "[data-analytics-opt-out]" ? control : null; },
    addEventListener(name, callback) {
      const callbacks = listeners.get(name) || [];
      callbacks.push(callback);
      listeners.set(name, callbacks);
    } };
  return { document, control,
    click() {
      assert.equal(control.disabled, false, "only an enabled control can be clicked");
      const event = { target: { closest(selector) { return selector === "[data-analytics-opt-out]" ? control : null; } } };
      for (const callback of listeners.get("click") || []) callback(event);
    },
    ready() {
      mounted = true; document.readyState = "complete";
      for (const callback of listeners.get("DOMContentLoaded") || []) callback();
    } };
}

test("privacy control reflects a saved opt-out without changing consent or sending events", () => {
  const local = storage(), ui = privacyControls();
  local.setItem("bountyboard.analytics.disabled.v1", "true");
  local.setItem("agent-bounties.analytics.google-consent.v1", "granted");
  const result = page("https://agentizable.ai/privacy.html", { local, document: ui.document });
  assert.equal(ui.control.textContent, "Analytics disabled on this browser");
  assert.equal(ui.control.attributes.get("aria-pressed"), "true");
  assert.equal(ui.control.disabled, true);
  assert.equal(local.getItem("bountyboard.analytics.disabled.v1"), "true");
  assert.equal(local.getItem("agent-bounties.analytics.google-consent.v1"), "granted");
  assert.equal(result.events.length, 0);
});

test("browser privacy signals show why collection is off and never save a new choice", () => {
  for (const privacy of [{ globalPrivacyControl: true }, { doNotTrack: "1" }, { doNotTrack: "yes" }, { msDoNotTrack: "1" }]) {
    for (const query of ["", "?analytics=off"]) {
      const local = storage(), ui = privacyControls();
      const result = page("https://agentizable.ai/privacy.html" + query, { privacy, local, document: ui.document });
      assert.equal(ui.control.textContent, "Analytics is off because of your browser settings");
      assert.equal(ui.control.attributes.get("aria-pressed"), "true");
      assert.equal(ui.control.disabled, true);
      assert.equal(local.getItem("bountyboard.analytics.disabled.v1"), null, "display must not persist a choice");
      assert.equal(local.getItem("agent-bounties.analytics.google-consent.v1"), null);
      assert.equal(result.events.length, 0);
    }
  }
});

test("an allowed visitor can opt out and the saved disabled state returns on the next page", () => {
  const ui = privacyControls();
  const result = page("https://agentizable.ai/privacy.html", { document: ui.document });
  assert.equal(ui.control.textContent, "Disable analytics on this browser");
  assert.equal(ui.control.attributes.get("aria-pressed"), "false");
  assert.equal(ui.control.disabled, false);
  assert.equal(result.events.length, 1, "existing page-view behavior stays unchanged");
  ui.click();
  assert.equal(ui.control.textContent, "Analytics disabled on this browser");
  assert.equal(ui.control.attributes.get("aria-pressed"), "true");
  assert.equal(ui.control.disabled, true);
  assert.equal(result.local.getItem("bountyboard.analytics.disabled.v1"), "true");
  assert.equal(result.local.getItem("agent-bounties.analytics.google-consent.v1"), "denied");
  assert.equal(result.events.length, 1, "updating the control must not send an event");
  assert.equal(result.analytics.track("market_view"), false);
  const nextUi = privacyControls();
  const next = page("https://agentizable.ai/privacy.html", { local: result.local, document: nextUi.document });
  assert.equal(nextUi.control.disabled, true);
  assert.equal(nextUi.control.attributes.get("aria-pressed"), "true");
  assert.equal(next.events.length, 0);
});

test("head-loaded controls initialize after DOM readiness without opting in", () => {
  const ui = privacyControls({ mounted: false, loading: true }), local = storage();
  local.setItem("bountyboard.analytics.disabled.v1", "true");
  const result = page("https://agentizable.ai/privacy.html", { local, document: ui.document });
  ui.ready();
  assert.equal(ui.control.textContent, "Analytics disabled on this browser");
  assert.equal(ui.control.attributes.get("aria-pressed"), "true");
  assert.equal(ui.control.disabled, true);
  assert.equal(result.events.length, 0);
});

test("focus, restored pages and other-tab preferences refresh without writing choices or sending events", () => {
  const privacy = {}, ui = privacyControls();
  const result = page("https://agentizable.ai/privacy.html", { privacy, document: ui.document });
  privacy.globalPrivacyControl = true;
  result.emit("focus");
  assert.equal(ui.control.textContent, "Analytics is off because of your browser settings");
  assert.equal(ui.control.disabled, true);
  assert.equal(result.local.getItem("bountyboard.analytics.disabled.v1"), null);
  privacy.globalPrivacyControl = false;
  result.emit("pageshow");
  assert.equal(ui.control.disabled, false);
  result.local.setItem("bountyboard.analytics.disabled.v1", "true");
  result.emit("storage", { key: "bountyboard.analytics.disabled.v1" });
  assert.equal(ui.control.textContent, "Analytics disabled on this browser");
  assert.equal(ui.control.disabled, true);
  result.local.removeItem("bountyboard.analytics.disabled.v1");
  result.emit("storage", { key: null });
  assert.equal(ui.control.disabled, false);
  assert.equal(ui.control.attributes.get("aria-pressed"), "false");
  assert.equal(result.events.length, 1, "none of the UI refreshes starts collection");
  assert.equal(result.local.getItem("agent-bounties.analytics.google-consent.v1"), null);
});

test("unavailable local storage leaves the privacy control unavailable and collection off", () => {
  const ui = privacyControls();
  const result = page("https://agentizable.ai/privacy.html", { document: ui.document, noLocalStorage: true });
  assert.equal(ui.control.textContent, "Analytics controls are unavailable");
  assert.equal(ui.control.disabled, true);
  assert.equal(result.events.length, 0);
});


const optOutKey = "bountyboard.analytics.disabled.v1";
const googleConsentKey = "agent-bounties.analytics.google-consent.v1";
const measurementId = "G-TEST123";

function activeGooglePage({ controls = false, privacy = {} } = {}) {
  const ui = privacyControls({ mounted: controls }), local = storage(), scripts = [];
  local.setItem(googleConsentKey, "granted");
  const query = ui.document.querySelector;
  ui.document.querySelector = function (selector) {
    if (selector === `script[data-google-tag="${measurementId}"]`) return scripts[0] || null;
    return query(selector);
  };
  ui.document.head = { appendChild(script) { scripts.push(script); } };
  ui.document.createElement = function (tag) {
    assert.equal(tag, "script");
    return { dataset: {} };
  };
  const result = page(`https://agentizable.ai/${controls ? "privacy" : "market"}.html`, {
    document: ui.document, local, privacy, measurementId,
  });
  assert.equal(scripts.length, 1, "saved permission actually loads Google before revocation");
  assert.match(scripts[0].src, /googletagmanager\.com\/gtag\/js\?id=G-TEST123$/);
  assert.equal(typeof result.window.gtag, "function");
  assert.equal(result.window.dataLayer[1][0], "config");
  assert.equal(result.window[`ga-disable-${measurementId}`], undefined);
  return { ...result, ui, scripts };
}

test("another tab opting out stops already loaded Google on pages with or without privacy controls", () => {
  for (const controls of [false, true]) {
    const result = activeGooglePage({ controls });
    const sender = page("https://agentizable.ai/privacy.html", { local: result.local });
    sender.analytics.optOut();
    const writes = result.local.writes.length, googleCommands = result.window.dataLayer.length;
    result.emit("storage", { key: optOutKey });
    result.emit("storage", { key: googleConsentKey });
    assert.equal(result.window[`ga-disable-${measurementId}`], true, "set Google's per-window automatic-event stop flag in the receiving tab");
    assert.equal(result.analytics.track("market_view"), false);
    assert.equal(result.events.length, 1, "no new first-party event after revocation");
    assert.equal(result.window.dataLayer.length, googleCommands, "no new Google event or consent command");
    assert.equal(result.local.writes.length, writes, "receiving tab must not rewrite preferences or browser identifiers");
    assert.equal(result.scripts.length, 1);
    if (controls) assert.equal(result.ui.control.disabled, true);
  }
});

test("Google consent revocation or removal stops the loaded tag without granting permission later", () => {
  for (const consent of ["denied", null]) {
    const result = activeGooglePage();
    if (consent === null) result.local.removeItem(googleConsentKey);
    else result.local.setItem(googleConsentKey, consent);
    const writes = result.local.writes.length, commands = result.window.dataLayer.length;
    result.emit("storage", { key: consent === null ? null : googleConsentKey });
    assert.equal(result.window[`ga-disable-${measurementId}`], true);
    assert.equal(result.local.writes.length, writes);
    assert.equal(result.local.getItem(optOutKey), null, "Google-only revocation does not rewrite the first-party choice");
    assert.equal(result.events.length, 1);
    result.local.setItem(googleConsentKey, "granted");
    result.emit("storage", { key: googleConsentKey });
    result.emit("focus");
    result.emit("pageshow");
    assert.equal(result.window[`ga-disable-${measurementId}`], true, "a notification never re-enables Google in this window");
    assert.equal(result.window.dataLayer.length, commands);
    assert.equal(result.scripts.length, 1);
    assert.equal(result.events.length, 1);
  }
});

test("new browser privacy signals stop active Google on focus or restored pages without saving a choice", () => {
  for (const signal of [{ globalPrivacyControl: true }, { doNotTrack: "1" }, { msDoNotTrack: "1" }]) {
    for (const event of ["focus", "pageshow"]) {
      const privacy = {}, result = activeGooglePage({ privacy });
      const writes = result.local.writes.length, commands = result.window.dataLayer.length;
      Object.assign(privacy, signal);
      result.emit(event);
      assert.equal(result.window[`ga-disable-${measurementId}`], true);
      assert.equal(result.analytics.track("market_view"), false);
      assert.equal(result.events.length, 1);
      assert.equal(result.local.writes.length, writes);
      assert.equal(result.local.getItem(googleConsentKey), "granted", "browser policy does not rewrite saved permission");
      for (const key of Object.keys(signal)) delete privacy[key];
      result.emit(event);
      assert.equal(result.window[`ga-disable-${measurementId}`], true, "refresh never silently enables a stopped tag");
      assert.equal(result.window.dataLayer.length, commands);
    }
  }
});

test("losing storage access stops an active Google tag even on a page with no privacy controls", () => {
  const result = activeGooglePage(), writes = result.local.writes.length;
  Object.defineProperty(result.window, "localStorage", { get() { throw new Error("Storage unavailable"); } });
  result.emit("focus");
  assert.equal(result.window[`ga-disable-${measurementId}`], true);
  assert.equal(result.events.length, 1);
  assert.equal(result.local.writes.length, writes);
});

test("a new desktop browser receives referral tags without the original browser identity", () => {
  const first = page(landing);
  const destination = first.analytics.handoffUrl(review);
  const links = home.bountyAssistantLinks("gpt", undefined, destination);
  const desktop = new URL(links.desktopUrl);
  assert.equal(desktop.searchParams.get("browserUrl"), destination);
  assert.equal(desktop.searchParams.get("prompt"), new URL(links.webUrl).searchParams.get("prompt"));
  assert.match(desktop.searchParams.get("prompt"), /utm_source=example-source/);
  assert.doesNotMatch(desktop.searchParams.get("prompt"), /utm_source=chatgpt/);
  const next = page(desktop.searchParams.get("browserUrl"));
  assert.equal(next.events[0].source, "example-source");
  assert.equal(next.events[0].campaign, "example-campaign");
  assert.notEqual(next.events[0].visitor_id, first.events[0].visitor_id);
  assert.notEqual(next.events[0].session_id, first.events[0].session_id);
  for (const text of [destination, desktop.searchParams.get("prompt")]) {
    assert.ok(!text.includes(first.events[0].visitor_id));
    assert.ok(!text.includes(first.events[0].session_id));
    assert.doesNotMatch(text, /never-forward|gclid|not-collected/);
  }
});

test("head-loaded analytics preserves the optional choice until the page body exists", () => {
  for (const optOutBeforeReady of [false, true]) {
    let ready, notice;
    const scripts = [], buttons = {};
    const document = {
      referrer: "", body: null,
      head: { appendChild(script) { scripts.push(script); } },
      addEventListener(name, callback) { if (name === "DOMContentLoaded") ready = callback; },
      querySelector() { return notice || null; },
      createElement() {
        return { dataset: {}, setAttribute() {}, remove() { notice = null; },
          querySelector(selector) { return { addEventListener(_name, callback) { buttons[selector] = callback; } }; } };
      },
    };
    const result = page("https://agentizable.ai/collaborate/", { document, measurementId: "G-TEST123" });
    assert.equal(result.events.length, 1, "first-party startup must survive early loading");
    assert.equal(notice, undefined);
    assert.equal(scripts.length, 0, "no Google script before consent");
    assert.equal(typeof ready, "function");
    if (optOutBeforeReady) result.analytics.optOut();
    // The consent bar is the first element of the body, above the header.
    document.body = { prepend(element) { notice = element; } };
    ready();
    if (optOutBeforeReady) assert.equal(notice, undefined, "recheck opt-out at DOM readiness");
    else {
      assert.ok(notice);
      buttons["[data-google-analytics-deny]"]();
      assert.equal(notice, null);
      assert.equal(result.analytics.status().google_analytics, "denied");
    }
    assert.equal(scripts.length, 0, "denial or opt-out must not load Google");
  }
});

test("web and copied assistant prompts retain the same review destination and task", () => {
  const first = page(landing);
  const destination = first.analytics.handoffUrl(review);
  const task = { request: "Keep retries idempotent.", solver_reward_usdc: "3.00" };
  const prompt = postingPrompt.build(task, destination);
  assert.ok(prompt.includes(`Use ${destination} in @Browser`));
  assert.ok(prompt.includes(JSON.stringify(task, null, 2)));
  for (const [provider, parameter] of [["claude", "q"], ["cursor", "text"]]) {
    const links = home.bountyAssistantLinks(provider, postingPrompt.build(task), destination);
    assert.equal(new URL(links.webUrl).searchParams.get(parameter), prompt);
  }
});

test("first touch and explicit destination pairs are preserved independently", () => {
  const original = page("https://agentizable.ai/?utm_source=github&utm_campaign=readme");
  const later = page(landing, { local: original.local });
  const destination = new URL(later.analytics.handoffUrl(review));
  assert.equal(destination.searchParams.get("utm_source"), "github");
  assert.equal(destination.searchParams.get("utm_campaign"), "readme");
  const explicit = `${review}&utm_source=another-source`;
  assert.equal(later.analytics.handoffUrl(explicit), explicit);
  assert.equal(new URL(later.analytics.handoffUrl(explicit)).searchParams.has("utm_campaign"), false);
});

test("opt-out and privacy signals survive the assistant transition", () => {
  const cases = [
    { url: `${landing}&analytics=off` },
    { url: landing, privacy: { globalPrivacyControl: true } },
    { url: landing, privacy: { doNotTrack: "1" } },
  ];
  for (const options of cases) {
    const first = page(options.url, options);
    assert.equal(first.events.length, 0);
    const destination = first.analytics.handoffUrl(review);
    assert.equal(new URL(destination).searchParams.get("analytics"), "off");
    const links = home.bountyAssistantLinks("gpt", undefined, destination);
    const prompt = new URL(links.webUrl).searchParams.get("prompt");
    assert.ok(prompt.includes(destination));
    assert.doesNotMatch(prompt, /utm_source=chatgpt/);
    const next = page(new URL(links.desktopUrl).searchParams.get("browserUrl"));
    assert.equal(next.events.length, 0);
    assert.equal(page("https://agentizable.ai/post.html", { local: next.local }).events.length, 0);
  }
});

test("source tags are never added to another origin or a credentialed URL", () => {
  const first = page(landing);
  for (const destination of ["https://example.org/post.html", "https://user@agentizable.ai/post.html", "javascript:alert(1)", "not a URL"]) {
    assert.equal(first.analytics.handoffUrl(destination), destination);
  }
});

test("untrusted review destinations cannot redirect the launcher or inject prompt instructions", () => {
  for (const destination of [
    "https://example.org/post.html", "https://agentizable.ai/authorize.html",
    "https://user@agentizable.ai/post.html", `${review}&wallet_signature=private`,
    `${review}&utm_source=first&utm_source=second`, `${review}&utm_campaign=bad%0Ainstructions`,
    `${review}&parentBounty=not-an-address`, `${review}#untrusted`,
  ]) {
    assert.equal(postingPrompt.reviewUrl(destination), null);
    assert.equal(postingPrompt.build(null, destination), postingPrompt.build());
    assert.equal(new URL(home.bountyAssistantLinks("gpt", undefined, destination).desktopUrl).searchParams.get("browserUrl"), review);
  }
  const parent = `${review}&parentBounty=0x${"1".repeat(40)}`;
  assert.equal(postingPrompt.reviewUrl(parent), parent);
  assert.ok(postingPrompt.build({ meta_child: { parent_bounty_contract: `0x${"1".repeat(40)}` } }, parent).includes(parent));
});
