import { keccak_256 } from "@noble/hashes/sha3";

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

export interface StandingMetaV4ReadinessCheck {
  name: string;
  ready: boolean;
  observed: string;
  required: string;
}

export interface OpenCompetitionReadinessCheck {
  name: string;
  ready: boolean;
  observed: string;
  required: string;
}

export interface OpenCompetitionReadinessReport {
  schema_version: "agent-bounties/open-competition-v1-readiness-v1";
  protocol_version: "open-competition-v1";
  competition_mode: "first_valid_submission";
  ready_to_compete: boolean;
  checks: OpenCompetitionReadinessCheck[];
  blockers: string[];
  first_means: string;
  ordering_authority: string;
  decision_authority: string;
  payment_authority: string;
  next_action: string;
  fairness_statement: string;
  evidence_boundary: string;
}

export type OpenCompetitionDeploymentState =
  | "source_only_not_ready_to_earn"
  | "sepolia_rehearsed_not_ready_to_earn"
  | "mainnet_canary_not_ready_to_earn"
  | "active_ready_to_earn";

export interface OpenCompetitionVerifierProfile {
  profile_id: string;
  protocol_version: "agent-bounties/open-competition-v1";
  network: "base-mainnet" | "base-sepolia";
  chain_id: number;
  display_name: string;
  module_kind: string;
  verifier_address: string;
  runtime_code_hash: string;
  configuration: Record<string, unknown>;
  benchmark_hash: string;
  evidence_schema_hash: string;
  evidence_schema: string;
  immutable_runtime_required: boolean;
  approved_for_rehearsal: boolean;
  public_inventory_eligible: boolean;
  deployment_state: OpenCompetitionDeploymentState;
  evidence_boundary: string;
}

export interface OpenCompetitionVerifierCatalog {
  schema_version: "agent-bounties/open-competition-v1-verifier-catalog-v1";
  protocol_version: "agent-bounties/open-competition-v1";
  network: "base-mainnet" | "base-sepolia";
  profiles: OpenCompetitionVerifierProfile[];
  evidence_boundary: string;
}

export interface OpenCompetitionCommitmentInput {
  network: "base-mainnet" | "base-sepolia";
  bounty: string;
  solver: string;
  submission_hash: string;
  evidence_hash: string;
}

export interface OpenCompetitionCommitmentEnvelope {
  schema_version: "agent-bounties/open-competition-v1-commitment-v1";
  network: "base-mainnet" | "base-sepolia";
  chain_id: number;
  bounty: string;
  solver: string;
  submission_hash: string;
  evidence_hash: string;
  salt: string;
  commitment: string;
  committed_block: number | null;
  reveal_deadline: number | null;
  evidence_boundary: string;
}

export interface OpenCompetitionCreateParams {
  solver_reward: number;
  verifier_reward: number;
  terms_hash: string;
  policy_hash: string;
  acceptance_criteria_hash: string;
  benchmark_hash: string;
  evidence_schema_hash: string;
  funding_deadline: number;
  competition_window_seconds: number;
  reveal_window_seconds: number;
  max_entries: number;
  verifier_reward_recipient: string;
}

export interface OpenCompetitionCreationRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  creator: string;
  creation_nonce: string;
  initial_funding: number;
  verifier_profile_id: string;
  params: OpenCompetitionCreateParams;
  funding_authorization?: Record<string, unknown> | null;
}

export interface OpenCompetitionCreationPlan {
  schema_version: "agent-bounties/open-competition-v1-creation-preparation-v1";
  protocol_version: "agent-bounties/open-competition-v1";
  network: "base-mainnet" | "base-sepolia";
  chain_id: number;
  funding_mode: "approval_then_create" | "eip3009_authorized";
  factory_contract: string;
  implementation_contract: string;
  creator: string;
  bounty_id: string;
  predicted_bounty_contract: string;
  verifier_profile_id: string;
  approve: Record<string, unknown> | null;
  create_competition: Record<string, unknown> | null;
  wallet_calls: Record<string, unknown>[];
  eip3009_authorization: Record<string, unknown> | null;
  ready_to_broadcast: boolean;
  public_inventory_eligible: boolean;
  next_action: string;
  evidence_boundary: string;
}

