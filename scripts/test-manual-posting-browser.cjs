"use strict";
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  const file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const body = fs.readFileSync(file); res.writeHead(200, {"Content-Type": ({".html":"text/html", ".js":"text/javascript", ".css":"text/css"})[path.extname(file)] || "application/octet-stream"}).end(body); }
  catch (_) { res.writeHead(404).end(); }
});
(async () => {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({headless:true, executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined});
  try {
    for (const width of [390, 1440]) {
      const context = await browser.newContext({viewport:{width,height:1000},reducedMotion:"reduce"});
      const externalWrites = [], errors = [];
      await context.route("**/*", route => {
        const req = route.request(), url = new URL(req.url());
        if (url.origin === origin) return route.continue();
        if (req.method() !== "GET") externalWrites.push(url.href);
        if (url.pathname.endsWith("/session")) return route.fulfill({json:{authenticated:false,providers:{google:true,microsoft:true,github:true},posting_drafts_enabled:false}});
        return route.fulfill({status:503,body:"Isolated browser test"});
      });
      const page = await context.newPage(); page.on("pageerror", e => errors.push(e.message));
      await page.goto(origin);
      await page.locator("#post-a-bounty").click();
      assert.equal(await page.locator("#bounty-launcher-title").innerText(), "Choose how to start");
      await page.locator("[data-bounty-manual]").click();
      await page.waitForURL("**/post.html?mode=manual#bounty-composer-form");
      assert.equal(await page.locator("[data-brief-mode]").inputValue(), "manual");
      await page.locator("[data-brief-title]").fill("Research competitors");
      await page.locator("[data-brief-criteria]").fill("Include 20 competitors\nLink each pricing source");
      await page.locator("#bounty-composer-input").fill("Deliver a comparison spreadsheet");
      await page.locator("[data-brief-budget]").fill("20.01");
      await page.locator("[data-brief-reserve]").fill("0.11");
      await page.locator("[data-brief-timezone]").fill("UTC");
      await page.locator("[data-brief-deadline]").fill(new Date(Date.now() + 7*86400000).toISOString().slice(0,16));
      await page.locator("[data-manual-review-consent]").check();
      await page.reload();
      assert.equal(await page.locator("[data-brief-title]").inputValue(), "Research competitors");
      assert.equal(await page.locator("[data-manual-review-consent]").isChecked(), true);
      await page.locator("[data-composer-submit]").click();
      await page.locator("#bounty-preview:not([hidden])").waitFor();
      await page.waitForFunction(() => document.querySelector("[data-brief-status]").textContent.startsWith("Proposal prepared"));
      const draft = await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().draft);
      assert.equal(draft.title, "Research competitors");
      assert.equal(draft.solver_reward_usdc, "19.90");
      assert.equal(draft.verifier_reward_usdc, "0.11");
      assert.equal(draft.benchmark.engine, "creator_review_v1");
      assert.deepEqual(draft.acceptance_criteria, ["Include 20 competitors", "Link each pricing source"]);
      assert.equal(await page.locator("[data-open-funding]").isDisabled(), true, "preparing does not approve or fund");
      await page.locator("[data-revise-card]").click();
      await page.locator("[data-brief-criteria]").fill("Include 25 competitors\nLink each pricing source");
      assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().draft_stale), true);
      await page.locator("[data-composer-submit]").click();
      await page.waitForFunction(() => document.querySelector("[data-card-criteria]").textContent.includes("25 competitors"));
      await page.reload();
      await page.locator("#bounty-preview:not([hidden])").waitFor();
      assert.equal(await page.locator("[data-brief-mode]").inputValue(), "manual");
      assert.equal(await page.locator("[data-brief-criteria]").inputValue(), "Include 25 competitors\nLink each pricing source");
      assert.equal(await page.locator("[data-open-funding]").isDisabled(), true);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "no horizontal overflow");
      fs.mkdirSync("target/manual-posting", {recursive:true});
      await page.screenshot({path:`target/manual-posting/review-${width}.png`,fullPage:true});
      await page.locator("[data-brief-mode]").selectOption("ai");
      assert.equal(await page.locator("[data-manual-fields]").isVisible(), false);
      assert.deepEqual(externalWrites, [], "preparation never calls a model, publishes or requests payment");
      assert.deepEqual(errors, []);
      await context.close(); console.log(`Manual entry, validation, native review, editing and recovery pass at ${width}px`);
    }
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); server.close(); process.exitCode=1; });
