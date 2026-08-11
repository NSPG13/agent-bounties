(() => {
  "use strict";

  const entries = new Map();
  const providers = new WeakMap();
  const listeners = new Map();

  function requireString(value, name, pattern = null) {
    const normalized = String(value || "").trim();
    if (!normalized || (pattern && !pattern.test(normalized))) {
      throw new TypeError(`Invalid wallet adapter ${name}.`);
    }
    return normalized;
  }

  function normalizeInfo(info) {
    if (!info || typeof info !== "object") throw new TypeError("Wallet adapter info is required.");
    return Object.freeze({
      uuid: requireString(
        info.uuid,
        "uuid",
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i,
      ),
      name: requireString(info.name, "name"),
      icon: requireString(info.icon, "icon"),
      rdns: requireString(info.rdns, "rdns", /^(?:[a-z0-9-]+\.)+[a-z0-9-]+$/i),
    });
  }

  function normalizeCapabilities(capabilities = {}) {
    const clone = {};
    for (const [key, value] of Object.entries(capabilities)) {
      if (typeof value === "function") continue;
      clone[key] = Array.isArray(value) ? Object.freeze([...value]) : value;
    }
    return Object.freeze(clone);
  }

  function assertProvider(provider) {
    if (!provider || typeof provider !== "object" || typeof provider.request !== "function") {
      throw new TypeError("Wallet adapter provider must implement EIP-1193 request().");
    }
    return provider;
  }

  function announce(entry) {
    window.dispatchEvent(new CustomEvent("eip6963:announceProvider", {
      detail: Object.freeze({ info: entry.info, provider: entry.provider }),
    }));
  }

  function register(input) {
    if (!input || typeof input !== "object") throw new TypeError("Wallet adapter registration is required.");
    const id = requireString(input.id, "id", /^[a-z0-9][a-z0-9._-]{2,79}$/i);
    const provider = assertProvider(input.provider);
    const existingByProvider = providers.get(provider);
    if (existingByProvider && existingByProvider !== id) {
      throw new Error("The same EIP-1193 provider cannot be registered under two adapter IDs.");
    }
    const entry = Object.freeze({
      id,
      info: normalizeInfo(input.info),
      provider,
      capabilities: normalizeCapabilities(input.capabilities),
      disconnect: typeof input.disconnect === "function" ? input.disconnect : null,
    });
    const existing = entries.get(id);
    if (existing && existing.provider !== provider) {
      throw new Error(`Wallet adapter ${id} is already registered.`);
    }
    entries.set(id, entry);
    providers.set(provider, id);
    queueMicrotask(() => announce(entry));
    window.dispatchEvent(new CustomEvent("agentbounties:wallet-adapter-registered", {
      detail: Object.freeze({ id, info: entry.info, capabilities: entry.capabilities }),
    }));
    return entry;
  }

  function list() {
    return [...entries.values()].map((entry) => Object.freeze({
      id: entry.id,
      info: entry.info,
      provider: entry.provider,
      capabilities: entry.capabilities,
    }));
  }

  function get(id) {
    return entries.get(String(id || "").trim()) || null;
  }

  function capabilitiesFor(provider) {
    const id = providers.get(provider);
    return id ? entries.get(id)?.capabilities || null : null;
  }

  async function disconnect(id) {
    const entry = get(id);
    if (!entry?.disconnect) return false;
    await entry.disconnect();
    return true;
  }

  function on(eventName, listener) {
    if (typeof listener !== "function") throw new TypeError("Wallet adapter listener must be a function.");
    const bucket = listeners.get(eventName) || new Set();
    bucket.add(listener);
    listeners.set(eventName, bucket);
    return () => bucket.delete(listener);
  }

  window.addEventListener("eip6963:requestProvider", () => {
    for (const entry of entries.values()) announce(entry);
  });

  window.addEventListener("agentbounties:wallet-adapter-event", (event) => {
    const name = String(event.detail?.name || "");
    for (const listener of listeners.get(name) || []) listener(event.detail?.payload);
  });

  window.AgentBountiesWalletAdapters = Object.freeze({
    register,
    list,
    get,
    capabilitiesFor,
    disconnect,
    on,
  });
})();
