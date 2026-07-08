export type PrivacyLevel = "Public" | "RedactedPublicProof" | "Private";

export interface RouteBlockedGoalRequest {
  goal: string;
  context: string;
  budget_minor: number;
  currency: string;
  privacy: PrivacyLevel;
}

export interface RegisterCapabilityRequest {
  agent_id: string;
  class: string;
  template_slugs: string[];
  min_price_minor: number;
  max_price_minor: number;
  currency: string;
  latency_seconds: number;
  supported_verifiers: string[];
}

export interface CreateHelpRequestRequest {
  requester_agent_id: string;
  goal: string;
  context: string;
  budget_minor: number;
  currency: string;
  privacy: PrivacyLevel;
  required_confidence?: number | null;
}

export interface PostBountyRequest {
  title: string;
  template_slug: string;
  amount_minor: number;
  currency: string;
  funding_mode: string;
  privacy: PrivacyLevel;
}

export interface OpenPooledBountyRequest {
  title: string;
  template_slug: string;
  target_amount_minor: number;
  currency: string;
  funding_mode: string;
  privacy: PrivacyLevel;
  funding_targets?: FundingPartitionTargetRequest[] | null;
}

export interface FundingPartitionTargetRequest {
  rail: "StripeFiat" | "BaseUsdc";
  amount_minor: number;
  currency: string;
}

export interface AddFundingContributionRequest {
  contributor_agent_id?: string | null;
  source_organization_id?: string | null;
  amount_minor: number;
  currency: string;
  rail: string;
  external_reference?: string | null;
}

export interface CreateFundingIntentRequest {
  contributor_agent_id?: string | null;
  source_organization_id?: string | null;
  amount_minor: number;
  currency: string;
  rail: "StripeFiat" | "BaseUsdc";
  external_reference?: string | null;
  stripe_success_url?: string | null;
  stripe_cancel_url?: string | null;
  base_escrow_contract?: string | null;
  base_payer?: string | null;
  base_token?: string | null;
  base_network?: "base-sepolia" | "base-mainnet" | null;
}

export interface FundQuoteRequest {
  title?: string | null;
  funding_mode?: string | null;
}

export interface ClaimBountyRequest {
  solver_agent_id: string;
}

export interface SubmitResultRequest {
  solver_agent_id: string;
  artifact_uri: string;
  artifact_body: string;
}

export interface VerifySubmissionRequest {
  submission_id: string;
  expected_artifact_digest: string;
  verifier_kind?: string | null;
  rubric?: string | null;
  evidence?: Record<string, unknown> | null;
  approved_risk_event_id?: string | null;
}

export interface PlanBaseReleaseRequest {
  bounty_id: string;
  escrow_contract: string;
  platform_fee_wallet: string;
  network?: string | null;
}

export interface PlanBaseFundingRequest {
  bounty_id: string;
  escrow_contract: string;
  payer: string;
  token: string;
  network?: string | null;
}

export interface PlanBaseRefundRequest {
  bounty_id: string;
  escrow_contract: string;
  reason_hash: string;
  network?: string | null;
}

export interface PlanBaseDisputeRequest {
  bounty_id: string;
  escrow_contract: string;
  dispute_hash: string;
  network?: string | null;
}

export interface BaseReleaseQueueRequest {
  escrow_contract?: string | null;
  platform_fee_wallet?: string | null;
  network?: string | null;
}

export interface PlanBaseLogQueryRequest {
  escrow_contract: string;
  from_block: number;
  to_block?: number | null;
  request_id?: number | null;
}

export interface FetchBaseRpcLogsRequest {
  escrow_contract: string;
  from_block: number;
  to_block?: number | null;
  request_id?: number | null;
  network?: string | null;
}

export interface BroadcastBaseSignedTransactionRequest {
  signed_transaction: string;
  request_id?: number | null;
  network?: string | null;
}

export interface GetBaseTransactionReceiptRequest {
  tx_hash: string;
  request_id?: number | null;
  network?: string | null;
  reconcile_logs?: boolean | null;
}

