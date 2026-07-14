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

export interface BroadcastBaseSignedTransactionRequest {
  signed_transaction: string;
  request_id?: number | null;
  network?: string | null;
}

export interface GetBaseTransactionReceiptRequest {
  tx_hash: string;
  request_id?: number | null;
  network?: string | null;
}

export type AutonomousBountyCreate = Record<string, unknown>;
export type AutonomousBountyContribution = Record<string, unknown>;
export type AutonomousVerificationAttestation = Record<string, unknown>;
export type AutonomousSignedAttestation = Record<string, unknown>;
export type AutonomousEvmLog = Record<string, unknown>;

export interface CanonicalChildBountyTermsRequest {
  parent_bounty_id: string;
  parent_round: number;
  parent_solver: string;
  parent_solver_reward: { amount: number; currency: "usdc" };
  verifier_module: string;
}

export interface AutonomousAuthorizationSignature {
  v: number;
  r: string;
  s: string;
}

export interface AutonomousLifecycleRequest {
  bounty_contract: string;
  network?: string | null;
  caller?: string | null;
}

export type StripeConnectSnapshot = Record<string, unknown>;
export type StripeWebhookEvent = Record<string, unknown>;
export type DiscoveryManifest = Record<string, unknown>;
export type DiscoveryManifestSchema = Record<string, unknown>;

export interface X402BountyFundingRequest {
  bounty_contract: string;
  amount?: number | null;
  network?: "base-mainnet" | "base-sepolia" | null;
  relayer?: string | null;
  payment_signature?: string | null;
}

export interface X402BountyFundingResponse {
  status: 200 | 202 | 400 | 402 | 404 | 409 | 413 | 422 | 429 | 503;
  payment_required: string | null;
  payment_response: string | null;
  body: Record<string, unknown>;
}

export type X402PaymentSigner = (
  paymentRequired: string,
  challengeBody: Record<string, unknown>,
) => Promise<string>;

export interface X402FundingLoopOptions {
  pollIntervalMs?: number;
  timeoutMs?: number;
}

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

function x402RelayId(body: Record<string, unknown>): string | null {
  const relay = body.relay;
  if (relay && typeof relay === "object" && "id" in relay) {
    const id = (relay as { id?: unknown }).id;
    if (typeof id === "string") return id;
  }
  const statusUrl = body.statusUrl;
  if (typeof statusUrl === "string") {
    const id = statusUrl.split("/").filter(Boolean).pop();
    return id || null;
  }
  return null;
}

