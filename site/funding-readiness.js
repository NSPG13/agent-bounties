(() => {
  "use strict";

  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
  const BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
  const HEX = /^0x[0-9a-f]+$/i;

  function parseUsdc(value) {
    const text = String(value ?? "").trim();
    if (!/^\d+(?:\.\d{1,6})?$/.test(text)) throw new Error("Enter a USDC amount with at most six decimal places.");
    const [whole, fraction = ""] = text.split(".");
    return BigInt(whole) * 1_000_000n + BigInt(fraction.padEnd(6, "0"));
  }

  function formatUnits(value, decimals = 6, digits = decimals) {
    value = BigInt(value);
    const absolute = value < 0n ? -value : value;
    const scale = 10n ** BigInt(decimals);
    const fraction = (absolute % scale).toString().padStart(decimals, "0").slice(0, digits).replace(/0+$/, "");
    return `${value < 0n ? "-" : ""}${absolute / scale}${fraction ? `.${fraction}` : ""}`;
  }

  function shortfall(required, balance) {
    const target = BigInt(required);
    const available = BigInt(balance);
    if (target < 0n || available < 0n) throw new Error("Balances and required amounts must be nonnegative.");
    return target > available ? target - available : 0n;
  }

  async function readBalances({ wallet, usdcAddress = BASE_USDC, provider = null, timeoutMs = 12000 } = {}) {
    if (!ADDRESS.test(wallet || "")) throw new Error("A valid public Base wallet address is required.");
    if (String(usdcAddress).toLowerCase() !== BASE_USDC) throw new Error("Balance checks require native Base USDC.");
    const controller = new AbortController();
    let timeout;
    const request = async (method, params) => {
      let result;
      if (provider) {
        result = await provider.request({ method, params });
      } else {
        const response = await fetch("https://mainnet.base.org", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }),
          cache: "no-store", credentials: "omit", referrerPolicy: "no-referrer", signal: controller.signal,
        });
        const payload = await response.json();
        if (!response.ok || payload?.error) throw new Error("Base balances are temporarily unavailable. Retry the balance check.");
        result = payload?.result;
      }
      if (typeof result !== "string" || !HEX.test(result)) throw new Error("Base returned an invalid balance response.");
      return result;
    };
    try {
      return await Promise.race([
        (async () => {
          const chainId = await request("eth_chainId", []);
          if (BigInt(chainId) !== 8453n) throw new Error("Choose Base mainnet before checking funding readiness.");
          const blockNumber = await request("eth_blockNumber", []);
          const [eth, usdc] = await Promise.all([
            request("eth_getBalance", [wallet, blockNumber]),
            request("eth_call", [{ to: BASE_USDC, data: `0x70a08231${wallet.slice(2).toLowerCase().padStart(64, "0")}` }, blockNumber]),
          ]);
          return { wallet: wallet.toLowerCase(), usdc: BigInt(usdc), eth: BigInt(eth), blockNumber, observedAt: new Date().toISOString() };
        })(),
        new Promise((_, reject) => {
          timeout = setTimeout(() => {
            controller.abort();
            reject(new Error("Base balance check timed out. Retry; no purchase or transaction was requested."));
          }, timeoutMs);
        }),
      ]);
    } finally {
      clearTimeout(timeout);
      controller.abort();
    }
  }

  async function estimateFees({ provider, wallet, calls, timeoutMs = 12000 } = {}) {
    if (!provider || typeof provider.request !== "function" || !ADDRESS.test(wallet || "")) throw new Error("A connected Base wallet is required for fee estimates.");
    if (!Array.isArray(calls) || !calls.length || calls.length > 8) throw new Error("Provide the exact bounded transaction calls to estimate.");
    const oracle = "0x420000000000000000000000000000000000000f";
    const notice = "Estimated separate-transaction fees at current Base rates, including L2 execution, L1 data and the operator fee. Wallet batching and later prices may differ. This is not a guaranteed maximum or confirmed sponsorship.";
    const results = calls.map((call) => ({ to: call?.to, gasUnits: null, l2ExecutionWei: null, l1DataWei: null, operatorWei: null, estimatedTotalWei: null, error: null }));
    const observedAt = new Date().toISOString();
    let timer;
    let expired = false;
    const request = async (method, params) => {
      if (expired) throw new Error("Fee estimate timed out.");
      const result = await provider.request({ method, params });
      if (typeof result !== "string" || !HEX.test(result)) throw new Error("Base returned an invalid fee estimate.");
      return result;
    };
    const estimate = async () => {
      if (BigInt(await request("eth_chainId", [])) !== 8453n) throw new Error("Fee estimation requires Base mainnet.");
      const [block, price] = await Promise.all([request("eth_blockNumber", []), request("eth_gasPrice", [])]);
      await Promise.all(calls.map(async (call, index) => {
        const result = results[index];
        try {
          if (!ADDRESS.test(call?.to || "") || !/^0x(?:[0-9a-f]{2})*$/i.test(call.data || "0x")) throw new Error("The exact transaction destination or calldata is invalid.");
          if ((call.accessList?.length || call.authorizationList?.length) || (call.type && !["0x2", "2", 2].includes(call.type))) throw new Error("This transaction type needs the wallet's fee estimate.");
          const value = BigInt(call.value || "0x0");
          if (value < 0n || value >= (1n << 256n)) throw new Error("The transaction value is invalid.");
          const transaction = { from: wallet, to: call.to, data: call.data || "0x", value: `0x${value.toString(16)}` };
          // Ordinary type-2 call, empty access list: 256 bytes conservatively
          // cover the unsigned RLP envelope. The oracle accounts for signing
          // bytes itself. This size allowance is not a guaranteed fee cap.
          const unsignedSize = BigInt((transaction.data.length - 2) / 2 + 256);
          const [gasRead, l1Read] = await Promise.allSettled([
            request("eth_estimateGas", [transaction, block]),
            request("eth_call", [{ to: oracle, data: `0xf1c7a58b${unsignedSize.toString(16).padStart(64, "0")}` }, block]),
          ]);
          if (l1Read.status === "fulfilled") result.l1DataWei = BigInt(l1Read.value);
          if (gasRead.status !== "fulfilled") throw new Error("Exact-call simulation failed; an earlier approval or account state may be required. Re-estimate after that call confirms.");
          result.gasUnits = BigInt(gasRead.value);
          result.l2ExecutionWei = result.gasUnits * BigInt(price);
          // GasPriceOracle implements the active OP Stack operator-fee formula.
          // A failed read remains unknown; it is never assumed to mean zero.
          result.operatorWei = BigInt(await request("eth_call", [{ to: oracle, data: `0x275aedd2${result.gasUnits.toString(16).padStart(64, "0")}` }, block]));
          if (l1Read.status !== "fulfilled") throw new Error("The Base L1 data fee is unavailable. Review the wallet's full fee before confirming.");
          result.estimatedTotalWei = result.l2ExecutionWei + result.l1DataWei + result.operatorWei;
        } catch (error) {
          result.error = error.message || "The complete fee estimate is unavailable.";
        }
      }));
      const complete = results.every((result) => result.estimatedTotalWei !== null);
      return { status: complete ? "estimated" : "partial", calls: results, estimatedTotalWei: complete ? results.reduce((sum, result) => sum + result.estimatedTotalWei, 0n) : null, maximumWei: null, blockNumber: block, observedAt, notice };
    };
    try {
      return await Promise.race([
        estimate(),
        new Promise((_, reject) => { timer = setTimeout(() => { expired = true; reject(new Error("Fee estimation timed out. The full wallet cost is still unknown.")); }, timeoutMs); }),
      ]);
    } catch (error) {
      return { status: "unavailable", calls: results, estimatedTotalWei: null, maximumWei: null, observedAt, notice, error: error.message || "Fee estimation is unavailable." };
    } finally { clearTimeout(timer); }
  }

  function describeWalletCalls({ calls, context } = {}) {
    if (!context || Number(context.chainId) !== 8453 || String(context.usdcAddress).toLowerCase() !== BASE_USDC
      || !ADDRESS.test(context.factoryAddress || "") || !ADDRESS.test(context.bountyAddress || "")
      || !Array.isArray(context.validatedCalls) || !Array.isArray(calls) || !calls.length || calls.length > 8) {
      throw new Error("The exact validated Base posting plan is required for wallet review.");
    }
    const amount = BigInt(context.fundingUsdcUnits);
    if (amount <= 0n || amount >= (1n << 256n)) throw new Error("The reviewed funding amount is invalid.");
    const lower = (value) => String(value || "").toLowerCase();
    const value = (call) => BigInt(call.value_wei ?? call.value ?? 0);
    const word = (data, index) => {
      const encoded = data.slice(10 + index * 64, 10 + (index + 1) * 64);
      if (!/^[0-9a-f]{64}$/.test(encoded)) throw new Error("The wallet call is missing committed arguments.");
      return BigInt(`0x${encoded}`);
    };
    const timestamp = (seconds) => {
      if (seconds <= 0n || seconds > 8640000000000n) throw new Error("The wallet call has an invalid deadline.");
      return new Date(Number(seconds) * 1000).toISOString();
    };
    const descriptions = calls.map((call) => {
      const data = lower(call?.data), to = lower(call?.to);
      if (!ADDRESS.test(to) || !/^0x(?:[0-9a-f]{2})+$/.test(data) || value(call) !== 0n
        || !context.validatedCalls.some((approved) => lower(approved.to) === to && lower(approved.data) === data && value(approved) === 0n)) {
        throw new Error("The wallet request differs from the validated posting plan.");
      }
      const selector = data.slice(0, 10);
      if (selector === "0x095ea7b3" && to === BASE_USDC) {
        if (data.length !== 138 || data.slice(10, 34) !== "0".repeat(24)) throw new Error("The USDC allowance call is malformed.");
        const spender = `0x${data.slice(34, 74)}`, allowance = word(data, 1);
        if (![lower(context.factoryAddress), lower(context.bountyAddress)].includes(spender) || allowance > amount) throw new Error("The USDC allowance exceeds the reviewed amount or names another spender.");
        return { kind: "allowance", amountUsdcUnits: allowance, movesUsdc: false, recipient: null, spender, expiresAt: null,
          summary: `Approve an allowance of ${formatUnits(allowance)} Base USDC for spender ${spender}. No USDC moves in this transaction. Token contract: ${BASE_USDC}. The allowance has no automatic expiry; it remains until spent or changed.` };
      }
      if (["0x9d2e414c", "0x61407894"].includes(selector) && to === lower(context.factoryAddress)) {
        const authorized = selector === "0x61407894";
        if (word(data, authorized ? 16 : 15) !== amount) throw new Error("The creation amount differs from the reviewed funding.");
        if (authorized && context.creatorAddress && `0x${data.slice(34, 74)}` !== lower(context.creatorAddress)) throw new Error("The funding authorization names another creator.");
        const deadline = timestamp(word(data, authorized ? 8 : 7));
        const expiresAt = authorized ? timestamp(word(data, 19)) : deadline;
        return { kind: "creation", amountUsdcUnits: amount, movesUsdc: true, recipient: lower(context.bountyAddress), spender: lower(context.factoryAddress), expiresAt,
          summary: `Create the reviewed bounty and transfer ${formatUnits(amount)} Base USDC to ${lower(context.bountyAddress)} through canonical factory ${to}. Contract funding deadline: ${deadline}.${authorized ? ` The USDC authorization expires ${expiresAt}.` : " The transaction itself has no separate automatic expiry."}` };
      }
      if (selector === "0xca1d209d" && to === lower(context.bountyAddress) && data.length === 74 && word(data, 0) === amount) {
        return { kind: "funding", amountUsdcUnits: amount, movesUsdc: true, recipient: to, spender: to, expiresAt: null,
          summary: `Contribute ${formatUnits(amount)} Base USDC to the reviewed bounty ${to}. The bounty's committed funding deadline applies; the transaction itself has no separate automatic expiry.` };
      }
      if (selector === "0x16d0f49a" && ADDRESS.test(context.termsRegistry || "") && to === lower(context.termsRegistry)) {
        return { kind: "terms_publication", amountUsdcUnits: 0n, movesUsdc: false, recipient: to, spender: null, expiresAt: null,
          summary: `Publish the reviewed child-bounty terms to registry ${to}. No USDC moves and this call does not create or fund a bounty. The transaction has no automatic expiry.` };
      }
      throw new Error("The wallet call is not a recognized action in the validated posting plan.");
    });
    return { network: "Base mainnet", chainId: 8453, calls: descriptions,
      transferUsdcUnits: descriptions.reduce((total, call) => total + (call.movesUsdc ? call.amountUsdcUnits : 0n), 0n),
      summary: `Base mainnet (8453).\n${descriptions.map((call, index) => `${index + 1}. ${call.summary}`).join("\n")}\nThese transactions require Base ETH for gas. Review the network fee in your wallet; no sponsorship is confirmed.` };
  }

  function validateFundingAuthorization({ typedData, context, nowMs = Date.now() } = {}) {
    const fail = () => { throw new Error("The USDC authorization does not match the approved bounty, wallet, amount, nonce, deadline or Base USDC domain. No signature was requested."); };
    const exactKeys = (object, keys) => object && typeof object === "object" && !Array.isArray(object)
      && Object.keys(object).sort().join(",") === [...keys].sort().join(",");
    const uint = (value) => {
      if ((typeof value === "number" && (!Number.isSafeInteger(value) || value < 0))
        || !["number", "string", "bigint"].includes(typeof value) || !/^(0|[1-9][0-9]*)$/.test(String(value))) return fail();
      const parsed = BigInt(value);
      if (parsed >= (1n << 256n)) return fail();
      return parsed;
    };
    const lower = (value) => String(value || "").toLowerCase();
    if (!context || uint(context.chainId) !== 8453n || lower(context.usdcAddress) !== BASE_USDC
      || !ADDRESS.test(context.creatorAddress || "") || !ADDRESS.test(context.bountyAddress || "")
      || !/^0x[0-9a-f]{64}$/i.test(context.creationNonce || "") || !Number.isFinite(nowMs)) return fail();
    const expectedTypes = {
      EIP712Domain: [["name", "string"], ["version", "string"], ["chainId", "uint256"], ["verifyingContract", "address"]],
      TransferWithAuthorization: [["from", "address"], ["to", "address"], ["value", "uint256"], ["validAfter", "uint256"], ["validBefore", "uint256"], ["nonce", "bytes32"]],
    };
    if (!exactKeys(typedData, ["types", "domain", "primaryType", "message"])
      || typedData.primaryType !== "TransferWithAuthorization" || !exactKeys(typedData.types, Object.keys(expectedTypes))) return fail();
    for (const [name, expected] of Object.entries(expectedTypes)) {
      const fields = typedData.types[name];
      if (!Array.isArray(fields) || fields.length !== expected.length || fields.some((field, index) => !exactKeys(field, ["name", "type"])
        || field.name !== expected[index][0] || field.type !== expected[index][1])) return fail();
    }
    const { domain, message } = typedData;
    if (!exactKeys(domain, ["name", "version", "chainId", "verifyingContract"])
      || domain.name !== "USD Coin" || domain.version !== "2" || uint(domain.chainId) !== 8453n || lower(domain.verifyingContract) !== BASE_USDC
      || !exactKeys(message, ["from", "to", "value", "validAfter", "validBefore", "nonce"])
      || lower(message.from) !== lower(context.creatorAddress) || lower(message.to) !== lower(context.bountyAddress)
      || uint(message.value) !== uint(context.fundingUsdcUnits) || uint(message.value) === 0n || uint(message.validAfter) !== 0n
      || !/^0x[0-9a-f]{64}$/i.test(message.nonce || "") || lower(message.nonce) !== lower(context.creationNonce)
      || uint(message.validBefore) !== uint(context.fundingDeadline)
      || uint(message.validBefore) <= BigInt(Math.floor(nowMs / 1000)) || uint(message.validBefore) > 8640000000000n) return fail();
    const expiresAt = new Date(Number(uint(message.validBefore)) * 1000).toISOString();
    // Serialize the exact checked payload now so a later mutable planner object
    // cannot change what the wallet is asked to sign after this disclosure.
    const serialized = JSON.stringify(typedData);
    return { serialized, from: lower(message.from), to: lower(message.to), amountUsdcUnits: uint(message.value), nonce: lower(message.nonce), expiresAt,
      summary: `Authorize ${formatUnits(uint(message.value))} USDC on Base mainnet (8453) from ${lower(message.from)} to bounty ${lower(message.to)}. Purpose: one-time funding of this exact bounty through the canonical factory. Expires ${expiresAt}. Token: ${BASE_USDC}. This signature costs no gas and does not itself create or fund the bounty. The following creation transaction requires Base ETH; no gas sponsorship is confirmed.` };
  }

  window.AgentBountiesFundingReadiness = Object.freeze({ parseUsdc, formatUnits, shortfall, readBalances, estimateFees, describeWalletCalls, validateFundingAuthorization });
})();