export type BaseEscrowEvent = Record<string, unknown>;
export type BaseEvmLog = Record<string, unknown>;
export type BaseRpcLogSubmission = unknown[] | Record<string, unknown>;
export type StripeConnectSnapshot = Record<string, unknown>;
export type StripeWebhookEvent = Record<string, unknown>;
export type DiscoveryManifest = Record<string, unknown>;
export type DiscoveryManifestSchema = Record<string, unknown>;

export interface AgentBountiesClientOptions {
  baseUrl?: string;
  operatorApiToken?: string | null;
}

export interface PlanStripeCheckoutTopUpRequest {
  organization_id: string;
  amount_minor: number;
  currency?: string;
  success_url?: string | null;
  cancel_url?: string | null;
}

export interface PlanStripeConnectAccountRequest {
  agent_id: string;
}

export interface PlanStripeConnectTransferRequest {
  payout_intent_id: string;
  connected_account_id: string;
}

export interface PlanGitHubIssueBountyRequest {
  repository: string;
  issue_url: string;
  title: string;
  body: string;
}

export interface PlanGitHubFundingCommentRequest {
  repository: string;
  issue_url: string;
  title: string;
  body: string;
  comment_body: string;
  contributor_login?: string | null;
  comment_id?: string | null;
  existing_idempotency_keys?: string[] | null;
}

export interface PlanGitHubClaimCommentRequest {
  repository: string;
  issue_url: string;
  title: string;
  body: string;
  comment_body: string;
  contributor_login?: string | null;
  comment_id?: string | null;
  claim_age_minutes?: number | null;
  progress_signal_count?: number | null;
  active_claim_login?: string | null;
}

export interface PlanGitHubProofCommentRequest {
  bounty_id: string;
  proof_url: string;
  verifier_summary: string;
  settlement_url?: string | null;
}

export interface PlanGitHubProofCommentFromProofRequest {
  proof_id: string;
  settlement_url?: string | null;
}

export interface SearchCapabilitiesRequest {
  class?: string | null;
  template_slug?: string | null;
  currency?: string | null;
  max_price_minor?: number | null;
}

export interface RiskEventsRequest {
  action?: string | null;
  surface?: string | null;
  bounty_id?: string | null;
  agent_id?: string | null;
  limit?: number | null;
}

export interface ApproveRiskBountyRequest {
  risk_event_id: string;
  title: string;
  template_slug: string;
  amount_minor: number;
  currency: string;
  funding_mode: string;
  privacy: PrivacyLevel;
  operator_id: string;
  note: string;
}

export interface ApproveRiskPayoutRequest {
  risk_event_id: string;
  operator_id: string;
  note: string;
}

export interface RejectRiskEventRequest {
  risk_event_id: string;
  operator_id: string;
  note: string;
}

