"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");
const metrics = require("../site/metrics.js");

const NOW = Date.parse("2026-08-12T20:22:19Z");

test("public dashboard presents one marketplace without mechanism counters", () => {
  const html = fs.readFileSync(path.join(__dirname, "..", "site", "metrics.html"), "utf8");
  assert.doesNotMatch(html, /Open Competition V[12]/i);
  assert.doesNotMatch(html, /standing meta/i);
  assert.doesNotMatch(html, /autonomous inventory/i);
  assert.match(html, /Active funded opportunities/i);
});

function platform(overrides = {}) {
  return {
    generated_at: "2026-08-12T20:21:19Z",
    platform_active_identities: {
      selected: 3,
      previous: 2,
      latest_week: 3,
      previous_week: 2,
      first_month: 4,
      lifetime: 5,
      roles: [
        { role: "posters", active_identities: 1 },
        { role: "commenters", active_identities: 1 },
      ],
    },
    daily: [
      { day: "2026-08-11", active_identities: 2, payout: { usdc: "1.000000" }, settled_rounds: 1 },
      { day: "2026-08-12", active_identities: 1, payout: { usdc: "0.100000" }, settled_rounds: 0 },
    ],
    coverage: { status: "ready" },
    ...overrides,
  };
}

function github(overrides = {}) {
  return {
    generated_at: "2026-08-12T20:00:00Z",
    periods: {
      "7d": {
        active_identities: 4,
        previous_active_identities: 1,
        roles: [
          { role: "issue_posters", active_identities: 1 },
          { role: "commenters", active_identities: 2 },
        ],
        daily: [
          { day: "2026-08-11", active_identities: 3 },
          { day: "2026-08-12", active_identities: 1 },
        ],
      },
      lifetime: { active_identities: 9 },
    },
    weekly: { latest_active_identities: 4, previous_active_identities: 1 },
    first_month: { active_identities: 7 },
    coverage: { status: "ready" },
    repository_acquisition: {
      generated_at: "2026-08-12T20:00:00Z",
      clone_events: 9654,
      unique_cloners: 446,
      page_views: 883,
      unique_visitors: 218,
      coverage: { status: "ready", unique_audiences_are_additive: false },
    },
    ...overrides,
  };
}

test("weekly growth handles empty, new, positive, and negative periods", () => {
  assert.equal(metrics.weeklyGrowth(0, 0), "0%");
  assert.equal(metrics.weeklyGrowth(3, 0), "New");
  assert.equal(metrics.weeklyGrowth(3, 2), "+50%");
  assert.equal(metrics.weeklyGrowth(1, 2), "-50%");
});

test("namespaced platform and GitHub aggregates add without cross-provider dedupe", () => {
  const merged = metrics.mergeMetrics(platform(), github(), "7d", NOW);
  assert.equal(merged.status, "ready");
  assert.equal(merged.active_complete, true);
  assert.equal(merged.active_identities, 7);
  assert.equal(merged.previous_active_identities, 3);
  assert.equal(merged.first_month_identities, 11);
  assert.equal(merged.lifetime_identities, 14);
  assert.deepEqual(
    merged.daily.map((day) => [day.day, day.active_identities, day.payout_usdc]),
    [
      ["2026-08-11", 5, 1],
      ["2026-08-12", 2, 0.1],
    ],
  );
  assert.equal(merged.roles.find((role) => role.role === "Posters").active_identities, 2);
  assert.equal(merged.roles.find((role) => role.role === "Commenters").active_identities, 3);
});

test("missing and partial sources are explicit and never treated as complete", () => {
  const missing = metrics.mergeMetrics(platform(), null, "7d", NOW);
  assert.equal(missing.status, "partial");
  assert.equal(missing.active_complete, false);
  assert.equal(missing.active_identities, 3);

  const partial = metrics.mergeMetrics(
    platform(),
    github({ coverage: { status: "partial" } }),
    "7d",
    NOW,
  );
  assert.equal(partial.status, "partial");
  assert.equal(partial.active_complete, false);

  const partialAndDelayed = metrics.mergeMetrics(
    platform({ generated_at: "2026-08-12T20:00:00Z" }),
    github({ coverage: { status: "partial" } }),
    "7d",
    NOW,
  );
  assert.equal(partialAndDelayed.status, "partial");
});

