"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const postingAuth = require("../site/posting-auth.js");
const composer = require("../site/bounty-composer-v2.js");

function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.has(key) ? values.get(key) : null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

function browser(href = "https://agentbounties.app/post.html") {
  const navigations = [];
  const current = new URL(href);
  return {
    sessionStorage: storage(),
    location: {
      href: current.href,
      origin: current.origin,
      pathname: current.pathname,
      hostname: current.hostname,
      assign: (value) => navigations.push({ method: "assign", value }),
      replace: (value) => navigations.push({ method: "replace", value }),
    },
    navigations,
  };
}

test("only the exact same-origin post target is accepted", () => {
  const win = browser();
  const target = "https://agentbounties.app/post.html?title=Glama&criterion=one&criterion=two&acquisition=opaque#review";
  assert.equal(postingAuth.safePostTarget(win, target), target);
  assert.equal(postingAuth.safePostTarget(win, "/post.html?from=webmcp"), "https://agentbounties.app/post.html?from=webmcp");
  for (const unsafe of [
    "https://evil.example/post.html",
    "https://agentbounties.app.evil.example/post.html",
    "https://user@agentbounties.app/post.html",
    "https://agentbounties.app/earn.html",
    "javascript:alert(1)",
  ]) assert.equal(postingAuth.safePostTarget(win, unsafe), null);
});

test("login begins without flattening or leaking the prepared URL", () => {
  const target = "https://agentbounties.app/post.html?title=Audit+Glama&criterion=a%2Fb&benchmark=%7B%22engine%22%3A%22sandboxed_regression_v1%22%7D&acquisition=aba1_opaque&handoff=uuid";
  const win = browser(target);
  assert.equal(postingAuth.begin(win, target, 1000), target);
  assert.deepEqual(win.navigations, [{ method: "assign", value: "https://agentbounties.app/?postReturn=1#login" }]);
  assert.equal(postingAuth.pending(win), target);
  assert.doesNotMatch(win.navigations[0].value, /title|criterion|benchmark|acquisition|handoff/);
});

test("account completion returns once and leaves a bounded public receipt", () => {
  const now = Date.now();
  const target = "https://agentbounties.app/post.html?from=ai-app&acquisition=opaque";
  const win = browser(target);
  postingAuth.remember(win, target, now);
  assert.equal(postingAuth.complete(win, { walletKind: "phone", address: "0x" + "AB".repeat(20) }, now + 1000), true);
  assert.deepEqual(win.navigations, [{ method: "replace", value: target }]);
  assert.equal(postingAuth.pending(win, now + 1000), null);
  assert.deepEqual(postingAuth.consumeReceipt(win, now + 1000), {
    schema: "agent-bounties/post-auth-receipt-v1",
    completed_at: now + 1000,
    wallet_kind: "phone",
    wallet_address: "0x" + "ab".repeat(20),
  });
  assert.equal(postingAuth.consumeReceipt(win, now + 1000), null);
  assert.equal(postingAuth.complete(win, {}, now + 1000), false);
});

test("expired, malformed, and storage-disabled continuation state fails closed", () => {
  const now = Date.now();
  const win = browser();
  postingAuth.remember(win, win.location.href, now - (5 * 60 * 60 * 1000));
  assert.equal(postingAuth.pending(win, now), null);
  win.sessionStorage.setItem(postingAuth.INTENT_KEY, "not-json");
  assert.equal(postingAuth.pending(win, now), null);
  const blocked = browser();
  blocked.sessionStorage.setItem = () => { throw new Error("disabled"); };
  assert.throws(() => postingAuth.begin(blocked), /could not preserve/i);
  assert.deepEqual(blocked.navigations, []);
});

test("posting action is driven by server account state", () => {
  assert.equal(composer.postingAccountStatus({ authenticated: false, account_status: "signed_out" }), "signed_out");
  assert.equal(composer.postingAccountStatus({ authenticated: true, account_status: "wallet_required", account_complete: false }), "setup");
  assert.equal(composer.postingAccountStatus({ authenticated: true, account_status: "ready", account_complete: true }), "ready");
  assert.equal(composer.postingAccountStatus({ authenticated: true, account_status: "ready", account_complete: false }), "unavailable");
  assert.deepEqual(composer.postingPrimaryAction("signed_out", true), { action: "login", disabled: false, label: "LOG IN TO POST" });
  assert.deepEqual(composer.postingPrimaryAction("setup", true), { action: "login", disabled: false, label: "FINISH SETUP TO POST" });
  assert.deepEqual(composer.postingPrimaryAction("checking", true), { action: "wait", disabled: true, label: "Checking login…" });
  assert.deepEqual(composer.postingPrimaryAction("unavailable", true), { action: "login", disabled: false, label: "CHECK ACCOUNT TO POST" });
  assert.deepEqual(composer.postingPrimaryAction("ready", true), { action: "approve", disabled: false, label: "Confirm bounty" });
});

