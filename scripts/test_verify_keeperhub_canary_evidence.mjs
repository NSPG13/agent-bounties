#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

import { verifyKeeperHubCanaryEvidence } from "./verify_keeperhub_canary_evidence.mjs";

const evidence = JSON.parse(
  await readFile(
    new URL("../docs/evidence/keeperhub-agents-onchain-canary-base-sepolia-2026-08-13.json", import.meta.url),
    "utf8",
  ),
);

function addressTopic(address) {
  return `0x${"0".repeat(24)}${address.toLowerCase().slice(2)}`;
}

function jsonResponse(result) {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function matchingReceipt() {
  return {
    transactionHash: evidence.transaction.hash,
    status: evidence.transaction.status,
    blockHash: evidence.transaction.block_hash,
    blockNumber: `0x${evidence.transaction.block_number.toString(16)}`,
    gasUsed: `0x${BigInt(evidence.transaction.gas_used).toString(16)}`,
    logs: [
      {
        address: evidence.transaction.factory,
        removed: false,
        topics: [
          evidence.transaction.canonical_event.topic0,
          evidence.transaction.bounty_id,
          addressTopic(evidence.transaction.bounty),
          addressTopic(evidence.transaction.creator),
        ],
      },
    ],
  };
}

function rpcFixture(receipt, chainId = "0x14a34") {
  return async (_url, init) => {
    const request = JSON.parse(init.body);
    if (request.method === "eth_chainId") return jsonResponse(chainId);
    if (request.method === "eth_getTransactionReceipt") return jsonResponse(receipt);
    throw new Error(`unexpected method: ${request.method}`);
  };
}

const verified = await verifyKeeperHubCanaryEvidence({
  evidence,
  rpcUrl: "https://sepolia.example",
  fetchImpl: rpcFixture(matchingReceipt()),
});
assert.equal(verified.verified, true);
assert.equal(verified.transaction_hash, evidence.transaction.hash);
assert.equal(verified.bounty, evidence.transaction.bounty);

await assert.rejects(
  verifyKeeperHubCanaryEvidence({
    evidence,
    rpcUrl: "https://sepolia.example",
    fetchImpl: rpcFixture(matchingReceipt(), "0x2105"),
  }),
  /RPC is not Base Sepolia/,
);

const failedReceipt = matchingReceipt();
failedReceipt.status = "0x0";
await assert.rejects(
  verifyKeeperHubCanaryEvidence({
    evidence,
    rpcUrl: "https://sepolia.example",
    fetchImpl: rpcFixture(failedReceipt),
  }),
  /transaction did not succeed/,
);

const tamperedEvent = matchingReceipt();
tamperedEvent.logs[0].topics[2] = addressTopic("0x0000000000000000000000000000000000000001");
await assert.rejects(
  verifyKeeperHubCanaryEvidence({
    evidence,
    rpcUrl: "https://sepolia.example",
    fetchImpl: rpcFixture(tamperedEvent),
  }),
  /expected one canonical creation event, found 0/,
);

await assert.rejects(
  verifyKeeperHubCanaryEvidence({
    evidence,
    rpcUrl: "http://sepolia.example",
    fetchImpl: rpcFixture(matchingReceipt()),
  }),
  /RPC URL must use HTTPS/,
);

console.log("verify_keeperhub_canary_evidence_tests=ok");
