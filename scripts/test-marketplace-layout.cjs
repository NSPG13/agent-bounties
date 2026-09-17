"use strict";
// Browser acceptance tests against synthetic inventory. No remote calls or wallet writes.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const { contract, item, opportunity } = require("./fixtures/funded-bounty.cjs");
const audit = require("./fixtures/opportunities-audit-20260916.json");
const assertHeadingLayout = require("./assert-heading-layout.cjs");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".webp": "image/webp" };
const server = http.createServer((req, res) => {
  let file = path.resolve(site, "." + decodeURIComponent(new URL(req.url, "http://localhost").pathname));
  if (file !== site && !file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try {
    if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
    const body = fs.readFileSync(file);
    res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(body);
  }
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
async function auditJourney(browser, origin, width) {
  const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: "reduce" });
  let mode = "audit";
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.pathname === "/v1/opportunities") {
      const payload = structuredClone(audit);
      if (mode === "unknown") delete payload.items[0].evidence_requirements.scoring_window;
      if (mode === "future") payload.items[7].evidence_requirements.scoring_window.starts_at = "2026-09-18T00:00:00Z";
      if (mode === "degraded") { payload.items = []; payload.degraded = true; }
      if (mode === "empty") payload.items = [];
      return route.fulfill({ json: payload, status: mode === "offline" ? 503 : 200 });
    }
    if (url.pathname === "/v1/metrics/platform") return route.fulfill({ json: { marketplace_payout_volume: { lifetime: { usdc: "23.75" }, lifetime_settled_rounds: 12 }, daily: [] } });
    if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ body: "window.agentBountiesPhoneWalletConfig={};", contentType: "text/javascript" });
    if (url.origin !== origin) return route.fulfill({ status: 503, body: "Isolated fixture" });
    return route.continue();
  });
  const page = await context.newPage(), errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.clock.setFixedTime(new Date(audit.generated_at));
  await page.goto(origin);
  await page.waitForFunction(() => document.querySelector("[data-live-bounties]").textContent === "3");
  assert.match(await page.locator("[data-live-weekly]").innerText(), /0 direct tasks · 3 competitions · 0 child-funding tasks/);
  await page.locator('.ab-metric a[href="earn.html"]').click();
  await page.locator(".opportunity-row").first().waitFor();
  assert.equal(await page.locator("[data-market-timing]").inputValue(), "now");
  assert.equal(await page.locator(".opportunity-row").count(), 3);
  assert.equal(await page.locator('.opportunity-row:not([data-phase="now"])').count(), 0);
  assert.match(await page.locator("[data-market-summary]").innerText(), /12 scoring closed/);
  await layout(page);
  await page.locator("[data-market-kind]").selectOption("direct");
  await page.getByRole("heading", { name: "No direct tasks in this view." }).waitFor();
  await page.getByRole("button", { name: "Show all work types" }).click();
  assert.equal(await page.locator("[data-market-timing]").inputValue(), "now", "clearing work type preserves the selected view");
  await page.locator("[data-market-kind]").selectOption("child_funding");
  assert.equal(await page.locator(".opportunity-row").count(), 0);
  await page.locator("[data-market-kind]").selectOption("competition");
  assert.equal(await page.locator(".opportunity-row").count(), 3);
  await page.locator("[data-market-timing]").selectOption("ended");
  assert.equal(await page.locator(".opportunity-row").count(), 12);
  const proof = page.getByRole("link", { name: "Continue to proof stage →", exact: true }).first();
  const proofUrl = new URL(await proof.getAttribute("href"), origin);
  assert.ok(audit.items.some(item => item.source_id === proofUrl.searchParams.get("bountyContract")));
  await proof.click();
  await page.waitForURL("**/competition.html?**");
  await page.waitForFunction(() => document.querySelector("[data-machine-request]")?.textContent.includes('"phase": "ended"'));
  await assertHeadingLayout(page, `closed competition at ${width}px`);
  const longTitle = audit.items.find(item => item.source_id === "0x5817b7742b085d333c7e7831daa62a490c493b56");
  assert.ok(longTitle, "real long-title competition fixture");
  await page.goto(`${origin}/competition.html?bountyContract=${longTitle.source_id}&network=base-mainnet`);
  await page.getByRole("heading", { name: longTitle.title, exact: true }).waitFor();
  for (const viewportWidth of width === 1440 ? [1920, 1280, 768] : [390, 320]) {
    await page.setViewportSize({ width: viewportWidth, height: 900 });
    await assertHeadingLayout(page, `long competition title at ${viewportWidth}px`);
    await layout(page);
    if (process.env.MARKET_LAYOUT_ARTIFACTS) await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, `competition-${viewportWidth}.png`) });
  }
  await page.setViewportSize({ width, height: 900 });
  await page.goto(origin + "/earn.html");
  await page.locator(".opportunity-row").first().waitFor();
  await page.locator("[data-market-timing]").selectOption("all");
  assert.equal(await page.locator(".opportunity-row").count(), 15);
  for (const state of ["future", "unknown", "degraded", "offline", "empty", "audit"]) {
    mode = state;
    await page.goto(origin);
    if (["degraded", "offline"].includes(state)) {
      await page.waitForFunction(() => document.querySelector("[data-live-weekly]").textContent.includes("inventory unavailable"));
      assert.equal(await page.locator("[data-live-bounties]").innerText(), "—");
    } else if (state === "unknown") {
      await page.waitForFunction(() => document.querySelector("[data-live-weekly]").textContent.includes("timing unavailable"));
      assert.equal(await page.locator("[data-live-bounties]").innerText(), "—");
    } else {
      await page.waitForFunction(expected => document.querySelector("[data-live-bounties]").textContent === expected, state === "empty" ? "0" : state === "future" ? "2" : "3");
    }
    await page.goto(origin + "/earn.html");
    await page.waitForFunction(() => document.querySelector("[data-opportunity-list]").getAttribute("aria-busy") === "false");
    const summary = await page.locator("[data-market-summary]").innerText();
    if (["degraded", "offline"].includes(state)) {
      assert.match(summary, /Live inventory unavailable/);
      assert.doesNotMatch(summary, /0 open/);
    } else if (state === "unknown") assert.match(summary, /Timing unavailable for 1; open-now total unconfirmed/);
    assert.equal(await page.locator(".opportunity-row").count(), ["degraded", "offline", "empty"].includes(state) ? 0 : state === "future" ? 2 : 3);
    assert.equal(await page.locator('.opportunity-row:not([data-phase="now"])').count(), 0);
    if (state === "future") {
      await page.locator("[data-market-timing]").selectOption("upcoming");
      assert.equal(await page.locator(".opportunity-row").count(), 1);
    }
  }
  if (process.env.MARKET_LAYOUT_ARTIFACTS) {
    await page.evaluate(() => document.fonts.ready);
    await page.screenshot({ path: path.join(process.env.MARKET_LAYOUT_ARTIFACTS, `audit-board-${width}.png`), fullPage: true });
  }
  assert.deepEqual(errors, []);
  await context.close();
  console.log(`audit homepage/board journey passed at ${width}px`);
}
async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined,
  });
  try {
    for (const [width, height] of [[1440,900],[946,838],[640,480],[390,600],[320,480],[768,320],[480,360]]) {
      const context = await browser.newContext({ viewport: { width, height }, reducedMotion: "reduce" });
      const page = await context.newPage(), errors = [];
      let mode = "ready";
      page.on("pageerror", e => errors.push(e.message));
      await context.route("**/*", route => {
        const url = new URL(route.request().url());
        if (url.pathname === "/v1/opportunities") return route.fulfill({ json: { schema_version: "agent-bounties/opportunity-projection-v1", network: "base-mainnet", applied_view: "ready_to_earn", degraded: false, source_statuses: [{ source_type: "canonical_base", available: true }], items: mode === "offline" ? [] : [{ ...opportunity, competition_mode: "exclusive_claim", deadline: "2099-01-01T00:00:00Z" }], generated_at: new Date().toISOString() }, status: mode === "offline" ? 503 : 200 });
        if (url.pathname === "/v1/base/autonomous-bounties/feed") return route.fulfill({ json: mode === "pending" ? [] : [mode === "verifier_blocked" ? { ...item, verification_ready: false, verification_readiness_reason: "benchmark not approved" } : item], status: mode === "offline" ? 503 : 200 });
        if (url.origin !== origin) return route.abort();
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ body: "window.agentBountiesPhoneWalletConfig={};", contentType: "text/javascript" });
        return route.continue();
      });
      await page.goto(`${origin}/earn.html?posted=${contract}`);
      await page.locator(".opportunity-row").waitFor();
      assert.match(await page.locator("[data-posted-notice]").innerText(), /On the board/);
      await layout(page);
      await bounds(page, ".opportunity-action a");
      await page.locator("[data-market-kind]").selectOption("competition");
      assert.equal(await page.locator(".opportunity-row").count(), 0);
      await page.locator("[data-market-kind]").selectOption("direct");
      await page.locator(".opportunity-row").waitFor();
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
      if (width === 1440) {
        mode = "verifier_blocked";
        await page.reload();
        await page.getByText(/Funding is confirmed, but the bounty is not ready for work/).waitFor();
        assert.match(await page.locator("[data-funded-status]").innerText(), /benchmark not approved/);
        assert.equal(await page.locator("[data-funded-redirect]").isVisible(), false);
        await page.goto(`${origin}/participate.html?bountyContract=${contract}&network=base-mainnet`);
        await page.getByText(/Funded · waiting for verification readiness/).waitFor();
        assert.equal(await page.locator("[data-work-prepare]").isVisible(), false);
        mode = "pending";
        await page.goto(`${origin}/funded.html?bountyContract=${contract}&network=base-mainnet`);
      }
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
        await page.waitForURL("**/#post-a-bounty");
        await page.locator("[data-bounty-launcher][open]").waitFor();
        await page.goto(`${origin}/post.html`);
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createPostingJournal(window).load()), null);
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().goal), "");
        await page.evaluate(({ contract, id }) => {
          const flow = window.AgentBountiesWorkflow, journal = flow.createPostingJournal(window);
          journal.prepare({ predicted_bounty_contract: contract, bounty_id: id }); journal.checkpoint("authorized");
          flow.createClient(window).start({ role: "post", goal: "Pending original task" });
        }, { contract, id: item.bounty_id });
        await page.goto(`${origin}/earn.html`);
        await page.locator(".feed-heading [data-new-bounty]").click();
        await page.waitForURL("**/#post-a-bounty");
        await page.locator("[data-bounty-launcher][open]").waitFor();
        await page.goto(`${origin}/post.html`);
        await page.locator("[data-recorded-posting]").waitFor();
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createPostingJournal(window).load().phase), "authorized");
        assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().goal), "Pending original task");
      }
      assert.deepEqual(errors, []);
      console.log(`marketplace layout passed ${width}x${height}`);
      await context.close();
    }
    for (const width of [390, 1440]) await auditJourney(browser, origin, width);
  } finally { await browser.close(); }
}
main().catch(e => { console.error(e); process.exitCode = 1; }).finally(() => server.close());
