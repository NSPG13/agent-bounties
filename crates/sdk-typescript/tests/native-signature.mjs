import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  AgentBountiesClient,
  buildOpenCompetitionV2PublicVector,
  generateOpenCompetitionCommitment,
} from "../dist/index.js";

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

test("open competition commitments use local random recovery envelopes", () => {
  const input = {
    network: "base-sepolia",
    bounty: "0x1111111111111111111111111111111111111111",
    solver: "0x2222222222222222222222222222222222222222",
    submission_hash: `0x${"aa".repeat(32)}`,
    evidence_hash: `0x${"bb".repeat(32)}`,
  };
  const first = generateOpenCompetitionCommitment(input);
  const second = generateOpenCompetitionCommitment(input);

  assert.equal(first.schema_version, "agent-bounties/open-competition-v1-commitment-v1");
  assert.equal(first.chain_id, 84532);
  assert.match(first.salt, /^0x[0-9a-f]{64}$/);
  assert.match(first.commitment, /^0x[0-9a-f]{64}$/);
  assert.notEqual(first.salt, second.salt);
  assert.notEqual(first.commitment, second.commitment);
  assert.equal(first.committed_block, null);
  assert.equal(first.reveal_deadline, null);
});

test("open competition entrant relay preserves the exact plan and signature", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    requests.push({ url, body: JSON.parse(init.body) });
    return new Response(JSON.stringify({ status: "broadcast" }), {
      status: 202,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    const plan = {
      schema_version: "agent-bounties/open-competition-entrant-wallet-action-v1",
      network: "base-mainnet",
      nonce: 7,
      payload_hash: `0x${"aa".repeat(32)}`,
    };
    const signature = `0x${"11".repeat(64)}1b`;
    await client.relayOpenCompetitionEntrantAction({
      idempotency_key: "entrant-relay-7",
      plan,
      signature,
    });

    assert.equal(
      requests[0].url,
      "https://api.example/v1/base/open-competition-v1/entrant-action-relays",
    );
    assert.deepEqual(requests[0].body.plan, plan);
    assert.equal(requests[0].body.signature, signature);
    assert.equal(requests[0].body.idempotency_key, "entrant-relay-7");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("open competition V2 creation preserves decimal strings", async () => {
  const requests = [];
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    requests.push({ url, body: JSON.parse(init.body) });
    return new Response(JSON.stringify({ valid: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  try {
    const client = new AgentBountiesClient("https://api.example");
    await client.validateOpenCompetitionV2({
      creator: "0x1111111111111111111111111111111111111111",
      creation_nonce: "9007199254740993",
      acknowledged_risk_hash: `0x${"11".repeat(32)}`,
      initial_funding: "1000001",
      params: {
        solver_reward: "1000000",
        keeper_reward: "1",
        funding_deadline: 1,
        proof_window_seconds: 1,
        winner_mode: "first_proven",
        score_direction: "higher_is_better",
        score_threshold: "0",
        proof_system: "groth16",
        program_vkey: `0x${"12".repeat(32)}`,
        source_hash: `0x${"13".repeat(32)}`,
        elf_hash: `0x${"14".repeat(32)}`,
        journal_schema_hash: `0x${"15".repeat(32)}`,
        metric_program_hash: `0x${"16".repeat(32)}`,
        execution_policy_hash: `0x${"17".repeat(32)}`,
        verification_policy_hash: `0x${"18".repeat(32)}`,
        settlement_policy_hash: `0x${"19".repeat(32)}`,
        beta_risk_hash: `0x${"11".repeat(32)}`,
      },
    });

    assert.equal(
      requests[0].url,
      "https://api.example/v1/base/open-competition-v2-beta1/validate",
    );
    assert.equal(requests[0].body.network, "base-mainnet");
    assert.equal(requests[0].body.creation_nonce, "9007199254740993");
    assert.equal(requests[0].body.initial_funding, "1000001");
    assert.equal(requests[0].body.params.solver_reward, "1000000");
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test("open competition V2 shared release vector matches exactly", async () => {
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../../programs/public-vector-metric-v1/fixtures/golden-v1.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const scope = fixture.scope;
  const hex = (values) => `0x${Buffer.from(values).toString("hex")}`;
  const result = buildOpenCompetitionV2PublicVector({
    scope: {
      chain_id: scope.chain_id,
      competition: hex(scope.competition),
      bounty_id: hex(scope.bounty_id),
      solver: hex(scope.solver),
      solver_nonce: String(scope.solver_nonce),
      proof_system: "groth16",
      program_vkey: hex(scope.program_vkey),
      source_hash: hex(scope.source_hash),
      elf_hash: hex(scope.elf_hash),
      execution_policy_hash: hex(scope.execution_policy_hash),
      settlement_policy_hash: hex(scope.settlement_policy_hash),
      beta_risk_hash: hex(scope.beta_risk_hash),
    },
    mode: fixture.mode,
    threshold: String(fixture.threshold),
    vectors: fixture.vectors,
  });
  assert.deepEqual(result, fixture.expected);
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
    await client.analyzeBountyFit("0x1111111111111111111111111111111111111111", null);

    assert.equal(
      requests[0].url,
      "https://api.example/v1/base/autonomous-bounties/feed?network=base-mainnet&claimable_only=false",
    );
    assert.equal(requests[0].init.headers["x-operator-token"], "operator-token");
    assert.equal(requests[1].url, "https://api.example/v1/analytics/site?window_hours=0");
    assert.equal(
      requests[2].url,
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
