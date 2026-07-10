import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  collectInventory,
  normalizeApiBaseUrl,
} from "../skills/agent-bounties/scripts/check-in.mjs";

async function fixture(name) {
  return JSON.parse(
    await readFile(new URL(`../skills/agent-bounties/fixtures/${name}`, import.meta.url), "utf8"),
  );
}

test("only reconciled real-value scoped status is claimable", async () => {
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("verified-claimable.json"),
  });

  assert.equal(report.hosted_api_healthy, true);
  assert.equal(report.verified_claimable_bounties.length, 1);
  assert.equal(report.verified_claimable_bounties[0].id, "base-valid");
  assert.equal(report.verified_claimable_bounties[0].evidence, "indexed_base_funding");
  assert.equal(
    report.verified_claimable_bounties[0].status_url,
    "https://api.example.test/v1/bounties/base-valid",
  );
  assert.deepEqual(
    report.excluded_claimable_candidates.map((item) => [item.id, item.reason]),
    [
      ["base-hash-only", "missing_indexed_base_funding"],
      ["simulated-demo", "simulated_value_is_not_earnable_money"],
    ],
  );
  assert.equal(report.recommended_action, "claim_verified_bounty");
  assert.equal(report.funding_candidates.length, 1);
});

test("unavailable hosted API cannot create imaginary inventory", async () => {
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("unavailable.json"),
  });

  assert.equal(report.hosted_api_healthy, false);
  assert.deepEqual(report.verified_claimable_bounties, []);
  assert.equal(report.recommended_action, "post_own_bounty");
  assert.ok(report.warnings.includes("hosted_api_health_not_confirmed"));
  assert.ok(report.warnings.includes("claimable_feed_unavailable"));
});

test("API URL rejects credentials and insecure remote HTTP", () => {
  assert.throws(
    () => normalizeApiBaseUrl("https://user:secret@api.example.test"),
    /credentials/,
  );
  assert.throws(() => normalizeApiBaseUrl("http://api.example.test"), /HTTPS/);
  assert.equal(normalizeApiBaseUrl("http://127.0.0.1:8080/"), "http://127.0.0.1:8080");
});
