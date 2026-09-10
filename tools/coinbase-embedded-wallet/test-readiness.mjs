import assert from "node:assert/strict";
import { test } from "node:test";
import { createReadinessGate } from "./src/readiness.js";

test("startup timeout is visible, bounded, and never retries itself", async () => {
  let calls = 0, clock = 100;
  const gate = createReadinessGate(() => { calls++; return new Promise(() => {}); }, { timeoutMs: 5, now: () => clock });
  await assert.rejects(gate.run(), { code: "coinbase_startup_unavailable" });
  assert.equal(calls, 1);
  await assert.rejects(gate.run(), { code: "coinbase_startup_unavailable" });
  await assert.rejects(gate.run(), { code: "coinbase_startup_paused" });
  assert.equal(calls, 2);
  assert.equal(gate.snapshot().state, "paused");
  clock += 60001;
  await assert.rejects(gate.run(), { code: "coinbase_startup_unavailable" });
  assert.equal(calls, 3);
});

test("concurrent probes share one check and a late result cannot hide a timeout", async () => {
  let calls = 0, finish;
  const gate = createReadinessGate(() => { calls++; return new Promise(resolve => { finish = resolve; }); }, { timeoutMs: 5 });
  const results = await Promise.allSettled([gate.run(), gate.run()]);
  assert.equal(calls, 1);
  assert.ok(results.every(result => result.status === "rejected"));
  finish();
  await Promise.resolve();
  assert.equal(gate.snapshot().state, "unavailable");
});

test("configuration failures are redacted and a successful manual retry recovers", async () => {
  let attempts = 0;
  const gate = createReadinessGate(async () => {
    if (++attempts === 1) throw new Error("Network Error https://provider.invalid?token=secret user@example.com");
  });
  await assert.rejects(gate.run(), error => !/secret|user@/.test(error.message));
  assert.equal((await gate.run()).state, "ready");
  assert.equal((await gate.run()).consecutiveFailures, 0);
  assert.equal(attempts, 2);
});
