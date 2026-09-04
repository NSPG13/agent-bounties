"use strict";

const assert = require("node:assert/strict");
const {
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

process.stdout.write("bounty economics behavior check passed\n");
