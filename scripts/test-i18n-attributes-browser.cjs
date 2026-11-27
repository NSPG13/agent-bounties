"use strict";
// Español on post.html: the form's placeholders, including the textareas', switch to
// Spanish in manual and AI mode; what a person typed in a textarea is never translated;
// and English restores the original placeholders.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".woff2": "font/woff2", ".png": "image/png", ".webp": "image/webp", ".jpg": "image/jpeg" };
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  let file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html"); res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file)); }
  catch (_) { res.writeHead(404).end(); }
});
const EN = {
  objective: "What should this bounty achieve? e.g. Get more customers to contact my business",
  description: "Context agents need: links, background, preferences.",
  acceptance_criteria: "One check per line, e.g.\nLanding page loads on mobile\nIncludes a contact form",
};
const ES = {
  objective: "¿Qué debe lograr esta recompensa?",
  description: "Contexto que necesitan los agentes: enlaces, antecedentes, preferencias.",
  acceptance_criteria: "Una comprobación por línea, p. ej.:\nLa página carga en el móvil\nIncluye un formulario de contacto",
};

(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  try {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: "reduce" });
    await context.route("**/*", (route) => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "" }));
    const page = await context.newPage(), errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(origin + "/post.html?lang=es");
    await page.waitForFunction(() => document.documentElement.lang === "es");
    const placeholder = (name) => page.locator(`textarea[name="${name}"]`).getAttribute("placeholder");
    const mode = page.locator("[data-brief-mode]");
    await mode.selectOption("manual");
    assert.ok((await placeholder("objective")).startsWith(ES.objective), "objective placeholder in Spanish");
    assert.equal(await placeholder("description"), ES.description, "description placeholder in Spanish");
    assert.equal(await placeholder("acceptance_criteria"), ES.acceptance_criteria, "criteria placeholder in Spanish, line breaks kept");
    await mode.selectOption("ai");
    assert.equal(await placeholder("description"), "Describe el resultado que necesitas.", "the AI-mode placeholder a script sets is translated too");
    await mode.selectOption("manual");
    assert.equal(await placeholder("description"), ES.description, "back in manual mode it is Spanish again");

    // A person's words in a textarea stay exactly as written, even when they match a dictionary phrase.
    await page.locator('textarea[name="description"]').fill("Post a bounty");
    await page.locator('textarea[name="description"]').evaluate((node) => { node.textContent = "Post a bounty"; });
    await page.waitForTimeout(50);
    assert.equal(await page.locator('textarea[name="description"]').inputValue(), "Post a bounty", "typed text is not translated");
    assert.equal(await page.locator('textarea[name="description"]').evaluate((node) => node.textContent), "Post a bounty", "textarea content is not translated");

    await page.locator('[data-lang-choice="en"]').first().click();
    for (const name of Object.keys(EN)) assert.equal(await placeholder(name), EN[name], `English restores the ${name} placeholder`);
    assert.deepEqual(errors, [], "no page errors");
    console.log("post.html placeholders: Spanish in manual and AI mode, typed text untouched, English restored");
  } finally {
    await browser.close();
    server.close();
  }
})().catch((error) => { console.error(error); process.exitCode = 1; });
