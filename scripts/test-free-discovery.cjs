"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { money, publicLink, assess } = require("../site/collaborate/assessment.js");
const { MAX_FEED_BYTES, FETCH_TIMEOUT_MS, readLiveFeed } = require("../examples/free-discovery/assess.cjs");
const fixture = () => JSON.parse(fs.readFileSync(path.join(__dirname, "../examples/free-discovery/fixture.json"), "utf8"));
const now = "2026-10-01T00:01:00Z";

test("exact money formatting preserves base units beyond floating-point precision", () => {
  const value = { amount: "9007199254740993", currency: "USDC", unit: "base_units", decimals: 6 };
  assert.equal(money(value).display, "9007199254.740993 USDC");
  assert.equal(money({ ...value, amount: "0" }).display, "0 USDC");
  assert.equal(money({ ...value, amount: "1" }).display, "0.000001 USDC");
  for (const invalid of [undefined, null, {}, { ...value, amount: 9007199254740993 }, { ...value, amount: "-1" }, { ...value, currency: "ETH" }, { ...value, decimals: 18 }, { ...value, amount: "1e6" }]) {
    assert.deepEqual(money(invalid), { display: "Not reported", positive: null, base_units: null });
  }
});
test("qualified snapshot invites inspection and never authorizes a claim", () => {
  const result = assess(fixture(), now);
  assert.equal(result.mode, "read_only");
  assert.equal(result.items[0].assessment, "inspect_terms");
  assert.equal(result.items[0].refundable_bond.display, "0.01 USDC");
  assert.equal(result.items[0].required_external_spend.display, "1 USDC");
  assert.match(result.items[0].next_action, /does not claim/);
  assert.match(result.evidence_boundary, /Gas, execution costs/);
  assert.equal(result.items[0].wallet_request, undefined);
});
test("unknown cash, unavailable verification and wrong state cannot pass qualification", () => {
  for (const change of [i => delete i.cash_economics, i => i.verification_ready = false, i => i.work_state = "claimed", i => i.payment_state = "unknown", i => i.network = "base-sepolia", i => i.source_type = "unfunded_offchain"]) {
    const feed = fixture(); change(feed.items[0]);
    const item = assess(feed, now).items[0];
    assert.equal(item.assessment, "needs_recheck");
    assert.ok(item.blockers.length);
  }
});
test("empty results and incomplete source state are distinct from ready work", () => {
  const feed = fixture(); feed.items = [];
  assert.equal(assess(feed, now).sample_size, 0);
  for (const unavailable of [true, undefined, "false"]) {
    const feed = fixture(); feed.degraded = unavailable;
    assert.equal(assess(feed, now).items[0].assessment, "needs_recheck");
  }
});
test("stale snapshots and impossible future clocks require a fresh read", () => {
  assert.equal(assess(fixture(), "2026-10-01T00:06:00Z").stale, true);
  assert.equal(assess(fixture(), "2026-09-30T23:58:00Z").items[0].assessment, "needs_recheck");
  const feed = fixture(); feed.generated_at = "invalid";
  assert.throws(() => assess(feed, now), /format/);
});
test("rejects foreign origins, unsafe URLs and credential-bearing links", () => {
  for (const value of ["javascript:alert(1)", "data:text/html,unsafe", "https://agentbounties.app.evil.example/", "https://evil.example/", "https://user:pass@agentbounties.app/", "http://agentbounties.app/", "https://github.com/other/repo/issues/1"]) assert.equal(publicLink(value), null);
  assert.equal(publicLink("https://github.com/NSPG13/agent-bounties/issues/648"), "https://github.com/NSPG13/agent-bounties/issues/648");
  const feed = fixture(); feed.items[0].public_url = "javascript:alert(1)";
  assert.equal(assess(feed, now).items[0].public_url, null);
  assert.equal(assess(feed, now).items[0].assessment, "needs_recheck");
});
test("bounds the sample and rejects malformed response contracts", () => {
  const feed = fixture(); feed.items = Array(20).fill(feed.items[0]);
  assert.equal(assess(feed, now).items.length, 5);
  for (const invalid of [null, {}, { ...feed, network: "base-sepolia" }, { ...feed, schema_version: "old" }, { ...feed, items: [null] }]) assert.throws(() => assess(invalid, now));
});

