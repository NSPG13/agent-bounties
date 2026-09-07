"use strict";
// Browser acceptance tests against synthetic inventory. No remote calls or wallet writes.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const { contract, item, opportunity } = require("./fixtures/funded-bounty.cjs");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".webp": "image/webp" };
const server = http.createServer((req, res) => {
  const file = path.resolve(site, "." + decodeURIComponent(new URL(req.url, "http://localhost").pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file)); }
  catch { res.writeHead(404).end(); }
});
async function bounds(page, selector) {
  await page.locator(selector).scrollIntoViewIfNeeded();
  const value = await page.locator(selector).evaluate(el => {
    const r = el.getBoundingClientRect(), hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return { rect: r.toJSON(), visible: r.left >= 0 && r.right <= innerWidth + 1 && r.top >= 0 && r.bottom <= innerHeight + 1 && r.height >= 43, hit: hit === el || el.contains(hit) };
  });
  assert.ok(value.visible && value.hit, selector + JSON.stringify(value));
}
async function layout(page) {
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "no horizontal clipping");
  assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), "rgb(7, 17, 12)");
  assert.notEqual(await page.evaluate(() => getComputedStyle(document.documentElement).scrollbarWidth), "none");
}
async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true });
  try {
    for (const [width, height] of [[1440,900],[946,838],[640,480],[390,600],[320,480],[768,320],[480,360]]) {
      const context = await browser.newContext({ viewport: { width, height }, reducedMotion: "reduce" });
      const page = await context.newPage(), errors = [];
      let mode = "ready";
      page.on("pageerror", e => errors.push(e.message));
      await context.route("**/*", route => {
        const url = new URL(route.request().url());
        if (url.pathname === "/v1/opportunities") return route.fulfill({ json: { schema_version: "agent-bounties/opportunity-projection-v1", items: mode === "offline" ? [] : [opportunity], generated_at: new Date().toISOString() }, status: mode === "offline" ? 503 : 200 });
        if (url.pathname === "/v1/base/autonomous-bounties/feed") return route.fulfill({ json: mode === "pending" ? [] : [item], status: mode === "offline" ? 503 : 200 });
        if (url.origin !== origin) return route.abort();
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ body: "window.agentBountiesPhoneWalletConfig={};", contentType: "text/javascript" });
        return route.continue();
      });
      await page.goto(`${origin}/earn.html?posted=${contract}`);
      await page.locator(".opportunity-row").waitFor();
      assert.match(await page.locator("[data-posted-notice]").innerText(), /On the board/);
      await layout(page);
      await bounds(page, ".opportunity-action a");
      if (process.env.MARKET_LAYOUT_ARTIFACTS && [1440,390].includes(width)) {
        fs.mkdirSync(process.env.MARKET_LAYOUT_ARTIFACTS, { recursive: true });
        await page.evaluate(() => scrollTo(0, 0));
        await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, `board-${width}.png`), fullPage: true });
      }
      await page.locator("[data-market-search]").fill("unmatched string");
      await page.locator("[data-market-clear]").click();
      await page.locator(".opportunity-row").waitFor();
      mode = "offline";
      await page.locator("[data-market-refresh]").click();
      await page.getByText("The board couldn’t refresh.").waitFor();
      assert.equal(await page.locator(".opportunity-row").count(), 0);
      mode = "ready";
      await page.locator("[data-market-refresh]").click();
      await page.locator(".opportunity-row").waitFor();
      await page.goto(`${origin}/participate.html?bountyContract=${contract}&network=base-mainnet`);
      await page.getByRole("heading", { name: item.terms.document.title }).waitFor();
      await layout(page);
      await bounds(page, "[data-work-prepare]");
      await bounds(page, "[data-work-refresh]");
      if (process.env.MARKET_LAYOUT_ARTIFACTS && [1440,390].includes(width)) {
        await page.evaluate(() => scrollTo(0, 0));
        await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, `workspace-${width}.png`), fullPage: true });
      }
      await page.goto(`${origin}/funded.html?bountyContract=${contract}&network=base-mainnet`);
      await page.getByRole("heading", { name: "Bounty funded." }).waitFor();
      await page.locator("[data-funded-stay]").click();
      assert.equal(await page.locator("[data-funded-amount]").innerText(), "17.068098 USDC");
      await layout(page); await bounds(page, "[data-funded-board]"); await bounds(page, "[data-funded-workspace]");
      if (process.env.MARKET_LAYOUT_ARTIFACTS && width === 1440) await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, "funded.png"), fullPage: true });
      mode = "pending";
      await page.reload(); await page.getByRole("heading", { name: "Funding is still being checked." }).waitFor();
      assert.equal(await page.locator("[data-funded-receipt]").isVisible(), false);
      mode = "offline";
      await page.locator("[data-funded-recheck]").click(); await page.getByRole("heading", { name: "We couldn’t check funding." }).waitFor();
      mode = "ready";
      await page.locator("[data-funded-recheck]").click(); await page.getByRole("heading", { name: "Bounty funded." }).waitFor();
      // Offline recovery deliberately stays in place for review.
      assert.equal(await page.locator("[data-funded-redirect]").isVisible(), false);
      if (width === 1440) {
        await page.reload();
        await page.waitForURL(`**/earn.html?posted=${contract}`);
        await page.locator(".opportunity-row").waitFor();
        // A confirmed journal is archived only by choosing another posting task.
        await page.evaluate(({ contract, id }) => {
          const journal = window.AgentBountiesWorkflow.createPostingJournal(window);
          journal.prepare({ predicted_bounty_contract: contract, bounty_id: id }); journal.checkpoint("funding_confirmed");
          window.AgentBountiesWorkflow.createClient(window).start({ role: "post", goal: "Previous completed task" });
        }, { contract, id: item.bounty_id });
        await page.locator(".feed-heading [data-new-bounty]").click();
        await page.waitForURL("**/post.html");
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createPostingJournal(window).load()), null);
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().goal), "");
        await page.evaluate(({ contract, id }) => {
          const flow = window.AgentBountiesWorkflow, journal = flow.createPostingJournal(window);
          journal.prepare({ predicted_bounty_contract: contract, bounty_id: id }); journal.checkpoint("authorized");
          flow.createClient(window).start({ role: "post", goal: "Pending original task" });
        }, { contract, id: item.bounty_id });
        await page.goto(`${origin}/earn.html`);
        await page.locator(".feed-heading [data-new-bounty]").click();
        await page.waitForURL("**/post.html");
        await page.locator("[data-recorded-posting]").waitFor();
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createPostingJournal(window).load().phase), "authorized");
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().goal), "Pending original task");
      }
      assert.deepEqual(errors, []);
      console.log(`marketplace layout passed ${width}x${height}`);
      await context.close();
    }
  } finally { await browser.close(); }
}
main().catch(e => { console.error(e); process.exitCode = 1; }).finally(() => server.close());
