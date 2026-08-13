#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const EVIDENCE_SCHEMA =
  "agent-bounties/keeperhub-agents-onchain-canary-evidence-v1";
export const BASE_SEPOLIA_CHAIN_ID = 84532;
export const DEFAULT_BASE_SEPOLIA_RPC_URL = "https://sepolia.base.org";

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function normalizedHex(value, bytes, label) {
  invariant(
    typeof value === "string" && new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value),
    `${label} must be ${bytes} bytes`,
  );
  return value.toLowerCase();
}

function addressTopic(address, label) {
  return `0x${"0".repeat(24)}${normalizedHex(address, 20, label).slice(2)}`;
}

function rpcOrigin(rpcUrl) {
  const url = new URL(rpcUrl);
  const local = url.hostname === "127.0.0.1" || url.hostname === "localhost";
  invariant(url.protocol === "https:" || (local && url.protocol === "http:"), "RPC URL must use HTTPS");
  return url.href;
}

async function rpcCall({ rpcUrl, method, params, fetchImpl }) {
  const response = await fetchImpl(rpcOrigin(rpcUrl), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
  });
  invariant(response.ok, `RPC ${method} returned HTTP ${response.status}`);
  const body = await response.json();
  invariant(isPlainObject(body), `RPC ${method} returned a malformed response`);
  invariant(!body.error, `RPC ${method} failed: ${body.error?.message || "unknown error"}`);
  return body.result;
}

export async function verifyKeeperHubCanaryEvidence({
  evidence,
  rpcUrl = DEFAULT_BASE_SEPOLIA_RPC_URL,
  fetchImpl = fetch,
}) {
  invariant(isPlainObject(evidence), "evidence must be a JSON object");
  invariant(evidence.schema_version === EVIDENCE_SCHEMA, "unsupported evidence schema");
  invariant(Number(evidence.network?.chain_id) === BASE_SEPOLIA_CHAIN_ID, "evidence is not Base Sepolia");

  const transaction = evidence.transaction;
  invariant(isPlainObject(transaction), "transaction evidence is required");
  const transactionHash = normalizedHex(transaction.hash, 32, "transaction hash");
  const blockHash = normalizedHex(transaction.block_hash, 32, "block hash");
  const factory = normalizedHex(transaction.factory, 20, "factory");
  const bounty = normalizedHex(transaction.bounty, 20, "bounty");
  const bountyId = normalizedHex(transaction.bounty_id, 32, "bounty ID");
  const creator = normalizedHex(transaction.creator, 20, "creator");
  const eventTopic = normalizedHex(
    transaction.canonical_event?.topic0,
    32,
    "canonical event topic",
  );
  invariant(
    transaction.canonical_event?.name === "CanonicalCompetitionCreated",
    "unexpected canonical event name",
  );
  invariant(transaction.canonical_factory_registration === true, "factory registration evidence is missing");

  const chainIdHex = await rpcCall({
    rpcUrl,
    method: "eth_chainId",
    params: [],
    fetchImpl,
  });
  invariant(BigInt(chainIdHex) === BigInt(BASE_SEPOLIA_CHAIN_ID), "RPC is not Base Sepolia");

  const receipt = await rpcCall({
    rpcUrl,
    method: "eth_getTransactionReceipt",
    params: [transactionHash],
    fetchImpl,
  });
  invariant(isPlainObject(receipt), "transaction receipt is unavailable");
  invariant(normalizedHex(receipt.transactionHash, 32, "receipt transaction hash") === transactionHash, "transaction hash mismatch");
  invariant(receipt.status === transaction.status && receipt.status === "0x1", "transaction did not succeed");
  invariant(normalizedHex(receipt.blockHash, 32, "receipt block hash") === blockHash, "block hash mismatch");
  invariant(BigInt(receipt.blockNumber) === BigInt(transaction.block_number), "block number mismatch");
  invariant(BigInt(receipt.gasUsed) === BigInt(transaction.gas_used), "gas used mismatch");
  invariant(Array.isArray(receipt.logs), "receipt logs are missing");

  const expectedTopics = [
    eventTopic,
    bountyId,
    addressTopic(bounty, "bounty"),
    addressTopic(creator, "creator"),
  ];
  const canonicalLogs = receipt.logs.filter((log) => {
    if (!isPlainObject(log) || log.removed === true) return false;
    if (String(log.address).toLowerCase() !== factory) return false;
    if (!Array.isArray(log.topics) || log.topics.length !== expectedTopics.length) return false;
    return expectedTopics.every((topic, index) => String(log.topics[index]).toLowerCase() === topic);
  });
  invariant(canonicalLogs.length === 1, `expected one canonical creation event, found ${canonicalLogs.length}`);

  return {
    schema_version: EVIDENCE_SCHEMA,
    verified: true,
    chain_id: BASE_SEPOLIA_CHAIN_ID,
    transaction_hash: transactionHash,
    block_number: Number(BigInt(receipt.blockNumber)),
    factory,
    bounty,
    bounty_id: bountyId,
    creator,
    canonical_event: "CanonicalCompetitionCreated",
    evidence_boundary: evidence.evidence_boundary,
  };
}

function parseCliArgs(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    invariant(key.startsWith("--"), `unexpected argument: ${key}`);
    const value = argv[index + 1];
    invariant(value !== undefined && !value.startsWith("--"), `missing value for ${key}`);
    options[key.slice(2)] = value;
    index += 1;
  }
  return options;
}

async function main() {
  const options = parseCliArgs(process.argv.slice(2));
  invariant(options.evidence, "--evidence is required");
  const evidence = JSON.parse(await readFile(resolve(options.evidence), "utf8"));
  const verified = await verifyKeeperHubCanaryEvidence({
    evidence,
    rpcUrl: options["rpc-url"] || process.env.BASE_SEPOLIA_RPC_URL || DEFAULT_BASE_SEPOLIA_RPC_URL,
  });
  process.stdout.write(`${JSON.stringify(verified, null, 2)}\n`);
}

const invokedAsScript = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (invokedAsScript) {
  main().catch((error) => {
    process.stderr.write(`verify_keeperhub_canary_evidence: ${error.message}\n`);
    process.exitCode = 1;
  });
}
