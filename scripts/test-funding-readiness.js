"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");

const root = path.resolve(__dirname, "..");
const wallet = `0x${"a".repeat(40)}`;
const usdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const runScript = (context, name) => vm.runInContext(fs.readFileSync(path.join(root, "site", name), "utf8"), context, { filename: name });

function helperContext() {
  const context = vm.createContext({ window: {}, AbortController, setTimeout, clearTimeout });
  runScript(context, "funding-readiness.js");
  return context.window.AgentBountiesFundingReadiness;
}

test("USDC shortfall keeps exact six-decimal arithmetic and never fabricates a fiat quote", () => {
  const api = helperContext();
  assert.equal(api.shortfall(api.parseUsdc("5"), api.parseUsdc("1.234567")), 3765433n);
  assert.equal(api.formatUnits(3765433n), "3.765433");
  assert.equal(api.shortfall(5n, 8n), 0n);
  assert.throws(() => api.parseUsdc("0.0000001"));
  assert.throws(() => api.parseUsdc("1e6"));
  assert.throws(() => api.shortfall(-1n, 0n));
});

test("readiness pins public Base balances to one block, without connecting or signing", async () => {
  const api = helperContext();
  const calls = [];
  const provider = { async request(request) {
    calls.push(request);
    return { eth_chainId: "0x2105", eth_blockNumber: "0x123", eth_getBalance: "0x12", eth_call: "0x1e8480" }[request.method];
  } };
  const result = await api.readBalances({ wallet, usdcAddress: usdc, provider });
  assert.equal(result.eth, 18n);
  assert.equal(result.usdc, 2000000n);
  assert.equal(calls[2].params[1], "0x123");
  assert.equal(calls[3].params[1], "0x123");
  assert.equal(calls[3].params[0].to, usdc);
  assert.equal(calls.some(({ method }) => /sign|Accounts|sendTransaction|switch/.test(method)), false);
});

test("readiness rejects wrong network, token, malformed balances and a stalled provider", async () => {
  const api = helperContext();
  await assert.rejects(api.readBalances({ wallet, provider: { request: async () => "0x1" } }), /Base mainnet/);
  await assert.rejects(api.readBalances({ wallet, usdcAddress: wallet }), /native Base USDC/);
  await assert.rejects(api.readBalances({ wallet, provider: { request: async () => "garbage" } }), /invalid balance/);
  await assert.rejects(api.readBalances({ wallet, provider: { request: () => new Promise(() => {}) }, timeoutMs: 5 }), /timed out/);
});

test("fee estimate includes live L1, L2 and operator fees without advertising a guaranteed maximum", async () => {
  const api = helperContext();
  const calls = [];
  const provider = { async request(request) {
    calls.push(request);
    if (request.method === "eth_call") return request.params[0].data.startsWith("0xf1c7a58b") ? "0x64" : "0x7";
    return { eth_chainId: "0x2105", eth_blockNumber: "0x123", eth_gasPrice: "0x3", eth_estimateGas: "0x5208" }[request.method];
  } };
  const result = await api.estimateFees({ provider, wallet, calls: [{ to: wallet, data: "0x1234", value: "0x0" }] });
  assert.equal(result.status, "estimated");
  assert.equal(result.calls[0].l2ExecutionWei, 63000n);
  assert.equal(result.calls[0].l1DataWei, 100n);
  assert.equal(result.calls[0].operatorWei, 7n);
  assert.equal(result.estimatedTotalWei, 63107n);
  assert.equal(result.maximumWei, null);
  assert.equal(calls.filter(({ method }) => method === "eth_call").every(({ params }) => params[1] === "0x123"), true);
  assert.equal(calls.find(({ method }) => method === "eth_estimateGas").params[0].data, "0x1234");
  assert.equal(calls.some(({ method }) => /sign|sendTransaction/.test(method)), false);
});

test("missing L1 or operator cost and failed allowance simulation never become zero-cost estimates", async () => {
  const api = helperContext();
  for (const fail of ["l1", "operator", "simulation"]) {
    const provider = { async request({ method, params }) {
      if (method === "eth_estimateGas" && fail === "simulation") throw new Error("missing allowance");
      if (method === "eth_call") {
        const isL1 = params[0].data.startsWith("0xf1c7a58b");
        if ((isL1 && fail === "l1") || (!isL1 && fail === "operator")) throw new Error("oracle unavailable");
        return "0x64";
      }
      return { eth_chainId: "0x2105", eth_blockNumber: "0x123", eth_gasPrice: "0x3", eth_estimateGas: "0x5208" }[method];
    } };
    const result = await api.estimateFees({ provider, wallet, calls: [{ to: wallet, data: "0x1234" }] });
    assert.equal(result.status, "partial");
    assert.equal(result.estimatedTotalWei, null);
    assert.equal(result.maximumWei, null);
    assert.ok(result.calls[0].error);
  }
  const timed = await api.estimateFees({ provider: { request: () => new Promise(() => {}) }, wallet, calls: [{ to: wallet }], timeoutMs: 5 });
  assert.equal(timed.status, "unavailable");
  assert.equal(timed.estimatedTotalWei, null);
});

