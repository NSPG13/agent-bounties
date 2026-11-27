"use strict";
// Exercise the actual page, durable draft and wallet boundaries. Every remote
// request is intercepted; no wallet, processor, public API or chain is used.
const fs=require('node:fs'),path=require('node:path'),vm=require('node:vm'),assert=require('node:assert/strict');
const {createHash}=require('node:crypto');
const root=path.resolve(__dirname,'..');
const base=fs.readFileSync(path.join(__dirname,'test-posting-layout.cjs'),'utf8');
const ctx=vm.createContext({require,__dirname,process,console,URL,Buffer});
vm.runInContext(base.slice(0,base.indexOf('async function recoveryRegressions'))+'\nthis.fixtureApi={server,fixtures,fixture,wallet,awaitPosting,playwright};',ctx);
const {server,fixtures,fixture,wallet,awaitPosting,playwright}=ctx.fixtureApi;
const evmCtx=vm.createContext({window:{},TextEncoder});
for(const f of ['evm.js','posting-v2-plan.js'])vm.runInContext(fs.readFileSync(path.join(root,'site',f),'utf8'),evmCtx);
const evm=evmCtx.window.AgentBountiesEvm,adapter=evmCtx.window.AgentBountiesPostingV2Plan;
const publication=require('../site/terms-publication.js');
const protocol=JSON.parse(fs.readFileSync(path.join(root,'site/protocol-v2.json')));
const hash=x=>evm.keccak256Hex(evm.textHex(publication.canonical(x)));
const statement='Isolated browser fixture agreement. No payment or legal acceptance leaves this browser test.';
const policy={schema_version:'agent-bounties/legal-policy-v1',terms_version:'fixture',privacy_version:'fixture',statement,statement_hash:'sha256:'+createHash('sha256').update(statement).digest('hex'),supported_actions:['post_bounty']};
const scenarios=['success','changed-plan','second-request-lost','second-request-rejected','wrong-network','no-eth','account-during-estimate','chain-during-estimate','other-device-during-quote',
 // Creation receipts: only an explicit status for the requested hash proves an outcome.
 'creation-reverted','receipt-empty','receipt-array','receipt-unknown-status','receipt-wrong-hash'];
