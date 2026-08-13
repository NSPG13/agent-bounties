"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const metrics = require("../site/metrics.js");

const NOW = Date.parse("2026-08-12T20:22:19Z");

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
