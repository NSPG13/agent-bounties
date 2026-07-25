use super::{
    agent_native_claim, compile_objective_with_cloud_agent, fund_bounty_with_x402, get_paid_status,
    get_x402_relay_status, list_autonomous_bounties, list_autonomous_verification_jobs,
    list_opportunities, list_unfunded_bounties, plan_autonomous_attestation_settlement,
    plan_autonomous_bounty_claim, plan_autonomous_module_settlement,
    plan_autonomous_verification_attestation, prepare_agent_to_earn,
    prepare_autonomous_bounty_submission, proxy_hosted_json, public_base_url_from_env,
    publish_autonomous_submission_evidence, publish_unfunded_bounty,
    submit_unfunded_bounty_solution, tools, AgentNativeClaimArgs, AutonomousBountyFeedArgs,
    AutonomousVerificationJobsArgs, CompileObjectiveWithCloudAgentArgs, GetX402RelayStatusArgs,
    ListUnfundedBountiesArgs, OpportunityListArgs, PaidStatusArgs,
    PlanAutonomousAttestationSettlementArgs, PlanAutonomousBountyClaimArgs,
    PlanAutonomousModuleSettlementArgs, PlanAutonomousVerificationAttestationArgs,
    PrepareAgentToEarnInput, PrepareAutonomousBountySubmissionArgs, PrepareBountyPostArgs,
    PublishAutonomousSubmissionEvidenceArgs, PublishUnfundedBountyArgs, SharedState,
    SubmitUnfundedBountySolutionArgs, ToolDescriptor, X402BountyFundingArgs,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::env;
use url::Url;
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const CHATGPT_SANDBOX_ENV: &str = "CHATGPT_APP_SANDBOX_MODE";
const POST_WIDGET_URI: &str = "ui://agent-bounties/post-bounty-v1.html";
const FEED_WIDGET_URI: &str = "ui://agent-bounties/live-feed-v4.html";
const POST_PAGE_URL: &str = "https://agentbounties.app/post.html";
const FEED_WIDGET_HTML: &str = include_str!("../../../site/chatgpt-bounty-feed-widget.html");
const FEED_CARD_ART: &[u8] = include_bytes!("../../../site/assets/bounty-quest-agent-v1.webp");
const CHATGPT_TOOL_NAMES: &[&str] = &[
    "get_bounty_feed",
    "render_bounty_feed",
    "prepare_bounty_action",
    "get_bounty_action_status",
    "compile_objective_with_cloud_agent",
    "list_bounty_comments",
    "add_bounty_comment",
    "create_share_bundle",
];

#[derive(Debug, Clone, Deserialize)]
struct ChatgptFeedArgs {
    network: Option<String>,
    view: Option<String>,
    source_type: Option<String>,
    work_state: Option<String>,
    payment_state: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct RenderFeedArgs {
    #[serde(default)]
    opportunity_ids: Vec<String>,
    #[serde(flatten)]
    feed: ChatgptFeedArgs,
}

#[derive(Debug, Clone, Deserialize)]
struct ListCommentsArgs {
    bounty_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AddCommentArgs {
    bounty_id: String,
    body: String,
    author: Option<String>,
    comment_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ShareBundleArgs {
    bounty_id: String,
    title: String,
    stage: String,
    bounty_url: String,
    status: String,
    reward: Option<String>,
    payment_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PrepareBountyActionArgs {
    idempotency_key: String,
    action: String,
    network: Option<String>,
    opportunity_id: Option<String>,
    bounty_contract: Option<String>,
    bounty_id: Option<String>,
    actor_wallet: Option<String>,
    amount_base_units: Option<u64>,
    #[serde(default)]
    details: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct GetBountyActionStatusArgs {
    intent_id: String,
}

fn custom_tool_descriptors() -> Vec<ToolDescriptor> {
    vec![
        ToolDescriptor {
            name: "get_bounty_feed",
            description: "Use this when the user or mounted feed needs fresh structured bounty data without rendering another widget. It is a read-only projection; each item includes its authoritative next action and evidence boundary. Use render_bounty_feed to display the interactive feed.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "network": {"type": ["string", "null"], "enum": ["base-mainnet", "base-sepolia", null]},
                    "view": {"type": ["string", "null"], "enum": ["recent", "engineering", "creative", "urgent", "seeking_funding", "ready_to_earn", null]},
                    "source_type": {"type": ["string", "null"], "enum": ["unfunded_offchain", "legacy_bounty", "canonical_base", null]},
                    "work_state": {"type": ["string", "null"], "enum": ["open", "claimable", "in_progress", "submitted", "completed", null]},
                    "payment_state": {"type": ["string", "null"], "enum": ["none", "seeking_funding", "escrowed", "paid", null]},
                    "limit": {"type": ["integer", "null"], "minimum": 1, "maximum": 300}
                },
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "render_bounty_feed",
            description: "Use this when the user wants the interactive Agent Bounties feed rendered inside ChatGPT. For model-selected results, call get_bounty_feed first and pass the chosen opportunity_ids; otherwise omit them to render the current filtered feed.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "opportunity_ids": {
                        "type": "array",
                        "maxItems": 30,
                        "uniqueItems": true,
                        "items": {"type": "string", "minLength": 1, "maxLength": 200},
                        "description": "Optional opportunity identifiers selected from get_bounty_feed."
                    },
                    "network": {"type": ["string", "null"], "enum": ["base-mainnet", "base-sepolia", null]},
                    "view": {"type": ["string", "null"], "enum": ["recent", "engineering", "creative", "urgent", "seeking_funding", "ready_to_earn", null]},
                    "source_type": {"type": ["string", "null"], "enum": ["unfunded_offchain", "legacy_bounty", "canonical_base", null]},
                    "work_state": {"type": ["string", "null"], "enum": ["open", "claimable", "in_progress", "submitted", "completed", null]},
                    "payment_state": {"type": ["string", "null"], "enum": ["none", "seeking_funding", "escrowed", "paid", null]},
                    "limit": {"type": ["integer", "null"], "minimum": 1, "maximum": 30}
                },
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "list_bounty_comments",
            description: "Use this when the user wants to read durable public in-chat comments on a bounty card. Comments are separate from canonical funding, verification, settlement, and payment evidence.",
            input_schema: json!({"type": "object", "properties": {"bounty_id": {"type": "string", "minLength": 1, "maxLength": 200}}, "required": ["bounty_id"], "additionalProperties": false}),
            authorization: None,
        },
        ToolDescriptor {
            name: "add_bounty_comment",
            description: "Use this when the user explicitly wants to comment on a bounty in the live in-chat feed. This publishes a bounded durable public comment and returns a share-ready follow-up; it never funds, claims, verifies, settles, or proves payment.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "bounty_id": {"type": "string", "minLength": 1, "maxLength": 200},
                    "body": {"type": "string", "minLength": 1, "maxLength": 500},
                    "author": {"type": ["string", "null"], "maxLength": 60},
                    "comment_id": {"type": ["string", "null"], "format": "uuid", "maxLength": 128}
                },
                "required": ["bounty_id", "body"],
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "create_share_bundle",
            description: "Use this after every meaningful bounty step to create a concise social caption, hashtags, and safe share intents. It is a pure formatter and never claims that a plan, hash, or hosted record is canonical evidence.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "bounty_id": {"type": "string", "minLength": 1, "maxLength": 200},
                    "title": {"type": "string", "minLength": 1, "maxLength": 200},
                    "stage": {"type": "string", "minLength": 1, "maxLength": 40, "description": "Short factual stage label such as terms prepared, funding requested, competing, completed, verified, or commented."},
                    "bounty_url": {"type": "string", "minLength": 1, "maxLength": 12000},
                    "status": {"type": "string", "minLength": 1, "maxLength": 80},
                    "reward": {"type": ["string", "null"], "maxLength": 80},
                    "payment_state": {"type": ["string", "null"], "maxLength": 80}
                },
                "required": ["bounty_id", "title", "stage", "bounty_url", "status"],
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "prepare_bounty_action",
            description: "Use this when the person wants to post, fund, compete, complete, or verify from an in-chat bounty card. It creates one idempotent first-party review session and returns an HTTPS authorization URL. It never asks ChatGPT for a wallet signature, private key, seed phrase, payment authorization, or verifier signature, and it never claims the action is complete.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "idempotency_key": {"type": "string", "minLength": 8, "maxLength": 200, "pattern": "^[A-Za-z0-9:._-]+$"},
                    "action": {"type": "string", "enum": ["post", "fund", "compete", "complete", "verify"]},
                    "network": {"type": ["string", "null"], "enum": ["base-mainnet", "base-sepolia", null]},
                    "opportunity_id": {"type": ["string", "null"], "minLength": 1, "maxLength": 200},
                    "bounty_contract": {"type": ["string", "null"], "pattern": "^0x[0-9a-fA-F]{40}$"},
                    "bounty_id": {"type": ["string", "null"], "pattern": "^0x[0-9a-fA-F]{64}$"},
                    "actor_wallet": {"type": ["string", "null"], "pattern": "^0x[0-9a-fA-F]{40}$"},
                    "amount_base_units": {"type": ["integer", "null"], "minimum": 1},
                    "details": {"type": "object", "description": "Bounded action-specific draft or evidence fields for first-party review."}
                },
                "required": ["idempotency_key", "action"],
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "get_bounty_action_status",
            description: "Use this when the person returns from first-party wallet review and the in-chat card needs to refresh one hosted bounty action. Confirmed status requires the exact indexed canonical event. A prepared session, signature, transaction hash, or receipt is never reported as completion or payment.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "intent_id": {"type": "string", "format": "uuid"}
                },
                "required": ["intent_id"],
                "additionalProperties": false
            }),
            authorization: None,
        },
    ]
}

pub(super) fn build_bounty_post_handoff(args: &PrepareBountyPostArgs) -> Result<Value, String> {
    let title = bounded_text(&args.title, "title", 200)?;
    let goal = bounded_text(&args.goal, "goal", 4_000)?;
    if args.acceptance_criteria.is_empty() || args.acceptance_criteria.len() > 20 {
        return Err("acceptance_criteria must contain between 1 and 20 items".to_string());
    }
    let acceptance_criteria = args
        .acceptance_criteria
        .iter()
        .map(|criterion| bounded_text(criterion, "acceptance criterion", 1_000))
        .collect::<Result<Vec<_>, _>>()?;
    let solver_reward = parse_usdc(&args.solver_reward_usdc, "solver_reward_usdc")?;
    let verifier_reward = parse_usdc(&args.verifier_reward_usdc, "verifier_reward_usdc")?;
    let target = solver_reward
        .checked_add(verifier_reward)
        .ok_or_else(|| "combined USDC target is too large".to_string())?;
    let source_url = optional_https_url(args.source_url.as_deref(), "source_url")?;
    let discovery_source = args
        .discovery_source
        .as_deref()
        .map(|value| bounded_text(value, "discovery_source", 500))
        .transpose()?;

    let mut post_url = Url::parse(POST_PAGE_URL).expect("static post URL is valid");
    {
        let mut query = post_url.query_pairs_mut();
        query.append_pair("from", "chatgpt-app");
        query.append_pair("title", &title);
        query.append_pair("goal", &goal);
        for criterion in &acceptance_criteria {
            query.append_pair("criterion", criterion);
        }
        query.append_pair("solverReward", &format_usdc(solver_reward));
        query.append_pair("verifierReward", &format_usdc(verifier_reward));
        query.append_pair("crowdfund", if args.crowdfund { "true" } else { "false" });
        if let Some(source_url) = &source_url {
            query.append_pair("sourceUrl", source_url);
        }
        query.append_pair(
            "discoverySource",
            discovery_source.as_deref().unwrap_or("ChatGPT app"),
        );
    }
    if post_url.as_str().len() > 12_000 {
        return Err(
            "the prepared bounty is too large for a safe browser handoff; shorten the goal or acceptance criteria"
                .to_string(),
        );
    }

    Ok(json!({
        "schema": "agent-bounties/chatgpt-post-handoff-v1",
        "state": "review_required_not_published",
        "title": title,
        "goal": goal,
        "acceptance_criteria": acceptance_criteria,
        "solver_reward_usdc": format_usdc(solver_reward),
        "verifier_reward_usdc": format_usdc(verifier_reward),
        "target_usdc": format_usdc(target),
        "initial_funding_usdc": if args.crowdfund { "0".to_string() } else { format_usdc(target) },
        "crowdfund": args.crowdfund,
        "source_url": source_url,
        "post_url": post_url.as_str(),
        "bounty_created": false,
        "wallet_signature_requested": false,
        "next_action": "Open the secure handoff, review every field, and choose whether to deposit 0 USDC now or fully fund. Then connect the creator wallet and approve only the exact Base transaction shown by that wallet.",
        "evidence_boundary": "No bounty id or contract exists yet. Only confirmed CanonicalBountyCreated proves creation; FundingAdded and BountyBecameClaimable prove funding and claimability."
    }))
}

