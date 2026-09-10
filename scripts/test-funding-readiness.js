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

test("wallet review distinguishes an allowance from the later funding transfer", () => {
  const api = helperContext();
  const factory = `0x${"b".repeat(40)}`, bounty = `0x${"c".repeat(40)}`;
  const word = (value) => BigInt(value).toString(16).padStart(64, "0");
  const allowance = { to: usdc, value_wei: 0, data: `0x095ea7b3${factory.slice(2).padStart(64, "0")}${word(5000000)}` };
  const args = Array(19).fill(word(0)); args[7] = word(1800000000); args[14] = word(17 * 32); args[15] = word(5000000); args[17] = word(1); args[18] = wallet.slice(2).padStart(64, "0");
  const creation = { to: factory, value_wei: 0, data: `0x9d2e414c${args.join("")}` };
  const context = { chainId: 8453, usdcAddress: usdc, factoryAddress: factory, bountyAddress: bounty, fundingUsdcUnits: 5000000n, validatedCalls: [allowance, creation] };
  const first = api.describeWalletCalls({ calls: [allowance], context });
  assert.equal(first.calls[0].kind, "allowance");
  assert.equal(first.calls[0].spender, factory);
  assert.equal(first.transferUsdcUnits, 0n);
  assert.match(first.summary, /No USDC moves/);
  assert.match(first.summary, /no automatic expiry/);
  const batch = api.describeWalletCalls({ calls: [allowance, creation], context });
  assert.equal(batch.transferUsdcUnits, 5000000n);
  assert.equal(batch.calls[1].recipient, bounty);
  assert.equal(batch.calls[1].expiresAt, "2027-01-15T08:00:00.000Z");
  assert.match(batch.summary, /through canonical factory/);
  assert.throws(() => api.describeWalletCalls({ calls: [{ ...allowance, data: allowance.data.slice(0, -64) + word(2n ** 256n - 1n) }], context }), /differs from/);
  const over = { ...allowance, data: allowance.data.slice(0, -64) + word(6000000) };
  assert.throws(() => api.describeWalletCalls({ calls: [over], context: { ...context, validatedCalls: [over] } }), /exceeds/);
});

test("wallet review uses decoded authorization expiry and requires matching validated calls", () => {
  const api = helperContext();
  const factory = `0x${"b".repeat(40)}`, bounty = `0x${"c".repeat(40)}`, registry = `0x${"d".repeat(40)}`;
  const word = (value) => BigInt(value).toString(16).padStart(64, "0");
  const args = Array(26).fill(word(0)); args[0] = wallet.slice(2).padStart(64, "0"); args[8] = word(1800000000); args[15] = word(24 * 32); args[16] = word(5000000); args[19] = word(1790000000); args[24] = word(1); args[25] = wallet.slice(2).padStart(64, "0");
  const authorized = { to: factory, value_wei: 0, data: `0x61407894${args.join("")}` };
  const publish = { to: registry, value_wei: 0, data: `0x16d0f49a${word(0)}` };
  const context = { chainId: 8453, usdcAddress: usdc, factoryAddress: factory, creatorAddress: wallet, bountyAddress: bounty, fundingUsdcUnits: 5000000n, termsRegistry: registry, validatedCalls: [authorized, publish] };
  const description = api.describeWalletCalls({ calls: [authorized], context });
  assert.equal(description.calls[0].expiresAt, new Date(1790000000 * 1000).toISOString());
  assert.match(description.summary, /authorization expires/);
  const terms = api.describeWalletCalls({ calls: [publish], context });
  assert.equal(terms.transferUsdcUnits, 0n);
  assert.match(terms.summary, /does not create or fund/);
  assert.throws(() => api.describeWalletCalls({ calls: [authorized], context: { ...context, validatedCalls: [] } }), /differs from/);
  assert.throws(() => api.describeWalletCalls({ calls: [authorized], context: { ...context, chainId: 1 } }), /validated Base/);
  const unknown = { to: factory, data: "0x12345678", value_wei: 0 };
  assert.throws(() => api.describeWalletCalls({ calls: [unknown], context: { ...context, validatedCalls: [unknown] } }), /not a recognized/);
});

