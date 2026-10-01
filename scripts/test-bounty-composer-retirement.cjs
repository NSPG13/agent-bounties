"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const creator = require("../site/creator-review.js");
const source = fs.readFileSync(path.join(__dirname, "../site/bounty-composer-v2.js"), "utf8");

function composer() {
  // Exercise real browser-only functions without registering UI or wallet events.
  const marker = "  function track(eventName, details) {";
  assert.equal(source.split(marker).length, 2);
  const loaded = source.replace(marker,
    `  window.testComposer = { state, requestJson, generateInitialDraft, buildMissionPlan, termsDocument }; return;\n${marker}`);
  const requests = [], events = [];
  const element = () => ({ dataset: {}, textContent: "" });
  const document = { getElementById: element, querySelector: selector => selector === "[data-posting-options]" ? null : element(), querySelectorAll: () => [] };
  const window = {
    AgentBountiesMetaChild: {}, AgentBountiesCreatorReview: creator,
    AgentBountiesWorkflow: require("../site/marketplace-workflow.js"),
    dispatchEvent: event => events.push(event),
  };
  const fetch = async (url, options) => {
    requests.push({ url, options });
    return { ok: true, text: async () => JSON.stringify({ terms_hash: "synthetic-fixture" }) };
  };
  vm.runInNewContext(loaded, { window, document, fetch, URL, URLSearchParams,
    CustomEvent: class { constructor(type, options) { this.type = type; this.detail = options.detail; } },
  });
  return { api: window.testComposer, requests, events };
}

test("draft and objective actions hand off to the caller's AI without HTTP", async () => {
  const { api, requests, events } = composer();
  api.state.originalRequest = "Preserve this exact caller-prepared result.";
  api.state.context = ["Use the acceptance checks already agreed."];
  await api.generateInitialDraft();
  await api.buildMissionPlan();
  assert.equal(requests.length, 0);
  assert.equal(events.length, 2);
  assert.ok(events.every(event => event.type === "agent-bounties:request-ai-handoff"
    && event.detail.intent === api.state.originalRequest));
});

test("publication preserves the caller's prepared content and creator review policy", () => {
  const { api, requests } = composer();
  const prepared = creator.prepare({
    title: "Caller prepared browser request", goal: "Preserve this exact approved result.",
    acceptance_criteria: ["The prepared content is retained.", "No generated or pending demo is added."],
    review_mode: "creator", delivery_deadline: new Date(Date.now() + 86400000).toISOString(),
  });
  api.state.draft = prepared;
  api.state.sourceUrl = "https://example.invalid/prepared-by-caller";
  api.state.account = "0x" + "12".repeat(20);
  const terms = api.termsDocument({ fixture: "no wallet operation" });
  assert.equal(terms.title, prepared.title);
  assert.equal(terms.goal, prepared.goal);
  assert.deepEqual(JSON.parse(JSON.stringify(terms.acceptance_criteria)), prepared.acceptance_criteria);
  assert.equal(terms.verification_policy.engine, "creator_review_v1");
  assert.equal(terms.source_url, api.state.sourceUrl);
  assert.equal(requests.length, 0);
});

test("publication requests carry the session cookie and prevent caching and referrer leaks", async () => {
  const { api, requests } = composer();
  const prepared = { goal: "Keep the exact user-approved content." };
  await api.requestJson("http://127.0.0.1/v1/base/autonomous-bounties/terms", {
    method: "POST", body: JSON.stringify(prepared),
  });
  assert.equal(requests.length, 1);
  assert.equal(requests[0].options.credentials, "include");
  assert.equal(requests[0].options.cache, "no-store");
  assert.equal(requests[0].options.referrerPolicy, "no-referrer");
  assert.deepEqual(JSON.parse(requests[0].options.body), prepared);
});
