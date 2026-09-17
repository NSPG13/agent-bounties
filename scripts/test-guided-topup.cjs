"use strict";
// Real browser with deterministic provider/API fixtures. No live payment traffic.
const assert=require("node:assert/strict"),fs=require("node:fs"),path=require("node:path"),http=require("node:http");
const {chromium}=require("../tools/browser-layout/node_modules/playwright");
const root=path.resolve(__dirname,".."),operation="8cd9dbc4-242a-4f99-8fce-03961773387a",wallet="0x1111111111111111111111111111111111111111";
const server=http.createServer((req,res)=>{const name=decodeURIComponent(new URL(req.url,"http://local").pathname);const file=path.resolve(root,"site",`.${name}`);if(!file.startsWith(path.join(root,"site")+path.sep)){res.writeHead(403).end();return;}try{res.setHeader("content-type",({".js":"application/javascript",".css":"text/css",".html":"text/html",".json":"application/json",".svg":"image/svg+xml"})[path.extname(file)]||"application/octet-stream");res.end(fs.readFileSync(file));}catch{res.writeHead(404).end();}});
(async()=>{await new Promise(r=>server.listen(0,"127.0.0.1",r));const base="https://agentbounties.app",browser=await chromium.launch({headless:true});let cases=0;
try{
 for(const width of [390,532,1280]){
  const context=await browser.newContext({viewport:{width,height:740}}),page=await context.newPage();const errors=[];page.on("pageerror",e=>errors.push(e.message));
  let attempt=null,shortfall="2010000",opened=0,prepared=0,providerError=null,flag=true,unconfigured=false;
  await context.route("**/*",async route=>{
   const url=new URL(route.request().url());if(url.origin===base){const file=path.resolve(root,"site",`.${url.pathname}`);if(!file.startsWith(path.join(root,"site")+path.sep))return route.abort();try{return route.fulfill({contentType:({".js":"application/javascript",".css":"text/css",".html":"text/html",".json":"application/json",".svg":"image/svg+xml"})[path.extname(file)]||"application/octet-stream",body:fs.readFileSync(file)});}catch{return route.fulfill({status:404,body:""});}}
   if(url.hostname!=="api.agentbounties.app")return route.abort();
   const p=url.pathname,body=route.request().postDataJSON(),json=value=>route.fulfill({status:200,contentType:"application/json",body:JSON.stringify(value),headers:{"access-control-allow-origin":base,"access-control-allow-credentials":"true"}});
   if(route.request().method()==="OPTIONS")return route.fulfill({status:204,headers:{"access-control-allow-origin":base,"access-control-allow-credentials":"true","access-control-allow-methods":"GET,POST","access-control-allow-headers":"content-type"}});
   if(p==="/v1/wallet-funding/capabilities")return json({guided_topup:flag});
   if(p==="/v1/site-auth/account")return json({wallets:[{address:wallet}]});
   if(p.endsWith("/readiness"))return json({operation_id:operation,wallet,usdc_shortfall_units:shortfall,required_usdc_units:"2010000",gas_sponsorship:{status:"awaiting_authorization",message:"The relay can pay gas after checking your exact authorization. No ETH purchase is needed."}});
   if(p.endsWith("/topup-options")&&unconfigured)return json({enabled:true,recommended:"handoff",providers:[{id:"coinbase",available:false,blocker:"provider_not_configured"},{id:"moonpay",available:false,blocker:"sandbox_does_not_fund_base"}]});
   if(p.endsWith("/topup-options")&&body.country==="ZZ")return json({enabled:true,recommended:"handoff",providers:[]});
   if(p.endsWith("/topup-options"))return json({enabled:true,recommended:"coinbase",providers:[{id:"coinbase",available:true,requirements:"Coinbase sign-in may be required.",options:{payment_currencies:[{id:"USD"}],payment_methods:[{id:"CARD"}]}},{id:"moonpay",available:true},{id:"metamask",available:false}]});
   if(p.endsWith("/topups/action")){assert.equal(body.attempt_id,attempt.id);if(body.action==="open"){opened++;attempt.status="pending";}else attempt.status="cancelled";return json({attempt});}
   if(p.endsWith("/topups")){if(route.request().method()==="POST"){if(providerError)return json({status:"unavailable",code:providerError,next_action:"Choose another available provider for your confirmed country and payment method."});prepared++;assert.equal(body.wallet,wallet);attempt={id:"10000000-0000-4000-8000-000000000001",operation_id:operation,wallet,provider:body.provider,status:"prepared",expires_at:new Date(Date.now()+300000).toISOString(),quote:{payment_total:"6.20",payment_currency:"USD",received_usdc:"5.00",excess_usdc:"2.99",fees:[{type:"provider",amount:"1.20"}]}};return json({status:"quote_ready",attempt,checkout_url:"https://pay.coinbase.com/buy?sessionToken=fixture"});}return json({attempt});}
   return json({});
  });
  await page.addInitScript(()=>{const tools=new Map();Object.defineProperty(document,"modelContext",{value:{registerTool:tool=>tools.set(tool.name,tool)}});window.testTopupTools=tools;});
  await page.goto(`${base}/onramp.html?operation_id=${operation}&wallet=${wallet}&amount=2.01&analytics=off`);
  await page.locator("[data-guided-country]").waitFor({state:"visible"});
  assert.equal(await page.locator("[data-legacy-topup]").isVisible(),false);
  const box=await page.locator("[data-guided-next]").boundingBox();assert.ok(box.y+box.height<740,`primary action below first viewport at ${width}`);
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
  fs.mkdirSync(path.join(root,"evidence/guided-wallet-funding"),{recursive:true});
  await page.screenshot({path:path.join(root,`evidence/guided-wallet-funding/wallet-${width}.png`),fullPage:false});
  await page.locator("[data-guided-country]").fill("MX");await page.locator("[data-guided-next]").click();
  await page.getByRole("button",{name:"Get provider quote",exact:true}).click();
  await page.getByRole("button",{name:"Open Coinbase checkout",exact:true}).waitFor();assert.equal(prepared,1);assert.equal(opened,0);
  assert.match(await page.locator("[data-guided-pay]").textContent(),/6.20 USD/);assert.match(await page.locator("[data-guided-excess]").textContent(),/2.99/);
  assert.equal(await page.evaluate(()=>window.testTopupTools.has("agent_bounties_prepare_topup")),true);
  await page.evaluate(()=>window.scrollTo(0,0));
  const quoteBox=await page.locator("[data-guided-next]").boundingBox();assert.ok(quoteBox.y+quoteBox.height<740,`quote action below first viewport at ${width}`);
  await page.screenshot({path:path.join(root,`evidence/guided-wallet-funding/quote-${width}.png`),fullPage:false});
  // A blocked popup neither opens the provider nor marks the order pending.
  await page.evaluate(()=>{window.open=()=>null;});await page.getByRole("button",{name:"Open Coinbase checkout",exact:true}).click();
  await page.getByText("Checkout could not open. Recheck the saved purchase before trying again.",{exact:true}).waitFor();assert.equal(opened,0);assert.equal(attempt.status,"prepared");
  await page.evaluate(()=>{window.AgentBountiesGuidedTopup.snapshot().attempt.expires_at=new Date(0).toISOString();});
  await page.getByRole("button",{name:"Open Coinbase checkout",exact:true}).click();await page.getByText("This unopened quote expired. Get a fresh quote.",{exact:true}).waitFor();assert.equal(opened,0);
  await page.reload();await page.getByRole("button",{name:"Replace unopened quote",exact:true}).waitFor();assert.equal(prepared,1);
  attempt.status="unknown";await page.reload();await page.getByRole("button",{name:"Check purchase and wallet",exact:true}).waitFor();assert.equal(await page.locator("[data-guided-choices]").isVisible(),false);
  await page.locator("[data-guided-guide]").click();assert.equal(await page.locator(".topup-guide li").count(),3);
  // Disabling rollout must preserve recovery instead of exposing the legacy purchase form.
  flag=false;await page.reload();await page.getByRole("button",{name:"Check purchase and wallet",exact:true}).waitFor();assert.equal(await page.locator("[data-legacy-topup]").isVisible(),false);flag=true;
  attempt.status="failed";attempt.quote.recovery_code="provider_purchase_failed";attempt.quote.support_url="https://help.coinbase.com/en/coinbase/trading-and-funding/coinbase-pay/using-onramp";
  await page.reload();await page.getByText("The purchase failed; a specific decline reason is unavailable here. Check the same order with provider support.",{exact:true}).waitFor();
  attempt.quote.recovery_code=null;attempt=null;await page.reload();await page.locator("[data-guided-country]").fill("ZZ");await page.getByRole("button",{name:"Check purchase options",exact:true}).click();await page.getByRole("button",{name:"Return to wallet review",exact:true}).waitFor();
  assert.equal(await page.locator("[data-guided-handoff]").isVisible(),true);
  await page.locator("[data-guided-country]").fill("MX");await page.getByRole("button",{name:"Check purchase options",exact:true}).click();providerError="region_unavailable";await page.getByRole("button",{name:"Get provider quote",exact:true}).click();await page.getByText("Choose another available provider for your confirmed country and payment method.",{exact:true}).waitFor();assert.equal(prepared,1);providerError=null;
  // An address displayed by another provider is never silently substituted.
  await page.evaluate(()=>{window.ethereum={isMetaMask:true,request:async()=>["0x2222222222222222222222222222222222222222"]};});
  const mismatch=await page.evaluate(async()=>{try{await window.AgentBountiesGuidedTopup.prepare({provider:"metamask"});return null;}catch(e){return e.message;}});assert.match(mismatch,/Choose the bounty wallet/);assert.equal(prepared,1);
  assert.deepEqual(errors,[]);
  attempt=null;unconfigured=true;await page.reload();await page.locator("[data-guided-country]").waitFor({state:"visible"});await page.locator("[data-guided-country]").fill("MX");await page.locator("[data-guided-next]").click();await page.getByText("Card purchases are not available here yet. Use an already funded Base wallet or return to your saved review.",{exact:true}).waitFor();
  assert.equal(await page.locator("[data-guided-card]").getAttribute("aria-busy"),"false");
  shortfall="0";await page.reload();await page.waitForURL(url=>url.pathname==="/post.html"&&url.searchParams.get("operation_id")===operation&&url.searchParams.get("analytics")==="off");
  assert.equal(opened,0,"returning wallet funds never authorizes a provider or bounty payment");
  await context.close();cases+=15;
 }
 console.log(`guided top-up browser checks: ${cases} passed; no live checkout or wallet calls`);
}finally{await browser.close();await new Promise(r=>server.close(r));}})().catch(error=>{console.error(error);server.close();process.exitCode=1;});
