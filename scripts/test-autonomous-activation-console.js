"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "tools", "autonomous-activation.html"), "utf8");
const script = fs.readFileSync(path.join(root, "tools", "autonomous-activation.js"), "utf8");
const bundle = JSON.parse(fs.readFileSync(path.join(root, "deployments", "base-mainnet-activation.json"), "utf8"));

for (const required of [
  "/deployments/base-mainnet-activation.json",
  "wallet_sendCalls",
  "eth_estimateGas",
  "eth_getTransactionCount",
  "eth_getCode",
  "eth_sendTransaction",
  "0xdb021126",
  "Indexer reconciliation is still required",
  "127.0.0.1",
  "localhost",
]) {
  assert.ok(script.includes(required), `activation console must include ${required}`);
}

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `activation console must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `activation page must exclude ${forbidden}`);
}

assert.ok(html.includes("Content-Security-Policy"));
assert.ok(html.includes("Use sequential fallback"));
assert.ok(!html.includes("https://cdn."));
assert.ok(!html.includes("<input"), "activation console must not accept transaction overrides");
assert.equal(bundle.deployment.from, "0x884834e884d6e93462655a2820140ad03e6747bc");
assert.equal(bundle.deployment.deployer_nonce, 4);
assert.equal(bundle.deployment.expected_factory, "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9");
assert.equal(bundle.deployment.expected_implementation, "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9");
assert.equal(bundle.creation_batch.wallet_calls.length, 5);
assert.equal(bundle.creation_batch.wallet_calls[0].function, "approve(address,uint256)");
assert.ok(bundle.creation_batch.wallet_calls[0].data.startsWith("0x095ea7b3"));
assert.ok(bundle.creation_batch.wallet_calls[0].data.endsWith("00000000000000000000000000000000000000000000000000000000003d0900"));
for (const call of bundle.creation_batch.wallet_calls.slice(1)) {
  assert.equal(call.to, bundle.deployment.expected_factory);
  assert.ok(call.data.startsWith("0x9d2e414c"));
}
console.log("autonomous activation console contract passed");
