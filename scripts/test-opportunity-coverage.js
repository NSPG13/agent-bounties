const test = require('node:test');
const assert = require('node:assert/strict');
const flow = require('../site/marketplace-workflow.js');
const history = require('../site/submissions.js');
const home = require('../site/solarpunk-home.js');
const fixture = require('./fixtures/funded-bounty.cjs').opportunity;
const item = { ...fixture, opportunity_id: `canonical:base-mainnet:${fixture.source_id}`, competition_mode: 'exclusive_claim', deadline: '2030-01-01T00:00:00Z' };
const status = (protocol, coverage, count) => ({ protocol, coverage, item_count: count, error: coverage === 'unavailable' ? 'read_model_unavailable' : null });
function projection() {
  return { schema_version: 'agent-bounties/opportunity-projection-v1', generated_at: '2026-10-17T02:00:00Z', network: 'base-mainnet', applied_view: 'ready_to_earn', degraded: true,
    coverage: 'partial', coverage_scope: 'inventory_page', requested_opportunity_id: null,
    source_statuses: [{source_type: 'canonical_base', available: false, coverage: 'partial', item_count: 1,
      protocol_statuses: [status('autonomous-v1', 'complete', 1), status('open-competition-v1', 'complete', 0), status('open-competition-v2-beta3', 'unavailable', null)]}], items: [structuredClone(item)] };
}
test('failed Beta3 leaves the healthy funded autonomous result usable and its count incomplete', () => {
  const result = flow.fundedInventory(projection());
  assert.deepEqual(result.items, [item]);
  assert.equal(result.coverage.state, 'partial');
  assert.deepEqual(result.coverage.missing, ['open-competition-v2-beta3']);
  assert.match(result.coverage.message, /full count is unknown/);
  const platform = { marketplace_payout_volume: { lifetime: { usdc: 5 }, lifetime_settled_rounds: 1 }, daily: [] };
  const snapshot = home.marketSnapshot(platform, projection(), Date.parse('2026-10-17T02:00:00Z'));
  assert.equal(snapshot.live, null);
  assert.equal(snapshot.availability.now, 1);
  assert.equal(snapshot.payout, 5);
});
test('all unavailable means unknown rather than a healthy zero; malformed partial data is rejected', () => {
  const payload = projection(); payload.items = []; payload.coverage = 'unavailable';
  const source = payload.source_statuses[0]; source.coverage = 'unavailable'; source.item_count = 0;
  source.protocol_statuses = source.protocol_statuses.map(s => status(s.protocol, 'unavailable', null));
  assert.equal(flow.fundedInventory(payload).coverage.state, 'unavailable');
  assert.match(flow.fundedInventory(payload).coverage.message, /number of bounties is unknown/);
  for (const mutate of [
    p => p.source_statuses[0].protocol_statuses.pop(),
    p => p.source_statuses[0].protocol_statuses[2].item_count = 0,
    p => p.source_statuses[0].available = true,
    p => p.coverage = 'complete',
    p => p.degraded = false,
    p => p.source_statuses[0].protocol_statuses[0].error = 'unavailable',
    p => p.source_statuses[0].protocol_statuses[0] = status('autonomous-v1', 'unavailable', null),
    p => p.items[0].verification_ready = false,
    p => p.items[0].payment_committed = false,
    p => p.items[0].network = 'base-sepolia',
  ]) { const changed = projection(); mutate(changed); assert.throws(() => flow.fundedInventory(changed)); }
});
test('exact identity coverage does not imply whole-protocol inventory coverage', () => {
  const payload = projection(); payload.applied_view = 'recent'; payload.degraded = false;
  payload.coverage = 'complete'; payload.coverage_scope = 'canonical_identity'; payload.requested_opportunity_id = item.opportunity_id;
  Object.assign(payload.source_statuses[0], {coverage: 'complete', available: true, protocol_statuses: [status('autonomous-v1', 'complete', 1)]});
  assert.equal(history.projection(payload, false, item.opportunity_id).items.length, 1);
  assert.throws(() => flow.canonicalCoverage(payload));
  assert.throws(() => history.projection(payload, false, item.opportunity_id + '0'));
});
test('partial paid history retains payment and canonical identity guards', () => {
  const payload = projection(); payload.applied_view = 'recent';
  Object.assign(payload.items[0], {work_state: 'completed', payment_state: 'paid', source_status: 'paid'});
  assert.equal(history.projection(payload, true).items.length, 1);
  payload.items[0].payment_state = 'escrowed';
  assert.throws(() => history.projection(payload, true));
});
test('the rendered board shows a healthy card and an explicit partial-list notice', async () => {
  const market = require('../site/marketplace.js');
  const nodes = new Map();
  const doc = { visibilityState: 'visible', querySelector(selector) {
    if (!nodes.has(selector)) nodes.set(selector, {textContent:'', innerHTML:'', value: selector === '[data-market-timing]' ? 'all' : '', hidden:true,
      setAttribute() {}, addEventListener() {}, querySelector() { return null; }});
    return nodes.get(selector);
  }};
  const payload = projection();
  const win = {location:new URL('https://agentizable.ai/marketplace.html'), setInterval() {}, async fetch() {return {ok:true,json:async()=>payload};}};
  market.startBoard(win, doc);
  await new Promise(resolve => setImmediate(resolve));
  assert.match(nodes.get('[data-market-summary]').textContent, /full count is unknown/);
  assert.match(nodes.get('[data-market-summary]').textContent, /1 checked listings shown/);
  assert.ok(nodes.get('[data-opportunity-list]').innerHTML.includes(item.title));
});
test('ChatGPT widget distinguishes missing coverage from an empty checked page', () => {
  const fs = require('node:fs');
  const vm = require('node:vm');
  const html = fs.readFileSync(require('node:path').join(__dirname, '../crates/mcp-server/assets/chatgpt-bounty-feed-widget.html'), 'utf8');
  const source = html.slice(html.indexOf('        function coverageNotice('), html.indexOf('        function render()'));
  const context = {}; vm.runInNewContext(source, context);
  assert.match(context.coverageNotice({coverage:'partial'}), /full count is unknown/);
  assert.match(context.coverageNotice({coverage:'unavailable'}), /number of bounties is unknown/);
  assert.match(context.coverageNotice({degraded:true}), /unknown/);
  assert.equal(context.coverageNotice({coverage:'complete'}), '');
});
test('capped pages never become a full homepage inventory total', () => {
  for (const has_more of [true, undefined]) {
    const payload = projection(); payload.coverage = 'complete'; payload.degraded = false; payload.has_more = has_more;
    Object.assign(payload.source_statuses[0], {coverage:'complete',available:true,protocol_statuses:[status('autonomous-v1','complete',301),status('open-competition-v1','complete',0),status('open-competition-v2-beta3','complete',0)]});
    payload.items = Array.from({length:300}, (_,i) => ({...item, source_id:'0x'+(i+1).toString(16).padStart(40,'0'), opportunity_id:'canonical:base-mainnet:0x'+(i+1).toString(16).padStart(40,'0')}));
    const result = home.marketSnapshot({marketplace_payout_volume:{lifetime:{usdc:5},lifetime_settled_rounds:1},daily:[]},payload,Date.parse('2026-10-17T02:00:00Z'));
    assert.equal(result.live, null);
    assert.equal(result.availability.now, 300);
    assert.equal(result.coverage.limited, true);
    assert.match(result.coverage.message,/limited page.*full count is unknown/);
  }
});