test("stale required data is delayed and unavailable platform is blocking", () => {
  const stalePlatform = platform({ generated_at: "2026-08-12T20:00:00Z" });
  assert.equal(metrics.mergeMetrics(stalePlatform, github(), "7d", NOW).status, "delayed");
  assert.equal(metrics.mergeMetrics(null, github(), "7d", NOW).status, "unavailable");
});

test("empty series is valid and charts expose keyboard-focusable evidence points", () => {
  assert.deepEqual(metrics.mergeDaily([], []), []);
  assert.equal(metrics.chartSvg([], "active_identities", "identities"), "");
  const svg = metrics.chartSvg(
    [
      { day: "2026-08-11", active_identities: 1 },
      { day: "2026-08-12", active_identities: 2 },
    ],
    "active_identities",
    "identities",
  );
  assert.match(svg, /role="img"/);
  assert.match(svg, /tabindex="0"/);
  assert.match(svg, /<title>2026-08-12: 2 active identities<\/title>/);
});

test("lifetime acquisition lookback is bounded to the public API contract", () => {
  assert.equal(metrics.acquisitionWindowHours("7d", NOW), 168);
  assert.ok(metrics.acquisitionWindowHours("lifetime", NOW) >= 1);
  assert.ok(metrics.acquisitionWindowHours("lifetime", Date.parse("2028-08-12T00:00:00Z")) <= 8760);
});

test("repository traffic has an independent honest status and comparable baseline", () => {
  assert.equal(metrics.dashboardStatus("ready", "ready"), "ready");
  assert.equal(metrics.dashboardStatus("ready", "unavailable"), "partial");
  assert.equal(metrics.dashboardStatus("ready", "delayed"), "delayed");
  assert.equal(metrics.dashboardStatus("unavailable", "ready"), "unavailable");
  assert.equal(metrics.ratioMultiple(9654, 2448).toFixed(1), "3.9");
  assert.equal(metrics.ratioMultiple(9654, 0), null);
});

test("interface usage aggregates fixed API CLI and MCP rows without claiming users", () => {
  const summary = metrics.interfaceUsageSummary({
    interfaces: [
      { interface: "mcp", protocol_era: "legacy", request_count: 20, successful_request_count: 19, first_observed_at: "2026-08-13T17:00:00Z", last_observed_at: "2026-08-13T17:12:00Z" },
      { interface: "cli", protocol_era: "not_applicable", request_count: 7, successful_request_count: 7, first_observed_at: "2026-08-13T17:02:00Z", last_observed_at: "2026-08-13T17:13:00Z" },
      { interface: "mcp", protocol_era: "http_adapter", request_count: 5, successful_request_count: 5, first_observed_at: "2026-08-13T17:03:00Z", last_observed_at: "2026-08-13T17:14:00Z" },
      { interface: "mcp", protocol_era: "modern", request_count: 5, successful_request_count: 5, first_observed_at: "2026-08-13T17:04:00Z", last_observed_at: "2026-08-13T17:15:00Z" },
      { interface: "api", protocol_era: "not_applicable", request_count: 3, successful_request_count: 3, first_observed_at: "2026-08-13T17:05:00Z", last_observed_at: "2026-08-13T17:16:00Z" },
    ],
  });

  assert.equal(summary.status, "ready");
  assert.equal(summary.rows.length, 5);
  assert.equal(summary.request_count, 40);
  assert.equal(summary.successful_request_count, 39);
  assert.equal(summary.success_rate, 39 / 40);
  assert.equal(summary.mcp_request_count, 30);
  assert.equal(summary.mcp_share, 0.75);
  assert.equal(summary.rows.find((row) => row.key === "mcp:legacy").success_rate, 19 / 20);
  assert.equal(summary.first_observed_at, "2026-08-13T17:00:00.000Z");
  assert.equal(summary.last_observed_at, "2026-08-13T17:16:00.000Z");
});

