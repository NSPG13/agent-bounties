use app::{
    build_live_money_readiness_report, stripe_secret_key_mode_from_secret,
    AddFundingContributionRequest, ApproveRiskBountyRequest, ApproveRiskPayoutRequest,
    BountyNetwork, BountyStatusResponse, ClaimBountyRequest, CreateFundingIntentRequest,
    CreateHelpRequestRequest, FundQuoteRequest, FundingIntentReport, LiveMoneyReadinessConfig,
    OpenPooledBountyRequest, PlanStripeTransferRequest, PooledFundingReport, PostBountyRequest,
    RegisterAgentRequest, RegisterCapabilityRequest, RejectRiskEventRequest, RequestQuotesRequest,
    ReviewedBountyApproval, RiskEventFilter, SubmitResultRequest, VerifySubmissionRequest,
};
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bounty_router::BountyRouter;
use chain_base::{
    autonomous_bounty_is_earning_ready, base_network_descriptor, broadcast_signed_transaction,
    build_autonomous_bounty_feed, build_autonomous_bounty_terms_record,
    build_autonomous_submission_evidence_record, build_autonomous_verification_jobs,
    decode_autonomous_bounty_logs, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, fetch_transaction_receipt, normalize_evm_address,
    plan_canonical_child_bounty_terms as build_canonical_child_bounty_terms_plan,
    validate_attestation_request_against_feed, validate_autonomous_creation_against_terms,
    AutonomousBountyAuthorizationSignature, AutonomousBountyContribution, AutonomousBountyCreate,
    AutonomousBountyFeedItem, AutonomousBountyRecoveryReservations,
    AutonomousBountySubmissionAuthorizationRequest, AutonomousBountyTxPlanner,
    AutonomousSignedAttestation, AutonomousVerificationAttestationRequest, BaseNetworkDescriptor,
    BaseRpcUrlConfig, CanonicalChildBountyTermsRequest, EvmLog,
};
use chrono::Utc;
use db::{BountyStatusScope, PostgresStore};
use domain::{
    Agent, AutonomousBountyTermsDocument, BountyStatus, CapabilityClass, EvalRun, HelpRequest,
    Money, PaymentRail, PayoutStatus, PrivacyLevel, RiskReviewRecord,
};
use eval_harness::{EvalSuiteResult, LoopSuiteResult};
use github_app::{
    bounty_check_output, claim_comment_plan, funding_comment_plan, parse_issue_form_bounty,
    proof_comment_plan, GitHubClaimCommentInput, GitHubFundingCommentInput, GitHubProofComment,
};
use ledger::Ledger;
use payments_stripe::{
    execute_stripe_request, CheckoutTopUpRequest, ConnectAccountSnapshot, StripeEventDeduper,
    StripePlanner, StripeRequestIntent, StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use risk::RiskPolicy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::env;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;
use uuid::Uuid;

#[derive(Debug)]
struct AppState {
    network: Mutex<BountyNetwork>,
    eval_runs: Mutex<Vec<EvalRun>>,
    base_rpc_urls: BaseRpcUrlConfig,
    base_broadcast_enabled: bool,
    stripe_secret_key: Option<String>,
    stripe_live_execution_enabled: bool,
    stripe_api_base_url: String,
    stripe_payment_method_configuration: Option<String>,
    operator_api_token: Option<String>,
    store: Option<PostgresStore>,
    recovery_reservations: AutonomousBountyRecoveryReservations,
}

type SharedState = Arc<AppState>;
const OPERATOR_TOKEN_HEADER: &str = "x-operator-token";

async fn health() -> impl IntoResponse {
    health_response(&deployment_revision())
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

fn non_empty_secret(secret: String) -> Option<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn require_operator(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<(), Json<serde_json::Value>> {
    let Some(expected) = state.operator_api_token.as_deref() else {
        return Ok(());
    };
    let Some(provided) = operator_token_from_headers(headers) else {
        return Err(mcp_error("operator authorization required"));
    };
    if constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(mcp_error("operator authorization required"))
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolDescriptor {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization: Option<ToolAuthorization>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolAuthorization {
    kind: &'static str,
    header: &'static str,
    bearer: bool,
    required_when: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RouteBlockedGoalArgs {
    goal: String,
    context: String,
    budget_minor: i64,
    currency: String,
    privacy: PrivacyLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanStripeCheckoutTopUpArgs {
    organization_id: Uuid,
    amount_minor: i64,
    currency: String,
    success_url: Option<String>,
    cancel_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanStripeConnectAccountArgs {
    agent_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanStripeConnectTransferArgs {
    payout_intent_id: Uuid,
    connected_account_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LiveMoneyReadinessArgs {
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubIssueBountyArgs {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubFundingCommentArgs {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubClaimCommentArgs {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubProofCommentArgs {
    bounty_id: Uuid,
    proof_url: String,
    verifier_summary: String,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubProofCommentForProofArgs {
    proof_id: Uuid,
    settlement_url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SearchCapabilitiesArgs {
    class: Option<CapabilityClass>,
    template_slug: Option<String>,
    currency: Option<String>,
    max_price_minor: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyCreationArgs {
    network: Option<String>,
    create: AutonomousBountyCreate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyContributionArgs {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedCreationArgs {
    network: Option<String>,
    create: AutonomousBountyCreate,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedContributionArgs {
    network: Option<String>,
    contribution: AutonomousBountyContribution,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct X402BountyFundingArgs {
    network: Option<String>,
    bounty_contract: String,
    amount: Option<u64>,
    relayer: Option<String>,
    payment_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GetX402RelayStatusArgs {
    relay_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyClaimArgs {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: Option<String>,
    authorization_valid_before: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountyAuthorizedClaimArgs {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    authorization_nonce: String,
    authorization_valid_before: u64,
    signature: AutonomousBountyAuthorizationSignature,
    relayer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionArgs {
    network: Option<String>,
    bounty_contract: String,
    solver: String,
    submission_hash: String,
    evidence_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousBountySubmissionAuthorizationArgs {
    network: Option<String>,
    submission: AutonomousBountySubmissionAuthorizationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousVerificationAttestationArgs {
    network: Option<String>,
    attestation: AutonomousVerificationAttestationRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousModuleSettlementArgs {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    proof: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousAttestationSettlementArgs {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
    attestations: Vec<AutonomousSignedAttestation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanAutonomousLifecycleArgs {
    network: Option<String>,
    bounty_contract: String,
    caller: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DecodeAutonomousBountyEventsArgs {
    logs: Vec<EvmLog>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ListAutonomousBountyEventsArgs {
    network: Option<String>,
    bounty_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousBountyTermsArgs {
    creator_wallet: String,
    document: AutonomousBountyTermsDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GetAutonomousBountyTermsArgs {
    terms_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PublishAutonomousSubmissionEvidenceArgs {
    network: Option<String>,
    bounty_contract: String,
    bounty_id: String,
    round: u64,
    solver_wallet: String,
    artifact_reference: String,
    evidence: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GetAutonomousSubmissionEvidenceArgs {
    network: Option<String>,
    bounty_contract: String,
    round: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AutonomousBountyFeedArgs {
    network: Option<String>,
    claimable_only: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AutonomousVerificationJobsArgs {
    network: Option<String>,
    verifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BroadcastBaseSignedTransactionArgs {
    signed_transaction: String,
    request_id: Option<u64>,
    network: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GetBaseTransactionReceiptArgs {
    tx_hash: String,
    request_id: Option<u64>,
    network: Option<String>,
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
    let recovery_reservations_raw = env::var("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS").ok();
    let recovery_reservations =
        AutonomousBountyRecoveryReservations::parse_csv(recovery_reservations_raw.as_deref())
            .map_err(|error| {
                anyhow::anyhow!("BASE_RECOVERY_RESERVED_BOUNTY_CONTRACTS is invalid: {error}")
            })?;
    let state: SharedState = Arc::new(AppState {
        network: Mutex::new(network),
        eval_runs: Mutex::new(eval_runs),
        base_rpc_urls: BaseRpcUrlConfig::from_env(),
        base_broadcast_enabled: env::var("ENABLE_BASE_TX_BROADCAST")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok(),
        stripe_live_execution_enabled: env::var("ENABLE_STRIPE_LIVE_EXECUTION")
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false),
        stripe_api_base_url: env::var("STRIPE_API_BASE_URL")
            .unwrap_or_else(|_| STRIPE_API_BASE_URL.to_string()),
        stripe_payment_method_configuration: env::var("STRIPE_PAYMENT_METHOD_CONFIGURATION")
            .ok()
            .and_then(non_empty_secret),
        operator_api_token: env::var("OPERATOR_API_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        store,
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
        .route("/tools", get(tools))
        .route("/tools/route_blocked_goal", post(route_blocked_goal))
        .route("/tools/register_agent", post(register_agent))
        .route("/tools/register_capability", post(register_capability))
        .route("/tools/search_capabilities", post(search_capabilities))
        .route("/tools/request_quotes", post(request_quotes))
        .route("/tools/fund_quote_as_bounty", post(fund_quote_as_bounty))
        .route("/tools/post_bounty", post(post_bounty))
        .route("/tools/open_pooled_bounty", post(open_pooled_bounty))
        .route("/tools/create_funding_intent", post(create_funding_intent))
        .route("/tools/add_bounty_funding", post(add_bounty_funding))
        .route(
            "/tools/list_claimable_bounties",
            get(list_claimable_bounties),
        )
        .route("/tools/claim_bounty", post(claim_bounty))
        .route("/tools/submit_result", post(submit_result))
        .route("/tools/request_verification", post(request_verification))
        .route("/tools/get_bounty_status", post(get_bounty_status))
        .route("/tools/get_paid_status", post(get_paid_status))
        .route(
            "/tools/plan_stripe_checkout_top_up",
            post(plan_stripe_checkout_top_up),
        )
        .route(
            "/tools/plan_stripe_connect_account",
            post(plan_stripe_connect_account),
        )
        .route(
            "/tools/plan_stripe_connect_transfer",
            post(plan_stripe_connect_transfer),
        )
        .route(
            "/tools/execute_stripe_checkout_top_up",
            post(execute_stripe_checkout_top_up),
        )
        .route(
            "/tools/execute_stripe_connect_account",
            post(execute_stripe_connect_account),
        )
        .route(
            "/tools/execute_stripe_connect_transfer",
            post(execute_stripe_connect_transfer),
        )
        .route(
            "/tools/reconcile_stripe_connect_snapshot",
            post(reconcile_stripe_connect_snapshot),
        )
        .route(
            "/tools/reconcile_stripe_transfer_event",
            post(reconcile_stripe_transfer_event),
        )
        .route(
            "/tools/reconcile_stripe_checkout_webhook",
            post(reconcile_stripe_checkout_webhook),
        )
        .route(
            "/tools/plan_github_issue_bounty",
            post(plan_github_issue_bounty),
        )
        .route(
            "/tools/plan_github_funding_comment",
            post(plan_github_funding_comment),
        )
        .route(
            "/tools/plan_github_claim_comment",
            post(plan_github_claim_comment),
        )
        .route(
            "/tools/plan_github_proof_comment",
            post(plan_github_proof_comment),
        )
        .route(
            "/tools/plan_github_proof_comment_for_proof",
            post(plan_github_proof_comment_for_proof),
        )
        .route(
            "/tools/broadcast_base_signed_transaction",
            post(broadcast_base_signed_transaction),
        )
        .route(
            "/tools/get_base_transaction_receipt",
            post(get_base_transaction_receipt),
        )
        .route(
            "/tools/plan_autonomous_canonical_child_terms",
            post(plan_autonomous_canonical_child_terms),
        )
        .route(
            "/tools/plan_autonomous_bounty_creation",
            post(plan_autonomous_bounty_creation),
        )
        .route(
            "/tools/plan_autonomous_bounty_authorized_creation",
            post(plan_autonomous_bounty_authorized_creation),
        )
        .route(
            "/tools/plan_autonomous_bounty_contribution",
            post(plan_autonomous_bounty_contribution),
        )
        .route(
            "/tools/plan_autonomous_bounty_authorized_contribution",
            post(plan_autonomous_bounty_authorized_contribution),
        )
        .route("/tools/fund_bounty_with_x402", post(fund_bounty_with_x402))
        .route("/tools/get_x402_relay_status", post(get_x402_relay_status))
        .route(
            "/tools/plan_autonomous_bounty_claim",
            post(plan_autonomous_bounty_claim),
        )
        .route(
            "/tools/plan_autonomous_bounty_authorized_claim",
            post(plan_autonomous_bounty_authorized_claim),
        )
        .route(
            "/tools/plan_autonomous_bounty_submission",
            post(plan_autonomous_bounty_submission),
        )
        .route(
            "/tools/plan_autonomous_bounty_submission_authorization",
            post(plan_autonomous_bounty_submission_authorization),
        )
        .route(
            "/tools/plan_autonomous_verification_attestation",
            post(plan_autonomous_verification_attestation),
        )
        .route(
            "/tools/plan_autonomous_module_settlement",
            post(plan_autonomous_module_settlement),
        )
        .route(
            "/tools/plan_autonomous_attestation_settlement",
            post(plan_autonomous_attestation_settlement),
        )
        .route(
            "/tools/plan_autonomous_expire_claim",
            post(plan_autonomous_expire_claim),
        )
        .route(
            "/tools/plan_autonomous_expire_submission",
            post(plan_autonomous_expire_submission),
        )
        .route(
            "/tools/plan_autonomous_cancel",
            post(plan_autonomous_cancel),
        )
        .route(
            "/tools/plan_autonomous_refund_withdrawal",
            post(plan_autonomous_refund_withdrawal),
        )
        .route(
            "/tools/decode_autonomous_bounty_events",
            post(decode_autonomous_bounty_events),
        )
        .route(
            "/tools/list_autonomous_bounty_events",
            post(list_autonomous_bounty_events),
        )
        .route(
            "/tools/publish_autonomous_bounty_terms",
            post(publish_autonomous_bounty_terms),
        )
        .route(
            "/tools/get_autonomous_bounty_terms",
            post(get_autonomous_bounty_terms),
        )
        .route(
            "/tools/publish_autonomous_submission_evidence",
            post(publish_autonomous_submission_evidence),
        )
        .route(
            "/tools/get_autonomous_submission_evidence",
            post(get_autonomous_submission_evidence),
        )
        .route(
            "/tools/list_autonomous_bounties",
            post(list_autonomous_bounties),
        )
        .route(
            "/tools/list_autonomous_verification_jobs",
            post(list_autonomous_verification_jobs),
        )
        .route("/tools/run_bountybench", get(run_bountybench))
        .route("/tools/run_abusebench", get(run_abusebench))
        .route("/tools/run_judgebench", get(run_judgebench))
        .route("/tools/run_eval_loops", get(run_eval_loops))
        .route("/tools/get_eval_runs", get(get_eval_runs))
        .route("/tools/get_risk_policy", get(get_risk_policy))
        .route(
            "/tools/get_live_money_readiness",
            post(get_live_money_readiness),
        )
        .route("/tools/list_risk_events", post(list_risk_events))
        .route("/tools/list_risk_reviews", get(list_risk_reviews))
        .route("/tools/approve_risk_bounty", post(approve_risk_bounty))
        .route("/tools/approve_risk_payout", post(approve_risk_payout))
        .route("/tools/reject_risk_event", post(reject_risk_event))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = service_bind_addr(
        env::var("MCP_BIND_ADDR").ok().as_deref(),
        env::var("PORT").ok().as_deref(),
        "127.0.0.1:8090",
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

async fn agent_bounties_discovery() -> Json<web_public::DiscoveryManifest> {
    let api_base_url = public_base_url_from_env();
    let mcp_base_url = mcp_base_url_from_env();
    Json(web_public::discovery_manifest(&api_base_url, &mcp_base_url))
}

async fn llms_txt() -> String {
    let api_base_url = public_base_url_from_env();
    let mcp_base_url = mcp_base_url_from_env();
    web_public::render_llms_txt(&api_base_url, &mcp_base_url)
}

fn public_base_url_from_env() -> String {
    env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string())
}

fn mcp_base_url_from_env() -> String {
    env::var("MCP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string())
}

async fn discovery_manifest_schema() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/schema+json")],
        web_public::discovery_manifest_schema_json(),
    )
}

async fn tools() -> Json<Vec<ToolDescriptor>> {
    Json(vec![
        tool(
            "route_blocked_goal",
            "Route a blocked agent goal into a template, quote, bounty, or verification step.",
            object_tool_schema(
                json!({
                    "goal": string_property("Task, workflow, job, or goal where the requester is blocked."),
                    "context": string_property("Relevant constraints, logs, artifacts, URLs, or prior attempts."),
                    "budget_minor": integer_property("Maximum budget in minor units."),
                    "currency": string_property("Lowercase currency code, for example usdc or usd."),
                    "privacy": privacy_property()
                }),
                &["goal", "context", "budget_minor", "currency", "privacy"],
            ),
        ),
        tool(
            "register_agent",
            "Register an agent and optional payout wallet.",
            object_tool_schema(
                json!({
                    "handle": string_property("Public or local agent handle."),
                    "payout_wallet": nullable_string_property("Optional EVM payout wallet for Base USDC settlements.")
                }),
                &["handle"],
            ),
        ),
        tool(
            "request_quotes",
            "Create a help request and request solver quotes.",
            object_tool_schema(
                json!({
                    "requester_agent_id": uuid_property("Requester agent UUID."),
                    "goal": string_property("Work goal."),
                    "context": string_property("Work context and constraints."),
                    "budget_minor": integer_property("Budget in minor units."),
                    "currency": string_property("Currency code."),
                    "privacy": privacy_property(),
                    "required_confidence": nullable_number_property("Optional verifier confidence threshold.")
                }),
                &[
                    "requester_agent_id",
                    "goal",
                    "context",
                    "budget_minor",
                    "currency",
                    "privacy",
                ],
            ),
        ),
        tool(
            "post_bounty",
            "Post a funded bounty.",
            object_tool_schema(
                json!({
                    "title": string_property("Bounty title."),
                    "template_slug": string_property("Reusable bounty template slug."),
                    "amount_minor": integer_property("Funded amount in minor units."),
                    "currency": string_property("Currency code."),
                    "funding_mode": funding_mode_property(),
                    "privacy": privacy_property()
                }),
                &[
                    "title",
                    "template_slug",
                    "amount_minor",
                    "currency",
                    "funding_mode",
                    "privacy",
                ],
            ),
        ),
        tool(
            "open_pooled_bounty",
            "Open an unfunded pooled bounty target so multiple contributors can add funds before it becomes claimable.",
            object_tool_schema(
                json!({
                    "title": string_property("Bounty title."),
                    "template_slug": string_property("Reusable bounty template slug."),
                    "target_amount_minor": integer_property("Target amount in minor units before the bounty becomes claimable."),
                    "currency": string_property("Currency code."),
                    "funding_mode": funding_mode_property(),
                    "privacy": privacy_property()
                }),
                &[
                    "title",
                    "template_slug",
                    "target_amount_minor",
                    "currency",
                    "funding_mode",
                    "privacy",
                ],
            ),
        ),
        tool(
            "create_funding_intent",
            "Create a Stripe funding intent for a bounty. Base USDC funding uses the autonomous-v1 contract flow. The intent does not confirm funding until a verified webhook is reconciled.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Bounty UUID."),
                    "contributor_agent_id": nullable_uuid_property("Optional contributor agent UUID."),
                    "source_organization_id": nullable_uuid_property("Stripe-funded organization UUID. Required for StripeFiat intents."),
                    "amount_minor": integer_property("Intent amount in minor units."),
                    "currency": string_property("Currency code for the rail partition."),
                    "rail": enum_property(&["StripeFiat"], "Stripe funding rail for this intent."),
                    "external_reference": nullable_string_property("Optional per-bounty idempotency reference for duplicate detection."),
                    "stripe_success_url": nullable_string_property("Optional Stripe Checkout success URL."),
                    "stripe_cancel_url": nullable_string_property("Optional Stripe Checkout cancel URL."),
                }),
                &["bounty_id", "amount_minor", "currency", "rail"],
            ),
        ),
        tool(
            "add_bounty_funding",
            "Add an applied funding contribution to a pooled bounty and return the updated funding summary.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Pooled bounty UUID."),
                    "contributor_agent_id": nullable_uuid_property("Optional contributor agent UUID."),
                    "source_organization_id": nullable_uuid_property("Stripe-funded organization balance to reserve from when rail is StripeFiat."),
                    "amount_minor": integer_property("Contribution amount in minor units."),
                    "currency": string_property("Currency code."),
                    "rail": enum_property(&["Simulated", "StripeFiat"], "Legacy off-chain contribution rail. Base USDC uses the autonomous bounty contribution tools and canonical FundingAdded events."),
                    "external_reference": nullable_string_property("Optional per-bounty idempotency reference from the funding rail.")
                }),
                &["bounty_id", "amount_minor", "currency", "rail"],
            ),
        ),
        tool(
            "list_claimable_bounties",
            "List funded public bounty work that agents can claim immediately.",
            empty_tool_schema(),
        ),
        tool(
            "search_capabilities",
            "Search public solver capabilities before requesting quotes.",
            object_tool_schema(
                json!({
                    "class": nullable_enum_property(&[
                        "Coding",
                        "Research",
                        "Extraction",
                        "Verification",
                        "Documentation",
                        "Ci",
                        "BrowserWorkflow"
                    ], "Optional capability class filter."),
                    "template_slug": nullable_string_property("Optional reusable bounty template slug."),
                    "currency": nullable_string_property("Optional lowercase currency code."),
                    "max_price_minor": nullable_integer_property("Optional maximum acceptable minimum price in minor units.")
                }),
                &[],
            ),
        ),
        tool(
            "claim_bounty",
            "Claim claimable paid work.",
            bounty_solver_schema(),
        ),
        tool(
            "submit_result",
            "Submit an artifact and proof digest.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Bounty UUID."),
                    "solver_agent_id": uuid_property("Claiming solver agent UUID."),
                    "artifact_uri": string_property("Artifact URI or location hint."),
                    "artifact_body": string_property("Artifact body used for deterministic hashing in local flows.")
                }),
                &[
                    "bounty_id",
                    "solver_agent_id",
                    "artifact_uri",
                    "artifact_body",
                ],
            ),
        ),
        tool(
            "request_verification",
            "Ask a verifier to check a submission.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Bounty UUID."),
                    "submission_id": uuid_property("Submission UUID."),
                    "expected_artifact_digest": string_property("Expected SHA-256 digest for deterministic verifiers."),
                    "verifier_kind": nullable_enum_property(&[
                        "Manual",
                        "JsonSchema",
                        "DockerCommand",
                        "GitHubCi",
                        "HttpCallback",
                        "AiJudgeFilter"
                    ], "Optional verifier kind."),
                    "rubric": nullable_string_property("Optional human or AI-judge rubric."),
                    "evidence": nullable_object_property("Optional verifier evidence payload. For GitHubCi, include repository, pull_request_url, commit_sha, and check_run { id, name, status, conclusion, head_sha, html_url, repository.full_name }."),
                    "approved_risk_event_id": nullable_uuid_property("Optional approved payout risk event UUID that permits verification to continue after operator review.")
                }),
                &["bounty_id", "submission_id", "expected_artifact_digest"],
            ),
        ),
        tool(
            "get_bounty_status",
            "Read bounty lifecycle and verification status.",
            bounty_id_schema(),
        ),
        tool(
            "get_paid_status",
            "Read payout status for a bounty or agent. After verified value, the response includes an ordered post_value_loop for sharing evidence, telling the human/operator, asking for a star/upvote, posting a useful bounty, and returning to funded work.",
            object_tool_schema(
                json!({
                    "bounty_id": nullable_uuid_property("Optional bounty UUID for bounty-level payout status."),
                    "agent_id": nullable_uuid_property("Optional agent UUID for agent-level earnings and payout status.")
                }),
                &[],
            ),
        ),
        tool(
            "register_capability",
            "Publish agent capability and price bands.",
            object_tool_schema(
                json!({
                    "agent_id": uuid_property("Agent UUID."),
                    "class": enum_property(&[
                        "Coding",
                        "Research",
                        "Extraction",
                        "Verification",
                        "Documentation",
                        "Ci",
                        "BrowserWorkflow"
                    ], "Capability class."),
                    "template_slugs": string_array_property("Template slugs the agent can handle."),
                    "min_price_minor": integer_property("Minimum price in minor units."),
                    "max_price_minor": integer_property("Maximum price in minor units."),
                    "currency": string_property("Currency code."),
                    "latency_seconds": integer_property("Expected completion latency in seconds."),
                    "supported_verifiers": array_property(verifier_kind_property(), "Verifier kinds the agent supports.")
                }),
                &[
                    "agent_id",
                    "class",
                    "template_slugs",
                    "min_price_minor",
                    "max_price_minor",
                    "currency",
                    "latency_seconds",
                    "supported_verifiers",
                ],
            ),
        ),
        tool(
            "fund_quote_as_bounty",
            "Convert an accepted quote into a funded claimable bounty.",
            object_tool_schema(
                json!({
                    "quote_id": uuid_property("Quote UUID."),
                    "title": nullable_string_property("Optional bounty title override."),
                    "funding_mode": nullable_enum_property(&["Simulated", "StripeFiatLedger"], "Optional off-chain funding mode override. Base USDC uses autonomous-v1 tools.")
                }),
                &["quote_id"],
            ),
        ),
        tool(
            "plan_stripe_checkout_top_up",
            "Build a Stripe Checkout Session request intent for funding a fiat platform balance.",
            object_tool_schema(
                json!({
                    "organization_id": uuid_property("Organization UUID credited after a paid webhook."),
                    "amount_minor": integer_property("Top-up amount in minor units."),
                    "currency": string_property("Stripe currency code."),
                    "success_url": nullable_string_property("Optional Checkout success URL."),
                    "cancel_url": nullable_string_property("Optional Checkout cancel URL.")
                }),
                &["organization_id", "amount_minor", "currency"],
            ),
        ),
        tool(
            "plan_stripe_connect_account",
            "Build a Stripe Accounts v2 request intent for agent fiat payout onboarding.",
            object_tool_schema(
                json!({ "agent_id": uuid_property("Agent UUID.") }),
                &["agent_id"],
            ),
        ),
        tool(
            "plan_stripe_connect_transfer",
            "Build a Stripe Connect transfer request intent for a specific fiat payout intent. The transfer must still be executed and reconciled from Stripe evidence before payout is marked paid.",
            object_tool_schema(
                json!({
                    "payout_intent_id": uuid_property("Stripe fiat payout intent UUID from bounty status."),
                    "connected_account_id": string_property("Stripe connected account ID receiving the transfer.")
                }),
                &["payout_intent_id", "connected_account_id"],
            ),
        ),
        tool(
            "get_live_money_readiness",
            "Return non-secret Stripe/Base readiness gates for this hosted service before agents or operators rely on real-value movement.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-mainnet", "base-sepolia"], "Base network to inspect. Defaults to base-mainnet.")
                }),
                &[],
            ),
        ),
        operator_tool(
            "execute_stripe_checkout_top_up",
            "Create a live Stripe Checkout Session for funding a fiat platform balance when operator-enabled.",
            object_tool_schema(
                json!({
                    "organization_id": uuid_property("Organization UUID credited after a paid webhook."),
                    "amount_minor": integer_property("Top-up amount in minor units."),
                    "currency": string_property("Stripe currency code."),
                    "success_url": nullable_string_property("Optional Checkout success URL."),
                    "cancel_url": nullable_string_property("Optional Checkout cancel URL.")
                }),
                &["organization_id", "amount_minor", "currency"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "execute_stripe_connect_account",
            "Create a live Stripe Accounts v2 connected account when operator-enabled.",
            object_tool_schema(
                json!({ "agent_id": uuid_property("Agent UUID.") }),
                &["agent_id"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "execute_stripe_connect_transfer",
            "Execute a Stripe Connect transfer for a fiat payout intent when operator-enabled. The payout is still marked paid only after transfer event reconciliation.",
            object_tool_schema(
                json!({
                    "payout_intent_id": uuid_property("Stripe fiat payout intent UUID from bounty status."),
                    "connected_account_id": string_property("Stripe connected account ID receiving the transfer.")
                }),
                &["payout_intent_id", "connected_account_id"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reconcile_stripe_connect_snapshot",
            "Apply Stripe Connect payout eligibility to blocked fiat payout intents.",
            object_tool_schema(
                json!({
                    "agent_id": uuid_property("Agent UUID."),
                    "connected_account_id": nullable_string_property("Stripe connected account ID."),
                    "payouts_enabled": boolean_property("Whether Stripe reports payouts enabled."),
                    "disabled_reason": nullable_string_property("Stripe disabled reason, if any."),
                    "currently_due": string_array_property("Currently due onboarding requirements.")
                }),
                &[
                    "agent_id",
                    "connected_account_id",
                    "payouts_enabled",
                    "disabled_reason",
                    "currently_due",
                ],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reconcile_stripe_transfer_event",
            "Apply a Stripe transfer.created event as fiat payout evidence for a payout intent.",
            object_tool_schema(
                json!({
                    "id": string_property("Stripe transfer event ID."),
                    "type": string_property("Stripe event type, usually transfer.created."),
                    "payload": object_property("Normalized Stripe transfer payload with payout metadata.")
                }),
                &["id", "type", "payload"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reconcile_stripe_checkout_webhook",
            "Apply a paid Stripe Checkout top-up webhook to the platform ledger.",
            object_tool_schema(
                json!({
                    "id": string_property("Stripe event ID."),
                    "type": string_property("Stripe event type, usually checkout.session.completed."),
                    "payload": object_property("Normalized Stripe event payload.")
                }),
                &["id", "type", "payload"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        tool(
            "plan_github_issue_bounty",
            "Parse a GitHub paid-bounty issue form and produce check-run output for dogfooding.",
            object_tool_schema(
                json!({
                    "repository": string_property("GitHub repository, for example owner/repo."),
                    "issue_url": string_property("Canonical GitHub issue URL."),
                    "title": string_property("Issue title."),
                    "body": string_property("Rendered issue form markdown body.")
                }),
                &["repository", "issue_url", "title", "body"],
            ),
        ),
        tool(
            "plan_github_funding_comment",
            "Parse a GitHub public co-funding comment into an operator reconciliation signal without crediting funds.",
            object_tool_schema(
                json!({
                    "repository": string_property("GitHub repository, for example owner/repo."),
                    "issue_url": string_property("Canonical GitHub issue URL for the paid bounty issue."),
                    "title": string_property("Issue title."),
                    "body": string_property("Rendered issue form markdown body."),
                    "comment_body": string_property("GitHub issue comment body, for example `/agent-bounty fund 5 USDC via BaseUsdcEscrow`."),
                    "contributor_login": nullable_string_property("Optional GitHub login that authored the funding signal."),
                    "comment_id": nullable_string_property("Optional GitHub comment ID used to build an idempotency key."),
                    "funding_api_base_url": nullable_string_property("Optional hosted API base URL to prefill the public StripeFiat funding page for Stripe funding comments."),
                    "existing_idempotency_keys": string_array_property("Previously processed funding-comment idempotency keys for duplicate detection.")
                }),
                &["repository", "issue_url", "title", "body", "comment_body"],
            ),
        ),
        tool(
            "plan_github_claim_comment",
            "Parse a GitHub public claim or attempt comment into a reservation, stale-release, or review signal without authorizing settlement.",
            object_tool_schema(
                json!({
                    "repository": string_property("GitHub repository, for example owner/repo."),
                    "issue_url": string_property("Canonical GitHub issue URL for the paid bounty issue."),
                    "title": string_property("Issue title."),
                    "body": string_property("Rendered issue form markdown body."),
                    "comment_body": string_property("GitHub issue comment body, for example `/agent-bounty claim` followed by `plan: ...`."),
                    "contributor_login": nullable_string_property("Optional GitHub login that authored the claim signal."),
                    "comment_id": nullable_string_property("Optional GitHub comment ID used to build a reservation id."),
                    "claim_age_minutes": nullable_integer_property("Optional age of the active claim reservation in minutes."),
                    "progress_signal_count": integer_property("Known count of external progress signals, such as PRs or progress comments."),
                    "active_claim_login": nullable_string_property("Optional login that currently holds the active claim reservation.")
                }),
                &["repository", "issue_url", "title", "body", "comment_body"],
            ),
        ),
        tool(
            "plan_github_proof_comment",
            "Build a GitHub proof comment and check-run output after a bounty is accepted.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Bounty UUID."),
                    "proof_url": string_property("Public proof URL."),
                    "verifier_summary": string_property("Verifier summary to include in the comment."),
                    "settlement_url": nullable_string_property("Optional settlement transaction or record URL.")
                }),
                &["bounty_id", "proof_url", "verifier_summary", "settlement_url"],
            ),
        ),
        tool(
            "plan_github_proof_comment_for_proof",
            "Build a GitHub proof comment and check-run output from a stored public proof record.",
            object_tool_schema(
                json!({
                    "proof_id": uuid_property("Public proof record UUID."),
                    "settlement_url": nullable_string_property("Optional settlement transaction or record URL.")
                }),
                &["proof_id", "settlement_url"],
            ),
        ),
        operator_tool(
            "broadcast_base_signed_transaction",
            "Broadcast a signed Base transaction through the configured RPC URL when operator-enabled.",
            object_tool_schema(
                json!({
                    "signed_transaction": string_property("0x-prefixed signed raw EVM transaction."),
                    "request_id": nullable_integer_property("Optional JSON-RPC request id."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["signed_transaction"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        tool(
            "get_base_transaction_receipt",
            "Fetch a Base transaction receipt. Canonical bounty state still comes from the autonomous indexer, not from this receipt alone.",
            object_tool_schema(
                json!({
                    "tx_hash": string_property("0x-prefixed transaction hash."),
                    "request_id": nullable_integer_property("Optional JSON-RPC request id."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet.")
                }),
                &["tx_hash"],
            ),
        ),
        tool(
            "plan_autonomous_canonical_child_terms",
            "Derive the exact task criteria, parent-and-round benchmark commitment, minimum USDC target, deterministic verifier configuration, and proof encoding for a canonical child bounty. The parent cannot pass until a different wallet completes the child and receives canonical settlement.",
            object_tool_schema(
                json!({
                    "parent_bounty_id": string_property("Parent canonical bytes32 bounty ID."),
                    "parent_round": integer_property("Current positive parent claim round."),
                    "parent_solver": string_property("Active parent solver; this wallet must create the child."),
                    "parent_solver_reward": money_property("Parent solver reward; the child target must preserve at least this much USDC.", false),
                    "child_acceptance_criteria": {
                        "type": "array",
                        "description": "One to twenty explicit, deterministic acceptance criteria for the child task.",
                        "items": { "type": "string" },
                        "minItems": 1,
                        "maxItems": 20
                    },
                    "verifier_module": string_property("Deployed canonical-child-v1 verifier module committed by the parent bounty.")
                }),
                &[
                    "parent_bounty_id", "parent_round", "parent_solver", "parent_solver_reward",
                    "child_acceptance_criteria", "verifier_module"
                ],
            ),
        ),
        tool(
            "plan_autonomous_bounty_creation",
            "Build a canonical Base USDC autonomous bounty creation plan. The plan supports a wallet-batched approve/create path and returns the predictable bounty address plus Circle USDC EIP-3009 authorization data for a gas-sponsored relayer path.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "create": autonomous_bounty_create_property()
                }),
                &["create"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_authorized_creation",
            "After the creator signs the exact EIP-3009 typed data returned by plan_autonomous_bounty_creation, build the single gas-sponsorable factory transaction that creates and funds the predictable bounty contract.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "create": autonomous_bounty_create_property(),
                    "signature": {
                        "type": "object",
                        "properties": {
                            "v": integer_property("EIP-3009 recovery id: 0, 1, 27, or 28."),
                            "r": string_property("0x-prefixed bytes32 signature r."),
                            "s": string_property("0x-prefixed bytes32 signature s.")
                        },
                        "required": ["v", "r", "s"],
                        "additionalProperties": false
                    },
                    "relayer": nullable_string_property("Optional wallet that will sponsor and submit the factory transaction.")
                }),
                &["create", "signature"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_contribution",
            "Build wallet-batchable approve/fund calls for a permissionless pooled USDC contribution to an existing canonical bounty contract.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "contribution": {
                        "type": "object",
                        "properties": {
                            "bounty_contract": string_property("Canonical bounty contract receiving USDC."),
                            "contributor": string_property("Funding agent or human wallet."),
                            "amount": money_property("Exact USDC contribution; it must not exceed remaining target funding.", false),
                            "authorization_nonce": nullable_string_property("Optional unique bytes32 for the one-signature EIP-3009 path."),
                            "authorization_valid_before": nullable_integer_property("Optional Unix expiry paired with authorization_nonce.")
                        },
                        "required": ["bounty_contract", "contributor", "amount"],
                        "additionalProperties": false
                    }
                }),
                &["contribution"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_authorized_contribution",
            "After a funder signs the EIP-3009 typed data returned by plan_autonomous_bounty_contribution, build the single gas-sponsorable transaction that transfers USDC into the bounty and records the contribution.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "contribution": {
                        "type": "object",
                        "properties": {
                            "bounty_contract": string_property("Indexed canonical bounty contract."),
                            "contributor": string_property("Wallet that signed the authorization."),
                            "amount": money_property("Exact USDC amount authorized.", false),
                            "authorization_nonce": string_property("Unique bytes32 signed in the authorization."),
                            "authorization_valid_before": integer_property("Unix expiry signed in the authorization.")
                        },
                        "required": ["bounty_contract", "contributor", "amount", "authorization_nonce", "authorization_valid_before"],
                        "additionalProperties": false
                    },
                    "signature": {
                        "type": "object",
                        "properties": {
                            "v": integer_property("EIP-3009 recovery id: 0, 1, 27, or 28."),
                            "r": string_property("0x-prefixed bytes32 signature r."),
                            "s": string_property("0x-prefixed bytes32 signature s.")
                        },
                        "required": ["v", "r", "s"],
                        "additionalProperties": false
                    },
                    "relayer": nullable_string_property("Optional wallet sponsoring the transaction.")
                }),
                &["contribution", "signature"],
            ),
        ),
        tool(
            "fund_bounty_with_x402",
            "Fund a canonical bounty through x402 v2. Omit payment_signature for the exact EIP-3009 challenge; retry with the signature and the hosted gas-only relayer broadcasts. Success requires 200 plus PAYMENT-RESPONSE backed by confirmed canonical FundingAdded.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Indexed canonical bounty contract."),
                    "amount": nullable_integer_property("Optional USDC base-unit contribution; defaults to the remaining target."),
                    "relayer": nullable_string_property("Optional gas-paying Base wallet for self-relay fallback. Omit it when the hosted relay is enabled."),
                    "payment_signature": nullable_string_property("Optional base64 x402 v2 PaymentPayload copied from the PAYMENT-SIGNATURE header. Omit it to receive the exact challenge.")
                }),
                &["bounty_contract"],
            ),
        ),
        tool(
            "get_x402_relay_status",
            "Poll a durable hosted x402 relay. A transaction hash is pending evidence; only status=confirmed with PAYMENT-RESPONSE proves canonical bounty funding.",
            object_tool_schema(
                json!({
                    "relay_id": string_property("Relay UUID returned by fund_bounty_with_x402.")
                }),
                &["relay_id"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_claim",
            "Build a wallet-batched USDC bond approval and claim. The indexed bond equals one verifier reward: acceptance or verifier timeout returns it, rejection replaces the paid verifier reserve, and a no-submission timeout adds it to the completion bonus.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Fully funded canonical bounty contract."),
                    "solver": string_property("Solver wallet that will receive payout if verification passes."),
                    "authorization_nonce": nullable_string_property("Optional unique bytes32 for a one-signature EIP-3009 bond authorization."),
                    "authorization_valid_before": nullable_integer_property("Optional Unix expiry for the bond authorization.")
                }),
                &["bounty_contract", "solver"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_authorized_claim",
            "After the solver signs the exact EIP-3009 bond returned by plan_autonomous_bounty_claim, build one gas-sponsorable transaction that deposits the bond and activates the claim.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Fully funded canonical bounty contract."),
                    "solver": string_property("Wallet that signed the claim-bond authorization."),
                    "authorization_nonce": string_property("Unique bytes32 signed in the authorization."),
                    "authorization_valid_before": integer_property("Unix expiry signed in the authorization."),
                    "signature": {
                        "type": "object",
                        "properties": {
                            "v": integer_property("EIP-3009 recovery id: 0, 1, 27, or 28."),
                            "r": string_property("0x-prefixed bytes32 signature r."),
                            "s": string_property("0x-prefixed bytes32 signature s.")
                        },
                        "required": ["v", "r", "s"],
                        "additionalProperties": false
                    },
                    "relayer": nullable_string_property("Optional wallet sponsoring the transaction.")
                }),
                &["bounty_contract", "solver", "authorization_nonce", "authorization_valid_before", "signature"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_submission",
            "Build the solver's submission commitment call. The hashes must identify the artifact and evidence evaluated by the immutable verifier policy.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Claimed canonical bounty contract."),
                    "solver": string_property("Wallet holding the current claim."),
                    "submission_hash": string_property("0x-prefixed bytes32 artifact commitment."),
                    "evidence_hash": string_property("0x-prefixed bytes32 evidence-package commitment.")
                }),
                &["bounty_contract", "solver", "submission_hash", "evidence_hash"],
            ),
        ),
        tool(
            "plan_autonomous_bounty_submission_authorization",
            "Build the exact EIP-712 submission authorization an active solver signs for a gas-sponsored submitWithSignature relay.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "submission": {
                        "type": "object",
                        "properties": {
                            "bounty_contract": string_property("Claimed canonical bounty contract."),
                            "bounty_id": string_property("Current canonical bytes32 bounty id."),
                            "round": integer_property("Current positive claim round."),
                            "solver": string_property("Wallet holding the current claim."),
                            "submission_hash": string_property("Nonzero bytes32 artifact commitment."),
                            "evidence_hash": string_property("Nonzero bytes32 evidence commitment."),
                            "policy_hash": string_property("Immutable bytes32 verification-policy commitment."),
                            "deadline": integer_property("Unix expiry no later than the active claim deadline.")
                        },
                        "required": ["bounty_contract", "bounty_id", "round", "solver", "submission_hash", "evidence_hash", "policy_hash", "deadline"],
                        "additionalProperties": false
                    }
                }),
                &["submission"],
            ),
        ),
        tool(
            "plan_autonomous_verification_attestation",
            "Build the exact EIP-712 payload a committed verifier signs for the current indexed submission. The planner rejects stale rounds, changed hashes, unauthorized verifiers, and deadlines beyond verification expiry.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "attestation": autonomous_verification_attestation_property()
                }),
                &["attestation"],
            ),
        ),
        tool(
            "plan_autonomous_module_settlement",
            "Build the permissionless deterministic verifier transaction. A passing verifier call transfers solver and verifier rewards atomically; a plan or transaction hash is not payout evidence.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Submitted canonical deterministic-module bounty."),
                    "caller": nullable_string_property("Optional wallet sponsoring the permissionless call."),
                    "proof": string_property("0x-prefixed proof bytes consumed by the committed verifier module.")
                }),
                &["bounty_contract", "proof"],
            ),
        ),
        tool(
            "plan_autonomous_attestation_settlement",
            "Build the permissionless quorum relay. The canonical contract validates each committed verifier signature and atomically pays on pass or reopens on reject.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Submitted canonical quorum bounty."),
                    "caller": nullable_string_property("Optional wallet sponsoring the relay."),
                    "attestations": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 8,
                        "items": autonomous_signed_attestation_property()
                    }
                }),
                &["bounty_contract", "attestations"],
            ),
        ),
        tool(
            "plan_autonomous_expire_claim",
            "Build the permissionless transaction that reopens an expired claim.",
            autonomous_lifecycle_schema("Claimed canonical bounty contract."),
        ),
        tool(
            "plan_autonomous_expire_submission",
            "Build the permissionless transaction that reopens an expired submission.",
            autonomous_lifecycle_schema("Submitted canonical bounty contract."),
        ),
        tool(
            "plan_autonomous_cancel",
            "Build cancellation for the creator or any caller after the immutable funding deadline. Contributors then withdraw their own refunds.",
            autonomous_lifecycle_schema("Open or claimable canonical bounty contract."),
        ),
        tool(
            "plan_autonomous_refund_withdrawal",
            "Build a contributor's pull-refund transaction after cancellation.",
            autonomous_lifecycle_schema("Cancelled canonical bounty contract."),
        ),
        tool(
            "decode_autonomous_bounty_events",
            "Decode raw EVM logs into evidence-bound autonomous bounty events. Unknown token-transfer logs are ignored; malformed recognized protocol logs fail closed.",
            object_tool_schema(
                json!({
                    "logs": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "address": string_property("Event-emitting factory or bounty contract."),
                                "topics": string_array_property("0x-prefixed 32-byte EVM log topics."),
                                "data": string_property("0x-prefixed ABI-encoded event data."),
                                "tx_hash": string_property("Confirmed transaction hash."),
                                "block_number": integer_property("Confirmed block number."),
                                "log_index": integer_property("Transaction log index."),
                                "occurred_at": nullable_string_property("Optional RFC3339 event timestamp.")
                            },
                            "required": ["address", "topics", "data", "tx_hash", "block_number", "log_index"],
                            "additionalProperties": false
                        }
                    }
                }),
                &["logs"],
            ),
        ),
        tool(
            "list_autonomous_bounty_events",
            "Read persisted confirmed canonical factory and bounty events. Use BountySettled as payout evidence; a signature, plan, or transaction hash alone is not settlement.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_id": nullable_string_property("Optional 0x-prefixed bytes32 autonomous bounty id filter.")
                }),
                &[],
            ),
        ),
        tool(
            "publish_autonomous_bounty_terms",
            "Publish a bounded public task document and receive deterministic Keccak commitments for the factory call. Publication is not funding or canonical listing; only a matching canonical factory event creates the bounty.",
            object_tool_schema(
                json!({
                    "creator_wallet": string_property("Wallet expected to create the canonical bounty."),
                    "document": autonomous_bounty_terms_property()
                }),
                &["creator_wallet", "document"],
            ),
        ),
        tool(
            "get_autonomous_bounty_terms",
            "Resolve and independently hash-check the exact public task specification committed by an on-chain termsHash.",
            object_tool_schema(
                json!({
                    "terms_hash": string_property("0x-prefixed Keccak hash from a canonical bounty contract.")
                }),
                &["terms_hash"],
            ),
        ),
        tool(
            "publish_autonomous_submission_evidence",
            "After SubmissionAdded is indexed, publish the exact public artifact reference and evidence object whose SHA-256 commitments match the current canonical submission. Conflicting replays fail closed.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Indexed canonical submitted bounty contract."),
                    "bounty_id": string_property("0x-prefixed canonical bounty id."),
                    "round": integer_property("Current submission round."),
                    "solver_wallet": string_property("Wallet that holds the indexed claim."),
                    "artifact_reference": string_property("Public repository, commit, artifact URI, or canonical result string."),
                    "evidence": object_property("Public evidence object evaluated under the immutable evidence schema.")
                }),
                &["bounty_contract", "bounty_id", "round", "solver_wallet", "artifact_reference", "evidence"],
            ),
        ),
        tool(
            "get_autonomous_submission_evidence",
            "Retrieve hash-checked public evidence for a canonical bounty round so deterministic or AI verifier agents can evaluate it.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "bounty_contract": string_property("Indexed canonical bounty contract."),
                    "round": integer_property("Positive submission round.")
                }),
                &["bounty_contract", "round"],
            ),
        ),
        tool(
            "list_autonomous_bounties",
            "Discover canonical autonomous bounties joined to their hash-verified public terms and complete event history. Set claimable_only=true to find work an agent can claim and earn from now.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "claimable_only": nullable_boolean_property("When true, return only fully funded unclaimed bounties.")
                }),
                &[],
            ),
        ),
        tool(
            "list_autonomous_verification_jobs",
            "List live canonical submissions whose immutable terms and hash-matched evidence preimages are ready for deterministic, signed, or AI-judge verification. Optionally filter quorum jobs to one verifier wallet.",
            object_tool_schema(
                json!({
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
                    "verifier": nullable_string_property("Optional committed verifier wallet; deterministic module jobs remain visible to any relayer.")
                }),
                &[],
            ),
        ),
        tool(
            "run_bountybench",
            "Run deterministic routing eval fixtures.",
            empty_tool_schema(),
        ),
        tool(
            "run_abusebench",
            "Run deterministic abuse and payout-safety eval fixtures.",
            empty_tool_schema(),
        ),
        tool(
            "run_judgebench",
            "Run deterministic product-quality AI-judge filter fixtures.",
            empty_tool_schema(),
        ),
        tool(
            "run_eval_loops",
            "Run loop-based router, template, verifier, proof, and abuse eval harnesses.",
            empty_tool_schema(),
        ),
        tool(
            "get_eval_runs",
            "Return compact eval-run history recorded by this MCP server.",
            empty_tool_schema(),
        ),
        tool(
            "get_risk_policy",
            "Return deterministic risk and settlement policy limits before posting, claiming, or releasing paid work.",
            empty_tool_schema(),
        ),
        tool(
            "list_risk_events",
            "List deterministic risk events that need operator review or explain blocked automatic flows.",
            object_tool_schema(
                json!({
                    "action": nullable_enum_property(&["Allow", "NeedsReview", "Block"], "Optional risk action filter."),
                    "surface": nullable_enum_property(&["HelpRequest", "Bounty", "Submission", "Verification", "Payout"], "Optional risk surface filter."),
                    "bounty_id": nullable_uuid_property("Optional bounty UUID filter."),
                    "agent_id": nullable_uuid_property("Optional agent UUID filter."),
                    "limit": nullable_integer_property("Optional maximum number of newest events to return, capped at 500.")
                }),
                &[],
            ),
        ),
        tool(
            "list_risk_reviews",
            "List operator review decisions recorded against deterministic risk events.",
            empty_tool_schema(),
        ),
        operator_tool(
            "approve_risk_bounty",
            "Approve a NeedsReview bounty risk event into a funded claimable bounty after operator review.",
            object_tool_schema(
                json!({
                    "risk_event_id": uuid_property("Risk event UUID being approved."),
                    "title": string_property("Bounty title to bind to the reviewed risk subject."),
                    "template_slug": string_property("Bounty template slug."),
                    "amount_minor": integer_property("Bounty amount in minor units."),
                    "currency": string_property("Lowercase currency code, for example usdc."),
                    "funding_mode": funding_mode_property(),
                    "privacy": privacy_property(),
                    "operator_id": string_property("Human or service operator identifier."),
                    "note": string_property("Concise reason for approving this review item.")
                }),
                &[
                    "risk_event_id",
                    "title",
                    "template_slug",
                    "amount_minor",
                    "currency",
                    "funding_mode",
                    "privacy",
                    "operator_id",
                    "note",
                ],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "approve_risk_payout",
            "Approve a NeedsReview payout risk event so the matching verification request can continue after operator review.",
            object_tool_schema(
                json!({
                    "risk_event_id": uuid_property("Payout risk event UUID being approved."),
                    "operator_id": string_property("Human or service operator identifier."),
                    "note": string_property("Concise reason for approving this payout review item.")
                }),
                &["risk_event_id", "operator_id", "note"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reject_risk_event",
            "Reject a NeedsReview risk event and record an operator audit note without mutating bounty or payment state.",
            object_tool_schema(
                json!({
                    "risk_event_id": uuid_property("Risk event UUID being rejected."),
                    "operator_id": string_property("Human or service operator identifier."),
                    "note": string_property("Concise reason for rejecting this review item.")
                }),
                &["risk_event_id", "operator_id", "note"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
    ])
}

const OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED: &str = "OPERATOR_API_TOKEN is configured.";

fn tool(name: &'static str, description: &'static str, input_schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        input_schema,
        authorization: None,
    }
}

fn operator_tool(
    name: &'static str,
    description: &'static str,
    input_schema: Value,
    required_when: &'static str,
) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        input_schema,
        authorization: Some(ToolAuthorization {
            kind: "operator_api_token",
            header: OPERATOR_TOKEN_HEADER,
            bearer: true,
            required_when,
        }),
    }
}

