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
      await context.addInitScript(()=>{
        const walletCase=new URL(location.href).searchParams.get("walletCase");
        if (!walletCase) return;
        const embedded={request:async({method})=>method==="eth_requestAccounts"||method==="eth_accounts"?["0x00000000000000000000000000000000000000b2"]:null,on(){},removeListener(){}};
        const stub={
          select:async()=>{ if (walletCase==="reject") throw {code:4001}; return {kind:"embedded",provider:embedded,accounts:["0x00000000000000000000000000000000000000b2"]}; },
          loadEmbedded:async()=>{ throw {reason:"wallet_selector_unavailable"}; },
          hasPending:()=>walletCase==="resume", pendingAddress:()=>"0x00000000000000000000000000000000000000b3",
          beginPending(){}, clearPending(){},
        };
        Object.defineProperty(window,"AgentBountiesWalletLink",{configurable:true,get:()=>stub,set(){}});
      });
      const reads=[], errors=[], deletes=[]; page.on("pageerror",error=>errors.push(error.message));
      // Two extra saved drafts: one never reached a wallet step (deletable), one the server refuses to delete.
      const removable="26000000-0000-4000-8000-000000000002", kept="27000000-0000-4000-8000-000000000003";
      let extraDrafts=[], accountExtra={};
      const draft={schema:"agent-bounties/posting-draft-v1",id:operation,role:"post",goal:"UX TEST — saved raindrop icon",preferences:"",brief:null,draft:null,draft_stale:false,reference_attachment:null};
      const session={authenticated:true,account_status:"wallet_required",account_complete:false,posting_drafts_enabled:true,providers:{google:true},user:{id:"ab".repeat(32),name:"UX Test",email:"",provider:"google"}};
      const saved={id:`draft:${operation}`,operation_id:operation,title:draft.goal,group:"drafts",status:"Saved draft",next_actor:"you",next_action:"Continue reviewing this saved draft.",continuation_url:`${origin}/post.html?operation_id=${operation}#bounty-preview`,updated_at:"2026-09-25T12:00:00Z",payment_state:"unverified"};
      await context.route("**/*",async route=>{
        const req=route.request(), url=new URL(req.url());
        if (["/auth/session","/v1/site-auth/session"].includes(url.pathname)) return route.fulfill({json:session});
        if (["/auth/account","/v1/site-auth/account"].includes(url.pathname)) return route.fulfill({json:{...session,...accountExtra,data_status:"unavailable",reason:"marketplace_identity_unlinked",wallets:[],saved_drafts:{status:"available",items:[saved,
          {...saved,id:"unsafe",continuation_url:"https://outside.example/post.html"},
          {...saved,id:"false-paid",group:"paid",status:"Paid",payment_state:"paid"},...extraDrafts],next_offset:null}}});
        if (url.pathname.endsWith("/wallet/challenge")) return route.fulfill({status:503,json:{error:"unavailable"}});
        const deleting=url.pathname.match(/\/posting-drafts\/([0-9a-f-]{36})\/delete$/);
        if (deleting && req.method()==="POST") {
          deletes.push({id:deleting[1],body:req.postDataJSON()});
          if (deleting[1]===kept) return route.fulfill({status:409,json:{error:"draft_has_wallet_activity"}});
          extraDrafts=extraDrafts.filter(item=>item.operation_id!==deleting[1]);
          return route.fulfill({json:{operation_id:deleting[1],deleted:true}});
        }
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
      if (width===1440) {
        extraDrafts=[removable,kept].map((id,index)=>({id:`draft:${id}`,operation_id:id,title:`Old draft ${index+1}`,group:"drafts",status:"Saved draft",next_actor:"you",next_action:"Continue reviewing this saved draft.",continuation_url:`${origin}/post.html?operation_id=${id}`,revision:3,can_delete:true}));
        await page.goto(`${origin}/?analytics=off#account`);
        const panel=page.locator("[data-account-inbox]");
        await page.getByRole("heading",{name:"Saved drafts (3)"}).waitFor();
        assert.equal(await panel.getByRole("heading",{name:"Continue your work"}).count(),0,"the Continue your work section is gone");
        // A deletable draft has Delete where the "Saved draft" chip was, in the page's button style.
        const deletableRow=panel.locator("li",{has:page.getByRole("link",{name:"Old draft 1"})});
        assert.equal(await deletableRow.locator(".account-inbox-chip").count(),0,"no Saved draft chip beside Delete");
        assert.equal(await deletableRow.locator(".account-inbox-row-head [data-draft-delete]").count(),1,"Delete sits in the row head");
        const look=await deletableRow.locator("[data-draft-delete]").evaluate(el=>{const s=getComputedStyle(el),r=el.getBoundingClientRect(),head=el.parentElement.getBoundingClientRect();return {cls:el.className,radius:s.borderTopLeftRadius,height:r.height,right:Math.round(head.right-r.right)};});
        assert.match(look.cls,/\bab-button\b.*\bab-button--compact\b.*\bab-button--danger\b/);
        assert.equal(look.radius,"999px"); assert.ok(look.height>=36 && look.height<=40,`compact height ${look.height}`); assert.ok(look.right<=1,"Delete is right-aligned in the row head");
        // A draft the API doesn't mark deletable keeps its chip and gets no Delete.
        const plainRow=panel.locator("li",{has:page.getByRole("link",{name:draft.goal})});
        assert.equal(await plainRow.locator(".account-inbox-chip").innerText(),"Saved draft");
        assert.equal(await plainRow.locator("[data-draft-delete]").count(),0);
        // Delete asks for confirmation inline, sends the revision the person saw and removes the row.
        await panel.getByRole("button",{name:"Delete draft “Old draft 1”"}).click();
        assert.equal(deletes.length,0,"the first press only asks for confirmation");
        await panel.getByRole("button",{name:"Keep"}).first().click();
        await panel.getByRole("button",{name:"Delete draft “Old draft 1”"}).click();
        await panel.locator("[data-draft-delete-confirm]:visible").click();
        await panel.getByRole("heading",{name:"Saved drafts (2)"}).waitFor();
        assert.deepEqual(deletes,[{id:removable,body:{expected_revision:3}}]);
        assert.match(await panel.locator(".account-inbox-status").innerText(),/Deleted “Old draft 1”/);
        // A draft the server refuses to delete stays, with the reason.
        await panel.getByRole("button",{name:"Delete draft “Old draft 2”"}).click();
        await panel.locator("[data-draft-delete-confirm]:visible").click();
        await panel.getByText("This draft has a recorded wallet step, so it is kept.",{exact:false}).waitFor();
        assert.equal(await panel.getByRole("link",{name:"Old draft 2"}).count(),1);
        extraDrafts=[];
        await page.goto("about:blank");
        // Sections: Needs action is open when it has items; Linked wallets, Bounty emails and
        // Settings start closed, and each viewer's choice survives a reload on this device.
        extraDrafts=[{id:"needs:1",title:"Approve the delivered logo",group:"needs_action",status:"Review due",next_actor:"you",next_action:"Review the submission.",continuation_url:`${origin}/participate.html?bounty=0x1`}];
        await page.goto(`${origin}/?analytics=off#account`);
        const needs=page.locator('[data-account-inbox] details[data-group="needs_action"]');
        await needs.locator("summary h4",{hasText:"Needs action (1)"}).waitFor();
        assert.equal(await needs.evaluate(el=>el.open),true,"Needs action opens when it has items");
        const sectionsOpen=()=>page.locator("[data-account-section]").evaluateAll(els=>Object.fromEntries(els.map(el=>[el.dataset.accountSection,el.open])));
        assert.deepEqual(await sectionsOpen(),{wallets:false,emails:false,settings:false});
        assert.equal(await page.locator("[data-auth-logout]").isVisible(),false,"Sign out sits in the closed Settings");
        await page.locator('[data-account-section="wallets"] > summary').click();
        await page.locator('[data-account-section="settings"] > summary').focus();
        await page.keyboard.press("Enter");
        assert.equal(await page.locator("[data-auth-logout]").isVisible(),true,"the keyboard opens Settings");
        await page.waitForFunction(()=>JSON.parse(localStorage.getItem("agent-bounties.account-sections.v1")||"{}").settings===true);
        await page.reload();
        await needs.waitFor();
        await page.waitForFunction(()=>document.querySelector('[data-account-section="wallets"]')?.open===true);
        assert.deepEqual(await sectionsOpen(),{wallets:true,emails:false,settings:true},"remembered after reload");
        // Linking a wallet from elsewhere (Bounty emails' "Link my verifier wallet" clicks the same
        // button) opens Linked wallets, so its progress, errors and recovery are visible.
        await page.evaluate(()=>{document.querySelector('[data-account-section="wallets"]').open=false;document.querySelector("[data-wallet-link]").click();});
        await page.waitForFunction(()=>document.querySelector('[data-account-section="wallets"]').open===true);
        await page.keyboard.press("Escape");
        await page.evaluate(()=>localStorage.removeItem("agent-bounties.account-sections.v1"));
        const walletOutcome=async(walletCase,start)=>{
          await page.goto(`${origin}/?analytics=off&walletCase=${walletCase}#account`);
          await needs.waitFor();
          if (start) await page.evaluate(()=>{const s=document.querySelector('[data-account-section="wallets"]');s.open=false;document.querySelector("[data-wallet-link]").click();});
          await page.waitForFunction(()=>document.querySelector("[data-wallet-status]")?.textContent.trim().length>0 && !/Restoring|Choose|Preparing/.test(document.querySelector("[data-wallet-status]").textContent));
          return page.evaluate(()=>{const section=document.querySelector('[data-account-section="wallets"]'),status=document.querySelector("[data-wallet-status]"),finish=document.querySelector("[data-wallet-finish]");
            return {open:section.open,status:status.textContent.trim(),statusVisible:status.checkVisibility(),finishVisible:Boolean(finish?.checkVisibility())};});
        };
        // (1) Bounty emails' "Link my verifier wallet" with the wallet choice rejected: the error is visible.
        const rejected=await walletOutcome("reject",true);
        assert.ok(rejected.open&&rejected.statusVisible,JSON.stringify(rejected)); assert.match(rejected.status,/cancelled/);
        // (2) An embedded wallet is ready but its ownership challenge fails: the error and Finish linking are visible.
        const unfinished=await walletOutcome("challenge",true);
        assert.ok(unfinished.open&&unfinished.statusVisible&&unfinished.finishVisible,JSON.stringify(unfinished)); assert.match(unfinished.status,/Finish linking/);
        // (3) A pending wallet restoration after sign-in fails: the error is visible without anyone opening the section.
        const resumed=await walletOutcome("resume",false);
        assert.ok(resumed.open&&resumed.statusVisible,JSON.stringify(resumed));
        await page.keyboard.press("Escape");
        // An account that can't be confirmed shows the setup view, with every section open.
        accountExtra={account_status:"unavailable",account_complete:false};
        await page.reload();
        await page.waitForFunction(()=>document.querySelector("[data-auth-dialog]")?.dataset.view==="setup");
        assert.deepEqual(await sectionsOpen(),{wallets:true,emails:true,settings:true},"nothing is hidden outside the account view");
        accountExtra={};
        await page.evaluate(()=>localStorage.removeItem("agent-bounties.account-sections.v1"));
        extraDrafts=[];
        await page.goto("about:blank");
      }
      await page.goto(`${origin}/?analytics=off#account`);
      const inbox=page.locator("[data-account-inbox]");
      await page.getByRole("heading",{name:"Saved drafts (1)"}).waitFor();
      assert.equal(await inbox.isVisible(),true);
      assert.equal(await page.locator("[data-account-stats]").isVisible(),false);
      // The previous API's wallet-less session is a ready posting account, not unfinished setup.
      assert.deepEqual(await page.locator("[data-auth-dialog]").evaluate(el=>[el.dataset.accountStatus,el.dataset.solverStatus]),["ready","wallet_required"]);
      await page.waitForFunction(()=>Boolean(window.AgentBountiesWebMCP));
      const agent=await page.evaluate(()=>window.AgentBountiesWebMCP.call("agent_bounties_get_account_activity"));
      assert.equal(agent.items.length,1); assert.equal(agent.items[0].operation_id,operation);
      assert.equal(agent.items[0].continuation_url,await inbox.getByRole("link",{name:draft.goal}).getAttribute("href"));
      assert.equal(agent.filtered_item_count,2); assert.equal(agent.activity_status,"unavailable");
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
      process.stdout.write(`Account inbox: ${width}px — human and browser-agent continuation agree; exact draft resumes and survives reload\n`);
    }
  } finally { await browser.close(); await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1;server.close();});
