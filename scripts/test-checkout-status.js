const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const repoRoot = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(repoRoot, "site", "main.js"), "utf8");

const context = {
  console,
  document: {
    getElementById() {
      return null;
    },
  },
  URLSearchParams,
};
context.window = context;
vm.createContext(context);
vm.runInContext(source, context, { filename: "site/main.js" });

const checkout = context.window.AgentBountiesCheckoutStatus;

function report(overrides = {}) {
  return {
    bounty: { id: "bounty-1", status: "Claimable" },
    funding_summary: {
      applied: { amount: 0, currency: "usd" },
      remaining: { amount: 500, currency: "usd" },
      claimable: false,
      ...overrides.funding_summary,
    },
    funding_intents: overrides.funding_intents || [],
  };
}

assert.strictEqual(checkout.displayMoney({ amount: 500, currency: "usd" }), "5.00 USD");
assert.strictEqual(checkout.displayMoney({ amount: 1000000, currency: "usdc" }), "1.000000 USDC");

const awaitingWithUnrelatedClaimability = checkout.checkoutStatusLines(
  report({
    funding_summary: {
      applied: { amount: 1000000, currency: "usdc" },
      remaining: { amount: 0, currency: "usdc" },
      claimable: true,
    },
    funding_intents: [
      {
        id: "stripe-intent-1",
        rail: "StripeFiat",
        status: "AwaitingEvidence",
        external_reference: "checkout-target",
      },
    ],
  }),
  { bountyId: "bounty-1", externalReference: "checkout-target" },
);
assert(awaitingWithUnrelatedClaimability.includes("State: waiting for webhook"));
assert(!awaitingWithUnrelatedClaimability.includes("State: funding reconciled"));
assert(awaitingWithUnrelatedClaimability.includes("Bounty claimable: yes"));
assert(awaitingWithUnrelatedClaimability.includes("matching Checkout funding intent to show Applied webhook evidence"));

const mismatchedIdentifier = checkout.checkoutStatusLines(
  report({
    funding_summary: {
      applied: { amount: 500, currency: "usd" },
      remaining: { amount: 0, currency: "usd" },
      claimable: true,
    },
    funding_intents: [
      {
        id: "stripe-intent-2",
        rail: "StripeFiat",
        status: "Applied",
        external_reference: "different-checkout",
      },
    ],
  }),
  { bountyId: "bounty-1", externalReference: "checkout-target" },
);
assert(mismatchedIdentifier.includes("State: waiting for webhook"));
assert(mismatchedIdentifier.includes("Funding intent: not identified for external reference checkout-target"));
assert(!mismatchedIdentifier.includes("Funding intent id: stripe-intent-2"));
assert(!mismatchedIdentifier.includes("different-checkout"));

const appliedMatch = checkout.checkoutStatusLines(
  report({
    funding_summary: {
      applied: { amount: 500, currency: "usd" },
      remaining: { amount: 0, currency: "usd" },
      claimable: true,
    },
    funding_intents: [
      {
        id: "stripe-intent-3",
        rail: "StripeFiat",
        status: "Applied",
        external_reference: "checkout-target",
      },
    ],
  }),
  { bountyId: "bounty-1", externalReference: "checkout-target" },
);
assert(appliedMatch.includes("State: funding reconciled"));
assert(appliedMatch.includes("Funding intent id: stripe-intent-3"));

const rejectedMatch = checkout.checkoutStatusLines(
  report({
    funding_intents: [
      {
        id: "stripe-intent-4",
        rail: "StripeFiat",
        status: "Rejected",
        external_reference: "checkout-target",
      },
    ],
  }),
  { bountyId: "bounty-1", externalReference: "checkout-target" },
);
assert(rejectedMatch.includes("State: needs operator review"));

const unavailable = checkout.checkoutUnavailableStatusLines("Hosted bounty status returned 503");
assert(unavailable.includes("State: needs operator review"));
assert(unavailable.includes("Hosted API status is unavailable"));

console.log("checkout status classifier tests passed");