fn empty_tool_schema() -> Value {
    object_tool_schema(json!({}), &[])
}

fn bounty_id_schema() -> Value {
    object_tool_schema(
        json!({ "bounty_id": uuid_property("Bounty UUID.") }),
        &["bounty_id"],
    )
}

fn bounty_solver_schema() -> Value {
    object_tool_schema(
        json!({
            "bounty_id": uuid_property("Bounty UUID."),
            "solver_agent_id": uuid_property("Solver agent UUID.")
        }),
        &["bounty_id", "solver_agent_id"],
    )
}

fn object_tool_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn string_property(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn nullable_string_property(description: &str) -> Value {
    json!({ "type": ["string", "null"], "description": description })
}

fn uuid_property(description: &str) -> Value {
    json!({ "type": "string", "format": "uuid", "description": description })
}

fn nullable_uuid_property(description: &str) -> Value {
    json!({ "type": ["string", "null"], "format": "uuid", "description": description })
}

fn integer_property(description: &str) -> Value {
    json!({ "type": "integer", "description": description })
}

fn nullable_integer_property(description: &str) -> Value {
    json!({ "type": ["integer", "null"], "description": description })
}

fn nullable_number_property(description: &str) -> Value {
    json!({ "type": ["number", "null"], "description": description })
}

fn boolean_property(description: &str) -> Value {
    json!({ "type": "boolean", "description": description })
}

fn nullable_boolean_property(description: &str) -> Value {
    json!({ "type": ["boolean", "null"], "description": description })
}

fn object_property(description: &str) -> Value {
    json!({ "type": "object", "description": description })
}

fn money_property(description: &str, allow_zero: bool) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "amount": {
                "type": "integer",
                "minimum": if allow_zero { 0 } else { 1 },
                "description": "USDC base units; native USDC uses six decimal places."
            },
            "currency": {
                "type": "string",
                "const": "usdc"
            }
        },
        "required": ["amount", "currency"],
        "additionalProperties": false
    })
}

