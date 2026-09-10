"use strict";
const assert = require("node:assert/strict");
const test = require("node:test");
const { webcrypto, randomUUID } = require("node:crypto");
const workflow = require("../site/marketplace-workflow.js");
const posting = require("../site/posting-session.js");
const JOURNAL = "agent-bounties.posting-operation.v1";
const OPERATION = "12345678-1234-4234-8234-123456789abc";
const BOUNTY = "0x" + "ab".repeat(32), CONTRACT = "0x" + "cd".repeat(20), HASH = "0x" + "ef".repeat(32);
const clone = (value) => JSON.parse(JSON.stringify(value));
const account = (id = "owner-a") => ({ authenticated: true, user: { id } });
function deferred() { let resolve; const promise = new Promise((done) => { resolve = done; }); return { promise, resolve }; }
function storage() { const values = new Map(); return { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, String(value)), removeItem: (key) => values.delete(key) }; }
class Server {
  constructor() { this.records = new Map(); this.requests = []; this.events = []; this.inventory = []; }
  record(owner = "owner-a", id = OPERATION) { return this.records.get(`${owner}:${id}`); }
  async fetch(win, raw, options = {}) {
    const url = new URL(raw), method = options.method || "GET", body = options.body ? JSON.parse(options.body) : undefined;
    this.requests.push({ owner: win.user, url, method, body, options });
    const reply = (value, status = 200) => new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
    if (url.pathname.endsWith("/events")) return reply(this.events);
    if (url.pathname.endsWith("/opportunities")) return reply({ schema_version: "agent-bounties/opportunity-projection-v1", items: this.inventory });
    const id = url.pathname.split("/").pop(), key = `${win.user}:${id}`, old = this.records.get(key);
    if (!win.user) return reply({ error: "authentication_required" }, 401);
    if (method === "GET") {
      const result = old ? clone(old) : null;
      if (this.pauseGet) { const pause = this.pauseGet; this.pauseGet = null; pause.started.resolve(); await pause.release.promise; }
      return result ? reply(result) : reply({ error: "draft_not_found" }, 404);
    }
    if (!body.recovery_state || typeof body.recovery_state !== "object" || Array.isArray(body.recovery_state)) return reply({ error: "invalid_draft_or_recovery_state" }, 400);
    const hash = await posting.digest(win, body.draft);
    if (body.approved_draft_hash && body.approved_draft_hash !== hash) return reply({ error: "invalid_draft_or_approval" }, 400);
    if (old && old.draft_hash === hash && old.approved_draft_hash === body.approved_draft_hash && posting.stable(old.recovery_state) === posting.stable(body.recovery_state)) return reply(old);
    if ((old?.revision || 0) !== body.expected_revision) return reply({ error: "revision_conflict" }, 409);
    const value = { operation_id: id, draft: body.draft, draft_hash: hash, approved_draft_hash: body.approved_draft_hash, recovery_state: body.recovery_state, revision: (old?.revision || 0) + 1, updated_at: new Date().toISOString() };
    this.records.set(key, clone(value));
    return reply(value);
  }
  pauseNextGet() { const pause = { started: deferred(), release: deferred() }; this.pauseGet = pause; return pause; }
}
function browser(server, options = {}) {
  const listeners = new Map(), timers = new Map(); let nextTimer = 0;
  const win = { location: new URL(options.url || "https://agentbounties.app/post.html"), sessionStorage: options.storage || storage(), user: options.owner || "owner-a",
    crypto: { subtle: webcrypto.subtle, randomUUID }, AgentBountiesWorkflow: workflow,
    document: { hidden: false, querySelector: () => null }, CustomEvent: class { constructor(type, data) { this.type = type; this.detail = data?.detail; } },
    addEventListener: (name, fn) => { const callbacks = listeners.get(name) || []; callbacks.push(fn); listeners.set(name, callbacks); },
    dispatchEvent: (event) => { for (const fn of listeners.get(event.type) || []) fn(event); },
    setTimeout: (fn) => { const id = ++nextTimer; timers.set(id, fn); return id; }, clearTimeout: (id) => timers.delete(id) };
  win.fetch = (url, args) => server.fetch(win, url, args);
  return win;
}
function seed(win, id = OPERATION) {
  workflow.createClient(win).save({ schema: "agent-bounties/guided-journey-v1", id, role: "post", goal: "Create a 30-second video", steps: {},
    brief: { budget_usdc: "5.00", timezone: "America/Mexico_City", deadline_at: "2026-09-10T21:00:00-06:00" },
    draft: { title: "Animate the homepage", goal: "Create a 30-second video", acceptance_criteria: ["30 seconds", "Use the frozen reference"], solver_reward_usdc: "4.50", verifier_reward_usdc: "0.50", review_mode: "creator", delivery_deadline: "2026-09-10T21:00:00-06:00", image: { sha256: "original" }, meta_child: { parent: "binding" } } });
}
function current(win) { return workflow.createClient(win).load(); }
function edit(win, mutate) { const value = clone(current(win)); mutate(value); workflow.createClient(win).save(value); }
async function ready(server = new Server()) { const win = browser(server); seed(win); const session = posting.create(win); await session.hydrate(account()); return { server, win, session }; }
async function second(server, options = {}) { const win = browser(server, { url: `https://agentbounties.app/post.html?operation_id=${OPERATION}`, ...options }); const session = posting.create(win); await session.hydrate(account(win.user)); return { win, session }; }
function pending(hash = HASH) { return { bounty_contract: CONTRACT, bounty_id: BOUNTY, phase: "sending", transactions: [hash], authorizationIssued: true }; }

