"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const home = require("../site/solarpunk-home.js");
const postingPrompt = require("../site/posting-prompt.js");

test("rolling auth deployments still require explicit server wallet proof", () => {
  const wallets = [{address:"0x" + "11".repeat(20)}];
  assert.equal(home.accountSetupStatus({identity_link_status:"verified"},wallets),"ready");
  assert.equal(home.accountSetupStatus({linked:true},wallets),"ready");
  assert.equal(home.accountSetupStatus({unlinked:true},[]),"wallet_required");
  assert.equal(home.accountSetupStatus({reason:"marketplace_identity_unlinked"},[]),"wallet_required");
  assert.equal(home.accountSetupStatus({},wallets),"unavailable");
  assert.equal(home.accountSetupStatus({linked:true},[]),"unavailable");
  assert.equal(home.accountSetupStatus({identity_link_status:"verified",account_status:"unavailable",account_complete:false},wallets),"unavailable");
  assert.equal(home.accountSetupStatus({linked:true,account_complete:false},wallets),"unavailable");
  assert.equal(home.accountSetupStatus({account_status:"wallet_required",account_complete:true},[]),"unavailable");
});

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
  assert.equal(
    home.authProviderPath("microsoft", { hostname: "agentbounties.app" }),
    "https://api.agentbounties.app/v1/site-auth/login/microsoft",
  );
  assert.equal(
    home.authApiPath("/session", { hostname: "agentbounties.app" }),
    "https://api.agentbounties.app/v1/site-auth/session",
  );
  assert.equal(home.authResultMessage("success", "github"), "Signed in with GitHub.");
  assert.match(home.authResultMessage("error", null, "invalid_state"), /could not be verified/i);
  assert.doesNotMatch(home.authResultMessage("error", null, "unexpected-secret"), /unexpected-secret/);
});

test("bounty assistant handoffs carry one bounded initialization message", () => {
  const prompt = home.BOUNTY_POSTING_PROMPT;
  assert.equal(prompt, postingPrompt.build());
  assert.match(prompt, /agentbounties\.app\/.well-known\/agent-bounties\.json/i);
  assert.match(prompt, /agentbounties\.app\/llms\.txt/i);
  assert.match(prompt, /Leave publication, legal, funding and payment consent to me/i);
  assert.match(prompt, /Leave wallet confirmations to me/i);
  assert.match(prompt, /discover WebMCP tools/i);
  assert.match(prompt, /preserve my answers and draft/i);
  assert.match(prompt, /only for missing business decisions/i);
  assert.match(prompt, /Never request private keys or seed phrases/i);
  assert.match(prompt, /confirmed canonical Base USDC evidence/i);
  assert.match(prompt, /check funding readiness/);
  assert.match(prompt, /phone-wallet QR pairing/);
  assert.match(prompt, /resume automatically/);
  assert.match(prompt, /never repeat uncertain transactions/);
  assert.match(prompt, /canonical creation, funding and claimability/);
  assert.match(prompt, /public ready-to-earn inventory/);
  assert.match(prompt, /Return its public link/);
  assert.match(prompt, /posting and funding remain incomplete/);
  assert.ok(prompt.length < 2500);

  const gpt = home.bountyAssistantLinks("GPT");
  const claude = home.bountyAssistantLinks("claude");
  const cursor = home.bountyAssistantLinks("cursor");
  const custom = home.bountyAssistantLinks("custom");
  const desktop = new URL(gpt.desktopUrl);
  assert.equal(desktop.protocol, "codex:");
  assert.equal(desktop.hostname, "threads");
  assert.equal(desktop.pathname, "/new");
  assert.equal(desktop.searchParams.get("browserUrl"), "https://agentbounties.app/post.html?from=webmcp");
  assert.equal(new URL(gpt.webUrl).origin, "https://chatgpt.com");
  const gptPrompt = new URL(gpt.webUrl).searchParams.get("prompt");
  assert.ok(gptPrompt.startsWith(prompt));
  assert.match(gptPrompt, /utm_source=chatgpt/);
  assert.match(gptPrompt, /canonical URL/);
  assert.equal(desktop.searchParams.get("prompt"), gptPrompt);
  assert.equal(desktop.searchParams.has("submit"), false);
  assert.equal(gpt.webPrefillsPrompt, true);
  // Claude Code's own scheme, not the Claude Desktop chat scheme.
  assert.equal(claude.desktopUrl, `claude-cli://open?q=${encodeURIComponent(prompt)}`);
  assert.equal(new URL(claude.desktopUrl).protocol, "claude-cli:");
  assert.equal(claude.desktopLabel, "Claude Code");
  assert.equal(new URL(claude.webUrl).origin, "https://claude.ai");
  assert.equal(new URL(claude.webUrl).searchParams.get("q"), prompt);
  assert.equal(cursor.desktopUrl, `cursor://anysphere.cursor-deeplink/prompt?text=${encodeURIComponent(prompt)}`);
  assert.equal(new URL(cursor.webUrl).origin, "https://cursor.com");
  assert.equal(new URL(cursor.webUrl).pathname, "/link/prompt");
  assert.equal(new URL(cursor.webUrl).searchParams.get("text"), prompt);
  assert.equal(cursor.webPrefillsPrompt, true);
  assert.equal(custom.webUrl, null);
  assert.equal(home.bountyAssistantLinks("unknown"), null);
});

