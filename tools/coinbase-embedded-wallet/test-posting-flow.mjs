import assert from 'node:assert/strict';
import {test} from 'node:test';
import {mkdir} from 'node:fs/promises';
import path from 'node:path';
import {api,setup,connect,signFunding} from './posting-flow-fixture.mjs';

for(const mobile of [false,true])test(`real API accepts creator-review terms and Coinbase signature; exact confirmed inventory completes the posting (${mobile?'phone':'desktop'})`,async()=>{
  const h=await setup({mobile});try{
    await connect(h);await signFunding(h);
    const panel=h.page.locator('.wallet-auth-panel');
    assert.ok((await panel.boundingBox()).width<=h.page.viewportSize().width);
    assert.equal(await panel.evaluate(element=>element.scrollWidth<=element.clientWidth),true);
    assert.doesNotMatch(await panel.innerText(),/Estimated network fee\s+0 ETH/);
    if(process.env.POSTING_TEST_EVIDENCE_DIR){await mkdir(process.env.POSTING_TEST_EVIDENCE_DIR,{recursive:true});await panel.screenshot({path:path.join(process.env.POSTING_TEST_EVIDENCE_DIR,`coinbase-payment-review-${mobile?'phone':'desktop'}.png`)});}
    assert.equal(h.state.signed,1);assert.equal(h.state.sent,0);
    assert.ok(h.state.requests.some(request=>request.path?.endsWith('/authorized-creation-plan')&&request.status===200));
    await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
    await h.page.waitForURL('**/funded.html?**');
    assert.equal(new URL(h.page.url()).searchParams.get('bountyContract'),h.state.plan.predicted_bounty_contract);
    assert.equal(h.state.sent,1);assert.deepEqual(h.state.errors,[]);
    const persisted=await fetch(api+'/v1/base/autonomous-bounties/terms/'+h.state.terms.terms_hash);
    assert.equal(persisted.status,200);assert.equal((await persisted.json()).terms_hash,h.state.terms.terms_hash);
  }finally{await h.context.close();}
});

test('same-tab WebMCP sign-in preserves the operation, amounts, deadline and unapproved draft',async()=>{
  const h=await setup({authenticated:false});try{
    const before=await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load());
    const options=await h.page.evaluate(()=>window.localPageTools.get('agent_bounties_get_posting_options').execute());
    assert.deepEqual(options.choices.map(choice=>choice.review_mode),['creator','automated']);
    assert.match(options.campaign_limit,/shared budget cap/);
    await h.page.evaluate(()=>window.localPageTools.get('agent_bounties_open_account_setup').execute());
    await h.page.waitForURL('**/?postReturn=1#login');
    h.state.authenticated=true;await h.page.reload(); // Simulated OAuth completion only.
    await h.page.waitForURL('**/post.html?**');
    await h.page.waitForFunction(()=>window.AgentBountiesComposer);
    const after=await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load());
    assert.equal(after.id,before.id);
    for(const field of ['title','goal','acceptance_criteria','solver_reward_usdc','verifier_reward_usdc','task_window_days','review_mode','delivery_deadline','reference_attachment','meta_child'])assert.deepEqual(after.draft[field],before.draft[field],field);
    assert.equal(await h.page.evaluate(()=>window.AgentBountiesComposer.review().explicitly_approved),false);
    assert.equal(h.context.pages().length,1);assert.equal(h.state.signed,0);assert.equal(h.state.sent,0);
  }finally{await h.context.close();}
});

test('missing legal consent blocks publication and signatures',async()=>{
  const h=await setup();try{
    await connect(h);await h.page.locator('[data-fund-now]').click();
    await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('Read and accept'));
    assert.equal(h.state.terms,null);assert.equal(h.state.signed,0);assert.equal(h.state.sent,0);
  }finally{await h.context.close();}
});

test('cancelling the first signature allows another review of the same saved operation',async()=>{
  const h=await setup();try{
    await connect(h);const operation=await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load().id);
    await h.page.locator('[data-legal-consent-checkbox]').check();await h.page.locator('[data-fund-now]').click();
    const dialog=h.page.getByRole('dialog',{name:'Approve bounty funding signature',exact:true});await dialog.waitFor();
    await dialog.getByRole('button',{name:'Cancel',exact:true}).click();
    await h.page.waitForFunction(()=>!document.querySelector('[data-fund-now]').disabled);
    assert.equal(h.state.signed,0);assert.equal(h.state.sent,0);
    await h.page.locator('[data-fund-now]').click();
    try{await dialog.waitFor();}catch(error){throw new Error(`${error.message}\nPayment: ${await h.page.locator('[data-payment-status]').innerText()}\nAPI: ${JSON.stringify(h.state.requests.filter(request=>request.path&&request.status>=400))}`);}
    assert.equal(await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load().id),operation);
    await dialog.getByRole('button',{name:'Cancel',exact:true}).click();
  }finally{await h.context.close();}
});