fn autonomous_bounty_create_property() -> Value {
    json!({
        "type": "object",
        "description": "Complete immutable autonomous bounty policy. Hashes must commit to public or retrievable canonical terms before any wallet signs.",
        "properties": {
            "creator": string_property("Creator or posting agent wallet address."),
            "solver_reward": money_property("Amount paid atomically to the successful solver.", false),
            "verifier_reward": money_property("Amount split across successful precommitted verifiers; may be zero.", true),
            "terms_hash": string_property("0x-prefixed bytes32 canonical terms hash."),
            "policy_hash": string_property("0x-prefixed bytes32 full verification-policy hash."),
            "acceptance_criteria_hash": string_property("0x-prefixed bytes32 explicit acceptance-criteria hash."),
            "benchmark_hash": string_property("0x-prefixed bytes32 deterministic benchmark or judge benchmark hash."),
            "evidence_schema_hash": string_property("0x-prefixed bytes32 evidence-package schema hash."),
            "funding_deadline": integer_property("Unix timestamp after which an incomplete crowdfund can be cancelled."),
            "claim_window_seconds": integer_property("Seconds a solver has to submit after claiming."),
            "verification_window_seconds": integer_property("Seconds committed verifiers have to settle after submission."),
            "verification_mode": enum_property(&["deterministic_module", "signed_quorum", "ai_judge_quorum"], "Immutable on-chain verification mechanism."),
            "verifier_module": nullable_string_property("Deterministic verifier contract; null for quorum modes."),
            "verifier_reward_recipient": nullable_string_property("Deterministic verifier reward wallet; null for quorum modes."),
            "verifiers": string_array_property("One to eight precommitted verifier wallets for quorum modes; empty for deterministic mode."),
            "threshold": integer_property("Number of matching verifier signatures required; AI judge mode requires at least two."),
            "initial_funding": money_property("Creation-time funding. Zero creates a crowdfundable bounty; target funding makes it immediately claimable.", true),
            "creation_nonce": string_property("Unique nonzero random bytes32. It binds the CREATE2 bounty id and address.")
        },
        "required": [
            "creator", "solver_reward", "verifier_reward", "terms_hash", "policy_hash",
            "acceptance_criteria_hash", "benchmark_hash", "evidence_schema_hash",
            "funding_deadline", "claim_window_seconds", "verification_window_seconds",
            "verification_mode", "verifiers", "threshold", "initial_funding", "creation_nonce"
        ],
        "additionalProperties": false
    })
}