test("draft hashes use JCS UTF-16 key order and preserve legitimate decimal values", async () => {
  const win = browser(new Server());
  const value = { "\ue000": 1, "\u{10000}": 2, small: 0.000001, smaller: 1e-7, size: 0.4, zero: -0, max: Number.MAX_SAFE_INTEGER };
  const canonical = '{"max":9007199254740991,"size":0.4,"small":0.000001,"smaller":1e-7,"zero":0,"𐀀":2,"":1}';
  assert.equal(posting.stable(value), canonical);
  const expected = await webcrypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
  assert.equal(await posting.digest(win, value), Buffer.from(expected).toString("hex"));
  assert.equal(await posting.digest(win, value), await posting.digest(win, JSON.parse(JSON.stringify(value))));
  for (const [input, hash] of [
    [{ amount: "0.000001", count: Number.MAX_SAFE_INTEGER, title: "Vídeo 🌞" }, "1d41456cb39a557eaf5f7ecf46d1f4e271a82739853539dde9852f0c97265fa5"],
    [{ small: 0.000001, size: 0.4, zero: -0 }, "b2b7a42de9ea755488182eab51d797d5fbe7d7acea401c9f48c082989cb5eb42"],
    [{ "\ue000": 1, "𐀀": 2 }, "9d4cdc71dda603c42f9b21d88d0c2ffc31a76cd1bd461d7359406cf169845f1e"],
  ]) assert.equal(await posting.digest(win, input), hash);
});

test("unsafe integers, invalid Unicode and non-JSON values are rejected before hashing", async () => {
  const win = browser(new Server());
  for (const [value, error] of [
    [{ amount: 9007199254740992 }, /unsafe integer/], [{ amount: -9007199254740992 }, /unsafe integer/],
    [JSON.parse('{"amount":9007199254740993}'), /unsafe integer/],
    [{ amount: Infinity }, /finite/], [{ amount: NaN }, /finite/],
    [{ title: "\ud800" }, /Unicode/], [{ "\udfff": 1 }, /Unicode/],
    [{ created: new Date() }, /JSON values/], [{ bad: [undefined] }, /JSON values/],
    [{ bytes: "x".repeat(65536) }, /64KiB/],
  ]) await assert.rejects(posting.digest(win, value), error);
  assert.match(await posting.digest(win, { amount: "9007199254740993", title: "Emoji😀" }), /^[0-9a-f]{64}$/);
});

test("fresh authenticated draft uses object recovery state and private credentialed requests", async () => {
  const { server, session } = await ready();
  assert.equal(session.snapshot().status, "saved");
  assert.deepEqual(server.record().recovery_state, { display_context: { timezone: "America/Mexico_City" } });
  assert.equal(server.record().approved_draft_hash, null);
  for (const request of server.requests) { assert.equal(request.options.credentials, "include"); assert.equal(request.options.cache, "no-store"); assert.equal(request.options.referrerPolicy, "no-referrer"); }
  assert.equal(session.snapshot().continuation_url, `https://agentbounties.app/post.html?operation_id=${OPERATION}#bounty-preview`);
  assert.doesNotMatch(session.snapshot().continuation_url, /Create|budget|email|signature/);
});

