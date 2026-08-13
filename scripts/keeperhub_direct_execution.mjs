#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const REQUEST_SCHEMA = "agent-bounties/keeperhub-direct-execution-request-v1";
export const RECEIPT_SCHEMA = "agent-bounties/keeperhub-direct-execution-receipt-v1";
export const BASE_SEPOLIA_CHAIN_ID = 84532;
export const BASE_SEPOLIA_OPEN_COMPETITION_FACTORY =
  "0x7231f1312448fa60078fb56cdb6e2c392bd1269b";

const DEFAULT_BASE_URL = "https://app.keeperhub.com";
const TERMINAL_STATUSES = new Set(["completed", "failed"]);

function invariant(condition, message) {
  if (!condition) throw new Error(message);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isAddress(value) {
  return typeof value === "string" && /^0x[0-9a-fA-F]{40}$/.test(value);
}

function isBytes32(value) {
  return typeof value === "string" && /^0x[0-9a-fA-F]{64}$/.test(value);
}

function canonicalize(value) {
  if (Array.isArray(value)) return value.map(canonicalize);
  if (!isPlainObject(value)) return value;
  return Object.fromEntries(
    Object.keys(value)
      .sort()
      .map((key) => [key, canonicalize(value[key])]),
  );
}

export function requestFingerprint(request) {
  return `sha256:${createHash("sha256")
    .update(JSON.stringify(canonicalize(request)))
    .digest("hex")}`;
}

export function validateRequest(request) {
  invariant(isPlainObject(request), "request must be a JSON object");
  invariant(request.schema_version === REQUEST_SCHEMA, "unsupported request schema");
  invariant(request.operation === "contract_call", "only contract_call is supported");
  invariant(
    Number(request.chain_id) === BASE_SEPOLIA_CHAIN_ID,
    "KeeperHub canary execution is restricted to Base Sepolia (84532)",
  );
  invariant(
    String(request.contract_address).toLowerCase() === BASE_SEPOLIA_OPEN_COMPETITION_FACTORY,
    "contract address is not the rehearsed Base Sepolia Open Competition factory",
  );
  invariant(request.function_name === "createCompetition", "function is not createCompetition");
  invariant(Array.isArray(request.function_args), "function_args must be a JSON array");
  invariant(request.function_args.length === 3, "createCompetition requires exactly three arguments");
  invariant(
    request.function_args[1] === "0" || request.function_args[1] === 0,
    "the hackathon canary must have zero initial funding",
  );
  invariant(isBytes32(request.function_args[2]), "creation nonce must be bytes32");
  invariant(Array.isArray(request.abi) && request.abi.length > 0, "abi must be a non-empty JSON array");
  invariant(request.value === undefined || request.value === "0", "native value must be zero");
  invariant(
    typeof request.evidence_boundary === "string" && request.evidence_boundary.length >= 40,
    "evidence_boundary is required",
  );
  invariant(request.simulate === undefined, "simulate is controlled by the command, not the request file");
  return request;
}

export function buildKeeperHubBody(request, simulate) {
  validateRequest(request);
  invariant(typeof simulate === "boolean", "simulate must be a boolean");
  return {
    contractAddress: request.contract_address,
    chainId: Number(request.chain_id),
    functionName: request.function_name,
    functionArgs: JSON.stringify(request.function_args),
    abi: JSON.stringify(request.abi),
    value: "0",
    simulate,
  };
}

function validateApiKey(apiKey) {
  invariant(typeof apiKey === "string" && /^kh_[A-Za-z0-9_-]{8,}$/.test(apiKey), "KH_API_KEY is missing or invalid");
}

function validateBaseUrl(baseUrl) {
  const url = new URL(baseUrl);
  invariant(url.protocol === "https:" || url.hostname === "127.0.0.1", "KeeperHub base URL must use HTTPS");
  return url.origin;
}

async function parseResponse(response) {
  const text = await response.text();
  let body = null;
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { detail: text.slice(0, 500) };
  }
  if (!response.ok) {
    const code = body?.error || body?.code || `http_${response.status}`;
    const requestId = body?.request_id || response.headers.get("x-request-id") || "unknown";
    throw new Error(`KeeperHub request failed: ${code} (request_id=${requestId})`);
  }
  return body;
}