export interface OpenCompetitionSafeState {
  schema_version: "agent-bounties/open-competition-v1-state-v1";
  protocol_version: "agent-bounties/open-competition-v1";
  network: "base-mainnet" | "base-sepolia";
  chain_id: number;
  deployment_state: OpenCompetitionDeploymentState;
  safe_block_number: number;
  safe_block_hash: string;
  safe_block_timestamp: number;
  bounty_contract: string;
  onchain_ready_to_enter: boolean;
  public_inventory_eligible: boolean;
  blockers: string[];
  [key: string]: unknown;
}

export type OpenCompetitionOperation =
  | "prepare_open_competition_commit"
  | "prepare_open_competition_reveal"
  | "get_open_competition_status"
  | "withdraw_open_competition_bond";

export interface OpenCompetitionActionRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  bounty_contract: string;
  arguments: Record<string, unknown>;
}

export interface OpenCompetitionCommitRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  bounty_contract: string;
  solver: string;
  commitment: string;
}

export interface OpenCompetitionRevealRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  bounty_contract: string;
  solver: string;
  commitment_envelope: OpenCompetitionCommitmentEnvelope;
  proof: string;
}

export interface OpenCompetitionActionPlan {
  schema_version: "agent-bounties/open-competition-v1-action-v1";
  protocol_version: "open-competition-v1";
  competition_mode: "first_valid_submission";
  operation: OpenCompetitionOperation;
  allowed: boolean;
  target_contract: string | null;
  function: string | null;
  arguments: Record<string, unknown>;
  blocker: string | null;
  next_action: string;
  evidence_boundary: string;
}

export type OpenCompetitionEntrantAction = "commit" | "reveal" | "withdraw_bond";

export interface OpenCompetitionEntrantActionPreparationRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  wallet: string;
  bounty_contract: string;
  action: OpenCompetitionEntrantAction;
  commitment?: string | null;
  commitment_envelope?: OpenCompetitionCommitmentEnvelope | null;
  proof?: string | null;
  deadline_seconds?: number | null;
}

export interface OpenCompetitionEntrantActionPlan {
  schema_version: "agent-bounties/open-competition-entrant-wallet-action-v1";
  network: "base-mainnet" | "base-sepolia";
  chain_id: number;
  wallet: string;
  delegate: string;
  policy_hash: string;
  policy_version: number;
  action: OpenCompetitionEntrantAction;
  action_code: number;
  nonce: number;
  deadline: number;
  payload: string;
  payload_hash: string;
  signing_payload: Record<string, unknown>;
  relay_call: Record<string, unknown>;
  evidence_boundary: string;
}

export interface OpenCompetitionEntrantRelayRequest {
  idempotency_key: string;
  plan: OpenCompetitionEntrantActionPlan;
  signature: string;
}

export interface OpenCompetitionEntrantRelayResponse {
  schema_version: "agent-bounties/open-competition-entrant-relay-v1";
  id: string;
  network: "base-mainnet" | "base-sepolia";
  wallet: string;
  bounty_contract: string;
  action: number;
  wallet_nonce: number;
  status: "prepared" | "relaying" | "broadcast" | "confirmed" | "failed";
  retryable: boolean;
  transaction_hash: string | null;
  receipt_block: number | null;
  receipt_block_hash: string | null;
  canonical_safe_block: number | null;
  canonical_safe_block_hash: string | null;
  canonical_event: string | null;
  payment_proven: boolean;
  next_action: string;
  evidence_boundary: string;
}

export type OpenCompetitionV2Network = "base-mainnet" | "base-sepolia";
export type OpenCompetitionV2WinnerMode = "first_proven" | "best_score";
export type OpenCompetitionV2ScoreDirection = "higher_is_better" | "lower_is_better";
export type OpenCompetitionV2ProofSystem = "groth16" | "plonk";
export type OpenCompetitionV2MetricMode =
  | "all_equal"
  | "maximize_exact_matches"
  | "minimize_absolute_error";

export interface OpenCompetitionV2MetricCase {
  expected: number;
  observed: number;
  weight: number;
}

export interface OpenCompetitionV2MetricRequest {
  mode: OpenCompetitionV2MetricMode;
  threshold: string;
  vectors: OpenCompetitionV2MetricCase[];
}