pub(super) async fn mcp_post(
    State(state): State<SharedState>,
    Json(payload): Json<Value>,
) -> Response {
    let responses = if let Some(batch) = payload.as_array() {
        let mut responses = Vec::new();
        for request in batch {
            if let Some(response) = handle_request(state.clone(), request.clone()).await {
                responses.push(response);
            }
        }
        if responses.is_empty() {
            return StatusCode::ACCEPTED.into_response();
        }
        Value::Array(responses)
    } else if let Some(response) = handle_request(state, payload).await {
        response
    } else {
        return StatusCode::ACCEPTED.into_response();
    };

    (StatusCode::OK, Json(responses)).into_response()
}

pub(super) async fn mcp_get() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("allow", "POST")],
        "This stateless MCP endpoint accepts JSON-RPC over POST.",
    )
        .into_response()
}

pub(super) async fn mcp_delete() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn handle_request(state: SharedState, request: Value) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(json_rpc_error(Value::Null, -32600, "Invalid Request"));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(json_rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid Request",
        ));
    };
    let id = id?;
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method {
        "initialize" => Ok(initialize_result(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": chatgpt_tools().await})),
        "tools/call" => call_tool(state, &params).await,
        "resources/list" => Ok(json!({"resources": [feed_widget_resource_descriptor()]})),
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "resources/read" => read_resource(&params),
        _ => return Some(json_rpc_error(id, -32601, "Method not found")),
    };

    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json_rpc_error(id, -32602, &error),
    })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(MCP_PROTOCOL_VERSION);
    let protocol_version = match requested {
        "2024-11-05" | "2025-03-26" | "2025-06-18" => requested,
        _ => MCP_PROTOCOL_VERSION,
    };
    let sandbox = chatgpt_sandbox_mode();
    let instructions = if sandbox {
        "Sandbox mode is active. Use get_bounty_feed and render_bounty_feed to exercise the complete in-chat bounty UI. Use prepare_bounty_action and get_bounty_action_status to exercise the Stripe-style hosted handoff without opening a wallet. Every tool returns deterministic fixture data and performs no network write, wallet action, public comment, publication, funding, claim, submission, verification, settlement, or payment. Never describe sandbox output as canonical evidence."
    } else {
        "Use get_bounty_feed to inspect fresh structured bounty data, then render_bounty_feed to show the mounted interactive feed in ChatGPT. For post, fund, compete, complete, or verify, call prepare_bounty_action and open only its first-party HTTPS authorization URL. Never request or accept a wallet signature, private key, seed phrase, payment authorization, or verifier signature in ChatGPT. Refresh with get_bounty_action_status; only confirmed canonical events change the card, and only BountySettled proves solver payment. Use compile_objective_with_cloud_agent to break a broad objective into smaller reviewable child bounties. Use create_share_bundle after every meaningful step."
    };
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false}
        },
        "serverInfo": {
            "name": if sandbox { "agent-bounties-sandbox" } else { "agent-bounties" },
            "title": if sandbox { "Agent Bounties Sandbox" } else { "Agent Bounties" },
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": instructions
    })
}

async fn chatgpt_tools() -> Vec<Value> {
    let mut descriptors = tools().await.0;
    descriptors.extend(custom_tool_descriptors());
    descriptors
        .into_iter()
        .filter(|descriptor| CHATGPT_TOOL_NAMES.contains(&descriptor.name))
        .map(mcp_tool_descriptor)
        .collect()
}

fn mcp_tool_descriptor(descriptor: ToolDescriptor) -> Value {
    mcp_tool_descriptor_for_mode(descriptor, chatgpt_sandbox_mode())
}

fn mcp_tool_descriptor_for_mode(descriptor: ToolDescriptor, sandbox: bool) -> Value {
    let (read_only, destructive, open_world, idempotent) = if sandbox {
        (true, false, false, true)
    } else {
        tool_impact(descriptor.name)
    };
    let mut value = Map::new();
    value.insert("name".to_string(), json!(descriptor.name));
    value.insert("title".to_string(), json!(tool_title(descriptor.name)));
    let base_description = chatgpt_tool_description(descriptor.name, descriptor.description);
    let description = if sandbox {
        format!(
            "{base_description} Sandbox mode is active: this call returns simulated fixture data and performs no external write or wallet action."
        )
    } else {
        base_description.to_string()
    };
    value.insert("description".to_string(), json!(description));
    value.insert("inputSchema".to_string(), descriptor.input_schema);
    value.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "openWorldHint": open_world,
            "idempotentHint": idempotent
        }),
    );
    value.insert("securitySchemes".to_string(), json!([{"type": "noauth"}]));
    let mut metadata = json!({
        "securitySchemes": [{"type": "noauth"}],
        "ui": {"visibility": ["model", "app"]},
        "agentBountiesSandbox": sandbox
    });
    if matches!(descriptor.name, "get_bounty_feed" | "render_bounty_feed") {
        value.insert("outputSchema".to_string(), feed_output_schema());
    }
    if matches!(
        descriptor.name,
        "prepare_bounty_action" | "get_bounty_action_status"
    ) {
        value.insert("outputSchema".to_string(), bounty_action_output_schema());
    }
    if matches!(
        descriptor.name,
        "list_bounty_comments" | "add_bounty_comment"
    ) {
        value.insert("outputSchema".to_string(), bounty_comments_output_schema());
    }
    if descriptor.name == "create_share_bundle" {
        value.insert("outputSchema".to_string(), share_bundle_output_schema());
    }
    if descriptor.name == "compile_objective_with_cloud_agent" {
        value.insert("outputSchema".to_string(), objective_plan_output_schema());
    }
    let resource_uri = match descriptor.name {
        "prepare_bounty_post" => Some(POST_WIDGET_URI),
        "render_bounty_feed" => Some(FEED_WIDGET_URI),
        _ => None,
    };
    if let Some(resource_uri) = resource_uri {
        if descriptor.name == "prepare_bounty_post" {
            value.insert("outputSchema".to_string(), post_handoff_output_schema());
        }
        metadata["ui"]["resourceUri"] = json!(resource_uri);
        metadata["openai/outputTemplate"] = json!(resource_uri);
        metadata["openai/toolInvocation/invoking"] =
            json!(if descriptor.name == "render_bounty_feed" {
                "Opening live bounty feed..."
            } else {
                "Preparing bounty handoff..."
            });
        metadata["openai/toolInvocation/invoked"] =
            json!(if descriptor.name == "render_bounty_feed" {
                "Live feed ready"
            } else {
                "Bounty ready to review"
            });
    }
    value.insert("_meta".to_string(), metadata);
    Value::Object(value)
}

fn tool_impact(name: &str) -> (bool, bool, bool, bool) {
    match name {
        "prepare_bounty_action" => (false, false, true, true),
        "fund_bounty_with_x402" => (false, true, true, true),
        "agent_native_claim" => (false, true, true, true),
        "publish_unfunded_bounty" => (false, true, true, true),
        "submit_unfunded_bounty_solution" => (false, true, true, true),
        "publish_autonomous_submission_evidence" => (false, true, true, true),
        "add_bounty_comment" => (false, true, true, false),
        _ => (true, false, false, true),
    }
}

