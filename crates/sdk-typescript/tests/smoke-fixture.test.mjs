import assert from "node:assert/strict";
import test from "node:test";
import { localFixtureClient } from "../dist/smoke-fixture.js";

test("fixture credentials are confined to local service URLs", () => {
  for (const url of [
    "https://api.agentizable.ai", "http://remote.example",
    "http://localhost.remote.example", "http://localhost@remote.example",
    "http://token@127.0.0.1", "http://127.0.0.1/private",
    "http://127.0.0.1?forward=remote", "http://127.0.0.1#remote",
  ]) {
    assert.throws(() => localFixtureClient(url, "local-fixture-only"), /Fixture smoke requires/);
  }
  for (const token of [undefined, "", " "]) {
    assert.throws(() => localFixtureClient("http://127.0.0.1:18280", token), /OPERATOR_API_TOKEN is required/);
  }
});

test("local fixture setup explicitly authenticates registration", async () => {
  const originalFetch = globalThis.fetch;
  const requests = [];
  globalThis.fetch = async (url, init) => {
    requests.push({ url, init });
    return new Response(JSON.stringify({ id: "local-fixture-agent" }), {
      status: 200, headers: { "content-type": "application/json" },
    });
  };
  try {
    const client = localFixtureClient("http://localhost:18280/", "local-fixture-only");
    await client.registerAgent("local-fixture");
    assert.equal(requests.length, 1);
    assert.equal(requests[0].url, "http://localhost:18280/v1/agents");
    assert.equal(requests[0].init.headers["x-operator-token"], "local-fixture-only");
    const session = `abws_${"11".repeat(32)}`;
    const wallet = localFixtureClient("http://localhost:18280/", session, "wallet");
    await wallet.registerAgent("local-wallet-fixture");
    assert.equal(requests[1].init.headers.authorization, `Bearer ${session}`);
    assert.equal(requests[1].init.headers["x-operator-token"], undefined);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
