(() => {
  "use strict";

  const adapters = new Map();
  const providers = new WeakSet();
  const ADDRESS = /^0x[0-9a-fA-F]{40}$/;

  function assertAdapter(adapter) {
    if (!adapter || typeof adapter !== "object") throw new TypeError("Wallet adapter must be an object.");
    if (!/^[a-z0-9][a-z0-9._-]{1,63}$/.test(String(adapter.id || ""))) {
      throw new TypeError("Wallet adapter id is invalid.");
    }
    if (!adapter.provider || typeof adapter.provider.request !== "function") {
      throw new TypeError("Wallet adapter must expose an EIP-1193 provider.");
    }
    const info = adapter.info || {};
    if (!info.name || !info.rdns || !info.icon) throw new TypeError("Wallet adapter EIP-6963 metadata is incomplete.");
    return adapter;
  }

  function adapterDetail(adapter) {
    return Object.freeze({
      info: Object.freeze({ ...adapter.info }),
      provider: adapter.provider,
    });
  }

  function announce(adapter) {
    window.dispatchEvent(new CustomEvent("eip6963:announceProvider", {
      detail: adapterDetail(adapter),
    }));
    window.dispatchEvent(new CustomEvent("agentbounties:wallet-adapter-announced", {
      detail: Object.freeze({ id: adapter.id, capabilities: Object.freeze({ ...(adapter.capabilities || {}) }) }),
    }));
  }

  function register(adapterInput) {
    const adapter = assertAdapter(adapterInput);
    if (adapters.has(adapter.id)) throw new Error(`Wallet adapter ${adapter.id} is already registered.`);
    if (providers.has(adapter.provider)) throw new Error("This wallet provider is already registered.");
    adapters.set(adapter.id, Object.freeze({ ...adapter }));
    providers.add(adapter.provider);
    announce(adapter);
    return () => {
      adapters.delete(adapter.id);
      window.dispatchEvent(new CustomEvent("agentbounties:wallet-adapter-removed", {
        detail: Object.freeze({ id: adapter.id }),
      }));
    };
  }

  function get(id) {
    return adapters.get(String(id || "")) || null;
  }

  function list() {
    return Array.from(adapters.values());
  }

  async function open(id, context = {}) {
    const adapter = get(id);
    if (!adapter) throw new Error(`Wallet adapter ${id} is unavailable.`);
    if (typeof adapter.open === "function") return adapter.open(context);
    return adapter.provider.request({ method: "eth_requestAccounts" });
  }

  function connectedAddress(accounts) {
    const account = Array.isArray(accounts) ? String(accounts[0] || "") : "";
    return ADDRESS.test(account) ? account.toLowerCase() : null;
  }

  window.addEventListener("eip6963:requestProvider", () => {
    for (const adapter of adapters.values()) announce(adapter);
  });

  window.AgentBountiesWalletAdapters = Object.freeze({
    schemaVersion: "agent-bounties/wallet-adapter-registry-v1",
    register,
    get,
    list,
    open,
    connectedAddress,
  });

  window.dispatchEvent(new CustomEvent("agentbounties:wallet-adapter-registry-ready", {
    detail: window.AgentBountiesWalletAdapters,
  }));
})();