async fn call_tool(state: SharedState, params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "tools/call requires a tool name".to_string())?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if !CHATGPT_TOOL_NAMES.contains(&name) {
        return Err(format!(
            "unknown or unavailable public ChatGPT app tool: {name}"
        ));
    }

    if chatgpt_sandbox_mode() {
        return sandbox_tool_result(name, &arguments).await;
    }

    match name {
        "get_bounty_feed" => {
            let args: ChatgptFeedArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid get_bounty_feed arguments: {error}"))?;
            return Ok(tool_result(
                load_bounty_feed(args, &[]).await?,
                "Returned a fresh public opportunity projection. Funding, claimability, verification, settlement, and payment remain bound to each source’s authoritative evidence.",
                false,
            ));
        }
        "render_bounty_feed" => {
            let args: RenderFeedArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid render_bounty_feed arguments: {error}"))?;
            let opportunity_ids = args
                .opportunity_ids
                .iter()
                .map(|value| bounded_opportunity_id(value))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(tool_result(
                load_bounty_feed(args.feed, &opportunity_ids).await?,
                "Rendered the live Agent Bounties feed inside ChatGPT. Every card keeps work state, payment state, and canonical evidence boundaries separate.",
                false,
            ));
        }
        "list_bounty_comments" => {
            let args: ListCommentsArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid list_bounty_comments arguments: {error}"))?;
            let bounty_id = bounded_opportunity_id(&args.bounty_id)?;
            return Ok(tool_result(
                fetch_comments(&bounty_id).await?,
                "Returned public in-chat comments for this bounty. Comments are conversation context, not payment or verification evidence.",
                false,
            ));
        }
        "add_bounty_comment" => {
            let args: AddCommentArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid add_bounty_comment arguments: {error}"))?;
            let bounty_id = bounded_opportunity_id(&args.bounty_id)?;
            let body = bounded_text(&args.body, "body", 500)?;
            let author = args
                .author
                .as_deref()
                .map(|value| bounded_text(value, "author", 60))
                .transpose()?
                .unwrap_or_else(|| "you".to_string());
            let id = match args.comment_id.as_deref() {
                Some(value) => Uuid::parse_str(value)
                    .map_err(|_| "comment_id must be a UUID when provided".to_string())?,
                None => Uuid::new_v4(),
            };
            let result = proxy_hosted_json(
                reqwest::Client::new()
                    .post(format!(
                        "{}/v1/opportunities/{}/comments",
                        public_base_url_from_env().trim_end_matches('/'),
                        bounty_id
                    ))
                    .json(&json!({
                        "id": id,
                        "author": author,
                        "body": body,
                    })),
            )
            .await
            .0;
            return Ok(tool_result(
                legacy_result(result)?,
                "Comment published to the durable public bounty feed. Share the updated card when ready; comments remain conversation context, not payment or verification evidence.",
                false,
            ));
        }
        "create_share_bundle" => {
            let args: ShareBundleArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid create_share_bundle arguments: {error}"))?;
            return Ok(tool_result(build_share_bundle(&args)?, "Prepared a share-ready bounty card caption and social intents. Sharing is optional and does not change canonical payment state.", false));
        }
        "prepare_bounty_action" => {
            let args: PrepareBountyActionArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_bounty_action arguments: {error}"))?;
            let action = bounded_text(&args.action, "action", 16)?;
            if !matches!(
                action.as_str(),
                "post" | "fund" | "compete" | "complete" | "verify"
            ) {
                return Err("action must be post, fund, compete, complete, or verify".to_string());
            }
            let idempotency_key =
                bounded_public_key(&args.idempotency_key, "idempotency_key", 8, 200)?;
            let details = if args.details.is_null() {
                json!({})
            } else if args.details.is_object() {
                args.details
            } else {
                return Err("details must be an object".to_string());
            };
            let result = proxy_hosted_json(
                reqwest::Client::new()
                    .post(format!(
                        "{}/v1/chatgpt/action-intents",
                        public_base_url_from_env().trim_end_matches('/')
                    ))
                    .json(&json!({
                        "idempotency_key": idempotency_key,
                        "action": action,
                        "network": args.network,
                        "opportunity_id": args.opportunity_id,
                        "bounty_contract": args.bounty_contract,
                        "bounty_id": args.bounty_id,
                        "actor_wallet": args.actor_wallet,
                        "amount_base_units": args.amount_base_units,
                        "details": details,
                    })),
            )
            .await
            .0;
            return Ok(tool_result(
                legacy_result(result)?,
                "Prepared one first-party wallet-review session. No signature or payment credential entered ChatGPT, and the action remains unconfirmed until its exact canonical event is indexed.",
                true,
            ));
        }
        "get_bounty_action_status" => {
            let args: GetBountyActionStatusArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid get_bounty_action_status arguments: {error}"))?;
            let intent_id = Uuid::parse_str(&args.intent_id)
                .map_err(|_| "intent_id must be a UUID".to_string())?;
            let result = proxy_hosted_json(reqwest::Client::new().get(format!(
                "{}/v1/chatgpt/action-intents/{intent_id}",
                public_base_url_from_env().trim_end_matches('/')
            )))
            .await
            .0;
            return Ok(tool_result(
                legacy_result(result)?,
                "Refreshed the hosted action against indexed canonical events. A transaction hash or receipt alone never confirms the action; only BountySettled proves solver payment.",
                false,
            ));
        }
        _ => {}
    }

    let (legacy, narration) = match name {
        "fund_bounty_with_x402" => {
            let args: X402BountyFundingArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid fund_bounty_with_x402 arguments: {error}"))?;
            (
                fund_bounty_with_x402(State(state), Json(args)).await.0,
                "Requested the canonical x402 funding challenge. Only a confirmed FundingAdded event changes funding state; a challenge, signature, or transaction hash alone is not funding evidence.",
            )
        }
        "get_x402_relay_status" => {
            let args: GetX402RelayStatusArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid get_x402_relay_status arguments: {error}"))?;
            (
                get_x402_relay_status(State(state), Json(args)).await.0,
                "Returned the current relay status. Only a confirmed FundingAdded event changes the bounty's funded amount.",
            )
        }
        "prepare_agent_to_earn" => {
            let args: PrepareAgentToEarnInput = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_agent_to_earn arguments: {error}"))?;
            (
                prepare_agent_to_earn(State(state), Json(args)).await.0,
                "Returned wallet-neutral claim readiness. A readiness response is not a claim, completion, verification, settlement, or payment.",
            )
        }
        "agent_native_claim" => {
            let args: AgentNativeClaimArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid agent_native_claim arguments: {error}"))?;
            (
                agent_native_claim(State(state), Json(args)).await.0,
                "Advanced the retry-safe claim flow. Only confirmed BountyClaimed proves that the solver owns the active round.",
            )
        }
        "plan_autonomous_bounty_claim" => {
            let args: PlanAutonomousBountyClaimArgs =
                serde_json::from_value(arguments).map_err(|error| {
                    format!("invalid plan_autonomous_bounty_claim arguments: {error}")
                })?;
            (
                plan_autonomous_bounty_claim(State(state), Json(args)).await.0,
                "Prepared the exact direct-wallet claim path. Only confirmed canonical claim events make a solver claim active.",
            )
        }
        "prepare_autonomous_bounty_submission" => {
            let args: PrepareAutonomousBountySubmissionArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid prepare_autonomous_bounty_submission arguments: {error}")
                })?;
            (
                prepare_autonomous_bounty_submission(State(state), Json(args)).await.0,
                "Prepared a bounded submission and evidence package. Preparation is not completion or verification evidence.",
            )
        }
        "publish_autonomous_submission_evidence" => {
            let args: PublishAutonomousSubmissionEvidenceArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                format!("invalid publish_autonomous_submission_evidence arguments: {error}")
            })?;
            (
                publish_autonomous_submission_evidence(State(state), Json(args))
                    .await
                    .0,
                "Published hash-matched public submission evidence after the canonical SubmissionAdded event. Publication alone is not verification or payout.",
            )
        }
        "list_autonomous_verification_jobs" => {
            let args: AutonomousVerificationJobsArgs =
                serde_json::from_value(arguments).map_err(|error| {
                    format!("invalid list_autonomous_verification_jobs arguments: {error}")
                })?;
            (
                list_autonomous_verification_jobs(State(state), Json(args))
                    .await
                    .0,
                "Returned current hash-matched verification jobs. A job or verdict is not settlement evidence.",
            )
        }
        "plan_autonomous_verification_attestation" => {
            let args: PlanAutonomousVerificationAttestationArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid plan_autonomous_verification_attestation arguments: {error}")
                })?;
            (
                plan_autonomous_verification_attestation(State(state), Json(args)).await.0,
                "Prepared a verification attestation path. Only the configured canonical verifier/quorum transition proves verification.",
            )
        }
        "plan_autonomous_module_settlement" => {
            let args: PlanAutonomousModuleSettlementArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid plan_autonomous_module_settlement arguments: {error}")
                })?;
            (
                plan_autonomous_module_settlement(State(state), Json(args))
                    .await
                    .0,
                "Prepared the exact deterministic verifier transaction. Only a confirmed canonical settlement event proves the outcome and payout.",
            )
        }
        "plan_autonomous_attestation_settlement" => {
            let args: PlanAutonomousAttestationSettlementArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                format!("invalid plan_autonomous_attestation_settlement arguments: {error}")
            })?;
            (
                plan_autonomous_attestation_settlement(State(state), Json(args))
                    .await
                    .0,
                "Prepared the exact signed-quorum settlement transaction. Signatures and plans are not canonical settlement evidence.",
            )
        }
        "get_paid_status" => {
            let args: PaidStatusArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid get_paid_status arguments: {error}"))?;
            (
                get_paid_status(State(state), Json(args)).await.0,
                "Returned reconciled payout status and the appropriate share trigger when canonical evidence exists.",
            )
        }
        "compile_objective_with_cloud_agent" => {
            let args: CompileObjectiveWithCloudAgentArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid compile_objective_with_cloud_agent arguments: {error}")
                })?;
            (
                compile_objective_with_cloud_agent(State(state), Json(args)).await.0,
                "Compiled a bounded bounty graph. The decomposition is advisory until each child has independently reviewable terms, funding, verification, and evidence.",
            )
        }
        "publish_unfunded_bounty" => {
            let args: PublishUnfundedBountyArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid publish_unfunded_bounty arguments: {error}"))?;
            (
                publish_unfunded_bounty(State(state), Json(args)).await.0,
                "Published a public unfunded bounty and returned the bounded Agent Bounties demo-agent response. Agents can discover it, but no wallet, USDC, payment promise, or canonical bounty was involved.",
            )
        }
        "list_unfunded_bounties" => {
            let args: ListUnfundedBountiesArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid list_unfunded_bounties arguments: {error}"))?;
            (
                list_unfunded_bounties(Json(args)).await.0,
                "Returned recent public unfunded bounty opportunities and their solutions. They are not yet canonical, funded, claimable, or guaranteed to pay.",
            )
        }
        "submit_unfunded_bounty_solution" => {
            let args: SubmitUnfundedBountySolutionArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid submit_unfunded_bounty_solution arguments: {error}")
                })?;
            (
                submit_unfunded_bounty_solution(Json(args)).await.0,
                "Published the registered agent's solution to the open unfunded bounty. This creates no payment claim.",
            )
        }
        "prepare_bounty_post" => {
            let args: PrepareBountyPostArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_bounty_post arguments: {error}"))?;
            let value = build_bounty_post_handoff(&args)?;
            return Ok(tool_result(
                value,
                "Prepared a reviewable wallet handoff. No bounty has been published or created yet.",
                true,
            ));
        }
        "list_autonomous_bounties" => {
            let args: AutonomousBountyFeedArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid list_autonomous_bounties arguments: {error}"))?;
            (
                list_autonomous_bounties(State(state), Json(args)).await.0,
                "Returned canonical, event-derived bounty inventory.",
            )
        }
        _ => return Err(format!("unknown or unavailable ChatGPT app tool: {name}")),
    };
    match legacy_result(legacy) {
        Ok(value) => Ok(tool_result(value, narration, false)),
        Err(error) => Ok(tool_error(error)),
    }
}

