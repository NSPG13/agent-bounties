"use strict";

const assert = require("node:assert/strict");
const {
  parseDistributionAttribution,
  parsePreparedRewardSplit,
  rewardSplitForTotal,
} = require("../site/bounty-composer-v2.js");

const staged = parsePreparedRewardSplit("9", "1");
const reviewed = rewardSplitForTotal(10, staged);
const submitted = rewardSplitForTotal(10, staged);

assert.deepEqual(reviewed, {
  total: 10_000_000n,
  solver: 9_000_000n,
  verifier: 1_000_000n,
});
assert.deepEqual(submitted, reviewed);
assert.equal(submitted.verifier, 1_000_000n, "the verifier-sized solver bond must retain the approved amount");

const manual = rewardSplitForTotal(10);
assert.deepEqual(manual, {
  total: 10_000_000n,
  solver: 9_800_000n,
  verifier: 200_000n,
});

assert.throws(
  () => parsePreparedRewardSplit("9.0000001", "1"),
  /up to six places/,
);

assert.throws(
  () => parsePreparedRewardSplit("1.999999", "0.010001"),
  /at least 2 USDC for the solver/,
);

assert.deepEqual(rewardSplitForTotal(2.01), {
  total: 2_010_000n,
  solver: 2_000_000n,
  verifier: 10_000n,
});

const acquisition = `aba1_${"ab".repeat(32)}.${"cd".repeat(32)}`;
const handoff = "10000000-0000-4000-8000-000000000001";
const attributed = new URLSearchParams({ acquisition, handoff });
assert.deepEqual(parseDistributionAttribution(attributed), { acquisition, handoff });

for (const missing of ["acquisition", "handoff"]) {
  const partial = new URLSearchParams(attributed);
  partial.delete(missing);
  assert.throws(() => parseDistributionAttribution(partial), /must include both/);
}

const malformed = new URLSearchParams(attributed);
malformed.set("acquisition", "aba1_not-opaque");
assert.throws(() => parseDistributionAttribution(malformed), /malformed/);

process.stdout.write("bounty economics behavior check passed\n");