function exactProjection(entry = item, view = 'recent') {
  const payload = projection();
  payload.items = [structuredClone(entry)];
  payload.applied_view = view;
  payload.degraded = false;
  payload.coverage = 'complete';
  payload.coverage_scope = 'canonical_identity';
  payload.requested_opportunity_id = entry.opportunity_id;
  payload.has_more = false;
  const protocol = entry.opportunity_id.startsWith('canonical:') ? 'autonomous-v1'
    : entry.opportunity_id.startsWith('open-competition-v2:') ? 'open-competition-v2-beta3' : 'open-competition-v1';
  Object.assign(payload.source_statuses[0], {coverage:'complete', available:true, item_count:1, protocol_statuses:[status(protocol,'complete',1)]});
  return payload;
}
function exactClient(payload, httpStatus = 200) {
  const calls = [];
  const win = {location:new URL('https://agentizable.ai/marketplace.html'), async fetch(url, options) {
    calls.push({url:new URL(url), options});
    return {ok:httpStatus === 200, status:httpStatus, json:async()=>structuredClone(payload)};
  }};
  return {client:flow.createClient(win), calls};
}
test('selected bounty reads only its scoped identity for each supported protocol, including old completed work', async () => {
  for (const prefix of ['canonical', 'open-competition', 'open-competition-v2']) {
    const entry = {...item, opportunity_id:`${prefix}:base-mainnet:${item.source_id}`, work_state:'completed', source_status:'paid', payment_state:'paid', terms_hash:null};
    const {client, calls} = exactClient(exactProjection(entry));
    const result = await client.inspect(entry.opportunity_id);
    assert.equal(result.opportunity_id, entry.opportunity_id);
    assert.equal(result.ready, false);
    assert.equal(calls.length, 1, 'no full-market or second-view fallback');
    assert.equal(calls[0].url.searchParams.get('opportunity_id'), entry.opportunity_id);
    assert.equal(calls[0].url.searchParams.get('limit'), '1');
    assert.equal(calls[0].url.searchParams.get('source_type'), 'canonical_base');
    assert.equal(calls[0].options.credentials, 'omit');
    assert.equal(calls[0].options.headers.Authorization, undefined);
    assert.equal(calls[0].options.method, 'GET');
  }
});
test('missing or private-filtered identities stop after one read; the client does not widen access or scan another view', async () => {
  const payload = exactProjection(); payload.items = [];
  payload.source_statuses[0].item_count = 0; payload.source_statuses[0].protocol_statuses[0].item_count = 0;
  for (const httpStatus of [200, 401, 403, 404, 503]) {
    const {client, calls} = exactClient(payload, httpStatus);
    await assert.rejects(client.inspect(item.opportunity_id));
    assert.equal(calls.length, 1);
    assert.equal(calls[0].options.credentials, 'omit');
    assert.equal(calls[0].options.headers.Authorization, undefined);
  }
});
test('unavailable coverage never starts a second scan or creates an action', async () => {
  const payload = exactProjection(item, 'ready_to_earn');
  payload.items = []; payload.coverage = 'unavailable'; payload.degraded = true;
  Object.assign(payload.source_statuses[0], {coverage:'unavailable',available:false,item_count:0,protocol_statuses:[status('autonomous-v1','unavailable',null)]});
  const {client, calls} = exactClient(payload);
  await assert.rejects(client.prepareAction({action:'solve', opportunity_id:item.opportunity_id}), /unavailable/);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].options.method, 'GET');
});
test('identity, network, scope and incomplete response mismatches cannot substitute another bounty', async () => {
  for (const mutate of [
    p => p.requested_opportunity_id = item.opportunity_id.replace('canonical:', 'open-competition:'),
    p => p.items[0].opportunity_id = item.opportunity_id.replace('canonical:', 'open-competition:'),
    p => p.items[0].source_id = '0x' + '22'.repeat(20),
    p => p.items[0].source_type = 'legacy_bounty',
    p => p.items[0].network = 'base-sepolia',
    p => p.network = 'base-sepolia',
    p => p.applied_view = 'ready_to_earn',
    p => p.coverage_scope = 'inventory_page',
    p => delete p.source_statuses[0].coverage,
    p => p.items.push(structuredClone(p.items[0])),
    p => p.has_more = true,
    p => p.source_statuses[0].protocol_statuses[0].protocol = 'open-competition-v1',
  ]) {
    const payload = exactProjection(); mutate(payload);
    const {client, calls} = exactClient(payload);
    await assert.rejects(client.inspect(item.opportunity_id));
    assert.equal(calls.length, 1);
  }
});
test('a scoped result retains every existing readiness guard before action preparation', async () => {
  for (const mutate of [
    i => i.verification_ready = false, i => i.payment_committed = false,
    i => i.work_state = 'submitted', i => i.source_status = 'cancelled',
    i => i.payment_state = 'none', i => i.funded_amount.amount = '1',
    i => i.reward.amount = '0', i => i.terms_hash = null,
  ]) {
    const payload = exactProjection(item, 'ready_to_earn'); mutate(payload.items[0]);
    const {client, calls} = exactClient(payload);
    await assert.rejects(client.prepareAction({action:'solve',opportunity_id:item.opportunity_id}), /not ready/);
    assert.equal(calls.length, 1);
    assert.equal(calls[0].options.method, 'GET');
    assert.equal(calls[0].url.searchParams.get('view'), 'ready_to_earn');
  }
});
test('ambiguous addresses and unsupported identity forms fail before any request', async () => {
  for (const id of [item.source_id, null, {}, item.opportunity_id.replace('base-mainnet','base-sepolia'),
    item.opportunity_id.replace('canonical:','unfunded:'), item.opportunity_id + '&view=recent', ' ' + item.opportunity_id]) {
    const {client, calls} = exactClient(exactProjection());
    await assert.rejects(client.inspect(id), /full opportunity ID/);
    assert.equal(calls.length, 0);
  }
});
test('mixed-case contract spelling selects the same normalized exact identity', async () => {
  const entry = {...item, source_id:'0x' + 'ab'.repeat(20), opportunity_id:'canonical:base-mainnet:0x' + 'ab'.repeat(20), terms_hash:null};
  const {client, calls} = exactClient(exactProjection(entry));
  assert.equal((await client.inspect(entry.opportunity_id.replaceAll('ab', 'aB'))).opportunity_id,entry.opportunity_id);
  assert.equal(calls[0].url.searchParams.get('opportunity_id'),entry.opportunity_id);
});

