"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { webcrypto, createHash } = require("node:crypto");
const canary = require("../site/posting-draft-canary.js");

function harness(options = {}) {
  const values = new Map([["agent-bounties.guided-journey.v1", "existing private journey"], ["agent-bounties.posting-approval.v1", "existing approval"]]);
  const calls = [], writes = [], events = [], rows = new Map();
  let account = "a".repeat(64), lost = false;
  const storage = { getItem: (key) => values.get(key) || null, setItem: (key, value) => { writes.push(key); values.set(key, value); } };
  const response = (status, data) => ({ status, json: async () => data, headers: { get: (name) => name === "cache-control" ? "no-store" : null } });
  const win = { crypto: webcrypto, sessionStorage: storage, fetch: async (url, request) => {
    calls.push({ url, ...request });
    assert.equal(new URL(url).origin, canary.API);
    assert.equal(request.cache, "no-store");
    if (url.endsWith("/v1/site-auth/session")) return response(200, { authenticated: true, user: { id: account }, posting_drafts_enabled: options.enabled !== false });
    const id = url.match(/\/v1\/site-auth\/posting-drafts\/([0-9a-f-]+)$/)?.[1];
    assert.ok(id, "only session and private draft routes are permitted");
    if (request.credentials === "omit") return response(options.anonymousStatus || 401, { error: "authentication_required" });
    if (request.method === "POST") {
      const body = JSON.parse(request.body);
      assert.equal(body.approved_draft_hash, null);
      assert.deepEqual(body.recovery_state, {});
      assert.equal(body.draft.draft, null);
      assert.equal(body.draft.brief, null);
      assert.equal(body.draft.id, id);
      const existing = rows.get(id), hash = createHash("sha256").update(canary.stable(body.draft)).digest("hex");
      if (existing && existing.draft_hash !== hash) return response(options.staleStatus || 409, { error: "revision_conflict" });
      const row = existing || { operation_id: id, revision: 1, draft: body.draft, draft_hash: options.badHash ? "b".repeat(64) : hash, approved_draft_hash: null, recovery_state: {} };
      rows.set(id, row);
      if (options.loseCreate && !lost) { lost = true; throw new Error("Lost response"); }
      return response(200, row);
    }
    return rows.has(id) ? response(200, rows.get(id)) : response(404, { error: "draft_not_found" });
  } };
  return { win, values, writes, calls, events, rows, client: canary.create(win, (event) => events.push(event)), setAccount(value) { account = value; } };
}

test("canary verifies create, exact replay, stale conflict, and anonymous denial without touching a journey", async () => {
  const h = harness();
  const record = await h.client.run();
  assert.equal(h.rows.size, 1);
  assert.equal(h.calls.filter((call) => call.method === "POST").length, 3);
  assert.equal(h.calls.at(-1).credentials, "omit");
  assert.equal(h.events.filter((event) => event.kind === "pass").length, 5);
  assert.deepEqual(h.writes, [canary.KEY]);
  assert.equal(h.values.get("agent-bounties.guided-journey.v1"), "existing private journey");
  assert.equal(h.values.get("agent-bounties.posting-approval.v1"), "existing approval");
  assert.equal(h.rows.get(record.operation_id).revision, 1);
  assert.equal(h.rows.get(record.operation_id).draft.goal, canary.envelope(record.operation_id).goal);
});

test("reloaded page verifies only its same-account canary without another mutation", async () => {
  const h = harness();
  const record = await h.client.run(), before = h.calls.length;
  const reloaded = canary.create(h.win);
  const data = await reloaded.resume();
  assert.equal(data.operation_id, record.operation_id);
  assert.deepEqual(h.calls.slice(before).map((call) => call.method), ["GET", "GET"]);
  assert.deepEqual(h.writes, [canary.KEY]);
});

test("disabled rollout does not create or store a canary", async () => {
  const h = harness({ enabled: false });
  await assert.rejects(h.client.run(), /not enabled/);
  assert.equal(h.calls.length, 1);
  assert.equal(h.rows.size, 0);
  assert.deepEqual(h.writes, []);
});

test("account switch fails before reading the previous account's private record", async () => {
  const h = harness();
  await h.client.run();
  const previous = h.values.get(canary.KEY), before = h.calls.length;
  h.setAccount("b".repeat(64));
  await assert.rejects(canary.create(h.win).resume(), /different account/);
  assert.equal(h.calls.length, before + 1);
  assert.equal(h.values.get(canary.KEY), previous);
});

test("unexpected stale success, unauthenticated success, or hash mismatch fails the canary", async () => {
  await assert.rejects(harness({ staleStatus: 200 }).client.run(), /expected HTTP 409, received 200/);
  await assert.rejects(harness({ anonymousStatus: 200 }).client.run(), /expected HTTP 401, received 200/);
  await assert.rejects(harness({ badHash: true }).client.run(), /did not match/);
});

test("a lost create response preserves its ID and reload performs a read only", async () => {
  const h = harness({ loseCreate: true });
  await assert.rejects(h.client.run(), /Lost response/);
  const before = h.calls.length;
  assert.equal(h.rows.size, 1);
  const resumed = await canary.create(h.win).resume();
  assert.equal(resumed.operation_id, JSON.parse(h.values.get(canary.KEY)).operation_id);
  assert.deepEqual(h.calls.slice(before).map((call) => call.method), ["GET", "GET"]);
  assert.equal(h.rows.size, 1);
});