function authorizationFixture() {
  const bounty = `0x${"c".repeat(40)}`;
  const nonce = `0x${"ef".repeat(32)}`;
  const typedData = {
    types: {
      EIP712Domain: [{ name: "name", type: "string" }, { name: "version", type: "string" }, { name: "chainId", type: "uint256" }, { name: "verifyingContract", type: "address" }],
      TransferWithAuthorization: [{ name: "from", type: "address" }, { name: "to", type: "address" }, { name: "value", type: "uint256" }, { name: "validAfter", type: "uint256" }, { name: "validBefore", type: "uint256" }, { name: "nonce", type: "bytes32" }],
    },
    domain: { name: "USD Coin", version: "2", chainId: 8453, verifyingContract: usdc },
    primaryType: "TransferWithAuthorization",
    message: { from: wallet, to: bounty, value: "5000000", validAfter: "0", validBefore: "1800000000", nonce },
  };
  const context = { chainId: 8453, usdcAddress: usdc, creatorAddress: wallet, bountyAddress: bounty, fundingUsdcUnits: 5000000n, creationNonce: nonce, fundingDeadline: 1800000000 };
  return { typedData, context, nowMs: Date.parse("2026-09-10T00:00:00Z") };
}

test("a funding authorization is disclosed and serialized only after all approved bindings match", () => {
  const api = helperContext();
  const fixture = authorizationFixture();
  const checked = api.validateFundingAuthorization(fixture);
  assert.equal(checked.to, fixture.context.bountyAddress);
  assert.equal(checked.from, wallet);
  assert.equal(checked.amountUsdcUnits, 5000000n);
  assert.equal(checked.expiresAt, "2027-01-15T08:00:00.000Z");
  assert.match(checked.summary, /Authorize 5 USDC on Base mainnet \(8453\)/);
  assert.match(checked.summary, /signature costs no gas/);
  assert.deepEqual(JSON.parse(checked.serialized), fixture.typedData);
  fixture.typedData.message.value = "6000000";
  assert.equal(JSON.parse(checked.serialized).message.value, "5000000", "the wallet receives the serialized checked payload even if the planner object later changes");
});

test("changed EIP3009 identity, economics, expiry or exact types cannot request a funding signature", () => {
  const api = helperContext();
  const mutations = [
    (data) => { data.primaryType = "ReceiveWithAuthorization"; },
    (data) => { data.domain.name = "USDC"; },
    (data) => { data.domain.version = "1"; },
    (data) => { data.domain.chainId = 1; },
    (data) => { data.domain.verifyingContract = wallet; },
    (data) => { data.message.from = data.message.to; },
    (data) => { data.message.to = wallet; },
    (data) => { data.message.value = "5000001"; },
    (data) => { data.message.value = Number.MAX_SAFE_INTEGER + 1; },
    (data) => { data.message.validAfter = "1"; },
    (data) => { data.message.validBefore = "1800000001"; },
    (data) => { data.message.nonce = `0x${"ab".repeat(32)}`; },
    (data) => { data.types.TransferWithAuthorization[2].type = "uint128"; },
    (data) => { data.types.TransferWithAuthorization.reverse(); },
    (data) => { data.types.EIP712Domain[0].name = "unexpected"; },
    (data) => { data.types.Unreviewed = []; },
    (data) => { data.message.extra = "unreviewed"; },
    (data) => { data.domain.salt = `0x${"01".repeat(32)}`; },
  ];
  for (const mutate of mutations) {
    const fixture = authorizationFixture(); mutate(fixture.typedData);
    assert.throws(() => api.validateFundingAuthorization(fixture), /authorization does not match/);
  }
  const expired = authorizationFixture(); expired.nowMs = 1800000000 * 1000;
  assert.throws(() => api.validateFundingAuthorization(expired), /authorization does not match/);
  const zero = authorizationFixture(); zero.typedData.message.value = "0"; zero.context.fundingUsdcUnits = 0n;
  assert.throws(() => api.validateFundingAuthorization(zero), /authorization does not match/);
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
