"use strict";
const {test} = require("node:test"), assert = require("node:assert/strict");
const auth = require("../site/wallet-session.js");
const address = "0x" + "ab".repeat(20), api="https://api.agentbounties.app";
const id = "11111111-2222-3333-4444-555555555555", token = "abws_"+"12".repeat(32);
function challenge(now=Date.now()) {
 const issued = Math.floor(now/1000)*1000;
 return {challenge_id:id,chain_id:8453,address,expires_at:new Date(issued+300000).toISOString(),message:
 `${new URL(api).host} wants you to sign in with your Ethereum account:\n${address}\n\nSign in to Agent Bounties for 15 minutes. This does not link a website account or authorize a transaction, token approval, or payment.\n\nURI: ${api}/v1/auth/wallet/session\nVersion: 1\nChain ID: 8453\nNonce: ${"33".repeat(32)}\nIssued At: ${new Date(issued).toISOString()}\nExpiration Time: ${new Date(issued+900000).toISOString()}`};
}
function harness(alter=value=>value) {
 const win={}, calls=[], requests=[];let account=address;
 const provider={request:async q=>{calls.push(q);if(q.method==="eth_accounts")return [account];if(q.method==="eth_chainId")return "0x2105";if(q.method==="personal_sign")return "0x"+"55".repeat(65);throw new Error(q.method);}};
 const request=async(path,body)=>{requests.push({path,body});if(path.endsWith("/challenge"))return alter(challenge());return {token,token_type:"Bearer",principal:{kind:"wallet",chain_id:8453,address},expires_at:new Date(Date.now()+899000).toISOString()};};
 return {win,calls,requests,run:()=>auth.authenticate(win,provider,address,request,api),switchWallet:()=>{account="0x"+"cc".repeat(20);}};
}
test("short wallet login sends only a checked ownership message and reuses its in-memory session",async()=>{
 const h=harness();assert.equal(await h.run(),token);assert.equal(await h.run(),token);
 assert.equal(h.calls.filter(v=>v.method==="personal_sign").length,1);
 assert.equal(h.requests.length,2);assert.equal(h.requests[1].body.challenge_id,id);
 const signed=Buffer.from(h.calls.find(v=>v.method==="personal_sign").params[0].slice(2),"hex").toString();
 assert.match(signed,/does not link a website account or authorize a transaction/);
 auth.clear(h.win);await h.run();assert.equal(h.requests.length,4);
});
test("altered SIWE destination, authority, chain, expiry or resources cannot request a signature",async()=>{
 for(const mutate of [v=>({...v,message:v.message.replace("api.agentbounties.app wants","evil.example wants")}),v=>({...v,message:v.message.replace("does not link","does link")}),v=>({...v,chain_id:1}),v=>({...v,message:v.message+"\nResources: arbitrary"}),v=>({...v,expires_at:"invalid"}),v=>({...v,message:v.message.replace("900","901"),address:"0x"+"cc".repeat(20)})]) {
  const h=harness(mutate);await assert.rejects(h.run());assert.equal(h.calls.some(v=>v.method==="personal_sign"),false);
 }
});
test("account switches reject a cached login without granting the other wallet authority",async()=>{
 const h=harness();await h.run();h.switchWallet();await assert.rejects(h.run(),/wallet changed/);assert.equal(h.requests.length,2);
});
