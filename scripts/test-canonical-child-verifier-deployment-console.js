"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "tools", "canonical-child-verifier-deployment.html"), "utf8");
const script = fs.readFileSync(path.join(root, "tools", "canonical-child-verifier-deployment.js"), "utf8");
const bundle = JSON.parse(fs.readFileSync(path.join(root, "deployments", "canonical-child-verifier-base-mainnet-deployment.json"), "utf8"));

for (const required of [
  "/deployments/canonical-child-verifier-base-mainnet-deployment.json",
  "eip6963:requestProvider",
  "MetaMask",
  "eth_estimateGas",
  "eth_getTransactionCount",
  "eth_getCode",
  "eth_sendTransaction",
  "expected_runtime_code",
  "0x044f3e72",
  "0x77de6ca7",
  "127.0.0.1",
  "localhost",
]) {
  assert.ok(script.includes(required), `deployment console must include ${required}`);
}

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `deployment console must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `deployment page must exclude ${forbidden}`);
}

assert.ok(html.includes("Content-Security-Policy"));
assert.ok(html.includes('id="wallet-provider"'));
assert.ok(!html.includes("<input"), "deployment console must not accept transaction overrides");
assert.equal(bundle.schema_version, "agent-bounties/canonical-child-verifier-deployment-v1");
assert.equal(bundle.chain_id, 8453);
assert.equal(bundle.canonical_factory, "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9");
assert.equal(bundle.settlement_token, "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913");
assert.equal(bundle.acceptance_criteria_hash, "0xa103c2c907f96e03a2f2b0e6b2209e0a3ca53686f7e9f79d89d7bfa1f8e314de");
assert.ok(script.includes(`const ACCEPTANCE_CRITERIA_HASH = "${bundle.acceptance_criteria_hash}";`));
assert.equal(bundle.deployment.to, null);
assert.equal(bundle.deployment.value_wei, 0);
assert.ok(bundle.deployment.data.startsWith("0x60c06040"));
assert.equal(bundle.deployment.expected_runtime_code.length, bundle.deployment.runtime_code_bytes * 2 + 2);
console.log("canonical child verifier deployment console contract passed");
