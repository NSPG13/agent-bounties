"use strict";
// Generates the exact v2 terms and creation the posting page builds, from the
// real site code at a fixed clock, and keeps the shared fixture current. The
// Rust test crates/chain-base/tests/website_v2_posting.rs checks the same
// fixture against the API's publication, public-earning and planner code.
// Regenerate after an intended change: WRITE_WEBSITE_V2_FIXTURE=1 node --test <this file>
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const root = path.resolve(__dirname, "..");
const fixturePath = path.join(root, "crates/chain-base/tests/fixtures/website-v2-posting.json");
const read = name => fs.readFileSync(path.join(root, "site", name), "utf8");
const NOW = Date.parse("2026-11-17T00:00:00Z");
const CREATOR = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"; // Public fixture key 1, never funded.

function generate() {
  class FixedDate extends Date {
    constructor(...args) { super(...(args.length ? args : [NOW])); }
    static now() { return NOW; }
  }
  const window = {};
  const context = vm.createContext({ window, document: { getElementById: () => ({ dataset: {} }), querySelector: selector => selector === "[data-posting-options]" ? null : { dataset: {} }, querySelectorAll: () => [] },
    Date: FixedDate, TextEncoder, URL, URLSearchParams, crypto: { getRandomValues: bytes => bytes.fill(0x5a) } });
  for (const name of ["evm.js", "posting-v2-plan.js", "creator-review.js"]) vm.runInContext(read(name), context);
  window.AgentBountiesMetaChild = {};
  window.AgentBountiesWorkflow = require("../site/marketplace-workflow.js");
  const marker = "  function track(eventName, details) {";
  const composer = read("bounty-composer-v2.js");
  assert.equal(composer.split(marker).length, 2);
  vm.runInContext(composer.replace(marker, `  window.testComposer = { state, contractTerms, termsDocument, createPayload, currentRewardSplit }; return;\n${marker}`), context);
  const site = window.testComposer, protocol = JSON.parse(read("protocol-v2.json"));
  const draft = window.AgentBountiesCreatorReview.prepare({ title: "Website v2 posting fixture", goal: "Deliver one reviewed artifact for the website v2 posting fixture.",
    acceptance_criteria: ["The artifact is public over HTTPS.", "Its SHA-256 digest matches the submitted evidence."], review_mode: "creator",
    delivery_deadline: "2026-12-15T18:00:00-06:00" });
  Object.assign(site.state, { draft, account: CREATOR, fundingUsdc: 20, taskWindowDays: 7, deliveryDeadline: draft.delivery_deadline,
    sourceUrl: "https://agentizable.ai/website-v2-posting-fixture", bountyImage: null, scope: "single", metaParent: null, reviewStale: false });
  const committed = site.contractTerms(protocol, site.currentRewardSplit());
  const document = JSON.parse(JSON.stringify(site.termsDocument(committed)));
  const canonical = require("../site/terms-publication.js").canonical, evm = window.AgentBountiesEvm;
  const hash = value => evm.keccak256Hex(evm.textHex(canonical(value)));
  // This document has no optional fields the server would normalize, so the
  // server's record hashes equal these; the Rust test asserts exactly that.
  const terms = { terms_hash: hash(document), policy_hash: hash(document.verification_policy), acceptance_criteria_hash: hash(document.acceptance_criteria),
    benchmark_hash: hash(document.benchmark), evidence_schema_hash: hash(document.evidence_schema) };
  const create = JSON.parse(JSON.stringify(site.createPayload(terms, committed)));
  const encoded = window.AgentBountiesPostingV2Plan.encode(create, protocol);
  const amounts = window.AgentBountiesPostingV2Plan.economics(create.solver_reward.amount, create.verifier_reward.amount, protocol);
  const word = value => BigInt(value).toString(16).padStart(64, "0");
  return {
    schema_version: "agent-bounties/website-v2-posting-fixture-v1",
    boundary: "Generated from the real posting page code at a fixed clock. Synthetic creator; never funded. Not payment evidence.",
    published_at: new Date(NOW).toISOString(),
    network: protocol.network, factory: protocol.factory, implementation: protocol.implementation,
    platform_fee_bps: protocol.platform_fee_bps, platform_fee_recipient: protocol.platform_fee_recipient,
    publication: { creator_wallet: CREATOR, document },
    terms, create,
    expected: { bounty_id: encoded.bountyId, predicted_bounty_contract: encoded.bounty, platform_fee: String(amounts.fee), target_amount: String(amounts.total),
      wallet_calls: [{ to: protocol.native_usdc, data: `0x095ea7b3${protocol.factory.slice(2).padStart(64, "0")}${word(amounts.total)}` }, { to: protocol.factory, data: encoded.data }] },
  };
}

test("the shared website v2 fixture matches what the posting page builds", () => {
  const generated = generate();
  if (process.env.WRITE_WEBSITE_V2_FIXTURE === "1") fs.writeFileSync(fixturePath, `${JSON.stringify(generated, null, 2)}\n`);
  assert.deepEqual(JSON.parse(fs.readFileSync(fixturePath, "utf8")), generated,
    "The posting page's v2 terms or creation changed. Regenerate the fixture and rerun the Rust website_v2_posting test.");
});

test("the fixture funds rewards plus the rounded-up fee and commits the creator as reviewer", () => {
  const { create, publication, expected } = generate();
  const terms = publication.document.contract_terms;
  assert.equal(terms.protocol_version, "agent-bounties/autonomous-v2");
  assert.equal(BigInt(terms.platform_fee.amount), (BigInt(terms.solver_reward.amount) * 750n + 9999n) / 10000n);
  assert.equal(create.initial_funding.amount, terms.solver_reward.amount + terms.verifier_reward.amount + terms.platform_fee.amount);
  assert.equal(String(create.initial_funding.amount), expected.target_amount);
  assert.equal(terms.funding_deadline, publication.document.benchmark.delivery_deadline);
  assert.deepEqual(create.verifiers, [CREATOR]);
  assert.equal(publication.document.verification_policy.engine, "creator_review_v1");
});