async function keeperFetch({ fetchImpl, baseUrl, apiKey, path, method, body, idempotencyKey }) {
  validateApiKey(apiKey);
  const headers = {
    Authorization: `Bearer ${apiKey}`,
    Accept: "application/json",
    "Content-Type": "application/json",
    "x-request-id": `agent-bounties-${crypto.randomUUID()}`,
  };
  if (idempotencyKey) headers["Idempotency-Key"] = idempotencyKey;
  const response = await fetchImpl(`${validateBaseUrl(baseUrl)}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return { response, body: await parseResponse(response) };
}

export async function simulateRequest({
  request,
  apiKey,
  fetchImpl = fetch,
  baseUrl = DEFAULT_BASE_URL,
}) {
  const { body } = await keeperFetch({
    fetchImpl,
    baseUrl,
    apiKey,
    path: "/api/execute/contract-call",
    method: "POST",
    body: buildKeeperHubBody(request, true),
  });
  invariant(body.success === true, "KeeperHub simulation did not report success");
  invariant(body.wouldRevert === false, "KeeperHub simulation would revert");
  return {
    schema_version: RECEIPT_SCHEMA,
    mode: "simulation",
    request_fingerprint: requestFingerprint(request),
    chain_id: BASE_SEPOLIA_CHAIN_ID,
    contract_address: BASE_SEPOLIA_OPEN_COMPETITION_FACTORY,
    provider: "KeeperHub",
    simulation: body,
    evidence_boundary:
      "A simulation is not an onchain transaction, bounty funding, settlement, or payment evidence.",
  };
}

export async function executeRequest({
  request,
  apiKey,
  idempotencyKey,
  fetchImpl = fetch,
  baseUrl = DEFAULT_BASE_URL,
  sleep = (milliseconds) => new Promise((resolveSleep) => setTimeout(resolveSleep, milliseconds)),
  maxPolls = 60,
}) {
  invariant(
    typeof idempotencyKey === "string" && /^[A-Za-z0-9:._-]{16,128}$/.test(idempotencyKey),
    "idempotency key must be 16-128 safe characters",
  );
  const keeperBody = buildKeeperHubBody(request, false);
  delete keeperBody.simulate;
  const { body: accepted } = await keeperFetch({
    fetchImpl,
    baseUrl,
    apiKey,
    path: "/api/execute/contract-call",
    method: "POST",
    body: keeperBody,
    idempotencyKey,
  });
  invariant(typeof accepted.executionId === "string" && accepted.executionId.length > 0, "missing executionId");

  let statusBody = null;
  for (let poll = 0; poll < maxPolls; poll += 1) {
    const { response, body } = await keeperFetch({
      fetchImpl,
      baseUrl,
      apiKey,
      path: `/api/execute/${encodeURIComponent(accepted.executionId)}/status`,
      method: "GET",
    });
    statusBody = body;
    if (TERMINAL_STATUSES.has(body.status)) break;
    const hintSeconds = Number(response.headers.get("x-poll-interval-hint") || "2");
    const boundedMilliseconds = Math.min(Math.max(hintSeconds, 1), 10) * 1_000;
    await sleep(boundedMilliseconds);
  }

  invariant(statusBody !== null, "KeeperHub status was not returned");
  invariant(statusBody.status === "completed", `KeeperHub execution ended as ${statusBody.status || "unknown"}`);
  invariant(/^0x[0-9a-fA-F]{64}$/.test(statusBody.transactionHash), "completed execution has no transaction hash");
  invariant(
    typeof statusBody.transactionLink === "string" && statusBody.transactionLink.startsWith("https://"),
    "completed execution has no explorer link",
  );

  return {
    schema_version: RECEIPT_SCHEMA,
    mode: "execution",
    provider: "KeeperHub",
    request_fingerprint: requestFingerprint(request),
    idempotency_key: idempotencyKey,
    execution_id: accepted.executionId,
    chain_id: BASE_SEPOLIA_CHAIN_ID,
    contract_address: BASE_SEPOLIA_OPEN_COMPETITION_FACTORY,
    status: statusBody.status,
    transaction_hash: statusBody.transactionHash,
    transaction_link: statusBody.transactionLink,
    completed_at: statusBody.completedAt || null,
    gas_used_wei: statusBody.gasUsedWei || null,
    expected_effect: request.expected_effect || null,
    evidence_boundary:
      "This receipt proves one KeeperHub-submitted Base Sepolia transaction. It does not prove bounty funding, solver settlement, or payment.",
  };
}

function parseCliArgs(argv) {
  const command = argv[0];
  const options = {};
  for (let index = 1; index < argv.length; index += 1) {
    const key = argv[index];
    invariant(key.startsWith("--"), `unexpected argument: ${key}`);
    const value = argv[index + 1];
    invariant(value !== undefined && !value.startsWith("--"), `missing value for ${key}`);
    options[key.slice(2)] = value;
    index += 1;
  }
  return { command, options };
}

async function main() {
  const { command, options } = parseCliArgs(process.argv.slice(2));
  invariant(command === "simulate" || command === "execute", "usage: keeperhub_direct_execution.mjs <simulate|execute> --request FILE [options]");
  invariant(options.request, "--request is required");
  const request = validateRequest(JSON.parse(await readFile(resolve(options.request), "utf8")));
  const apiKey = process.env.KH_API_KEY;
  let receipt;
  if (command === "simulate") {
    receipt = await simulateRequest({ request, apiKey });
  } else {
    invariant(options["idempotency-key"], "--idempotency-key is required for execution");
    invariant(options.receipt, "--receipt is required for execution");
    receipt = await executeRequest({
      request,
      apiKey,
      idempotencyKey: options["idempotency-key"],
    });
    await writeFile(resolve(options.receipt), `${JSON.stringify(receipt, null, 2)}\n`, { flag: "wx" });
  }
  process.stdout.write(`${JSON.stringify(receipt, null, 2)}\n`);
}

const invokedAsScript = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (invokedAsScript) {
  main().catch((error) => {
    process.stderr.write(`keeperhub_direct_execution: ${error.message}\n`);
    process.exitCode = 1;
  });
}
