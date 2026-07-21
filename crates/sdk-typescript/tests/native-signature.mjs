import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import { AgentBountiesClient } from "../dist/index.js";

test("public declarations match the compatibility fixture", async () => {
  const declarations = (await readFile(new URL("../dist/index.d.ts", import.meta.url), "utf8"))
    .replace(/\r\n/g, "\n");
  const fixture = JSON.parse(
    await readFile(new URL("../fixtures/public-api.json", import.meta.url), "utf8"),
  );
  assert.equal(declarations.split("\n").length, fixture.normalized_declaration_lines);
  assert.equal(
    createHash("sha256").update(declarations).digest("hex"),
    fixture.normalized_declaration_sha256,
  );
});

test("agentNativeClaim replays a native wallet signature unchanged", async () => {
  const walletSignature = `0x${"11".repeat(64)}1b`;
  const requests = [];
  const responses = [
    {
      signing_payload: { primaryType: "ReceiveWithAuthorization" },
      candidate: { status: "authorization_ready" },
    },
    {
      signing_payload: null,
      candidate: { status: "claimed" },
      canonical_event_id: "00000000-0000-0000-0000-000000000001",
    },
  ];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (_url, init) => {
    requests.push(JSON.parse(init.body));
    return new Response(JSON.stringify(responses.shift()), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    const response = await client.agentNativeClaim(
      {
        idempotency_key: "native-signature-test",
        bounty_contract: "0x1111111111111111111111111111111111111111",
        solver_wallet: "0x2222222222222222222222222222222222222222",
      },
      async () => walletSignature,
    );

    assert.equal(response.candidate.status, "claimed");
    assert.equal(requests.length, 2);
    assert.equal(requests[1].wallet_signature, walletSignature);
    assert.equal(requests[1].signature, undefined);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("canonical child planning sends task acceptance criteria", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (_url, init) => {
    requests.push(JSON.parse(init.body));
    return new Response(JSON.stringify({ benchmark_hash: "0x1234" }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    const criteria = ["The committed regression test passes."];
    await client.planAutonomousCanonicalChildTerms({
      parent_bounty_id: `0x${"11".repeat(32)}`,
      parent_round: 1,
      parent_solver: "0x2222222222222222222222222222222222222222",
      parent_solver_reward: { amount: 2_000_000, currency: "usdc" },
      child_acceptance_criteria: criteria,
      verifier_module: "0x3333333333333333333333333333333333333333",
    });

    assert.deepEqual(requests[0].child_acceptance_criteria, criteria);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("compileObjective sends a bounded objective graph request", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    requests.push({ url, body: JSON.parse(init.body) });
    return new Response(JSON.stringify({
      schema_version: "agent-bounties/cloud-objective-plan-v1",
      tasks: [],
    }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    await client.compileObjective({
      objective: "Ship a replayable release",
      constraints: ["Keep settlement deterministic."],
      max_tasks: 4,
      solver_budget_usdc: "8.00",
    });

    assert.equal(requests[0].url, "https://api.example/v1/cloud-agent/objective-plans");
    assert.equal(requests[0].body.max_tasks, 4);
    assert.equal(requests[0].body.solver_budget_usdc, "8.00");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("query methods preserve ordering, false values, and operator headers", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    requests.push({ url, init });
    return new Response("{}", { status: 200 });
  };

  try {
    const client = new AgentBountiesClient({
      baseUrl: "https://api.example",
      operatorApiToken: "operator-token",
    });
    await client.listAutonomousBounties("base-mainnet", false);
    await client.getSiteAnalytics(0);
    await client.getGuildCharter();
    await client.getGuildAdventurerProfile("agent/id with spaces");
    await client.analyzeBountyFit("0x1111111111111111111111111111111111111111", null);

    assert.equal(
      requests[0].url,
      "https://api.example/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
    );
    assert.equal(requests[0].init.headers["x-operator-token"], "operator-token");
    assert.equal(requests[1].url, "https://api.example/v1/analytics/site?window_hours=0");
    assert.equal(
      requests[2].url,
      "https://api.example/v1/guild/charter",
    );
    assert.equal(
      requests[3].url,
      "https://api.example/v1/guild/adventurers/agent%2Fid%20with%20spaces",
    );
    assert.equal(
      requests[4].url,
      "https://api.example/v1/base/autonomous-bounties/0x1111111111111111111111111111111111111111/analysis",
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("webhook requests preserve signature, method, and JSON body", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    requests.push({ url, init });
    return new Response("{}", { status: 200 });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    const event = { id: "evt_123", type: "checkout.session.completed" };
    await client.reconcileStripeCheckoutWebhook(event, "stripe-signature");

    assert.equal(requests[0].url, "https://api.example/v1/stripe/checkout-webhooks");
    assert.equal(requests[0].init.method, "POST");
    assert.equal(requests[0].init.headers["stripe-signature"], "stripe-signature");
    assert.deepEqual(JSON.parse(requests[0].init.body), event);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
