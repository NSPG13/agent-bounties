import test from 'node:test';
import assert from 'node:assert/strict';
import algosdk from 'algosdk';
import { createApp } from '../app.mjs';
import { readConfig } from '../config.mjs';
import { buildReport, parseFilters, createFeedLoader } from '../report.mjs';

const payTo = algosdk.generateAccount().addr.toString();
const payer = algosdk.generateAccount().addr.toString();
const config = readConfig({ ALGORAND_PAY_TO: payTo, PUBLIC_ORIGIN: 'https://reports.example' });
const url = `${config.origin}/v1/opportunity-report`;
const fixture = { status: 'claimable', terms_valid: true, verification_ready: true, bounty_contract: '0x'+'a'.repeat(40), solver_reward: '2000000', claim_bond: '100000', funded_amount: '2100000', target_amount: '2100000', required_external_spend: '0', terms: { document: { title: 'Fix a parser', goal: 'Make parsing deterministic', contract_terms: {}, benchmark: {} } }, events: [{kind:'bounty_became_claimable',tx_hash:'0x'+'b'.repeat(64),block_number:1}] };

function setup({ valid = true, success = true, failSource = false } = {}) {
  const calls = { verify: 0, settle: 0, feed: 0 };
  const facilitator = {
    getSupported: async () => ({ kinds: [{x402Version:2,scheme:'exact',network:config.network,extra:{feePayer:payer}}], extensions:[], signers:{} }),
    verify: async (payload, requirements) => { calls.verify++; calls.requirements=requirements; calls.payload=payload; return {isValid:valid,payer,...(!valid?{invalidReason:'invalid_signature'}:{})}; },
    settle: async () => { calls.settle++; return {success, transaction:success?'A'.repeat(52):'',network:config.network,payer,...(!success?{errorReason:'settlement_failed'}:{})}; },
  };
  const app = createApp(config, { facilitator, loadFeed: async () => { calls.feed++; if(failSource)throw new Error('down');return {rows:[structuredClone(fixture)],observedAt:'2026-10-01T06:00:00.000Z'}; } });
  return { app, calls };
}
async function challenge(app) {
  const r=await app.request(url); assert.equal(r.status,402);
  return JSON.parse(Buffer.from(r.headers.get('payment-required'),'base64').toString());
}
function signed(ch) { return {'payment-signature':Buffer.from(JSON.stringify({x402Version:2,resource:ch.resource,accepted:ch.accepts[0],payload:{paymentGroup:[],paymentIndex:0},extensions:ch.extensions})).toString('base64')}; }

test('unpaid challenge is Algorand USDC through the supported facilitator, tagged and discoverable',async()=>{
  const {app,calls}=setup(); const ch=await challenge(app);
  assert.equal(ch.accepts[0].network,config.network);assert.equal(ch.accepts[0].asset,'31566704');
  assert.equal(ch.accepts[0].amount,'10000');assert.equal(ch.accepts[0].payTo,payTo);
  assert.equal(ch.accepts[0].extra.tag,'x402-global-challenge');assert.equal(ch.accepts[0].extra.feePayer,payer);
  assert.ok(ch.extensions.bazaar);assert.equal(ch.resource.url,url);
  assert.deepEqual([calls.verify,calls.settle,calls.feed],[0,0,0]);
});
test('verified payment is settled before returning the report and receipt header',async()=>{
  const {app,calls}=setup(); const ch=await challenge(app);
  const r=await app.request(url,{headers:signed(ch)});
  assert.equal(r.status,200);assert.ok(r.headers.get('payment-response'));
  assert.equal((await r.json()).matchingCount,1);assert.deepEqual([calls.verify,calls.settle,calls.feed],[1,1,1]);
  assert.equal(calls.payload.extensions.bazaar.info.input.method,'GET');assert.equal(calls.requirements.extra.tag,'x402-global-challenge');
});
test('invalid payment cannot read the report or settle',async()=>{
  const {app,calls}=setup({valid:false});const r=await app.request(url,{headers:signed(await challenge(app))});
  assert.equal(r.status,402);assert.equal(calls.settle,0);assert.equal(calls.feed,0);
});
test('failed settlement never returns a paid report',async()=>{
  const {app,calls}=setup({success:false});const r=await app.request(url,{headers:signed(await challenge(app))});
  assert.ok(r.status>=400);assert.equal(calls.settle,1);assert.equal((await r.text()).includes('Fix a parser'),false);
});
test('source failure does not settle a verified authorization',async()=>{
  const {app,calls}=setup({failSource:true});const r=await app.request(url,{headers:signed(await challenge(app))});
  assert.equal(r.status,503);assert.equal(calls.settle,0);
});
test('malformed payment, methods, duplicated inputs, and alternate routes cannot bypass payment',async()=>{
  const {app,calls}=setup();
  for(const path of ['/v1/opportunity-report/extra','/v1/opportunity-report%2Fextra']){assert.equal((await app.request(config.origin+path)).status,404);}
  assert.ok((await app.request(url,{headers:{'payment-signature':'not-json'}})).status>=400);
  assert.equal((await app.request(url,{method:'POST'})).status,405);
  assert.equal((await app.request(url+'?limit=10&limit=50')).status,400);
  assert.equal(calls.settle,0);assert.equal(calls.feed,0);
});
test('report excludes unfunded, invalid, unavailable, expired, and over-budget work',()=>{
  const rows=[fixture,...[
    {funded_amount:'1'},{terms_valid:false},{verification_ready:false},{status:'settled'},{claim_bond:'9999999'},
    {events:[]},{terms:{document:{...fixture.terms.document,benchmark:{delivery_deadline:1}}}},
  ].map(p=>({...structuredClone(fixture),...p}))];
  const r=buildReport(rows,parseFilters(url),'2026-10-01T00:00:00.000Z');assert.equal(r.matchingCount,1);
  assert.equal(r.opportunities[0].rewardUSDC,'2.000000');assert.equal(r.opportunities[0].bondUSDC,'0.100000');
  assert.equal(buildReport(rows,parseFilters(url+'?q=absent')).matchingCount,0);
});
test('bad amounts fail closed and query cannot redirect the source',()=>{
  assert.throws(()=>buildReport([{...fixture,solver_reward:'NaN'}],parseFilters(url)));
  assert.throws(()=>parseFilters(url+'?url=http://localhost'));
  assert.throws(()=>readConfig({ALGORAND_PAY_TO:payTo,ALGORAND_NETWORK:'mainnet',PUBLIC_ORIGIN:'http://evil.example'}));
  assert.throws(()=>readConfig({ALGORAND_PAY_TO:'not-an-address'}));
});
test('upstream errors never become a successful empty report',async()=>{
  const load=createFeedLoader(async()=>new Response('unavailable',{status:503}));await assert.rejects(load);
});
