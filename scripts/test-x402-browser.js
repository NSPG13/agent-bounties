"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { webcrypto } = require("node:crypto");

const source = fs.readFileSync(path.join(__dirname, "..", "site", "x402-browser.js"), "utf8");
const account = "0x1111111111111111111111111111111111111111";
const bounty = "0x2222222222222222222222222222222222222222";
const usdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
const amount = "2500000";
const api = "https://api.agentbounties.app";
const resourceUrl = `${api}/v1/x402/base/bounties/${bounty}/funding?network=base-mainnet&amount=${amount}`;

function b64(value) {
  return Buffer.from(JSON.stringify(value), "utf8").toString("base64");
}

function response(status, body, headers = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...headers },
  });
}

function required(overrides = {}) {
  const accepted = {
    scheme: "agent-bounty-fund",
    network: "eip155:8453",
    asset: usdc,
    amount,
    payTo: bounty,
    maxTimeoutSeconds: 300,
    extra: {
      assetTransferMethod: "eip3009",
      name: "USD Coin",
      version: "2",
      fundingMethod: "fundWithAuthorization",
      fundingEvent: "FundingAdded",
      protocol: "agent-bounties/autonomous-v1",
    },
    ...(overrides.accepted || {}),
  };
  return {
    x402Version: 2,
    error: "Authorize Base USDC funding",
    resource: { url: resourceUrl, description: "Fund", mimeType: "application/json" },
    accepts: [accepted],
    ...(overrides.root || {}),
  };
}

function settlement() {
  return {
    success: true,
    payer: account,
    transaction: `0x${"ab".repeat(32)}`,
    network: "eip155:8453",
    amount,
  };
}

function load(fetchImpl) {
  const windowObject = {
    AgentBountiesX402: null,
    AgentBountiesLegal: {
      latestReceipt() { return { acceptance_id: "acceptance-test-123" }; },
    },
    setTimeout,
  };
  const context = {
    window: windowObject,
    fetch: fetchImpl,
    Response,
    Headers,
    Request,
    URL,
    URLSearchParams,
    TextEncoder,
    TextDecoder,
    Uint8Array,
    DOMException,
    crypto: webcrypto,
    atob: (value) => Buffer.from(value, "base64").toString("binary"),
    btoa: (value) => Buffer.from(value, "binary").toString("base64"),
    console,
  };
  vm.runInNewContext(source, context, { filename: "site/x402-browser.js" });
  return windowObject.AgentBountiesX402;
}

async function confirmedFlow() {
  const calls = [];
  const providerCalls = [];
  const apiClient = load(async (url, options = {}) => {
    calls.push({ url: String(url), options });
    if (calls.length === 1) {
      return response(402, { status: "payment_required" }, { "payment-required": b64(required()) });
    }
    if (calls.length === 2) {
      if (!options.headers?.["payment-signature"]) throw new Error("signed retry omitted payment-signature");
      return response(200, { funded: true }, { "payment-response": b64(settlement()) });
    }
    throw new Error("unexpected fetch");
  });
  const provider = {
    async request(request) {
      providerCalls.push(request);
      if (request.method !== "eth_signTypedData_v4") throw new Error(`unexpected wallet method ${request.method}`);
      const typedData = JSON.parse(request.params[1]);
      if (typedData.domain.chainId !== 8453 || typedData.domain.verifyingContract.toLowerCase() !== usdc) {
        throw new Error("typed data domain mismatch");
      }
      if (typedData.message.to.toLowerCase() !== bounty || typedData.message.value !== amount) {
        throw new Error("typed data message mismatch");
      }
      return `0x${"cd".repeat(65)}`;
    },
  };
  const result = await apiClient.fund({ apiBase: api, provider, account, bountyContract: bounty, amountBaseUnits: amount, usdcAddress: usdc });
  if (!result.confirmed || result.transactionHash !== settlement().transaction) throw new Error("confirmed result mismatch");
  if (providerCalls.length !== 1 || providerCalls[0].method !== "eth_signTypedData_v4") throw new Error("wallet should sign once");
  if (calls.some((call) => call.options.method && call.options.method !== "GET")) throw new Error("x402 funding must use GET retries");
  if (calls.some((call) => call.options.headers?.["x-agent-bounties-legal-acceptance"] !== "acceptance-test-123")) {
    throw new Error("x402 funding did not preserve the reviewed legal acceptance receipt");
  }
  if (providerCalls.some((call) => call.method === "eth_sendTransaction")) throw new Error("user wallet must not pay gas or broadcast");
}

async function pendingFlow() {
  let call = 0;
  const apiClient = load(async (url, options = {}) => {
    call += 1;
    if (call === 1) return response(402, {}, { "payment-required": b64(required()) });
    if (call === 2) return response(202, { relay: { id: "relay-123" } });
    if (String(url).endsWith("/v1/x402/base/relays/relay-123")) {
      return response(200, {}, { "payment-response": b64(settlement()) });
    }
    throw new Error(`unexpected pending fetch ${url}`);
  });
  const provider = { request: async () => `0x${"ef".repeat(65)}` };
  const result = await apiClient.fund({ apiBase: api, provider, account, bountyContract: bounty, amountBaseUnits: amount, usdcAddress: usdc });
  if (!result.confirmed) throw new Error("pending relay did not reconcile to canonical confirmation");
}

async function rejectsTamperedChallenge() {
  let calls = 0;
  const apiClient = load(async () => {
    calls += 1;
    return response(402, {}, { "payment-required": b64(required({ accepted: { payTo: account } })) });
  });
  const provider = { request: async () => { throw new Error("wallet must not be asked to sign a tampered challenge"); } };
  let rejected = false;
  try {
    await apiClient.fund({ apiBase: api, provider, account, bountyContract: bounty, amountBaseUnits: amount, usdcAddress: usdc });
  } catch (error) {
    rejected = error.code === "requirements_mismatch";
  }
  if (!rejected || calls !== 1) throw new Error("tampered challenge was not rejected before signing");
}

(async () => {
  await confirmedFlow();
  await pendingFlow();
  await rejectsTamperedChallenge();
  console.log("Browser x402 funding signs exact EIP-3009 data, uses the gas relay, and waits for canonical confirmation");
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
