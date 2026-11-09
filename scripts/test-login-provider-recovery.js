"use strict";
const test = require("node:test"), assert = require("node:assert/strict");
const { fetchAuthSession, createSessionRecovery } = require("../site/solarpunk-home.js");
const postingAuth = require("../site/posting-auth.js");
function browser(fetch) {
  let alarm, cleared = false;
  return { location: new URL("https://agentizable.ai/#login"), AbortController, fetch,
    setTimeout: (fn, ms) => { assert.equal(ms,10000);alarm=fn;return 1; },
    clearTimeout: id => { assert.equal(id,1);cleared=true; },
    expire: () => alarm(), cleared: () => cleared };
}
test("fresh session loads only server-confirmed provider availability with credentials", async () => {
  const payload={authenticated:false,user:null,providers:{github:true,google:true,microsoft:false}};
  const win=browser(async (url,options)=>{
    assert.equal(url,"https://api.agentizable.ai/v1/site-auth/session");
    assert.equal(options.credentials,"include");assert.equal(options.cache,"no-store");
    assert.equal(options.headers.Accept,"application/json");assert.ok(options.signal);
    return {ok:true,json:async()=>payload};
  });
  assert.deepEqual(await fetchAuthSession(win),payload);assert.ok(win.cleared());
});
test("an outage is retryable and never becomes an empty signed-out session", async () => {
  let calls=0;const payload={authenticated:false,providers:{github:true}};
  const win=browser(async()=>++calls===1?{ok:false,status:503}:{ok:true,json:async()=>payload});
  await assert.rejects(fetchAuthSession(win),/unavailable/);assert.ok(win.cleared());
  assert.deepEqual(await fetchAuthSession(win),payload);assert.equal(calls,2);
});
test("unreadable and malformed session responses cannot enable providers",async()=>{
  for (const payload of [null,{}, {authenticated:false,providers:[]}, {authenticated:false,providers:{google:"true"}}]) {
    const win=browser(async()=>({ok:true,json:async()=>payload}));
    await assert.rejects(fetchAuthSession(win),/Invalid/);assert.ok(win.cleared());
  }
  const win=browser(async()=>({ok:true,json:async()=>{throw new Error("HTML response");}}));
  await assert.rejects(fetchAuthSession(win));assert.ok(win.cleared());
});
test("a stalled session request aborts after the bounded timeout and clears its timer", async () => {
  const win=browser((_url,{signal})=>new Promise((_resolve,reject)=>signal.addEventListener("abort",()=>reject(new Error("Aborted")))));
  const pending=fetchAuthSession(win);win.expire();await assert.rejects(pending,/Aborted/);assert.ok(win.cleared());
});

function recoveryBrowser(search, { walletPending = false, resumeWallet } = {}) {
  let calls = 0;
  const payload = { authenticated: true, user: { id: "test-user" }, providers: { github: true } };
  const win = browser(async () => ++calls === 1
    ? { ok: false, status: 503 } : { ok: true, json: async () => payload });
  win.location = new URL(`https://agentizable.ai/${search}#login`);
  const stored = new Map();
  win.sessionStorage = {
    getItem: key => stored.get(key) ?? null,
    setItem: (key, value) => stored.set(key, value),
    removeItem: key => stored.delete(key),
  };
  const events = { navigation: [], history: [], status: [], analytics: [], wallets: 0, dialogs: 0 };
  win.location.replace = target => events.navigation.push(target);
  win.history = { replaceState: (_state, _title, target) => {
    events.history.push(target);
    win.location.href = new URL(target, win.location).href;
  } };
  win.agentBountiesAnalytics = { track: event => events.analytics.push(event) };
  win.AgentBountiesWalletLink = { hasPending: id => walletPending && id === "test-user" };
  const state = { currentUser: null, accountStatus: "checking", leavingForWorkspace: false };
  const recover = createSessionRecovery(win, {
    loadSession: async () => {
      try {
        const session = await fetchAuthSession(win);
        state.currentUser = session.user;
        state.accountStatus = "ready";
        return session;
      } catch { return null; }
    },
    getState: () => state,
    resumeWallet: async () => { events.wallets++; await resumeWallet?.(state); },
    returnToPreparedPost: () => Boolean(state.currentUser && state.accountStatus === "ready" && postingAuth.complete(win)),
    setStatus: message => events.status.push(message),
    showDialog: () => { events.dialogs++; },
  });
  return { win, events, state, recover };
}