fn autonomous_verification_attestation_property() -> Value {
    json!({
        "type": "object",
        "properties": {
            "bounty_contract": string_property("Indexed canonical submitted bounty contract."),
            "bounty_id": string_property("0x-prefixed canonical bounty id."),
            "round": integer_property("Current positive submission round."),
            "verifier": string_property("Wallet in the immutable verifier set."),
            "submission_hash": string_property("Current indexed artifact commitment."),
            "evidence_hash": string_property("Current indexed evidence commitment."),
            "policy_hash": string_property("Immutable policy commitment."),
            "passed": boolean_property("Verifier decision."),
            "response_hash": string_property("0x-prefixed hash of the verifier response and reproducibility record."),
            "deadline": integer_property("Unix signature expiry no later than the submission verification deadline.")
        },
        "required": [
            "bounty_contract", "bounty_id", "round", "verifier", "submission_hash",
            "evidence_hash", "policy_hash", "passed", "response_hash", "deadline"
        ],
        "additionalProperties": false
    })
}

fn autonomous_signed_attestation_property() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verifier": string_property("Committed verifier wallet that signed the exact EIP-712 payload."),
            "passed": boolean_property("Signed verifier decision; every quorum member must agree."),
            "response_hash": string_property("Signed 0x-prefixed verifier response hash."),
            "deadline": integer_property("Signed Unix expiry."),
            "signature": string_property("0x-prefixed EOA or ERC-1271-compatible signature bytes.")
        },
        "required": ["verifier", "passed", "response_hash", "deadline", "signature"],
        "additionalProperties": false
    })
}

fn autonomous_lifecycle_schema(contract_description: &str) -> Value {
    object_tool_schema(
        json!({
            "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-mainnet."),
            "bounty_contract": string_property(contract_description),
            "caller": nullable_string_property("Optional wallet that will send the transaction; refund withdrawal requires it.")
        }),
        &["bounty_contract"],
    )
}

fn autonomous_bounty_terms_property() -> Value {
    json!({
        "type": "object",
        "description": "Public content-addressed task specification. On-chain creation commits the returned hashes.",
        "properties": {
            "schema_version": { "type": "string", "const": "agent-bounties/terms-v1" },
            "contract_terms": object_property("Immutable protocol version, creator, Base network, native USDC token, solver/verifier rewards, equal claim bond, initial funding, deadlines, work windows, and creation nonce."),
            "title": string_property("Short digital-work bounty title."),
            "goal": string_property("Complete outcome the solver must produce."),
            "acceptance_criteria": string_array_property("Explicit binary or measurable acceptance criteria."),
            "benchmark": object_property("Deterministic benchmark or AI-judge benchmark definition."),
            "evidence_schema": object_property("Required submission evidence fields and formats."),
            "verification_policy": object_property("Complete immutable verifier mechanism, engines, wallets, thresholds, model versions, prompts, rubrics, and decoding parameters."),
            "source_url": nullable_string_property("Optional public issue, repository, or task source URL."),
            "discovery_source": nullable_string_property("Optional answer describing how the poster found Agent Bounties.")
        },
        "required": ["schema_version", "contract_terms", "title", "goal", "acceptance_criteria", "benchmark", "evidence_schema", "verification_policy"],
        "additionalProperties": false
    })
}

fn nullable_object_property(description: &str) -> Value {
    json!({ "type": ["object", "null"], "description": description })
}

fn array_property(items: Value, description: &str) -> Value {
    json!({ "type": "array", "items": items, "description": description })
}

fn string_array_property(description: &str) -> Value {
    array_property(string_property("Array item."), description)
}

fn enum_property(values: &[&str], description: &str) -> Value {
    json!({ "type": "string", "enum": values, "description": description })
}

fn nullable_enum_property(values: &[&str], description: &str) -> Value {
    json!({ "type": ["string", "null"], "enum": values, "description": description })
}

fn privacy_property() -> Value {
    enum_property(
        &["Public", "RedactedPublicProof", "Private"],
        "Privacy level.",
    )
}

fn funding_mode_property() -> Value {
    enum_property(
        &["Simulated", "StripeFiatLedger"],
        "Off-chain funding mode. Base USDC uses autonomous-v1 tools.",
    )
}

fn verifier_kind_property() -> Value {
    enum_property(
        &[
            "Manual",
            "JsonSchema",
            "DockerCommand",
            "GitHubCi",
            "HttpCallback",
            "AiJudgeFilter",
        ],
        "Verifier kind.",
    )
}

async fn route_blocked_goal(
    State(state): State<SharedState>,
    Json(args): Json<RouteBlockedGoalArgs>,
) -> Json<serde_json::Value> {
    let agent = Agent::new("mcp-requester");
    let request = HelpRequest::new(
        agent.id,
        args.goal,
        args.context,
        Money::new(args.budget_minor, args.currency).unwrap_or(Money {
            amount: 1,
            currency: "usdc".to_string(),
        }),
        args.privacy,
    );
    let capabilities = state
        .network
        .lock()
        .expect("state poisoned")
        .capabilities
        .values()
        .cloned()
        .collect::<Vec<_>>();
    let decision = BountyRouter::default().route_blocked_goal(&request, &capabilities);
    mcp_json(decision)
}

async fn register_agent(
    State(state): State<SharedState>,
    Json(args): Json<RegisterAgentRequest>,
) -> Json<serde_json::Value> {
    let agent = {
        let mut network = state.network.lock().expect("state poisoned");
        network.register_agent(args)
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_agent(&agent).await {
            return mcp_error(error);
        }
    }
    mcp_json(agent)
}

async fn register_capability(
    State(state): State<SharedState>,
    Json(args): Json<RegisterCapabilityRequest>,
) -> Json<serde_json::Value> {
    let capability = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.register_capability(args) {
            Ok(capability) => capability,
            Err(error) => return mcp_error(error),
        }
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_capability(&capability).await {
            return mcp_error(error);
        }
    }
    mcp_json(capability)
}

async fn search_capabilities(
    State(state): State<SharedState>,
    Json(args): Json<SearchCapabilitiesArgs>,
) -> Json<serde_json::Value> {
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
    let api_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let class_filter = args.class.map(|class| format!("{class:?}"));
    let template_filter = args.template_slug;
    let currency_filter = args.currency.map(|currency| currency.to_ascii_lowercase());
    let mut feed = web_public::public_capability_feed(
        &capabilities,
        &agents,
        &reputation_events,
        &settlements,
        &api_base_url,
    );
    feed.retain(|item| {
        class_filter
            .as_ref()
            .map(|class| item.class == *class)
            .unwrap_or(true)
            && template_filter
                .as_ref()
                .map(|template| item.template_slugs.iter().any(|slug| slug == template))
                .unwrap_or(true)
            && currency_filter
                .as_ref()
                .map(|currency| item.currency == *currency)
                .unwrap_or(true)
            && args
                .max_price_minor
                .map(|max_price| item.min_price_minor <= max_price)
                .unwrap_or(true)
    });
    mcp_json(feed)
}

async fn request_quotes(
    State(state): State<SharedState>,
    Json(args): Json<CreateHelpRequestRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.create_help_request(args).and_then(|help_request| {
            network.request_quotes(RequestQuotesRequest {
                help_request_id: help_request.id,
            })
        })
    };
    let quotes = match result {
        Ok(quotes) => quotes,
        Err(error) => {
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_help_request(&quotes.help_request).await {
            return mcp_error(error);
        }
        for quote in &quotes.quotes {
            if let Err(error) = store.upsert_quote(quote).await {
                return mcp_error(error);
            }
        }
    }
    mcp_json(quotes)
}

async fn fund_quote_as_bounty(
    State(state): State<SharedState>,
    Json(args): Json<FundQuoteRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .fund_quote_as_bounty(args)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
    };
    let (bounty, ledger_entries) = match result {
        Ok(result) => result,
        Err(error) => {
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    if let Err(error) = persist_bounty_and_ledger(&state, &bounty, &ledger_entries).await {
        return mcp_error(error);
    }
    mcp_json(bounty)
}

async fn post_bounty(
    State(state): State<SharedState>,
    Json(args): Json<PostBountyRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .post_funded_bounty(args)
            .map(|bounty| (bounty, network.ledger.entries().to_vec()))
    };
    let (bounty, ledger_entries) = match result {
        Ok(result) => result,
        Err(error) => {
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    if let Err(error) = persist_bounty_and_ledger(&state, &bounty, &ledger_entries).await {
        return mcp_error(error);
    }
    mcp_json(bounty)
}

async fn open_pooled_bounty(
    State(state): State<SharedState>,
    Json(args): Json<OpenPooledBountyRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.open_pooled_bounty(args)
    };
    let bounty = match result {
        Ok(bounty) => bounty,
        Err(error) => {
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    if let Err(error) = persist_bounty_and_ledger(&state, &bounty, &[]).await {
        return mcp_error(error);
    }
    mcp_json(bounty)
}

async fn add_bounty_funding(
    State(state): State<SharedState>,
    Json(args): Json<AddFundingContributionRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.add_funding_contribution(args)
    };
    let report = match result {
        Ok(report) => report,
        Err(error) => return mcp_error(error),
    };
    if let Err(error) = persist_pooled_funding_report(&state, &report).await {
        return mcp_error(error);
    }
    mcp_json(report)
}

async fn create_funding_intent(
    State(state): State<SharedState>,
    Json(args): Json<CreateFundingIntentRequest>,
) -> Json<serde_json::Value> {
    let platform_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.create_funding_intent(args, platform_base_url)
    };
    let report = match result {
        Ok(report) => report,
        Err(error) => return mcp_error(error),
    };
    if let Err(error) = persist_funding_intent_report(&state, &report).await {
        return mcp_error(error);
    }
    mcp_json(report)
}

async fn list_claimable_bounties(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let bounties = {
        let network = state.network.lock().expect("state poisoned");
        network.list_claimable_bounties()
    };
    let api_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    mcp_json(web_public::public_bounty_feed(&bounties, &api_base_url))
}

async fn claim_bounty(
    State(state): State<SharedState>,
    Json(args): Json<ClaimBountyRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.claim_bounty(args) {
            Ok(bounty) => {
                let claim = network
                    .claims
                    .values()
                    .find(|claim| claim.bounty_id == bounty.id)
                    .expect("claim exists after successful claim")
                    .clone();
                Ok((bounty, claim))
            }
            Err(error) => Err(error),
        }
    };
    let (bounty, claim) = match result {
        Ok(result) => result,
        Err(error) => return mcp_error(error),
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_bounty(&bounty).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_claim(&claim).await {
            return mcp_error(error);
        }
    }
    mcp_json(bounty)
}

async fn submit_result(
    State(state): State<SharedState>,
    Json(args): Json<SubmitResultRequest>,
) -> Json<serde_json::Value> {
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network.submit_result(args).map(|submission| {
            let bounty = network
                .bounties
                .get(&submission.bounty_id)
                .expect("submission bounty exists")
                .clone();
            (submission, bounty)
        })
    };
    let (submission, bounty) = match result {
        Ok(result) => result,
        Err(error) => {
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_bounty(&bounty).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_submission(&submission).await {
            return mcp_error(error);
        }
    }
    mcp_json(submission)
}

