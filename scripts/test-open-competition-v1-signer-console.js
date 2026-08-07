"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "scripts", "open-competition-v1-signer.html"), "utf8");
const script = fs.readFileSync(path.join(root, "scripts", "open-competition-v1-signer.js"), "utf8");

for (const required of [
  "eip6963:announceProvider",
  "eip6963:requestProvider",
  "io.metamask",
  "wallet-provider",
  "eth_chainId",
  "wallet_switchEthereumChain",
  "wallet_addEthereumChain",
  "eth_getTransactionCount",
  "eth_getCode",
  "eth_sendTransaction",
  "eth_getTransactionReceipt",
  "expected_runtime_code",
  "expected_implementation_runtime_code",
]) assert.ok(script.includes(required), `signer console must include ${required}`);

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.toLowerCase().includes(forbidden.toLowerCase()), `signer console must exclude ${forbidden}`);
  assert.ok(!html.toLowerCase().includes(forbidden.toLowerCase()), `signer page must exclude ${forbidden}`);
}

assert.ok(html.includes('id="wallet-provider"'));
assert.ok(html.includes('id="bundle"'));
assert.ok(script.includes("Load and inspect the frozen bundle before connecting a wallet."));
assert.ok(script.includes('Connected account ${account || "(none)"} is not the frozen admin'));
console.log("Open Competition V1 signer console contract passed");
