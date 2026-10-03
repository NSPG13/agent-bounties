"use strict";
const assert = require("node:assert/strict");
const test = require("node:test");
const proof = require("../site/payout-proof.js");
const saved = require("./fixtures/historical-payout-proof.json");
const fixture = () => structuredClone(saved);

test("all historical factories reconcile captured57settlements/65payouts and daily totals", () => {
  const { platform, pages, expected } = fixture();
  const result = proof.audit(platform, pages);
  assert.equal(result.status, "ready");
  for (const [field, value] of Object.entries(expected)) assert.equal(result.summary[field], value);
  assert.equal(proof.formatAmount(result.summary.total_base_units), "73.065 USDC");
  assert.equal(result.chain_freshness_verified, false);
  assert.equal(platform.coverage.status, "partial");
  assert.equal(result.excluded_summary, null);
  const factories = new Set(result.rows.filter(row => row.protocol === "open-competition-v2").map(row => row.factory_contract));
  assert.equal(factories.size, 2);
});
test("omitted old-factory rows remain a mismatch and source errors are not zero", () => {
  const { platform, pages } = fixture();
  pages[0].records = pages[0].records.filter(row => row.factory_contract !== "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4");
  pages[0].total_rows = pages[0].records.length;
  assert.equal(proof.audit(platform, pages).status, "partial");
  assert.equal(proof.audit(platform, null).status, "unavailable");
});
test("duplicates, changed scope, policy exclusions and partial pages fail closed", () => {
  for (const mutation of [
    (p, pages) => { pages[0].records[1] = pages[0].records[0]; },
    (p, pages) => { pages[0].snapshot = `sha256:${"b".repeat(64)}`; },
    (p, pages) => { pages[0].policy_hash = `sha256:${"b".repeat(64)}`; },
    (p, pages) => { pages[0].ended_at = "2026-09-30T00:00:00Z"; },
    (p, pages) => { pages[0].records[0].total_base_units = "2"; },
    (p, pages) => { pages[0].records[0].solver_base_units = 990000; },
    (p, pages) => { pages[0].records[0].network = "base-sepolia"; },
    (p, pages) => { pages[0].excluded_bounty_contracts.push(pages[0].records[0].contract_address); },
    (p, pages) => { pages[0].complete = false; },
    (p, pages) => { pages[0].offset = 1; },
    (p, pages) => { pages[0].total_rows = 5001; },
    (p, pages) => { p.daily[0].payout.usdc_base_units = null; },
    (p, pages) => { pages[0].records[0].tx_hash = "javascript:alert(1)"; },
  ]) {
    const { platform, pages } = fixture(); mutation(platform, pages);
    assert.equal(proof.audit(platform, pages).status, "unavailable");
  }
});
test("integer arithmetic and display preserve values beyond safe JSON numbers", () => {
  const { platform, pages } = fixture();
  const row = pages[0].records[0];
  row.solver_base_units = "9007199254740993"; row.verifier_base_units = "0";
  row.total_base_units = row.solver_base_units;
  pages[0].records = [row]; pages[0].total_rows = 1;
  const payout = platform.marketplace_payout_volume;
  for (const key of ["selected", "selected_solver_pay"]) payout[key].usdc_base_units = row.solver_base_units;
  for (const key of ["selected_verifier_pay", "selected_keeper_pay", "selected_completion_bonus"]) payout[key].usdc_base_units = "0";
  payout.selected_settled_rounds = 1;
  platform.daily = [{ day: row.occurred_at.slice(0, 10), payout: { usdc_base_units: row.total_base_units }, settled_rounds: 1 }];
  assert.equal(proof.audit(platform, pages).status, "ready");
  assert.equal(proof.formatAmount(row.total_base_units), "9,007,199,254.740993 USDC");
  platform.daily[0].payout.usdc_base_units = "9007199254740992";
  assert.equal(proof.audit(platform, pages).status, "partial");
});
test("only an explicitly complete empty selection can reconcile to zero", () => {
  const { platform, pages } = fixture();
  pages[0].records = []; pages[0].total_rows = 0;
  for (const key of ["selected", "selected_solver_pay", "selected_verifier_pay", "selected_keeper_pay", "selected_completion_bonus"]) platform.marketplace_payout_volume[key].usdc_base_units = "0";
  platform.marketplace_payout_volume.selected_settled_rounds = 0; platform.daily = [];
  assert.equal(proof.audit(platform, pages).status, "ready");
  pages[0].complete = false;
  assert.equal(proof.audit(platform, pages).status, "unavailable");
});
test("multi-page replay requires continuous offsets and complete selection", () => {
  const { platform, pages } = fixture();
  const second = structuredClone(pages[0]), cursor = `v1:30:sha256:${"c".repeat(64)}`;
  second.records = second.records.slice(30); second.offset = 30;
  pages[0].records = pages[0].records.slice(0, 30); pages[0].complete = false; pages[0].next_cursor = cursor;
  pages.push(second);
  assert.equal(proof.audit(platform, pages).status, "ready");
  pages[1].offset = 29;
  assert.equal(proof.audit(platform, pages).status, "unavailable");
});
test("bounded transport rejects redirects/status/oversize and cancels the stream", async () => {
  const url = "https://api.agentbounties.app/v1/metrics/platform?period=lifetime";
  let options;
  const response = await proof.requestJson(async (_url, opts) => { options = opts; return new Response('{}'); }, url);
  assert.deepEqual(response, {}); assert.equal(options.redirect, "error"); assert.equal(options.credentials, "omit");
  await assert.rejects(proof.requestJson(async () => new Response("changed", { status: 409 }), url), /restart/);
  await assert.rejects(proof.requestJson(async () => new Response("x".repeat(262145)), url), /byte limit/);
  await assert.rejects(proof.requestJson(async () => assert.fail(), "https://attacker.invalid/proof"), /Unapproved/);
});
test("reader requests fixed API scope and returns a reproducible JSON report", async () => {
  const { platform, pages } = fixture(); let calls = 0;
  const result = await proof.read(async (url) => {
    calls += 1; const parsed = new URL(url);
    if (calls === 1) return new Response(JSON.stringify(platform));
    assert.equal(parsed.pathname, "/v1/metrics/platform/payouts");
    assert.equal(parsed.searchParams.get("snapshot"), platform.payout_proof.snapshot);
    assert.equal(parsed.searchParams.get("as_of"), platform.window.ended_at);
    return new Response(JSON.stringify(pages[0]));
  });
  assert.equal(calls, 2); assert.equal(result.audit.status, "ready"); assert.doesNotThrow(() => JSON.stringify(result));
});