async fn request_verification(
    State(state): State<SharedState>,
    Json(args): Json<VerifySubmissionRequest>,
) -> Json<serde_json::Value> {
    let mut network = {
        let mut guard = state.network.lock().expect("state poisoned");
        std::mem::take(&mut *guard)
    };
    let result = network.verify_submission(args).await;
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
        Err(error) => {
            {
                let mut guard = state.network.lock().expect("state poisoned");
                *guard = network;
            }
            if let Err(persist_error) = persist_all_risk_events(&state).await {
                return mcp_error(persist_error);
            }
            return mcp_error(error);
        }
    };
    {
        let mut guard = state.network.lock().expect("state poisoned");
        *guard = network;
    }
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_bounty(&bounty).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_verifier_result(&verifier_result).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_proof_record(&proof).await {
            return mcp_error(error);
        }
        for settlement in &settlements {
            if let Err(error) = store.upsert_settlement(settlement).await {
                return mcp_error(error);
            }
        }
        for contribution in &funding_contributions {
            if let Err(error) = store.upsert_funding_contribution(contribution).await {
                return mcp_error(error);
            }
        }
        for event in &reputation_events {
            if let Err(error) = store.upsert_reputation_event(event).await {
                return mcp_error(error);
            }
        }
        for signal in &template_signals {
            if let Err(error) = store.upsert_template_signal(signal).await {
                return mcp_error(error);
            }
        }
        if let Err(error) = persist_ledger_entries(store, &ledger_entries).await {
            return mcp_error(error);
        }
    }
    mcp_json(proof)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BountyIdArgs {
    bounty_id: Uuid,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PaidStatusArgs {
    bounty_id: Option<Uuid>,
    agent_id: Option<Uuid>,
}

async fn get_bounty_status(
    State(state): State<SharedState>,
    Json(args): Json<BountyIdArgs>,
) -> Json<serde_json::Value> {
    match bounty_status_snapshot(&state, args.bounty_id).await {
        Ok(status) => mcp_json(status),
        Err(error) => mcp_error(error),
    }
}

async fn bounty_status_snapshot(
    state: &SharedState,
    bounty_id: Uuid,
) -> Result<BountyStatusResponse, String> {
    if let Some(store) = &state.store {
        let scope = store
            .load_bounty_status_scope(bounty_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "bounty not found".to_string())?;
        return bounty_status_from_scope(scope);
    }

    let status = {
        let network = state.network.lock().expect("state poisoned");
        network
            .status(bounty_id)
            .map_err(|error| error.to_string())?
    };
    Ok(status)
}

fn bounty_status_from_scope(scope: BountyStatusScope) -> Result<BountyStatusResponse, String> {
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
        .map_err(|error| error.to_string())?;
    Ok(status)
}

async fn get_paid_status(
    State(state): State<SharedState>,
    Json(args): Json<PaidStatusArgs>,
) -> Json<serde_json::Value> {
    match (args.bounty_id, args.agent_id) {
        (Some(bounty_id), None) => match bounty_status_snapshot(&state, bounty_id).await {
            Ok(status) => {
                let api = public_base_url_from_env();
                let proof_url = status.proofs.first().map(|proof| {
                    format!("{}/public/proofs/{}", api.trim_end_matches('/'), proof.id)
                });
                let trigger = if status.bounty.status == BountyStatus::Paid
                    && status.bounty.funding_mode != domain::FundingMode::Simulated
                {
                    Some(web_public::PostValueTrigger::ReconciledPayout)
                } else if !status.proofs.is_empty() {
                    Some(web_public::PostValueTrigger::VerifiedCompletion)
                } else if status.funding_summary.claimable {
                    Some(web_public::PostValueTrigger::FundedBounty)
                } else {
                    None
                };
                let share_url = proof_url.unwrap_or_else(|| {
                    format!("{}/public/bounties/{bounty_id}", api.trim_end_matches('/'))
                });
                let post_value_loop = trigger
                    .map(|trigger| web_public::post_value_loop(Some(trigger), Some(&share_url)));
                mcp_json(serde_json::json!({
                    "scope": "bounty",
                    "bounty_id": bounty_id,
                    "bounty_status": status.bounty.status,
                    "settlements": status.settlements,
                    "template_signals": status.template_signals,
                    "risk_events": status.risk_events,
                    "post_value_loop": post_value_loop
                }))
            }
            Err(error) => mcp_error(error),
        },
        (None, Some(agent_id)) => {
            let network = state.network.lock().expect("state poisoned");
            match network.agent_payout_status(agent_id) {
                Ok(status) => {
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
                    let api = public_base_url_from_env();
                    let share_url = evidence_payout
                        .map(|payout| {
                            format!(
                                "{}/public/proofs/{}",
                                api.trim_end_matches('/'),
                                payout.proof_record_id
                            )
                        })
                        .unwrap_or_else(|| {
                            format!("{}/public/agents/{agent_id}", api.trim_end_matches('/'))
                        });
                    let post_value_loop = trigger.map(|trigger| {
                        web_public::post_value_loop(Some(trigger), Some(&share_url))
                    });
                    mcp_json(serde_json::json!({
                        "scope": "agent",
                        "agent_id": agent_id,
                        "agent": status.agent,
                        "payouts": status.payouts,
                        "totals": status.totals,
                        "reputation_events": status.reputation_events,
                        "post_value_loop": post_value_loop
                    }))
                }
                Err(error) => mcp_error(error),
            }
        }
        (None, None) => mcp_error("get_paid_status requires bounty_id or agent_id"),
        (Some(_), Some(_)) => {
            mcp_error("get_paid_status accepts either bounty_id or agent_id, not both")
        }
    }
}

async fn plan_stripe_checkout_top_up(
    State(state): State<SharedState>,
    Json(args): Json<PlanStripeCheckoutTopUpArgs>,
) -> Json<serde_json::Value> {
    match stripe_checkout_top_up_intent(&state, args) {
        Ok(intent) => mcp_json(intent),
        Err(error) => mcp_error(error),
    }
}

fn stripe_checkout_top_up_intent(
    state: &SharedState,
    args: PlanStripeCheckoutTopUpArgs,
) -> Result<StripeRequestIntent, Box<dyn std::error::Error + Send + Sync>> {
    let platform_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let planner = stripe_planner_for_state(state, platform_base_url.clone());
    let amount = Money::new(args.amount_minor, args.currency)?;
    Ok(planner.checkout_top_up(&CheckoutTopUpRequest {
        organization_id: args.organization_id,
        amount,
        success_url: args
            .success_url
            .unwrap_or_else(|| format!("{platform_base_url}/stripe/success")),
        cancel_url: args
            .cancel_url
            .unwrap_or_else(|| format!("{platform_base_url}/stripe/cancel")),
    })?)
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

async fn plan_stripe_connect_account(
    Json(args): Json<PlanStripeConnectAccountArgs>,
) -> Json<serde_json::Value> {
    match stripe_connect_account_intent(args) {
        Ok(intent) => mcp_json(intent),
        Err(error) => mcp_error(error),
    }
}

fn stripe_connect_account_intent(
    args: PlanStripeConnectAccountArgs,
) -> Result<payments_stripe::ConnectAccountV2CreateIntent, payments_stripe::StripeIntegrationError>
{
    StripePlanner::new("http://127.0.0.1:8080").connect_account_v2(args.agent_id)
}

async fn plan_stripe_connect_transfer(
    State(state): State<SharedState>,
    Json(args): Json<PlanStripeConnectTransferArgs>,
) -> Json<serde_json::Value> {
    match stripe_connect_transfer_plan(&state, args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

fn stripe_connect_transfer_plan(
    state: &SharedState,
    args: PlanStripeConnectTransferArgs,
) -> Result<app::StripeTransferPlan, app::AppError> {
    let platform_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let network = state.network.lock().expect("state poisoned");
    network.plan_stripe_transfer(
        PlanStripeTransferRequest {
            payout_intent_id: args.payout_intent_id,
            connected_account_id: args.connected_account_id,
        },
        platform_base_url,
    )
}

async fn execute_stripe_checkout_top_up(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<PlanStripeCheckoutTopUpArgs>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let intent = match stripe_checkout_top_up_intent(&state, args) {
        Ok(intent) => intent,
        Err(error) => return mcp_error(error),
    };
    execute_stripe_intent(&state, intent).await
}

async fn execute_stripe_connect_account(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<PlanStripeConnectAccountArgs>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let intent = match stripe_connect_account_intent(args) {
        Ok(intent) => intent.request,
        Err(error) => return mcp_error(error),
    };
    execute_stripe_intent(&state, intent).await
}

async fn execute_stripe_connect_transfer(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<PlanStripeConnectTransferArgs>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let plan = match stripe_connect_transfer_plan(&state, args) {
        Ok(plan) => plan,
        Err(error) => return mcp_error(error),
    };
    execute_stripe_intent(&state, plan.request).await
}

async fn execute_stripe_intent(
    state: &SharedState,
    intent: StripeRequestIntent,
) -> Json<serde_json::Value> {
    if !state.stripe_live_execution_enabled {
        return mcp_error("live Stripe execution is disabled");
    }
    let secret_key = match state
        .stripe_secret_key
        .as_deref()
        .filter(|secret| !secret.trim().is_empty())
    {
        Some(secret_key) => secret_key,
        None => return mcp_error("STRIPE_SECRET_KEY is not configured"),
    };
    match execute_stripe_request(&intent, secret_key, &state.stripe_api_base_url).await {
        Ok(report) => mcp_json(report),
        Err(error) => mcp_error(error),
    }
}

async fn reconcile_stripe_connect_snapshot(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<ConnectAccountSnapshot>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.apply_stripe_connect_snapshot(args) {
            Ok(reconciliation) => reconciliation,
            Err(error) => return mcp_error(error),
        }
    };
    if let Some(store) = &state.store {
        for bounty in &reconciliation.bounties {
            if let Err(error) = store.upsert_bounty(bounty).await {
                return mcp_error(error);
            }
        }
        for settlement in &reconciliation.settlements {
            if let Err(error) = store.upsert_settlement(settlement).await {
                return mcp_error(error);
            }
        }
        if let Err(error) = persist_ledger_entries(store, &reconciliation.ledger_entries).await {
            return mcp_error(error);
        }
    }
    mcp_json(reconciliation)
}

async fn reconcile_stripe_transfer_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<StripeWebhookEvent>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let evidence = match StripeEventDeduper::default().apply_connect_transfer(&args) {
        Ok(evidence) => evidence,
        Err(error) => return mcp_error(error),
    };
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.apply_stripe_transfer_evidence(evidence) {
            Ok(reconciliation) => reconciliation,
            Err(error) => return mcp_error(error),
        }
    };
    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            if let Err(error) = store
                .upsert_payment_event(&reconciliation.evidence.payment_event)
                .await
            {
                return mcp_error(error);
            }
        }
        if let Some(settlement) = &reconciliation.settlement {
            if let Err(error) = store.upsert_settlement(settlement).await {
                return mcp_error(error);
            }
        }
        if let Some(bounty) = &reconciliation.bounty {
            if let Err(error) = store.upsert_bounty(bounty).await {
                return mcp_error(error);
            }
        }
        if let Err(error) = persist_ledger_entries(store, &reconciliation.ledger_entries).await {
            return mcp_error(error);
        }
    }
    mcp_json(reconciliation)
}

async fn reconcile_stripe_checkout_webhook(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<StripeWebhookEvent>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let funding_credit = match StripeEventDeduper::default().apply_checkout_top_up(&args) {
        Ok(funding_credit) => funding_credit,
        Err(error) => return mcp_error(error),
    };
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.apply_stripe_funding_credit(funding_credit) {
            Ok(reconciliation) => reconciliation,
            Err(error) => return mcp_error(error),
        }
    };
    if let Some(store) = &state.store {
        if !reconciliation.duplicate {
            if let Err(error) = store
                .upsert_payment_event(&reconciliation.funding_credit.payment_event)
                .await
            {
                return mcp_error(error);
            }
        }
        if let Some(intent) = &reconciliation.funding_intent {
            if let Err(error) = store.upsert_funding_intent(intent).await {
                return mcp_error(error);
            }
        }
        if let Some(report) = &reconciliation.funding_report {
            if let Err(error) = store.upsert_bounty(&report.bounty).await {
                return mcp_error(error);
            }
            if let Err(error) = store
                .upsert_funding_contribution(&report.contribution)
                .await
            {
                return mcp_error(error);
            }
        }
        if let Err(error) = persist_ledger_entries(store, &reconciliation.ledger_entries).await {
            return mcp_error(error);
        }
    }
    mcp_json(reconciliation)
}

async fn plan_github_issue_bounty(
    Json(args): Json<PlanGitHubIssueBountyArgs>,
) -> Json<serde_json::Value> {
    let parsed =
        parse_issue_form_bounty(&args.repository, &args.issue_url, &args.title, &args.body);
    match parsed {
        Ok(bounty) => {
            let check = bounty_check_output(Ok(&bounty));
            mcp_json(serde_json::json!({
                "ready": true,
                "parsed": bounty,
                "error": null,
                "check": check
            }))
        }
        Err(error) => {
            let check = bounty_check_output(Err(&error));
            mcp_json(serde_json::json!({
                "ready": false,
                "parsed": null,
                "error": error.to_string(),
                "check": check
            }))
        }
    }
}

async fn plan_github_funding_comment(
    Json(args): Json<PlanGitHubFundingCommentArgs>,
) -> Json<serde_json::Value> {
    mcp_json(funding_comment_plan(GitHubFundingCommentInput {
        repository: args.repository,
        issue_url: args.issue_url,
        title: args.title,
        body: args.body,
        comment_body: args.comment_body,
        contributor_login: args.contributor_login,
        comment_id: args.comment_id,
        funding_api_base_url: args.funding_api_base_url,
        existing_idempotency_keys: args.existing_idempotency_keys,
    }))
}

async fn plan_github_claim_comment(
    Json(args): Json<PlanGitHubClaimCommentArgs>,
) -> Json<serde_json::Value> {
    mcp_json(claim_comment_plan(GitHubClaimCommentInput {
        repository: args.repository,
        issue_url: args.issue_url,
        title: args.title,
        body: args.body,
        comment_body: args.comment_body,
        contributor_login: args.contributor_login,
        comment_id: args.comment_id,
        claim_age_minutes: args.claim_age_minutes,
        progress_signal_count: args.progress_signal_count,
        active_claim_login: args.active_claim_login,
    }))
}

async fn plan_github_proof_comment(
    Json(args): Json<PlanGitHubProofCommentArgs>,
) -> Json<serde_json::Value> {
    let comment = GitHubProofComment {
        bounty_id: args.bounty_id,
        proof_url: args.proof_url,
        verifier_summary: args.verifier_summary,
        settlement_url: args.settlement_url,
    };
    mcp_json(proof_comment_plan(comment))
}

async fn plan_github_proof_comment_for_proof(
    State(state): State<SharedState>,
    Json(args): Json<PlanGitHubProofCommentForProofArgs>,
) -> Json<serde_json::Value> {
    let public_base_url = public_base_url_from_env();
    let network = state.network.lock().expect("state poisoned");
    let Some(proof) = network.proofs.get(&args.proof_id) else {
        return mcp_error("proof not found");
    };
    if proof.privacy == PrivacyLevel::Private {
        return mcp_error("proof not found");
    }
    let Some(verifier) = network.verifier_results.get(&proof.verifier_result_id) else {
        return mcp_error("proof verifier result not found");
    };
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
        settlement_url: args.settlement_url,
    };
    mcp_json(proof_comment_plan(comment))
}

async fn broadcast_base_signed_transaction(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<BroadcastBaseSignedTransactionArgs>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    if !state.base_broadcast_enabled {
        return mcp_error(
            "Base transaction broadcast is disabled; set ENABLE_BASE_TX_BROADCAST=true",
        );
    }
    let request_id = args.request_id.unwrap_or(1);
    let network_name = args.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = match state.base_rpc_urls.resolve(network_name) {
        Ok(resolved) => resolved,
        Err(error) => return mcp_error(error),
    };
    let request = match eth_send_raw_transaction_request(&args.signed_transaction, request_id) {
        Ok(request) => request,
        Err(error) => return mcp_error(error),
    };
    let response =
        match broadcast_signed_transaction(&rpc_url, &args.signed_transaction, request_id).await {
            Ok(response) => response,
            Err(error) => return mcp_error(error),
        };

    mcp_json(serde_json::json!({
        "network": network,
        "request": request,
        "tx_hash": response.result,
        "next_step": "Poll get_base_transaction_receipt for inclusion. The autonomous indexer independently reconciles canonical factory and bounty logs; a receipt alone never proves settlement."
    }))
}

