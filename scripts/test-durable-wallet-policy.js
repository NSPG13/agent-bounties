const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");
const { TextEncoder } = require("util");

const repoRoot = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(repoRoot, "site", "durable-wallet-policy.js"), "utf8");
new vm.Script(source, { filename: "site/durable-wallet-policy.js" });

const document = {
  readyState: "loading",
  addEventListener() {},
  getElementById() { return null; },
  querySelector() { return null; },
};
const window = {
  addEventListener() {},
  dispatchEvent() {},
  ethereum: null,
};
const context = vm.createContext({
  console,
  document,
  window,
  TextEncoder,
  Event: class Event {},
  setTimeout,
  clearTimeout,
});
const instrumented = source.replace(
  /\}\)\(\);\s*$/,
  "globalThis.__durablePolicyTest = { keccak256Hex, textHex, selector }; })();",
);
new vm.Script(instrumented, { filename: "site/durable-wallet-policy.js" }).runInContext(context);
const test = context.__durablePolicyTest;

assert.strictEqual(
  test.keccak256Hex("0x"),
  "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
);
assert.strictEqual(test.selector("balanceOf(address)"), "0x70a08231");
assert.strictEqual(test.selector("owner()"), "0x8da5cb5b");
assert.strictEqual(test.selector("policy()"), "0x0505c8c9");
assert.strictEqual(test.selector("policyVersion()"), "0x58355ead");
assert.strictEqual(test.selector("periodSpent()"), "0x81497000");
assert.strictEqual(
  test.selector("configurePolicy((address,uint64,uint64,uint64,uint256,uint256,uint256,uint256,uint8,uint8,address,bytes32,bytes32))"),
  "0x27d3543c",
);
assert.match(
  test.keccak256Hex(test.textHex("PolicyBootstrapped(bytes32,address,bytes32)")),
  /^0x[0-9a-f]{64}$/,
);

console.log("durable wallet policy cryptographic contract passed");