export interface OpenCompetitionV2MetricScope {
  chain_id: number;
  competition: string;
  bounty_id: string;
  solver: string;
  solver_nonce: string;
  proof_system: OpenCompetitionV2ProofSystem;
  program_vkey: string;
  source_hash: string;
  elf_hash: string;
  execution_policy_hash: string;
  settlement_policy_hash: string;
  beta_risk_hash: string;
}

export interface OpenCompetitionV2PublicVectorInput extends OpenCompetitionV2MetricRequest {
  scope: OpenCompetitionV2MetricScope;
}

export interface OpenCompetitionV2PublicVectorResult {
  passed: boolean;
  score: string;
  verification_policy_hash: string;
  submission_hash: string;
  evidence_hash: string;
  journal_hex: string;
}

export interface OpenCompetitionV2CreateParams {
  solver_reward: string;
  keeper_reward: string;
  funding_deadline: number;
  proof_window_seconds: number;
  winner_mode: OpenCompetitionV2WinnerMode;
  score_direction: OpenCompetitionV2ScoreDirection;
  score_threshold: string;
  proof_system: OpenCompetitionV2ProofSystem;
  program_vkey: string;
  source_hash: string;
  elf_hash: string;
  journal_schema_hash: string;
  metric_program_hash: string;
  execution_policy_hash: string;
  verification_policy_hash: string;
  settlement_policy_hash: string;
  beta_risk_hash: string;
}

export interface OpenCompetitionV2CreationRequest {
  network?: OpenCompetitionV2Network | null;
  creator: string;
  creation_nonce: string;
  acknowledged_risk_hash: string;
  initial_funding: string;
  params: OpenCompetitionV2CreateParams;
}

export interface OpenCompetitionV2FundingRequest {
  network?: OpenCompetitionV2Network | null;
  contributor: string;
  competition_contract: string;
  amount: string;
  acknowledged_risk_hash: string;
}

export interface OpenCompetitionV2ProofQuoteRequest {
  network?: OpenCompetitionV2Network | null;
  competition_contract: string;
  solver: string;
  solver_nonce: string;
  artifact_hash: string;
  relay: boolean;
  metric: OpenCompetitionV2MetricRequest;
}

export interface OpenCompetitionV2ProofRequest {
  network?: OpenCompetitionV2Network | null;
  competition_contract: string;
  solver: string;
  solver_nonce: string;
  proof_system: OpenCompetitionV2ProofSystem;
  public_values: string;
  proof: string;
  authorization_deadline: number;
  solver_signature?: string | null;
}

export interface OpenCompetitionV2ActionRequest {
  network?: OpenCompetitionV2Network | null;
  competition_contract: string;
  caller?: string | null;
  action:
    | "finalize_best_score"
    | "cancel_funding"
    | "expire_competition"
    | "cancel_unavailable_gateway"
    | "withdraw_refund_for";
  contributor?: string | null;
}

export interface StandingMetaV4ReadinessReport {
  schema_version: "agent-bounties/standing-meta-v4-readiness-v1";
  protocol_version: "standing-meta-v4";
  ready_to_earn: boolean;
  successful_settlement_margin_base_units: string | null;
  checks: StandingMetaV4ReadinessCheck[];
  blockers: string[];
  next_action: string;
  decision_authority: string;
  payment_authority: string;
  fairness_statement: string;
  evidence_boundary: string;
}

export type StandingMetaV4Operation =
  | "prepare_standing_meta_v4_claim"
  | "prepare_anonymous_stake_registration"
  | "set_anonymous_stake_availability"
  | "list_verification_assignments"
  | "submit_primary_verdict"
  | "waive_verification_appeal"
  | "open_verification_appeal"
  | "submit_appeal_vote"
  | "finalize_verification_case";

export interface StandingMetaV4ActionRequest {
  network?: "base-mainnet" | "base-sepolia" | null;
  arguments: Record<string, unknown>;
}

