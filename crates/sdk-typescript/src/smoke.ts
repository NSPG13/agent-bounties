import { AgentBountiesClient, hashArtifact } from "./index.js";

declare const process: { argv: string[] };

type JsonObject = Record<string, unknown>;

function requireCondition(condition: boolean, message: string): void {
  if (!condition) throw new Error(message);
}

function asObject(value: unknown, label: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  return value as JsonObject;
}

function asArray(value: unknown, label: string): unknown[] {
  if (!Array.isArray(value)) throw new Error(`${label} must be an array`);
  return value;
}

function stringField(value: JsonObject, field: string): string {
  const result = value[field];
  if (typeof result !== "string") throw new Error(`${field} must be a string`);
  return result;
}

function baseUrlFromArgs(): string {
  const index = process.argv.indexOf("--base-url");
  return index >= 0 && process.argv[index + 1]
    ? process.argv[index + 1]
    : "http://127.0.0.1:8080";
}

async function exerciseSurface(client: AgentBountiesClient): Promise<JsonObject> {
  const suffix = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  const discovery = asObject(await client.getDiscoveryManifest(), "discovery");
  requireCondition(
    discovery.schema === "https://agentbounties.org/schemas/discovery-manifest.v2.json",
    "discovery manifest missing v2 schema",
  );
  const protocol = asObject(discovery.protocol, "discovery.protocol");
  requireCondition(protocol.version === "agent-bounties/autonomous-v1", "missing autonomous-v1");
  requireCondition(
    protocol.operator_settlement_signer === false,
    "autonomous protocol must not expose an operator settlement signer",
  );

  const tools = asArray(discovery.agent_tools, "discovery.agent_tools");
  for (const tool of [
    "list_autonomous_bounties",
    "plan_autonomous_canonical_child_terms",
    "plan_autonomous_bounty_creation",
    "plan_autonomous_bounty_contribution",
    "agent_native_claim",
    "plan_autonomous_bounty_claim",
    "plan_autonomous_bounty_submission",
    "prepare_autonomous_bounty_submission",
    "plan_autonomous_bounty_submission_authorization",
    "plan_autonomous_module_settlement",
    "plan_autonomous_attestation_settlement",
  ]) {
    requireCondition(tools.includes(tool), `discovery manifest missing ${tool}`);
  }
  requireCondition(
    tools.every(
      (tool) =>
        typeof tool !== "string" ||
        (!tool.startsWith("plan_base_") && !tool.startsWith("reconcile_base_")),
    ),
    "discovery manifest leaked retired V1 tools",
  );

  const endpoints = asObject(discovery.endpoints, "discovery.endpoints");
  for (const endpoint of [
    "autonomous_creation_plan",
    "autonomous_bounty_feed",
    "autonomous_verification_jobs",
    "autonomous_events",
    "autonomous_terms_publish",
  ]) {
    requireCondition(typeof endpoints[endpoint] === "string", `missing endpoint ${endpoint}`);
  }
  const schema = asObject(await client.getDiscoveryManifestSchema(), "schema");
  requireCondition(schema.$id === discovery.schema, "schema id mismatch");
  const endpointRequired = asArray(
    asObject(asObject(schema.properties, "schema.properties").endpoints, "schema.endpoints")
      .required,
    "schema endpoint requirements",
  );
  requireCondition(
      endpointRequired.includes("autonomous_creation_plan") &&
      endpointRequired.includes("autonomous_agent_native_claim") &&
      endpointRequired.includes("autonomous_attestation_settlement_plan"),
    "v2 schema does not require autonomous endpoints",
  );

  const route = asObject(
    await client.routeBlockedGoal({
      goal: "Fix the TypeScript autonomous SDK smoke",
      context: "The result has deterministic acceptance criteria.",
      budget_minor: 1_000_000,
      currency: "usdc",
      privacy: "Public",
    }),
    "route",
  );
  requireCondition("capability_class" in route, "router response missing capability class");
  requireCondition(
    asArray(await client.decodeAutonomousBountyEvents([]), "decoded events").length === 0,
    "autonomous event decoder rejected an empty batch",
  );
  asArray(await client.listAutonomousBountyEvents("base-mainnet"), "autonomous events");

  for (const method of [
    "publishAutonomousBountyTerms",
    "publishAutonomousSubmissionEvidence",
    "planAutonomousCanonicalChildTerms",
    "planAutonomousBountyCreation",
    "planAutonomousBountyAuthorizedCreation",
    "planAutonomousBountyContribution",
    "planAutonomousBountyAuthorizedContribution",
    "agentNativeClaim",
    "planAutonomousBountyClaim",
    "planAutonomousBountyAuthorizedClaim",
    "planAutonomousBountySubmission",
    "prepareAutonomousBountySubmission",
    "planAutonomousBountySubmissionAuthorization",
    "planAutonomousVerificationAttestation",
    "planAutonomousModuleSettlement",
    "planAutonomousAttestationSettlement",
    "planAutonomousCancel",
    "planAutonomousRefundWithdrawal",
  ] as const) {
    requireCondition(typeof client[method] === "function", `TypeScript SDK missing ${method}`);
  }

  const solver = asObject(
    await client.registerAgent(
      `typescript-autonomous-smoke-solver-${suffix}`,
      "0x2222222222222222222222222222222222222222",
    ),
    "solver",
  );
  const bounty = asObject(
    await client.openPooledBounty({
      title: `TypeScript SDK deterministic local loop ${suffix}`,
      template_slug: "extract-data-to-schema",
      target_amount_minor: 1_000,
      currency: "usdc",
      funding_mode: "Simulated",
      privacy: "Public",
    }),
    "bounty",
  );
  const bountyId = stringField(bounty, "id");
  const funded = asObject(
    await client.addFundingContribution(bountyId, {
      amount_minor: 1_000,
      currency: "usdc",
      rail: "Simulated",
      external_reference: `typescript-autonomous-smoke-${suffix}`,
    }),
    "funded",
  );
  requireCondition(
    asObject(funded.bounty, "funded.bounty").status === "Claimable",
    "funding did not reach target",
  );
  const claimed = asObject(
    await client.claimBounty(bountyId, { solver_agent_id: stringField(solver, "id") }),
    "claimed",
  );
  requireCondition(claimed.status === "Claimed", "claim did not become active");
  const artifact = JSON.stringify({ sdk: "typescript", autonomous: true });
  const submission = asObject(
    await client.submitResult(bountyId, {
      solver_agent_id: stringField(solver, "id"),
      artifact_uri: "memory://typescript-autonomous-smoke.json",
      artifact_body: artifact,
    }),
    "submission",
  );
  const proof = asObject(
    await client.requestVerification(bountyId, {
      submission_id: stringField(submission, "id"),
      expected_artifact_digest: await hashArtifact(artifact),
      verifier_kind: "JsonSchema",
    }),
    "proof",
  );
  requireCondition("proof_hash" in proof, "verification did not produce proof_hash");
  const status = asObject(await client.getBountyStatus(bountyId), "status");
  const statusBounty = asObject(status.bounty, "status.bounty");
  requireCondition(statusBounty.status === "Paid", "local loop did not reach Paid");

  return {
    sdk: "typescript",
    schema: discovery.schema,
    protocol: protocol.version,
    bounty_id: bountyId,
    status: statusBounty.status,
    autonomous_tools: tools.length,
  };
}

console.log(
  JSON.stringify(
    await exerciseSurface(new AgentBountiesClient({ baseUrl: baseUrlFromArgs() })),
    null,
    2,
  ),
);
