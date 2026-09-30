"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const create = require("../site/google-ads-measurement.js");
const acquisition = `aba1_${"a".repeat(64)}.${"b".repeat(64)}`;
const operation = "f441abca-8ef3-4027-8059-e655c258d939";
const handoff = "cb6edbaa-c36a-4b41-a7e9-90f48a225923";
const consentKey = "agent-bounties.ads-consent.v1";
function storage() { const map=new Map(); return { getItem:k=>map.get(k)??null,setItem:(k,v)=>map.set(k,v),removeItem:k=>map.delete(k),map }; }
function page(options={}) {
  const requests=[], listeners={}, local=options.local||storage();
  const win={ location:new URL(options.url||"https://agentbounties.app/bug-fix.html?gclid=fixture_click_123&secret=not-forwarded"),localStorage:local,navigator:options.privacy||{},crypto:crypto.webcrypto,AbortController,setTimeout,clearTimeout,CustomEvent:class {constructor(type,x){this.type=type;this.detail=x.detail}},dispatchEvent(){},addEventListener:(key,fn)=>listeners[key]=fn,
    history:{replaceState(_a,_b,url){win.location=new URL(url)}},document:{readyState:"loading",addEventListener(){},querySelectorAll(){return []}},
    async fetch(url,req){requests.push({url,body:JSON.parse(req.body)});if(options.fail)throw Error("offline");return {ok:true,json:async()=>url.endsWith("website-acquisitions")?{acquisition,expires_at:new Date(Date.now()+86400000).toISOString()}:{acquisition,handoff,revoked:true}}}
  };
  return {api:create(win),win,requests,local,listeners};
}
test("no collection before separate consent and no raw identifiers in shared URL or storage",async()=>{
  const p=page(); assert.equal(p.requests.length,0);assert.equal(p.win.location.searchParams.has("gclid"),false);
  assert.equal(p.local.map.size,0);await p.api.allow();assert.equal(p.requests.length,1);
  assert.equal(p.requests[0].body.consent_version,"google-ads-outcomes-v1");
  assert.ok(!JSON.stringify([...p.local.map]).includes("fixture_click_123"));
});
test("old Google Analytics permission is not ad measurement permission",async()=>{
  const local=storage();local.setItem("agent-bounties.analytics.google-consent.v1","granted");const p=page({local});
  await p.api.prepare(operation);assert.equal(p.requests.length,0);
});
test("privacy controls and denial prevent capture",async()=>{
  for(const privacy of [{globalPrivacyControl:true},{doNotTrack:"1"}]){const p=page({privacy});await p.api.allow();await p.api.prepare(operation);assert.equal(p.requests.length,0);}
  const p=page();await p.api.withdraw();await p.api.prepare(operation);assert.equal(p.requests.length,0);
});
test("saved opaque handoff resumes on another browser and never grants wallet authority",async()=>{
  const p=page();await p.api.allow();const pair=await p.api.prepare(operation);assert.deepEqual(pair,{acquisition,handoff});
  const other=page({url:"https://agentbounties.app/post.html"});other.api.restore(operation,pair);assert.deepEqual(other.api.current(operation),pair);
  assert.deepEqual(await other.api.prepare(operation),pair);assert.equal(other.requests[0].body.operation_id,operation);
  assert.ok(!JSON.stringify(other.requests).includes("fixture_click_123"));
});
test("repeated parallel capture and handoff retries preserve the original acquisition",async()=>{
  const p=page();await Promise.all([p.api.allow(),p.api.allow()]);assert.equal(p.requests.filter(x=>x.url.endsWith("website-acquisitions")).length,1);
  const first=await p.api.prepare(operation);const repeated=await p.api.prepare(operation);assert.deepEqual(first,repeated);assert.equal(p.requests.length,2);
});
test("existing first touch is carried separately from the later Google click",async()=>{
  const local=storage();local.setItem("bountyboard.analytics.attribution.v1",JSON.stringify({source:"github",campaign:"readme",expires_at:Date.now()+100000}));
  const p=page({local});await p.api.allow();assert.equal(p.requests[0].body.first_touch_source,"github");
});
test("withdrawal clears local identifiers and queues server deletion on an outage",async()=>{
  const p=page();await p.api.allow();await p.api.prepare(operation);await p.api.withdraw();assert.equal(p.api.current(operation),null);assert.equal(p.requests.at(-1).url.endsWith("/revoke"),true);
  const local=storage();local.setItem("agent-bounties.ads-acquisition.v1",JSON.stringify({acquisition,expires_at:Date.now()+10000}));
  const fail=page({local,fail:true,url:"https://agentbounties.app/privacy.html"});await fail.api.withdraw();assert.deepEqual(JSON.parse(local.getItem("agent-bounties.ads-withdrawal.v1")),[acquisition]);
});
test("withdrawal revokes restored cross-browser handoffs too",async()=>{
  const p=page({url:"https://agentbounties.app/privacy.html"});p.api.restore(operation,{acquisition,handoff});
  await p.api.withdraw();assert.equal(p.requests.at(-1).body.acquisition,acquisition);assert.equal(p.api.current(operation),null);
});
test("missing, ambiguous and malformed click IDs never get exported",async()=>{
  for(const suffix of ["", "?gclid=short", "?gclid=fixture_click_123&wbraid=fixture_click_456", "?gclid=fixture_click_123&gclid=fixture_click_456"]){const p=page({url:"https://agentbounties.app/bug-fix.html"+suffix});await p.api.allow();assert.equal(p.requests.length,0);}
});
test("outage returns no fabricated attribution and never throws into posting",async()=>{
  const p=page({fail:true});await p.api.allow();assert.equal(await p.api.prepare(operation),null);assert.equal(p.api.current(operation),null);
});
test("storage blocked keeps ad measurement off",async()=>{
  const local={getItem(){throw Error()},setItem(){throw Error()},removeItem(){throw Error()}};
  const p=page({local});await p.api.allow();assert.equal(p.requests.length,0);
});
