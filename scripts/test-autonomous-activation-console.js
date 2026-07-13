"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "tools", "autonomous-activation.html"), "utf8");
const script = fs.readFileSync(path.join(root, "tools", "autonomous-activation.js"), "utf8");
const bundle = JSON.parse(fs.readFileSync(path.join(root, "deployments", "canonical-child-seeds-base-mainnet.json"), "utf8"));
const verifierBundle = JSON.parse(fs.readFileSync(path.join(root, "deployments", "canonical-child-verifier-base-mainnet-deployment.json"), "utf8"));

for (const required of [
  "/deployments/canonical-child-seeds-base-mainnet.json",
  "/deployments/canonical-child-verifier-base-mainnet-deployment.json",
  "0x40adac5a1d00a725f77682f8940b893eaed31ecf",
  "wallet_sendCalls",
  "atomicRequired: true",
  "eip6963:requestProvider",
  "Coinbase Wallet",
  "eth_getTransactionCount",
  "eth_getCode",
  "eth_sendTransaction",
  "0xdb021126",
  "bountyIsActivated",
  "pendingBounties",
  "remainingFunding",
  "0x41506fc1",
  "0x8a2b02be",
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
assert.ok(html.includes('id="wallet-provider"'));
assert.ok(html.includes("Use sequential fallback"));
assert.ok(!html.includes('id="deploy"'));
assert.ok(!html.includes("https://cdn."));
assert.ok(!html.includes("<input"), "activation console must not accept transaction overrides");
assert.equal(bundle.deployment.from, "0x884834e884d6e93462655a2820140ad03e6747bc");
assert.equal(bundle.deployment.deployer_nonce, 4);
assert.equal(bundle.deployment.expected_factory, "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9");
assert.equal(bundle.deployment.expected_implementation, "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9");
assert.deepEqual(bundle.bounties.map((item) => item.issue), [217, 218, 219, 220]);
assert.equal(verifierBundle.deployment.expected_contract, "0x40adac5a1d00a725f77682f8940b893eaed31ecf");
assert.ok(script.includes(`const ACCEPTANCE_CRITERIA_HASH = "${verifierBundle.acceptance_criteria_hash}";`));
assert.ok(script.includes(`bundle.manifest_canonical_json_keccak256 !== "${bundle.manifest_canonical_json_keccak256}"`));
assert.equal(bundle.creation_batch.wallet_calls.length, 5);
assert.equal(bundle.creation_batch.wallet_calls[0].function, "approve(address,uint256)");
assert.ok(bundle.creation_batch.wallet_calls[0].data.startsWith("0x095ea7b3"));
assert.ok(bundle.creation_batch.wallet_calls[0].data.endsWith("00000000000000000000000000000000000000000000000000000000003d0900"));
for (const call of bundle.creation_batch.wallet_calls.slice(1)) {
  assert.equal(call.to, bundle.deployment.expected_factory);
  assert.ok(call.data.startsWith("0x9d2e414c"));
}
console.log("autonomous activation console contract passed");
