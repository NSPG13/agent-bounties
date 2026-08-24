"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const marketplace = require("../site/marketplace.js");
globalThis.AgentBountiesMarketplace = marketplace;
const competition = require("../site/competition.js");

const STARTS_AT = "2026-08-25T00:00:00Z";
const ENDS_AT = "2026-09-01T00:00:00Z";

function amount(value) {
  return { amount: String(Math.round(value * 1_000_000)), decimals: 6, unit: "base_units", asset: "USDC" };
}

function v2Opportunity(overrides = {}) {
  return {
    opportunity_id: "open-competition-v2:gmv-activation-01",
    source_type: "canonical_base",
    source_status: "active",
    source_id: "0x1111111111111111111111111111111111111111",
    network: "base-mainnet",
    title: "Create the most externally funded marketplace GMV",
    goal: "Post useful work, fund it, and reach canonical settlement with another wallet.",
    work_state: "claimable",
    payment_state: "escrowed",
    payment_committed: true,
    verification_ready: true,
    competition_mode: "best_score",
    terms_hash: null,
    reward: amount(3),
    funded_amount: amount(3.04),
    funding_target: amount(3.04),
    cash_economics: { required_external_spend: amount(0.11) },
    evidence_requirements: {
      program_profile: "forward-canonical-gmv-attribution-metric-v2",
      verification_policy_hash: "0x2222222222222222222222222222222222222222222222222222222222222222",
      scoring_formula: "sum(settlement_gmv * entrant_funding / total_funding)",
      scoring_window: { starts_at: STARTS_AT, ends_at: ENDS_AT },
      qualifying_action: {
        entrant_binding: "The funding wallet must equal the competition entrant.",
        excluded: ["operator-funded work", "unsettled deposits"],
      },
      snapshot_url: "https://api.agentbounties.app/v1/base/open-competition-v2-beta3/gmv-snapshots/gmv-activation-01",
    },
    next_action: { action: "enter_open_competition_v2", url: "https://api.agentbounties.app/v1/relay" },
    evidence_boundary: "Only confirmed canonical settlement events count.",
    ...overrides,
  };
}

test("V2 readiness is mechanism-aware and does not depend on a legacy terms hash", () => {
  const item = v2Opportunity();
  assert.equal(marketplace.isReadyToEarn(item), true);
  assert.equal(marketplace.detailUrl(item), "competition.html?bountyContract=0x1111111111111111111111111111111111111111&network=base-mainnet");

  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ verification_ready: false })), false);
  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ evidence_requirements: { program_profile: "forward-canonical-gmv-attribution-metric-v2" } })), false);
  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ funded_amount: amount(3) })), false);
});

test("non-V2 readiness still requires its canonical terms hash", () => {
  const nonV2 = v2Opportunity({
    opportunity_id: "autonomous:example",
    source_status: "claimable",
    competition_mode: null,
    evidence_requirements: {},
    next_action: { action: "claim", url: "https://agentbounties.app/claim" },
  });
  assert.equal(marketplace.isReadyToEarn(nonV2), false);
  assert.equal(marketplace.isReadyToEarn({ ...nonV2, terms_hash: "0xabc" }), true);
});

test("the board exposes unambiguous preparation, scoring, and proof phases", () => {
  const item = v2Opportunity();
  assert.match(marketplace.timingState(item, Date.parse("2026-08-24T00:00:00Z")).label, /^Starts in /);
  assert.match(marketplace.timingState(item, Date.parse("2026-08-26T00:00:00Z")).label, /^Scoring now · /);
  assert.equal(marketplace.timingState(item, Date.parse("2026-09-02T00:00:00Z")).label, "Scoring closed · proof phase");
});

test("economics includes child capital and the complete losing exposure", () => {
  const result = competition.economics(3, 0.11, 3, 0, 0.25);
  assert.ok(Math.abs(result.win - (-0.11)) < 1e-9);
  assert.ok(Math.abs(result.loss - (-3.11)) < 1e-9);
  assert.ok(Math.abs(result.expected - (-2.36)) < 1e-9);
  assert.equal(result.totalCost, 3.11);
});

test("the unified card shows complete variable-cost and losing-exposure formulas", () => {
  const item = v2Opportunity();
  const context = marketplace.decisionContext(item);
  const card = marketplace.renderOpportunity(item, 0, Date.parse("2026-08-26T00:00:00Z"));

  assert.deepEqual(context, {
    win: "If you win: 2.89 USDC minus child funding and labor",
    loss: "If you lose: child funding plus 0.11 USDC hosted costs and labor",
  });
  assert.match(card, /If you win: 2\.89 USDC minus child funding and labor/);
  assert.match(card, /If you lose: child funding plus 0\.11 USDC hosted costs and labor/);
  assert.match(card, /Calculate and participate/);
  assert.doesNotMatch(card, /published margin if you win/);
});

test("the participation manifest and prefilled child brief are contract-specific", () => {
  const item = v2Opportunity();
  const timing = marketplace.timingState(item, Date.parse("2026-08-26T00:00:00Z"));
  const manifest = competition.participationManifest(item, timing);
  const child = competition.childTemplate(item);

  assert.equal(manifest.schema_version, "agent-bounties/competition-participation-manifest-v1");
  assert.equal(manifest.competition_contract, item.source_id);
  assert.equal(manifest.network, "base-mainnet");
  assert.equal(manifest.scoring.formula, item.evidence_requirements.scoring_formula);
  assert.equal(manifest.proof_snapshot_url, item.evidence_requirements.snapshot_url);
  assert.match(manifest.hosted_proof_quote.url, /open-competition-v2-beta3\/proof-quotes$/);
  assert.equal(manifest.hosted_proof_quote.request_template.competition_contract, item.source_id);
  assert.equal(manifest.hosted_proof_quote.request_template.metric.profile_id, "forward-canonical-gmv-attribution-metric-v2");
  assert.equal(Object.hasOwn(manifest.hosted_proof_quote.request_template, "artifact_hash"), false);
  assert.match(child, new RegExp(item.source_id));
  assert.match(child, /Fully fund before another wallet claims or enters/);
  assert.match(child, /confirmed canonical settlement/);
  assert.equal(
    competition.childPostUrl(item),
    "./?parentCompetition=0x1111111111111111111111111111111111111111&network=base-mainnet#post-a-bounty",
  );
});
