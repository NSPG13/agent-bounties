import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { readFile, mkdir } from "node:fs/promises";
import { build } from "esbuild";
import { fakeSdk } from "./test-auth-sdk.mjs";
import { createServer } from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../site");
const ADDRESS = "0x" + "11".repeat(20);
const EMBEDDED = "0x" + "22".repeat(20);
let browser, server, origin, testAdapter;
before(async () => {
  const built = await build({
    entryPoints: [path.resolve(root, '../tools/coinbase-embedded-wallet/src/index.js')],
    bundle: true, write: false, format: 'iife', platform: 'browser',
    define: { 'process.env.NODE_ENV': '"production"' },
    plugins: [{ name: 'test-cdp-boundary', setup(build) {
      build.onResolve({ filter: /^@coinbase\/cdp-/ }, () => ({ path: 'fake-cdp', namespace: 'test-sdk' }));
      build.onLoad({ filter: /.*/, namespace: 'test-sdk' }, () => ({ contents: fakeSdk, loader: 'js', resolveDir: path.resolve(root, '../tools/coinbase-embedded-wallet') }));
    } }],
  });
  testAdapter = built.outputFiles[0].text;
  server = createServer(async (request, response) => {
    const pathname = decodeURIComponent(new URL(request.url, "http://localhost").pathname);
    const filename = path.resolve(root, `.${pathname === "/" ? "/index.html" : pathname}`);
    if (!filename.startsWith(root + path.sep)) { response.writeHead(403).end(); return; }
    try {
      let body = await readFile(filename);
      if (pathname === "/wallet-config.js") body = Buffer.from(body.toString().replace("__COINBASE_CDP_PROJECT_ID__", "qa-wallet-project"));
      response.setHeader("Content-Type", ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".webp": "image/webp" })[path.extname(filename)] || "application/octet-stream");
      response.end(body);
    } catch { response.writeHead(404).end(); }
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  origin = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({ headless: true });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise((resolve) => server.close(resolve));
});

async function openAccount(page) {
  const accountLink = page.getByRole("link", { name: "Account", exact: true });
  const menu = page.getByRole("button", { name: "Menu", exact: false });
  if (await menu.isVisible() && await menu.getAttribute("aria-expanded") !== "true") await menu.click();
  await accountLink.click();
}

async function account({ installed = true, linked = false, mobile = false, adapter = false, pending = null, failVerify = false, failRefresh = false } = {}) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const proofs = [], errors = [];
  let wallets = linked ? [{ address: ADDRESS }] : [];
  let rejectVerification = failVerify;
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("https://**/*", (route) => route.fulfill({ json: {} }));
  await page.route("**/auth/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let body = {};
    if (pathname.endsWith("/session")) body = { authenticated: true, user: { id: "qa", name: "Test member", provider: "google" }, providers: { google: true } };
    if (pathname.endsWith("/account")) {
      if (failRefresh && wallets.some(wallet => wallet.address === EMBEDDED)) { await route.fulfill({status:503,json:{}}); return; }
      body = { wallets, available: false };
    }
    if (pathname.endsWith("/wallet/challenge")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      body = { challenge_id: "ownership-only", message: `Agent Bounties wallet ownership verification\n\nSign this message to link the wallet to your signed-in Agent Bounties account.\nThis proves address control only. It does not authorize a transaction, token approval, or payment.\nWallet: ${request.address}` };
    }
    if (pathname.endsWith("/wallet/verify")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      if (rejectVerification) { rejectVerification = false; await route.fulfill({status:503,json:{error:'wallet_link_store_unavailable'}}); return; }
      wallets = [...wallets, { address: request.address }]; body = { wallets };
    }
    await route.fulfill({ json: body });
  });
  await page.addInitScript(({ installed, address }) => {
    window.walletTestCalls = [];
    if (installed) window.ethereum = { isMetaMask: true, request: async (request) => {
      window.walletTestCalls.push({ wallet: "MetaMask", ...request });
      return request.method === "eth_requestAccounts" ? [address] : "0x" + "ab".repeat(65);
    } };
  }, { installed, address: ADDRESS });
  if (pending) await page.addInitScript((intent) => {
    if (!sessionStorage.getItem('test-intent-seeded')) {
      sessionStorage.setItem('agentbounties:pending-embedded-account-link',JSON.stringify(intent));
      sessionStorage.setItem('test-intent-seeded','true');
    }
  }, pending);
  // Replace only the external SDK boundary; exercise the real chooser and account handler.
  await page.route("**/vendor/coinbase-embedded-wallet.bundle.js?*", (route) => route.fulfill({
    contentType: "text/javascript",
    body: adapter ? testAdapter : `window.AgentBountiesCoinbaseEmbeddedWallet = { enabled: true, provider: { request: async (request) => {
      window.walletTestCalls.push({ wallet: "embedded", ...request });
      return request.method === "eth_requestAccounts" ? ["${EMBEDDED}"] : "0x" + "cd".repeat(65);
    } } };`,
  }));
  await page.goto(origin);
  await openAccount(page);
  const link = page.locator("[data-wallet-link]");
  await page.waitForFunction(() => document.querySelector("[data-wallet-list]").textContent !== "Checking verified wallets…");
  return { context, page, proofs, errors, link };
}

