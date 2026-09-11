"use strict";
// Isolated browser tests. Fixtures only; no real accounts, wallet requests or payments.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".webp": "image/webp" };
const pages = fs.readdirSync(site, { recursive: true }).filter(file => file.endsWith(".html")).map(file => file.replaceAll("\\", "/")).sort((a, b) => a === "index.html" ? -1 : b === "index.html" ? 1 : a.localeCompare(b));
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost");
  let file = path.resolve(site, "." + decodeURIComponent(url.pathname));
  if (file !== site && !file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try {
    if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
    res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file));
  } catch { res.writeHead(404).end(); }
});
const cash = value => ({ usdc: String(value), usdc_base_units: String(value * 1e6) });
function fixture(period, mode) {
  const amount = mode === "zero" ? 0 : period === "7d" ? 7 : 12;
  const now = new Date().toISOString();
  return {
    generated_at: now, coverage: { status: "ready" }, platform_active_identities: { selected: 2, latest_week: 2, previous_week: 1, lifetime: 2, first_month: 1, roles: [] },
    marketplace_payout_volume: { selected: cash(amount), selected_settled_rounds: amount ? 2 : 0, selected_solver_pay: cash(amount), selected_verifier_pay: cash(0), selected_keeper_pay: cash(0), selected_completion_bonus: cash(0) },
    current_inventory: { status: mode === "partial" ? "partial" : "ready", generated_at: now, active_funded_opportunities: amount ? 3 : 0, available_funding_usdc: 30, available_solver_rewards_usdc: 27, available_verifier_rewards_usdc: 3 },
    mature_claim_to_settlement: { settlement_rate: .5, mature_claimed_rounds: 4, immature_claimed_rounds: 2 },
    daily: [{ day: "2026-09-05", active_identities: 1, payout: cash(0) }, { day: "2026-09-06", active_identities: 2, payout: cash(amount) }], platform_revenue: cash(0),
  };
}
async function assertFits(page, selector) {
  await page.locator(selector).scrollIntoViewIfNeeded();
  const result = await page.locator(selector).evaluate(el => {
    const r = el.getBoundingClientRect(), hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return { fits: r.left >= 0 && r.right <= innerWidth + 1 && r.top >= 0 && r.bottom <= innerHeight + 1, hit: hit === el || el.contains(hit), rect: r.toJSON() };
  });
  assert.ok(result.fits && result.hit, `${selector}: ${JSON.stringify(result)}`);
}
async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined,
  });
  let mode = "ready";
  const errors = [];
  async function context(options) {
    const ctx = await browser.newContext({ reducedMotion: "reduce", ...options });
    await ctx.route("**/*", route => {
      const url = new URL(route.request().url()), now = new Date().toISOString();
      if (url.pathname === "/v1/metrics/platform") return mode === "offline" ? route.fulfill({ status: 503, body: "unavailable" }) : route.fulfill({ json: fixture(url.searchParams.get("period"), mode) });
      if (url.pathname === "/generated/github-participation.json") return route.fulfill({ json: { generated_at: now, coverage: { status: "ready" }, periods: Object.fromEntries(["lifetime", "90d", "28d", "7d"].map(p => [p, { active_identities: 3 }])), repository_acquisition: { generated_at: now, started_at: now, ended_at: now, coverage: { status: "ready" } } } });
      if (url.pathname.endsWith("/events")) return route.fulfill({ json: [] });
      if (url.pathname === "/v1/analytics/site") return route.fulfill({ json: { generated_at: now, interfaces: [], overview: { unique_visitors: 0, sessions: 0 } } });
      if (url.pathname.includes("/auth/session")) return route.fulfill({ json: { authenticated: false, providers: {} } });
      if (url.origin !== origin) return route.fulfill({ status: 503, body: "Isolated test" });
      if (url.pathname === "/phone-wallet-config.js") return route.fulfill({ body: "window.agentBountiesPhoneWalletConfig={};", contentType: "text/javascript" });
      return route.continue();
    });
    return ctx;
  }
  try {
    for (const width of [1280, 390]) {
      const ctx = await context({ viewport: { width, height: 800 } }), page = await ctx.newPage();
      let reference;
      for (const file of pages) {
        if (file === "posting-draft-canary.html") {
          const sessionUrl = "https://api.agentbounties.app/v1/site-auth/session";
          await page.route(sessionUrl, route => route.fulfill({
            headers: { "Cache-Control": "no-store" },
            json: { authenticated: false, user: null, posting_drafts_enabled: false },
          }));
          await page.goto(`${origin}/${file}`);
          await page.waitForFunction(() => document.getElementById("canary-authenticated")?.textContent === "No");
          assert.equal(await page.locator("header, nav, form").count(), 0, "private canary remains isolated");
          assert.deepEqual(await page.locator("script").evaluateAll(els => els.map(el => el.getAttribute("src"))), ["posting-draft-canary.js"]);
          assert.equal(await page.locator('meta[name="robots"]').getAttribute("content"), "noindex, nofollow");
          assert.equal(await page.locator('meta[name="referrer"]').getAttribute("content"), "no-referrer");
          assert.equal(await page.locator("#canary-enabled").innerText(), "No");
          assert.equal(await page.locator("#canary-operation-id").innerText(), "None created");
          assert.equal(await page.locator("#canary-run").isDisabled(), true, "signed-out diagnostic cannot write");
          for (const selector of ["#canary-refresh", "#canary-run", "#canary-resume", "#canary-reload"]) await assertFits(page, selector);
          await page.unroute(sessionUrl);
          continue;
        }
        await page.goto(`${origin}/${file}`);
        await page.locator(".ab-site-header.is-enhanced").waitFor();
        assert.equal(await page.locator("[data-site-header]").count(), 1, file);
        assert.equal(await page.locator("body > .ab-footer").count(), 1, `one page footer outside article cards: ${file}`);
        assert.equal(await page.locator(".ab-footer").count(), 1, file);
        assert.equal(await page.locator("nav[aria-label='Primary navigation']").count(), 1, file);
        const links = await page.locator(".ab-site-nav a").evaluateAll(els => els.map(el => {
          const url = new URL(el.href);
          return { text: el.textContent.trim(), href: url.pathname + url.search + url.hash };
        }));
        assert.deepEqual(links, [{ text: "How it works", href: "/#how-it-works" }, { text: "Browse work", href: "/earn.html" }], file);
        const appearance = await page.locator("[data-site-header]").evaluate(el => {
          const css = getComputedStyle(el), brand = getComputedStyle(el.querySelector("strong"));
          return { height: el.getBoundingClientRect().height, background: css.backgroundImage, padding: css.padding, brand: brand.font, color: brand.color };
        });
        if (!reference) reference = appearance;
        assert.deepEqual(appearance, reference, `same homepage header: ${file} at ${width}`);
        assert.equal(await page.locator(".ab-site-menu, [data-site-header] .ab-site-login").count(), 0);
        for (const n of [1, 2]) await assertFits(page, `.ab-site-nav a:nth-child(${n})`);
      }
      console.log(`Shared homepage header and accessible links passed on ${pages.length} pages at ${width}px`);
      await page.goto(`${origin}/blog/`);
      await page.locator(".ab-site-login").click();
      await page.locator("[data-auth-dialog][open]").waitFor();
      await page.locator("[data-auth-close]").click();
      await page.locator(".ab-site-login").click();
      await page.locator("[data-auth-dialog][open]").waitFor();
      await ctx.close();
    }
    const nojs = await context({ javaScriptEnabled: false, viewport: { width: 390, height: 800 } });
    const fallback = await nojs.newPage();
    for (const file of ["index.html", "about.html", "blog/index.html", "metrics.html", "install/chatgpt-dev/index.html"]) {
      await fallback.goto(`${origin}/${file}`);
      for (const n of [1, 2]) await assertFits(fallback, `.ab-site-nav a:nth-child(${n})`);
    }
    await nojs.close();
    for (const [width, height] of [[1440,900],[946,838],[640,480],[390,600],[320,480],[768,320],[480,360]]) {
      const ctx = await context({ viewport: { width, height } }), page = await ctx.newPage();
      page.on("pageerror", e => errors.push(e.message));
      mode = "ready";
      await page.goto(`${origin}/metrics.html`);
      await page.waitForFunction(() => document.querySelector("[data-payout-volume]").textContent === "12 USDC");
      assert.equal(await page.locator("[data-settled-rounds]").innerText(), "2");
      assert.equal(await page.locator("[data-overview-ready]").innerText(), "3");
      assert.equal(await page.locator(".metrics-detail[open]").count(), 0);
      assert.match(await page.locator("[data-dashboard-notice]").innerText(), /incomplete/);
      assert.equal(await page.locator("[data-payout-chart] svg").count(), 1);
      if (process.env.NAV_LAYOUT_ARTIFACTS && [1440,390].includes(width)) {
        fs.mkdirSync(process.env.NAV_LAYOUT_ARTIFACTS, { recursive: true });
        await page.screenshot({ path: path.join(process.env.NAV_LAYOUT_ARTIFACTS, `metrics-${width}.png`), fullPage: true });
      }
      await assertFits(page, '[data-period="7d"]');
      await page.locator('[data-period="7d"]').click();
      await page.waitForFunction(() => document.querySelector("[data-payout-volume]").textContent === "7 USDC");
      assert.equal(await page.locator("[data-overview-ready]").innerText(), "3");
      await page.goto(`${origin}/metrics.html#payout-audit`);
      assert.equal(await page.locator("#payment-records").getAttribute("open"), "");
      await page.locator("#payout-audit-title").scrollIntoViewIfNeeded();
      assert.equal(await page.locator("#payout-audit-title").isVisible(), true);
      for (const id of ["participation", "funding-trends", "discovery-details", "data-sources"]) await page.locator(`#${id} > summary`).click();
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `no page clipping with all details open at ${width}`);
      await page.locator("[data-period='28d']").click();
      mode = "offline";
      await page.locator('[data-period="7d"]').click();
      await page.waitForFunction(() => document.querySelector("[data-overall-status]").dataset.status === "unavailable");
      assert.equal(await page.locator("[data-payout-volume]").innerText(), "—");
      assert.equal(await page.locator("[data-overview-ready]").innerText(), "—");
      assert.equal(await page.locator("[data-platform-revenue]").innerText(), "—");
      mode = "partial";
      await page.locator('[data-period="lifetime"]').click();
      await page.waitForFunction(() => document.querySelector("[data-payout-volume]").textContent === "12 USDC");
      assert.equal(await page.locator("[data-overview-ready]").innerText(), "—");
      mode = "zero";
      await page.locator('[data-period="7d"]').click();
      await page.waitForFunction(() => document.querySelector("[data-payout-volume]").textContent === "0 USDC");
      assert.equal(await page.locator("[data-overview-ready]").innerText(), "0");
      assert.deepEqual(errors, []);
      console.log(`Metrics overview, periods, proof deep link, scrolling and unavailable/partial/zero passed ${width}x${height}`);
      await ctx.close();
    }
  } finally { await browser.close(); }
}
main().catch(e => { console.error(e); process.exitCode = 1; }).finally(() => server.close());
