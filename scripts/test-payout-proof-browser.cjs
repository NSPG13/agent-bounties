"use strict";
// Isolated source fixtures; no production, account, wallet or payment calls.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const fixture = require("./fixtures/historical-payout-proof.json");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json", ".svg": "image/svg+xml", ".woff2": "font/woff2" };
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost"), file = path.resolve(site, "." + url.pathname);
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const bytes = fs.readFileSync(file); res.writeHead(200, { "content-type": mime[path.extname(file)] || "application/octet-stream" }).end(bytes); }
  catch { res.writeHead(404).end(); }
});
(async () => {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined });
  try {
    for (const width of [320, 390, 1440]) {
      const context = await browser.newContext({ viewport: { width, height: 900 }, reducedMotion: "reduce" });
      const page = await context.newPage(), errors = [];
      page.on("pageerror", error => errors.push(error.message));
      let mode = "ready", period = "lifetime";
      await context.route("**/*", route => {
        const url = new URL(route.request().url());
        const data = structuredClone(fixture);
        if (url.pathname === "/v1/metrics/platform") {
          period = url.searchParams.get("period"); data.platform.window.period = period;
          data.platform.generated_at = new Date().toISOString();
          return route.fulfill({ json: data.platform });
        }
        if (url.pathname === "/v1/metrics/platform/payouts") {
          assert.equal(url.searchParams.get("snapshot"), data.platform.payout_proof.snapshot);
          if (mode === "changed") return route.fulfill({ status: 409, json: { code: "snapshot_changed", restart_required: true } });
          const result = data.pages[0]; result.period = period;
          if (mode === "mismatch") { result.records = result.records.slice(1); result.total_rows -= 1; }
          return route.fulfill({ json: result });
        }
        if (url.origin !== origin) return route.fulfill({ status: 503, body: "Isolated test" });
        if (url.pathname.startsWith("/generated/")) return route.fulfill({ status: 503, body: "Isolated test" });
        return route.continue();
      });
      await page.goto(`${origin}/metrics.html#payout-audit`);
      await page.waitForFunction(() => document.querySelector("[data-audit-status]").textContent === "reconciled");
      assert.equal(await page.locator("[data-audit-total]").innerText(), "73.065 USDC");
      assert.equal(await page.locator("[data-audit-settlements]").innerText(), "57");
      assert.equal(await page.locator("[data-audit-payout-events]").innerText(), "65");
      assert.equal(await page.locator("[data-audit-rows] tr").count(), 65);
      assert.match(await page.locator("[data-dashboard-notice]").innerText(), /incomplete/);
      const links = await page.locator("[data-audit-rows] a").evaluateAll(nodes => nodes.map(node => ({ text: node.textContent, href: node.href })));
      assert.equal(links.length, 130);
      assert.ok(links.filter(link => link.text === "Proof JSON").every(link => new URL(link.href).pathname === "/v1/metrics/platform/payouts"));
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${width}: no document overflow`);
      mode = "changed";
      await page.locator('[data-period="90d"]').focus(); await page.keyboard.press("Enter");
      await page.waitForFunction(() => document.querySelector("[data-audit-status]").textContent === "unavailable");
      assert.equal(await page.locator("[data-audit-total]").innerText(), "—");
      assert.equal(await page.locator("[data-audit-rows] tr").count(), 0);
      mode = "ready";
      await page.locator('[data-period="lifetime"]').focus(); await page.keyboard.press("Enter");
      await page.waitForFunction(() => document.querySelector("[data-audit-status]").textContent === "reconciled");
      await page.reload();
      await page.waitForFunction(() => document.querySelector("[data-audit-status]").textContent === "reconciled");
      mode = "mismatch";
      await page.locator('[data-period="90d"]').focus(); await page.keyboard.press("Enter");
      await page.waitForFunction(() => document.querySelector("[data-audit-status]").textContent === "mismatch");
      assert.match(await page.locator("[data-audit-copy]").innerText(), /does not match/);
      assert.deepEqual(errors, []);
      console.log(`Historical proof:65rows/exact total/links/keyboard409recovery/reload/mismatch/no overflow pass at${width}`);
      await context.close();
    }
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => server.close());