for (const installed of [true, false]) {
  for (const linked of [true, false]) {
    test(`${linked ? "Link another" : "Link wallet"} offers creation with ${installed ? "MetaMask" : "no wallet"} installed`, async () => {
      const { context, page, proofs, errors, link } = await account({ installed, linked, mobile: !installed });
      try {
        assert.equal(await link.innerText(), linked ? "Link another" : "Link wallet");
        await link.click();
        assert.equal(await page.getByRole("dialog", { name: "Choose a wallet" }).isVisible(), true);
        assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
        await page.getByRole("button", { name: "I don’t have a wallet — create one", exact: false }).click();
        await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("verified and linked"));
        const calls = await page.evaluate(() => window.walletTestCalls);
        assert.deepEqual(calls.map(({ wallet, method }) => [wallet, method]), [["embedded", "eth_requestAccounts"], ["embedded", "personal_sign"]]);
        assert.equal(calls[1].params[1], EMBEDDED);
        assert.equal(proofs[1].address, EMBEDDED);
        assert.equal(proofs[1].challenge_id, "ownership-only");
        assert.deepEqual(errors, []);
      } finally { await context.close(); }
    });
  }
}

test("Escape returns to the account without a wallet call; MetaMask works only after selection", async () => {
  const { context, page, proofs, link } = await account();
  try {
    await link.click();
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => !document.querySelector("[data-wallet-link]").disabled);
    assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
    assert.deepEqual(proofs, []);
    assert.equal(await link.evaluate((element) => element === document.activeElement), true);
    await link.click();
    await page.getByRole("button", { name: "MetaMask Choose an address in this wallet." }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("verified and linked"));
    assert.deepEqual((await page.evaluate(() => window.walletTestCalls)).map(({ wallet, method }) => [wallet, method]), [["MetaMask", "eth_requestAccounts"], ["MetaMask", "personal_sign"]]);
    assert.equal(proofs[1].address, ADDRESS);
  } finally { await context.close(); }
});

async function evidence(page, name) {
  const directory = process.env.WALLET_QA_EVIDENCE_DIR;
  if (!directory) return;
  await mkdir(directory, {recursive:true});
  await page.screenshot({path:path.join(directory, name + '.png')});
}

