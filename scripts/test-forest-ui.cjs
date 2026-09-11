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
  const data = period => ({ reward_usdc: period === "daily" ? "3" : "26", reward_funding_status: "partially_funded", reward_payout_status: "not_paid", ranking: { period: { kind: period, starts_at: "2026-09-10T00:00:00Z", ends_at: "2026-09-17T00:00:00Z" }, rules: ["Rank by qualifying confirmed completions."], entries: populated ? [3,1,2].map(rank => ({ rank, solver_wallet: `0x${String(rank).repeat(40)}`, prize_eligible_bounties: 10 - rank, eligible_solver_rewards_usdc_base_units: `${(10-rank)*1000000}` })) : [] } });
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
    for (const width of [390,768,1280,1440,1920]) {
      const ctx = await browser.newContext({viewport:{width,height:900},reducedMotion:"reduce",colorScheme:"light"});
      let mode = "empty", account = "signed_out";
      await ctx.route("**/*", route => {
        const url = new URL(route.request().url());
        if (/^\/auth\/login\/(google|microsoft|github)$/.test(url.pathname)) return route.fulfill({contentType:"text/html",body:"<!doctype html><title>OAuth handoff fixture</title><p>Provider handoff reached.</p>"});
        if (url.pathname.includes("/auth/session") || url.pathname === "/v1/site-auth/session") return route.fulfill({json:{authenticated:account!=="signed_out",account_status:account,account_complete:account==="ready",providers:{google:true,microsoft:true,github:true},user:account!=="signed_out"?{id:"forest-qa",name:"Preview account",email:"preview@example.test"}:null,posting_drafts_enabled:false}});
        if (["/auth/account","/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({json:{authenticated:true,account_status:account,account_complete:account==="ready",user:{id:"forest-qa",name:"Preview account",email:"preview@example.test"},wallets:account==="ready"?[{address:"0x"+"1".repeat(40),label:"Preview wallet",provider_id:"metamask",wallet_type:"browser",chain_ids:[8453],linked_at:"2026-09-10T00:00:00Z",last_verified_at:"2026-09-10T00:00:00Z"}]:[],data_status:"unavailable",reason:"marketplace_evidence_unavailable"}});
        if (url.pathname.endsWith("/leaderboard")) return route.fulfill({status:mode==="offline"?503:200,json:mode==="invalid"?{}:leaderboard(mode==="ready")});
        if (url.pathname === "/phone-wallet-config.js") return route.fulfill({body:"window.agentBountiesPhoneWalletConfig={};",contentType:"text/javascript"});
        if (url.origin !== origin) return route.fulfill({status:503,body:"Isolated fixture"});
        return route.continue();
      });
      const page=await ctx.newPage(), errors=[];page.on("pageerror", e=>errors.push(e.message));
      await page.goto(origin);
      await page.evaluate(()=>document.fonts.ready);
      assert.equal(await page.locator("html").getAttribute("data-theme"),"dark","default ignores OS until Auto selected");
      assert.ok(await page.evaluate(()=>document.fonts.check('32px "Cal Sans"') && document.fonts.check('16px Satoshi')));
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth),"home fits");
      await page.locator(".ab-forest-poster").evaluate(image => image.decode());
      assert.equal(await page.locator("[data-forest-scene]").getAttribute("data-media-state"),"artwork");
      assert.equal(await page.locator("[data-forest-video]").getAttribute("src"),null,"reduced motion does not fetch the background video");
      assert.equal(await page.locator("[data-forest-pause]").isVisible(),false,"reduced motion uses the static artwork");
      assert.ok(await page.locator(".ab-forest-poster").evaluate(image=>image.currentSrc.includes(innerWidth<=700?"agent-hall-loop-poster-small-v2.webp":"agent-hall-loop-poster-v2.webp")),"responsive forest artwork");
      assert.deepEqual(await page.locator(".ab-site-nav a").allTextContents(), ["How it works", "Browse work"]);
      assert.equal(await page.locator(".ab-site-menu, [data-site-header] .ab-site-login").count(), 0);
      assert.equal(await page.locator("#hero-title mark").innerText(), "get your work done");
      const titleLines = await page.locator(".ab-title-line").evaluateAll(els => els.map(el => ({ height: el.getBoundingClientRect().height, line: parseFloat(getComputedStyle(el).lineHeight), width: el.clientWidth, scroll: el.scrollWidth })));
      assert.equal(titleLines.length, 3);
      for (const line of titleLines) assert.ok(Math.abs(line.height - line.line) < 1 && line.scroll <= line.width + 1, "headline stays on three lines");
      assert.equal(await page.locator(".ab-usecases-label").innerText(), "USE CASES");
      assert.equal(await page.locator(".ab-usecases-label").evaluate(el => getComputedStyle(el).color), "rgb(255, 255, 255)");
      assert.equal(await page.locator(".ab-payoff").evaluate(el => getComputedStyle(el).textDecorationLine), "underline");
      assert.equal(await page.locator(".ab-only").evaluate(el => getComputedStyle(el).backgroundColor), "rgba(0, 0, 0, 0)");
      assert.equal(await page.locator(".ab-only").evaluate(el => getComputedStyle(el).transform), "none");
      assert.equal(await page.locator("[data-outcome-word]").evaluate(el => getComputedStyle(el).backgroundColor), "rgb(199, 245, 66)");
      if (width >= 1280) assert.ok(await page.locator("#hero-title").evaluate(el => parseFloat(getComputedStyle(el).fontSize) >= 64), "desktop headline is larger");
      const metrics = await page.locator(".ab-metric").evaluateAll(els => els.map(el => { const r = el.getBoundingClientRect(); return { x:r.x, y:r.y, background:getComputedStyle(el).backgroundColor, transform:getComputedStyle(el).transform }; }));
      assert.ok(metrics.every(metric => metric.background === "rgba(0, 0, 0, 0)" && metric.transform === "none"), "metrics are open statistics, without cards");
      assert.equal(new Set(metrics.map(metric => width <= 700 ? metric.x : metric.y)).size, 1, "metrics stack on mobile and align horizontally on desktop");
      assert.ok(await page.locator(".ab-metrics").evaluate(el => el.scrollWidth <= el.clientWidth), "metrics never need horizontal scrolling");
      for(const selector of [".ab-how-it-works", ".ab-site-board"]) await fits(page,selector);
      assert.equal(await page.locator(".ab-orbit-ring, .ab-orbit-sphere").count(),0);
      assert.match(await page.locator("#manifesto-title").innerText(),/If you could define.*done.*for the task,/s);
      assert.equal(await page.locator(".ab-ticker").count(),1,"one row of use cases");
      await capture(page,`home-${width}`);
      await page.emulateMedia({reducedMotion:"no-preference"});
      await page.waitForFunction(()=>document.querySelector("[data-forest-scene]").dataset.mediaState==="video");
      assert.equal(await page.locator("[data-forest-video]").evaluate(el=>el.videoWidth),width<=700?1920:3840,"responsive video retains high resolution");
      await page.locator("[data-forest-pause]").click();
      await page.evaluate(()=>scrollTo(0,0));
      await capture(page,`home-video-${width}`);
      await page.locator("[data-forest-pause]").click();
      await page.emulateMedia({reducedMotion:"reduce"});
      await page.locator("[data-deck-next]").click();assert.equal(await page.locator("[data-deck-status]").innerText(),"Example 2 of 3");
      await page.locator("[data-example-deck]").focus();await page.keyboard.press("ArrowLeft");assert.equal(await page.locator("[data-deck-status]").innerText(),"Example 1 of 3");
      await page.locator('button[data-theme-choice="light"]').click();
      assert.equal(await page.locator("html").getAttribute("data-theme"),"light");
      await capture(page,`home-light-${width}`);
      await page.reload();assert.equal(await page.locator("html").getAttribute("data-theme"),"light");
      await page.locator('button[data-theme-choice="auto"]').click();await page.emulateMedia({colorScheme:"dark"});
      await page.waitForFunction(()=>document.documentElement.dataset.theme==="dark");
      assert.equal(await page.locator('button[data-theme-choice="auto"]').getAttribute("aria-pressed"),"true");
      await page.locator('button[data-theme-choice="dark"]').click();
      const task="Prepare a source-backed competitor report.";
      await page.locator("#home-task").fill(task);
      await page.locator("#post-a-bounty").click();
      await page.locator("[data-bounty-launcher][open]").waitFor();
      assert.ok((await page.locator("[data-bounty-prompt]").textContent()).includes(await page.locator("#home-task").inputValue()), "AI picker keeps the typed task");
      assert.equal(new URL(page.url()).pathname, "/", "posting CTA opens the picker in place");
      await capture(page,`ai-picker-${width}`);
      await page.locator("[data-bounty-close]").click();
      await page.goto(origin+"/post.html");
      assert.equal(await page.locator("#bounty-composer-input").inputValue(),task,"first task survives synchronous journey creation");
      await page.reload();assert.equal(await page.locator("#bounty-composer-input").inputValue(),task,"saved after reload");
      assert.equal(await page.locator('[data-stage-target="fund"]').isDisabled(),true);
      await page.goto(origin);await page.locator("#home-task").fill("A different idea");await page.locator("#post-a-bounty").click();
      await page.locator("[data-bounty-launcher][open]").waitFor();
      assert.ok((await page.locator("[data-bounty-prompt]").textContent()).includes(await page.locator("#home-task").inputValue()), "AI picker keeps the typed task");
      assert.equal(new URL(page.url()).pathname, "/", "posting CTA opens the picker in place");
      await page.locator("[data-bounty-close]").click();
      await page.goto(origin+"/post.html");
      await page.getByRole("button",{name:"Keep saved brief",exact:true}).click();assert.equal(await page.locator("#bounty-composer-input").inputValue(),task);
      await page.goto(origin+"/post.html?task=New%20review%20request");await page.getByRole("button",{name:"Use this new idea",exact:true}).click();assert.equal(await page.locator("#bounty-composer-input").inputValue(),"New review request");
      await page.evaluate(()=>scrollTo(0,0));await capture(page,`post-${width}`);
      await page.goto(origin+"/#login");await page.locator("[data-auth-dialog][open]").waitFor();
      assert.equal(await page.locator("#auth-title").innerText(),"Create an account");
      assert.deepEqual(await page.locator("[data-auth-provider]").evaluateAll(els=>els.map(el=>el.dataset.authProvider)),["google","microsoft","github"]);
      assert.equal(await page.locator("[data-auth-form] input, [data-auth-unavailable]").count(),0,"no custom account credentials or dead recovery controls");
      await capture(page,`register-${width}`);
      await page.locator("[data-auth-mode-toggle]").click();assert.equal(await page.locator("#auth-title").innerText(),"Sign in");
      assert.equal(await page.getByRole("button",{name:"Sign in with Google",exact:true}).count(),1);
      await capture(page,`signin-${width}`);
      await page.locator("[data-auth-close]").click();
      await page.locator(".ab-site-login").click();
      assert.equal(await page.locator("#auth-title").innerText(),"Create an account","reopening defaults to account creation");
      await page.locator("[data-auth-close]").click();
      if(width===390) {
        for(const provider of ["google","microsoft","github"]) {
          await page.goto(origin+"/#login");
          await page.waitForFunction(key=>document.querySelector(`[data-auth-provider="${key}"]`).getAttribute("aria-disabled")==="false",provider);
          await page.locator(`[data-auth-provider="${provider}"]`).click();
          await page.waitForURL(`**/auth/login/${provider}`);
        }
      }
      await page.goto(origin+"/leaderboard.html");await page.getByText("No qualifying completions yet this period.",{exact:false}).waitFor();
      assert.equal(await page.locator("[data-leaderboard-table]").isVisible(),false);assert.match(await page.locator("[data-prize-title]").innerText(),/26 USDC weekly/);
      await capture(page,`leaderboard-empty-${width}`);
      await page.locator('[data-leaderboard-period="daily"]').click();assert.match(await page.locator("[data-prize-title]").innerText(),/3 USDC daily/);
      mode="ready";await page.locator("[data-leaderboard-refresh]").click();await page.locator("[data-leaderboard-table]").waitFor();
      assert.deepEqual(await page.locator(".ab-podium-place").evaluateAll(els=>els.map(el=>el.dataset.rank)),["2","1","3"]);
      assert.deepEqual(await page.locator("tbody tr td:first-child").allTextContents(),["#1","#2","#3"]);
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth <= innerWidth),"leaderboard fits");
      await capture(page,`leaderboard-populated-${width}`);
      mode="offline";await page.locator("[data-leaderboard-refresh]").click();await page.getByText("Rankings are temporarily unavailable.",{exact:false}).waitFor();assert.equal(await page.locator(".ab-prize").isVisible(),false);
      mode="invalid";await page.locator("[data-leaderboard-refresh]").click();await page.waitForFunction(()=>!document.querySelector("[data-leaderboard-refresh]").disabled);assert.equal(await page.locator("[data-leaderboard-table]").isVisible(),false);
      for(const state of ["wallet_required","ready"]) {
        account=state;await page.goto(origin+"/#account");await page.locator("[data-auth-dialog][open]").waitFor();
        await page.waitForFunction(expected=>document.querySelector("[data-auth-dialog]").dataset.accountStatus===expected,state);
        await capture(page,`account-${state}-${width}`);
        await page.locator("[data-auth-close]").click();
        await page.locator('button[data-theme-choice="light"]').click();
        await page.locator(".ab-site-login").click();await capture(page,`account-${state}-light-${width}`);
        await page.locator("[data-auth-close]").click();
        await page.locator('button[data-theme-choice="dark"]').click();
      }
      assert.deepEqual(errors,[]);await ctx.close();console.log(`PASS forest UI, themes, draft handoff, auth, leaderboard evidence at ${width}px`);
    }
    const motion = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: "no-preference" });
    await motion.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({ status: 503, body: "Isolated motion check" }));
    const animated = await motion.newPage(); await animated.goto(origin);
    const word = animated.locator("[data-outcome-word]");
    await animated.locator("#home-task").fill("Keep my own task while the headline changes");
    for (const expected of ["Cheaper", "Better", "Faster"]) await animated.waitForFunction(value => document.querySelector("[data-outcome-word]").textContent === value, expected);
    assert.equal(await animated.locator("#home-task").inputValue(), "Keep my own task while the headline changes");
    await animated.emulateMedia({reducedMotion:"reduce"});
    const stoppedWord = await word.innerText();
    await animated.waitForTimeout(3000);
    assert.equal(await word.innerText(), stoppedWord, "reduced motion stops the word loop");
    await animated.emulateMedia({reducedMotion:"no-preference"});
    await animated.waitForFunction(()=>document.querySelector("[data-forest-scene]").dataset.forestMotion==="running");
    await animated.waitForFunction(()=>document.querySelector("[data-forest-scene]").dataset.mediaState==="video");
    const video = animated.locator("[data-forest-video]");
    const media = await video.evaluate(el=>({duration:el.duration,width:el.videoWidth,height:el.videoHeight,muted:el.muted,loop:el.loop}));
    assert.ok(Math.abs(media.duration-6)<.1,"delivered clip is six seconds");
    assert.equal(media.width,3840,"desktop delivers the 4K clip");
    assert.equal(media.height,2160);
    assert.ok(media.muted && media.loop,"background plays silently and loops");
    const frameTime = await video.evaluate(el=>el.currentTime);
    await animated.waitForFunction(before=>Math.abs(document.querySelector("[data-forest-video]").currentTime-before)>.1,frameTime);
    await video.evaluate(el=>{el.currentTime=el.duration-.25;});
    await animated.waitForFunction(()=>document.querySelector("[data-forest-video]").currentTime<1);
    assert.equal(await video.evaluate(el=>el.paused),false,"the actual media repeats across its loop boundary");
    // Pause is persistent, leaves the task intact, and never triggers wallet/account work.
    await animated.locator("#home-task").fill("Keep this draft while pausing the scene");
    await animated.locator("[data-forest-pause]").click();
    assert.equal(await animated.locator("[data-forest-pause]").getAttribute("aria-pressed"),"true");
    assert.equal(await video.evaluate(el=>el.paused),true);
    await animated.reload();
    assert.equal(await animated.locator("[data-forest-pause]").innerText(),"Play atmosphere");
    assert.equal(await animated.locator("#home-task").inputValue(),"Keep this draft while pausing the scene");
    assert.equal(await video.getAttribute("src"),null,"saved pause avoids loading video after reload");
    await animated.locator("[data-forest-pause]").click();
    assert.equal(await animated.locator("[data-forest-scene]").getAttribute("data-forest-motion"),"running");
    await animated.waitForFunction(()=>!document.querySelector("[data-forest-video]").paused);
    await animated.locator(".ab-tickers").scrollIntoViewIfNeeded();
    await animated.waitForFunction(()=>document.querySelector("[data-forest-video]").paused);
    await animated.mouse.move(1, 1);
    const row = animated.locator(".ab-ticker");
    assert.equal(await row.evaluate(el=>getComputedStyle(el).animationDuration),"360s");
    const initial = await row.evaluate(el => getComputedStyle(el).transform);
    await animated.waitForFunction(before => getComputedStyle(document.querySelector(".ab-ticker")).transform !== before, initial);
    const ticker = await row.boundingBox();
    await animated.mouse.move(640, ticker.y + ticker.height / 2);
    await animated.waitForFunction(() => getComputedStyle(document.querySelector(".ab-ticker")).animationPlayState === "paused");
    await animated.emulateMedia({ reducedMotion: "reduce" });
    assert.equal(await row.evaluate(el => getComputedStyle(el).animationName), "none");
    assert.equal(await animated.locator("[data-forest-pause]").isVisible(),false);
    assert.equal(await animated.locator("[data-forest-scene]").getAttribute("data-forest-motion"),"paused");
    assert.equal(await video.isVisible(),false,"live reduced-motion switch returns to poster");
    await animated.waitForFunction(() => !document.querySelector('[data-enter="waiting"]'));
    await motion.close();
    const broken = await browser.newContext({ viewport: { width: 390, height: 900 }, reducedMotion: "no-preference" });
    await broken.route("**/*", route => {
      const url=new URL(route.request().url());
      return url.pathname.endsWith(".mp4") || url.origin!==origin ? route.fulfill({status:503,body:"Isolated media outage"}) : route.continue();
    });
    const fallback=await broken.newPage();await fallback.goto(origin);
    await fallback.waitForFunction(()=>document.querySelector("[data-forest-video]").error);
    assert.equal(await fallback.locator("[data-forest-scene]").getAttribute("data-media-state"),"artwork");
    assert.equal(await fallback.locator("[data-forest-video]").isVisible(),false);
    await fallback.locator("#home-task").fill("My task survives a failed video download");
    await fallback.locator("#post-a-bounty").click();await fallback.locator("[data-bounty-launcher][open]").waitFor();
    assert.match(await fallback.locator("[data-bounty-prompt]").textContent(),/My task survives a failed video download/);
    await fallback.goto(origin+"/post.html");
    assert.equal(await fallback.locator("#bounty-composer-input").inputValue(),"My task survives a failed video download");
    await broken.close();
    const blocked = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: "no-preference" });
    await blocked.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.fulfill({status:503,body:"Isolated autoplay check"}));
    await blocked.addInitScript(()=>{
      const play=HTMLMediaElement.prototype.play;
      let refused=false;
      HTMLMediaElement.prototype.play=function(...args) {
        if(this.matches("[data-forest-video]")&&!refused) {refused=true;return Promise.reject(new DOMException("Autoplay requires a gesture","NotAllowedError"));}
        return play.apply(this,args);
      };
    });
    const manual=await blocked.newPage();await manual.goto(origin);
    await manual.getByRole("button",{name:"Play atmosphere",exact:true}).waitFor();
    assert.equal(await manual.locator("[data-forest-video]").isVisible(),false);
    await manual.getByRole("button",{name:"Play atmosphere",exact:true}).click();
    await manual.waitForFunction(()=>document.querySelector("[data-forest-scene]").dataset.mediaState==="video");
    assert.equal(await manual.locator("[data-forest-video]").evaluate(el=>el.paused),false,"a manual gesture recovers from autoplay refusal");
    await blocked.close();
    console.log("PASS six-second video playback, persistent pause, offscreen pause, media error fallback, manual autoplay recovery, pointer pause, and live reduced-motion changes");
  } finally {await browser.close();}
}
main().catch(e=>{console.error(e);process.exitCode=1;}).finally(()=>server.close());