fn chatgpt_sandbox_mode() -> bool {
    env::var(CHATGPT_SANDBOX_ENV).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

async fn sandbox_tool_result(name: &str, arguments: &Value) -> Result<Value, String> {
    let (value, narration, wallet_review) = match name {
        "get_bounty_feed" => {
            let args: ChatgptFeedArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid get_bounty_feed arguments: {error}"))?;
            (
                sandbox_bounty_feed(args, &[])?,
                "Returned the safe Agent Bounties sandbox feed. Every item and lifecycle response is simulated and no external write can occur.",
                false,
            )
        }
        "render_bounty_feed" => {
            let args: RenderFeedArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid render_bounty_feed arguments: {error}"))?;
            let opportunity_ids = args
                .opportunity_ids
                .iter()
                .map(|value| bounded_opportunity_id(value))
                .collect::<Result<Vec<_>, _>>()?;
            (
                sandbox_bounty_feed(args.feed, &opportunity_ids)?,
                "Rendered the complete Agent Bounties sandbox feed inside ChatGPT. Its cards are interactive, but every lifecycle result is simulated and performs no external write.",
                false,
            )
        }
        "prepare_bounty_action" => {
            let args: PrepareBountyActionArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid prepare_bounty_action arguments: {error}"))?;
            let action = bounded_text(&args.action, "action", 16)?;
            let id = sandbox_action_intent_id(&action)?;
            (
                sandbox_action_response(
                    &action,
                    id,
                    "review_required",
                    args.opportunity_id,
                    args.bounty_contract,
                    args.bounty_id,
                    args.amount_base_units,
                    args.details,
                ),
                "Prepared a sandbox-only first-party review session. No network write, wallet request, signature, transaction, or payment occurred.",
                true,
            )
        }
        "get_bounty_action_status" => {
            let args: GetBountyActionStatusArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid get_bounty_action_status arguments: {error}"))?;
            let id = Uuid::parse_str(&args.intent_id)
                .map_err(|_| "intent_id must be a UUID".to_string())?;
            let action = sandbox_action_from_intent_id(id)?;
            (
                sandbox_action_response(
                    action,
                    id,
                    "confirmed",
                    None,
                    None,
                    None,
                    None,
                    json!({}),
                ),
                "Returned simulated canonical confirmation for UI testing only. No indexed event, settlement, or payment exists.",
                false,
            )
        }
        "list_bounty_comments" => {
            let args: ListCommentsArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid list_bounty_comments arguments: {error}"))?;
            let bounty_id = bounded_opportunity_id(&args.bounty_id)?;
            let comments = sandbox_comments(&bounty_id);
            let comment_count = comments.len();
            (
                json!({
                    "sandbox": true,
                    "bounty_id": bounty_id,
                    "comments": comments,
                    "comment_count": comment_count,
                    "evidence_boundary": "Sandbox comments are ephemeral fixtures and were not published."
                }),
                "Returned ephemeral sandbox comments. No production comment was read or written.",
                false,
            )
        }
        "add_bounty_comment" => {
            let args: AddCommentArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid add_bounty_comment arguments: {error}"))?;
            let bounty_id = bounded_opportunity_id(&args.bounty_id)?;
            let body = bounded_text(&args.body, "body", 500)?;
            let author = args
                .author
                .as_deref()
                .map(|value| bounded_text(value, "author", 60))
                .transpose()?
                .unwrap_or_else(|| "you".to_string());
            let comment_id = match args.comment_id.as_deref() {
                Some(value) => Uuid::parse_str(value)
                    .map_err(|_| "comment_id must be a UUID when provided".to_string())?,
                None => Uuid::new_v4(),
            };
            let mut comments = sandbox_comments(&bounty_id);
            comments.push(json!({
                "id": comment_id,
                "author": author,
                "body": body,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "sandbox": true
            }));
            let comment_count = comments.len();
            (
                json!({
                    "sandbox": true,
                    "bounty_id": bounty_id,
                    "comments": comments,
                    "comment_count": comment_count,
                    "published": false,
                    "evidence_boundary": "This comment exists only in the current sandbox response and was not published."
                }),
                "Simulated a comment response without publishing anything.",
                false,
            )
        }
        "create_share_bundle" => {
            let args: ShareBundleArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid create_share_bundle arguments: {error}"))?;
            let mut value = build_share_bundle(&args)?;
            mark_sandbox(
                &mut value,
                "This share bundle describes a sandbox interaction only and must not be represented as a live bounty event.",
            );
            (
                value,
                "Prepared a sandbox-labeled share bundle. No social post was opened or published.",
                false,
            )
        }
        "fund_bounty_with_x402" => {
            let signed = arguments
                .get("payment_signature")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            let value = if signed {
                json!({
                    "sandbox": true,
                    "http_status": 202,
                    "body": {
                        "state": "sandbox_relay_complete",
                        "funded": false,
                        "canonical_event_id": null,
                        "next_action": "Sandbox complete. No relay, transaction, FundingAdded event, or USDC movement occurred."
                    },
                    "evidence_boundary": "A sandbox signature is not funding evidence and was not sent anywhere."
                })
            } else {
                json!({
                    "sandbox": true,
                    "http_status": 402,
                    "payment_required_header": "sandbox-x402-challenge",
                    "body": {
                        "state": "authorization_required",
                        "wallet_request": {
                            "method": "sandbox_sign",
                            "message": "Agent Bounties sandbox funding authorization"
                        }
                    },
                    "evidence_boundary": "This is a local fixture challenge. Do not submit a real wallet signature."
                })
            };
            (
                value,
                "Simulated the two-step x402 funding path. No wallet request, relay, transaction, or USDC movement occurred.",
                false,
            )
        }
        "get_x402_relay_status" => (
            json!({
                "sandbox": true,
                "state": "sandbox_relay_complete",
                "funded": false,
                "canonical_event_id": null,
                "evidence_boundary": "No relay or FundingAdded event exists."
            }),
            "Returned a simulated relay status without contacting a relay.",
            false,
        ),
        "prepare_agent_to_earn" => (
            json!({
                "sandbox": true,
                "ready": true,
                "wallet_address": arguments.get("wallet_address").cloned().unwrap_or(Value::Null),
                "checks": [
                    {"name": "sandbox_fixture", "passed": true},
                    {"name": "external_write_disabled", "passed": true}
                ],
                "evidence_boundary": "Readiness is simulated and is not a canonical claim."
            }),
            "Returned simulated claim readiness without reading a wallet or chain.",
            false,
        ),
        "agent_native_claim" => {
            let signed = arguments
                .get("wallet_signature")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            let body = if signed {
                json!({
                    "state": "sandbox_claim_complete",
                    "claimed": false,
                    "canonical_event_id": null,
                    "next_action": "Sandbox complete. No BountyClaimed event exists."
                })
            } else {
                json!({
                    "state": "authorization_ready",
                    "wallet_request": {
                        "method": "sandbox_signTypedData",
                        "params": ["0x0000000000000000000000000000000000000000", {"sandbox": true}]
                    }
                })
            };
            (
                json!({
                    "sandbox": true,
                    "body": body,
                    "evidence_boundary": "No signature was transmitted and no canonical claim was created."
                }),
                "Simulated the approval-gated claim flow without signing or claiming.",
                false,
            )
        }
        "plan_autonomous_bounty_claim" => (
            sandbox_plan(name, "BountyClaimed"),
            "Prepared a sandbox-only claim plan. No transaction was created or broadcast.",
            false,
        ),
        "prepare_autonomous_bounty_submission" => {
            let bounty_contract = arguments
                .get("bounty_contract")
                .cloned()
                .unwrap_or_else(|| json!("0xabc1000000000000000000000000000000000001"));
            let network = arguments
                .get("network")
                .cloned()
                .unwrap_or_else(|| json!("base-mainnet"));
            let solver_wallet = arguments
                .get("solver_wallet")
                .cloned()
                .unwrap_or(Value::Null);
            let artifact_reference = arguments
                .get("artifact_reference")
                .cloned()
                .unwrap_or(Value::Null);
            let evidence = arguments
                .get("evidence")
                .cloned()
                .unwrap_or_else(|| json!({}));
            (
                json!({
                    "sandbox": true,
                    "bounty_contract": bounty_contract,
                    "bounty_id": format!("0x{}", "11".repeat(32)),
                    "round": 1,
                    "expected_canonical_event": "SubmissionAdded",
                    "signing_payload": {"sandbox": true, "method": "sandbox_submit"},
                    "evidence_publication": {
                        "network": network,
                        "bounty_contract": bounty_contract,
                        "bounty_id": format!("0x{}", "11".repeat(32)),
                        "round": 1,
                        "solver_wallet": solver_wallet,
                        "artifact_reference": artifact_reference,
                        "evidence": evidence,
                        "sandbox": true
                    },
                    "evidence_boundary": "This package is a fixture. No SubmissionAdded event or public evidence exists."
                }),
                "Prepared a sandbox completion package without signing, submitting, or publishing.",
                false,
            )
        }
        "publish_autonomous_submission_evidence" => (
            json!({
                "sandbox": true,
                "state": "sandbox_publication_complete",
                "published": false,
                "round": arguments.get("round").cloned().unwrap_or_else(|| json!(1)),
                "evidence_boundary": "No evidence was published and no verification state changed."
            }),
            "Simulated evidence publication without writing to a public service.",
            false,
        ),
        "list_autonomous_verification_jobs" => (
            sandbox_verification_jobs(),
            "Returned deterministic sandbox verification jobs. They are fixtures, not live verifier work.",
            false,
        ),
        "plan_autonomous_verification_attestation" => (
            sandbox_plan(name, "VerificationAttested"),
            "Prepared a sandbox attestation plan without signing or publishing an attestation.",
            false,
        ),
        "plan_autonomous_module_settlement" => (
            sandbox_plan(name, "BountySettled"),
            "Prepared a sandbox deterministic-verifier plan without creating or broadcasting a transaction.",
            false,
        ),
        "plan_autonomous_attestation_settlement" => (
            sandbox_plan(name, "BountySettled"),
            "Prepared a sandbox quorum-settlement plan without creating or broadcasting a transaction.",
            false,
        ),
        "get_paid_status" => (
            json!({
                "sandbox": true,
                "paid": false,
                "state": "sandbox_only",
                "canonical_event_id": null,
                "evidence_boundary": "No payout lookup occurred and no BountySettled event is claimed."
            }),
            "Returned a sandbox payout fixture. It is not payment evidence.",
            false,
        ),
        "compile_objective_with_cloud_agent" => {
            let objective = arguments
                .get("objective")
                .and_then(Value::as_str)
                .map(|value| bounded_text(value, "objective", 4_000))
                .transpose()?
                .unwrap_or_else(|| "Sandbox objective".to_string());
            (
                json!({
                    "sandbox": true,
                    "objective": objective,
                    "tasks": [
                        {
                            "title": "Specify the public contract",
                            "acceptance_criteria": ["Define the exact artifact and public evidence boundary."]
                        },
                        {
                            "title": "Implement the smallest working slice",
                            "acceptance_criteria": ["Deliver one independently testable outcome."]
                        },
                        {
                            "title": "Verify and publish evidence",
                            "acceptance_criteria": ["Run the committed checks and publish bounded evidence."]
                        }
                    ],
                    "published": false,
                    "evidence_boundary": "These are sandbox drafts. No child bounty was posted or funded."
                }),
                "Compiled three sandbox child-bounty drafts without calling a cloud agent or publishing anything.",
                false,
            )
        }
        "publish_unfunded_bounty" => {
            let title = arguments
                .get("title")
                .and_then(Value::as_str)
                .map(|value| bounded_text(value, "title", 200))
                .transpose()?
                .unwrap_or_else(|| "Sandbox bounty".to_string());
            (
                json!({
                    "sandbox": true,
                    "bounty_id": Uuid::new_v4(),
                    "title": title,
                    "public_url": "https://agentbounties.app/earn.html",
                    "funding_status": "sandbox_unfunded",
                    "published": false,
                    "payment_promised": false,
                    "evidence_boundary": "No public request, wallet, USDC commitment, or canonical bounty was created."
                }),
                "Simulated an unfunded bounty post without publishing it.",
                false,
            )
        }
        "list_unfunded_bounties" => (
            json!({
                "sandbox": true,
                "items": [{
                    "id": "sandbox-unfunded-1",
                    "title": "Sandbox unfunded request",
                    "funding_status": "unfunded",
                    "published": false
                }],
                "evidence_boundary": "This is fixture inventory only."
            }),
            "Returned sandbox unfunded inventory without reading production.",
            false,
        ),
        "submit_unfunded_bounty_solution" => (
            json!({
                "sandbox": true,
                "state": "sandbox_solution_complete",
                "published": false,
                "payment_claim_created": false,
                "evidence_boundary": "No solution was published and no payment claim exists."
            }),
            "Simulated an unfunded solution without publishing it.",
            false,
        ),
        "prepare_bounty_post" => {
            let args: PrepareBountyPostArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid prepare_bounty_post arguments: {error}"))?;
            let mut value = build_bounty_post_handoff(&args)?;
            mark_sandbox(
                &mut value,
                "The handoff is a sandbox fixture. No bounty was published, created, signed, or funded.",
            );
            (
                value,
                "Prepared a sandbox wallet-review card without publishing or requesting a wallet signature.",
                true,
            )
        }
        "list_autonomous_bounties" => {
            let items = sandbox_bounty_feed(
                ChatgptFeedArgs {
                    network: Some("base-mainnet".to_string()),
                    view: None,
                    source_type: Some("canonical_base".to_string()),
                    work_state: None,
                    payment_state: None,
                    limit: Some(30),
                },
                &[],
            )?["items"]
                .clone();
            (
                json!({
                    "sandbox": true,
                    "items": items,
                    "evidence_boundary": "This is deterministic sandbox inventory, not event-derived chain state."
                }),
                "Returned deterministic sandbox canonical inventory without reading a chain or indexer.",
                false,
            )
        }
        _ => {
            return Err(format!(
                "unknown or unavailable ChatGPT app sandbox tool: {name}"
            ))
        }
    };
    Ok(tool_result(value, narration, wallet_review))
}

fn sandbox_action_intent_id(action: &str) -> Result<Uuid, String> {
    let suffix = match action {
        "post" => 1,
        "fund" => 2,
        "compete" => 3,
        "complete" => 4,
        "verify" => 5,
        _ => return Err("action must be post, fund, compete, complete, or verify".to_string()),
    };
    Uuid::parse_str(&format!("00000000-0000-4000-8000-{suffix:012}"))
        .map_err(|_| "failed to build sandbox action intent".to_string())
}

fn sandbox_action_from_intent_id(id: Uuid) -> Result<&'static str, String> {
    match id.as_bytes()[15] {
        1 => Ok("post"),
        2 => Ok("fund"),
        3 => Ok("compete"),
        4 => Ok("complete"),
        5 => Ok("verify"),
        _ => Err("unknown sandbox action intent".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn sandbox_action_response(
    action: &str,
    intent_id: Uuid,
    status: &str,
    opportunity_id: Option<String>,
    bounty_contract: Option<String>,
    bounty_id: Option<String>,
    amount_base_units: Option<u64>,
    details: Value,
) -> Value {
    let expected_event = match action {
        "post" => "canonical_bounty_created",
        "fund" => "funding_added",
        "compete" => "bounty_claimed",
        "complete" => "submission_added",
        "verify" => "bounty_settled",
        _ => "sandbox_event",
    };
    json!({
        "schema_version": "agent-bounties/chatgpt-action-intent-v1",
        "intent_id": intent_id,
        "action": action,
        "status": status,
        "network": "base-mainnet",
        "opportunity_id": opportunity_id,
        "bounty_contract": bounty_contract,
        "bounty_id": bounty_id,
        "actor_wallet": null,
        "amount_base_units": amount_base_units,
        "details": if details.is_object() { details } else { json!({}) },
        "authorization_url": format!(
            "https://agentbounties.app/authorize.html?sandbox=1&intent={intent_id}"
        ),
        "expected_canonical_events": if action == "verify" {
            json!(["bounty_settled", "submission_rejected"])
        } else {
            json!([expected_event])
        },
        "transaction_hash": null,
        "canonical_event_id": null,
        "canonical_event_kind": if status == "confirmed" {
            Value::String(expected_event.to_string())
        } else {
            Value::Null
        },
        "confirmed_block": null,
        "paid": false,
        "expires_at": (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        "share_after": true,
        "next_action": if status == "confirmed" {
            "Sandbox UI confirmation complete. Share only as a sandbox test."
        } else {
            "In production, open the first-party review URL. Sandbox mode never opens a wallet."
        },
        "sandbox": true,
        "evidence_boundary": "Sandbox fixture only. No hosted record, wallet request, signature, transaction, canonical event, settlement, or payment exists."
    })
}

fn sandbox_bounty_feed(args: ChatgptFeedArgs, opportunity_ids: &[String]) -> Result<Value, String> {
    let mut value = sandbox_feed_projection();
    let items = value
        .get_mut("items")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "sandbox projection did not contain items".to_string())?;
    items.retain(|item| {
        let id_matches = opportunity_ids.is_empty()
            || item
                .get("opportunity_id")
                .and_then(Value::as_str)
                .is_some_and(|id| opportunity_ids.iter().any(|selected| selected == id));
        let source_matches = args.source_type.as_deref().map_or(true, |expected| {
            item.get("source_type").and_then(Value::as_str) == Some(expected)
        });
        let work_matches = args.work_state.as_deref().map_or(true, |expected| {
            item.get("work_state").and_then(Value::as_str) == Some(expected)
        });
        let payment_matches = args.payment_state.as_deref().map_or(true, |expected| {
            item.get("payment_state").and_then(Value::as_str) == Some(expected)
        });
        let network_matches = args.network.as_deref().map_or(true, |expected| {
            value_network(item).map_or(expected == "base-mainnet", |actual| actual == expected)
        });
        let view_matches = match args.view.as_deref() {
            Some("seeking_funding") => {
                item.get("payment_state").and_then(Value::as_str) == Some("seeking_funding")
            }
            Some("ready_to_earn") => {
                item.get("work_state").and_then(Value::as_str) == Some("claimable")
            }
            Some("engineering") => item
                .get("categories")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value == "engineering")),
            Some("creative") => item
                .get("categories")
                .and_then(Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value == "creative")),
            Some("urgent") => item.get("work_state").and_then(Value::as_str) == Some("submitted"),
            Some("recent") | None => true,
            Some(_) => true,
        };
        id_matches
            && source_matches
            && work_matches
            && payment_matches
            && network_matches
            && view_matches
    });
    items.truncate(args.limit.unwrap_or(30).clamp(1, 30) as usize);
    Ok(value)
}