test("offline command remains the default and does not call fetch", () => {
  const { execFileSync } = require("node:child_process");
  const cli = path.resolve(__dirname, "../examples/free-discovery/assess.cjs");
  const actual = JSON.parse(execFileSync(process.execPath, [cli], { encoding: "utf8" }));
  assert.equal(actual.source_kind, "offline_fixture_not_adoption");
  const guarded = JSON.parse(execFileSync(process.execPath, ["-e", `global.fetch = () => { throw new Error('Network forbidden'); }; require(${JSON.stringify(cli)}).main([]);`], { encoding: "utf8" }));
  assert.ok(Number.isFinite(Date.parse(actual.observed_at)));
  assert.ok(Number.isFinite(Date.parse(guarded.observed_at)));
  delete actual.observed_at;
  delete guarded.observed_at;
  assert.deepEqual(guarded, actual);
});

test("live reader accepts the exact byte boundary and returns the same assessment input", async () => {
  const body = JSON.stringify(fixture());
  const padded = body + " ".repeat(MAX_FEED_BYTES - Buffer.byteLength(body));
  const result = await readLiveFeed({ fetchImpl: async () => new Response(padded) });
  assert.deepEqual(result, fixture());
  assert.equal(FETCH_TIMEOUT_MS, 10000);
});

test("oversized ASCII and multibyte responses stop before the remaining chunks", async () => {
  for (const character of ["a", "é"]) {
    const bytes = Buffer.from(JSON.stringify({ extra: character.repeat(1200000 / Buffer.byteLength(character)) }));
    const chunks = [bytes.subarray(0, 600000), bytes.subarray(600000, 1200000), bytes.subarray(1200000)];
    let reads = 0, canceled = false, signal;
    const stream = new ReadableStream({
      pull(controller) { controller.enqueue(chunks[reads++]); },
      cancel() { canceled = true; }
    }, { highWaterMark: 0 });
    await assert.rejects(readLiveFeed({ fetchImpl: async (_url, options) => {
      signal = options.signal;
      return new Response(stream);
    } }), /exceeds 1,000,000 bytes/);
    assert.equal(reads, 2);
    assert.equal(canceled, true);
    assert.equal(signal.aborted, true);
  }
});

test("HTTP failures, empty bodies and malformed JSON cannot produce an assessment", async () => {
  let canceled = false;
  const stream = new ReadableStream({ cancel() { canceled = true; } }, { highWaterMark: 0 });
  await assert.rejects(readLiveFeed({ fetchImpl: async () => new Response(stream, { status: 503 }) }), /HTTP 503/);
  assert.equal(canceled, true);
  await assert.rejects(readLiveFeed({ fetchImpl: async () => new Response(null) }), /no response body/);
  await assert.rejects(readLiveFeed({ fetchImpl: async () => new Response("{malformed") }), SyntaxError);
});

test("native fetch refuses redirects and times out during headers or body", async () => {
  const http = require("node:http");
  let redirectedRequests = 0;
  const server = http.createServer((request, response) => {
    if (request.url === "/redirect") { response.writeHead(302, { location: "/redirect-target" }); response.end(); }
    else if (request.url === "/redirect-target") { redirectedRequests++; response.end("{}"); }
    else if (request.url === "/stall-body") { response.writeHead(200); response.write("{"); }
    // /stall-headers deliberately sends nothing until the caller aborts.
  });
  await new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const base = `http://127.0.0.1:${server.address().port}`;
  try {
    const localFetch = route => (_url, options) => fetch(base + route, options);
    await assert.rejects(readLiveFeed({ fetchImpl: localFetch("/redirect") }), /fetch failed/);
    assert.equal(redirectedRequests, 0);
    for (const route of ["/stall-headers", "/stall-body"]) {
      await assert.rejects(readLiveFeed({ fetchImpl: localFetch(route), timeoutMs: 50 }), /timed out/);
    }
  } finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
});

test("live transport failure leaves stdout empty and exits nonzero", () => {
  const { spawnSync } = require("node:child_process");
  const cli = path.resolve(__dirname, "../examples/free-discovery/assess.cjs");
  // Run the actual entry point with an injected local response and no network.
  const script = `global.fetch = async () => new Response('unavailable', { status: 503 }); process.argv = [process.execPath, ${JSON.stringify(cli)}, '--live']; require('node:module').runMain();`;
  const result = spawnSync(process.execPath, ["-e", script], { encoding: "utf8" });
  assert.equal(result.status, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /HTTP 503/);
});