async fn get_base_transaction_receipt(
    State(state): State<SharedState>,
    Json(args): Json<GetBaseTransactionReceiptArgs>,
) -> Json<serde_json::Value> {
    let request_id = args.request_id.unwrap_or(1);
    let network_name = args.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = match state.base_rpc_urls.resolve(network_name) {
        Ok(resolved) => resolved,
        Err(error) => return mcp_error(error),
    };
    let request = match eth_get_transaction_receipt_request(&args.tx_hash, request_id) {
        Ok(request) => request,
        Err(error) => return mcp_error(error),
    };
    let tx_hash = request.params[0].clone();
    let response = match fetch_transaction_receipt(&rpc_url, &tx_hash, request_id).await {
        Ok(response) => response,
        Err(error) => return mcp_error(error),
    };
    let Some(receipt) = response.result else {
        return mcp_json(serde_json::json!({
            "network": network,
            "request": request,
            "receipt_found": false,
            "tx_hash": tx_hash,
            "block_number": null,
            "succeeded": null,
            "log_count": 0,
            "receipt": null
        }));
    };
    let block_number = match receipt.block_number() {
        Ok(block_number) => block_number,
        Err(error) => return mcp_error(error),
    };
    let succeeded = match receipt.succeeded() {
        Ok(succeeded) => succeeded,
        Err(error) => return mcp_error(error),
    };
    let log_count = receipt.logs.len();

    mcp_json(serde_json::json!({
        "network": network,
        "request": request,
        "receipt_found": true,
        "tx_hash": tx_hash,
        "block_number": block_number,
        "succeeded": succeeded,
        "log_count": log_count,
        "receipt": receipt
    }))
}

fn configured_autonomous_planner(network: &str) -> Result<AutonomousBountyTxPlanner, String> {
    let descriptor = base_network_descriptor(network).map_err(|error| error.to_string())?;
    let (factory_env, implementation_env) = match descriptor.chain_id {
        8_453 => (
            "BASE_MAINNET_BOUNTY_FACTORY",
            "BASE_MAINNET_BOUNTY_IMPLEMENTATION",
        ),
        84_532 => (
            "BASE_SEPOLIA_BOUNTY_FACTORY",
            "BASE_SEPOLIA_BOUNTY_IMPLEMENTATION",
        ),
        _ => return Err("unsupported Base network".to_string()),
    };
    let (factory, implementation) = autonomous_planner_addresses(
        descriptor.chain_id,
        env::var(factory_env).ok(),
        env::var(implementation_env).ok(),
    )?;
    AutonomousBountyTxPlanner::new(factory, implementation).map_err(|error| error.to_string())
}

const CANONICAL_BASE_MAINNET_BOUNTY_FACTORY: &str = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
const CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION: &str =
    "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";

fn configured_address(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn autonomous_planner_addresses(
    chain_id: u64,
    configured_factory: Option<String>,
    configured_implementation: Option<String>,
) -> Result<(String, String), String> {
    let factory = configured_address(configured_factory);
    let implementation = configured_address(configured_implementation);
    if chain_id == 8_453 {
        if factory.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        }) || implementation.as_deref().is_some_and(|address| {
            !address.eq_ignore_ascii_case(CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION)
        }) {
            return Err("configured Base mainnet autonomous deployment does not match the canonical attested deployment".to_string());
        }
        return Ok((
            CANONICAL_BASE_MAINNET_BOUNTY_FACTORY.to_string(),
            CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION.to_string(),
        ));
    }
    if chain_id == 84_532 {
        return Ok((
            factory.ok_or_else(|| {
                "hosted autonomous protocol is not configured: set BASE_SEPOLIA_BOUNTY_FACTORY"
                    .to_string()
            })?,
            implementation.ok_or_else(|| {
                "hosted autonomous protocol is not configured: set BASE_SEPOLIA_BOUNTY_IMPLEMENTATION"
                    .to_string()
            })?,
        ));
    }
    Err("unsupported Base network".to_string())
}

async fn require_indexed_canonical_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<(), String> {
    let item = indexed_autonomous_bounty(state, network, bounty_contract).await?;
    if item.terms_valid {
        Ok(())
    } else {
        Err(format!(
            "canonical bounty terms do not match contract commitments: {}",
            item.validation_errors.join("; ")
        ))
    }
}

async fn indexed_autonomous_bounty(
    state: &SharedState,
    network: &str,
    bounty_contract: &str,
) -> Result<AutonomousBountyFeedItem, String> {
    let Some(store) = &state.store else {
        return Err(
            "DATABASE_URL is required before planning actions against a canonical bounty"
                .to_string(),
        );
    };
    let planner = configured_autonomous_planner(network)?;
    let events = store
        .list_autonomous_bounty_events(network)
        .await
        .map_err(|error| error.to_string())?;
    let contracts = store
        .list_canonical_autonomous_bounty_contracts(network, &planner.factory_contract)
        .await
        .map_err(|error| error.to_string())?;
    if !contracts
        .iter()
        .any(|contract| contract.eq_ignore_ascii_case(bounty_contract))
    {
        return Err("bounty contract is not indexed as canonical for this network".to_string());
    }
    let terms = store
        .list_autonomous_bounty_terms()
        .await
        .map_err(|error| error.to_string())?;
    let mut feed =
        build_autonomous_bounty_feed(events, terms, false).map_err(|error| error.to_string())?;
    state.recovery_reservations.apply(&mut feed, false);
    feed.into_iter()
        .find(|item| item.bounty_contract.eq_ignore_ascii_case(bounty_contract))
        .ok_or_else(|| "canonical bounty has no indexed feed state".to_string())
}

