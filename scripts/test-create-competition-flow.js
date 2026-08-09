"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "create-competition.js"),
  "utf8",
);

const account = "0x884834e884d6e93462655a2820140ad03e6747bc";
const address = (suffix) => `0x${suffix.padStart(40, "0")}`;
const listeners = new Map();
const sentTransactions = [];
let releaseAcceptance;

function element(extra = {}) {
  return { dataset: {}, textContent: "", hidden: false, ...extra };
}

const form = element({
  elements: {
    solverReward: { value: "0.50" },
    verifierReward: { value: "0.05" },
    competitionHours: { value: "24" },
    revealHours: { value: "1" },
    maxEntries: { value: "4" },
  },
  addEventListener(type, listener) {
    listeners.set(`form:${type}`, listener);
  },
});
const submitButton = element({ disabled: true });
const connectButton = element({
  addEventListener(type, listener) {
    listeners.set(`connect:${type}`, listener);
  },
});
const elements = new Map([
  ["[data-create-output]", element()],
  ["[data-creator-wallet]", element()],
  ["[data-create-competition-form]", form],
  ["[data-create-competition-form] button[type='submit']", submitButton],
  ["[data-profile-name]", element()],
  ["[data-create-stage]", element()],
  ["[data-connect-creator]", connectButton],
  ["[data-created-address]", element()],
  ["[data-created-link]", element({ href: "" })],
  ["[data-created-events]", element({ href: "" })],
  ["[data-created-competition]", element({ hidden: true })],
]);

const provider = {
  isMetaMask: true,
  async request({ method }) {
    if (method === "eth_requestAccounts") return [account];
    if (method === "eth_chainId") return "0x2105";
    if (method === "eth_sendTransaction") {
      const hash = `0x${String(sentTransactions.length + 1).padStart(64, "0")}`;
      sentTransactions.push(hash);
      return hash;
    }
    if (method === "eth_getTransactionReceipt") return { status: "0x1" };
    throw new Error(`unexpected provider method: ${method}`);
  },
};

const profile = {
  profile_id: "leading-zero-work-v1-difficulty-16-mainnet-canary",
  display_name: "Scope-bound hash work",
  public_inventory_eligible: true,
  deployment_state: "active_ready_to_earn",
  benchmark_hash: `0x${"11".repeat(32)}`,
  evidence_schema_hash: `0x${"22".repeat(32)}`,
};
const walletCalls = [
  { from: account, to: address("1"), data: "0x12", value_wei: 0 },
  { from: account, to: address("2"), data: "0x34", value_wei: 0 },
];

async function fetch(url) {
  const value = String(url);
  let body;
  if (value === "protocol.json") body = { api_base_url: "https://api.example" };
  else if (value.includes("/verifiers?")) body = { profiles: [profile] };
  else if (value.endsWith("/creation-preparation")) {
    body = {
      creator: account,
      verifier_profile_id: profile.profile_id,
      ready_to_broadcast: true,
      public_inventory_eligible: true,
      wallet_calls: walletCalls,
      bounty_id: `0x${"33".repeat(32)}`,
      predicted_bounty_contract: address("3"),
    };
  } else if (value.includes("/events?")) {
    body = { events: [{ kind: "canonical_competition_created" }, { kind: "competition_opened" }] };
  } else throw new Error(`unexpected fetch URL: ${value}`);
  return { ok: true, async json() { return body; } };
}

const context = {
  console,
  document: {
    body: {},
    querySelector(selector) {
      if (!elements.has(selector)) throw new Error(`unexpected selector: ${selector}`);
      return elements.get(selector);
    },
  },
  fetch,
  setTimeout,
  window: {
    ethereum: provider,
    AgentBountiesEvm: {
      keccak256Hex() { return `0x${"44".repeat(32)}`; },
      randomBytes32() { return `0x${"55".repeat(32)}`; },
      textHex(value) { return value; },
    },
    AgentBountiesLegal: {
      requireAcceptance() {
        return new Promise((resolve) => { releaseAcceptance = resolve; });
      },
    },
  },
};

(async () => {
  vm.runInNewContext(source, context, { filename: "site/create-competition.js" });
  await new Promise((resolve) => setImmediate(resolve));
  await listeners.get("connect:click")({});

  const submitEvent = {
    currentTarget: form,
    preventDefault() {},
  };
  const submission = listeners.get("form:submit")(submitEvent);
  submitEvent.currentTarget = null;
  releaseAcceptance();
  await submission;

  if (sentTransactions.length !== 2) {
    throw new Error(`async form target was lost before wallet calls: ${sentTransactions.length}`);
  }
  const output = elements.get("[data-create-output]").textContent;
  if (!output.startsWith("Canonical creation, funding")) {
    throw new Error(`creation did not reach canonical confirmation: ${output}`);
  }
  console.log("Open Competition creation preserves the form across async legal consent");
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
