"use strict";
const { test } = require("node:test"), assert = require("node:assert/strict");
const { calendar } = require("../site/review-deadline.js");
const job = { bounty_contract: "0x" + "aa".repeat(20), round: 2, verification_expires_at: 2000000000 };
test("review calendar binds the canonical round and UTC deadline", () => {
  const raw = calendar(job, 1900000000000);
  assert.ok(raw.split("\r\n").every(line => Buffer.byteLength(line) <= 75));
  const body = raw.replace(/\r\n /g, "");
  assert.match(body, /DTEND:20330518T033320Z\r\n/);
  assert.match(body, /-2-2000000000@agentbounties.app/);
  assert.match(body, /TRIGGER:-PT1H/);
  assert.match(body, /not payment evidence/);
  assert.ok(body.endsWith("END:VCALENDAR\r\n"));
});
test("missing, expired or injected calendar inputs fail closed", () => {
  for (const patch of [{ round: 0 }, { verification_expires_at: 1 }, { verification_expires_at: Infinity }, { bounty_contract: "bad\r\nATTENDEE:attacker" }]) {
    assert.throws(() => calendar({ ...job, ...patch }, 1900000000000));
  }
});
