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
      // A competition's qualifying bounty still opens the "Choose how to start" dialog in place.
      await page.goto(origin + "/?parentCompetition=0x" + "1".repeat(40) + "&network=base-mainnet");
      await page.locator("#post-a-bounty").click();
      await page.locator("[data-bounty-launcher][open]").waitFor();
      assert.equal(new URL(page.url()).pathname, "/", "the competition flow keeps its dialog");
      assert.equal(await page.locator("[data-bounty-manual]").isVisible(), false, "the competition dialog offers no unbound manual form");
      await page.waitForFunction(() => /parent competition|could not be verified/.test(document.querySelector("[data-bounty-launch-status]").textContent));
      // Older links to #post-a-bounty go straight to the form.
      await page.goto(origin + "/#post-a-bounty");
      await page.waitForURL("**/post.html?mode=manual#bounty-composer-form");
      // "Post a bounty" opens the posting form directly, with no chooser dialog.
      await page.goto(origin);
      assert.equal(await page.locator("#post-a-bounty").getAttribute("aria-haspopup"), null, "the button no longer announces a dialog");
      await page.locator("#post-a-bounty").click();
      await page.waitForURL("**/post.html?mode=manual#bounty-composer-form");
      // AI browser extensions are recommended beside the form (below it on narrow screens), linking to official pages.
      const help = page.locator(".posting-ai-help");
      assert.deepEqual(await help.locator("a").evaluateAll(links => links.map(link => [link.textContent.trim(), link.href, link.rel, link.target])), [
        ["ChatGPT (recommended)", "https://chromewebstore.google.com/detail/chatgpt/hehggadaopoacecdllhhajmbjkdcmajg", "noopener noreferrer", "_blank"],
        ["Claude", "https://chromewebstore.google.com/detail/claude/fcoeoabgfenejglbffodgkkbkcdhcgfn", "noopener noreferrer", "_blank"]]);
      const helpBox = await help.boundingBox(), formBox = await page.locator("#bounty-composer-form").boundingBox();
      if (width >= 1180) assert.ok(helpBox.x >= formBox.x + formBox.width && helpBox.y < formBox.y + formBox.height, "the help sits beside the form");
      else assert.ok(helpBox.y >= formBox.y + formBox.height - 1, "the help sits below the form");
      assert.equal(await page.locator("[data-brief-mode]").inputValue(), "manual");
      await page.locator("[data-brief-title]").fill("Research competitors");
      await page.locator("[data-brief-objective]").fill("Compare competitor prices");
      // Criteria are written in one box, one per line.
      await page.locator("[data-brief-criteria]").fill("Includes prices and services\nDelivered as a table\nInclude 20 competitors");
      await page.locator("[data-brief-budget]").fill("20.01");
      assert.equal(await page.locator("[data-brief-reserve]").count(), 0, "the review reward is not a form field");
      assert.match(await page.locator("[data-brief-review-share]").innerText(), /10% of the total \(2\.00 USDC\)/);
      await page.locator("[data-brief-deadline]").fill(new Date(Date.now() + 7*86400000).toISOString().slice(0,16));
      await page.reload();
      assert.equal(await page.locator("[data-brief-title]").inputValue(), "Research competitors");
      assert.equal(await page.locator("[data-manual-review-consent]").count(), 0, "no separate review checkbox");
      assert.equal(await page.locator("[data-posting-basics]").count(), 0, "the form opens without the explainer box");
      await page.locator("[data-composer-submit]").click();
      await page.locator("#bounty-preview:not([hidden])").waitFor();
      await page.waitForFunction(() => document.querySelector("[data-brief-status]").textContent.startsWith("Proposal prepared"));
      // Routine draft messages and the continuation link stay out of sight; only problems show.
      for (const quiet of ["[data-brief-status]", "[data-draft-sync-status]", "[data-copy-continuation]", "[data-continue-posting]"]) assert.equal(await page.locator(quiet).isVisible(), false, quiet);
      const draft = await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().draft);
      assert.equal(draft.title, "Research competitors");
      assert.equal(draft.goal, "Compare competitor prices", "with no description the objective is the goal");
      assert.equal(await page.locator("#bounty-composer-input").inputValue(), "", "the description stays empty");
      assert.equal(draft.solver_reward_usdc, "18.01");
      assert.equal(draft.verifier_reward_usdc, "2.00", "10% of the total goes to the creator's review");
      assert.equal(draft.benchmark.engine, "creator_review_v1");
      assert.deepEqual(draft.acceptance_criteria, ["Includes prices and services", "Delivered as a table", "Include 20 competitors"]);
      assert.equal(await page.locator("[data-open-funding]").isDisabled(), true, "preparing does not approve or fund");
      await page.locator("[data-revise-card]").click();
      // "Edit brief" focuses the description on the next frame; wait for it before typing elsewhere.
      await page.waitForFunction(() => document.activeElement?.id === "bounty-composer-input");
      assert.equal(await page.locator("#bounty-composer-input").inputValue(), "");
      assert.equal(await page.locator("[data-composer-label]").innerText(), "Description");
      await page.locator("[data-brief-criteria]").fill("Includes prices and services\nDelivered as a table\nInclude 25 competitors");
      assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().draft_stale), true);
      await page.locator("[data-composer-submit]").click();
      await page.waitForFunction(() => document.querySelector("[data-card-criteria]").textContent.includes("25 competitors"));
      assert.equal(await page.evaluate(() => window.AgentBountiesWorkflow.createClient(window).load().draft.goal), "Compare competitor prices", "editing criteria does not copy them into the goal");
      await page.reload();
      await page.locator("#bounty-preview:not([hidden])").waitFor();
      assert.equal(await page.locator("[data-brief-mode]").inputValue(), "manual");
      assert.equal(await page.locator("[data-brief-criteria]").inputValue(), "Includes prices and services\nDelivered as a table\nInclude 25 competitors");
      assert.equal(await page.locator("[data-open-funding]").isDisabled(), true);
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), "no horizontal overflow");
      fs.mkdirSync("target/manual-posting", {recursive:true});
      await page.screenshot({path:`target/manual-posting/review-${width}.png`,fullPage:true});
      await page.locator("[data-brief-mode]").selectOption("ai");
      assert.equal(await page.locator("[data-brief-title]").isVisible(), false, "AI mode hides the manual fields");
      assert.equal(await page.locator("[data-ai-options] > summary").isVisible(), true, "AI mode shows the AI handoff");
      assert.deepEqual(externalWrites, [], "preparation never calls a model, publishes or requests payment");
      assert.deepEqual(errors, []);
      await context.close(); console.log(`Manual entry, validation, native review, editing and recovery pass at ${width}px`);
    }
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => { console.error(error); server.close(); process.exitCode=1; });
