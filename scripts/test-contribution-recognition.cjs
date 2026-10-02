"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { SCHEMA, validate, shareText, render, mount } = require("../site/collaborate/recognition.js");
const NOW = Date.parse("2026-10-02T12:00:00Z");
const { fixture } = require("./fixtures/contribution-recognition.cjs");

class Element {
  constructor(tag) { this.tagName = tag; this.children = []; this.textContent = ""; this.attributes = {}; this.events = {}; }
  append(...elements) { this.children.push(...elements); }
  replaceChildren(...elements) { this.children = elements; }
  setAttribute(name, value) { this.attributes[name] = value; }
  addEventListener(name, fn) { this.events[name] = fn; }
  focus() { this.focused = true; }
  select() { this.selected = true; }
  scrollIntoView() { this.scrolled = true; }
}
function all(node) { return [node, ...node.children.flatMap(all)]; }
function contents(node) { return all(node).map(e => e.textContent).join("\n"); }
function dom() {
  const container = new Element("div"), status = new Element("p");
  const doc = { createElement: tag => new Element(tag), getElementById: id => id === "recognition-records" ? container : id === "recognition-status" ? status : all(container).find(e => e.id === id) };
  return { doc, container, status };
}
test("published registry is valid and contains no synthetic fixtures", () => {
  const data = JSON.parse(fs.readFileSync(path.join(__dirname, "../site/collaborate/recognition.json"), "utf8"));
  validate(data);
  assert.ok(data.records.every(row => row.id !== "synthetic-fixture"));
});
test("reviewed contribution with explicit credit consent preserves its evidence and limits", () => {
  const data = fixture();
  assert.deepEqual(validate(data, NOW), data);
  const card = shareText(data.records[0]);
  assert.match(card, /Contribution reviewed/);
  assert.match(card, /Fixture only; no external participant/);
  assert.match(card, /Payment is not assessed/);
  assert.match(card, /#contribution-synthetic-fixture/);
});
test("missing consent, unreviewed work and fabricated payment state fail closed", () => {
  for (const change of [
    row => { row.credit_consent.public_credit = false; },
    row => { delete row.credit_consent; },
    row => { row.evidence[0].kind = "connection"; },
    row => { row.payment_state = "paid"; },
    row => { row.evidence[0].kind = "settled"; },
    row => { row.identity.state = "verified_agent"; },
    row => { row.identity.state = "owner_confirmed"; },
  ]) { const data = fixture(); change(data.records[0]); assert.throws(() => validate(data, NOW)); }
});
test("owned-profile confirmation needs a public project evidence reference", () => {
  const data = fixture(); data.records[0].identity = { state: "owner_confirmed", evidence_url: "https://github.com/NSPG13/agent-bounties/issues/1#issuecomment-3" };
  assert.doesNotThrow(() => validate(data, NOW));
  data.records[0].identity.evidence_url = "https://github.com/elsewhere/example/issues/1";
  assert.throws(() => validate(data, NOW));
});
test("credentials, private hosts, active URLs, query secrets and escaped paths cannot become public links", () => {
  for (const url of ["javascript:alert(1)", "https://127.0.0.1/private", "https://github.com/NSPG13/agent-bounties/issues/1?token=secret", "https://user:secret@github.com/NSPG13/agent-bounties/issues/1", "https://github.com.evil.test/NSPG13/agent-bounties/issues/1", "https://github.com/NSPG13/agent-bounties/issues/%31", " https://github.com/NSPG13/agent-bounties/issues/1", "https://github.com/NSPG13/agent-bounties/issues/2/../1"]) {
    const data = fixture(); data.records[0].credit_consent.evidence_url = url;
    assert.throws(() => validate(data, NOW), url);
  }
});
test("duplicate IDs, repeated evidence and overlarge or unknown fields fail the whole record", () => {
  for (const change of [
    data => data.records.push(structuredClone(data.records[0])),
    data => data.records[0].evidence.push(structuredClone(data.records[0].evidence[0])),
    data => { data.records[0].display_name = "x".repeat(81); },
    data => { data.records[0].artifact.revision = ""; },
    data => { data.records[0].evidence[0].limitations = ""; },
    data => { data.private_email = "not published"; },
    data => { data.records[0].artifact.extra = "not published"; },
    data => { data.records = Array.from({ length: 51 }, (_, i) => ({ ...data.records[0], id: `fixture-${i}` })); },
  ]) { const data = fixture(); change(data); assert.throws(() => validate(data, NOW)); }
});
test("impossible dates and evidence newer than the snapshot are rejected", () => {
  for (const value of ["2026-02-30T00:00:00Z", "2026-10-03T00:00:00Z", "2026-10-01T13:00:00Z", "unknown"]) {
    const data = fixture(); data.records[0].evidence[0].checked_at = value;
    assert.throws(() => validate(data, NOW));
  }
});
test("human cards retain proof boundaries and copy only on explicit action", async () => {
  const d = dom(), writes = [], data = fixture();
  data.records[0].display_name = "<img src=x onerror=alert(1)>";
  render(d.doc, d.container, data, { writeText: async value => writes.push(value) });
  assert.match(contents(d.container), /Profile ownership not independently confirmed/);
  assert.match(contents(d.container), /Payment is not assessed/);
  assert.equal(all(d.container).filter(e => e.tagName === "img").length, 0);
  assert.equal(writes.length, 0);
  await all(d.container).find(e => e.tagName === "button").events.click();
  assert.equal(writes[0], shareText(data.records[0]));
  assert.match(contents(d.container), /nothing was posted/);
});
test("clipboard failure offers focused selectable text without publishing", async () => {
  const d = dom(); render(d.doc, d.container, fixture(), undefined);
  const fallback = all(d.container).find(e => e.tagName === "textarea");
  assert.equal(fallback.hidden, true);
  await all(d.container).find(e => e.tagName === "button").events.click();
  assert.equal(fallback.hidden, false); assert.equal(fallback.focused, true); assert.equal(fallback.selected, true);
  assert.match(fallback.value, /Fixture only/);
});
test("empty record is an honest opt-in state, not a user census", () => {
  const d = dom(), data = fixture(); data.records = [];
  render(d.doc, d.container, data, undefined);
  assert.match(contents(d.container), /No contributors have been listed through this opt-in process yet/);
  assert.match(contents(d.container), /not external participation/);
});
test("public read omits credentials, clears its timeout and resumes an exact contribution anchor", async () => {
  const d = dom(), requests = [], cleared = [];
  await mount({ document: d.doc, navigator: {}, location: { hash: "#contribution-synthetic-fixture" }, AbortController, setTimeout: () => 123, clearTimeout: id => cleared.push(id), fetch: async (...args) => { requests.push(args); return { ok: true, text: async () => JSON.stringify(fixture()) }; } });
  assert.equal(requests.length, 1); assert.equal(requests[0][0], "recognition.json"); assert.equal(requests[0][1].credentials, "omit");
  assert.deepEqual(cleared, [123]); assert.equal(d.container.attributes["aria-busy"], "false");
  assert.equal(d.doc.getElementById("contribution-synthetic-fixture").scrolled, true);
});
test("network and invalid data failures remain unavailable, never a fabricated empty population", async () => {
  for (const fetch of [async () => { throw Error("offline"); }, async () => ({ ok: false }), async () => ({ ok: true, text: async () => "{}" }), async () => ({ ok: true, text: async () => "x".repeat(1000001) })]) {
    const d = dom(); d.container.append(new Element("old-card"));
    await mount({ document: d.doc, navigator: {}, location: { hash: "" }, AbortController, setTimeout: () => 1, clearTimeout: () => {}, fetch });
    assert.equal(d.container.children.length, 0); assert.match(d.status.textContent, /could not be verified/); assert.match(d.status.textContent, /not evidence of zero participation/);
    assert.equal(d.container.attributes["aria-busy"], "false");
  }
});
