"use strict";
// Workspace visitors share the home account dialog; it must send them back once, never loop.
// Builds on the approach proposed in #1742.
const test = require("node:test");
const assert = require("node:assert/strict");
const { workspaceAccountReturn, workspaceReturnTarget, workspaceReturnNeedsWallet } = require("../site/solarpunk-home.js");

const ORIGIN = "https://agentizable.ai";
const PROJECT = "0988a16c-3d9a-4c16-8c7d-d5664aee2961";
const BOUNTY = `0x${"ab".repeat(32)}`;

test("only a selected project return can omit the wallet; canonical and unknown targets retain it", () => {
  assert.equal(workspaceReturnNeedsWallet(`/workspace/?project=${PROJECT}`, ORIGIN), false);
  for (const target of ["/workspace/", `/workspace/?bounty=${BOUNTY}`,
    `/workspace/?project=${PROJECT}&bounty=${BOUNTY}`, "/workspace/?project=",
    "/workspace/?project=x&funded=false", "/workspace/?project=a&project=b",
    "https://evil.test/workspace/?project=test", undefined]) {
    assert.equal(workspaceReturnNeedsWallet(target, ORIGIN), true, String(target));
  }
});

function browser(target, { hash = "#login" } = {}) {
  const values = new Map();
  const win = {
    navigations: [],
    location: {
      origin: ORIGIN,
      pathname: "/",
      search: target === undefined ? "" : `?workspaceReturn=${encodeURIComponent(target)}`,
      hash,
      assign: (to) => win.navigations.push(["assign", to]),
      replace: (to) => win.navigations.push(["replace", to]),
    },
    history: { replaceState: (_state, _title, url) => {
      const parsed = new URL(url, ORIGIN);
      Object.assign(win.location, { pathname: parsed.pathname, search: parsed.search, hash: parsed.hash });
    } },
    sessionStorage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, String(value)),
      removeItem: (key) => values.delete(key),
    },
  };
  return win;
}

// The Workspace's own redirect to the account dialog.
function arrive(win, target) {
  win.location.search = `?workspaceReturn=${encodeURIComponent(target)}`;
  win.location.hash = "#login";
}

test("accepts the routes the live Workspace sends and nothing else", () => {
  for (const target of ["/workspace/", `/workspace/?project=${PROJECT}`, `/workspace/?bounty=${BOUNTY}`, `/workspace/?project=${PROJECT}&bounty=${BOUNTY}`]) {
    assert.equal(workspaceReturnTarget(target, ORIGIN), target, target);
  }
  for (const target of [
    "https://evil.test/workspace/", "//evil.test/workspace/", "/\\evil.test/workspace/", "workspace/",
    "/workspace", "/workspace/callback", "/workspace/?cognito", "/workspace/#ab_workspace_token=secret",
    "/workspace/?project=a&project=b", "/workspace/?project=", "/workspace/?bounty=0x1234",
    `/workspace/?project=${"a".repeat(65)}`, "https://user:pass@agentizable.ai/workspace/", "/workspace/?next=/admin",
    null, 42, `/${"a".repeat(250)}`,
  ]) {
    assert.equal(workspaceReturnTarget(target, ORIGIN), null, String(target));
  }
});

test("remembers the destination across OAuth, strips it from the address bar and returns once", () => {
  const target = `/workspace/?project=${PROJECT}&bounty=${BOUNTY}`;
  const win = browser(target);
  assert.deepEqual(workspaceAccountReturn(win, "remember", { now: 100 }), { target, bounced: false });
  assert.equal(win.location.search, "");
  assert.equal(win.location.hash, "#login", "the dialog still opens");
  // OAuth returns to the home page without the parameter; the tab still holds the intent.
  win.location.search = "?auth=success&provider=google";
  assert.deepEqual(workspaceAccountReturn(win, "remember", { now: 200 }), { target, bounced: false });
  assert.deepEqual(win.navigations, []);
  assert.deepEqual(workspaceAccountReturn(win, "complete", { now: 300 }), { target, bounced: false });
  assert.deepEqual(win.navigations, [["replace", target]]);
  assert.equal(workspaceAccountReturn(win, "pending", { now: 301 }), null);
  assert.equal(workspaceAccountReturn(win, "complete", { now: 302 }), null);
  assert.equal(win.navigations.length, 1);
});

test("a ready account bounced straight back is not redirected again automatically", () => {
  const win = browser("/workspace/");
  workspaceAccountReturn(win, "remember", { now: 1_000 });
  workspaceAccountReturn(win, "complete", { now: 2_000 });
  arrive(win, "/workspace/");
  assert.deepEqual(workspaceAccountReturn(win, "remember", { now: 3_000 }), { target: "/workspace/", bounced: true });
  assert.equal(workspaceAccountReturn(win, "complete", { now: 3_001 }), null);
  assert.equal(win.navigations.length, 1, "no automatic loop");
  // The person can still choose to try again.
  assert.deepEqual(workspaceAccountReturn(win, "complete", { now: 3_002, manual: true }), { target: "/workspace/", bounced: true });
  assert.deepEqual(win.navigations.at(-1), ["assign", "/workspace/"]);
  // Long after the last return, arriving again is a fresh onboarding.
  arrive(win, "/workspace/");
  assert.deepEqual(workspaceAccountReturn(win, "remember", { now: 3_002 + 2 * 60 * 1000 }), { target: "/workspace/", bounced: false });
});

test("expired, cancelled and invalid intents do not redirect later visits", () => {
  for (const action of ["expire", "clear", "invalid"]) {
    const win = browser("/workspace/");
    workspaceAccountReturn(win, "remember", { now: 100 });
    if (action === "clear") workspaceAccountReturn(win, "clear");
    if (action === "invalid") { arrive(win, "https://evil.test/workspace/"); workspaceAccountReturn(win, "remember", { now: 101 }); }
    assert.equal(workspaceAccountReturn(win, "complete", { now: action === "expire" ? 100 + 30 * 60 * 1000 : 102 }), null, action);
    assert.deepEqual(win.navigations, [], action);
  }
  const plain = browser(undefined, { hash: "#account" });
  assert.equal(workspaceAccountReturn(plain, "remember", { now: 1 }), null);
  assert.equal(workspaceAccountReturn(plain, "complete", { now: 2 }), null);
  assert.deepEqual(plain.navigations, []);
});

test("storage failures fail closed", () => {
  const win = browser("/workspace/");
  win.sessionStorage = { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); }, removeItem() { throw new Error("blocked"); } };
  assert.equal(workspaceAccountReturn(win, "remember", { now: 1 }), null);
  assert.equal(workspaceAccountReturn(win, "complete", { now: 2 }), null);
  assert.deepEqual(win.navigations, []);
});
