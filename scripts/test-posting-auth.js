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
