#!/usr/bin/env node
"use strict";

import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";

import { removeProfile, stopChrome } from "./test-live-coinbase-wallet.mjs";

class FakeChrome extends EventEmitter {
  constructor({ exitOnTerm = false } = {}) {
    super();
    this.exitCode = null;
    this.signalCode = null;
    this.exitOnTerm = exitOnTerm;
    this.signals = [];
  }

  kill(signal) {
    this.signals.push(signal);
    if (signal === "SIGKILL" || this.exitOnTerm) {
      this.signalCode = signal;
      queueMicrotask(() => this.emit("exit", null, signal));
    }
    return true;
  }
}

test("stopChrome escalates when Chromium does not exit after SIGTERM", async () => {
  const chrome = new FakeChrome();
  await stopChrome(chrome, { gracefulTimeoutMs: 1, forceTimeoutMs: 100 });
  assert.deepEqual(chrome.signals, ["SIGTERM", "SIGKILL"]);
  assert.equal(chrome.signalCode, "SIGKILL");
});

test("stopChrome does not send SIGKILL after a graceful exit", async () => {
  const chrome = new FakeChrome({ exitOnTerm: true });
  await stopChrome(chrome, { gracefulTimeoutMs: 100, forceTimeoutMs: 100 });
  assert.deepEqual(chrome.signals, ["SIGTERM"]);
  assert.equal(chrome.signalCode, "SIGTERM");
});

test("removeProfile retries transient non-empty profile cleanup", async () => {
  let receivedPath = null;
  let receivedOptions = null;
  await removeProfile("/tmp/profile", async (profile, options) => {
    receivedPath = profile;
    receivedOptions = options;
  });
  assert.equal(receivedPath, "/tmp/profile");
  assert.deepEqual(receivedOptions, {
    recursive: true,
    force: true,
    maxRetries: 5,
    retryDelay: 200,
  });
});
