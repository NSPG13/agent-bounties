// Acceptance outcomes, not a count of implementation assertions. Real API/DB;
// Coinbase login, SDK transport and Base responses remain explicit fixtures.
import assert from 'node:assert/strict';
import {after,test} from 'node:test';
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {setup,connect,signFunding} from './posting-flow-fixture.mjs';

const results=[];
const fields=['id','goal','acceptance_criteria','solver_reward_usdc','verifier_reward_usdc','task_window_days','review_mode','delivery_deadline','reference_attachment'];
const cases=['normal','sign_in','cancel_signature','cancel_signature_reload','cancel_transaction','cancel_transaction_reload','usdc_topup','usdc_topup_reload','eth_topup','eth_topup_reload','gas_topup','gas_topup_reload','lost_reply'];
const tool=(page,name,input={})=>page.evaluate(({name,input})=>window.localPageTools.get(name).execute(input),{name,input});
async function snapshot(page){return page.evaluate(()=>{const j=window.AgentBountiesWorkflow.createClient(window).load();return {...j.draft,id:j.id};});}
async function openSignature(h){await h.page.locator('[data-legal-consent-checkbox]').check();await h.page.locator('[data-fund-now]').click();await h.page.getByRole('dialog',{name:'Approve bounty funding signature',exact:true}).waitFor();}
async function reconnect(h){
  await h.page.reload();await h.page.waitForFunction(()=>window.AgentBountiesComposer?.review().explicitly_approved);
  await tool(h.page,'agent_bounties_open_funding_review');
  await h.page.locator('[data-wallet-options] button').filter({hasText:'Verified ownership'}).first().click();
  await h.page.getByRole('button',{name:'Use this wallet',exact:true}).click();
  await h.page.getByRole('button',{name:/Use or recover Coinbase embedded wallet/}).click();
  await h.page.waitForFunction(()=>document.querySelector('[data-wallet-state]').textContent.includes('Connected for this session'));
}
async function send(h){await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();await h.page.waitForURL('**/funded.html?**');}

for(const mobile of [false,true])for(const scenario of cases)test(`${mobile?'phone':'desktop'}: ${scenario} completes and preserves the operation`,async()=>{
  const options={mobile,reference:true,authenticated:scenario!=='sign_in',lostReply:scenario==='lost_reply',...(scenario.startsWith('usdc_')?{usdc:'0x0'}:{}),...(scenario.startsWith('eth_')?{eth:'0x0'}:{}),...(scenario.startsWith('gas_')?{eth:'0x1'}:{})};
  const row={scenario,viewport:mobile?'phone':'desktop',started_at:new Date().toISOString(),completion:false,recovery:!['normal','sign_in'].includes(scenario),continuity:[],tool_invocations:[],fallbacks:[{action:'credential and wallet confirmation controls',reason:'human-owned approvals; no tool may approve on behalf of a user'}],error:null};
  results.push(row);let h;
  try{
    h=await setup(options);h.page.setDefaultTimeout(5000);
    const initial=await snapshot(h.page);row.initial=initial;
    if(scenario==='sign_in'){
      await tool(h.page,'agent_bounties_open_account_setup');await h.page.waitForURL('**/?postReturn=1#login');
      h.state.authenticated=true;await h.page.reload();await h.page.waitForURL('**/post.html?**');
      await h.page.waitForFunction(()=>window.AgentBountiesComposer?.review().status==='staged');
    }
    await connect(h);
    if(scenario.startsWith('cancel_signature')){
      await openSignature(h);await h.page.getByRole('dialog',{name:'Approve bounty funding signature',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();
      await h.page.waitForFunction(()=>!document.querySelector('[data-fund-now]').disabled);
      assert.equal(h.state.signed,0);assert.equal(h.state.sent,0);
    }else if(scenario.startsWith('cancel_transaction')){
      await signFunding(h);await h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();
      await h.page.waitForFunction(()=>!document.querySelector('.wallet-auth-overlay'));
    }else if(scenario.startsWith('gas_')){
      await openSignature(h);await h.page.getByRole('button',{name:'Sign funding authorization',exact:true}).click();
      await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('Not enough Base ETH'));
    }
    if(scenario.endsWith('_reload'))await reconnect(h);
    h.state.usdc='0x5f5e100';h.state.eth='0x2386f26fc10000';
    await h.page.locator('[data-recheck-balance]').click();
    const current=await snapshot(h.page);
    for(const field of fields){const preserved=JSON.stringify(current[field])===JSON.stringify(initial[field]);row.continuity.push({field,preserved});assert.ok(preserved,`Lost or changed ${field}`);}
    assert.equal(h.context.pages().length,1);row.external_handoffs=0;
    if(scenario.startsWith('cancel_transaction')||scenario.startsWith('gas_')){
      assert.equal(await h.page.locator('[data-fund-now]').isEnabled(),true,'Known-unsent signed operation must be resumable');
      await h.page.locator('[data-fund-now]').click();
      await h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).waitFor();
    }else await signFunding(h);
    row.tool_invocations=h.state.tools;
    if(scenario==='lost_reply'){
      await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
      await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('may have been submitted'));
      await h.page.reload();await h.page.waitForFunction(()=>window.AgentBountiesComposer);
      h.state.canonical=true;h.state.inventory=true;
      const status=await tool(h.page,'agent_bounties_get_posting_status');
      assert.ok(status.creation_confirmed&&status.funding_confirmed&&status.claimable&&status.public_inventory_verified);
    }else await send(h);
    assert.equal(h.state.signed,1,'One funding authorization, reused on continuation');
    assert.equal(h.state.sent,1,'One transaction submission');
    assert.deepEqual(h.state.errors,[]);
    row.legal_acceptances=h.state.requests.filter(r=>r.path==='/v1/legal/acceptances').length;
    assert.equal(row.legal_acceptances,1,'Unchanged legal approval must not be requested again');
    row.canonical=h.state.canonicalChecks.findLast(r=>r.creation_confirmed&&r.funding_confirmed&&r.claimable&&r.public_inventory_verified);
    assert.ok(row.canonical,'All four canonical checks must have been observed');
    assert.equal(row.canonical.bounty_contract,h.state.plan.predicted_bounty_contract);
    assert.equal(row.canonical.operation_id,initial.id);
    row.completion=true;
  }catch(error){row.error=error.message;if(h){row.current=await snapshot(h.page).catch(()=>null);row.review=await h.page.evaluate(()=>window.AgentBountiesComposer?.review()).catch(()=>null);}throw error;}
  finally{
    if(h){row.tool_invocations=h.state.tools;row.signatures=h.state.signed;row.submissions=h.state.sent;row.native_webmcp_available=h.state.nativeWebMCP;row.api_failures=h.state.requests.filter(r=>r.path&&r.status>=400);await h.context.close();}
    row.finished_at=new Date().toISOString();
  }
});
after(async()=>{
  const out=process.env.POSTING_METRICS_OUT||'target/posting-metrics/latest.json';await mkdir(path.dirname(out),{recursive:true});
  const ratio=rows=>({successes:rows.filter(r=>r.completion).length,attempts:rows.length});
  const continuity=results.flatMap(r=>r.continuity);
  await writeFile(out,JSON.stringify({environment:'Real local site/API/PostgreSQL; simulated OAuth/CDP/Base; WebMCP registration shim',completion:ratio(results),recovery:ratio(results.filter(r=>r.recovery)),continuity:{successes:continuity.filter(r=>r.preserved).length,attempts:continuity.length},human_comprehension:'UNVERIFIED',live_coinbase:'UNVERIFIED',native_webmcp:'UNVERIFIED',results},null,2)+'\n');
});
