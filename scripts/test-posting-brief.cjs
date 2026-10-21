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
assert.deepEqual(brief.proposedSplit("5.00"), { solver: "5.00", reserve: "0.50", total: "5.50" });
assert.deepEqual(brief.proposedSplit("0.02"), { solver: "0.02", reserve: "0.01", total: "0.03" });
assert.deepEqual(brief.proposedSplit("5.05"), { solver: "5.05", reserve: "0.51", total: "5.56" });
for (const invalid of ["0", "-1", "1.001", "NaN", "1e2", ""]) assert.equal(brief.proposedSplit(invalid), null);
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
  const input = { title: "Research competitors", goal: "Deliver a comparison spreadsheet", criteria: "Include 20 competitors\nLink each pricing source", solver_reward_usdc: "20.01", review_reward_usdc: "0.11", deadline_at: "2026-10-10T12:00:00-06:00", review_consent: true };
  const draft = brief.manualDraft(input, now);
  assert.equal(draft.solver_reward_usdc, "20.01");
  assert.equal(draft.verifier_reward_usdc, "0.11");
  assert.deepEqual(draft.acceptance_criteria, ["Include 20 competitors", "Link each pricing source"]);
  assert.equal(draft.review_mode, "creator");
  assert.equal(draft.delivery_deadline, input.deadline_at);
  assert.equal(draft.task_window_days, 8);
  for (const bad of [{title:""}, {criteria:""}, {criteria:Array(21).fill("check").join("\n")}, {criteria:"x".repeat(1001)}, {solver_reward_usdc:"1.99"}, {review_reward_usdc:"0"}, {solver_reward_usdc:"1e3"}, {deadline_at:"invalid"}, {deadline_at:"2026-10-01T00:00:00Z"}, {deadline_at:"2027-01-01T00:00:00Z"}, {review_consent:false}]) assert.throws(() => brief.manualDraft({...input,...bad}, now));
  assert.equal(brief.manualDraft({...input, solver_reward_usdc:"2.00",review_reward_usdc:"0.01"},now).solver_reward_usdc,"2.00");
});


test("entered reward is never reduced by verification and legacy budgets are not silently reinterpreted", () => {
  const now = Date.parse("2026-10-18T00:00:00Z");
  const input = { title: "Regression", goal: "Deliver an artifact", criteria: "Artifact opens", solver_reward_usdc: "100.00", review_reward_usdc: "10.00", deadline_at: "2026-10-20T00:00:00Z", review_consent: true };
  const draft = brief.manualDraft(input, now);
  assert.equal(draft.solver_reward_usdc, "100.00");
  assert.equal(draft.verifier_reward_usdc, "10.00");
  assert.deepEqual(brief.proposedSplit("100"), { solver: "100.00", reserve: "10.00", total: "110.00" });
  assert.equal(brief.restoredSolverReward({solver_reward_usdc:"100.00"}, draft), "100.00");
  assert.equal(brief.restoredSolverReward({budget_usdc:"110.00"}, draft), "100");
  assert.equal(brief.restoredSolverReward({budget_usdc:"100.00",review_reward_usdc:"10.00"}), "90");
  assert.equal(brief.restoredSolverReward({budget_usdc:"1.000003",review_reward_usdc:"0.000002"}), "1.000001");
  assert.equal(brief.restoredSolverReward({budget_usdc:"100.00"}), "");
  assert.equal(brief.restoredSolverReward({solver_reward_usdc:""}, draft), "");
  assert.equal(brief.restoredSolverReward({}, draft), "100.00");
  assert.throws(() => brief.manualDraft({...input,solver_reward_usdc:undefined,budget_usdc:"100.00"},now));
  for (const reserve of ["-1", "0", "NaN", "9000000000"]) assert.throws(() => brief.manualDraft({...input,review_reward_usdc:reserve},now));
});


test("legacy restoration uses the staged allocation instead of a hidden manual default", () => {
  const legacy = { budget_usdc: "20.00", review_reward_usdc: "0.10" };
  const staged = Object.freeze({ solver_reward_usdc: "18.00", verifier_reward_usdc: "2.00" });
  assert.equal(brief.restoredSolverReward(legacy, staged), "18");
  assert.equal(brief.restoredSolverReward(legacy), "19.9", "an unstaged legacy brief keeps its own allocation");
  assert.equal(brief.restoredSolverReward({ ...legacy, solver_reward_usdc: "17.00" }, staged), "17.00", "an explicit edited reward remains an explicit edit");
  assert.equal(brief.restoredSolverReward({ budget_usdc: "1.000003", review_reward_usdc: "0.10" }, { verifier_reward_usdc: "0.000002" }), "1.000001");
  for (const verifier_reward_usdc of [undefined, null, "", "0", "-1", "NaN", "20", "21"]) {
    assert.equal(brief.restoredSolverReward(legacy, { verifier_reward_usdc }), "", "an invalid staged allocation must not fall back to the hidden default");
  }
});
