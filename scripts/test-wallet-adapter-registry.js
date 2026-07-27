"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(path.join(__dirname, "..", "site", "wallet-adapter-registry.js"), "utf8");

class EventTargetLike {
  constructor() { this.listeners = new Map(); }
  addEventListener(name, listener) {
    const bucket = this.listeners.get(name) || [];
    bucket.push(listener);
    this.listeners.set(name, bucket);
  }
  dispatchEvent(event) {
    for (const listener of this.listeners.get(event.type) || []) listener(event);
    return true;
  }
}

class CustomEventLike {
  constructor(type, options = {}) { this.type = type; this.detail = options.detail; }
}

(async () => {
  const windowObject = new EventTargetLike();
  const context = {
    window: windowObject,
    CustomEvent: CustomEventLike,
    WeakMap,
    Map,
    Set,
    Object,
    Array,
    String,
    TypeError,
    Error,
    queueMicrotask,
  };
  vm.runInNewContext(source, context, { filename: "site/wallet-adapter-registry.js" });
  const registry = windowObject.AgentBountiesWalletAdapters;
  if (!registry) throw new Error("registry was not exposed");

  const announcements = [];
  windowObject.addEventListener("eip6963:announceProvider", (event) => announcements.push(event.detail));
  const provider = { request: async () => [] };
  const capabilities = { embedded: true, authMethods: ["email", "sms"] };
  const entry = registry.register({
    id: "test-embedded",
    info: {
      uuid: "12345678-1234-4123-8123-123456789abc",
      name: "Test embedded wallet",
      icon: "data:image/svg+xml,%3Csvg/%3E",
      rdns: "app.agentbounties.wallet.test",
    },
    provider,
    capabilities,
  });
  await new Promise((resolve) => queueMicrotask(resolve));
  if (announcements.length !== 1 || announcements[0].provider !== provider) {
    throw new Error("registration did not announce through EIP-6963");
  }
  if (registry.get("test-embedded")?.provider !== provider) throw new Error("registry lookup failed");
  if (registry.capabilitiesFor(provider)?.embedded !== true) throw new Error("provider capabilities lookup failed");
  capabilities.embedded = false;
  capabilities.authMethods.push("oauth");
  if (entry.capabilities.embedded !== true || entry.capabilities.authMethods.length !== 2) {
    throw new Error("capabilities were not defensively copied and frozen");
  }
  windowObject.dispatchEvent(new CustomEventLike("eip6963:requestProvider"));
  if (announcements.length !== 2) throw new Error("provider was not re-announced on request");

  let duplicateRejected = false;
  try {
    registry.register({
      id: "other-id",
      info: {
        uuid: "87654321-4321-4321-8321-cba987654321",
        name: "Duplicate",
        icon: "data:image/svg+xml,%3Csvg/%3E",
        rdns: "app.agentbounties.wallet.duplicate",
      },
      provider,
    });
  } catch (_error) {
    duplicateRejected = true;
  }
  if (!duplicateRejected) throw new Error("same provider was accepted under two adapter IDs");

  console.log("Wallet adapter registry is vendor-neutral, EIP-6963-compatible, immutable, and deduplicated");
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