test("discoverability scorecard accepts only complete fresh aggregate providers", () => {
  const response = {
    schema_version: "agent-bounties/discoverability-summary-v1",
    status: "ready",
    generated_at: "2026-08-24T12:00:00Z",
    sources: ["search_console", "github", "first_party", "external_interfaces"].map((provider, index) => ({
      provider,
      available: true,
      stale: false,
      data_through: `2026-08-${20 + index}T00:00:00Z`,
    })),
    human_reach: {
      search_impressions: 350,
      organic_clicks: 5,
      google_average_position: 7.8,
      github_unique_visitors: 300,
      captured_chatgpt_referrals: 38,
      opportunity_feed_clicks: 25,
      market_to_funded_opportunity_ctr: 0.058,
    },
    automation_reach: {
      a2a_interactions: 11,
      mcp_interactions: 12,
      api_cli_interactions: 13,
      feed_interactions: 14,
      github_unique_cloners: 530,
    },
  };
  const summary = metrics.discoverabilitySummary(response);
  assert.equal(summary.status, "ready");
  assert.equal(summary.human_reach.search_impressions, 350);
  assert.equal(summary.automation_reach.github_unique_cloners, 530);
  assert.equal(summary.data_through, "2026-08-20T00:00:00.000Z");
});

test("discoverability scorecard fails closed for stale, partial, malformed, or invalid counts", () => {
  const ready = {
    schema_version: "agent-bounties/discoverability-summary-v1",
    status: "ready",
    generated_at: "2026-08-24T12:00:00Z",
    sources: ["search_console", "github", "first_party", "external_interfaces"].map((provider) => ({
      provider, available: true, stale: false, data_through: "2026-08-23T00:00:00Z",
    })),
    human_reach: {
      search_impressions: 1, organic_clicks: 1, google_average_position: 1,
      github_unique_visitors: 1, captured_chatgpt_referrals: 1,
      opportunity_feed_clicks: 1, market_to_funded_opportunity_ctr: 0.058,
    },
    automation_reach: {
      a2a_interactions: 1, mcp_interactions: 1, api_cli_interactions: 1,
      feed_interactions: 1, github_unique_cloners: 1,
    },
  };
  assert.equal(metrics.discoverabilitySummary({ ...ready, status: "unavailable" }).status, "unavailable");
  assert.equal(metrics.discoverabilitySummary({ ...ready, sources: ready.sources.slice(1) }).status, "unavailable");
  assert.equal(metrics.discoverabilitySummary({ ...ready, sources: ready.sources.map((source, index) => index ? source : { ...source, stale: true }) }).status, "unavailable");
  assert.equal(metrics.discoverabilitySummary({ ...ready, human_reach: { ...ready.human_reach, search_impressions: -1 } }).status, "unavailable");
});

test("interface usage handles empty coverage duplicate buckets and malformed success counts", () => {
  assert.equal(metrics.interfaceUsageSummary(null).status, "unavailable");

  const empty = metrics.interfaceUsageSummary({ interfaces: [] });
  assert.equal(empty.status, "ready");
  assert.equal(empty.request_count, 0);
  assert.equal(empty.mcp_share, null);
  assert.equal(empty.rows.every((row) => row.request_count === 0), true);

  const combined = metrics.interfaceUsageSummary({
    interfaces: [
      { interface: "api", protocol_era: "not_applicable", request_count: 2, successful_request_count: 9 },
      { interface: "api", protocol_era: "not_applicable", request_count: 3, successful_request_count: -1 },
      { interface: "unknown", protocol_era: "future", request_count: 100, successful_request_count: 100 },
    ],
  });
  assert.equal(combined.request_count, 5);
  assert.equal(combined.successful_request_count, 2);
  assert.equal(combined.rows.find((row) => row.key === "api:not_applicable").request_count, 5);
  assert.equal(combined.rows.find((row) => row.key === "api:not_applicable").successful_request_count, 2);
});

