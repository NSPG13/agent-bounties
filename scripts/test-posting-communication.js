const assert=require('node:assert/strict');
const {test}=require('node:test');
const {readFileSync}=require('node:fs');
const flow=require('../site/marketplace-workflow.js');
const prompt=require('../site/posting-prompt.js');

test('each supported option explains decision-maker, evidence, cost, limitations and actual protocol',()=>{
  assert.equal(flow.POSTING_OPTIONS.choices.length,2);
  for(const option of flow.POSTING_OPTIONS.choices){
    for(const field of ['decision_maker','evidence','costs','limitations'])assert.ok(option[field]?.length>15,`${option.review_mode}: ${field}`);
    assert.equal(option.protocol_id,flow.POSTING_OPTIONS.protocol.id);
    assert.match(option.costs,/2 USDC/);assert.match(option.costs,/0\.01 USDC/);
  }
  assert.match(flow.POSTING_OPTIONS.choices[0].costs,/pays you after either completed verdict/);
  assert.match(flow.POSTING_OPTIONS.choices[1].limitations,/unsupported or unavailable test cannot be funded/i);
  assert.match(flow.POSTING_OPTIONS.campaign_limit,/shared budget cap/);
});
test('all four recommendations give a task-specific reason and protocol without approval',()=>{
  for(const [kind,mode,reason] of [['outreach','creator',/company identity/],['research','creator',/sources/],['creative','creator',/design/],['software','automated',/regression test/]]){
    const {recommendation}=flow.postingOptions(kind);assert.equal(recommendation.review_mode,mode);
    assert.equal(recommendation.protocol_id,flow.POSTING_OPTIONS.protocol.id);assert.match(recommendation.reason,reason);assert.equal(recommendation.approval_required,true);
  }
  assert.equal(flow.postingOptions().recommendation,null);
  assert.throws(()=>flow.postingOptions('unsupported'));
});
test('routine guidance follows the three-sentence limit; detailed consent is explicitly separate',()=>{
  for(const [key,text] of Object.entries(flow.GUIDANCE))if(key!=='consent')assert.ok(text.split(/(?<=[.!?])\s+(?=[A-Z])/).length<=3,key);
  assert.match(flow.GUIDANCE.style,/one primary next action/);
  assert.match(prompt.build(),/at most three short sentences/);
});
test('the page renders the shared choices and explains the posting sequence and protocol',()=>{
  const page=readFileSync(require.resolve('../site/post.html'),'utf8');
  const code=readFileSync(require.resolve('../site/bounty-composer-v2.js'),'utf8');
  assert.match(page,/Describe the work, choose how it is checked, then review the cost/);
  assert.match(page,/autonomous-v1 on Base/);
  assert.match(code,/AgentBountiesWorkflow\.POSTING_OPTIONS\.choices/);
  for(const field of ['decision_maker','evidence','costs','limitations'])assert.ok(code.includes(`choice.${field}`));
});
