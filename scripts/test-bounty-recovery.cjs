"use strict";
const assert = require("node:assert/strict");
const { test } = require("node:test");
const fs = require("node:fs"), vm = require("node:vm"), path = require("node:path");
const scope = { window: {}, TextEncoder, crypto: require("node:crypto").webcrypto };
vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../site/evm.js"), "utf8"), scope);
const evm = scope.window.AgentBountiesEvm;
const core = require("../site/bounty-recovery.js")(evm);
const OWNER = "0x" + "1".repeat(40), OTHER = "0x" + "2".repeat(40), BOUNTY = "0x" + "3".repeat(40);
const ID = "0x" + "4".repeat(64), TX = "0x" + "5".repeat(64), BLOCK = "0x" + "6".repeat(64);
const word = n => "0x" + evm.uint256Word(n), addr = a => "0x" + evm.addressWord(a);
function fixture() {
  const data = new Map(), held = new Set();
  const storage = { getItem: k => data.get(k) || null, setItem: (k, v) => data.set(k, v), removeItem: k => data.delete(k) };
  const locks = { request: async (name, _options, fn) => { if (held.has(name)) return fn(null); held.add(name); try { return await fn({ name }); } finally { held.delete(name); } } };
  const f = { state: 1, contribution: 17068098n, funded: 17068098n, chain: core.CHAIN, walletChain: core.CHAIN, account: OWNER, code: core.CODE, canonical: 1n, receipt: null, safe: "0x64", calls: [], sends: [], stored: data, storage, locks, estimated: 0 };
  f.rpc = async (method, params) => {
    f.calls.push({ method, params });
    if (method === "eth_chainId") return f.chain;
    if (method === "eth_getBlockByNumber") return { number: params[0] === "safe" || params[0] === "latest" ? f.safe : params[0], hash: f.blockHash || BLOCK };
    if (method === "eth_getCode") return f.code;
    if (method === "eth_estimateGas") { f.estimated++; if (f.simulationFailure) throw new Error("simulation failed"); return "0x122be"; }
    if (method === "eth_getTransactionReceipt") return f.receipt;
    if (method === "eth_getTransactionByHash") return f.transaction;
    if (method === "eth_call") {
      const signature = params[0].data.slice(0, 10);
      const values = { "factory()": addr(core.FACTORY), "settlementToken()": addr(core.TOKEN), "creator()": addr(OWNER), "bountyId()": ID, "status()": word(f.latestState !== undefined && params[1] === f.safe && f.planned ? f.latestState : f.state), "fundedAmount()": word(f.funded), "timeoutBondPool()": word(0), "isCanonicalBounty(address)": word(f.canonical), "contributions(address)": word(f.contribution) };
      for (const [name, value] of Object.entries(values)) if (core.selector(name) === signature) return value;
      throw new Error("Unexpected chain read " + signature);
    }
    throw new Error("Unexpected RPC " + method);
  };
  f.plan = async (action, contract, account) => { f.planned = true; return { from: account, to: contract, value_wei: 0, data: core.selector(action === "cancel" ? "cancel()" : "withdrawRefund()"), ...f.badPlan }; };
  f.provider = { request: async ({ method, params }) => {
    if (method === "eth_chainId") return f.walletChain;
    if (method === "eth_accounts") return [f.account];
    if (method === "eth_sendTransaction") {
      f.sends.push(params[0]);
      assert.ok([...data.values()].some(value => JSON.parse(value).txHash === null), "Persist before invoking wallet");
      if (f.sendError) throw f.sendError;
      if (f.waitSend) await f.waitSend;
      f.transaction = { hash: TX, ...params[0], input: params[0].data };
      return TX;
    }
    throw new Error("Unexpected wallet method " + method);
  } };
  f.engine = () => core.create(f);
  f.request = (engine = f.engine(), action = "cancel", account = OWNER) => engine.request({ contract: BOUNTY, account, action, provider: f.provider });
  f.confirm = (action = "cancel", overrides = {}) => {
    const refund = action === "refund";
    f.receipt = { transactionHash: TX, blockNumber: "0x64", blockHash: BLOCK, status: "0x1", logs: [{ address: BOUNTY, topics: [core.topic(refund ? "RefundWithdrawn(bytes32,address,uint256,uint256,uint256)" : "BountyCancelled(bytes32,uint256)"), ID, ...(refund ? [addr(OWNER)] : [])], data: refund ? "0x" + evm.uint256Word(17068098n) + evm.uint256Word(0) + evm.uint256Word(17068098n) : word(0) }], ...overrides };
    f.state = 5;
    if (refund) { f.contribution = 0n; f.funded = 0n; }
  };
  return f;
}
test("an already connected wallet does not receive another connection prompt", async () => {
  const calls = [], provider = { request: async ({ method }) => { calls.push(method); return [OWNER]; } };
  assert.equal(await core.connectWallet(provider), OWNER);
  assert.deepEqual(calls, ["eth_accounts"]);
});
test("a stalled wallet read times out before any prompt, plan or send", async () => {
  const f = fixture(); f.walletReadTimeoutMs = 5;
  f.provider.request = () => new Promise(() => {});
  await assert.rejects(f.request(), /wallet did not answer/);
  assert.equal(f.planned, undefined); assert.equal(f.sends.length, 0); assert.equal(f.stored.size, 0);
  assert.throws(() => core.walletRead(f.provider, "eth_sendTransaction"), /Only wallet connection reads/);
});
test("connection timeout and concurrent retry never duplicate the wallet prompt", async () => {
  let resolve, prompts = 0, connected = false;
  const provider = { request: ({ method }) => {
    if (method === "eth_accounts") return Promise.resolve(connected ? [OWNER] : []);
    assert.equal(method, "eth_requestAccounts"); prompts++;
    return new Promise(done => { resolve = done; });
  } };
  await assert.rejects(core.connectWallet(provider, 5), /finish its connection request/);
  const next = core.connectWallet(provider, 100), concurrent = core.connectWallet(provider, 100);
  await new Promise(done => setImmediate(done)); connected = true; resolve([OWNER]);
  assert.deepEqual(await Promise.all([next, concurrent]), [OWNER, OWNER]); assert.equal(prompts, 1);
  assert.equal(await core.connectWallet(provider), OWNER); assert.equal(prompts, 1);
});
test("rejected and malformed account connections cannot become a connected wallet", async () => {
  for (const accounts of [null, {}, [], ["invalid"]]) {
    const provider = { request: async ({ method }) => method === "eth_accounts" ? [] : accounts };
    await assert.rejects(core.connectWallet(provider));
  }
  let reject = true, prompts = 0;
  const provider = { request: async ({ method }) => {
    if (method === "eth_accounts") return [];
    prompts++; if (reject) throw Object.assign(new Error("rejected"), { code: 4001 }); return [OWNER];
  } };
  await assert.rejects(core.connectWallet(provider), { code: 4001 }); reject = false;
  assert.equal(await core.connectWallet(provider), OWNER); assert.equal(prompts, 2);
});
test("a stalled network switch times out and reuses the same prompt on retry", async () => {
  let finish, calls = 0;
  const provider = { request: ({ method, params }) => {
    calls++; assert.equal(method, "wallet_switchEthereumChain"); assert.deepEqual(params, [{ chainId: core.CHAIN }]);
    return new Promise(resolve => { finish = resolve; });
  } };
  await assert.rejects(core.switchToBase(provider, 5), /switching to Base/);
  const retry = core.switchToBase(provider, 100); finish(null); await retry; assert.equal(calls, 1);
});
test("read-only preparation never requests a signature", async () => {
  const f = fixture(), view = await f.engine().snapshot(BOUNTY, OWNER);
  assert.equal(view.funded, "17068098"); assert.equal(f.sends.length, 0);
  assert.ok(f.calls.filter(x => x.method === "eth_call").every(x => x.params[1] === "0x64"));
});
test("cancel, wait for safe event, then separately refund to the original contributor", async () => {
  const f = fixture(), engine = f.engine();
  await f.request(engine);
  assert.deepEqual(f.sends[0], { from: OWNER, to: BOUNTY, value: "0x0", data: "0xea8a1af0", chainId: "0x2105" });
  assert.equal((await engine.reconcile(BOUNTY, OWNER)).status, "pending");
  await assert.rejects(f.request(engine), /previous wallet request/);
  f.confirm(); f.receipt.blockNumber = "0x65";
  assert.equal((await engine.reconcile(BOUNTY, OWNER)).status, "pending");
  f.safe = "0x65";
  assert.equal((await engine.reconcile(BOUNTY, OWNER)).action, "cancel");
  await f.request(engine, "refund");
  assert.equal(f.sends[1].data, core.selector("withdrawRefund()"));
  f.confirm("refund");
  const result = await engine.reconcile(BOUNTY, OWNER);
  assert.equal(result.status, "confirmed"); assert.equal(result.amount, "17068098");
  await assert.rejects(f.request(engine, "refund"), /remaining contribution/);
  assert.equal(f.sends.length, 2);
});
for (const state of [2, 3, 4, 5]) test(`state ${state} cannot be cancelled`, async () => {
  const f = fixture(); f.state = state;
  await assert.rejects(f.request(), /cannot be cancelled/); assert.equal(f.sends.length, 0);
});
test("wrong owner, chain, counterfeit clone and noncanonical contract fail closed", async () => {
  for (const change of [{ account: OTHER }, { walletChain: "0x1" }, { chain: "0x1" }, { code: "0x1234" }, { canonical: 0n }]) {
    const f = Object.assign(fixture(), change); await assert.rejects(f.request()); assert.equal(f.sends.length, 0);
  }
  const f = fixture(); f.account = OTHER;
  await assert.rejects(f.request(f.engine(), "cancel", OTHER), /created this bounty/);
});
test("malicious plan cannot change recipient, sender, value or function", async () => {
  for (const badPlan of [{ to: OTHER }, { from: OTHER }, { value_wei: 1 }, { data: core.selector("claim()") }]) {
    const f = fixture(); f.badPlan = badPlan;
    await assert.rejects(f.request(), /does not match/); assert.equal(f.sends.length, 0);
  }
});
test("fresh state and simulation are checked before opening wallet", async () => {
  const f = fixture(); f.latestState = 2;
  await assert.rejects(f.request(), /cannot be cancelled/); assert.equal(f.sends.length, 0);
  const g = fixture(); g.simulationFailure = true;
  await assert.rejects(g.request(), /simulation failed/); assert.equal(g.sends.length, 0);
});
test("lost wallet response survives reload and prevents repeat requests", async () => {
  const f = fixture(); f.sendError = new Error("connection lost");
  await assert.rejects(f.request(), /connection lost/);
  const reloaded = f.engine(); assert.equal((await reloaded.reconcile(BOUNTY, OWNER)).status, "unknown");
  await assert.rejects(f.request(reloaded), /previous wallet request/); assert.equal(f.sends.length, 1);
});
test("explicit wallet rejection permits a new reviewed request", async () => {
  const f = fixture(); f.sendError = Object.assign(new Error("rejected"), { code: 4001 });
  await assert.rejects(f.request(), /rejected/); assert.equal(f.engine().saved(BOUNTY, OWNER), null);
});
test("storage failure and concurrent tabs cannot open a second wallet request", async () => {
  const f = fixture(); f.storage.setItem = () => { throw new Error("storage blocked"); };
  await assert.rejects(f.request(), /storage blocked/); assert.equal(f.sends.length, 0);
  const g = fixture(); let release; g.waitSend = new Promise(resolve => { release = resolve; });
  const first = g.request();
  await assert.rejects(g.request(), /another tab/);
  release(); await first; assert.equal(g.sends.length, 1);
});
test("reorgs, missing events, wrong refund recipients and mismatched transactions never confirm", async () => {
  const f = fixture(), e = f.engine(); await f.request(e); f.confirm();
  f.blockHash = "0x" + "7".repeat(64); assert.equal((await e.reconcile(BOUNTY, OWNER)).status, "pending");
  f.blockHash = BLOCK; f.receipt.logs = []; await assert.rejects(e.reconcile(BOUNTY, OWNER), /event is missing/);
  f.confirm(); f.transaction.to = OTHER; await assert.rejects(e.reconcile(BOUNTY, OWNER), /does not match/);
  const g = fixture(), e2 = g.engine(); g.state = 5; await g.request(e2, "refund"); g.confirm("refund");
  g.receipt.logs[0].topics[2] = addr(OTHER); await assert.rejects(e2.reconcile(BOUNTY, OWNER), /does not belong/);
});
test("a safely confirmed revert permits a retry without claiming completion", async () => {
  const f = fixture(), e = f.engine(); await f.request(e); f.confirm("cancel", { status: "0x0", logs: [] });
  assert.equal((await e.reconcile(BOUNTY, OWNER)).status, "failed"); assert.equal(e.saved(BOUNTY, OWNER), null);
});

