use app::{
    ApproveRiskBountyRequest, BaseReleaseQueueRequest, BountyNetwork, ClaimBountyRequest,
    CreateHelpRequestRequest, FundQuoteRequest, PlanBaseDisputeRequest, PlanBaseRefundRequest,
    PlanBaseReleaseRequest, PostBountyRequest, RegisterAgentRequest, RegisterCapabilityRequest,
    RejectRiskEventRequest, RequestQuotesRequest, ReviewedBountyApproval, RiskEventFilter,
    SubmitResultRequest, VerifySubmissionRequest,
};
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use bounty_router::BountyRouter;
use chain_base::{
    broadcast_signed_transaction, eth_get_transaction_receipt_request,
    eth_send_raw_transaction_request, fetch_base_escrow_logs, fetch_transaction_receipt,
    rpc_logs_to_evm_logs, BaseEscrowLogQuery, BaseRpcUrlConfig, EvmLog, RpcLogSubmission,
};
use chrono::Utc;
use db::PostgresStore;
use domain::{Agent, CapabilityClass, EvalRun, HelpRequest, Money, PrivacyLevel, RiskReviewRecord};
use eval_harness::{EvalSuiteResult, LoopSuiteResult};
use github_app::{
    bounty_check_output, parse_issue_form_bounty, proof_check_output, proof_comment_fingerprint,
    GitHubProofComment,
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
    store: Option<PostgresStore>,
}