fn value_network(item: &Value) -> Option<&str> {
    item.get("opportunity_id")
        .and_then(Value::as_str)
        .and_then(|value| value.split(':').nth(1))
}

fn sandbox_feed_projection() -> Value {
    let money = |amount: u64| {
        json!({
            "amount": amount.to_string(),
            "currency": "USDC",
            "unit": "base_units",
            "decimals": 6
        })
    };
    json!({
        "schema_version": "agent-bounties/opportunity-projection-v1",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "state_token": Uuid::new_v4(),
        "network": "base-mainnet",
        "degraded": false,
        "sandbox": true,
        "sandbox_notice": "Safe ChatGPT host test. All cards and lifecycle responses are deterministic fixtures; no production read, write, wallet request, social post, or payment occurs.",
        "evidence_boundary": "Sandbox fixtures are not canonical bounty, funding, claim, submission, verification, settlement, or payment evidence.",
        "items": [
            {
                "opportunity_id": "canonical_base:base-mainnet:0xabc1000000000000000000000000000000000001",
                "source_id": "0xabc1000000000000000000000000000000000001",
                "source_type": "canonical_base",
                "title": "Ship the in-chat bounty feed",
                "goal": "Make every bounty lifecycle action work through the MCP Apps bridge and preserve canonical evidence boundaries.",
                "categories": ["engineering", "featured"],
                "skills": ["Rust", "MCP Apps", "UI"],
                "public_url": "https://agentbounties.app/earn.html",
                "work_state": "claimable",
                "payment_state": "escrowed",
                "payment_committed": true,
                "reward": money(3_500_000),
                "funded_amount": money(4_000_000),
                "funding_target": money(4_000_000),
                "bond": money(500_000),
                "verification_method": "signed quorum",
                "verification_ready": true,
                "evidence_boundary": "Sandbox only. In production, BountyClaimed, SubmissionAdded, and BountySettled remain authoritative.",
                "comments": [{"author": "maya", "body": "The card makes the state and reward easy to scan.", "sandbox": true}]
            },
            {
                "opportunity_id": "canonical_base:base-mainnet:0xabc2000000000000000000000000000000000002",
                "source_id": "0xabc2000000000000000000000000000000000002",
                "source_type": "canonical_base",
                "title": "Fund a deterministic documentation fix",
                "goal": "Exercise the two-step funding interaction without moving USDC or contacting a relay.",
                "categories": ["documentation"],
                "skills": ["Technical writing"],
                "public_url": "https://agentbounties.app/earn.html",
                "work_state": "open",
                "payment_state": "seeking_funding",
                "payment_committed": false,
                "reward": money(2_000_000),
                "funded_amount": money(400_000),
                "funding_target": money(2_200_000),
                "bond": money(200_000),
                "verification_method": "deterministic module",
                "verification_ready": true,
                "evidence_boundary": "Sandbox only. A challenge or fixture signature is not FundingAdded.",
                "comments": []
            },
            {
                "opportunity_id": "canonical_base:base-mainnet:0xabc3000000000000000000000000000000000003",
                "source_id": "0xabc3000000000000000000000000000000000003",
                "source_type": "canonical_base",
                "title": "Verify the submitted accessibility audit",
                "goal": "Exercise deterministic and signed-quorum verification plans against fixture evidence.",
                "categories": ["verification"],
                "skills": ["Accessibility", "Evidence review"],
                "public_url": "https://agentbounties.app/earn.html",
                "work_state": "submitted",
                "payment_state": "escrowed",
                "payment_committed": true,
                "reward": money(5_000_000),
                "funded_amount": money(5_500_000),
                "funding_target": money(5_500_000),
                "bond": money(500_000),
                "verification_method": "signed quorum",
                "verification_ready": true,
                "evidence_boundary": "Sandbox only. A fixture verdict or signature cannot prove settlement.",
                "comments": []
            },
            {
                "opportunity_id": "canonical_base:base-mainnet:0xabc4000000000000000000000000000000000004",
                "source_id": "0xabc4000000000000000000000000000000000004",
                "source_type": "canonical_base",
                "title": "Complete the claimed card accessibility pass",
                "goal": "Prepare a public artifact commitment and hash-matched evidence package entirely through the ChatGPT host bridge.",
                "categories": ["engineering", "completion"],
                "skills": ["Accessibility", "Evidence"],
                "public_url": "https://agentbounties.app/earn.html",
                "work_state": "in_progress",
                "payment_state": "escrowed",
                "payment_committed": true,
                "reward": money(4_000_000),
                "funded_amount": money(4_500_000),
                "funding_target": money(4_500_000),
                "bond": money(500_000),
                "verification_method": "signed quorum",
                "verification_ready": true,
                "evidence_boundary": "Sandbox only. Preparing or publishing fixture evidence is not SubmissionAdded, verification, settlement, or payment.",
                "comments": []
            },
            {
                "opportunity_id": "canonical_base:base-mainnet:0xabc5000000000000000000000000000000000005",
                "source_id": "0xabc5000000000000000000000000000000000005",
                "source_type": "canonical_base",
                "title": "Run the deterministic card verifier",
                "goal": "Exercise the committed deterministic-module verification branch without signing, broadcasting, or settling anything.",
                "categories": ["verification", "deterministic"],
                "skills": ["Proof review", "MCP Apps"],
                "public_url": "https://agentbounties.app/earn.html",
                "work_state": "submitted",
                "payment_state": "escrowed",
                "payment_committed": true,
                "reward": money(3_000_000),
                "funded_amount": money(3_300_000),
                "funding_target": money(3_300_000),
                "bond": money(300_000),
                "verification_method": "deterministic module",
                "verification_ready": true,
                "evidence_boundary": "Sandbox only. A proof or transaction plan is not BountySettled or solver payment.",
                "comments": []
            }
        ]
    })
}

fn sandbox_comments(opportunity_id: &str) -> Vec<Value> {
    if opportunity_id.contains("abc100") {
        vec![json!({
            "id": Uuid::nil(),
            "author": "maya",
            "body": "The card makes the state and reward easy to scan.",
            "sandbox": true
        })]
    } else {
        Vec::new()
    }
}

fn sandbox_verification_jobs() -> Value {
    json!({
        "sandbox": true,
        "jobs": [
            {
                "sandbox": true,
                "bounty_contract": "0xabc3000000000000000000000000000000000003",
                "bounty_id": format!("0x{}", "22".repeat(32)),
                "round": 1,
                "verification_mode": "signed_quorum",
                "eligible_verifiers": [
                    "0x1111111111111111111111111111111111111111",
                    "0x2222222222222222222222222222222222222222"
                ],
                "threshold": 2,
                "verification_expires_at": chrono::Utc::now().timestamp() + 7_200,
                "terms": {"policy_hash": format!("0x{}", "33".repeat(32))},
                "submission_evidence": {
                    "artifact_hash": format!("0x{}", "44".repeat(32)),
                    "evidence_hash": format!("0x{}", "55".repeat(32))
                },
                "evidence_boundary": "Sandbox fixture only; no live verification job exists."
            },
            {
                "sandbox": true,
                "bounty_contract": "0xabc5000000000000000000000000000000000005",
                "bounty_id": format!("0x{}", "66".repeat(32)),
                "round": 1,
                "verification_mode": "deterministic_module",
                "eligible_verifiers": [],
                "threshold": 1,
                "verification_expires_at": chrono::Utc::now().timestamp() + 7_200,
                "terms": {
                    "policy_hash": format!("0x{}", "77".repeat(32)),
                    "verifier_module": "0x5555555555555555555555555555555555555555"
                },
                "submission_evidence": {
                    "artifact_hash": format!("0x{}", "88".repeat(32)),
                    "evidence_hash": format!("0x{}", "99".repeat(32))
                },
                "evidence_boundary": "Sandbox fixture only; no live deterministic verification job exists."
            }
        ],
        "evidence_boundary": "Sandbox fixtures only; no verification, transaction, settlement, or payment exists."
    })
}

fn sandbox_plan(tool_name: &str, expected_event: &str) -> Value {
    json!({
        "sandbox": true,
        "state": "sandbox_wallet_review",
        "tool": tool_name,
        "transaction_created": false,
        "transaction_broadcast": false,
        "expected_event": expected_event,
        "evidence_boundary": format!(
            "This is a sandbox plan. No signature, transaction, {expected_event} event, settlement, or payment exists."
        )
    })
}

fn mark_sandbox(value: &mut Value, evidence_boundary: &str) {
    if let Some(object) = value.as_object_mut() {
        object.insert("sandbox".to_string(), json!(true));
        object.insert("evidence_boundary".to_string(), json!(evidence_boundary));
    }
}

async fn load_bounty_feed(
    args: ChatgptFeedArgs,
    opportunity_ids: &[String],
) -> Result<Value, String> {
    let mut value = legacy_result(
        list_opportunities(Json(OpportunityListArgs {
            network: args.network,
            view: args.view,
            source_type: args.source_type,
            work_state: args.work_state,
            payment_state: args.payment_state,
            limit: args.limit,
        }))
        .await
        .0,
    )?;
    if !opportunity_ids.is_empty() {
        let items = value
            .get_mut("items")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| "opportunity projection did not contain items".to_string())?;
        items.retain(|item| {
            item.get("opportunity_id")
                .and_then(Value::as_str)
                .is_some_and(|id| opportunity_ids.iter().any(|selected| selected == id))
        });
    }
    let mut value = attach_comments(value).await;
    let state_token = value
        .get("generated_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(object) = value.as_object_mut() {
        object.insert("state_token".to_string(), json!(state_token));
    }
    Ok(value)
}

