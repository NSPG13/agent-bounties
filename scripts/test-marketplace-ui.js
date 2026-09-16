"use strict";

const test = require("node:test");
const assert = require("node:assert/strict");

const marketplace = require("../site/marketplace.js");
const workflow = require("../site/marketplace-workflow.js");
const home = require("../site/solarpunk-home.js");
// Public inventory captured during the September 16 audit; freeze its clock.
const audit = require("./fixtures/opportunities-audit-20260916.json");
const AUDIT_NOW = Date.parse(audit.generated_at);
const platform = { marketplace_payout_volume: { lifetime: { usdc: "23.75" }, lifetime_settled_rounds: 12 }, daily: [] };
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

test("direct task filtering excludes competitions, child funding and unknown mechanisms", () => {
  const competition = v2Opportunity();
  const direct = v2Opportunity({ opportunity_id: "canonical:direct", source_status: "claimable", competition_mode: "exclusive_claim", evidence_requirements: {}, terms_hash: "0xabc", next_action: { action: "prepare_agent_to_earn" } });
  const child = { ...direct, standing_meta_bounty: true };
  const legacyCompetition = { ...direct, competition_mode: "first_valid_submission" };
  const unknown = { ...direct, competition_mode: null };
  const items = [competition, direct, child, legacyCompetition, unknown];
  assert.deepEqual(marketplace.filterItems(items, "", "all", Date.parse(STARTS_AT), "direct"), [direct]);
  assert.deepEqual(marketplace.filterItems(items, "", "all", Date.parse(STARTS_AT), "competition"), [competition, legacyCompetition]);
  assert.deepEqual(marketplace.filterItems(items, "", "all", Date.parse(STARTS_AT), "child_funding"), [child]);
  assert.equal(workflow.summarize(direct).participation_kind, "direct");
  assert.equal(workflow.summarize(child).participation_kind, "child_funding");
  assert.match(marketplace.emptyState("direct"), /No direct tasks/);
  assert.doesNotMatch(marketplace.emptyState("direct"), /post your own|free|guaranteed/i);
  assert.match(marketplace.renderOpportunity(direct, 0, Date.parse(STARTS_AT)), /refundable bond, gas and execution costs/);
});

test("V2 readiness is mechanism-aware and does not depend on a legacy terms hash", () => {
  const item = v2Opportunity();
  assert.equal(marketplace.isReadyToEarn(item), true);
  assert.equal(marketplace.detailUrl(item), "competition.html?bountyContract=0x1111111111111111111111111111111111111111&network=base-mainnet");

  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ verification_ready: false })), false);
  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ evidence_requirements: { program_profile: "forward-canonical-gmv-attribution-metric-v2" } })), false);
  assert.equal(marketplace.isReadyToEarn(v2Opportunity({ funded_amount: amount(3) })), false);
});

test("audit: homepage and default board agree on 3 open, 12 scoring closed, zero direct tasks", () => {
  const { items } = workflow.fundedInventory(audit);
  assert.equal(items.length, 15);
  assert.ok(items.every(workflow.ready), "machine ready_to_earn still includes all 15 funded opportunities");
  const counts = workflow.inventoryCounts(items, AUDIT_NOW);
  assert.deepEqual(counts, { now: 3, upcoming: 0, ended: 12, closed: 0, unavailable: 0, direct: 0, competition: 3, child_funding: 0, unknown: 0 });
  const snapshot = home.marketSnapshot(platform, audit, AUDIT_NOW);
  assert.equal(snapshot.live, 3);
  assert.deepEqual(snapshot.availability, counts);
  assert.equal(marketplace.filterItems(items, "", undefined, AUDIT_NOW).length, 3);
  assert.equal(marketplace.filterItems(items, "", "ended", AUDIT_NOW).length, 12);
  assert.equal(marketplace.filterItems(items, "", "all", AUDIT_NOW).length, 15);
  assert.equal(marketplace.filterItems(items, "", "now", AUDIT_NOW, "direct").length, 0);
  for (const item of marketplace.filterItems(items, "", "ended", AUDIT_NOW)) {
    const card = marketplace.renderOpportunity(item, 0, AUDIT_NOW);
    assert.ok(card.includes(workflow.detailUrl(item).replaceAll("&", "&amp;")), "existing participants retain the exact contract link");
    assert.match(card, /Continue to proof stage/);
  }
});

test("open-now boundaries exclude future, closed, missing and malformed timing", () => {
  const entry = audit.items[0];
  const timed = (scoring_window) => ({ ...entry, evidence_requirements: { ...entry.evidence_requirements, scoring_window } });
  const at = Date.parse("2026-09-16T12:00:00Z");
  const cases = [
    [undefined, "unavailable"], [null, "unavailable"], [{}, "unavailable"],
    [{ starts_at: "invalid", ends_at: "2026-09-18T00:00:00Z" }, "unavailable"],
    [{ starts_at: "2026-09-18T00:00:00Z", ends_at: "2026-09-17T00:00:00Z" }, "unavailable"],
    [{ starts_at: "2026-09-17T00:00:00Z", ends_at: "2026-09-18T00:00:00Z" }, "upcoming"],
    [{ starts_at: "2026-09-16T12:00:00Z", ends_at: "2026-09-17T00:00:00Z" }, "now"],
    [{ starts_at: "2026-09-15T00:00:00Z", ends_at: "2026-09-16T12:00:00Z" }, "ended"],
  ];
  for (const [window, expected] of cases) {
    const item = timed(window);
    assert.equal(workflow.phase(item, at), expected);
    assert.equal(marketplace.timingState(item, at).phase, expected);
    assert.equal(marketplace.filterItems([item], "", undefined, at).length, expected === "now" ? 1 : 0);
    assert.equal(home.marketSnapshot(platform, { ...audit, items: [item] }, at).live, expected === "unavailable" ? null : expected === "now" ? 1 : 0);
    assert.equal(workflow.ready(item), true, "timing never changes the API funding predicate");
  }
});

