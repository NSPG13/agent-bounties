#!/usr/bin/env node

import assert from "node:assert/strict";

import { buildCanaryRequest } from "./build_keeperhub_open_competition_canary.mjs";
import {
  buildKeeperHubBody,
  executeRequest,
  simulateRequest,
  validateRequest,
} from "./keeperhub_direct_execution.mjs";

const WALLET = "0x884834e884d6e93462655a2820140ad03e6747bc";
const API_KEY = "kh_test_key_for_adapter";
const TX_HASH = `0x${"ab".repeat(32)}`;

function response(status, body, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

const request = buildCanaryRequest({
  wallet: WALLET,
  sourceUrl: "https://github.com/NSPG13/agent-bounties/issues/931",
  nowSeconds: 1_786_600_000,
});
validateRequest(request);

const simulationBody = buildKeeperHubBody(request, true);
assert.equal(simulationBody.chainId, 84532);
assert.equal(simulationBody.simulate, true);
assert.equal(JSON.parse(simulationBody.functionArgs)[1], "0");

assert.throws(
  () => validateRequest({ ...request, chain_id: 8453 }),
  /restricted to Base Sepolia/,
);
assert.throws(
  () => validateRequest({ ...request, function_args: [request.function_args[0], "1", request.function_args[2]] }),
  /zero initial funding/,
);

const simulateCalls = [];
const simulated = await simulateRequest({
  request,
  apiKey: API_KEY,
  baseUrl: "https://keeperhub.test",
  fetchImpl: async (url, init) => {
    simulateCalls.push({ url, init });
    return response(200, {
      success: true,
      status: "simulated",
      from: WALLET,
      to: request.contract_address,
      value: "0",
      gasEstimate: "180000",
      simulatedReturnValue: ["0x0000000000000000000000000000000000000001", `0x${"cd".repeat(32)}`],
      wouldRevert: false,
    });
  },
});
assert.equal(simulated.mode, "simulation");
assert.equal(simulateCalls.length, 1);
assert.equal(JSON.parse(simulateCalls[0].init.body).simulate, true);
assert.match(simulateCalls[0].init.headers.Authorization, /^Bearer kh_/);

const executeCalls = [];
const executed = await executeRequest({
  request,
  apiKey: API_KEY,
  idempotencyKey: "keeperhub-test-0001",
  baseUrl: "https://keeperhub.test",
  sleep: async () => {},
  fetchImpl: async (url, init) => {
    executeCalls.push({ url, init });
    if (url.endsWith("/api/execute/contract-call")) {
      return response(202, { executionId: "direct_test_1", status: "completed" });
    }
    return response(
      200,
      {
        executionId: "direct_test_1",
        status: "completed",
        type: "contract-call",
        transactionHash: TX_HASH,
        transactionLink: `https://sepolia.basescan.org/tx/${TX_HASH}`,
        gasUsedWei: "12345",
        completedAt: "2026-08-13T00:00:00Z",
      },
      { "x-poll-interval-hint": "0" },
    );
  },
});
assert.equal(executed.status, "completed");
assert.equal(executed.transaction_hash, TX_HASH);
assert.equal(executeCalls.length, 2);
assert.equal(executeCalls[0].init.headers["Idempotency-Key"], "keeperhub-test-0001");
assert.equal(Object.hasOwn(JSON.parse(executeCalls[0].init.body), "simulate"), false);
assert.equal(executeCalls[1].init.method, "GET");

const serialized = JSON.stringify(executed);
assert.equal(serialized.includes(API_KEY), false);
console.log("keeperhub_direct_execution_tests=ok");