async fn attach_comments(mut value: Value) -> Value {
    if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(opportunity_id) = item.get("opportunity_id").and_then(Value::as_str) {
                let comments = fetch_comments(opportunity_id)
                    .await
                    .ok()
                    .and_then(|payload| payload.get("comments").cloned())
                    .unwrap_or_else(|| json!([]));
                if let Some(object) = item.as_object_mut() {
                    object.insert("comments".to_string(), comments);
                }
            }
        }
    }
    value
}

async fn fetch_comments(opportunity_id: &str) -> Result<Value, String> {
    let opportunity_id = bounded_opportunity_id(opportunity_id)?;
    let result = proxy_hosted_json(reqwest::Client::new().get(format!(
        "{}/v1/opportunities/{}/comments?limit=100",
        public_base_url_from_env().trim_end_matches('/'),
        opportunity_id
    )))
    .await
    .0;
    legacy_result(result)
}

fn build_share_bundle(args: &ShareBundleArgs) -> Result<Value, String> {
    let bounty_id = bounded_text(&args.bounty_id, "bounty_id", 200)?;
    let title = bounded_text(&args.title, "title", 200)?;
    let stage = bounded_text(&args.stage, "stage", 40)?;
    let status = bounded_text(&args.status, "status", 80)?;
    let bounty_url = safe_share_url(&args.bounty_url)?;
    let reward = args
        .reward
        .as_deref()
        .map(|value| bounded_text(value, "reward", 80))
        .transpose()?;
    let payment_state = args
        .payment_state
        .as_deref()
        .map(|value| bounded_text(value, "payment_state", 80))
        .transpose()?
        .unwrap_or_else(|| "not stated".to_string());
    let reward_copy = reward
        .as_deref()
        .map(|value| format!(" Reward target: {value}."))
        .unwrap_or_default();
    let caption = format!(
        "{stage}: {title}. Status: {status}.{reward_copy} Payment state: {payment_state}. Explore the quest: {bounty_url} #AgentBounties"
    );
    let encoded_caption = encode_component(&caption);
    let encoded_url = encode_component(&bounty_url);
    Ok(json!({
        "schema": "agent-bounties/chatgpt-share-bundle-v1",
        "bounty_id": bounty_id,
        "stage": stage,
        "share_url": bounty_url,
        "caption": caption,
        "hashtags": ["#AgentBounties", "#BuildInPublic", "#AIWork"],
        "intents": {
            "x": format!("https://x.com/intent/post?text={encoded_caption}&url={encoded_url}"),
            "linkedin": format!("https://www.linkedin.com/sharing/share-offsite/?url={encoded_url}"),
            "facebook": format!("https://www.facebook.com/sharer/sharer.php?u={encoded_url}"),
            "instagram": "https://www.instagram.com/".to_string()
        },
        "instagram_caption": caption,
        "evidence_boundary": "This share bundle describes the selected stage only. A transaction hash, planner response, comment, or individual AI output is not canonical funding, verification, settlement, or payment evidence.",
        "next_action": "Copy the caption or open a social share intent, then return to the feed."
    }))
}

fn safe_share_url(value: &str) -> Result<String, String> {
    let value = bounded_text(value, "bounty_url", 12_000)?;
    let parsed =
        Url::parse(&value).map_err(|_| "bounty_url must be a valid public URL".to_string())?;
    let local_http =
        parsed.scheme() == "http" && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1"));
    if parsed.scheme() != "https" && !local_http {
        return Err("bounty_url must use HTTPS (or local HTTP during development)".to_string());
    }
    Ok(parsed.to_string())
}

fn encode_component(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn legacy_result(value: Value) -> Result<Value, String> {
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    value
        .pointer("/content/0/json")
        .cloned()
        .ok_or_else(|| "tool returned an invalid legacy response".to_string())
}

fn tool_result(value: Value, narration: &str, wallet_review: bool) -> Value {
    let structured_content = if value.is_object() {
        value
    } else {
        json!({"items": value})
    };
    let mut result = json!({
        "content": [{"type": "text", "text": narration}],
        "structuredContent": structured_content
    });
    if wallet_review {
        result["_meta"] = json!({
            "handoff_kind": "wallet_review",
            "private_key_requested": false,
            "seed_phrase_requested": false
        });
    }
    result
}

fn tool_error(error: String) -> Value {
    json!({
        "content": [{"type": "text", "text": error}],
        "isError": true
    })
}

fn read_resource(params: &Value) -> Result<Value, String> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| "resources/read requires uri".to_string())?;
    match uri {
        FEED_WIDGET_URI => Ok(json!({"contents": [feed_widget_resource_contents()]})),
        _ => Err("unknown resource URI".to_string()),
    }
}

fn feed_widget_resource_descriptor() -> Value {
    json!({
        "uri": FEED_WIDGET_URI,
        "name": "Agent Bounties live feed",
        "title": "Live bounty feed",
        "description": "An interactive, shareable feed of public bounty opportunities with lifecycle-aware actions.",
        "mimeType": "text/html;profile=mcp-app"
    })
}

fn feed_widget_resource_contents() -> Value {
    json!({
        "uri": FEED_WIDGET_URI,
        "mimeType": "text/html;profile=mcp-app",
        "text": feed_widget_html(),
        "_meta": {
            "ui": {
                "prefersBorder": false,
                "domain": "https://mcp.agentbounties.app",
                "csp": {
                    "connectDomains": [],
                    "resourceDomains": []
                }
            },
            "openai/widgetDescription": "An Instagram-inspired interactive bounty feed. Each item is rendered as a complete Pokémon-card-style quest card with public status, rewards, funding state, evidence boundary, comments, lifecycle action, and a share step.",
            "openai/widgetPrefersBorder": false,
            "openai/widgetDomain": "https://mcp.agentbounties.app",
            "openai/widgetCSP": {
                "connect_domains": [],
                "resource_domains": [],
                "redirect_domains": ["https://agentbounties.app", "https://x.com", "https://www.linkedin.com", "https://www.instagram.com"]
            }
        }
    })
}

fn feed_widget_html() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(FEED_CARD_ART);
    FEED_WIDGET_HTML.replace(
        "__BOUNTY_CARD_ART_DATA_URI__",
        &format!("data:image/webp;base64,{encoded}"),
    )
}

fn post_handoff_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema": {"type": "string"},
            "state": {"type": "string"},
            "title": {"type": "string"},
            "goal": {"type": "string"},
            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
            "solver_reward_usdc": {"type": "string"},
            "verifier_reward_usdc": {"type": "string"},
            "target_usdc": {"type": "string"},
            "initial_funding_usdc": {"type": "string"},
            "crowdfund": {"type": "boolean"},
            "source_url": {"type": ["string", "null"]},
            "post_url": {"type": "string"},
            "bounty_created": {"type": "boolean"},
            "wallet_signature_requested": {"type": "boolean"},
            "next_action": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["schema", "state", "title", "goal", "acceptance_criteria", "solver_reward_usdc", "verifier_reward_usdc", "target_usdc", "initial_funding_usdc", "crowdfund", "post_url", "bounty_created", "wallet_signature_requested", "next_action", "evidence_boundary"],
        "additionalProperties": false
    })
}

fn feed_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "generated_at": {"type": "string"},
            "network": {"type": "string"},
            "items": {"type": "array", "items": {"type": "object"}},
            "degraded": {"type": "boolean"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["schema_version", "generated_at", "network", "items", "degraded", "evidence_boundary"],
        "additionalProperties": true
    })
}

fn bounty_action_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "intent_id": {"type": "string", "format": "uuid"},
            "action": {"type": "string", "enum": ["post", "fund", "compete", "complete", "verify"]},
            "status": {"type": "string", "enum": ["review_required", "pending_confirmation", "confirmed", "failed", "expired"]},
            "network": {"type": "string"},
            "opportunity_id": {"type": ["string", "null"]},
            "bounty_contract": {"type": ["string", "null"]},
            "bounty_id": {"type": ["string", "null"]},
            "actor_wallet": {"type": ["string", "null"]},
            "amount_base_units": {"type": ["integer", "null"]},
            "details": {"type": "object"},
            "authorization_url": {"type": "string"},
            "expected_canonical_events": {"type": "array", "items": {"type": "string"}},
            "transaction_hash": {"type": ["string", "null"]},
            "canonical_event_id": {"type": ["string", "null"], "format": "uuid"},
            "canonical_event_kind": {"type": ["string", "null"]},
            "confirmed_block": {"type": ["integer", "null"]},
            "paid": {"type": "boolean"},
            "expires_at": {"type": "string"},
            "share_after": {"type": "boolean"},
            "next_action": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "schema_version", "intent_id", "action", "status", "network",
            "authorization_url", "expected_canonical_events", "paid", "expires_at",
            "share_after", "next_action", "evidence_boundary"
        ],
        "additionalProperties": true
    })
}

fn bounty_comments_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "opportunity_id": {"type": "string"},
            "bounty_id": {"type": "string"},
            "comments": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "opportunity_id": {"type": "string"},
                        "author": {"type": "string"},
                        "body": {"type": "string"},
                        "created_at": {"type": "string"}
                    },
                    "required": ["id", "author", "body", "created_at"],
                    "additionalProperties": true
                }
            },
            "comment_count": {"type": "integer"},
            "published": {"type": "boolean"},
            "share_after": {"type": "boolean"},
            "sandbox": {"type": "boolean"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["comments", "comment_count", "evidence_boundary"],
        "additionalProperties": true
    })
}

fn share_bundle_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema": {"type": "string"},
            "bounty_id": {"type": "string"},
            "stage": {"type": "string"},
            "share_url": {"type": "string"},
            "caption": {"type": "string"},
            "hashtags": {"type": "array", "items": {"type": "string"}},
            "intents": {
                "type": "object",
                "properties": {
                    "x": {"type": "string"},
                    "linkedin": {"type": "string"},
                    "facebook": {"type": "string"},
                    "instagram": {"type": "string"}
                },
                "required": ["x", "linkedin", "facebook", "instagram"],
                "additionalProperties": false
            },
            "instagram_caption": {"type": "string"},
            "sandbox": {"type": "boolean"},
            "evidence_boundary": {"type": "string"},
            "next_action": {"type": "string"}
        },
        "required": [
            "schema", "bounty_id", "stage", "share_url", "caption", "hashtags",
            "intents", "instagram_caption", "evidence_boundary", "next_action"
        ],
        "additionalProperties": true
    })
}

fn objective_plan_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "provider": {"type": "string"},
            "model": {"type": "string"},
            "title": {"type": "string"},
            "objective": {"type": "string"},
            "success_definition": {"type": "string"},
            "tasks": {"type": "array", "items": {"type": "object"}},
            "parallel_layers": {
                "type": "array",
                "items": {"type": "array", "items": {"type": "string"}}
            },
            "solver_budget_usdc": {"type": ["string", "null"]},
            "execution_policy": {"type": "object"},
            "verification_policy": {"type": "object"},
            "settlement_policy": {"type": "object"},
            "questions": {"type": "array", "items": {"type": "string"}},
            "risk_flags": {"type": "array", "items": {"type": "string"}},
            "source_url": {"type": ["string", "null"]},
            "published": {"type": "boolean"},
            "sandbox": {"type": "boolean"},
            "next_action": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["objective", "tasks", "evidence_boundary"],
        "additionalProperties": true
    })
}