async function x402ResponseBody(response: Response): Promise<Record<string, unknown>> {
  const text = await response.text();
  if (!text) return { error: `HTTP ${response.status}` };
  try {
    const parsed = JSON.parse(text) as unknown;
    return parsed && typeof parsed === "object"
      ? (parsed as Record<string, unknown>)
      : { error: text };
  } catch {
    return { error: text };
  }
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

  private async autonomousPost(action: string, body: Record<string, unknown>): Promise<unknown> {
    return this.request(`/v1/base/autonomous-bounties/${action}`, {
      method: "POST",
      body: JSON.stringify(body),
    });
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
    return this.request("/schemas/discovery-manifest.v2.json") as Promise<DiscoveryManifestSchema>;
  }

  async getX402Discovery(): Promise<Record<string, unknown>> {
    return this.request("/.well-known/x402.json") as Promise<Record<string, unknown>>;
  }

  async requestX402BountyFunding(
    request: X402BountyFundingRequest,
  ): Promise<X402BountyFundingResponse> {
    const params = new URLSearchParams();
    params.set("network", request.network ?? "base-mainnet");
    if (request.amount != null) params.set("amount", String(request.amount));
    if (request.relayer) params.set("relayer", request.relayer);
    const path = `/v1/x402/base/bounties/${request.bounty_contract}/funding?${params.toString()}`;
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "GET",
      headers: {
        ...(this.operatorApiToken ? { "x-operator-token": this.operatorApiToken } : {}),
        ...(request.payment_signature
          ? { "PAYMENT-SIGNATURE": request.payment_signature }
          : {}),
      },
    });
    if (![200, 202, 400, 402, 404, 409, 413, 422, 429, 503].includes(response.status)) {
      throw new Error(`${path} failed: ${response.status}`);
    }
    return {
      status: response.status,
      payment_required: response.headers.get("PAYMENT-REQUIRED"),
      payment_response: response.headers.get("PAYMENT-RESPONSE"),
      body: await x402ResponseBody(response),
    } as X402BountyFundingResponse;
  }

  async getX402RelayStatus(relayId: string): Promise<X402BountyFundingResponse> {
    const path = `/v1/x402/base/relays/${relayId}`;
    const response = await fetch(`${this.baseUrl}${path}`, {
      headers: this.operatorApiToken
        ? { "x-operator-token": this.operatorApiToken }
        : undefined,
    });
    if (![200, 202, 404, 422, 503].includes(response.status)) {
      throw new Error(`${path} failed: ${response.status}`);
    }
    return {
      status: response.status,
      payment_required: response.headers.get("PAYMENT-REQUIRED"),
      payment_response: response.headers.get("PAYMENT-RESPONSE"),
      body: await x402ResponseBody(response),
    } as X402BountyFundingResponse;
  }

  async fundX402Bounty(
    request: Omit<X402BountyFundingRequest, "payment_signature">,
    signer: X402PaymentSigner,
    options: X402FundingLoopOptions = {},
  ): Promise<X402BountyFundingResponse> {
    const pollIntervalMs = options.pollIntervalMs ?? 1_000;
    const timeoutMs = options.timeoutMs ?? 60_000;
    const deadline = Date.now() + timeoutMs;
    const challenge = await this.requestX402BountyFunding(request);
    if (challenge.status !== 402 || !challenge.payment_required) {
      throw new Error("x402 funding endpoint did not return a signable PAYMENT-REQUIRED challenge");
    }
    const paymentSignature = await signer(challenge.payment_required, challenge.body);
    if (!paymentSignature) throw new Error("x402 signer returned an empty PAYMENT-SIGNATURE");

    let response = await this.requestX402BountyFunding({
      ...request,
      payment_signature: paymentSignature,
    });
    while (response.status !== 200) {
      if (
        [400, 402, 404, 409, 413, 422, 429].includes(response.status) ||
        Date.now() >= deadline
      ) {
        throw new Error(
          response.status === 402
            ? "x402 authorization expired or no longer matches the funding challenge"
            : response.status === 429
            ? "x402 hosted relay rolling quota is exhausted"
            : response.status === 422
            ? "x402 authorization failed without canonical funding"
            : [400, 404, 409, 413].includes(response.status)
            ? `x402 funding request was rejected with HTTP ${response.status}`
            : "x402 funding timed out before canonical confirmation",
        );
      }
      await new Promise((resolve) => setTimeout(resolve, pollIntervalMs));
      const relayId = x402RelayId(response.body);
      response = relayId
        ? await this.getX402RelayStatus(relayId)
        : await this.requestX402BountyFunding({
            ...request,
            payment_signature: paymentSignature,
          });
      if (response.status === 503) {
        response = await this.requestX402BountyFunding({
          ...request,
          payment_signature: paymentSignature,
        });
      }
    }
    if (!response.payment_response) {
      throw new Error("confirmed x402 funding is missing PAYMENT-RESPONSE");
    }
    return response;
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

  async publishAutonomousBountyTerms(
    creatorWallet: string,
    document: Record<string, unknown>,
  ): Promise<unknown> {
    return this.autonomousPost("terms", {
      creator_wallet: creatorWallet,
      document,
    });
  }

  async getAutonomousBountyTerms(termsHash: string): Promise<unknown> {
    return this.request(`/v1/base/autonomous-bounties/terms/${termsHash}`);
  }

  async publishAutonomousSubmissionEvidence(request: {
    network?: string | null;
    bounty_contract: string;
    bounty_id: string;
    round: number;
    solver_wallet: string;
    artifact_reference: string;
    evidence: Record<string, unknown>;
  }): Promise<unknown> {
    return this.autonomousPost("submission-evidence", {
      ...request,
      network: request.network ?? null,
    });
  }

  async getAutonomousSubmissionEvidence(
    bountyContract: string,
    round: number,
    network?: string | null,
  ): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    const query = params.toString();
    return this.request(
      `/v1/base/autonomous-bounties/submission-evidence/${bountyContract}/${round}${query ? `?${query}` : ""}`,
    );
  }

  async listAutonomousBounties(
    network?: string | null,
    claimableOnly?: boolean | null,
  ): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    if (claimableOnly != null) params.set("claimable_only", String(claimableOnly));
    const query = params.toString();
    return this.request(`/v1/base/autonomous-bounties/feed${query ? `?${query}` : ""}`);
  }

  async listAutonomousVerificationJobs(
    network?: string | null,
    verifier?: string | null,
  ): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    if (verifier) params.set("verifier", verifier);
    const query = params.toString();
    return this.request(
      `/v1/base/autonomous-bounties/verification-jobs${query ? `?${query}` : ""}`,
    );
  }

  async listAutonomousBountyEvents(
    network?: string | null,
    bountyId?: string | null,
  ): Promise<unknown> {
    const params = new URLSearchParams();
    if (network) params.set("network", network);
    if (bountyId) params.set("bounty_id", bountyId);
    const query = params.toString();
    return this.request(`/v1/base/autonomous-bounties/events${query ? `?${query}` : ""}`);
  }

  async decodeAutonomousBountyEvents(logs: AutonomousEvmLog[]): Promise<unknown> {
    return this.autonomousPost("decode-events", { logs });
  }

  async planAutonomousBountyCreation(
    create: AutonomousBountyCreate,
    network?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("creation-plan", { network: network ?? null, create });
  }

  async planAutonomousCanonicalChildTerms(
    request: CanonicalChildBountyTermsRequest,
  ): Promise<unknown> {
    return this.autonomousPost("canonical-child-terms-plan", { ...request });
  }

  async planAutonomousBountyAuthorizedCreation(
    create: AutonomousBountyCreate,
    signature: AutonomousAuthorizationSignature,
    network?: string | null,
    relayer?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("authorized-creation-plan", {
      network: network ?? null,
      create,
      signature,
      relayer: relayer ?? null,
    });
  }

  async planAutonomousBountyContribution(
    contribution: AutonomousBountyContribution,
    network?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("contribution-plan", {
      network: network ?? null,
      contribution,
    });
  }

  async planAutonomousBountyAuthorizedContribution(
    contribution: AutonomousBountyContribution,
    signature: AutonomousAuthorizationSignature,
    network?: string | null,
    relayer?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("authorized-contribution-plan", {
      network: network ?? null,
      contribution,
      signature,
      relayer: relayer ?? null,
    });
  }

  async planAutonomousBountyClaim(request: {
    network?: string | null;
    bounty_contract: string;
    solver: string;
    authorization_nonce?: string | null;
    authorization_valid_before?: number | null;
  }): Promise<unknown> {
    return this.autonomousPost("claim-plan", {
      ...request,
      network: request.network ?? null,
      authorization_nonce: request.authorization_nonce ?? null,
      authorization_valid_before: request.authorization_valid_before ?? null,
    });
  }

  async planAutonomousBountyAuthorizedClaim(request: {
    network?: string | null;
    bounty_contract: string;
    solver: string;
    authorization_nonce: string;
    authorization_valid_before: number;
    signature: AutonomousAuthorizationSignature;
    relayer?: string | null;
  }): Promise<unknown> {
    return this.autonomousPost("authorized-claim-plan", {
      ...request,
      network: request.network ?? null,
      relayer: request.relayer ?? null,
    });
  }

  async planAutonomousBountySubmission(request: {
    network?: string | null;
    bounty_contract: string;
    solver: string;
    submission_hash: string;
    evidence_hash: string;
  }): Promise<unknown> {
    return this.autonomousPost("submission-plan", {
      ...request,
      network: request.network ?? null,
    });
  }

  async prepareAutonomousBountySubmission(request: {
    network?: string | null;
    bounty_contract: string;
    solver_wallet: string;
    artifact_reference: string;
    evidence: Record<string, unknown>;
  }): Promise<unknown> {
    return this.autonomousPost("submission-preparation", {
      ...request,
      network: request.network ?? null,
    });
  }

  async planAutonomousBountySubmissionAuthorization(
    submission: {
      bounty_contract: string;
      bounty_id: string;
      round: number;
      solver: string;
      submission_hash: string;
      evidence_hash: string;
      policy_hash: string;
      deadline: number;
    },
    network?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("submission-authorization-plan", {
      network: network ?? null,
      submission,
    });
  }

  async planAutonomousVerificationAttestation(
    attestation: AutonomousVerificationAttestation,
    network?: string | null,
  ): Promise<unknown> {
    return this.autonomousPost("verification-attestation-plan", {
      network: network ?? null,
      attestation,
    });
  }

  async planAutonomousModuleSettlement(request: {
    network?: string | null;
    bounty_contract: string;
    caller?: string | null;
    proof: string;
  }): Promise<unknown> {
    return this.autonomousPost("module-settlement-plan", {
      ...request,
      network: request.network ?? null,
      caller: request.caller ?? null,
    });
  }

  async planAutonomousAttestationSettlement(request: {
    network?: string | null;
    bounty_contract: string;
    caller?: string | null;
    attestations: AutonomousSignedAttestation[];
  }): Promise<unknown> {
    return this.autonomousPost("attestation-settlement-plan", {
      ...request,
      network: request.network ?? null,
      caller: request.caller ?? null,
    });
  }

  private async planAutonomousLifecycle(
    action: string,
    request: AutonomousLifecycleRequest,
  ): Promise<unknown> {
    return this.autonomousPost(`${action}-plan`, {
      ...request,
      network: request.network ?? null,
      caller: request.caller ?? null,
    });
  }

  async planAutonomousExpireClaim(request: AutonomousLifecycleRequest): Promise<unknown> {
    return this.planAutonomousLifecycle("expire-claim", request);
  }

  async planAutonomousExpireSubmission(request: AutonomousLifecycleRequest): Promise<unknown> {
    return this.planAutonomousLifecycle("expire-submission", request);
  }

  async planAutonomousCancel(request: AutonomousLifecycleRequest): Promise<unknown> {
    return this.planAutonomousLifecycle("cancel", request);
  }

  async planAutonomousRefundWithdrawal(request: AutonomousLifecycleRequest): Promise<unknown> {
    return this.planAutonomousLifecycle("refund-withdrawal", request);
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