test("canonical payout audit reconciles exact public event arithmetic", () => {
  const autonomous = [
    {
      kind: "bounty_settled",
      contract_address: "0x9999999999999999999999999999999999999999",
      bounty_id: "0xaaa",
      tx_hash: "0x111",
      block_number: 11,
      log_index: 4,
      occurred_at: "2026-08-11T12:00:00Z",
      data: { round: 1, solver_reward: 1_000_000, verifier_reward: 100_000, timeout_bond_bonus: 50_000 },
    },
    {
      kind: "submission_rejected",
      contract_address: "0x2222222222222222222222222222222222222222",
      bounty_id: "0xbbb",
      tx_hash: "0x222",
      block_number: 12,
      log_index: 5,
      occurred_at: "2026-08-12T12:00:00Z",
      data: { round: 2, verifier_reward: 100_000, forfeited_bond: 9_000_000 },
    },
    {
      kind: "bounty_settled",
      contract_address: "0x3333333333333333333333333333333333333333",
      bounty_id: "0xccc",
      tx_hash: "0x333",
      block_number: 13,
      log_index: 6,
      occurred_at: "2026-08-13T00:00:00Z",
      data: { solver_reward: 99_000_000, verifier_reward: 0, timeout_bond_bonus: 0 },
    },
  ];
  const competition = {
    events: [
      {
        kind: "competition_submission_rejected",
        contract_address: "0x4444444444444444444444444444444444444444",
        bounty_id: "0xddd",
        tx_hash: "0x444",
        block_number: 14,
        log_index: 7,
        occurred_at: "2026-08-11T13:00:00Z",
        data: { submission_sequence: 1, bond_paid_to_verifier: 200_000, refund: 8_000_000 },
      },
      {
        kind: "bounty_settled",
        contract_address: "0x5555555555555555555555555555555555555555",
        bounty_id: "0xeee",
        tx_hash: "0x555",
        block_number: 15,
        log_index: 8,
        occurred_at: "2026-08-12T13:00:00Z",
        data: { submission_sequence: 2, solver_reward: 2_000_000, verifier_reward: 200_000, timeout_bond_bonus: 75_000 },
      },
    ],
  };
  const competitionV2 = {
    events: [
      {
        kind: "competition_settled",
        contract_address: "0x6666666666666666666666666666666666666666",
        bounty_id: "0xfff",
        tx_hash: "0x666",
        block_number: 16,
        log_index: 9,
        occurred_at: "2026-08-12T14:00:00Z",
        data: { solver_reward: 3_000_000, keeper_reward: 40_000 },
      },
    ],
  };
  const rows = metrics.canonicalPayoutRows(autonomous, competition, competitionV2, {
    started_at: "2026-08-11T00:00:00Z",
    ended_at: "2026-08-13T00:00:00Z",
  });
  const summary = metrics.payoutAuditSummary(rows);

  assert.equal(rows.length, 5);
  assert.equal(rows[0].tx_hash, "0x666");
  assert.match(rows[0].explorer_url, /basescan\.org\/tx\/0x666#eventlog$/);
  assert.match(rows[0].api_url, /bounty_id=0xfff$/);
  assert.equal(rows[0].protocol, "Open competition");
  assert.equal(rows[0].keeper_base_units, 40_000);
  assert.equal(rows.some((row) => row.contract_address.startsWith("0x9999")), true);
  assert.deepEqual(summary, {
    payout_events: 5,
    settlement_events: 3,
    solver_base_units: 6_000_000,
    verifier_base_units: 600_000,
    keeper_base_units: 40_000,
    bonus_base_units: 125_000,
    total_base_units: 6_765_000,
  });
});

test("policy-excluded payout values preserve exact USDC precision", () => {
  assert.equal(metrics.formatUsdc(0.525, { maximumFractionDigits: 6 }), "0.525 USDC");
});

test("public policy excludes two canaries and exact current aggregate reconciles", () => {
  const policy = JSON.parse(fs.readFileSync(
    path.join(__dirname, "..", "site", "generated", "public-metrics-policy.json"),
    "utf8",
  ));
  const occurredAt = "2026-08-12T12:00:00Z";
  const autonomous = [];
  for (let index = 0; index < 36; index += 1) {
    autonomous.push({
      kind: "bounty_settled",
      contract_address: `0x${String(index + 1).padStart(40, "0")}`,
      bounty_id: `autonomous-${index}`,
      tx_hash: `0xa${index}`,
      block_number: 100 + index,
      log_index: 0,
      occurred_at: occurredAt,
      data: {
        solver_reward: 1_000_000,
        verifier_reward: 40_000,
        timeout_bond_bonus: index === 0 ? 30_000 : 0,
      },
    });
  }
  for (let index = 0; index < 7; index += 1) {
    autonomous.push({
      kind: "submission_rejected",
      contract_address: `0x${String(index + 101).padStart(40, "0")}`,
      bounty_id: `rejection-${index}`,
      tx_hash: `0xb${index}`,
      block_number: 200 + index,
      log_index: 0,
      occurred_at: occurredAt,
      data: { verifier_reward: 100_000 },
    });
  }
  const competition = { events: [{
    kind: "bounty_settled",
    contract_address: "0x7777777777777777777777777777777777777777",
    bounty_id: "competition-v1",
    tx_hash: "0xc1",
    block_number: 300,
    log_index: 0,
    occurred_at: occurredAt,
    data: { solver_reward: 3_190_000, verifier_reward: 230_000, timeout_bond_bonus: 0 },
  }] };
  const competitionV2 = { events: [] };
  for (let index = 0; index < 5; index += 1) {
    competitionV2.events.push({
      kind: "competition_settled",
      contract_address: `0x${String(index + 201).padStart(40, "0")}`,
      bounty_id: `competition-v2-${index}`,
      tx_hash: `0xd${index}`,
      block_number: 400 + index,
      log_index: 0,
      occurred_at: occurredAt,
      data: { solver_reward: 3_000_000, keeper_reward: 40_000 },
    });
  }
  [policy.excluded_bounty_contracts[0], policy.excluded_bounty_contracts[3]].forEach((contract, index) => {
    competitionV2.events.push({
      kind: "competition_settled",
      contract_address: index === 0 ? contract.toUpperCase().replace("0X", "0x") : contract,
      bounty_id: `canary-${index}`,
      tx_hash: `0xe${index}`,
      block_number: 500 + index,
      log_index: 0,
      occurred_at: occurredAt,
      data: { solver_reward: 250_000, keeper_reward: 12_500 },
    });
  });
  const aggregate = {
    generated_at: occurredAt,
    window: { started_at: "2026-08-01T00:00:00Z", ended_at: "2026-08-13T00:00:00Z" },
    marketplace_payout_volume: {
      selected: { usdc_base_units: "56790000" },
      selected_solver_pay: { usdc_base_units: "54190000" },
      selected_verifier_pay: { usdc_base_units: "2370000" },
      selected_keeper_pay: { usdc_base_units: "200000" },
      selected_completion_bonus: { usdc_base_units: "30000" },
      selected_settled_rounds: 42,
    },
  };

  const audit = metrics.payoutAuditSnapshot(aggregate, autonomous, competition, competitionV2, policy);
  assert.equal(audit.status, "ready");
  assert.deepEqual(audit.summary, {
    payout_events: 49,
    settlement_events: 42,
    solver_base_units: 54_190_000,
    verifier_base_units: 2_370_000,
    keeper_base_units: 200_000,
    bonus_base_units: 30_000,
    total_base_units: 56_790_000,
  });
  assert.equal(audit.excluded_summary.payout_events, 2);
  assert.equal(audit.excluded_summary.settlement_events, 2);
  assert.equal(audit.excluded_summary.total_base_units, 525_000);
});

test("missing or malformed public policy makes the proof ledger unavailable", () => {
  const aggregate = {
    window: { started_at: "2026-08-01T00:00:00Z", ended_at: "2026-08-13T00:00:00Z" },
    marketplace_payout_volume: {},
  };
  const streams = [[], { events: [] }, { events: [] }];
  assert.equal(metrics.payoutAuditSnapshot(aggregate, ...streams, null).status, "unavailable");
  assert.equal(metrics.payoutAuditSnapshot(aggregate, ...streams, {
    schema_version: "agent-bounties/public-metrics-policy-v1",
    maintainer_github_logins: [],
    maintainer_comment_authors: [],
    maintainer_wallets: [],
    excluded_bounty_contracts: ["not-an-address"],
    wallet_ownership_boundary: "Declared addresses only.",
  }).status, "unavailable");
});
