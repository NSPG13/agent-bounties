"use strict";
const test = require("node:test"), assert = require("node:assert/strict");
const { fetchAuthSession } = require("../site/solarpunk-home.js");
function browser(fetch) {
  let alarm, cleared = false;
  return { location: new URL("https://agentizable.ai/#login"), AbortController, fetch,
    setTimeout: (fn, ms) => { assert.equal(ms,10000);alarm=fn;return 1; },
    clearTimeout: id => { assert.equal(id,1);cleared=true; },
    expire: () => alarm(), cleared: () => cleared };
}
test("fresh session loads only server-confirmed provider availability with credentials", async () => {
  const payload={authenticated:false,user:null,providers:{github:true,google:true,microsoft:false}};
  const win=browser(async (url,options)=>{
    assert.equal(url,"https://api.agentizable.ai/v1/site-auth/session");
    assert.equal(options.credentials,"include");assert.equal(options.cache,"no-store");
    assert.equal(options.headers.Accept,"application/json");assert.ok(options.signal);
    return {ok:true,json:async()=>payload};
  });
  assert.deepEqual(await fetchAuthSession(win),payload);assert.ok(win.cleared());
});
test("an outage is retryable and never becomes an empty signed-out session", async () => {
  let calls=0;const payload={authenticated:false,providers:{github:true}};
  const win=browser(async()=>++calls===1?{ok:false,status:503}:{ok:true,json:async()=>payload});
  await assert.rejects(fetchAuthSession(win),/unavailable/);assert.ok(win.cleared());
  assert.deepEqual(await fetchAuthSession(win),payload);assert.equal(calls,2);
});
test("unreadable and malformed session responses cannot enable providers",async()=>{
  for (const payload of [null,{}, {authenticated:false,providers:[]}, {authenticated:false,providers:{google:"true"}}]) {
    const win=browser(async()=>({ok:true,json:async()=>payload}));
    await assert.rejects(fetchAuthSession(win),/Invalid/);assert.ok(win.cleared());
  }
  const win=browser(async()=>({ok:true,json:async()=>{throw new Error("HTML response");}}));
  await assert.rejects(fetchAuthSession(win));assert.ok(win.cleared());
});
test("a stalled session request aborts after the bounded timeout and clears its timer", async () => {
  const win=browser((_url,{signal})=>new Promise((_resolve,reject)=>signal.addEventListener("abort",()=>reject(new Error("Aborted")))));
  const pending=fetchAuthSession(win);win.expire();await assert.rejects(pending,/Aborted/);assert.ok(win.cleared());
});