for (const redirect of [false, true]) {
  test(`${redirect ? 'Google redirect' : 'Email completion'} returns to a visible wallet and one ownership confirmation`, async () => {
    const {context,page,link,proofs,errors} = await account({adapter:true,mobile:redirect});
    try {
      await link.click();
      await page.getByRole('button',{name:'I don’t have a wallet — create one',exact:false}).click();
      await page.getByRole('button',{name:redirect?'Continue with Google':'Complete email verification',exact:true}).click();
      const confirm = page.getByRole('dialog',{name:'Confirm wallet ownership',exact:true});
      await confirm.waitFor();
      assert.equal(await page.getByRole('dialog',{name:'Your activity',exact:true}).isVisible(),true);
      assert.match(await page.locator('[data-wallet-list]').textContent(),/Ready to verify/);
      assert.equal((await page.evaluate(()=>window.walletTestCalls)).some(call=>call.method==='personal_sign'),false);
      assert.equal(proofs.length,1);
      assert.equal(await page.evaluate(async () => {
        try { await window.AgentBountiesCoinbaseEmbeddedWallet.provider.request({method:'personal_sign',params:['second message','0x'+'22'.repeat(20)]}); }
        catch (error) { return error.code; }
      }),-32002);
      await evidence(page,redirect?'oauth-return-mobile':'email-ready-desktop');
      await page.getByRole('button',{name:'Verify and link wallet',exact:true}).click();
      await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified and linked'));
      assert.equal(await page.getByRole('dialog',{name:'Your activity',exact:true}).isVisible(),true);
      assert.match(await page.locator('[data-wallet-list]').textContent(),/Verified/);
      assert.equal(await page.locator('[data-wallet-list] code').getAttribute('title'),EMBEDDED);
      assert.equal((await page.evaluate(()=>window.walletTestCalls)).filter(call=>call.method==='personal_sign').length,1);
      assert.equal(await page.evaluate(()=>sessionStorage.getItem('agentbounties:pending-embedded-account-link')),null);
      await evidence(page,redirect?'linked-mobile':'linked-desktop');
      await link.click();
      await page.getByRole('button',{name:'I don’t have a wallet — create one',exact:false}).click();
      await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('already verified and linked'));
      assert.equal(proofs.length,2);
      await page.reload();
      await openAccount(page);
      await page.waitForFunction(()=>document.querySelector('[data-wallet-list]').textContent.includes('Verified'));
      assert.equal(proofs.length,2);
      assert.deepEqual(errors,[]);
    } finally { await context.close(); }
  });
}

test('cancelled ownership review retains the wallet and retries without another sign-in', async () => {
  const {context,page,link,proofs,errors} = await account({adapter:true});
  try {
    await link.click();
    await page.getByRole('button',{name:'I don’t have a wallet — create one',exact:false}).click();
    await page.getByRole('button',{name:'Complete email verification'}).click();
    await page.getByRole('button',{name:'Not now',exact:true}).click();
    await page.getByRole('button',{name:'Finish linking',exact:true}).click();
    await page.getByRole('dialog',{name:'Confirm wallet ownership'}).waitFor();
    assert.equal((await page.evaluate(()=>window.walletTestCalls)).some(call=>call.method==='personal_sign'),false);
    await page.getByRole('button',{name:'Verify and link wallet',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified and linked'));
    assert.equal(proofs.length,3);
    await page.getByRole('button',{name:'Recovery settings',exact:true}).click();
    await page.getByRole('dialog',{name:'Protect access to this wallet'}).waitFor();
    assert.deepEqual(errors,[]);
  } finally { await context.close(); }
});

test('verification failure never marks the wallet verified and can be retried', async () => {
  const {context,page,link,errors} = await account({adapter:true,failVerify:true});
  try {
    await link.click();
    await page.getByRole('button',{name:'I don’t have a wallet — create one',exact:false}).click();
    await page.getByRole('button',{name:'Complete email verification'}).click();
    await page.getByRole('button',{name:'Verify and link wallet',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('not finished'));
    assert.equal(await page.locator('.wallet-verified').count(),0);
    await page.getByRole('button',{name:'Finish linking',exact:true}).click();
    await page.getByRole('button',{name:'Verify and link wallet',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified and linked'));
    assert.deepEqual(errors,[]);
  } finally { await context.close(); }
});

test('confirmed wallet remains visible when activity refresh fails', async () => {
  const {context,page,link} = await account({failRefresh:true});
  try {
    await link.click();
    await page.getByRole('button',{name:'I don’t have a wallet — create one',exact:false}).click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified and linked'));
    assert.equal(await page.locator('[data-wallet-list] code').getAttribute('title'),EMBEDDED);
    assert.match(await page.locator('[data-wallet-list]').textContent(),/Verified/);
  } finally { await context.close(); }
});

for (const pending of [{userId:'another-member',startedAt:Date.now()},{userId:'qa',startedAt:Date.now()-31*60*1000}]) {
  test(`stale or different-account intent cannot resume wallet access (${pending.userId})`,async()=>{
    const {context,page,proofs} = await account({adapter:true,pending});
    try {
      assert.equal(await page.evaluate(()=>window.AgentBountiesCoinbaseEmbeddedWallet),undefined);
      assert.deepEqual(await page.evaluate(()=>window.walletTestCalls),[]);
      assert.deepEqual(proofs,[]);
      assert.equal(await page.evaluate(()=>sessionStorage.getItem('agentbounties:pending-embedded-account-link')),null);
    } finally { await context.close(); }
  });
}