test("exact terms approval survives same-operation continuation without granting payment", async () => {
  const { server, session } = await ready(); await session.approve();
  const other = await second(server);
  assert.equal(other.session.snapshot().operation_id, OPERATION);
  assert.equal(await other.session.approved(), true);
  assert.equal(current(other.win).draft.solver_reward_usdc, "4.50");
  assert.equal(current(other.win).draft.delivery_deadline, "2026-09-10T21:00:00-06:00");
  assert.deepEqual(await other.session.reconcile(), { creation_confirmed: false, funding_confirmed: false, claimable: false, public_inventory_verified: false });
});

test("every material draft change invalidates approval; wallet journal has a separate hash", async () => {
  const { win, session, server } = await ready(); await session.approve();
  const original = clone(current(win)), approvedHash = server.record().approved_draft_hash;
  for (const mutate of [
    (value) => { value.draft.goal = "A different result"; },
    (value) => { value.draft.solver_reward_usdc = "9.50"; },
    (value) => { value.draft.delivery_deadline = "2026-09-11T21:00:00-06:00"; },
    (value) => { value.draft.image.sha256 = "replacement"; },
    (value) => { value.draft.meta_child.parent = "different"; },
    (value) => { value.reference_attachment = { sha256: "different" }; },
    (value) => { value.draft_stale = true; },
  ]) { workflow.createClient(win).save(clone(original)); edit(win, mutate); assert.equal(await session.approved(), false); }
  workflow.createClient(win).save(original);
  win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  await session.flush();
  assert.equal(await session.approved(), true);
  assert.equal(server.record().approved_draft_hash, approvedHash);
  assert.deepEqual(server.record().recovery_state, { ...pending(), display_context: { timezone: "America/Mexico_City" } });
  assert.equal(server.record().draft.recovery_state, undefined);
});

test("formatting-only brief edits preserve approval and the chosen timezone across devices", async () => {
  const one = await ready(); await one.session.approve();
  const originalHash = one.server.record().approved_draft_hash;
  edit(one.win, (value) => { value.brief.budget_usdc = "005.000000"; value.brief.deadline_local = "2026-09-10T21:00"; value.brief.deadline_at = "2026-09-11T03:00:00Z"; value.brief.goal = value.goal; });
  assert.equal(await one.session.approved(), true);
  await one.session.flush();
  assert.equal(one.server.record().approved_draft_hash, originalHash);
  const two = await second(one.server);
  assert.equal(await two.session.approved(), true);
  assert.equal(current(two.win).brief.budget_usdc, "5");
  assert.equal(current(two.win).brief.timezone, "America/Mexico_City");
  assert.equal(current(two.win).brief.deadline_at, "2026-09-11T03:00:00.000Z");
  assert.equal(two.win.sessionStorage.getItem(JOURNAL), null);
  edit(two.win, (value) => { value.brief.timezone = "UTC"; });
  assert.equal(await two.session.approved(), true);
  await two.session.flush();
  const three = await second(one.server);
  assert.equal(current(three.win).brief.timezone, "UTC");
  assert.equal(await three.session.approved(), true);
});

test("concurrent edits conflict without overwriting either draft", async () => {
  const one = await ready(); const two = await second(one.server);
  edit(one.win, (value) => { value.draft.goal = "First device edit"; }); await one.session.flush();
  edit(two.win, (value) => { value.draft.goal = "Second device edit"; });
  await assert.rejects(two.session.flush(), /revision_conflict/);
  assert.equal(two.session.snapshot().conflict, true);
  assert.equal(current(two.win).draft.goal, "Second device edit");
  assert.equal(one.server.record().draft.draft.goal, "First device edit");
  await assert.rejects(two.session.approve(), /saved draft/);
});

