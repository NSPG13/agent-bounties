"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { workspaceAccountReturn } = require("../site/solarpunk-home.js");
function browser(target) {
  const values = new Map(); const navigations = [];
  return { navigations, location: { origin: "https://agentizable.ai", search: `?workspaceReturn=${encodeURIComponent(target)}`, replace: t => navigations.push(t) },
    sessionStorage: { getItem: k => values.get(k) ?? null, setItem: (k,v) => values.set(k,v), removeItem: k => values.delete(k) } };
}
test("the same account form preserves a bounty across OAuth and returns once", () => {
  const target = "/workspace/?project=0988a16c-3d9a-4c16-8c7d-d5664aee2961";
  const win = browser(target);
  assert.equal(workspaceAccountReturn(win, "remember", 100), target);
  win.location.search = "?auth=success";
  assert.equal(workspaceAccountReturn(win, "remember", 200), target);
  assert.deepEqual(win.navigations, []);
  assert.equal(workspaceAccountReturn(win, "complete", 300), target);
  assert.equal(workspaceAccountReturn(win, "complete", 400), null);
  assert.deepEqual(win.navigations, [target]);
});
test("account return refuses external, credential-bearing and unexpected paths", () => {
  for (const target of ["https://evil.test/workspace/", "//evil.test/workspace/", "/workspace/?cognito", "/workspace/callback", "/workspace/#token=secret", "/workspace/?project=a&project=b", "/workspace/?project=", "https://user:pass@agentizable.ai/workspace/"]) {
    const win = browser(target);
    assert.equal(workspaceAccountReturn(win, "remember", 100), null, target);
    assert.equal(workspaceAccountReturn(win, "complete", 101), null);
    assert.deepEqual(win.navigations, []);
  }
});
test("expired and cancelled onboarding does not redirect later account visits", () => {
  for (const action of ["expire", "clear"]) {
    const win = browser("/workspace/");
    workspaceAccountReturn(win, "remember", 100);
    if (action === "clear") workspaceAccountReturn(win, "clear", 101);
    assert.equal(workspaceAccountReturn(win, "complete", action === "expire" ? 1800100 : 102), null);
    assert.deepEqual(win.navigations, []);
  }
});