test("RPC retries rate limits once on the pinned fallback and serializes reads", async () => {
  const calls = []; let active = 0;
  const rpc = core.createRpc(async (url, init) => {
    assert.equal(++active, 1); await new Promise(resolve => setTimeout(resolve, 2)); active--;
    const body = JSON.parse(init.body); calls.push(url);
    return url.includes("publicnode") ? { status: 429 } : { status: 200, ok: true, json: async () => ({ ...body, result: core.CHAIN }) };
  });
  assert.deepEqual(await Promise.all([rpc("eth_chainId", []), rpc("eth_chainId", [])]), [core.CHAIN, core.CHAIN]);
  assert.deepEqual(calls, ["https://base-rpc.publicnode.com", "https://mainnet.base.org", "https://mainnet.base.org"]);
  await assert.rejects(rpc("eth_sendTransaction", []), /only supports reads/);
  assert.equal(calls.length, 3);
});
test("RPC never retries a simulation revert or invalid response", async () => {
  for (const result of [{ error: { code: 3, message: "execution reverted" } }, { id: 999, result: "0x1" }]) {
    let calls = 0;
    const rpc = core.createRpc(async (_, init) => { calls++; return { status: 200, ok: true, json: async () => ({ ...JSON.parse(init.body), ...result }) }; });
    await assert.rejects(rpc("eth_estimateGas", []), /could not verify/); assert.equal(calls, 1);
  }
});
test("RPC unavailability is bounded and snapshot rejects a changed block", async () => {
  let calls = 0;
  const rpc = core.createRpc(async () => { calls++; throw new Error("offline"); });
  await assert.rejects(rpc("eth_chainId", []), /busy or unavailable/); assert.equal(calls, 2);
  const f = fixture(); const read = f.rpc;
  f.rpc = async (method, params) => method === "eth_getBlockByNumber" && params[0] === "0x64" ? { number: "0x64", hash: ID } : read(method, params);
  await assert.rejects(f.engine().snapshot(BOUNTY, OWNER), /block changed/);
});
test("restricted receipt history falls back without repeating a wallet request", async () => {
  for (const status of [401, 403]) {
    const calls = [];
    const rpc = core.createRpc(async (url, init) => {
      const body = JSON.parse(init.body); calls.push([url, body.method]);
      return url.includes("publicnode") ? { status } : { status: 200, ok: true, json: async () => ({ jsonrpc: "2.0", id: body.id, result: { status: "0x1" } }) };
    });
    assert.deepEqual(await rpc("eth_getTransactionReceipt", [TX]), { status: "0x1" });
    assert.equal(calls.length, 2); assert.ok(calls.every(x => x[1] === "eth_getTransactionReceipt"));
  }
});
test("an old receipt cannot erase a newer pending wallet request", async () => {
  const f = fixture(), e = f.engine(); await f.request(e); f.confirm();
  const key = [...f.stored.keys()][0], read = f.rpc;
  let newer;
  f.rpc = async (method, params) => {
    if (method === "eth_getTransactionByHash") {
      newer = JSON.stringify({ ...JSON.parse(f.stored.get(key)), action: "refund", txHash: null });
      f.stored.set(key, newer);
    }
    return read(method, params);
  };
  await f.engine().reconcile(BOUNTY, OWNER);
  assert.equal(f.stored.get(key), newer);
});
test("wallet approval progress begins only after the exact request is saved", async () => {
  const f = fixture(), stages = [];
  await f.engine().request({ contract: BOUNTY, account: OWNER, action: "cancel", provider: f.provider, onProgress: stage => {
    stages.push(stage);
    if (stage === "wallet_approval") assert.ok([...f.stored.values()].some(x => JSON.parse(x).txHash === null));
  } });
  assert.deepEqual(stages, ["wallet_check", "bounty_check", "wallet_approval"]); assert.equal(f.sends.length, 1);
});

