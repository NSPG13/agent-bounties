"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const home = require("../site/solarpunk-home.js");

test("scene lighting follows the declared local-time bands", () => {
  assert.equal(home.sceneBlend(0).phase, "night");
  assert.equal(home.sceneBlend(330).phase, "dawn");
  assert.equal(home.sceneBlend(600).phase, "day");
  assert.equal(home.sceneBlend(1065).weights.day, .5);
  assert.equal(home.sceneBlend(1155).weights.dusk, .5);
  assert.equal(home.sceneBlend(1260).phase, "night");
  const transition = home.sceneBlend(300).weights;
  assert.equal(transition.dawn + transition.night, 1);
});

test("scene review overrides are localhost-only", () => {
  assert.equal(home.sceneTimeOverride("?sceneTime=23:30", "localhost"), 1410);
  assert.equal(home.sceneTimeOverride("?sceneTime=05:15", "127.0.0.1"), 315);
  assert.equal(home.sceneTimeOverride("?sceneTime=23:30", "agentbounties.app"), null);
  assert.equal(home.sceneTimeOverride("?sceneTime=25:00", "localhost"), null);
});

test("OAuth provider routes and callback messages are bounded", () => {
  assert.equal(home.authProviderPath("Google"), "/auth/login/google");
  assert.equal(home.authProviderPath("github"), "/auth/login/github");
  assert.equal(home.authProviderPath("unknown"), null);
  assert.equal(home.authResultMessage("success", "github"), "Signed in with GitHub.");
  assert.match(home.authResultMessage("error", null, "invalid_state"), /could not be verified/i);
  assert.doesNotMatch(home.authResultMessage("error", null, "unexpected-secret"), /unexpected-secret/);
});

test("bounty assistant handoffs carry one bounded initialization message", () => {
  const prompt = home.BOUNTY_POSTING_PROMPT;
  assert.match(prompt, /agentbounties\.app\/.well-known\/agent-bounties\.json/i);
  assert.match(prompt, /agentbounties\.app\/llms\.txt/i);
  assert.match(prompt, /approve it before any public write/i);
  assert.match(prompt, /never ask for a seed phrase or private key/i);
  assert.match(prompt, /confirmed canonical Base USDC evidence/i);
  assert.ok(prompt.length < 2000);

  const gpt = home.bountyAssistantLinks("GPT");
  const claude = home.bountyAssistantLinks("claude");
  const cursor = home.bountyAssistantLinks("cursor");
  const custom = home.bountyAssistantLinks("custom");
  assert.equal(gpt.desktopUrl, null);
  assert.equal(new URL(gpt.webUrl).origin, "https://chatgpt.com");
  assert.equal(new URL(gpt.webUrl).searchParams.get("prompt"), prompt);
  assert.equal(claude.desktopUrl, `claude://claude.ai/new?q=${encodeURIComponent(prompt)}`);
  assert.equal(new URL(claude.webUrl).origin, "https://claude.ai");
  assert.equal(cursor.desktopUrl, `cursor://anysphere.cursor-deeplink/prompt?text=${encodeURIComponent(prompt)}`);
  assert.equal(cursor.webUrl, "https://cursor.com/agents");
  assert.equal(cursor.webPrefillsPrompt, false);
  assert.equal(custom.webUrl, null);
  assert.equal(home.bountyAssistantLinks("unknown"), null);
});

test("wallet linking helpers encode exact EIP-191 input without exposing raw errors", () => {
  assert.equal(
    home.shortWalletAddress("0x1234567890abcdef1234567890abcdef12345678"),
    "0x123456…345678",
  );
  assert.equal(home.utf8Hex("Link wallet"), "0x4c696e6b2077616c6c6574");
  assert.match(home.walletLinkErrorMessage({ code: 4001 }), /cancelled/i);
  assert.match(home.walletLinkErrorMessage({ reason: "wallet_signature_invalid" }), /did not prove control/i);
  assert.doesNotMatch(home.walletLinkErrorMessage({ reason: "secret-provider-error" }), /secret-provider-error/);
});

