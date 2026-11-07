"use strict";
// Real account dialog with synthetic sessions; all external requests are intercepted.
const assert = require("node:assert/strict");
const fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const mime = { ".html":"text/html", ".js":"text/javascript", ".css":"text/css", ".svg":"image/svg+xml" };
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  const file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const bytes = fs.readFileSync(file); res.writeHead(200, {"content-type":mime[path.extname(file)] || "application/octet-stream"}).end(bytes); }
  catch { res.writeHead(404).end(); }
});
const project = "0988a16c-3d9a-4c16-8c7d-d5664aee2961", bounty = `0x${"ab".repeat(32)}`;
(async () => {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless:true });
  try {
    for (const [name, target, linked, available, returns] of [
      ["test", `/workspace/?project=${project}`, false, true, true],
      ["canonical", `/workspace/?bounty=${bounty}`, false, true, false],
      ["mixed", `/workspace/?project=${project}&bounty=${bounty}`, false, true, false],
      ["generic", "/workspace/", false, true, false],
      ["linked canonical", `/workspace/?bounty=${bounty}`, true, true, true],
      ["unavailable account", `/workspace/?project=${project}`, false, false, false],
    ]) {
      const context = await browser.newContext(), page = await context.newPage();
      let signedIn = false;
      const errors = [], navigations = [], walletRequests = [];
      page.on("pageerror", error => errors.push(error.message));
      await context.route("**/*", async route => {
        const req = route.request(), url = new URL(req.url());
        const session = { authenticated:signedIn, providers:{google:true}, account_status:"ready", account_complete:true,
          solver_status:linked ? "ready" : "wallet_required", solver_ready:linked,
          user:signedIn ? {id:"ab".repeat(32),name:"Test account",email:"tester@example.test",provider:"google"} : null };
        if (["/auth/session", "/v1/site-auth/session"].includes(url.pathname)) return route.fulfill({json:session});
        if (["/auth/account", "/v1/site-auth/account"].includes(url.pathname)) return route.fulfill(available
          ? {json:{...session, wallets:linked ? [{address:`0x${"a".repeat(40)}`,verified:true}] : [], saved_drafts:{status:"available",items:[]}}}
          : {status:503,json:{error:"unavailable"}});
        if (url.pathname.includes("wallet") && req.method() !== "GET") walletRequests.push(url.pathname);
        if (url.pathname.endsWith("/review-notifications")) return route.fulfill({json:{enabled:false,email_verified:true,wallet_linked:linked}});
        if (url.pathname === "/auth/login/google") return route.fulfill({contentType:"text/html",body:"<!doctype html><title>Synthetic provider</title>"});
        if (url.pathname === "/workspace/") {
          navigations.push(url.pathname + url.search);
          return route.fulfill({contentType:"text/html",body:"<!doctype html><title>Workspace destination</title>"});
        }
        if (url.pathname === "/v1/opportunities") return route.fulfill({json:{items:[]}});
        if (url.origin !== origin) return route.abort();
        return route.continue();
      });
      await page.goto(`${origin}/?workspaceReturn=${encodeURIComponent(target)}#login`);
      await page.locator("[data-auth-dialog][open]").waitFor();
      await page.waitForFunction(() => document.querySelector('[data-auth-provider="google"]').getAttribute("aria-disabled") === "false");
      await page.locator('[data-auth-provider="google"]').click();
      await page.waitForURL("**/auth/login/google");
      signedIn = true;
      await page.goto(`${origin}/?auth=success&provider=google#login`);
      if (returns) {
        await page.waitForURL(origin + target);
        assert.deepEqual(navigations, [target], name);
      } else {
        await page.waitForFunction(status => document.querySelector("[data-auth-dialog]")?.dataset.accountStatus === status, available ? "ready" : "unavailable");
        assert.deepEqual(navigations, [], name);
        assert.equal(await page.getByRole("button", {name:"Continue to Workspace"}).isVisible(), false, name);
      }
      assert.deepEqual(walletRequests, [], "no wallet link/signature request is made by the test-entry exception");
      assert.deepEqual(errors, [], name);
      await context.close();
      console.log(`PASS Workspace account return: ${name}`);
    }
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; server.close(); });
