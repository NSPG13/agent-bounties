"use strict";
// Shared ⓘ info tip in a real browser: hover, keyboard focus, tap, tap outside and Esc;
// the icon's name and the tip in English and Spanish; and the tip on screen at 390px,
// in light and dark.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".woff2": "font/woff2", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".mp4": "video/mp4" };
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  let file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html"); res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file)); }
  catch (_) { res.writeHead(404).end(); }
});
const PAGES = [
  { path: "/api-integration.html", en: "Review the terms first. Fund upfront with USDC on Base.", es: "Revisa primero los términos. Financia por adelantado con USDC en Base." },
  { path: "/check-work.html", en: "Choose files or a folder. Folder names inside your work are kept.", es: "Elige archivos o una carpeta. Se conservan los nombres de las carpetas dentro de tu trabajo." },
];

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  try {
    for (const width of [1440, 390]) for (const theme of ["light", "dark"]) for (const target of PAGES) {
      const touch = width === 390;
      const context = await browser.newContext({ viewport: { width, height: touch ? 844 : 900 }, colorScheme: theme, reducedMotion: "reduce", hasTouch: touch, isMobile: touch });
      await context.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
      const page = await context.newPage(), errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(origin + target.path);
      const deny = page.locator("[data-google-analytics-deny]");
      if (await deny.count() && await deny.isVisible()) await deny.click();
      const tip = page.locator(".ab-info-tip-text", { hasText: target.en });
      const button = page.locator(`.ab-info:has(.ab-info-tip-text:text-is("${target.en}")) > button`);
      const where = `${target.path} ${width}px ${theme}`;
      await button.waitFor();
      assert.equal(await button.count(), 1, `${where}: one icon for the tip`);
      assert.equal(await tip.isHidden(), true, `${where}: the help text is no longer a visible line`);
      assert.equal(await button.getAttribute("type"), "button");
      assert.equal(await button.getAttribute("aria-label"), "What is this?");
      assert.equal(await button.getAttribute("aria-describedby"), await tip.getAttribute("id"));
      assert.equal(await tip.getAttribute("role"), "tooltip");
      const box = await button.boundingBox();
      assert.ok(box.width >= 24 && box.height >= 24, `${where}: the icon is at least 24px`);
      const colors = await button.evaluate((node) => ({ icon: getComputedStyle(node).color, page: getComputedStyle(document.body).backgroundColor }));
      assert.notEqual(colors.icon, colors.page, `${where}: the icon is visible against the page`);
      await button.scrollIntoViewIfNeeded();
      const onScreen = async () => {
        const r = await tip.boundingBox();
        assert.ok(r.x >= 7.5 && r.x + r.width <= width - 7.5, `${where}: the tip stays 8px inside the screen (${Math.round(r.x)}–${Math.round(r.x + r.width)})`);
        assert.ok(r.width <= 240.5, `${where}: the tip is at most 240px wide`);
      };

      if (touch) {
        await button.tap();
        assert.equal(await tip.isVisible(), true, `${where}: a tap opens the tip`);
        assert.equal(await button.getAttribute("aria-expanded"), "true");
        await onScreen();
        await button.tap();
        assert.equal(await tip.isHidden(), true, `${where}: a second tap closes it`);
        await button.tap();
        await page.touchscreen.tap(5, 300);
        assert.equal(await tip.isHidden(), true, `${where}: a tap outside closes it`);
      } else {
        await button.hover();
        assert.equal(await tip.isVisible(), true, `${where}: hover opens the tip`);
        await onScreen();
        await page.mouse.move(5, 5);
        await page.waitForTimeout(50);
        assert.equal(await tip.isHidden(), true, `${where}: moving away closes it`);
        await button.click();
        await page.mouse.move(5, 5);
        await page.waitForTimeout(50);
        assert.equal(await tip.isVisible(), true, `${where}: a click pins it open`);
        await page.mouse.click(5, 300);
        assert.equal(await tip.isHidden(), true, `${where}: a click outside closes it`);
      }
      // Inside a transformed container (like the animated account dialog) the tip still
      // opens right under its icon, not offset by the container's position.
      await button.evaluate((node) => { const box = node.closest(".ab-info").parentElement; box.style.transform = "translateY(0) scale(1)"; box.style.marginLeft = "4px"; });
      await button.focus();
      const gap = await button.evaluate((node) => { const icon = node.getBoundingClientRect(), tip = node.nextElementSibling.getBoundingClientRect(); return { below: tip.top - icon.bottom, above: icon.top - tip.bottom, left: tip.left, right: tip.right }; });
      assert.ok(Math.abs(gap.below - 6) <= 1.5 || Math.abs(gap.above - 6) <= 1.5, `${where}: in a transformed container the tip sits 6px from its icon (${JSON.stringify(gap)})`);
      assert.ok(gap.left >= 7.5 && gap.right <= width - 7.5, `${where}: in a transformed container the tip stays on screen`);
      await page.keyboard.press("Escape");
      await button.evaluate((node) => { const box = node.closest(".ab-info").parentElement; box.style.transform = ""; box.style.marginLeft = ""; node.blur(); });
      // Keyboard: focus opens, Esc closes and keeps focus on the icon.
      await button.focus();
      assert.equal(await tip.isVisible(), true, `${where}: keyboard focus opens the tip`);
      await page.keyboard.press("Escape");
      assert.equal(await tip.isHidden(), true, `${where}: Esc closes it`);
      assert.equal(await button.evaluate((node) => node === document.activeElement), true, `${where}: focus stays on the icon`);
      assert.deepEqual(errors, [], `${where}: no page errors`);

      // Spanish: the icon's name and the tip follow the language switch.
      {
        await page.goto(origin + target.path + "?lang=es");
        const esButton = page.locator(".ab-info > button").first();
        await esButton.waitFor();
        await page.waitForFunction(() => document.documentElement.lang === "es");
        assert.equal(await esButton.getAttribute("aria-label"), "¿Para qué sirve?", `${where}: Spanish name for the icon`);
        assert.equal(await page.locator(".ab-info-tip-text", { hasText: target.es }).count(), 1, `${where}: the tip is in Spanish`);
        await page.locator("[data-lang-choice=\"en\"]").first().click();
        assert.equal(await page.locator(".ab-info > button").first().getAttribute("aria-label"), "What is this?", `${where}: English restores the name`);
      }
      await context.close();
    }
    console.log("info tip browser checks passed");
  } finally {
    await browser.close();
    server.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