const uncertainReceipts=['receipt-empty','receipt-array','receipt-unknown-status','receipt-wrong-hash'];
(async()=>{
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const origin='http://127.0.0.1:'+server.address().port;
 const browser=await playwright.chromium.launch({headless:true,executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE||undefined});
 try { for(const scenario of scenarios){
  const context=await browser.newContext({viewport:{width:scenario==='success'?390:1440,height:950}});
  const mock=await fixtures(context,origin,{wallets:[wallet]});const requests=[],errors=[];let finalPlan;
  await context.route('https://api.agentizable.ai/**',async route=>{
   const req=route.request(),u=new URL(req.url());
   if(u.pathname==='/v1/legal/policy')return route.fulfill({json:policy});
   if(u.pathname==='/v1/legal/acceptances')return route.fulfill({json:{...req.postDataJSON(),acceptance_id:'fixture-receipt'}});
   if(u.pathname==='/v1/base/autonomous-bounties/terms'){
    const input=req.postDataJSON(),document={...input.document};
    const record={creator_wallet:input.creator_wallet,document};for(const [key,value] of Object.entries({terms_hash:document,policy_hash:document.verification_policy,acceptance_criteria_hash:document.acceptance_criteria,benchmark_hash:document.benchmark,evidence_schema_hash:document.evidence_schema}))record[key]=hash(value);
    requests.push('terms');return route.fulfill({json:record});
   }
   if(u.pathname.startsWith('/v1/base/autonomous-bounties/v2/')){
    const input=req.postDataJSON(),create=input.create||{solver_reward:input.solver_reward,verifier_reward:input.verifier_reward};
    const amounts=adapter.economics(create.solver_reward.amount,create.verifier_reward.amount,protocol);
    const quote={protocol_version:adapter.VERSION,solver_reward:String(amounts.solver),verifier_reward:String(amounts.verifier),claim_bond:String(amounts.verifier),platform_fee_bps:750,platform_fee:String(amounts.fee),platform_fee_recipient:protocol.platform_fee_recipient,target_amount:String(amounts.total)};
    if(u.pathname.endsWith('/quote')){requests.push('quote'); if(scenario==='other-device-during-quote')for(const [key,value] of mock.drafts)mock.drafts.set(key,{...value,revision:value.revision+1,approved_draft_hash:null}); return route.fulfill({json:quote});}
    const encoded=adapter.encode(create,protocol);
    finalPlan={protocol_version:adapter.VERSION,network:{chain_id:8453,native_usdc_token_address:protocol.native_usdc},factory_contract:protocol.factory,implementation_contract:protocol.implementation,bounty_id:encoded.bountyId,predicted_bounty_contract:encoded.bounty,quote,
     wallet_calls:[{from:create.creator,to:protocol.native_usdc,value_wei:0,data:'0x095ea7b3'+protocol.factory.slice(2).padStart(64,'0')+amounts.total.toString(16).padStart(64,'0')},{from:create.creator,to:protocol.factory,value_wei:0,data:encoded.data}]};
    if(scenario==='changed-plan')finalPlan.wallet_calls[1].data=finalPlan.wallet_calls[1].data.slice(0,-1)+'f';
    // Advertising a signature must not send this new creation through V1.
    finalPlan.eip3009_authorization={primaryType:'ReceiveWithAuthorization'};
    requests.push('plan');return route.fulfill({json:finalPlan});
   }
   if(u.pathname.includes('creation-relay'))throw Error('Retired relay reached');
   return route.fallback();
  });
  // Empty, complete canonical evidence on whichever API base the page uses:
  // nothing was created, so a refused creation may be finished.
  const protocol_statuses=['autonomous-v1','open-competition-v1','open-competition-v2-beta3','autonomous-v2'].map(protocol=>({protocol,coverage:'complete',item_count:0,error:null}));
  await context.route('**/v1/base/autonomous-bounties/events?**',route=>route.fulfill({json:[]}));
  await context.route('**/v1/opportunities?**',route=>route.fulfill({json:{schema_version:'agent-bounties/opportunity-projection-v1',coverage_scope:'inventory_page',coverage:'complete',degraded:false,items:[],
   source_statuses:[{source_type:'canonical_base',coverage:'complete',available:true,item_count:0,protocol_statuses}]}}));
  await context.addInitScript(({scenario,address})=>{
   window.__v2Requests=[];window.__v2Sends=0;
   window.ethereum={isMetaMask:true,request:async req=>{
    window.__v2Requests.push(req);
    if(['eth_requestAccounts','eth_accounts'].includes(req.method))return [window.__changedAccount?'0x2222222222222222222222222222222222222222':address];
    if(req.method==='eth_chainId')return scenario==='wrong-network'||window.__changedChain?'0x1':'0x2105';
    if(req.method==='wallet_switchEthereumChain')throw Error('fixture refused switch');
    if(req.method==='eth_blockNumber')return '0x64';
    if(req.method==='eth_getBalance')return scenario==='no-eth'?'0x0':'0x2386f26fc10000';
    if(req.method==='eth_call')return req.params[0].data.startsWith('0x70a08231')?'0x5f5e100':'0x1';
    if(req.method==='eth_estimateGas'){if(scenario==='account-during-estimate')window.__changedAccount=true;if(scenario==='chain-during-estimate')window.__changedChain=true;return '0x186a0';}if(req.method==='eth_gasPrice')return '0xf4240';
    if(req.method==='eth_getTransactionReceipt'){
     const hash=req.params[0],creation=hash==='0x'+'2'.repeat(64);
     if(creation&&scenario==='creation-reverted')return {status:'0x0',transactionHash:hash};
     if(creation&&scenario==='receipt-empty')return {};
     if(creation&&scenario==='receipt-array')return [];
     if(creation&&scenario==='receipt-unknown-status')return {status:true,transactionHash:hash};
     if(creation&&scenario==='receipt-wrong-hash')return {status:'0x0',transactionHash:'0x'+'9'.repeat(64)};
     return {status:'0x1',transactionHash:hash};
    }
    if(req.method==='eth_sendTransaction'){
     window.__v2Sends++;if(window.__v2Sends===2&&scenario==='second-request-lost')throw Error('lost wallet reply');
     if(window.__v2Sends===2&&scenario==='second-request-rejected'&&!sessionStorage.getItem('__v2RejectedOnce')){sessionStorage.setItem('__v2RejectedOnce','1');throw Object.assign(Error('User rejected second request'),{code:4001});}
     if(window.__v2Sends===2&&!/^(creation-reverted|receipt-)/.test(scenario))window.__v2Finished=true;
     return '0x'+String(window.__v2Sends).repeat(64);
    }
    throw Error('Unexpected wallet request '+req.method);
   }};
  },{scenario,address:wallet.address});
  const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
  await page.goto(origin+'/post.html?from=webmcp&analytics=off');await awaitPosting(page);
  await page.evaluate(draft=>window.AgentBountiesComposer.stage(draft),fixture);
  await page.waitForFunction(()=>document.querySelector('[data-approve-card]').dataset.nextAction==='approve');
  await page.locator('[data-approve-card]').click();await page.locator('#funding-dialog[open]').waitFor();
  await page.getByRole('button',{name:/MetaMask/}).click();
  if(['wrong-network','no-eth'].includes(scenario)){
   await page.waitForFunction(()=>document.querySelector('[data-wallet-state]').textContent.includes('Connected for this session')||document.querySelector('[data-payment-status]')?.textContent.includes('fixture'));
   assert.equal(await page.locator('[data-fund-now]').isDisabled(),true);
  } else {
   await page.locator('[data-v2-cost-consent]').waitFor({state:'visible'});
   assert.match(await page.locator('[data-funding-total]').innerText(),/21.35 USDC.*Base ETH/);
   assert.match(await page.locator('[data-funding-split]').innerText(),/Platform fee: 1.35 USDC/);
   await page.locator('[data-legal-consent-checkbox]').check();assert.equal(await page.locator('[data-fund-now]').isDisabled(),true);
   await page.locator('[data-v2-cost-consent]').check();assert.equal(await page.locator('[data-fund-now]').isEnabled(),true);
   await page.locator('[data-fund-now]').click();
   await page.waitForFunction(()=>window.__v2Finished||document.querySelector('[data-funding-feedback]')?.dataset.tone==='error');
   const journal=await page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-operation.v1')));
   const walletCalls=await page.evaluate(()=>window.__v2Requests);
   const sends=walletCalls.filter(r=>r.method==='eth_sendTransaction');
   assert.equal(walletCalls.some(r=>/signTyped|sendCalls/.test(r.method)),false);
   if(scenario==='changed-plan'){assert.equal(sends.length,0);assert.equal(journal,null);}
   else if(['account-during-estimate','chain-during-estimate','other-device-during-quote'].includes(scenario)){assert.equal(sends.length,0);assert.equal(journal,null,'a request the wallet never received leaves no recorded operation');}
   else {assert.equal(sends.length,2);assert.equal(sends[0].params[0].data,finalPlan.wallet_calls[0].data);assert.equal(sends[1].params[0].data,finalPlan.wallet_calls[1].data);assert.ok(journal);}
   if(scenario==='second-request-lost'){
    // The wallet may have the creation: never offer it again, only reconcile.
    assert.equal(journal.transactions.length,1);assert.equal(journal.phase,'sending');assert.equal(await page.locator('[data-fund-now]').isDisabled(),true);
    await page.reload();await awaitPosting(page);assert.ok(await page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-operation.v1'))));
   }
   if(scenario==='creation-reverted'){
    // An explicit 0x0 receipt for the requested hash proves the creation reverted: no USDC moved.
    assert.equal(journal.transactions.length,2);assert.equal(journal.phase,'v2_creation_pending');
    assert.match(await page.locator('[data-payment-status]').textContent(),/reverted.*No USDC has moved/);
    assert.equal(await page.locator('[data-fund-now]').isEnabled(),true);assert.equal(await page.locator('[data-fund-now]').innerText(),'Finish posting');
   }
   if(uncertainReceipts.includes(scenario)){
    // A malformed or mismatched receipt proves nothing: stay locked, never claim that no USDC moved.
    assert.equal(journal.transactions.length,2);assert.notEqual(journal.phase,'v2_creation_pending');
    const status=await page.locator('[data-payment-status]').textContent();
    assert.match(status,/could not be verified.*do not pay again/);assert.doesNotMatch(status,/No USDC has moved|reverted|Finish posting/);
    assert.equal(await page.locator('[data-fund-now]').isDisabled(),true);assert.notEqual(await page.locator('[data-fund-now]').innerText(),'Finish posting');
    await page.reload();await awaitPosting(page);
    const kept=await page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-operation.v1')));
    assert.ok(kept);assert.notEqual(kept.phase,'v2_creation_pending');assert.equal(kept.transactions.length,2);
    assert.equal((await page.evaluate(()=>window.__v2Requests||[])).filter(r=>r.method==='eth_sendTransaction').length,0,'a reload never re-sends an uncertain creation');
   }
   if(scenario==='second-request-rejected'){
    // A refused creation moved no USDC: the same bounty can be finished, even after a reload.
    assert.equal(journal.transactions.length,1);assert.equal(journal.phase,'v2_creation_pending');
    assert.match(await page.locator('[data-payment-status]').textContent(),/No USDC has moved/);
    assert.equal(await page.locator('[data-fund-now]').isEnabled(),true);assert.equal(await page.locator('[data-fund-now]').innerText(),'Finish posting');
    await page.reload();await awaitPosting(page);
    assert.equal((await page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-operation.v1')))).phase,'v2_creation_pending');
    if(!await page.locator('#funding-dialog[open]').count())await page.locator('[data-open-funding]').click();
    await page.locator('#funding-dialog[open]').waitFor();
    await page.getByRole('button',{name:/MetaMask/}).click();
    await page.locator('[data-fund-now]').filter({hasText:'Finish posting'}).waitFor();
    // The fee-and-gas acknowledgement is bound to this page session, so it is asked again.
    await page.locator('[data-v2-cost-consent]').check();
    await page.locator('[data-fund-now]').click();
    await page.waitForFunction(()=>window.__v2Requests.filter(r=>r.method==='eth_sendTransaction').length===2||document.querySelector('[data-funding-feedback]')?.dataset.tone==='error');
    const resumed=(await page.evaluate(()=>window.__v2Requests)).filter(r=>r.method==='eth_sendTransaction');
    // This mock reports no allowance on Base, so the exact allowance is requested again before the same creation.
    assert.deepEqual(resumed.map(r=>r.params[0].data),finalPlan.wallet_calls.map(c=>c.data),await page.locator('[data-payment-status]').textContent());
    assert.equal((await page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-operation.v1')))).bounty_contract,journal.bounty_contract);
    requests.splice(3);
   }
   assert.deepEqual(requests,['terms','quote','plan']);
  }
  assert.deepEqual(errors,[]);console.log('PASS',scenario);await context.close();
 }} finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(e=>{console.error(e);server.close();process.exitCode=1;});
