use app::{
    AddFundingContributionRequest, ApproveRiskBountyRequest, ApproveRiskPayoutRequest,
    BaseReleaseQueueRequest, BountyNetwork, ClaimBountyRequest, CreateFundingIntentRequest,
    CreateHelpRequestRequest, FundQuoteRequest, FundingIntentReport, OpenPooledBountyRequest,
    PlanBaseDisputeRequest, PlanBaseFundingRequest, PlanBaseRefundRequest, PlanBaseReleaseRequest,
    PlanStripeTransferRequest, PooledFundingReport, PostBountyRequest, RegisterAgentRequest,
    RegisterCapabilityRequest, RejectRiskEventRequest, RequestQuotesRequest,
    ReviewedBountyApproval, RiskEventFilter, SubmitResultRequest, VerifySubmissionRequest,
};
use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bounty_router::BountyRouter;
use chain_base::{
    broadcast_signed_transaction, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, fetch_base_escrow_logs, fetch_transaction_receipt,
    rpc_logs_to_evm_logs, BaseEscrowEvent, BaseEscrowLogQuery, BaseRpcUrlConfig, EvmLog,
    RpcLogSubmission,
};
use chrono::Utc;
use db::PostgresStore;
use domain::{Agent, CapabilityClass, EvalRun, HelpRequest, Money, PrivacyLevel, RiskReviewRecord};
use eval_harness::{EvalSuiteResult, LoopSuiteResult};
use github_app::{
    bounty_check_output, funding_comment_plan, parse_issue_form_bounty, proof_comment_plan,
    GitHubFundingCommentInput, GitHubProofComment,
};
use ledger::Ledger;
use payments_stripe::{
    execute_stripe_request, CheckoutTopUpRequest, ConnectAccountSnapshot, StripeEventDeduper,
    StripePlanner, StripeRequestIntent, StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use risk::RiskPolicy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::env;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;
use uuid::Uuid;
use worker::BaseEscrowLogWorker;

#[derive(Debug)]
struct AppState {
    network: Mutex<BountyNetwork>,
    base_log_worker: Mutex<BaseEscrowLogWorker>,
    eval_runs: Mutex<Vec<EvalRun>>,
    base_rpc_urls: BaseRpcUrlConfig,
    base_broadcast_enabled: bool,
    stripe_secret_key: Option<String>,
    stripe_live_execution_enabled: bool,
    stripe_api_base_url: String,
    operator_api_token: Option<String>,
    store: Option<PostgresStore>,
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
    #[serde(default)]
    existing_idempotency_keys: Vec<String>,
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
struct PlanBaseLogQueryArgs {
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FetchBaseRpcLogsArgs {
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: Option<u64>,
    network: Option<String>,
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
    reconcile_logs: Option<bool>,
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
    let (network, base_log_worker) = if let Some(store) = &store {
        (
            hydrate_network(store).await?,
            hydrate_base_log_worker(store).await?,
        )
    } else {
        (BountyNetwork::default(), BaseEscrowLogWorker::default())
    };
    let eval_runs = if let Some(store) = &store {
        store.list_eval_runs().await?
    } else {
        Vec::new()
    };
    let state: SharedState = Arc::new(AppState {
        network: Mutex::new(network),
        base_log_worker: Mutex::new(base_log_worker),
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
        operator_api_token: env::var("OPERATOR_API_TOKEN")
            .ok()
            .and_then(non_empty_secret),
        store,
    });
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/llms.txt", get(llms_txt))
        .route(
            "/schemas/discovery-manifest.v1.json",
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
            "/tools/plan_github_proof_comment",
            post(plan_github_proof_comment),
        )
        .route(
            "/tools/plan_github_proof_comment_for_proof",
            post(plan_github_proof_comment_for_proof),
        )
        .route(
            "/tools/reconcile_base_evm_logs",
            post(reconcile_base_evm_logs),
        )
        .route(
            "/tools/reconcile_base_escrow_event",
            post(reconcile_base_escrow_event),
        )
        .route(
            "/tools/reconcile_base_rpc_logs",
            post(reconcile_base_rpc_logs),
        )
        .route("/tools/fetch_base_rpc_logs", post(fetch_base_rpc_logs))
        .route(
            "/tools/broadcast_base_signed_transaction",
            post(broadcast_base_signed_transaction),
        )
        .route(
            "/tools/get_base_transaction_receipt",
            post(get_base_transaction_receipt),
        )
        .route("/tools/plan_base_log_query", post(plan_base_log_query))
        .route("/tools/plan_base_funding", post(plan_base_funding))
        .route(
            "/tools/list_base_release_queue",
            post(list_base_release_queue),
        )
        .route("/tools/plan_base_release", post(plan_base_release))
        .route("/tools/plan_base_refund", post(plan_base_refund))
        .route("/tools/plan_base_dispute", post(plan_base_dispute))
        .route("/tools/run_bountybench", get(run_bountybench))
        .route("/tools/run_abusebench", get(run_abusebench))
        .route("/tools/run_judgebench", get(run_judgebench))
        .route("/tools/run_eval_loops", get(run_eval_loops))
        .route("/tools/get_eval_runs", get(get_eval_runs))
        .route("/tools/get_risk_policy", get(get_risk_policy))
        .route("/tools/list_risk_events", post(list_risk_events))
        .route("/tools/list_risk_reviews", get(list_risk_reviews))
        .route("/tools/approve_risk_bounty", post(approve_risk_bounty))
        .route("/tools/approve_risk_payout", post(approve_risk_payout))
        .route("/tools/reject_risk_event", post(reject_risk_event))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = env::var("MCP_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8090".to_string());
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
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
                    "privacy": privacy_property(),
                    "funding_targets": array_property(
                        json!({
                            "type": "object",
                            "properties": {
                                "rail": enum_property(&["StripeFiat", "BaseUsdc"], "Real payment rail for this target."),
                                "amount_minor": integer_property("Target amount in minor units for this rail/currency partition."),
                                "currency": string_property("Currency code for this rail partition.")
                            },
                            "required": ["rail", "amount_minor", "currency"]
                        }),
                        "Required for MixedRails bounties. Each target is settled separately by rail and currency."
                    )
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
            "Create a real-rail funding intent for a bounty and return the Stripe Checkout or Base escrow next action. The intent does not confirm funding until webhook or escrow-log evidence is reconciled.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Bounty UUID."),
                    "contributor_agent_id": nullable_uuid_property("Optional contributor agent UUID."),
                    "source_organization_id": nullable_uuid_property("Stripe-funded organization UUID. Required for StripeFiat intents."),
                    "amount_minor": integer_property("Intent amount in minor units."),
                    "currency": string_property("Currency code for the rail partition."),
                    "rail": enum_property(&["StripeFiat", "BaseUsdc"], "Real payment rail for this intent."),
                    "external_reference": nullable_string_property("Optional per-bounty idempotency reference for duplicate detection."),
                    "stripe_success_url": nullable_string_property("Optional Stripe Checkout success URL."),
                    "stripe_cancel_url": nullable_string_property("Optional Stripe Checkout cancel URL."),
                    "base_escrow_contract": nullable_string_property("Base escrow contract address. Required for BaseUsdc intents."),
                    "base_payer": nullable_string_property("Base payer wallet address. Required for BaseUsdc intents."),
                    "base_token": nullable_string_property("USDC token address. Required for BaseUsdc intents."),
                    "base_network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Base network. Defaults to base-sepolia.")
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
                    "rail": enum_property(&["Simulated", "StripeFiat"], "Off-chain contribution rail. BaseUsdc funding must be indexed from escrow events through reconcile_base_escrow_event."),
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
            "Read payout status for a bounty or agent.",
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
                    "funding_mode": nullable_enum_property(&["Simulated", "BaseUsdcEscrow", "StripeFiatLedger", "MixedRails"], "Optional funding mode override.")
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
                    "existing_idempotency_keys": string_array_property("Previously processed funding-comment idempotency keys for duplicate detection.")
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
            "reconcile_base_escrow_event",
            "Apply one normalized Base escrow event after it has been indexed.",
            object_tool_schema(
                json!({
                    "id": uuid_property("Stable indexed event UUID."),
                    "log_key": string_property("Idempotency key for the indexed escrow log."),
                    "tx_hash": string_property("Base transaction hash."),
                    "block_number": integer_property("Base block number."),
                    "onchain_escrow_id": integer_property("On-chain escrow id from the contract."),
                    "bounty_id": uuid_property("Platform bounty UUID."),
                    "kind": enum_property(&["Created", "Released", "Refunded", "Disputed", "Paused"], "Escrow event kind."),
                    "status": enum_property(&["Created", "Funded", "Disputed", "Released", "Refunded"], "Escrow status after the event."),
                    "token": nullable_string_property("Token address for Created events."),
                    "amount": {
                        "type": ["object", "null"],
                        "properties": {
                            "amount": {"type": "integer"},
                            "currency": {"type": "string"}
                        }
                    },
                    "terms_hash": nullable_string_property("Terms hash for Created events."),
                    "proof_hash": nullable_string_property("Proof hash for Released events."),
                    "reason_hash": nullable_string_property("Reason hash for Refunded events."),
                    "dispute_hash": nullable_string_property("Dispute hash for Disputed events."),
                    "occurred_at": string_property("RFC3339 timestamp for the indexed event.")
                }),
                &[
                    "id",
                    "log_key",
                    "tx_hash",
                    "block_number",
                    "onchain_escrow_id",
                    "bounty_id",
                    "kind",
                    "status",
                    "occurred_at",
                ],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reconcile_base_evm_logs",
            "Decode and apply raw Base escrow EVM logs with duplicate protection.",
            json!({
                "type": "array",
                "items": object_property("chain_base::EvmLog payload."),
                "description": "Array of raw EVM logs from the Base escrow contract."
            }),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "reconcile_base_rpc_logs",
            "Normalize provider-shaped eth_getLogs results or a full JSON-RPC response and reconcile Base escrow events.",
            json!({
                "type": ["array", "object"],
                "items": object_property("chain_base::RpcEvmLog payload with address, topics, data, transactionHash, blockNumber, and logIndex."),
                "description": "Either an array of raw eth_getLogs result objects or the full JSON-RPC response with a result array."
            }),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
        ),
        operator_tool(
            "fetch_base_rpc_logs",
            "Fetch Base escrow logs from the configured RPC URL and reconcile the resulting escrow events.",
            object_tool_schema(
                json!({
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "from_block": integer_property("Inclusive starting block number."),
                    "to_block": nullable_integer_property("Optional inclusive ending block number; null uses latest."),
                    "request_id": nullable_integer_property("Optional JSON-RPC request id."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["escrow_contract", "from_block"],
            ),
            OPERATOR_TOKEN_REQUIRED_WHEN_CONFIGURED,
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
        operator_tool(
            "get_base_transaction_receipt",
            "Fetch a Base transaction receipt and optionally reconcile escrow logs from that receipt.",
            object_tool_schema(
                json!({
                    "tx_hash": string_property("0x-prefixed transaction hash."),
                    "request_id": nullable_integer_property("Optional JSON-RPC request id."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia."),
                    "reconcile_logs": nullable_boolean_property("When true, normalize receipt logs and run the Base escrow indexer.")
                }),
                &["tx_hash"],
            ),
            "OPERATOR_API_TOKEN is configured and reconcile_logs=true.",
        ),
        tool(
            "plan_base_log_query",
            "Build an eth_getLogs JSON-RPC request for Base escrow events.",
            object_tool_schema(
                json!({
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "from_block": integer_property("Inclusive starting block number."),
                    "to_block": nullable_integer_property("Optional inclusive ending block number; null uses latest."),
                    "request_id": nullable_integer_property("Optional JSON-RPC request id.")
                }),
                &["escrow_contract", "from_block"],
            ),
        ),
        tool(
            "plan_base_funding",
            "Build unsigned Base USDC approve and createEscrow transactions for a posted bounty.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Base USDC bounty UUID to fund on-chain."),
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "payer": string_property("Payer EVM address that will approve USDC and create escrow."),
                    "token": string_property("USDC token EVM address on the selected Base network."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["bounty_id", "escrow_contract", "payer", "token"],
            ),
        ),
        tool(
            "plan_base_release",
            "Build an unsigned Base escrow release transaction for a payable bounty.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Payable bounty UUID."),
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "platform_fee_wallet": string_property("Platform fee recipient EVM address."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["bounty_id", "escrow_contract", "platform_fee_wallet"],
            ),
        ),
        tool(
            "plan_base_refund",
            "Build an unsigned Base escrow refund transaction for a refundable bounty.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Funded, claimed, submitted, disputed, or refunding bounty UUID."),
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "reason_hash": string_property("0x-prefixed bytes32 refund reason hash."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["bounty_id", "escrow_contract", "reason_hash"],
            ),
        ),
        tool(
            "plan_base_dispute",
            "Build an unsigned Base escrow dispute marker transaction for a submitted or verifying bounty.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Submitted or verifying bounty UUID."),
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "dispute_hash": string_property("0x-prefixed bytes32 dispute evidence hash."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network; defaults to base-sepolia.")
                }),
                &["bounty_id", "escrow_contract", "dispute_hash"],
            ),
        ),
        tool(
            "list_base_release_queue",
            "List payable Base settlements and release-planning readiness errors.",
            object_tool_schema(
                json!({
                    "escrow_contract": nullable_string_property("Optional escrow contract address for release planning."),
                    "platform_fee_wallet": nullable_string_property("Optional platform fee recipient address."),
                    "network": nullable_enum_property(&["base-sepolia", "base-mainnet"], "Optional Base network for embedded release plans; defaults to base-sepolia.")
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
        &[
            "Simulated",
            "BaseUsdcEscrow",
            "StripeFiatLedger",
            "MixedRails",
        ],
        "Funding mode. MixedRails requires funding_targets.",
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
    let network = state.network.lock().expect("state poisoned");
    match network.status(args.bounty_id) {
        Ok(status) => mcp_json(status),
        Err(error) => mcp_error(error),
    }
}

async fn get_paid_status(
    State(state): State<SharedState>,
    Json(args): Json<PaidStatusArgs>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    match (args.bounty_id, args.agent_id) {
        (Some(bounty_id), None) => match network.status(bounty_id) {
            Ok(status) => mcp_json(serde_json::json!({
                "scope": "bounty",
                "bounty_id": bounty_id,
                "bounty_status": status.bounty.status,
                "settlements": status.settlements,
                "template_signals": status.template_signals,
                "risk_events": status.risk_events
            })),
            Err(error) => mcp_error(error),
        },
        (None, Some(agent_id)) => match network.agent_payout_status(agent_id) {
            Ok(status) => mcp_json(serde_json::json!({
                "scope": "agent",
                "agent_id": agent_id,
                "agent": status.agent,
                "payouts": status.payouts,
                "totals": status.totals,
                "reputation_events": status.reputation_events
            })),
            Err(error) => mcp_error(error),
        },
        (None, None) => mcp_error("get_paid_status requires bounty_id or agent_id"),
        (Some(_), Some(_)) => {
            mcp_error("get_paid_status accepts either bounty_id or agent_id, not both")
        }
    }
}

async fn plan_stripe_checkout_top_up(
    Json(args): Json<PlanStripeCheckoutTopUpArgs>,
) -> Json<serde_json::Value> {
    match stripe_checkout_top_up_intent(args) {
        Ok(intent) => mcp_json(intent),
        Err(error) => mcp_error(error),
    }
}

fn stripe_checkout_top_up_intent(
    args: PlanStripeCheckoutTopUpArgs,
) -> Result<StripeRequestIntent, Box<dyn std::error::Error + Send + Sync>> {
    let platform_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let planner = StripePlanner::new(platform_base_url.clone());
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
    let intent = match stripe_checkout_top_up_intent(args) {
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
        existing_idempotency_keys: args.existing_idempotency_keys,
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

async fn reconcile_base_escrow_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(event): Json<BaseEscrowEvent>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let indexed_event = event.clone();
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        let reconciliation = match network.apply_base_escrow_event(event) {
            Ok(reconciliation) => reconciliation,
            Err(error) => return mcp_error(error),
        };
        if let Err(error) = state
            .base_log_worker
            .lock()
            .expect("state poisoned")
            .ingest_indexed_event(indexed_event.clone())
        {
            return mcp_error(error);
        }
        reconciliation
    };
    if let Some(store) = &state.store {
        if let Err(error) = store.upsert_bounty(&reconciliation.bounty).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_base_escrow_event(&indexed_event).await {
            return mcp_error(error);
        }
        if let Err(error) = store.upsert_escrow(&reconciliation.escrow).await {
            return mcp_error(error);
        }
        for intent in &reconciliation.funding_intents {
            if let Err(error) = store.upsert_funding_intent(intent).await {
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

async fn reconcile_base_evm_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(logs): Json<Vec<EvmLog>>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    process_base_evm_logs(&state, logs).await
}

async fn reconcile_base_rpc_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(submission): Json<RpcLogSubmission>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let logs = match rpc_logs_to_evm_logs(submission.into_logs()) {
        Ok(logs) => logs,
        Err(error) => return mcp_error(error),
    };
    process_base_evm_logs(&state, logs).await
}

async fn fetch_base_rpc_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<FetchBaseRpcLogsArgs>,
) -> Json<serde_json::Value> {
    if let Err(error) = require_operator(&state, &headers) {
        return error;
    }
    let query = match BaseEscrowLogQuery::new(args.escrow_contract, args.from_block, args.to_block)
    {
        Ok(query) => query,
        Err(error) => return mcp_error(error),
    };
    let request_id = args.request_id.unwrap_or(1);
    let network_name = args.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = match state.base_rpc_urls.resolve(network_name) {
        Ok(resolved) => resolved,
        Err(error) => return mcp_error(error),
    };
    let request = query.rpc_request(request_id);
    let response = match fetch_base_escrow_logs(&rpc_url, &query, request_id).await {
        Ok(response) => response,
        Err(error) => return mcp_error(error),
    };
    let logs = match rpc_logs_to_evm_logs(response.result) {
        Ok(logs) => logs,
        Err(error) => return mcp_error(error),
    };
    let fetched_logs = logs.len();
    let reconciliation = process_base_evm_logs(&state, logs).await.0;
    if reconciliation.get("error").is_some() {
        return Json(reconciliation);
    }

    mcp_json(serde_json::json!({
        "network": network,
        "request": request,
        "fetched_logs": fetched_logs,
        "reconciliation": reconciliation["content"][0]["json"].clone()
    }))
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
        "next_step": "Poll get_base_transaction_receipt with reconcile_logs=true; payment state changes only after escrow logs are indexed."
    }))
}

async fn get_base_transaction_receipt(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(args): Json<GetBaseTransactionReceiptArgs>,
) -> Json<serde_json::Value> {
    if args.reconcile_logs.unwrap_or(false) {
        if let Err(error) = require_operator(&state, &headers) {
            return error;
        }
    }
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
            "receipt": null,
            "reconciliation": null
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
    let reconciliation = if args.reconcile_logs.unwrap_or(false) {
        let logs = match receipt.logs_to_evm_logs() {
            Ok(logs) => logs,
            Err(error) => return mcp_error(error),
        };
        let reconciliation = process_base_evm_logs(&state, logs).await.0;
        if reconciliation.get("error").is_some() {
            return Json(reconciliation);
        }
        Some(reconciliation["content"][0]["json"].clone())
    } else {
        None
    };

    mcp_json(serde_json::json!({
        "network": network,
        "request": request,
        "receipt_found": true,
        "tx_hash": tx_hash,
        "block_number": block_number,
        "succeeded": succeeded,
        "log_count": log_count,
        "receipt": receipt,
        "reconciliation": reconciliation
    }))
}

async fn process_base_evm_logs(state: &SharedState, logs: Vec<EvmLog>) -> Json<serde_json::Value> {
    let (report, indexed_events, bounties, funding_intents, escrows, settlements) = {
        let mut network = state.network.lock().expect("state poisoned");
        let mut worker = state.base_log_worker.lock().expect("state poisoned");
        let report = worker.process_logs(logs, &mut network);
        let applied_event_ids = report
            .applied_events
            .iter()
            .map(|event| event.event_id)
            .collect::<HashSet<_>>();
        let indexed_events = worker
            .indexed_events()
            .iter()
            .filter(|event| applied_event_ids.contains(&event.id))
            .cloned()
            .collect::<Vec<_>>();
        let bounty_ids = report
            .applied_events
            .iter()
            .map(|event| event.bounty_id)
            .collect::<HashSet<_>>();
        let bounties = bounty_ids
            .iter()
            .filter_map(|id| network.bounties.get(id).cloned())
            .collect::<Vec<_>>();
        let funding_intents = network
            .funding_intents
            .values()
            .filter(|intent| bounty_ids.contains(&intent.bounty_id))
            .cloned()
            .collect::<Vec<_>>();
        let escrows = network
            .escrows
            .values()
            .filter(|escrow| bounty_ids.contains(&escrow.bounty_id))
            .cloned()
            .collect::<Vec<_>>();
        let settlements = network
            .settlements
            .values()
            .filter(|settlement| bounty_ids.contains(&settlement.bounty_id))
            .cloned()
            .collect::<Vec<_>>();
        (
            report,
            indexed_events,
            bounties,
            funding_intents,
            escrows,
            settlements,
        )
    };
    if let Some(store) = &state.store {
        for bounty in &bounties {
            if let Err(error) = store.upsert_bounty(bounty).await {
                return mcp_error(error);
            }
        }
        for event in &indexed_events {
            if let Err(error) = store.upsert_base_escrow_event(event).await {
                return mcp_error(error);
            }
        }
        for intent in &funding_intents {
            if let Err(error) = store.upsert_funding_intent(intent).await {
                return mcp_error(error);
            }
        }
        for escrow in &escrows {
            if let Err(error) = store.upsert_escrow(escrow).await {
                return mcp_error(error);
            }
        }
        for settlement in &settlements {
            if let Err(error) = store.upsert_settlement(settlement).await {
                return mcp_error(error);
            }
        }
        if let Err(error) = persist_ledger_entries(store, &report.ledger_entries).await {
            return mcp_error(error);
        }
    }
    mcp_json(report)
}

async fn plan_base_log_query(Json(args): Json<PlanBaseLogQueryArgs>) -> Json<serde_json::Value> {
    match BaseEscrowLogQuery::new(args.escrow_contract, args.from_block, args.to_block) {
        Ok(query) => mcp_json(query.rpc_request(args.request_id.unwrap_or(1))),
        Err(error) => mcp_error(error),
    }
}

async fn list_base_release_queue(
    State(state): State<SharedState>,
    Json(args): Json<BaseReleaseQueueRequest>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    mcp_json(network.list_base_release_queue(args))
}

async fn plan_base_funding(
    State(state): State<SharedState>,
    Json(args): Json<PlanBaseFundingRequest>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    match network.plan_base_funding(args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_base_release(
    State(state): State<SharedState>,
    Json(args): Json<PlanBaseReleaseRequest>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    match network.plan_base_release(args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_base_refund(
    State(state): State<SharedState>,
    Json(args): Json<PlanBaseRefundRequest>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    match network.plan_base_refund(args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
}

async fn plan_base_dispute(
    State(state): State<SharedState>,
    Json(args): Json<PlanBaseDisputeRequest>,
) -> Json<serde_json::Value> {
    let network = state.network.lock().expect("state poisoned");
    match network.plan_base_dispute(args) {
        Ok(plan) => mcp_json(plan),
        Err(error) => mcp_error(error),
    }
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
mod tests {
    use super::*;

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
            "reconcile_base_evm_logs",
            "reconcile_base_rpc_logs",
            "fetch_base_rpc_logs",
            "broadcast_base_signed_transaction",
            "get_base_transaction_receipt",
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
        assert_eq!(
            open_pooled.input_schema["properties"]["funding_targets"]["type"],
            "array"
        );
        assert!(
            open_pooled.input_schema["properties"]["funding_mode"]["enum"]
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
        assert!(create_intent.input_schema["properties"]["rail"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "BaseUsdc"));

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

        let plan_base_log_query = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_log_query")
            .expect("plan_base_log_query descriptor exists");
        assert!(plan_base_log_query.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "from_block"));
        assert_eq!(
            plan_base_log_query.input_schema["properties"]["to_block"]["type"][0],
            "integer"
        );

        let reconcile_base_rpc_logs = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "reconcile_base_rpc_logs")
            .expect("reconcile_base_rpc_logs descriptor exists");
        assert!(reconcile_base_rpc_logs.input_schema["type"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "object"));

        let fetch_base_rpc_logs = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "fetch_base_rpc_logs")
            .expect("fetch_base_rpc_logs descriptor exists");
        assert!(fetch_base_rpc_logs.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "escrow_contract"));
        assert!(
            fetch_base_rpc_logs.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-sepolia")
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
        assert_eq!(
            get_base_transaction_receipt.input_schema["properties"]["reconcile_logs"]["type"][0],
            "boolean"
        );
        assert!(get_base_transaction_receipt
            .authorization
            .as_ref()
            .unwrap()
            .required_when
            .contains("reconcile_logs=true"));

        let plan_base_funding = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_funding")
            .expect("plan_base_funding descriptor exists");
        assert!(plan_base_funding.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "payer"));
        assert!(
            plan_base_funding.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-mainnet")
        );

        let plan_base_refund = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_refund")
            .expect("plan_base_refund descriptor exists");
        assert!(plan_base_refund.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "reason_hash"));
        assert!(
            plan_base_refund.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-mainnet")
        );

        let plan_base_dispute = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_dispute")
            .expect("plan_base_dispute descriptor exists");
        assert!(plan_base_dispute.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "dispute_hash"));
        assert!(
            plan_base_dispute.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-mainnet")
        );

        let plan_base_release = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_release")
            .expect("plan_base_release descriptor exists");
        assert!(
            plan_base_release.input_schema["properties"]["network"]["enum"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "base-mainnet")
        );

        let list_base_release_queue = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "list_base_release_queue")
            .expect("list_base_release_queue descriptor exists");
        assert!(
            list_base_release_queue.input_schema["properties"]["network"]["enum"]
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
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                77,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
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
            State(state),
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
        assert_eq!(plan["fingerprint"].as_str().unwrap().len(), 64);
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
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
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
        let bounty = &approval["content"][0]["json"]["bounty"];
        let funded = reconcile_base_escrow_event(
            State(state.clone()),
            HeaderMap::new(),
            Json(chain_base::simulated_created_event(
                Uuid::parse_str(bounty["id"].as_str().unwrap()).unwrap(),
                99,
                "0x3333333333333333333333333333333333333333",
                domain::Money::new(25_000_000, "usdc").unwrap(),
                bounty["terms_hash"].as_str().unwrap(),
            )),
        )
        .await
        .0;
        assert_eq!(
            funded["content"][0]["json"]["bounty"]["status"],
            "Claimable"
        );

        let reviews = list_risk_reviews(State(state)).await.0;
        let review_items = reviews["content"][0]["json"].as_array().unwrap();
        assert_eq!(review_items.len(), 1);
        assert_eq!(review_items[0]["outcome"], "Approved");
    }

    #[tokio::test]
    async fn payout_review_tool_approves_verification_risk_event() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let result = network.post_funded_bounty(PostBountyRequest {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount_minor: 25_000_000,
            currency: "usdc".to_string(),
            funding_mode: domain::FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let bounty_event_id = network.risk_events.values().next().unwrap().id;
        let approval = network
            .approve_risk_bounty(ApproveRiskBountyRequest {
                risk_event_id: bounty_event_id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved bounty scope".to_string(),
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                approval.bounty.id,
                99,
                "0x3333333333333333333333333333333333333333",
                approval.bounty.amount.clone(),
                approval.bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
        network
            .claim_bounty(ClaimBountyRequest {
                bounty_id: approval.bounty.id,
                solver_agent_id: solver.id,
            })
            .unwrap();
        let submission = network
            .submit_result(SubmitResultRequest {
                bounty_id: approval.bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "https://github.com/example/repo/pull/1".to_string(),
                artifact_body: "{\"check\":\"green\"}".to_string(),
            })
            .unwrap();
        let result = network
            .verify_submission(VerifySubmissionRequest {
                bounty_id: approval.bounty.id,
                submission_id: submission.id,
                expected_artifact_digest: "not-used-by-github-ci".to_string(),
                verifier_kind: None,
                rubric: None,
                evidence: Some(github_ci_evidence()),
                approved_risk_event_id: None,
            })
            .await;
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let payout_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Payout),
                bounty_id: Some(approval.bounty.id),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let state = test_state_with_network(network);

        let review = approve_risk_payout(
            State(state.clone()),
            HeaderMap::new(),
            Json(ApproveRiskPayoutRequest {
                risk_event_id: payout_event_id,
                operator_id: "operator-1".to_string(),
                note: "Approved payout after verifier scope review".to_string(),
            }),
        )
        .await
        .0;

        assert_eq!(review["content"][0]["json"]["surface"], "Payout");
        assert_eq!(review["content"][0]["json"]["outcome"], "Approved");
        let reviews = list_risk_reviews(State(state)).await.0;
        let review_items = reviews["content"][0]["json"].as_array().unwrap();
        assert_eq!(review_items.len(), 2);
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
    }

    fn test_state() -> SharedState {
        test_state_with_network(BountyNetwork::default())
    }

    fn test_state_with_network(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(network),
            base_log_worker: Mutex::new(BaseEscrowLogWorker::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            operator_api_token: None,
            store: None,
        })
    }

    fn test_state_with_operator_token(token: &str) -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(BountyNetwork::default()),
            base_log_worker: Mutex::new(BaseEscrowLogWorker::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            operator_api_token: Some(token.to_string()),
            store: None,
        })
    }

    fn github_ci_evidence() -> serde_json::Value {
        json!({
            "repository": "example/repo",
            "pull_request_url": "https://github.com/example/repo/pull/1",
            "commit_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "check_run": {
                "id": 123456789_u64,
                "name": "full-check",
                "status": "completed",
                "conclusion": "success",
                "head_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "html_url": "https://github.com/example/repo/actions/runs/123456789",
                "repository": {
                    "full_name": "example/repo"
                }
            }
        })
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
}

async fn hydrate_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: store
            .list_agents()
            .await?
            .into_iter()
            .map(|agent| (agent.id, agent))
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

async fn hydrate_base_log_worker(store: &PostgresStore) -> anyhow::Result<BaseEscrowLogWorker> {
    Ok(BaseEscrowLogWorker::from_indexed_events(
        "usdc",
        store.list_base_escrow_events().await?,
    )?)
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