test("503 then retry resumes the saved prepared bounty exactly once", async () => {
  for (const query of ["?postReturn=1", "?auth=success&provider=github"]) {
    const { win, events, recover } = recoveryBrowser(query);
    const target = "https://agentizable.ai/post.html?title=Saved%20bounty";
    postingAuth.remember(win, target);
    assert.equal(await recover(), null);
    assert.equal(postingAuth.pending(win), target);
    assert.deepEqual(events.navigation, []);
    assert.equal((await recover()).authenticated, true);
    assert.deepEqual(events.navigation, [target]);
    assert.equal(postingAuth.pending(win), null);
    assert.ok(postingAuth.consumeReceipt(win));
    await recover();
    assert.deepEqual(events.navigation, [target]);
    assert.equal(postingAuth.consumeReceipt(win), null);
    assert.equal(events.dialogs, 0, "no callback dialog over the navigation");
  }
});

test("503 then retry handles and removes the OAuth callback once, retaining unrelated URL state", async () => {
  for (const result of ["success", "error"]) {
    const { win, events, recover } = recoveryBrowser(`?auth=${result}&provider=github&reason=access_denied&keep=1`);
    assert.equal(await recover(), null);
    assert.equal(new URLSearchParams(win.location.search).get("auth"), result);
    assert.equal(events.dialogs, 0);
    await recover();
    assert.equal(events.dialogs, 1);
    assert.deepEqual(events.history, ["/?keep=1#login"]);
    assert.deepEqual(events.status, [result === "success" ? "Signed in with GitHub." : "Sign-in was cancelled before access was granted."]);
    assert.deepEqual(events.analytics, result === "success" ? ["auth_completed"] : []);
    await recover();
    assert.equal(events.dialogs, 1);
    assert.equal(events.history.length, 1);
    assert.equal(events.status.length, 1);
    assert.equal(events.analytics.length, result === "success" ? 1 : 0);
  }
});

test("concurrent successful retries share wallet restoration and the following post handoff", async () => {
  let release, entered;
  const walletStarted = new Promise(resolve => { entered = resolve; });
  const walletDone = new Promise(resolve => { release = resolve; });
  const { win, events, recover } = recoveryBrowser("?postReturn=1", {
    walletPending: true, resumeWallet: () => { entered(); return walletDone; },
  });
  const target = "https://agentizable.ai/post.html?title=Keep%20context";
  postingAuth.remember(win, target);
  await recover();
  const first = recover();
  await walletStarted;
  const second = recover();
  assert.equal(events.wallets, 1);
  assert.deepEqual(events.navigation, [], "wait for wallet restoration before returning");
  release();
  await Promise.all([first, second]);
  await recover();
  assert.equal(events.wallets, 1);
  assert.deepEqual(events.navigation, [target]);
});

test("account mode preserves the pending post and Workspace navigation takes precedence", async () => {
  const account = recoveryBrowser("?auth=success&provider=github");
  account.win.location.hash = "#account";
  postingAuth.remember(account.win, "https://agentizable.ai/post.html");
  await account.recover();
  await account.recover();
  assert.deepEqual(account.events.navigation, []);
  assert.ok(postingAuth.pending(account.win));
  assert.deepEqual(account.events.history, ["/#account"]);

  for (const alreadyLeaving of [true, false]) {
    const workspace = recoveryBrowser("?auth=success&provider=github", {
      walletPending: true, resumeWallet: state => { state.leavingForWorkspace = true; },
    });
    workspace.state.leavingForWorkspace = alreadyLeaving;
    postingAuth.remember(workspace.win, "https://agentizable.ai/post.html");
    await workspace.recover();
    await workspace.recover();
    assert.equal(workspace.events.wallets, alreadyLeaving ? 0 : 1);
    assert.equal(workspace.events.dialogs, 0);
    assert.deepEqual(workspace.events.navigation, []);
    assert.deepEqual(workspace.events.analytics, []);
  }
});
