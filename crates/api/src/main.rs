use app::{
    hash_artifact, AddFundingContributionRequest, ApproveRiskBountyRequest,
    ApproveRiskPayoutRequest, BaseEscrowReconciliation, BaseReleaseQueueRequest, BountyNetwork,
    BountyStatusResponse, ClaimBountyRequest, CreateHelpRequestRequest, FundQuoteRequest,
    OpenPooledBountyRequest, PlanBaseDisputeRequest, PlanBaseFundingRequest, PlanBaseRefundRequest,
    PlanBaseReleaseRequest, PooledFundingReport, PostBountyRequest, QuoteSet, RegisterAgentRequest,
    RegisterCapabilityRequest, RejectRiskEventRequest, RequestQuotesRequest,
    ReviewedBountyApproval, RiskEventFilter, SubmitResultRequest, VerifySubmissionRequest,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use bounty_router::{BountyRouter, RouteDecision};
use chain_base::{
    broadcast_signed_transaction, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, fetch_base_escrow_logs, fetch_transaction_receipt,
    rpc_logs_to_evm_logs, BaseEscrowEvent, BaseEscrowLogQuery, BaseNetworkDescriptor,
    BaseRpcUrlConfig, ChainBaseError, EthGetLogsRequest, EthGetTransactionReceiptRequest,
    EthSendRawTransactionRequest, RpcLogSubmission, RpcTransactionReceipt,
};
use chrono::Utc;
use db::PostgresStore;
use domain::{
    Agent, Capability, CapabilityClass, EvalRun, HelpRequest, Money, PayoutStatus, PrivacyLevel,
    RiskEvent, RiskReviewRecord, VerificationDecision, VerifierKind,
};
use eval_harness::{
    bundled_abuse_fixtures, bundled_fixtures, bundled_judge_fixtures, run_eval_loops, AbuseBench,
    BountyBench, EvalSuiteResult, JudgeBench, LoopSuiteResult,
};
use github_app::{
    bounty_check_output, parse_issue_form_bounty, proof_comment_plan, GitHubCheckRunOutput,
    GitHubIssueFormBounty, GitHubProofComment, GitHubProofCommentPlan,
};
use ledger::Ledger;
use payments_stripe::{
    execute_stripe_request, verify_webhook_signature, CheckoutTopUpRequest, ConnectAccountSnapshot,
    StripeEventDeduper, StripeExecutionReport, StripePlanner, StripeRequestIntent,
    StripeWebhookEvent, STRIPE_API_BASE_URL,
};
use risk::{RiskPolicy, RiskPolicyDescriptor};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::sync::{Arc, Mutex};
use tower_http::cors::CorsLayer;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, Http, HttpAuthScheme, SecurityScheme};
use utoipa::openapi::Components;
use utoipa::{Modify, OpenApi, ToSchema};
use uuid::Uuid;
use worker::BaseEscrowLogWorker;

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        llms_txt,
        discovery_manifest_schema,
        agent_bounties_discovery,
        risk_policy,
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
        register_capability,
        search_capabilities,
        create_help_request,
        request_quotes,
        fund_quote,
        list_claimable_bounties,
        public_bounty_feed,
        public_capability_feed,
        reconcile_base_escrow_event,
        reconcile_base_evm_logs,
        plan_base_log_query,
        fetch_base_rpc_logs,
        reconcile_base_rpc_logs,
        broadcast_base_signed_transaction,
        get_base_transaction_receipt,
        plan_base_funding,
        list_base_release_queue,
        plan_stripe_checkout_top_up,
        plan_stripe_connect_account,
        plan_base_refund,
        plan_base_dispute,
        execute_stripe_checkout_top_up,
        execute_stripe_connect_account,
        reconcile_stripe_connect_snapshot,
        reconcile_stripe_checkout_webhook,
        plan_github_issue_bounty,
        plan_github_proof_comment,
        plan_github_proof_comment_from_proof,
        post_bounty,
        open_pooled_bounty,
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
        PlanGitHubIssueBountyRequest,
        PlanGitHubProofCommentRequest,
        PlanGitHubProofCommentFromProofRequest,
        PlanBaseLogQueryRequest,
        FetchBaseRpcLogsRequest,
        BroadcastBaseSignedTransactionRequest,
        GetBaseTransactionReceiptRequest,
        SearchCapabilitiesRequest
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
    base_log_worker: Arc<Mutex<BaseEscrowLogWorker>>,
    eval_runs: Arc<Mutex<Vec<EvalRun>>>,
    stripe_webhook_secret: Option<Vec<u8>>,
    allow_unsigned_stripe_webhooks: bool,
    stripe_secret_key: Option<String>,
    stripe_live_execution_enabled: bool,
    stripe_api_base_url: String,
    store: Option<PostgresStore>,
    base_rpc_urls: BaseRpcUrlConfig,
    base_broadcast_enabled: bool,
    operator_api_token: Option<String>,
    public_base_url: String,
    mcp_base_url: String,
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
struct PlanGitHubIssueBountyRequest {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct PlanBaseLogQueryRequest {
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct FetchBaseRpcLogsRequest {
    escrow_contract: String,
    from_block: u64,
    to_block: Option<u64>,
    request_id: Option<u64>,
    network: Option<String>,
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
    reconcile_logs: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaseRpcLogFetchReport {
    network: BaseNetworkDescriptor,
    request: EthGetLogsRequest,
    fetched_logs: usize,
    reconciliation: worker::BaseLogPipelineReport,
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
    reconciliation: Option<worker::BaseLogPipelineReport>,
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
        network: Arc::new(Mutex::new(network)),
        base_log_worker: Arc::new(Mutex::new(base_log_worker)),
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
        stripe_api_base_url: env::var("STRIPE_API_BASE_URL")
            .unwrap_or_else(|_| STRIPE_API_BASE_URL.to_string()),
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
    });
    let app = Router::new()
        .route("/health", get(health))
        .route("/llms.txt", get(llms_txt))
        .route(
            "/schemas/discovery-manifest.v1.json",
            get(discovery_manifest_schema),
        )
        .route(
            "/.well-known/agent-bounties.json",
            get(agent_bounties_discovery),
        )
        .route("/v1/discovery", get(agent_bounties_discovery))
        .route("/v1/risk/policy", get(risk_policy))
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
        .route(
            "/v1/bounties/:id/funding-contributions",
            post(add_funding_contribution),
        )
        .route("/v1/bounties/:id/claim", post(claim_bounty))
        .route("/v1/bounties/:id/submit", post(submit_result))
        .route("/v1/bounties/:id/verify", post(verify_submission))
        .route("/v1/bounties/:id", get(bounty_status))
        .route("/v1/base/escrow-events", post(reconcile_base_escrow_event))
        .route("/v1/base/evm-logs", post(reconcile_base_evm_logs))
        .route("/v1/base/rpc-logs", post(reconcile_base_rpc_logs))
        .route("/v1/base/fetch-rpc-logs", post(fetch_base_rpc_logs))
        .route(
            "/v1/base/broadcast-signed-transaction",
            post(broadcast_base_signed_transaction),
        )
        .route(
            "/v1/base/transaction-receipt",
            post(get_base_transaction_receipt),
        )
        .route("/v1/base/log-query", post(plan_base_log_query))
        .route("/v1/base/funding-plan", post(plan_base_funding))
        .route("/v1/base/release-queue", post(list_base_release_queue))
        .route("/v1/base/release-plan", post(plan_base_release))
        .route("/v1/base/refund-plan", post(plan_base_refund))
        .route("/v1/base/dispute-plan", post(plan_base_dispute))
        .route(
            "/v1/stripe/checkout-top-ups",
            post(plan_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/connect-accounts",
            post(plan_stripe_connect_account),
        )
        .route(
            "/v1/stripe/live/checkout-top-ups",
            post(execute_stripe_checkout_top_up),
        )
        .route(
            "/v1/stripe/live/connect-accounts",
            post(execute_stripe_connect_account),
        )
        .route(
            "/v1/stripe/connect-snapshots",
            post(reconcile_stripe_connect_snapshot),
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
        .route("/public/bounties/:id", get(public_bounty_detail_page))
        .route("/public/templates", get(public_template_index))
        .route("/public/templates/:slug", get(public_template_page))
        .route("/api-docs/openapi.json", get(openapi_json))
        .route("/docs", get(api_docs))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = env::var("API_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[utoipa::path(get, path = "/health", responses((status = 200, body = String)))]
async fn health() -> &'static str {
    "ok"
}

#[utoipa::path(get, path = "/llms.txt", responses((status = 200, body = String)))]
async fn llms_txt(State(state): State<SharedState>) -> String {
    web_public::render_llms_txt(&state.public_base_url, &state.mcp_base_url)
}

#[utoipa::path(get, path = "/schemas/discovery-manifest.v1.json", responses((status = 200, body = String)))]
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
<p>The discovery manifest schema is available at <a href="/schemas/discovery-manifest.v1.json">/schemas/discovery-manifest.v1.json</a>.</p>
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

#[utoipa::path(get, path = "/v1/risk/policy", responses((status = 200, body = RiskPolicyDescriptor)))]
async fn risk_policy() -> Json<RiskPolicyDescriptor> {
    Json(RiskPolicy::default().descriptor())
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
) -> Result<Json<app::AgentPayoutStatusResponse>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .agent_payout_status(id)
        .map(Json)
        .map_err(|error| match error {
            app::AppError::AgentNotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        })
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
    path = "/v1/base/escrow-events",
    responses(
        (status = 200, description = "Reconciled normalized Base escrow event"),
        (status = 400, description = "Invalid escrow event or state transition"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reconcile_base_escrow_event(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(event): Json<BaseEscrowEvent>,
) -> Result<Json<BaseEscrowReconciliation>, StatusCode> {
    require_operator(&state, &headers)?;
    let indexed_event = event.clone();
    let reconciliation = {
        let mut network = state.network.lock().expect("state poisoned");
        let reconciliation = network
            .apply_base_escrow_event(event)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        state
            .base_log_worker
            .lock()
            .expect("state poisoned")
            .ingest_indexed_event(indexed_event.clone())
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        reconciliation
    };
    if let Some(store) = &state.store {
        store
            .upsert_bounty(&reconciliation.bounty)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_base_escrow_event(&indexed_event)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store
            .upsert_escrow(&reconciliation.escrow)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    path = "/v1/base/evm-logs",
    responses(
        (status = 200, description = "Decoded and reconciled raw Base escrow EVM logs"),
        (status = 400, description = "Invalid log payload or escrow event order"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reconcile_base_evm_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(logs): Json<Vec<chain_base::EvmLog>>,
) -> Result<Json<worker::BaseLogPipelineReport>, StatusCode> {
    require_operator(&state, &headers)?;
    process_base_evm_logs(&state, logs).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/base/rpc-logs",
    responses(
        (status = 200, description = "Reconcile provider-shaped Base eth_getLogs results"),
        (status = 400, description = "Invalid provider log payload"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn reconcile_base_rpc_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(submission): Json<RpcLogSubmission>,
) -> Result<Json<worker::BaseLogPipelineReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let logs = rpc_logs_to_evm_logs(submission.into_logs()).map_err(|_| StatusCode::BAD_REQUEST)?;
    process_base_evm_logs(&state, logs).await.map(Json)
}

#[utoipa::path(
    post,
    path = "/v1/base/fetch-rpc-logs",
    request_body = FetchBaseRpcLogsRequest,
    responses(
        (status = 200, description = "Fetch Base escrow logs from configured RPC and reconcile them"),
        (status = 400, description = "Invalid fetch request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured"),
        (status = 503, description = "Requested Base RPC URL is not configured")
    ),
    security(("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn fetch_base_rpc_logs(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<FetchBaseRpcLogsRequest>,
) -> Result<Json<BaseRpcLogFetchReport>, StatusCode> {
    require_operator(&state, &headers)?;
    let query = BaseEscrowLogQuery::new(
        request.escrow_contract,
        request.from_block,
        request.to_block,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let request_id = request.request_id.unwrap_or(1);
    let network_name = request.network.as_deref().unwrap_or("base-sepolia");
    let (network, rpc_url) = state
        .base_rpc_urls
        .resolve(network_name)
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let rpc_request = query.rpc_request(request_id);
    let response = fetch_base_escrow_logs(&rpc_url, &query, request_id)
        .await
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let logs = rpc_logs_to_evm_logs(response.result).map_err(|_| StatusCode::BAD_GATEWAY)?;
    let fetched_logs = logs.len();
    let reconciliation = process_base_evm_logs(&state, logs).await?;

    Ok(Json(BaseRpcLogFetchReport {
        network,
        request: rpc_request,
        fetched_logs,
        reconciliation,
    }))
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
            "Poll POST /v1/base/transaction-receipt with reconcile_logs=true; payment state changes only after escrow logs are indexed."
                .to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/base/transaction-receipt",
    request_body = GetBaseTransactionReceiptRequest,
    responses(
        (status = 200, description = "Fetch Base transaction receipt and optionally reconcile escrow logs"),
        (status = 400, description = "Invalid receipt request"),
        (status = 401, description = "Operator token required when OPERATOR_API_TOKEN is configured and reconcile_logs=true"),
        (status = 503, description = "Requested Base RPC URL is not configured")
    ),
    security((), ("operator_api_token" = []), ("operator_bearer" = []))
)]
async fn get_base_transaction_receipt(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(request): Json<GetBaseTransactionReceiptRequest>,
) -> Result<Json<BaseTransactionReceiptReport>, StatusCode> {
    if request.reconcile_logs.unwrap_or(false) {
        require_operator(&state, &headers)?;
    }
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
            reconciliation: None,
        }));
    };

    let block_number = receipt
        .block_number()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let succeeded = receipt
        .succeeded()
        .map_err(|error| base_rpc_fetch_status(&error))?;
    let log_count = receipt.logs.len();
    let reconciliation = if request.reconcile_logs.unwrap_or(false) {
        let logs = receipt
            .logs_to_evm_logs()
            .map_err(|error| base_rpc_fetch_status(&error))?;
        Some(process_base_evm_logs(&state, logs).await?)
    } else {
        None
    };

    Ok(Json(BaseTransactionReceiptReport {
        network,
        request: rpc_request,
        receipt_found: true,
        tx_hash,
        block_number,
        succeeded,
        log_count,
        receipt: Some(receipt),
        reconciliation,
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

async fn process_base_evm_logs(
    state: &SharedState,
    logs: Vec<chain_base::EvmLog>,
) -> Result<worker::BaseLogPipelineReport, StatusCode> {
    let (report, indexed_events, bounties, escrows, settlements) = {
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
        (report, indexed_events, bounties, escrows, settlements)
    };

    if let Some(store) = &state.store {
        for bounty in &bounties {
            store
                .upsert_bounty(bounty)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for event in &indexed_events {
            store
                .upsert_base_escrow_event(event)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for escrow in &escrows {
            store
                .upsert_escrow(escrow)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        for settlement in &settlements {
            store
                .upsert_settlement(settlement)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        persist_ledger_entries(store, &report.ledger_entries).await?;
    }

    Ok(report)
}

#[utoipa::path(post, path = "/v1/base/log-query", request_body = PlanBaseLogQueryRequest, responses((status = 200, description = "Base eth_getLogs request for escrow events")))]
async fn plan_base_log_query(
    Json(request): Json<PlanBaseLogQueryRequest>,
) -> Result<Json<EthGetLogsRequest>, StatusCode> {
    BaseEscrowLogQuery::new(
        request.escrow_contract,
        request.from_block,
        request.to_block,
    )
    .map(|query| Json(query.rpc_request(request.request_id.unwrap_or(1))))
    .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/funding-plan", responses((status = 200, description = "Unsigned Base escrow funding transaction plan")))]
async fn plan_base_funding(
    State(state): State<SharedState>,
    Json(request): Json<PlanBaseFundingRequest>,
) -> Result<Json<app::BaseFundingPlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_base_funding(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/release-plan", responses((status = 200, description = "Unsigned Base escrow release transaction plan")))]
async fn plan_base_release(
    State(state): State<SharedState>,
    Json(request): Json<PlanBaseReleaseRequest>,
) -> Result<Json<app::BaseReleasePlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_base_release(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/refund-plan", responses((status = 200, description = "Unsigned Base escrow refund transaction plan")))]
async fn plan_base_refund(
    State(state): State<SharedState>,
    Json(request): Json<PlanBaseRefundRequest>,
) -> Result<Json<app::BaseRefundPlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_base_refund(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/dispute-plan", responses((status = 200, description = "Unsigned Base escrow dispute transaction plan")))]
async fn plan_base_dispute(
    State(state): State<SharedState>,
    Json(request): Json<PlanBaseDisputeRequest>,
) -> Result<Json<app::BaseDisputePlan>, StatusCode> {
    let network = state.network.lock().expect("state poisoned");
    network
        .plan_base_dispute(request)
        .map(Json)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

#[utoipa::path(post, path = "/v1/base/release-queue", responses((status = 200, description = "Pending Base release queue")))]
async fn list_base_release_queue(
    State(state): State<SharedState>,
    Json(request): Json<BaseReleaseQueueRequest>,
) -> Json<Vec<app::BaseReleaseQueueItem>> {
    let network = state.network.lock().expect("state poisoned");
    Json(network.list_base_release_queue(request))
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
    let planner = StripePlanner::new(platform_base_url.clone());
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
    let network = state.network.lock().expect("state poisoned");
    network
        .status(id)
        .map(Json)
        .map_err(|_| StatusCode::NOT_FOUND)
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
    let paid_minor = network
        .settlements
        .values()
        .flat_map(|settlement| &settlement.payout_intents)
        .filter(|intent| {
            intent.recipient_agent_id == id
                && intent.status == PayoutStatus::Paid
                && intent.amount.currency == "usdc"
        })
        .map(|intent| intent.amount.amount)
        .sum();

    Ok(Html(web_public::render_agent_profile(
        &agent,
        accepted_count,
        reputation_score,
        paid_minor,
        "usdc",
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

async fn public_bounty_detail_page(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, StatusCode> {
    let bounty = {
        let network = state.network.lock().expect("state poisoned");
        network
            .bounties
            .get(&id)
            .ok_or(StatusCode::NOT_FOUND)?
            .clone()
    };
    if bounty.privacy == PrivacyLevel::Private {
        return Err(StatusCode::NOT_FOUND);
    }

    let item = web_public::public_bounty_feed_item(&bounty, &state.public_base_url);
    Ok(Html(web_public::render_bounty_detail_page(&item)))
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
    use app::{
        AddFundingContributionRequest, ClaimBountyRequest, OpenPooledBountyRequest,
        PostBountyRequest, RegisterAgentRequest, RegisterCapabilityRequest, SubmitResultRequest,
        VerifySubmissionRequest,
    };
    use chain_base::{
        evm_address_word, evm_bytes32_word, evm_event_topic, evm_uint256_word, evm_words_data,
        EvmLog,
    };
    use domain::{
        Bounty, BountyStatus, CapabilityClass, FundingMode, Money, PaymentEventStatus, PaymentRail,
        PayoutStatus, ProofRecord, VerifierKind,
    };
    use github_app::GitHubCheckConclusion;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    type TestHmacSha256 = Hmac<Sha256>;

    #[tokio::test]
    async fn base_funding_plan_endpoint_builds_bounty_bound_transactions() {
        let mut network = BountyNetwork::default();
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fund API bounty on Base".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        let state = test_state(network);

        let funding_plan = plan_base_funding(
            State(state.clone()),
            Json(PlanBaseFundingRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                payer: "0x2222222222222222222222222222222222222222".to_string(),
                token: "0x3333333333333333333333333333333333333333".to_string(),
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(funding_plan.network.chain_id, 8_453);
        assert_eq!(funding_plan.bounty.id, bounty.id);
        assert_eq!(
            funding_plan.create.terms_hash,
            bounty.terms_hash.clone().unwrap()
        );
        assert_eq!(funding_plan.funding.network.chain_id, 8_453);
        assert_eq!(
            funding_plan.funding.approve.function,
            "approve(address,uint256)"
        );
        assert_eq!(
            funding_plan.funding.create_escrow.function,
            "createEscrow(bytes32,address,uint256,bytes32)"
        );

        let created = chain_base::simulated_created_event(
            bounty.id,
            7,
            "0x3333333333333333333333333333333333333333",
            bounty.amount.clone(),
            bounty.terms_hash.clone().unwrap(),
        );
        let _ = reconcile_base_escrow_event(State(state.clone()), HeaderMap::new(), Json(created))
            .await
            .unwrap();
        let rejected = plan_base_funding(
            State(state),
            Json(PlanBaseFundingRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                payer: "0x2222222222222222222222222222222222222222".to_string(),
                token: "0x3333333333333333333333333333333333333333".to_string(),
                network: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(rejected, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn raw_base_evm_log_endpoint_marks_bounty_paid() {
        let (network, bounty, proof) = payable_base_bounty().await;
        let state = test_state(network);
        let logs = raw_created_and_released_logs(&bounty, &proof);

        let report = reconcile_base_evm_logs(State(state.clone()), HeaderMap::new(), Json(logs))
            .await
            .unwrap()
            .0;

        assert!(report.failures.is_empty());
        assert_eq!(report.applied_events.len(), 2);
        assert_eq!(report.ledger_entries.len(), 1);
        let network = state.network.lock().expect("state poisoned");
        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
    }

    #[tokio::test]
    async fn normalized_created_event_seeds_raw_log_endpoint() {
        let (network, bounty, proof) = payable_base_bounty().await;
        let state = test_state(network);
        let created = chain_base::simulated_created_event(
            bounty.id,
            7,
            "0x3333333333333333333333333333333333333333",
            bounty.amount.clone(),
            bounty.terms_hash.clone().unwrap(),
        );

        let _ = reconcile_base_escrow_event(State(state.clone()), HeaderMap::new(), Json(created))
            .await
            .unwrap();
        let release_plan = plan_base_release(
            State(state.clone()),
            Json(PlanBaseReleaseRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                platform_fee_wallet: "0x4444444444444444444444444444444444444444".to_string(),
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(release_plan.network.chain_id, 8_453);
        assert_eq!(release_plan.release_call.onchain_escrow_id, 7);
        assert_eq!(release_plan.release_call.recipients.len(), 2);
        let release_log = raw_released_log(7, &format!("0x{}", proof.proof_hash), 11, 0);
        let report = reconcile_base_evm_logs(
            State(state.clone()),
            HeaderMap::new(),
            Json(vec![release_log]),
        )
        .await
        .unwrap()
        .0;

        assert!(report.failures.is_empty());
        assert_eq!(report.applied_events.len(), 1);
        let network = state.network.lock().expect("state poisoned");
        let status = network.status(bounty.id).unwrap();
        assert_eq!(status.bounty.status, BountyStatus::Paid);
    }

    #[tokio::test]
    async fn base_release_queue_endpoint_returns_ready_plan() {
        let (network, bounty, _proof) = payable_base_bounty().await;
        let state = test_state(network);
        let created = chain_base::simulated_created_event(
            bounty.id,
            7,
            "0x3333333333333333333333333333333333333333",
            bounty.amount.clone(),
            bounty.terms_hash.clone().unwrap(),
        );
        let _ = reconcile_base_escrow_event(State(state.clone()), HeaderMap::new(), Json(created))
            .await
            .unwrap();

        let queue = list_base_release_queue(
            State(state),
            Json(BaseReleaseQueueRequest {
                escrow_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
                platform_fee_wallet: Some("0x4444444444444444444444444444444444444444".to_string()),
                network: None,
            }),
        )
        .await
        .0;

        assert_eq!(queue.len(), 1);
        assert!(queue[0].ready);
        assert_eq!(queue[0].onchain_escrow_id, Some(7));
        assert!(queue[0].release_plan.is_some());
    }

    #[tokio::test]
    async fn agent_paid_status_endpoint_summarizes_solver_receivables() {
        let (network, _bounty, _proof) = payable_base_bounty().await;
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

        assert_eq!(response.agent.id, solver_id);
        assert_eq!(response.payouts.len(), 1);
        assert_eq!(response.payouts[0].status, PayoutStatus::Pending);
        assert_eq!(response.totals[0].currency, "usdc");
        assert_eq!(response.totals[0].pending_minor, 900_000);
        assert_eq!(response.totals[0].paid_minor, 0);
    }

    #[tokio::test]
    async fn base_refund_and_dispute_plan_endpoints_build_unsigned_transactions() {
        let mut network = BountyNetwork::default();
        let solver = network.register_agent(RegisterAgentRequest {
            handle: "solver".to_string(),
            payout_wallet: Some("0x2222222222222222222222222222222222222222".to_string()),
        });
        let bounty = network
            .post_funded_bounty(PostBountyRequest {
                title: "Dispute API bounty".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                7,
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
        network
            .submit_result(SubmitResultRequest {
                bounty_id: bounty.id,
                solver_agent_id: solver.id,
                artifact_uri: "s3://api/disputed.json".to_string(),
                artifact_body: "{\"ok\":false}".to_string(),
            })
            .unwrap();
        let state = test_state(network);

        let refund_plan = plan_base_refund(
            State(state.clone()),
            Json(PlanBaseRefundRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                reason_hash: format!("0x{}", "aa".repeat(32)),
                network: None,
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(refund_plan.network.chain_id, 84_532);
        assert_eq!(refund_plan.onchain_escrow_id, 7);
        assert_eq!(refund_plan.transaction.function, "refund(uint256,bytes32)");

        let dispute_plan = plan_base_dispute(
            State(state),
            Json(PlanBaseDisputeRequest {
                bounty_id: bounty.id,
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                dispute_hash: format!("0x{}", "bb".repeat(32)),
                network: Some("base-mainnet".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(dispute_plan.network.chain_id, 8_453);
        assert_eq!(dispute_plan.onchain_escrow_id, 7);
        assert_eq!(
            dispute_plan.transaction.function,
            "markDisputed(uint256,bytes32)"
        );
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
        let (network, bounty, proof) = payable_base_bounty().await;
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
        let (mut network, _bounty, mut proof) = payable_base_bounty().await;
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
    async fn discovery_endpoint_advertises_mcp_and_payment_entrypoints() {
        let state = test_state(BountyNetwork::default());
        let manifest = agent_bounties_discovery(State(state)).await.0;

        assert_eq!(
            manifest.endpoints.discovery,
            "http://127.0.0.1:8080/.well-known/agent-bounties.json"
        );
        assert_eq!(
            manifest.endpoints.llms_txt,
            "http://127.0.0.1:8080/llms.txt"
        );
        assert!(manifest
            .endpoints
            .agent_quickstart
            .contains("docs/agent-quickstart.md"));
        assert_eq!(
            manifest.endpoints.risk_policy,
            "http://127.0.0.1:8080/v1/risk/policy"
        );
        assert_eq!(
            manifest.endpoints.risk_events,
            "http://127.0.0.1:8080/v1/risk/events"
        );
        assert_eq!(
            manifest.endpoints.risk_reviews,
            "http://127.0.0.1:8080/v1/risk/reviews"
        );
        assert_eq!(
            manifest.endpoints.risk_bounty_approvals,
            "http://127.0.0.1:8080/v1/risk/bounty-approvals"
        );
        assert_eq!(
            manifest.endpoints.risk_payout_approvals,
            "http://127.0.0.1:8080/v1/risk/payout-approvals"
        );
        assert_eq!(
            manifest.endpoints.risk_event_rejections,
            "http://127.0.0.1:8080/v1/risk/events/{risk_event_id}/reject"
        );
        assert_eq!(
            manifest.endpoints.agent_paid_status,
            "http://127.0.0.1:8080/v1/agents/{agent_id}/paid-status"
        );
        assert_eq!(
            manifest.endpoints.github_proof_comment_from_proof_plan,
            "http://127.0.0.1:8080/v1/github/proof-comment-plan-from-proof"
        );
        assert_eq!(manifest.risk_policy.low_value_usdc_cap_minor, 10_000_000);
        assert!(manifest
            .agent_entrypoints
            .iter()
            .any(|entrypoint| entrypoint.name == "route_blocked_goal"));
        assert!(manifest
            .payment_rails
            .iter()
            .any(|rail| rail.name == "Base Sepolia USDC escrow"));
    }

    #[tokio::test]
    async fn api_docs_endpoint_points_to_openapi_json() {
        let html = api_docs().await.0;

        assert!(html.contains("/api-docs/openapi.json"));
        assert!(html.contains("/llms.txt"));
        assert!(html.contains("/schemas/discovery-manifest.v1.json"));
        assert!(html.contains("/.well-known/agent-bounties.json"));
    }

    #[tokio::test]
    async fn openapi_json_endpoint_contains_agent_router_path() {
        let document = openapi_json().await.0;
        let value = serde_json::to_value(document).unwrap();
        let paths = value["paths"].as_object().unwrap();

        assert!(paths.contains_key("/v1/route-blocked-goal"));
        assert!(paths.contains_key("/llms.txt"));
        assert!(paths.contains_key("/schemas/discovery-manifest.v1.json"));
        assert!(paths.contains_key("/v1/risk/policy"));
        assert!(paths.contains_key("/v1/risk/events"));
        assert!(paths.contains_key("/v1/risk/reviews"));
        assert!(paths.contains_key("/v1/risk/bounty-approvals"));
        assert!(paths.contains_key("/v1/risk/payout-approvals"));
        assert!(paths.contains_key("/v1/risk/events/{id}/reject"));
        assert!(paths.contains_key("/v1/agents/{id}/paid-status"));
        assert!(paths.contains_key("/v1/capabilities/search"));
        assert!(paths.contains_key("/v1/base/escrow-events"));
        assert!(paths.contains_key("/v1/base/evm-logs"));
        assert!(paths.contains_key("/v1/base/log-query"));
        assert!(paths.contains_key("/v1/base/rpc-logs"));
        assert!(paths.contains_key("/v1/base/fetch-rpc-logs"));
        assert!(paths.contains_key("/v1/base/broadcast-signed-transaction"));
        assert!(paths.contains_key("/v1/base/transaction-receipt"));
        assert!(paths.contains_key("/v1/base/refund-plan"));
        assert!(paths.contains_key("/v1/base/dispute-plan"));
        assert!(paths.contains_key("/v1/stripe/live/checkout-top-ups"));
        assert!(paths.contains_key("/v1/stripe/live/connect-accounts"));
        assert!(paths.contains_key("/v1/stripe/connect-snapshots"));
        assert!(paths.contains_key("/v1/stripe/checkout-webhooks"));
        assert!(paths.contains_key("/v1/github/issue-bounty-plan"));
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
            "/v1/base/escrow-events",
            "/v1/base/evm-logs",
            "/v1/base/rpc-logs",
            "/v1/base/fetch-rpc-logs",
            "/v1/base/broadcast-signed-transaction",
            "/v1/stripe/live/checkout-top-ups",
            "/v1/stripe/live/connect-accounts",
            "/v1/stripe/connect-snapshots",
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

        let receipt_security = paths["/v1/base/transaction-receipt"]["post"]["security"]
            .as_array()
            .unwrap();
        assert!(receipt_security
            .iter()
            .any(|requirement| requirement.as_object().unwrap().is_empty()));
        assert!(receipt_security
            .iter()
            .any(|requirement| requirement.get("operator_api_token").is_some()));
        assert!(paths["/v1/base/transaction-receipt"]["post"]["responses"]["401"].is_object());

        assert!(
            paths["/v1/stripe/checkout-webhooks"]["post"]
                .get("security")
                .is_none(),
            "Stripe checkout webhook must remain callable by Stripe without operator auth"
        );
        assert!(paths["/v1/stripe/checkout-webhooks"]["post"]["responses"]["503"].is_object());
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
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
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
        let funded = reconcile_base_escrow_event(
            State(state.clone()),
            HeaderMap::new(),
            Json(chain_base::simulated_created_event(
                approval.bounty.id,
                99,
                "0x3333333333333333333333333333333333333333",
                approval.bounty.amount.clone(),
                approval.bounty.terms_hash.clone().unwrap(),
            )),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(funded.bounty.status, BountyStatus::Claimable);
        assert_eq!(funded.ledger_entries.len(), 1);
        let reviews = list_risk_reviews(State(state)).await.0;
        assert_eq!(reviews.len(), 1);
    }

    #[tokio::test]
    async fn risk_payout_approval_endpoint_records_review_for_verification() {
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
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });
        assert!(matches!(result, Err(app::AppError::RiskNeedsReview(_))));
        let bounty_event_id = network
            .list_risk_events(RiskEventFilter {
                action: Some(domain::RiskAction::NeedsReview),
                surface: Some(domain::RiskSurface::Bounty),
                limit: Some(1),
                ..RiskEventFilter::default()
            })
            .first()
            .unwrap()
            .id;
        let approval = network
            .approve_risk_bounty(ApproveRiskBountyRequest {
                risk_event_id: bounty_event_id,
                title: "Fix deterministic payout reconciliation failure".to_string(),
                template_slug: "fix-ci-failure".to_string(),
                amount_minor: 25_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
                operator_id: "operator-1".to_string(),
                note: "Approved bounty scope".to_string(),
            })
            .unwrap();
        apply_base_funding_event(&mut network, &approval.bounty, 99);
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
        let state = test_state(network);

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
        .unwrap()
        .0;

        assert_eq!(review.outcome, domain::RiskReviewOutcome::Approved);
        assert_eq!(review.surface, domain::RiskSurface::Payout);
        assert_eq!(review.bounty_id, Some(approval.bounty.id));
        let reviews = list_risk_reviews(State(state)).await.0;
        assert_eq!(reviews.len(), 2);
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
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        apply_base_funding_event(&mut network, &public, 1);
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
    }

    #[tokio::test]
    async fn public_bounty_detail_page_exposes_public_bounty_metadata() {
        let mut network = BountyNetwork::default();
        let public = network
            .post_funded_bounty(PostBountyRequest {
                title: "Fix <public> CI".to_string(),
                template_slug: "small-code-change".to_string(),
                amount_minor: 40_000_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        apply_base_funding_event(&mut network, &public, 1);
        let state = test_state(network);

        let html = public_bounty_detail_page(State(state), Path(public.id))
            .await
            .unwrap()
            .0;

        assert!(html.contains("Fix &lt;public&gt; CI"));
        assert!(!html.contains("Fix <public> CI"));
        assert!(html.contains(r#"rel="canonical""#));
        assert!(html.contains(r#"name="agent-bounty:funding-state" content="funded""#));
        assert!(html.contains(r#"name="agent-bounty:claimability" content="claimable""#));
        assert!(html.contains(&format!("/v1/bounties/{}/claim", public.id)));
        assert!(html.contains(&format!(
            "/v1/bounties/{}/funding-contributions",
            public.id
        )));
        assert!(html.contains("/public/proofs/{proof_id}"));
    }

    #[tokio::test]
    async fn public_bounty_detail_page_returns_404_for_private_bounties() {
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

        let error = match public_bounty_detail_page(State(state), Path(private.id)).await {
            Err(error) => error,
            Ok(_) => panic!("private bounty detail page should not render"),
        };

        assert_eq!(error, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn base_log_query_endpoint_plans_eth_getlogs_request() {
        let request = plan_base_log_query(Json(PlanBaseLogQueryRequest {
            escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
            from_block: 123,
            to_block: Some(456),
            request_id: Some(9),
        }))
        .await
        .unwrap()
        .0;

        assert_eq!(request.method, "eth_getLogs");
        assert_eq!(request.id, 9);
        assert_eq!(request.params[0].from_block, "0x7b");
        assert_eq!(request.params[0].to_block, "0x1c8");
        assert_eq!(request.params[0].topics[0].len(), 4);
    }

    #[tokio::test]
    async fn base_rpc_log_endpoint_normalizes_provider_logs_and_marks_bounty_paid() {
        let (network, bounty, proof) = payable_base_bounty().await;
        let state = test_state(network);
        let logs = raw_created_and_released_logs(&bounty, &proof)
            .into_iter()
            .map(rpc_log_from_evm_log)
            .collect::<Vec<_>>();

        let report = reconcile_base_rpc_logs(
            State(state.clone()),
            HeaderMap::new(),
            Json(chain_base::RpcLogSubmission::Response(
                chain_base::EthGetLogsResponse {
                    jsonrpc: "2.0".to_string(),
                    id: 1,
                    result: logs,
                },
            )),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.applied_events.len(), 2);
        let status = bounty_status(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
    }

    #[tokio::test]
    async fn base_fetch_rpc_logs_endpoint_fetches_provider_logs_and_marks_bounty_paid() {
        let (network, bounty, proof) = payable_base_bounty().await;
        let logs = raw_created_and_released_logs(&bounty, &proof)
            .into_iter()
            .map(rpc_log_from_evm_log)
            .collect::<Vec<_>>();
        let rpc_url = spawn_rpc_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "result": logs
        }));
        let state = test_state_with_base_rpc(network, rpc_url);

        let report = fetch_base_rpc_logs(
            State(state.clone()),
            HeaderMap::new(),
            Json(FetchBaseRpcLogsRequest {
                escrow_contract: "0x1111111111111111111111111111111111111111".to_string(),
                from_block: 10,
                to_block: Some(11),
                request_id: Some(5),
                network: Some("base-sepolia".to_string()),
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(report.network.chain_id, 84_532);
        assert_eq!(report.request.method, "eth_getLogs");
        assert_eq!(report.request.params[0].from_block, "0xa");
        assert_eq!(report.fetched_logs, 2);
        assert_eq!(report.reconciliation.applied_events.len(), 2);
        let status = bounty_status(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
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
    async fn base_transaction_receipt_endpoint_reconciles_release_logs() {
        let (network, bounty, proof) = payable_base_bounty().await;
        let release_log = raw_released_log(7, &format!("0x{}", proof.proof_hash), 11, 0);
        let receipt_tx_hash = release_log.tx_hash.clone();
        let rpc_url = spawn_rpc_response(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 14,
            "result": {
                "transactionHash": receipt_tx_hash.clone(),
                "blockNumber": "0xb",
                "status": "0x1",
                "logs": [rpc_log_from_evm_log(release_log)]
            }
        }));
        let state = test_state_with_base_rpc(network, rpc_url);
        let created = chain_base::simulated_created_event(
            bounty.id,
            7,
            "0x3333333333333333333333333333333333333333",
            bounty.amount.clone(),
            bounty.terms_hash.clone().unwrap(),
        );
        let _ = reconcile_base_escrow_event(State(state.clone()), HeaderMap::new(), Json(created))
            .await
            .unwrap();

        let report = get_base_transaction_receipt(
            State(state.clone()),
            HeaderMap::new(),
            Json(GetBaseTransactionReceiptRequest {
                tx_hash: receipt_tx_hash,
                request_id: Some(14),
                network: Some("base-sepolia".to_string()),
                reconcile_logs: Some(true),
            }),
        )
        .await
        .unwrap()
        .0;

        assert!(report.receipt_found);
        assert_eq!(report.block_number, Some(11));
        assert_eq!(report.succeeded, Some(true));
        assert_eq!(report.log_count, 1);
        assert_eq!(
            report
                .reconciliation
                .as_ref()
                .expect("reconciliation")
                .applied_events
                .len(),
            1
        );
        let status = bounty_status(State(state), Path(bounty.id))
            .await
            .unwrap()
            .0;
        assert_eq!(status.bounty.status, BountyStatus::Paid);
        assert_eq!(
            status.settlements[0].payout_intents[0].status,
            PayoutStatus::Paid
        );
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
                title: "Fund shared docs work".to_string(),
                template_slug: "write-docs-for-area".to_string(),
                target_amount_minor: 1_000,
                currency: "usdc".to_string(),
                funding_mode: FundingMode::Simulated,
                privacy: PrivacyLevel::Public,
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
    async fn public_verifier_profile_summarizes_verifier_results() {
        let (network, _bounty, _proof) = payable_base_bounty().await;
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
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
        })
    }

    fn test_state_with_unsigned_stripe_webhooks(network: BountyNetwork) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: true,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
        })
    }

    fn test_state_with_stripe_webhook_secret(network: BountyNetwork, secret: &[u8]) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: Some(secret.to_vec()),
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
        })
    }

    fn test_state_with_operator_token(network: BountyNetwork, token: &str) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: Some(token.to_string()),
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
        })
    }

    fn test_state_with_base_rpc(
        network: BountyNetwork,
        base_sepolia_rpc_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
            base_rpc_urls: BaseRpcUrlConfig {
                base_sepolia: Some(base_sepolia_rpc_url),
                base_mainnet: None,
            },
            base_broadcast_enabled: true,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
        })
    }

    fn test_state_with_stripe_live(
        network: BountyNetwork,
        stripe_api_base_url: String,
    ) -> SharedState {
        Arc::new(AppState {
            network: Arc::new(Mutex::new(network)),
            base_log_worker: Arc::new(Mutex::new(BaseEscrowLogWorker::default())),
            eval_runs: Arc::new(Mutex::new(Vec::new())),
            stripe_webhook_secret: None,
            allow_unsigned_stripe_webhooks: false,
            stripe_secret_key: Some("sk_test_mock".to_string()),
            stripe_live_execution_enabled: true,
            stripe_api_base_url,
            store: None,
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            operator_api_token: None,
            public_base_url: "http://127.0.0.1:8080".to_string(),
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
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

    async fn payable_base_bounty() -> (BountyNetwork, Bounty, ProofRecord) {
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
                funding_mode: FundingMode::BaseUsdcEscrow,
                privacy: PrivacyLevel::Public,
            })
            .unwrap();
        apply_base_funding_event(&mut network, &bounty, 7);
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

    fn apply_base_funding_event(
        network: &mut BountyNetwork,
        bounty: &Bounty,
        onchain_escrow_id: u128,
    ) {
        network
            .apply_base_escrow_event(chain_base::simulated_created_event(
                bounty.id,
                onchain_escrow_id,
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                bounty.terms_hash.clone().unwrap(),
            ))
            .unwrap();
    }

    fn raw_created_and_released_logs(bounty: &Bounty, proof: &ProofRecord) -> Vec<EvmLog> {
        let terms_hash = format!("0x{}", bounty.terms_hash.clone().unwrap());
        let proof_hash = format!("0x{}", proof.proof_hash);
        vec![
            raw_created_log(
                7,
                bounty.id,
                "0x2222222222222222222222222222222222222222",
                "0x3333333333333333333333333333333333333333",
                bounty.amount.clone(),
                &terms_hash,
                10,
                0,
            ),
            raw_released_log(7, &proof_hash, 11, 0),
        ]
    }

    fn rpc_log_from_evm_log(log: EvmLog) -> chain_base::RpcEvmLog {
        chain_base::RpcEvmLog {
            address: log.address,
            topics: log.topics,
            data: log.data,
            transaction_hash: log.tx_hash,
            block_number: format!("0x{:x}", log.block_number),
            log_index: format!("0x{:x}", log.log_index),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn raw_created_log(
        escrow_id: u128,
        bounty_id: Uuid,
        payer: &str,
        token: &str,
        amount: Money,
        terms_hash: &str,
        block_number: u64,
        log_index: u64,
    ) -> EvmLog {
        EvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![
                evm_event_topic("EscrowCreated(uint256,bytes32,address,address,uint256,bytes32)"),
                evm_uint256_word(escrow_id),
                evm_bytes32_word(&bounty_bytes32(bounty_id)).unwrap(),
                evm_address_word(payer).unwrap(),
            ],
            data: evm_words_data(&[
                evm_address_word(token).unwrap(),
                evm_uint256_word(amount.amount.try_into().unwrap()),
                evm_bytes32_word(terms_hash).unwrap(),
            ])
            .unwrap(),
            tx_hash: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            block_number,
            log_index,
            occurred_at: None,
        }
    }

    fn raw_released_log(
        escrow_id: u128,
        proof_hash: &str,
        block_number: u64,
        log_index: u64,
    ) -> EvmLog {
        EvmLog {
            address: "0x1111111111111111111111111111111111111111".to_string(),
            topics: vec![
                evm_event_topic("EscrowReleased(uint256,bytes32)"),
                evm_uint256_word(escrow_id),
            ],
            data: evm_words_data(&[evm_bytes32_word(proof_hash).unwrap()]).unwrap(),
            tx_hash: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            block_number,
            log_index,
            occurred_at: None,
        }
    }

    fn bounty_bytes32(bounty_id: Uuid) -> String {
        format!("0x{}{}", "0".repeat(32), bounty_id.simple())
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

    fn github_ci_evidence() -> serde_json::Value {
        serde_json::json!({
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

    fn valid_github_issue_body() -> String {
        r#"### Goal
Fix the failing CI check.

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC
"#
        .to_string()
    }
}
