use app::{
    build_audience_report, build_live_money_readiness_report, build_objective_canonical_evidence,
    hash_artifact, stripe_secret_key_mode_from_secret, AddFundingContributionRequest,
    ApproveRiskBountyRequest, ApproveRiskPayoutRequest, BountyNetwork, BountyStatusResponse,
    ClaimBountyRequest, CreateFundingIntentRequest, CreateHelpRequestRequest, FundQuoteRequest,
    FundingIntentReport, LiveMoneyReadinessConfig, LiveMoneyReadinessReport,
    OpenPooledBountyRequest, PlanStripeTransferRequest as AppPlanStripeTransferRequest,
    PooledFundingReport, PostBountyRequest, QuoteSet, RecordAudienceInteractionRequest,
    RecordDiscoveryResponseRequest, RecordOutreachAttemptRequest, RegisterAgentRequest,
    RegisterCapabilityRequest, RejectRiskEventRequest, RequestQuotesRequest,
    ReviewedBountyApproval, RiskEventFilter, StripeTransferPlan, StripeTransferReconciliation,
    SubmitResultRequest, UpsertAudienceMemberRequest, UpsertContributorContactRequest,
    VerifySubmissionRequest,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use bounty_router::{BountyRouter, RouteDecision};
use chain_base::{
    autonomous_bounty_is_earning_ready, base_network_descriptor, broadcast_signed_transaction,
    build_autonomous_bounty_feed, build_autonomous_bounty_terms_record,
    build_autonomous_submission_evidence_record, build_autonomous_submission_preparation,
    build_autonomous_verification_jobs, decode_autonomous_bounty_logs,
    eth_get_transaction_receipt_request, eth_send_raw_transaction_request, fetch_block_number,
    fetch_transaction_receipt, normalize_evm_address,
    plan_canonical_child_bounty_terms as build_canonical_child_bounty_terms_plan,
    validate_attestation_request_against_feed, validate_autonomous_creation_against_terms,
    AutonomousBountyAuthorizationSignature, AutonomousBountyAuthorizedClaimPlan,
    AutonomousBountyAuthorizedContributionPlan, AutonomousBountyAuthorizedCreationPlan,
    AutonomousBountyClaimPlan, AutonomousBountyContribution, AutonomousBountyContributionPlan,
    AutonomousBountyCreate, AutonomousBountyCreationPlan, AutonomousBountyEvent,
    AutonomousBountyEventKind, AutonomousBountyFeedItem, AutonomousBountyRecoveryReservations,
    AutonomousBountySubmissionAuthorizationRequest,
    AutonomousBountySubmissionAuthorizationTypedData, AutonomousBountySubmissionPreparation,
    AutonomousBountyTxPlanner, AutonomousSignedAttestation,
    AutonomousVerificationAttestationRequest, AutonomousVerificationAttestationTypedData,
    AutonomousVerificationJob, BaseNetworkDescriptor, BaseRelayedTransaction, BaseRpcUrlConfig,
    BaseTransactionRelayer, CanonicalChildBountyTermsPlan, CanonicalChildBountyTermsRequest,
    ChainBaseError, EthGetTransactionReceiptRequest, EthSendRawTransactionRequest, EvmLog,
    EvmTransactionIntent, RpcTransactionReceipt, AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION,
    AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR,
};
use chrono::Utc;
use db::{
    BountyStatusScope, DbError, GitHubIssueSyncBountyUpsert, NewX402RelayAttempt, PostgresStore,
    X402RelayAttempt, X402RelayStatus,
};
use domain::{
    Agent, AudienceInteraction, AudienceMember, AudienceReport, AutonomousBountyTermsDocument,
    AutonomousBountyTermsRecord, AutonomousSubmissionEvidenceRecord, BountyStatus, Capability,
    CapabilityClass, ContributorContact, DiscoveryResponse, EvalRun, HelpRequest, Id, Money,
    Objective, ObjectiveAction, ObjectiveActionPlan, ObjectiveCanonicalEvidence,
    ObjectiveCreationDraft, ObjectiveCreationPlan, ObjectiveError, ObjectiveView, OutreachAttempt,
    PaymentRail, PayoutStatus, PrivacyLevel, RiskEvent, RiskReviewRecord, SignedObjectiveAction,
    SignedObjectiveCreation, VerificationDecision, VerifierKind,
};
use eval_harness::{
    bundled_abuse_fixtures, bundled_fixtures, bundled_judge_fixtures, run_eval_loops, AbuseBench,
    BountyBench, EvalSuiteResult, JudgeBench, LoopSuiteResult,
};
use github_app::{
    bounty_check_output, claim_comment_plan, funding_comment_plan, issue_api_sync_plan,
    parse_issue_form_bounty, proof_comment_plan, GitHubCheckRunOutput, GitHubClaimCommentInput,
    GitHubClaimCommentPlan, GitHubFundingCommentInput, GitHubFundingCommentPlan,
    GitHubIssueApiSyncInput, GitHubIssueApiSyncPlan, GitHubIssueFormBounty, GitHubProofComment,
    GitHubProofCommentPlan,
};
use ledger::Ledger;
use payments_stripe::{
    apply_checkout_payment_method_configuration, execute_stripe_request, verify_webhook_signature,
    CheckoutTopUpRequest, ConnectAccountSnapshot, StripeEventDeduper, StripeExecutionReport,
    StripePlanner, StripeRequestIntent, StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use payments_x402::{
    base_usdc_funding_challenge, decode_payment_signature_header, encode_payment_required_header,
    encode_payment_response_header, validate_funding_payload, PaymentRequired, SettlementResponse,
    AGENT_BOUNTY_FUND_SCHEME, PAYMENT_REQUIRED_HEADER, PAYMENT_RESPONSE_HEADER,
    PAYMENT_SIGNATURE_HEADER, X402_VERSION,
};
use risk::{RiskPolicy, RiskPolicyDescriptor};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration, Instant};
use tower_http::cors::CorsLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::Components;
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        llms_txt,
        discovery_manifest_schema,
        agent_bounties_discovery,
        x402_discovery,
        risk_policy,
        live_money_readiness,
        list_risk_events,
        list_risk_reviews,
        approve_risk_bounty,
        approve_risk_payout,
        reject_risk_event,
        route_blocked_goal,
        run_bountybench,
        run_abusebench,
        run_judgebench,
        run_eval_loop_suite,
        list_eval_runs,
        register_agent,
        agent_paid_status,
        upsert_contributor_contact,
        list_contributor_contacts,
        upsert_audience_member,
        list_audience_members,
        record_audience_interaction,
        list_audience_interactions,
        record_discovery_response,
        list_discovery_responses,
        record_outreach_attempt,
        list_outreach_attempts,
        audience_report,
        plan_objective_creation,
        create_objective,
        list_objectives,
        get_objective,
        plan_objective_action,
        apply_objective_action,
        reconcile_objective,
        register_capability,
        search_capabilities,
        create_help_request,
        request_quotes,
        fund_quote,
        list_claimable_bounties,
        public_bounty_feed,
        public_funding_feed,
        public_capability_feed,
        x402_base_bounty_funding,
        get_x402_relay,
        broadcast_base_signed_transaction,
        get_base_transaction_receipt,
        plan_autonomous_canonical_child_terms,
        plan_autonomous_bounty_creation,
        plan_autonomous_bounty_authorized_creation,
        plan_autonomous_bounty_contribution,
        plan_autonomous_bounty_authorized_contribution,
        plan_autonomous_bounty_claim,
        plan_autonomous_bounty_authorized_claim,
        plan_autonomous_bounty_submission,
        prepare_autonomous_bounty_submission,
        plan_autonomous_bounty_submission_authorization,
        plan_autonomous_verification_attestation,
        plan_autonomous_module_settlement,
        plan_autonomous_attestation_settlement,
        plan_autonomous_expire_claim,
        plan_autonomous_expire_submission,
        relay_autonomous_timeout,
        plan_autonomous_cancel,
        plan_autonomous_refund_withdrawal,
        decode_autonomous_bounty_events,
        list_autonomous_bounty_events,
        publish_autonomous_bounty_terms,
        get_autonomous_bounty_terms,
        publish_autonomous_submission_evidence,
        get_autonomous_submission_evidence,
        autonomous_bounty_feed,
        autonomous_verification_jobs,
        plan_stripe_checkout_top_up,
        plan_stripe_connect_account,
        plan_stripe_connect_transfer,
        execute_stripe_funding_intent_checkout,
        execute_stripe_checkout_top_up,
        execute_stripe_connect_account,
        execute_stripe_connect_transfer,
        reconcile_stripe_connect_snapshot,
        reconcile_stripe_transfer_event,
        reconcile_stripe_checkout_webhook,
        plan_github_issue_bounty,
        plan_github_issue_api_sync,
        sync_github_issue_api_bounty,
        plan_github_funding_comment,
        plan_github_claim_comment,
        plan_github_proof_comment,
        plan_github_proof_comment_from_proof,
        post_bounty,
        open_pooled_bounty,
        create_funding_intent,
        add_funding_contribution,
        claim_bounty,
        submit_result,
        verify_submission,
        bounty_status
    ),
    components(schemas(
        RouteRequest,
        RouteDecision,
        EvalSuiteResult,
        LoopSuiteResult,
        EvalRun,
        RiskEvent,
        RiskReviewRecord,
        RiskPolicyDescriptor,
        PlanStripeCheckoutTopUpRequest,
        PlanStripeConnectAccountRequest,
        PlanStripeConnectTransferRequest,
        PlanGitHubIssueBountyRequest,
        PlanGitHubIssueApiSyncRequest,
        PlanGitHubFundingCommentRequest,
        PlanGitHubClaimCommentRequest,
        PlanGitHubProofCommentRequest,
        PlanGitHubProofCommentFromProofRequest,
        BroadcastBaseSignedTransactionRequest,
        GetBaseTransactionReceiptRequest,
        SearchCapabilitiesRequest,
        ContributorContact,
        AudienceMember,
        AudienceInteraction,
        DiscoveryResponse,
        OutreachAttempt,
        AudienceReport
        ,ObjectiveCreationDraft
        ,ObjectiveCreationPlan
        ,SignedObjectiveCreation
        ,ObjectiveAction
        ,ObjectiveActionPlan
        ,SignedObjectiveAction
        ,ObjectiveView
    )),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Components::new);
        components.add_security_scheme(
            "operator_api_token",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                OPERATOR_TOKEN_HEADER,
                "Operator API token required for hosted mutation surfaces when OPERATOR_API_TOKEN is configured.",
            ))),
        );
        let mut bearer = Http::new(HttpAuthScheme::Bearer);
        bearer.bearer_format = Some("operator-api-token".to_string());
        bearer.description =
            Some("Bearer form of the operator API token for hosted mutation surfaces.".to_string());
        components.add_security_scheme("operator_bearer", SecurityScheme::Http(bearer));
    }
}

#[derive(Clone)]
struct AppState {
    network: Arc<Mutex<BountyNetwork>>,
    eval_runs: Arc<Mutex<Vec<EvalRun>>>,
    stripe_webhook_secret: Option<Vec<u8>>,
    allow_unsigned_stripe_webhooks: bool,
    stripe_secret_key: Option<String>,
    stripe_live_execution_enabled: bool,
    stripe_public_checkout_enabled: bool,
    stripe_api_base_url: String,
    stripe_payment_method_configuration: Option<String>,
    store: Option<PostgresStore>,
    base_rpc_urls: BaseRpcUrlConfig,
    base_broadcast_enabled: bool,
    operator_api_token: Option<String>,
    public_base_url: String,
    mcp_base_url: String,
    x402_relayer: X402HostedRelayerConfig,
    recovery_reservations: AutonomousBountyRecoveryReservations,
}

#[derive(Clone)]
struct X402HostedRelayerConfig {
    enabled: bool,
    relayer: Option<Arc<BaseTransactionRelayer>>,
    min_amount: u64,
    max_amount: u64,
    max_gas: u64,
    max_fee_per_gas_wei: u128,
    max_daily_attempts: u32,
    max_daily_attempts_per_contributor: u32,
    confirmations: u64,
    wait_seconds: u64,
    rpc_timeout_seconds: u64,
    lease_seconds: u64,
}

impl Default for X402HostedRelayerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            relayer: None,
            min_amount: 100_000,
            max_amount: 5_000_000,
            max_gas: 300_000,
            max_fee_per_gas_wei: 10_000_000_000,
            max_daily_attempts: 100,
            max_daily_attempts_per_contributor: 10,
            confirmations: 2,
            wait_seconds: 20,
            rpc_timeout_seconds: 15,
            lease_seconds: 45,
        }
    }
}

impl X402HostedRelayerConfig {
    fn from_env() -> anyhow::Result<Self> {
        let enabled = env_flag("ENABLE_X402_HOSTED_RELAY");
        let relayer = env::var("X402_RELAYER_PRIVATE_KEY")
            .ok()
            .and_then(non_empty_secret)
            .map(|private_key| BaseTransactionRelayer::from_private_key(&private_key))
            .transpose()
            .map_err(|_| anyhow::anyhow!("X402_RELAYER_PRIVATE_KEY is invalid"))?
            .map(Arc::new);
        if enabled && relayer.is_none() {
            anyhow::bail!("ENABLE_X402_HOSTED_RELAY requires X402_RELAYER_PRIVATE_KEY");
        }
        let min_amount = env_u64("X402_RELAYER_MIN_USDC_BASE_UNITS", 100_000)?;
        let max_amount = env_u64("X402_RELAYER_MAX_USDC_BASE_UNITS", 5_000_000)?;
        let max_gas = env_u64("X402_RELAYER_MAX_GAS", 300_000)?;
        let max_fee_per_gas_wei = env_u128("X402_RELAYER_MAX_FEE_PER_GAS_WEI", 10_000_000_000)?;
        if min_amount == 0 || max_amount < min_amount || max_gas == 0 || max_fee_per_gas_wei == 0 {
            anyhow::bail!("x402 relayer amount, gas, and fee caps must be positive");
        }
        let max_daily_attempts = u32::try_from(env_u64("X402_RELAYER_MAX_DAILY_ATTEMPTS", 100)?)
            .map_err(|_| anyhow::anyhow!("X402_RELAYER_MAX_DAILY_ATTEMPTS exceeds u32"))?;
        let max_daily_attempts_per_contributor = u32::try_from(env_u64(
            "X402_RELAYER_MAX_DAILY_ATTEMPTS_PER_CONTRIBUTOR",
            10,
        )?)
        .map_err(|_| {
            anyhow::anyhow!("X402_RELAYER_MAX_DAILY_ATTEMPTS_PER_CONTRIBUTOR exceeds u32")
        })?;
        if max_daily_attempts == 0
            || max_daily_attempts_per_contributor == 0
            || max_daily_attempts_per_contributor > max_daily_attempts
        {
            anyhow::bail!("x402 relayer rolling-24-hour quotas are invalid");
        }
        let rpc_timeout_seconds = env_u64("X402_RELAYER_RPC_TIMEOUT_SECONDS", 15)?.clamp(1, 30);
        let lease_seconds = env_u64("X402_RELAYER_LEASE_SECONDS", 45)?.max(15);
        if lease_seconds <= rpc_timeout_seconds {
            anyhow::bail!("X402_RELAYER_LEASE_SECONDS must exceed the RPC timeout");
        }
        Ok(Self {
            enabled,
            relayer,
            min_amount,
            max_amount,
            max_gas,
            max_fee_per_gas_wei,
            max_daily_attempts,
            max_daily_attempts_per_contributor,
            confirmations: env_u64("X402_RELAYER_CONFIRMATIONS", 2)?.max(1),
            wait_seconds: env_u64("X402_RELAYER_WAIT_SECONDS", 20)?.min(60),
            rpc_timeout_seconds,
            lease_seconds,
        })
    }

    fn address(&self) -> Option<String> {
        self.relayer.as_ref().map(|relayer| relayer.address())
    }
}

type SharedState = Arc<AppState>;
const OPERATOR_TOKEN_HEADER: &str = "x-operator-token";