for (const path of ["/post", "/post.html"]) test(`account sign-in preserves ${path} continuation`, () => {
  const target = `https://agentbounties.app${path}?operation_id=10000000-0000-4000-8000-000000000001#bounty-preview`;
  const win = browser(target);
  assert.equal(postingAuth.begin(win), target);
  assert.equal(postingAuth.pending(win), target);
  assert.equal(postingAuth.complete(win, {}), true);
  assert.equal(win.navigations.at(-1).value, target);
  for (const unsafe of ["/post/extra", "/post.html/extra", "/other/post", "https://evil.example/post"]) {
    assert.equal(postingAuth.safePostTarget(win, unsafe), null);
  }
});

// Exercise the actual browser controller, including early-return rendering and
// requests that never produce headers or finish their JSON body.
const vm = require('node:vm');
const fs = require('node:fs');
const controllerSource = fs.readFileSync(require.resolve('../site/bounty-composer-v2.js'), 'utf8');
function postingController(fetchImpl) {
  const element = () => ({ disabled: false, textContent: '', dataset: {} });
  const ui = Object.fromEntries(['fundNow', 'fund', 'approve', 'confidence', 'recovery'].map(key => [key, element()]));
  const state = { draft: {}, imageReady: true, reviewStale: false, postingAccountStatus: 'checking', approved: false };
  const timers = new Map(); let timerId = 0;
  const context = { ui, state, API: 'https://api.agentbounties.app', AbortController,
    fetch: fetchImpl, postingAccountStatus: composer.postingAccountStatus, postingPrimaryAction: composer.postingPrimaryAction,
    expiredDeliveryDeadline: () => false, supportedVerificationPolicy: () => {},
    postingJournal: { load: () => null }, postingSession: { canContinue: () => false, invalidate: () => {}, hydrate: async () => {}, approved: async () => false, snapshot: () => ({ status: "saved", conflict: false }) },
    restoration: Promise.resolve(), restorationError: null,
    setStatus(message) { context.message = message; }, updatePostingTracker: () => {},
    window: { location: { hostname: 'agentbounties.app' },
      setTimeout(fn) { const id = ++timerId; timers.set(id, fn); return id; },
      clearTimeout(id) { timers.delete(id); } },
  };
  const start = controllerSource.indexOf('  function postingAuthMessage()');
  const end = controllerSource.indexOf('  function beginPostingLogin()', start);
  const controller = vm.runInNewContext(controllerSource.slice(start, end) + '\n({syncPrimaryAction, loadPostingAccount})', context);
  return { ...controller, ui, state, timers, context };
}
const readySession = { authenticated: true, account_status: 'ready', account_complete: true };
const jsonResponse = value => ({ ok: true, json: async () => value });
const drain = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

test('restored stale proposal replaces Checking login with its real blocker', async () => {
  const c = postingController(async url => jsonResponse(url.endsWith('/session') ? readySession : { wallets: [] }));
  c.syncPrimaryAction();
  assert.equal(c.ui.approve.textContent, 'Checking login…');
  c.state.reviewStale = true;
  await c.loadPostingAccount();
  assert.equal(c.state.postingAccountStatus, 'ready');
  assert.equal(c.ui.approve.textContent, 'Update proposal first');
  assert.equal(c.ui.approve.dataset.nextAction, 'blocked');
  assert.equal(c.ui.approve.disabled, true);
  assert.match(c.context.message, /brief changed/i);
  c.state.reviewStale = false;
  c.syncPrimaryAction();
  assert.equal(c.ui.approve.textContent, 'Approve bounty card');
  assert.equal(c.ui.approve.disabled, false);
  assert.equal(c.state.approved, false);
});

test('brief/reference invalidation immediately updates the visible action', () => {
  const c = postingController();
  const start = controllerSource.indexOf('    invalidate() {', controllerSource.indexOf('window.AgentBountiesComposer ='));
  const end = controllerSource.indexOf('    stage(value)', start);
  const invalidate = vm.runInNewContext('({' + controllerSource.slice(start, end) + '}).invalidate', {
    ...c.context, postingBusy: false, stagedFingerprint: 'old', syncPrimaryAction: c.syncPrimaryAction,
  });
  c.ui.approve.textContent = 'Checking login…';
  c.state.approved = true;
  invalidate();
  assert.equal(c.ui.approve.textContent, 'Update proposal first');
  assert.equal(c.ui.approve.dataset.approved, 'false');
  assert.equal(c.ui.fund.disabled, true);
  assert.equal(c.ui.fundNow.disabled, true);
});

