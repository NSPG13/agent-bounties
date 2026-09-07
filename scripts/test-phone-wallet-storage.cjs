"use strict";
// Exercise the pinned WalletConnect storage driver in real Chromium. No wallet,
// relay, RPC or private session is used. All records are synthetic fixtures.
const assert = require("node:assert/strict");
const http = require("node:http");
const path = require("node:path");
const { pathToFileURL } = require("node:url");
const { chromium } = require("../tools/browser-layout/node_modules/playwright");
const { build } = require("../tools/phone-wallet/node_modules/esbuild");
const root = path.resolve(__dirname, "..");

async function main() {
  const { storageRecoveryPlugin } = await import(pathToFileURL(path.join(root, "tools/phone-wallet/storage-recovery-plugin.mjs")));
  const bundles = {};
  for (const patched of [false, true]) {
    const result = await build({
      stdin: { contents: 'export { KeyValueStorage } from "@walletconnect/keyvaluestorage"; export { createStore, get, set, promisifyRequest } from "idb-keyval";', resolveDir: path.join(root, "tools/phone-wallet") },
      bundle: true, write: false, platform: "browser", format: "esm", target: "es2022",
      plugins: patched ? [storageRecoveryPlugin] : [],
    });
    bundles[patched ? "/storage.js" : "/unpatched.js"] = result.outputFiles[0].text;
  }
  const server = http.createServer((req, res) => {
    if (req.url === "/") return res.writeHead(200, { "Content-Type": "text/html" }).end("<!doctype html><title>Storage regression fixture</title>");
    if (bundles[req.url]) return res.writeHead(200, { "Content-Type": "text/javascript" }).end(bundles[req.url]);
    res.writeHead(404).end();
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  const deadline = setTimeout(() => { void browser?.close(); }, 45000);
  try {
    const origin = "http://127.0.0.1:" + server.address().port;
    browser = await chromium.launch({ headless: true });
    const context = await browser.newContext();
    await context.route("**/*", route => new URL(route.request().url()).origin === origin ? route.continue() : route.abort());
    await context.addInitScript(() => {
      window.storageFaults = { connections: [], opens: 0, failures: 0, errorName: "InvalidStateError", nextOpenVersion: null };
      const open = IDBFactory.prototype.open, transaction = IDBDatabase.prototype.transaction;
      IDBFactory.prototype.open = function (...args) {
        storageFaults.opens++;
        if (storageFaults.nextOpenVersion !== null) { args[1] = storageFaults.nextOpenVersion; storageFaults.nextOpenVersion = null; }
        const request = open.apply(this, args);
        request.addEventListener("success", () => storageFaults.connections.push(request.result));
        return request;
      };
      IDBDatabase.prototype.transaction = function (...args) {
        if (storageFaults.failures > 0) {
          storageFaults.failures--;
          throw new DOMException("Injected storage failure", storageFaults.errorName);
        }
        return transaction.apply(this, args);
      };
    });
    const page = await context.newPage();
    await page.goto(origin);
    const baseline = await page.evaluate(async () => {
      const { createStore, set, get } = await import("/unpatched.js");
      const store = createStore("unpatched-fixture", "records");
      await set("session", "preserved", store);
      storageFaults.connections.at(-1).close();
      try { await get("session", store); return "unexpected success"; }
      catch (failure) { return failure.name; }
    });
    assert.equal(baseline, "InvalidStateError", "The regression must reproduce with the pinned upstream factory");

    const preservation = await page.evaluate(async () => {
      const { KeyValueStorage } = await import("/storage.js");
      localStorage.setItem("posting-fixture", JSON.stringify({ draft: "reviewed", operation: "awaiting-receipt" }));
      localStorage.setItem("wc@2:fixture:session", JSON.stringify({ fixture: "legacy-session" }));
      const storage = window.fixtureStorage = new KeyValueStorage();
      const migrated = await storage.getItem("wc@2:fixture:session");
      await storage.setItem("wc@2:fixture:history", { id: 1, status: "awaiting-response" });
      const before = storageFaults.opens;
      storageFaults.connections.filter(db => db.name === "WALLET_CONNECT_V2_INDEXED_DB").forEach(db => db.close());
      const values = await Promise.all([storage.getItem("wc@2:fixture:session"), storage.getItem("wc@2:fixture:history")]);
      const reopened = storageFaults.opens - before;
      await storage.setItem("wc@2:fixture:history", { id: 1, status: "received" });
      return { migrated, values, reopened, updated: await storage.getItem("wc@2:fixture:history"), posting: JSON.parse(localStorage.getItem("posting-fixture")) };
    });
    assert.deepEqual(preservation, {
      migrated: { fixture: "legacy-session" }, values: [{ fixture: "legacy-session" }, { id: 1, status: "awaiting-response" }], reopened: 1,
      updated: { id: 1, status: "received" }, posting: { draft: "reviewed", operation: "awaiting-receipt" },
    });

    const lifecycle = await page.evaluate(async () => {
      const upgrade = indexedDB.open("WALLET_CONNECT_V2_INDEXED_DB", 2);
      await new Promise((resolve, reject) => { upgrade.onsuccess = resolve; upgrade.onerror = () => reject(upgrade.error); });
      upgrade.result.close();
      const afterVersionChange = await fixtureStorage.getItem("wc@2:fixture:session");
      const db = storageFaults.connections.at(-1);
      db.close(); db.dispatchEvent(new Event("close"));
      const afterClose = await fixtureStorage.getItem("wc@2:fixture:history");
      return { afterVersionChange, afterClose };
    });
    assert.deepEqual(lifecycle, { afterVersionChange: { fixture: "legacy-session" }, afterClose: { id: 1, status: "received" } });

    const replay = await page.evaluate(async () => {
      const { createStore, set, get, promisifyRequest } = await import("/storage.js");
      const store = createStore("callback-fixture", "records");
      await set("seed", true, store);
      storageFaults.connections.at(-1).close();
      let writeCalls = 0;
      await store("readwrite", objectStore => {
        writeCalls++;
        objectStore.put("once", "write");
        return promisifyRequest(objectStore.transaction);
      });
      let callbackCalls = 0, rejectionCalls = 0, abortedCalls = 0;
      const names = [];
      try { await store("readonly", () => { callbackCalls++; throw new DOMException("Callback failed", "InvalidStateError"); }); }
      catch (failure) { names.push(failure.name); }
      try { await store("readonly", () => { rejectionCalls++; return Promise.reject(new DOMException("Async callback failed", "InvalidStateError")); }); }
      catch (failure) { names.push(failure.name); }
      try { await store("readwrite", objectStore => {
        abortedCalls++; objectStore.put("must-not-commit", "aborted");
        const done = promisifyRequest(objectStore.transaction); objectStore.transaction.abort(); return done;
      }); } catch { names.push("aborted"); }
      const before = storageFaults.opens;
      storageFaults.failures = 2;
      let exhaustedCalls = 0;
      try { await store("readonly", () => { exhaustedCalls++; }); }
      catch (failure) { names.push(failure.name); }
      const boundedOpens = storageFaults.opens - before;
      const afterExhaustion = await get("write", store);
      storageFaults.failures = 1; storageFaults.errorName = "QuotaExceededError";
      const beforeQuota = storageFaults.opens;
      try { await get("write", store); } catch (failure) { names.push(failure.name); }
      return { writeCalls, callbackCalls, rejectionCalls, abortedCalls, exhaustedCalls, boundedOpens, afterExhaustion, quotaOpens: storageFaults.opens - beforeQuota, abortedValue: await get("aborted", store), names };
    });
    assert.deepEqual(replay, { writeCalls: 1, callbackCalls: 1, rejectionCalls: 1, abortedCalls: 1, exhaustedCalls: 0, boundedOpens: 1,
      afterExhaustion: "once", quotaOpens: 0, abortedValue: undefined, names: ["InvalidStateError", "InvalidStateError", "aborted", "InvalidStateError", "QuotaExceededError"] });

    const failedOpen = await page.evaluate(async () => {
      const { createStore, get, promisifyRequest } = await import("/storage.js");
      const request = indexedDB.open("failed-open-fixture", 2);
      request.onupgradeneeded = () => request.result.createObjectStore("records").put("preserved", "session");
      const db = await promisifyRequest(request); db.close();
      storageFaults.nextOpenVersion = 1;
      const store = createStore("failed-open-fixture", "records");
      let failureName;
      try { await get("session", store); } catch (failure) { failureName = failure.name; }
      return { failureName, restored: await get("session", store) };
    });
    assert.deepEqual(failedOpen, { failureName: "VersionError", restored: "preserved" });

    await page.reload();
    const restored = await page.evaluate(async () => {
      const { KeyValueStorage } = await import("/storage.js");
      const storage = new KeyValueStorage();
      return { session: await storage.getItem("wc@2:fixture:session"), history: await storage.getItem("wc@2:fixture:history"), posting: JSON.parse(localStorage.getItem("posting-fixture")) };
    });
    assert.deepEqual(restored, { session: { fixture: "legacy-session" }, history: { id: 1, status: "received" }, posting: { draft: "reviewed", operation: "awaiting-receipt" } });
    console.log("Phone-wallet storage: upstream failure reproduced; sessions, migration, concurrent reconnect, database lifecycle, bounded failures, no operation replay and reload preservation passed.");
  } finally {
    clearTimeout(deadline);
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
}
main().catch(failure => { console.error(failure); process.exitCode = 1; });