test("typing during refresh cannot be overwritten by a delayed server reply", async () => {
  const one = await ready(); const two = await second(one.server);
  edit(one.win, (value) => { value.draft.goal = "Remote revision"; }); await one.session.flush();
  const pause = one.server.pauseNextGet(), refreshing = two.session.refresh(); await pause.started.promise;
  edit(two.win, (value) => { value.draft.goal = "Typed while loading"; }); pause.release.resolve();
  await assert.rejects(refreshing, /local changes were preserved/);
  assert.equal(current(two.win).draft.goal, "Typed while loading");
  assert.equal(two.session.snapshot().conflict, true);
});

test("same-owner browser reload uploads unsynced local edits against unchanged server base", async () => {
  const one = await ready(); edit(one.win, (value) => { value.draft.goal = "Offline edit"; });
  const reload = browser(one.server, { storage: one.win.sessionStorage });
  const session = posting.create(reload); await session.hydrate(account());
  assert.equal(current(reload).draft.goal, "Offline edit");
  assert.equal(one.server.record().draft.draft.goal, "Offline edit");
  assert.equal(session.snapshot().status, "saved");
});

test("restoring account state cannot erase a local uncertain wallet transaction", async () => {
  const one = await ready(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  await one.session.reload();
  assert.deepEqual(JSON.parse(one.win.sessionStorage.getItem(JOURNAL)), pending());
  await one.session.flush({ requireServer: true });
  const two = await second(one.server);
  assert.deepEqual(JSON.parse(two.win.sessionStorage.getItem(JOURNAL)), pending());
  two.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending("0x" + "11".repeat(32))));
  await assert.rejects(two.session.reload(), /different wallet progress/);
  assert.equal(JSON.parse(two.win.sessionStorage.getItem(JOURNAL)).transactions[0], "0x" + "11".repeat(32));
});

test("batch and canonical creation checkpoints remain protected even without a transaction list", async () => {
  const one = await ready();
  for (const phase of ["batch_submitted", "creation_confirmed"]) {
    const local = { bounty_contract: CONTRACT, bounty_id: BOUNTY, phase, transactions: [] };
    one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(local));
    await one.session.reload();
    assert.deepEqual(JSON.parse(one.win.sessionStorage.getItem(JOURNAL)), local);
    one.server.record().recovery_state = { ...local, phase: "sending" };
    await assert.rejects(one.session.reload(), /different wallet progress/);
    one.server.record().recovery_state = {};
  }
});

