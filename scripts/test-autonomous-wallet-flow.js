const assert = require("assert");
const fs = require("fs");
const path = require("path");
const vm = require("vm");

const repoRoot = path.resolve(__dirname, "..");
const source = fs.readFileSync(path.join(repoRoot, "site", "autonomous.js"), "utf8");
const protocol = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "site", "protocol.json"), "utf8"),
);

new vm.Script(source, { filename: "site/autonomous.js" });

for (const required of [
  "eth_signTypedData_v4",
  "wallet_sendCalls",
  "create_bounty",
  "eip3009_authorization",
  "/v1/base/autonomous-bounties/creation-plan",
  "/v1/base/autonomous-bounties/authorized-creation-plan",
  "/v1/base/autonomous-bounties/contribution-plan",
  "/v1/base/autonomous-bounties/authorized-contribution-plan",
  "/v1/base/autonomous-bounties/claim-plan",
  "/v1/base/autonomous-bounties/authorized-claim-plan",
  "/v1/base/autonomous-bounties/submission-plan",
  "FundingAdded",
  "BountyClaimed",
  "submission_added",
  "BountySettled",
]) {
  assert(source.includes(required), `autonomous wallet flow missing ${required}`);
}

for (const retired of [
  "createEscrow",
  "EscrowReleased",
  "/v1/base/funding-plan",
  "/v1/base/release-plan",
  "settlement signer",
]) {
  assert(!source.includes(retired), `autonomous wallet flow contains retired behavior: ${retired}`);
}

assert.strictEqual(protocol.protocol_version, "agent-bounties/autonomous-v1");
assert.strictEqual(protocol.network, "base-mainnet");
assert.strictEqual(protocol.chain_id, 8453);
assert.strictEqual(protocol.status, "pending_external_review_and_deployment");
assert.strictEqual(protocol.factory, null);
assert.strictEqual(protocol.implementation, null);

for (const page of ["index.html", "post.html", "funding.html", "earn.html", "operator.html"]) {
  const html = fs.readFileSync(path.join(repoRoot, "site", page), "utf8");
  assert(html.includes("autonomous.js"), `${page} does not load autonomous.js`);
  assert(!html.includes("main.js"), `${page} loads the retired browser bundle`);
}

console.log("autonomous wallet flow contract passed");
