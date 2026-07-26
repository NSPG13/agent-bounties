"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const source = fs.readFileSync(
  path.join(__dirname, "..", "site", "moonpay-direct-fallback.js"),
  "utf8",
);

function element(properties = {}) {
  const listeners = new Map();
  const attributes = new Map();
  return {
    textContent: "",
    value: "",
    checked: false,
    disabled: false,
    href: "#",
    dataset: {},
    ...properties,
    addEventListener(type, listener) {
      listeners.set(type, listener);
    },
    setAttribute(name, value) {
      attributes.set(name, String(value));
    },
    getAttribute(name) {
      return attributes.get(name);
    },
    dispatch(type, event = {}) {
      const listener = listeners.get(type);
      if (!listener) throw new Error(`missing ${type} listener`);
      return listener(event);
    },
  };
}

async function main() {
  const wallet = "0x1234567890abcdef1234567890abcdef12345678";
  const nodes = new Map([
    ["[data-wallet-address]", element({ textContent: "Not connected" })],
    ["[data-onramp-asset]", element({ value: "usdc" })],
    ["[data-fiat-amount]", element({ value: "42.50" })],
    ["[data-onramp-ack]", element({ checked: true })],
    ["[data-direct-moonpay]", element()],
    ["[data-copy-direct-wallet]", element({ disabled: true })],
    ["[data-direct-asset]", element()],
    ["[data-direct-amount]", element()],
    ["[data-direct-wallet]", element()],
    ["[data-direct-moonpay-output]", element()],
    ["[data-connect-wallet]", element()],
    ["[data-wallet-provider]", element()],
  ]);

  let mutationCallback = null;
  let copied = null;
  const context = {
    document: {
      querySelector(selector) {
        return nodes.get(selector) || null;
      },
      createRange() {
        return { selectNodeContents() {} };
      },
    },
    navigator: {
      clipboard: {
        async writeText(value) {
          copied = value;
        },
      },
    },
    window: {
      getSelection() {
        return { removeAllRanges() {}, addRange() {} };
      },
    },
    MutationObserver: class {
      constructor(callback) {
        mutationCallback = callback;
      }
      observe() {}
    },
    setTimeout(callback) {
      callback();
      return 1;
    },
  };

  vm.runInNewContext(source, context, { filename: "site/moonpay-direct-fallback.js" });

  const link = nodes.get("[data-direct-moonpay]");
  const copy = nodes.get("[data-copy-direct-wallet]");
  if (link.href !== "https://www.moonpay.com/buy/usdc") {
    throw new Error(`USDC fallback URL mismatch: ${link.href}`);
  }
  if (link.getAttribute("aria-disabled") !== "true" || copy.disabled !== true) {
    throw new Error("fallback opened before a valid destination wallet was present");
  }

  nodes.get("[data-wallet-address]").textContent = wallet;
  mutationCallback();
  if (link.getAttribute("aria-disabled") !== "false" || copy.disabled !== false) {
    throw new Error("fallback did not enable after wallet connection and acknowledgement");
  }
  if (nodes.get("[data-direct-wallet]").textContent !== wallet) {
    throw new Error("connected wallet was not presented for manual verification");
  }
  if (link.href.includes(wallet) || link.href.includes("walletAddress")) {
    throw new Error("manual fallback leaked the wallet into MoonPay's public URL");
  }

  let prevented = false;
  link.dispatch("click", { preventDefault() { prevented = true; } });
  if (prevented) throw new Error("ready direct checkout was unexpectedly blocked");

  nodes.get("[data-onramp-asset]").value = "eth";
  nodes.get("[data-onramp-asset]").dispatch("change");
  if (link.href !== "https://www.moonpay.com/buy/eth") {
    throw new Error(`ETH fallback URL mismatch: ${link.href}`);
  }
  if (nodes.get("[data-direct-asset]").textContent !== "ETH on Base (ETH_BASE)") {
    throw new Error("ETH Base review label was not rendered");
  }

  await copy.dispatch("click");
  if (copied !== wallet) throw new Error(`copied wallet mismatch: ${copied}`);

  nodes.get("[data-onramp-ack]").checked = false;
  nodes.get("[data-onramp-ack]").dispatch("change");
  prevented = false;
  link.dispatch("click", { preventDefault() { prevented = true; } });
  if (!prevented || link.getAttribute("aria-disabled") !== "true") {
    throw new Error("manual checkout was not blocked after acknowledgement was withdrawn");
  }

  console.log("MoonPay direct fallback preserves wallet, Base asset, and funding boundaries");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
