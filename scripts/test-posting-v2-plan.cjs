"use strict";
const { test } = require("node:test"), assert = require("node:assert/strict");
const fs = require("node:fs"), path = require("node:path"), vm = require("node:vm");
const root = path.resolve(__dirname, ".."), context = vm.createContext({ window: {} });
for (const name of ["evm.js", "posting-v2-plan.js"])
  vm.runInContext(fs.readFileSync(path.join(root, "site", name), "utf8"), context);
const api = context.window.AgentBountiesPostingV2Plan;
const fixture = JSON.parse(fs.readFileSync(path.join(root, "crates/chain-base/tests/fixtures/autonomous-v2-loop.json")));
const data = fixture.create_bounty_calldata.paid_after_reject;
const hex = i => `0x${data.slice(10 + i * 64, 74 + i * 64)}`;
const num = i => Number(BigInt(hex(i)));
const money = amount => ({ amount, currency: "usdc" });
const protocol = { protocol_version: api.VERSION, chain_id: fixture.chain_id, factory: fixture.factory,
  implementation: fixture.implementation, native_usdc: fixture.settlement_token,
  platform_fee_bps: fixture.platform_fee_bps, platform_fee_recipient: fixture.platform_fee_recipient };
const create = { creator: fixture.creator, solver_reward: money(num(0)), verifier_reward: money(num(1)),
  terms_hash: hex(2), policy_hash: hex(3), acceptance_criteria_hash: hex(4), benchmark_hash: hex(5), evidence_schema_hash: hex(6),
  funding_deadline: num(7), claim_window_seconds: num(8), verification_window_seconds: num(9), verification_mode: "signed_quorum",
  verifier_module: null, verifier_reward_recipient: null, threshold: num(13), initial_funding: money(num(17)),
  creation_nonce: hex(18), verifiers: Array.from({ length: num(19) }, (_, i) => `0x${hex(20 + i).slice(-40)}`) };
function plan() {
  return { protocol_version: api.VERSION, network: { chain_id: fixture.chain_id, native_usdc_token_address: fixture.settlement_token },
    factory_contract: fixture.factory, implementation_contract: fixture.implementation,
    bounty_id: fixture.logs[0].topics[1], predicted_bounty_contract: fixture.bounties.paid_after_reject,
    quote: { protocol_version: api.VERSION, solver_reward: "1000000", verifier_reward: "100000", claim_bond: "100000",
      platform_fee_bps: 750, platform_fee: "75000", platform_fee_recipient: fixture.platform_fee_recipient, target_amount: "1175000" },
    wallet_calls: [
      { from: fixture.creator, to: fixture.settlement_token, value_wei: 0,
        data: "0x095ea7b3" + fixture.factory.slice(2).padStart(64, "0") + BigInt(fixture.target_amount).toString(16).padStart(64, "0") },
      { from: fixture.creator, to: fixture.factory, value_wei: 0, data },
    ] };
}
test("unsigned v2 encoding and CREATE2 identity match the independent Anvil/Foundry fixture", () => {
  const actual = api.encode(create, protocol);
  assert.equal(actual.data, data);
  assert.equal(actual.bounty, fixture.bounties.paid_after_reject);
  assert.equal(actual.bountyId, fixture.logs[0].topics[1]);
  assert.equal(api.validatePlan(plan(), create, protocol).total, 1175000n);
});
test("fees round up in micro-USDC, without changing solver or verifier rewards", () => {
  const actual = api.economics(2000001n, 100000n, protocol);
  assert.equal(actual.fee, 150001n); assert.equal(actual.total, 2250002n);
  assert.throws(() => api.economics(Number.MAX_SAFE_INTEGER + 1, 1, protocol));
  assert.throws(() => api.economics("1e6", 1, protocol));
});
test("flat API response is required; an invented nested plan cannot select wallet calls", () => {
  assert.throws(() => api.validatePlan({ plan: plan(), quote: plan().quote }, create, protocol));
});
test("every money, network and immutable address mismatch stops before wallet use", () => {
  for (const key of ["solver_reward", "verifier_reward", "claim_bond", "platform_fee", "target_amount"]) {
    const p = plan(); p.quote[key] = String(BigInt(p.quote[key]) + 1n);
    assert.throws(() => api.validatePlan(p, create, protocol), key);
  }
  for (const key of ["factory_contract", "implementation_contract", "predicted_bounty_contract", "bounty_id"]) {
    const p = plan(); p[key] = p[key].slice(0, -1) + (p[key].endsWith("0") ? "1" : "0");
    assert.throws(() => api.validatePlan(p, create, protocol), key);
  }
  const p = plan(); p.network.chain_id = 1; assert.throws(() => api.validatePlan(p, create, protocol));
  const fee = plan(); fee.quote.platform_fee_bps = 0; assert.throws(() => api.validatePlan(fee, create, protocol));
});
test("all exact wallet call bytes, order, sender and zero native value are bound", () => {
  for (const index of [0, 1]) for (const key of ["from", "to", "data", "value_wei"]) {
    const p = plan(); p.wallet_calls[index][key] = key === "value_wei" ? 1 : "0x00";
    assert.throws(() => api.validatePlan(p, create, protocol), `${index}:${key}`);
  }
  const p = plan(); p.wallet_calls.reverse(); assert.throws(() => api.validatePlan(p, create, protocol));
  const extra = plan(); extra.wallet_calls.push(extra.wallet_calls[0]); assert.throws(() => api.validatePlan(extra, create, protocol));
});
test("module, duplicate quorum, partial gates and underfunding cannot enter this route", () => {
  for (const patch of [{ verification_mode: "deterministic_module" }, { verifiers: [fixture.creator, fixture.creator] },
    { threshold: 0 }, { claim_eligibility_registry: fixture.factory }, { claim_eligibility_source: create.terms_hash },
    { initial_funding: money(1100000) }, { funding_deadline: 0 }]) {
    assert.throws(() => api.validatePlan(plan(), { ...create, ...patch }, protocol));
  }
});
test("validated calls are detached from later API mutations and exclude authorization payloads", () => {
  const p = plan(); p.eip3009_authorization = { untrusted: true };
  const result = api.validatePlan(p, create, protocol); p.wallet_calls[0].data = "0x00";
  assert.equal(result.calls[0].data, plan().wallet_calls[0].data);
  assert.equal(result.eip3009_authorization, undefined);
});
