"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "bounty-entry.js"),
  "utf8",
);

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

console.log("homepage bounty intent reaches chat with exact, one-time autostart semantics");
