"use strict";
// English | Español on the home page: default English, Spanish via the switch or
// ?lang=es, remembered, <html lang> follows, and English restores the original text.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site"), shots = process.env.HOME_LANGUAGE_SHOTS || "";
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".woff2": "font/woff2", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg", ".mp4": "video/mp4" };
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  let file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html"); res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file)); }
  catch (_) { res.writeHead(404).end(); }
});

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  const bodyText = (page) => page.evaluate(() => document.body.innerText);
  try {
    for (const width of [1440, 390]) for (const theme of ["light", "dark"]) {
      const context = await browser.newContext({ viewport: { width, height: width === 390 ? 844 : 900 }, colorScheme: theme, reducedMotion: "reduce", deviceScaleFactor: shots ? 2 : 1 });
      await context.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
      const page = await context.newPage(), errors = [];
      page.on("pageerror", (error) => errors.push(error.message));
      await page.goto(origin + "/");
      await page.evaluate(() => document.fonts.ready);
      const deny = page.locator("[data-google-analytics-deny]");
      if (await deny.count() && await deny.isVisible()) await deny.click();
      const control = page.locator(".ab-footer .ab-lang-control");
      assert.equal(await page.locator("[data-site-header] .ab-lang-control, [data-site-header] [data-lang-choice]").count(), 0, "the header has no language switch");
      assert.equal(await page.locator("html").getAttribute("lang"), "en", "English by default");
      assert.equal(await control.locator('button[data-lang-choice="en"]').getAttribute("aria-pressed"), "true");
      assert.deepEqual(await control.locator("button").allInnerTexts(), ["English", "Español"]);
      await control.scrollIntoViewIfNeeded();
      assert.ok(await control.isVisible(), "the switch is visible in the footer");
      // Beside Light | Dark: one group, same line, same format, no overlap.
      const layout = await page.evaluate(() => {
        const lang = document.querySelector(".ab-footer .ab-lang-control"), theme = document.querySelector(".ab-footer .ab-theme-control");
        const box = (node) => node.getBoundingClientRect(), style = (node) => getComputedStyle(node);
        const pick = (node) => ({ height: box(node).height, padding: style(node).padding, border: style(node).border, radius: style(node).borderRadius, font: style(node).font, color: style(node).color, background: style(node).backgroundColor });
        const button = (control, pressed) => control.querySelector(`button[aria-pressed="${pressed}"]`);
        return { sameGroup: lang.parentElement === theme.parentElement && lang.parentElement.classList.contains("ab-footer-controls") && lang.nextElementSibling === theme,
          a: box(lang), b: box(theme), control: [pick(lang), pick(theme)], on: [pick(button(lang, "true")), pick(button(theme, "true"))], off: [pick(button(lang, "false")), pick(button(theme, "false"))] };
      });
      assert.ok(layout.sameGroup, "the language switch sits directly beside the theme switch");
      assert.ok(Math.abs((layout.a.top + layout.a.height / 2) - (layout.b.top + layout.b.height / 2)) < 1, "both are vertically centered on one line");
      assert.ok(layout.a.right <= layout.b.left, "the two switches do not overlap");
      assert.deepEqual(layout.control[0], layout.control[1], "the containers share one format");
      assert.deepEqual(layout.on[0], layout.on[1], "the active buttons share one format");
      assert.deepEqual(layout.off[0], layout.off[1], "the inactive buttons share one format");
      await page.waitForTimeout(1500); // Let the page's own scripts settle their text first.
      const english = await bodyText(page), englishTitle = await page.title();
      if (shots) await page.screenshot({ path: path.join(shots, `home-en-${width}-${theme}.png`) });

      await control.locator('button[data-lang-choice="es"]').click();
      assert.equal(await page.locator("html").getAttribute("lang"), "es");
      assert.equal(await page.title(), "Agentizable.ai | Paga por resultados verificables");
      assert.match(await page.locator("#post-a-bounty").innerText(), /Publicar una recompensa/);
      assert.equal(await control.locator('button[data-lang-choice="es"]').getAttribute("aria-pressed"), "true");
      assert.deepEqual(await control.locator("button").allInnerTexts(), ["English", "Español"], "the switch itself is not translated");
      assert.equal(await page.evaluate(() => localStorage.getItem("agent-bounties.lang")), "es");
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "Spanish text fits without horizontal scroll");
      if (shots) await page.screenshot({ path: path.join(shots, `home-es-${width}-${theme}.png`) });
      // Text the page writes later is translated too.
      await page.locator("[data-deck-next]").click();
      assert.equal(await page.locator("[data-deck-status]").innerText(), "Ejemplo 2 de 3");

      await page.reload();
      assert.equal(await page.locator("html").getAttribute("lang"), "es", "the choice is remembered");
      assert.match(await page.locator("#post-a-bounty").innerText(), /Publicar una recompensa/);

      await page.locator('.ab-footer .ab-lang-control button[data-lang-choice="en"]').click();
      assert.equal(await page.locator("html").getAttribute("lang"), "en");
      assert.equal(await page.locator("[data-missing-ingredient]").innerText(), "More compute", "the hero phrase returns to English at once");
      await page.waitForTimeout(500);
      assert.equal(await bodyText(page), english, "English restores the original text exactly");
      assert.equal(await page.title(), englishTitle);

      // ?lang= wins over the saved choice and is remembered.
      await page.goto(origin + "/?lang=es");
      assert.equal(await page.locator("html").getAttribute("lang"), "es");
      await page.goto(origin + "/");
      assert.equal(await page.locator("html").getAttribute("lang"), "es");
      await page.goto(origin + "/?lang=en");
      assert.equal(await page.locator("html").getAttribute("lang"), "en");
      assert.deepEqual(errors, []);
      await context.close();
    }
    {
      // With motion, the hero types its phrase in Spanish; switching back shows English at once.
      const motion = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "no-preference" });
      await motion.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
      const page = await motion.newPage();
      await page.goto(origin + "/?lang=es");
      await page.waitForFunction(() => document.querySelector("[data-missing-ingredient]").textContent === "datos privados", null, { timeout: 15000 });
      await page.locator('.ab-footer .ab-lang-control button[data-lang-choice="en"]').click();
      const english = ["More compute", "private data", "specialized software", "greater intelligence", "more agents", "an Independent verifier", "the right credentials", "influence with X"];
      assert.ok(english.includes(await page.locator("[data-missing-ingredient]").textContent()), "the typed phrase is English right after switching");
      await motion.close();
    }
    {
      // A bounty titled with a dictionary phrase keeps its own words in Spanish mode.
      const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });
      await ctx.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
      const page = await ctx.newPage();
      await page.goto(origin + "/post.html?lang=es");
      await page.evaluate(() => { const title = document.querySelector("[data-card-title]"); title.textContent = "Research"; document.querySelector("[data-card-criteria]").innerHTML = "<li>Pass the regression tests</li>"; });
      await page.waitForTimeout(200);
      assert.equal(await page.locator("[data-card-title]").textContent(), "Research", "user titles are never translated");
      assert.equal(await page.locator("[data-card-criteria] li").textContent(), "Pass the regression tests", "user criteria are never translated");
      await ctx.close();
    }
    // Every page offers the switch, and the shared header and footer follow the choice.
    const context = await browser.newContext({ viewport: { width: 390, height: 844 } });
    await context.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
    const page = await context.newPage();
    for (const file of ["post.html", "earn.html", "about.html", "blog/"]) {
      await page.goto(`${origin}/${file}?lang=es`);
      const control = page.locator(".ab-footer .ab-footer-controls > .ab-lang-control");
      assert.equal(await control.count(), 1, `${file} has the language switch in the footer`);
      assert.equal(await page.locator("[data-site-header] [data-lang-choice]").count(), 0, `${file} header has no language switch`);
      assert.equal(await page.locator("html").getAttribute("lang"), "es", `${file} follows the choice`);
      assert.equal(await page.locator(".ab-footer").getByText("Términos del servicio").count(), 1, `${file} footer is translated`);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), `${file} fits at 390px`);
    }
    await context.close();
    console.log("Language switch: on every page; home fully in Español via switch and ?lang, remembered, html lang, exact English restore at 1440px and 390px, light and dark");
  } finally { await browser.close(); server.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