export interface StandingMetaV4ActionPlan {
  schema_version: "agent-bounties/standing-meta-v4-action-v1";
  protocol_version: "standing-meta-v4";
  operation: StandingMetaV4Operation;
  allowed: boolean;
  target_contract: string | null;
  function: string | null;
  arguments: Record<string, unknown>;
  blocker: string | null;
  next_action: string;
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

export interface SiteAnalyticsReport extends Record<string, unknown> {
  schema_version: "agent-bounties/site-analytics-v1";
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
  rates: Array<Record<string, unknown>>;
  definitions: string[];
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

function openCompetitionHexBytes(
  value: string,
  bytes: number,
  label: string,
): Uint8Array {
  if (!new RegExp(`^0x[0-9a-fA-F]{${bytes * 2}}$`).test(value)) {
    throw new Error(`${label} must be a ${bytes}-byte 0x-prefixed hex value`);
  }
  const result = new Uint8Array(bytes);
  for (let index = 0; index < bytes; index += 1) {
    result[index] = Number.parseInt(value.slice(2 + index * 2, 4 + index * 2), 16);
  }
  return result;
}

function openCompetitionNonzeroBytes32(value: string, label: string): Uint8Array {
  const result = openCompetitionHexBytes(value, 32, label);
  if (!result.some((byte) => byte !== 0)) {
    throw new Error(`${label} must be nonzero`);
  }
  return result;
}

function openCompetitionAddressWord(value: string): Uint8Array {
  const address = openCompetitionHexBytes(value, 20, "address");
  const result = new Uint8Array(32);
  result.set(address, 12);
  return result;
}

function openCompetitionUint256(value: bigint): Uint8Array {
  if (value < 0n || value >= 1n << 256n) {
    throw new Error("uint256 value is out of bounds");
  }
  const result = new Uint8Array(32);
  let remaining = value;
  for (let index = 31; index >= 0; index -= 1) {
    result[index] = Number(remaining & 0xffn);
    remaining >>= 8n;
  }
  return result;
}

function openCompetitionConcat(parts: Uint8Array[]): Uint8Array {
  const result = new Uint8Array(parts.reduce((total, part) => total + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.length;
  }
  return result;
}

function openCompetitionHex(value: Uint8Array): string {
  return `0x${Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

const OPEN_COMPETITION_V2_JOURNAL_DOMAIN = openCompetitionHexBytes(
  "0x40c861b5ff675d94ed282cd66e1e55bb38f03fe560786960e64d50b593ada7ba",
  32,
  "journal domain",
);
const OPEN_COMPETITION_V2_POLICY_DOMAIN = openCompetitionHexBytes(
  "0xf6a226ca20aaca3b9c0b4a609939c334b6c2b03500a5df45188df8bcd7c2b369",
  32,
  "policy domain",
);
const OPEN_COMPETITION_V2_SUBMISSION_DOMAIN = openCompetitionHexBytes(
  "0x402204460b00978c26cee42ae0089d94fe8b0b17bd90c45a6cd78d466463a507",
  32,
  "submission domain",
);
const OPEN_COMPETITION_V2_EVIDENCE_DOMAIN = openCompetitionHexBytes(
  "0x16f60f26d350a38e6993a5454967d1efb0461d93785b7cdb38ba463284c5ab15",
  32,
  "evidence domain",
);
const OPEN_COMPETITION_V2_JOURNAL_SCHEMA_HASH = openCompetitionHexBytes(
  "0xd9c492538aa0822e8a1d651886e79a2b8ddfc2c3428b3ed92e19d337eefe77d4",
  32,
  "journal schema",
);
const OPEN_COMPETITION_V2_METRIC_PROGRAM_HASH = openCompetitionHexBytes(
  "0x1c27fc20ab65264c7db2997c8b76f78d7291cdb91243481bcae1e88f77beb88a",
  32,
  "metric program",
);
const OPEN_COMPETITION_V2_PROOF_SYSTEM_WORDS: Record<OpenCompetitionV2ProofSystem, Uint8Array> = {
  groth16: openCompetitionHexBytes(
    "0x0fbfc39a4f588598b55fce747dc8dde3f1b661a9d538dc174b464d210d12a81d",
    32,
    "Groth16 proof system",
  ),
  plonk: openCompetitionHexBytes(
    "0x91e36d74d5d8703299314b82f85cab384a3df8064725b371f1f9f4ad49238f1b",
    32,
    "PLONK proof system",
  ),
};
const OPEN_COMPETITION_V2_MODE_TAGS: Record<OpenCompetitionV2MetricMode, number> = {
  all_equal: 0,
  maximize_exact_matches: 1,
  minimize_absolute_error: 2,
};

function openCompetitionUnsignedBytes(value: bigint, bytes: number, label: string): Uint8Array {
  if (value < 0n || value >= 1n << BigInt(bytes * 8)) {
    throw new Error(`${label} is out of bounds`);
  }
  const result = new Uint8Array(bytes);
  let remaining = value;
  for (let index = bytes - 1; index >= 0; index -= 1) {
    result[index] = Number(remaining & 0xffn);
    remaining >>= 8n;
  }
  return result;
}

function openCompetitionSignedBytes(value: bigint, bytes: number, label: string): Uint8Array {
  const bits = BigInt(bytes * 8);
  const minimum = -(1n << (bits - 1n));
  const maximum = 1n << (bits - 1n);
  if (value < minimum || value >= maximum) throw new Error(`${label} is out of bounds`);
  return openCompetitionUnsignedBytes(value < 0n ? (1n << bits) + value : value, bytes, label);
}

function openCompetitionUintWord(value: bigint, bits: number, label: string): Uint8Array {
  if (value < 0n || value >= 1n << BigInt(bits)) throw new Error(`${label} is out of bounds`);
  return openCompetitionUnsignedBytes(value, 32, label);
}

export function buildOpenCompetitionV2PublicVector(
  input: OpenCompetitionV2PublicVectorInput,
): OpenCompetitionV2PublicVectorResult {
  if (!(input.mode in OPEN_COMPETITION_V2_MODE_TAGS)) throw new Error("mode is unsupported");
  if (!(input.scope.proof_system in OPEN_COMPETITION_V2_PROOF_SYSTEM_WORDS)) {
    throw new Error("proof_system is unsupported");
  }
  const modeTag = OPEN_COMPETITION_V2_MODE_TAGS[input.mode];
  const threshold = BigInt(input.threshold);
  if (input.mode !== "all_equal" && threshold < 0n) {
    throw new Error("threshold cannot be negative for this mode");
  }
  if (input.vectors.length < 1 || input.vectors.length > 10_000) {
    throw new Error("vectors must contain 1..10000 cases");
  }
  const policyParts = [
    OPEN_COMPETITION_V2_POLICY_DOMAIN,
    Uint8Array.of(modeTag),
    openCompetitionSignedBytes(threshold, 16, "threshold"),
    openCompetitionUnsignedBytes(BigInt(input.vectors.length), 4, "vector count"),
  ];
  const submissionParts = [
    OPEN_COMPETITION_V2_SUBMISSION_DOMAIN,
    openCompetitionUnsignedBytes(BigInt(input.vectors.length), 4, "vector count"),
  ];
  let score = 0n;
  let totalWeight = 0n;
  for (const vector of input.vectors) {
    if (
      !Number.isSafeInteger(vector.expected) ||
      !Number.isSafeInteger(vector.observed) ||
      !Number.isSafeInteger(vector.weight) ||
      vector.weight < 1 ||
      vector.weight >= 2 ** 32
    ) {
      throw new Error("vector values must be safe integers and weight must fit positive u32");
    }
    const expected = BigInt(vector.expected);
    const observed = BigInt(vector.observed);
    const weight = BigInt(vector.weight);
    totalWeight += weight;
    if (input.mode === "minimize_absolute_error") {
      score += (expected >= observed ? expected - observed : observed - expected) * weight;
    } else if (expected === observed) {
      score += weight;
    }
    openCompetitionSignedBytes(score, 16, "score");
    policyParts.push(openCompetitionSignedBytes(expected, 8, "expected"));
    policyParts.push(openCompetitionUnsignedBytes(weight, 4, "weight"));
    submissionParts.push(openCompetitionSignedBytes(observed, 8, "observed"));
  }
  const passed =
    input.mode === "all_equal"
      ? score === totalWeight
      : input.mode === "maximize_exact_matches"
        ? score >= threshold
        : score <= threshold;
  const verificationPolicyHash = keccak_256(openCompetitionConcat(policyParts));
  const submissionHash = keccak_256(openCompetitionConcat(submissionParts));
  const evidenceHash = keccak_256(
    openCompetitionConcat([
      OPEN_COMPETITION_V2_EVIDENCE_DOMAIN,
      verificationPolicyHash,
      submissionHash,
    ]),
  );
  const journal = openCompetitionConcat([
    OPEN_COMPETITION_V2_JOURNAL_DOMAIN,
    openCompetitionUintWord(BigInt(input.scope.chain_id), 64, "chain_id"),
    openCompetitionAddressWord(input.scope.competition),
    openCompetitionNonzeroBytes32(input.scope.bounty_id, "bounty_id"),
    openCompetitionAddressWord(input.scope.solver),
    openCompetitionUintWord(BigInt(input.scope.solver_nonce), 128, "solver_nonce"),
    submissionHash,
    evidenceHash,
    OPEN_COMPETITION_V2_PROOF_SYSTEM_WORDS[input.scope.proof_system],
    openCompetitionNonzeroBytes32(input.scope.program_vkey, "program_vkey"),
    openCompetitionNonzeroBytes32(input.scope.source_hash, "source_hash"),
    openCompetitionNonzeroBytes32(input.scope.elf_hash, "elf_hash"),
    OPEN_COMPETITION_V2_JOURNAL_SCHEMA_HASH,
    OPEN_COMPETITION_V2_METRIC_PROGRAM_HASH,
    openCompetitionNonzeroBytes32(input.scope.execution_policy_hash, "execution_policy_hash"),
    verificationPolicyHash,
    openCompetitionNonzeroBytes32(input.scope.settlement_policy_hash, "settlement_policy_hash"),
    openCompetitionNonzeroBytes32(input.scope.beta_risk_hash, "beta_risk_hash"),
    openCompetitionUintWord(passed ? 1n : 0n, 8, "passed"),
    openCompetitionSignedBytes(score, 32, "score"),
  ]);
  return {
    passed,
    score: score.toString(),
    verification_policy_hash: openCompetitionHex(verificationPolicyHash),
    submission_hash: openCompetitionHex(submissionHash),
    evidence_hash: openCompetitionHex(evidenceHash),
    journal_hex: openCompetitionHex(journal),
  };
}

export function generateOpenCompetitionCommitment(
  input: OpenCompetitionCommitmentInput,
): OpenCompetitionCommitmentEnvelope {
  const chainId = input.network === "base-mainnet" ? 8453 : input.network === "base-sepolia" ? 84532 : null;
  if (chainId === null) {
    throw new Error("network must be base-mainnet or base-sepolia");
  }
  if (!globalThis.crypto?.getRandomValues) {
    throw new Error("cryptographically secure randomness is unavailable");
  }
  const salt = new Uint8Array(32);
  globalThis.crypto.getRandomValues(salt);
  const submissionHash = openCompetitionNonzeroBytes32(input.submission_hash, "submission_hash");
  const evidenceHash = openCompetitionNonzeroBytes32(input.evidence_hash, "evidence_hash");
  const domain = keccak_256(
    new TextEncoder().encode("agent-bounties/open-competition-v1-solution"),
  );
  const commitment = keccak_256(
    openCompetitionConcat([
      domain,
      openCompetitionUint256(BigInt(chainId)),
      openCompetitionAddressWord(input.bounty),
      openCompetitionAddressWord(input.solver),
      submissionHash,
      evidenceHash,
      salt,
    ]),
  );
  return {
    schema_version: "agent-bounties/open-competition-v1-commitment-v1",
    network: input.network,
    chain_id: chainId,
    bounty: input.bounty.toLowerCase(),
    solver: input.solver.toLowerCase(),
    submission_hash: input.submission_hash.toLowerCase(),
    evidence_hash: input.evidence_hash.toLowerCase(),
    salt: openCompetitionHex(salt),
    commitment: openCompetitionHex(commitment),
    committed_block: null,
    reveal_deadline: null,
    evidence_boundary:
      "This recovery envelope contains the secret salt. Store it locally and send only commitment during entry preparation.",
  };
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

  async getStandingMetaV4Readiness(
    network: "base-mainnet" | "base-sepolia" = "base-mainnet",
  ): Promise<StandingMetaV4ReadinessReport> {
    return this.query("/v1/base/standing-meta-v4/readiness", {
      network,
    }) as Promise<StandingMetaV4ReadinessReport>;
  }

  async getOpenCompetitionReadiness(
    bountyContract: string,
    network: "base-mainnet" | "base-sepolia" = "base-mainnet",
    solver?: string,
    verifierProfileId?: string,
  ): Promise<OpenCompetitionReadinessReport> {
    return this.query("/v1/base/open-competition-v1/readiness", {
      network,
      bounty_contract: bountyContract,
      solver,
      verifier_profile_id: verifierProfileId,
    }) as Promise<OpenCompetitionReadinessReport>;
  }

  async listOpenCompetitionVerifiers(
    network: "base-mainnet" | "base-sepolia" = "base-mainnet",
  ): Promise<OpenCompetitionVerifierCatalog> {
    return this.query("/v1/base/open-competition-v1/verifiers", {
      network,
    }) as Promise<OpenCompetitionVerifierCatalog>;
  }

  async prepareOpenCompetitionCreation(
    request: OpenCompetitionCreationRequest,
  ): Promise<OpenCompetitionCreationPlan> {
    const path = request.funding_authorization
      ? "authorized-creation-preparation"
      : "creation-preparation";
    return this.post(`/v1/base/open-competition-v1/${path}`, {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<OpenCompetitionCreationPlan>;
  }

  async getOpenCompetitionState(
    bountyContract: string,
    network: "base-mainnet" | "base-sepolia" = "base-mainnet",
    solver?: string,
    verifierProfileId?: string,
  ): Promise<OpenCompetitionSafeState> {
    return this.query("/v1/base/open-competition-v1/state", {
      network,
      bounty_contract: bountyContract,
      solver,
      verifier_profile_id: verifierProfileId,
    }) as Promise<OpenCompetitionSafeState>;
  }

  private openCompetitionAction(
    path: string,
    request: OpenCompetitionActionRequest,
  ): Promise<OpenCompetitionActionPlan> {
    return this.post(`/v1/base/open-competition-v1/${path}`, {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<OpenCompetitionActionPlan>;
  }

  async prepareOpenCompetitionCommit(
    request: OpenCompetitionCommitRequest,
  ): Promise<OpenCompetitionActionPlan> {
    return this.post("/v1/base/open-competition-v1/commit-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<OpenCompetitionActionPlan>;
  }

  async prepareOpenCompetitionReveal(
    request: OpenCompetitionRevealRequest,
  ): Promise<OpenCompetitionActionPlan> {
    return this.post("/v1/base/open-competition-v1/reveal-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<OpenCompetitionActionPlan>;
  }

  async getOpenCompetitionStatus(
    request: OpenCompetitionActionRequest,
  ): Promise<OpenCompetitionActionPlan> {
    return this.openCompetitionAction("status", request);
  }

  async withdrawOpenCompetitionBond(
    request: OpenCompetitionActionRequest,
  ): Promise<OpenCompetitionActionPlan> {
    return this.openCompetitionAction("bond-withdrawal-preparation", request);
  }

  async prepareOpenCompetitionEntrantAction(
    request: OpenCompetitionEntrantActionPreparationRequest,
  ): Promise<OpenCompetitionEntrantActionPlan> {
    return this.post("/v1/base/open-competition-v1/entrant-action-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<OpenCompetitionEntrantActionPlan>;
  }

  async relayOpenCompetitionEntrantAction(
    request: OpenCompetitionEntrantRelayRequest,
  ): Promise<OpenCompetitionEntrantRelayResponse> {
    return this.post(
      "/v1/base/open-competition-v1/entrant-action-relays",
      request,
    ) as Promise<OpenCompetitionEntrantRelayResponse>;
  }

  async getOpenCompetitionEntrantRelay(
    relayId: string,
  ): Promise<OpenCompetitionEntrantRelayResponse> {
    return this.query(
      `/v1/base/open-competition-v1/entrant-action-relays/${encodeURIComponent(relayId)}`,
      {},
    ) as Promise<OpenCompetitionEntrantRelayResponse>;
  }

  async getOpenCompetitionV2Profiles(
    network: OpenCompetitionV2Network = "base-mainnet",
  ): Promise<unknown> {
    return this.query("/v1/base/open-competition-v2-beta1/profiles", { network });
  }

  async validateOpenCompetitionV2(request: OpenCompetitionV2CreationRequest): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/validate", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async prepareOpenCompetitionV2Creation(
    request: OpenCompetitionV2CreationRequest,
  ): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/creation-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async prepareOpenCompetitionV2Funding(
    request: OpenCompetitionV2FundingRequest,
  ): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/funding-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async listOpenCompetitionV2Inventory(
    network: OpenCompetitionV2Network = "base-mainnet",
    state?: string,
  ): Promise<unknown> {
    return this.query("/v1/base/open-competition-v2-beta1/inventory", { network, state });
  }

  async listOpenCompetitionV2Events(
    network: OpenCompetitionV2Network = "base-mainnet",
    bountyId?: string,
  ): Promise<unknown> {
    return this.query("/v1/base/open-competition-v2-beta1/events", {
      network,
      bounty_id: bountyId,
    });
  }

  async createOpenCompetitionV2ProofQuote(
    request: OpenCompetitionV2ProofQuoteRequest,
  ): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/proof-quotes", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async prepareOpenCompetitionV2Proof(
    request: OpenCompetitionV2ProofRequest,
  ): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/proof-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async prepareOpenCompetitionV2Action(
    request: OpenCompetitionV2ActionRequest,
  ): Promise<unknown> {
    return this.post("/v1/base/open-competition-v2-beta1/action-preparation", {
      ...request,
      network: request.network ?? "base-mainnet",
    });
  }

  async getOpenCompetitionV2ProofJob(jobId: string): Promise<unknown> {
    return this.request(
      `/v1/base/open-competition-v2-beta1/proof-jobs/${encodeURIComponent(jobId)}`,
    );
  }

  async payOpenCompetitionV2ProofJob(
    jobId: string,
    paymentSignature?: string,
  ): Promise<X402BountyFundingResponse> {
    const path = `/v1/base/open-competition-v2-beta1/proof-jobs/${encodeURIComponent(jobId)}/payment`;
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: {
        ...(this.operatorApiToken ? { "x-operator-token": this.operatorApiToken } : {}),
        ...(paymentSignature ? { "PAYMENT-SIGNATURE": paymentSignature } : {}),
      },
    });
    if (![200, 202, 402, 404, 409, 422, 503].includes(response.status)) {
      throw new Error(`${path} failed: ${response.status}`);
    }
    return {
      status: response.status as X402BountyFundingResponse["status"],
      payment_required: response.headers.get("PAYMENT-REQUIRED"),
      payment_response: response.headers.get("PAYMENT-RESPONSE"),
      body: await x402ResponseBody(response),
    };
  }

  async authorizeOpenCompetitionV2ProofRelay(
    jobId: string,
    authorizationDeadline: number,
    solverSignature?: string,
  ): Promise<unknown> {
    return this.post(
      `/v1/base/open-competition-v2-beta1/proof-jobs/${encodeURIComponent(jobId)}/relay-authorization`,
      {
        authorization_deadline: authorizationDeadline,
        solver_signature: solverSignature,
      },
    );
  }

  private standingMetaV4Action(
    path: string,
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.post(`/v1/base/standing-meta-v4/${path}`, {
      ...request,
      network: request.network ?? "base-mainnet",
    }) as Promise<StandingMetaV4ActionPlan>;
  }

  async prepareStandingMetaV4Claim(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("claim-preparation", request);
  }

  async prepareAnonymousStakeRegistration(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("stake-registration-preparation", request);
  }

  async setAnonymousStakeAvailability(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("stake-availability-preparation", request);
  }

  async listVerificationAssignments(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("verification-assignments", request);
  }

  async submitPrimaryVerdict(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("primary-verdict-preparation", request);
  }

  async waiveVerificationAppeal(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("appeal-waiver-preparation", request);
  }

  async openVerificationAppeal(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("appeal-opening-preparation", request);
  }

  async submitAppealVote(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("appeal-vote-preparation", request);
  }

  async finalizeVerificationCase(
    request: StandingMetaV4ActionRequest,
  ): Promise<StandingMetaV4ActionPlan> {
    return this.standingMetaV4Action("finalization-preparation", request);
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

  async deleteUnclaimedBounty(
    request: AutonomousLifecycleRequest & { caller: string },
  ): Promise<unknown> {
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