async fn plan_autonomous_canonical_child_terms(
    Json(args): Json<CanonicalChildBountyTermsRequest>,
) -> Json<serde_json::Value> {
    match build_canonical_child_bounty_terms_plan(&args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_bounty_creation(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyCreationArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) = require_autonomous_creation_terms(&state, network, &args.create).await {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_creation(network, &args.create)
            .map_err(|e| e.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_bounty_authorized_creation(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyAuthorizedCreationArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) = require_autonomous_creation_terms(&state, network, &args.create).await {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_authorized_creation(
                network,
                &args.create,
                &args.signature,
                args.relayer.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn require_autonomous_creation_terms(
    state: &SharedState,
    network: &str,
    create: &AutonomousBountyCreate,
) -> Result<(), String> {
    let Some(store) = &state.store else {
        return Err("DATABASE_URL is required before planning canonical creation".to_string());
    };
    let terms = store
        .get_autonomous_bounty_terms(&create.terms_hash)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "published autonomous bounty terms are unavailable".to_string())?;
    validate_autonomous_creation_against_terms(network, create, &terms)
        .map_err(|error| error.to_string())
}

async fn plan_autonomous_bounty_contribution(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyContributionArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) =
        require_indexed_canonical_bounty(&state, network, &args.contribution.bounty_contract).await
    {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_contribution(network, &args.contribution)
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_bounty_authorized_contribution(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyAuthorizedContributionArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) =
        require_indexed_canonical_bounty(&state, network, &args.contribution.bounty_contract).await
    {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_authorized_contribution(
                network,
                &args.contribution,
                &args.signature,
                args.relayer.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn fund_bounty_with_x402(
    State(_state): State<SharedState>,
    Json(args): Json<X402BountyFundingArgs>,
) -> Json<serde_json::Value> {
    let bounty_contract = match normalize_evm_address(&args.bounty_contract) {
        Ok(address) => address,
        Err(error) => return mcp_error(error),
    };
    let network = args.network.unwrap_or_else(|| "base-mainnet".to_string());
    if base_network_descriptor(&network).is_err() {
        return mcp_error("network must be base-mainnet or base-sepolia");
    }
    let url = format!(
        "{}/v1/x402/base/bounties/{}/funding",
        public_base_url_from_env().trim_end_matches('/'),
        bounty_contract
    );
    let mut query = vec![("network", network)];
    if let Some(amount) = args.amount {
        query.push(("amount", amount.to_string()));
    }
    if let Some(relayer) = args.relayer {
        query.push(("relayer", relayer));
    }
    let mut request = reqwest::Client::new().get(url).query(&query);
    if let Some(payment_signature) = args.payment_signature {
        request = request.header("PAYMENT-SIGNATURE", payment_signature);
    }
    proxy_x402_response(request).await
}

async fn get_x402_relay_status(
    State(_state): State<SharedState>,
    Json(args): Json<GetX402RelayStatusArgs>,
) -> Json<serde_json::Value> {
    let url = format!(
        "{}/v1/x402/base/relays/{}",
        public_base_url_from_env().trim_end_matches('/'),
        args.relay_id
    );
    proxy_x402_response(reqwest::Client::new().get(url)).await
}

async fn proxy_x402_response(request: reqwest::RequestBuilder) -> Json<serde_json::Value> {
    let response = match request
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            return mcp_error(if error.is_timeout() {
                "canonical x402 API timed out"
            } else {
                "canonical x402 API is unavailable"
            })
        }
    };
    let status = response.status();
    let payment_required = response
        .headers()
        .get("PAYMENT-REQUIRED")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let payment_response = response
        .headers()
        .get("PAYMENT-RESPONSE")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let body_text = match response.text().await {
        Ok(body) => body,
        Err(_) => return mcp_error("canonical x402 API response body is unavailable"),
    };
    let body = serde_json::from_str::<serde_json::Value>(&body_text).unwrap_or_else(|_| {
        json!({
            "error": if body_text.is_empty() {
                format!("HTTP {}", status.as_u16())
            } else {
                body_text
            }
        })
    });
    mcp_json(json!({
        "http_status": status.as_u16(),
        "payment_required_header": payment_required,
        "payment_response_header": payment_response,
        "body": body
    }))
}

async fn plan_autonomous_bounty_claim(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyClaimArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if let Err(error) = require_claimable_autonomous_item(&item) {
        return mcp_error(error);
    }
    let claim_bond = match item.claim_bond.parse::<u128>() {
        Ok(value) => value,
        Err(_) => return mcp_error("indexed claim bond is invalid"),
    };
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_claim(
                network,
                &args.bounty_contract,
                &args.solver,
                claim_bond,
                args.authorization_nonce.as_deref(),
                args.authorization_valid_before,
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_bounty_authorized_claim(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountyAuthorizedClaimArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if let Err(error) = require_claimable_autonomous_item(&item) {
        return mcp_error(error);
    }
    let claim_bond = match item.claim_bond.parse::<u128>() {
        Ok(value) => value,
        Err(_) => return mcp_error("indexed claim bond is invalid"),
    };
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_authorized_claim(
                network,
                &args.bounty_contract,
                &args.solver,
                claim_bond,
                &args.authorization_nonce,
                args.authorization_valid_before,
                &args.signature,
                args.relayer.as_deref(),
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

fn require_claimable_autonomous_item(item: &AutonomousBountyFeedItem) -> Result<(), String> {
    if !item.terms_valid {
        return Err(format!(
            "canonical bounty terms do not match contract commitments: {}",
            item.validation_errors.join("; ")
        ));
    }
    if !autonomous_bounty_is_earning_ready(item) {
        return Err(format!(
            "canonical bounty is not executable earning inventory: {}",
            item.verification_readiness_reason
        ));
    }
    Ok(())
}

async fn plan_autonomous_bounty_submission(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountySubmissionArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) =
        require_indexed_canonical_bounty(&state, network, &args.bounty_contract).await
    {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_submission(
                &args.bounty_contract,
                &args.solver,
                &args.submission_hash,
                &args.evidence_hash,
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_bounty_submission_authorization(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousBountySubmissionAuthorizationArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) =
        require_indexed_canonical_bounty(&state, network, &args.submission.bounty_contract).await
    {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_submission_authorization(network, &args.submission)
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_verification_attestation(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousVerificationAttestationArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item =
        match indexed_autonomous_bounty(&state, network, &args.attestation.bounty_contract).await {
            Ok(item) => item,
            Err(error) => return mcp_error(error),
        };
    let observed_at = match u64::try_from(Utc::now().timestamp()) {
        Ok(value) => value,
        Err(_) => return mcp_error("system clock is before Unix epoch"),
    };
    if let Err(error) =
        validate_attestation_request_against_feed(&item, &args.attestation, observed_at)
    {
        return mcp_error(error);
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_verification_attestation(network, &args.attestation)
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_module_settlement(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousModuleSettlementArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if !item.terms_valid
        || item.status != "submitted"
        || autonomous_item_mode(&item).as_deref() != Some("deterministic_module")
    {
        return mcp_error("bounty is not a submitted deterministic-module bounty");
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_module_settlement(&args.bounty_contract, args.caller.as_deref(), &args.proof)
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_attestation_settlement(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousAttestationSettlementArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if !item.terms_valid {
        return mcp_error(format!(
            "canonical bounty terms do not match contract commitments: {}",
            item.validation_errors.join("; ")
        ));
    }
    let Some(policy) = item
        .terms
        .as_ref()
        .map(|terms| &terms.document.verification_policy)
    else {
        return mcp_error("canonical bounty terms are unavailable");
    };
    let mechanism = policy
        .get("mechanism")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let threshold = policy
        .get("threshold")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok());
    let allowed = policy.get("verifiers").and_then(Value::as_array);
    if item.status != "submitted"
        || !matches!(mechanism, "signed_quorum" | "ai_judge_quorum")
        || threshold != Some(args.attestations.len())
        || allowed.is_none_or(|verifiers| {
            args.attestations.iter().any(|attestation| {
                !verifiers.iter().any(|value| {
                    value.as_str().is_some_and(|verifier| {
                        verifier.eq_ignore_ascii_case(&attestation.verifier)
                    })
                })
            })
        })
    {
        return mcp_error("attestation quorum does not match the current immutable policy");
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_attestation_settlement(
                &args.bounty_contract,
                args.caller.as_deref(),
                &args.attestations,
            )
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_expire_claim(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousLifecycleArgs>,
) -> Json<serde_json::Value> {
    plan_autonomous_lifecycle(state, args, "claimed", "expireClaim()").await
}

async fn plan_autonomous_expire_submission(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousLifecycleArgs>,
) -> Json<serde_json::Value> {
    plan_autonomous_lifecycle(state, args, "submitted", "expireSubmission()").await
}

async fn plan_autonomous_cancel(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousLifecycleArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if !matches!(item.status.as_str(), "open" | "claimable") {
        return mcp_error("bounty is not cancellable");
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_cancel(&args.bounty_contract, args.caller.as_deref())
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_refund_withdrawal(
    State(state): State<SharedState>,
    Json(args): Json<PlanAutonomousLifecycleArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if item.status != "cancelled" {
        return mcp_error("bounty is not cancelled");
    }
    let Some(contributor) = args.caller.as_deref() else {
        return mcp_error("caller is required for refund withdrawal");
    };
    match configured_autonomous_planner(network).and_then(|planner| {
        planner
            .plan_refund_withdrawal(&args.bounty_contract, contributor)
            .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_autonomous_lifecycle(
    state: SharedState,
    args: PlanAutonomousLifecycleArgs,
    expected_status: &str,
    function: &str,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    if item.status != expected_status {
        return mcp_error("bounty lifecycle state does not allow this call");
    }
    match configured_autonomous_planner(network).and_then(|planner| {
        match function {
            "expireClaim()" => {
                planner.plan_expire_claim(&args.bounty_contract, args.caller.as_deref())
            }
            "expireSubmission()" => {
                planner.plan_expire_submission(&args.bounty_contract, args.caller.as_deref())
            }
            _ => unreachable!("known autonomous lifecycle function"),
        }
        .map_err(|error| error.to_string())
    }) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

fn autonomous_item_mode(item: &AutonomousBountyFeedItem) -> Option<String> {
    item.terms
        .as_ref()
        .and_then(|terms| terms.document.verification_policy.get("mechanism"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn decode_autonomous_bounty_events(
    Json(args): Json<DecodeAutonomousBountyEventsArgs>,
) -> Json<serde_json::Value> {
    match decode_autonomous_bounty_logs(args.logs) {
        Ok(events) => mcp_json(events),
        Err(error) => mcp_error(error),
    }
}

async fn list_autonomous_bounty_events(
    State(state): State<SharedState>,
    Json(args): Json<ListAutonomousBountyEventsArgs>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return mcp_json(Vec::<serde_json::Value>::new());
    };
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    match store.list_autonomous_bounty_events(network).await {
        Ok(mut events) => {
            if let Some(bounty_id) = args.bounty_id {
                events.retain(|event| event.bounty_id.eq_ignore_ascii_case(&bounty_id));
            }
            mcp_json(events)
        }
        Err(error) => mcp_error(error),
    }
}

async fn publish_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Json(args): Json<PublishAutonomousBountyTermsArgs>,
) -> Json<serde_json::Value> {
    let record =
        match build_autonomous_bounty_terms_record(&args.creator_wallet, args.document, Utc::now())
        {
            Ok(record) => record,
            Err(error) => return mcp_error(error),
        };
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to publish public bounty terms");
    };
    match store.upsert_autonomous_bounty_terms(&record).await {
        Ok(()) => mcp_json(record),
        Err(error) => mcp_error(error),
    }
}

async fn get_autonomous_bounty_terms(
    State(state): State<SharedState>,
    Json(args): Json<GetAutonomousBountyTermsArgs>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to resolve public bounty terms");
    };
    match store.get_autonomous_bounty_terms(&args.terms_hash).await {
        Ok(Some(record)) => mcp_json(record),
        Ok(None) => mcp_error("unknown autonomous bounty terms hash"),
        Err(error) => mcp_error(error),
    }
}

async fn publish_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Json(args): Json<PublishAutonomousSubmissionEvidenceArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let item = match indexed_autonomous_bounty(&state, network, &args.bounty_contract).await {
        Ok(item) => item,
        Err(error) => return mcp_error(error),
    };
    let record = match build_autonomous_submission_evidence_record(
        network,
        &item,
        &args.bounty_contract,
        &args.bounty_id,
        args.round,
        &args.solver_wallet,
        &args.artifact_reference,
        args.evidence,
        Utc::now(),
    ) {
        Ok(record) => record,
        Err(error) => return mcp_error(error),
    };
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to publish submission evidence");
    };
    match store.upsert_autonomous_submission_evidence(&record).await {
        Ok(persisted) => mcp_json(persisted),
        Err(error) => mcp_error(error),
    }
}

async fn get_autonomous_submission_evidence(
    State(state): State<SharedState>,
    Json(args): Json<GetAutonomousSubmissionEvidenceArgs>,
) -> Json<serde_json::Value> {
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    if let Err(error) =
        require_indexed_canonical_bounty(&state, network, &args.bounty_contract).await
    {
        return mcp_error(error);
    }
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to resolve submission evidence");
    };
    match store
        .get_autonomous_submission_evidence(network, &args.bounty_contract, args.round)
        .await
    {
        Ok(Some(record)) => mcp_json(record),
        Ok(None) => mcp_error("submission evidence has not been published"),
        Err(error) => mcp_error(error),
    }
}

async fn list_autonomous_bounties(
    State(state): State<SharedState>,
    Json(args): Json<AutonomousBountyFeedArgs>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to discover indexed autonomous bounties");
    };
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let events = match store.list_autonomous_bounty_events(network).await {
        Ok(events) => events,
        Err(error) => return mcp_error(error),
    };
    let terms = match store.list_autonomous_bounty_terms().await {
        Ok(terms) => terms,
        Err(error) => return mcp_error(error),
    };
    let mut feed = match build_autonomous_bounty_feed(events, terms, false) {
        Ok(feed) => feed,
        Err(error) => return mcp_error(error),
    };
    state
        .recovery_reservations
        .apply(&mut feed, args.claimable_only.unwrap_or(false));
    mcp_json(feed)
}

async fn list_autonomous_verification_jobs(
    State(state): State<SharedState>,
    Json(args): Json<AutonomousVerificationJobsArgs>,
) -> Json<serde_json::Value> {
    let Some(store) = &state.store else {
        return mcp_error("DATABASE_URL is required to discover verification jobs");
    };
    let network = args.network.as_deref().unwrap_or("base-mainnet");
    let events = match store.list_autonomous_bounty_events(network).await {
        Ok(events) => events,
        Err(error) => return mcp_error(error),
    };
    let terms = match store.list_autonomous_bounty_terms().await {
        Ok(terms) => terms,
        Err(error) => return mcp_error(error),
    };
    let evidence = match store.list_autonomous_submission_evidence(network).await {
        Ok(evidence) => evidence,
        Err(error) => return mcp_error(error),
    };
    let mut feed = match build_autonomous_bounty_feed(events, terms, false) {
        Ok(feed) => feed,
        Err(error) => return mcp_error(error),
    };
    state
        .recovery_reservations
        .exclude_from_verification_jobs(&mut feed);
    let observed_at = match u64::try_from(Utc::now().timestamp()) {
        Ok(value) => value,
        Err(_) => return mcp_error("system clock is before Unix epoch"),
    };
    let mut jobs = match build_autonomous_verification_jobs(network, feed, evidence, observed_at) {
        Ok(jobs) => jobs,
        Err(error) => return mcp_error(error),
    };
    if let Some(verifier) = args.verifier {
        let verifier = match normalize_evm_address(verifier) {
            Ok(verifier) => verifier,
            Err(error) => return mcp_error(error),
        };
        jobs.retain(|job| {
            job.verification_mode == "deterministic_module"
                || job
                    .eligible_verifiers
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&verifier))
        });
    }
    mcp_json(jobs)
}

async fn run_bountybench(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let result = eval_harness::BountyBench::default()
        .run(&eval_harness::bundled_fixtures())
        .expect("bundled routing fixtures pass");
    if let Err(error) = record_eval_run(&state, eval_run_from_suite(&result)).await {
        return mcp_error(error);
    }
    mcp_json(result)
}

async fn run_abusebench(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let result = eval_harness::AbuseBench::default()
        .run(&eval_harness::bundled_abuse_fixtures())
        .expect("bundled abuse fixtures pass");
    if let Err(error) = record_eval_run(&state, eval_run_from_suite(&result)).await {
        return mcp_error(error);
    }
    mcp_json(result)
}

async fn run_judgebench(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let result = eval_harness::JudgeBench::default()
        .run(&eval_harness::bundled_judge_fixtures())
        .expect("bundled judge fixtures pass");
    if let Err(error) = record_eval_run(&state, eval_run_from_suite(&result)).await {
        return mcp_error(error);
    }
    mcp_json(result)
}

async fn run_eval_loops(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let result = eval_harness::run_eval_loops()
        .await
        .expect("bundled eval loops pass");
    if let Err(error) = record_eval_run(&state, eval_run_from_loop_suite(&result)).await {
        return mcp_error(error);
    }
    mcp_json(result)
}

async fn get_eval_runs(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let runs = state.eval_runs.lock().expect("state poisoned").clone();
    mcp_json(runs)
}

async fn get_risk_policy() -> Json<serde_json::Value> {
    mcp_json(RiskPolicy::default().descriptor())
}

async fn get_live_money_readiness(
    State(state): State<SharedState>,
    Json(args): Json<LiveMoneyReadinessArgs>,
) -> Json<serde_json::Value> {
    let network = args
        .network
        .filter(|network| !network.trim().is_empty())
        .unwrap_or_else(|| "base-mainnet".to_string());
    match build_live_money_readiness_report(live_money_readiness_config(&state, &network)) {
        Ok(report) => mcp_json(report),
        Err(error) => mcp_error(error.to_string()),
    }
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
        stripe_webhook_secret_configured: env_nonempty_value("STRIPE_WEBHOOK_SECRET").is_some(),
        allow_unsigned_stripe_webhooks: env_flag("ALLOW_UNSIGNED_STRIPE_WEBHOOKS"),
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

fn env_flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false)
}

async fn list_risk_events(
    State(state): State<SharedState>,
    Json(filter): Json<RiskEventFilter>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    mcp_json(network.list_risk_events(filter))
}

async fn list_risk_reviews(State(state): State<SharedState>) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    mcp_json(network.list_risk_reviews())
}

async fn approve_risk_bounty(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<ApproveRiskBountyRequest>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let result = {
        let mut network = state.network.lock().expect("state poisoned");
        network
            .approve_risk_bounty(args)
            .map(|approval| (approval, network.ledger.entries().to_vec()))
    };
    let (approval, ledger_entries) = match result {
        Ok(result) => result,
        Err(error) => return mcp_error(error),
    };
    if let Err(error) = persist_reviewed_bounty_approval(&state, &approval, &ledger_entries).await {
        return mcp_error(error);
    }
    mcp_json(approval)
}

async fn approve_risk_payout(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<ApproveRiskPayoutRequest>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let review = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.approve_risk_payout(args) {
            Ok(review) => review,
            Err(error) => return mcp_error(error),
        }
    };
    if let Err(error) = persist_risk_review(&state, &review).await {
        return mcp_error(error);
    }
    mcp_json(review)
}

async fn reject_risk_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<RejectRiskEventRequest>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let review = {
        let mut network = state.network.lock().expect("state poisoned");
        match network.reject_risk_event(args) {
            Ok(review) => review,
            Err(error) => return mcp_error(error),
        }
    };
    if let Err(error) = persist_risk_review(&state, &review).await {
        return mcp_error(error);
    }
    mcp_json(review)
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

async fn record_eval_run(state: &SharedState, run: EvalRun) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_eval_run(&run)
            .await
            .map_err(|error| error.to_string())?;
    }
    state
        .eval_runs
        .lock()
        .expect("state poisoned")
        .insert(0, run);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn tool_descriptors_publish_machine_readable_input_schemas() {
        let descriptors = tools().await.0;

        assert!(descriptors.len() >= 20);
        for descriptor in &descriptors {
            assert!(
                descriptor.input_schema.get("type").is_some(),
                "{} missing input_schema.type",
                descriptor.name
            );
        }

        let route = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "route_blocked_goal")
            .expect("route_blocked_goal descriptor exists");
        assert_eq!(route.input_schema["type"], "object");
        assert!(route.authorization.is_none());
        assert!(route.input_schema["properties"]["privacy"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "Private"));

        let operator_tools = [
            "execute_stripe_checkout_top_up",
            "execute_stripe_connect_account",
            "execute_stripe_connect_transfer",
            "reconcile_stripe_connect_snapshot",
            "reconcile_stripe_transfer_event",
            "reconcile_stripe_checkout_webhook",
            "broadcast_base_signed_transaction",
            "approve_risk_bounty",
            "approve_risk_payout",
            "reject_risk_event",
        ];
        for tool_name in operator_tools {
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.name == tool_name)
                .unwrap_or_else(|| panic!("{tool_name} descriptor exists"));
            let authorization = descriptor
                .authorization
                .as_ref()
                .unwrap_or_else(|| panic!("{tool_name} missing operator authorization metadata"));
            assert_eq!(authorization.kind, "operator_api_token");
            assert_eq!(authorization.header, OPERATOR_TOKEN_HEADER);
            assert!(authorization.bearer);
        }

        let stripe_checkout = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_stripe_checkout_top_up")
            .expect("plan_stripe_checkout_top_up descriptor exists");
        assert!(stripe_checkout.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "amount_minor"));
        assert_eq!(
            stripe_checkout.input_schema["properties"]["organization_id"]["format"],
            "uuid"
        );

        let execute_stripe_checkout = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "execute_stripe_checkout_top_up")
            .expect("execute_stripe_checkout_top_up descriptor exists");
        assert!(execute_stripe_checkout.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "organization_id"));

        let execute_stripe_connect = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "execute_stripe_connect_account")
            .expect("execute_stripe_connect_account descriptor exists");
        assert_eq!(
            execute_stripe_connect.input_schema["properties"]["agent_id"]["format"],
            "uuid"
        );

        let plan_stripe_transfer = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_stripe_connect_transfer")
            .expect("plan_stripe_connect_transfer descriptor exists");
        assert!(plan_stripe_transfer.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "payout_intent_id"));

        let plan_github_issue = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_issue_bounty")
            .expect("plan_github_issue_bounty descriptor exists");
        assert!(plan_github_issue.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "body"));

        let plan_github_funding = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_funding_comment")
            .expect("plan_github_funding_comment descriptor exists");
        assert!(plan_github_funding.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "comment_body"));
        assert_eq!(
            plan_github_funding.input_schema["properties"]["existing_idempotency_keys"]["type"],
            "array"
        );
        assert_eq!(
            plan_github_funding.input_schema["properties"]["funding_api_base_url"]["type"][0],
            "string"
        );

        let plan_github_claim = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_claim_comment")
            .expect("plan_github_claim_comment descriptor exists");
        assert!(plan_github_claim.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "comment_body"));
        assert_eq!(
            plan_github_claim.input_schema["properties"]["progress_signal_count"]["type"],
            "integer"
        );

        let plan_github_proof = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_proof_comment")
            .expect("plan_github_proof_comment descriptor exists");
        assert_eq!(
            plan_github_proof.input_schema["properties"]["bounty_id"]["format"],
            "uuid"
        );
        let plan_github_proof_for_proof = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_proof_comment_for_proof")
            .expect("plan_github_proof_comment_for_proof descriptor exists");
        assert_eq!(
            plan_github_proof_for_proof.input_schema["properties"]["proof_id"]["format"],
            "uuid"
        );

        let list_claimable = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "list_claimable_bounties")
            .expect("list_claimable_bounties descriptor exists");
        assert!(list_claimable.input_schema["required"]
            .as_array()
            .unwrap()
            .is_empty());

        let open_pooled = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "open_pooled_bounty")
            .expect("open_pooled_bounty descriptor exists");
        assert!(open_pooled.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "target_amount_minor"));
        assert!(open_pooled.input_schema["properties"]["funding_targets"].is_null());
        assert!(
            !open_pooled.input_schema["properties"]["funding_mode"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "MixedRails")
        );

        let create_intent = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "create_funding_intent")
            .expect("create_funding_intent descriptor exists");
        assert!(create_intent.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "bounty_id"));
        assert_eq!(
            create_intent.input_schema["properties"]["rail"]["enum"],
            json!(["StripeFiat"])
        );

        let add_funding = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "add_bounty_funding")
            .expect("add_bounty_funding descriptor exists");
        assert_eq!(
            add_funding.input_schema["properties"]["bounty_id"]["format"],
            "uuid"
        );
        assert_eq!(
            add_funding.input_schema["properties"]["source_organization_id"]["format"],
            "uuid"
        );
        assert!(add_funding.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "amount_minor"));

        let get_paid_status = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "get_paid_status")
            .expect("get_paid_status descriptor exists");
        assert!(get_paid_status.input_schema["required"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(
            get_paid_status.input_schema["properties"]["agent_id"]["format"],
            "uuid"
        );

        let search_capabilities = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "search_capabilities")
            .expect("search_capabilities descriptor exists");
        assert_eq!(
            search_capabilities.input_schema["properties"]["max_price_minor"]["type"][0],
            "integer"
        );
        assert!(
            search_capabilities.input_schema["properties"]["class"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "Coding")
        );

        let broadcast_base_signed_transaction = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "broadcast_base_signed_transaction")
            .expect("broadcast_base_signed_transaction descriptor exists");
        assert!(broadcast_base_signed_transaction.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "signed_transaction"));

        let get_base_transaction_receipt = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "get_base_transaction_receipt")
            .expect("get_base_transaction_receipt descriptor exists");
        assert!(get_base_transaction_receipt.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "tx_hash"));
        assert!(get_base_transaction_receipt.input_schema["properties"]
            .get("reconcile_logs")
            .is_none());
        assert!(get_base_transaction_receipt.authorization.is_none());

        for retired in [
            "plan_base_log_query",
            "reconcile_base_escrow_event",
            "reconcile_base_evm_logs",
            "reconcile_base_rpc_logs",
            "fetch_base_rpc_logs",
            "get_base_indexer_status",
            "plan_base_funding",
            "plan_base_release",
            "plan_base_refund",
            "plan_base_dispute",
            "list_base_release_queue",
        ] {
            assert!(
                descriptors
                    .iter()
                    .all(|descriptor| descriptor.name != retired),
                "retired escrow tool {retired} must not be discoverable"
            );
        }

        for autonomous in [
            "plan_autonomous_canonical_child_terms",
            "plan_autonomous_bounty_creation",
            "plan_autonomous_bounty_authorized_creation",
            "plan_autonomous_bounty_contribution",
            "plan_autonomous_bounty_authorized_contribution",
            "fund_bounty_with_x402",
            "get_x402_relay_status",
            "plan_autonomous_bounty_claim",
            "plan_autonomous_bounty_authorized_claim",
            "plan_autonomous_bounty_submission",
            "plan_autonomous_bounty_submission_authorization",
            "list_autonomous_verification_jobs",
            "decode_autonomous_bounty_events",
            "list_autonomous_bounty_events",
            "publish_autonomous_bounty_terms",
            "get_autonomous_bounty_terms",
            "list_autonomous_bounties",
        ] {
            assert!(
                descriptors
                    .iter()
                    .any(|descriptor| descriptor.name == autonomous),
                "autonomous tool {autonomous} must be discoverable"
            );
        }

        let x402_funding = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "fund_bounty_with_x402")
            .expect("fund_bounty_with_x402 descriptor exists");
        assert!(x402_funding.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "bounty_contract"));
        assert!(x402_funding.authorization.is_none());

        let canonical_child_terms = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_autonomous_canonical_child_terms")
            .expect("canonical child terms descriptor exists");
        assert!(canonical_child_terms.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "child_acceptance_criteria"));
        assert_eq!(
            canonical_child_terms.input_schema["properties"]["child_acceptance_criteria"]
                ["minItems"],
            1
        );

        let get_live_money_readiness = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "get_live_money_readiness")
            .expect("get_live_money_readiness descriptor exists");
        assert!(
            get_live_money_readiness.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-mainnet")
        );

        let get_eval_runs = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "get_eval_runs")
            .expect("get_eval_runs descriptor exists");
        assert!(get_eval_runs.input_schema["required"]
            .as_array()
            .unwrap()
            .is_empty());

        let get_risk_policy = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "get_risk_policy")
            .expect("get_risk_policy descriptor exists");
        assert!(get_risk_policy.input_schema["required"]
            .as_array()
            .unwrap()
            .is_empty());

        let list_risk_events = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "list_risk_events")
            .expect("list_risk_events descriptor exists");
        assert!(list_risk_events.input_schema["required"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(
            list_risk_events.input_schema["properties"]["action"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "NeedsReview")
        );

        let approve_risk_bounty = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "approve_risk_bounty")
            .expect("approve_risk_bounty descriptor exists");
        assert!(approve_risk_bounty.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "risk_event_id"));

        let approve_risk_payout = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "approve_risk_payout")
            .expect("approve_risk_payout descriptor exists");
        assert!(approve_risk_payout.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "operator_id"));

        let reject_risk_event = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "reject_risk_event")
            .expect("reject_risk_event descriptor exists");
        assert!(reject_risk_event.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "note"));
    }

    #[tokio::test]
    async fn github_claim_comment_planner_rejects_claim_without_progress() {
        let response = plan_github_claim_comment(Json(PlanGitHubClaimCommentArgs {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_github_issue_body(),
            comment_body:
                "/agent-bounty claim\nI'm reviewing the codebase and will open a PR shortly."
                    .to_string(),
            contributor_login: Some("claim-bot".to_string()),
            comment_id: Some("789".to_string()),
            claim_age_minutes: Some(1),
            progress_signal_count: 0,
            active_claim_login: None,
        }))
        .await
        .0;

        let payload = &response["content"][0]["json"];
        assert_eq!(payload["ready"], false);
        assert_eq!(payload["check"]["conclusion"], "ActionRequired");
        assert!(payload["error"]
            .as_str()
            .unwrap()
            .contains("concrete progress signal"));
    }

    #[tokio::test]
    async fn github_proof_comment_for_proof_uses_stored_public_proof() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix MCP proof comments".to_string(),
                template_slug: "small-code-change".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
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
                artifact_uri: "s3://mcp/artifact.json".to_string(),
                artifact_body: artifact.to_string(),
            })
            .unwrap();
        let proof = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: app::hash_artifact(artifact),
                verifier_kind: Some(domain::VerifierKind::JsonSchema),
                rubric: None,
                evidence: None,
                approved_risk_event_id: None,
            })
            .await
            .unwrap();
        let state = test_state_with_network(network);

        let response = plan_github_proof_comment_for_proof(
            State(state.clone()),
            Json(PlanGitHubProofCommentForProofArgs {
                proof_id: proof.id,
                settlement_url: None,
            }),
        )
        .await
        .0;
        let plan = &response["content"][0]["json"];

        assert_eq!(plan["comment"]["bounty_id"], bounty.id.to_string());
        assert_eq!(
            plan["comment"]["proof_url"],
            format!("http://127.0.0.1:8080/public/proofs/{}", proof.id)
        );
        assert!(plan["comment"]["verifier_summary"]
            .as_str()
            .unwrap()
            .contains("JsonSchema"));
        assert!(plan["markdown"]
            .as_str()
            .unwrap()
            .contains("Tell your human or operator"));
        assert_eq!(plan["fingerprint"].as_str().unwrap().len(), 64);

        let paid_status = get_paid_status(
            State(state),
            Json(PaidStatusArgs {
                bounty_id: None,
                agent_id: Some(solver.id),
            }),
        )
        .await
        .0;
        let paid_status = &paid_status["content"][0]["json"];
        assert_eq!(
            paid_status["post_value_loop"]["trigger"],
            "verified_completion"
        );
        assert!(paid_status["post_value_loop"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["kind"] == "tell_your_human"));
        assert!(paid_status["post_value_loop"]["self_interest"]
            .as_str()
            .unwrap()
            .contains("more and higher-value funded bounties"));
    }

    #[tokio::test]
    async fn risk_policy_tool_exposes_settlement_limits() {
        let policy = get_risk_policy().await.0;
        let body = &policy["content"][0]["json"];

        assert_eq!(body["low_value_usdc_cap_minor"], 10_000_000);
        assert_eq!(body["low_value_usdc_cap_currency"], "usdc");
        assert_eq!(body["ai_judges_can_authorize_payment"], false);
        assert!(body["settlement_invariants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|rule| rule.as_str().unwrap().contains("indexed escrow logs")));
    }

    #[tokio::test]
    async fn live_money_readiness_tool_reports_non_secret_defaults() {
        let response = get_live_money_readiness(
            State(test_state()),
            Json(LiveMoneyReadinessArgs {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .0;
        let body = &response["content"][0]["json"];

        assert_eq!(body["network"], "Base");
        assert_eq!(body["network_chain_id"], 8_453);
        assert_eq!(body["stripe_secret_key_mode"], "unset");
        assert_eq!(
            body["stripe_payment_method_configuration_configured"],
            false
        );
        assert_eq!(body["supplied_usdc_token_matches_native"], true);
        assert_eq!(body["live_money_ready"], false);
        assert!(body["checks"].as_array().unwrap().iter().any(|check| {
            check["name"] == "Autonomous bounty factory" && check["configured"] == true
        }));
        assert!(body["checks"].as_array().unwrap().iter().any(|check| {
            check["name"]
                .as_str()
                .unwrap()
                .contains("Stripe live-money execution")
        }));
        assert!(body["checks"].as_array().unwrap().iter().any(|check| {
            check["name"]
                .as_str()
                .unwrap()
                .contains("payment-method configuration")
        }));
    }

    #[test]
    fn mainnet_planner_and_readiness_pin_the_attested_deployment() {
        let expected = autonomous_planner_addresses(8_453, None, None).unwrap();
        assert_eq!(expected.0, CANONICAL_BASE_MAINNET_BOUNTY_FACTORY);
        assert_eq!(expected.1, CANONICAL_BASE_MAINNET_BOUNTY_IMPLEMENTATION);
        assert_eq!(
            canonical_mainnet_factory(None, None).as_deref(),
            Some(CANONICAL_BASE_MAINNET_BOUNTY_FACTORY)
        );

        assert!(autonomous_planner_addresses(
            8_453,
            Some("0x1111111111111111111111111111111111111111".to_string()),
            None,
        )
        .is_err());
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

    #[tokio::test]
    async fn live_money_readiness_tool_reports_payment_method_configuration_without_id() {
        let response = get_live_money_readiness(
            State(test_state_with_stripe_payment_method_configuration(
                "pmc_paypal_enabled",
            )),
            Json(LiveMoneyReadinessArgs {
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .0;
        let body = &response["content"][0]["json"];
        let text = serde_json::to_string(body).unwrap();

        assert_eq!(body["stripe_payment_method_configuration_configured"], true);
        assert!(body["checks"].as_array().unwrap().iter().any(|check| {
            check["name"] == "Stripe Checkout payment-method configuration"
                && check["configured"] == true
        }));
        assert!(!text.contains("pmc_paypal_enabled"));
    }

    #[tokio::test]
    async fn stripe_checkout_webhook_replay_preserves_applied_event() {
        let state = test_state();
        let organization_id = Uuid::new_v4();
        let event = stripe_checkout_event("evt_mcp_paid", "cs_mcp_paid", organization_id);

        let first = reconcile_stripe_checkout_webhook(
            State(state.clone()),
            HeaderMap::new(),
            Json(event.clone()),
        )
        .await
        .0;
        let first_body = &first["content"][0]["json"];

        assert_eq!(first_body["duplicate"], false);
        assert_eq!(first_body["ledger_entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            first_body["funding_credit"]["payment_event"]["status"],
            "Applied"
        );

        let replay =
            reconcile_stripe_checkout_webhook(State(state.clone()), HeaderMap::new(), Json(event))
                .await
                .0;
        let replay_body = &replay["content"][0]["json"];

        assert_eq!(replay_body["duplicate"], true);
        assert!(replay_body["ledger_entries"].as_array().unwrap().is_empty());
        assert_eq!(
            replay_body["funding_credit"]["payment_event"]["status"],
            "IgnoredDuplicate"
        );

        let network = state.network.lock().expect("state poisoned");
        assert_eq!(network.payment_events.len(), 1);
        assert_eq!(network.ledger.entries().len(), 1);
        assert_eq!(
            network.payment_events.values().next().unwrap().status,
            domain::PaymentEventStatus::Applied
        );
    }

    #[tokio::test]
    async fn risk_events_tool_lists_review_queue() {
        let state = test_state();
        {
            let mut network = state.network.lock().expect("state poisoned");
            let result = network.post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            });
            assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        }

        let response = list_risk_events(
            State(state),
            Json(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(10),
                ..RiskEventFilter::default()
            }),
        )
        .await
        .0;
        let events = response["content"][0]["json"].as_array().unwrap();

        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["action"], "NeedsReview");
        assert!(events[0]["reasons"][0]
            .as_str()
            .unwrap()
            .contains("low-value cap"));
    }

    #[tokio::test]
    async fn risk_review_tools_approve_and_list_review_records() {
        let state = test_state();
        let risk_event_id = {
            let mut network = state.network.lock().expect("state poisoned");
            let result = network.post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            });
            assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
            network.risk_events.values().next().unwrap().id
        };

        let approval = approve_risk_bounty(
            State(state.clone()),
            HeaderMap::new(),
            Json(ApproveRiskBountyRequest {
                risk_event_id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::StripeFiatLedger,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved after manual scope review".to_string(),
            }),
        )
        .await
        .0;

        assert_eq!(
            approval["content"][0]["json"]["bounty"]["status"],
            "Unfunded"
        );
        assert_eq!(
            approval["content"][0]["json"]["review"]["outcome"],
            "Approved"
        );
        let reviews = list_risk_reviews(State(state)).await.0;
        let review_items = reviews["content"][0]["json"].as_array().unwrap();
        assert_eq!(review_items.len(), 1);
        assert_eq!(review_items[0]["outcome"], "Approved");
    }

    #[tokio::test]
    async fn reject_risk_event_tool_records_rejection_without_bounty() {
        let state = test_state();
        let risk_event_id = {
            let mut network = state.network.lock().expect("state poisoned");
            let result = network.post_funded_bounty(PostBountyRequest {
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            });
            assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
            network.risk_events.values().next().unwrap().id
        };

        let response = reject_risk_event(
            State(state.clone()),
            HeaderMap::new(),
            Json(RejectRiskEventRequest {
                risk_event_id,
                operator_id: "operator-1".to_string(),
                note: "Rejected until payer completes manual onboarding".to_string(),
            }),
        )
        .await
        .0;

        assert_eq!(response["content"][0]["json"]["outcome"], "Rejected");
        let network = state.network.lock().expect("state poisoned");
        assert!(network.bounties.is_empty());
    }

    #[tokio::test]
    async fn eval_tools_record_local_run_history() {
        let state = test_state();

        let result = run_bountybench(State(state.clone())).await.0;
        assert_eq!(
            result["content"][0]["json"]["suite"],
            "BountyBench/router-v0"
        );

        let runs = get_eval_runs(State(state)).await.0;
        let history = runs["content"][0]["json"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["suite"], "BountyBench/router-v0");
        assert_eq!(history[0]["passed"], true);
    }

    #[tokio::test]
    async fn operator_token_blocks_protected_mcp_tools_when_configured() {
        let state = test_state_with_operator_token("secret-token");

        let response = broadcast_base_signed_transaction(
            State(state.clone()),
            HeaderMap::new(),
            Json(BroadcastBaseSignedTransactionArgs {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .0;
        assert_eq!(response["error"], "operator authorization required");

        let mut headers = HeaderMap::new();
        headers.insert(OPERATOR_TOKEN_HEADER, "secret-token".parse().unwrap());
        let response = broadcast_base_signed_transaction(
            State(state),
            headers,
            Json(BroadcastBaseSignedTransactionArgs {
                signed_transaction: "0x010203".to_string(),
                request_id: Some(13),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .0;
        assert!(response["error"]
            .as_str()
            .unwrap()
            .contains("Base transaction broadcast is disabled"));
    }

    #[tokio::test]
    async fn llms_txt_exposes_agent_orientation() {
        let text = llms_txt().await;

        assert!(text.contains("# Agent Bounties"));
        assert!(text.contains("route_blocked_goal"));
        assert!(text.contains("/.well-known/agent-bounties.json"));
        assert!(text.contains("docs/agent-quickstart.md"));
        assert!(text.contains("agent-bounties/autonomous-v1"));
        assert!(text.contains("list_autonomous_bounties"));
        assert!(text.contains("BountySettled"));
        assert!(!text.contains("createEscrow"));
    }

    #[tokio::test]
    async fn stripe_checkout_tool_applies_payment_method_configuration() {
        let state = test_state_with_stripe_payment_method_configuration("pmc_paypal_enabled");

        let response = plan_stripe_checkout_top_up(
            State(state),
            Json(PlanStripeCheckoutTopUpArgs {
                organization_id: Uuid::new_v4(),
                amount_minor: 5_000,
                currency: "usd".to_string(),
                success_url: None,
                cancel_url: None,
            }),
        )
        .await
        .0;
        let body = &response["content"][0]["json"]["body"];

        assert_eq!(body["payment_method_configuration"], "pmc_paypal_enabled");
        assert!(body.get("payment_method_types").is_none());
    }

    fn test_state() -> SharedState {
        test_state_with_network(BountyNetwork::default())
    }

    fn test_state_with_network(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(network),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            operator_api_token: None,
            store: None,
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_stripe_payment_method_configuration(
        payment_method_configuration: &str,
    ) -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(BountyNetwork::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: Some(payment_method_configuration.to_string()),
            operator_api_token: None,
            store: None,
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn test_state_with_operator_token(token: &str) -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(BountyNetwork::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            stripe_payment_method_configuration: None,
            operator_api_token: Some(token.to_string()),
            store: None,
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn valid_github_issue_body() -> String {
        "### Goal\nFix the failing CI check.\n\n### Acceptance criteria\nThe test job is green and the patch explains the failure.\n\n### Template\nfix-ci-failure\n\n### Suggested amount\n10 USDC\n".to_string()
    }

    fn stripe_checkout_event(
        event_id: &str,
        session_id: &str,
        organization_id: Uuid,
    ) -> StripeWebhookEvent {
        StripeWebhookEvent {
            id: event_id.to_string(),
            event_type: "checkout.session.completed".to_string(),
            payload: json!({
                "id": session_id,
                "client_reference_id": organization_id.to_string(),
                "amount_total": 5_000,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_mcp_paid"
            }),
        }
    }

    #[test]
    fn mcp_bind_addr_prefers_explicit_config_then_host_port() {
        assert_eq!(
            service_bind_addr(Some("0.0.0.0:9001"), Some("10000"), "127.0.0.1:8090"),
            "0.0.0.0:9001"
        );
        assert_eq!(
            service_bind_addr(Some(""), Some("10000"), "127.0.0.1:8090"),
            "0.0.0.0:10000"
        );
        assert_eq!(
            service_bind_addr(None, Some(" 10002 "), "127.0.0.1:8090"),
            "0.0.0.0:10002"
        );
        assert_eq!(
            service_bind_addr(None, None, "127.0.0.1:8090"),
            "127.0.0.1:8090"
        );
    }
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

async fn persist_bounty_and_ledger(
    state: &SharedState,
    bounty: &domain::Bounty,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(bounty)
            .await
            .map_err(|error| error.to_string())?;
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
                .map_err(|error| error.to_string())?;
        }
        persist_ledger_entries(store, ledger_entries).await?;
    }
    Ok(())
}

async fn persist_reviewed_bounty_approval(
    state: &SharedState,
    approval: &ReviewedBountyApproval,
    ledger_entries: &[ledger::LedgerEntry],
) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&approval.bounty)
            .await
            .map_err(|error| error.to_string())?;
        store
            .upsert_risk_review(&approval.review)
            .await
            .map_err(|error| error.to_string())?;
        persist_ledger_entries(store, ledger_entries).await?;
    }
    Ok(())
}

async fn persist_pooled_funding_report(
    state: &SharedState,
    report: &PooledFundingReport,
) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&report.bounty)
            .await
            .map_err(|error| error.to_string())?;
        store
            .upsert_funding_contribution(&report.contribution)
            .await
            .map_err(|error| error.to_string())?;
        persist_ledger_entries(store, &report.ledger_entries).await?;
    }
    Ok(())
}

async fn persist_funding_intent_report(
    state: &SharedState,
    report: &FundingIntentReport,
) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&report.bounty)
            .await
            .map_err(|error| error.to_string())?;
        store
            .upsert_funding_intent(&report.intent)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn persist_risk_review(state: &SharedState, review: &RiskReviewRecord) -> Result<(), String> {
    if let Some(store) = &state.store {
        store
            .upsert_risk_review(review)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn persist_ledger_entries(
    store: &PostgresStore,
    entries: &[ledger::LedgerEntry],
) -> Result<(), String> {
    for entry in entries {
        store
            .insert_ledger_entry(entry)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

async fn persist_all_risk_events(state: &SharedState) -> Result<(), String> {
    let events = {
        let network = state.network.lock().expect("state poisoned");
        network.risk_events.values().cloned().collect::<Vec<_>>()
    };
    if let Some(store) = &state.store {
        for event in &events {
            store
                .upsert_risk_event(event)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn mcp_json(value: impl Serialize) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "content": [{ "type": "json", "json": value }] }))
}

fn mcp_error(error: impl ToString) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "error": error.to_string() }))
}

#[allow(dead_code)]
fn _register_agent(network: &mut BountyNetwork, handle: &str) -> domain::Agent {
    network.register_agent(RegisterAgentRequest {
        handle: handle.to_string(),
        payout_wallet: None,
    })
}
