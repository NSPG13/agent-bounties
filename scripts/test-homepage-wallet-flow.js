"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const listeners = new Map();
const walletCalls = [];

function addListener(target, type, listener) {
  const key = `${target}:${type}`;
  const existing = listeners.get(key) || [];
  existing.push(listener);
  listeners.set(key, existing);
}

const provider = {
  request: async ({ method }) => {
    walletCalls.push(method);
    if (method === "eth_requestAccounts") return ["0x1111111111111111111111111111111111111111"];
    if (method === "eth_chainId") return "0x2105";
    throw new Error(`Unexpected wallet method: ${method}`);
  },
  isMetaMask: true,
};

const output = { textContent: "", dataset: {} };
const selector = {
  value: "0",
  disabled: false,
  options: [],
  addEventListener(type, listener) {
    addListener("selector", type, listener);
  },
  append(option) {
    this.options.push(option);
  },
  replaceChildren(...options) {
    this.options = options;
    const selectedIndex = Math.max(0, options.findIndex((option) => option.selected));
    this.value = options[selectedIndex] ? options[selectedIndex].value : "0";
  },
  closest() {
    return null;
  },
};

const button = {
  dataset: { output: "wallet-status" },
  addEventListener(type, listener) {
    addListener("button", type, listener);
  },
  closest() {
    return null;
  },
};

const documentListeners = new Map();
const document = {
  id: undefined,
  addEventListener(type, listener) {
    documentListeners.set(type, listener);
  },
  createElement(tag) {
    if (tag !== "option") throw new Error(`Unexpected element creation: ${tag}`);
    return { value: "", textContent: "", selected: false };
  },
  getElementById(id) {
    return id === "wallet-status" ? output : null;
  },
  querySelector(selectorText) {
    if (selectorText === "[data-wallet-provider]") return selector;
    return null;
  },
  querySelectorAll(selectorText) {
    if (selectorText === "[data-connect-wallet]") return [button];
    if (selectorText === "[data-wallet-provider]") return [selector];
    return [];
  },
};

global.document = document;
global.window = {
  ethereum: provider,
  addEventListener(type, listener) {
    addListener("window", type, listener);
  },
  dispatchEvent(event) {
    for (const listener of listeners.get(`window:${event.type}`) || []) listener(event);
    return true;
  },
};
global.fetch = async (url) => {
  if (url !== "protocol.json") throw new Error(`Unexpected fetch: ${url}`);
  return {
    ok: true,
    async json() {
      return {
        status: "active",
        chain_id_hex: "0x2105",
        api_base_url: "https://api.agentbounties.app",
      };
    },
  };
};

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "autonomous.js"),
  "utf8",
);
vm.runInThisContext(source, { filename: "site/autonomous.js" });

(async () => {
  const initialize = documentListeners.get("DOMContentLoaded");
  if (!initialize) throw new Error("wallet controller did not register DOMContentLoaded initialization");
  await initialize();

  const click = (listeners.get("button:click") || [])[0];
  if (!click) throw new Error("homepage Connect Wallet button did not receive a click handler");
  await click();

  if (!walletCalls.includes("eth_requestAccounts")) {
    throw new Error("homepage click never requested wallet accounts");
  }
  if (!walletCalls.includes("eth_chainId")) {
    throw new Error("homepage click never verified the Base chain");
  }
  if (!output.textContent.startsWith("Connected: 0x1111")) {
    throw new Error(`homepage did not report a connected account: ${output.textContent}`);
  }

  console.log("homepage Connect Wallet click reaches the injected provider");
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
