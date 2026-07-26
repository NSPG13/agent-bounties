const ADDRESS = /^0x[0-9a-fA-F]{40}$/;
const HEX = /^0x(?:[0-9a-fA-F]{2})*$/;
const HASH = /^0x[0-9a-fA-F]{64}$/;

function quantity(value, label) {
  if (typeof value === "bigint") {
    if (value < 0n) throw new TypeError(`${label} cannot be negative.`);
    return value;
  }
  if (typeof value === "number") {
    if (!Number.isSafeInteger(value) || value < 0) throw new TypeError(`${label} is invalid.`);
    return BigInt(value);
  }
  const text = String(value ?? "0").trim();
  if (/^0x[0-9a-fA-F]+$/.test(text) || /^\d+$/.test(text)) {
    const parsed = BigInt(text);
    if (parsed < 0n) throw new TypeError(`${label} cannot be negative.`);
    return parsed;
  }
  throw new TypeError(`${label} is invalid.`);
}

export function normalizeEvmCall(call) {
  if (!call || typeof call !== "object") throw new TypeError("Wallet call is required.");
  const to = String(call.to || "");
  if (!ADDRESS.test(to)) throw new TypeError("Wallet call destination is invalid.");
  const data = call.data == null || call.data === "" ? "0x" : String(call.data);
  if (!HEX.test(data)) throw new TypeError("Wallet call data must be even-length hexadecimal bytes.");
  return Object.freeze({
    to: to.toLowerCase(),
    value: quantity(call.value ?? 0, "Wallet call value"),
    data: data.toLowerCase(),
  });
}

export function transactionToCalls(transaction, expectedFrom) {
  if (!transaction || typeof transaction !== "object") throw new TypeError("Transaction request is required.");
  const from = String(transaction.from || "");
  if (expectedFrom && from && from.toLowerCase() !== String(expectedFrom).toLowerCase()) {
    throw new Error("Transaction sender does not match the authenticated embedded wallet.");
  }
  return [normalizeEvmCall(transaction)];
}

export function walletRequestToCalls(request, expectedFrom, chainIdHex = "0x2105") {
  const envelope = request?.params?.[0];
  if (!envelope || !Array.isArray(envelope.calls) || envelope.calls.length === 0) {
    throw new TypeError("wallet_sendCalls requires at least one call.");
  }
  const from = String(envelope.from || "");
  if (expectedFrom && from && from.toLowerCase() !== String(expectedFrom).toLowerCase()) {
    throw new Error("Wallet call sender does not match the authenticated embedded wallet.");
  }
  if (envelope.chainId && String(envelope.chainId).toLowerCase() !== chainIdHex.toLowerCase()) {
    throw new Error("Embedded wallet calls must target Base mainnet.");
  }
  return envelope.calls.map(normalizeEvmCall);
}

function failureMessage(operation) {
  const receipts = Array.isArray(operation?.receipts) ? operation.receipts : [];
  for (const receipt of receipts) {
    const message = receipt?.revert?.message || receipt?.revert?.reason || receipt?.error;
    if (message) return String(message).slice(0, 320);
  }
  return "The sponsored wallet operation failed.";
}

export async function waitForUserOperationTransaction({
  userOperationHash,
  account,
  getOperation,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  timeoutMs = 150_000,
  intervalMs = 1_500,
  now = () => Date.now(),
}) {
  if (!HASH.test(String(userOperationHash || ""))) throw new TypeError("User operation hash is invalid.");
  if (!ADDRESS.test(String(account || ""))) throw new TypeError("Smart account address is invalid.");
  if (typeof getOperation !== "function") throw new TypeError("User operation reader is required.");
  const started = now();
  while (now() - started < timeoutMs) {
    const operation = await getOperation({
      userOperationHash,
      evmSmartAccount: account,
      network: "base",
    });
    const status = String(operation?.status || "").toLowerCase();
    if (status === "complete" || status === "completed") {
      const transactionHash = String(operation?.transactionHash || "");
      if (!HASH.test(transactionHash)) throw new Error("Sponsored operation completed without a valid transaction hash.");
      return transactionHash;
    }
    if (status === "failed") throw new Error(failureMessage(operation));
    await sleep(intervalMs);
  }
  throw new Error("The sponsored wallet operation is still pending. Check the same wallet before retrying.");
}

export function delegatedCode(value) {
  const code = String(value || "").toLowerCase();
  return /^0x[0-9a-f]+$/.test(code) && code !== "0x" && code !== "0x0" && code !== "0x00";
}
