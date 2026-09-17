import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { createPostingRequests } from "./src/posting-requests.js";

const wallet = "0x" + "22".repeat(20), bounty = "0x" + "33".repeat(20), factory = "0x" + "44".repeat(20);
const usdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const word = n => BigInt(n).toString(16).padStart(64, "0");
function fixture() {
  const args = Array(26).fill(word(0));
  args[0] = wallet.slice(2).padStart(64, "0"); args[8] = word(4102444800); args[16] = word(2010000); args[19] = word(4102444800);
  const tx = { from: wallet, to: factory, data: "0x61407894" + args.join(""), value: "0x0" };
  return { method: "eth_sendTransaction", params: [tx], agentBountiesPostingContext: {
    chainId: 8453, usdcAddress: usdc, creatorAddress: wallet, factoryAddress: factory,
    bountyAddress: bounty, fundingUsdcUnits: 2010000n, validatedCalls: [tx],
  } };
}
function harness(options = {}) {
  const env = { window: {}, setTimeout, clearTimeout, AbortController };
  vm.runInNewContext(readFileSync(new URL("../../site/funding-readiness.js", import.meta.url), "utf8"), env);
  const calls = [], reviews = [];
  let address = wallet;
  const request = createPostingRequests({
    currentAddress: async () => address,
    readiness: () => env.window.AgentBountiesFundingReadiness,
    confirm: async review => { reviews.push(review); await options.confirm?.(review); if (options.changeWallet) address = bounty; },
    getProvider: async () => ({ request: async input => {
      calls.push(input);
      if (input.method === "eth_sendTransaction") {
        if (options.lostReply) throw Object.assign(new Error("lost"), { code: 4001 });
        return "0x" + "ab".repeat(32);
      }
      if (input.method === "eth_call") {
        if (options.noFee) throw new Error("fee unavailable");
        return "0x64";
      }
      return { eth_chainId: "0x2105", eth_blockNumber: "0x100", eth_gasPrice: "0x3", eth_estimateGas: "0x5208", eth_getBalance: options.balance || "0x2386f26fc10000" }[input.method];
    } }),
  });
  return { request, calls, reviews, setAddress: value => { address = value; }, sends: () => calls.filter(call => call.method === "eth_sendTransaction") };
}
test("embedded posting reviews exact Base amount, recipient, expiry and full network fee before sending once", async () => {
  const h = harness(), input = fixture();
  assert.equal(await h.request(input), "0x" + "ab".repeat(32));
  assert.match(h.reviews[0].summary, /2.01 Base USDC/);
  assert.match(h.reviews[0].summary, new RegExp(bounty));
  assert.match(h.reviews[0].summary, /authorization expires/);
  assert.match(h.reviews[0].summary, /Estimated network fee:.*ETH, paid by you/);
  const details = Object.fromEntries(h.reviews[0].details);
  assert.equal(details.Network, "Base (chain 8453)");
  assert.equal(details.Amount, "2.01 Base USDC");
  assert.equal(details.Recipient, bounty);
  assert.equal(details["Factory contract"], factory);
  assert.ok(Date.parse(details.Expiry) > Date.now());
  assert.match(details["Estimated network fee"], /ETH \(paid by you\)/);
  assert.equal(h.sends().length, 1);
  assert.equal(h.sends()[0].params[0].chainId, "0x2105");
  await assert.rejects(h.request(input), /already sent/);
  assert.equal(h.sends().length, 1);
});
test("cancel, wrong chain, changed address/call, extra transaction fields and unavailable fees send nothing", async () => {
  for (const mutate of [input => input.params[0].chainId = "0x1", input => input.params[0].from = bounty,
    input => input.params[0].value = "0x1", input => input.params[0].authorizationList = [],
    input => input.params[0].to = bounty, input => delete input.agentBountiesPostingContext]) {
    const h = harness(), input = fixture(); mutate(input);
    await assert.rejects(h.request(input)); assert.equal(h.sends().length, 0);
  }
  for (const options of [{ noFee: true }, { balance: "0x1" }, { changeWallet: true }, { confirm: () => { throw Object.assign(new Error("cancelled"), { code: 4001 }); } }]) {
    const h = harness(options); await assert.rejects(h.request(fixture())); assert.equal(h.sends().length, 0);
  }
});
test("the durable checkpoint runs after human approval and before sending; storage failure and identity changes fail closed", async () => {
  let recorded = 0;
  const cancelled = harness({confirm: () => { throw Object.assign(new Error('cancelled'),{code:4001}); }});
  await assert.rejects(cancelled.request({...fixture(),agentBountiesBeforeSubmit:async()=>{recorded++;}}));
  assert.equal(recorded,0);assert.equal(cancelled.sends().length,0);
  const success = harness();
  await success.request({...fixture(),agentBountiesBeforeSubmit:async()=>{assert.equal(success.reviews.length,1);assert.equal(success.sends().length,0);recorded++;}});
  assert.equal(recorded,1);assert.equal(success.sends().length,1);
  const unavailable = harness();
  await assert.rejects(unavailable.request({...fixture(),agentBountiesBeforeSubmit:async()=>{throw new Error('storage unavailable');}}),/storage unavailable/);
  assert.equal(unavailable.sends().length,0);
  const changed = harness();
  await assert.rejects(changed.request({...fixture(),agentBountiesBeforeSubmit:async()=>changed.setAddress(bounty)}),/wallet changed while saving/);
  assert.equal(changed.sends().length,0);
});
test("a lost SDK response cannot masquerade as a safe rejection or trigger another send", async () => {
  const h = harness({ lostReply: true }), input = fixture();
  await assert.rejects(h.request(input), error => error.code === -32000 && /may have been submitted/.test(error.message));
  await assert.rejects(h.request(input), /already sent/);
  assert.equal(h.sends().length, 1);
});
test("the funding signature binds amount, token, recipient, nonce and expiry before the SDK can sign", async () => {
  const nonce = "0x" + "ef".repeat(32);
  const typedData = {
    types: {
      EIP712Domain: [{name:"name",type:"string"},{name:"version",type:"string"},{name:"chainId",type:"uint256"},{name:"verifyingContract",type:"address"}],
      TransferWithAuthorization: [{name:"from",type:"address"},{name:"to",type:"address"},{name:"value",type:"uint256"},{name:"validAfter",type:"uint256"},{name:"validBefore",type:"uint256"},{name:"nonce",type:"bytes32"}],
    },
    domain: {name:"USD Coin",version:"2",chainId:8453,verifyingContract:usdc}, primaryType:"TransferWithAuthorization",
    message: {from:wallet,to:bounty,value:"2010000",validAfter:"0",validBefore:"4102444800",nonce},
  };
  const input = {method:"eth_signTypedData_v4",params:[wallet,JSON.stringify(typedData)],
    agentBountiesPostingContext:{...fixture().agentBountiesPostingContext,creationNonce:nonce,fundingDeadline:4102444800}};
  const h = harness(); await h.request(input);
  assert.match(h.reviews[0].summary,/Authorize 2.01 USDC on Base/);
  assert.match(h.reviews[0].summary,/signature costs no gas/);
  const details = Object.fromEntries(h.reviews[0].details);
  assert.equal(details.Network, "Base (chain 8453)");
  assert.equal(details.Amount, "2.01 Base USDC");
  assert.equal(details.Recipient, bounty);
  assert.ok(Date.parse(details.Expiry) > Date.now());
  assert.equal(details["Signature fee"], "No network fee");
  assert.equal(h.calls.filter(call=>call.method==='eth_signTypedData_v4').length,1);
  for (const field of ['to','value','nonce','validBefore']) {
    const changed = structuredClone(typedData);
    changed.message[field] = {to:wallet,value:'2010001',nonce:'0x'+'ab'.repeat(32),validBefore:'4102444801'}[field];
    const other = harness();
    await assert.rejects(other.request({...input,params:[wallet,JSON.stringify(changed)]}));
    assert.equal(other.reviews.length,0);
    assert.equal(other.calls.length,0);
  }
});
test("concurrent requests cannot borrow confirmation and caller mutation cannot change the reviewed transaction", async () => {
  let release, opened;
  const ready = new Promise(resolve => { opened = resolve; });
  const h = harness({ confirm: () => new Promise(resolve => { release = resolve; opened(); }) });
  const input = fixture(), pending = h.request(input);
  await ready;
  input.params[0].to = bounty;
  await assert.rejects(h.request(fixture()), error => error.code === -32002);
  release(); await pending;
  assert.equal(h.sends()[0].params[0].to, factory);
});
