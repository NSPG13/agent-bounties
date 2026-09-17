import assert from 'node:assert/strict';
import {test,after} from 'node:test';
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {setup,connect,signFunding} from './posting-flow-fixture.mjs';

const results=[];
const scenarios=['missing_terms','synthetic_click','double_click','changed_wallet','expired_authorization','sdk_timeout','mismatched_inventory','concurrent_tabs','changed_terms','private_continuation','tampered_continuation','changed_legal_policy'];
const tool=(page,name,input={})=>page.evaluate(({name,input})=>window.localPageTools.get(name).execute(input),{name,input});
async function cancelTransaction(h){await signFunding(h);await h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();await h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).waitFor({state:'hidden'});}
async function restoreSecondTab(h){
  const saved=await h.page.evaluate(()=>Object.fromEntries(Object.entries(sessionStorage)));
  const second=await h.context.newPage();second.setDefaultTimeout(8000);
  await second.addInitScript(saved=>{for(const [key,value] of Object.entries(saved))sessionStorage.setItem(key,value);},saved);
  await second.goto(h.page.url());await second.waitForFunction(()=>window.AgentBountiesComposer?.review().explicitly_approved);
  await tool(second,'agent_bounties_open_funding_review');
  await second.locator('[data-wallet-options] button').filter({hasText:'Saved to your account'}).first().click();
  await second.getByRole('button',{name:'Use this wallet',exact:true}).click();
  await second.getByRole('button',{name:/Use Coinbase with email/}).click();
  await second.waitForFunction(()=>document.querySelector('[data-wallet-state]').textContent.includes('Connected for this session'));
  return second;
}

