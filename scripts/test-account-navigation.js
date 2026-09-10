"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const source = fs.readFileSync(require("node:path").join(__dirname, "../site/site-navigation.js"), "utf8");

class Element extends EventTarget {
  constructor() { super(); this.dataset = {}; this.attributes = {}; this.classList = { add() {} }; }
  getAttribute(key) { return this.attributes[key]; }
  setAttribute(key, value) { this.attributes[key] = value; }
}
function harness({ homepage = false, payload = { authenticated: false }, href = "./?postReturn=1#login" } = {}) {
  const win = new EventTarget(), doc = new EventTarget(), header = new Element(), toggle = new Element(), nav = new Element(), link = new Element();
  link.setAttribute("href", href);
  link.textContent = "Login";
  let calls = 0, failure = false;
  Object.assign(win, {
    location: new URL("https://agentbounties.app/post.html"), setTimeout, clearTimeout,
    matchMedia: () => ({ addEventListener() {} }),
    fetch: async () => { calls++; if (failure) throw new Error("offline"); return { ok: true, json: async () => payload }; },
  });
  header.querySelector = selector => ({ ".ab-site-menu": toggle, ".ab-site-nav": nav, ".ab-site-login": link })[selector];
  doc.querySelector = selector => selector === "[data-site-header]" ? header : selector === "[data-auth-dialog]" && homepage ? {} : null;
  vm.runInNewContext(source, { window: win, document: doc, URL, AbortController });
  return { win, link, calls: () => calls, fail: () => { failure = true; }, emit: payload => win.dispatchEvent(new CustomEvent("agentbounties:account-session", { detail: payload })) };
}
const tick = () => new Promise(resolve => setImmediate(resolve));

test("interior Account link uses server session and strips posting return redirect", async () => {
  const h = harness({ payload: { authenticated: true, user: { id: "fixture", name: "Member" } } });
  await tick();
  assert.equal(h.link.textContent, "Account");
  assert.equal(h.link.href, "https://agentbounties.app/#account");
  assert.equal(h.link.dataset.authenticated, "true");
  h.fail();
  h.win.dispatchEvent(new Event("focus"));
  await tick();
  assert.equal(h.link.textContent, "Account", "network failure is not sign-out evidence");
  h.emit({ authenticated: false, user: null });
  assert.equal(h.link.textContent, "Login");
  assert.equal(h.link.getAttribute("href"), "./?postReturn=1#login");
});

test("homepage shares its existing session and never introduces duplicate auth requests", async () => {
  const h = harness({ homepage: true });
  await tick();
  assert.equal(h.calls(), 0);
  h.emit({ authenticated: true, user: { id: "fixture", name: "Member" } });
  assert.equal(h.link.textContent, "Account");
  h.emit({ authenticated: true, user: { email: "unverified@example.invalid" } });
  assert.equal(h.link.textContent, "Login", "email alone is never a session identity");
});