fn non_empty_secret(secret: String) -> Option<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .map(|value| value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn env_u64(key: &str, default: u64) -> anyhow::Result<u64> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| anyhow::anyhow!("{key} must be a positive integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn env_u128(key: &str, default: u128) -> anyhow::Result<u128> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .parse::<u128>()
                .map_err(|_| anyhow::anyhow!("{key} must be a positive integer"))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn require_operator(state: &SharedState, headers: &HeaderMap) -> Result<(), StatusCode> {
    let Some(expected) = state.operator_api_token.as_deref() else {
        return Ok(());
    };
    let Some(provided) = operator_token_from_headers(headers) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

fn operator_token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get(OPERATOR_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(non_empty_borrowed)
    {
        return Some(value.to_string());
    }

    let authorization = headers.get("authorization")?.to_str().ok()?.trim();
    let token = authorization.strip_prefix("Bearer ")?;
    non_empty_borrowed(token).map(ToOwned::to_owned)
}

fn non_empty_borrowed(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RouteRequest {
    goal: String,
    context: String,
    budget_minor: i64,
    currency: String,
    privacy: PrivacyLevel,
}

#[derive(Debug, Deserialize)]
struct LiveMoneyReadinessQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeCheckoutTopUpRequest {
    organization_id: Uuid,
    amount_minor: i64,
    currency: String,
    success_url: Option<String>,
    cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeConnectAccountRequest {
    agent_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanStripeConnectTransferRequest {
    payout_intent_id: Uuid,
    connected_account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubIssueBountyRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubIssueApiSyncRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    api_base_url: Option<String>,
    #[serde(default)]
    existing_bounty_ids: Vec<Uuid>,
    hosted_api_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubFundingCommentRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    funding_api_base_url: Option<String>,
    #[serde(default)]
    existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubClaimCommentRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
    comment_body: String,
    contributor_login: Option<String>,
    comment_id: Option<String>,
    claim_age_minutes: Option<u64>,
    #[serde(default)]
    progress_signal_count: u32,
    active_claim_login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubProofCommentRequest {
    bounty_id: Uuid,
    proof_url: String,
    verifier_summary: String,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanGitHubProofCommentFromProofRequest {
    proof_id: Uuid,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GitHubIssueBountyPlan {
    ready: bool,
    parsed: Option<GitHubIssueFormBounty>,
    error: Option<String>,
    check: GitHubCheckRunOutput,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
struct SearchCapabilitiesRequest {
    class: Option<CapabilityClass>,
    template_slug: Option<String>,
    currency: Option<String>,
    max_price_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyCreationRequest {
    network: Option<String>,
    create: AutonomousBountyCreate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyContributionRequest {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedCreationRequest {
    network: Option<String>,
    create: AutonomousBountyCreate,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedContributionRequest {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyClaimRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: Option<String>,
    authorization_valid_before: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedClaimRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: String,
    authorization_valid_before: u64,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionRequest {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    submission_hash: String,
    evidence_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionAuthorizationRequest {
    network: Option<String>,
    submission: AutonomousBountySubmissionAuthorizationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PrepareAutonomousBountySubmissionRequest {
    network: Option<String>,
    bounty_contract: String,
    solver_wallet: String,
    artifact_reference: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousVerificationAttestationRequest {
    network: Option<String>,
    attestation: AutonomousVerificationAttestationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousModuleSettlementRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousAttestationSettlementRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    attestations: Vec<AutonomousSignedAttestation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousLifecycleRequest {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum AutonomousTimeoutAction {
    ExpireClaim,
    ExpireSubmission,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RelayAutonomousTimeoutRequest {
    network: Option<String>,
    bounty_contract: String,
    action: AutonomousTimeoutAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct RelayAutonomousTimeoutResponse {
    network: String,
    bounty_contract: String,
    action: AutonomousTimeoutAction,
    previous_bounty_state: String,
    expected_bounty_state: String,
    expected_canonical_event: String,
    transaction_hash: String,
    relayer: String,
    confirmed: bool,
    confirmed_block: Option<u64>,
    canonical_event_id: Option<String>,
    evidence_boundary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecodeAutonomousBountyEventsRequest {
    logs: Vec<EvmLog>,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousBountyEventsQuery {
    network: Option<String>,
    bounty_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct X402FundingQuery {
    network: Option<String>,
    amount: Option<u64>,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousBountyTermsRequest {
    creator_wallet: String,
    document: AutonomousBountyTermsDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousSubmissionEvidenceRequest {
    network: Option<String>,
    bounty_contract: String,
    bounty_id: String,
    round: u64,
    solver_wallet: String,
    artifact_reference: String,
    evidence: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousSubmissionEvidenceQuery {
    network: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousBountyFeedQuery {
    network: Option<String>,
    claimable_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct AutonomousVerificationJobsQuery {
    network: Option<String>,
    verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct BroadcastBaseSignedTransactionRequest {
    signed_transaction: String,
    request_id: Option<u64>,
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct GetBaseTransactionReceiptRequest {
    tx_hash: String,
    request_id: Option<u64>,
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseSignedTransactionBroadcastReport {
    network: BaseNetworkDescriptor,
    request: EthSendRawTransactionRequest,
    tx_hash: String,
    next_step: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseTransactionReceiptReport {
    network: BaseNetworkDescriptor,
    request: EthGetTransactionReceiptRequest,
    receipt_found: bool,
    tx_hash: String,
    block_number: Option<u64>,
    succeeded: Option<bool>,
    log_count: usize,
    receipt: Option<RpcTransactionReceipt>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = match env::var("DATABASE_URL") {
        Ok(database_url) => {
            let store = PostgresStore::connect(&database_url).await?;
            store.migrate().await?;
            Some(store)
        }
        Err(_) => None,
    };
    let network = if let Some(store) = &store {
        hydrate_network(store).await?
    } else {
        BountyNetwork::default()
    };
    let eval_runs = if let Some(store) = &store {
        store.list_eval_runs().await?
    } else {
        Vec::new()
    };
    let x402_relayer = X402HostedRelayerConfig::from_env()?;
    if x402_relayer.enabled && store.is_none() {
        anyhow::bail!("ENABLE_X402_HOSTED_RELAY requires DATABASE_URL");
    }
    let recovery_reservations_raw = env::var("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS").ok();
    let recovery_reservations =
        AutonomousBountyRecoveryReservations::parse_csv(recovery_reservations_raw.as_deref())
            .map_err(|error| {
                anyhow::anyhow!("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS is invalid: {error}")
            })?;
    let state: SharedState = Arc::new(AppState {
        network: Arc::new(Mutex::new(network)),
        eval_runs: Arc::new(Mutex::new(eval_runs)),
        stripe_webhook_secret: env::var("STRIPE_WEBHOOK_SECRET")
            .ok()
            .map(|secret| secret.into_bytes()),
        allow_unsigned_stripe_webhooks: env::var("ALLOW_UNSIGNED_STRIPE_WEBHOOKS")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok(),
        stripe_live_execution_enabled: env::var("ENABLE_STRIPE_LIVE_EXECUTION")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_public_checkout_enabled: env::var("ENABLE_STRIPE_PUBLIC_CHECKOUT")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_api_base_url: env::var("STRIPE_API_BASE_URL")
            .unwrap_or_else(|_| STRIPE_API_BASE_URL.to_string()),
        stripe_payment_method_configuration: env::var("STRIPE_PAYMENT_METHOD_CONFIGURATION")
            .ok()
            .and_then(non_empty_secret),
        store,
        base_rpc_urls: BaseRpcUrlConfig::from_env(),
        base_broadcast_enabled: env::var("ENABLE_BASE_TX_BROADCAST")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        operator_api_token: env::var("OPERATOR_API_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        public_base_url: env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string()),
        mcp_base_url: env::var("MCP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string()),
        x402_relayer,
        recovery_reservations,
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/llms.txt", get(llms_txt))
        .route(
            "/schemas/discovery-manifest.v2.json",
            get(discovery_manifest_schema),
        )
        .route(
            "/.well-known/agent-bounties.json",
            get(agent_bounties_discovery),
        )
        .route("/.well-known/x402.json", get(x402_discovery))
        .route("/v1/discovery", get(agent_bounties_discovery))
        .route("/v1/risk/policy", get(risk_policy))
        .route("/v1/readiness/live-money", get(live_money_readiness))
        .route("/v1/risk/events", get(list_risk_events))
        .route("/v1/risk/reviews", get(list_risk_reviews))
        .route("/v1/risk/bounty-approvals", post(approve_risk_bounty))
        .route("/v1/risk/payout-approvals", post(approve_risk_payout))
        .route("/v1/risk/events/:id/reject", post(reject_risk_event))
        .route("/v1/route-blocked-goal", post(route_blocked_goal))
        .route("/v1/evals/bountybench", get(run_bountybench))
        .route("/v1/evals/abusebench", get(run_abusebench))
        .route("/v1/evals/judgebench", get(run_judgebench))
        .route("/v1/evals/loops", get(run_eval_loop_suite))
        .route("/v1/evals/runs", get(list_eval_runs))
        .route("/v1/agents", post(register_agent))
        .route("/v1/agents/:id/paid-status", get(agent_paid_status))
        .route(
            "/v1/contributor-contacts",
            post(upsert_contributor_contact).get(list_contributor_contacts),
        )
        .route(
            "/v1/audience/members",
            post(upsert_audience_member).get(list_audience_members),
        )
        .route(
            "/v1/audience/interactions",
            post(record_audience_interaction).get(list_audience_interactions),
        )
        .route(
            "/v1/audience/discovery-responses",
            post(record_discovery_response).get(list_discovery_responses),
        )
        .route(
            "/v1/audience/outreach-attempts",
            post(record_outreach_attempt).get(list_outreach_attempts),
        )
        .route("/v1/audience/report", get(audience_report))
        .route(
            "/v1/objectives/creation-plans",
            post(plan_objective_creation),
        )
        .route(
            "/v1/objectives",
            post(create_objective).get(list_objectives),
        )
        .route("/v1/objectives/:id", get(get_objective))
        .route(
            "/v1/objectives/:id/action-plans",
            post(plan_objective_action),
        )
        .route("/v1/objectives/:id/actions", post(apply_objective_action))
        .route("/v1/objectives/:id/reconcile", post(reconcile_objective))
        .route("/v1/capabilities", post(register_capability))
        .route("/v1/capabilities/feed", get(public_capability_feed))
        .route("/v1/capabilities/search", post(search_capabilities))
        .route("/v1/help-requests", post(create_help_request))
        .route("/v1/help-requests/:id/quotes", post(request_quotes))
        .route("/v1/quotes/:id/fund-bounty", post(fund_quote))
        .route("/v1/bounties", post(post_bounty))
        .route("/v1/bounties/pooled", post(open_pooled_bounty))
        .route("/v1/bounties/claimable", get(list_claimable_bounties))
        .route("/v1/bounties/feed", get(public_bounty_feed))
        .route("/v1/bounties/funding-feed", get(public_funding_feed))
        .route(
            "/v1/x402/base/bounties/:bounty_contract/funding",
            get(x402_base_bounty_funding),
        )
        .route("/v1/x402/base/relays/:relay_id", get(get_x402_relay))
        .route(
            "/v1/bounties/:id/funding-intents",
            post(create_funding_intent),
        )
        .route(
            "/v1/bounties/:id/funding-contributions",
            post(add_funding_contribution),
        )
        .route("/v1/bounties/:id/claim", post(claim_bounty))
        .route("/v1/bounties/:id/submit", post(submit_result))
        .route("/v1/bounties/:id/verify", post(verify_submission))
        .route("/v1/bounties/:id", get(bounty_status))
        .route(
            "/v1/base/broadcast-signed-transaction",
            post(broadcast_base_signed_transaction),
        )
        .route(
            "/v1/base/transaction-receipt",
            post(get_base_transaction_receipt),
        )
        .route(
            "/v1/base/autonomous-bounties/canonical-child-terms-plan",
            post(plan_autonomous_canonical_child_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/creation-plan",
            post(plan_autonomous_bounty_creation),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-creation-plan",
            post(plan_autonomous_bounty_authorized_creation),
        )
        .route(
            "/v1/base/autonomous-bounties/contribution-plan",
            post(plan_autonomous_bounty_contribution),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-contribution-plan",
            post(plan_autonomous_bounty_authorized_contribution),
        )
        .route(
            "/v1/base/autonomous-bounties/claim-plan",
            post(plan_autonomous_bounty_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            post(plan_autonomous_bounty_authorized_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-plan",
            post(plan_autonomous_bounty_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-preparation",
            post(prepare_autonomous_bounty_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-authorization-plan",
            post(plan_autonomous_bounty_submission_authorization),
        )
        .route(
            "/v1/base/autonomous-bounties/verification-attestation-plan",
            post(plan_autonomous_verification_attestation),
        )
        .route(
            "/v1/base/autonomous-bounties/module-settlement-plan",
            post(plan_autonomous_module_settlement),
        )
        .route(
            "/v1/base/autonomous-bounties/attestation-settlement-plan",
            post(plan_autonomous_attestation_settlement),
        )
        .route(
            "/v1/base/autonomous-bounties/expire-claim-plan",
            post(plan_autonomous_expire_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/expire-submission-plan",
            post(plan_autonomous_expire_submission),
        )
        .route(
            "/v1/base/autonomous-bounties/timeout-relay",
            post(relay_autonomous_timeout),
        )
        .route(
            "/v1/base/autonomous-bounties/cancel-plan",
            post(plan_autonomous_cancel),
        )
        .route(
            "/v1/base/autonomous-bounties/refund-withdrawal-plan",
            post(plan_autonomous_refund_withdrawal),
        )
        .route(
            "/v1/base/autonomous-bounties/decode-events",
            post(decode_autonomous_bounty_events),
        )
        .route(
            "/v1/base/autonomous-bounties/events",
            get(list_autonomous_bounty_events),
        )
        .route(
            "/v1/base/autonomous-bounties/terms",
            post(publish_autonomous_bounty_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/terms/:terms_hash",
            get(get_autonomous_bounty_terms),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-evidence",
            post(publish_autonomous_submission_evidence),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-evidence/:bounty_contract/:round",
            get(get_autonomous_submission_evidence),
        )
        .route(
            "/v1/base/autonomous-bounties/feed",
            get(autonomous_bounty_feed),
        )
        .route(
            "/v1/base/autonomous-bounties/verification-jobs",
            get(autonomous_verification_jobs),
        )
        .route(
            "/v1/stripe/checkout-top-ups",
            post(plan_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/connect-accounts",
            post(plan_stripe_connect_account),
        )
        .route(
            "/v1/stripe/connect-transfers",
            post(plan_stripe_connect_transfer),
        )
        .route(
            "/v1/stripe/live/checkout-top-ups",
            post(execute_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/live/funding-intents/:id/checkout-session",
            post(execute_stripe_funding_intent_checkout),
        )
        .route(
            "/v1/stripe/live/connect-accounts",
            post(execute_stripe_connect_account),
        )
        .route(
            "/v1/stripe/live/connect-transfers",
            post(execute_stripe_connect_transfer),
        )
        .route(
            "/v1/stripe/connect-snapshots",
            post(reconcile_stripe_connect_snapshot),
        )
        .route(
            "/v1/stripe/transfer-events",
            post(reconcile_stripe_transfer_event),
        )
        .route(
            "/v1/stripe/checkout-webhooks",
            post(reconcile_stripe_checkout_webhook),
        )
        .route(
            "/v1/github/issue-bounty-plan",
            post(plan_github_issue_bounty),
        )
        .route(
            "/v1/github/issue-api-sync-plan",
            post(plan_github_issue_api_sync),
        )
        .route(
            "/v1/github/issue-api-sync",
            post(sync_github_issue_api_bounty),
        )
        .route(
            "/v1/github/funding-comment-plan",
            post(plan_github_funding_comment),
        )
        .route(
            "/v1/github/claim-comment-plan",
            post(plan_github_claim_comment),
        )
        .route(
            "/v1/github/proof-comment-plan",
            post(plan_github_proof_comment),
        )
        .route(
            "/v1/github/proof-comment-plan-from-proof",
            post(plan_github_proof_comment_from_proof),
        )
        .route("/public/proofs/:id", get(public_proof_page))
        .route("/public/agents/:id", get(public_agent_profile))
        .route("/public/capabilities", get(public_capability_feed_page))
        .route("/public/verifiers/:kind", get(public_verifier_profile))
        .route("/public/bounties", get(public_bounty_feed_page))
        .route("/public/funding", get(public_funding_feed_page))
        .route("/public/bounties/:id", get(public_bounty_page))
        .route("/public/templates", get(public_template_index))
        .route("/public/templates/:slug", get(public_template_page))
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/docs", get(api_docs))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = service_bind_addr(
        env::var("API_BIND_ADDR").ok().as_deref(),
        env::var("PORT").ok().as_deref(),
        "127.0.0.1:8080",
    );
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn service_bind_addr(configured: Option<&str>, port: Option<&str>, default_addr: &str) -> String {
    configured
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            port.filter(|value| !value.trim().is_empty())
                .map(|value| format!("0.0.0.0:{}", value.trim()))
        })
        .unwrap_or_else(|| default_addr.to_string())
}

#[utoipa::path(get, path = "/health", responses((status = 200, body = String)))]
async fn health(State(state): State<SharedState>) -> Response {
    let mut response = health_response(&deployment_revision()).into_response();
    response.headers_mut().insert(
        "x-agent-bounties-x402-relay",
        HeaderValue::from_static(if state.x402_relayer.enabled {
            "enabled"
        } else {
            "disabled"
        }),
    );
    if let Some(address) = state.x402_relayer.address() {
        if let Ok(value) = HeaderValue::from_str(&address) {
            response
                .headers_mut()
                .insert("x-agent-bounties-x402-relayer", value);
        }
    }
    response
}

fn deployment_revision() -> String {
    env::var("RENDER_GIT_COMMIT")
        .ok()
        .filter(|value| {
            value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
        })
        .unwrap_or_else(|| "local".to_string())
}

fn health_response(revision: &str) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-agent-bounties-revision",
        HeaderValue::from_str(revision).unwrap_or_else(|_| HeaderValue::from_static("invalid")),
    );
    headers.insert(
        "x-agent-bounties-protocol",
        HeaderValue::from_static("agent-bounties/autonomous-v1"),
    );
    (headers, "ok")
}

#[utoipa::path(get, path = "/llms.txt", responses((status = 200, body = String)))]
async fn llms_txt(State(state): State<SharedState>) -> String {
    web_public::render_llms_txt(&state.public_base_url, &state.mcp_base_url)
}

#[utoipa::path(get, path = "/schemas/discovery-manifest.v2.json", responses((status = 200, body = String)))]
async fn discovery_manifest_schema() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/schema+json")],
        web_public::discovery_manifest_schema_json(),
    )
}

async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

async fn api_docs() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Agent Bounty Network API</title>
<style>
body { color: #1f2937; font-family: system-ui, sans-serif; line-height: 1.5; margin: 2rem auto; max-width: 760px; padding: 0 1rem; }
a { color: #0f766e; }
code, pre { background: #f3f4f6; border-radius: 6px; }
code { padding: 0.15rem 0.3rem; }
pre { overflow-x: auto; padding: 1rem; }
</style>
</head>
<body>
<h1>Agent Bounty Network API</h1>
<p>The machine-readable OpenAPI document is available at <a href="/api-docs/openapi.json">/api-docs/openapi.json</a>.</p>
<p>Agent orientation is available at <a href="/llms.txt">/llms.txt</a>.</p>
<p>The autonomous discovery schema is available at <a href="/schemas/discovery-manifest.v2.json">/schemas/discovery-manifest.v2.json</a>.</p>
<pre><code>curl http://127.0.0.1:8080/.well-known/agent-bounties.json</code></pre>
</body>
</html>"#,
    )
}

#[utoipa::path(get, path = "/v1/discovery", responses((status = 200, description = "Agent discovery manifest")))]
async fn agent_bounties_discovery(
    State(state): State<SharedState>,
) -> Json<web_public::DiscoveryManifest> {
    Json(web_public::discovery_manifest(
        &state.public_base_url,
        &state.mcp_base_url,
    ))
}

#[utoipa::path(get, path = "/.well-known/x402.json", responses((status = 200, description = "x402 funding and discovery capabilities")))]
async fn x402_discovery(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let api = state.public_base_url.trim_end_matches('/');
    let hosted_relayer_address = state.x402_relayer.address();
    Json(serde_json::json!({
        "x402Version": X402_VERSION,
        "service": "Agent Bounties",
        "description": "AI agents can discover canonical bounties and authorize Base USDC funding without an allowance transaction.",
        "resources": [
            {
                "name": "canonical-bounty-funding",
                "method": "GET",
                "urlTemplate": format!("{api}/v1/x402/base/bounties/{{bounty_contract}}/funding?network=base-mainnet&amount={{usdc_base_units}}"),
                "scheme": AGENT_BOUNTY_FUND_SCHEME,
                "networks": ["eip155:8453", "eip155:84532"],
                "asset": "native USDC",
                "flow": [
                    "request without PAYMENT-SIGNATURE and receive 402 plus PAYMENT-REQUIRED",
                    "sign the exact EIP-3009 authorization in the challenge",
                    "retry with PAYMENT-SIGNATURE; the bounded hosted relayer simulates and broadcasts fundWithAuthorization",
                    "receive 200 plus PAYMENT-RESPONSE after canonical FundingAdded, or poll the returned statusUrl when the response is 202"
                ],
                "settlement": "The HTTP authorization response is not settlement. Only confirmed canonical FundingAdded changes funding state.",
                "genericExactCompatible": false
            },
            {
                "name": "open-bounty-discovery",
                "method": "GET",
                "url": format!("{api}/v1/base/autonomous-bounties/feed"),
                "price": "free"
            }
        ],
        "safety": {
            "standardExactToBountyContract": "rejected because a direct token transfer bypasses fundWithAuthorization and emits no FundingAdded",
            "authorizationReplay": "USDC EIP-3009 nonces are single-use on-chain",
            "paymentProof": "transaction plans, signatures, broadcasts, and transaction hashes are not funding evidence",
            "relayerCustody": "the hosted relayer holds gas only; the funder signs an exact amount, bounty, network, nonce, and expiration and the contract pulls USDC directly from that funder"
        },
        "hostedRelay": {
            "enabled": state.x402_relayer.enabled,
            "address": hosted_relayer_address,
            "minUsdcBaseUnits": state.x402_relayer.min_amount.to_string(),
            "maxUsdcBaseUnits": state.x402_relayer.max_amount.to_string(),
            "maxGas": state.x402_relayer.max_gas,
            "maxFeePerGasWei": state.x402_relayer.max_fee_per_gas_wei.to_string(),
            "maxDailyAttempts": state.x402_relayer.max_daily_attempts,
            "maxDailyAttemptsPerContributor": state.x402_relayer.max_daily_attempts_per_contributor,
            "confirmations": state.x402_relayer.confirmations,
            "statusUrlTemplate": format!("{api}/v1/x402/base/relays/{{relay_id}}"),
            "fallback": "When disabled, a valid signed retry returns a self-relay transaction plan instead of claiming settlement."
        },
        "bazaar": {
            "status": "custom funding scheme is self-described here and is not falsely advertised as supported by generic exact facilitators",
            "next": "add a separate standard exact paid resource only when it provides distinct agent value and a production facilitator is configured"
        },
        "mpp": {
            "status": "planned",
            "scope": "fiat-capable payment credentials, recurring or metered sessions, and Stripe-backed convenience rails; never canonical bounty settlement authority"
        }
    }))
}

#[utoipa::path(get, path = "/v1/risk/policy", responses((status = 200, body = RiskPolicyDescriptor)))]
async fn risk_policy() -> Json<RiskPolicyDescriptor> {
    Json(RiskPolicy::default().descriptor())
}

#[utoipa::path(
    get,
    path = "/v1/readiness/live-money",
    params(("network" = Option<String>, Query, description = "Base network, defaults to base-mainnet")),
    responses(
        (status = 200, description = "Non-secret live-money readiness report"),
        (status = 400, description = "Unknown Base network")
    )
)]
async fn live_money_readiness(
    State(state): State<SharedState>,
    Query(query): Query<LiveMoneyReadinessQuery>,
) -> Result<Json<LiveMoneyReadinessReport>, StatusCode> {
    let network = query
        .network
        .filter(|network| !network.trim().is_empty())
        .unwrap_or_else(|| "base-mainnet".to_string());
    build_live_money_readiness_report(live_money_readiness_config(&state, &network))
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn live_money_readiness_config(state: &SharedState, network: &str) -> LiveMoneyReadinessConfig {
    let descriptor = base_network_descriptor(network).ok();
    LiveMoneyReadinessConfig {
        network: network.to_string(),
        escrow_contract: descriptor
            .as_ref()
            .and_then(|descriptor| autonomous_factory_for_chain(descriptor.chain_id)),
        usdc_token: descriptor
            .as_ref()
            .and_then(base_usdc_token_for_chain)
            .or_else(|| descriptor.map(|descriptor| descriptor.native_usdc_token_address)),
        stripe_secret_key_mode: stripe_secret_key_mode_from_secret(
            state.stripe_secret_key.as_deref(),
        ),
        stripe_live_execution_enabled: state.stripe_live_execution_enabled,
        stripe_payment_method_configuration_configured: state
            .stripe_payment_method_configuration
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        stripe_webhook_secret_configured: state.stripe_webhook_secret.is_some(),
        allow_unsigned_stripe_webhooks: state.allow_unsigned_stripe_webhooks,
        operator_auth_configured: state.operator_api_token.is_some(),
        base_rpc_url_configured: state.base_rpc_urls.resolve(network).is_ok(),
        base_broadcast_enabled: state.base_broadcast_enabled,
    }
}

fn autonomous_factory_for_chain(chain_id: u64) -> Option<String> {
    match chain_id {
        84_532 => env_nonempty_value("BASE_SEPOLIA_BOUNTY_FACTORY"),
        8_453 => canonical_mainnet_factory(
            env_nonempty_value("BASE_MAINNET_BOUNTY_FACTORY"),
            env_nonempty_value("BASE_MAINNET_BOUNTY_IMPLEMENTATION"),
        ),
        _ => None,
    }
}

fn canonical_mainnet_factory(
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Option<String> {
    if configured_factory
        .as_deref()
        .is_some_and(|address| !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY))
        || configured_implementation.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION)
        })
    {
        None
    } else {
        Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string())
    }
}

fn base_usdc_token_for_chain(descriptor: &BaseNetworkDescriptor) -> Option<String> {
    let configured = match descriptor.chain_id {
        84_532 => env_nonempty_value("BASE_SEPOLIA_USDC_TOKEN"),
        8_453 => env_nonempty_value("BASE_MAINNET_USDC_TOKEN"),
        _ => None,
    };
    configured.or_else(|| Some(descriptor.native_usdc_token_address.clone()))
}

fn env_nonempty_value(name: &str) -> Option<String> {
    env::var(name).ok().and_then(non_empty_secret)
}

#[utoipa::path(get, path = "/v1/risk/events", responses((status = 200, body = Vec<RiskEvent>)))]
async fn list_risk_events(
    State(state): State<SharedState>,
    Query(filter): Query<RiskEventFilter>,
) -> Json<Vec<RiskEvent>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_risk_events(filter))
}

#[utoipa::path(get, path = "/v1/risk/reviews", responses((status = 200, body = Vec<RiskReviewRecord>)))]
async fn list_risk_reviews(State(state): State<SharedState>) -> Json<Vec<RiskReviewRecord>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_risk_reviews())
}

#[utoipa::path(
    post,
    path = "/v1/risk/bounty-approvals",
    responses(
        (status = 200, description = "Reviewed bounty approved into claimable state"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn approve_risk_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<ApproveRiskBountyRequest>,
) -> Result<Json<ReviewedBountyApproval>, StatusCode> {
    require_operator(&state, &headers)?;
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .approve_risk_bounty(request)
            .map(|approval| (approval, network.ledger.entries().to_vec()))
            .map_err(|_| StatusCode::BAD_REQUEST)
    };
    let (approval, ledger_entries) = result?;
    persist_reviewed_bounty_approval(&state, &approval, &ledger_entries).await?;
    Ok(Json(approval))
}

#[utoipa::path(
    post,
    path = "/v1/risk/payout-approvals",
    responses(
        (status = 200, body = RiskReviewRecord),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn approve_risk_payout(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<ApproveRiskPayoutRequest>,
) -> Result<Json<RiskReviewRecord>, StatusCode> {
    require_operator(&state, &headers)?;
    let review = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .approve_risk_payout(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    persist_risk_review(&state, &review).await?;
    Ok(Json(review))
}

#[utoipa::path(
    post,
    path = "/v1/risk/events/{id}/reject",
    responses(
        (status = 200, body = RiskReviewRecord),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reject_risk_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(mut request): Json<RejectRiskEventRequest>,
) -> Result<Json<RiskReviewRecord>, StatusCode> {
    require_operator(&state, &headers)?;
    request.risk_event_id = id;
    let review = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .reject_risk_event(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    persist_risk_review(&state, &review).await?;
    Ok(Json(review))
}

#[utoipa::path(post, path = "/v1/route-blocked-goal", request_body = RouteRequest, responses((status = 200, body = RouteDecision)))]
async fn route_blocked_goal(
    State(state): State<SharedState>,
    Json(request): Json<RouteRequest>,
) -> Json<RouteDecision> {
    let requester = Agent::new("api-requester");
    let help_request = HelpRequest::new(
        requester.id,
        request.goal,
        request.context,
        Money::new(request.budget_minor, request.currency).unwrap_or_else(|_| Money {
            amount: 1,
            currency: "usdc".to_string(),
        }),
        request.privacy,
    );

    let capabilities: Vec<Capability> = state
        .network
        .lock()
        .expect("state poisoned")
        .capabilities
        .values()
        .cloned()
        .collect();
    Json(BountyRouter::default().route_blocked_goal(&help_request, &capabilities))
}

#[utoipa::path(get, path = "/v1/evals/bountybench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_bountybench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = BountyBench::default()
        .run(&bundled_fixtures())
        .expect("bundled fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/abusebench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_abusebench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = AbuseBench::default()
        .run(&bundled_abuse_fixtures())
        .expect("bundled abuse fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/judgebench", responses((status = 200, body = EvalSuiteResult)))]
async fn run_judgebench(
    State(state): State<SharedState>,
) -> Result<Json<EvalSuiteResult>, StatusCode> {
    let result = JudgeBench::default()
        .run(&bundled_judge_fixtures())
        .expect("bundled judge fixtures pass");
    record_eval_run(&state, eval_run_from_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/loops", responses((status = 200, body = LoopSuiteResult)))]
async fn run_eval_loop_suite(
    State(state): State<SharedState>,
) -> Result<Json<LoopSuiteResult>, StatusCode> {
    let result = run_eval_loops().await.expect("bundled eval loops pass");
    record_eval_run(&state, eval_run_from_loop_suite(&result)).await?;
    Ok(Json(result))
}

#[utoipa::path(get, path = "/v1/evals/runs", responses((status = 200, body = Vec<EvalRun>)))]
async fn list_eval_runs(State(state): State<SharedState>) -> Json<Vec<EvalRun>> {
    let runs = state.eval_runs.lock().expect("state poisoned").clone();
    Json(runs)
}

#[utoipa::path(post, path = "/v1/agents")]
async fn register_agent(
    State(state): State<SharedState>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<domain::Agent>, StatusCode> {
    let agent = {
        let mut network = state.network.lock().expect("state poisoned");
        network.register_agent(request)
    };
    if let Some(store) = &state.store {
        store
            .upsert_agent(&agent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(agent))
}

#[utoipa::path(get, path = "/v1/agents/{id}/paid-status")]
async fn agent_paid_status(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let status = network
        .agent_payout_status(id)
        .map_err(|error| match error {
            app::AppError::AgentNotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        })?;
    let paid = status.payouts.iter().find(|payout| {
        payout.status == PayoutStatus::Paid && payout.rail != PaymentRail::Simulated
    });
    let evidence_payout = paid.or_else(|| status.payouts.first());
    let trigger = if paid.is_some() {
        Some(web_public::PostValueTrigger::ReconciledPayout)
    } else if evidence_payout.is_some() || !status.reputation_events.is_empty() {
        Some(web_public::PostValueTrigger::VerifiedCompletion)
    } else {
        None
    };
    let share_url = evidence_payout
        .map(|payout| {
            format!(
                "{}/public/proofs/{}",
                state.public_base_url.trim_end_matches('/'),
                payout.proof_record_id
            )
        })
        .unwrap_or_else(|| {
            format!(
                "{}/public/agents/{id}",
                state.public_base_url.trim_end_matches('/')
            )
        });
    let mut response =
        serde_json::to_value(status).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "post_value_loop".to_string(),
            trigger
                .map(|trigger| {
                    serde_json::to_value(web_public::post_value_loop(
                        Some(trigger),
                        Some(&share_url),
                    ))
                })
                .transpose()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .unwrap_or(serde_json::Value::Null),
        );
    }
    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/v1/contributor-contacts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = ContributorContact),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn upsert_contributor_contact(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpsertContributorContactRequest>,
) -> Result<Json<ContributorContact>, StatusCode> {
    require_operator(&state, &headers)?;
    let contact = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .upsert_contributor_contact(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_contributor_contact(&contact)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(contact))
}

#[utoipa::path(
    get,
    path = "/v1/contributor-contacts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<ContributorContact>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_contributor_contacts(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ContributorContact>>, StatusCode> {
    require_operator(&state, &headers)?;
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_contributor_contacts()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/members",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceMember),
        (status = 400, description = "Invalid public identity record"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn upsert_audience_member(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<UpsertAudienceMemberRequest>,
) -> Result<Json<AudienceMember>, StatusCode> {
    require_operator(&state, &headers)?;
    let member = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .upsert_audience_member(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_audience_member(&member)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(member))
}

#[utoipa::path(
    get,
    path = "/v1/audience/members",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<AudienceMember>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_audience_members(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AudienceMember>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_audience_members()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_audience_members()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/interactions",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceInteraction),
        (status = 400, description = "Invalid or unknown audience interaction"),
        (status = 409, description = "Provider event ID conflicts with an immutable stored event"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_audience_interaction(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordAudienceInteractionRequest>,
) -> Result<Json<AudienceInteraction>, StatusCode> {
    require_operator(&state, &headers)?;
    let (interaction, member) = {
        let mut network = state.network.lock().expect("state poisoned");
        let interaction = network
            .record_audience_interaction(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let member = network
            .audience_members
            .get(&interaction.audience_member_id)
            .cloned()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        (interaction, member)
    };
    if let Some(store) = &state.store {
        store
            .upsert_audience_interaction_with_member(&member, &interaction)
            .await
            .map_err(|error| match error {
                DbError::AudienceConflict(_) => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            })?;
    }
    Ok(Json(interaction))
}

#[utoipa::path(
    get,
    path = "/v1/audience/interactions",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<AudienceInteraction>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_audience_interactions(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AudienceInteraction>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_audience_interactions()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_audience_interactions()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/discovery-responses",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = DiscoveryResponse),
        (status = 400, description = "Response lacks a public source or private-storage consent"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_discovery_response(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordDiscoveryResponseRequest>,
) -> Result<Json<DiscoveryResponse>, StatusCode> {
    require_operator(&state, &headers)?;
    let response = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .record_discovery_response(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_discovery_response(&response)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/audience/discovery-responses",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<DiscoveryResponse>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_discovery_responses(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<DiscoveryResponse>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_discovery_responses()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_discovery_responses()))
}

#[utoipa::path(
    post,
    path = "/v1/audience/outreach-attempts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = OutreachAttempt),
        (status = 400, description = "Private outreach lacks explicit consent or public outreach lacks a URL"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn record_outreach_attempt(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<RecordOutreachAttemptRequest>,
) -> Result<Json<OutreachAttempt>, StatusCode> {
    require_operator(&state, &headers)?;
    let attempt = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .record_outreach_attempt(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_outreach_attempt(&attempt)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(attempt))
}

#[utoipa::path(
    get,
    path = "/v1/audience/outreach-attempts",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = Vec<OutreachAttempt>),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn list_outreach_attempts(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Vec<OutreachAttempt>>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        return store
            .list_outreach_attempts()
            .await
            .map(Json)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.list_outreach_attempts()))
}

#[utoipa::path(
    get,
    path = "/v1/audience/report",
    security(("operator_api_token" = []), ("operator_bearer" = [])),
    responses(
        (status = 200, body = AudienceReport),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    )
)]
async fn audience_report(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<AudienceReport>, StatusCode> {
    require_operator(&state, &headers)?;
    if let Some(store) = &state.store {
        let members = store
            .list_audience_members()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let interactions = store
            .list_audience_interactions()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let responses = store
            .list_discovery_responses()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let attempts = store
            .list_outreach_attempts()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        return Ok(Json(build_audience_report(
            &members,
            &interactions,
            &responses,
            &attempts,
        )));
    }
    let network = state.network.lock().expect("state poisoned");
    Ok(Json(network.audience_report()))
}

#[utoipa::path(
    post,
    path = "/v1/objectives/creation-plans",
    request_body = ObjectiveCreationDraft,
    responses(
        (status = 200, body = ObjectiveCreationPlan),
        (status = 400, description = "Invalid objective declaration or unsupported privacy claim")
    )
)]
async fn plan_objective_creation(
    Json(draft): Json<ObjectiveCreationDraft>,
) -> Result<Json<ObjectiveCreationPlan>, StatusCode> {
    Objective::plan_creation(draft)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives",
    request_body = SignedObjectiveCreation,
    responses(
        (status = 200, body = ObjectiveView),
        (status = 400, description = "Invalid declaration or wallet approval"),
        (status = 409, description = "Objective id already exists or plan is stale")
    )
)]
async fn create_objective(
    State(state): State<SharedState>,
    Json(request): Json<SignedObjectiveCreation>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let now = Utc::now();
    let objective = Objective::create(request, now).map_err(map_objective_error)?;
    persist_new_objective(&state, &objective).await?;
    Ok(Json(
        objective
            .view(&ObjectiveCanonicalEvidence::default(), now)
            .map_err(map_objective_error)?,
    ))
}

#[utoipa::path(
    get,
    path = "/v1/objectives",
    responses((status = 200, body = Vec<ObjectiveView>))
)]
async fn list_objectives(
    State(state): State<SharedState>,
) -> Result<Json<Vec<ObjectiveView>>, StatusCode> {
    let objectives = load_objectives(&state).await?;
    let evidence = load_objective_canonical_evidence(&state, &objectives).await?;
    let now = Utc::now();
    objectives
        .iter()
        .map(|objective| objective.view(&evidence, now).map_err(map_objective_error))
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

#[utoipa::path(
    get,
    path = "/v1/objectives/{id}",
    params(("id" = Uuid, Path, description = "Objective id")),
    responses(
        (status = 200, body = ObjectiveView),
        (status = 404, description = "Objective not found")
    )
)]
async fn get_objective(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let objective = load_objective(&state, id).await?;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    objective
        .view(&evidence, Utc::now())
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/action-plans",
    params(("id" = Uuid, Path, description = "Objective id")),
    request_body = ObjectiveAction,
    responses(
        (status = 200, body = ObjectiveActionPlan),
        (status = 404, description = "Objective or referenced record not found"),
        (status = 409, description = "Action is invalid in the current state")
    )
)]
async fn plan_objective_action(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(action): Json<ObjectiveAction>,
) -> Result<Json<ObjectiveActionPlan>, StatusCode> {
    let objective = load_objective(&state, id).await?;
    objective
        .plan_action(action, Utc::now())
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/actions",
    params(("id" = Uuid, Path, description = "Objective id")),
    request_body = SignedObjectiveAction,
    responses(
        (status = 200, body = ObjectiveView),
        (status = 400, description = "Invalid wallet approval"),
        (status = 404, description = "Objective or referenced record not found"),
        (status = 409, description = "Stale revision, invalid transition, or unmet readiness requirement")
    )
)]
async fn apply_objective_action(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(request): Json<SignedObjectiveAction>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    if request.plan.objective_id != id {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut objective = load_objective(&state, id).await?;
    let expected_revision = objective.revision;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    let now = Utc::now();
    objective
        .apply_action(request, now, &evidence)
        .map_err(map_objective_error)?;
    persist_objective_replacement(&state, &objective, expected_revision).await?;
    objective
        .view(&evidence, now)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(
    post,
    path = "/v1/objectives/{id}/reconcile",
    params(("id" = Uuid, Path, description = "Objective id")),
    responses(
        (status = 200, body = ObjectiveView, description = "Objective refreshed only from confirmed canonical bounty evidence"),
        (status = 404, description = "Objective not found"),
        (status = 409, description = "Concurrent objective update; reload and retry")
    )
)]
async fn reconcile_objective(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<ObjectiveView>, StatusCode> {
    let mut objective = load_objective(&state, id).await?;
    let expected_revision = objective.revision;
    let evidence =
        load_objective_canonical_evidence(&state, std::slice::from_ref(&objective)).await?;
    let now = Utc::now();
    if objective
        .reconcile_canonical_evidence(&evidence, now)
        .map_err(map_objective_error)?
    {
        persist_objective_replacement(&state, &objective, expected_revision).await?;
    }
    objective
        .view(&evidence, now)
        .map(Json)
        .map_err(map_objective_error)
}

#[utoipa::path(post, path = "/v1/capabilities")]
async fn register_capability(
    State(state): State<SharedState>,
    Json(request): Json<RegisterCapabilityRequest>,
) -> Result<Json<domain::Capability>, StatusCode> {
    let capability = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .register_capability(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        store
            .upsert_capability(&capability)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(capability))
}

#[utoipa::path(post, path = "/v1/help-requests")]
async fn create_help_request(
    State(state): State<SharedState>,
    Json(request): Json<CreateHelpRequestRequest>,
) -> Result<Json<domain::HelpRequest>, StatusCode> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .create_help_request(request)
            .map_err(|_| StatusCode::BAD_REQUEST)
    };
    let help_request = match result {
        Ok(help_request) => help_request,
        Err(status) => {
            persist_all_risk_events(&state).await?;
            return Err(status);
        }
    };
    if let Some(store) = &state.store {
        store
            .upsert_help_request(&help_request)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(help_request))
}

#[utoipa::path(post, path = "/v1/help-requests/{id}/quotes")]
async fn request_quotes(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<QuoteSet>, StatusCode> {
    let quote_set = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .request_quotes(RequestQuotesRequest {
                help_request_id: id,
            })
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        for quote in &quote_set.quotes {
            store
                .upsert_quote(quote)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
    }
    Ok(Json(quote_set))
}

#[utoipa::path(post, path = "/v1/quotes/{id}/fund-bounty")]
async fn fund_quote(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<FundQuoteRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    request.quote_id = id;
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .fund_quote_as_bounty(request)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
            .map_err(|_| StatusCode::BAD_REQUEST)
    };
    let (bounty, ledger_entries) = match result {
        Ok(result) => result,
        Err(status) => {
            persist_all_risk_events(&state).await?;
            return Err(status);
        }
    };
    persist_bounty_and_ledger(&state, &bounty, &ledger_entries).await?;
    Ok(Json(bounty))
}

#[utoipa::path(get, path = "/v1/bounties/claimable")]
async fn list_claimable_bounties(State(state): State<SharedState>) -> Json<Vec<domain::Bounty>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_claimable_bounties())
}

#[utoipa::path(get, path = "/v1/bounties/feed", responses((status = 200, description = "Public claimable bounty feed")))]
async fn public_bounty_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicBountyFeedItem>> {
    let bounties = {
        let network = state.network.lock().expect("state poisoned");
        network.list_claimable_bounties()
    };
    Json(web_public::public_bounty_feed(
        &bounties,
        &state.public_base_url,
    ))
}

#[utoipa::path(get, path = "/v1/bounties/funding-feed", responses((status = 200, description = "Public bounties that still need funding")))]
async fn public_funding_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicFundingFeedItem>> {
    let items = {
        let network = state.network.lock().expect("state poisoned");
        public_funding_feed_items(&network, &state.public_base_url)
    };
    Json(items)
}

#[utoipa::path(
    get,
    path = "/v1/x402/base/bounties/{bounty_contract}/funding",
    params(
        ("bounty_contract" = String, Path, description = "Canonical autonomous-v1 bounty contract"),
        ("network" = Option<String>, Query, description = "base-mainnet or base-sepolia"),
        ("amount" = Option<u64>, Query, description = "USDC base units; defaults to the remaining funding gap"),
        ("relayer" = Option<String>, Query, description = "Optional gas-paying Base address used in the returned transaction intent")
    ),
    responses(
        (status = 200, description = "Canonical FundingAdded confirmed; PAYMENT-RESPONSE contains the x402 settlement result"),
        (status = 202, description = "x402 envelope validated; the contract still verifies the EIP-3009 signature when the relay transaction is broadcast"),
        (status = 402, description = "PAYMENT-REQUIRED contains the exact x402 v2 funding challenge"),
        (status = 404, description = "Canonical indexed bounty not found"),
        (status = 409, description = "Bounty cannot accept the requested contribution"),
        (status = 413, description = "Requested amount exceeds the hosted relay cap"),
        (status = 422, description = "Authorization or hosted relay policy is invalid"),
        (status = 429, description = "Rolling hosted relay quota is exhausted"),
        (status = 503, description = "Hosted relayer or canonical RPC is temporarily unavailable")
    )
)]
async fn x402_base_bounty_funding(
    State(state): State<SharedState>,
    Path(bounty_contract): Path<String>,
    Query(query): Query<X402FundingQuery>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let requested_network = query.network.as_deref().unwrap_or("base-mainnet");
    let network =
        base_network_descriptor(requested_network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let network_key = match network.chain_id {
        8_453 => "base-mainnet",
        84_532 => "base-sepolia",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let caip2_network = format!("eip155:{}", network.chain_id);
    let bounty_contract =
        normalize_evm_address(&bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let item = indexed_autonomous_bounty(&state, network_key, &bounty_contract).await?;
    if !item.terms_valid {
        return Err(StatusCode::CONFLICT);
    }
    let amount = resolve_x402_funding_amount(
        &item.status,
        &item.target_amount,
        &item.funded_amount,
        query.amount,
    )?;
    if state.x402_relayer.enabled && amount > state.x402_relayer.max_amount {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if state.x402_relayer.enabled && amount < state.x402_relayer.min_amount {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let requested_relayer = query
        .relayer
        .as_deref()
        .map(normalize_evm_address)
        .transpose()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let hosted_relayer = state.x402_relayer.address();
    if state.x402_relayer.enabled
        && requested_relayer.as_deref().is_some_and(|requested| {
            hosted_relayer
                .as_deref()
                .is_none_or(|hosted| !requested.eq_ignore_ascii_case(hosted))
        })
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let relayer = hosted_relayer.or(requested_relayer);
    let mut resource_url = format!(
        "{}/v1/x402/base/bounties/{}/funding?network={network_key}&amount={amount}",
        state.public_base_url.trim_end_matches('/'),
        bounty_contract
    );
    if let Some(relayer) = &relayer {
        resource_url.push_str("&relayer=");
        resource_url.push_str(relayer);
    }
    let challenge = base_usdc_funding_challenge(
        resource_url,
        caip2_network,
        &network.native_usdc_token_address,
        &bounty_contract,
        amount,
        300,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let Some(header) = headers.get(PAYMENT_SIGNATURE_HEADER) else {
        return x402_payment_required_response(challenge);
    };
    let payload = match header
        .to_str()
        .map_err(|_| payments_x402::X402Error::InvalidBase64)
        .and_then(decode_payment_signature_header)
    {
        Ok(payload) => payload,
        Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
    };
    let now =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let authorization = match validate_funding_payload(&payload, &challenge, now) {
        Ok(authorization) => authorization,
        Err(error) => return x402_payment_required_error(challenge, &error.to_string()),
    };
    let contribution = AutonomousBountyContribution {
        bounty_contract: authorization.bounty_contract.clone(),
        contributor: authorization.contributor.clone(),
        amount: Money::new(authorization.amount as i64, "usdc")
            .map_err(|_| StatusCode::BAD_REQUEST)?,
        authorization_nonce: Some(authorization.nonce.clone()),
        authorization_valid_before: Some(authorization.valid_before),
    };
    let signature = AutonomousBountyAuthorizationSignature {
        v: authorization.v,
        r: authorization.r.clone(),
        s: authorization.s.clone(),
    };
    let plan = configured_autonomous_planner(network_key)?
        .plan_authorized_contribution(network_key, &contribution, &signature, relayer.as_deref())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if !state.x402_relayer.enabled {
        return x402_self_relay_response(&authorization, plan);
    }
    validate_hosted_x402_intent(
        &plan.relay_transaction,
        relayer.as_deref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
        &bounty_contract,
    )?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let request_fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    let attempt = store
        .reserve_x402_relay_attempt(
            &NewX402RelayAttempt {
                id: Uuid::new_v4(),
                idempotency_key: format!(
                    "x402:{network_key}:{}:{}",
                    authorization.bounty_contract, authorization.nonce
                ),
                network: network_key.to_string(),
                bounty_contract: authorization.bounty_contract,
                contributor: authorization.contributor,
                amount: authorization.amount,
                authorization_nonce: authorization.nonce,
                authorization_valid_before: authorization.valid_before,
                request_fingerprint,
                relayer_address: relayer.ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            },
            state.x402_relayer.max_daily_attempts,
            state.x402_relayer.max_daily_attempts_per_contributor,
        )
        .await
        .map_err(map_x402_db_error)?;
    let attempt = process_x402_hosted_relay(&state, attempt, &plan.relay_transaction).await?;
    x402_relay_response(&state, &attempt)
}

#[utoipa::path(
    get,
    path = "/v1/x402/base/relays/{relay_id}",
    params(("relay_id" = Uuid, Path, description = "Durable hosted x402 relay attempt ID")),
    responses(
        (status = 200, description = "Canonical FundingAdded confirmed"),
        (status = 202, description = "Relay is queued, broadcasting, or awaiting confirmation"),
        (status = 404, description = "Relay attempt not found")
    )
)]
async fn get_x402_relay(
    State(state): State<SharedState>,
    Path(relay_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut attempt = store
        .get_x402_relay_attempt(relay_id)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if attempt.status == X402RelayStatus::Broadcast {
        attempt = reconcile_x402_relay(&state, attempt).await?;
    }
    x402_relay_response(&state, &attempt)
}

fn x402_self_relay_response(
    authorization: &payments_x402::ValidatedFundingAuthorization,
    plan: chain_base::AutonomousBountyAuthorizedContributionPlan,
) -> Result<Response, StatusCode> {
    let mut response = (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "scheme": AGENT_BOUNTY_FUND_SCHEME,
            "status": "self_relay_required",
            "settled": false,
            "contributor": authorization.contributor,
            "bountyContract": authorization.bounty_contract,
            "amount": authorization.amount.to_string(),
            "authorizationNonce": authorization.nonce,
            "plan": plan,
            "nextStep": "Hosted relay is disabled. Simulate and broadcast plan.relay_transaction from the chosen gas-paying Base wallet, then wait for confirmed canonical FundingAdded.",
            "canonicalSuccessEvent": "FundingAdded",
            "paymentResponseHeaderPresent": false
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

fn validate_hosted_x402_intent(
    intent: &EvmTransactionIntent,
    relayer: &str,
    bounty_contract: &str,
) -> Result<(), StatusCode> {
    if intent.value_wei != 0
        || intent.function != AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION
        || !intent.to.eq_ignore_ascii_case(bounty_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    let calldata = intent
        .data
        .strip_prefix("0x")
        .ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;
    if calldata.len() != 8 + (8 * 64)
        || !calldata.starts_with(AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR)
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

async fn process_x402_hosted_relay(
    state: &SharedState,
    mut attempt: X402RelayAttempt,
    intent: &EvmTransactionIntent,
) -> Result<X402RelayAttempt, StatusCode> {
    if attempt.status == X402RelayStatus::Broadcast {
        return reconcile_x402_relay(state, attempt).await;
    }
    if attempt.status == X402RelayStatus::Confirmed
        || (attempt.status == X402RelayStatus::Failed && !attempt.retryable)
    {
        return Ok(attempt);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&attempt.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(lease_token) = store
        .acquire_x402_relayer_lease(&attempt.network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
    else {
        return store
            .get_x402_relay_attempt(attempt.id)
            .await
            .map_err(map_x402_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    };
    let claimed = store
        .claim_x402_relay_attempt(attempt.id, lease_token, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?;
    let Some(_claimed) = claimed else {
        store
            .release_x402_relayer_lease(&attempt.network, lease_token)
            .await
            .map_err(map_x402_db_error)?;
        return store
            .get_x402_relay_attempt(attempt.id)
            .await
            .map_err(map_x402_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    };
    let relay_result = match tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            base_network_descriptor(&attempt.network)
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .chain_id,
            intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(ChainBaseError::RelayerProvider(
            "relay RPC deadline exceeded".to_string(),
        )),
    };
    let persisted_result = match relay_result {
        Ok(transaction) => store
            .mark_x402_relay_broadcast(
                attempt.id,
                lease_token,
                &transaction.tx_hash,
                transaction.estimated_gas,
                transaction.gas_limit,
            )
            .await
            .map_err(map_x402_db_error),
        Err(error) => {
            let retryable = x402_relay_error_is_retryable(&error);
            store
                .mark_x402_relay_failed(
                    attempt.id,
                    Some(lease_token),
                    retryable,
                    "relay_rejected",
                    &error.to_string(),
                )
                .await
                .map_err(map_x402_db_error)
        }
    };
    let release_result = store
        .release_x402_relayer_lease(&attempt.network, lease_token)
        .await
        .map_err(map_x402_db_error);
    attempt = persisted_result?;
    release_result?;
    if attempt.status != X402RelayStatus::Broadcast {
        return Ok(attempt);
    }

    let deadline = Instant::now() + Duration::from_secs(state.x402_relayer.wait_seconds);
    loop {
        attempt = reconcile_x402_relay(state, attempt).await?;
        if attempt.status != X402RelayStatus::Broadcast || Instant::now() >= deadline {
            return Ok(attempt);
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn x402_relay_error_is_retryable(error: &ChainBaseError) -> bool {
    match error {
        ChainBaseError::InvalidRelayerPrivateKey
        | ChainBaseError::InvalidRelayIntent(_)
        | ChainBaseError::RelayerChainMismatch { .. } => false,
        ChainBaseError::RelayerProvider(message) => {
            !message.to_ascii_lowercase().contains("revert")
        }
        _ => true,
    }
}

async fn reconcile_x402_relay(
    state: &SharedState,
    attempt: X402RelayAttempt,
) -> Result<X402RelayAttempt, StatusCode> {
    tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        reconcile_x402_relay_inner(state, attempt),
    )
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
}

async fn reconcile_x402_relay_inner(
    state: &SharedState,
    attempt: X402RelayAttempt,
) -> Result<X402RelayAttempt, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let tx_hash = attempt
        .tx_hash
        .as_deref()
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(&attempt.network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let response = fetch_transaction_receipt(&rpc_url, tx_hash, 1)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(receipt) = response.result else {
        return Ok(attempt);
    };
    if receipt.succeeded().map_err(|_| StatusCode::BAD_GATEWAY)? == Some(false) {
        return store
            .mark_x402_relay_failed(
                attempt.id,
                None,
                false,
                "transaction_reverted",
                "The hosted relay transaction reverted; no canonical funding was applied.",
            )
            .await
            .map_err(map_x402_db_error);
    }
    let Some(block_number) = receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    else {
        return Ok(attempt);
    };
    let latest_block = fetch_block_number(&rpc_url, 2)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let observed_confirmations = latest_block.saturating_sub(block_number).saturating_add(1);
    if observed_confirmations < state.x402_relayer.confirmations {
        return Ok(attempt);
    }
    let confirmed_response = fetch_transaction_receipt(&rpc_url, tx_hash, 3)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let Some(confirmed_receipt) = confirmed_response.result else {
        return Ok(attempt);
    };
    if confirmed_receipt
        .block_number()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        != Some(block_number)
        || confirmed_receipt
            .succeeded()
            .map_err(|_| StatusCode::BAD_GATEWAY)?
            != Some(true)
    {
        return Ok(attempt);
    }
    let decoded = decode_autonomous_bounty_logs(
        confirmed_receipt
            .logs_to_evm_logs()
            .map_err(|_| StatusCode::BAD_GATEWAY)?,
    )
    .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let matching_events = decoded
        .into_iter()
        .filter(|event| {
            event
                .contract_address
                .eq_ignore_ascii_case(&attempt.bounty_contract)
                && event.tx_hash.eq_ignore_ascii_case(tx_hash)
        })
        .collect::<Vec<_>>();
    let funding_event = matching_events.iter().find(|event| {
        event.kind == chain_base::AutonomousBountyEventKind::FundingAdded
            && event.data["contributor"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(&attempt.contributor))
            && json_u128(&event.data["amount"]) == Some(u128::from(attempt.amount))
    });
    let Some(funding_event) = funding_event else {
        return store
            .mark_x402_relay_failed(
                attempt.id,
                None,
                false,
                "canonical_event_mismatch",
                "The confirmed transaction did not emit the exact canonical FundingAdded event.",
            )
            .await
            .map_err(map_x402_db_error);
    };
    let funding_event_id = funding_event.id;
    for event in &matching_events {
        store
            .upsert_autonomous_bounty_event(&attempt.network, event)
            .await
            .map_err(map_x402_db_error)?;
    }
    store
        .mark_x402_relay_confirmed(attempt.id, funding_event_id, block_number)
        .await
        .map_err(map_x402_db_error)
}

fn json_u128(value: &serde_json::Value) -> Option<u128> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn map_x402_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::X402RelayConflict(_) => StatusCode::CONFLICT,
        DbError::X402RelayQuotaExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn x402_relay_response(
    state: &SharedState,
    attempt: &X402RelayAttempt,
) -> Result<Response, StatusCode> {
    let status_url = format!(
        "{}/v1/x402/base/relays/{}",
        state.public_base_url.trim_end_matches('/'),
        attempt.id
    );
    if attempt.status == X402RelayStatus::Confirmed {
        let transaction = attempt
            .tx_hash
            .clone()
            .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
        let descriptor = base_network_descriptor(&attempt.network)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let settlement = SettlementResponse {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(attempt.contributor.clone()),
            transaction: transaction.clone(),
            network: format!("eip155:{}", descriptor.chain_id),
            amount: Some(attempt.amount.to_string()),
            extensions: None,
            extra: Some(BTreeMap::from([
                ("relayId".to_string(), serde_json::json!(attempt.id)),
                (
                    "canonicalEvent".to_string(),
                    serde_json::json!("FundingAdded"),
                ),
                (
                    "canonicalEventId".to_string(),
                    serde_json::json!(attempt.canonical_event_id),
                ),
                (
                    "confirmedBlock".to_string(),
                    serde_json::json!(attempt.confirmed_block),
                ),
            ])),
        };
        let encoded = encode_payment_response_header(&settlement)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut response = (
            StatusCode::OK,
            Json(serde_json::json!({
                "x402Version": X402_VERSION,
                "scheme": AGENT_BOUNTY_FUND_SCHEME,
                "status": "confirmed",
                "settled": true,
                "relay": x402_public_relay(attempt),
                "statusUrl": status_url,
                "settlement": settlement,
                "canonicalSuccessEvent": "FundingAdded",
                "paymentResponseHeaderPresent": true
            })),
        )
            .into_response();
        response.headers_mut().insert(
            HeaderName::from_static(PAYMENT_RESPONSE_HEADER),
            HeaderValue::from_str(&encoded).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, private"),
        );
        return Ok(response);
    }

    let (status, next_step) = match attempt.status {
        X402RelayStatus::Prepared | X402RelayStatus::Relaying => (
            StatusCode::ACCEPTED,
            "The hosted relay is queued. Poll statusUrl; do not treat this as funding.",
        ),
        X402RelayStatus::Broadcast => (
            StatusCode::ACCEPTED,
            "The transaction was broadcast. Poll statusUrl for confirmed canonical FundingAdded.",
        ),
        X402RelayStatus::Failed if attempt.retryable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "The relay failed before canonical funding and may be retried with the same signed request.",
        ),
        X402RelayStatus::Failed => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "The authorization did not produce canonical funding. Issue a new authorization after correcting the reported error.",
        ),
        X402RelayStatus::Confirmed => unreachable!("confirmed returned above"),
    };
    let mut response = (
        status,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "scheme": AGENT_BOUNTY_FUND_SCHEME,
            "status": x402_relay_status_name(attempt.status),
            "settled": false,
            "relay": x402_public_relay(attempt),
            "statusUrl": status_url,
            "nextStep": next_step,
            "canonicalSuccessEvent": "FundingAdded",
            "paymentResponseHeaderPresent": false
        })),
    )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

fn x402_public_relay(attempt: &X402RelayAttempt) -> serde_json::Value {
    serde_json::json!({
        "id": attempt.id,
        "network": attempt.network,
        "bountyContract": attempt.bounty_contract,
        "contributor": attempt.contributor,
        "amount": attempt.amount,
        "relayerAddress": attempt.relayer_address,
        "status": x402_relay_status_name(attempt.status),
        "retryable": attempt.retryable,
        "attemptCount": attempt.attempt_count,
        "transaction": attempt.tx_hash,
        "estimatedGas": attempt.estimated_gas,
        "gasLimit": attempt.gas_limit,
        "errorCode": attempt.error_code,
        "errorMessage": attempt.error_message,
        "canonicalEventId": attempt.canonical_event_id,
        "confirmedBlock": attempt.confirmed_block,
        "createdAt": attempt.created_at,
        "updatedAt": attempt.updated_at,
    })
}

fn x402_relay_status_name(status: X402RelayStatus) -> &'static str {
    match status {
        X402RelayStatus::Prepared => "prepared",
        X402RelayStatus::Relaying => "relaying",
        X402RelayStatus::Broadcast => "broadcast",
        X402RelayStatus::Confirmed => "confirmed",
        X402RelayStatus::Failed => "failed",
    }
}

fn x402_payment_required_error(
    mut challenge: PaymentRequired,
    error: &str,
) -> Result<Response, StatusCode> {
    challenge.error = Some(error.to_string());
    x402_payment_required_response(challenge)
}

fn resolve_x402_funding_amount(
    status: &str,
    target_amount: &str,
    funded_amount: &str,
    requested_amount: Option<u64>,
) -> Result<u64, StatusCode> {
    if status != "open" {
        return Err(StatusCode::CONFLICT);
    }
    let target = target_amount
        .parse::<u64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let funded = funded_amount
        .parse::<u64>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let remaining = target
        .checked_sub(funded)
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    let amount = requested_amount.unwrap_or(remaining);
    if remaining == 0 || amount == 0 || amount > remaining || amount > i64::MAX as u64 {
        return Err(StatusCode::CONFLICT);
    }
    Ok(amount)
}

fn x402_payment_required_response(challenge: PaymentRequired) -> Result<Response, StatusCode> {
    let encoded = encode_payment_required_header(&challenge)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut response = (
        StatusCode::PAYMENT_REQUIRED,
        Json(serde_json::json!({
            "x402Version": X402_VERSION,
            "status": "payment_required",
            "settled": false,
            "paymentRequired": challenge,
            "nextStep": "Sign the exact EIP-3009 authorization and retry this URL with the base64 x402 PaymentPayload in PAYMENT-SIGNATURE."
        })),
    )
        .into_response();
    response.headers_mut().insert(
        HeaderName::from_static(PAYMENT_REQUIRED_HEADER),
        HeaderValue::from_str(&encoded).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

#[utoipa::path(get, path = "/v1/capabilities/feed", responses((status = 200, description = "Public solver capability feed")))]
async fn public_capability_feed(
    State(state): State<SharedState>,
) -> Json<Vec<web_public::PublicCapabilityFeedItem>> {
    Json(capability_feed_from_state(&state))
}

#[utoipa::path(post, path = "/v1/capabilities/search", request_body = SearchCapabilitiesRequest, responses((status = 200, description = "Filtered public solver capability feed")))]
async fn search_capabilities(
    State(state): State<SharedState>,
    Json(request): Json<SearchCapabilitiesRequest>,
) -> Json<Vec<web_public::PublicCapabilityFeedItem>> {
    let mut feed = capability_feed_from_state(&state);
    filter_capability_feed(&mut feed, &request);
    Json(feed)
}

fn capability_feed_from_state(state: &SharedState) -> Vec<web_public::PublicCapabilityFeedItem> {
    let (capabilities, agents, reputation_events, settlements) = {
        let network = state.network.lock().expect("state poisoned");
        (
            network.capabilities.values().cloned().collect::<Vec<_>>(),
            network.agents.values().cloned().collect::<Vec<_>>(),
            network
                .reputation_events
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            network.settlements.values().cloned().collect::<Vec<_>>(),
        )
    };
    web_public::public_capability_feed(
        &capabilities,
        &agents,
        &reputation_events,
        &settlements,
        &state.public_base_url,
    )
}

fn filter_capability_feed(
    feed: &mut Vec<web_public::PublicCapabilityFeedItem>,
    request: &SearchCapabilitiesRequest,
) {
    let class_filter = request.class.as_ref().map(|class| format!("{class:?}"));
    let template_filter = request.template_slug.as_ref();
    let currency_filter = request
        .currency
        .as_ref()
        .map(|currency| currency.to_ascii_lowercase());
    feed.retain(|item| {
        class_filter
            .as_ref()
            .map(|class| item.class == *class)
            .unwrap_or(true)
            && template_filter
                .map(|template| item.template_slugs.iter().any(|slug| slug == template))
                .unwrap_or(true)
            && currency_filter
                .as_ref()
                .map(|currency| item.currency == *currency)
                .unwrap_or(true)
            && request
                .max_price_minor
                .map(|max_price| item.min_price_minor <= max_price)
                .unwrap_or(true)
    });
}

#[utoipa::path(post, path = "/v1/bounties")]
async fn post_bounty(
    State(state): State<SharedState>,
    Json(request): Json<PostBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .post_funded_bounty(request)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
            .map_err(|_| StatusCode::BAD_REQUEST)
    };
    let (bounty, ledger_entries) = match result {
        Ok(result) => result,
        Err(status) => {
            persist_all_risk_events(&state).await?;
            return Err(status);
        }
    };
    persist_bounty_and_ledger(&state, &bounty, &ledger_entries).await?;
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/pooled")]
async fn open_pooled_bounty(
    State(state): State<SharedState>,
    Json(request): Json<OpenPooledBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.open_pooled_bounty(request)
    };
    let bounty = match result {
        Ok(bounty) => bounty,
        Err(_) => {
            persist_all_risk_events(&state).await?;
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    persist_bounty_and_ledger(&state, &bounty, &[]).await?;
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/funding-intents")]
async fn create_funding_intent(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<CreateFundingIntentRequest>,
) -> Result<Json<FundingIntentReport>, StatusCode> {
    request.bounty_id = id;
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.create_funding_intent(request, state.public_base_url.clone())
    };
    let report = result.map_err(|_| StatusCode::BAD_REQUEST)?;
    persist_funding_intent_report(&state, &report).await?;
    Ok(Json(report))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/funding-contributions")]
async fn add_funding_contribution(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<AddFundingContributionRequest>,
) -> Result<Json<PooledFundingReport>, StatusCode> {
    request.bounty_id = id;
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.add_funding_contribution(request)
    };
    let report = result.map_err(|_| StatusCode::BAD_REQUEST)?;
    persist_pooled_funding_report(&state, &report).await?;
    Ok(Json(report))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/claim")]
async fn claim_bounty(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<ClaimBountyRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    request.bounty_id = id;
    let (bounty, claim) = {
        let mut network = state.network.lock().expect("state poisoned");
        let bounty = network
            .claim_bounty(request)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let claim = network
            .claims
            .values()
            .find(|claim| claim.bounty_id == bounty.id)
            .expect("claim exists after successful claim")
            .clone();
        (bounty, claim)
    };
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_claim(&claim)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(bounty))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/submit")]
async fn submit_result(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<SubmitResultRequest>,
) -> Result<Json<domain::Submission>, StatusCode> {
    request.bounty_id = id;
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .submit_result(request)
            .map(|submission| {
                let bounty = network
                    .bounties
                    .get(&submission.bounty_id)
                    .expect("submission bounty exists")
                    .clone();
                (submission, bounty)
            })
            .map_err(|_| StatusCode::BAD_REQUEST)
    };
    let (submission, bounty) = match result {
        Ok(result) => result,
        Err(status) => {
            persist_all_risk_events(&state).await?;
            return Err(status);
        }
    };
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_submission(&submission)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(Json(submission))
}

#[utoipa::path(post, path = "/v1/bounties/{id}/verify")]
async fn verify_submission(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(mut request): Json<VerifySubmissionRequest>,
) -> Result<Json<domain::ProofRecord>, StatusCode> {
    request.bounty_id = id;
    let mut network = {
        let mut guard = state.network.lock().expect("state poisoned");
        std::mem::take(&mut *guard)
    };
    let result = network.verify_submission(request).await;
    let (
        proof,
        bounty,
        verifier_result,
        settlements,
        funding_contributions,
        reputation_events,
        template_signals,
        ledger_entries,
    ) = match result {
        Ok(proof) => {
            let bounty = network
                .bounties
                .get(&proof.bounty_id)
                .expect("proof bounty exists")
                .clone();
            let verifier_result = network
                .verifier_results
                .get(&proof.verifier_result_id)
                .expect("proof verifier result exists")
                .clone();
            let settlements = network
                .settlements
                .values()
                .filter(|settlement| settlement.bounty_id == proof.bounty_id)
                .cloned()
                .collect::<Vec<_>>();
            let funding_contributions = network
                .funding_contributions
                .values()
                .filter(|contribution| contribution.bounty_id == proof.bounty_id)
                .cloned()
                .collect::<Vec<_>>();
            let reputation_events = network
                .reputation_events
                .values()
                .filter(|event| event.bounty_id == proof.bounty_id)
                .cloned()
                .collect::<Vec<_>>();
            let template_signals = network
                .template_signals
                .values()
                .filter(|signal| signal.bounty_id == proof.bounty_id)
                .cloned()
                .collect::<Vec<_>>();
            let ledger_entries = network.ledger.entries().to_vec();
            (
                proof,
                bounty,
                verifier_result,
                settlements,
                funding_contributions,
                reputation_events,
                template_signals,
                ledger_entries,
            )
        }
        Err(_) => {
            {
                let mut guard = state.network.lock().expect("state poisoned");
                *guard = network;
            }
            persist_all_risk_events(&state).await?;
            return Err(StatusCode::BAD_REQUEST);
        }
    };
    {
        let mut guard = state.network.lock().expect("state poisoned");
        *guard = network;
    }

    if let Some(store) = &state.store {
        store
            .upsert_bounty(&bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_verifier_result(&verifier_result)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_proof_record(&proof)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        for settlement in &settlements {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for contribution in &funding_contributions {
            store
                .upsert_funding_contribution(contribution)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for event in &reputation_events {
            store
                .upsert_reputation_event(event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for signal in &template_signals {
            store
                .upsert_template_signal(signal)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &ledger_entries).await?;
    }
    Ok(Json(proof))
}

#[utoipa::path(
    post,
    path = "/v1/base/broadcast-signed-transaction",
    request_body = BroadcastBaseSignedTransactionRequest,
    responses(
        (status = 200, description = "Broadcast a signed Base transaction through configured RPC"),
        (status = 400, description = "Invalid signed transaction request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 503, description = "Base transaction broadcast or RPC URL is not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn broadcast_base_signed_transaction(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<BroadcastBaseSignedTransactionRequest>,
) -> Result<Json<BaseSignedTransactionBroadcastReport>, StatusCode> {
    require_operator(&state, &headers)?;
    if !state.base_broadcast_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let request_id = request.request_id.unwrap_or(1);
    let network_name = request.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = state
        .base_rpc_urls
        .resolve(network_name)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let rpc_request = eth_send_raw_transaction_request(&request.signed_transaction, request_id)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let response = broadcast_signed_transaction(&rpc_url, &request.signed_transaction, request_id)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;

    Ok(Json(BaseSignedTransactionBroadcastReport {
        network,
        request: rpc_request,
        tx_hash: response.result,
        next_step:
            "Poll POST /v1/base/transaction-receipt for inclusion. The autonomous indexer independently reconciles canonical factory and bounty logs; a receipt alone never proves settlement."
                .to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/base/transaction-receipt",
    request_body = GetBaseTransactionReceiptRequest,
    responses(
        (status = 200, description = "Fetch a Base transaction receipt without mutating settlement state"),
        (status = 400, description = "Invalid receipt request"),
        (status = 503, description = "Requested Base RPC URL is not configured")
    )
)]
async fn get_base_transaction_receipt(
    State(state): State<SharedState>,
    Json(request): Json<GetBaseTransactionReceiptRequest>,
) -> Result<Json<BaseTransactionReceiptReport>, StatusCode> {
    let request_id = request.request_id.unwrap_or(1);
    let network_name = request.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = state
        .base_rpc_urls
        .resolve(network_name)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let rpc_request = eth_get_transaction_receipt_request(&request.tx_hash, request_id)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let tx_hash = rpc_request.params[0].clone();
    let response = fetch_transaction_receipt(&rpc_url, &tx_hash, request_id)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let Some(receipt) = response.result else {
        return Ok(Json(BaseTransactionReceiptReport {
            network,
            request: rpc_request,
            receipt_found: false,
            tx_hash,
            block_number: None,
            succeeded: None,
            log_count: 0,
            receipt: None,
        }));
    };

    let block_number = receipt
        .block_number()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let succeeded = receipt
        .succeeded()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let log_count = receipt.logs.len();

    Ok(Json(BaseTransactionReceiptReport {
        network,
        request: rpc_request,
        receipt_found: true,
        tx_hash,
        block_number,
        succeeded,
        log_count,
        receipt: Some(receipt),
    }))
}

fn base_rpc_fetch_status(error: &ChainBaseError) -> StatusCode {
    match error {
        ChainBaseError::UnknownNetwork(_)
        | ChainBaseError::InvalidAddress(_)
        | ChainBaseError::InvalidBlockRange { .. }
        | ChainBaseError::InvalidSignedTransaction(_)
        | ChainBaseError::InvalidTransactionHash(_) => StatusCode::BAD_REQUEST,
        ChainBaseError::MissingRpcUrl { .. } => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_GATEWAY,
    }
}

const CANONICAL_BASE_MAINNET_BOUNTY_FACTORY: &str = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
const CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION: &str =
    "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";

fn autonomous_planner_addresses(
    chain_id: u64,
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Result<(String, String), StatusCode> {
    let configured = |value: Option<String>| {
        value
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    let factory = configured(configured_factory);
    let implementation = configured(configured_implementation);
    if chain_id == 8_453 {
        if factory.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        }) || implementation.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION)
        }) {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        return Ok((
            CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string(),
            CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string(),
        ));
    }
    if chain_id == 84_532 {
        return Ok((
            factory.ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
            implementation.ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
        ));
    }
    Err(StatusCode::BAD_REQUEST)
}

fn configured_autonomous_planner(network: &str) -> Result<AutonomousBountyTxPlanner, StatusCode> {
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let (factory_env, implementation_env) = match descriptor.chain_id {
        8_453 => (
            "BASE_MAINNET_BOUNTY_FACTORY",
            "BASE_MAINNET_BOUNTY_IMPLEMENTATION",
        ),
        84_532 => (
            "BASE_SEPOLIA_BOUNTY_FACTORY",
            "BASE_SEPOLIA_BOUNTY_IMPLEMENTATION",
        ),
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let (factory, implementation) = autonomous_planner_addresses(
        descriptor.chain_id,
        env::var(factory_env).ok(),
        env::var(implementation_env).ok(),
    )?;
    AutonomousBountyTxPlanner::new(&factory, &implementation)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn require_indexed_canonical_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<(), StatusCode> {
    let item = indexed_autonomous_bounty(state, network, bounty_contract).await?;
    if item.terms_valid {
        Ok(())
    } else {
        Err(StatusCode::CONFLICT)
    }
}

async fn indexed_autonomous_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<AutonomousBountyFeedItem, StatusCode> {
    let Some(store) = &state.store else {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    };
    let planner = configured_autonomous_planner(network)?;
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let canonical_contracts = store
        .list_canonical_autonomous_bounty_contracts(network, &planner.factory_contract)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !canonical_contracts
        .iter()
        .any(|contract| contract.eq_ignore_ascii_case(bounty_contract))
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.recovery_reservations.apply(&mut feed, false);
    feed.into_iter()
        .find(|item| item.bounty_contract.eq_ignore_ascii_case(bounty_contract))
        .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/canonical-child-terms-plan", responses((status = 200, description = "Exact settled-child bounty terms and commitment plan")))]
async fn plan_autonomous_canonical_child_terms(
    Json(request): Json<CanonicalChildBountyTermsRequest>,
) -> Result<Json<CanonicalChildBountyTermsPlan>, StatusCode> {
    build_canonical_child_bounty_terms_plan(&request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/creation-plan", responses((status = 200, description = "Unsigned canonical autonomous bounty creation and initial-funding plan")))]
async fn plan_autonomous_bounty_creation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyCreationRequest>,
) -> Result<Json<AutonomousBountyCreationPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_autonomous_creation_terms(&state, network, &request.create).await?;
    configured_autonomous_planner(network)?
        .plan_creation(network, &request.create)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-creation-plan", responses((status = 200, description = "Relayer transaction plan after the creator signs Circle USDC EIP-3009 authorization")))]
async fn plan_autonomous_bounty_authorized_creation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedCreationRequest>,
) -> Result<Json<AutonomousBountyAuthorizedCreationPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_autonomous_creation_terms(&state, network, &request.create).await?;
    configured_autonomous_planner(network)?
        .plan_authorized_creation(
            network,
            &request.create,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn require_autonomous_creation_terms(
    state: &SharedState,
    network: &str,
    create: &AutonomousBountyCreate,
) -> Result<(), StatusCode> {
    let terms = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .get_autonomous_bounty_terms(&create.terms_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    validate_autonomous_creation_against_terms(network, create, &terms)
        .map_err(|_| StatusCode::CONFLICT)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/contribution-plan", responses((status = 200, description = "Unsigned permissionless pooled USDC contribution plan")))]
async fn plan_autonomous_bounty_contribution(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyContributionRequest>,
) -> Result<Json<AutonomousBountyContributionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.contribution.bounty_contract)
        .await?;
    configured_autonomous_planner(network)?
        .plan_contribution(network, &request.contribution)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-contribution-plan", responses((status = 200, description = "Single relayer transaction after a funder signs bounded Circle USDC authorization")))]
async fn plan_autonomous_bounty_authorized_contribution(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedContributionRequest>,
) -> Result<Json<AutonomousBountyAuthorizedContributionPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.contribution.bounty_contract)
        .await?;
    configured_autonomous_planner(network)?
        .plan_authorized_contribution(
            network,
            &request.contribution,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/claim-plan", responses((status = 200, description = "Wallet-batched USDC bond approval and direct claim plan")))]
async fn plan_autonomous_bounty_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyClaimRequest>,
) -> Result<Json<AutonomousBountyClaimPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    require_claimable_autonomous_item(&item)?;
    let claim_bond = item
        .claim_bond
        .parse::<u128>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    configured_autonomous_planner(network)?
        .plan_claim(
            network,
            &request.bounty_contract,
            &request.solver,
            claim_bond,
            request.authorization_nonce.as_deref(),
            request.authorization_valid_before,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/authorized-claim-plan", responses((status = 200, description = "Single relayer transaction after the solver signs the exact USDC claim bond authorization")))]
async fn plan_autonomous_bounty_authorized_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountyAuthorizedClaimRequest>,
) -> Result<Json<AutonomousBountyAuthorizedClaimPlan>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    require_claimable_autonomous_item(&item)?;
    let claim_bond = item
        .claim_bond
        .parse::<u128>()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    configured_autonomous_planner(network)?
        .plan_authorized_claim(
            network,
            &request.bounty_contract,
            &request.solver,
            claim_bond,
            &request.authorization_nonce,
            request.authorization_valid_before,
            &request.signature,
            request.relayer.as_deref(),
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn require_claimable_autonomous_item(item: &AutonomousBountyFeedItem) -> Result<(), StatusCode> {
    if !autonomous_bounty_is_earning_ready(item) {
        return Err(StatusCode::CONFLICT);
    }
    Ok(())
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-plan", responses((status = 200, description = "Unsigned submission commitment plan for autonomous verification")))]
async fn plan_autonomous_bounty_submission(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountySubmissionRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.bounty_contract).await?;
    configured_autonomous_planner(network)?
        .plan_submission(
            &request.bounty_contract,
            &request.solver,
            &request.submission_hash,
            &request.evidence_hash,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/autonomous-bounties/submission-preparation",
    responses(
        (status = 200, description = "Canonical active-claim validation, deterministic submission commitments, exact EIP-712 payload, and unsigned relay/evidence templates"),
        (status = 400, description = "Malformed wallet, artifact reference, evidence object, or network"),
        (status = 404, description = "Bounty contract is not an indexed canonical instance"),
        (status = 409, description = "Bounty is not an executable active claim owned by this solver or expires too soon"),
        (status = 503, description = "Canonical indexed state or planner configuration is unavailable")
    )
)]
async fn prepare_autonomous_bounty_submission(
    State(state): State<SharedState>,
    Json(request): Json<PrepareAutonomousBountySubmissionRequest>,
) -> Result<Json<AutonomousBountySubmissionPreparation>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    let observed_at_unix =
        u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    build_autonomous_submission_preparation(
        &configured_autonomous_planner(network)?,
        network,
        &item,
        &request.solver_wallet,
        &request.artifact_reference,
        request.evidence,
        observed_at_unix,
    )
    .map(Json)
    .map_err(|error| match error {
        ChainBaseError::InvalidSubmissionPreparation(_) => StatusCode::CONFLICT,
        ChainBaseError::InvalidSubmissionEvidence(_)
        | ChainBaseError::InvalidAddress(_)
        | ChainBaseError::InvalidCanonicalJson(_)
        | ChainBaseError::UnknownNetwork(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-authorization-plan", responses((status = 200, description = "Exact EIP-712 submission authorization for a gas-sponsored submitWithSignature relay")))]
async fn plan_autonomous_bounty_submission_authorization(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousBountySubmissionAuthorizationRequest>,
) -> Result<Json<AutonomousBountySubmissionAuthorizationTypedData>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    require_indexed_canonical_bounty(&state, network, &request.submission.bounty_contract).await?;
    configured_autonomous_planner(network)?
        .plan_submission_authorization(network, &request.submission)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/verification-attestation-plan", responses((status = 200, description = "Exact EIP-712 payload for one committed verifier to sign")))]
async fn plan_autonomous_verification_attestation(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousVerificationAttestationRequest>,
) -> Result<Json<AutonomousVerificationAttestationTypedData>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item =
        indexed_autonomous_bounty(&state, network, &request.attestation.bounty_contract).await?;
    let observed_at = u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_attestation_request_against_feed(&item, &request.attestation, observed_at)
        .map_err(|_| StatusCode::CONFLICT)?;
    configured_autonomous_planner(network)?
        .plan_verification_attestation(network, &request.attestation)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/module-settlement-plan", responses((status = 200, description = "Permissionless deterministic verifier call that atomically settles on pass")))]
async fn plan_autonomous_module_settlement(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousModuleSettlementRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    require_autonomous_item_mode(&item, "deterministic_module")?;
    configured_autonomous_planner(network)?
        .plan_module_settlement(
            &request.bounty_contract,
            request.caller.as_deref(),
            &request.proof,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/attestation-settlement-plan", responses((status = 200, description = "Permissionless committed verifier quorum relay that settles or reopens atomically")))]
async fn plan_autonomous_attestation_settlement(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousAttestationSettlementRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if !item.terms_valid {
        return Err(StatusCode::CONFLICT);
    }
    let mode = autonomous_item_mode(&item)?;
    if mode != "signed_quorum" && mode != "ai_judge_quorum" {
        return Err(StatusCode::CONFLICT);
    }
    let policy = item
        .terms
        .as_ref()
        .map(|terms| &terms.document.verification_policy)
        .ok_or(StatusCode::CONFLICT)?;
    let threshold = policy
        .get("threshold")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or(StatusCode::CONFLICT)?;
    if request.attestations.len() != threshold {
        return Err(StatusCode::CONFLICT);
    }
    let allowed = policy
        .get("verifiers")
        .and_then(serde_json::Value::as_array)
        .ok_or(StatusCode::CONFLICT)?;
    if request.attestations.iter().any(|attestation| {
        !allowed.iter().any(|value| {
            value
                .as_str()
                .is_some_and(|verifier| verifier.eq_ignore_ascii_case(&attestation.verifier))
        })
    }) {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_attestation_settlement(
            &request.bounty_contract,
            request.caller.as_deref(),
            &request.attestations,
        )
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/expire-claim-plan", responses((status = 200, description = "Permissionless expired-claim release plan")))]
async fn plan_autonomous_expire_claim(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "claimed" {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_expire_claim(&request.bounty_contract, request.caller.as_deref())
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/expire-submission-plan", responses((status = 200, description = "Permissionless expired-submission release plan")))]
async fn plan_autonomous_expire_submission(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "submitted" {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_expire_submission(&request.bounty_contract, request.caller.as_deref())
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/base/autonomous-bounties/timeout-relay",
    request_body = RelayAutonomousTimeoutRequest,
    responses(
        (status = 200, description = "Canonical timeout event confirmed", body = RelayAutonomousTimeoutResponse),
        (status = 202, description = "Bounded timeout transaction broadcast; confirmation pending", body = RelayAutonomousTimeoutResponse),
        (status = 404, description = "Canonical indexed bounty not found"),
        (status = 409, description = "Requested transition does not match the indexed bounty state or immutable deadline"),
        (status = 422, description = "Generated relay intent violated the bounded timeout policy"),
        (status = 503, description = "Hosted gas relayer, database lease, or Base RPC unavailable")
    )
)]
async fn relay_autonomous_timeout(
    State(state): State<SharedState>,
    Json(request): Json<RelayAutonomousTimeoutRequest>,
) -> Result<Response, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let descriptor = base_network_descriptor(network).map_err(|_| StatusCode::BAD_REQUEST)?;
    let network = match descriptor.chain_id {
        8_453 => "base-mainnet",
        84_532 => "base-sepolia",
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let bounty_contract =
        normalize_evm_address(&request.bounty_contract).map_err(|_| StatusCode::BAD_REQUEST)?;
    let item = indexed_autonomous_bounty(&state, network, &bounty_contract).await?;
    if !item.terms_valid || item.status != request.action.previous_bounty_state() {
        return Err(StatusCode::CONFLICT);
    }

    let relayer = state
        .x402_relayer
        .relayer
        .as_ref()
        .filter(|_| state.x402_relayer.enabled)
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relayer_address = relayer.address();
    let planner = configured_autonomous_planner(network)?;
    let intent = match request.action {
        AutonomousTimeoutAction::ExpireClaim => {
            planner.plan_expire_claim(&bounty_contract, Some(&relayer_address))
        }
        AutonomousTimeoutAction::ExpireSubmission => {
            planner.plan_expire_submission(&bounty_contract, Some(&relayer_address))
        }
    }
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_autonomous_timeout_intent(
        &intent,
        &bounty_contract,
        &relayer_address,
        request.action,
    )?;

    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let (_, rpc_url) = state
        .base_rpc_urls
        .resolve(network)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let lease_token = store
        .acquire_x402_relayer_lease(network, state.x402_relayer.lease_seconds)
        .await
        .map_err(map_x402_db_error)?
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let relay_result = tokio::time::timeout(
        Duration::from_secs(state.x402_relayer.rpc_timeout_seconds),
        relayer.simulate_and_broadcast(
            &rpc_url,
            descriptor.chain_id,
            &intent,
            state.x402_relayer.max_gas,
            state.x402_relayer.max_fee_per_gas_wei,
        ),
    )
    .await;
    let release_result = store
        .release_x402_relayer_lease(network, lease_token)
        .await
        .map_err(map_x402_db_error);
    let transaction = match relay_result {
        Ok(Ok(transaction)) => transaction,
        Ok(Err(error)) => return Err(timeout_relay_status(&error)),
        Err(_) => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    release_result?;

    let confirmation = wait_for_timeout_confirmation(
        &rpc_url,
        &bounty_contract,
        request.action,
        &transaction,
        state.x402_relayer.confirmations,
        state.x402_relayer.wait_seconds,
    )
    .await?;
    let response = RelayAutonomousTimeoutResponse {
        network: network.to_string(),
        bounty_contract,
        action: request.action,
        previous_bounty_state: request.action.previous_bounty_state().to_string(),
        expected_bounty_state: request.action.expected_bounty_state().to_string(),
        expected_canonical_event: request.action.expected_event_name().to_string(),
        transaction_hash: transaction.tx_hash,
        relayer: transaction.relayer,
        confirmed: confirmation.confirmed,
        confirmed_block: confirmation.confirmed_block,
        canonical_event_id: confirmation.canonical_event_id,
        evidence_boundary: format!(
            "Only a confirmed {} event proves this timeout transition. It is bond/lifecycle evidence, not bounty payout or BountySettled evidence.",
            request.action.expected_event_name()
        ),
    };
    Ok((
        if response.confirmed {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(response),
    )
        .into_response())
}

impl AutonomousTimeoutAction {
    fn previous_bounty_state(self) -> &'static str {
        match self {
            Self::ExpireClaim => "claimed",
            Self::ExpireSubmission => "submitted",
        }
    }

    fn expected_bounty_state(self) -> &'static str {
        "claimable"
    }

    fn function(self) -> &'static str {
        match self {
            Self::ExpireClaim => "expireClaim()",
            Self::ExpireSubmission => "expireSubmission()",
        }
    }

    fn calldata(self) -> &'static str {
        match self {
            Self::ExpireClaim => "0x1257d2c8",
            Self::ExpireSubmission => "0xf9251ec7",
        }
    }

    fn expected_event_name(self) -> &'static str {
        match self {
            Self::ExpireClaim => "ClaimExpired",
            Self::ExpireSubmission => "SubmissionExpired",
        }
    }

    fn expected_event_kind(self) -> AutonomousBountyEventKind {
        match self {
            Self::ExpireClaim => AutonomousBountyEventKind::ClaimExpired,
            Self::ExpireSubmission => AutonomousBountyEventKind::SubmissionExpired,
        }
    }
}

fn validate_autonomous_timeout_intent(
    intent: &EvmTransactionIntent,
    bounty_contract: &str,
    relayer: &str,
    action: AutonomousTimeoutAction,
) -> Result<(), StatusCode> {
    if intent.value_wei != 0
        || intent.function != action.function()
        || !intent.to.eq_ignore_ascii_case(bounty_contract)
        || intent
            .from
            .as_deref()
            .is_none_or(|from| !from.eq_ignore_ascii_case(relayer))
    {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    if !intent.data.eq_ignore_ascii_case(action.calldata()) {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }
    Ok(())
}

fn timeout_relay_status(error: &ChainBaseError) -> StatusCode {
    match error {
        ChainBaseError::RelayerProvider(message)
            if message.to_ascii_lowercase().contains("revert") =>
        {
            StatusCode::CONFLICT
        }
        ChainBaseError::InvalidRelayIntent(_)
        | ChainBaseError::RelayerChainMismatch { .. }
        | ChainBaseError::RelayerGasLimitExceeded { .. }
        | ChainBaseError::RelayerFeeCapExceeded { .. } => StatusCode::UNPROCESSABLE_ENTITY,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[derive(Debug)]
struct TimeoutRelayConfirmation {
    confirmed: bool,
    confirmed_block: Option<u64>,
    canonical_event_id: Option<String>,
}

async fn wait_for_timeout_confirmation(
    rpc_url: &str,
    bounty_contract: &str,
    action: AutonomousTimeoutAction,
    transaction: &BaseRelayedTransaction,
    required_confirmations: u64,
    wait_seconds: u64,
) -> Result<TimeoutRelayConfirmation, StatusCode> {
    let deadline = Instant::now() + Duration::from_secs(wait_seconds);
    loop {
        let receipt = fetch_transaction_receipt(rpc_url, &transaction.tx_hash, 1)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .result;
        if let Some(receipt) = receipt {
            if receipt
                .succeeded()
                .map_err(|error| base_rpc_fetch_status(&error))?
                == Some(false)
            {
                return Err(StatusCode::BAD_GATEWAY);
            }
            let block_number = receipt
                .block_number()
                .map_err(|error| base_rpc_fetch_status(&error))?
                .ok_or(StatusCode::BAD_GATEWAY)?;
            let events = decode_autonomous_bounty_logs(
                receipt
                    .logs_to_evm_logs()
                    .map_err(|error| base_rpc_fetch_status(&error))?,
            )
            .map_err(|error| base_rpc_fetch_status(&error))?;
            let event = events.into_iter().find(|event| {
                event.kind == action.expected_event_kind()
                    && event.contract_address.eq_ignore_ascii_case(bounty_contract)
            });
            let event = event.ok_or(StatusCode::BAD_GATEWAY)?;
            let latest_block = fetch_block_number(rpc_url, 2)
                .await
                .map_err(|error| base_rpc_fetch_status(&error))?;
            let confirmations = latest_block.saturating_sub(block_number).saturating_add(1);
            if confirmations >= required_confirmations {
                return Ok(TimeoutRelayConfirmation {
                    confirmed: true,
                    confirmed_block: Some(block_number),
                    canonical_event_id: Some(event.log_key),
                });
            }
        }
        if Instant::now() >= deadline {
            return Ok(TimeoutRelayConfirmation {
                confirmed: false,
                confirmed_block: None,
                canonical_event_id: None,
            });
        }
        sleep(Duration::from_secs(1)).await;
    }
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/cancel-plan", responses((status = 200, description = "Creator or post-deadline cancellation plan")))]
async fn plan_autonomous_cancel(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "open" && item.status != "claimable" {
        return Err(StatusCode::CONFLICT);
    }
    configured_autonomous_planner(network)?
        .plan_cancel(&request.bounty_contract, request.caller.as_deref())
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/refund-withdrawal-plan", responses((status = 200, description = "Contributor pull-refund transaction plan after cancellation")))]
async fn plan_autonomous_refund_withdrawal(
    State(state): State<SharedState>,
    Json(request): Json<PlanAutonomousLifecycleRequest>,
) -> Result<Json<EvmTransactionIntent>, StatusCode> {
    let network = request.network.as_deref().unwrap_or("base-mainnet");
    let item = indexed_autonomous_bounty(&state, network, &request.bounty_contract).await?;
    if item.status != "cancelled" {
        return Err(StatusCode::CONFLICT);
    }
    let contributor = request.caller.as_deref().ok_or(StatusCode::BAD_REQUEST)?;
    configured_autonomous_planner(network)?
        .plan_refund_withdrawal(&request.bounty_contract, contributor)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn autonomous_item_mode(item: &AutonomousBountyFeedItem) -> Result<&str, StatusCode> {
    item.terms
        .as_ref()
        .and_then(|terms| terms.document.verification_policy.get("mechanism"))
        .and_then(serde_json::Value::as_str)
        .ok_or(StatusCode::CONFLICT)
}

fn require_autonomous_item_mode(
    item: &AutonomousBountyFeedItem,
    expected: &str,
) -> Result<(), StatusCode> {
    if item.terms_valid && item.status == "submitted" && autonomous_item_mode(item)? == expected {
        Ok(())
    } else {
        Err(StatusCode::CONFLICT)
    }
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/decode-events", responses((status = 200, description = "Decoded autonomous factory, funding, claim, submission, settlement, and refund evidence")))]
async fn decode_autonomous_bounty_events(
    Json(request): Json<DecodeAutonomousBountyEventsRequest>,
) -> Result<Json<Vec<AutonomousBountyEvent>>, StatusCode> {
    decode_autonomous_bounty_logs(request.logs)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/events", responses((status = 200, description = "Persisted confirmed autonomous bounty events")))]
async fn list_autonomous_bounty_events(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyEventsQuery>,
) -> Result<Json<Vec<AutonomousBountyEvent>>, StatusCode> {
    let Some(store) = &state.store else {
        return Ok(Json(Vec::new()));
    };
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let mut events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(bounty_id) = query.bounty_id {
        events.retain(|event| event.bounty_id.eq_ignore_ascii_case(&bounty_id));
    }
    Ok(Json(events))
}

fn autonomous_terms_record(
    request: PublishAutonomousBountyTermsRequest,
) -> Result<AutonomousBountyTermsRecord, StatusCode> {
    build_autonomous_bounty_terms_record(&request.creator_wallet, request.document, Utc::now())
        .map_err(|error| match error {
            ChainBaseError::TermsDocumentTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::BAD_REQUEST,
        })
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/terms", responses((status = 200, description = "Content-addressed public bounty terms and contract hash commitments")))]
async fn publish_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Json(request): Json<PublishAutonomousBountyTermsRequest>,
) -> Result<Json<AutonomousBountyTermsRecord>, StatusCode> {
    let record = autonomous_terms_record(request)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .upsert_autonomous_bounty_terms(&record)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(record))
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/terms/{terms_hash}", params(("terms_hash" = String, Path, description = "0x-prefixed Keccak hash returned by terms publication and committed on-chain")), responses((status = 200, description = "Canonical public bounty terms"), (status = 404, description = "Unknown terms hash")))]
async fn get_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Path(terms_hash): Path<String>,
) -> Result<Json<AutonomousBountyTermsRecord>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .get_autonomous_bounty_terms(&terms_hash)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[utoipa::path(post, path = "/v1/base/autonomous-bounties/submission-evidence", responses((status = 200, description = "Immutable public preimages matching the current canonical SubmissionAdded hashes")))]
async fn publish_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Json(request): Json<PublishAutonomousSubmissionEvidenceRequest>,
) -> Result<Json<AutonomousSubmissionEvidenceRecord>, StatusCode> {
    let network = request
        .network
        .clone()
        .unwrap_or_else(|| "base-mainnet".to_string());
    let item = indexed_autonomous_bounty(&state, &network, &request.bounty_contract).await?;
    let record = autonomous_submission_evidence_record(&network, &item, request)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .upsert_autonomous_submission_evidence(&record)
        .await
        .map(Json)
        .map_err(|error| match error {
            DbError::AutonomousEvidenceConflict(_) => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/submission-evidence/{bounty_contract}/{round}", params(("bounty_contract" = String, Path, description = "Canonical bounty contract"), ("round" = u64, Path, description = "Positive submission round")), responses((status = 200, description = "Hash-checked public submission evidence"), (status = 404, description = "Evidence not published")))]
async fn get_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Path((bounty_contract, round)): Path<(String, u64)>,
    Query(query): Query<AutonomousSubmissionEvidenceQuery>,
) -> Result<Json<AutonomousSubmissionEvidenceRecord>, StatusCode> {
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    indexed_autonomous_bounty(&state, network, &bounty_contract).await?;
    state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?
        .get_autonomous_submission_evidence(network, &bounty_contract, round)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

fn autonomous_submission_evidence_record(
    network: &str,
    item: &AutonomousBountyFeedItem,
    request: PublishAutonomousSubmissionEvidenceRequest,
) -> Result<AutonomousSubmissionEvidenceRecord, StatusCode> {
    build_autonomous_submission_evidence_record(
        network,
        item,
        &request.bounty_contract,
        &request.bounty_id,
        request.round,
        &request.solver_wallet,
        &request.artifact_reference,
        request.evidence,
        Utc::now(),
    )
    .map_err(|_| StatusCode::CONFLICT)
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/feed", responses((status = 200, description = "Canonical on-chain bounties joined to content-addressed public terms")))]
async fn autonomous_bounty_feed(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousBountyFeedQuery>,
) -> Result<Json<Vec<AutonomousBountyFeedItem>>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .recovery_reservations
        .apply(&mut feed, query.claimable_only.unwrap_or(false));
    Ok(Json(feed))
}

#[utoipa::path(get, path = "/v1/base/autonomous-bounties/verification-jobs", responses((status = 200, description = "Live verifier jobs joined to immutable terms and hash-matched evidence preimages")))]
async fn autonomous_verification_jobs(
    State(state): State<SharedState>,
    Query(query): Query<AutonomousVerificationJobsQuery>,
) -> Result<Json<Vec<AutonomousVerificationJob>>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let network = query.network.as_deref().unwrap_or("base-mainnet");
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let evidence = store
        .list_autonomous_submission_evidence(network)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut feed = build_autonomous_bounty_feed(events, terms, false)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .recovery_reservations
        .exclude_from_verification_jobs(&mut feed);
    let observed_at = u64::try_from(Utc::now().timestamp()).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut jobs = build_autonomous_verification_jobs(network, feed, evidence, observed_at)
        .map_err(|_| StatusCode::CONFLICT)?;
    if let Some(verifier) = query.verifier {
        let verifier = normalize_evm_address(verifier).map_err(|_| StatusCode::BAD_REQUEST)?;
        jobs.retain(|job| {
            job.verification_mode == "deterministic_module"
                || job
                    .eligible_verifiers
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&verifier))
        });
    }
    Ok(Json(jobs))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/checkout-top-ups",
    request_body = PlanStripeCheckoutTopUpRequest,
    responses(
        (status = 200, description = "Stripe Checkout Session request intent"),
        (status = 400, description = "Invalid top-up request or amount below Stripe minimum")
    )
)]
async fn plan_stripe_checkout_top_up(
    State(state): State<SharedState>,
    Json(request): Json<PlanStripeCheckoutTopUpRequest>,
) -> Result<Json<payments_stripe::StripeRequestIntent>, StatusCode> {
    stripe_checkout_top_up_intent(&state, request).map(Json)
}

fn stripe_checkout_top_up_intent(
    state: &SharedState,
    request: PlanStripeCheckoutTopUpRequest,
) -> Result<StripeRequestIntent, StatusCode> {
    let PlanStripeCheckoutTopUpRequest {
        organization_id,
        amount_minor,
        currency,
        success_url,
        cancel_url,
    } = request;
    let platform_base_url = state.public_base_url.clone();
    let planner = stripe_planner_for_state(state, platform_base_url.clone());
    let amount = Money::new(amount_minor, currency).map_err(|_| StatusCode::BAD_REQUEST)?;
    planner
        .checkout_top_up(&CheckoutTopUpRequest {
            organization_id,
            amount,
            success_url: success_url
                .unwrap_or_else(|| format!("{platform_base_url}/stripe/success")),
            cancel_url: cancel_url.unwrap_or_else(|| format!("{platform_base_url}/stripe/cancel")),
        })
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn stripe_planner_for_state(
    state: &SharedState,
    platform_base_url: impl Into<String>,
) -> StripePlanner {
    let planner = StripePlanner::new(platform_base_url);
    if let Some(payment_method_configuration) = state.stripe_payment_method_configuration.as_deref()
    {
        planner.with_payment_method_configuration(payment_method_configuration)
    } else {
        planner
    }
}

fn apply_state_checkout_payment_method_configuration(
    state: &SharedState,
    intent: &mut StripeRequestIntent,
) -> Result<(), StatusCode> {
    apply_checkout_payment_method_configuration(
        intent,
        state.stripe_payment_method_configuration.as_deref(),
    )
    .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-accounts",
    request_body = PlanStripeConnectAccountRequest,
    responses(
        (status = 200, description = "Stripe Accounts v2 create request intent"),
        (status = 400, description = "Invalid Connect account planning request")
    )
)]
async fn plan_stripe_connect_account(
    Json(request): Json<PlanStripeConnectAccountRequest>,
) -> Result<Json<payments_stripe::ConnectAccountV2CreateIntent>, StatusCode> {
    stripe_connect_account_intent(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

fn stripe_connect_account_intent(
    request: PlanStripeConnectAccountRequest,
) -> Result<payments_stripe::ConnectAccountV2CreateIntent, payments_stripe::StripeIntegrationError>
{
    StripePlanner::new("http://127.0.0.1:8080").connect_account_v2(request.agent_id)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-transfers",
    request_body = PlanStripeConnectTransferRequest,
    responses(
        (status = 200, description = "Stripe Connect transfer request intent for a fiat payout"),
        (status = 400, description = "Invalid payout intent or transfer planning request")
    )
)]
async fn plan_stripe_connect_transfer(
    State(state): State<SharedState>,
    Json(request): Json<PlanStripeConnectTransferRequest>,
) -> Result<Json<StripeTransferPlan>, StatusCode> {
    stripe_connect_transfer_plan(&state, request).map(Json)
}

fn stripe_connect_transfer_plan(
    state: &SharedState,
    request: PlanStripeConnectTransferRequest,
) -> Result<StripeTransferPlan, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_stripe_transfer(
            AppPlanStripeTransferRequest {
                payout_intent_id: request.payout_intent_id,
                connected_account_id: request.connected_account_id,
            },
            state.public_base_url.clone(),
        )
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/checkout-top-ups",
    request_body = PlanStripeCheckoutTopUpRequest,
    responses(
        (status = 200, description = "Live Stripe Checkout Session execution report"),
        (status = 400, description = "Invalid top-up request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_checkout_top_up(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeCheckoutTopUpRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let intent = stripe_checkout_top_up_intent(&state, request)?;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/funding-intents/{id}/checkout-session",
    params(("id" = Uuid, Path, description = "Stripe fiat funding intent id")),
    responses(
        (status = 200, description = "Live Stripe Checkout Session execution report for a bounty funding intent"),
        (status = 400, description = "Unknown, non-Stripe, already-applied, or invalid funding intent"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Public Stripe Checkout execution is disabled or live Stripe execution is not configured")
    )
)]
async fn execute_stripe_funding_intent_checkout(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    if !state.stripe_public_checkout_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let intent = {
        let network = state.network.lock().expect("state poisoned");
        network.stripe_checkout_for_funding_intent(id, state.public_base_url.clone())
    }
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut intent = intent;
    apply_state_checkout_payment_method_configuration(&state, &mut intent)?;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/connect-accounts",
    request_body = PlanStripeConnectAccountRequest,
    responses(
        (status = 200, description = "Live Stripe Accounts v2 execution report"),
        (status = 400, description = "Invalid Connect request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_connect_account(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeConnectAccountRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let intent = stripe_connect_account_intent(request)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .request;
    execute_stripe_intent(&state, intent).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/stripe/live/connect-transfers",
    request_body = PlanStripeConnectTransferRequest,
    responses(
        (status = 200, description = "Live Stripe Connect transfer execution report"),
        (status = 400, description = "Invalid transfer request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 502, description = "Stripe API execution failed"),
        (status = 503, description = "Live Stripe execution is disabled or not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn execute_stripe_connect_transfer(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanStripeConnectTransferRequest>,
) -> Result<Json<StripeExecutionReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let plan = stripe_connect_transfer_plan(&state, request)?;
    execute_stripe_intent(&state, plan.request).await.map(Json)
}

async fn execute_stripe_intent(
    state: &SharedState,
    intent: StripeRequestIntent,
) -> Result<StripeExecutionReport, StatusCode> {
    if !state.stripe_live_execution_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let secret_key = state
        .stripe_secret_key
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    execute_stripe_request(&intent, secret_key, &state.stripe_api_base_url)
        .await
        .map_err(stripe_execution_status)
}

fn stripe_execution_status(error: payments_stripe::StripeIntegrationError) -> StatusCode {
    match error {
        payments_stripe::StripeIntegrationError::RequestFailed { .. }
        | payments_stripe::StripeIntegrationError::HttpTransport(_) => StatusCode::BAD_GATEWAY,
        _ => StatusCode::BAD_REQUEST,
    }
}

#[utoipa::path(
    post,
    path = "/v1/stripe/connect-snapshots",
    responses(
        (status = 200, description = "Reconciled Stripe Connect payout eligibility snapshot"),
        (status = 400, description = "Invalid Connect snapshot"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reconcile_stripe_connect_snapshot(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(snapshot): Json<ConnectAccountSnapshot>,
) -> Result<Json<app::StripeConnectPayoutReconciliation>, StatusCode> {
    require_operator(&state, &headers)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_connect_snapshot(snapshot)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    if let Some(store) = &state.store {
        for bounty in &reconciliation.bounties {
            store
                .upsert_bounty(bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for settlement in &reconciliation.settlements {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }
    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/transfer-events",
    responses(
        (status = 200, description = "Reconciled Stripe transfer event as fiat payout evidence"),
        (status = 400, description = "Invalid transfer event payload or signature"),
        (status = 503, description = "Webhook signature verification is not configured")
    )
)]
async fn reconcile_stripe_transfer_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<StripeTransferReconciliation>, StatusCode> {
    match &state.stripe_webhook_secret {
        Some(secret) => {
            let signature = headers
                .get("stripe-signature")
                .and_then(|value| value.to_str().ok())
                .ok_or(StatusCode::BAD_REQUEST)?;
            verify_webhook_signature(&body, signature, secret)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        None if state.allow_unsigned_stripe_webhooks => {}
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
    }
    let event: StripeWebhookEvent =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let evidence = StripeEventDeduper::default()
        .apply_connect_transfer(&event)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_transfer_evidence(evidence)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };

    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            store
                .upsert_payment_event(&reconciliation.evidence.payment_event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(settlement) = &reconciliation.settlement {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(bounty) = &reconciliation.bounty {
            store
                .upsert_bounty(bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }
    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/stripe/checkout-webhooks",
    responses(
        (status = 200, description = "Reconciled paid Stripe Checkout top-up webhook"),
        (status = 400, description = "Invalid webhook payload or signature"),
        (status = 503, description = "Webhook signature verification is not configured")
    )
)]
async fn reconcile_stripe_checkout_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<app::StripeFundingReconciliation>, StatusCode> {
    match &state.stripe_webhook_secret {
        Some(secret) => {
            let signature = headers
                .get("stripe-signature")
                .and_then(|value| value.to_str().ok())
                .ok_or(StatusCode::BAD_REQUEST)?;
            verify_webhook_signature(&body, signature, secret)
                .map_err(|_| StatusCode::BAD_REQUEST)?;
        }
        None if state.allow_unsigned_stripe_webhooks => {}
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
    }
    let event: StripeWebhookEvent =
        serde_json::from_slice(&body).map_err(|_| StatusCode::BAD_REQUEST)?;
    let funding_credit = StripeEventDeduper::default()
        .apply_checkout_top_up(&event)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .apply_stripe_funding_credit(funding_credit)
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };

    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            store
                .upsert_payment_event(&reconciliation.funding_credit.payment_event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(intent) = &reconciliation.funding_intent {
            store
                .upsert_funding_intent(intent)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        if let Some(report) = &reconciliation.funding_report {
            store
                .upsert_bounty(&report.bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            store
                .upsert_funding_contribution(&report.contribution)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &reconciliation.ledger_entries).await?;
    }

    Ok(Json(reconciliation))
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-bounty-plan",
    request_body = PlanGitHubIssueBountyRequest,
    responses((status = 200, description = "GitHub issue-form bounty parse and check-run plan"))
)]
async fn plan_github_issue_bounty(
    Json(request): Json<PlanGitHubIssueBountyRequest>,
) -> Json<GitHubIssueBountyPlan> {
    Json(github_issue_bounty_plan(request))
}

fn github_issue_bounty_plan(request: PlanGitHubIssueBountyRequest) -> GitHubIssueBountyPlan {
    let parsed = parse_issue_form_bounty(
        &request.repository,
        &request.issue_url,
        &request.title,
        &request.body,
    );
    match parsed {
        Ok(bounty) => GitHubIssueBountyPlan {
            ready: true,
            check: bounty_check_output(Ok(&bounty)),
            parsed: Some(bounty),
            error: None,
        },
        Err(error) => GitHubIssueBountyPlan {
            ready: false,
            check: bounty_check_output(Err(&error)),
            parsed: None,
            error: Some(error.to_string()),
        },
    }
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-api-sync-plan",
    request_body = PlanGitHubIssueApiSyncRequest,
    responses((status = 200, description = "GitHub issue to hosted API bounty sync plan"))
)]
async fn plan_github_issue_api_sync(
    Json(request): Json<PlanGitHubIssueApiSyncRequest>,
) -> Json<GitHubIssueApiSyncPlan> {
    Json(issue_api_sync_plan(GitHubIssueApiSyncInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        api_base_url: request.api_base_url,
        existing_bounty_ids: request.existing_bounty_ids,
        hosted_api_error: request.hosted_api_error,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/issue-api-sync",
    request_body = PlanGitHubIssueApiSyncRequest,
    responses(
        (status = 200, description = "GitHub issue synced into a hosted bounty record"),
        (status = 400, description = "Issue is invalid or hosted API state could not be planned"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn sync_github_issue_api_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<PlanGitHubIssueApiSyncRequest>,
) -> Result<Json<domain::Bounty>, StatusCode> {
    require_operator(&state, &headers)?;
    let plan = issue_api_sync_plan(GitHubIssueApiSyncInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        api_base_url: request.api_base_url,
        existing_bounty_ids: request.existing_bounty_ids,
        hosted_api_error: request.hosted_api_error,
    });
    if !plan.ready {
        return Err(StatusCode::BAD_REQUEST);
    }
    let parsed = plan.parsed.ok_or(StatusCode::BAD_REQUEST)?;
    let bounty_id = plan.bounty_id.ok_or(StatusCode::BAD_REQUEST)?;
    let idempotency_key = plan.idempotency_key.ok_or(StatusCode::BAD_REQUEST)?;
    let request = OpenPooledBountyRequest {
        bounty_id: None,
        idempotency_key: None,
        title: parsed.request.title,
        template_slug: parsed.template_slug,
        target_amount_minor: parsed.amount.amount,
        currency: parsed.amount.currency,
        funding_mode: parsed.funding_mode,
        privacy: parsed.privacy,
        funding_targets: vec![],
    };

    let bounty = if let Some(store) = &state.store {
        let candidate = {
            let mut network = state.network.lock().expect("state poisoned");
            network.build_github_issue_pooled_bounty(request, bounty_id, idempotency_key)
        };
        let candidate = match candidate {
            Ok(bounty) => bounty,
            Err(_) => {
                persist_all_risk_events(&state).await?;
                return Err(StatusCode::BAD_REQUEST);
            }
        };
        match store
            .upsert_github_issue_sync_bounty(&candidate)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            GitHubIssueSyncBountyUpsert::Upserted(bounty) => {
                state
                    .network
                    .lock()
                    .expect("state poisoned")
                    .bounties
                    .insert(bounty.id, bounty.clone());
                bounty
            }
            GitHubIssueSyncBountyUpsert::BlockedByActivity(existing) => {
                let id = existing.id;
                state
                    .network
                    .lock()
                    .expect("state poisoned")
                    .bounties
                    .insert(id, existing);
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    } else {
        let result = {
            let mut network = state.network.lock().expect("state poisoned");
            network.upsert_github_issue_pooled_bounty(request, bounty_id, idempotency_key)
        };
        let bounty = match result {
            Ok(bounty) => bounty,
            Err(_) => {
                persist_all_risk_events(&state).await?;
                return Err(StatusCode::BAD_REQUEST);
            }
        };
        persist_bounty_and_ledger(&state, &bounty, &[]).await?;
        bounty
    };
    Ok(Json(bounty))
}

#[utoipa::path(
    post,
    path = "/v1/github/funding-comment-plan",
    request_body = PlanGitHubFundingCommentRequest,
    responses((status = 200, description = "GitHub public funding-comment signal plan"))
)]
async fn plan_github_funding_comment(
    Json(request): Json<PlanGitHubFundingCommentRequest>,
) -> Json<GitHubFundingCommentPlan> {
    Json(funding_comment_plan(GitHubFundingCommentInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login,
        comment_id: request.comment_id,
        funding_api_base_url: request.funding_api_base_url,
        existing_idempotency_keys: request.existing_idempotency_keys,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/claim-comment-plan",
    request_body = PlanGitHubClaimCommentRequest,
    responses((status = 200, description = "GitHub public claim-comment reservation plan"))
)]
async fn plan_github_claim_comment(
    Json(request): Json<PlanGitHubClaimCommentRequest>,
) -> Json<GitHubClaimCommentPlan> {
    Json(claim_comment_plan(GitHubClaimCommentInput {
        repository: request.repository,
        issue_url: request.issue_url,
        title: request.title,
        body: request.body,
        comment_body: request.comment_body,
        contributor_login: request.contributor_login,
        comment_id: request.comment_id,
        claim_age_minutes: request.claim_age_minutes,
        progress_signal_count: request.progress_signal_count,
        active_claim_login: request.active_claim_login,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/github/proof-comment-plan",
    request_body = PlanGitHubProofCommentRequest,
    responses((status = 200, description = "GitHub proof comment markdown and check-run plan"))
)]
async fn plan_github_proof_comment(
    Json(request): Json<PlanGitHubProofCommentRequest>,
) -> Json<GitHubProofCommentPlan> {
    let comment = GitHubProofComment {
        bounty_id: request.bounty_id,
        proof_url: request.proof_url,
        verifier_summary: request.verifier_summary,
        settlement_url: request.settlement_url,
    };
    Json(proof_comment_plan(comment))
}

#[utoipa::path(
    post,
    path = "/v1/github/proof-comment-plan-from-proof",
    request_body = PlanGitHubProofCommentFromProofRequest,
    responses(
        (status = 200, description = "GitHub proof comment plan derived from a stored public proof"),
        (status = 404, description = "Proof not found, private, or missing verifier result")
    )
)]
async fn plan_github_proof_comment_from_proof(
    State(state): State<SharedState>,
    Json(request): Json<PlanGitHubProofCommentFromProofRequest>,
) -> Result<Json<GitHubProofCommentPlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    github_proof_comment_plan_from_proof(
        &network,
        &state.public_base_url,
        request.proof_id,
        request.settlement_url,
    )
    .map(Json)
}

fn github_proof_comment_plan_from_proof(
    network: &BountyNetwork,
    public_base_url: &str,
    proof_id: Uuid,
    settlement_url: Option<String>,
) -> Result<GitHubProofCommentPlan, StatusCode> {
    let proof = network.proofs.get(&proof_id).ok_or(StatusCode::NOT_FOUND)?;
    if proof.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    let verifier = network
        .verifier_results
        .get(&proof.verifier_result_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let verifier_summary = if verifier.summary.trim().is_empty() {
        format!("{:?} verifier accepted", verifier.kind)
    } else {
        format!("{:?}: {}", verifier.kind, verifier.summary.trim())
    };
    let comment = GitHubProofComment {
        bounty_id: proof.bounty_id,
        proof_url: format!(
            "{}/public/proofs/{}",
            public_base_url.trim_end_matches('/'),
            proof.id
        ),
        verifier_summary,
        settlement_url,
    };
    Ok(proof_comment_plan(comment))
}

#[utoipa::path(get, path = "/v1/bounties/{id}")]
async fn bounty_status(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Json<BountyStatusResponse>, StatusCode> {
    bounty_status_snapshot(&state, id).await.map(Json)
}

async fn bounty_status_snapshot(
    state: &SharedState,
    id: Uuid,
) -> Result<BountyStatusResponse, StatusCode> {
    if let Some(store) = &state.store {
        let scope = store
            .load_bounty_status_scope(id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .ok_or(StatusCode::NOT_FOUND)?;
        return bounty_status_from_scope(scope);
    }

    let status = {
        let network = state.network.lock().expect("state poisoned");
        network.status(id).map_err(|_| StatusCode::NOT_FOUND)?
    };
    Ok(status)
}

fn bounty_status_from_scope(scope: BountyStatusScope) -> Result<BountyStatusResponse, StatusCode> {
    let bounty_id = scope.bounty.id;
    let network = BountyNetwork {
        bounties: [(scope.bounty.id, scope.bounty)].into_iter().collect(),
        funding_intents: scope
            .funding_intents
            .into_iter()
            .map(|intent| (intent.id, intent))
            .collect(),
        funding_contributions: scope
            .funding_contributions
            .into_iter()
            .map(|contribution| (contribution.id, contribution))
            .collect(),
        escrows: scope
            .escrows
            .into_iter()
            .map(|escrow| (escrow.id, escrow))
            .collect(),
        claims: scope
            .claims
            .into_iter()
            .map(|claim| (claim.id, claim))
            .collect(),
        submissions: scope
            .submissions
            .into_iter()
            .map(|submission| (submission.id, submission))
            .collect(),
        verifier_results: scope
            .verifier_results
            .into_iter()
            .map(|result| (result.id, result))
            .collect(),
        proofs: scope
            .proofs
            .into_iter()
            .map(|proof| (proof.id, proof))
            .collect(),
        settlements: scope
            .settlements
            .into_iter()
            .map(|settlement| (settlement.id, settlement))
            .collect(),
        reputation_events: scope
            .reputation_events
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        template_signals: scope
            .template_signals
            .into_iter()
            .map(|signal| (signal.id, signal))
            .collect(),
        risk_events: scope
            .risk_events
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        ..BountyNetwork::default()
    };
    let status = network
        .status(bounty_id)
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(status)
}

async fn public_proof_page(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let proof = network
        .proofs
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    if proof.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    let verifier = network
        .verifier_results
        .get(&proof.verifier_result_id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();

    Ok(Html(web_public::render_proof_page(&proof, &verifier)))
}

async fn public_agent_profile(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    let agent = network
        .agents
        .get(&id)
        .ok_or(StatusCode::NOT_FOUND)?
        .clone();
    let reputation_score = network
        .reputation_events
        .values()
        .filter(|event| event.agent_id == id)
        .map(|event| event.delta)
        .sum();
    let accepted_count = network
        .reputation_events
        .values()
        .filter(|event| event.agent_id == id && event.delta > 0)
        .count();
    let real_paid_intents = network
        .settlements
        .values()
        .flat_map(|settlement| &settlement.payout_intents)
        .filter(|intent| {
            intent.recipient_agent_id == id
                && intent.status == PayoutStatus::Paid
                && intent.rail != PaymentRail::Simulated
        })
        .collect::<Vec<_>>();
    let paid_currency = real_paid_intents
        .iter()
        .find(|intent| intent.amount.currency == "usdc")
        .or_else(|| real_paid_intents.first())
        .map(|intent| intent.amount.currency.clone())
        .unwrap_or_else(|| "usdc".to_string());
    let paid_minor = real_paid_intents
        .iter()
        .filter(|intent| intent.amount.currency == paid_currency.as_str())
        .map(|intent| intent.amount.amount)
        .sum();

    Ok(Html(web_public::render_agent_profile(
        &agent,
        accepted_count,
        reputation_score,
        paid_minor,
        &paid_currency,
    )))
}

async fn public_verifier_profile(
    State(state): State<SharedState>,
    Path(kind): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let verifier_kind = parse_verifier_kind(&kind).ok_or(StatusCode::NOT_FOUND)?;
    let stats = {
        let network = state.network.lock().expect("state poisoned");
        let results = network
            .verifier_results
            .values()
            .filter(|result| result.kind == verifier_kind)
            .collect::<Vec<_>>();
        let total_checks = results.len();
        let accepted_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::Accepted)
            .count();
        let rejected_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::Rejected)
            .count();
        let needs_review_count = results
            .iter()
            .filter(|result| result.decision == VerificationDecision::NeedsReview)
            .count();
        let average_confidence = if total_checks == 0 {
            0.0
        } else {
            results.iter().map(|result| result.confidence).sum::<f32>() / total_checks as f32
        };
        web_public::VerifierProfileStats {
            total_checks,
            accepted_count,
            rejected_count,
            needs_review_count,
            average_confidence,
        }
    };
    Ok(Html(web_public::render_verifier_profile(
        &format!("{verifier_kind:?}"),
        &stats,
    )))
}

async fn public_bounty_feed_page(State(state): State<SharedState>) -> Html<String> {
    let bounties = {
        let network = state.network.lock().expect("state poisoned");
        network.list_claimable_bounties()
    };
    let items = web_public::public_bounty_feed(&bounties, &state.public_base_url);
    Html(web_public::render_bounty_feed_page(&items))
}

async fn public_funding_feed_page(State(state): State<SharedState>) -> Html<String> {
    let items = {
        let network = state.network.lock().expect("state poisoned");
        public_funding_feed_items(&network, &state.public_base_url)
    };
    Html(web_public::render_funding_feed_page(&items))
}

async fn public_bounty_page(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let status = {
        let network = state.network.lock().expect("state poisoned");
        network.status(id).map_err(|_| StatusCode::NOT_FOUND)?
    };
    if status.bounty.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Html(web_public::render_public_bounty_page(
        &public_bounty_page_model(&status, &state.public_base_url),
    )))
}

fn public_funding_feed_items(
    network: &BountyNetwork,
    public_base_url: &str,
) -> Vec<web_public::PublicFundingFeedItem> {
    let mut items = network
        .bounties
        .values()
        .filter_map(|bounty| network.status(bounty.id).ok())
        .filter(public_status_accepts_funding)
        .map(|status| public_funding_feed_item(&status, public_base_url))
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        public_remaining_partition_count(right)
            .cmp(&public_remaining_partition_count(left))
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.bounty_id.cmp(&right.bounty_id))
    });
    items
}

fn public_status_accepts_funding(status: &BountyStatusResponse) -> bool {
    let public_non_terminal = status.bounty.privacy != PrivacyLevel::Private
        && !matches!(
            status.bounty.status,
            BountyStatus::Paid
                | BountyStatus::Refunded
                | BountyStatus::Disputed
                | BountyStatus::Expired
        );
    let partition_remaining = status
        .funding_summary
        .partitions
        .iter()
        .any(|partition| partition.remaining.amount > 0);
    public_non_terminal && (partition_remaining || status.funding_summary.remaining.amount > 0)
}

fn public_remaining_partition_count(item: &web_public::PublicFundingFeedItem) -> usize {
    item.funding_partitions
        .iter()
        .filter(|partition| partition.remaining_minor > 0)
        .count()
}

fn public_funding_feed_item(
    status: &BountyStatusResponse,
    public_base_url: &str,
) -> web_public::PublicFundingFeedItem {
    let api = public_base_url.trim_end_matches('/');
    let bounty = &status.bounty;
    let funding_intent_url = format!("{api}/v1/bounties/{}/funding-intents", bounty.id);
    let public_url = format!("{api}/public/bounties/{}", bounty.id);
    let funding_partitions = status
        .funding_summary
        .partitions
        .iter()
        .map(|partition| web_public::PublicFundingPartition {
            rail: format!("{:?}", partition.rail),
            target_minor: partition.target.amount,
            confirmed_minor: partition.confirmed.amount,
            remaining_minor: partition.remaining.amount,
            currency: partition.target.currency.clone(),
            contribution_count: partition.contribution_count,
            escrow_count: partition.escrow_count,
            claimable: partition.claimable,
        })
        .collect::<Vec<_>>();
    let funding_intent_examples = web_public::public_funding_intent_examples(
        &bounty.id.to_string(),
        &funding_intent_url,
        &public_url,
        &format!("{:?}", bounty.funding_mode),
        status.funding_summary.remaining.amount,
        &status.funding_summary.remaining.currency,
        &funding_partitions,
    );
    web_public::PublicFundingFeedItem {
        bounty_id: bounty.id.to_string(),
        title: bounty.title.clone(),
        template_slug: bounty.template_slug.clone(),
        amount_minor: bounty.amount.amount,
        currency: bounty.amount.currency.clone(),
        funding_mode: format!("{:?}", bounty.funding_mode),
        status: format!("{:?}", bounty.status),
        privacy: format!("{:?}", bounty.privacy),
        terms_hash: bounty.terms_hash.clone(),
        created_at: bounty.created_at.to_rfc3339(),
        claimable: status.funding_summary.claimable,
        funding_target_minor: status.funding_summary.target.amount,
        funding_applied_minor: status.funding_summary.applied.amount,
        funding_remaining_minor: status.funding_summary.remaining.amount,
        contribution_count: status.funding_summary.contribution_count,
        public_url,
        status_url: format!("{api}/v1/bounties/{}", bounty.id),
        template_url: format!("{api}/public/templates/{}", bounty.template_slug),
        funding_intent_url,
        funding_contribution_url: format!("{api}/v1/bounties/{}/funding-contributions", bounty.id),
        funding_partitions,
        funding_intent_examples,
    }
}

fn public_bounty_page_model(
    status: &BountyStatusResponse,
    public_base_url: &str,
) -> web_public::PublicBountyPage {
    let api = public_base_url.trim_end_matches('/');
    let bounty = &status.bounty;
    let verification_type = status
        .verifier_results
        .iter()
        .max_by_key(|result| result.created_at)
        .map(|result| format!("{:?}", result.kind))
        .or_else(|| {
            web_public::bounty_templates()
                .into_iter()
                .find(|template| template.slug == bounty.template_slug)
                .map(|template| template.verifier.to_string())
        })
        .unwrap_or_else(|| "Unknown".to_string());
    let proof_urls = status
        .proofs
        .iter()
        .filter(|proof| proof.privacy != PrivacyLevel::Private)
        .map(|proof| format!("{api}/public/proofs/{}", proof.id))
        .collect();
    let public_url = format!("{api}/public/bounties/{}", bounty.id);
    let funding_partitions = status
        .funding_summary
        .partitions
        .iter()
        .map(|partition| web_public::PublicFundingPartition {
            rail: format!("{:?}", partition.rail),
            target_minor: partition.target.amount,
            confirmed_minor: partition.confirmed.amount,
            remaining_minor: partition.remaining.amount,
            currency: partition.target.currency.clone(),
            contribution_count: partition.contribution_count,
            escrow_count: partition.escrow_count,
            claimable: partition.claimable,
        })
        .collect::<Vec<_>>();
    let funding_intent_url = format!("{api}/v1/bounties/{}/funding-intents", bounty.id);
    let funding_intent_examples = web_public::public_funding_intent_examples(
        &bounty.id.to_string(),
        &funding_intent_url,
        &public_url,
        &format!("{:?}", bounty.funding_mode),
        status.funding_summary.remaining.amount,
        &status.funding_summary.remaining.currency,
        &funding_partitions,
    );
    let verifier_result_links = status
        .verifier_results
        .iter()
        .map(|result| web_public::PublicBountyRecordLink {
            label: format!(
                "{:?} {:?} verifier result {}",
                result.kind, result.decision, result.id
            ),
            url: format!("{public_url}#verifier-results"),
        })
        .collect();
    let settlement_links = status
        .settlements
        .iter()
        .map(|settlement| {
            let paid_payouts = settlement
                .payout_intents
                .iter()
                .filter(|intent| intent.status == PayoutStatus::Paid)
                .count();
            let total_payouts = settlement.payout_intents.len();
            web_public::PublicBountyRecordLink {
                label: format!(
                    "{:?} settlement {} ({paid_payouts}/{total_payouts} payouts paid)",
                    settlement.rail, settlement.id
                ),
                url: format!("{public_url}#settlements"),
            }
        })
        .collect();
    let template_signal_links = status
        .template_signals
        .iter()
        .map(|signal| web_public::PublicBountyRecordLink {
            label: format!("{} template signal {}", signal.template_slug, signal.id),
            url: format!("{api}/public/templates/{}", signal.template_slug),
        })
        .collect();
    web_public::PublicBountyPage {
        bounty_id: bounty.id.to_string(),
        title: bounty.title.clone(),
        template_slug: bounty.template_slug.clone(),
        amount_minor: bounty.amount.amount,
        currency: bounty.amount.currency.clone(),
        funding_mode: format!("{:?}", bounty.funding_mode),
        privacy: format!("{:?}", bounty.privacy),
        status: format!("{:?}", bounty.status),
        terms_hash: bounty.terms_hash.clone(),
        created_at: bounty.created_at.to_rfc3339(),
        verification_type,
        claimable: status.funding_summary.claimable,
        funding_target_minor: status.funding_summary.target.amount,
        funding_applied_minor: status.funding_summary.applied.amount,
        funding_remaining_minor: status.funding_summary.remaining.amount,
        contribution_count: status.funding_summary.contribution_count,
        public_url,
        claim_url: format!("{api}/v1/bounties/{}/claim", bounty.id),
        status_url: format!("{api}/v1/bounties/{}", bounty.id),
        template_url: format!("{api}/public/templates/{}", bounty.template_slug),
        funding_intent_url,
        funding_contribution_url: format!("{api}/v1/bounties/{}/funding-contributions", bounty.id),
        proof_urls,
        funding_partitions,
        funding_intent_examples,
        verifier_result_links,
        settlement_links,
        template_signal_links,
    }
}

async fn public_capability_feed_page(State(state): State<SharedState>) -> Html<String> {
    let (capabilities, agents, reputation_events, settlements) = {
        let network = state.network.lock().expect("state poisoned");
        (
            network.capabilities.values().cloned().collect::<Vec<_>>(),
            network.agents.values().cloned().collect::<Vec<_>>(),
            network
                .reputation_events
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            network.settlements.values().cloned().collect::<Vec<_>>(),
        )
    };
    let items = web_public::public_capability_feed(
        &capabilities,
        &agents,
        &reputation_events,
        &settlements,
        &state.public_base_url,
    );
    Html(web_public::render_capability_feed_page(&items))
}

async fn public_template_index() -> Html<String> {
    Html(web_public::render_template_index(
        &web_public::bounty_templates(),
    ))
}

async fn public_template_page(
    State(state): State<SharedState>,
    Path(slug): Path<String>,
) -> Result<Html<String>, StatusCode> {
    let template = web_public::bounty_templates()
        .into_iter()
        .find(|template| template.slug == slug)
        .ok_or(StatusCode::NOT_FOUND)?;
    let stats = {
        let network = state.network.lock().expect("state poisoned");
        let matching = network
            .template_signals
            .values()
            .filter(|signal| signal.template_slug == slug && signal.success)
            .collect::<Vec<_>>();
        let currency = matching
            .first()
            .map(|signal| signal.amount.currency.clone())
            .unwrap_or_else(|| "usdc".to_string());
        let accepted_value_minor = matching
            .iter()
            .filter(|signal| signal.amount.currency == currency)
            .map(|signal| signal.amount.amount)
            .sum();
        web_public::TemplateStats {
            accepted_count: matching.len(),
            accepted_value_minor,
            currency,
        }
    };
    Ok(Html(web_public::render_template_page(
        &template,
        Some(&stats),
    )))
}

fn parse_verifier_kind(kind: &str) -> Option<VerifierKind> {
    match kind.to_ascii_lowercase().replace(['-', '_'], "").as_str() {
        "manual" => Some(VerifierKind::Manual),
        "jsonschema" => Some(VerifierKind::JsonSchema),
        "dockercommand" => Some(VerifierKind::DockerCommand),
        "githubci" => Some(VerifierKind::GitHubCi),
        "httpcallback" => Some(VerifierKind::HttpCallback),
        "aijudgefilter" => Some(VerifierKind::AiJudgeFilter),
        _ => None,
    }
}

fn eval_run_from_suite(result: &EvalSuiteResult) -> EvalRun {
    EvalRun {
        id: Uuid::new_v4(),
        suite: result.suite.clone(),
        score: result.score,
        passed: result.passed,
        created_at: Utc::now(),
    }
}

fn eval_run_from_loop_suite(result: &LoopSuiteResult) -> EvalRun {
    EvalRun {
        id: Uuid::new_v4(),
        suite: result.suite.clone(),
        score: loop_suite_average_score(result),
        passed: result.passed,
        created_at: Utc::now(),
    }
}

fn loop_suite_average_score(result: &LoopSuiteResult) -> f32 {
    if result.loops.is_empty() {
        return 0.0;
    }

    let total = result
        .loops
        .iter()
        .map(|loop_result| {
            loop_result
                .candidates
                .iter()
                .map(|candidate| candidate.score)
                .fold(0.0_f32, f32::max)
        })
        .sum::<f32>();
    total / result.loops.len() as f32
}

async fn record_eval_run(state: &SharedState, run: EvalRun) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_eval_run(&run)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    state
        .eval_runs
        .lock()
        .expect("state poisoned")
        .insert(0, run);
    Ok(())
}

async fn hydrate_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: store
            .list_agents()
            .await?
            .into_iter()
            .map(|agent| (agent.id, agent))
            .collect(),
        contributor_contacts: store
            .list_contributor_contacts()
            .await?
            .into_iter()
            .map(|contact| (contact.id, contact))
            .collect(),
        audience_members: store
            .list_audience_members()
            .await?
            .into_iter()
            .map(|member| (member.id, member))
            .collect(),
        audience_interactions: store
            .list_audience_interactions()
            .await?
            .into_iter()
            .map(|interaction| (interaction.id, interaction))
            .collect(),
        discovery_responses: store
            .list_discovery_responses()
            .await?
            .into_iter()
            .map(|response| (response.id, response))
            .collect(),
        outreach_attempts: store
            .list_outreach_attempts()
            .await?
            .into_iter()
            .map(|attempt| (attempt.id, attempt))
            .collect(),
        capabilities: store
            .list_capabilities()
            .await?
            .into_iter()
            .map(|capability| (capability.id, capability))
            .collect(),
        help_requests: store
            .list_help_requests()
            .await?
            .into_iter()
            .map(|help_request| (help_request.id, help_request))
            .collect(),
        quotes: store
            .list_quotes()
            .await?
            .into_iter()
            .map(|quote| (quote.id, quote))
            .collect(),
        bounties: store
            .list_bounties()
            .await?
            .into_iter()
            .map(|bounty| (bounty.id, bounty))
            .collect(),
        objectives: store
            .list_objectives()
            .await?
            .into_iter()
            .map(|objective| (objective.id, objective))
            .collect(),
        funding_intents: store
            .list_funding_intents()
            .await?
            .into_iter()
            .map(|intent| (intent.id, intent))
            .collect(),
        funding_contributions: store
            .list_funding_contributions()
            .await?
            .into_iter()
            .map(|contribution| (contribution.id, contribution))
            .collect(),
        escrows: store
            .list_escrows()
            .await?
            .into_iter()
            .map(|escrow| (escrow.id, escrow))
            .collect(),
        claims: store
            .list_claims()
            .await?
            .into_iter()
            .map(|claim| (claim.id, claim))
            .collect(),
        submissions: store
            .list_submissions()
            .await?
            .into_iter()
            .map(|submission| (submission.id, submission))
            .collect(),
        verifier_results: store
            .list_verifier_results()
            .await?
            .into_iter()
            .map(|result| (result.id, result))
            .collect(),
        proofs: store
            .list_proof_records()
            .await?
            .into_iter()
            .map(|proof| (proof.id, proof))
            .collect(),
        settlements: store
            .list_settlements()
            .await?
            .into_iter()
            .map(|settlement| (settlement.id, settlement))
            .collect(),
        reputation_events: store
            .list_reputation_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        template_signals: store
            .list_template_signals()
            .await?
            .into_iter()
            .map(|signal| (signal.id, signal))
            .collect(),
        risk_events: store
            .list_risk_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        risk_reviews: store
            .list_risk_reviews()
            .await?
            .into_iter()
            .map(|review| (review.id, review))
            .collect(),
        payment_events: store
            .list_payment_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        ledger: Ledger::from_entries(store.list_ledger_entries().await?)?,
        ..BountyNetwork::default()
    })
}

fn map_objective_error(error: ObjectiveError) -> StatusCode {
    match error {
        ObjectiveError::ProposalNotFound(_)
        | ObjectiveError::ContributionNeedNotFound(_)
        | ObjectiveError::ContributionOfferNotFound(_)
        | ObjectiveError::UnknownParticipant(_) => StatusCode::NOT_FOUND,
        ObjectiveError::StaleAction
        | ObjectiveError::ProposalExpired
        | ObjectiveError::ProposalAlreadyAccepted
        | ObjectiveError::InvalidAction(_, _)
        | ObjectiveError::NotReady(_)
        | ObjectiveError::AmendmentsUnavailable => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    }
}

fn map_objective_db_error(error: DbError) -> StatusCode {
    match error {
        DbError::ObjectiveAlreadyExists(_) | DbError::ObjectiveRevisionConflict { .. } => {
            StatusCode::CONFLICT
        }
        DbError::ObjectiveNotFound(_) => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn load_objective(state: &SharedState, id: Id) -> Result<Objective, StatusCode> {
    if let Some(store) = &state.store {
        return store
            .get_objective(id)
            .await
            .map_err(map_objective_db_error)?
            .ok_or(StatusCode::NOT_FOUND);
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .get(&id)
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)
}

async fn load_objectives(state: &SharedState) -> Result<Vec<Objective>, StatusCode> {
    if let Some(store) = &state.store {
        return store
            .list_objectives()
            .await
            .map_err(map_objective_db_error);
    }
    let mut objectives = state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .values()
        .cloned()
        .collect::<Vec<_>>();
    objectives.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(objectives)
}

async fn persist_new_objective(
    state: &SharedState,
    objective: &Objective,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .create_objective(objective)
            .await
            .map_err(map_objective_db_error)?;
    } else {
        let mut network = state.network.lock().expect("state poisoned");
        if network.objectives.contains_key(&objective.id) {
            return Err(StatusCode::CONFLICT);
        }
        network.objectives.insert(objective.id, objective.clone());
        return Ok(());
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .insert(objective.id, objective.clone());
    Ok(())
}

async fn persist_objective_replacement(
    state: &SharedState,
    objective: &Objective,
    expected_revision: u64,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .replace_objective(objective, expected_revision)
            .await
            .map_err(map_objective_db_error)?;
    } else {
        let mut network = state.network.lock().expect("state poisoned");
        let current_revision = network
            .objectives
            .get(&objective.id)
            .map(|current| current.revision)
            .ok_or(StatusCode::NOT_FOUND)?;
        if current_revision != expected_revision {
            return Err(StatusCode::CONFLICT);
        }
        network.objectives.insert(objective.id, objective.clone());
        return Ok(());
    }
    state
        .network
        .lock()
        .expect("state poisoned")
        .objectives
        .insert(objective.id, objective.clone());
    Ok(())
}

async fn load_objective_canonical_evidence(
    state: &SharedState,
    objectives: &[Objective],
) -> Result<ObjectiveCanonicalEvidence, StatusCode> {
    let Some(store) = &state.store else {
        return Ok(ObjectiveCanonicalEvidence::default());
    };
    let mut networks = BTreeSet::new();
    for objective in objectives {
        let Some(bundle) = objective.accepted_value_bundle.as_ref() else {
            continue;
        };
        if let Some(payment) = &bundle.monetary_payment {
            networks.insert(payment.bounty.network.clone());
        }
        for need in &bundle.contribution_needs {
            if let domain::ContributionCompensation::Paid { payment } = &need.compensation {
                networks.insert(payment.bounty.network.clone());
            }
        }
    }
    if networks.is_empty() {
        return Ok(ObjectiveCanonicalEvidence::default());
    }
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(map_objective_db_error)?;
    let mut evidence = ObjectiveCanonicalEvidence::default();
    for network in networks {
        let events = store
            .list_autonomous_bounty_events(&network)
            .await
            .map_err(map_objective_db_error)?;
        let mut feed = build_autonomous_bounty_feed(events, terms.clone(), false)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        state.recovery_reservations.apply(&mut feed, false);
        let mut network_evidence = build_objective_canonical_evidence(&network, &feed);
        evidence.funding.append(&mut network_evidence.funding);
        evidence
            .settlements
            .append(&mut network_evidence.settlements);
    }
    Ok(evidence)
}

async fn persist_bounty_and_ledger(
    state: &SharedState,
    bounty: &domain::Bounty,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let contributions = {
            let network = state.network.lock().expect("state poisoned");
            network
                .funding_contributions
                .values()
                .filter(|contribution| contribution.bounty_id == bounty.id)
                .cloned()
                .collect::<Vec<_>>()
        };
        for contribution in &contributions {
            store
                .upsert_funding_contribution(contribution)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, ledger_entries).await?;
    }
    Ok(())
}

async fn persist_reviewed_bounty_approval(
    state: &SharedState,
    approval: &ReviewedBountyApproval,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&approval.bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_risk_review(&approval.review)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        persist_ledger_entries(store, ledger_entries).await?;
    }
    Ok(())
}

async fn persist_pooled_funding_report(
    state: &SharedState,
    report: &PooledFundingReport,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&report.bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_funding_contribution(&report.contribution)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        persist_ledger_entries(store, &report.ledger_entries).await?;
    }
    Ok(())
}

async fn persist_funding_intent_report(
    state: &SharedState,
    report: &FundingIntentReport,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&report.bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_funding_intent(&report.intent)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(())
}

async fn persist_risk_review(
    state: &SharedState,
    review: &RiskReviewRecord,
) -> Result<(), StatusCode> {
    if let Some(store) = &state.store {
        store
            .upsert_risk_review(review)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(())
}

async fn persist_ledger_entries(
    store: &PostgresStore,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), StatusCode> {
    for entry in ledger_entries {
        store
            .insert_ledger_entry(entry)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(())
}

async fn persist_all_risk_events(state: &SharedState) -> Result<(), StatusCode> {
    let Some(store) = &state.store else {
        return Ok(());
    };
    let events = {
        let network = state.network.lock().expect("state poisoned");
        network.risk_events.values().cloned().collect::<Vec<_>>()
    };
    for event in &events {
        store
            .upsert_risk_event(event)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    Ok(())
}

#[allow(dead_code)]
fn expected_digest_for_body(body: &str) -> String {
    hash_artifact(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::B256,
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use app::{
        AddFundingContributionRequest, ClaimBountyRequest, CreateFundingIntentRequest,
        OpenPooledBountyRequest, PostBountyRequest, RegisterAgentRequest,
        RegisterCapabilityRequest, SubmitResultRequest, VerifySubmissionRequest,
    };
    use domain::{
        AffectedPartyDeclaration, Bounty, BountyStatus, CapabilityClass, DeliverableAccessPolicy,
        ExpectedEffect, FundingIntentStatus, FundingMode, IdentityDisclosure, ObjectiveAuthority,
        ObjectiveAuthorityKind, ObjectiveParticipant, ObjectivePrivacyDeclaration, ObjectiveStatus,
        ObjectiveVerificationMechanism, ObjectiveVerificationPolicy, ParticipantKind,
        PaymentEventStatus, PaymentRail, PayoutStatus, ProofRecord, PublicEvidencePolicy,
        RightsPolicy, VerifierKind,
    };
    use github_app::GitHubCheckConclusion;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        str::FromStr,
        thread,
    };

    type TestHmacSha256 = Hmac<Sha256>;

    #[tokio::test]
    async fn objective_api_requires_signed_creation_and_preserves_role_boundaries() {
        let signer: PrivateKeySigner =
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                .parse()
                .unwrap();
        let participant_id = Uuid::new_v4();
        let draft = ObjectiveCreationDraft {
            id: Uuid::new_v4(),
            title: "Publish a verified public report".to_string(),
            desired_outcome: "A source-linked report passes its committed review.".to_string(),
            human_purpose: "Enable an informed decision by the named beneficiary.".to_string(),
            participants: vec![ObjectiveParticipant {
                id: participant_id,
                kind: ParticipantKind::Organization,
                display_name: "Requesting organization".to_string(),
                wallet: format!("{:#x}", signer.address()),
                identity_disclosure: IdentityDisclosure::Pseudonymous,
                public_identity_reference: None,
            }],
            requesting_party_id: participant_id,
            beneficiary_ids: vec![participant_id],
            affected_parties: vec![AffectedPartyDeclaration {
                participant_id,
                expected_effect: ExpectedEffect::Mixed,
                description: "Receives the result and bears the decision risk.".to_string(),
            }],
            authority: ObjectiveAuthority {
                kind: ObjectiveAuthorityKind::OrganizationWallet,
                member_ids: vec![participant_id],
                threshold: 1,
                public_statement:
                    "One declared organization wallet controls binding objective decisions."
                        .to_string(),
            },
            available_resources: Vec::new(),
            expected_final_deliverable: "Public report and evidence package".to_string(),
            requested_access_policy: DeliverableAccessPolicy::Public,
            requested_rights_policy: RightsPolicy {
                owner_ids: vec![participant_id],
                license_or_terms: "CC-BY-4.0".to_string(),
                restrictions: Vec::new(),
            },
            requested_final_verification: ObjectiveVerificationPolicy {
                mechanism: ObjectiveVerificationMechanism::CommittedVerifier {
                    verifier_id: participant_id,
                },
                acceptance_criteria: vec!["Every claim links to inspectable evidence.".to_string()],
                evidence_schema: "https://example.test/report-evidence.schema.json".to_string(),
                evidence_schema_hash: format!("0x{}", "1".repeat(64)),
                trust_assumptions: vec![
                    "The named verifier wallet follows the public criteria.".to_string()
                ],
            },
            privacy: ObjectivePrivacyDeclaration {
                blockchain_information_is_public: true,
                evidence_policy: PublicEvidencePolicy::Public,
                redaction_limits: "No private data is accepted by this public objective."
                    .to_string(),
            },
        };
        let plan = plan_objective_creation(Json(draft)).await.unwrap().0;
        let commitment = B256::from_str(&plan.commitment_hash).unwrap();
        let signature = signer.sign_message_sync(commitment.as_slice()).unwrap();
        let signed = SignedObjectiveCreation {
            plan,
            approvals: vec![domain::WalletApproval {
                participant_id,
                signature: signature.to_string(),
            }],
        };
        let state = test_state(BountyNetwork::default());
        let created = create_objective(State(state.clone()), Json(signed.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(created.objective.status, ObjectiveStatus::OpenForProposals);
        assert_eq!(created.objective.requesting_party_id, participant_id);
        assert_eq!(created.objective.authority.member_ids, vec![participant_id]);
        assert!(!created.readiness.ready);
        assert!(created
            .readiness
            .blockers
            .iter()
            .any(|blocker| blocker.contains("provider proposal")));

        let listed = list_objectives(State(state.clone())).await.unwrap().0;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].objective.id, created.objective.id);
        assert_eq!(
            create_objective(State(state), Json(signed))
                .await
                .unwrap_err(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn health_identifies_protocol_and_deployed_revision() {
        let response = health_response("0123456789abcdef0123456789abcdef01234567").into_response();

        assert_eq!(
            response.headers()["x-agent-bounties-revision"],
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            response.headers()["x-agent-bounties-protocol"],
            "agent-bounties/autonomous-v1"
        );
    }

    #[test]
    fn timeout_relay_accepts_only_exact_permissionless_transition_intent() {
        let bounty = "0x1111111111111111111111111111111111111111";
        let relayer = "0x2222222222222222222222222222222222222222";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: bounty.to_string(),
            value_wei: 0,
            data: AutonomousTimeoutAction::ExpireSubmission
                .calldata()
                .to_string(),
            function: AutonomousTimeoutAction::ExpireSubmission
                .function()
                .to_string(),
        };

        assert!(validate_autonomous_timeout_intent(
            &intent,
            bounty,
            relayer,
            AutonomousTimeoutAction::ExpireSubmission,
        )
        .is_ok());

        intent.data = AutonomousTimeoutAction::ExpireClaim.calldata().to_string();
        assert_eq!(
            validate_autonomous_timeout_intent(
                &intent,
                bounty,
                relayer,
                AutonomousTimeoutAction::ExpireSubmission,
            ),
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
        intent.data = AutonomousTimeoutAction::ExpireSubmission
            .calldata()
            .to_string();
        intent.value_wei = 1;
        assert_eq!(
            validate_autonomous_timeout_intent(
                &intent,
                bounty,
                relayer,
                AutonomousTimeoutAction::ExpireSubmission,
            ),
            Err(StatusCode::UNPROCESSABLE_ENTITY)
        );
    }

    #[test]
    fn x402_payment_required_response_uses_v2_wire_header_and_no_store() {
        let challenge = base_usdc_funding_challenge(
            "https://api.example/v1/x402/base/bounties/0x1111111111111111111111111111111111111111/funding?network=base-mainnet&amount=150000",
            "eip155:8453",
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "0x1111111111111111111111111111111111111111",
            150_000,
            300,
        )
        .unwrap();

        let response = x402_payment_required_response(challenge.clone()).unwrap();

        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        let decoded = payments_x402::decode_payment_required_header(
            response.headers()[PAYMENT_REQUIRED_HEADER]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(decoded, challenge);
        assert_eq!(decoded.accepts[0].scheme, AGENT_BOUNTY_FUND_SCHEME);
    }

    #[test]
    fn x402_funding_amount_defaults_to_gap_and_rejects_overfunding_or_wrong_state() {
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", None).unwrap(),
            1_850_000
        );
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", Some(250000)).unwrap(),
            250_000
        );
        assert_eq!(
            resolve_x402_funding_amount("open", "2000000", "150000", Some(1_850_001)).unwrap_err(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            resolve_x402_funding_amount("claimable", "2000000", "2000000", None).unwrap_err(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn hosted_x402_intent_allows_only_exact_zero_value_funding_call() {
        let relayer = "0x2222222222222222222222222222222222222222";
        let bounty = "0x1111111111111111111111111111111111111111";
        let mut intent = EvmTransactionIntent {
            from: Some(relayer.to_string()),
            to: bounty.to_string(),
            value_wei: 0,
            data: format!(
                "0x{}{}",
                AUTONOMOUS_FUND_WITH_AUTHORIZATION_SELECTOR,
                "00".repeat(8 * 32)
            ),
            function: AUTONOMOUS_FUND_WITH_AUTHORIZATION_FUNCTION.to_string(),
        };
        assert!(validate_hosted_x402_intent(&intent, relayer, bounty).is_ok());

        intent.value_wei = 1;
        assert_eq!(
            validate_hosted_x402_intent(&intent, relayer, bounty).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        intent.value_wei = 0;
        intent.data = "0xdeadbeef".to_string();
        assert_eq!(
            validate_hosted_x402_intent(&intent, relayer, bounty).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn confirmed_x402_relay_emits_payment_response_only_after_canonical_event() {
        let state = test_state(BountyNetwork::default());
        let now = Utc::now();
        let attempt = X402RelayAttempt {
            id: Uuid::new_v4(),
            idempotency_key: "x402:test".to_string(),
            network: "base-mainnet".to_string(),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            contributor: "0x3333333333333333333333333333333333333333".to_string(),
            amount: 150_000,
            authorization_nonce: format!("0x{}", "44".repeat(32)),
            authorization_valid_before: 2_000_000_000,
            request_fingerprint: "fingerprint".to_string(),
            relayer_address: "0x2222222222222222222222222222222222222222".to_string(),
            status: X402RelayStatus::Confirmed,
            retryable: false,
            attempt_count: 1,
            tx_hash: Some(format!("0x{}", "55".repeat(32))),
            estimated_gas: Some(100_000),
            gas_limit: Some(120_000),
            error_code: None,
            error_message: None,
            canonical_event_id: Some(Uuid::new_v4()),
            confirmed_block: Some(123),
            created_at: now,
            updated_at: now,
        };

        let public_relay = x402_public_relay(&attempt).to_string();
        assert!(!public_relay.contains(&attempt.idempotency_key));
        assert!(!public_relay.contains(&attempt.authorization_nonce));
        assert!(!public_relay.contains(&attempt.request_fingerprint));

        let response = x402_relay_response(&state, &attempt).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let settlement = payments_x402::decode_payment_response_header(
            response.headers()[PAYMENT_RESPONSE_HEADER]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert!(settlement.success);
        assert_eq!(settlement.amount.as_deref(), Some("150000"));
        assert_eq!(settlement.network, "eip155:8453");
    }

    #[tokio::test]
    async fn x402_discovery_is_explicit_about_custom_funding_and_mpp_boundary() {
        let state = test_state(BountyNetwork::default());
        let document = x402_discovery(State(state)).await.0;

        assert_eq!(document["x402Version"], X402_VERSION);
        assert_eq!(document["resources"][0]["scheme"], AGENT_BOUNTY_FUND_SCHEME);
        assert_eq!(document["resources"][0]["genericExactCompatible"], false);
        assert_eq!(document["hostedRelay"]["enabled"], false);
        assert_eq!(document["hostedRelay"]["minUsdcBaseUnits"], "100000");
        assert_eq!(document["hostedRelay"]["maxDailyAttempts"], 100);
        assert_eq!(
            document["hostedRelay"]["maxDailyAttemptsPerContributor"],
            10
        );
        assert!(document["hostedRelay"]["statusUrlTemplate"]
            .as_str()
            .unwrap()
            .contains("/v1/x402/base/relays/{relay_id}"));
        assert_eq!(document["mpp"]["status"], "planned");
        assert!(document["safety"]["standardExactToBountyContract"]
            .as_str()
            .unwrap()
            .contains("FundingAdded"));
    }

    #[tokio::test]
    async fn agent_paid_status_endpoint_summarizes_solver_receivables() {
        let (network, _bounty, _proof) = completed_simulated_bounty().await;
        let solver_id = network
            .settlements
            .values()
            .flat_map(|settlement| &settlement.payout_intents)
            .find(|intent| intent.amount.currency == "usdc")
            .expect("solver payout intent exists")
            .recipient_agent_id;
        let state = test_state(network);

        let response = agent_paid_status(State(state), Path(solver_id))
            .await
            .unwrap()
            .0;
        let post_value = &response["post_value_loop"];
        assert_eq!(post_value["trigger"], "verified_completion");
        assert!(post_value["self_interest"]
            .as_str()
            .unwrap()
            .contains("more and higher-value funded bounties"));
        assert!(post_value["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["kind"] == "tell_your_human"));
        let response: app::AgentPayoutStatusResponse = serde_json::from_value(response).unwrap();

        assert_eq!(response.agent.id, solver_id);
        assert_eq!(response.payouts.len(), 1);
        assert_eq!(response.payouts[0].status, PayoutStatus::Paid);
        assert_eq!(response.totals[0].currency, "usdc");
        assert_eq!(response.totals[0].pending_minor, 0);
        assert_eq!(response.totals[0].paid_minor, 1_000_000);
    }

    #[tokio::test]
    async fn simulated_paid_status_uses_verified_completion_copy() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "simulated-solver".to_string(),
            payout_wallet: None,
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Verify simulated distribution copy".to_string(),
                template_slug: "small-code-change".to_string(),
                amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"simulated\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://tests/simulated.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: hash_artifact(artifact),
                verifier_kind: Some(VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();
        let state = test_state(network);

        let response = agent_paid_status(State(state), Path(solver.id))
            .await
            .unwrap()
            .0;

        assert_eq!(
            response["post_value_loop"]["trigger"],
            "verified_completion"
        );
        assert!(!response["post_value_loop"]["value_statement"]
            .as_str()
            .unwrap()
            .contains("received a reconciled payout"));
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_credits_platform_balance_once() {
        let organization_id = Uuid::new_v4();
        let state = test_state_with_unsigned_stripe_webhooks(BountyNetwork::default());
        let body = stripe_checkout_event_body("evt_paid", "cs_paid", organization_id);

        let first = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(body.clone()),
        )
        .await
        .unwrap()
        .0;

        assert!(!first.duplicate);
        assert_eq!(first.ledger_entries.len(), 1);
        assert_eq!(
            first.funding_credit.payment_event.status,
            PaymentEventStatus::Applied
        );

        let replay = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(body),
        )
        .await
        .unwrap()
        .0;

        assert!(replay.duplicate);
        assert!(replay.ledger_entries.is_empty());
        assert_eq!(
            replay.funding_credit.payment_event.status,
            PaymentEventStatus::IgnoredDuplicate
        );
        let network = state.network.lock().expect("state poisoned");
        assert_eq!(network.payment_events.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_rejects_unsigned_when_not_explicitly_allowed() {
        let organization_id = Uuid::new_v4();
        let state = test_state(BountyNetwork::default());
        let body =
            stripe_checkout_event_body("evt_unsigned_paid", "cs_unsigned_paid", organization_id);

        assert_eq!(
            reconcile_stripe_checkout_webhook(State(state), HeaderMap::new(), Bytes::from(body),)
                .await
                .unwrap_err(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_requires_valid_signature_when_secret_configured() {
        let organization_id = Uuid::new_v4();
        let secret = b"whsec_test";
        let state = test_state_with_stripe_webhook_secret(BountyNetwork::default(), secret);
        let body = stripe_checkout_event_body("evt_signed_paid", "cs_signed_paid", organization_id);

        assert_eq!(
            reconcile_stripe_checkout_webhook(
                State(state.clone()),
                HeaderMap::new(),
                Bytes::from(body.clone()),
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );

        let mut bad_headers = HeaderMap::new();
        bad_headers.insert("stripe-signature", "t=1700000000,v1=00".parse().unwrap());
        assert_eq!(
            reconcile_stripe_checkout_webhook(
                State(state.clone()),
                bad_headers,
                Bytes::from(body.clone()),
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );

        let mut signed_headers = HeaderMap::new();
        signed_headers.insert(
            "stripe-signature",
            stripe_signature_header(&body, secret).parse().unwrap(),
        );
        let signed = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            signed_headers,
            Bytes::from(body),
        )
        .await
        .unwrap()
        .0;

        assert!(!signed.duplicate);
        assert_eq!(signed.ledger_entries.len(), 1);
        assert_eq!(
            signed.funding_credit.payment_event.status,
            PaymentEventStatus::Applied
        );
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_plans_checkout_session() {
        let organization_id = Uuid::new_v4();
        let state = test_state(BountyNetwork::default());

        let intent = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(intent.endpoint, "/v1/checkout/sessions");
        assert!(intent.idempotency_key.contains("checkout_top_up"));
        assert_eq!(intent.body["mode"], "payment");
        assert_eq!(
            intent.body["client_reference_id"],
            organization_id.to_string()
        );
        assert_eq!(
            intent.body["success_url"],
            "http://127.0.0.1:8080/stripe/success"
        );
        assert_eq!(
            intent.body["cancel_url"],
            "http://127.0.0.1:8080/stripe/cancel"
        );
        assert!(intent.body.get("payment_method_types").is_none());
        assert!(intent.body.get("payment_method_configuration").is_none());
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_applies_payment_method_configuration() {
        let organization_id = Uuid::new_v4();
        let state = test_state_with_stripe_payment_method_configuration(
            BountyNetwork::default(),
            "pmc_paypal_enabled",
        );

        let intent = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(
            intent.body["payment_method_configuration"],
            "pmc_paypal_enabled"
        );
        assert!(intent.body.get("payment_method_types").is_none());
    }

    #[tokio::test]
    async fn stripe_checkout_top_up_endpoint_rejects_below_minimum() {
        let state = test_state(BountyNetwork::default());

        let error = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id: Uuid::new_v4(),
                amount_minor: 49,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn stripe_connect_account_endpoint_uses_accounts_v2() {
        let agent_id = Uuid::new_v4();

        let intent =
            plan_stripe_connect_account(Json(PlanStripeConnectAccountRequest { agent_id }))
                .await
                .unwrap()
                .0;

        assert_eq!(intent.request.endpoint, "/v2/core/accounts");
        assert_eq!(
            intent.request.body["metadata"]["agent_id"],
            agent_id.to_string()
        );
    }

    #[tokio::test]
    async fn live_stripe_checkout_endpoint_returns_execution_report() {
        let organization_id = Uuid::new_v4();
        let stripe_api_base_url = spawn_rpc_response(serde_json::json!({
            "id": "cs_test_live",
            "object": "checkout.session",
            "url": "https://checkout.stripe.com/c/pay/cs_test_live",
            "livemode": false
        }));
        let state = test_state_with_stripe_live(BountyNetwork::default(), stripe_api_base_url);

        let report = execute_stripe_checkout_top_up(
            State(state),
            HeaderMap::new(),
            Json(PlanStripeCheckoutTopUpRequest {
                organization_id,
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.status, 200);
        assert_eq!(report.stripe_id.as_deref(), Some("cs_test_live"));
        assert_eq!(
            report.url.as_deref(),
            Some("https://checkout.stripe.com/c/pay/cs_test_live")
        );
        assert_eq!(report.request.endpoint, "/v1/checkout/sessions");
    }

    #[tokio::test]
    async fn public_stripe_funding_intent_checkout_executes_bounty_checkout() {
        let mut network = BountyNetwork::default();
        let organization_id = Uuid::new_v4();
        let bounty = network
            .open_pooled_bounty(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Debit card funded bounty".to_string(),
                template_slug: "small-code-change".to_string(),
                target_amount_minor: 5_000,
                currency: "usd".to_string(),
                funding_mode: domain::FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            })
            .unwrap();
        let funding_intent = network
            .create_funding_intent(
                CreateFundingIntentRequest {
                    bounty_id: bounty.id,
                    contributor_agent_id: None,
                    source_organization_id: Some(organization_id),
                    amount_minor: 5_000,
                    currency: "usd".to_string(),
                    rail: domain::PaymentRail::StripeFiat,
                    external_reference: Some("card-funding-test".to_string()),
                    stripe_success_url: Some(
                        "https://nspg13.github.io/agent-bounties/success.html".to_string(),
                    ),
                    stripe_cancel_url: Some(
                        "https://nspg13.github.io/agent-bounties/cancel.html".to_string(),
                    ),
                },
                "http://127.0.0.1:8080",
            )
            .unwrap()
            .intent;
        let stripe_api_base_url = spawn_rpc_response(serde_json::json!({
            "id": "cs_test_bounty",
            "object": "checkout.session",
            "url": "https://checkout.stripe.com/c/pay/cs_test_bounty",
            "livemode": false
        }));
        let state = test_state_with_stripe_public_checkout_and_payment_method_configuration(
            network,
            stripe_api_base_url,
            "pmc_paypal_enabled",
        );

        let report = execute_stripe_funding_intent_checkout(State(state), Path(funding_intent.id))
            .await
            .unwrap()
            .0;

        assert_eq!(report.status, 200);
        assert_eq!(report.request.endpoint, "/v1/checkout/sessions");
        assert_eq!(
            report.request.idempotency_key,
            format!("bounty_funding_intent:{}", funding_intent.id)
        );
        assert_eq!(
            report.request.body["success_url"],
            "https://nspg13.github.io/agent-bounties/success.html"
        );
        assert_eq!(
            report.request.body["cancel_url"],
            "https://nspg13.github.io/agent-bounties/cancel.html"
        );
        assert_eq!(
            report.request.body["metadata"]["bounty_id"],
            bounty.id.to_string()
        );
        assert_eq!(
            report.request.body["metadata"]["funding_intent_id"],
            funding_intent.id.to_string()
        );
        assert_eq!(
            report.request.body["payment_method_configuration"],
            "pmc_paypal_enabled"
        );
        assert!(report.request.body.get("payment_method_types").is_none());
        assert_eq!(
            report.url.as_deref(),
            Some("https://checkout.stripe.com/c/pay/cs_test_bounty")
        );
    }

    #[tokio::test]
    async fn public_stripe_funding_intent_checkout_is_disabled_by_default() {
        let state =
            test_state_with_stripe_live(BountyNetwork::default(), "http://127.0.0.1:9".to_string());

        let error = execute_stripe_funding_intent_checkout(State(state), Path(Uuid::new_v4()))
            .await
            .unwrap_err();

        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn live_stripe_execution_is_disabled_by_default() {
        let state = test_state(BountyNetwork::default());

        let error = execute_stripe_connect_account(
            State(state),
            HeaderMap::new(),
            Json(PlanStripeConnectAccountRequest {
                agent_id: Uuid::new_v4(),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn github_issue_bounty_plan_parses_valid_issue_form() {
        let plan = plan_github_issue_bounty(Json(PlanGitHubIssueBountyRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/7".to_string(),
            title: "[bounty]: Fix CI".to_string(),
            body: valid_github_issue_body(),
        }))
        .await
        .0;

        assert!(plan.ready);
        assert!(plan.error.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
        let parsed = plan.parsed.expect("parsed bounty");
        assert_eq!(parsed.template_slug, "fix-ci-failure");
        assert_eq!(parsed.amount.amount, 10_000_000);
        assert_eq!(parsed.amount.currency, "usdc");
    }

    #[tokio::test]
    async fn github_issue_bounty_plan_returns_action_required_for_bad_issue_form() {
        let plan = plan_github_issue_bounty(Json(PlanGitHubIssueBountyRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/8".to_string(),
            title: "[bounty]: Missing fields".to_string(),
            body: "### Goal\nFix CI".to_string(),
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.parsed.is_none());
        assert!(plan.error.expect("error").contains("missing required"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn public_pooled_bounty_cannot_overwrite_existing_unfunded_bounty() {
        let state = test_state(BountyNetwork::default());
        let existing = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Original public bounty".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let error = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: Some(existing.id),
                idempotency_key: Some(format!(
                    "github-issue-sync:agent-bounties/example:{}",
                    existing.id
                )),
                title: "Overwrite public bounty".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 2_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
        let status = bounty_status(State(state), Path(existing.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.bounty.title, "Original public bounty");
        assert_eq!(status.bounty.privacy, PrivacyLevel::Public);
        assert_eq!(status.bounty.amount.amount, 1_000);
    }

    #[tokio::test]
    async fn github_issue_api_sync_is_operator_gated_and_title_edit_stable() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let denied = sync_github_issue_api_bounty(
            State(state.clone()),
            HeaderMap::new(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let first = sync_github_issue_api_bounty(
            State(state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        let edited = sync_github_issue_api_bounty(
            State(state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                115,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body_with_goal(
                    "Keep the same hosted bounty after an issue title edit.",
                ),
            )),
        )
        .await
        .unwrap()
        .0;

        let other_issue = sync_github_issue_api_bounty(
            State(state),
            headers,
            Json(github_issue_api_sync_request(
                116,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(edited.id, first.id);
        assert_ne!(other_issue.id, first.id);
        assert_eq!(
            edited.title,
            "[bounty]: Sync hosted GitHub issue bounty records"
        );
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn audience_audit_persists_idempotently_across_processes() {
        let database_url = postgres_test_database_url();
        let first_store = PostgresStore::connect(&database_url).await.unwrap();
        first_store.migrate().await.unwrap();
        let first_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            first_store,
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let unique = Uuid::new_v4();

        let member = upsert_audience_member(
            State(first_state.clone()),
            headers.clone(),
            Json(UpsertAudienceMemberRequest {
                provider: domain::AudienceProvider::Github,
                external_id: format!("github-user-{unique}"),
                handle: format!("audience-{unique}"),
                public_profile_url: Some(format!("https://github.com/audience-{unique}")),
                roles: vec![],
                observed_at: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let stale_store = PostgresStore::connect(&database_url).await.unwrap();
        let stale_network = hydrate_network(&stale_store).await.unwrap();
        let stale_state =
            test_state_with_operator_token_and_store(stale_network, "secret-token", stale_store);
        let event_id = format!("pull-request:{unique}");
        let interaction = record_audience_interaction(
            State(first_state),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id.clone(),
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: Some(format!(
                    "https://github.com/NSPG13/agent-bounties/pull/{unique}"
                )),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        let star = record_audience_interaction(
            State(stale_state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: format!("star:{unique}"),
                kind: domain::AudienceInteractionKind::RepoStarred,
                public_url: Some("https://github.com/NSPG13/agent-bounties/stargazers".to_string()),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_ne!(star.id, interaction.id);
        let conflicting_replay = record_audience_interaction(
            State(stale_state),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id.clone(),
                kind: domain::AudienceInteractionKind::FundingSignaled,
                public_url: interaction.public_url.clone(),
                occurred_at: Some(interaction.occurred_at),
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflicting_replay, StatusCode::CONFLICT);

        let second_store = PostgresStore::connect(&database_url).await.unwrap();
        let second_network = hydrate_network(&second_store).await.unwrap();
        let second_state =
            test_state_with_operator_token_and_store(second_network, "secret-token", second_store);
        let replay = record_audience_interaction(
            State(second_state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: event_id,
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: interaction.public_url.clone(),
                occurred_at: Some(interaction.occurred_at),
                referrer_url: None,
                campaign: Some("postgres-audience-test".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(replay.id, interaction.id);

        let persisted = list_audience_interactions(State(second_state.clone()), headers.clone())
            .await
            .unwrap()
            .0
            .into_iter()
            .filter(|candidate| candidate.audience_member_id == member.id)
            .collect::<Vec<_>>();
        assert_eq!(persisted.len(), 2);
        let persisted_member = list_audience_members(State(second_state.clone()), headers.clone())
            .await
            .unwrap()
            .0
            .into_iter()
            .find(|candidate| candidate.id == member.id)
            .unwrap();
        assert!(persisted_member
            .roles
            .contains(&domain::AudienceRole::Contributor));
        assert!(persisted_member
            .roles
            .contains(&domain::AudienceRole::Promoter));
        assert_eq!(
            persisted_member.lifecycle_stage,
            domain::AudienceLifecycleStage::Retained
        );
        let report = audience_report(State(second_state), headers)
            .await
            .unwrap()
            .0;
        assert!(report.total_members >= 1);
        assert!(report.total_interactions >= 1);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn github_issue_api_sync_postgres_rejects_stale_cross_process_activity() {
        let database_url = postgres_test_database_url();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let sync_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            store.clone(),
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let issue_number = (Uuid::new_v4().as_u128() % 1_000_000_000_000) as u64 + 1;

        let first = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        )
        .await
        .unwrap()
        .0;

        let edited = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync hosted GitHub issue bounty records",
                valid_github_issue_body_with_goal(
                    "Keep the same hosted bounty after an issue title edit.",
                ),
            )),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(edited.id, first.id);
        assert_eq!(
            edited.title,
            "[bounty]: Sync hosted GitHub issue bounty records"
        );

        let mcp_store = PostgresStore::connect(&database_url).await.unwrap();
        let mcp_network = hydrate_network(&mcp_store).await.unwrap();
        assert!(mcp_network.bounties.contains_key(&first.id));
        let mcp_state =
            test_state_with_operator_token_and_store(mcp_network, "secret-token", mcp_store);
        let funding_report = create_funding_intent(
            State(mcp_state),
            Path(first.id),
            Json(CreateFundingIntentRequest {
                bounty_id: first.id,
                contributor_agent_id: None,
                source_organization_id: Some(Uuid::new_v4()),
                amount_minor: first.amount.amount,
                currency: first.amount.currency.clone(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some(format!("stale-sync-{issue_number}")),
                stripe_success_url: None,
                stripe_cancel_url: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(funding_report.0.intent.bounty_id, first.id);
        assert_eq!(
            funding_report.0.intent.status,
            FundingIntentStatus::AwaitingEvidence
        );

        let rejected = sync_github_issue_api_bounty(
            State(sync_state.clone()),
            headers,
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Unsafe edit after funding activity",
                valid_github_issue_body_with_goal(
                    "This stale API process must not overwrite a funded row.",
                ),
            )),
        )
        .await
        .unwrap_err();
        assert_eq!(rejected, StatusCode::BAD_REQUEST);

        let persisted = store
            .list_bounties()
            .await
            .unwrap()
            .into_iter()
            .find(|bounty| bounty.id == first.id)
            .unwrap();
        assert_eq!(persisted.title, edited.title);
        assert_eq!(persisted.amount.amount, edited.amount.amount);
        assert_eq!(persisted.status, BountyStatus::Unfunded);

        let funding_intents = store
            .list_funding_intents()
            .await
            .unwrap()
            .into_iter()
            .filter(|intent| intent.bounty_id == first.id)
            .collect::<Vec<_>>();
        assert_eq!(funding_intents.len(), 1);

        let stale_status = bounty_status(State(sync_state), Path(first.id))
            .await
            .unwrap()
            .0;
        assert_eq!(stale_status.bounty.title, edited.title);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn github_issue_api_sync_postgres_serializes_concurrent_initial_sync() {
        let database_url = postgres_test_database_url();
        let first_store = PostgresStore::connect(&database_url).await.unwrap();
        first_store.migrate().await.unwrap();
        let second_store = PostgresStore::connect(&database_url).await.unwrap();
        let first_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            first_store.clone(),
        );
        let second_state = test_state_with_operator_token_and_store(
            BountyNetwork::default(),
            "secret-token",
            second_store,
        );
        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let issue_number = (Uuid::new_v4().as_u128() % 1_000_000_000_000) as u64 + 1;

        let first_sync = sync_github_issue_api_bounty(
            State(first_state),
            headers.clone(),
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        );
        let second_sync = sync_github_issue_api_bounty(
            State(second_state),
            headers,
            Json(github_issue_api_sync_request(
                issue_number,
                "[bounty]: Sync GitHub issue into API",
                valid_github_issue_body(),
            )),
        );
        let (first_result, second_result) = tokio::join!(first_sync, second_sync);
        let first = first_result.unwrap().0;
        let second = second_result.unwrap().0;

        assert_eq!(first.id, second.id);
        assert_eq!(first.title, "[bounty]: Sync GitHub issue into API");
        assert_eq!(second.title, "[bounty]: Sync GitHub issue into API");

        let matching_bounties = first_store
            .list_bounties()
            .await
            .unwrap()
            .into_iter()
            .filter(|bounty| bounty.id == first.id)
            .collect::<Vec<_>>();
        assert_eq!(matching_bounties.len(), 1);
        assert_eq!(matching_bounties[0].status, BountyStatus::Unfunded);
        assert_eq!(matching_bounties[0].amount.amount, first.amount.amount);
    }

    #[tokio::test]
    async fn github_funding_comment_plan_flags_operator_reconciliation() {
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("funding signal");
        assert!(signal.requires_operator_reconciliation);
        assert_eq!(signal.amount.currency, "usdc");
        assert!(signal.funding_handoff_url.is_none());
        assert!(signal.idempotency_key.ends_with(":comment:123"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_funding_comment_plan_returns_stripe_handoff_url() {
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body_with_funding_mode("StripeFiatLedger"),
            comment_body: "/agent-bounty fund 5 USD via StripeFiatLedger".to_string(),
            contributor_login: Some("human-funder".to_string()),
            comment_id: Some("124".to_string()),
            funding_api_base_url: Some("https://api.agentbounties.example".to_string()),
            existing_idempotency_keys: vec![],
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("funding signal");
        let handoff = signal.funding_handoff_url.expect("handoff url");
        assert!(handoff.contains("https://nspg13.github.io/agent-bounties/funding.html"));
        assert!(handoff.contains("apiBaseUrl=https%3A%2F%2Fapi.agentbounties.example"));
        assert!(handoff.contains("rail=StripeFiat"));
        assert!(handoff.contains("externalReference=github-funding-comment%3A"));
        assert!(plan.check.text.contains("Stripe Checkout funding handoff"));
        assert!(plan
            .check
            .text
            .contains("verified Stripe webhook reconciliation"));
    }

    #[tokio::test]
    async fn github_funding_comment_plan_rejects_duplicate_signal() {
        let existing_key =
            "github-funding-comment:agent-bounties/agent-bounties:https://github.com/agent-bounties/agent-bounties/issues/20:comment:123";
        let plan = plan_github_funding_comment(Json(PlanGitHubFundingCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Co-funding".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![existing_key.to_string()],
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("duplicate funding signal"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn github_claim_comment_plan_reserves_progress_backed_claim() {
        let plan = plan_github_claim_comment(Json(PlanGitHubClaimCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_github_issue_body(),
            comment_body: "/agent-bounty claim\nPlan: add deterministic stale claim tests."
                .to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("456".to_string()),
            claim_age_minutes: Some(5),
            progress_signal_count: 0,
            active_claim_login: None,
        }))
        .await
        .0;

        assert!(plan.ready);
        let signal = plan.signal.expect("claim signal");
        assert!(!signal.settlement_authority);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_claim_comment_plan_rejects_templated_claim_without_progress() {
        let plan = plan_github_claim_comment(Json(PlanGitHubClaimCommentRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_github_issue_body(),
            comment_body:
                "/agent-bounty claim\nI'm reviewing the codebase and will open a PR shortly."
                    .to_string(),
            contributor_login: Some("claim-bot".to_string()),
            comment_id: Some("457".to_string()),
            claim_age_minutes: Some(1),
            progress_signal_count: 0,
            active_claim_login: None,
        }))
        .await
        .0;

        assert!(!plan.ready);
        assert!(plan.signal.is_none());
        assert!(plan.error.unwrap().contains("concrete progress signal"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_returns_markdown_and_fingerprint() {
        let bounty_id = Uuid::new_v4();
        let plan = plan_github_proof_comment(Json(PlanGitHubProofCommentRequest {
            bounty_id,
            proof_url: "https://agentbounties.local/public/proofs/abc".to_string(),
            verifier_summary: "GitHub CI passed".to_string(),
            settlement_url: Some("https://basescan.org/tx/0xabc".to_string()),
        }))
        .await
        .0;

        assert_eq!(plan.comment.bounty_id, bounty_id);
        assert!(plan.markdown.contains("Proof:"));
        assert!(plan.markdown.contains("Settlement:"));
        assert_eq!(plan.fingerprint.len(), 64);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_from_proof_uses_stored_public_proof() {
        let (network, bounty, proof) = completed_simulated_bounty().await;
        let state = test_state(network);
        let plan = plan_github_proof_comment_from_proof(
            State(state),
            Json(PlanGitHubProofCommentFromProofRequest {
                proof_id: proof.id,
                settlement_url: Some("https://basescan.org/tx/0xabc".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(plan.comment.bounty_id, bounty.id);
        assert_eq!(
            plan.comment.proof_url,
            format!("http://127.0.0.1:8080/public/proofs/{}", proof.id)
        );
        assert!(plan.comment.verifier_summary.contains("JsonSchema"));
        assert!(plan.markdown.contains("Settlement:"));
        assert_eq!(plan.fingerprint.len(), 64);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[tokio::test]
    async fn github_proof_comment_plan_from_proof_rejects_private_proofs() {
        let (mut network, _bounty, mut proof) = completed_simulated_bounty().await;
        proof.privacy = PrivacyLevel::Private;
        network.proofs.insert(proof.id, proof.clone());
        let state = test_state(network);
        let error = plan_github_proof_comment_from_proof(
            State(state.clone()),
            Json(PlanGitHubProofCommentFromProofRequest {
                proof_id: proof.id,
                settlement_url: None,
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::NOT_FOUND);
        let public_error = public_proof_page(State(state), Path(proof.id))
            .await
            .unwrap_err();
        assert_eq!(public_error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn discovery_endpoint_advertises_autonomous_protocol_only() {
        let state = test_state(BountyNetwork::default());
        let manifest = agent_bounties_discovery(State(state)).await.0;

        assert_eq!(
            manifest.schema,
            "https://agentbounties.org/schemas/discovery-manifest.v2.json"
        );
        assert_eq!(manifest.protocol["version"], "agent-bounties/autonomous-v1");
        assert_eq!(manifest.protocol["operator_settlement_signer"], false);
        assert_eq!(
            manifest.endpoints.autonomous_canonical_child_terms_plan,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/canonical-child-terms-plan"
        );
        assert_eq!(
            manifest.endpoints.autonomous_creation_plan,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/creation-plan"
        );
        assert_eq!(
            manifest.endpoints.autonomous_bounty_feed,
            "http://127.0.0.1:8080/v1/base/autonomous-bounties/feed"
        );
        assert!(manifest
            .agent_tools
            .iter()
            .any(|tool| tool == "plan_autonomous_canonical_child_terms"));
        assert!(manifest
            .agent_tools
            .iter()
            .any(|tool| tool == "plan_autonomous_bounty_submission"));
        assert!(manifest
            .agent_tools
            .iter()
            .all(|tool| !tool.starts_with("plan_base_")));
    }

    #[tokio::test]
    async fn canonical_child_terms_endpoint_matches_contract_vectors() {
        let plan = plan_autonomous_canonical_child_terms(Json(CanonicalChildBountyTermsRequest {
            parent_bounty_id: format!("0x{}", "ab".repeat(32)),
            parent_round: 1,
            parent_solver: "0x3333333333333333333333333333333333333333".to_string(),
            parent_solver_reward: Money::new(900_000, "usdc").unwrap(),
            child_acceptance_criteria: vec!["Produce the committed digital artifact.".to_string()],
            verifier_module: "0x4444444444444444444444444444444444444444".to_string(),
        }))
        .await
        .unwrap()
        .0;

        assert_eq!(
            plan.acceptance_criteria_hash,
            chain_base::keccak256_canonical_json(&serde_json::json!(plan.acceptance_criteria))
                .unwrap()
        );
        assert_eq!(plan.required_child_status, "settled");
        assert_eq!(plan.minimum_child_target.amount, 900_000);
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_reports_non_secret_defaults() {
        let state = test_state(BountyNetwork::default());
        let report = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.network, "Base");
        assert_eq!(report.network_chain_id, 8_453);
        assert_eq!(report.stripe_secret_key_mode, "unset");
        assert!(!report.stripe_payment_method_configuration_configured);
        assert_eq!(report.supplied_usdc_token_matches_native, Some(true));
        assert!(!report.live_money_ready);
        assert!(report
            .checks
            .iter()
            .any(|check| { check.name == "Autonomous bounty factory" && check.configured }));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "Stripe live-money execution gate"));
        assert!(report
            .checks
            .iter()
            .any(|check| check.name == "Stripe Checkout payment-method configuration"));
        assert!(!serde_json::to_string(&report)
            .unwrap()
            .contains("pmc_paypal_enabled"));
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_reports_payment_method_configuration_without_id() {
        let state = test_state_with_stripe_payment_method_configuration(
            BountyNetwork::default(),
            "pmc_paypal_enabled",
        );
        let report = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        let json = serde_json::to_string(&report).unwrap();

        assert!(report.stripe_payment_method_configuration_configured);
        assert!(report.checks.iter().any(|check| {
            check.name == "Stripe Checkout payment-method configuration"
                && check.configured
                && check.env_vars == vec!["STRIPE_PAYMENT_METHOD_CONFIGURATION".to_string()]
        }));
        assert!(!json.contains("pmc_paypal_enabled"));
    }

    #[tokio::test]
    async fn live_money_readiness_endpoint_rejects_unknown_network() {
        let state = test_state(BountyNetwork::default());
        let error = live_money_readiness(
            State(state),
            Query(LiveMoneyReadinessQuery {
                network: Some("optimism".to_string()),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(error, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn api_docs_endpoint_points_to_openapi_json() {
        let html = api_docs().await.0;

        assert!(html.contains("/api-docs/openapi.json"));
        assert!(html.contains("/llms.txt"));
        assert!(html.contains("/schemas/discovery-manifest.v2.json"));
        assert!(html.contains("/.well-known/agent-bounties.json"));
    }

    #[test]
    fn mainnet_planner_uses_only_the_canonical_attested_deployment() {
        let expected = autonomous_planner_addresses(8_453, None, None).unwrap();
        assert_eq!(expected.0, CANONICAL_BASE_MAINNET_BOUNTY_FACTORY);
        assert_eq!(expected.1, CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION);

        let matching = autonomous_planner_addresses(
            8_453,
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_uppercase()),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string()),
        )
        .unwrap();
        assert_eq!(matching, expected);

        assert_eq!(
            autonomous_planner_addresses(
                8_453,
                Some("0x1111111111111111111111111111111111111111".to_string()),
                None,
            ),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
    }

    #[test]
    fn mainnet_readiness_uses_canonical_factory_and_rejects_drift() {
        assert_eq!(
            canonical_mainnet_factory(None, None).as_deref(),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        );
        assert_eq!(
            canonical_mainnet_factory(
                Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_uppercase()),
                Some(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_uppercase()),
            )
            .as_deref(),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        );
        assert_eq!(
            canonical_mainnet_factory(
                Some("0x1111111111111111111111111111111111111111".to_string()),
                None,
            ),
            None
        );
        assert_eq!(
            canonical_mainnet_factory(
                None,
                Some("0x2222222222222222222222222222222222222222".to_string()),
            ),
            None
        );
    }

    #[test]
    fn sepolia_planner_still_requires_explicit_addresses() {
        assert_eq!(
            autonomous_planner_addresses(84_532, None, None),
            Err(StatusCode::SERVICE_UNAVAILABLE)
        );
        assert!(autonomous_planner_addresses(
            84_532,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            Some("0x2222222222222222222222222222222222222222".to_string()),
        )
        .is_ok());
    }

    #[tokio::test]
    async fn openapi_json_endpoint_contains_agent_router_path() {
        let document = openapi_json().await.0;
        let value = serde_json::to_value(document).unwrap();
        let paths = value["paths"].as_object().unwrap();

        assert!(paths.contains_key("/v1/route-blocked-goal"));
        assert!(paths.contains_key("/llms.txt"));
        assert!(paths.contains_key("/schemas/discovery-manifest.v2.json"));
        assert!(paths.contains_key("/v1/risk/policy"));
        assert!(paths.contains_key("/v1/readiness/live-money"));
        assert!(paths.contains_key("/v1/risk/events"));
        assert!(paths.contains_key("/v1/risk/reviews"));
        assert!(paths.contains_key("/v1/risk/bounty-approvals"));
        assert!(paths.contains_key("/v1/risk/payout-approvals"));
        assert!(paths.contains_key("/v1/risk/events/{id}/reject"));
        assert!(paths.contains_key("/v1/agents/{id}/paid-status"));
        assert!(paths.contains_key("/v1/contributor-contacts"));
        assert!(paths.contains_key("/v1/audience/members"));
        assert!(paths.contains_key("/v1/audience/interactions"));
        assert!(paths.contains_key("/v1/audience/discovery-responses"));
        assert!(paths.contains_key("/v1/audience/outreach-attempts"));
        assert!(paths.contains_key("/v1/audience/report"));
        assert!(paths.contains_key("/v1/capabilities/search"));
        assert!(paths.contains_key("/.well-known/x402.json"));
        assert!(paths.contains_key("/v1/x402/base/bounties/{bounty_contract}/funding"));
        assert!(paths.contains_key("/v1/x402/base/relays/{relay_id}"));
        assert!(paths.contains_key("/v1/base/broadcast-signed-transaction"));
        assert!(paths.contains_key("/v1/base/transaction-receipt"));
        for autonomous in [
            "/v1/base/autonomous-bounties/canonical-child-terms-plan",
            "/v1/base/autonomous-bounties/creation-plan",
            "/v1/base/autonomous-bounties/authorized-creation-plan",
            "/v1/base/autonomous-bounties/contribution-plan",
            "/v1/base/autonomous-bounties/authorized-contribution-plan",
            "/v1/base/autonomous-bounties/claim-plan",
            "/v1/base/autonomous-bounties/authorized-claim-plan",
            "/v1/base/autonomous-bounties/submission-plan",
            "/v1/base/autonomous-bounties/submission-preparation",
            "/v1/base/autonomous-bounties/submission-authorization-plan",
            "/v1/base/autonomous-bounties/verification-jobs",
            "/v1/base/autonomous-bounties/decode-events",
            "/v1/base/autonomous-bounties/events",
            "/v1/base/autonomous-bounties/terms",
            "/v1/base/autonomous-bounties/terms/{terms_hash}",
            "/v1/base/autonomous-bounties/feed",
        ] {
            assert!(paths.contains_key(autonomous), "missing {autonomous}");
        }
        for objective in [
            "/v1/objectives/creation-plans",
            "/v1/objectives",
            "/v1/objectives/{id}",
            "/v1/objectives/{id}/action-plans",
            "/v1/objectives/{id}/actions",
            "/v1/objectives/{id}/reconcile",
        ] {
            assert!(paths.contains_key(objective), "missing {objective}");
        }
        for retired in [
            "/v1/base/indexer-status",
            "/v1/base/escrow-events",
            "/v1/base/evm-logs",
            "/v1/base/log-query",
            "/v1/base/rpc-logs",
            "/v1/base/fetch-rpc-logs",
            "/v1/base/funding-plan",
            "/v1/base/release-queue",
            "/v1/base/release-plan",
            "/v1/base/refund-plan",
            "/v1/base/dispute-plan",
        ] {
            assert!(
                !paths.contains_key(retired),
                "retired path leaked: {retired}"
            );
        }
        assert!(paths.contains_key("/v1/stripe/live/checkout-top-ups"));
        assert!(paths.contains_key("/v1/stripe/live/funding-intents/{id}/checkout-session"));
        assert!(paths.contains_key("/v1/stripe/live/connect-accounts"));
        assert!(paths.contains_key("/v1/stripe/connect-snapshots"));
        assert!(paths.contains_key("/v1/stripe/checkout-webhooks"));
        assert!(paths.contains_key("/v1/bounties/{id}/funding-intents"));
        assert!(paths.contains_key("/v1/github/issue-bounty-plan"));
        assert!(paths.contains_key("/v1/github/issue-api-sync-plan"));
        assert!(paths.contains_key("/v1/github/issue-api-sync"));
        assert!(paths.contains_key("/v1/github/funding-comment-plan"));
        assert!(paths.contains_key("/v1/github/claim-comment-plan"));
        assert!(paths.contains_key("/v1/github/proof-comment-plan"));
        assert!(paths.contains_key("/v1/github/proof-comment-plan-from-proof"));
        assert!(paths.contains_key("/v1/evals/loops"));
        assert!(paths.contains_key("/v1/evals/runs"));

        let security_schemes = value["components"]["securitySchemes"]
            .as_object()
            .expect("security schemes");
        assert_eq!(
            security_schemes["operator_api_token"]["name"],
            OPERATOR_TOKEN_HEADER
        );
        assert_eq!(security_schemes["operator_api_token"]["in"], "header");
        assert_eq!(security_schemes["operator_bearer"]["scheme"], "bearer");

        for path in [
            "/v1/risk/bounty-approvals",
            "/v1/risk/payout-approvals",
            "/v1/risk/events/{id}/reject",
            "/v1/contributor-contacts",
            "/v1/base/broadcast-signed-transaction",
            "/v1/stripe/live/checkout-top-ups",
            "/v1/stripe/live/connect-accounts",
            "/v1/stripe/connect-snapshots",
            "/v1/github/issue-api-sync",
        ] {
            let security = paths[path]["post"]["security"].as_array().unwrap();
            assert!(
                security
                    .iter()
                    .any(|requirement| requirement.get("operator_api_token").is_some()),
                "{path} missing operator_api_token security"
            );
            assert!(
                security
                    .iter()
                    .any(|requirement| requirement.get("operator_bearer").is_some()),
                "{path} missing operator_bearer security"
            );
            assert!(paths[path]["post"]["responses"]["401"].is_object());
        }

        assert!(paths["/v1/base/transaction-receipt"]["post"]
            .get("security")
            .is_none());
        assert!(paths["/v1/base/transaction-receipt"]["post"]["responses"]
            .get("401")
            .is_none());

        assert!(
            paths["/v1/stripe/live/funding-intents/{id}/checkout-session"]["post"]
                .get("security")
                .is_none(),
            "Public funder Checkout must not require operator auth"
        );
        assert!(
            paths["/v1/stripe/live/funding-intents/{id}/checkout-session"]["post"]["responses"]
                ["503"]
                .is_object()
        );
        assert!(
            paths["/v1/stripe/checkout-webhooks"]["post"]
                .get("security")
                .is_none(),
            "Stripe checkout webhook must remain callable by Stripe without operator auth"
        );
        assert!(paths["/v1/stripe/checkout-webhooks"]["post"]["responses"]["503"].is_object());
    }

    #[tokio::test]
    async fn contributor_contacts_are_operator_gated() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let denied = upsert_contributor_contact(
            State(state.clone()),
            HeaderMap::new(),
            Json(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec!["#24".to_string()],
                contact_consent: false,
                wallet_consent: true,
                outreach_allowed: false,
                source: Some("github-comment-opt-in".to_string()),
                notes: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let contact = upsert_contributor_contact(
            State(state.clone()),
            headers.clone(),
            Json(UpsertContributorContactRequest {
                github_login: "qilu13".to_string(),
                email: None,
                payout_wallet: Some("0x1111111111111111111111111111111111111111".to_string()),
                associated_prs: vec!["#24".to_string()],
                contact_consent: false,
                wallet_consent: true,
                outreach_allowed: false,
                source: Some("github-comment-opt-in".to_string()),
                notes: None,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(contact.github_login, "qilu13");
        assert!(contact.wallet_consent);
        let contacts = list_contributor_contacts(State(state), headers)
            .await
            .unwrap()
            .0;
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].associated_prs, vec!["#24".to_string()]);
    }

    #[tokio::test]
    async fn audience_audit_is_operator_gated_and_reports_public_attribution() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");
        let request = UpsertAudienceMemberRequest {
            provider: domain::AudienceProvider::Github,
            external_id: "U_123".to_string(),
            handle: "nexicturbo".to_string(),
            public_profile_url: Some("https://github.com/nexicturbo".to_string()),
            roles: vec![],
            observed_at: None,
        };
        let denied = upsert_audience_member(
            State(state.clone()),
            HeaderMap::new(),
            Json(request.clone()),
        )
        .await
        .unwrap_err();
        assert_eq!(denied, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let member = upsert_audience_member(State(state.clone()), headers.clone(), Json(request))
            .await
            .unwrap()
            .0;
        let _ = record_audience_interaction(
            State(state.clone()),
            headers.clone(),
            Json(RecordAudienceInteractionRequest {
                audience_member_id: member.id,
                provider_event_id: "pull-request:138".to_string(),
                kind: domain::AudienceInteractionKind::PullRequestOpened,
                public_url: Some("https://github.com/NSPG13/agent-bounties/pull/138".to_string()),
                occurred_at: None,
                referrer_url: None,
                campaign: Some("github-bounty-label".to_string()),
                source_interaction_id: None,
            }),
        )
        .await
        .unwrap();
        let _ = record_outreach_attempt(
            State(state.clone()),
            headers.clone(),
            Json(RecordOutreachAttemptRequest {
                audience_member_id: member.id,
                provider_event_id: "issue-comment:feedback:138".to_string(),
                channel: domain::OutreachChannel::GithubPublic,
                public_url: Some(
                    "https://github.com/NSPG13/agent-bounties/pull/138#issuecomment-1".to_string(),
                ),
                prompt_version: "distribution-v1".to_string(),
                status: domain::OutreachStatus::Responded,
                sent_at: None,
            }),
        )
        .await
        .unwrap();
        let _ = record_discovery_response(
            State(state.clone()),
            headers.clone(),
            Json(RecordDiscoveryResponseRequest {
                audience_member_id: member.id,
                interaction_id: None,
                provider_response_id: "pr-body:138".to_string(),
                public_source_url: Some(
                    "https://github.com/NSPG13/agent-bounties/pull/138".to_string(),
                ),
                found_via: "GitHub issue list".to_string(),
                motivation: "clear payout-integrity scope".to_string(),
                improvement_suggestion: "show durable payment evidence".to_string(),
                agent_or_tool: Some("coding agent".to_string()),
                private_storage_consent: false,
                captured_at: None,
            }),
        )
        .await
        .unwrap();

        let report = audience_report(State(state), headers).await.unwrap().0;
        assert_eq!(report.total_members, 1);
        assert_eq!(report.total_interactions, 1);
        assert_eq!(report.members_asked_for_discovery_feedback, 1);
        assert_eq!(report.members_with_discovery_responses, 1);
    }

    #[tokio::test]
    async fn eval_endpoints_record_local_run_history() {
        let state = test_state(BountyNetwork::default());

        let result = run_bountybench(State(state.clone())).await.unwrap().0;
        assert!(result.passed);

        let runs = list_eval_runs(State(state)).await.0;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].suite, "BountyBench/router-v0");
        assert!(runs[0].passed);
    }

    #[tokio::test]
    async fn risk_policy_endpoint_exposes_settlement_limits() {
        let policy = risk_policy().await.0;

        assert_eq!(policy.low_value_usdc_cap_minor, 10_000_000);
        assert_eq!(policy.low_value_usdc_cap_currency, "usdc");
        assert!(!policy.ai_judges_can_authorize_payment);
        assert!(policy
            .settlement_invariants
            .iter()
            .any(|rule| rule.contains("Stripe ledger credits")));
    }

    #[tokio::test]
    async fn risk_events_endpoint_lists_review_queue() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let state = test_state(network);

        let events = list_risk_events(
            State(state),
            Query(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(10),
                ..RiskEventFilter::default()
            }),
        )
        .await
        .0;

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, domain::RiskAction::NeedsReview);
        assert!(events[0].reasons[0].contains("low-value cap"));
    }

    #[tokio::test]
    async fn risk_bounty_approval_endpoint_creates_funding_ready_bounty() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let risk_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let state = test_state(network);

        let approval = approve_risk_bounty(
            State(state.clone()),
            HeaderMap::new(),
            Json(ApproveRiskBountyRequest {
                risk_event_id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved after manual scope review".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(approval.bounty.status, BountyStatus::Unfunded);
        assert!(approval.bounty.terms_hash.is_some());
        assert_eq!(approval.review.outcome, domain::RiskReviewOutcome::Approved);
        let reviews = list_risk_reviews(State(state)).await.0;
        assert_eq!(reviews.len(), 1);
    }

    #[tokio::test]
    async fn risk_rejection_endpoint_records_review_without_bounty() {
        let mut network = BountyNetwork::default();
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let risk_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let state = test_state(network);

        let review = reject_risk_event(
            State(state.clone()),
            HeaderMap::new(),
            Path(risk_event_id),
            Json(RejectRiskEventRequest {
                risk_event_id: Uuid::nil(),
                operator_id: "operator-1".to_string(),
                note: "Rejected until payer completes manual onboarding".to_string(),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(review.outcome, domain::RiskReviewOutcome::Rejected);
        let network = state.network.lock().unwrap();
        assert!(network.bounties.is_empty());
    }

    #[tokio::test]
    async fn llms_txt_endpoint_points_agents_to_discovery_and_mcp() {
        let state = test_state(BountyNetwork::default());

        let text = llms_txt(State(state)).await;

        assert!(text.contains("# Agent Bounties"));
        assert!(text.contains("/.well-known/agent-bounties.json"));
        assert!(text.contains("docs/agent-quickstart.md"));
        assert!(text.contains("http://127.0.0.1:8090/tools"));
        assert!(text.contains("route_blocked_goal"));
        assert!(text.contains("agent-bounties/autonomous-v1"));
        assert!(text.contains("list_autonomous_bounties"));
        assert!(text.contains("BountySettled"));
        assert!(!text.contains("createEscrow"));
    }

    #[tokio::test]
    async fn public_bounty_feed_excludes_private_bounties() {
        let mut network = BountyNetwork::default();
        let public = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix public CI".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let private = network
            .post_funded_bounty(PostBountyRequest {
                title: "Private ledger work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 2_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
            })
            .unwrap();
        let state = test_state(network);

        let feed = public_bounty_feed(State(state)).await.0;

        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].bounty_id, public.id.to_string());
        assert_ne!(feed[0].bounty_id, private.id.to_string());
        assert_eq!(
            feed[0].claim_url,
            format!("http://127.0.0.1:8080/v1/bounties/{}/claim", public.id)
        );
        assert_eq!(
            feed[0].public_url,
            format!("http://127.0.0.1:8080/public/bounties/{}", public.id)
        );
    }

    #[tokio::test]
    async fn public_funding_feed_lists_only_public_bounties_with_remaining_funding() {
        let state = test_state(BountyNetwork::default());
        let partial = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund public docs".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let _partial_funding = add_funding_contribution(
            State(state.clone()),
            Path(partial.id),
            Json(AddFundingContributionRequest {
                bounty_id: partial.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("public-funding-feed-partial".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        let stripe = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund public Stripe work".to_string(),
                template_slug: "payment-state-machine".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let private = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund private work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Private,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let funded = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Funded public work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let _funded_contribution = add_funding_contribution(
            State(state.clone()),
            Path(funded.id),
            Json(AddFundingContributionRequest {
                bounty_id: funded.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 1_000,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("public-funding-feed-funded".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        let feed = public_funding_feed(State(state.clone())).await.0;
        let ids = feed
            .iter()
            .map(|item| item.bounty_id.clone())
            .collect::<Vec<_>>();

        assert!(ids.contains(&partial.id.to_string()));
        assert!(ids.contains(&stripe.id.to_string()));
        assert!(!ids.contains(&private.id.to_string()));
        assert!(!ids.contains(&funded.id.to_string()));
        let partial_item = feed
            .iter()
            .find(|item| item.bounty_id == partial.id.to_string())
            .expect("partial public bounty should be in funding feed");
        assert_eq!(partial_item.funding_remaining_minor, 600);
        assert!(partial_item
            .funding_partitions
            .iter()
            .any(|partition| partition.remaining_minor == 600));

        let html = public_funding_feed_page(State(state)).await.0;
        assert!(html.contains("Fundable Agent Bounties"));
        assert!(html.contains("agent-bounty-funding-feed"));
        assert!(html.contains(&format!("/public/bounties/{}", partial.id)));
        assert!(html.contains(&format!("/v1/bounties/{}/funding-intents", partial.id)));
        assert!(!html.contains(&format!("/public/bounties/{}", private.id)));
        assert!(!html.contains(&format!("/public/bounties/{}", funded.id)));
    }

    #[tokio::test]
    async fn public_bounty_detail_exposes_agent_actions() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix public <CI>".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let state = test_state(network);

        let html = public_bounty_page(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;

        assert!(html.contains("Fix public &lt;CI&gt;"));
        assert!(html.contains("Funding State"));
        assert!(html.contains("Funding partitions"));
        assert!(html.contains("application/ld+json"));
        assert!(html.contains("agent-bounty-public-status"));
        assert!(html.contains("Machine status"));
        assert!(html.contains(r#"data-agent-action="claim""#));
        assert!(!html.contains("Add funding"));
        assert!(!html.contains(r#"rel="payment""#));
        assert!(html.contains(&format!("/public/bounties/{}", bounty.id)));
        assert!(html.contains(&format!("/v1/bounties/{}/claim", bounty.id)));
        assert!(!html.contains(&format!("/v1/bounties/{}/funding-contributions", bounty.id)));
    }

    #[tokio::test]
    async fn public_bounty_detail_exposes_cofunding_while_target_remains() {
        let state = test_state(BountyNetwork::default());
        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund shared public work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;
        let partial = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400_000,
                currency: "USDC".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("partial-public-page".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.remaining.amount, 600_000);

        let html = public_bounty_page(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;

        assert!(html.contains("partially funded"));
        assert!(html.contains("Co-funding command:"));
        assert!(html.contains(&format!(
            "/agent-bounty fund {} 0.6 USDC via Simulated",
            bounty.id
        )));
        assert!(html.contains(r#"rel="payment""#));
        assert!(html.contains(r#"data-agent-action="add_funding_evidence""#));
        assert!(!html.contains(r#"data-agent-action="create_funding_intent""#));
        assert!(html.contains(&format!("/v1/bounties/{}/funding-contributions", bounty.id)));
        assert!(!html.contains(r#"data-agent-action="claim""#));
    }

    #[tokio::test]
    async fn public_bounty_detail_hides_private_bounties() {
        let mut network = BountyNetwork::default();
        let private = network
            .post_funded_bounty(PostBountyRequest {
                title: "Private ledger work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                amount_minor: 2_000_000,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Private,
            })
            .unwrap();
        let state = test_state(network);

        let error = public_bounty_page(State(state), Path(private.id))
            .await
            .unwrap_err();

        assert_eq!(error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn base_broadcast_signed_transaction_endpoint_returns_tx_hash() {
        let rpc_url = spawn_rpc_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 13,
            "result": format!("0x{}", "cc".repeat(32))
        }));
        let state = test_state_with_base_rpc(BountyNetwork::default(), rpc_url);

        let report = broadcast_base_signed_transaction(
            State(state),
            HeaderMap::new(),
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.network.chain_id, 84_532);
        assert_eq!(report.request.method, "eth_sendRawTransaction");
        assert_eq!(report.request.params[0], "0x010203");
        assert_eq!(report.tx_hash, format!("0x{}", "cc".repeat(32)));
        assert!(report.next_step.contains("transaction-receipt"));
    }

    #[tokio::test]
    async fn operator_token_blocks_protected_api_calls_when_configured() {
        let state = test_state_with_operator_token(BountyNetwork::default(), "secret-token");

        let error = broadcast_base_signed_transaction(
            State(state.clone()),
            HeaderMap::new(),
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        let error = broadcast_base_signed_transaction(
            State(state),
            headers,
            Json(BroadcastBaseSignedTransactionRequest {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn public_capability_search_finds_registered_solvers() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "capability-solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        network
            .register_capability(RegisterCapabilityRequest {
                agent_id: solver.id,
                class: CapabilityClass::Coding,
                template_slugs: vec!["small-code-change".to_string()],
                min_price_minor: 500_000,
                max_price_minor: 1_000_000,
                currency: "usdc".to_string(),
                latency_seconds: 600,
                supported_verifiers: vec![VerifierKind::JsonSchema],
            })
            .unwrap();
        let state = test_state(network);

        let feed = public_capability_feed(State(state.clone())).await.0;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].agent_id, solver.id.to_string());
        assert_eq!(
            feed[0].agent_profile_url,
            format!("http://127.0.0.1:8080/public/agents/{}", solver.id)
        );

        let search = search_capabilities(
            State(state),
            Json(SearchCapabilitiesRequest {
                class: Some(CapabilityClass::Coding),
                template_slug: Some("small-code-change".to_string()),
                currency: Some("USDC".to_string()),
                max_price_minor: Some(600_000),
            }),
        )
        .await
        .0;

        assert_eq!(search.len(), 1);
        assert_eq!(search[0].agent_handle, "capability-solver");
    }

    #[tokio::test]
    async fn pooled_funding_endpoints_make_bounty_claimable_at_target() {
        let state = test_state(BountyNetwork::default());

        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund shared docs work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let partial = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 400,
                currency: "USDC".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("first".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(partial.bounty.status, BountyStatus::Unfunded);
        assert_eq!(partial.funding_summary.remaining.amount, 600);

        let funded = add_funding_contribution(
            State(state.clone()),
            Path(bounty.id),
            Json(AddFundingContributionRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 600,
                currency: "usdc".to_string(),
                rail: PaymentRail::Simulated,
                external_reference: Some("second".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert!(funded.funding_summary.claimable);
        assert_eq!(funded.funding_summary.contribution_count, 2);

        let status = bounty_status(State(state.clone()), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.funding_contributions.len(), 2);
        assert_eq!(status.funding_summary.applied.amount, 1_000);
        let feed = list_claimable_bounties(State(state)).await.0;
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].id, bounty.id);
    }

    #[tokio::test]
    async fn funding_intent_endpoint_waits_for_verified_stripe_webhook() {
        let state = test_state_with_unsigned_stripe_webhooks(BountyNetwork::default());
        let organization_id = Uuid::new_v4();
        let bounty = open_pooled_bounty(
            State(state.clone()),
            Json(OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: "Fund Stripe API intent".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                target_amount_minor: 500,
                currency: "usd".to_string(),
                funding_mode: FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                funding_targets: vec![],
            }),
        )
        .await
        .unwrap()
        .0;

        let intent = create_funding_intent(
            State(state.clone()),
            Path(bounty.id),
            Json(CreateFundingIntentRequest {
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(organization_id),
                amount_minor: 500,
                currency: "usd".to_string(),
                rail: PaymentRail::StripeFiat,
                external_reference: Some("api-stripe-intent".to_string()),
                stripe_success_url: None,
                stripe_cancel_url: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(intent.requires_reconciliation);
        assert_eq!(intent.funding_summary.applied.amount, 0);

        let status_before = bounty_status(State(state.clone()), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status_before.funding_intents.len(), 1);
        assert!(status_before.funding_contributions.is_empty());

        let event = serde_json::json!({
            "id": "evt_api_intent",
            "type": "checkout.session.completed",
            "payload": {
                "id": "cs_api_intent",
                "client_reference_id": organization_id.to_string(),
                "amount_total": 500,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_api_intent",
                "metadata": {
                    "bounty_id": bounty.id.to_string(),
                    "funding_intent_id": intent.intent.id.to_string()
                }
            }
        });
        let reconciliation = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Bytes::from(serde_json::to_vec(&event).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert!(reconciliation.funding_report.is_some());
        assert_eq!(reconciliation.ledger_entries.len(), 2);

        let status_after = bounty_status(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status_after.funding_contributions.len(), 1);
        assert_eq!(status_after.funding_summary.applied.amount, 500);
        assert!(status_after.funding_summary.claimable);
    }

    #[tokio::test]
    async fn public_verifier_profile_summarizes_verifier_results() {
        let (network, _bounty, _proof) = completed_simulated_bounty().await;
        let state = test_state(network);

        let html = public_verifier_profile(State(state), Path("JsonSchema".to_string()))
            .await
            .unwrap()
            .0;

        assert!(html.contains("JsonSchema Verifier"));
        assert!(html.contains("Total checks"));
        assert!(html.contains("<dt>Accepted</dt><dd>1</dd>"));
    }

    fn test_state(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn postgres_test_database_url() -> String {
        std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("AGENT_BOUNTIES_TEST_DATABASE_URL must be set for ignored Postgres sync tests")
    }

    fn test_state_with_unsigned_stripe_webhooks(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: true,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_stripe_webhook_secret(network: BountyNetwork, secret: &[u8]) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: Some(secret.to_vec()),
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_operator_token(network: BountyNetwork, token: &str) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: Some(token.to_string()),
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_operator_token_and_store(
        network: BountyNetwork,
        token: &str,
        store: PostgresStore,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: Some(store),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: Some(token.to_string()),
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_base_rpc(
        network: BountyNetwork,
        base_sepolia_rpc_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig {
                base_sepolia: Some(base_sepolia_rpc_url),
                base_mainnet: None,
            },
            base_broadcast_enabled: true,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_stripe_live(
        network: BountyNetwork,
        stripe_api_base_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: Some("sk_test_mock".to_string()),
            stripe_live_execution_enabled: true,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url,
            stripe_payment_method_configuration: None,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_stripe_payment_method_configuration(
        network: BountyNetwork,
        payment_method_configuration: &str,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_public_checkout_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: Some(payment_method_configuration.to_string()),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_stripe_public_checkout_and_payment_method_configuration(
        network: BountyNetwork,
        stripe_api_base_url: String,
        payment_method_configuration: &str,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: Some("sk_test_mock".to_string()),
            stripe_live_execution_enabled: true,
            stripe_public_checkout_enabled: true,
            stripe_api_base_url,
            stripe_payment_method_configuration: Some(payment_method_configuration.to_string()),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            x402_relayer: X402HostedRelayerConfig::default(),
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn spawn_rpc_response(response: serde_json::Value) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer).unwrap();
            let body = response.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    #[test]
    fn api_bind_addr_prefers_explicit_config_then_host_port() {
        assert_eq!(
            service_bind_addr(Some("0.0.0.0:9000"), Some("10000"), "127.0.0.1:8080"),
            "0.0.0.0:9000"
        );
        assert_eq!(
            service_bind_addr(Some(""), Some("10000"), "127.0.0.1:8080"),
            "0.0.0.0:10000"
        );
        assert_eq!(
            service_bind_addr(None, Some(" 10001 "), "127.0.0.1:8080"),
            "0.0.0.0:10001"
        );
        assert_eq!(
            service_bind_addr(None, None, "127.0.0.1:8080"),
            "127.0.0.1:8080"
        );
    }

    async fn completed_simulated_bounty() -> (BountyNetwork, Bounty, ProofRecord) {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Extract data".to_string(),
                template_slug: "extract-data-to-schema".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let artifact = "{\"ok\":true}";
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://api/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: expected_digest_for_body(artifact),
                verifier_kind: Some(domain::VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();
        (network, bounty, proof)
    }

    fn stripe_checkout_event_body(
        event_id: &str,
        session_id: &str,
        organization_id: Uuid,
    ) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": event_id,
            "type": "checkout.session.completed",
            "payload": {
                "id": session_id,
                "client_reference_id": organization_id.to_string(),
                "amount_total": 5_000,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_paid"
            }
        }))
        .unwrap()
    }

    fn stripe_signature_header(payload: &[u8], secret: &[u8]) -> String {
        let timestamp = Utc::now().timestamp();
        let mut signed_payload = timestamp.to_string().into_bytes();
        signed_payload.push(b'.');
        signed_payload.extend_from_slice(payload);
        let mut mac = TestHmacSha256::new_from_slice(secret).unwrap();
        mac.update(&signed_payload);
        format!(
            "t={},v1={}",
            timestamp,
            hex::encode(mac.finalize().into_bytes())
        )
    }

    fn valid_github_issue_body() -> String {
        valid_github_issue_body_with_funding_mode("StripeFiatLedger")
    }

    fn github_issue_api_sync_request(
        issue_number: u64,
        title: &str,
        body: String,
    ) -> PlanGitHubIssueApiSyncRequest {
        PlanGitHubIssueApiSyncRequest {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: format!(
                "https://github.com/agent-bounties/agent-bounties/issues/{issue_number}"
            ),
            title: title.to_string(),
            body,
            api_base_url: Some("https://api.agentbounties.example".to_string()),
            existing_bounty_ids: vec![],
            hosted_api_error: None,
        }
    }

    fn valid_github_issue_body_with_goal(goal: &str) -> String {
        format!(
            r#"### Goal
{goal}

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC

### Funding mode
StripeFiatLedger
"#
        )
    }

    fn valid_github_issue_body_with_funding_mode(funding_mode: &str) -> String {
        format!(
            r#"### Goal
Fix the failing CI check.

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC

### Funding mode
{funding_mode}
"#
        )
    }
}
