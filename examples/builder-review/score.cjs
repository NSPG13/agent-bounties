"use strict";
// Local evidence accounting only. Does not execute artifacts, fetch links or publish.
const fs = require("node:fs");
const rubric = require("../../site/collaborate/rubric.json");
function requireThat(value) { if (!value) throw Error("Invalid builder review; use the current template and rubric."); }
function keys(value, expected) {
  requireThat(value && typeof value === "object" && !Array.isArray(value));
  requireThat(Object.keys(value).length === expected.length && expected.every(key => Object.hasOwn(value, key)));
}
function text(value, max, empty = false) {
  requireThat(typeof value === "string" && value.length <= max && (empty || value.trim().length > 0));
}
function publicURL(value, profile = false) {
  const u = new URL(value);
  requireThat(u.protocol === "https:" && !u.username && !u.password && !u.search && !u.port && u.href === value);
  requireThat(u.hostname === "github.com" || (!profile && u.hostname === "agentbounties.app"));
  if (profile) requireThat(/^\/[A-Za-z0-9][A-Za-z0-9-]{0,38}$/.test(u.pathname) && !u.hash);
  else requireThat(u.pathname.length > 1);
}
function score(review, now = Date.now()) {
  keys(review, ["schema_version", "rubric_version", "category", "artifact", "reviewer_url", "checked_at", "environment", "limitations", "checks"]);
  requireThat(review.schema_version === "agent-bounties/builder-review-v1" && review.rubric_version === rubric.version);
  requireThat(typeof review.category === "string" && Object.hasOwn(rubric.category_checks, review.category));
  keys(review.artifact, ["url", "revision"]);
  if (review.artifact.url !== null) publicURL(review.artifact.url);
  if (review.artifact.revision !== null) text(review.artifact.revision, 120);
  if (review.reviewer_url !== null) publicURL(review.reviewer_url, true);
  if (review.checked_at !== null) {
    requireThat(typeof review.checked_at === "string" && /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(review.checked_at));
    const time = Date.parse(review.checked_at);
    requireThat(Number.isFinite(time) && time <= now && new Date(time).toISOString().replace(".000Z", "Z") === review.checked_at);
  }
  text(review.environment, 2000); text(review.limitations, 2000);
  requireThat(Array.isArray(review.checks) && review.checks.length === rubric.checks.length);
  const ids = new Set(), counts = { pass: 0, fail: 0, not_checked: 0 };
  for (const check of review.checks) {
    keys(check, ["id", "result", "evidence", "observation"]);
    requireThat(rubric.checks.some(item => item.id === check.id) && !ids.has(check.id)); ids.add(check.id);
    requireThat(typeof check.result === "string" && Object.hasOwn(counts, check.result)); counts[check.result]++;
    text(check.evidence, 4000, check.result === "not_checked");
    text(check.observation, 4000, check.result === "not_checked");
    if (check.result !== "not_checked") requireThat(review.artifact.url && review.artifact.revision && review.reviewer_url && review.checked_at);
  }
  return {
    schema_version: "agent-bounties/builder-review-result-v1",
    rubric_version: rubric.version, category: review.category, artifact: review.artifact,
    state: counts.not_checked ? "incomplete" : counts.fail ? "revision_needed" : "reported_pass",
    passed: counts.pass, failed: counts.fail, not_checked: counts.not_checked,
    score: counts.not_checked ? null : counts.pass, maximum: rubric.checks.length,
    evidence_verified_by_calculator: false, payment_authority: false,
    next_step: "A maintainer must reproduce and inspect the linked evidence before acceptance or consent-based publication.",
    review
  };
}
if (require.main === module) {
  try {
    const args = process.argv.slice(2);
    requireThat(args.length === 1);
    const stat = fs.statSync(args[0]); requireThat(stat.isFile() && stat.size <= 100000);
    console.log(JSON.stringify(score(JSON.parse(fs.readFileSync(args[0], "utf8"))), null, 2));
  } catch (_) {
    console.error("Review could not be scored. Usage: node examples/builder-review/score.cjs REVIEW.json. Invalid input is not a zero score.");
    process.exitCode = 1;
  }
}
module.exports = { score };
