"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { money, publicLink, assess } = require("../site/collaborate/assessment.js");
const fixture = () => JSON.parse(fs.readFileSync(path.join(__dirname, "../examples/free-discovery/fixture.json"), "utf8"));
const now = "2026-10-01T00:01:00Z";

test("exact money formatting preserves base units beyond floating-point precision", () => {
  const value = { amount: "9007199254740993", currency: "USDC", unit: "base_units", decimals: 6 };
  assert.equal(money(value).display, "9007199254.740993 USDC");
  assert.equal(money({ ...value, amount: "0" }).display, "0 USDC");
  assert.equal(money({ ...value, amount: "1" }).display, "0.000001 USDC");
  for (const invalid of [undefined, null, {}, { ...value, amount: 9007199254740993 }, { ...value, amount: "-1" }, { ...value, currency: "ETH" }, { ...value, decimals: 18 }, { ...value, amount: "1e6" }]) {
    assert.deepEqual(money(invalid), { display: "Not reported", positive: null, base_units: null });
  }
});
test("qualified snapshot invites inspection and never authorizes a claim", () => {
  const result = assess(fixture(), now);
  assert.equal(result.mode, "read_only");
  assert.equal(result.items[0].assessment, "inspect_terms");
  assert.equal(result.items[0].refundable_bond.display, "0.01 USDC");
  assert.equal(result.items[0].required_external_spend.display, "1 USDC");
  assert.match(result.items[0].next_action, /does not claim/);
  assert.match(result.evidence_boundary, /Gas, execution costs/);
  assert.equal(result.items[0].wallet_request, undefined);
});
test("unknown cash, unavailable verification and wrong state cannot pass qualification", () => {
  for (const change of [i => delete i.cash_economics, i => i.verification_ready = false, i => i.work_state = "claimed", i => i.payment_state = "unknown", i => i.network = "base-sepolia", i => i.source_type = "unfunded_offchain"]) {
    const feed = fixture(); change(feed.items[0]);
    const item = assess(feed, now).items[0];
    assert.equal(item.assessment, "needs_recheck");
    assert.ok(item.blockers.length);
  }
});
test("empty results and incomplete source state are distinct from ready work", () => {
  const feed = fixture(); feed.items = [];
  assert.equal(assess(feed, now).sample_size, 0);
  for (const unavailable of [true, undefined, "false"]) {
    const feed = fixture(); feed.degraded = unavailable;
    assert.equal(assess(feed, now).items[0].assessment, "needs_recheck");
  }
});
test("stale snapshots and impossible future clocks require a fresh read", () => {
  assert.equal(assess(fixture(), "2026-10-01T00:06:00Z").stale, true);
  assert.equal(assess(fixture(), "2026-09-30T23:58:00Z").items[0].assessment, "needs_recheck");
  const feed = fixture(); feed.generated_at = "invalid";
  assert.throws(() => assess(feed, now), /format/);
});
test("rejects foreign origins, unsafe URLs and credential-bearing links", () => {
  for (const value of ["javascript:alert(1)", "data:text/html,unsafe", "https://agentbounties.app.evil.example/", "https://evil.example/", "https://user:pass@agentbounties.app/", "http://agentbounties.app/", "https://github.com/other/repo/issues/1"]) assert.equal(publicLink(value), null);
  assert.equal(publicLink("https://github.com/NSPG13/agent-bounties/issues/648"), "https://github.com/NSPG13/agent-bounties/issues/648");
  const feed = fixture(); feed.items[0].public_url = "javascript:alert(1)";
  assert.equal(assess(feed, now).items[0].public_url, null);
  assert.equal(assess(feed, now).items[0].assessment, "needs_recheck");
});
test("bounds the sample and rejects malformed response contracts", () => {
  const feed = fixture(); feed.items = Array(20).fill(feed.items[0]);
  assert.equal(assess(feed, now).items.length, 5);
  for (const invalid of [null, {}, { ...feed, network: "base-sepolia" }, { ...feed, schema_version: "old" }, { ...feed, items: [null] }]) assert.throws(() => assess(invalid, now));
});