fn chatgpt_tool_description(name: &str, fallback: &'static str) -> &'static str {
    match name {
        "get_bounty_feed" => "Use this when the model or mounted feed needs fresh structured bounty data without rendering another widget. It is read-only; use render_bounty_feed only when the person wants the interactive feed shown.",
        "render_bounty_feed" => "Use this when the person wants the interactive Agent Bounties feed rendered inside ChatGPT. For model-selected results, inspect get_bounty_feed first and pass only the chosen opportunity_ids.",
        "prepare_bounty_action" => "Use this when the person wants to post, fund, compete, complete, or verify from an in-chat card. Create one idempotent first-party authorization session; never request a wallet or verifier signature in ChatGPT and never describe prepared status as completion.",
        "get_bounty_action_status" => "Use this when the person returns from first-party authorization and the card needs canonical status. Confirmed requires the exact indexed action-specific event; only BountySettled proves solver payment.",
        "fund_bounty_with_x402" => "Use this when the person explicitly wants to fund one canonical Base bounty. Request the x402 challenge first, replay only with the exact wallet-signed authorization, and do not call a challenge, signature, relay, or transaction hash funding evidence.",
        "get_x402_relay_status" => "Use this when an earlier x402 funding response returned a relay_id that still needs canonical confirmation. Only a matching confirmed FundingAdded event changes funding state.",
        "prepare_agent_to_earn" => "Use this when the person has selected one funded claimable bounty and supplied a public Base wallet. Check wallet policy, bond, claimability, and verification readiness without requesting a secret or changing state.",
        "agent_native_claim" => "Use this when the person explicitly wants to compete for one funded verification-ready bounty. Reuse one idempotency key, request at most the exact bounded wallet signature returned by the tool, and replay until BountyClaimed is confirmed.",
        "plan_autonomous_bounty_claim" => "Use this when the hosted claim relay is unavailable and the person wants the direct wallet fallback. It prepares bond-and-claim calls only; it does not claim the bounty.",
        "prepare_autonomous_bounty_submission" => "Use this when the active solver has completed a claimed bounty and wants to commit a public artifact and evidence. Preparation is not SubmissionAdded, verification, settlement, or payment.",
        "publish_autonomous_submission_evidence" => "Use this when confirmed SubmissionAdded exists and the solver wants to publish the exact artifact and evidence preimages matching the canonical commitments. This public write is not verification or payout proof.",
        "list_autonomous_verification_jobs" => "Use this when the person wants to verify submitted work. Return only indexed jobs whose committed verifier path is ready; do not invent a verifier mode or verdict.",
        "plan_autonomous_verification_attestation" => "Use this when a committed quorum verifier has evaluated the current indexed submission and needs the exact EIP-712 attestation payload. The plan and signature are not settlement evidence.",
        "plan_autonomous_module_settlement" => "Use this when the loaded verification job commits a deterministic module and the person wants its permissionless verifier transaction. A plan, broadcast, or transaction hash is not settlement or payment evidence.",
        "plan_autonomous_attestation_settlement" => "Use this when enough matching signatures from the precommitted verifier quorum are available and the person wants the permissionless settlement transaction. Only confirmed BountySettled proves solver payment.",
        "get_paid_status" => "Use this when the person asks whether a bounty solver was paid. Report paid language only for reconciled rail evidence; for autonomous-v1 this requires a matching confirmed BountySettled event.",
        "compile_objective_with_cloud_agent" => "Use this when the person has a broad digital objective that should be broken into smaller independently reviewable bounty drafts. The model output is advisory and creates, funds, claims, verifies, or settles nothing.",
        "list_bounty_comments" => "Use this when the person or mounted feed needs recent public comments for one bounty. Comments are conversation context and never funding, verification, settlement, or payment evidence.",
        "add_bounty_comment" => "Use this when the person explicitly wants to publish a bounded public comment on one bounty. Do not include secrets or restricted personal data; a comment changes no canonical lifecycle state.",
        "create_share_bundle" => "Use this when the person wants a factual social-ready caption and card intent after a bounty step. Sharing is optional and changes no funding, claim, verification, settlement, or payment state.",
        "publish_unfunded_bounty" => "Use this when the person explicitly wants to publish a public voluntary request with no wallet and zero committed USDC. It is not canonical, funded, claimable, or guaranteed to pay.",
        "list_unfunded_bounties" => "Use this when the person explicitly asks for voluntary or unpaid Agent Bounties work. Keep these records separate from funded earning opportunities and never promise payment.",
        "submit_unfunded_bounty_solution" => "Use this when a registered agent explicitly wants to publish or replace its public solution to an open unfunded request. This public write creates no payment claim.",
        "prepare_bounty_post" => "Use this when the person wants something done with a funded or crowdfunded canonical bounty. Prepare a reviewable wallet handoff only; move no funds, request no secret, and do not claim the bounty exists yet.",
        "list_autonomous_bounties" => "Use this when the person wants funded Agent Bounties work or canonical lifecycle inventory. Set claimable_only=true for work that is currently funded and ready to compete for.",
        _ => fallback,
    }
}

fn tool_title(name: &str) -> &'static str {
    match name {
        "get_bounty_feed" => "Refresh bounty feed data",
        "render_bounty_feed" => "Open live bounty feed",
        "prepare_bounty_action" => "Prepare secure bounty action",
        "get_bounty_action_status" => "Check canonical action status",
        "fund_bounty_with_x402" => "Request funding challenge",
        "get_x402_relay_status" => "Check funding relay",
        "prepare_agent_to_earn" => "Check claim readiness",
        "agent_native_claim" => "Compete for bounty",
        "plan_autonomous_bounty_claim" => "Prepare claim path",
        "prepare_autonomous_bounty_submission" => "Prepare completion evidence",
        "publish_autonomous_submission_evidence" => "Publish completion evidence",
        "list_autonomous_verification_jobs" => "Load verification job",
        "plan_autonomous_verification_attestation" => "Prepare verification path",
        "plan_autonomous_module_settlement" => "Prepare deterministic verification",
        "plan_autonomous_attestation_settlement" => "Prepare quorum settlement",
        "get_paid_status" => "Check reconciled payout",
        "compile_objective_with_cloud_agent" => "Break objective into bounties",
        "list_bounty_comments" => "Read bounty comments",
        "add_bounty_comment" => "Comment on bounty",
        "create_share_bundle" => "Prepare share card",
        "publish_unfunded_bounty" => "Publish no-wallet bounty",
        "list_unfunded_bounties" => "List unfunded bounties",
        "submit_unfunded_bounty_solution" => "Submit unfunded bounty solution",
        "prepare_bounty_post" => "Prepare bounty for wallet review",
        "list_autonomous_bounties" => "List canonical bounties",
        _ => "Agent Bounties tool",
    }
}

fn bounded_text(value: &str, field: &str, max_chars: usize) -> Result<String, String> {
    let value = value.trim();
    let count = value.chars().count();
    if count == 0 {
        return Err(format!("{field} must not be empty"));
    }
    if count > max_chars {
        return Err(format!(
            "{field} must contain at most {max_chars} characters"
        ));
    }
    Ok(value.to_string())
}

fn bounded_public_key(
    value: &str,
    field: &str,
    minimum: usize,
    maximum: usize,
) -> Result<String, String> {
    let value = value.trim();
    let count = value.chars().count();
    if count < minimum
        || count > maximum
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
        })
    {
        return Err(format!(
            "{field} must contain {minimum}..{maximum} public identifier characters"
        ));
    }
    Ok(value.to_string())
}

fn bounded_opportunity_id(value: &str) -> Result<String, String> {
    let value = bounded_text(value, "bounty_id", 200)?;
    if !value.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '_' | '-')
    }) {
        return Err(
            "bounty_id must contain only public opportunity identifier characters".to_string(),
        );
    }
    Ok(value)
}

fn optional_https_url(value: Option<&str>, field: &str) -> Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed =
        Url::parse(value).map_err(|_| format!("{field} must be a valid public HTTPS URL"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(format!("{field} must be a valid public HTTPS URL"));
    }
    Ok(Some(parsed.to_string()))
}

fn parse_usdc(value: &str, field: &str) -> Result<u64, String> {
    let value = value.trim();
    let mut parts = value.split('.');
    let whole = parts
        .next()
        .filter(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        .ok_or_else(|| format!("{field} must be a positive USDC decimal with at most 6 places"))?;
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some()
        || fraction.len() > 6
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(format!(
            "{field} must be a positive USDC decimal with at most 6 places"
        ));
    }
    let whole = whole
        .parse::<u64>()
        .map_err(|_| format!("{field} is too large"))?;
    if whole > 1_000_000 {
        return Err(format!("{field} must not exceed 1000000 USDC"));
    }
    let mut padded = fraction.to_string();
    padded.push_str(&"0".repeat(6 - padded.len()));
    let fraction = padded.parse::<u64>().unwrap_or(0);
    let amount = whole
        .checked_mul(1_000_000)
        .and_then(|amount| amount.checked_add(fraction))
        .ok_or_else(|| format!("{field} is too large"))?;
    if amount == 0 {
        return Err(format!("{field} must be greater than zero"));
    }
    Ok(amount)
}

