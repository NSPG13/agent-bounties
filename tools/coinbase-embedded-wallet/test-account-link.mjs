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
  browser = await chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined,
  });
});
after(async () => {
  await browser?.close();
  if (server) await new Promise((resolve) => server.close(resolve));
});

async function openAccount(page) {
  const accountLink = page.getByRole("link", { name: /^(Account|Finish setup)$/, exact: true });
  const menu = page.getByRole("button", { name: "Menu", exact: false });
  if (await menu.isVisible() && await menu.getAttribute("aria-expanded") !== "true") await menu.click();
  await accountLink.click();
}

async function account({ installed = true, linked = false, linkedAddress = ADDRESS, linkedProvider = null, startupUnavailable = false, mobile = false, adapter = false, pending = null, failVerify = false, failRefresh = false, phone = false, invalidVerify = false, unavailable = false, legacy = false, postTarget = null, postReturnRoute = false, delayedPhoneSign = false, expectAutoReturn = false } = {}) {
  const context = await browser.newContext({ viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const proofs = [], errors = [];
  let wallets = linked ? [{ address: linkedAddress, provider_id: linkedProvider }] : [];
  let rejectVerification = failVerify;
  const network = { unavailable };
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("https://**/*", (route) => route.fulfill({ json: {} }));
  await page.route("**/auth/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let body = {};
    if (pathname.endsWith("/session")) body = {
      authenticated: true,
      user: { id: "qa", name: "Test member", provider: "google" },
      providers: { google: true },
      account_status: wallets.length ? "ready" : "wallet_required",
      account_complete: wallets.length > 0,
    };
    if (pathname.endsWith("/account")) {
      if (network.unavailable || failRefresh && wallets.some(wallet => wallet.address === EMBEDDED)) { await route.fulfill({status:503,json:{}}); return; }
      body = { wallets, available: false, account_status: wallets.length ? "ready" : "wallet_required", account_complete: wallets.length > 0 };
    }
    if (pathname.endsWith("/wallet/challenge")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      body = { challenge_id: "ownership-only", message: `Agent Bounties wallet ownership verification\n\nSign this message to link the wallet to your signed-in Agent Bounties account.\nThis proves address control only. It does not authorize a transaction, token approval, or payment.\nWallet: ${request.address}` };
    }
    if (pathname.endsWith("/wallet/verify")) {
      const request = route.request().postDataJSON(); proofs.push(request);
      if (invalidVerify) { await route.fulfill({json:{}}); return; }
      if (rejectVerification) { rejectVerification = false; await route.fulfill({status:503,json:{error:'wallet_link_store_unavailable'}}); return; }
      wallets = [...wallets, { address: request.address }]; body = { wallets, linked: true, account_status: "ready", account_complete: true };
    }
    if (pathname.endsWith("/wallet/unlink")) {
      wallets = wallets.filter(wallet => wallet.address !== route.request().postDataJSON().address);
      body = { wallets, unlinked: true, account_status: wallets.length ? "ready" : "wallet_required", account_complete: wallets.length > 0 };
    }
    if (legacy) {
      delete body.account_status; delete body.account_complete;
      if (pathname.endsWith('/account')) {
        body.identity_link_status = wallets.length ? 'verified' : 'unlinked';
        body.reason = wallets.length ? 'marketplace_evidence_unavailable' : 'marketplace_identity_unlinked';
      }
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
  if (startupUnavailable) await page.addInitScript(() => { window.testCdpUnavailable = true; });
  if (phone) await page.route("**/phone-wallet.js?*", route => route.fulfill({contentType:"text/javascript",body:`
    window.AgentBountiesPhoneWallet = { state: () => ({available:true}), provider: {request: async request => {
      window.walletTestCalls.push({wallet:'phone', ...request});
      if (request.method === 'eth_requestAccounts') return ['${EMBEDDED}'];
      if (${JSON.stringify(delayedPhoneSign)} && request.method === 'personal_sign') return new Promise(resolve => {
        window.resolvePhoneOwnership = () => resolve('0x' + 'cd'.repeat(65));
      });
      return '0x' + 'cd'.repeat(65);
    }} };
  `}));
  if (pending) await page.addInitScript((intent) => {
    if (!sessionStorage.getItem('test-intent-seeded')) {
      sessionStorage.setItem('agentbounties:pending-embedded-account-link',JSON.stringify(intent));
      sessionStorage.setItem('test-intent-seeded','true');
    }
  }, pending);
  if (postTarget) await page.addInitScript((target) => {
    if (!sessionStorage.getItem("test-post-return-seeded")) {
      sessionStorage.setItem("agentbounties:post-auth-return:v1", JSON.stringify({
        schema: "agent-bounties/post-auth-return-v1",
        target,
        started_at: Date.now(),
      }));
      sessionStorage.setItem("test-post-return-seeded", "true");
    }
  }, postTarget);
  // Replace only the external SDK boundary; exercise the real chooser and account handler.
  await page.route("**/vendor/coinbase-embedded-wallet.bundle.js?*", (route) => route.fulfill({
    contentType: "text/javascript",
    body: adapter ? testAdapter : `window.AgentBountiesCoinbaseEmbeddedWallet = { enabled: true, provider: { request: async (request) => {
      window.walletTestCalls.push({ wallet: "embedded", ...request });
      sessionStorage.setItem("test-embedded-wallet-methods", JSON.stringify([...(JSON.parse(sessionStorage.getItem("test-embedded-wallet-methods") || "[]")), request.method]));
      return ["eth_accounts", "eth_requestAccounts"].includes(request.method) ? ["${EMBEDDED}"] : "0x" + "cd".repeat(65);
    } } };`,
  }));
  await page.goto(postReturnRoute ? `${origin}/?postReturn=1#login` : origin);
  if (expectAutoReturn) {
    await page.waitForURL(postTarget);
    return { context, page, proofs, errors, link: null, network };
  }
  await openAccount(page);
  const link = page.locator("[data-wallet-link]");
  await page.waitForFunction(() => document.querySelector("[data-wallet-list]").textContent !== "Checking verified wallets…");
  return { context, page, proofs, errors, link, network };
}

test("pending setup is resumable and never shows activity before wallet proof", async () => {
  const {context,page,link,proofs} = await account();
  try {
    assert.equal(await page.locator('#auth-title').innerText(), 'Finish your account');
    assert.equal(await page.locator('[data-account-stats]').isVisible(), false);
    await evidence(page, 'setup-desktop');
    await page.getByRole('button',{name:'Close account dialog'}).click();
    await openAccount(page);
    assert.equal(await page.locator('#auth-title').innerText(), 'Finish your account');
    await link.click();
    await page.keyboard.press('Escape');
    assert.deepEqual(proofs, []);
    await page.reload();
    await openAccount(page);
    assert.equal(await page.locator('#auth-title').innerText(), 'Finish your account');
    assert.deepEqual(await page.evaluate(()=>window.walletTestCalls), []);
  } finally { await context.close(); }
});

for (const legacy of [false, true]) for (const linked of [false, true]) test(`phone wallet is available through ${linked ? 'Link another' : 'Link wallet'} (${legacy ? 'prior' : 'current'} API)`, async () => {
  const {context,page,link,proofs} = await account({phone:true,installed:false,linked,mobile:true,legacy});
  try {
    assert.equal(await page.locator('.ab-phone-launcher').count(),0);
    await link.click();
    const phone = page.getByRole('button',{name:/^Use a phone wallet Open your mobile wallet/});
    assert.equal(await phone.count(),1);
    assert.deepEqual(await page.evaluate(()=>window.walletTestCalls),[]);
    await evidence(page,'phone-chooser-mobile');
    await phone.click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified and linked'));
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'ready');
    assert.deepEqual((await page.evaluate(()=>window.walletTestCalls)).map(call=>call.method),['eth_requestAccounts','personal_sign']);
    assert.equal(proofs.length,2);
  } finally { await context.close(); }
});

function preparedPostTarget() {
  const benchmark = {
    engine: "sandboxed_regression_v1",
    runner_manifest: {
      benchmark_digest: "sha256:eed1340e372c85f87f8718696c03973748fb3fbaec7b4e90041d77d3513f9656",
      command: ["python", "/benchmark/check.py"],
      cpu_millis: 1000,
      image: "docker.io/library/python@sha256:d657ab0ade19f404a6ccc883ab399540de667aff751748ce23c07330c5a89e64",
      max_benchmark_bytes: 1048576,
      max_benchmark_files: 100,
      max_output_bytes: 1048576,
      max_source_bytes: 67108864,
      max_source_files: 1000,
      memory_bytes: 536870912,
      pids_limit: 128,
      platform: "linux/amd64",
      schema_version: "agent-bounties/regression-sandbox-v1",
      test_seed: 1,
      timeout_seconds: 120,
      tmpfs_bytes: 268435456,
      workdir: "/workspace",
    },
    source: {
      commit: "0fae18cf9be464132cde52dfb9d464d836e8f024",
      kind: "github_commit",
      repository: "NSPG13/agent-bounties",
      subdirectory: "benchmarks/distribution-v1/glama-onboarding-audit",
    },
  };
  const evidenceSchema = {
    additionalProperties: false,
    properties: { source_snapshot_digest: { pattern: "^sha256:[0-9a-f]{64}$", type: "string" } },
    required: ["source_snapshot_digest"],
    type: "object",
  };
  const params = new URLSearchParams({
    from: "ai-app",
    title: "Audit the Glama onboarding path",
    goal: "Verify the attributed MCP path and publish canonical evidence.",
    solverReward: "2",
    verifierReward: "0.1",
    taskWindowDays: "7",
    crowdfund: "false",
    discoverySource: "Glama paid-rail mainnet canary",
    benchmark: JSON.stringify(benchmark),
    evidenceSchema: JSON.stringify(evidenceSchema),
    acquisition: `aba1_${"a".repeat(64)}.${"b".repeat(64)}`,
    handoff: "52b6a4da-4d81-4581-b61e-a3782321cba7",
  });
  params.append("criterion", "Connect to the attributed Glama MCP route.");
  params.append("criterion", "Publish redacted initialize and tools/list evidence.");
  return `${origin}/post.html?${params}`;
}

test("phone ownership feedback returns to the exact prepared bounty without a payment request", async () => {
  const target = preparedPostTarget();
  const { context, page, link, errors } = await account({
    installed: false,
    mobile: true,
    phone: true,
    postTarget: target,
    postReturnRoute: true,
    delayedPhoneSign: true,
  });
  try {
    await link.click();
    await page.getByRole("button", { name: /^Use a phone wallet Open your mobile wallet/ }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("connected. Review the ownership-only message"));
    assert.match(await page.locator("[data-wallet-status]").innerText(), /Phone wallet 0x222222…222222 connected/);
    assert.deepEqual((await page.evaluate(() => window.walletTestCalls)).map(call => call.method), ["eth_requestAccounts", "personal_sign"]);
    await page.evaluate(() => window.resolvePhoneOwnership());
    await page.waitForURL(target);
    await page.getByRole("button", { name: "Confirm bounty", exact: true }).waitFor();
    assert.equal(page.url(), target);
    assert.match(await page.locator("[data-composer-status]").innerText(), /prepared bounty is intact/i);
    assert.equal((await page.evaluate(() => window.walletTestCalls)).some(call => ["eth_sendTransaction", "wallet_sendCalls"].includes(call.method)), false);
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test("an already verified embedded wallet resumes directly into the exact prepared bounty", async () => {
  const target = preparedPostTarget();
  const { context, page, errors } = await account({
    installed: false,
    linked: true,
    linkedAddress: EMBEDDED,
    pending: { userId: "qa", startedAt: Date.now() },
    postTarget: target,
    expectAutoReturn: true,
  });
  try {
    await page.getByRole("button", { name: "Confirm bounty", exact: true }).waitFor();
    assert.equal(page.url(), target);
    assert.match(await page.locator("[data-composer-status]").innerText(), /prepared bounty is intact/i);
    assert.deepEqual(await page.evaluate(() => JSON.parse(sessionStorage.getItem("test-embedded-wallet-methods"))), ["eth_accounts"]);
    assert.equal((await page.evaluate(() => window.walletTestCalls)).some(call => ["personal_sign", "eth_sendTransaction", "wallet_sendCalls"].includes(call.method)), false);
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test("an HTTP success without verified ownership never completes setup", async () => {
  const {context,page,link} = await account({invalidVerify:true});
  try {
    await link.click();
    await page.getByRole('button',{name:'MetaMask Choose an address in this wallet.'}).click();
    await page.waitForFunction(()=>!document.querySelector('[data-wallet-link]').disabled);
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'wallet_required');
    assert.equal(await page.locator('.wallet-verified').count(),0);
    assert.equal(await page.locator('[data-account-stats]').isVisible(),false);
  } finally { await context.close(); }
});

test("wallet storage failure has a retry and cannot be mistaken for missing wallets", async () => {
  const {context,page,network,link} = await account({unavailable:true,mobile:true});
  try {
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'unavailable');
    assert.equal(await link.isVisible(),false);
    assert.equal(await page.getByRole('button',{name:'Check again',exact:true}).isVisible(),true);
    network.unavailable=false;
    await page.getByRole('button',{name:'Check again',exact:true}).click();
    await link.waitFor({state:'visible'});
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'wallet_required');
    await evidence(page,'setup-mobile');
  } finally { await context.close(); }
});

test("removing the final wallet revokes completion even if the activity refresh fails", async () => {
  const {context,page,network} = await account({linked:true});
  try {
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'ready');
    assert.deepEqual(await page.evaluate(()=>window.walletTestCalls),[]);
    const remove = page.locator('[data-wallet-unlink]');
    await remove.click();
    assert.match(await page.locator('[data-wallet-status]').innerText(),/last wallet returns your account to setup/);
    network.unavailable=true;
    await remove.click();
    await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('Wallet removed.'));
    assert.equal(await page.locator('.wallet-verified').count(),0);
    assert.equal(await page.locator('[data-account-stats]').isVisible(),false);
    assert.notEqual(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'ready');
    network.unavailable=false;
    await page.reload(); await openAccount(page);
    assert.equal(await page.locator('[data-auth-dialog]').getAttribute('data-account-status'),'wallet_required');
  } finally { await context.close(); }
});

for (const installed of [true, false]) {
  for (const linked of [true, false]) {
    test(`${linked ? "Link another" : "Link wallet"} offers creation with ${installed ? "MetaMask" : "no wallet"} installed`, async () => {
      const { context, page, proofs, errors, link } = await account({ installed, linked, mobile: !installed });
      try {
        assert.equal(await link.innerText(), linked ? "Link another" : "Link wallet");
        await link.click();
        assert.equal(await page.getByRole("dialog", { name: "Choose a wallet" }).isVisible(), true);
        assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
        await page.getByRole("button", { name: "Use or recover Coinbase embedded wallet", exact: false }).click();
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
      await page.getByRole('button',{name:'Use or recover Coinbase embedded wallet',exact:false}).click();
      await page.getByRole('button',{name:redirect?'Continue with Google':'Complete email verification',exact:true}).click();
      const confirm = page.getByRole('dialog',{name:'Confirm wallet ownership',exact:true});
      await confirm.waitFor();
      assert.equal(await page.getByRole('dialog',{name:'Finish your account',exact:true}).isVisible(),true);
      assert.equal(await page.locator('[data-account-stats]').isVisible(),false);
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
      await page.getByRole('button',{name:'Use or recover Coinbase embedded wallet',exact:false}).click();
      await page.waitForFunction(()=>document.querySelector('[data-wallet-status]').textContent.includes('verified ownership and is connected'));
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
    await page.getByRole('button',{name:'Use or recover Coinbase embedded wallet',exact:false}).click();
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
    await page.getByRole('button',{name:'Use or recover Coinbase embedded wallet',exact:false}).click();
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
    await page.getByRole('button',{name:'Use or recover Coinbase embedded wallet',exact:false}).click();
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

test("Coinbase configuration failure remains visible with retry and another-wallet route", async () => {
  const { context, page, link, proofs, errors } = await account({ adapter: true, startupUnavailable: true });
  try {
    await link.click();
    const recovery = page.getByRole("button", { name: /^Use or recover Coinbase embedded wallet/ });
    await recovery.click();
    await page.waitForFunction(() => document.querySelector(".wallet-link-status").textContent.includes("configuration could not load"));
    assert.equal(await recovery.isEnabled(), true);
    assert.equal(await page.getByRole("dialog", { name: "Choose a wallet" }).isVisible(), true);
    assert.doesNotMatch(await page.locator(".wallet-link-status").innerText(), /private@example/);
    assert.deepEqual(proofs, []);
    assert.deepEqual(await page.evaluate(() => window.walletTestCalls), []);
    await recovery.click();
    await page.waitForFunction(() => window.AgentBountiesCoinbaseEmbeddedWallet.readiness().state === "paused");
    await recovery.click();
    await page.waitForFunction(() => document.querySelector(".wallet-link-status").textContent.includes("Retry in a minute"));
    await page.getByRole("button", { name: "MetaMask Choose an address in this wallet." }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("verified and linked"));
    assert.deepEqual((await page.evaluate(() => window.walletTestCalls)).map(call => call.wallet), ["MetaMask", "MetaMask"]);
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test("restoring a selected linked wallet rejects another address before ownership or payment", async () => {
  const { context, page, proofs } = await account({ linked: true, linkedProvider: "coinbase-embedded", linkedAddress: ADDRESS });
  try {
    assert.match(await page.locator("[data-wallet-list]").innerText(), /Signing session not connected/);
    await page.getByRole("button", { name: "Restore Coinbase signing session" }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("different address"));
    assert.deepEqual(proofs, []);
    assert.deepEqual((await page.evaluate(() => window.walletTestCalls)).map(call => call.method), ["eth_requestAccounts"]);
    assert.match(await page.locator("[data-wallet-list]").innerText(), /Signing session not connected/);
  } finally { await context.close(); }
});

test("Account stays separate from a saved posting return and restores only the matching signing address", async () => {
  const target = preparedPostTarget();
  const { context, page, proofs } = await account({ linked: true, linkedProvider: "coinbase-embedded", linkedAddress: EMBEDDED, postTarget: target });
  try {
    await page.evaluate(() => { location.hash = "account"; });
    assert.equal(await page.getByRole("link", { name: "Account", exact: true }).getAttribute("data-authenticated"), "true");
    await page.getByRole("button", { name: "Restore Coinbase signing session" }).click();
    await page.waitForFunction(() => document.querySelector("[data-wallet-status]").textContent.includes("connected for this session"));
    assert.match(page.url(), /#account$/);
    assert.deepEqual(proofs, []);
    assert.match(await page.locator("[data-wallet-list]").innerText(), /Connected for this session/);
    await page.getByRole("button", { name: "Return to bounty", exact: true }).click();
    await page.waitForURL(target);
  } finally { await context.close(); }
});