type SharedState = Arc<AppState>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolDescriptor {
    name: &'static str,
    description: &'static str,
    input_schema: Value,
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
struct PlanGitHubIssueBountyArgs {
    repository: String,
    issue_url: String,
    title: String,
    body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlanGitHubProofCommentArgs {
    bounty_id: Uuid,
    proof_url: String,
    verifier_summary: String,
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
        store,
    });
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/llms.txt", get(llms_txt))
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
            "/tools/execute_stripe_checkout_top_up",
            post(execute_stripe_checkout_top_up),
        )
        .route(
            "/tools/execute_stripe_connect_account",
            post(execute_stripe_connect_account),
        )
        .route(
            "/tools/reconcile_stripe_connect_snapshot",
            post(reconcile_stripe_connect_snapshot),
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
            "/tools/plan_github_proof_comment",
            post(plan_github_proof_comment),
        )
        .route(
            "/tools/reconcile_base_evm_logs",
            post(reconcile_base_evm_logs),
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
        .route("/tools/reject_risk_event", post(reject_risk_event))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = env::var("MCP_BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8090".to_string());
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn agent_bounties_discovery() -> Json<web_public::DiscoveryManifest> {
    let api_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let mcp_base_url =
        env::var("MCP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string());
    Json(web_public::discovery_manifest(&api_base_url, &mcp_base_url))
}

async fn llms_txt() -> String {
    let api_base_url =
        env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let mcp_base_url =
        env::var("MCP_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".to_string());
    web_public::render_llms_txt(&api_base_url, &mcp_base_url)
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
                    "evidence": nullable_object_property("Optional verifier evidence payload.")
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
                    "funding_mode": nullable_enum_property(&["Simulated", "BaseUsdcEscrow", "StripeFiatLedger"], "Optional funding mode override.")
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
        ),
        tool(
            "execute_stripe_connect_account",
            "Create a live Stripe Accounts v2 connected account when operator-enabled.",
            object_tool_schema(
                json!({ "agent_id": uuid_property("Agent UUID.") }),
                &["agent_id"],
            ),
        ),
        tool(
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
        ),
        tool(
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
            "reconcile_base_evm_logs",
            "Decode and apply raw Base escrow EVM logs with duplicate protection.",
            json!({
                "type": "array",
                "items": object_property("chain_base::EvmLog payload."),
                "description": "Array of raw EVM logs from the Base escrow contract."
            }),
        ),
        tool(
            "reconcile_base_rpc_logs",
            "Normalize provider-shaped eth_getLogs results or a full JSON-RPC response and reconcile Base escrow events.",
            json!({
                "type": ["array", "object"],
                "items": object_property("chain_base::RpcEvmLog payload with address, topics, data, transactionHash, blockNumber, and logIndex."),
                "description": "Either an array of raw eth_getLogs result objects or the full JSON-RPC response with a result array."
            }),
        ),
        tool(
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
        ),
        tool(
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
        ),
        tool(
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
            "plan_base_release",
            "Build an unsigned Base escrow release transaction for a payable bounty.",
            object_tool_schema(
                json!({
                    "bounty_id": uuid_property("Payable bounty UUID."),
                    "escrow_contract": string_property("Escrow contract EVM address."),
                    "platform_fee_wallet": string_property("Platform fee recipient EVM address.")
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
                    "reason_hash": string_property("0x-prefixed bytes32 refund reason hash.")
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
                    "dispute_hash": string_property("0x-prefixed bytes32 dispute evidence hash.")
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
                    "platform_fee_wallet": nullable_string_property("Optional platform fee recipient address.")
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
        tool(
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
        ),
        tool(
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
        ),
    ])
}

fn tool(name: &'static str, description: &'static str, input_schema: Value) -> ToolDescriptor {
    ToolDescriptor {
        name,
        description,
        input_schema,
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
        &["Simulated", "BaseUsdcEscrow", "StripeFiatLedger"],
        "Funding rail.",
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

async fn execute_stripe_checkout_top_up(
    State(state): State<SharedState>,
    Json(args): Json<PlanStripeCheckoutTopUpArgs>,
) -> Json<serde_json::Value> {
    let intent = match stripe_checkout_top_up_intent(args) {
        Ok(intent) => intent,
        Err(error) => return mcp_error(error),
    };
    execute_stripe_intent(&state, intent).await
}

async fn execute_stripe_connect_account(
    State(state): State<SharedState>,
    Json(args): Json<PlanStripeConnectAccountArgs>,
) -> Json<serde_json::Value> {
    let intent = match stripe_connect_account_intent(args) {
        Ok(intent) => intent.request,
        Err(error) => return mcp_error(error),
    };
    execute_stripe_intent(&state, intent).await
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
    Json(args): Json<ConnectAccountSnapshot>,
) -> Json<serde_json::Value> {
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

async fn reconcile_stripe_checkout_webhook(
    State(state): State<SharedState>,
    Json(args): Json<StripeWebhookEvent>,
) -> Json<serde_json::Value> {
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
        if let Err(error) = store
            .upsert_payment_event(&reconciliation.funding_credit.payment_event)
            .await
        {
            return mcp_error(error);
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

async fn plan_github_proof_comment(
    Json(args): Json<PlanGitHubProofCommentArgs>,
) -> Json<serde_json::Value> {
    let comment = GitHubProofComment {
        bounty_id: args.bounty_id,
        proof_url: args.proof_url,
        verifier_summary: args.verifier_summary,
        settlement_url: args.settlement_url,
    };
    let markdown = comment.markdown();
    let fingerprint = proof_comment_fingerprint(&comment);
    let check = proof_check_output(&comment);
    mcp_json(serde_json::json!({
        "comment": comment,
        "markdown": markdown,
        "fingerprint": fingerprint,
        "check": check
    }))
}

async fn reconcile_base_evm_logs(
    State(state): State<SharedState>,
    Json(logs): Json<Vec<EvmLog>>,
) -> Json<serde_json::Value> {
    process_base_evm_logs(&state, logs).await
}

async fn reconcile_base_rpc_logs(
    State(state): State<SharedState>,
    Json(submission): Json<RpcLogSubmission>,
) -> Json<serde_json::Value> {
    let logs = match rpc_logs_to_evm_logs(submission.into_logs()) {
        Ok(logs) => logs,
        Err(error) => return mcp_error(error),
    };
    process_base_evm_logs(&state, logs).await
}

async fn fetch_base_rpc_logs(
    State(state): State<SharedState>,
    Json(args): Json<FetchBaseRpcLogsArgs>,
) -> Json<serde_json::Value> {
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
    Json(args): Json<BroadcastBaseSignedTransactionArgs>,
) -> Json<serde_json::Value> {
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
            if let Err(error) = store.upsert_bounty(bounty).await {
                return mcp_error(error);
            }
        }
        for event in &indexed_events {
            if let Err(error) = store.upsert_base_escrow_event(event).await {
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
    Json(args): Json<ApproveRiskBountyRequest>,
) -> Json<serde_json::Value> {
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

async fn reject_risk_event(
    State(state): State<SharedState>,
    Json(args): Json<RejectRiskEventRequest>,
) -> Json<serde_json::Value> {
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
        assert!(route.input_schema["properties"]["privacy"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "Private"));

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

        let plan_github_issue = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_issue_bounty")
            .expect("plan_github_issue_bounty descriptor exists");
        assert!(plan_github_issue.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "body"));

        let plan_github_proof = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_github_proof_comment")
            .expect("plan_github_proof_comment descriptor exists");
        assert_eq!(
            plan_github_proof.input_schema["properties"]["bounty_id"]["format"],
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

        let plan_base_refund = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_refund")
            .expect("plan_base_refund descriptor exists");
        assert!(plan_base_refund.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "reason_hash"));

        let plan_base_dispute = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "plan_base_dispute")
            .expect("plan_base_dispute descriptor exists");
        assert!(plan_base_dispute.input_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "dispute_hash"));

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
            "Claimable"
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
    async fn llms_txt_exposes_agent_orientation() {
        let text = llms_txt().await;

        assert!(text.contains("# Agent Bounties"));
        assert!(text.contains("route_blocked_goal"));
        assert!(text.contains("/.well-known/agent-bounties.json"));
    }

    fn test_state() -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(BountyNetwork::default()),
            base_log_worker: Mutex::new(BaseEscrowLogWorker::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: STRIPE_API_BASE_URL.to_string(),
            store: None,
        })
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
