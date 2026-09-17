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
      });
      const page = await context.newPage(); page.on("pageerror", e => errors.push(e.message));
      await page.goto(`${origin}/recover-bounty.html?bountyContract=${BOUNTY}&analytics=off`);
      await page.getByRole("button", { name: "Connect my wallet", exact: true }).click();
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