test("direct and child-funding work require a known unexpired deadline", () => {
  const direct = { ...require("./fixtures/funded-bounty.cjs").opportunity, competition_mode: "exclusive_claim" };
  for (const standing_meta_bounty of [false, true]) {
    const item = { ...direct, standing_meta_bounty };
    assert.equal(workflow.phase(item, AUDIT_NOW), "unavailable");
    assert.equal(workflow.phase({ ...item, deadline: audit.generated_at }, AUDIT_NOW), "closed");
    assert.equal(workflow.phase({ ...item, deadline: "2026-09-17T00:00:00Z" }, AUDIT_NOW), "now");
  }
});

test("all views sort direct tasks first, then nearest relevant deadline, unknown last", () => {
  const direct = { ...require("./fixtures/funded-bounty.cjs").opportunity, competition_mode: "exclusive_claim", deadline: "2026-09-22T00:00:00Z" };
  const late = { ...audit.items[0], opportunity_id: "late", deadline: "2026-10-02T00:00:00Z" };
  const early = { ...audit.items[0], opportunity_id: "early", deadline: "2026-10-01T00:00:00Z" };
  const unknown = { ...late, opportunity_id: "unknown", deadline: null };
  assert.deepEqual(marketplace.filterItems([unknown, late, early, direct], "", "all", AUDIT_NOW), [direct, early, late, unknown]);
  assert.deepEqual(marketplace.filterItems([unknown, late, early], "", "ended", AUDIT_NOW), [early, late, unknown]);
  const future = (id, end) => ({ ...late, opportunity_id: id, evidence_requirements: { ...late.evidence_requirements, scoring_window: { starts_at: "2026-09-17T00:00:00Z", ends_at: end } } });
  const a = future("a", "2026-09-18T00:00:00Z"), b = future("b", "2026-09-19T00:00:00Z");
  assert.deepEqual(marketplace.filterItems([b, a], "", "upcoming", AUDIT_NOW), [a, b]);
});

test("homepage and board both reject unavailable inventory instead of reporting zero", async () => {
  for (const change of [
    { schema_version: "wrong" }, { network: "base-sepolia" }, { applied_view: "recent" }, { degraded: true },
    { degraded: undefined }, { source_statuses: [] }, { source_statuses: [{ source_type: "canonical_base", available: false }] },
    { items: null }, { items: [{ ...audit.items[0], verification_ready: false }] },
  ]) {
    const payload = { ...audit, items: [], ...change };
    assert.throws(() => home.marketSnapshot(platform, payload, AUDIT_NOW));
    await assert.rejects(marketplace.loadOpportunities({ location: { hostname: "agentbounties.app" }, fetch: async () => ({ ok: true, json: async () => payload }) }));
  }
  for (const fetch of [async () => ({ ok: false, status: 503 }), async () => { throw new Error("Offline"); }]) {
    await assert.rejects(workflow.loadFundedInventory({ location: { hostname: "agentbounties.app" }, fetch }));
  }
  assert.equal(home.marketSnapshot(platform, { ...audit, items: [] }, AUDIT_NOW).live, 0, "healthy empty inventory may truthfully report zero");
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
  assert.equal(marketplace.timingState(item, Date.parse("2026-09-02T00:00:00Z")).label, "Scoring closed / proof stage");
});

test("economics includes child capital and the complete losing exposure", () => {
  const result = competition.economics(3, 0.11, 3, 0, 0.25);
  assert.ok(Math.abs(result.win - (-0.11)) < 1e-9);
  assert.ok(Math.abs(result.loss - (-3.11)) < 1e-9);
  assert.ok(Math.abs(result.expected - (-2.36)) < 1e-9);
  assert.equal(result.totalCost, 3.11);
  assert.ok(Math.abs(result.breakEvenProbability - (3.11 / 3)) < 1e-9);

  const treatment = competition.economics(6, 0.11, 3, 0, 0.25);
  assert.ok(Math.abs(treatment.breakEvenProbability - (3.11 / 6)) < 1e-9);
  assert.ok(Math.abs(treatment.expected - (-1.61)) < 1e-9);
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
  assert.match(child, /Intended business use/);
  assert.match(child, /remains useful even if this competition entry loses/);
  assert.match(child, /does not by itself prove commercial usefulness/);
  assert.equal(
    competition.childPostUrl(item),
    "post.html?parentCompetition=0x1111111111111111111111111111111111111111&network=base-mainnet&from=webmcp-child",
  );
});
