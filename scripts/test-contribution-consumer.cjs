"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawnSync } = require("node:child_process");
const { URL, LIMIT, consume, readLive, readFile, main } = require("../examples/contribution-evidence/read.cjs");
const { fixture } = require("./fixtures/contribution-recognition.cjs");
const NOW = Date.parse("2026-10-02T12:00:00Z");
function response(data) { return new Response(JSON.stringify(data), { headers: { "content-type": "application/json; charset=utf-8" } }); }
test("consumer preserves scoped identity, consent, revision and limits without inferring payment", () => {
  const input = fixture(), r = consume(input, { now: NOW, source: "offline fixture" });
  assert.equal(r.state, "available"); assert.deepEqual(r.records, input.records);
  assert.equal(r.identity_assertions_verified_by_client, false); assert.equal(r.payment_state, "not_assessed");
  assert.equal(r.registry_updated_at, input.updated_at); assert.equal(r.source, "offline fixture");
  assert.match(r.limitations, /not a census or lifetime work history/);
});
test("profile match is case-insensitive and a no-match is distinct from an empty registry", () => {
  assert.equal(consume(fixture(), { profileURL: "https://github.com/EXAMPLE", now: NOW }).matched_count, 1);
  const none = consume(fixture(), { profileURL: "https://github.com/another", now: NOW });
  assert.equal(none.state, "no_match"); assert.equal(none.registry_record_count, 1);
  const input = fixture(); input.records = [];
  assert.equal(consume(input, { now: NOW }).state, "empty_registry");
});
test("publisher's owner-confirmed assertion is preserved without client verification", () => {
  const input = fixture(); input.records[0].identity = { state: "owner_confirmed", evidence_url: "https://github.com/NSPG13/agent-bounties/issues/1#issuecomment-3" };
  const r = consume(input, { now: NOW });
  assert.equal(r.records[0].identity.state, "owner_confirmed"); assert.equal(r.identity_assertions_verified_by_client, false);
});
test("one explicit public read uses fixed endpoint, no credentials, no redirects and bounded timeout", async () => {
  const calls = [];
  const r = await readLive({ now: NOW, fetchImpl: async (...args) => { calls.push(args); return response(fixture()); } });
  assert.equal(r.state, "available"); assert.equal(calls.length, 1); assert.equal(calls[0][0], URL);
  const options = calls[0][1]; assert.equal(options.method, "GET"); assert.equal(options.credentials, "omit");
  assert.equal(options.redirect, "error"); assert.equal(options.signal.aborted, false);
  assert.deepEqual(options.headers, { accept: "application/json" });
});
test("invalid profile is rejected before network access", async () => {
  let calls = 0;
  for (const profileURL of ["https://github.com/example?secret=x", "https://github.com.evil.test/example", "javascript:alert(1)", ["https://github.com/example"]]) {
    await assert.rejects(readLive({ profileURL, fetchImpl: async () => { calls++; return response(fixture()); } }));
  }
  assert.equal(calls, 0);
});
test("network, abort, HTTP, media, parsing and schema failures reject instead of returning empty", async () => {
  for (const fetchImpl of [
    async () => { throw Error("offline"); }, async () => { throw new DOMException("timeout", "TimeoutError"); },
    async () => new Response("not found", { status: 404 }),
    async () => new Response("<html>bad gateway</html>", { headers: { "content-type": "text/html" } }),
    async () => new Response("{", { headers: { "content-type": "application/json" } }),
    async () => response({}), async () => { const x = fixture(); x.records[0].payment_state = "paid"; return response(x); },
    async () => new Response(Uint8Array.from([0xff]), { headers: { "content-type": "application/json" } })
  ]) await assert.rejects(readLive({ now: NOW, fetchImpl }));
});
test("oversized stream is cancelled before parsing", async () => {
  let cancelled = false;
  const body = new ReadableStream({ start(controller) { controller.enqueue(new Uint8Array(LIMIT + 1)); }, cancel() { cancelled = true; } });
  await assert.rejects(readLive({ fetchImpl: async () => new Response(body, { headers: { "content-type": "application/json" } }) }), /size limit/);
  assert.equal(cancelled, true);
});
test("a streamed JSON body is assembled without losing exact evidence", async () => {
  const bytes = new TextEncoder().encode(JSON.stringify(fixture()));
  const body = new ReadableStream({ start(c) { c.enqueue(bytes.slice(0, 99)); c.enqueue(bytes.slice(99)); c.close(); } });
  const r = await readLive({ now: NOW, fetchImpl: async () => new Response(body, { headers: { "content-type": "application/json" } }) });
  assert.deepEqual(r.records, fixture().records);
});
test("saved snapshots reopen with the same references and limits; CLI errors have nonzero status", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "contribution-consumer-"));
  try {
    const file = path.join(dir, "snapshot.json"); fs.writeFileSync(file, JSON.stringify(fixture()));
    assert.deepEqual(readFile(file, { now: NOW }).records, fixture().records);
    assert.equal((await main(["--file", file, "--profile", "https://github.com/example"])).matched_count, 1);
    const script = path.resolve(__dirname, "../examples/contribution-evidence/read.cjs");
    const good = spawnSync(process.execPath, [script, "--file", file], { encoding: "utf8" });
    assert.ifError(good.error); assert.equal(good.status, 0); assert.equal(JSON.parse(good.stdout).state, "available");
    fs.writeFileSync(file, "{}");
    const bad = spawnSync(process.execPath, [script, "--file", file], { encoding: "utf8" });
    assert.ifError(bad.error); assert.equal(bad.status, 1); assert.equal(JSON.parse(bad.stdout).state, "unavailable"); assert.equal(JSON.parse(bad.stdout).records, null);
    fs.writeFileSync(file, " ".repeat(LIMIT + 1)); assert.throws(() => readFile(file));
  } finally { fs.rmSync(dir, { recursive: true, force: true }); }
});
test("missing, ambiguous or unsupported CLI arguments fail without fetching", async () => {
  for (const args of [[], ["--file"], ["--live", "--file", "anything"], ["--live", "--live"], ["--url", "https://example.com"], ["--profile"], ["--file", "--live"]]) await assert.rejects(main(args));
});
