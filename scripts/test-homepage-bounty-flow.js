"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const homeMetrics = require("../site/home-metrics.js");

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "bounty-entry.js"),
  "utf8",
);

const externalIdentities = homeMetrics.lifetimeExternalActiveIdentities(
  { coverage: { status: "ready" }, platform_active_identities: { lifetime: 38 } },
  { coverage: { status: "ready" }, periods: { lifetime: { active_identities: 112 } } },
);
if (externalIdentities?.total !== 150) {
  throw new Error(`homepage external identity metric did not reconcile 38 + 112: ${JSON.stringify(externalIdentities)}`);
}
if (homeMetrics.lifetimeExternalActiveIdentities(
  { coverage: { status: "ready" }, platform_active_identities: { lifetime: 38 } },
  { coverage: { status: "unavailable" }, periods: { lifetime: { active_identities: 112 } } },
) !== null) {
  throw new Error("homepage identity metric exposed a partial count when GitHub was unavailable");
}
if (homeMetrics.lifetimeExternalActiveIdentities(
  { coverage: { status: "partial" }, platform_active_identities: { lifetime: 38 } },
  { coverage: { status: "ready" }, periods: { lifetime: { active_identities: 112 } } },
) !== null) {
  throw new Error("homepage identity metric exposed a partial count when the platform was incomplete");
}

function loadEntry({ failStorage = false } = {}) {
  const values = new Map();
  const assigned = [];
  const sessionStorage = {
    setItem(key, value) {
      if (failStorage) throw new Error("storage unavailable");
      values.set(key, value);
    },
    getItem(key) {
      if (failStorage) throw new Error("storage unavailable");
      return values.has(key) ? values.get(key) : null;
    },
    removeItem(key) {
      if (failStorage) throw new Error("storage unavailable");
      values.delete(key);
    },
  };
  const window = {
    sessionStorage,
    location: {
      search: "",
      assign(destination) {
        assigned.push(destination);
      },
    },
  };
  const context = {
    URLSearchParams,
    encodeURIComponent,
    window,
  };
  vm.runInNewContext(source, context, { filename: "site/bounty-entry.js" });
  return { api: window.AgentBountyEntry, assigned, values };
}

const exactMessage = "Build a public climate data dashboard & document the API?";
const normal = loadEntry();

if (!normal.api.start(`  ${exactMessage}  `)) {
  throw new Error("non-empty homepage intent was rejected");
}
if (normal.assigned[0] !== "objective.html?source=home&autostart=1") {
  throw new Error(`stored intent leaked into or missed the destination URL: ${normal.assigned[0]}`);
}

const consumed = normal.api.consume("?source=home&autostart=1");
if (!consumed.autostart || consumed.message !== exactMessage) {
  throw new Error(`stored intent was not consumed exactly once: ${JSON.stringify(consumed)}`);
}
const consumedAgain = normal.api.consume("?source=home&autostart=1");
if (consumedAgain.autostart || consumedAgain.message) {
  throw new Error("homepage intent was not removed after consumption");
}

const direct = normal.api.consume("?goal=prefill-only");
if (direct.autostart || direct.message) {
  throw new Error("ordinary goal prefill unexpectedly enabled autostart");
}

const fallback = loadEntry({ failStorage: true });
fallback.api.start(exactMessage);
const fallbackUrl = fallback.assigned[0];
if (!fallbackUrl.startsWith("objective.html?source=home&autostart=1&goal=")) {
  throw new Error(`storage fallback did not preserve a native query handoff: ${fallbackUrl}`);
}
const fallbackIntent = fallback.api.consume(fallbackUrl.slice(fallbackUrl.indexOf("?")));
if (!fallbackIntent.autostart || fallbackIntent.message !== exactMessage) {
  throw new Error(`query fallback did not recover the exact intent: ${JSON.stringify(fallbackIntent)}`);
}

const empty = loadEntry();
if (empty.api.start("   ") || empty.assigned.length) {
  throw new Error("empty homepage intent should not navigate");
}

console.log("homepage bounty intent and lifetime external identity reconciliation are valid");
