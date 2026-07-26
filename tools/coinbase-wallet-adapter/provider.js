const AUTHENTICATED_METHODS = new Set([
  "eth_requestAccounts",
  "eth_sendTransaction",
  "eth_sign",
  "eth_signTransaction",
  "eth_signTypedData",
  "eth_signTypedData_v3",
  "eth_signTypedData_v4",
  "personal_sign",
  "wallet_sendCalls",
]);

export function userRejected(message = "Wallet connection was cancelled.") {
  const error = new Error(message);
  error.code = 4001;
  return error;
}

export function createAuthenticatedProvider({
  loadProvider,
  ensureAuthenticated,
  isAuthenticated,
  chainIdHex = "0x2105",
}) {
  if (typeof loadProvider !== "function" || typeof ensureAuthenticated !== "function" || typeof isAuthenticated !== "function") {
    throw new TypeError("Embedded wallet provider dependencies are incomplete.");
  }

  const listeners = new Map();
  let underlying = null;
  let loading = null;

  async function provider() {
    if (underlying) return underlying;
    if (!loading) {
      loading = Promise.resolve(loadProvider()).then((value) => {
        if (!value || typeof value.request !== "function") throw new Error("Coinbase did not return an EIP-1193 provider.");
        underlying = value;
        for (const [event, handlers] of listeners.entries()) {
          for (const handler of handlers) underlying.on?.(event, handler);
        }
        return underlying;
      });
    }
    return loading;
  }

  const wrapper = {
    async request({ method, params = [] } = {}) {
      if (!method || typeof method !== "string") throw new TypeError("Wallet request method is required.");
      if (method === "eth_chainId") return chainIdHex;
      if (method === "wallet_switchEthereumChain" || method === "wallet_addEthereumChain") {
        const requested = String(params?.[0]?.chainId || "").toLowerCase();
        if (!requested || requested === chainIdHex.toLowerCase()) return null;
      }
      if (method === "eth_accounts" && !(await isAuthenticated())) return [];
      if (AUTHENTICATED_METHODS.has(method)) await ensureAuthenticated();
      return (await provider()).request({ method, params });
    },

    on(event, handler) {
      if (typeof handler !== "function") return wrapper;
      const handlers = listeners.get(event) || new Set();
      handlers.add(handler);
      listeners.set(event, handlers);
      underlying?.on?.(event, handler);
      return wrapper;
    },

    removeListener(event, handler) {
      listeners.get(event)?.delete(handler);
      underlying?.removeListener?.(event, handler);
      return wrapper;
    },

    async disconnect() {
      const value = await provider();
      return value.disconnect?.();
    },
  };

  return wrapper;
}

export function accountFromUser(user) {
  const objectAddress = user?.evmAccountObjects?.[0]?.address;
  const legacyAddress = user?.evmAccounts?.[0];
  const value = String(objectAddress || legacyAddress || "");
  return /^0x[0-9a-fA-F]{40}$/.test(value) ? value.toLowerCase() : null;
}
