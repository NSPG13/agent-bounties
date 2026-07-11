import assert from "node:assert/strict";
import { access, readFile } from "node:fs/promises";
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

test("portable skill metadata and install contracts remain publishable", async () => {
  const skill = await readFile(
    new URL("../skills/agent-bounties/SKILL.md", import.meta.url),
    "utf8",
  );
  const readme = await readFile(new URL("../README.md", import.meta.url), "utf8");
  const distribution = await readFile(
    new URL("../docs/openclaw-distribution.md", import.meta.url),
    "utf8",
  );
  const grouping = JSON.parse(
    await readFile(new URL("../skills.sh.json", import.meta.url), "utf8"),
  );

  assert.match(skill, /^---\r?\nname: agent-bounties\r?\n/);
  assert.match(skill, /\r?\nversion: 1\.0\.0\r?\n/);
  assert.match(skill, /\r?\nauthor: Agent Bounties contributors\r?\n/);
  assert.match(skill, /\r?\n  hermes:\r?\n/);
  assert.match(skill, /\r?\n    category: agent-commerce\r?\n/);
  assert.match(skill, /\r?\n  openclaw:\r?\n/);
  assert.match(skill, /\r?\n      bins: \[node\]\r?\n/);

  assert.equal(grouping.$schema, "https://skills.sh/schemas/skills.sh.schema.json");
  const categories = grouping.groupings.filter((item) =>
    item.skills.includes("agent-bounties"),
  );
  assert.deepEqual(categories.map((item) => item.title), ["Agent Commerce"]);

  const commands = [
    "npx skills add NSPG13/agent-bounties --skill agent-bounties --yes",
    "hermes skills install NSPG13/agent-bounties/skills/agent-bounties",
    "openclaw skills install git:NSPG13/agent-bounties@main --as agent-bounties",
  ];
  for (const command of commands) {
    assert.ok(readme.includes(command), `README is missing ${command}`);
    assert.ok(distribution.includes(command), `distribution docs are missing ${command}`);
  }

  const bundleFiles = [
    "LICENSE",
    "SKILL.md",
    "fixtures/unavailable.json",
    "fixtures/verified-claimable.json",
    "references/payment-truth.md",
    "scripts/check-in.mjs",
  ];
  for (const path of bundleFiles) {
    await access(new URL(`../skills/agent-bounties/${path}`, import.meta.url));
  }
});

test("only active canonical autonomous inventory is claimable", async () => {
  const report = await collectInventory({
    apiBaseUrl: "https://api.example.test",
    fixture: await fixture("verified-claimable.json"),
  });

  assert.equal(report.hosted_api_healthy, true);
  assert.equal(report.protocol_status, "active");
  assert.equal(report.verified_claimable_bounties.length, 1);
  assert.equal(
    report.verified_claimable_bounties[0].id,
    "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  );
  assert.equal(
    report.verified_claimable_bounties[0].evidence,
    "confirmed_canonical_autonomous_bounty",
  );
  assert.equal(
    report.verified_claimable_bounties[0].claim_plan_url,
    "https://api.example.test/v1/base/autonomous-bounties/claim-plan",
  );
  assert.deepEqual(
    report.excluded_claimable_candidates.map((item) => [item.id, item.reason]),
    [
      [
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "terms_or_contract_commitments_invalid",
      ],
    ],
  );
  assert.equal(report.recommended_action, "claim_verified_bounty");
  assert.equal(report.funding_candidates.length, 1);
  assert.equal(report.live_verification_jobs.length, 1);
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
  assert.ok(report.warnings.includes("autonomous_feed_unavailable"));
  assert.ok(report.warnings.includes("autonomous_protocol_not_active"));
});

test("API URL rejects credentials and insecure remote HTTP", () => {
  assert.throws(
    () => normalizeApiBaseUrl("https://user:secret@api.example.test"),
    /credentials/,
  );
  assert.throws(() => normalizeApiBaseUrl("http://api.example.test"), /HTTPS/);
  assert.equal(normalizeApiBaseUrl("http://127.0.0.1:8080/"), "http://127.0.0.1:8080");
});
