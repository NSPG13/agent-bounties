"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(
  path.join(root, "scripts", "open-competition-entrant-wallet-mainnet-signer.html"),
  "utf8",
);
const script = fs.readFileSync(
  path.join(root, "scripts", "open-competition-entrant-wallet-mainnet-signer.js"),
  "utf8",
);

for (const required of [
  "agent-bounties/open-competition-entrant-wallet-mainnet-release-bundle-v1",
  "cb3476158ee39a8928dba73da6861d5f782792ce",
  "0x884834e884d6e93462655a2820140ad03e6747bc",
  "0x4e59b44847b379578588920ca78fbf26c0b4956c",
  "0x2105",
  "https://mainnet.base.org",
  "eip6963:announceProvider",
  "wallet_switchEthereumChain",
  "eth_getBlockByNumber",
  "eth_getBalance",
  "eth_getCode",
  "eth_sendTransaction",
  "eth_getTransactionReceipt",
  "single_zero_value_create2_call",
  "Object.values(value.activation).some(Boolean)",
  "public_creation_enabled: false",
  "public_commitments_enabled: false",
  "public_inventory_enabled: false",
]) {
  assert.ok(script.includes(required), `mainnet entrant signer must include ${required}`);
}

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `mainnet entrant signer must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `mainnet entrant page must exclude ${forbidden}`);
}

assert.ok(html.includes("One zero-value call goes only to the canonical CREATE2 deployer."));
assert.ok(html.includes('id="reviewed"'));
assert.ok(html.includes("Hosted relay and public inventory remain disabled."));
assert.ok(html.includes("open-competition-entrant-wallet-mainnet-signer.js?v=release-v1"));
console.log("Open Competition entrant-wallet mainnet signer console contract passed");
