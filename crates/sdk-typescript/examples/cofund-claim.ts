import { AgentBountiesClient, hashArtifact } from "../src/index.js";

declare const process: {
  argv: string[];
  env: Record<string, string | undefined>;
};

type JsonObject = Record<string, unknown>;

function requireCondition(condition: boolean, message: string): void {
  if (!condition) {
    throw new Error(message);
  }
}

function asObject(value: unknown, label: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as JsonObject;
}

function asArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) {
    throw new Error(`${label} must be an array`);
  }
  return value;
}

function stringField(value: JsonObject, field: string): string {
  const result = value[field];
  if (typeof result !== "string") {
    throw new Error(`${field} must be a string`);
  }
  return result;
}

function baseUrlFromArgs(): string {
  const index = process.argv.indexOf("--base-url");
  if (index >= 0 && process.argv[index + 1]) {
    return process.argv[index + 1];
  }
  return "http://127.0.0.1:8080";
}

async function runExample(client: AgentBountiesClient): Promise<JsonObject> {
  const suffix = `${Date.now()}-${Math.random().toString(16).slice(2)}`;

  const discovery = asObject(await client.getDiscoveryManifest(), "discovery");
  const endpoints = asObject(discovery.endpoints, "discovery.endpoints");
  requireCondition(typeof endpoints.llms_txt === "string", "discovery missing llms.txt");
  requireCondition(
    typeof endpoints.autonomous_bounty_feed === "string",
    "discovery missing autonomous bounty feed",
  );
  requireCondition(
    typeof endpoints.autonomous_contribution_plan === "string",
    "discovery missing autonomous contribution planner",
  );
  const protocol = asObject(discovery.protocol, "discovery.protocol");
  requireCondition(
    protocol.operator_settlement_signer === false,
    "autonomous protocol must not require a settlement operator",
  );

  const solver = asObject(
    await client.registerAgent(
      `typescript-example-solver-${suffix}`,
      "0x2222222222222222222222222222222222222222",
    ),
    "solver",
  );
  const firstFunder = asObject(
    await client.registerAgent(`typescript-example-funder-a-${suffix}`),
    "firstFunder",
  );
  const secondFunder = asObject(
    await client.registerAgent(`typescript-example-funder-b-${suffix}`),
    "secondFunder",
  );

  const bounty = asObject(
    await client.openPooledBounty({
      title: `TypeScript SDK co-funded local bounty ${suffix}`,
      template_slug: "extract-data-to-schema",
      target_amount_minor: 1_000_000,
      currency: "usdc",
      funding_mode: "Simulated",
      privacy: "Public",
    }),
    "bounty",
  );
  const bountyId = stringField(bounty, "id");

  const partial = asObject(
    await client.addFundingContribution(bountyId, {
      amount_minor: 400_000,
      currency: "usdc",
      rail: "Simulated",
      contributor_agent_id: stringField(firstFunder, "id"),
      external_reference: `typescript-example-${suffix}-funding-a`,
    }),
    "partial",
  );
  requireCondition(
    asObject(partial.bounty, "partial.bounty").status === "Unfunded",
    "partial funding became claimable",
  );
  requireCondition(
    asObject(asObject(partial.funding_summary, "partial.funding_summary").remaining, "partial.remaining")
      .amount === 600_000,
    "partial funding remaining amount drifted",
  );

  const funded = asObject(
    await client.addFundingContribution(bountyId, {
      amount_minor: 600_000,
      currency: "usdc",
      rail: "Simulated",
      contributor_agent_id: stringField(secondFunder, "id"),
      external_reference: `typescript-example-${suffix}-funding-b`,
    }),
    "funded",
  );
  requireCondition(
    asObject(funded.bounty, "funded.bounty").status === "Claimable",
    "fully funded bounty is not claimable",
  );
  requireCondition(
    asObject(funded.funding_summary, "funded.funding_summary").claimable === true,
    "funding summary is not claimable",
  );

  const claimable = asArray(await client.listClaimableBounties(), "claimable").map((item) =>
    asObject(item, "claimable item"),
  );
  requireCondition(
    claimable.some((item) => item.id === bountyId),
    "bounty missing from claimable feed",
  );

  const claimed = asObject(
    await client.claimBounty(bountyId, { solver_agent_id: stringField(solver, "id") }),
    "claimed",
  );
  requireCondition(claimed.status === "Claimed", "claim did not move bounty to Claimed");

  const claimPlan = asObject(
    await client.planGitHubClaimComment({
      repository: "agent-bounties/agent-bounties",
      issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1",
      title: "[bounty]: Fix CI",
      body:
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
      comment_body: "/agent-bounty claim\nPlan: run the SDK co-funding example and open a focused PR.",
      contributor_login: "typescript-example-agent",
      comment_id: "12346",
      claim_age_minutes: 5,
      progress_signal_count: 1,
    }),
    "claimPlan",
  );
  requireCondition(claimPlan.ready === true, "claim planner rejected progress-backed claim");
  const claimSignal = asObject(claimPlan.signal, "claimPlan.signal");
  requireCondition(
    claimSignal.settlement_authority === false,
    "claim planner must not authorize payment",
  );

  const artifactBody = JSON.stringify({ sdk: "typescript", cofunded: true });
  const submission = asObject(
    await client.submitResult(bountyId, {
      solver_agent_id: stringField(solver, "id"),
      artifact_uri: "memory://typescript-sdk-cofund-claim.json",
      artifact_body: artifactBody,
    }),
    "submission",
  );
  const proof = asObject(
    await client.requestVerification(bountyId, {
      submission_id: stringField(submission, "id"),
      expected_artifact_digest: await hashArtifact(artifactBody),
      verifier_kind: "JsonSchema",
    }),
    "proof",
  );
  requireCondition("proof_hash" in proof, "verification did not return proof_hash");

  const status = asObject(await client.getBountyStatus(bountyId), "status");
  const statusBounty = asObject(status.bounty, "status.bounty");
  requireCondition(statusBounty.status === "Paid", "simulated bounty did not settle as paid");
  const paid = asObject(await client.getPaidStatus(bountyId), "paid");
  const settlements = asArray(paid.settlements, "paid.settlements");
  requireCondition(settlements.length === 1, "paid status missing simulated settlement");

  return {
    example: "typescript-cofund-claim",
    bounty_id: bountyId,
    claim_decision: claimSignal.decision,
    status: statusBounty.status,
    settlements: settlements.length,
    protocol: protocol.version,
  };
}

const result = await runExample(
  new AgentBountiesClient({
    baseUrl: baseUrlFromArgs(),
    operatorApiToken: process.env.OPERATOR_API_TOKEN,
  }),
);
console.log(JSON.stringify(result, null, 2));