test("account dashboard formats linked evidence and bounty activity", () => {
  const view = home.accountDashboardView({
    data_status: "available",
    wallets: [{ address: "0x1234567890abcdef1234567890abcdef12345678", linked_at: "2026-08-23T00:00:00Z" }],
    stats: {
      participating_bounties: 3,
      completed_posted_bounties: 12,
      earned_usdc: "1250.5",
      spent_usdc: "84",
      leaderboard_rank: 7,
    },
    activities: {
      participating: [{ title: "Audit the settlement index", status: "Evidence review" }],
      completed_posts: [{ title: "Design the agent handoff", status: "Settled" }],
    },
  });
  assert.equal(view.available, true);
  assert.equal(view.participating, "3");
  assert.equal(view.completedPosts, "12");
  assert.equal(view.earned, "1,250.50 USDC");
  assert.equal(view.spent, "84.00 USDC");
  assert.equal(view.rank, "#7");
  assert.equal(view.participatingItems[0].title, "Audit the settlement index");
  assert.equal(view.wallets[0].label, "0x123456…345678");
});

test("account dashboard never fabricates values for unlinked or malformed evidence", () => {
  const unlinked = home.accountDashboardView({
    data_status: "unavailable",
    reason: "marketplace_identity_unlinked",
  });
  assert.equal(unlinked.available, false);
  assert.equal(unlinked.earned, "—");
  assert.match(unlinked.message, /link and verify a wallet/i);

  const malformed = home.accountDashboardView({
    data_status: "available",
    stats: { participating_bounties: -1 },
    activities: { participating: [], completed_posts: [] },
  });
  assert.equal(malformed.available, false);
  assert.equal(malformed.rank, "—");
});

test("daily scene seeds are deterministic", () => {
  const first = home.seededRandom("2026-08-22:review");
  const second = home.seededRandom("2026-08-22:review");
  assert.deepEqual([first(), first(), first()], [second(), second(), second()]);
  assert.notEqual(home.seededRandom("one")(), home.seededRandom("two")());
});

test("procedural flame motion stays natural and bounded", () => {
  const samples = Array.from({ length: 240 }, (_, index) => home.flameMotion(index / 30, .73, 1.8));
  assert.ok(samples.every((sample) => sample.sway >= -1 && sample.sway <= 1));
  assert.ok(samples.every((sample) => sample.lift >= .78 && sample.lift <= 1));
  assert.ok(samples.some((sample, index) => index > 0 && Math.abs(sample.sway - samples[index - 1].sway) > .001));
  assert.ok(samples.every((sample, index) => index === 0 || Math.abs(sample.lift - samples[index - 1].lift) < .03));
});

function readyItem(overrides = {}) {
  return {
    source_type: "canonical_base",
    work_state: "claimable",
    payment_state: "escrowed",
    payment_committed: true,
    verification_ready: true,
    created_at: "2026-08-20T12:00:00Z",
    ...overrides,
  };
}

function evidence(overrides = {}) {
  return {
    applied_view: "ready_to_earn",
    degraded: false,
    source_statuses: [{ source_type: "canonical_base", available: true }],
    items: [readyItem(), readyItem({ created_at: "2026-07-01T12:00:00Z" })],
    ...overrides,
  };
}

function platform(overrides = {}) {
  return {
    marketplace_payout_volume: {
      lifetime: { usdc: "23.75" },
      lifetime_settled_rounds: 12,
    },
    daily: [
      { day: "2026-08-20", settled_rounds: 2 },
      { day: "2026-07-01", settled_rounds: 8 },
    ],
    ...overrides,
  };
}

test("market snapshot exposes only truthful canonical evidence", () => {
  const snapshot = home.marketSnapshot(platform(), evidence(), Date.parse("2026-08-22T12:00:00Z"));
  assert.deepEqual(snapshot, {
    payout: 23.75,
    live: 2,
    completed: 12,
    addedThisWeek: 1,
    completedThisWeek: 2,
  });
});

test("partial, delayed, malformed, and non-ready evidence fail closed", () => {
  assert.throws(() => home.marketSnapshot(platform(), evidence({ degraded: true })));
  assert.throws(() => home.marketSnapshot(platform(), evidence({ source_statuses: [] })));
  assert.throws(() => home.marketSnapshot(platform(), evidence({ items: [readyItem({ verification_ready: false })] })));
  assert.throws(() => home.marketSnapshot(platform({ marketplace_payout_volume: {} }), evidence()));
  assert.throws(() => home.marketSnapshot(null, null));
});
