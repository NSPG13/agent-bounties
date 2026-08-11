(() => {
  "use strict";

  const BASE_CHAIN_ID = 8453;
  const BASE_NETWORK = "base-mainnet";
  const CAIP2_NETWORK = "eip155:8453";
  const SCHEME = "agent-bounty-fund";
  const HEADER_LIMIT = 32 * 1024;
  const POLL_INTERVAL_MS = 1_250;
  const POLL_TIMEOUT_MS = 75_000;
  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const BYTES32 = /^0x[0-9a-fA-F]{64}$/;

  class X402FundingError extends Error {
    constructor(message, code = "x402_funding_error", details = null) {
      super(message);
      this.name = "X402FundingError";
      this.code = code;
      this.details = details;
    }
  }

  function normalizeAddress(value, label) {
    const normalized = String(value || "").trim().toLowerCase();
    if (!ADDRESS.test(normalized)) throw new X402FundingError(`${label} is not a valid EVM address.`, "invalid_address");
    return normalized;
  }

  function normalizePositiveInteger(value, label) {
    const normalized = String(value ?? "").trim();
    if (!/^[1-9]\d*$/.test(normalized)) {
      throw new X402FundingError(`${label} must be a positive base-unit integer.`, "invalid_amount");
    }
    return normalized;
  }

  function decodeBase64Json(header, label) {
    const value = String(header || "").trim();
    if (!value) throw new X402FundingError(`${label} is missing.`, "missing_header");
    if (value.length > HEADER_LIMIT) throw new X402FundingError(`${label} exceeds the safety limit.`, "header_too_large");
    try {
      const binary = atob(value);
      const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
      const decoded = JSON.parse(new TextDecoder().decode(bytes));
      if (!decoded || typeof decoded !== "object" || Array.isArray(decoded)) throw new Error("not an object");
      return decoded;
    } catch (_error) {
      throw new X402FundingError(`${label} is not valid base64 JSON.`, "invalid_header");
    }
  }

  function encodeBase64Json(value) {
    const bytes = new TextEncoder().encode(JSON.stringify(value));
    let binary = "";
    for (let index = 0; index < bytes.length; index += 0x8000) {
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    return btoa(binary);
  }

  async function jsonBody(response) {
    try {
      const body = await response.clone().json();
      return body && typeof body === "object" && !Array.isArray(body) ? body : {};
    } catch (_error) {
      return {};
    }
  }

  function legalHeaders() {
    const receipt = window.AgentBountiesLegal?.latestReceipt?.();
    const acceptanceId = String(receipt?.acceptance_id || "").trim();
    return acceptanceId ? { "x-agent-bounties-legal-acceptance": acceptanceId } : {};
  }

  function exactResourceUrl(apiBase, bountyContract, amountBaseUnits) {
    const url = new URL(`/v1/x402/base/bounties/${bountyContract}/funding`, `${String(apiBase).replace(/\/$/, "")}/`);
    url.searchParams.set("network", BASE_NETWORK);
    url.searchParams.set("amount", amountBaseUnits);
    return url;
  }

  function validateChallenge(required, expected) {
    if (required.x402Version !== 2) throw new X402FundingError("The server returned an unsupported x402 version.", "unsupported_version");
    if (!Array.isArray(required.accepts) || required.accepts.length !== 1) {
      throw new X402FundingError("The server returned an ambiguous funding challenge.", "ambiguous_challenge");
    }
    const accepted = required.accepts[0];
    const extra = accepted?.extra || {};
    const expectedUrl = new URL(expected.resourceUrl);
    let resourceUrl;
    try {
      resourceUrl = new URL(required.resource?.url);
    } catch (_error) {
      throw new X402FundingError("The funding challenge resource URL is invalid.", "resource_mismatch");
    }
    const expectedParams = [...expectedUrl.searchParams.entries()].sort();
    const resourceParams = [...resourceUrl.searchParams.entries()].sort();
    const resourceMatches = resourceUrl.origin === expectedUrl.origin
      && resourceUrl.pathname === expectedUrl.pathname
      && JSON.stringify(resourceParams) === JSON.stringify(expectedParams);
    if (!resourceMatches) throw new X402FundingError("The funding challenge resource does not match this request.", "resource_mismatch");

    const maxTimeoutSeconds = Number(accepted.maxTimeoutSeconds);
    if (!Number.isInteger(maxTimeoutSeconds) || maxTimeoutSeconds < 15 || maxTimeoutSeconds > 900) {
      throw new X402FundingError("The funding authorization timeout is outside the accepted safety range.", "invalid_timeout");
    }
    if (accepted.scheme !== SCHEME
      || accepted.network !== CAIP2_NETWORK
      || normalizeAddress(accepted.asset, "Challenge asset") !== expected.usdc
      || normalizePositiveInteger(accepted.amount, "Challenge amount") !== expected.amount
      || normalizeAddress(accepted.payTo, "Challenge recipient") !== expected.bountyContract
      || extra.assetTransferMethod !== "eip3009"
      || extra.fundingMethod !== "fundWithAuthorization"
      || extra.fundingEvent !== "FundingAdded"
      || extra.name !== "USD Coin"
      || String(extra.version) !== "2"
      || extra.protocol !== "agent-bounties/autonomous-v1") {
      throw new X402FundingError("The funding challenge does not exactly match Agent Bounties' canonical Base USDC path.", "requirements_mismatch");
    }
    return { accepted, maxTimeoutSeconds };
  }

  function randomNonce() {
    const bytes = new Uint8Array(32);
    crypto.getRandomValues(bytes);
    return `0x${[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }

  function authorizationFor(account, accepted, maxTimeoutSeconds) {
    const now = Math.floor(Date.now() / 1000);
    const validBefore = now + Math.max(7, maxTimeoutSeconds - 10);
    return {
      from: account,
      to: normalizeAddress(accepted.payTo, "Authorization recipient"),
      value: normalizePositiveInteger(accepted.amount, "Authorization amount"),
      validAfter: "0",
      validBefore: String(validBefore),
      nonce: randomNonce(),
    };
  }

  function typedDataFor(accepted, authorization) {
    return {
      types: {
        EIP712Domain: [
          { name: "name", type: "string" },
          { name: "version", type: "string" },
          { name: "chainId", type: "uint256" },
          { name: "verifyingContract", type: "address" },
        ],
        TransferWithAuthorization: [
          { name: "from", type: "address" },
          { name: "to", type: "address" },
          { name: "value", type: "uint256" },
          { name: "validAfter", type: "uint256" },
          { name: "validBefore", type: "uint256" },
          { name: "nonce", type: "bytes32" },
        ],
      },
      primaryType: "TransferWithAuthorization",
      domain: {
        name: accepted.extra.name,
        version: String(accepted.extra.version),
        chainId: BASE_CHAIN_ID,
        verifyingContract: normalizeAddress(accepted.asset, "USDC contract"),
      },
      message: authorization,
    };
  }

  function relayId(body) {
    const direct = body?.relay?.id;
    if (typeof direct === "string" && direct.trim()) return direct.trim();
    if (typeof body?.statusUrl === "string") {
      try {
        const url = new URL(body.statusUrl);
        return url.pathname.replace(/\/$/, "").split("/").pop() || null;
      } catch (_error) {
        return null;
      }
    }
    return null;
  }

  async function assertConfirmed(response, expected) {
    const paymentResponse = decodeBase64Json(response.headers.get("payment-response"), "PAYMENT-RESPONSE");
    if (paymentResponse.success !== true
      || paymentResponse.network !== CAIP2_NETWORK
      || normalizeAddress(paymentResponse.payer, "Settlement payer") !== expected.account
      || (paymentResponse.amount != null && String(paymentResponse.amount) !== expected.amount)
      || !/^0x[0-9a-fA-F]{64}$/.test(String(paymentResponse.transaction || ""))) {
      throw new X402FundingError("The hosted relay response does not prove the expected canonical funding.", "invalid_settlement_response", paymentResponse);
    }
    return {
      confirmed: true,
      transactionHash: paymentResponse.transaction,
      paymentResponse,
      body: await jsonBody(response),
    };
  }

  async function pollRelay(apiBase, id, expected, signal) {
    const started = Date.now();
    while (Date.now() - started < POLL_TIMEOUT_MS) {
      if (signal?.aborted) throw new DOMException("Funding cancelled.", "AbortError");
      await new Promise((resolve) => window.setTimeout(resolve, POLL_INTERVAL_MS));
      const url = new URL(`/v1/x402/base/relays/${encodeURIComponent(id)}`, `${String(apiBase).replace(/\/$/, "")}/`);
      const response = await fetch(url, {
        method: "GET",
        mode: "cors",
        credentials: "omit",
        cache: "no-store",
        signal,
        headers: legalHeaders(),
      });
      if (response.status === 200) return assertConfirmed(response, expected);
      if (response.status === 202 || response.status === 503) continue;
      const body = await jsonBody(response);
      throw new X402FundingError(body.error || `Funding relay failed with HTTP ${response.status}.`, "relay_failed", body);
    }
    throw new X402FundingError("The contribution was authorized, but canonical FundingAdded was not confirmed before the polling window ended. Check status before trying again.", "relay_timeout");
  }

  async function fund(options) {
    const provider = options?.provider;
    if (!provider || typeof provider.request !== "function") throw new X402FundingError("An EIP-1193 wallet provider is required.", "missing_provider");
    const account = normalizeAddress(options.account, "Contributor wallet");
    const bountyContract = normalizeAddress(options.bountyContract, "Bounty contract");
    const amount = normalizePositiveInteger(options.amountBaseUnits, "Contribution amount");
    const usdc = normalizeAddress(options.usdcAddress, "Base USDC contract");
    const resource = exactResourceUrl(options.apiBase, bountyContract, amount);
    const expected = { account, bountyContract, amount, usdc, resourceUrl: resource.href };

    const challenge = await fetch(resource, {
      method: "GET",
      mode: "cors",
      credentials: "omit",
      cache: "no-store",
      signal: options.signal,
      headers: legalHeaders(),
    });
    const challengeBody = await jsonBody(challenge);
    if (challenge.status !== 402) {
      throw new X402FundingError(
        challengeBody.error || `The hosted funding relay did not issue a 402 challenge (HTTP ${challenge.status}).`,
        "challenge_failed",
        challengeBody,
      );
    }
    const required = decodeBase64Json(challenge.headers.get("payment-required"), "PAYMENT-REQUIRED");
    const { accepted, maxTimeoutSeconds } = validateChallenge(required, expected);
    const authorization = authorizationFor(account, accepted, maxTimeoutSeconds);
    if (!BYTES32.test(authorization.nonce)) throw new X402FundingError("The generated authorization nonce is invalid.", "invalid_nonce");
    const typedData = typedDataFor(accepted, authorization);
    const signature = await provider.request({
      method: "eth_signTypedData_v4",
      params: [account, JSON.stringify(typedData)],
    });
    if (!/^0x[0-9a-fA-F]{130}$/.test(String(signature || ""))) {
      throw new X402FundingError("The wallet returned an invalid EIP-3009 signature.", "invalid_signature");
    }
    const paymentPayload = {
      x402Version: 2,
      resource: required.resource,
      accepted,
      payload: { signature, authorization },
      ...(required.extensions == null ? {} : { extensions: required.extensions }),
    };
    const paymentSignature = encodeBase64Json(paymentPayload);
    if (paymentSignature.length > HEADER_LIMIT) throw new X402FundingError("The signed payment header exceeds the safety limit.", "header_too_large");

    const retry = await fetch(resource, {
      method: "GET",
      mode: "cors",
      credentials: "omit",
      cache: "no-store",
      signal: options.signal,
      headers: { ...legalHeaders(), "payment-signature": paymentSignature },
    });
    if (retry.status === 200) return assertConfirmed(retry, expected);
    const retryBody = await jsonBody(retry);
    if (retry.status === 202 || retry.status === 503) {
      const id = relayId(retryBody);
      if (!id) throw new X402FundingError("The pending hosted relay response did not include a durable relay identifier.", "missing_relay_id", retryBody);
      return pollRelay(options.apiBase, id, expected, options.signal);
    }
    const reason = retry.status === 429
      ? "The gas-sponsored funding relay has reached its bounded rolling quota. No funding was recorded."
      : retryBody.error || `The funding authorization was rejected with HTTP ${retry.status}.`;
    throw new X402FundingError(reason, "authorization_rejected", retryBody);
  }

  window.AgentBountiesX402 = Object.freeze({ fund, X402FundingError });
})();
