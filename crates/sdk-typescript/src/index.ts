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
  child_acceptance_criteria: string[];
  verifier_module: string;
}

export interface AutonomousAuthorizationSignature {
  v: number;
  r: string;
  s: string;
}

export interface AgentNativeClaimRequest {
  idempotency_key?: string;
  network?: "base-mainnet" | "base-sepolia";
  bounty_contract: string;
  solver_wallet: string;
  agent_id?: string | null;
  request_bond_sponsorship?: boolean;
  wallet_signature?: string;
  signature?: AutonomousAuthorizationSignature;
  source?: string;
}

export interface AgentNativeClaimResponse {
  schema_version: string;
  candidate: Record<string, unknown> & { status?: string };
  waitlist_position?: number | null;
  claim_bond: string;
  sponsorship_requested: boolean;
  sponsorship_available: boolean;
  sponsorship_protocol?: "agent-bounties/atomic-claim-sponsor-v1" | null;
  sponsor_contract?: string | null;
  sponsorship?: Record<string, unknown> | null;
  signing_payload?: Record<string, unknown> | null;
  wallet_request?: {
    method: "eth_signTypedData_v4";
    params: [string, string];
  } | null;
  claim_transaction_hash?: string | null;
  canonical_event_id?: string | null;
  next_action: string;
  next_request?: Record<string, unknown> | null;
  browser_fallback_url: string;
  evidence_boundary: string;
}

export type AgentClaimSigner = (
  signingPayload: Record<string, unknown>,
  walletRequest?: NonNullable<AgentNativeClaimResponse["wallet_request"]>,
) => Promise<string | AutonomousAuthorizationSignature>;