const autonomousV2 = () => ({...structuredClone(item), platform_fee: {protocol_version:'agent-bounties/autonomous-v2'}});
function currentProjection() {
  const p = projection(); p.items.push(autonomousV2());
  p.items[1].source_id = '0x' + '33'.repeat(20);
  p.items[1].opportunity_id = `canonical:base-mainnet:${p.items[1].source_id}`;
  p.source_statuses[0].item_count = 2;
  p.source_statuses[0].protocol_statuses.splice(1, 0, status('autonomous-v2','complete',1));
  return p;
}
test('four-family inventory retains both autonomous versions without hiding unavailable competition coverage', () => {
  const p = currentProjection(), result = flow.fundedInventory(p);
  assert.equal(result.items.length, 2);
  assert.equal(result.coverage.state, 'partial');
  assert.deepEqual(result.coverage.missing, ['open-competition-v2-beta3']);
  assert.equal(flow.isV2(p.items[1]), false, 'autonomous v2 is not competition v2');
});
test('autonomous v2 exact identity uses its authoritative status without a marketplace scan', async () => {
  const entry = autonomousV2(), p = exactProjection(entry);
  p.items[0].terms_hash = null; // Avoid the separate terms read in this coverage test.
  p.source_statuses[0].protocol_statuses = [status('autonomous-v2','complete',1)];
  const {client,calls} = exactClient(p);
  assert.equal((await client.inspect(entry.opportunity_id)).opportunity_id, entry.opportunity_id);
  assert.equal(calls.length, 1);
});
test('autonomous version metadata cannot borrow another protocol status or count', () => {
  for (const mutate of [
    p => p.source_statuses[0].protocol_statuses.splice(1,1),
    p => p.source_statuses[0].protocol_statuses[1].item_count = 0,
    p => p.source_statuses[0].protocol_statuses[1] = status('autonomous-v2','unavailable',null),
    p => p.source_statuses[0].protocol_statuses[1].protocol = 'autonomous-v1',
    p => p.source_statuses[0].protocol_statuses[1].protocol = 'autonomous-v3',
    p => p.items[1].platform_fee.protocol_version = 'agent-bounties/autonomous-v3',
    p => p.items[1].platform_fee.protocol_version = 'agent-bounties/open-competition-v2-beta3',
    p => delete p.items[1].platform_fee,
    p => p.items[1].opportunity_id = p.items[1].opportunity_id.replace('canonical:', 'open-competition-v2:'),
  ]) { const p = currentProjection(); mutate(p); assert.throws(() => flow.fundedInventory(p)); }
  const p = exactProjection(autonomousV2());
  assert.throws(() => flow.canonicalCoverage(p,item.opportunity_id), /records/);
});
test('unresolved autonomous identity is unavailable and never produces an action', async () => {
  const p = exactProjection(item,'ready_to_earn');
  p.items=[];p.coverage='unavailable';p.degraded=true;
  Object.assign(p.source_statuses[0],{available:false,coverage:'unavailable',item_count:0,protocol_statuses:[status('autonomous','unavailable',null)]});
  const {client,calls}=exactClient(p);
  await assert.rejects(client.prepareAction({action:'solve',opportunity_id:item.opportunity_id}), /unavailable/);
  assert.equal(calls.length,1);
  p.source_statuses[0].protocol_statuses[0] = status('autonomous','complete',0);
  assert.throws(() => flow.canonicalCoverage(p,item.opportunity_id), /malformed/);
});

test('present malformed fee metadata cannot fall back to roomy legacy v1 coverage', () => {
  for (const malformed of [{},{protocol_version:null},'malformed',42,[],{protocol_version:''}]) {
    const p=currentProjection();p.source_statuses[0].protocol_statuses[0].item_count=61;
    p.items[1].platform_fee=malformed;
    assert.throws(()=>flow.fundedInventory(p),/records/);
  }
  for (const legacyFee of [undefined,null,{protocol_version:'agent-bounties/autonomous-v1'}]) {
    const p=projection();p.items[0].platform_fee=legacyFee;
    assert.equal(flow.fundedInventory(p).items.length,1);
  }
});