for (const phase of ['headers', 'body']) test(`hung session ${phase} times out and a later retry recovers without approval`, async () => {
  let recover = false;
  const c = postingController(async url => {
    if (recover) return jsonResponse(url.endsWith('/session') ? readySession : { wallets: [] });
    if (phase === 'headers') return new Promise(() => {});
    return { ok: true, json: () => new Promise(() => {}) };
  });
  c.state.accountSession = readySession;
  c.state.linkedWallets = [{ address: 'old' }];
  const pending = c.loadPostingAccount(); await drain();
  for (const timer of [...c.timers.values()]) timer();
  await pending;
  assert.equal(c.state.postingAccountStatus, 'unavailable');
  assert.equal(c.state.accountSession, null);
  assert.equal(c.state.linkedWallets.length, 0);
  assert.equal(c.ui.approve.textContent, 'CHECK ACCOUNT TO POST');
  assert.equal(c.ui.approve.disabled, false);
  assert.equal(c.ui.fund.disabled, true);
  assert.equal(c.timers.size, 0);
  recover = true; await c.loadPostingAccount();
  assert.equal(c.ui.approve.textContent, 'Approve bounty card');
  assert.equal(c.state.approved, false);
});

test('optional wallet discovery cannot hide a completed login or hang initialization', async () => {
  const c = postingController(async url => url.endsWith('/session') ? jsonResponse(readySession) : new Promise(() => {}));
  const pending = c.loadPostingAccount(); await drain();
  assert.equal(c.ui.approve.textContent, 'Approve bounty card');
  assert.equal(c.ui.approve.disabled, false);
  for (const timer of [...c.timers.values()]) timer();
  await pending;
  assert.equal(c.state.postingAccountStatus, 'ready');
  assert.equal(c.state.approved, false);
  assert.equal(c.timers.size, 0);
});

test('missing and loading proposals do not retain login labels', () => {
  const c = postingController();
  c.state.draft = null; c.syncPrimaryAction();
  assert.equal(c.ui.approve.textContent, 'Prepare a proposal first');
  c.state.draft = {}; c.state.imageReady = false; c.syncPrimaryAction();
  assert.equal(c.ui.approve.textContent, 'Loading bounty image…');
  c.state.imageError = 'failed'; c.syncPrimaryAction();
  assert.equal(c.ui.approve.textContent, 'Fix bounty image');
});


test('a ready account cannot approve while its saved draft is still hydrating', async () => {
  const c = postingController(async url => jsonResponse(url.endsWith('/session') ? readySession : { wallets: [] }));
  let finish;
  c.context.postingSession.hydrate = () => new Promise(resolve => { finish = resolve; });
  const pending = c.loadPostingAccount(); await drain();
  assert.equal(c.state.postingAccountStatus, 'restoring');
  assert.equal(c.ui.approve.textContent, 'Restoring saved draft…');
  assert.equal(c.ui.approve.disabled, true);
  assert.equal(c.ui.fundNow.disabled, true);
  finish(); await pending;
  assert.equal(c.ui.approve.textContent, 'Approve bounty card');
  assert.equal(c.ui.approve.disabled, false);
  assert.equal(c.state.approved, false);
});

test('failed account draft hydration never exposes approval', async () => {
  const c = postingController(async () => jsonResponse(readySession));
  c.context.postingSession.hydrate = async () => { throw new Error('draft unavailable'); };
  await c.loadPostingAccount();
  assert.equal(c.ui.approve.textContent, 'CHECK ACCOUNT TO POST');
  assert.equal(c.ui.approve.dataset.nextAction, 'login');
  assert.equal(c.ui.fundNow.disabled, true);
  assert.equal(c.state.accountSession, null);
});


test('nonthrowing hydration failure remains blocked', async () => {
  const c = postingController(async () => jsonResponse(readySession));
  c.context.postingSession.snapshot = () => ({ status: 'unavailable', conflict: false });
  await c.loadPostingAccount();
  assert.equal(c.state.postingAccountStatus, 'unavailable');
  assert.equal(c.ui.approve.dataset.nextAction, 'login');
  assert.equal(c.ui.fundNow.disabled, true);
});