async function pageFixture(change = () => {}) {
  const f = fixture(), nodes = new Map(), events = new Map();
  const node = id => {
    if (!nodes.has(id)) nodes.set(id, { textContent: "", hidden: false, disabled: false, addEventListener: (type, fn) => events.set(`${id}:${type}`, fn) });
    return nodes.get(id);
  };
  change(f);
  const pageCore = { ...core, create: () => f.engine(), connectWallet: provider => core.connectWallet(provider, 10, 5), walletRead: (provider, method) => core.walletRead(provider, method, 5) };
  const timers = [];
  vm.runInNewContext(fs.readFileSync(path.join(__dirname, "../site/bounty-recovery-page.js"), "utf8"), {
    window: { AgentBountiesRecovery: pageCore, AgentBountiesWalletLink: { select: async options => { assert.equal(options.purpose, "recovery"); return { label: "MetaMask", kind: "browser", provider: f.provider }; } } },
    document: { getElementById: node }, localStorage: f.storage, navigator: { locks: f.locks },
    location: { search: `?bountyContract=${BOUNTY}` }, URLSearchParams, AbortSignal,
    fetch: () => { throw new Error("No network in page fixture"); },
    setTimeout: (fn, ms) => { const t = setTimeout(fn, ms); t.unref(); timers.push(t); return t; }, clearTimeout,
  });
  await new Promise(done => setImmediate(done));
  return { f, node, click: id => events.get(`recovery-${id}:click`)({ isTrusted: true }), close: () => timers.forEach(clearTimeout) };
}
test("recovery page connects the already-authorized extension and shows its balance", async () => {
  const p = await pageFixture(); await p.click("connect");
  assert.equal(p.node("recovery-wallet").textContent, OWNER);
  assert.equal(p.node("recovery-send").disabled, false); assert.equal(p.f.sends.length, 0);
  p.close();
});
test("stalled connection unlocks the UI and gives an actionable error", async () => {
  const p = await pageFixture(f => { f.provider.request = () => new Promise(() => {}); });
  await p.click("connect");
  assert.match(p.node("recovery-status").textContent, /Open and unlock/);
  assert.equal(p.node("recovery-connect").disabled, false);
  assert.equal(p.node("recovery-refresh").disabled, false);
  assert.equal(p.node("recovery-send").disabled, true); assert.equal(p.f.sends.length, 0); p.close();
});
test("an existing MetaMask connection prompt explains where to finish it", async () => {
  const p = await pageFixture(f => { f.provider.request = async ({ method }) => {
    if (method === "eth_accounts") return [];
    throw Object.assign(new Error("Already pending"), { code: -32002 });
  }; });
  await p.click("connect"); assert.match(p.node("recovery-status").textContent, /already open in your wallet/);
  assert.equal(p.node("recovery-connect").disabled, false); assert.equal(p.node("recovery-send").disabled, true); p.close();
});
test("check status stays available during wallet approval without another send", async () => {
  let finish;
  const p = await pageFixture(f => { f.waitSend = new Promise(resolve => { finish = resolve; }); });
  await p.click("connect"); const sending = p.click("send"); await new Promise(done => setImmediate(done));
  assert.match(p.node("recovery-status").textContent, /Open your wallet to confirm/);
  assert.equal(p.node("recovery-refresh").disabled, false); assert.equal(p.node("recovery-send").disabled, true);
  await p.click("refresh"); assert.match(p.node("recovery-status").textContent, /Do not send it again/);
  assert.equal(p.f.sends.length, 1); finish(); await sending; p.close();
});

