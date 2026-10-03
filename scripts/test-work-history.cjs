const test = require('node:test'), assert = require('node:assert/strict');
const h = require('../site/work-history.js'), fixture = require('./fixtures/public-wallet-work-history.json');
const wallet = '0x515d935be52e76f0d9b37aa99fc33947b3b65f62';
const clone = value => JSON.parse(JSON.stringify(value));
const fetched = async url => new Response(JSON.stringify(fixture[h.SOURCES.find(s => s.url === url).id]));
test('captured public source replay preserves two distinct jobs and separates rewards from bonds', async () => {
  const result = await h.inspect(wallet.toUpperCase().replace('0X','0x'), fetched);
  assert.equal(result.records.length, 2); assert.equal(result.solver_reward_total_base_units, '1600000');
  assert.equal(new Set(result.records.map(r=>r.bounty_contract)).size,2);
  for (const row of result.records) { assert.equal(row.solver_reward_base_units,'800000'); assert.equal(row.returned_bond_base_units,'100000'); assert.match(row.tx_hash,/^0x[0-9a-f]{64}$/); }
  assert.equal(result.status,'inspected_sources_available'); assert.equal(result.lifetime_complete,false); assert.equal(result.chain_revalidated,false);
});
test('all three protocol fixtures retain source, factory and transaction/log references', () => {
  for (const source of h.SOURCES) {
    const payload=fixture[source.id], events=Array.isArray(payload)?payload:payload.events, settled=events.find(e=>e.kind===source.kind);
    const row=h.sourceRows(source,payload,settled.data.solver).rows.find(r=>r.tx_hash===settled.tx_hash);
    assert.equal(row.factory,source.factory);assert.equal(row.log_index,settled.log_index);assert.equal(row.source_url,source.url);assert.equal(row.event_kind,source.kind);
    if(!source.bond)assert.equal(row.returned_bond_base_units,null);
  }
});
test('large integer strings remain exact and unsafe native numbers are rejected', () => {
  const p=clone(fixture['autonomous-v1']);p.find(e=>e.kind==='bounty_settled').data.solver_reward='9007199254740993';
  assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rows[0].solver_reward_base_units,'9007199254740993');
  assert.equal(h.formatAmount('9007199254740993'),'9007199254.740993');assert.equal(h.formatAmount(null),'Not reported');assert.throws(()=>h.units(9007199254740992));
  try { assert.equal(h.parse('{"reward":9007199254740993}').reward,'9007199254740993'); } catch(error) { assert.match(error.message,/inexact numeric/); }
});
test('duplicates are counted once, conflicting duplicates invalidate the source', () => {
  const p=clone(fixture['autonomous-v1']), row=p.find(e=>e.kind==='bounty_settled');p.push(clone(row));
  assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rows.length,2);
  p.at(-1).data.solver_reward=1;assert.throws(()=>h.sourceRows(h.SOURCES[0],p,wallet),/Conflicting duplicate/);
});
test('wallet mismatch and non-settlement events cannot become payment observations', () => {
  const p=clone(fixture['autonomous-v1']);assert.equal(h.sourceRows(h.SOURCES[0],p,'0x'+'1'.repeat(40)).rows.length,0);
  p.filter(e=>e.kind==='bounty_settled').forEach(e=>e.kind='submission_added');assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rows.length,0);
});
test('matching settlement without canonical creation is rejected and disclosed', () => {
  const p=clone(fixture['autonomous-v1']).filter(e=>e.kind==='bounty_settled');const r=h.sourceRows(h.SOURCES[0],p,wallet);assert.equal(r.rows.length,0);assert.equal(r.rejected,2);
  const altered=clone(fixture['autonomous-v1']);altered.filter(e=>e.kind==='canonical_bounty_created').forEach(e=>e.contract_address='0x'+'1'.repeat(40));assert.equal(h.sourceRows(h.SOURCES[0],altered,wallet).rejected,2);
});
test('wrong network/protocol/factory envelopes fail closed and event mismatch is disclosed', () => {
  for(const [key,value] of [['network','base-sepolia'],['protocol_version','wrong'],['factory_contract','0x'+'1'.repeat(40)]]){const p=clone(fixture[h.SOURCES[1].id]);p[key]=value;assert.throws(()=>h.sourceRows(h.SOURCES[1],p,wallet));}
  const p=clone(fixture['autonomous-v1']);p.find(e=>e.kind==='bounty_settled').protocol_version='wrong';assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rejected,1);
});
test('invalid wallet makes no request; malformed amounts cannot count as zero', async () => {
  let calls=0;await assert.rejects(h.inspect('bad',()=>{calls++;}));assert.equal(calls,0);
  for(const value of [-1,1.5,true,'1e6']) {const p=clone(fixture['autonomous-v1']);p.find(e=>e.kind==='bounty_settled').data.solver_reward=value;assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rejected,1);}
});
test('explicitly noncanonical evidence and invalid event time cannot count as settlements', () => {
  for(const mutate of [e=>e.data.canonical_payment_evidence=false,e=>e.occurred_at=1,e=>e.occurred_at='invalid',e=>e.network='base-sepolia']){const p=clone(fixture['autonomous-v1']);mutate(p.find(e=>e.kind==='bounty_settled'));assert.equal(h.sourceRows(h.SOURCES[0],p,wallet).rejected,1);}
});
test('one failed source gives partial results; all failures never become an empty success', async () => {
  const partial=await h.inspect(wallet,url=>url===h.SOURCES[1].url?Promise.reject(new Error('offline')):fetched(url));assert.equal(partial.status,'partial');assert.equal(partial.records.length,2);assert.equal(partial.sources[1].error,'offline');
  const none=await h.inspect(wallet,()=>Promise.reject(new Error('offline')));assert.equal(none.status,'unavailable');assert.equal(none.solver_reward_total_base_units,null);
});
test('usable empty sources have a scoped no-match state', async () => {
  const r=await h.inspect('0x'+'1'.repeat(40),fetched);assert.equal(r.status,'inspected_sources_available');assert.equal(r.records.length,0);assert.equal(r.solver_reward_total_base_units,'0');assert.equal(r.lifetime_complete,false);
});
test('streamed byte limit cancels overflow without parsing or following links', async () => {
  let cancelled=false;const stream=new ReadableStream({start(c){c.enqueue(new Uint8Array(h.LIMIT+1));},cancel(){cancelled=true;}});
  await assert.rejects(h.readSource(h.SOURCES[0],async()=>new Response(stream)),/response limit/);assert.equal(cancelled,true);
});
test('transport pins three public URLs, refuses redirects and omits credentials', async () => {
  const requests=[];await h.inspect(wallet,async(url,options)=>{requests.push({url,options});return fetched(url);});assert.equal(requests.length,3);
  for(const r of requests){assert.ok(h.SOURCES.some(s=>s.url===r.url));assert.equal(r.options.redirect,'error');assert.equal(r.options.credentials,'omit');assert.equal(r.options.referrerPolicy,'no-referrer');assert.equal(r.options.headers.Authorization,undefined);assert.ok(r.options.signal instanceof AbortSignal);}
});
