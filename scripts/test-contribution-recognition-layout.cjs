"use strict";
// Synthetic local evidence only. External requests are blocked before delivery.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const { fixture } = require("./fixtures/contribution-recognition.cjs");
const { shareText } = require("../site/collaborate/recognition.js");
const site = path.resolve(__dirname, "../site");
let source = fixture();
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json" };
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://localhost");
  if (url.pathname === "/collaborate/recognition.json") return res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(source));
  const name = url.pathname.endsWith("/") ? `${url.pathname}index.html` : url.pathname;
  const file = path.resolve(site, `.${name}`);
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const bytes = fs.readFileSync(file); res.writeHead(200, { "content-type": mime[path.extname(file)] || "application/octet-stream" }).end(bytes); }
  catch (_) { res.writeHead(404).end(); }
});
(async () => {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`, browser = await chromium.launch({ headless: true });
  try {
    for (const width of [1440, 390, 320]) {
      const context = await browser.newContext({ viewport: { width, height: 900 }, permissions: ["clipboard-read", "clipboard-write"] });
      const page = await context.newPage(), errors = [], blocked = [];
      page.on("pageerror", e => errors.push(e.message));
      await context.route("**/*", async route => {
        if (new URL(route.request().url()).origin !== origin) { blocked.push(route.request().url()); return route.abort(); }
        return route.continue();
      });
      source = fixture();
      await page.goto(`${origin}/collaborate/?analytics=off#contribution-synthetic-fixture`);
      const card = page.locator("#contribution-synthetic-fixture");
      await card.waitFor();
      assert.match(await card.innerText(), /Profile ownership not independently confirmed/);
      assert.match(await card.innerText(), /Payment is not assessed/);
      assert.ok((await card.boundingBox()).y >= 0, "direct card anchor is visible");
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), true, `${width}px overflow`);
      const copy = card.getByRole("button", { name: "Copy evidence card" });
      await copy.focus();
      await page.keyboard.press("Enter");
      await card.getByText("Copied. Sharing is optional; nothing was posted.", { exact: true }).waitFor();
      assert.equal(await page.evaluate(() => navigator.clipboard.readText()), shareText(source.records[0]));
      for (const theme of ["dark", "light"]) {
        await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        const contrast = await copy.evaluate(el => {
          const style = getComputedStyle(el);
          const luminance = rgb => rgb.match(/[\d.]+/g).slice(0, 3).map(Number).map(n => n / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((sum, v, i) => sum + v * [.2126, .7152, .0722][i], 0);
          const a = luminance(style.color), b = luminance(style.backgroundColor);
          return (Math.max(a, b) + .05) / (Math.min(a, b) + .05);
        });
        assert.ok(contrast >= 4.5, `${width}px ${theme}: evidence copy contrast ${contrast}`);
        if (process.env.CONTRIBUTION_EVIDENCE_DIR) {
          fs.mkdirSync(process.env.CONTRIBUTION_EVIDENCE_DIR, { recursive: true });
          await card.scrollIntoViewIfNeeded();
          await page.screenshot({ path: path.join(process.env.CONTRIBUTION_EVIDENCE_DIR, `recognition-${width}-${theme}.png`) });
        }
      }
      await page.evaluate(() => { Object.defineProperty(navigator.clipboard, "writeText", { value: () => Promise.reject(Error("denied")), configurable: true }); });
      await copy.click();
      const fallback = card.getByRole("textbox");
      await fallback.waitFor();
      assert.equal(await fallback.inputValue(), shareText(source.records[0]));
      assert.equal(await fallback.evaluate(el => document.activeElement === el), true);
      source = {};
      await page.reload();
      await page.getByText(/Recognition records could not be verified/).waitFor();
      assert.equal(await page.locator(".recognition-card").count(), 0);
      source = { ...fixture(), records: [] };
      await page.reload();
      await page.getByText(/No contributors have been listed through this opt-in process yet/).waitFor();
      assert.equal(await page.locator(".recognition-card").count(), 0);
      if (width === 390 && process.env.CONTRIBUTION_EVIDENCE_DIR) {
        await page.locator("#contributions").scrollIntoViewIfNeeded();
        await page.screenshot({ path: path.join(process.env.CONTRIBUTION_EVIDENCE_DIR, "recognition-empty-390.png") });
      }
      assert.deepEqual(errors, []);
      await context.close();
      process.stdout.write(`Recognition ${width}px: proof boundaries, anchor, keyboard copy/readback, fallback, empty/unavailable states; external requests blocked (${blocked.length})\n`);
    }
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); process.exitCode = 1; server.close(); });
