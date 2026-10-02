"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");
const path = require("node:path");
const api = require("../site/collaborate/assessment.js");
const source = fs.readFileSync(path.join(__dirname, "../site/collaborate/collaborate.js"), "utf8");
const fixture = JSON.parse(fs.readFileSync(path.join(__dirname, "../examples/free-discovery/fixture.json"), "utf8"));
class Element {
  constructor() { this.children = []; this.listeners = {}; this.attributes = {}; this.disabled = false; this.textContent = ""; }
  appendChild(child) { this.children.push(child); return child; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(k, v) { this.attributes[k] = v; }
  addEventListener(k, fn) { this.listeners[k] = fn; }
  click() { return this.listeners.click?.(); }
  remove() {}
}
function harness({ saved = null, storageFails = false, response = fixture, error = null, ok = true } = {}) {
  const names = ["discovery-status", "discovery-results", "check-discovery", "download-assessment", "clear-assessment"];
  const nodes = Object.fromEntries(names.map(name => [name, new Element()]));
  nodes["download-assessment"].disabled = nodes["clear-assessment"].disabled = true;
  let stored = saved, requests = [], nextError = error;
  const downloads = [], exported = [], revoked = [], cleanup = [];
  class DownloadURL extends URL {
    static createObjectURL(blob) { exported.push(blob); return "blob:assessment-test"; }
    static revokeObjectURL(url) { revoked.push(url); }
  }
  const context = {
    window: { AgentBountiesDiscovery: api, setTimeout: (fn, ms) => ms === 1000 ? cleanup.push(fn) : setTimeout(fn, ms), clearTimeout },
    document: { getElementById: id => nodes[id], createElement: tag => { const e = new Element(); if (tag === "a") e.click = () => downloads.push({ href: e.href, filename: e.download }); return e; }, querySelectorAll: () => [], body: new Element() },
    localStorage: { getItem: () => stored, setItem: (_, value) => { if (storageFails) throw Error("blocked"); stored = value; }, removeItem: () => { stored = null; } },
    fetch: async (url, options) => { requests.push({url,options}); if (nextError) throw nextError; return { ok, status: 503, text: async () => JSON.stringify(response) }; },
    AbortController, URL: DownloadURL, Blob, navigator: {},
  };
  vm.runInNewContext(source, context);
  return { nodes, requests, exported, downloads, revoked, cleanup, stored: () => stored, fail: () => { nextError = Error("offline"); } };
}
test("one explicit read creates a saved assessment without credentials or writes", async () => {
  const h = harness();
  assert.equal(h.requests.length, 0);
  await h.nodes["check-discovery"].click();
  assert.equal(h.requests.length, 1);
  assert.equal(h.requests[0].url, api.FEED_URL);
  assert.equal(h.requests[0].options.credentials, "omit");
  assert.equal(h.requests[0].options.method, undefined);
  assert.equal(JSON.parse(h.stored()).mode, "read_only");
  assert.equal(h.nodes["download-assessment"].disabled, false);
  assert.match(h.nodes["discovery-status"].textContent, /Assessment ready/);
  assert.equal(h.nodes["discovery-results"].attributes["aria-busy"], "false");
});
test("failure preserves previous saved output and restores retry controls", async () => {
  const h = harness(); await h.nodes["check-discovery"].click();
  const saved = h.stored(); h.fail(); await h.nodes["check-discovery"].click();
  assert.equal(h.stored(), saved);
  assert.equal(h.nodes["check-discovery"].disabled, false);
  assert.equal(h.nodes["download-assessment"].disabled, false);
  assert.match(h.nodes["discovery-status"].textContent, /previous saved assessment is unchanged/);
});
test("unavailable storage still lets a person export a successful assessment", async () => {
  const h = harness({ storageFails: true }); await h.nodes["check-discovery"].click();
  assert.equal(h.stored(), null);
  assert.equal(h.nodes["download-assessment"].disabled, false);
  assert.match(h.nodes["discovery-status"].textContent, /storage is unavailable/);
});
test("download contains the exact assessment and releases its temporary object URL", async () => {
  const h = harness(); await h.nodes["check-discovery"].click();
  await h.nodes["download-assessment"].click();
  assert.deepEqual(h.downloads, [{ href: "blob:assessment-test", filename: "agent-bounties-discovery-assessment.json" }]);
  assert.equal(h.exported[0].type, "application/json");
  assert.deepEqual(JSON.parse(await h.exported[0].text()), JSON.parse(h.stored()));
  h.cleanup.forEach(fn => fn());
  assert.deepEqual(h.revoked, ["blob:assessment-test"]);
  assert.equal(h.requests.length, 1, "Export must not make another request");
});
test("reload restores a historical snapshot without any automatic request", () => {
  const saved = JSON.stringify(api.assess(fixture)); const h = harness({ saved });
  assert.equal(h.requests.length, 0);
  assert.equal(h.nodes["download-assessment"].disabled, false);
  assert.match(h.nodes["discovery-status"].textContent, /historical snapshot/);
});
test("clear removes the local continuation and disables its export", async () => {
  const h = harness({ saved: JSON.stringify(api.assess(fixture)) });
  await h.nodes["clear-assessment"].click();
  assert.equal(h.stored(), null);
  assert.equal(h.nodes["discovery-results"].children.length, 0);
  assert.equal(h.nodes["download-assessment"].disabled, true);
});
test("corrupt saved data cannot leave partial cards or an enabled export", () => {
  const corrupt = api.assess(fixture); delete corrupt.items[0].reward;
  const h = harness({ saved: JSON.stringify(corrupt) });
  assert.equal(h.nodes["discovery-results"].children.length, 0);
  assert.equal(h.nodes["download-assessment"].disabled, true);
  assert.match(h.nodes["discovery-status"].textContent, /No readable saved assessment/);
});
test("bad HTTP, malformed JSON contract and timeout do not erase or create saved work", async () => {
  for (const options of [{ok:false},{response:{}},{error:Object.assign(Error("timeout"),{name:"AbortError"})}]) {
    const h = harness(options); await h.nodes["check-discovery"].click();
    assert.equal(h.stored(), null);
    assert.equal(h.nodes["check-discovery"].disabled, false);
    assert.equal(h.nodes["download-assessment"].disabled, true);
  }
});
