"use strict";
const test = require("node:test"), assert = require("node:assert/strict");
const { parse, postingHref } = require("../site/posting-drafts.js");
const ORIGIN = "https://agentizable.ai";
const A = "12345678-1234-4234-8234-123456789abc", B = "22345678-1234-4234-8234-123456789abc", C = "32345678-1234-4234-8234-123456789abc";
const row = (id, extra = {}) => ({ id: `draft:${id}`, operation_id: id, title: `Draft ${id.slice(0, 1)}`, group: "drafts", status: "Saved draft",
  continuation_url: `${ORIGIN}/post.html?operation_id=${id}#bounty-preview`, revision: 2, can_delete: true, updated_at: "2026-11-20T10:00:00Z", ...extra });

test("lists the account's drafts with their first-party links and marks the one open here", () => {
  const view = parse({ status: "available", items: [row(A), row(B), row(C, { group: "needs_action", status: "Check posting status", can_delete: false })], next_offset: 50 }, ORIGIN, A.toUpperCase());
  assert.equal(view.available, true); assert.equal(view.nextOffset, 50);
  assert.deepEqual(view.items.map((item) => item.operationId), [A, B, C]);
  assert.equal(view.items[0].open, true); assert.equal(view.items[0].status, "Open now");
  assert.equal(view.items[0].deletable, false, "the draft open in this tab is never deleted from the list");
  assert.equal(view.items[1].deletable, true);
  assert.equal(view.items[2].deletable, false, "a draft with a wallet step needs its status checked, not deleted");
  assert.equal(view.items[2].needsAction, true);
  assert.equal(view.items[1].href, `${ORIGIN}/post.html?operation_id=${B}#bounty-preview`);
});

test("drops foreign links, malformed identifiers, duplicates and unknown groups; Delete needs a server revision", () => {
  const view = parse({ status: "available", items: [
    row(A, { continuation_url: "https://evil.example/post.html" }),
    row(A, { continuation_url: `${ORIGIN}/participate.html` }),
    row("not-a-uuid"),
    row(B, { group: "paid" }),
    row(C, { revision: 0 }), row(C),
  ] }, ORIGIN, null);
  assert.deepEqual(view.items.map((item) => [item.operationId, item.deletable]), [[C, false]]);
  assert.equal(postingHref("javascript:alert(1)", ORIGIN), null);
  assert.equal(postingHref(`https://user:pw@agentizable.ai/post.html`, ORIGIN), null);
});

test("an unavailable list is reported as unavailable, never as no drafts", () => {
  for (const payload of [null, { status: "unavailable", items: [] }, { status: "available" }]) {
    const view = parse(payload, ORIGIN, null);
    assert.equal(view.available, false); assert.deepEqual(view.items, []);
  }
});