async function onrampContext({ saved = new Map(), balance = "0x1e8480", contract = "", checkoutError = false } = {}) {
  const elements = new Map();
  const opened = [];
  const requests = [];
  const element = (selector) => {
    if (!elements.has(selector)) elements.set(selector, {
      value: "", checked: false, hidden: false, disabled: false, textContent: "", dataset: {}, handlers: {},
      addEventListener(type, callback) { this.handlers[type] = callback; },
      append() {}, setAttribute(name, value) { this[name] = value; },
    });
    return elements.get(selector);
  };
  element("[data-onramp-asset]").value = "usdc";
  const location = new URL(`https://agentbounties.app/onramp.html?amount=5&wallet=${wallet}&operation=posting-123${contract ? `&bountyContract=${contract}` : ""}&return=${encodeURIComponent("https://agentbounties.app/post.html?journey=saved-123")}`);
  location.assign = () => {};
  const window = { handlers: {}, addEventListener(type, handler) { this.handlers[type] = handler; },
    dispatchEvent(event) { this.handlers[event.type]?.(event); },
    open(url, target) {
      const tab = { url, target, opener: {}, closed: false, focus() {}, close() { this.closed = true; } };
      tab.location = { replace(value) { tab.url = value; } };
      opened.push(tab);
      return tab;
    },
  };
  const context = vm.createContext({ window, location, URL, URLSearchParams, AbortController,
    Event: class Event { constructor(type) { this.type = type; } },
    setTimeout: (fn, delay) => setTimeout(fn, delay === 250 ? 0 : delay), clearTimeout, setInterval: () => 0,
    localStorage: { getItem: (key) => saved.get(key) || null, setItem: (key, value) => saved.set(key, value) },
    document: { hidden: false, querySelector: element, querySelectorAll: () => [], createElement: () => ({ textContent: "" }), addEventListener() {} },
    fetch: async (url, options = {}) => {
      requests.push({ url, options });
      if (url === "protocol.json") return { ok: true, json: async () => ({ status: "active", network: "base-mainnet", chain_id: 8453, native_usdc: usdc, mcp_base_url: "https://mcp.agentbounties.app" }) };
      if (String(url).includes("/checkout")) {
        if (checkoutError) throw new Error("connection lost");
        return { ok: false, status: 503, json: async () => ({ message: "Partner unavailable" }) };
      }
      const { method } = JSON.parse(options.body);
      return { ok: true, json: async () => ({ result: { eth_chainId: "0x2105", eth_blockNumber: "0x123", eth_getBalance: "0x12", eth_call: balance }[method] }) };
    },
  });
  runScript(context, "funding-readiness.js");
  runScript(context, "moonpay-onramp.js");
  await new Promise((resolve) => setTimeout(resolve, 10));
  return { context, element, opened, requests, saved };
}

test("handoff reads destination balance and shows the exact shortfall without a guessed USD minimum", async () => {
  const { element, requests } = await onrampContext();
  assert.equal(element("[data-usdc-shortfall]").textContent, "3 USDC");
  assert.equal(element("[data-fiat-amount]").value, "");
  assert.match(element("[data-quote-guidance]").textContent, /receive at least 3 USDC/);
  assert.equal(requests.filter(({ url }) => String(url).includes("/checkout")).length, 0);
});

test("an open purchase is reused and blocks duplicates across reloads until explicitly resolved", async () => {
  const first = await onrampContext();
  first.element("[data-onramp-ack]").checked = true;
  first.context.window.AgentBountiesOnramp.openDirectCheckout();
  assert.equal(first.opened.length, 1);
  assert.equal(first.opened[0].target, "agent-bounties-wallet-topup");
  assert.equal(first.opened[0].opener, null);
  assert.equal(first.opened[0].url, "https://www.moonpay.com/buy/usdc");
  assert.throws(() => first.context.window.AgentBountiesOnramp.openDirectCheckout(), /existing purchase/);
  const metadata = [...first.saved.values()].join("");
  assert.match(metadata, /posting-123/);
  assert.equal(metadata.includes("https:"), false);
  const second = await onrampContext({ saved: first.saved });
  second.element("[data-onramp-ack]").checked = true;
  assert.throws(() => second.context.window.AgentBountiesOnramp.openDirectCheckout(), /existing purchase/);
  assert.equal(second.opened.length, 0);
  second.element("[data-purchase-resolved]").checked = true;
  second.element("[data-clear-purchase]").handlers.click();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(second.context.window.AgentBountiesOnramp.hasPendingPurchase(), false);
  assert.equal(second.element("[data-onramp-ack]").checked, false);
});

test("an uncertain checkout response cannot trigger a repeated request", async () => {
  const app = await onrampContext({ contract: `0x${"b".repeat(40)}`, checkoutError: true });
  app.element("[data-fiat-amount]").value = "5.00";
  app.element("[data-onramp-ack]").checked = true;
  app.element("[data-start-moonpay]").handlers.click();
  app.element("[data-start-moonpay]").handlers.click();
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(app.requests.filter(({ url }) => String(url).includes("/checkout")).length, 1);
  assert.equal(app.context.window.AgentBountiesOnramp.hasPendingPurchase(), true);
  assert.equal(app.element("[data-purchase-recovery]").hidden, false);
  assert.match(app.element("[data-purchase-recovery-copy]").textContent, /status is unverified/);
});