test("a prompt beyond the documented Claude Code limit takes the web handoff", () => {
  const long = "x".repeat(5001);
  const links = home.bountyAssistantLinks("claude", long);
  assert.equal(links.desktopUrl, null);
  assert.equal(new URL(links.webUrl).searchParams.get("q"), long);

  // A message that still encodes within the limit keeps the terminal handoff.
  const fits = "y".repeat(4000);
  assert.equal(encodeURIComponent(fits).length <= 5000, true);
  assert.equal(home.bountyAssistantLinks("claude", fits).desktopUrl, `claude-cli://open?q=${fits}`);

  // Percent-encoding counts: 2,000 spaces encode to 6,000 characters.
  const encodesOver = " ".repeat(2000);
  assert.equal(home.bountyAssistantLinks("claude", encodesOver).desktopUrl, null);

  // The shipped posting message, with room for a typed task, stays under the limit.
  assert.equal(encodeURIComponent(home.BOUNTY_POSTING_PROMPT).length < 5000, true);
});

test("desktop schemes are only fired where a desktop app can register one", () => {
  const linux = { userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/141 Safari/537.36" };
  const mac = { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15" };
  const windows = { userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36" };
  const chromeOS = { userAgent: "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36" };
  const android = { userAgent: "Mozilla/5.0 (Linux; Android 16; Pixel 9a) AppleWebKit/537.36" };
  const iPadDesktopMode = { platform: "MacIntel", maxTouchPoints: 5, userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)" };

  assert.equal(home.desktopPlatform(linux), "linux");
  assert.equal(home.desktopPlatform(mac), "mac");
  assert.equal(home.desktopPlatform(windows), "windows");
  assert.equal(home.desktopPlatform(chromeOS), "chromeos");
  assert.equal(home.desktopPlatform(android), "mobile");
  assert.equal(home.desktopPlatform({ userAgentData: { mobile: true } }), "mobile");
  assert.equal(home.desktopPlatform(iPadDesktopMode), "mobile");
  assert.equal(home.desktopPlatform({}), "unknown");
  assert.equal(home.desktopPlatform({ userAgentData: { platform: "Windows" } }), "windows");

  // Claude Code, Codex and Cursor all register their schemes on every desktop platform.
  for (const platform of [mac, windows, linux, chromeOS, {}]) {
    assert.equal(home.supportsDesktopHandoff(platform), true);
  }
  assert.equal(home.supportsDesktopHandoff(), true);

  // A phone or tablet can register none of them.
  assert.equal(home.supportsDesktopHandoff(android), false);
  assert.equal(home.supportsDesktopHandoff(iPadDesktopMode), false);
  assert.equal(home.supportsDesktopHandoff({ userAgentData: { mobile: true } }), false);
});

test("desktop handoff keeps arbitrary prompt text inside one prompt parameter", () => {
  const prompt = 'A & B? #launch "prototype"\nBudget: 3 USDC; deadline: mañana';
  const links = home.bountyAssistantLinks("gpt", prompt);
  const desktop = new URL(links.desktopUrl);
  assert.ok(desktop.searchParams.get("prompt").startsWith(prompt));
  assert.deepEqual([...desktop.searchParams.keys()], ["prompt", "browserUrl"]);
  assert.equal(desktop.searchParams.get("prompt"), new URL(links.webUrl).searchParams.get("prompt"));
});

test("competition posting handoffs preserve only live canonical context", () => {
  const contract = "0x1111111111111111111111111111111111111111";
  const request = home.parseCompetitionPostingRequest(`?parentCompetition=${contract}&network=base-mainnet`);
  assert.deepEqual(request, { requested: true, valid: true, contract, network: "base-mainnet" });
  assert.equal(home.parseCompetitionPostingRequest("").requested, false);
  assert.equal(home.parseCompetitionPostingRequest(`?parentCompetition=${contract}&network=base-sepolia`).valid, false);

  const item = {
    opportunity_id: `open-competition-v2:base-mainnet:${contract}`,
    source_id: contract,
    network: "base-mainnet",
    source_status: "active",
    work_state: "claimable",
    payment_state: "escrowed",
    payment_committed: true,
    verification_ready: true,
    competition_mode: "best_score",
    evidence_requirements: {
      program_profile: "forward-canonical-gmv-attribution-metric-v2",
      scoring_window: {
        starts_at: "2026-08-24T00:00:00Z",
        ends_at: "2026-08-31T00:00:00Z",
      },
    },
  };
  assert.equal(home.competitionPostingItem({ schema_version: "agent-bounties/opportunity-projection-v1", items: [item] }, request), item);
  assert.throws(() => home.competitionPostingItem({ schema_version: "agent-bounties/opportunity-projection-v1", items: [{ ...item, verification_ready: false }] }, request));

  const prompt = home.competitionPostingPrompt(item);
  assert.match(prompt, new RegExp(contract));
  assert.match(prompt, /2026-08-24T00:00:00Z through 2026-08-31T00:00:00Z/);
  assert.match(prompt, /ask me to fill only its bracketed placeholders/i);
  assert.match(prompt, /complete win, loss, and expected economics/i);
  assert.doesNotMatch(prompt, /Begin by asking/);
  assert.match(new URL(home.bountyAssistantLinks("gpt", prompt).webUrl).searchParams.get("prompt"), /utm_source=chatgpt/);
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
    source_id: "0x" + "1".repeat(40), network: "base-mainnet", source_status: "claimable",
    competition_mode: "exclusive_claim", terms_hash: "0xabc", deadline: "2026-09-01T00:00:00Z",
    reward: { amount: "1000000", decimals: 6, unit: "base_units" },
    funded_amount: { amount: "1000000", decimals: 6, unit: "base_units" },
    funding_target: { amount: "1000000", decimals: 6, unit: "base_units" },
    created_at: "2026-08-20T12:00:00Z",
    ...overrides,
  };
}

function evidence(overrides = {}) {
  return {
    schema_version: "agent-bounties/opportunity-projection-v1", network: "base-mainnet",
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
    availability: { now: 2, upcoming: 0, ended: 0, closed: 0, unavailable: 0, direct: 2, competition: 0, child_funding: 0, unknown: 0 },
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
