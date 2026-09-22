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
  return { win, calls, chooser, dialog, create: () => dialog().querySelector("[data-wallet-create-button]").children[0] };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));

test("phone pairing is one contextual choice even when announced and injected", async () => {
  const h = harness();
  const phone = { request() { throw new Error("selection must not request a signature"); } };
  h.win.AgentBountiesPhoneWallet = { provider: phone, state: () => ({ available: true }) };
  h.win.ethereum.providers = [h.win.ethereum, phone];
  const event = new Event("eip6963:announceProvider");
  event.detail = { provider: phone, info: { name: "Phone wallet (QR)" } };
  h.win.dispatchEvent(event);
  assert.equal(h.chooser.choices().filter(item => item.provider === phone).length, 1);
  const selected = h.chooser.select();
  h.dialog().querySelector("[data-wallet-choices]").children[3].children[1].children[0].click();
  assert.equal((await selected).provider, phone);
  assert.deepEqual(h.calls, []);
});

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
  h.dialog().querySelector("[data-wallet-choices]").children[3].children[1].children[0].click();
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
  h.dialog().querySelector("[data-wallet-choices]").children[0].click();
  assert.match(h.dialog().querySelector("[role=status]").textContent, /unavailable/);
  h.chooser.cancel();
  await assert.rejects(selection, { code: 4001 });
});

test("OAuth continuation retains its selected address and remains bound to the account", () => {
  const h = harness(), records = new Map();
  h.win.sessionStorage = { getItem: key => records.get(key) ?? null, setItem: (key, value) => records.set(key, value), removeItem: key => records.delete(key) };
  const selectedAddress = "0x" + "AB".repeat(20);
  h.chooser.beginPending("member", selectedAddress);
  assert.equal(h.chooser.hasPending("member"), true);
  assert.equal(h.chooser.pendingAddress("member"), selectedAddress.toLowerCase());
  assert.equal(h.chooser.hasPending("another-member"), false);
  assert.equal(h.chooser.pendingAddress("member"), null);
  h.chooser.beginPending("member");
  assert.equal(h.chooser.hasPending("member"), true);
  assert.equal(h.chooser.pendingAddress("member"), null);
  const key = "agentbounties:pending-embedded-account-link";
  records.set(key, JSON.stringify({ userId: "member", startedAt: Date.now(), expectedAddress: "invalid" }));
  assert.equal(h.chooser.hasPending("member"), false);
});

const options = h => h.dialog().querySelector("[data-wallet-choices]").children;
test("three distinct brands never connect or switch a wallet on discovery", async () => {
  const h = harness(); const pending = h.chooser.select();
  assert.deepEqual(options(h).slice(0, 3).map(item => item.children[0].textContent), ["Coinbase", "MetaMask", "MoonPay"]);
  options(h)[2].click();
  assert.match(options(h)[0].textContent, /not supported here/);
  assert.deepEqual(h.calls, []);
  assert.equal(h.dialog().open, true);
  h.dialog().querySelector("[data-wallet-back]").click();
  options(h)[1].click(); options(h)[0].click();
  assert.equal((await pending).provider, h.win.ethereum);
  assert.deepEqual(h.calls, []);
});

test("Coinbase compatibility flags never turn it into the MetaMask choice", async () => {
  const h = harness(); h.win.ethereum.isCoinbaseWallet = true;
  const pending = h.chooser.select(); options(h)[1].click();
  assert.equal(options(h).some(item => item.children[0]?.textContent === "Connect MetaMask"), false);
  h.dialog().querySelector("[data-wallet-back]").click(); options(h)[0].click();
  assert.equal(options(h)[0].children[0].textContent, "Connect Coinbase"); options(h)[0].click();
  assert.equal((await pending).provider, h.win.ethereum);
  assert.deepEqual(h.calls, []);
});

test("phone continuation saves the existing review before copying, without wallet requests", async () => {
  const h = harness(); const calls = [];
  const url = "https://agentbounties.app/post.html?operation_id=11111111-1111-4111-8111-111111111111&funding_review=1#bounty-preview";
  h.win.navigator = { clipboard: { writeText: async value => calls.push(["copy", value]) } };
  const pending = h.chooser.select({ prepareContinuation: async () => { calls.push(["save"]); return url; } });
  options(h)[0].click(); options(h)[1].click(); await tick();
  assert.deepEqual(calls, [["save"], ["copy", url]]);
  assert.match(h.dialog().querySelector("[role=status]").textContent, /Link copied/);
  assert.deepEqual(h.calls, []); h.chooser.cancel(); await assert.rejects(pending, {code:4001});
});


test("a different announced brand with MetaMask compatibility stays under Other wallets", async () => {
  const h = harness();
  const event = new Event("eip6963:announceProvider");
  event.detail = {provider:h.win.ethereum, info:{name:"Trust Wallet", rdns:"com.trustwallet.app"}};
  h.win.dispatchEvent(event);
  const pending = h.chooser.select(); options(h)[1].click();
  assert.equal(options(h).some(item => item.children[0]?.textContent === "Connect MetaMask"), false);
  h.chooser.cancel(); await assert.rejects(pending, {code:4001});
});
