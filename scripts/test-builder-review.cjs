"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawnSync } = require("node:child_process");
const { score } = require("../examples/builder-review/score.cjs");
const template = require("../site/collaborate/review-template.json");
const rubric = require("../site/collaborate/rubric.json");
const NOW = Date.parse("2026-10-02T12:00:00Z");
function fixture() {
  const r = structuredClone(template);
  Object.assign(r, { artifact: { url: "https://github.com/example/example/blob/fixture/README.md", revision: "synthetic-fixture-only" }, reviewer_url: "https://github.com/example", checked_at: "2026-10-02T10:00:00Z", environment: "Synthetic test environment; no real review.", limitations: "Fixture only; no external activity, public review or publication consent." });
  for (const c of r.checks) Object.assign(c, { result: "pass", evidence: `offline synthetic output: ${c.id}`, observation: "Fixture only; supplied judgment for calculator verification." });
  return r;
}
test("unfinished template has six unknown checks and no score or authority", () => {
  const r = score(structuredClone(template), NOW);
  assert.equal(r.state, "incomplete"); assert.equal(r.score, null); assert.equal(r.not_checked, 6);
  assert.equal(r.evidence_verified_by_calculator, false); assert.equal(r.payment_authority, false);
});
test("six passes retain evidence and are reported judgments, not verified acceptance", () => {
  const input = fixture(), r = score(input, NOW);
  assert.equal(r.state, "reported_pass"); assert.equal(r.score, 6); assert.equal(r.maximum, 6);
  assert.deepEqual(r.review, input); assert.equal(r.evidence_verified_by_calculator, false);
  assert.match(r.next_step, /maintainer must reproduce/);
});
test("failed cost or recovery cannot be averaged away; missing checks remain incomplete", () => {
  for (const id of ["cost", "recovery"]) {
    const input = fixture(); input.checks.find(c => c.id === id).result = "fail";
    let r = score(input, NOW); assert.equal(r.state, "revision_needed"); assert.equal(r.failed, 1); assert.equal(r.score, 5);
    input.checks.find(c => c.id === "artifact").result = "not_checked";
    r = score(input, NOW); assert.equal(r.state, "incomplete"); assert.equal(r.failed, 1); assert.equal(r.score, null);
  }
});
test("unknown, duplicate, omitted or injected checks cannot inflate the result", () => {
  for (const change of [
    r => r.checks.pop(), r => r.checks.push(r.checks[0]), r => { r.checks[1].id = r.checks[0].id; },
    r => { r.checks[0].id = "followers"; }, r => { r.checks[0].result = "__proto__"; },
    r => { r.checks[0].result = "accepted"; }, r => { r.score = 100; },
    r => { r.rubric_version = "old"; }, r => { r.category = "trading"; },
    r => { r.category = ["read-only-integration"]; }, r => { r.checks[0].result = ["pass"]; },
  ]) { const r = fixture(); change(r); assert.throws(() => score(r, NOW)); }
});
test("assessed checks need evidence, exact artifact revision, reviewer and valid time", () => {
  for (const change of [
    r => { r.checks[0].evidence = ""; }, r => { r.checks[0].observation = " "; },
    r => { r.artifact.revision = null; }, r => { r.artifact.url = null; }, r => { r.reviewer_url = null; },
    r => { r.checked_at = null; }, r => { r.checked_at = "2026-02-30T00:00:00Z"; },
    r => { r.checked_at = "2026-10-03T00:00:00Z"; }, r => { r.limitations = ""; },
    r => { r.artifact.url = "https://user:secret@github.com/example/example"; },
    r => { r.reviewer_url = "https://github.com/example?token=secret"; },
  ]) { const r = fixture(); change(r); assert.throws(() => score(r, NOW)); }
});
test("all challenge categories have explicit reproducible checks and preserve their category", () => {
  const challenge = require("../site/collaborate/challenge.json");
  assert.deepEqual(Object.keys(rubric.category_checks).sort(), challenge.categories.map(c => c.id).sort());
  for (const category of Object.keys(rubric.category_checks)) {
    const r = fixture(); r.category = category;
    assert.equal(score(r, NOW).category, category);
    assert.equal(rubric.category_checks[category].length, 3);
  }
  assert.equal(new Set(rubric.checks.map(c => c.id)).size, 6);
  assert.equal(rubric.scoring.total_checks, rubric.checks.length);
});
test("CLI performs local accounting and invalid input exits nonzero without a zero score", () => {
  const root = path.resolve(__dirname, ".."), script = path.join(root, "examples/builder-review/score.cjs");
  const good = spawnSync(process.execPath, [script, path.join(root, "site/collaborate/review-template.json")], { encoding: "utf8" });
  assert.ifError(good.error); assert.equal(good.status, 0); assert.equal(JSON.parse(good.stdout).score, null);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "builder-review-"));
  try {
    const file = path.join(dir, "invalid.json"); fs.writeFileSync(file, "{}");
    const bad = spawnSync(process.execPath, [script, file], { encoding: "utf8" });
    assert.ifError(bad.error); assert.equal(bad.status, 1); assert.equal(bad.stdout, ""); assert.match(bad.stderr, /not a zero score/);
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