test('confirmed events alone and another bounty in inventory cannot complete this posting',async()=>{
  const h=await setup({delayInventory:true});try{
    await connect(h);await signFunding(h);await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
    await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('still being reconciled'));
    const status=()=>h.page.evaluate(()=>window.localPageTools.get('agent_bounties_get_posting_status').execute());
    assert.equal((await status()).public_inventory_verified,false);
    h.state.inventory=true;h.state.inventoryContract='0x'+'11'.repeat(20);
    assert.equal((await status()).public_inventory_verified,false);
    h.state.inventoryContract=null;assert.equal((await status()).public_inventory_verified,true);assert.equal(h.state.sent,1);
  }finally{await h.context.close();}
});

for(const [label,options] of [['USDC shortfall',{usdc:'0x0'}],['ETH shortfall',{eth:'0x0'}]])test(`${label} stops before any funding signature`,async()=>{
  const h=await setup({...options,mobile:true});try{await connect(h);assert.equal(await h.page.locator('[data-fund-now]').isDisabled(),true);assert.equal(h.state.signed,0);assert.equal(h.state.sent,0);}finally{await h.context.close();}
});

test('a positive ETH balance below the estimated fee cannot send a transaction',async()=>{
  const h=await setup({eth:'0x1'});try{
    await connect(h);
    await h.page.locator('[data-legal-consent-checkbox]').check();
    await h.page.locator('[data-fund-now]').click();
    await h.page.getByRole('button',{name:'Sign funding authorization',exact:true}).click();
    await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('Not enough Base ETH'));
    assert.equal(h.state.signed,1);assert.equal(h.state.sent,0);
    assert.equal(await h.page.locator('[data-fund-now]').isEnabled(),true);
    assert.ok((await h.page.evaluate(()=>window.AgentBountiesComposer.review())).posting_operation);
    h.state.eth='0x2386f26fc10000';await h.page.locator('[data-recheck-balance]').click();
    await h.page.locator('[data-fund-now]').click();
    await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
    await h.page.waitForURL('**/funded.html?**');
    assert.equal(h.state.signed,1);assert.equal(h.state.sent,1);
  }finally{await h.context.close();}
});

test('cancelling after issuing an authorization preserves that operation and never re-signs on reload',async()=>{
  const h=await setup();try{
    await connect(h);await signFunding(h);
    const before=await h.page.evaluate(()=>window.AgentBountiesComposer.review().posting_operation);
    await h.page.getByRole('dialog',{name:'Confirm bounty transaction',exact:true}).getByRole('button',{name:'Cancel',exact:true}).click();
    await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').dataset.state==='error'||document.querySelector('[data-payment-status]').textContent.includes('cancel'));
    assert.equal(h.state.signed,1);assert.equal(h.state.sent,0);
    await h.page.reload();await h.page.waitForFunction(()=>window.AgentBountiesComposer);
    const after=await h.page.evaluate(()=>window.AgentBountiesComposer.review().posting_operation);
    assert.equal(after.bounty_id,before.bounty_id);
    assert.equal(after.bounty_contract,before.bounty_contract);
    assert.equal(h.state.signed,1);assert.equal(h.state.sent,0);
  }finally{await h.context.close();}
});

test('lost Coinbase reply survives reload and reconciles the same operation without resending',async()=>{
  const h=await setup({lostReply:true});try{
    await connect(h);const operation=await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load().id);
    await signFunding(h);await h.page.getByRole('button',{name:'Send transaction',exact:true}).click();
    await h.page.waitForFunction(()=>document.querySelector('[data-payment-status]').textContent.includes('may have been submitted'));
    assert.equal(h.state.sent,1);await h.page.reload();
    await h.page.waitForFunction(()=>window.AgentBountiesComposer);
    assert.equal(await h.page.evaluate(()=>window.AgentBountiesWorkflow.createClient(window).load().id),operation);
    assert.equal(h.state.sent,1);h.state.canonical=true;h.state.inventory=true;
    const result=await h.page.evaluate(()=>window.localPageTools.get('agent_bounties_get_posting_status').execute());
    assert.equal(result.public_inventory_verified,true);assert.equal(h.state.sent,1);
  }finally{await h.context.close();}
});
