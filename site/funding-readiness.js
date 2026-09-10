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

  window.AgentBountiesFundingReadiness = Object.freeze({ parseUsdc, formatUnits, shortfall, readBalances, estimateFees });
})();
