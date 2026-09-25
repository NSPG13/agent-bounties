"use strict";
// Browser continuation test with synthetic records and no remote requests.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const { createHash } = require("node:crypto");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const root = path.resolve(__dirname, "..");
const mime = { ".html":"text/html", ".js":"text/javascript", ".css":"text/css", ".svg":"image/svg+xml", ".webp":"image/webp" };
const server = http.createServer((req,res) => {
  const file = path.resolve(root,"site", "." + (new URL(req.url,"http://localhost").pathname === "/" ? "/index.html" : new URL(req.url,"http://localhost").pathname));
  if (!file.startsWith(path.join(root,"site")+path.sep)) return res.writeHead(403).end();
  try { const bytes=fs.readFileSync(file); res.writeHead(200,{"content-type":mime[path.extname(file)]||"application/octet-stream"}).end(bytes); } catch { res.writeHead(404).end(); }
});
function sorted(value) { return value && typeof value === "object" ? Array.isArray(value) ? value.map(sorted) : Object.fromEntries(Object.keys(value).sort().map(key=>[key,sorted(value[key])])) : value; }
(async () => {
  await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
  const origin=`http://127.0.0.1:${server.address().port}`, operation="25000000-0000-4000-8000-000000000001";
  const browser=await chromium.launch({headless:true});
  try {
    for (const width of [1440,390,320]) {
      const context=await browser.newContext({viewport:{width,height:900}}), page=await context.newPage();
      const reads=[], errors=[]; page.on("pageerror",error=>errors.push(error.message));
      const draft={schema:"agent-bounties/posting-draft-v1",id:operation,role:"post",goal:"UX TEST — saved raindrop icon",preferences:"",brief:null,draft:null,draft_stale:false,reference_attachment:null};
      const session={authenticated:true,account_status:"wallet_required",account_complete:false,posting_drafts_enabled:true,providers:{google:true},user:{id:"ab".repeat(32),name:"UX Test",email:"",provider:"google"}};
      const saved={id:`draft:${operation}`,operation_id:operation,title:draft.goal,group:"drafts",status:"Saved draft",next_actor:"you",next_action:"Continue reviewing this saved draft.",continuation_url:`${origin}/post.html?operation_id=${operation}#bounty-preview`,updated_at:"2026-09-25T12:00:00Z",payment_state:"unverified"};
      await context.route("**/*",async route=>{
        const req=route.request(), url=new URL(req.url());
        if (["/auth/session","/v1/site-auth/session"].includes(url.pathname)) return route.fulfill({json:session});
        if (["/auth/account","/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({json:{...session,data_status:"unavailable",reason:"marketplace_identity_unlinked",wallets:[],saved_drafts:{status:"available",items:[saved],next_offset:null}}});
        if (url.pathname.endsWith(`/posting-drafts/${operation}`)) {
          reads.push(req.method());
          const envelope=req.method()==="POST" ? req.postDataJSON().draft : draft;
          return route.fulfill({json:{operation_id:operation,draft:envelope,recovery_state:{},draft_hash:createHash("sha256").update(JSON.stringify(sorted(envelope))).digest("hex"),approved_draft_hash:null,revision:1,updated_at:"2026-09-25T12:00:00Z"}});
        }
        if (url.pathname.endsWith("/review-notifications")) return route.fulfill({json:{enabled:false,email_verified:false,wallet_linked:false,status:"disabled"}});
        if (url.pathname==="/v1/opportunities") return route.fulfill({json:{items:[],generated_at:"2026-09-25T12:00:00Z"}});
        if (url.origin!==origin) return route.abort();
        return route.continue();
      });
      await page.goto(`${origin}/?analytics=off#account`);
      const inbox=page.locator("[data-account-inbox]");
      await page.getByRole("heading",{name:"Saved drafts (1)"}).waitFor();
      assert.equal(await inbox.isVisible(),true);
      assert.equal(await page.locator("[data-account-stats]").isVisible(),false);
      assert.equal(await page.locator(".auth-dialog").evaluate(el=>el.scrollWidth<=el.clientWidth+1),true,`${width}: dialog overflows`);
      if (process.env.ACCOUNT_INBOX_EVIDENCE_DIR) {
        fs.mkdirSync(process.env.ACCOUNT_INBOX_EVIDENCE_DIR,{recursive:true});
        for (const theme of ["dark","light"]) {
          await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;},theme);
          await page.screenshot({path:path.join(process.env.ACCOUNT_INBOX_EVIDENCE_DIR,`account-inbox-${width}-${theme}.png`)});
          assert.equal(await page.locator(".auth-dialog").evaluate(el=>el.scrollWidth<=el.clientWidth+1),true);
          assert.equal(await inbox.getByRole("link",{name:draft.goal}).evaluate(el=>getComputedStyle(el).color),await page.locator("#linked-wallets-title").evaluate(el=>getComputedStyle(el).color));
        }
      }
      await inbox.getByRole("link",{name:draft.goal}).click();
      await page.waitForURL(url=>url.searchParams.get("operation_id")===operation);
      await page.waitForFunction(id=>window.AgentBountiesWorkflow?.createClient(window).load()?.id===id,operation);
      assert.ok(reads.includes("GET"),"restores the exact private record");
      assert.equal(await page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load().goal),draft.goal);
      await page.reload();
      await page.waitForFunction(id=>window.AgentBountiesWorkflow?.createClient(window).load()?.id===id,operation);
      assert.deepEqual(errors,[]);
      await context.close();
      process.stdout.write(`Account inbox: ${width}px — visible before wallet setup; exact draft resumes and survives reload\n`);
    }
  } finally { await browser.close(); await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1;server.close();});