export interface AgentClaimLoopOptions {
  pollIntervalMs?: number;
  timeoutMs?: number;
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

export type OpportunityWorkState =
  | "open"
  | "claimable"
  | "in_progress"
  | "submitted"
  | "completed";
export type OpportunityPaymentState = "none" | "seeking_funding" | "escrowed" | "paid";
export type OpportunitySourceType = "unfunded_offchain" | "legacy_bounty" | "canonical_base";
export type OpportunityView =
  | "recent"
  | "engineering"
  | "creative"
  | "urgent"
  | "seeking_funding"
  | "ready_to_earn";

export interface OpportunityQuery {
  network?: "base-mainnet" | "base-sepolia" | null;
  view?: OpportunityView | null;
  source_type?: OpportunitySourceType | null;
  work_state?: OpportunityWorkState | null;
  payment_state?: OpportunityPaymentState | null;
  limit?: number | null;
}

export interface OpportunityProjection extends Record<string, unknown> {
  schema_version: "agent-bounties/opportunity-projection-v1";
  generated_at: string;
  network: string;
  applied_view: OpportunityView | null;
  degraded: boolean;
  source_statuses: Array<Record<string, unknown>>;
  items: Array<Record<string, unknown>>;
}

export interface DiscoveryRewardFilter {
  amount: string;
  currency: string;
  unit: "base_units" | "minor_units";
  decimals: number;
}

export interface DiscoverySubscriptionFilters {
  skills?: string[];
  categories?: string[];
  minimum_committed_reward?: DiscoveryRewardFilter | null;
  work_states?: OpportunityWorkState[];
  payment_states?: OpportunityPaymentState[];
  verification_methods?: string[];
  source_types?: OpportunitySourceType[];
  deadline_within_hours?: number | null;
}

export interface DiscoverySubscription extends Record<string, unknown> {
  schema_version: "agent-bounties/discovery-subscription-v1";
  subscription_id: string;
  endpoint_url: string;
  event_types: Array<"opportunity_published" | "opportunity_state_changed">;
  filters: DiscoverySubscriptionFilters;
  enabled: boolean;
  created_at: string;
}

export interface CreatedDiscoverySubscription extends DiscoverySubscription {
  management_token: string;
  signing_secret: string;
}

export interface OpportunityConversionFunnel extends Record<string, unknown> {
  schema_version: "agent-bounties/opportunity-conversion-funnel-v1";
  window_hours: number;
  stages: Array<Record<string, unknown>>;
  rates: Array<Record<string, unknown>>;
  average_seconds_to_first_solution: number | null;
  average_seconds_creation_to_settlement: number | null;
  actors: Record<string, unknown>;
}

export interface SiteAnalyticsCurrentChannel extends Record<string, unknown> {
  source: string;
  campaign: string | null;
  referrer_host: string | null;
  visitors: number;
  sessions: number;
  page_views: number;
  opportunity_exposures: number;
  funded_bounty_clicks: number;
  canonical_posts_confirmed: number;
  funding_starts: number;
  claims_confirmed: number;
}

export interface SiteAnalyticsContext extends Record<string, unknown> {
  placement: string | null;
  variant: string | null;
  opportunity_class: string | null;
  events: number;
  sessions: number;
  visitors: number;
  opportunity_exposures: number;
  funded_bounty_clicks: number;
  claims_confirmed: number;
}

export interface SiteAnalyticsHost extends Record<string, unknown> {
  site_host: "unknown" | "bountyboard.global" | "agentbounties.app" | "localhost";
  events: number;
  visitors: number;
  sessions: number;
  page_views: number;
  market_views: number;
  opportunity_exposures: number;
  funded_bounty_clicks: number;
  canonical_posts_confirmed: number;
  funding_starts: number;
  claims_confirmed: number;
}

export interface SiteAnalyticsOrderedConversion extends Record<string, unknown> {
  metric: string;
  start_event: string;
  outcome_event: string;
  matching_scope: string;
  window_seconds: number | null;
  denominator_events: number;
  denominator_sessions: number;
  denominator_visitors: number;
  numerator_events: number;
  numerator_sessions: number;
  numerator_visitors: number;
  value: number | null;
}

export interface SiteAnalyticsReport extends Record<string, unknown> {
  schema_version:
    | "agent-bounties/site-analytics-v1"
    | "agent-bounties/site-analytics-v2";
  window_hours: number;
  window_started_at: string;
  generated_at: string;
  overview: {
    unique_visitors: number;
    returning_visitors: number;
    sessions: number;
    page_views: number;
    first_event_at: string | null;
    last_event_at: string | null;
  };
  event_counts: Array<Record<string, unknown>>;
  daily: Array<Record<string, unknown>>;
  channels: Array<Record<string, unknown>>;
  current_channels?: SiteAnalyticsCurrentChannel[];
  contexts?: SiteAnalyticsContext[];
  hosts?: SiteAnalyticsHost[];
  ordered_conversions?: SiteAnalyticsOrderedConversion[];
  rates: Array<Record<string, unknown>>;
  definitions: string[];
  evidence_boundary: string;
}

export type AdventurerRank = "F" | "E" | "D" | "C" | "B" | "A" | "S";

export interface GuildRankBand {
  rank: AdventurerRank;
  minimum_reputation_points: number;
}

export interface GuildCharter extends Record<string, unknown> {
  schema_version: "agent-bounties/guild-domain-v1";
  product_name: string;
  setting: "Global Guild Hall";
  participant_term: "adventurer";
  mission_term: "mission";
  ranks: GuildRankBand[];
  mission_difficulties: AdventurerRank[];
  default_access: string;
  poster_optional_eligibility: string[];
  mission_eligibility_publishing_available: boolean;
  party_mutations_available: boolean;
  trust_review_scale: string;
  trust_review_mutations_available: boolean;
  affiliation_requirements: string[];
  affiliation_verification_available: boolean;
  supported_bounty_promises: Array<"money" | "other_asset_promise">;
  other_asset_delivery_verification_available: boolean;
  enforcement_boundary: string;
  payment_evidence_boundary: string;
}

export interface GuildAdventurerProfile extends Record<string, unknown> {
  schema_version: "agent-bounties/guild-adventurer-profile-v1";
  agent_id: string;
  handle: string;
  reputation_points: number;
  adventurer_rank: AdventurerRank;
  accepted_bounties: number;
  trust_score: number | null;
  trust_review_count: number;
  affiliation_status: string;
  evidence_boundary: string;
}

export interface CloudBountyAnalysis extends Record<string, unknown> {
  schema_version: "agent-bounties/cloud-bounty-analysis-v1";
  terms_hash: string;
  required_skills: string[];
  hard_requirements: string[];
  deliverable_checklist: string[];
  evidence_checklist: string[];
  verification_risks: string[];
  ambiguous_requirements: string[];
  missing_information: string[];
  confidence: number;
}

export interface CloudObjectivePlanRequest {
  objective: string;
  context?: string | null;
  constraints?: string[];
  max_tasks?: number;
  solver_budget_usdc?: string | null;
  source_url?: string | null;
  idempotency_key?: string | null;
}

export interface CloudObjectiveTask extends Record<string, unknown> {
  task_id: string;
  title: string;
  goal: string;
  depends_on: string[];
  acceptance_criteria: string[];
  verifier: Record<string, unknown>;
  evidence_schema: Record<string, unknown>;
  effort_weight: number;
  suggested_solver_reward_usdc: string | null;
}

export interface CloudObjectivePlan extends Record<string, unknown> {
  schema_version: "agent-bounties/cloud-objective-plan-v1";
  provider: string;
  model: string;
  title: string;
  objective: string;
  success_definition: string;
  tasks: CloudObjectiveTask[];
  parallel_layers: string[][];
  solver_budget_usdc: string | null;
  execution_policy: Record<string, unknown>;
  verification_policy: Record<string, unknown>;
  settlement_policy: Record<string, unknown>;
  questions: string[];
  risk_flags: string[];
  next_action: string;
  evidence_boundary: string;
}

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

export type AgentWalletSigningCapability =
  | "eip712_typed_data"
  | "eip3009_receive_with_authorization"
  | "send_transaction"
  | "wallet_send_calls";

export type AgentWalletProfile =
  | "generic-evm"
  | "metamask-agent-wallet"
  | "circle-agent-wallet"
  | "cdp-server-wallet"
  | "privy-server-wallet";

export interface PrepareAgentToEarnRequest {
  network: "base-mainnet" | "base-sepolia";
  wallet_address: string;
  bounty_contract: string;
  claim_bond_base_units?: string | null;
  signing_capabilities: AgentWalletSigningCapability[];
  wallet_profile?: AgentWalletProfile | null;
  policy: {
    allowed_chain_ids: number[];
    allowed_contracts: string[];
    per_transaction_usdc_base_units: string;
    rolling_24h_usdc_base_units: string;
    human_approval_policy: "always" | "out_of_policy" | "never";
  };
}

export interface AgentWalletReadinessReport extends Record<string, unknown> {
  schema_version: "agent-bounties/agent-wallet-readiness-v1";
  ready: boolean;
  status: "ready" | "blocked";
  recommended_claim_path: "agent_native_claim" | "direct_wallet_claim_plan" | null;
  checks: Array<Record<string, unknown>>;
  next_actions: string[];
}

export interface AgentWalletReadinessProblem extends Record<string, unknown> {
  schema_version: "agent-bounties/agent-wallet-readiness-problem-v1";
  state: "failed";
  failed_transition: string;
  error: string;
  retryable: boolean;
  message: string;
  next_action: string;
}

export class AgentBountiesHttpError extends Error {
  readonly path: string;
  readonly status: number;
  readonly body: unknown;

