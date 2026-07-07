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
}

export interface PlanBaseRefundRequest {
  bounty_id: string;
  escrow_contract: string;
  reason_hash: string;
}

export interface PlanBaseDisputeRequest {
  bounty_id: string;
  escrow_contract: string;
  dispute_hash: string;
}

export interface BaseReleaseQueueRequest {
  escrow_contract?: string | null;
  platform_fee_wallet?: string | null;
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

export interface PlanGitHubIssueBountyRequest {
  repository: string;
  issue_url: string;
  title: string;
  body: string;
}

export interface PlanGitHubProofCommentRequest {
  bounty_id: string;
  proof_url: string;
  verifier_summary: string;
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
  constructor(private readonly baseUrl = "http://127.0.0.1:8080") {}

  private async request(path: string, init?: RequestInit): Promise<unknown> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      ...init,
      headers: {
        "content-type": "application/json",
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

  async getRiskPolicy(): Promise<unknown> {
    return this.request("/v1/risk/policy");
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

  async listClaimableBounties(): Promise<unknown> {
    return this.request("/v1/bounties/claimable");
  }

  async listPublicBountyFeed(): Promise<unknown> {
    return this.request("/v1/bounties/feed");
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

  async executeStripeConnectAccount(request: PlanStripeConnectAccountRequest): Promise<unknown> {
    return this.request("/v1/stripe/live/connect-accounts", {
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
