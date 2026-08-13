#!/usr/bin/env node

import { createHash, randomUUID } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

import {
  BASE_SEPOLIA_CHAIN_ID,
  BASE_SEPOLIA_OPEN_COMPETITION_FACTORY,
  REQUEST_SCHEMA,
  validateRequest,
} from "./keeperhub_direct_execution.mjs";

export const BASE_SEPOLIA_LEADING_ZERO_VERIFIER =
  "0x9601a40b35ad6843846732c6cb73c4c82f9ba850";

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function bytes32(label) {
  return `0x${createHash("sha256").update(label).digest("hex")}`;
}

function isAddress(value) {
  return typeof value === "string" && /^0x[0-9a-fA-F]{40}$/.test(value);
}

export function buildCanaryRequest({ wallet, sourceUrl, nowSeconds = Math.floor(Date.now() / 1_000) }) {
  invariant(isAddress(wallet), "--wallet must be an EVM address");
  const parsedSource = new URL(sourceUrl);
  invariant(parsedSource.protocol === "https:", "--source-url must use HTTPS");
  invariant(Number.isSafeInteger(nowSeconds) && nowSeconds > 0, "nowSeconds is invalid");

  const canaryId = `keeperhub-agents-onchain-${nowSeconds}-${randomUUID()}`;
  const params = {
    solverReward: "100000",
    verifierReward: "10000",
    termsHash: bytes32(`${canaryId}:terms:${sourceUrl}`),
    policyHash: bytes32(`${canaryId}:policy:deterministic-first`),
    acceptanceCriteriaHash: bytes32(`${canaryId}:criteria:keeperhub-receipt`),
    benchmarkHash: bytes32(`${canaryId}:benchmark:leading-zero-16`),
    evidenceSchemaHash: bytes32("agent-bounties/keeperhub-direct-execution-receipt-v1"),
    fundingDeadline: String(nowSeconds + 7 * 24 * 60 * 60),
    competitionWindowSeconds: "86400",
    revealWindowSeconds: "3600",
    maxEntries: 4,
    verifierModule: BASE_SEPOLIA_LEADING_ZERO_VERIFIER,
    verifierRewardRecipient: wallet,
  };

  const request = {
    schema_version: REQUEST_SCHEMA,
    operation: "contract_call",
    chain_id: BASE_SEPOLIA_CHAIN_ID,
    contract_address: BASE_SEPOLIA_OPEN_COMPETITION_FACTORY,
    function_name: "createCompetition",
    function_args: [params, "0", bytes32(`${canaryId}:creation-nonce`)],
    abi: [
      {
        type: "function",
        name: "createCompetition",
        stateMutability: "nonpayable",
        inputs: [
          {
            name: "params",
            type: "tuple",
            components: [
              { name: "solverReward", type: "uint256" },
              { name: "verifierReward", type: "uint256" },
              { name: "termsHash", type: "bytes32" },
              { name: "policyHash", type: "bytes32" },
              { name: "acceptanceCriteriaHash", type: "bytes32" },
              { name: "benchmarkHash", type: "bytes32" },
              { name: "evidenceSchemaHash", type: "bytes32" },
              { name: "fundingDeadline", type: "uint64" },
              { name: "competitionWindowSeconds", type: "uint64" },
              { name: "revealWindowSeconds", type: "uint64" },
              { name: "maxEntries", type: "uint8" },
              { name: "verifierModule", type: "address" },
              { name: "verifierRewardRecipient", type: "address" },
            ],
          },
          { name: "initialFunding", type: "uint256" },
          { name: "creationNonce", type: "bytes32" },
        ],
        outputs: [
          { name: "bountyAddress", type: "address" },
          { name: "bountyId", type: "bytes32" },
        ],
      },
    ],
    value: "0",
    source_url: sourceUrl,
    title: "KeeperHub execution canary — unfunded Open Competition",
    expected_effect: {
      event: "CanonicalCompetitionCreated",
      initial_funding_usdc_units: "0",
      target_usdc_units: "110000",
      public_inventory_state: "funding_needed",
    },
    evidence_boundary:
      "This request creates one new unfunded Base Sepolia canary. It moves no USDC, changes no existing bounty, and cannot prove settlement or payment.",
  };
  return validateRequest(request);
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
  invariant(options.wallet, "--wallet is required");
  invariant(options["source-url"], "--source-url is required");
  invariant(options.output, "--output is required");
  const request = buildCanaryRequest({ wallet: options.wallet, sourceUrl: options["source-url"] });
  await writeFile(resolve(options.output), `${JSON.stringify(request, null, 2)}\n`, { flag: "wx" });
  process.stdout.write(`${resolve(options.output)}\n`);
}

const invokedAsScript = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (invokedAsScript) {
  main().catch((error) => {
    process.stderr.write(`build_keeperhub_open_competition_canary: ${error.message}\n`);
    process.exitCode = 1;
  });
}
