import assert from "node:assert/strict";
import {
  delegatedCode,
  normalizeEvmCall,
  transactionToCalls,
  waitForUserOperationTransaction,
  walletRequestToCalls,
} from "./sponsorship.js";

const account = "0x1234567890abcdef1234567890abcdef12345678";
const target = "0xabcdefabcdefabcdefabcdefabcdefabcdefabcd";
const hash = `0x${"12".repeat(32)}`;
const txHash = `0x${"34".repeat(32)}`;

assert.deepEqual(normalizeEvmCall({ to: target, value: "0x0", data: "0x1234" }), {
  to: target,
  value: 0n,
  data: "0x1234",
});
assert.deepEqual(transactionToCalls({ from: account, to: target, data: "0x", value: "0" }, account), [{
  to: target,
  value: 0n,
  data: "0x",
}]);
assert.throws(
  () => transactionToCalls({ from: target, to: target, data: "0x" }, account),
  /sender does not match/,
);
assert.deepEqual(
  walletRequestToCalls({
    params: [{
      chainId: "0x2105",
      from: account,
      calls: [
        { to: target, data: "0x", value: "0x0" },
        { to: account, data: "0xabcd", value: 1n },
      ],
    }],
  }, account),
  [
    { to: target, data: "0x", value: 0n },
    { to: account, data: "0xabcd", value: 1n },
  ],
);
assert.throws(
  () => walletRequestToCalls({ params: [{ chainId: "0x1", from: account, calls: [{ to: target }] }] }, account),
  /Base mainnet/,
);
assert.equal(delegatedCode("0x"), false);
assert.equal(delegatedCode("0x0"), false);
assert.equal(delegatedCode("0xef0100abcdef"), true);

let tick = 0;
const requests = [];
const completed = await waitForUserOperationTransaction({
  userOperationHash: hash,
  account,
  timeoutMs: 10_000,
  intervalMs: 1,
  now: () => tick,
  sleep: async (milliseconds) => { tick += milliseconds; },
  getOperation: async (request) => {
    requests.push(request);
    return requests.length < 3
      ? { status: "pending" }
      : { status: "complete", transactionHash: txHash };
  },
});
assert.equal(completed, txHash);
assert.equal(requests.length, 3);
assert.deepEqual(requests[0], {
  userOperationHash: hash,
  evmSmartAccount: account,
  network: "base",
});

await assert.rejects(
  waitForUserOperationTransaction({
    userOperationHash: hash,
    account,
    getOperation: async () => ({
      status: "failed",
      receipts: [{ revert: { message: "Paymaster policy rejected the call" } }],
    }),
  }),
  /Paymaster policy rejected/,
);

console.log("Coinbase sponsorship helpers preserve sender, Base chain, call data, and confirmed transaction evidence");