fn format_usdc(amount: u64) -> String {
    let whole = amount / 1_000_000;
    let fraction = amount % 1_000_000;
    if fraction == 0 {
        return whole.to_string();
    }
    format!("{whole}.{fraction:06}")
        .trim_end_matches('0')
        .to_string()
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_args() -> PrepareBountyPostArgs {
        PrepareBountyPostArgs {
            title: "Fix the reconciliation regression".to_string(),
            goal: "Make the committed regression test pass.".to_string(),
            acceptance_criteria: vec![
                "The committed test exits zero.".to_string(),
                "A regression test covers the prior failure.".to_string(),
            ],
            solver_reward_usdc: "2.00".to_string(),
            verifier_reward_usdc: "0.10".to_string(),
            source_url: Some("https://github.com/NSPG13/agent-bounties/issues/386".to_string()),
            crowdfund: false,
            task_window_days: None,
            discovery_source: Some("ChatGPT user feedback".to_string()),
        }
    }

    #[test]
    fn handoff_is_prefilled_but_never_claims_creation_or_signature() {
        let handoff = build_bounty_post_handoff(&valid_args()).unwrap();
        let post_url = Url::parse(handoff["post_url"].as_str().unwrap()).unwrap();
        let pairs = post_url.query_pairs().collect::<Vec<_>>();

        assert_eq!(handoff["state"], "review_required_not_published");
        assert_eq!(handoff["target_usdc"], "2.1");
        assert_eq!(handoff["initial_funding_usdc"], "2.1");
        assert_eq!(handoff["bounty_created"], false);
        assert_eq!(handoff["wallet_signature_requested"], false);
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "title" && value == "Fix the reconciliation regression"));
        assert_eq!(
            pairs.iter().filter(|(key, _)| key == "criterion").count(),
            2
        );
    }

    #[test]
    fn handoff_rejects_non_https_sources_and_invalid_money() {
        let mut args = valid_args();
        args.source_url = Some("http://example.com/private".to_string());
        assert!(build_bounty_post_handoff(&args)
            .unwrap_err()
            .contains("HTTPS"));

        args.source_url = None;
        args.solver_reward_usdc = "0".to_string();
        assert!(build_bounty_post_handoff(&args)
            .unwrap_err()
            .contains("greater than zero"));
    }

    #[test]
    fn public_resource_catalog_exposes_only_the_live_feed_widget() {
        let descriptor = feed_widget_resource_descriptor();
        assert_eq!(descriptor["uri"], FEED_WIDGET_URI);
        assert_eq!(
            read_resource(&json!({"uri": FEED_WIDGET_URI})).unwrap()["contents"][0]["uri"],
            FEED_WIDGET_URI
        );
        assert!(read_resource(&json!({"uri": POST_WIDGET_URI}))
            .unwrap_err()
            .contains("unknown resource URI"));
    }

    #[tokio::test]
    async fn app_tools_have_required_annotations_and_widget_metadata() {
        let tools = chatgpt_tools().await;
        for tool in &tools {
            assert!(
                tool["description"]
                    .as_str()
                    .is_some_and(|description| description.starts_with("Use this when")),
                "{} must tell ChatGPT when to choose it",
                tool["name"]
            );
            assert_eq!(
                tool["_meta"]["ui"]["visibility"],
                json!(["model", "app"]),
                "{} must be callable by the model and mounted widget",
                tool["name"]
            );
            assert_eq!(
                tool["outputSchema"]["type"], "object",
                "{} must declare its structured output contract",
                tool["name"]
            );
        }
        let prepare = tools
            .iter()
            .find(|tool| tool["name"] == "prepare_bounty_action")
            .expect("hosted action preparation tool");
        let status = tools
            .iter()
            .find(|tool| tool["name"] == "get_bounty_action_status")
            .expect("canonical action status tool");

        assert_eq!(prepare["annotations"]["readOnlyHint"], false);
        assert_eq!(prepare["annotations"]["destructiveHint"], false);
        assert_eq!(prepare["annotations"]["openWorldHint"], true);
        assert_eq!(prepare["annotations"]["idempotentHint"], true);
        assert_eq!(
            prepare["outputSchema"]["properties"]["status"]["enum"],
            json!([
                "review_required",
                "pending_confirmation",
                "confirmed",
                "failed",
                "expired"
            ])
        );
        assert!(prepare["description"]
            .as_str()
            .unwrap()
            .starts_with("Use this when"));
        assert_eq!(status["annotations"]["readOnlyHint"], true);
        assert_eq!(status["annotations"]["destructiveHint"], false);
        assert_eq!(status["annotations"]["openWorldHint"], false);
        assert_eq!(status["annotations"]["idempotentHint"], true);

        let feed = tools
            .iter()
            .find(|tool| tool["name"] == "get_bounty_feed")
            .expect("feed tool");
        assert_eq!(feed["annotations"]["readOnlyHint"], true);
        assert!(feed["_meta"]["ui"].get("resourceUri").is_none());

        let render_feed = tools
            .iter()
            .find(|tool| tool["name"] == "render_bounty_feed")
            .expect("render feed tool");
        assert_eq!(render_feed["annotations"]["readOnlyHint"], true);
        assert_eq!(render_feed["_meta"]["ui"]["resourceUri"], FEED_WIDGET_URI);
        assert_eq!(
            render_feed["_meta"]["openai/outputTemplate"],
            FEED_WIDGET_URI
        );
        assert_eq!(
            render_feed["_meta"]["ui"]["visibility"],
            json!(["model", "app"])
        );
        assert_eq!(
            render_feed["outputSchema"]["required"],
            json!([
                "schema_version",
                "generated_at",
                "network",
                "items",
                "degraded",
                "evidence_boundary"
            ])
        );

        for name in [
            "prepare_bounty_action",
            "get_bounty_action_status",
            "compile_objective_with_cloud_agent",
            "list_bounty_comments",
            "add_bounty_comment",
            "create_share_bundle",
        ] {
            assert!(
                tools.iter().any(|tool| tool["name"] == name),
                "missing public ChatGPT app tool: {name}"
            );
        }
        for forbidden in [
            "fund_bounty_with_x402",
            "agent_native_claim",
            "plan_autonomous_bounty_claim",
            "prepare_autonomous_bounty_submission",
            "plan_autonomous_module_settlement",
            "plan_autonomous_attestation_settlement",
            "prepare_bounty_post",
        ] {
            assert!(
                tools.iter().all(|tool| tool["name"] != forbidden),
                "direct wallet or settlement tool leaked into the public ChatGPT surface: {forbidden}"
            );
        }

        let comment = tools
            .iter()
            .find(|tool| tool["name"] == "add_bounty_comment")
            .expect("comment tool");
        assert_eq!(comment["annotations"]["readOnlyHint"], false);
        assert_eq!(comment["annotations"]["destructiveHint"], true);
        assert_eq!(comment["annotations"]["idempotentHint"], false);
    }

    #[test]
    fn feed_resource_is_embedded_as_an_interactive_mcp_app() {
        let contents = feed_widget_resource_contents();
        let html = contents["text"].as_str().unwrap();
        assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
        assert_eq!(contents["uri"], FEED_WIDGET_URI);
        assert_eq!(
            contents["_meta"]["openai/widgetCSP"]["redirect_domains"],
            json!([
                "https://agentbounties.app",
                "https://x.com",
                "https://www.linkedin.com",
                "https://www.instagram.com"
            ])
        );
        assert!(html.contains("class=\"art-frame\""));
        for tool_name in [
            "get_bounty_feed",
            "add_bounty_comment",
            "create_share_bundle",
            "compile_objective_with_cloud_agent",
            "prepare_bounty_action",
            "get_bounty_action_status",
        ] {
            assert!(
                html.contains(tool_name),
                "feed widget must call {tool_name} through the host bridge"
            );
        }
        for forbidden in [
            "fund_bounty_with_x402",
            "agent_native_claim",
            "wallet_signature",
            "PAYMENT-SIGNATURE",
            "plan_autonomous_module_settlement",
            "plan_autonomous_attestation_settlement",
        ] {
            assert!(
                !html.contains(forbidden),
                "feed widget must keep {forbidden} outside ChatGPT"
            );
        }
        for visible_control in [
            "data-action=\"post\"",
            "data-action=\"breakdown\"",
            "data-action=\"comment\"",
            "data-action=\"share\"",
            "data-action=\"download-card\"",
            "data-action=\"social-instagram\"",
            "Post a bounty in chat",
            "Break down a large objective",
        ] {
            assert!(
                html.contains(visible_control),
                "feed widget must render {visible_control}"
            );
        }
        assert!(html.contains("data:image/webp;base64,"));
        assert!(!html.contains("__BOUNTY_CARD_ART_DATA_URI__"));
        assert!(html.contains("bridgeRequest(\"tools/call\""));
        assert!(!html.contains("window.location"));
        assert!(html.contains("catch (clipboardError)"));
        assert!(html.contains("document.execCommand(\"copy\")"));
        assert!(html.contains("Sandbox · no writes"));
        assert!(html.contains("safe fixture data"));
        assert!(html.contains("#b9ef37"));
        assert!(html.contains("#18d9ac"));
        assert!(html.contains("#e8bd26"));
    }

    #[test]
    fn mounted_feed_result_does_not_claim_wallet_handoff() {
        let feed_result = tool_result(json!({"items": []}), "Rendered the live feed.", false);
        assert!(feed_result.get("_meta").is_none());

        let post_result = tool_result(
            json!({"state": "review_required_not_published"}),
            "Prepared wallet review.",
            true,
        );
        assert_eq!(post_result["_meta"]["handoff_kind"], "wallet_review");
        assert_eq!(post_result["_meta"]["private_key_requested"], false);
        assert_eq!(post_result["_meta"]["seed_phrase_requested"], false);
    }

    #[test]
    fn share_bundle_is_bounded_and_keeps_payment_evidence_explicit() {
        let args = ShareBundleArgs {
            bounty_id: "bounty-42".to_string(),
            title: "Ship the verifier dashboard".to_string(),
            stage: "verification".to_string(),
            bounty_url: "https://agentbounties.app/bounties/bounty-42".to_string(),
            status: "submitted; awaiting verifier".to_string(),
            reward: Some("12 USDC".to_string()),
            payment_state: Some("escrowed".to_string()),
        };
        let bundle = build_share_bundle(&args).unwrap();
        assert_eq!(bundle["stage"], "verification");
        assert!(bundle["caption"]
            .as_str()
            .unwrap()
            .contains("#AgentBounties"));
        assert!(bundle["intents"]["x"]
            .as_str()
            .unwrap()
            .contains("intent/post"));
        assert!(bundle["evidence_boundary"]
            .as_str()
            .unwrap()
            .contains("not canonical"));

        let mut unsafe_args = args;
        unsafe_args.bounty_url = "javascript:alert(1)".to_string();
        assert!(build_share_bundle(&unsafe_args).is_err());
    }

    #[tokio::test]
    async fn sandbox_descriptors_are_read_only_and_explicit() {
        let mut descriptors = tools().await.0;
        descriptors.extend(custom_tool_descriptors());
        let sandbox_tools = descriptors
            .into_iter()
            .filter(|descriptor| CHATGPT_TOOL_NAMES.contains(&descriptor.name))
            .map(|descriptor| mcp_tool_descriptor_for_mode(descriptor, true))
            .collect::<Vec<_>>();

        assert_eq!(sandbox_tools.len(), CHATGPT_TOOL_NAMES.len());
        for tool in &sandbox_tools {
            assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
            assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool}");
            assert_eq!(tool["annotations"]["openWorldHint"], false, "{tool}");
            assert_eq!(tool["annotations"]["idempotentHint"], true, "{tool}");
            assert_eq!(tool["_meta"]["agentBountiesSandbox"], true, "{tool}");
            assert!(
                tool["description"]
                    .as_str()
                    .unwrap()
                    .contains("Sandbox mode is active"),
                "{tool}"
            );
        }
    }

    #[tokio::test]
    async fn sandbox_lifecycle_contracts_never_claim_external_writes() {
        let render = sandbox_tool_result("render_bounty_feed", &json!({}))
            .await
            .unwrap();
        assert_eq!(render["structuredContent"]["sandbox"], true);
        assert_eq!(
            render["structuredContent"]["items"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let cards = render["structuredContent"]["items"].as_array().unwrap();
        assert!(cards.iter().any(|item| item["work_state"] == "in_progress"));
        assert!(cards.iter().any(|item| {
            item["work_state"] == "submitted"
                && item["verification_method"] == "deterministic module"
        }));

        let comment = sandbox_tool_result(
            "add_bounty_comment",
            &json!({
                "bounty_id": "canonical_base:base-mainnet:0xabc1000000000000000000000000000000000001",
                "body": "Safe sandbox comment",
                "author": "tester",
                "comment_id": Uuid::new_v4()
            }),
        )
        .await
        .unwrap();
        assert_eq!(comment["structuredContent"]["published"], false);
        assert_eq!(comment["structuredContent"]["sandbox"], true);

        for action in ["post", "fund", "compete", "complete", "verify"] {
            let prepared = sandbox_tool_result(
                "prepare_bounty_action",
                &json!({
                    "idempotency_key": format!("sandbox-{action}-flow"),
                    "action": action,
                    "network": "base-mainnet",
                    "opportunity_id": "canonical_base:base-mainnet:0xabc1000000000000000000000000000000000001",
                    "bounty_contract": if action == "post" {
                        Value::Null
                    } else {
                        json!("0xabc1000000000000000000000000000000000001")
                    },
                    "bounty_id": null,
                    "actor_wallet": null,
                    "amount_base_units": if action == "fund" { json!(1_000_000) } else { Value::Null },
                    "details": {}
                }),
            )
            .await
            .unwrap();
            let intent = &prepared["structuredContent"];
            assert_eq!(intent["sandbox"], true, "{action}");
            assert_eq!(intent["status"], "review_required", "{action}");
            assert_eq!(intent["transaction_hash"], Value::Null, "{action}");
            assert_eq!(intent["canonical_event_id"], Value::Null, "{action}");
            assert_eq!(intent["paid"], false, "{action}");

            let refreshed = sandbox_tool_result(
                "get_bounty_action_status",
                &json!({"intent_id": intent["intent_id"]}),
            )
            .await
            .unwrap();
            assert_eq!(refreshed["structuredContent"]["status"], "confirmed");
            assert_eq!(
                refreshed["structuredContent"]["canonical_event_id"],
                Value::Null
            );
            assert_eq!(refreshed["structuredContent"]["paid"], false);
        }
    }

    #[test]
    fn sandbox_feed_filters_fixture_cards_without_network_access() {
        let seeking_funding = sandbox_bounty_feed(
            ChatgptFeedArgs {
                network: Some("base-mainnet".to_string()),
                view: Some("seeking_funding".to_string()),
                source_type: None,
                work_state: None,
                payment_state: Some("seeking_funding".to_string()),
                limit: Some(30),
            },
            &[],
        )
        .unwrap();
        let items = seeking_funding["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["payment_state"], "seeking_funding");
        assert_eq!(seeking_funding["sandbox"], true);
    }

    #[test]
    fn call_tool_structured_content_is_always_an_object() {
        let result = tool_result(json!([{"id": 1}]), "List result", false);
        assert!(result["structuredContent"].is_object());
        assert_eq!(result["structuredContent"]["items"][0]["id"], 1);
    }

    #[test]
    fn comment_identifier_is_path_safe_before_hosted_request() {
        assert_eq!(
            bounded_opportunity_id("canonical:base-mainnet:0xabc").unwrap(),
            "canonical:base-mainnet:0xabc"
        );
        assert!(bounded_opportunity_id("bad/id").is_err());
        assert!(bounded_opportunity_id(" ").is_err());
    }
}
