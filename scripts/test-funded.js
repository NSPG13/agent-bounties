"use strict";
const test = require("node:test"), assert = require("node:assert/strict");
const { confirmedItem, formatAmount } = require("../site/funded.js");
const { item, contract } = require("./fixtures/funded-bounty.cjs");

test("receipt requires exact canonical creation, full funding and claimability evidence", () => {
  assert.ok(confirmedItem([item], contract));
  for (const kind of ["canonical_bounty_created", "funding_added", "bounty_became_claimable"]) {
    assert.equal(confirmedItem([{ ...item, events: item.events.filter(e => e.kind !== kind) }], contract), null, kind);
  }
  for (const changes of [{ terms_valid: false }, { validation_errors: ["invalid"] }, { status: "open" }, { funded_amount: "1" }, { target_amount: "0" }, { target_amount: "NaN" }, { bounty_id: "bad" }]) {
    assert.equal(confirmedItem([{ ...item, ...changes }], contract), null);
  }
});
test("URL claims, another bounty's events, unconfirmed logs and a hash alone cannot show success", () => {
  assert.equal(confirmedItem([], contract), null);
  assert.equal(confirmedItem([item], "javascript:alert(1)"), null);
  for (const changes of [{ bounty_id: "0x" + "6".repeat(64) }, { block_number: 0 }, { block_number: 1.5 }, { log_index: -1 }, { tx_hash: "batch-identifier" }, { id: "" }]) {
    assert.equal(confirmedItem([{ ...item, events: item.events.map(e => ({ ...e, ...changes })) }], contract), null);
  }
  assert.equal(confirmedItem([{ ...item, events: item.events.map(e => e.kind === "funding_added" ? { ...e, contract_address: "0x" + "9".repeat(40) } : e) }], contract), null);
  assert.equal(confirmedItem([{ ...item, events: item.events.map(e => e.kind === "funding_added" ? { ...e, data: { funded_amount: 1 } } : e) }], contract), null);
});
test("confirmed funding does not imply current readiness or solver payment; amounts retain every USDC decimal", () => {
  assert.ok(confirmedItem([{ ...item, verification_ready: false }], contract));
  assert.ok(confirmedItem([{ ...item, status: "claimed" }], contract));
  assert.equal(formatAmount("17068098"), "17.068098 USDC");
  assert.equal(formatAmount("1"), "0.000001 USDC");
  assert.equal(formatAmount("2000000"), "2.00 USDC");
  assert.equal(formatAmount("9007199254740993000001"), "9007199254740993.000001 USDC");
});
