"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "scripts", "open-competition-v1-mainnet-signer.html"), "utf8");
const script = fs.readFileSync(path.join(root, "scripts", "open-competition-v1-mainnet-signer.js"), "utf8");

for (const required of [
  "agent-bounties/open-competition-v1-mainnet-bundle-v1",
  "bc9b3cc9f9f95a87df671be2d13199ac9d06ebcf",
  "0x2105",
  "https://mainnet.base.org",
  "eip6963:announceProvider",
  "wallet_switchEthereumChain",
  "eth_getTransactionCount",
  "eth_getBalance",
  "eth_getCode",
  "eth_call",
  "eth_sendTransaction",
  "eth_getTransactionReceipt",
  "keccak256Hex",
  "public_inventory_eligible: false",
  "total_admin_usdc_budget_base_units",
  "Pinned Base safe block is no longer canonical",
]) assert.ok(script.includes(required), `mainnet signer must include ${required}`);

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `mainnet signer must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `mainnet page must exclude ${forbidden}`);
}

assert.ok(html.includes("Base mainnet · hidden canary only"));
assert.ok(html.includes('id="reviewed"'));
assert.ok(html.includes("open-competition-v1-mainnet-signer.js?v=mainnet-v1"));
console.log("Open Competition V1 mainnet signer console contract passed");
