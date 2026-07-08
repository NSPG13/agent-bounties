import { AgentBountiesClient, hashArtifact } from "./index.js";

declare const process: {
  argv: string[];
  env?: Record<string, string | undefined>;
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

function requireBaseIndexerStatusContract(value: JsonObject): void {
  requireCondition(
    typeof value.heartbeat_found === "boolean",
    "Base indexer status did not expose a heartbeat_found boolean",
  );
  requireCondition(
    value.worker_healthy === null || typeof value.worker_healthy === "boolean",
    "Base indexer status did not expose nullable worker_healthy",
  );
  for (const field of [
    "last_poll_status",
    "last_poll_started_at",
    "last_poll_completed_at",
    "last_poll_skipped_reason",
    "last_poll_error_message",
    "heartbeat_updated_at",
  ]) {
    const fieldValue = value[field];
    requireCondition(
      fieldValue === null || typeof fieldValue === "string",
      `Base indexer status did not expose nullable string field ${field}`,
    );
  }
  for (const field of [
    "last_poll_latest_block",
    "last_poll_confirmed_to_block",
    "last_poll_from_block",
    "last_poll_to_block",
    "last_poll_fetched_logs",
    "last_poll_persisted_cursor_block",
  ]) {
    const fieldValue = value[field];
    requireCondition(
      fieldValue === null || (typeof fieldValue === "number" && Number.isInteger(fieldValue)),
      `Base indexer status did not expose nullable numeric field ${field}`,
    );
  }
  const evidenceBoundaries = asArray(
    value.evidence_boundaries,
    "Base indexer evidence boundaries",
  );
  requireCondition(
    evidenceBoundaries.some(
      (boundary) => typeof boundary === "string" && boundary.includes("does not fund"),
    ),
    "Base indexer status did not explain that status evidence is not settlement",
  );
  requireCondition(
    evidenceBoundaries.some(
      (boundary) =>
        typeof boundary === "string"
        && boundary.includes("heartbeat proves only the last recorded poll outcome"),
    ),
    "Base indexer status did not explain the heartbeat evidence boundary",
  );
}

function baseUrlFromArgs(): string {
  const index = process.argv.indexOf("--base-url");
  if (index >= 0 && process.argv[index + 1]) {
    return process.argv[index + 1];
  }
  return "http://127.0.0.1:8080";
}

function operatorApiTokenFromArgs(): string | undefined {
  const index = process.argv.indexOf("--operator-api-token");
  if (index >= 0 && process.argv[index + 1]) {
    return process.argv[index + 1];
  }
  const token = process.env?.OPERATOR_API_TOKEN;
  return token && token.trim() ? token : undefined;
}

function githubCiEvidence(): JsonObject {
  return {
    repository: "example/repo",
    pull_request_url: "https://github.com/example/repo/pull/1",
    pull_request: {
      author_login: "solver-agent",
      merged: true,
      merged_by_login: "maintainer",
      reviews: [
        {
          author_login: "maintainer",
          state: "APPROVED",
        },
      ],
    },
    commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    check_run: {
      id: 123456789,
      name: "full-check",
      status: "completed",
      conclusion: "success",
      head_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      html_url: "https://github.com/example/repo/actions/runs/123456789",
      repository: { full_name: "example/repo" },
    },
  };
}

async function main(): Promise<void> {
  const client = new AgentBountiesClient({
    baseUrl: baseUrlFromArgs(),
    operatorApiToken: operatorApiTokenFromArgs(),
  });
  const suffix = `${Date.now()}-${Math.random().toString(16).slice(2)}`;

  const discovery = asObject(await client.getDiscoveryManifest(), "discovery");
  requireCondition("agent_entrypoints" in discovery, "discovery manifest missing agent entrypoints");
  requireCondition(
    discovery.schema === "https://agentbounties.org/schemas/discovery-manifest.v1.json",
    "discovery manifest missing expected schema id",
  );
  const endpoints = asObject(discovery.endpoints, "discovery.endpoints");
  requireCondition(
    typeof endpoints.llms_txt === "string",
    "discovery manifest missing llms.txt endpoint",
  );
  requireCondition(
    typeof endpoints.agent_quickstart === "string",
    "discovery manifest missing agent quickstart endpoint",
  );
  requireCondition(
    typeof endpoints.public_bounties === "string",
    "discovery manifest missing public bounty pages endpoint",
  );
  requireCondition(
    typeof endpoints.public_bounty === "string",
    "discovery manifest missing public bounty detail endpoint",
  );
  requireCondition(
    typeof endpoints.discovery_schema === "string",
    "discovery manifest missing schema endpoint",
  );
  const discoverySchema = asObject(
    await client.getDiscoveryManifestSchema(),
    "discoverySchema",
  );
  requireCondition(
    discoverySchema.$id === discovery.schema,
    "discovery schema id did not match manifest schema id",
  );
  const discoverySchemaRequired = asArray(discoverySchema.required, "discoverySchema.required");
  requireCondition(
    discoverySchemaRequired.includes("agent_entrypoints"),
    "discovery schema must require agent entrypoints",
  );
  requireCondition(
    discoverySchemaRequired.includes("payment_rails"),
    "discovery schema must require payment rails",
  );
  const discoverySchemaProperties = asObject(
    discoverySchema.properties,
    "discoverySchema.properties",
  );
  const endpointSchema = asObject(
    discoverySchemaProperties.endpoints,
    "discoverySchema.properties.endpoints",
  );
  const endpointRequired = asArray(
    endpointSchema.required,
    "discoverySchema.properties.endpoints.required",
  );
  requireCondition(
    endpointRequired.includes("discovery_schema"),
    "discovery schema must require its own endpoint",
  );
  requireCondition(
    endpointRequired.includes("github_issue_template"),
    "discovery schema must require the GitHub bounty issue template endpoint",
  );
  requireCondition(
    endpointRequired.includes("agent_quickstart"),
    "discovery schema must require the agent quickstart endpoint",
  );
  requireCondition(
    endpointRequired.includes("public_bounties"),
    "discovery schema must require the public bounty pages endpoint",
  );
  requireCondition(
    endpointRequired.includes("public_bounty"),
    "discovery schema must require the public bounty detail endpoint",
  );
  requireCondition(
    endpointRequired.includes("github_proof_comment_from_proof_plan"),
    "discovery schema must require the proof-record GitHub proof comment planner endpoint",
  );
  requireCondition(
    endpointRequired.includes("github_funding_comment_plan"),
    "discovery schema must require the GitHub funding comment planner endpoint",
  );
  requireCondition(
    endpointRequired.includes("github_claim_comment_plan"),
    "discovery schema must require the GitHub claim comment planner endpoint",
  );
  requireCondition(
    endpointRequired.includes("base_escrow_events"),
    "discovery schema must require the Base escrow event endpoint",
  );
  requireCondition(
    endpointRequired.includes("live_money_readiness"),
    "discovery schema must require the live-money readiness endpoint",
  );
  requireCondition(
    endpointRequired.includes("base_indexer_status"),
    "discovery schema must require the Base indexer status endpoint",
  );
  requireCondition(
    typeof endpoints.base_fetch_rpc_logs === "string",
    "discovery manifest missing Base RPC fetch endpoint",
  );
  requireCondition(
    typeof endpoints.base_escrow_events === "string",
    "discovery manifest missing Base escrow event reconciliation endpoint",
  );
  requireCondition(
    typeof endpoints.base_broadcast_signed_transaction === "string",
    "discovery manifest missing Base signed transaction broadcast endpoint",
  );
  requireCondition(
    typeof endpoints.base_transaction_receipt === "string",
    "discovery manifest missing Base transaction receipt endpoint",
  );
  requireCondition(
    typeof endpoints.base_funding_plan === "string",
    "discovery manifest missing Base funding planning endpoint",
  );
  requireCondition(
    typeof endpoints.base_refund_plan === "string",
    "discovery manifest missing Base refund planning endpoint",
  );
  requireCondition(
    typeof endpoints.base_dispute_plan === "string",
    "discovery manifest missing Base dispute planning endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_live_checkout_top_ups === "string",
    "discovery manifest missing live Stripe Checkout execution endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_live_funding_intent_checkouts === "string",
    "discovery manifest missing live Stripe funding-intent Checkout endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_connect_transfers === "string",
    "discovery manifest missing Stripe Connect transfer planner endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_connect_snapshots === "string",
    "discovery manifest missing Stripe Connect snapshot reconciliation endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_live_connect_accounts === "string",
    "discovery manifest missing live Stripe Connect execution endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_live_connect_transfers === "string",
    "discovery manifest missing live Stripe Connect transfer execution endpoint",
  );
  requireCondition(
    typeof endpoints.stripe_transfer_events === "string",
    "discovery manifest missing Stripe transfer event reconciliation endpoint",
  );
  requireCondition(
    typeof endpoints.github_issue_bounty_plan === "string",
    "discovery manifest missing GitHub issue bounty planner endpoint",
  );
  requireCondition(
    typeof endpoints.github_funding_comment_plan === "string",
    "discovery manifest missing GitHub funding comment planner endpoint",
  );
  requireCondition(
    typeof endpoints.github_claim_comment_plan === "string",
    "discovery manifest missing GitHub claim comment planner endpoint",
  );
  requireCondition(
    typeof endpoints.github_proof_comment_plan === "string",
    "discovery manifest missing GitHub proof comment planner endpoint",
  );
  requireCondition(
    typeof endpoints.github_proof_comment_from_proof_plan === "string",
    "discovery manifest missing proof-record GitHub proof comment planner endpoint",
  );
  requireCondition(
    typeof endpoints.eval_runs === "string",
    "discovery manifest missing eval run history endpoint",
  );
  requireCondition(
    typeof endpoints.risk_policy === "string",
    "discovery manifest missing risk policy endpoint",
  );
  requireCondition(
    typeof endpoints.live_money_readiness === "string",
    "discovery manifest missing live-money readiness endpoint",
  );
  requireCondition(
    typeof endpoints.base_indexer_status === "string",
    "discovery manifest missing Base indexer status endpoint",
  );
  requireCondition(
    typeof endpoints.risk_events === "string",
    "discovery manifest missing risk review events endpoint",
  );
  requireCondition(
    typeof endpoints.risk_reviews === "string",
    "discovery manifest missing risk review records endpoint",
  );
  requireCondition(
    typeof endpoints.risk_bounty_approvals === "string",
    "discovery manifest missing risk bounty approval endpoint",
  );
  requireCondition(
    typeof endpoints.risk_payout_approvals === "string",
    "discovery manifest missing risk payout approval endpoint",
  );
  requireCondition(
    typeof endpoints.agent_paid_status === "string",
    "discovery manifest missing agent payout status endpoint",
  );
  requireCondition(
    typeof endpoints.bounty_funding_intents === "string",
    "discovery manifest missing bounty funding intent endpoint",
  );

  const riskPolicy = asObject(await client.getRiskPolicy(), "riskPolicy");
  requireCondition(
    riskPolicy.low_value_usdc_cap_minor === 10_000_000,
    "risk policy did not expose the low-value Base USDC cap",
  );
  requireCondition(
    riskPolicy.ai_judges_can_authorize_payment === false,
    "risk policy must state that AI judges cannot authorize payment",
  );
  const liveMoneyReadiness = asObject(
    await client.getLiveMoneyReadiness("base-mainnet"),
    "liveMoneyReadiness",
  );
  requireCondition(
    liveMoneyReadiness.network_chain_id === 8453,
    "live-money readiness did not expose Base mainnet chain id",
  );
  requireCondition(
    String(liveMoneyReadiness.network_native_usdc_token_address).toLowerCase()
      === "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    "live-money readiness did not expose native Base USDC",
  );
  requireCondition(
    typeof liveMoneyReadiness.live_money_ready === "boolean",
    "live-money readiness did not expose a boolean live_money_ready gate",
  );
  requireCondition(
    !String(liveMoneyReadiness.stripe_secret_key_mode).startsWith("sk_")
      && !String(liveMoneyReadiness.stripe_secret_key_mode).startsWith("rk_"),
    "live-money readiness exposed Stripe secret material",
  );
  const baseIndexerStatus = asObject(
    await client.getBaseIndexerStatus("base-mainnet"),
    "baseIndexerStatus",
  );
  requireCondition(
    baseIndexerStatus.network_chain_id === 8453,
    "Base indexer status did not expose Base mainnet chain id",
  );
  requireCondition(
    typeof baseIndexerStatus.indexer_ready === "boolean",
    "Base indexer status did not expose an indexer_ready boolean",
  );
  requireBaseIndexerStatusContract(baseIndexerStatus);
  let reviewRequired = false;
  try {
    await client.postBounty({
      title: `TypeScript SDK review-required bounty ${suffix}`,
      template_slug: "fix-ci-failure",
      amount_minor: 25_000_000,
      currency: "usdc",
      funding_mode: "BaseUsdcEscrow",
      privacy: "Public",
    });
  } catch (error) {
    reviewRequired = error instanceof Error && error.message.includes("400");
  }
  requireCondition(reviewRequired, "over-cap Base USDC bounty should require review");
  const riskEvents = asArray(
    await client.getRiskEvents({ action: "NeedsReview", surface: "Bounty", limit: 10 }),
    "riskEvents",
  ).map((event) => asObject(event, "risk event"));
  const reviewEvent = riskEvents.find((event) => {
    const reasons = asArray(event.reasons, "risk event reasons");
    return (
      event.action === "NeedsReview" &&
      reasons.some((reason) => typeof reason === "string" && reason.includes("low-value cap"))
    );
  });
  if (reviewEvent === undefined) {
    throw new Error("risk events did not include the review-required bounty");
  }
  const reviewedApproval = asObject(
    await client.approveRiskBounty({
      risk_event_id: stringField(reviewEvent, "id"),
      title: `TypeScript SDK review-required bounty ${suffix}`,
      template_slug: "fix-ci-failure",
      amount_minor: 25_000_000,
      currency: "usdc",
      funding_mode: "BaseUsdcEscrow",
      privacy: "Public",
      operator_id: "typescript-sdk-smoke",
      note: "Approved review-required bounty during TypeScript SDK smoke.",
    }),
    "reviewedApproval",
  );
  const reviewedBounty = asObject(reviewedApproval.bounty, "reviewedApproval.bounty");
  requireCondition(
    reviewedBounty.status === "Unfunded",
    "risk approval did not create a funding-ready bounty",
  );
  const reviewedReview = asObject(reviewedApproval.review, "reviewedApproval.review");
  requireCondition(reviewedReview.outcome === "Approved", "risk approval did not record review");
  const riskReviews = asArray(await client.listRiskReviews(), "riskReviews").map((review) =>
    asObject(review, "risk review"),
  );
  requireCondition(
    riskReviews.some(
      (review) => review.outcome === "Approved" && review.bounty_id === reviewedBounty.id,
    ),
    "risk review list did not include approval",
  );

  const reviewSolver = asObject(
    await client.registerAgent(
      `typescript-sdk-review-solver-${suffix}`,
      "0x2222222222222222222222222222222222222222",
    ),
    "reviewSolver",
  );
  const reviewedBountyId = stringField(reviewedBounty, "id");
  const reviewedCreatedEvent = {
    id: crypto.randomUUID(),
    log_key: `typescript-sdk-review:${reviewedBountyId}:created`,
    tx_hash: `0x${crypto.randomUUID().replaceAll("-", "")}`,
    block_number: 2,
    onchain_escrow_id: 2,
    bounty_id: reviewedBountyId,
    kind: "Created",
    status: "Funded",
    token: "0x3333333333333333333333333333333333333333",
    amount: { amount: 25_000_000, currency: "usdc" },
    terms_hash: stringField(reviewedBounty, "terms_hash"),
    proof_hash: null,
    reason_hash: null,
    dispute_hash: null,
    occurred_at: new Date().toISOString(),
  };
  const reviewedFunding = asObject(
    await client.reconcileBaseEscrowEvent(reviewedCreatedEvent),
    "reviewedFunding",
  );
  requireCondition(
    asObject(reviewedFunding.bounty, "reviewedFunding.bounty").status === "Claimable",
    "reviewed Base escrow create event did not make bounty claimable",
  );
  const reviewedClaim = asObject(
    await client.claimBounty(reviewedBountyId, {
      solver_agent_id: stringField(reviewSolver, "id"),
    }),
    "reviewedClaim",
  );
  requireCondition(
    reviewedClaim.status === "Claimed",
    "reviewed bounty claim did not move to Claimed",
  );
  const reviewedSubmission = asObject(
    await client.submitResult(reviewedBountyId, {
      solver_agent_id: stringField(reviewSolver, "id"),
      artifact_uri: "https://github.com/example/repo/pull/1",
      artifact_body: JSON.stringify({ check: "green" }),
    }),
    "reviewedSubmission",
  );
  const reviewedEvidence = githubCiEvidence();
  try {
    await client.requestVerification(reviewedBountyId, {
      submission_id: stringField(reviewedSubmission, "id"),
      expected_artifact_digest: "not-used-by-github-ci",
      evidence: reviewedEvidence,
    });
    throw new Error("high-value payout should require review before verification");
  } catch (error) {
    if (!(error instanceof Error) || !error.message.includes("400")) {
      throw error;
    }
  }
  const payoutEvents = asArray(
    await client.getRiskEvents({
      action: "NeedsReview",
      surface: "Payout",
      bounty_id: reviewedBountyId,
      limit: 10,
    }),
    "payoutEvents",
  ).map((event) => asObject(event, "payout event"));
  const payoutEvent = payoutEvents.find((event) => {
    const reasons = asArray(event.reasons, "payout event reasons");
    return (
      event.action === "NeedsReview" &&
      reasons.some(
        (reason) => typeof reason === "string" && reason.includes("automatic release cap"),
      )
    );
  });
  if (payoutEvent === undefined) {
    throw new Error("payout risk event was not recorded");
  }
  const payoutReview = asObject(
    await client.approveRiskPayout({
      risk_event_id: stringField(payoutEvent, "id"),
      operator_id: "typescript-sdk-smoke",
      note: "Approved payout review during TypeScript SDK smoke.",
    }),
    "payoutReview",
  );
  requireCondition(payoutReview.surface === "Payout", "payout approval used wrong surface");
  const reviewedProof = asObject(
    await client.requestVerification(reviewedBountyId, {
      submission_id: stringField(reviewedSubmission, "id"),
      expected_artifact_digest: "not-used-by-github-ci",
      evidence: reviewedEvidence,
      approved_risk_event_id: stringField(payoutEvent, "id"),
    }),
    "reviewedProof",
  );
  requireCondition("proof_hash" in reviewedProof, "reviewed payout verification missing proof");
  const reviewedStatus = asObject(await client.getBountyStatus(reviewedBountyId), "reviewedStatus");
  const reviewedStatusBounty = asObject(reviewedStatus.bounty, "reviewedStatus.bounty");
  requireCondition(
    reviewedStatusBounty.status === "Payable",
    "reviewed payout bounty is not Payable",
  );

  const route = asObject(
    await client.routeBlockedGoal({
      goal: "Patch the TypeScript SDK live smoke bounty flow",
      context: "The agent needs a small coding task with deterministic verification.",
      budget_minor: 1_000_000,
      currency: "usdc",
      privacy: "Public",
    }),
    "route",
  );
  requireCondition("capability_class" in route, "route response missing capability_class");
  const capabilityClass = stringField(route, "capability_class");
  const templateSlug =
    typeof route.template_slug === "string" ? route.template_slug : "small-code-change";

  const requester = asObject(
    await client.registerAgent(`typescript-sdk-requester-${suffix}`),
    "requester",
  );
  const solver = asObject(
    await client.registerAgent(
      `typescript-sdk-solver-${suffix}`,
      "0x2222222222222222222222222222222222222222",
    ),
    "solver",
  );
  const stripeCheckout = asObject(
    await client.planStripeCheckoutTopUp({
      organization_id: stringField(requester, "id"),
      amount_minor: 5_000,
    }),
    "stripeCheckout",
  );
  requireCondition(
    stripeCheckout.endpoint === "/v1/checkout/sessions",
    "Stripe Checkout top-up planner used the wrong endpoint",
  );
  const stripeConnect = asObject(
    await client.planStripeConnectAccount({ agent_id: stringField(solver, "id") }),
    "stripeConnect",
  );
  const stripeConnectRequest = asObject(stripeConnect.request, "stripeConnect.request");
  requireCondition(
    stripeConnectRequest.endpoint === "/v2/core/accounts",
    "Stripe Connect account planner used the wrong endpoint",
  );
  let unknownTransferRejected = false;
  try {
    await client.planStripeConnectTransfer({
      payout_intent_id: crypto.randomUUID(),
      connected_account_id: "acct_test_sdk_smoke",
    });
  } catch (error) {
    unknownTransferRejected = error instanceof Error && error.message.includes("400");
  }
  requireCondition(
    unknownTransferRejected,
    "unknown Stripe transfer payout intent should return 400",
  );
  const githubIssuePlan = asObject(
    await client.planGitHubIssueBounty({
      repository: "agent-bounties/agent-bounties",
      issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1",
      title: "[bounty]: Fix CI",
      body:
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
    }),
    "githubIssuePlan",
  );
  requireCondition(githubIssuePlan.ready === true, "GitHub issue planner rejected valid issue");
  const githubIssueCheck = asObject(githubIssuePlan.check, "githubIssuePlan.check");
  requireCondition(
    githubIssueCheck.conclusion === "Success",
    "GitHub issue planner did not produce a success check",
  );
  const githubFundingPlan = asObject(
    await client.planGitHubFundingComment({
      repository: "agent-bounties/agent-bounties",
      issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1",
      title: "[bounty]: Fix CI",
      body:
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
      comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow",
      contributor_login: "typescript-sdk-smoke",
      comment_id: "12345",
    }),
    "githubFundingPlan",
  );
  requireCondition(
    githubFundingPlan.ready === true,
    "GitHub funding comment planner rejected valid signal",
  );
  const githubFundingSignal = asObject(githubFundingPlan.signal, "githubFundingPlan.signal");
  requireCondition(
    githubFundingSignal.requires_operator_reconciliation === true,
    "GitHub funding comment planner must require operator reconciliation",
  );
  const githubClaimPlan = asObject(
    await client.planGitHubClaimComment({
      repository: "agent-bounties/agent-bounties",
      issue_url: "https://github.com/agent-bounties/agent-bounties/issues/1",
      title: "[bounty]: Fix CI",
      body:
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n",
      comment_body: "/agent-bounty claim\nPlan: open a focused PR and run cargo test -p github-app.",
      contributor_login: "typescript-sdk-smoke",
      comment_id: "12346",
      claim_age_minutes: 5,
      progress_signal_count: 1,
    }),
    "githubClaimPlan",
  );
  requireCondition(
    githubClaimPlan.ready === true,
    "GitHub claim comment planner rejected progress-backed claim",
  );
  const githubClaimSignal = asObject(githubClaimPlan.signal, "githubClaimPlan.signal");
  requireCondition(
    githubClaimSignal.decision === "Reserved",
    "GitHub claim comment planner did not reserve progress-backed claim",
  );
  requireCondition(
    githubClaimSignal.settlement_authority === false,
    "GitHub claim comment planner must not authorize payment settlement",
  );
  requireCondition(
    stringField(asObject(githubClaimPlan.check, "githubClaimPlan.check"), "text").includes(
      "How did you find Agent Bounties?",
    ),
    "GitHub claim comment planner must carry the distribution feedback prompt",
  );
  const githubProofPlan = asObject(
    await client.planGitHubProofComment({
      bounty_id: stringField(solver, "id"),
      proof_url: "https://agentbounties.local/public/proofs/sdk-smoke",
      verifier_summary: "GitHub CI passed",
    }),
    "githubProofPlan",
  );
  requireCondition(
    stringField(githubProofPlan, "fingerprint").length === 64,
    "GitHub proof comment planner did not produce a stable fingerprint",
  );
  const baseLogQuery = asObject(
    await client.planBaseLogQuery({
      escrow_contract: "0x1111111111111111111111111111111111111111",
      from_block: 123,
      request_id: 11,
    }),
    "baseLogQuery",
  );
  requireCondition(baseLogQuery.method === "eth_getLogs", "Base log query used the wrong method");
  const baseLogQueryParams = asArray(baseLogQuery.params, "baseLogQuery.params").map((param) =>
    asObject(param, "baseLogQuery param"),
  );
  requireCondition(
    baseLogQueryParams[0].fromBlock === "0x7b",
    "Base log query did not encode fromBlock",
  );
  const baseRpcLogReport = asObject(
    await client.reconcileBaseRpcLogs({
      jsonrpc: "2.0",
      id: 11,
      result: [],
    }),
    "baseRpcLogReport",
  );
  requireCondition(
    baseRpcLogReport.decoded_events === 0,
    "Base RPC log reconciliation did not accept an empty provider response",
  );

  await client.registerCapability({
    agent_id: stringField(solver, "id"),
    class: capabilityClass,
    template_slugs: [templateSlug],
    min_price_minor: 500_000,
    max_price_minor: 1_000_000,
    currency: "usdc",
    latency_seconds: 600,
    supported_verifiers: ["JsonSchema"],
  });
  const capabilityFeed = asArray(await client.listCapabilityFeed(), "capabilityFeed").map((item) =>
    asObject(item, "capability feed item"),
  );
  requireCondition(
    capabilityFeed.some((item) => item.agent_id === stringField(solver, "id")),
    "registered solver missing from public capability feed",
  );
  const capabilitySearch = asArray(
    await client.searchCapabilities({
      class: capabilityClass,
      template_slug: templateSlug,
      currency: "usdc",
      max_price_minor: 1_000_000,
    }),
    "capabilitySearch",
  ).map((item) => asObject(item, "capability search item"));
  requireCondition(
    capabilitySearch.some((item) => item.agent_id === stringField(solver, "id")),
    "registered solver missing from filtered capability search",
  );

  const helpRequest = asObject(
    await client.createHelpRequest({
      requester_agent_id: stringField(requester, "id"),
      goal: "Patch the TypeScript SDK live smoke bounty flow",
      context: "Return a JSON artifact that proves the client can complete work.",
      budget_minor: 1_000_000,
      currency: "usdc",
      privacy: "Public",
    }),
    "helpRequest",
  );
  const quoteSet = asObject(await client.requestQuotes(stringField(helpRequest, "id")), "quoteSet");
  const quotes = asArray(quoteSet.quotes, "quotes").map((quote) => asObject(quote, "quote"));
  requireCondition(quotes.length >= 1, "quote flow did not return a solver quote");

  const bounty = asObject(
    await client.fundQuoteAsBounty(stringField(quotes[0], "id"), {
      title: "TypeScript SDK live smoke bounty",
      funding_mode: "BaseUsdcEscrow",
    }),
    "bounty",
  );
  const bountyId = stringField(bounty, "id");

  const fundingFeed = asArray(await client.listPublicFundingFeed(), "fundingFeed").map((item) =>
    asObject(item, "funding feed item"),
  );
  const fundingItem = fundingFeed.find((item) => item.bounty_id === bountyId);
  requireCondition(
    fundingItem !== undefined,
    "unfunded Base SDK bounty missing from funding feed",
  );
  const fundingExamples = asArray(
    asObject(fundingItem, "fundingItem").funding_intent_examples,
    "fundingIntentExamples",
  ).map((item) => asObject(item, "funding intent example"));
  requireCondition(
    fundingExamples.some((example) => {
      const requestBody = asObject(example.request_body, "funding intent request body");
      return (
        example.rail === "BaseUsdc" &&
        requestBody.base_network === "base-sepolia" &&
        example.operator_reconciliation_required === true
      );
    }),
    "funding feed missing Base USDC funding intent example",
  );

  const fundingPlan = asObject(
    await client.planBaseFunding({
      bounty_id: bountyId,
      escrow_contract: "0x1111111111111111111111111111111111111111",
      payer: "0x2222222222222222222222222222222222222222",
      token: "0x3333333333333333333333333333333333333333",
      network: "base-mainnet",
    }),
    "fundingPlan",
  );
  requireCondition(
    asObject(fundingPlan.network, "fundingPlan.network").chain_id === 8_453,
    "Base funding plan did not honor explicit Base mainnet network",
  );
  requireCondition(
    asObject(fundingPlan.create, "fundingPlan.create").terms_hash === stringField(bounty, "terms_hash"),
    "Base funding plan did not use bounty terms hash",
  );
  requireCondition(
    asObject(asObject(fundingPlan.funding, "fundingPlan.funding").create_escrow, "fundingPlan.create_escrow")
      .function === "createEscrow(bytes32,address,uint256,bytes32)",
    "Base funding plan used the wrong createEscrow function",
  );

  const createdEvent = {
    id: crypto.randomUUID(),
    log_key: `typescript-sdk-smoke:${bountyId}:created`,
    tx_hash: `0x${crypto.randomUUID().replaceAll("-", "")}`,
    block_number: 1,
    onchain_escrow_id: 1,
    bounty_id: bountyId,
    kind: "Created",
    status: "Funded",
    token: "0x3333333333333333333333333333333333333333",
    amount: { amount: 1_000_000, currency: "usdc" },
    terms_hash: stringField(bounty, "terms_hash"),
    proof_hash: null,
    reason_hash: null,
    dispute_hash: null,
    occurred_at: new Date().toISOString(),
  };
  const escrowReconciliation = asObject(
    await client.reconcileBaseEscrowEvent(createdEvent),
    "escrowReconciliation",
  );
  requireCondition(
    asObject(escrowReconciliation.bounty, "escrowReconciliation.bounty").status === "Claimable",
    "Base escrow create event did not make bounty claimable",
  );
  requireCondition(
    asObject(escrowReconciliation.escrow, "escrowReconciliation.escrow").status === "Funded",
    "Base escrow create event did not produce funded escrow state",
  );

  const feed = asArray(await client.listPublicBountyFeed(), "feed").map((item) =>
    asObject(item, "feed item"),
  );
  requireCondition(
    feed.some((item) => item.bounty_id === bountyId),
    "funded SDK bounty missing from public feed",
  );

  const claimed = asObject(
    await client.claimBounty(bountyId, { solver_agent_id: stringField(solver, "id") }),
    "claimed",
  );
  requireCondition(claimed.status === "Claimed", "claim did not move bounty to Claimed");

  const artifactBody = JSON.stringify({ sdk: "typescript", ok: true });
  const submission = asObject(
    await client.submitResult(bountyId, {
      solver_agent_id: stringField(solver, "id"),
      artifact_uri: "s3://agent-bounties/typescript-sdk-smoke/artifact.json",
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
  const proofRecordPlan = asObject(
    await client.planGitHubProofCommentFromProof({
      proof_id: stringField(proof, "id"),
    }),
    "proofRecordPlan",
  );
  const proofRecordComment = asObject(proofRecordPlan.comment, "proofRecordPlan.comment");
  requireCondition(
    proofRecordComment.bounty_id === bountyId,
    "proof-record GitHub proof comment planner used the wrong bounty",
  );
  requireCondition(
    stringField(proofRecordComment, "proof_url").endsWith(`/public/proofs/${stringField(proof, "id")}`),
    "proof-record GitHub proof comment planner used the wrong proof URL",
  );
  requireCondition(
    stringField(proofRecordPlan, "fingerprint").length === 64,
    "proof-record GitHub proof comment planner did not produce a stable fingerprint",
  );

  const status = asObject(await client.getBountyStatus(bountyId), "status");
  const statusBounty = asObject(status.bounty, "status.bounty");
  requireCondition(statusBounty.status === "Payable", "verified bounty is not Payable");
  const paid = asObject(await client.getPaidStatus(bountyId), "paid");
  const settlements = asArray(paid.settlements, "paid.settlements");
  requireCondition(settlements.length >= 1, "paid status missing settlement records");
  const agentPaid = asObject(
    await client.getAgentPaidStatus(stringField(solver, "id")),
    "agentPaid",
  );
  const agentPayouts = asArray(agentPaid.payouts, "agentPaid.payouts");
  requireCondition(agentPayouts.length >= 1, "agent paid status missing payout lines");
  const agentTotals = asArray(agentPaid.totals, "agentPaid.totals").map((item) =>
    asObject(item, "agent paid total"),
  );
  requireCondition(
    agentTotals.some((total) => total.currency === "usdc" && total.pending_minor === 900_000),
    "agent paid status missing pending USDC total",
  );
  const releaseQueue = asArray(
    await client.listBaseReleaseQueue({
      escrow_contract: "0x1111111111111111111111111111111111111111",
      platform_fee_wallet: "0x4444444444444444444444444444444444444444",
    }),
    "releaseQueue",
  ).map((item) => asObject(item, "release queue item"));
  const releaseQueueItem = releaseQueue.find(
    (item) => asObject(item.bounty, "release queue bounty").id === bountyId,
  );
  if (releaseQueueItem === undefined) {
    throw new Error("Base release queue did not return the SDK smoke bounty");
  }
  requireCondition(releaseQueueItem.ready === true, "Base release queue did not become ready");
  const queueReleasePlan = asObject(
    releaseQueueItem.release_plan,
    "releaseQueue.release_plan",
  );
  requireCondition(
    asObject(queueReleasePlan.network, "releaseQueue.release_plan.network").chain_id === 84_532,
    "Base release queue did not default to Base Sepolia",
  );
  const releasePlan = asObject(
    await client.planBaseRelease({
      bounty_id: bountyId,
      escrow_contract: "0x1111111111111111111111111111111111111111",
      platform_fee_wallet: "0x4444444444444444444444444444444444444444",
      network: "base-mainnet",
    }),
    "releasePlan",
  );
  requireCondition(
    asObject(releasePlan.network, "releasePlan.network").chain_id === 8_453,
    "Base release plan did not honor explicit Base mainnet network",
  );
  requireCondition(
    asObject(releasePlan.transaction, "releasePlan.transaction").function ===
      "release(uint256,address[],uint256[],bytes32)",
    "Base release plan used the wrong transaction function",
  );
  const evalLoops = asObject(await client.runEvalLoops(), "evalLoops");
  requireCondition(evalLoops.passed === true, "eval loop suite did not pass");
  requireCondition(asArray(evalLoops.loops, "evalLoops.loops").length === 5, "eval loop count changed");
  const evalRuns = asArray(await client.getEvalRuns(), "evalRuns");
  requireCondition(
    evalRuns.some((run) => asObject(run, "evalRun").suite === "EvalLoops/all-v0"),
    "eval run history did not record EvalLoops/all-v0",
  );

  console.log(
    JSON.stringify(
      {
        sdk_smoke: "ok",
        language: "typescript",
        bounty_id: bountyId,
        status: statusBounty.status,
        settlements: settlements.length,
      },
      null,
      2,
    ),
  );
}

await main();
