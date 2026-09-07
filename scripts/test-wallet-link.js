"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const createWalletLink = require("../site/wallet-link.js");

class Element extends EventTarget {
  constructor(tag) { super(); this.tagName = tag; this.children = []; this.nodes = new Map(); this.classList = { add() {} }; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  get childElementCount() { return this.children.length; }
  setAttribute() {}
  focus() { this.focused = true; }
  remove() { this.removed = true; }
  showModal() { this.open = true; }
  close() { this.open = false; this.dispatchEvent(new Event("close")); }
  querySelector(key) { if (!this.nodes.has(key)) this.nodes.set(key, new Element("div")); return this.nodes.get(key); }
  click() { if (!this.disabled) this.dispatchEvent(new Event("click")); }
}
function harness(configured = true) {
  const win = new EventTarget();
  Object.assign(win, {
    Event, setTimeout, clearTimeout,
    document: { baseURI: "https://agentbounties.app/", head: new Element("head"), body: new Element("body"), createElement: (tag) => new Element(tag) },
    AgentBountiesWalletConfig: { providers: { coinbaseEmbedded: { enabled: configured } } },
  });
  const calls = [];
  win.ethereum = { isMetaMask: true, request: async (request) => { calls.push(request); return []; } };
  const chooser = createWalletLink(win);
  const dialog = () => win.document.body.children[0];
  return { win, calls, chooser, dialog, create: () => dialog().querySelector("[data-wallet-create]").children[0] };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));

test("discovery and opening never wake MetaMask, even as the only wallet", async () => {
  const h = harness();
  assert.equal(h.chooser.choices()[0].label, "MetaMask");
  const selection = h.chooser.select();
  assert.equal(h.dialog().open, true);
  assert.equal(h.create().disabled, false);
  assert.deepEqual(h.calls, []);
  h.dialog().querySelector(".wallet-link-close").click();
  await assert.rejects(selection, { code: 4001 });
  assert.deepEqual(h.calls, []);
});

test("selecting an announced wallet returns only that provider and deduplicates fallback", async () => {
  const h = harness();
  const another = { request() { throw new Error("discovery must not call me"); } };
  for (const [provider, name] of [[h.win.ethereum, "MetaMask"], [another, "Other wallet"]]) {
    const event = new Event("eip6963:announceProvider");
    event.detail = { provider, info: { name } };
    h.win.dispatchEvent(event);
  }
  assert.equal(h.chooser.choices().length, 2);
  const selection = h.chooser.select();
  h.dialog().querySelector("[data-wallet-choices]").children[1].click();
  assert.equal((await selection).provider, another);
  assert.deepEqual(h.calls, []);
});

test("new-wallet option loads Coinbase and never uses the installed wallet", async () => {
  const h = harness();
  const selection = h.chooser.select();
  h.create().click();
  const embedded = { request() {} };
  h.win.AgentBountiesCoinbaseEmbeddedWallet = { enabled: true, provider: embedded };
  assert.equal(h.win.document.head.children.length, 1);
  h.win.document.head.children[0].onload();
  h.win.document.head.children[1].onload();
  assert.equal((await selection).provider, embedded);
  assert.deepEqual(h.calls, []);
});

test("cancelling a loading wallet cannot resume an old selection or a new chooser", async () => {
  const h = harness();
  const selection = h.chooser.select();
  h.create().click();
  h.chooser.cancel();
  await assert.rejects(selection, { code: 4001 });
  const next = h.chooser.select();
  h.win.AgentBountiesCoinbaseEmbeddedWallet = { enabled: true, provider: { request() {} } };
  h.win.document.head.children[0].onload();
  h.win.document.head.children[1].onload();
  await tick();
  assert.equal(h.dialog().open, true);
  h.chooser.cancel();
  await assert.rejects(next, { code: 4001 });
  assert.deepEqual(h.calls, []);
});

test("a bundle failure stays in the chooser and allows an explicit retry", async () => {
  const h = harness();
  const selection = h.chooser.select();
  h.create().click();
  h.win.document.head.children[0].onerror();
  await tick();
  assert.match(h.dialog().querySelector("[role=status]").textContent, /try again/);
  assert.equal(h.create().disabled, false);
  h.create().click();
  assert.equal(h.win.document.head.children.length, 2);
  h.win.document.head.children[1].onerror();
  await tick();
  h.chooser.cancel();
  await assert.rejects(selection, { code: 4001 });
  assert.deepEqual(h.calls, []);
});

test("no-wallet and unconfigured deployments do not fall through to an injected provider", async () => {
  const h = harness(false);
  delete h.win.ethereum;
  const selection = h.chooser.select();
  assert.equal(h.create().disabled, true);
  assert.equal(h.chooser.choices().length, 0);
  assert.match(h.dialog().querySelector("[role=status]").textContent, /not configured/);
  h.chooser.cancel();
  await assert.rejects(selection, { code: 4001 });
});