if (process.env.RECOVERY_BROWSER_TEST === "1") test("real browser: review, reload pending cancellation, refund, mobile layout", async () => {
  const http = require("node:http");
  const { chromium } = require("playwright");
  const site = path.resolve(__dirname, "../site");
  const server = http.createServer((req, res) => {
    const filename = path.resolve(site, "." + new URL(req.url, "http://localhost").pathname);
    if (!filename.startsWith(site + path.sep)) return res.writeHead(403).end();
    try { const content = fs.readFileSync(filename); res.writeHead(200, { "content-type": ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css" })[path.extname(filename)] || "application/octet-stream" }).end(content); }
    catch (_) { res.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    for (const width of [390, 1280]) {
      const f = fixture(), errors = [];
      const context = await browser.newContext({ viewport: { width, height: 900 } });
      await context.route("**/*", async route => {
        const url = new URL(route.request().url());
        if (["https://mainnet.base.org", "https://base-rpc.publicnode.com"].includes(url.origin)) {
          if (url.origin === "https://base-rpc.publicnode.com") return route.fulfill({ status: 429, body: "busy" });
          const body = route.request().postDataJSON();
          return route.fulfill({ json: { jsonrpc: "2.0", id: body.id, result: await f.rpc(body.method, body.params) } });
        }
        if (url.pathname.endsWith("cancel-plan") || url.pathname.endsWith("refund-withdrawal-plan")) {
          const body = route.request().postDataJSON();
          return route.fulfill({ json: await f.plan(url.pathname.endsWith("cancel-plan") ? "cancel" : "refund", body.bounty_contract, body.caller) });
        }
        if (url.origin !== origin) return route.abort();
        return route.continue();
      });
      await context.exposeBinding("recoveryWalletFixture", async (_, { method, params }) => {
        if (method === "eth_accounts" || method === "eth_requestAccounts") return [OWNER];
        if (method === "eth_chainId") return core.CHAIN;
        assert.equal(method, "eth_sendTransaction");
        f.sends.push(params[0]); f.transaction = { hash: TX, ...params[0], input: params[0].data };
        return TX;
      });
      await context.addInitScript(() => {
        window.ethereum = { isMetaMask: true, on: () => {}, request: args => window.recoveryWalletFixture(args) };
        const provider = { isMetaMask: true, on: () => {}, request: args => window.recoveryWalletFixture(args) };
        const brave = { isBraveWallet: true, isMetaMask: true, request: () => { throw new Error("Wrong wallet selected"); } };
        window.addEventListener("eip6963:requestProvider", () => {
          for (const detail of [
            { provider, info: { name: "MetaMask", rdns: "io.metamask", uuid: "11111111-1111-4111-8111-111111111111" } },
            { provider: brave, info: { name: "Brave Wallet", rdns: "com.brave.wallet", uuid: "22222222-2222-4222-8222-222222222222" } },
          ]) window.dispatchEvent(new CustomEvent("eip6963:announceProvider", { detail }));
        });
      });
      const page = await context.newPage(); page.on("pageerror", e => errors.push(e.message));
      await page.goto(`${origin}/recover-bounty.html?bountyContract=${BOUNTY}&analytics=off`);
      await page.getByRole("button", { name: "Connect my wallet", exact: true }).click();
      assert.equal(await page.getByRole("button", { name: /MetaMask in this browser/ }).count(), 1);
      assert.equal(await page.getByRole("button", { name: /Brave Wallet in this browser/ }).count(), 1);
      assert.equal(await page.getByRole("dialog").getByText(/Buy USDC/).count(), 0);
      if (process.env.RECOVERY_SCREENSHOT_DIR) await page.screenshot({ path: path.join(process.env.RECOVERY_SCREENSHOT_DIR, `wallet-picker-${width}.png`) });
      await page.getByRole("button", { name: /MetaMask/ }).click();
      await page.getByText("Ready to cancel.", { exact: false }).waitFor();
      assert.equal(f.sends.length, 0);
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      if (process.env.RECOVERY_SCREENSHOT_DIR) await page.screenshot({ path: path.join(process.env.RECOVERY_SCREENSHOT_DIR, `recovery-${width}.png`), fullPage: true });
      await page.getByRole("button", { name: "Cancel old bounty", exact: true }).click();
      await page.getByText("Your transaction was sent.", { exact: false }).waitFor();
      assert.equal(f.sends.length, 1);
      await page.reload();
      await page.getByRole("button", { name: "Connect my wallet", exact: true }).click();
      await page.getByRole("button", { name: /MetaMask/ }).click();
      await page.getByText("Your transaction was sent.", { exact: false }).waitFor();
      assert.equal(await page.getByRole("button", { name: "Cancel old bounty", exact: true }).isEnabled(), false);
      assert.equal(f.sends.length, 1);
      f.confirm();
      await page.getByRole("button", { name: "Check status", exact: true }).click();
      await page.getByText("Cancellation confirmed. Choose Return my funds", { exact: false }).waitFor();
      f.receipt = null;
      await page.getByRole("button", { name: "Return my funds", exact: true }).click();
      await page.getByText("Your transaction was sent.", { exact: false }).waitFor();
      f.confirm("refund");
      await page.getByRole("button", { name: "Check status", exact: true }).click();
      await page.getByText("17.068098 USDC returned", { exact: false }).waitFor();
      assert.equal(f.sends.length, 2); assert.deepEqual(errors, []);
      await context.close();
    }
  } finally { if (browser) await browser.close(); await new Promise(resolve => server.close(resolve)); }
});
