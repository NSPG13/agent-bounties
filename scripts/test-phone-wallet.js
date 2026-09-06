"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const { webcrypto } = require("node:crypto");
const createPhoneWallet = require("../site/phone-wallet.js");
const address = "0x" + "12".repeat(20);
const uri = `wc:${"a".repeat(64)}@2?relay-protocol=irn&symKey=${"b".repeat(64)}`;
const flush = async () => { for (let i = 0; i < 6; i++) await new Promise(setImmediate); };
function fixture({ configured = true, storage = new Map(), restored = false, failLoad = false } = {}) {
  const events = new Map(), nodes = [], timers = new Map(), providers = [], options = [], requests = [], qrValues = [];
  let loads = 0, timerId = 0;
  const element = (tag) => {
    const listeners = new Map();
    const node = { tag, hidden: false, open: false, children: [], listeners, textContent: "", attributes: {},
      setAttribute(k, v) { this.attributes[k] = v; }, removeAttribute(k) { delete this[k]; delete this.attributes[k]; },
      addEventListener(k, fn) { listeners.set(k, fn); }, append(...children) { this.children.push(...children); },
      showModal() { this.open = true; }, close() { this.open = false; listeners.get("close")?.(); }, click() { return listeners.get("click")?.(); } };
    nodes.push(node); return node;
  };
  const win = { document: { body: element("body"), createElement: element }, crypto: webcrypto,
    agentBountiesPhoneWalletConfig: { projectId: configured ? "1".repeat(32) : "" }, location: new URL("https://agentbounties.app/"),
    localStorage: { getItem: (k) => storage.get(k), setItem: (k, v) => storage.set(k, v), removeItem: (k) => storage.delete(k) },
    addEventListener(k, fn) { if (!events.has(k)) events.set(k, []); events.get(k).push(fn); }, dispatchEvent(event) { for (const fn of events.get(event.type) || []) fn(event); },
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
    setTimeout(fn) { timers.set(++timerId, fn); return timerId; }, clearTimeout(id) { timers.delete(id); },
  };
  const api = createPhoneWallet(win, { loadVendor: async () => {
    loads++; if (failLoad) throw new Error("private relay error");
    return { qrDataUrl: async (value) => { qrValues.push(value); return "data:image/png;base64,fixture"; }, createProvider: async (config) => {
      options.push(config);
      const listeners = new Map(); let resolveConnect, rejectConnect;
      const sdk = { accounts: [], chainId: 8453, session: null, connects: 0, disconnects: 0, cleanups: 0,
        signer: { cleanupPendingPairings: async () => { sdk.cleanups++; } },
        on(k, fn) { listeners.set(k, fn); }, emit(k, value) { listeners.get(k)?.(value); },
        async connect() { this.connects++; const pending = new Promise((resolve, reject) => { resolveConnect = resolve; rejectConnect = reject; }); this.emit("display_uri", uri); return pending; },
        approve(account = address, chain = 8453, methods = config.optionalMethods) { this.accounts = [account]; this.session = { expiry: Date.now() / 1000 + 3600, namespaces: { eip155: { accounts: [`eip155:${chain}:${account}`], methods } } }; this.emit("accountsChanged", this.accounts); resolveConnect?.(); },
        reject() { rejectConnect(Object.assign(new Error("User rejected"), { code: 4001 })); },
        async disconnect() { this.disconnects++; this.accounts = []; this.session = null; this.emit("disconnect"); },
        async request(request) { requests.push(request); if (this.requestError) throw this.requestError; return "confirmed wallet response"; },
      };
      if (restored) sdk.approve(); providers.push(sdk); return sdk;
    } };
  } });
  return { api, win, providers, nodes, storage, requests, timers, options, qrValues, loads: () => loads,
    button: (text) => nodes.find((n) => n.tag === "button" && n.textContent === text), qr: () => nodes.find((n) => n.tag === "img") };
}
test("discovery and status load no relay or wallet, and advertise an EIP-6963 provider", () => {
  const env = fixture(); let announced;
  env.win.addEventListener("eip6963:announceProvider", (e) => { announced = e.detail; });
  env.win.dispatchEvent({ type: "eip6963:requestProvider" });
  assert.equal(announced.provider, env.api.provider); assert.equal(announced.info.name, "Phone wallet (QR)");
  assert.equal(env.api.state().connected, false); assert.equal(env.loads(), 0);
});
test("QR creation is preparation and returns no pairing secrets", async () => {
  const env = fixture(); const opened = await env.api.openReview(); await flush();
  assert.equal(opened.review_open, true); assert.equal(env.api.state().status, "pairing"); assert.equal(env.api.state().connected, false);
  assert.equal(env.qr().hidden, false); assert.deepEqual(env.requests, []);
  const result = JSON.stringify(env.api.state()); assert.ok(!result.includes("wc:")); assert.ok(!result.includes("symKey")); assert.ok(!result.includes("session"));
  assert.deepEqual(env.options[0].chains, [8453]); assert.deepEqual(env.options[0].optionalChains, [8453]);
  assert.equal(env.options[0].telemetryEnabled, false); assert.equal(env.options[0].logger, "silent");
  assert.equal(env.options[0].metadata.url, "https://agentbounties.app");
});
test("only a valid Base session confirms connection, closes QR, and permits exact requests", async () => {
  const env = fixture(); const connecting = env.api.provider.request({ method: "eth_requestAccounts" }); await flush();
  assert.equal(env.api.state().connected, false); env.providers[0].approve(); assert.deepEqual(await connecting, [address]);
  assert.equal(env.api.state().connected, true); assert.equal(env.api.state().payment_authorized, false); assert.equal(env.qr().src, undefined);
  const request = { method: "eth_sendTransaction", params: [{ from: address, to: "0x" + "34".repeat(20), data: "0x1234", value: "0x0" }] };
  await env.api.provider.request(request); assert.equal(env.requests[0], request);
});
test("double opening shares one pending connection and QR", async () => {
  const env = fixture(); await env.api.openReview(); await env.api.openReview(); await flush();
  assert.equal(env.providers.length, 1); assert.equal(env.providers[0].connects, 1); assert.equal(env.qrValues.length, 1);
});
test("cancellation erases QR and late approval cannot replace a successful retry", async () => {
  const env = fixture(); const first = env.api.provider.request({ method: "eth_requestAccounts" }); await flush();
  const rejected = assert.rejects(first, { code: 4001 }); env.button("Close").click(); await rejected;
  assert.equal(env.qr().src, undefined); assert.equal(env.api.state().connected, false);
  const second = env.api.provider.request({ method: "eth_requestAccounts" }); await flush();
  assert.notEqual(env.options[0].customStoragePrefix, env.options[1].customStoragePrefix);
  env.providers[1].approve(); await second; env.providers[0].approve(); await flush();
  assert.equal(env.providers[0].disconnects, 1); assert.equal(env.providers[1].disconnects, 0); assert.equal(env.api.state().connected, true);
});
test("QR expiry cancels a pending request without sending anything", async () => {
  const env = fixture(); const pending = env.api.provider.request({ method: "eth_requestAccounts" }); await flush();
  const rejected = assert.rejects(pending, { code: 4001 }); [...env.timers.values()][0](); await rejected;
  assert.equal(env.api.state().status, "expired"); assert.equal(env.qr().src, undefined); assert.equal(env.requests.length, 0);
});
test("approved session restores across navigation without QR or another approval", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(); await flush();
  const next = fixture({ storage: env.storage, restored: true });
  assert.deepEqual(await next.api.provider.request({ method: "eth_accounts" }), [address]);
  assert.equal(next.providers[0].connects, 0); assert.equal(next.qrValues.length, 0); assert.equal(next.api.state().review_open, false);
});
test("rejection, wrong network, missing config and network failures never connect", async () => {
  for (const action of ["reject", "wrong-chain", "missing-config", "network"]) {
    const env = fixture({ configured: action !== "missing-config", failLoad: action === "network" });
    const pending = env.api.provider.request({ method: "eth_requestAccounts" }); const rejected = assert.rejects(pending);
    await flush(); if (action === "reject") env.providers[0].reject(); if (action === "wrong-chain") env.providers[0].approve(address, 1);
    await rejected; assert.equal(env.api.state().connected, false); assert.equal(env.requests.length, 0); assert.equal(env.qr()?.src, undefined);
  }
});
test("wallet rejection and uncertain transaction errors are preserved without retries", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(); await flush();
  for (const code of [4001, 4900, -32000]) {
    const original = Object.assign(new Error("Uncertain request"), { code }); env.providers[0].requestError = original;
    await assert.rejects(env.api.provider.request({ method: "eth_sendTransaction", params: [{ from: address }] }), (caught) => caught === original);
  }
  assert.equal(env.requests.length, 3);
});
test("unapproved methods fail before dispatch; unsupported batch can use existing safe fallback", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(address, 8453, ["eth_sendTransaction"]); await flush();
  await assert.rejects(env.api.provider.request({ method: "wallet_sendCalls", params: [] }), { code: 4200 });
  await assert.rejects(env.api.provider.request({ method: "eth_sign", params: [] }), { code: 4200 });
  assert.equal(env.requests.length, 0);
});
test("account change, revocation and expiry invalidate the previous account", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(); await flush();
  const other = "0x" + "56".repeat(20); env.providers[0].approve(other);
  assert.equal(env.api.state().address, other);
  await assert.rejects(env.api.provider.request({ method: "eth_sendTransaction", params: [{ from: address }] }), { code: 4100 });
  await assert.rejects(env.api.provider.request({ method: "eth_signTypedData_v4", params: [address, "{}"] }), { code: 4100 });
  assert.equal(env.requests.length, 0);
  env.providers[0].session.expiry = 1;
  assert.deepEqual(await env.api.provider.request({ method: "eth_accounts" }), []); assert.equal(env.api.state().connected, false);
  await env.api.disconnect(); assert.equal(env.storage.size, 0); assert.equal(env.api.state().connected, false);
});
test("a changed chain blocks signing and balance reads until Base is restored", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(); await flush();
  env.providers[0].chainId = 1; env.providers[0].emit("chainChanged", "0x1");
  await assert.rejects(env.api.provider.request({ method: "eth_getBalance", params: [address, "latest"] }), { code: 4901 });
  await assert.rejects(env.api.provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x1" }] }), { code: 4901 });
  assert.equal(env.requests.length, 0);
  await env.api.provider.request({ method: "wallet_switchEthereumChain", params: [{ chainId: "0x2105" }] }); assert.equal(env.requests.length, 1);
});
test("disconnect failure is explicit and cannot be mistaken for confirmed revocation", async () => {
  const env = fixture(); await env.api.openReview(); await flush(); env.providers[0].approve(); await flush();
  env.providers[0].disconnect = async () => { throw new Error("Relay lost"); };
  const result = await env.api.disconnect(); assert.equal(result.status, "error"); assert.match(result.message, /could not confirm disconnection/);
  await assert.rejects(env.api.provider.request({ method: "personal_sign", params: ["0x1234", address] }), { code: 4900 });
});
