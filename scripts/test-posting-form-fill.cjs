"use strict";
// The posting form is the only place a bounty is published. Agents fill the same named inputs a person
// types; WebMCP reports field errors and never stages, approves, signs or funds on its own.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site");
const server = http.createServer((req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  const file = path.resolve(site, "." + (pathname === "/" ? "/index.html" : pathname));
  if (!file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { const body = fs.readFileSync(file); res.writeHead(200, { "Content-Type": ({ ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".json": "application/json" })[path.extname(file)] || "application/octet-stream" }).end(body); }
  catch (_) { res.writeHead(404).end(); }
});
(async () => {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ headless: true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined });
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, reducedMotion: "reduce", timezoneId: "America/Mexico_City" });
    const writes = [], errors = [];
    await context.route("**/*", (route) => {
      const request = route.request(), url = new URL(request.url());
      if (url.origin === origin) return route.continue();
      if (request.method() !== "GET") writes.push(url.href);
      if (url.pathname.endsWith("/session")) return route.fulfill({ json: { authenticated: false, providers: { google: true }, posting_drafts_enabled: false } });
      return route.fulfill({ status: 503, body: "Isolated browser test" });
    });
    const page = await context.newPage(); page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(origin + "/post.html?mode=manual");
    await page.waitForFunction(() => window.AgentBountiesWebMCP?.call && window.AgentBountiesPostingBrief);
    const call = (input) => page.evaluate((value) => window.AgentBountiesWebMCP.call("agent_bounties_fill_posting_form", value), input);

    // The approved field names fill the same inputs a person sees, by name.
    const deadline = new Date(Date.now() + 5 * 86400000).toISOString();
    const filled = await call({ title: "Landing page", objective: "Get more customers to contact my business", description: "Spanish and English.",
      acceptance_criteria: ["Loads on mobile", "Includes a contact form"], total_budget_usdc: "10.00", delivery_deadline: deadline });
    assert.equal(filled.status, "form_filled", JSON.stringify(filled.field_errors));
    assert.deepEqual(filled.field_errors, []);
    assert.equal(filled.approval_required, true);
    const named = await page.evaluate(() => Object.fromEntries(["title", "objective", "description", "acceptance_criteria", "total_budget_usdc", "delivery_deadline", "timezone"]
      .map((name) => [name, document.querySelector(`#bounty-composer-form [name="${name}"]`)?.value])));
    assert.equal(named.title, "Landing page");
    assert.equal(named.objective, "Get more customers to contact my business");
    assert.equal(named.description, "Spanish and English.");
    assert.equal(named.acceptance_criteria, "Loads on mobile\nIncludes a contact form");
    assert.equal(named.total_budget_usdc, "10.00");
    assert.equal(named.timezone, "America/Mexico_City");
    assert.equal(await page.evaluate((iso) => window.AgentBountiesPostingBrief.wallTime(Date.parse(iso), "America/Mexico_City"), deadline), named.delivery_deadline, "the deadline shows in the person's time zone");
    assert.match(await page.locator("[data-brief-review-share]").textContent(), /1\.00 USDC/, "the platform's 10% review reward");
    assert.equal(await page.locator("#bounty-preview").isVisible(), false, "filling never stages the review card");

    // The person presses Review draft; the card shows the platform split.
    await page.locator("[data-composer-submit]").click();
    await page.locator("#bounty-preview").waitFor({ state: "visible" });
    await page.waitForFunction(() => /9\.00/.test(document.querySelector("[data-card-reward]")?.textContent || ""));

    // Field errors come back with stable codes; the form keeps what was filled.
    const tooSmall = await call({ title: "Small", objective: "Try", acceptance_criteria: ["One"], total_budget_usdc: "2.00", delivery_deadline: deadline });
    assert.equal(tooSmall.status, "form_needs_changes");
    assert.ok(tooSmall.field_errors.some((item) => item.code === "min_total_budget" && item.field === "total_budget_usdc"), JSON.stringify(tooSmall.field_errors));
    const late = await call({ title: "Late", objective: "Try", acceptance_criteria: ["One"], total_budget_usdc: "5", delivery_deadline: new Date(Date.now() + 40 * 86400000).toISOString() });
    assert.ok(late.field_errors.some((item) => item.field === "delivery_deadline"), JSON.stringify(late.field_errors));

    // Deprecated names still work: goal → objective, the two rewards → total, a day count → deadline.
    const legacy = await call({ title: "Old agent", goal: "Fix the parser", acceptance_criteria: ["Tests pass"], solver_reward_usdc: "9", verifier_reward_usdc: "1", task_window_days: 3 });
    assert.equal(legacy.status, "form_filled", JSON.stringify(legacy.field_errors));
    assert.equal(await page.locator('[name="objective"]').inputValue(), "Fix the parser");
    assert.equal(await page.locator('[name="total_budget_usdc"]').inputValue(), "10.00");

    // Automated verification is an agents-only section: collapsed and read-only until an agent fills it.
    assert.equal(await page.locator("[data-advanced-verification]").evaluate((node) => node.open), false);
    assert.equal(await page.locator('[name="benchmark"]').evaluate((node) => node.readOnly), true);
    await call({ title: "Parser", objective: "Fix the parser", acceptance_criteria: ["Tests pass"], total_budget_usdc: "10", delivery_deadline: deadline, benchmark: { kind: "command", command: "npm test" }, evidence_schema: { type: "object" } });
    assert.equal(await page.locator("[data-advanced-verification]").evaluate((node) => node.open), true);
    assert.deepEqual(JSON.parse(await page.locator('[name="benchmark"]').inputValue()), { kind: "command", command: "npm test" });

    // From another page, the values wait for post.html and fill it there.
    await page.goto(origin + "/earn.html");
    await page.waitForFunction(() => window.AgentBountiesWebMCP?.call);
    const away = await page.evaluate(() => window.AgentBountiesWebMCP.call("agent_bounties_fill_posting_form", { title: "From the board", objective: "A small fix", acceptance_criteria: ["Done"], total_budget_usdc: "3" }));
    assert.equal(away.status, "navigating_to_form");
    await page.waitForURL("**/post.html?from=webmcp");
    await page.waitForFunction(() => document.querySelector('[name="title"]')?.value === "From the board");

    // A prepared link (MCP prepare_bounty_post) prefills post.html with the same names; older links still work.
    const linkDeadline = new Date(Date.now() + 4 * 86400000).toISOString();
    for (const [label, query] of [
      ["new names", { title: "Linked bounty", objective: "Translate the help page", description: "Spanish (Mexico).", acceptance_criteria: ["Every heading translated", "Links unchanged"], total_budget_usdc: "10.00", delivery_deadline: linkDeadline }],
      ["older names", { title: "Linked bounty", goal: "Translate the help page\n\nDetails: Spanish (Mexico).", criterion: ["Every heading translated", "Links unchanged"], solverReward: "9.80", verifierReward: "0.20", deliveryDeadline: linkDeadline }],
    ]) {
      const linkPage = await context.newPage(); linkPage.on("pageerror", (error) => errors.push(error.message));
      const url = new URL(origin + "/post.html");
      url.searchParams.set("from", "ai-app"); url.searchParams.set("operation_id", "6b2a0000-0000-4000-8000-00000000000" + (label === "new names" ? "1" : "2")); url.searchParams.set("reviewMode", "creator");
      for (const [key, value] of Object.entries(query)) for (const item of [].concat(value)) url.searchParams.append(key, item);
      await linkPage.goto(url.href);
      await linkPage.locator("#bounty-preview").waitFor({ state: "visible" });
      await linkPage.waitForFunction(() => /9\.00/.test(document.querySelector("[data-card-reward]")?.textContent || ""));
      assert.equal(await linkPage.locator("[data-card-title]").textContent(), "Linked bounty", label);
      assert.match(await linkPage.locator("[data-card-goal]").textContent(), /Translate the help page\s+Details: Spanish \(Mexico\)\./, label);
      assert.equal(await linkPage.locator("[data-card-criteria] li").count(), 2, label);
      assert.equal(await linkPage.evaluate(() => window.AgentBountiesComposer.review().explicitly_approved), false, `${label}: nothing is approved until the person approves`);
      await linkPage.close();
    }

    assert.deepEqual(writes, [], "filling the form makes no external writes");
    assert.deepEqual(errors, []);
    console.log("Posting form fill: named fields, platform split, coded field errors, deprecated aliases, agents-only verification, cross-page handoff; nothing staged or signed");
  } finally { await browser.close(); server.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
