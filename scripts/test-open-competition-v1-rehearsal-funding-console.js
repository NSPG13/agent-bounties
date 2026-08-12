const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "scripts/open-competition-v1-rehearsal-funding.html"), "utf8");
const script = fs.readFileSync(path.join(root, "scripts/open-competition-v1-rehearsal-funding.js"), "utf8");

for (const required of [
  "agent-bounties/open-competition-v1-rehearsal-funding-v1",
  "0x884834e884d6e93462655a2820140ad03e6747bc",
  "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
  "500000000000000n",
  "500000n",
  "wallet_switchEthereumChain",
  "eth_sendTransaction",
  "a9059cbb",
  "https://sepolia.base.org",
  "https://base-sepolia-rpc.publicnode.com",
  "eth !== EXPECTED_ETH || usdc !== EXPECTED_USDC",
]) assert.ok(script.includes(required), `funding console must include ${required}`);

for (const forbidden of ["privateKey", "private_key", "seed phrase", "mnemonic", "eth_sign"]) {
  assert.ok(!script.includes(forbidden), `funding console must not include ${forbidden}`);
}

assert.ok(html.includes('id="reviewed"'));
assert.ok(html.includes('id="execute"'));
assert.ok(html.includes("open-competition-v1-rehearsal-funding.js?v=2"));
console.log("Open Competition V1 rehearsal funding console contract passed");