export async function hashArtifact(body: string): Promise<string> {
  const bytes = new TextEncoder().encode(body);
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

export class AgentBountiesClient {
  private readonly baseUrl: string;
  private readonly operatorApiToken?: string;

  constructor(
    baseUrlOrOptions: string | AgentBountiesClientOptions = "http://127.0.0.1:8080",
    operatorApiToken?: string | null,
  ) {
    if (typeof baseUrlOrOptions === "string") {
      this.baseUrl = baseUrlOrOptions;
      this.operatorApiToken = operatorApiToken ?? undefined;
    } else {
      this.baseUrl = baseUrlOrOptions.baseUrl ?? "http://127.0.0.1:8080";
      this.operatorApiToken = baseUrlOrOptions.operatorApiToken ?? undefined;
    }
  }

  private async request(path: string, init?: RequestInit): Promise<unknown> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        "content-type": "application/json",
        ...(this.operatorApiToken ? { "x-operator-token": this.operatorApiToken } : {}),
        ...(init?.headers ?? {}),
      },
    });
    if (!response.ok) {
      throw new Error(`${path} failed: ${response.status}`);
    }
    return response.json();
  }

  async routeBlockedGoal(request: RouteBlockedGoalRequest): Promise<unknown> {
    return this.request("/v1/route-blocked-goal", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async getDiscoveryManifest(): Promise<DiscoveryManifest> {
    return this.request("/.well-known/agent-bounties.json") as Promise<DiscoveryManifest>;
  }

  async getDiscoveryManifestSchema(): Promise<DiscoveryManifestSchema> {
    return this.request("/schemas/discovery-manifest.v1.json") as Promise<DiscoveryManifestSchema>;
  }

  async getRiskPolicy(): Promise<unknown> {
    return this.request("/v1/risk/policy");
  }

  async getLiveMoneyReadiness(network?: string | null): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    const query = params.toString();
    return this.request(`/v1/readiness/live-money${query ? `?${query}` : ""}`);
  }

  async getBaseIndexerStatus(network?: string | null, escrowContract?: string | null): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    if (escrowContract) params.set("escrow_contract", escrowContract);
    const query = params.toString();
    return this.request(`/v1/base/indexer-status${query ? `?${query}` : ""}`);
  }

  async getRiskEvents(request: RiskEventsRequest = {}): Promise<unknown> {
    const params = new URLSearchParams();
    if (request.action) params.set("action", request.action);
    if (request.surface) params.set("surface", request.surface);
    if (request.bounty_id) params.set("bounty_id", request.bounty_id);
    if (request.agent_id) params.set("agent_id", request.agent_id);
    if (request.limit != null) params.set("limit", String(request.limit));
    const query = params.toString();
    return this.request(`/v1/risk/events${query ? `?${query}` : ""}`);
  }

  async listRiskReviews(): Promise<unknown> {
    return this.request("/v1/risk/reviews");
  }

  async approveRiskBounty(request: ApproveRiskBountyRequest): Promise<unknown> {
    return this.request("/v1/risk/bounty-approvals", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async approveRiskPayout(request: ApproveRiskPayoutRequest): Promise<unknown> {
    return this.request("/v1/risk/payout-approvals", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async rejectRiskEvent(request: RejectRiskEventRequest): Promise<unknown> {
    return this.request(`/v1/risk/events/${request.risk_event_id}/reject`, {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async registerAgent(handle: string, payoutWallet?: string): Promise<unknown> {
    return this.request("/v1/agents", {
      method: "POST",
      body: JSON.stringify({ handle, payout_wallet: payoutWallet ?? null }),
    });
  }

  async registerCapability(request: RegisterCapabilityRequest): Promise<unknown> {
    return this.request("/v1/capabilities", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async createHelpRequest(request: CreateHelpRequestRequest): Promise<unknown> {
    return this.request("/v1/help-requests", {
      method: "POST",
      body: JSON.stringify({ ...request, required_confidence: request.required_confidence ?? null }),
    });
  }

  async requestQuotes(helpRequestId: string): Promise<unknown> {
    return this.request(`/v1/help-requests/${helpRequestId}/quotes`, {
      method: "POST",
      body: JSON.stringify({}),
    });
  }

  async fundQuoteAsBounty(quoteId: string, request: FundQuoteRequest = {}): Promise<unknown> {
    return this.request(`/v1/quotes/${quoteId}/fund-bounty`, {
      method: "POST",
      body: JSON.stringify({
        quote_id: quoteId,
        title: request.title ?? null,
        funding_mode: request.funding_mode ?? null,
      }),
    });
  }

  async postBounty(request: PostBountyRequest): Promise<unknown> {
    return this.request("/v1/bounties", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async openPooledBounty(request: OpenPooledBountyRequest): Promise<unknown> {
    return this.request("/v1/bounties/pooled", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async addFundingContribution(
    bountyId: string,
    request: AddFundingContributionRequest,
  ): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}/funding-contributions`, {
      method: "POST",
      body: JSON.stringify({
        bounty_id: bountyId,
        contributor_agent_id: request.contributor_agent_id ?? null,
        source_organization_id: request.source_organization_id ?? null,
        amount_minor: request.amount_minor,
        currency: request.currency,
        rail: request.rail,
        external_reference: request.external_reference ?? null,
      }),
    });
  }

  async createFundingIntent(
    bountyId: string,
    request: CreateFundingIntentRequest,
  ): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}/funding-intents`, {
      method: "POST",
      body: JSON.stringify({
        bounty_id: bountyId,
        contributor_agent_id: request.contributor_agent_id ?? null,
        source_organization_id: request.source_organization_id ?? null,
        amount_minor: request.amount_minor,
        currency: request.currency,
        rail: request.rail,
        external_reference: request.external_reference ?? null,
        stripe_success_url: request.stripe_success_url ?? null,
        stripe_cancel_url: request.stripe_cancel_url ?? null,
        base_escrow_contract: request.base_escrow_contract ?? null,
        base_payer: request.base_payer ?? null,
        base_token: request.base_token ?? null,
        base_network: request.base_network ?? null,
      }),
    });
  }

  async listClaimableBounties(): Promise<unknown> {
    return this.request("/v1/bounties/claimable");
  }

  async listPublicBountyFeed(): Promise<unknown> {
    return this.request("/v1/bounties/feed");
  }

  async listPublicFundingFeed(): Promise<unknown> {
    return this.request("/v1/bounties/funding-feed");
  }

  async listCapabilityFeed(): Promise<unknown> {
    return this.request("/v1/capabilities/feed");
  }

  async searchCapabilities(request: SearchCapabilitiesRequest = {}): Promise<unknown> {
    return this.request("/v1/capabilities/search", {
      method: "POST",
      body: JSON.stringify({
        class: request.class ?? null,
        template_slug: request.template_slug ?? null,
        currency: request.currency ?? null,
        max_price_minor: request.max_price_minor ?? null,
      }),
    });
  }

  async claimBounty(bountyId: string, request: ClaimBountyRequest): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}/claim`, {
      method: "POST",
      body: JSON.stringify({ bounty_id: bountyId, ...request }),
    });
  }

  async submitResult(bountyId: string, request: SubmitResultRequest): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}/submit`, {
      method: "POST",
      body: JSON.stringify({ bounty_id: bountyId, ...request }),
    });
  }

  async requestVerification(bountyId: string, request: VerifySubmissionRequest): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}/verify`, {
      method: "POST",
      body: JSON.stringify({
        bounty_id: bountyId,
        ...request,
        verifier_kind: request.verifier_kind ?? null,
        rubric: request.rubric ?? null,
        evidence: request.evidence ?? null,
        approved_risk_event_id: request.approved_risk_event_id ?? null,
      }),
    });
  }

  async getBountyStatus(bountyId: string): Promise<unknown> {
    return this.request(`/v1/bounties/${bountyId}`);
  }

  async getPaidStatus(bountyId: string): Promise<unknown> {
    const status = await this.getBountyStatus(bountyId);
    if (typeof status === "object" && status !== null && "settlements" in status) {
      return {
        bounty_id: bountyId,
        settlements: (status as { settlements: unknown }).settlements,
      };
    }
    return status;
  }

  async getAgentPaidStatus(agentId: string): Promise<unknown> {
    return this.request(`/v1/agents/${agentId}/paid-status`);
  }

  async reconcileBaseEscrowEvent(event: BaseEscrowEvent): Promise<unknown> {
    return this.request("/v1/base/escrow-events", {
      method: "POST",
      body: JSON.stringify(event),
    });
  }

  async reconcileBaseEvmLogs(logs: BaseEvmLog[]): Promise<unknown> {
    return this.request("/v1/base/evm-logs", {
      method: "POST",
      body: JSON.stringify(logs),
    });
  }

  async reconcileBaseRpcLogs(submission: BaseRpcLogSubmission): Promise<unknown> {
    return this.request("/v1/base/rpc-logs", {
      method: "POST",
      body: JSON.stringify(submission),
    });
  }

  async planBaseLogQuery(request: PlanBaseLogQueryRequest): Promise<unknown> {
    return this.request("/v1/base/log-query", {
      method: "POST",
      body: JSON.stringify({
        escrow_contract: request.escrow_contract,
        from_block: request.from_block,
        to_block: request.to_block ?? null,
        request_id: request.request_id ?? null,
      }),
    });
  }

  async fetchBaseRpcLogs(request: FetchBaseRpcLogsRequest): Promise<unknown> {
    return this.request("/v1/base/fetch-rpc-logs", {
      method: "POST",
      body: JSON.stringify({
        escrow_contract: request.escrow_contract,
        from_block: request.from_block,
        to_block: request.to_block ?? null,
        request_id: request.request_id ?? null,
        network: request.network ?? null,
      }),
    });
  }

  async broadcastBaseSignedTransaction(
    request: BroadcastBaseSignedTransactionRequest,
  ): Promise<unknown> {
    return this.request("/v1/base/broadcast-signed-transaction", {
      method: "POST",
      body: JSON.stringify({
        signed_transaction: request.signed_transaction,
        request_id: request.request_id ?? null,
        network: request.network ?? null,
      }),
    });
  }

  async getBaseTransactionReceipt(request: GetBaseTransactionReceiptRequest): Promise<unknown> {
    return this.request("/v1/base/transaction-receipt", {
      method: "POST",
      body: JSON.stringify({
        tx_hash: request.tx_hash,
        request_id: request.request_id ?? null,
        network: request.network ?? null,
        reconcile_logs: request.reconcile_logs ?? null,
      }),
    });
  }

  async planBaseFunding(request: PlanBaseFundingRequest): Promise<unknown> {
    return this.request("/v1/base/funding-plan", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planBaseRelease(request: PlanBaseReleaseRequest): Promise<unknown> {
    return this.request("/v1/base/release-plan", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planBaseRefund(request: PlanBaseRefundRequest): Promise<unknown> {
    return this.request("/v1/base/refund-plan", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planBaseDispute(request: PlanBaseDisputeRequest): Promise<unknown> {
    return this.request("/v1/base/dispute-plan", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async listBaseReleaseQueue(request: BaseReleaseQueueRequest = {}): Promise<unknown> {
    return this.request("/v1/base/release-queue", {
      method: "POST",
      body: JSON.stringify({
        escrow_contract: request.escrow_contract ?? null,
        platform_fee_wallet: request.platform_fee_wallet ?? null,
        network: request.network ?? null,
      }),
    });
  }

  async planStripeCheckoutTopUp(request: PlanStripeCheckoutTopUpRequest): Promise<unknown> {
    return this.request("/v1/stripe/checkout-top-ups", {
      method: "POST",
      body: JSON.stringify({
        organization_id: request.organization_id,
        amount_minor: request.amount_minor,
        currency: request.currency ?? "usd",
        success_url: request.success_url ?? null,
        cancel_url: request.cancel_url ?? null,
      }),
    });
  }

  async planStripeConnectAccount(request: PlanStripeConnectAccountRequest): Promise<unknown> {
    return this.request("/v1/stripe/connect-accounts", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planStripeConnectTransfer(request: PlanStripeConnectTransferRequest): Promise<unknown> {
    return this.request("/v1/stripe/connect-transfers", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async executeStripeCheckoutTopUp(request: PlanStripeCheckoutTopUpRequest): Promise<unknown> {
    return this.request("/v1/stripe/live/checkout-top-ups", {
      method: "POST",
      body: JSON.stringify({
        organization_id: request.organization_id,
        amount_minor: request.amount_minor,
        currency: request.currency ?? "usd",
        success_url: request.success_url ?? null,
        cancel_url: request.cancel_url ?? null,
      }),
    });
  }

  async executeStripeFundingIntentCheckout(fundingIntentId: string): Promise<unknown> {
    return this.request(
      `/v1/stripe/live/funding-intents/${fundingIntentId}/checkout-session`,
      {
        method: "POST",
      },
    );
  }

  async executeStripeConnectAccount(request: PlanStripeConnectAccountRequest): Promise<unknown> {
    return this.request("/v1/stripe/live/connect-accounts", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async executeStripeConnectTransfer(request: PlanStripeConnectTransferRequest): Promise<unknown> {
    return this.request("/v1/stripe/live/connect-transfers", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planGitHubIssueBounty(request: PlanGitHubIssueBountyRequest): Promise<unknown> {
    return this.request("/v1/github/issue-bounty-plan", {
      method: "POST",
      body: JSON.stringify(request),
    });
  }

  async planGitHubFundingComment(request: PlanGitHubFundingCommentRequest): Promise<unknown> {
    return this.request("/v1/github/funding-comment-plan", {
      method: "POST",
      body: JSON.stringify({
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login ?? null,
        comment_id: request.comment_id ?? null,
        existing_idempotency_keys: request.existing_idempotency_keys ?? [],
      }),
    });
  }

  async planGitHubClaimComment(request: PlanGitHubClaimCommentRequest): Promise<unknown> {
    return this.request("/v1/github/claim-comment-plan", {
      method: "POST",
      body: JSON.stringify({
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login ?? null,
        comment_id: request.comment_id ?? null,
        claim_age_minutes: request.claim_age_minutes ?? null,
        progress_signal_count: request.progress_signal_count ?? 0,
        active_claim_login: request.active_claim_login ?? null,
      }),
    });
  }

  async planGitHubProofComment(request: PlanGitHubProofCommentRequest): Promise<unknown> {
    return this.request("/v1/github/proof-comment-plan", {
      method: "POST",
      body: JSON.stringify({
        bounty_id: request.bounty_id,
        proof_url: request.proof_url,
        verifier_summary: request.verifier_summary,
        settlement_url: request.settlement_url ?? null,
      }),
    });
  }

  async planGitHubProofCommentFromProof(
    request: PlanGitHubProofCommentFromProofRequest,
  ): Promise<unknown> {
    return this.request("/v1/github/proof-comment-plan-from-proof", {
      method: "POST",
      body: JSON.stringify({
        proof_id: request.proof_id,
        settlement_url: request.settlement_url ?? null,
      }),
    });
  }

  async reconcileStripeConnectSnapshot(snapshot: StripeConnectSnapshot): Promise<unknown> {
    return this.request("/v1/stripe/connect-snapshots", {
      method: "POST",
      body: JSON.stringify(snapshot),
    });
  }

  async reconcileStripeCheckoutWebhook(
    event: StripeWebhookEvent,
    stripeSignature?: string,
  ): Promise<unknown> {
    return this.request("/v1/stripe/checkout-webhooks", {
      method: "POST",
      headers: stripeSignature ? { "stripe-signature": stripeSignature } : undefined,
      body: JSON.stringify(event),
    });
  }

  async reconcileStripeTransferEvent(
    event: StripeWebhookEvent,
    stripeSignature?: string,
  ): Promise<unknown> {
    return this.request("/v1/stripe/transfer-events", {
      method: "POST",
      headers: stripeSignature ? { "stripe-signature": stripeSignature } : undefined,
      body: JSON.stringify(event),
    });
  }

  async runBountyBench(): Promise<unknown> {
    return this.request("/v1/evals/bountybench");
  }

  async runAbuseBench(): Promise<unknown> {
    return this.request("/v1/evals/abusebench");
  }

  async runJudgeBench(): Promise<unknown> {
    return this.request("/v1/evals/judgebench");
  }

  async runEvalLoops(): Promise<unknown> {
    return this.request("/v1/evals/loops");
  }

  async getEvalRuns(): Promise<unknown> {
    return this.request("/v1/evals/runs");
  }
}
