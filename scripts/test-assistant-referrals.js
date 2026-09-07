"use strict";

const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const crypto = require("node:crypto");
const assert = require("node:assert/strict");
const { test } = require("node:test");
const home = require("../site/solarpunk-home.js");
const postingPrompt = require("../site/posting-prompt.js");
const source = fs.readFileSync(path.join(__dirname, "../site/analytics.js"), "utf8");

function storage() {
  const values = new Map();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, String(value)), removeItem: key => values.delete(key) };
}

function page(url, options = {}) {
  const events = [];
  const window = {
    location: new URL(url), localStorage: options.local || storage(), sessionStorage: storage(),
    crypto: crypto.webcrypto,
    fetch(endpoint, request) {
      assert.equal(endpoint, "https://api.agentbounties.app/v1/analytics/events");
      events.push(JSON.parse(request.body));
      return Promise.resolve({ ok: true });
    },
  };
  vm.runInNewContext(source, {
    window, navigator: options.privacy || {}, URL, URLSearchParams, Date, Uint8Array,
    document: { referrer: "", addEventListener() {} },
  });
  return { analytics: window.agentBountiesAnalytics, local: window.localStorage, events };
}

const landing = "https://agentbounties.app/?utm_source=example-source&utm_campaign=example-campaign&secret=never-forward&gclid=not-collected";
const review = "https://agentbounties.app/post.html?from=webmcp";

test("a new desktop browser receives referral tags without the original browser identity", () => {
  const first = page(landing);
  const destination = first.analytics.handoffUrl(review);
  const links = home.bountyAssistantLinks("gpt", undefined, destination);
  const desktop = new URL(links.desktopUrl);
  assert.equal(desktop.searchParams.get("browserUrl"), destination);
  assert.equal(desktop.searchParams.get("prompt"), new URL(links.webUrl).searchParams.get("prompt"));
  assert.match(desktop.searchParams.get("prompt"), /utm_source=example-source/);
  assert.doesNotMatch(desktop.searchParams.get("prompt"), /utm_source=chatgpt/);
  const next = page(desktop.searchParams.get("browserUrl"));
  assert.equal(next.events[0].source, "example-source");
  assert.equal(next.events[0].campaign, "example-campaign");
  assert.notEqual(next.events[0].visitor_id, first.events[0].visitor_id);
  assert.notEqual(next.events[0].session_id, first.events[0].session_id);
  for (const text of [destination, desktop.searchParams.get("prompt")]) {
    assert.ok(!text.includes(first.events[0].visitor_id));
    assert.ok(!text.includes(first.events[0].session_id));
    assert.doesNotMatch(text, /never-forward|gclid|not-collected/);
  }
});

test("web and copied assistant prompts retain the same review destination and task", () => {
  const first = page(landing);
  const destination = first.analytics.handoffUrl(review);
  const task = { request: "Keep retries idempotent.", solver_reward_usdc: "3.00" };
  const prompt = postingPrompt.build(task, destination);
  assert.ok(prompt.includes(`Use ${destination} in @Browser`));
  assert.ok(prompt.includes(JSON.stringify(task, null, 2)));
  for (const [provider, parameter] of [["claude", "q"], ["cursor", "text"]]) {
    const links = home.bountyAssistantLinks(provider, postingPrompt.build(task), destination);
    assert.equal(new URL(links.webUrl).searchParams.get(parameter), prompt);
  }
});

test("first touch and explicit destination pairs are preserved independently", () => {
  const original = page("https://agentbounties.app/?utm_source=github&utm_campaign=readme");
  const later = page(landing, { local: original.local });
  const destination = new URL(later.analytics.handoffUrl(review));
  assert.equal(destination.searchParams.get("utm_source"), "github");
  assert.equal(destination.searchParams.get("utm_campaign"), "readme");
  const explicit = `${review}&utm_source=another-source`;
  assert.equal(later.analytics.handoffUrl(explicit), explicit);
  assert.equal(new URL(later.analytics.handoffUrl(explicit)).searchParams.has("utm_campaign"), false);
});

test("opt-out and privacy signals survive the assistant transition", () => {
  const cases = [
    { url: `${landing}&analytics=off` },
    { url: landing, privacy: { globalPrivacyControl: true } },
    { url: landing, privacy: { doNotTrack: "1" } },
  ];
  for (const options of cases) {
    const first = page(options.url, options);
    assert.equal(first.events.length, 0);
    const destination = first.analytics.handoffUrl(review);
    assert.equal(new URL(destination).searchParams.get("analytics"), "off");
    const links = home.bountyAssistantLinks("gpt", undefined, destination);
    const prompt = new URL(links.webUrl).searchParams.get("prompt");
    assert.ok(prompt.includes(destination));
    assert.doesNotMatch(prompt, /utm_source=chatgpt/);
    const next = page(new URL(links.desktopUrl).searchParams.get("browserUrl"));
    assert.equal(next.events.length, 0);
    assert.equal(page("https://agentbounties.app/post.html", { local: next.local }).events.length, 0);
  }
});

test("source tags are never added to another origin or a credentialed URL", () => {
  const first = page(landing);
  for (const destination of ["https://example.org/post.html", "https://user@agentbounties.app/post.html", "javascript:alert(1)", "not a URL"]) {
    assert.equal(first.analytics.handoffUrl(destination), destination);
  }
});

test("untrusted review destinations cannot redirect the launcher or inject prompt instructions", () => {
  for (const destination of [
    "https://example.org/post.html", "https://agentbounties.app/authorize.html",
    "https://user@agentbounties.app/post.html", `${review}&wallet_signature=private`,
    `${review}&utm_source=first&utm_source=second`, `${review}&utm_campaign=bad%0Ainstructions`,
    `${review}&parentBounty=not-an-address`, `${review}#untrusted`,
  ]) {
    assert.equal(postingPrompt.reviewUrl(destination), null);
    assert.equal(postingPrompt.build(null, destination), postingPrompt.build());
    assert.equal(new URL(home.bountyAssistantLinks("gpt", undefined, destination).desktopUrl).searchParams.get("browserUrl"), review);
  }
  const parent = `${review}&parentBounty=0x${"1".repeat(40)}`;
  assert.equal(postingPrompt.reviewUrl(parent), parent);
  assert.ok(postingPrompt.build({ meta_child: { parent_bounty_contract: `0x${"1".repeat(40)}` } }, parent).includes(parent));
});
