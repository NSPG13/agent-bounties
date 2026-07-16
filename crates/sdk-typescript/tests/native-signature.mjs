import assert from "node:assert/strict";
import test from "node:test";

import { AgentBountiesClient } from "../dist/index.js";

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