  constructor(path: string, status: number, body: unknown) {
    super(`${path} failed: ${status}`);
    this.name = "AgentBountiesHttpError";
    this.path = path;
    this.status = status;
    this.body = body;
  }
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

function parseHttpBody(text: string, status: number): unknown {
  if (!text) return { error: `HTTP ${status}` };
  try {
    return JSON.parse(text) as unknown;
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
    const body = parseHttpBody(await response.text(), response.status);
    if (!response.ok) {
      throw new AgentBountiesHttpError(path, response.status, body);
    }
    return body;
  }

  private post(path: string, body?: unknown, headers?: HeadersInit): Promise<unknown> {
    return this.request(path, {
      method: "POST",
      ...(headers ? { headers } : {}),
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  }

  private queryPath(
    path: string,
    values: object,
  ): string {
    const params = new URLSearchParams();
    for (const [key, value] of Object.entries(values)) {
      if (value != null && value !== "") params.set(key, String(value));
    }
    const encoded = params.toString();
    return `${path}${encoded ? `?${encoded}` : ""}`;
  }

  private query(path: string, values: object): Promise<unknown> {
    return this.request(this.queryPath(path, values));
  }

  private async autonomousPost(action: string, body: Record<string, unknown>): Promise<unknown> {
    return this.post(`/v1/base/autonomous-bounties/${action}`, body);
  }

  async routeBlockedGoal(request: RouteBlockedGoalRequest): Promise<unknown> {
    return this.post("/v1/route-blocked-goal", request);
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

  async compileObjective(request: CloudObjectivePlanRequest): Promise<CloudObjectivePlan> {
    return this.post("/v1/cloud-agent/objective-plans", {
      constraints: [],
      max_tasks: 5,
      ...request,
    }) as Promise<CloudObjectivePlan>;
  }

  async requestX402BountyFunding(
    request: X402BountyFundingRequest,
  ): Promise<X402BountyFundingResponse> {
    const path = this.queryPath(`/v1/x402/base/bounties/${request.bounty_contract}/funding`, {
      network: request.network ?? "base-mainnet",
      amount: request.amount,
      relayer: request.relayer,
    });
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
    return this.query("/v1/readiness/live-money", { network });
  }

  async prepareAgentToEarn(
    request: PrepareAgentToEarnRequest,
  ): Promise<AgentWalletReadinessReport> {
    return this.post(
      "/v1/base/agent-wallet/readiness",
      request,
    ) as Promise<AgentWalletReadinessReport>;
  }

  async getRiskEvents(request: RiskEventsRequest = {}): Promise<unknown> {
    return this.query("/v1/risk/events", request);
  }

  async listRiskReviews(): Promise<unknown> {
    return this.request("/v1/risk/reviews");
  }

  async approveRiskBounty(request: ApproveRiskBountyRequest): Promise<unknown> {
    return this.post("/v1/risk/bounty-approvals", request);
  }

  async approveRiskPayout(request: ApproveRiskPayoutRequest): Promise<unknown> {
    return this.post("/v1/risk/payout-approvals", request);
  }

  async rejectRiskEvent(request: RejectRiskEventRequest): Promise<unknown> {
    return this.post(`/v1/risk/events/${request.risk_event_id}/reject`, request);
  }

  async registerAgent(handle: string, payoutWallet?: string): Promise<unknown> {
    return this.post("/v1/agents", { handle, payout_wallet: payoutWallet ?? null });
  }

  async registerCapability(request: RegisterCapabilityRequest): Promise<unknown> {
    return this.post("/v1/capabilities", request);
  }

  async createHelpRequest(request: CreateHelpRequestRequest): Promise<unknown> {
    return this.post("/v1/help-requests", {
      ...request,
      required_confidence: request.required_confidence ?? null,
    });
  }

  async requestQuotes(helpRequestId: string): Promise<unknown> {
    return this.post(`/v1/help-requests/${helpRequestId}/quotes`, {});
  }

  async fundQuoteAsBounty(quoteId: string, request: FundQuoteRequest = {}): Promise<unknown> {
    return this.post(`/v1/quotes/${quoteId}/fund-bounty`, {
      quote_id: quoteId,
      title: request.title ?? null,
      funding_mode: request.funding_mode ?? null,
    });
  }

  async postBounty(request: PostBountyRequest): Promise<unknown> {
    return this.post("/v1/bounties", request);
  }

  async openPooledBounty(request: OpenPooledBountyRequest): Promise<unknown> {
    return this.post("/v1/bounties/pooled", request);
  }

  async addFundingContribution(
    bountyId: string,
    request: AddFundingContributionRequest,
  ): Promise<unknown> {
    return this.post(`/v1/bounties/${bountyId}/funding-contributions`, {
      bounty_id: bountyId,
      contributor_agent_id: request.contributor_agent_id ?? null,
      source_organization_id: request.source_organization_id ?? null,
      amount_minor: request.amount_minor,
      currency: request.currency,
      rail: request.rail,
      external_reference: request.external_reference ?? null,
    });
  }

  async createFundingIntent(
    bountyId: string,
    request: CreateFundingIntentRequest,
  ): Promise<unknown> {
    return this.post(`/v1/bounties/${bountyId}/funding-intents`, {
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
    return this.post("/v1/capabilities/search", {
      class: request.class ?? null,
      template_slug: request.template_slug ?? null,
      currency: request.currency ?? null,
      max_price_minor: request.max_price_minor ?? null,
    });
  }

  async claimBounty(bountyId: string, request: ClaimBountyRequest): Promise<unknown> {
    return this.post(`/v1/bounties/${bountyId}/claim`, { bounty_id: bountyId, ...request });
  }

  async submitResult(bountyId: string, request: SubmitResultRequest): Promise<unknown> {
    return this.post(`/v1/bounties/${bountyId}/submit`, { bounty_id: bountyId, ...request });
  }

  async requestVerification(bountyId: string, request: VerifySubmissionRequest): Promise<unknown> {
    return this.post(`/v1/bounties/${bountyId}/verify`, {
      bounty_id: bountyId,
      ...request,
      verifier_kind: request.verifier_kind ?? null,
      rubric: request.rubric ?? null,
      evidence: request.evidence ?? null,
      approved_risk_event_id: request.approved_risk_event_id ?? null,
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
    return this.query(
      `/v1/base/autonomous-bounties/submission-evidence/${bountyContract}/${round}`,
      { network },
    );
  }

  async listAutonomousBounties(
    network?: string | null,
    claimableOnly?: boolean | null,
  ): Promise<unknown> {
    return this.query("/v1/base/autonomous-bounties/feed", {
      network,
      claimable_only: claimableOnly,
    });
  }

  async getSolverLeaderboard(
    network?: string | null,
    at?: string | null,
  ): Promise<unknown> {
    return this.query("/v1/base/autonomous-bounties/leaderboard", { network, at });
  }

  async listOpportunities(query: OpportunityQuery = {}): Promise<OpportunityProjection> {
    return this.query("/v1/opportunities", query) as Promise<OpportunityProjection>;
  }

  async createDiscoverySubscription(
    endpointUrl: string,
    filters: DiscoverySubscriptionFilters = {},
  ): Promise<CreatedDiscoverySubscription> {
    return this.post("/v1/discovery/subscriptions", {
      endpoint_url: endpointUrl,
      filters,
    }) as Promise<CreatedDiscoverySubscription>;
  }

  async getDiscoverySubscription(
    subscriptionId: string,
    managementToken: string,
  ): Promise<DiscoverySubscription> {
    return this.request(`/v1/discovery/subscriptions/${subscriptionId}`, {
      headers: { authorization: `Bearer ${managementToken}` },
    }) as Promise<DiscoverySubscription>;
  }

  async deleteDiscoverySubscription(
    subscriptionId: string,
    managementToken: string,
  ): Promise<void> {
    await this.request(`/v1/discovery/subscriptions/${subscriptionId}`, {
      method: "DELETE",
      headers: { authorization: `Bearer ${managementToken}` },
    });
  }

  async getOpportunityConversionFunnel(
    windowHours?: number | null,
  ): Promise<OpportunityConversionFunnel> {
    return this.query("/v1/opportunities/conversion-funnel", {
      window_hours: windowHours,
    }) as Promise<OpportunityConversionFunnel>;
  }

  async getSiteAnalytics(windowHours?: number | null): Promise<SiteAnalyticsReport> {
    return this.query("/v1/analytics/site", {
      window_hours: windowHours,
    }) as Promise<SiteAnalyticsReport>;
  }

  async getGuildCharter(): Promise<GuildCharter> {
    return this.request("/v1/guild/charter") as Promise<GuildCharter>;
  }

  async getGuildAdventurerProfile(agentId: string): Promise<GuildAdventurerProfile> {
    return this.request(`/v1/guild/adventurers/${encodeURIComponent(agentId)}`) as Promise<GuildAdventurerProfile>;
  }

  async analyzeBountyFit(
    bountyContract: string,
    network?: "base-mainnet" | "base-sepolia" | null,
  ): Promise<CloudBountyAnalysis> {
    return this.query(`/v1/base/autonomous-bounties/${bountyContract}/analysis`, {
      network,
    }) as Promise<CloudBountyAnalysis>;
  }

  async listAutonomousVerificationJobs(
    network?: string | null,
    verifier?: string | null,
  ): Promise<unknown> {
    return this.query("/v1/base/autonomous-bounties/verification-jobs", {
      network,
      verifier,
    });
  }

  async listAutonomousBountyEvents(
    network?: string | null,
    bountyId?: string | null,
  ): Promise<unknown> {
    return this.query("/v1/base/autonomous-bounties/events", {
      network,
      bounty_id: bountyId,
    });
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

  async agentNativeClaim(
    request: AgentNativeClaimRequest,
    signer?: AgentClaimSigner,
    options: AgentClaimLoopOptions = {},
  ): Promise<AgentNativeClaimResponse> {
    const body: AgentNativeClaimRequest = {
      ...request,
      idempotency_key: request.idempotency_key ?? `sdk-typescript-${globalThis.crypto.randomUUID()}`,
      network: request.network ?? "base-mainnet",
      request_bond_sponsorship: request.request_bond_sponsorship ?? false,
      source: request.source ?? "sdk-typescript",
    };
    let response = (await this.post(
      "/v1/base/autonomous-bounties/claims",
      body,
    )) as AgentNativeClaimResponse;
    if (!signer || !response.signing_payload) return response;

    const signature = await signer(response.signing_payload, response.wallet_request ?? undefined);
    if (typeof signature === "string") {
      if (!/^0x[0-9a-fA-F]{130}$/.test(signature)) {
        throw new Error("agent claim signer must return one 65-byte 0x-prefixed signature");
      }
      body.wallet_signature = signature;
    } else {
      if (!signature?.r || !signature?.s || !Number.isInteger(signature.v)) {
        throw new Error("agent claim signer must return a wallet signature or legacy v, r, and s");
      }
      body.signature = signature;
    }
    const deadline = Date.now() + (options.timeoutMs ?? 60_000);
    while (true) {
      response = (await this.post(
        "/v1/base/autonomous-bounties/claims",
        body,
      )) as AgentNativeClaimResponse;
      const status = response.candidate?.status;
      if (status === "claimed") {
        if (!response.canonical_event_id) {
          throw new Error("claimed response is missing canonical_event_id");
        }
        return response;
      }
      if (["failed", "superseded", "withdrawn"].includes(status ?? "")) {
        throw new Error(`agent claim ended in terminal state ${status}`);
      }
      if (status === "waitlisted") return response;
      if (Date.now() >= deadline) {
        throw new Error("agent claim timed out; replay the same idempotency key and signature");
      }
      await new Promise((resolve) => setTimeout(resolve, options.pollIntervalMs ?? 1_000));
    }
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
    return this.post("/v1/base/broadcast-signed-transaction", {
      signed_transaction: request.signed_transaction,
      request_id: request.request_id ?? null,
      network: request.network ?? null,
    });
  }

  async getBaseTransactionReceipt(request: GetBaseTransactionReceiptRequest): Promise<unknown> {
    return this.post("/v1/base/transaction-receipt", {
      tx_hash: request.tx_hash,
      request_id: request.request_id ?? null,
      network: request.network ?? null,
    });
  }

  async planStripeCheckoutTopUp(request: PlanStripeCheckoutTopUpRequest): Promise<unknown> {
    return this.post("/v1/stripe/checkout-top-ups", {
      organization_id: request.organization_id,
      amount_minor: request.amount_minor,
      currency: request.currency ?? "usd",
      success_url: request.success_url ?? null,
      cancel_url: request.cancel_url ?? null,
    });
  }

  async planStripeConnectAccount(request: PlanStripeConnectAccountRequest): Promise<unknown> {
    return this.post("/v1/stripe/connect-accounts", request);
  }

  async planStripeConnectTransfer(request: PlanStripeConnectTransferRequest): Promise<unknown> {
    return this.post("/v1/stripe/connect-transfers", request);
  }

  async executeStripeCheckoutTopUp(request: PlanStripeCheckoutTopUpRequest): Promise<unknown> {
    return this.post("/v1/stripe/live/checkout-top-ups", {
      organization_id: request.organization_id,
      amount_minor: request.amount_minor,
      currency: request.currency ?? "usd",
      success_url: request.success_url ?? null,
      cancel_url: request.cancel_url ?? null,
    });
  }

  async executeStripeFundingIntentCheckout(fundingIntentId: string): Promise<unknown> {
    return this.post(`/v1/stripe/live/funding-intents/${fundingIntentId}/checkout-session`);
  }

  async executeStripeConnectAccount(request: PlanStripeConnectAccountRequest): Promise<unknown> {
    return this.post("/v1/stripe/live/connect-accounts", request);
  }

  async executeStripeConnectTransfer(request: PlanStripeConnectTransferRequest): Promise<unknown> {
    return this.post("/v1/stripe/live/connect-transfers", request);
  }

  async planGitHubIssueBounty(request: PlanGitHubIssueBountyRequest): Promise<unknown> {
    return this.post("/v1/github/issue-bounty-plan", request);
  }

  async planGitHubFundingComment(request: PlanGitHubFundingCommentRequest): Promise<unknown> {
    return this.post("/v1/github/funding-comment-plan", {
      repository: request.repository,
      issue_url: request.issue_url,
      title: request.title,
      body: request.body,
      comment_body: request.comment_body,
      contributor_login: request.contributor_login ?? null,
      comment_id: request.comment_id ?? null,
      existing_idempotency_keys: request.existing_idempotency_keys ?? [],
    });
  }

  async planGitHubClaimComment(request: PlanGitHubClaimCommentRequest): Promise<unknown> {
    return this.post("/v1/github/claim-comment-plan", {
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
    });
  }

  async planGitHubProofComment(request: PlanGitHubProofCommentRequest): Promise<unknown> {
    return this.post("/v1/github/proof-comment-plan", {
      bounty_id: request.bounty_id,
      proof_url: request.proof_url,
      verifier_summary: request.verifier_summary,
      settlement_url: request.settlement_url ?? null,
    });
  }

  async planGitHubProofCommentFromProof(
    request: PlanGitHubProofCommentFromProofRequest,
  ): Promise<unknown> {
    return this.post("/v1/github/proof-comment-plan-from-proof", {
      proof_id: request.proof_id,
      settlement_url: request.settlement_url ?? null,
    });
  }

  async reconcileStripeConnectSnapshot(snapshot: StripeConnectSnapshot): Promise<unknown> {
    return this.post("/v1/stripe/connect-snapshots", snapshot);
  }

  async reconcileStripeCheckoutWebhook(
    event: StripeWebhookEvent,
    stripeSignature?: string,
  ): Promise<unknown> {
    return this.post(
      "/v1/stripe/checkout-webhooks",
      event,
      stripeSignature ? { "stripe-signature": stripeSignature } : undefined,
    );
  }

  async reconcileStripeTransferEvent(
    event: StripeWebhookEvent,
    stripeSignature?: string,
  ): Promise<unknown> {
    return this.post(
      "/v1/stripe/transfer-events",
      event,
      stripeSignature ? { "stripe-signature": stripeSignature } : undefined,
    );
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
