"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "tools", "base-sepolia-sponsor-activation.html"), "utf8");
const script = fs.readFileSync(path.join(root, "tools", "base-sepolia-sponsor-activation.js"), "utf8");
const bundle = JSON.parse(fs.readFileSync(path.join(root, "deployments", "base-sepolia-sponsor-activation.json"), "utf8"));

for (const required of [
  "/deployments/base-sepolia-sponsor-activation.json",
  "eip6963:requestProvider",
  "MetaMask",
  "Coinbase Wallet",
  "Brave Wallet",
  "wallet_switchEthereumChain",
  "eth_getTransactionCount",
  '"pending"',
  "eth_estimateGas",
  "eth_getCode",
  "web3_sha3",
  "eth_sendTransaction",
  "expected_runtime_code",
  "expected_implementation_runtime_code",
  "Nonce drift",
  "0x7b9e618d",
  "0x249379ad",
  "0xf7c37ccd",
  "0x890371b2",
  "127.0.0.1",
  "localhost",
  "0x2c911d92c9580a1c7a86e1d48173f54e11224ab84435ca6c6213c4a66b35d1e5",
  "0xee2c4631dffdfa40566b2e98cd4111f405c72ba461cb4aeca76f67cbbaa72efe",
  "Sponsor balance drift",
]) assert.ok(script.includes(required), `activation console must include ${required}`);

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `activation console must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `activation page must exclude ${forbidden}`);
}

assert.ok(html.includes("Content-Security-Policy"));
assert.ok(html.includes('id="wallet-provider"'));
assert.ok(!html.includes("<input"), "activation console must not accept transaction overrides");
assert.equal(bundle.schema_version, "agent-bounties/base-sepolia-sponsor-activation-v1");
assert.equal(bundle.chain_id, 84532);
assert.equal(bundle.deployer, "0x884834e884d6e93462655a2820140ad03e6747bc");
assert.equal(bundle.settlement_token, "0x036cbd53842c5426634e7929541ec2318f3dcf7e");
assert.equal(bundle.factory.from_nonce, 1);
assert.equal(bundle.verifier.from_nonce, 2);
assert.equal(bundle.sponsor.from_nonce, 3);
assert.equal(bundle.factory.expected_contract, "0x9601a40b35ad6843846732c6cb73c4c82f9ba850");
assert.equal(bundle.verifier.expected_contract, "0x7231f1312448fa60078fb56cdb6e2c392bd1269b");
assert.equal(bundle.sponsor.expected_contract, "0xa1e2e93530114f7fe64c251556b8de13dad7d157");
assert.equal(bundle.verifier.difficulty_bits, 16);
assert.equal(bundle.sponsor.max_bond_base_units, 100000);
assert.equal(bundle.sponsor.max_network_per_day_base_units, 1000000);
assert.equal(bundle.sponsor.max_lifetime_per_solver_base_units, 100000);
assert.equal(bundle.sponsor_funding.amount_base_units, 100000);
assert.equal(bundle.sponsor_funding.recipient, bundle.sponsor.expected_contract);
for (const deployment of [bundle.factory, bundle.verifier, bundle.sponsor]) {
  assert.equal(deployment.to, null);
  assert.equal(deployment.value_wei, 0);
  assert.equal(deployment.expected_runtime_code.length, deployment.runtime_code_bytes * 2 + 2);
}
console.log("Base Sepolia sponsor activation console contract passed");
