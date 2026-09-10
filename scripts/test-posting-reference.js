"use strict";
const assert = require("node:assert/strict");
const test = require("node:test");
const { readFileSync } = require("node:fs");
const { join } = require("node:path");
const { webcrypto } = require("node:crypto");
const references = require("../site/posting-reference.js");
const home = require("../site/solarpunk-home.js");

function browser(fetch) {
  return { location: new URL("https://agentbounties.app/post.html"), fetch, crypto: webcrypto, AbortController, setTimeout, clearTimeout };
}
function image(phase = "day", variant = "desktop") {
  return readFileSync(join(__dirname, "../site", references.asset(phase, variant).path));
}
function response(bytes = image(), headers = {}) {
  return new Response(bytes, { headers: { "content-type": "image/webp", ...headers } });
}
async function capture(phase = "day", variant = "desktop") {
  return references.captureHomepage({ phase, variant }, browser(async () => response(image(phase, variant))));
}

test("all eight homepage originals match immutable revision hashes and lengths", async () => {
  for (const phase of ["dawn", "day", "dusk", "night"]) for (const variant of ["desktop", "mobile"]) {
    const attachment = await capture(phase, variant);
    assert.equal(attachment.byte_length, image(phase, variant).byteLength);
    assert.ok(attachment.byte_length <= references.MAX_BYTES);
    assert.match(attachment.asset_url, new RegExp(`/agent-bounties/${references.REVISION}/site/`));
    assert.equal(attachment.source_url, "https://agentbounties.app/");
    assert.ok(Date.now() - Date.parse(attachment.captured_at) < 1000);
    assert.deepEqual(references.validate(JSON.parse(JSON.stringify(attachment))), attachment);
    assert.ok(JSON.stringify(attachment).length < 800);
    assert.equal(attachment.data_url, undefined);
  }
});

test("suggested image follows actual homepage dominant plate at every minute", () => {
  for (let minute = 0; minute < 1440; minute++) {
    const date = new Date(2026, 8, 10, Math.floor(minute / 60), minute % 60, 0);
    assert.equal(references.suggestedPhase(date), home.sceneBlend(minute).phase, `minute ${minute}`);
  }
});

test("capture uses only the selected same-origin asset with no credentials or redirects", async () => {
  let requested;
  await references.captureHomepage({ phase: "day", variant: "desktop" }, browser(async (url, options) => {
    requested = { url, options }; return response();
  }));
  assert.equal(requested.url, "https://agentbounties.app/assets/solarpunk/scene-day.webp");
  assert.equal(requested.options.credentials, "omit");
  assert.equal(requested.options.redirect, "error");
  assert.equal(requested.options.cache, "no-store");
  await assert.rejects(references.captureHomepage({ phase: "../../evil", variant: "desktop" }, browser(() => assert.fail("must not fetch"))), /Choose one/);
});

test("changed bytes, bogus MIME and empty responses cannot produce frozen references", async () => {
  const tampered = Buffer.from(image()); tampered[100] ^= 1;
  for (const [result, message] of [
    [response(tampered), /has changed/],
    [response(image(), { "content-type": "text/html" }), /not a WebP/],
    [response(new Uint8Array()), /empty/],
    [new Response("unavailable", { status: 503 }), /503/],
  ]) await assert.rejects(references.captureHomepage({ phase: "day", variant: "desktop" }, browser(async () => result)), message);
});

test("size bounds are enforced before reading and while streaming", async () => {
  const selection = { phase: "day", variant: "desktop" };
  await assert.rejects(references.captureHomepage(selection, browser(async () => response(image(), { "content-length": String(references.MAX_BYTES + 1) }))), /size limit/);
  let cancelled = false;
  const stream = new ReadableStream({ pull(controller) { controller.enqueue(new Uint8Array(100000)); }, cancel() { cancelled = true; } });
  await assert.rejects(references.captureHomepage(selection, browser(async () => new Response(stream, { headers: { "content-type": "image/webp" } }))), /size limit/);
  assert.equal(cancelled, true);
});

test("network timeout fails clearly without a captured descriptor", async () => {
  const win = browser((_url, options) => new Promise((_resolve, reject) => options.signal.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")))));
  win.setTimeout = (fn) => setTimeout(fn, 1);
  await assert.rejects(references.captureHomepage({ phase: "day", variant: "desktop" }, win), /within 8 seconds.*Nothing was captured/);
});

test("restored references reject altered hashes, mutable URLs, size, time and extra payload", async () => {
  const attachment = await capture();
  for (const patch of [
    { sha256: `sha256:${"0".repeat(64)}` },
    { asset_url: attachment.asset_url.replace(references.REVISION, "main") },
    { asset_url: "https://evil.example/image.webp" },
    { byte_length: references.MAX_BYTES + 1 },
    { captured_at: "yesterday" },
    { captured_at: new Date(Date.now() + 3600000).toISOString() },
    { source_url: "https://evil.example/" },
    { phase: "night" },
    { data_url: "data:image/webp;base64,AAAA" },
  ]) assert.throws(() => references.validate({ ...attachment, ...patch }));
});

test("reference is bound in evidence without replacing verifier fields or cover images", async () => {
  const attachment = await capture();
  const schema = { type: "object", required: ["artifact_url", "artifact_sha256"], properties: { artifact_sha256: { type: "string" } } };
  const bound = references.withEvidence(schema, attachment);
  assert.deepEqual(bound.required, schema.required);
  assert.deepEqual(bound.properties, schema.properties);
  assert.equal(schema[references.EXTENSION], undefined);
  assert.deepEqual(bound[references.EXTENSION], attachment);
  assert.deepEqual(references.withEvidence(bound, attachment), bound);
  assert.throws(() => references.withEvidence(bound, { ...attachment, captured_at: "2026-09-01T12:00:00Z" }), /different reference/);
  assert.throws(() => references.withEvidence(null, attachment), /evidence schema/);
  assert.equal(references.withEvidence(schema, null), schema);
});
