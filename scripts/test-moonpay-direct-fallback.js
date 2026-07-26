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
    min: "",
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
    ["[data-fiat-amount]", element({ value: "42.50" })],
    ["[data-onramp-ack]", element({ checked: true })],
    ["[data-start-moonpay]", element()],
    ["[data-onramp-output]", element()],
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
  const amount = nodes.get("[data-fiat-amount]");
  const partnerButton = nodes.get("[data-start-moonpay]");
  if (amount.min !== "20") {
    throw new Error(`MoonPay minimum was not applied to the amount input: ${amount.min}`);
  }
  if (link.href !== "https://www.moonpay.com/buy/usdc") {
    throw new Error(`USDC fallback URL mismatch: ${link.href}`);
  }
  if (nodes.get("[data-direct-asset]").textContent !== "USDC on Base (USDC_BASE)") {
    throw new Error("Base USDC review label was not rendered");
  }
  if (link.getAttribute("aria-disabled") !== "true" || copy.disabled !== true) {
    throw new Error("fallback opened before a valid destination wallet was present");
  }

  nodes.get("[data-wallet-address]").textContent = wallet;
  mutationCallback();
  if (link.getAttribute("aria-disabled") !== "false" || copy.disabled !== false) {
    throw new Error("fallback did not enable after wallet connection, amount review, and acknowledgement");
  }
  if (nodes.get("[data-direct-wallet]").textContent !== wallet) {
    throw new Error("connected wallet was not presented for manual verification");
  }
  if (link.href.includes(wallet) || link.href.includes("walletAddress")) {
    throw new Error("manual fallback leaked the wallet into MoonPay's public URL");
  }
  if (source.includes("www.moonpay.com/buy/eth") || source.includes("ETH_BASE")) {
    throw new Error("the embedded-wallet onboarding fallback must remain Base-USDC-only");
  }

  amount.value = "19.99";
  amount.dispatch("input");
  if (link.getAttribute("aria-disabled") !== "true") {
    throw new Error("direct checkout remained enabled below MoonPay's minimum");
  }
  if (!nodes.get("[data-direct-amount]").textContent.includes("below MoonPay minimum")) {
    throw new Error("the below-minimum amount was not made visible to the user");
  }
  let prevented = false;
  let propagationStopped = false;
  partnerButton.dispatch("click", {
    preventDefault() { prevented = true; },
    stopImmediatePropagation() { propagationStopped = true; },
  });
  if (!prevented || !propagationStopped) {
    throw new Error("signed partner checkout was not stopped below MoonPay's minimum");
  }
  if (!nodes.get("[data-onramp-output]").textContent.includes("at least $20.00")) {
    throw new Error("the partner checkout did not explain MoonPay's minimum");
  }

  amount.value = "42.50";
  amount.dispatch("input");
  prevented = false;
  link.dispatch("click", { preventDefault() { prevented = true; } });
  if (prevented) throw new Error("ready direct checkout was unexpectedly blocked");

  await copy.dispatch("click");
  if (copied !== wallet) throw new Error(`copied wallet mismatch: ${copied}`);

  nodes.get("[data-onramp-ack]").checked = false;
  nodes.get("[data-onramp-ack]").dispatch("change");
  prevented = false;
  link.dispatch("click", { preventDefault() { prevented = true; } });
  if (!prevented || link.getAttribute("aria-disabled") !== "true") {
    throw new Error("manual checkout was not blocked after acknowledgement was withdrawn");
  }

  console.log("MoonPay direct fallback preserves minimum, wallet, Base USDC, and protocol-action boundaries");
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
