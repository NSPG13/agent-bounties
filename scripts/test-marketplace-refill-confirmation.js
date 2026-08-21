"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(root, "site", "marketplace-refill-confirmation.js"), "utf8");
const page = fs.readFileSync(path.join(root, "site", "marketplace-refill-confirmation.html"), "utf8");
const match = source.match(/const FROZEN_ENTRIES = Object\.freeze\((\[[\s\S]*?\n\s*\])\);\n\n\s*const ui/);
assert.ok(match, "frozen entry array must remain statically reviewable");
const entries = JSON.parse(match[1]);

assert.equal(entries.length, 10);
assert.equal(new Set(entries.map((entry) => entry.candidate_id)).size, 10);
assert.equal(new Set(entries.map((entry) => entry.predicted_competition)).size, 10);
assert.equal(new Set(entries.map((entry) => entry.bounty_id)).size, 10);
const hash = "0x" + crypto.createHash("sha256").update(JSON.stringify(entries)).digest("hex");
assert.equal(hash, "0x9704dee0c561b28324df167adc793ae88c7ceff755561296076050ee63c1855a");

for (const entry of entries) {
  assert.equal(entry.calls.length, 2);
  const [approval, creation] = entry.calls;
  assert.equal(approval.to, "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913");
  assert.equal(approval.value, "0x0");
  assert.ok(approval.data.startsWith("0x095ea7b3"));
  assert.equal("0x" + approval.data.slice(34, 74), entry.predicted_competition);
  assert.equal(BigInt("0x" + approval.data.slice(74, 138)), 3_040_000n);
  assert.equal(creation.to, "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4");
  assert.equal(creation.value, "0x0");
  assert.ok(creation.data.startsWith("0x7058f671"));
  assert.equal(creation.data.length, 1290);
}

assert.equal(entries.flatMap((entry) => entry.calls).length, 20);
assert.match(source, /atomicRequired: true/);
assert.match(source, /wallet\("wallet_sendCalls"/);
assert.match(source, /wallet\("wallet_getCallsStatus"/);
assert.doesNotMatch(source, /eth_sendTransaction/);
assert.doesNotMatch(source, /for \(const call of calls\(\)\)/);
assert.match(page, /<meta name="robots" content="noindex,nofollow">/);
assert.match(page, />Confirm 30\.40 USDC funding<\/button>/);
assert.match(page, /Either all twenty calls succeed together or none do\./);

console.log("marketplace refill confirmation invariants passed");
