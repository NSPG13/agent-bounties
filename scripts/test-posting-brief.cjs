"use strict";
const assert = require("node:assert/strict");
const { test } = require("node:test");
const brief = require("../site/posting-brief.js");

// A remote browser's host zone must not change the agreed calendar deadline.
for (const hostZone of ["UTC", "Asia/Shanghai", "America/Los_Angeles"]) {
  process.env.TZ = hostZone;
  assert.equal(brief.resolveDeadline("2026-09-10T21:00", "America/Mexico_City").iso, "2026-09-10T21:00:00-06:00");
  assert.equal(brief.resolveDeadline("2026-09-10T21:00", "America/Chicago").iso, "2026-09-10T21:00:00-05:00");
  assert.equal(brief.resolveDeadline("2026-12-10T21:00", "America/Chicago").iso, "2026-12-10T21:00:00-06:00");
  assert.equal(brief.wallTime(Date.parse("2026-09-11T03:00:00Z"), "America/Mexico_City"), "2026-09-10T21:00");
}
assert.match(brief.resolveDeadline("2026-03-08T02:30", "America/Chicago").error, /does not exist/);
assert.match(brief.resolveDeadline("2026-11-01T01:30", "America/Chicago").error, /occurs twice/);
assert.equal(brief.resolveDeadline("2026-11-01T01:30", "-05:00").iso, "2026-11-01T01:30:00-05:00");
assert.equal(brief.resolveDeadline("2026-11-01T01:30", "-06:00").iso, "2026-11-01T01:30:00-06:00");
assert.equal(brief.resolveDeadline("2026-09-10T21:00", "Asia/Kathmandu").iso, "2026-09-10T21:00:00+05:45");
for (const [date, zone] of [["2026-02-30T21:00", "UTC"], ["2026-09-10T21:00", "CST"], ["2026-09-10T21:00", "+14:30"], ["2026-09-10T21:00", "Wrong/Zone"]]) {
  assert.equal(brief.resolveDeadline(date, zone).iso, null);
  assert.ok(brief.resolveDeadline(date, zone).error);
}
assert.deepEqual(brief.proposedSplit("5.00"), { solver: "4.50", reserve: "0.50", total: "5.00" });
assert.deepEqual(brief.proposedSplit("0.02"), { solver: "0.01", reserve: "0.01", total: "0.02" });
assert.deepEqual(brief.proposedSplit("5.05"), { solver: "4.54", reserve: "0.51", total: "5.05" });
for (const invalid of ["0.01", "-1", "1.001", "NaN", "1e2", ""]) assert.equal(brief.proposedSplit(invalid), null);
const now = Date.parse("2026-09-11T02:00:00Z");
const deadline = "2026-09-10T21:00:00-06:00";
assert.equal(brief.countdown(deadline, now), "1h 0m remaining");
assert.equal(brief.countdown(deadline, now + 3599000), "1 minute remaining");
assert.match(brief.countdown(deadline, now + 3600000), /Deadline passed/);
assert.equal(brief.countdown(null, now), "");
const draft = { solver_reward_usdc: "3.00", acceptance_criteria: Array(7).fill("Deliver one checked artifact") };
const warnings = brief.warnings({ goal: "Make a 30 second video", budget: "5.00", deadline, draft }, now);
assert.equal(warnings.length, 2);
assert.match(warnings[0], /two hours/);
assert.match(warnings[1], /3\.00 USDC/);
assert.match(warnings[1], /scope guidance/);
assert.deepEqual(brief.warnings({ goal: "Correct a typo", budget: "5", deadline: "2026-09-20T21:00:00-06:00" }, now), []);
assert.deepEqual(brief.warnings({}), []);
console.log("Posting brief: exact calendar zones, DST boundaries, cent splits and urgency checks passed.");

test("budget estimator keeps assumptions explicit and rounds work up to a cent", () => {
  const { estimateBudget } = require("../site/posting-brief.js");
  assert.deepEqual(estimateBudget("1.01", "20.01", "1"), { solver: "20.22", reserve: "1.00", total: "21.22" });
  assert.equal(estimateBudget("-1", "20", "1"), null);
  assert.equal(estimateBudget("1", "Infinity", "1"), null);
  assert.equal(estimateBudget("100000", "100000", "1"), null);
});


test("manual form preserves explicit terms without an AI and rejects incomplete or unfundable drafts", () => {
  const now = Date.parse("2026-10-03T00:00:00Z");
  const input = { title: "Research competitors", goal: "Deliver a comparison spreadsheet", criteria: "Include 20 competitors\nLink each pricing source", budget_usdc: "20.01", review_reward_usdc: "0.11", deadline_at: "2026-10-10T12:00:00-06:00", review_consent: true };
  const draft = brief.manualDraft(input, now);
  assert.equal(draft.solver_reward_usdc, "19.90");
  assert.equal(draft.verifier_reward_usdc, "0.11");
  assert.deepEqual(draft.acceptance_criteria, ["Include 20 competitors", "Link each pricing source"]);
  assert.equal(draft.review_mode, "creator");
  assert.equal(draft.delivery_deadline, input.deadline_at);
  assert.equal(draft.task_window_days, 8);
  for (const bad of [{title:""}, {criteria:""}, {criteria:Array(21).fill("check").join("\n")}, {criteria:"x".repeat(1001)}, {budget_usdc:"2.10"}, {review_reward_usdc:"0"}, {budget_usdc:"1e3"}, {deadline_at:"invalid"}, {deadline_at:"2026-10-01T00:00:00Z"}, {deadline_at:"2027-01-01T00:00:00Z"}, {review_consent:false}]) assert.throws(() => brief.manualDraft({...input,...bad}, now));
  assert.equal(brief.manualDraft({...input, budget_usdc:"2.01",review_reward_usdc:"0.01"},now).solver_reward_usdc,"2.00");
});
