"use strict";
// Browser journeys for the new presentation layer. All services are isolated fixtures.
const assert = require("node:assert/strict"), fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const site = path.resolve(__dirname, "../site"), artifacts = process.env.FOREST_UI_ARTIFACTS;
const mime = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".json": "application/json", ".webp": "image/webp", ".mp4": "video/mp4", ".ttf": "font/ttf", ".woff2": "font/woff2" };
const server = http.createServer((req, res) => {
  let file = path.resolve(site, "." + decodeURIComponent(new URL(req.url, "http://localhost").pathname));
  if (file !== site && !file.startsWith(site + path.sep)) return res.writeHead(403).end();
  try { if (fs.statSync(file).isDirectory()) file = path.join(file, "index.html"); res.writeHead(200, { "Content-Type": mime[path.extname(file)] || "application/octet-stream" }).end(fs.readFileSync(file)); }
  catch { res.writeHead(404).end(); }
});
function leaderboard(populated) {
  const data = period => ({ reward_usdc: period === "daily" ? "3" : "26", reward_funding_status: "partially_funded", reward_payout_status: "not_paid", ranking: { period: { kind: period, starts_at: "2026-09-10T00:00:00Z", ends_at: period === "daily" ? "2026-09-11T00:00:00Z" : "2026-09-17T00:00:00Z" }, rules: ["Rank by qualifying completions."], entries: populated ? [3,1,2].map(rank => ({ rank, solver_wallet: `0x${String(rank).repeat(40)}`, prize_eligible_bounties: 10 - rank, eligible_solver_rewards_usdc_base_units: `${(10-rank)*1000000}` })) : [] } });
  return { schema_version: "agent-bounties/solver-leaderboard-v1", network: "base-mainnet", daily: data("daily"), weekly: data("weekly") };
}
async function capture(page, name) {
  if (!artifacts) return;
  fs.mkdirSync(artifacts, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: path.join(artifacts, name + ".png"), fullPage: !(await page.locator("dialog[open]").count()), animations: "disabled" });
  await page.screenshot({ path: path.join(artifacts, name + "-viewport.png"), fullPage: false, animations: "disabled" });
}
async function fits(page, selector) {
  const result = await page.locator(selector).evaluate(el => {
    const rect = el.getBoundingClientRect(), hit = document.elementFromPoint(rect.x + rect.width/2, rect.y + rect.height/2);
    return { fits: rect.x >= 0 && rect.right <= innerWidth && rect.height > 0, hit: el === hit || el.contains(hit) };
  });
  assert.ok(result.fits && result.hit, selector + JSON.stringify(result));
}
async function main() {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({headless:true, executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE || undefined});
  try {
    for (const width of [320,390,701,768,900,1280,1440,1920]) {
      const ctx = await browser.newContext({viewport:{width,height:900},reducedMotion:"reduce",colorScheme:"light",locale:"en-US",timezoneId:"America/Los_Angeles"});
      let mode = "empty", account = "signed_out", payout = "1000.25";
      await ctx.route("**/*", route => {
        const url = new URL(route.request().url());
        if (/^\/auth\/login\/(google|microsoft|github)$/.test(url.pathname)) return route.fulfill({contentType:"text/html",body:"<!doctype html><title>OAuth handoff fixture</title><p>Provider handoff reached.</p>"});
        if (url.pathname.includes("/auth/session") || url.pathname === "/v1/site-auth/session") return route.fulfill({json:{authenticated:account!=="signed_out",account_status:account,account_complete:account==="ready",providers:{google:true,microsoft:true,github:true},user:account!=="signed_out"?{id:"forest-qa",name:"Preview account",email:"preview@example.test"}:null,posting_drafts_enabled:false}});
        if (["/auth/account","/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({json:{authenticated:true,account_status:account,account_complete:account==="ready",user:{id:"forest-qa",name:"Preview account",email:"preview@example.test"},wallets:account==="ready"?[{address:"0x"+"1".repeat(40),label:"Preview wallet",provider_id:"metamask",wallet_type:"browser",chain_ids:[8453],linked_at:"2026-09-10T00:00:00Z",last_verified_at:"2026-09-10T00:00:00Z"}]:[],data_status:"unavailable",reason:"marketplace_evidence_unavailable"}});
        if (url.pathname.endsWith("/leaderboard")) return route.fulfill({status:mode==="offline"?503:200,json:mode==="invalid"?{}:leaderboard(mode==="ready")});
        if (url.pathname === "/v1/metrics/platform") return route.fulfill({json:{marketplace_payout_volume:{lifetime:{usdc:payout},lifetime_settled_rounds:1234},daily:[]}});
        if (url.pathname === "/v1/opportunities") return route.fulfill({json:{schema_version:"agent-bounties/opportunity-projection-v1",network:"base-mainnet",applied_view:"ready_to_earn",degraded:false,source_statuses:[{source_type:"canonical_base",available:true}],items:[]}});
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({body:"window.agentBountiesPhoneWalletConfig={};",contentType:"text/javascript"});
        if (url.origin !== origin) return route.fulfill({status:503,body:"Isolated fixture"});
        return route.continue();
      });
      const page=await ctx.newPage(), errors=[];page.on("pageerror", e=>errors.push(e.message));
      await page.goto(origin);
      await page.waitForLoadState("networkidle");
      await page.evaluate(()=>document.fonts.ready);
      assert.equal(await page.locator("html").getAttribute("data-theme"),"dark","default ignores OS until Auto selected");
      assert.ok(await page.evaluate(()=>document.fonts.check('32px "Cal Sans"') && document.fonts.check('16px Satoshi')));
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth),"home fits");
      await page.locator(".ab-forest-poster").evaluate(image => image.decode());
      assert.equal(await page.locator("[data-forest-scene]").getAttribute("data-media-state"),"artwork");
      assert.equal(await page.locator("[data-forest-video]").getAttribute("src"),null,"reduced motion does not fetch the background video");
      assert.equal(await page.locator("[data-forest-pause]").isVisible(),false,"reduced motion uses the static artwork");
      assert.ok(await page.locator(".ab-forest-poster").evaluate(image=>image.currentSrc.includes(innerWidth<=700?"agent-hall-loop-poster-small-v2.webp":"agent-hall-loop-poster-v2.webp")),"responsive forest artwork");
      assert.deepEqual(await page.locator(".ab-site-nav a").allTextContents(), ["Log in", "Workspace", "Browse work"]);
      assert.equal(await page.locator(".ab-site-menu, [data-site-header] .ab-site-login").count(), 0);
      assert.equal(await page.locator("#hero-title mark").innerText(), "what yours is missing");
      const titleLines = await page.locator(".ab-title-line").evaluateAll(els => els.map(el => ({ height: el.getBoundingClientRect().height, line: parseFloat(getComputedStyle(el).lineHeight), width: el.clientWidth, scroll: el.scrollWidth })));
      assert.equal(titleLines.length, 2);
      for (const line of titleLines) assert.ok(Math.abs(line.height - line.line) < 1 && line.scroll <= line.width + 1, "headline stays on two lines");
      assert.equal((await page.locator("#hero-title").textContent()).trim(), "Other AIs have what yours is missing");
      assert.equal(await page.locator(".ab-ingredient-label").innerText(), "With");
      assert.equal(await page.locator(".ab-hero textarea, .ab-hero input:not([type=hidden])").count(), 0, "hero has text instead of a task input");
      assert.equal(await page.locator("[data-missing-ingredient]").innerText(), "More compute");
      assert.equal(await page.locator(".ab-ingredient-result").innerText(), "they will get your work done.");
      assert.equal(await page.locator("#hero-payment-note").innerText(), "only pay for results, not tokens.");
      const paymentNote = await page.locator("#hero-payment-note").boundingBox(), postButton = await page.locator("#post-a-bounty").boundingBox();
      assert.ok(paymentNote.y >= postButton.y + postButton.height, "payment note sits beneath the posting button");
      assert.equal(await page.locator(".ab-hero-subtitle [data-missing-ingredient]").count(), 1, "animation is part of the subtitle");
      assert.ok(await page.locator(".ab-ingredient").evaluate(el => el.scrollWidth <= el.clientWidth), "ingredient text fits");
      assert.equal(await page.locator(".ab-funding-note").count(), 0);
      assert.match(await page.locator(".ab-faq article").first().innerText(), /rejected result does not pay the solver.*another attempt.*refundable.*cancellation.*withdraw their refund/s);
      if (width >= 1280) assert.ok(await page.locator("#hero-title").evaluate(el => parseFloat(getComputedStyle(el).fontSize) >= 64), "desktop headline is larger");
      const metrics = await page.locator(".ab-metric").evaluateAll(els => els.map(el => { const r = el.getBoundingClientRect(); return { x:r.x, y:r.y, background:getComputedStyle(el).backgroundColor, transform:getComputedStyle(el).transform }; }));
      assert.ok(metrics.every(metric => metric.background === "rgba(0, 0, 0, 0)" && metric.transform === "none"), "metrics are open statistics, without cards");
      if (width <= 700) assert.equal(new Set(metrics.map(metric => metric.x)).size, 1, "metrics stack on mobile");
      else if (width < 1100) { assert.equal(new Set(metrics.map(metric => metric.x)).size, 2, "tablet metrics use two columns"); assert.equal(new Set(metrics.map(metric => metric.y)).size, 2, "tablet metrics use two rows"); }
      else assert.equal(new Set(metrics.map(metric => metric.y)).size, 1, "desktop metrics align horizontally");
      await page.waitForFunction(() => document.querySelector("[data-market-volume]").textContent === "$1,000.25");
      assert.match(await page.locator(".ab-metric", { has: page.locator("[data-market-volume]") }).innerText(), /USD paid out.*USDC payouts at \$1 face value/s);
      assert.equal(await page.locator("[data-completed-bounties]").innerText(), "1,234");
      const metricValues = await page.locator(".ab-metric output, .ab-metric strong").evaluateAll(els => els.map(el => {
        const range = document.createRange(); range.selectNodeContents(el);
        const text = range.getBoundingClientRect(), cell = el.closest(".ab-metric").getBoundingClientRect();
        return { value: el.textContent, fits: text.left >= cell.left && text.right <= cell.right, width: text.width, available: cell.width };
      }));
      assert.ok(metricValues.every(value => value.fits), "populated metric values remain inside their own cell: " + JSON.stringify(metricValues));
      assert.ok(await page.locator(".ab-metrics").evaluate(el => el.scrollWidth <= el.clientWidth), "metrics never need horizontal scrolling");
      for (const [total, shown] of [["1000.25", "$1,000.25"], ["123456.78", "$123,456.78"], ["1000.25", "$1,000.25"]]) {
        payout = total;
        await page.evaluate(() => dispatchEvent(new Event("online")));
        await page.waitForFunction(value => document.querySelector("[data-market-volume]").textContent === value, shown);
        const fitted = await page.locator(".ab-metric output, .ab-metric strong").evaluateAll(els => els.map(el => {
          const range = document.createRange(); range.selectNodeContents(el);
          const text = range.getBoundingClientRect(), cell = el.closest(".ab-metric"), box = cell.getBoundingClientRect(), css = getComputedStyle(cell);
          const left = box.left + parseFloat(css.borderLeftWidth) + parseFloat(css.paddingLeft), right = box.right - parseFloat(css.borderRightWidth) - parseFloat(css.paddingRight);
          return { value: el.textContent, size: getComputedStyle(el).fontSize, inside: text.left >= left - .5 && text.right <= right + .5, centred: Math.abs((text.left + text.right) - (left + right)) <= 2 };
        }));
        assert.ok(fitted.every(value => value.inside && value.centred), `metric values stay centred inside their cell padding at ${width}px: ` + JSON.stringify(fitted));
        const sizes = fitted.map(value => parseFloat(value.size));
        assert.ok(Math.max(...sizes) - Math.min(...sizes) < 1, "metric values share one size: " + JSON.stringify(fitted));
        assert.ok(Math.min(...sizes) >= 24, "metric values stay display-sized: " + JSON.stringify(fitted));
      }
      for(const selector of [".ab-site-signin", ".ab-site-board"]) await fits(page,selector);
      assert.equal(await page.locator(".ab-orbit-ring, .ab-orbit-sphere").count(),0);
      assert.match(await page.locator("#manifesto-title").innerText(),/If you can define.*done.*make it a bounty/s);
      assert.equal(await page.locator(".ab-ticker").count(),1,"one row of use cases");
      await capture(page,`home-${width}`);
      await page.emulateMedia({reducedMotion:"no-preference"});
      await page.waitForFunction(()=>document.querySelector("[data-forest-scene]").dataset.mediaState==="video");
      assert.equal(await page.locator("[data-forest-video]").evaluate(el=>el.videoWidth),width<=700?1920:3840,"responsive video retains high resolution");
      await page.locator("[data-forest-pause]").click();
      await page.evaluate(()=>scrollTo(0,0));
      await capture(page,`home-video-${width}`);
      await page.locator("[data-forest-pause]").click();
      await page.evaluate(() => { account = "ready"; dispatchEvent(new Event("online")); });
      await page.waitForSelector(".ab-site-login");
      await page.getByRole("button", { name: "Sign up with Google" }).click();
      await page.waitForLoadState("networkidle");
      assert.ok(await page.evaluate(() => document.body.innerText.includes("Provider handoff reached.")), "oauth handoff");
      await page.goBack();
      await page.waitForSelector(".ab-site-login");
      mode = "invalid";
      await page.evaluate(() => dispatchEvent(new Event("online")));
      await page.waitForSelector(".ab-prize:not(:visible)");
      assert.ok(await page.locator(".ab-prize").evaluate(el => getComputedStyle(el).display === "none"), "prize hidden");
      assert.ok(await page.locator("[data-leaderboard-table]").evaluate(el => getComputedStyle(el).display === "none"), "invalid table hidden");
    }
  } finally { await browser.close(); await new Promise(r => server.close(r)); }
}
main();