test("account switches and reloads never upload the previous owner's private draft or journal", async () => {
  const one = await ready(); await one.session.approve(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  one.win.user = "owner-b"; await one.session.hydrate(account("owner-b"));
  assert.equal(current(one.win).draft, undefined);
  assert.equal(one.win.sessionStorage.getItem(JOURNAL), null);
  assert.equal(await one.session.approved(), false);
  assert.equal(one.server.record("owner-b"), undefined);
  for (const request of one.server.requests.filter((item) => item.owner === "owner-b" && item.method === "POST")) assert.equal(request.body.draft.draft, null);
  one.win.user = "owner-a"; await one.session.hydrate(account());
  assert.equal(current(one.win).draft.title, "Animate the homepage");
  assert.deepEqual(JSON.parse(one.win.sessionStorage.getItem(JOURNAL)), pending());
  assert.equal(await one.session.approved(), true);
  const reload = browser(one.server, { storage: one.win.sessionStorage, owner: "owner-b" });
  await posting.create(reload).hydrate(account("owner-b"));
  assert.equal(current(reload).draft ?? null, null);
  assert.equal(reload.sessionStorage.getItem(JOURNAL), null);
});

test("logout preserves the original account's draft for a later same-account login", async () => {
  const one = await ready(); one.win.user = null; await one.session.hydrate({ authenticated: false });
  assert.equal(current(one.win).draft, undefined);
  one.win.user = "owner-a"; await one.session.hydrate(account());
  assert.equal(current(one.win).draft.title, "Animate the homepage");
});

test("delayed previous-account replies are ignored after account switch", async () => {
  const one = await ready(), two = await second(one.server);
  edit(one.win, (value) => { value.draft.goal = "Private A revision"; }); await one.session.flush();
  const pause = one.server.pauseNextGet(), refreshing = two.session.refresh(); await pause.started.promise;
  two.win.user = "owner-b"; await two.session.hydrate(account("owner-b")); pause.release.resolve(); await refreshing;
  assert.equal(current(two.win).draft, undefined);
  assert.equal(await two.session.approved(), false);
});

test("a queued save cannot transmit previous-owner content after account switch", async () => {
  const one = await ready(), started = deferred(), release = deferred();
  const original = one.win.crypto.subtle;
  let pause = true;
  one.win.crypto.subtle = { digest: async (...args) => { if (pause) { pause = false; started.resolve(); await release.promise; } return original.digest(...args); } };
  const saving = one.session.flush({ requireServer: true });
  await started.promise;
  one.win.user = "owner-b"; const switching = one.session.hydrate(account("owner-b"));
  release.resolve(); await assert.rejects(saving, /active account changed/); await switching;
  assert.equal(one.server.record("owner-b"), undefined);
  for (const request of one.server.requests.filter((item) => item.owner === "owner-b" && item.method === "POST")) assert.equal(request.body.draft.draft, null);
});

test("a save cannot pair older terms with newer wallet progress while hashing", async () => {
  const one = await ready(), started = deferred(), release = deferred();
  const original = one.win.crypto.subtle, revision = one.server.record().revision;
  let pause = true;
  one.win.crypto.subtle = { digest: async (...args) => { if (pause) { pause = false; started.resolve(); await release.promise; } return original.digest(...args); } };
  const saving = one.session.flush({ requireServer: true }); await started.promise;
  edit(one.win, (value) => { value.draft.goal = "New reviewed scope"; });
  one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  release.resolve(); await assert.rejects(saving, /changed while preparing/);
  assert.equal(one.server.record().revision, revision);
  assert.notEqual(one.server.record().draft.draft.goal, "New reviewed scope");
  assert.equal(one.server.record().recovery_state.phase, undefined);
  await one.session.flush({ requireServer: true });
  assert.equal(one.server.record().draft.draft.goal, "New reviewed scope");
  assert.equal(one.server.record().recovery_state.phase, "sending");
});

test("an explicit new MCP operation may save matching imported terms without auto-approval", async () => {
  const server = new Server(), win = browser(server, { url: `https://agentbounties.app/post.html?operation_id=${OPERATION}` }); seed(win);
  const session = posting.create(win); await session.hydrate(account());
  assert.equal(server.record().operation_id, OPERATION);
  assert.equal(await session.approved(), false);
  const absent = await second(new Server());
  assert.equal(absent.session.snapshot().status, "unavailable");
});

function canonicalEvent(kind, extra = {}) {
  return { id: randomUUID(), kind, bounty_id: BOUNTY, tx_hash: HASH, block_number: 100, contract_address: CONTRACT, data: {}, ...extra };
}
function readyItem() {
  const money = (amount) => ({ amount, unit: "base_units", decimals: 6 });
  return { opportunity_id: `autonomous:${BOUNTY}`, network: "base-mainnet", source_type: "canonical_base", source_id: CONTRACT,
    work_state: "claimable", payment_state: "escrowed", payment_committed: true, verification_ready: true, source_status: "claimable", terms_hash: HASH,
    reward: money("4500000"), funded_amount: money("5000000"), funding_target: money("5000000") };
}
function canonicalEvents() {
  return [canonicalEvent("canonical_bounty_created", { contract_address: "0x" + "12".repeat(20), data: { bounty_contract: CONTRACT, terms_hash: HASH } }),
    canonicalEvent("funding_added", { data: { amount: "5000000", funded_amount: "5000000", target_amount: "5000000" } }),
    canonicalEvent("bounty_became_claimable", { data: { funded_amount: "5000000" } })];
}
test("partial, mismatched, and missing inventory evidence cannot archive a wallet operation", async () => {
  const one = await ready(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  for (const [events, inventory] of [
    [canonicalEvents().map((event) => ({ ...event, bounty_id: "0x" + "99".repeat(32) })), [readyItem()]],
    [canonicalEvents(), []],
    [canonicalEvents().filter((event) => event.kind !== "funding_added"), [readyItem()]],
    [canonicalEvents().filter((event) => event.kind !== "bounty_became_claimable"), [readyItem()]],
    [canonicalEvents().map((event) => event.kind === "funding_added" ? { ...event, data: { funded_amount: "1", target_amount: "5000000" } } : event), [readyItem()]],
    [canonicalEvents(), [{ ...readyItem(), terms_hash: "0x" + "33".repeat(32) }]],
    [canonicalEvents().map((event) => event.kind === "canonical_bounty_created" ? { ...event, data: { ...event.data, terms_hash: "garbage" } } : event), [{ ...readyItem(), terms_hash: "garbage" }]],
  ]) {
    one.server.events = events; one.server.inventory = inventory;
    const result = await one.session.reconcile();
    assert.equal(result.public_inventory_verified, false);
    assert.equal(result.paid, false);
    assert.equal(JSON.parse(one.win.sessionStorage.getItem(JOURNAL)).phase, "sending");
  }
});

test("matching creation, full funding and ready inventory confirm exactly the recorded bounty", async () => {
  const one = await ready(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  one.server.events = canonicalEvents(); one.server.inventory = [readyItem()];
  const result = await one.session.reconcile();
  assert.equal(result.creation_confirmed, true); assert.equal(result.funding_confirmed, true);
  assert.equal(result.claimable, true); assert.equal(result.public_inventory_verified, true); assert.equal(result.paid, false);
  assert.match(result.public_url, new RegExp(CONTRACT));
  assert.equal(JSON.parse(one.win.sessionStorage.getItem(JOURNAL)).phase, "funding_confirmed");
});

test("a saved rejected attempt cannot fork without atomic shared-operation retirement", async () => {
  const one = await ready(); await one.session.approve();
  const error = { code: -32602, message: "Invalid params 0 > atomicRequired - Expected a value of type `boolean`, but received: `undefined`" };
  const original = { bounty_contract: CONTRACT, bounty_id: BOUNTY, phase: "sending", transactions: [], error_capture_version: 1, wallet_method: "wallet_sendCalls", wallet_error: error };
  one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(original)); await one.session.flush();
  const journal = workflow.createPostingJournal(one.win), archived = journal.archiveRejectedBatch(original, error, original);
  await assert.rejects(one.session.beginAfterArchive(archived, { preserveDraft: true }), /requires reconciliation/);
  assert.equal(one.session.snapshot().operation_id, OPERATION);
  assert.equal(current(one.win).draft.title, "Animate the homepage");
  assert.equal(one.server.record().recovery_state.phase, "sending");
  assert.equal(one.server.records.size, 1);
});

test("arbitrary or uncertain archives never create a new posting operation", async () => {
  const one = await ready(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  await assert.rejects(one.session.beginAfterArchive({ ...pending(), phase: "funding_confirmed" }), /still recorded/);
  one.win.sessionStorage.removeItem(JOURNAL);
  await assert.rejects(one.session.beginAfterArchive({ ...pending(), phase: "funding_confirmed" }), /exact archived/);
  const archived = { ...pending(), phase: "batch_wallet_rejected" };
  one.win.sessionStorage.setItem(`${JOURNAL}.rejected`, JSON.stringify([archived]));
  await assert.rejects(one.session.beginAfterArchive(archived, { preserveDraft: true }), /requires reconciliation|authorized or broadcast/);
  assert.equal(one.session.snapshot().operation_id, OPERATION);
  assert.equal(one.server.records.size, 1);
});

test("explicit new-task after canonical completion saves a fresh operation without old approval", async () => {
  const one = await ready(); await one.session.approve(); one.win.sessionStorage.setItem(JOURNAL, JSON.stringify(pending()));
  one.server.events = canonicalEvents(); one.server.inventory = [readyItem()];
  await one.session.reconcile(); await one.session.flush();
  const next = workflow.createClient(one.win).start({ role: "post", new_task: true, goal: "A new task" });
  await one.session.flush();
  assert.notEqual(next.id, OPERATION);
  assert.equal(one.session.snapshot().operation_id, next.id);
  assert.equal(one.server.record().recovery_state.phase, "funding_confirmed");
  assert.equal(one.server.record("owner-a", next.id).draft.goal, "A new task");
  assert.equal(one.server.record("owner-a", next.id).approved_draft_hash, null);
  assert.equal(await one.session.approved(), false);
});