for(const mobile of [false,true])for(const scenario of scenarios)test(`${mobile?'phone':'desktop'} safety: ${scenario}`,async()=>{
  const row={scenario,viewport:mobile?'phone':'desktop',pass:false,expected_signatures:1,expected_submissions:0,started_at:new Date().toISOString()};results.push(row);
  const h=await setup({mobile,sdkTimeout:scenario==='sdk_timeout',delayInventory:scenario==='mismatched_inventory'});h.page.setDefaultTimeout(8000);
  try{
    await connect(h);
    if(['missing_terms','synthetic_click','changed_terms'].includes(scenario))row.expected_signatures=0;
    if(scenario==='missing_terms'){
      assert.equal(await h.page.locator('[data-fund-now]').isDisabled(),true,'Payment stays disabled without legal consent');
      assert.equal(await h.page.locator('[data-funding-consent-hint]').isVisible(),true);
      await h.page.locator('[data-fund-now]').evaluate(button=>button.click());
      assert.equal(h.state.terms,null);
    }else if(scenario==='synthetic_click'){
      await h.page.locator('[data-legal-consent-checkbox]').check();
      await h.page.locator('[data-fund-now]').evaluate(button=>button.click());
      assert.equal(h.state.terms,null);
    }else if(scenario==='changed_terms'){
      await tool(h.page,'agent_bounties_stage_funded_bounty',{...h.draft,goal:'A materially different delivery'});
      assert.equal((await h.page.evaluate(()=>window.AgentBountiesComposer.review())).explicitly_approved,false);
      assert.equal(await h.page.locator('[data-fund-now]').isDisabled(),true);
    }else if(scenario==='double_click'){
      row.expected_submissions=1;
      await h.page.locator('[data-legal-consent-checkbox]').check();await h.page.locator('[data-fund-now]').dblclick();
      await h.page.getByRole('button',{name:'Sign funding authorization',exact:true}).click();
      await h.page.getByRole('button',{name:'Send transaction',exact:true}).dblclick();
      await h.page.waitForURL('**/funded.html?**');
    }else if(scenario==='concurrent_tabs'){
      row.expected_submissions=1;
      await cancelTransaction(h);const second=await restoreSecondTab(h);
      await Promise.all([h.page.locator('[data-fund-now]').click(),second.locator('[data-fund-now]').click()]);
      await Promise.all([h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).waitFor(),second.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).waitFor()]);
      let arrivals=0,release;const barrier=new Promise(resolve=>{release=resolve;});
      h.state.reservationBarrier=async()=>{if(++arrivals===2)release();await barrier;};
      await Promise.all([h.page.getByRole('button',{name:'Send transaction',exact:true}).click(),second.getByRole('button',{name:'Send transaction',exact:true}).click()]);
      await Promise.any([h.page.waitForURL('**/funded.html?**'),second.waitForURL('**/funded.html?**')]);
      row.submission_attempts=[...new Set(h.state.draftWrites.map(r=>r.recovery.submission_attempt_id).filter(Boolean))];
      assert.equal(row.submission_attempts.length,2,'Both tabs must contend with different reservation identities');
      assert.ok(h.state.requests.some(r=>r.path?.includes('/posting-drafts/')&&r.status===409),'The loser must fail the durable reservation');
    }else if(scenario==='changed_legal_policy'){
      await cancelTransaction(h);h.state.legalChanged=true;await h.page.reload();
      await h.page.waitForFunction(()=>window.AgentBountiesComposer?.review().explicitly_approved);
      await tool(h.page,'agent_bounties_open_funding_review');
      await h.page.locator('[data-wallet-options] button').filter({hasText:'Saved to your account'}).first().click();
      await h.page.getByRole('button',{name:'Use this wallet',exact:true}).click();
      await h.page.getByRole('button',{name:/Use Coinbase with email/}).click();
      await h.page.locator('[data-legal-consent-checkbox]').check();
      await h.page.locator('[data-fund-now]').click();
      await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('legal terms changed'));
    }else if(['private_continuation','tampered_continuation'].includes(scenario)){
      await cancelTransaction(h);
      if(scenario==='private_continuation'){
        const sensitive=await h.page.evaluate(()=>JSON.parse(sessionStorage.getItem('agent-bounties.posting-continuation.v1')).signature);
        const output=await tool(h.page,'agent_bounties_get_posting_status');
        assert.equal(JSON.stringify(output).includes(sensitive),false);
        assert.equal(JSON.stringify(h.state.draftWrites).includes(sensitive),false);
      }else{
        await h.page.evaluate(()=>{const key='agent-bounties.posting-continuation.v1';const data=JSON.parse(sessionStorage.getItem(key));data.create.initial_funding.amount++;sessionStorage.setItem(key,JSON.stringify(data));});
        await h.page.locator('[data-fund-now]').click();
        await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('integrity'));
      }
    }else{
      await signFunding(h);
      if(scenario==='changed_wallet')await h.page.evaluate(()=>{window.testCdpAddress='0x'+'99'.repeat(20);});
      if(scenario==='expired_authorization')await h.page.evaluate(()=>{const future=Date.now()+40*86400000;Date.now=()=>future;});
      if(['sdk_timeout','mismatched_inventory'].includes(scenario))row.expected_submissions=1;
      await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
      if(scenario==='changed_wallet'||scenario==='expired_authorization'){
        await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('changed')||document.querySelector('[data-payment-status]').textContent.includes('expired'));
      }else if(scenario==='sdk_timeout'){
        await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('may have been submitted'));
        await h.page.reload();await h.page.waitForFunction(()=>window.AgentBountiesComposer);
        const status=await tool(h.page,'agent_bounties_get_posting_status');
        assert.equal(status.public_inventory_verified,false);assert.equal(await h.page.locator('[data-fund-now]').isDisabled(),true);
        row.false_success_observations=1;
      }else{
        await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('still being reconciled'));
        assert.equal((await tool(h.page,'agent_bounties_get_posting_status')).public_inventory_verified,false);
        h.state.inventory=true;h.state.inventoryContract='0x'+'11'.repeat(20);
        assert.equal((await tool(h.page,'agent_bounties_get_posting_status')).public_inventory_verified,false);
        row.false_success_observations=2;
      }
    }
    assert.equal(h.state.signed,row.expected_signatures);assert.equal(h.state.sent,row.expected_submissions);
    row.pass=true;
  }catch(error){row.error=error.message;row.review=await h.page.evaluate(()=>window.AgentBountiesComposer?.review()).catch(()=>null);throw error;}
  finally{row.signatures=h.state.signed;row.submissions=h.state.sent;row.api_failures=h.state.requests.filter(r=>r.path&&r.status>=400);row.finished_at=new Date().toISOString();await h.context.close();}
});
after(async()=>{const out=process.env.POSTING_SAFETY_OUT||'target/posting-metrics/safety.json';await mkdir(path.dirname(out),{recursive:true});await writeFile(out,JSON.stringify({environment:'Real local site/API/PostgreSQL; simulated Coinbase/Base',passed:results.filter(r=>r.pass).length,attempts:results.length,results},null,2)+'\n');});
