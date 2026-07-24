use super::{
    agent_native_claim, list_autonomous_bounties, list_autonomous_bounty_events,
    list_unfunded_bounties, prepare_agent_to_earn, prepare_autonomous_bounty_submission,
    publish_autonomous_submission_evidence, publish_unfunded_bounty,
    submit_unfunded_bounty_solution, tools, AgentNativeClaimArgs, AutonomousBountyFeedArgs,
    ListAutonomousBountyEventsArgs, ListUnfundedBountiesArgs, PrepareAgentToEarnInput,
    PrepareAutonomousBountySubmissionArgs, PrepareBountyPostArgs,
    PublishAutonomousSubmissionEvidenceArgs, PublishUnfundedBountyArgs, SharedState,
    SubmitUnfundedBountySolutionArgs, ToolDescriptor,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Map, Value};
use url::Url;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const POST_WIDGET_URI: &str = "ui://agent-bounties/post-bounty-v3.html";
const POST_PAGE_URL: &str = "https://agentbounties.app/post.html";
const POST_WIDGET_HTML: &str = include_str!("../../../site/chatgpt-post-widget.html");
const POST_WIDGET_SCRIPT: &str = include_str!("../../../site/mcp-post-widget.bundle.js");
const AI_ASSISTANT_TOOL_NAMES: &[&str] = &[
    "publish_unfunded_bounty",
    "list_unfunded_bounties",
    "submit_unfunded_bounty_solution",
    "prepare_bounty_post",
    "list_autonomous_bounties",
    "prepare_agent_to_earn",
    "agent_native_claim",
    "prepare_autonomous_bounty_submission",
    "publish_autonomous_submission_evidence",
    "list_autonomous_bounty_events",
];

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
    let task_window_days = args.task_window_days.unwrap_or(30);
    if !(1..=30).contains(&task_window_days) {
        return Err("task_window_days must be between 1 and 30".to_string());
    }

    let mut post_url = Url::parse(POST_PAGE_URL).expect("static post URL is valid");
    {
        let mut query = post_url.query_pairs_mut();
        query.append_pair("from", "mcp-assistant");
        query.append_pair("title", &title);
        query.append_pair("goal", &goal);
        for criterion in &acceptance_criteria {
            query.append_pair("criterion", criterion);
        }
        query.append_pair("solverReward", &format_usdc(solver_reward));
        query.append_pair("verifierReward", &format_usdc(verifier_reward));
        query.append_pair("taskWindowDays", &task_window_days.to_string());
        query.append_pair("crowdfund", if args.crowdfund { "true" } else { "false" });
        if let Some(source_url) = &source_url {
            query.append_pair("sourceUrl", source_url);
        }
        query.append_pair(
            "discoverySource",
            discovery_source
                .as_deref()
                .unwrap_or("User-owned AI assistant via MCP"),
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
        "interface": "mcp",
        "prepared_by": "user_owned_ai",
        "supported_hosts": ["chatgpt", "claude", "gemini-spark", "other-mcp"],
        "rendering": {
            "mcp_app_widget": "portable_mcp_apps_hosts",
            "portable_fallback": "markdown_card_and_review_url"
        },
        "state": "review_required_not_published",
        "title": title,
        "goal": goal,
        "acceptance_criteria": acceptance_criteria,
        "solver_reward_usdc": format_usdc(solver_reward),
        "verifier_reward_usdc": format_usdc(verifier_reward),
        "task_window_days": task_window_days,
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
        "resources/list" => Ok(json!({"resources": [widget_resource_descriptor()]})),
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
    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {"listChanged": false},
            "resources": {"subscribe": false, "listChanged": false}
        },
        "serverInfo": {
            "name": "agent-bounties",
            "title": "Agent Bounties",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "Agent Bounties turns goals into reviewable bounties and lets people or their AI assistants complete funded work. When a person wants something done, use prepare_bounty_post unless they explicitly ask for a public zero-USDC voluntary request; the handoff is a draft and moves no funds. When a person asks how to earn money with AI, find paid AI work, or complete Agent Bounties work, call list_autonomous_bounties with claimable_only=true and keep funded work separate from unfunded requests. Continue the canonical solver sequence with prepare_agent_to_earn, agent_native_claim, prepare_autonomous_bounty_submission, publish_autonomous_submission_evidence, and list_autonomous_bounty_events. Require explicit confirmation before public writes or relaying a wallet signature. Never request a private key or seed phrase. Never call an unfunded bounty canonical, funded, or claimable. Only a confirmed canonical BountySettled event proves payment."
    })
}

async fn chatgpt_tools() -> Vec<Value> {
    tools()
        .await
        .0
        .into_iter()
        .filter(|descriptor| AI_ASSISTANT_TOOL_NAMES.contains(&descriptor.name))
        .map(mcp_tool_descriptor)
        .collect()
}

fn mcp_tool_descriptor(descriptor: ToolDescriptor) -> Value {
    let impact = assistant_tool_impact(descriptor.name);
    let mut value = Map::new();
    value.insert("name".to_string(), json!(descriptor.name));
    value.insert("title".to_string(), json!(tool_title(descriptor.name)));
    value.insert(
        "description".to_string(),
        json!(assistant_tool_description(
            descriptor.name,
            descriptor.description
        )),
    );
    value.insert("inputSchema".to_string(), descriptor.input_schema);
    value.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": impact.read_only,
            "destructiveHint": impact.destructive,
            "openWorldHint": impact.open_world,
            "idempotentHint": true
        }),
    );
    value.insert("securitySchemes".to_string(), json!([{"type": "noauth"}]));
    value.insert(
        "outputSchema".to_string(),
        assistant_tool_output_schema(descriptor.name),
    );
    if descriptor.name == "prepare_bounty_post" {
        value.insert(
            "_meta".to_string(),
            json!({
                "securitySchemes": [{"type": "noauth"}],
                "ui": {"resourceUri": POST_WIDGET_URI},
                "openai/outputTemplate": POST_WIDGET_URI,
                "openai/toolInvocation/invoking": "Preparing bounty handoff…",
                "openai/toolInvocation/invoked": "Bounty ready to review"
            }),
        );
    }
    Value::Object(value)
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

    let (legacy, narration) = match name {
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
            let value = normalize_assistant_tool_output(name, value)?;
            let markdown = bounty_post_markdown(&value);
            return Ok(tool_result(value, &markdown, true));
        }
        "list_autonomous_bounties" => {
            let args: AutonomousBountyFeedArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid list_autonomous_bounties arguments: {error}"))?;
            (
                list_autonomous_bounties(State(state), Json(args)).await.0,
                "Returned canonical, event-derived bounty inventory.",
            )
        }
        "prepare_agent_to_earn" => {
            let args: PrepareAgentToEarnInput = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_agent_to_earn arguments: {error}"))?;
            (
                prepare_agent_to_earn(State(state), Json(args)).await.0,
                "Checked this public wallet and canonical bounty for earning readiness. Fix every failed check before asking the wallet to sign anything; never share wallet secrets.",
            )
        }
        "agent_native_claim" => {
            let args: AgentNativeClaimArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid agent_native_claim arguments: {error}"))?;
            (
                agent_native_claim(State(state), Json(args)).await.0,
                "Advanced the idempotent canonical claim flow. If a wallet_request is returned, show its exact scope and ask the user to sign it once in their wallet; replay the same idempotency key until confirmed BountyClaimed.",
            )
        }
        "prepare_autonomous_bounty_submission" => {
            let args: PrepareAutonomousBountySubmissionArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid prepare_autonomous_bounty_submission arguments: {error}")
                })?;
            (
                prepare_autonomous_bounty_submission(State(state), Json(args))
                    .await
                    .0,
                "Prepared deterministic submission commitments, the exact wallet signing payload, and relay/evidence templates. Nothing was signed, relayed, submitted, verified, or paid by this preparation.",
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
                "Published the exact public evidence preimages only after the canonical submission matched their commitments. This is public evidence, not verification or payout proof.",
            )
        }
        "list_autonomous_bounty_events" => {
            let args: ListAutonomousBountyEventsArgs =
                serde_json::from_value(arguments).map_err(|error| {
                    format!("invalid list_autonomous_bounty_events arguments: {error}")
                })?;
            (
                list_autonomous_bounty_events(State(state), Json(args)).await.0,
                "Returned confirmed canonical lifecycle events. Report a solver as paid only when the matching BountySettled event is present.",
            )
        }
        _ => return Err(format!("unknown or unavailable AI assistant tool: {name}")),
    };
    match legacy_result(legacy) {
        Ok(value) => match normalize_assistant_tool_output(name, value) {
            Ok(value) => Ok(tool_result(value, narration, false)),
            Err(error) => Ok(tool_error(error)),
        },
        Err(error) => Ok(tool_error(error)),
    }
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

fn normalize_assistant_tool_output(name: &str, value: Value) -> Result<Value, String> {
    let value = match name {
        "list_unfunded_bounties" | "list_autonomous_bounties" => {
            if !value.is_array() {
                return Err(format!("{name} returned a non-array bounty list"));
            }
            json!({"bounties": value})
        }
        "list_autonomous_bounty_events" => {
            if !value.is_array() {
                return Err("list_autonomous_bounty_events returned a non-array event list".into());
            }
            json!({"events": value})
        }
        _ => value,
    };
    validate_output_schema_value(
        &assistant_tool_output_schema(name),
        &value,
        "structuredContent",
    )
    .map_err(|error| format!("{name} returned invalid structured content: {error}"))?;
    Ok(value)
}

fn bounty_post_markdown(value: &Value) -> String {
    let title = markdown_text(
        value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Bounty draft"),
    );
    let goal = markdown_text(
        value
            .get("goal")
            .and_then(Value::as_str)
            .unwrap_or("No goal supplied."),
    );
    let solver = value
        .get("solver_reward_usdc")
        .and_then(Value::as_str)
        .unwrap_or("—");
    let verifier = value
        .get("verifier_reward_usdc")
        .and_then(Value::as_str)
        .unwrap_or("—");
    let days = value
        .get("task_window_days")
        .and_then(Value::as_u64)
        .unwrap_or(30);
    let post_url = value
        .get("post_url")
        .and_then(Value::as_str)
        .unwrap_or(POST_PAGE_URL);
    let criteria = value
        .get("acceptance_criteria")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|item| format!("- {}", markdown_text(item)))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| "- Review required".to_string());

    format!(
        "## {title}\n\n{goal}\n\n**Reward target:** {solver} USDC solver + {verifier} USDC verifier  \n**Work window:** {days} day{}\n\n**Done when**\n{criteria}\n\n[Review this draft on Agent Bounties]({post_url})\n\n_Draft only — nothing has been posted, funded, or signed._",
        if days == 1 { "" } else { "s" }
    )
}

fn markdown_text(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '`'
                | '*'
                | '_'
                | '{'
                | '}'
                | '['
                | ']'
                | '<'
                | '>'
                | '#'
                | '+'
                | '-'
                | '.'
                | '!'
                | '|'
                | '('
                | ')'
        ) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn tool_result(value: Value, narration: &str, widget: bool) -> Value {
    let mut result = json!({
        "content": [{"type": "text", "text": narration}],
        "structuredContent": value
    });
    if widget {
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
    if uri != POST_WIDGET_URI {
        return Err("unknown resource URI".to_string());
    }
    Ok(json!({"contents": [widget_resource_contents()]}))
}

fn widget_resource_descriptor() -> Value {
    json!({
        "uri": POST_WIDGET_URI,
        "name": "Agent Bounties post review",
        "title": "Review and post bounty",
        "description": "Review bounty terms prepared in the user's AI account and continue to Agent Bounties.",
        "mimeType": "text/html;profile=mcp-app"
    })
}

fn widget_resource_contents() -> Value {
    let widget_html = POST_WIDGET_HTML.replace(
        "<!-- MCP_POST_WIDGET_SCRIPT -->",
        &format!("<script>{POST_WIDGET_SCRIPT}</script>"),
    );
    json!({
        "uri": POST_WIDGET_URI,
        "mimeType": "text/html;profile=mcp-app",
        "text": widget_html,
        "_meta": {
            "ui": {
                "prefersBorder": true,
                "domain": "840fc8c66eefe46904e7dd2c78e7fd12.claudemcpcontent.com",
                "csp": {
                    "connectDomains": [],
                    "resourceDomains": []
                }
            },
            "openai/widgetDescription": "A read-only bounty card prepared in the user's AI conversation. Its button opens Agent Bounties for explicit review and wallet approval.",
            "openai/widgetPrefersBorder": true,
            "openai/widgetDomain": "https://mcp.agentbounties.app",
            "openai/widgetCSP": {
                "connect_domains": [],
                "resource_domains": [],
                "redirect_domains": ["https://agentbounties.app"]
            }
        }
    })
}

fn post_handoff_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema": {"type": "string"},
            "interface": {"type": "string"},
            "prepared_by": {"type": "string"},
            "supported_hosts": {"type": "array", "items": {"type": "string"}},
            "rendering": {
                "type": "object",
                "properties": {
                    "mcp_app_widget": {"type": "string"},
                    "portable_fallback": {"type": "string"}
                },
                "required": ["mcp_app_widget", "portable_fallback"],
                "additionalProperties": false
            },
            "state": {"type": "string"},
            "title": {"type": "string"},
            "goal": {"type": "string"},
            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
            "solver_reward_usdc": {"type": "string"},
            "verifier_reward_usdc": {"type": "string"},
            "task_window_days": {"type": "integer"},
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
        "required": ["schema", "interface", "prepared_by", "supported_hosts", "rendering", "state", "title", "goal", "acceptance_criteria", "solver_reward_usdc", "verifier_reward_usdc", "task_window_days", "target_usdc", "initial_funding_usdc", "crowdfund", "source_url", "post_url", "bounty_created", "wallet_signature_requested", "next_action", "evidence_boundary"],
        "additionalProperties": false
    })
}

fn assistant_tool_output_schema(name: &str) -> Value {
    match name {
        "publish_unfunded_bounty" => unfunded_bounty_output_schema(),
        "list_unfunded_bounties" => json!({
            "type": "object",
            "properties": {
                "bounties": {
                    "type": "array",
                    "items": unfunded_bounty_output_schema()
                }
            },
            "required": ["bounties"],
            "additionalProperties": false
        }),
        "submit_unfunded_bounty_solution" => unfunded_solution_output_schema(),
        "prepare_bounty_post" => post_handoff_output_schema(),
        "list_autonomous_bounties" => json!({
            "type": "object",
            "properties": {
                "bounties": {
                    "type": "array",
                    "items": autonomous_bounty_output_schema()
                }
            },
            "required": ["bounties"],
            "additionalProperties": false
        }),
        "prepare_agent_to_earn" | "agent_native_claim" => hosted_api_output_schema(),
        "prepare_autonomous_bounty_submission" => autonomous_submission_output_schema(),
        "publish_autonomous_submission_evidence" => submission_evidence_output_schema(),
        "list_autonomous_bounty_events" => json!({
            "type": "object",
            "properties": {
                "events": {
                    "type": "array",
                    "items": autonomous_event_output_schema()
                }
            },
            "required": ["events"],
            "additionalProperties": false
        }),
        _ => json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
    }
}

fn hosted_api_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "http_status": {"type": "integer"},
            "body": {"type": "object"}
        },
        "required": ["http_status", "body"],
        "additionalProperties": false
    })
}

fn unfunded_solution_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "solution_id": {"type": "string"},
            "agent_id": {"type": "string"},
            "summary": {"type": "string"},
            "deliverable_markdown": {"type": "string"},
            "evidence": {},
            "attribution_status": {"type": "string"},
            "created_at": {"type": "string"},
            "updated_at": {"type": "string"}
        },
        "required": [
            "solution_id",
            "agent_id",
            "summary",
            "deliverable_markdown",
            "evidence",
            "attribution_status",
            "created_at",
            "updated_at"
        ],
        "additionalProperties": false
    })
}

fn cloud_demo_solution_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "provider": {"type": "string"},
            "model": {"type": "string"},
            "agent_name": {"type": "string"},
            "completion_status": {"type": "string"},
            "summary": {"type": "string"},
            "deliverable_markdown": {"type": "string"},
            "evidence": {},
            "limitations": {"type": "array", "items": {"type": "string"}},
            "payment_due_usdc": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "schema_version",
            "provider",
            "model",
            "agent_name",
            "completion_status",
            "summary",
            "deliverable_markdown",
            "evidence",
            "limitations",
            "payment_due_usdc",
            "evidence_boundary"
        ],
        "additionalProperties": false
    })
}

fn unfunded_bounty_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "bounty_id": {"type": "string"},
            "bounty_kind": {"type": "string"},
            "funding_status": {"type": "string"},
            "status": {"type": "string"},
            "title": {"type": "string"},
            "goal": {"type": "string"},
            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
            "source_url": {"type": ["string", "null"]},
            "demo_agent_solution": cloud_demo_solution_output_schema(),
            "agent_solutions": {
                "type": "array",
                "items": unfunded_solution_output_schema()
            },
            "wallet_required": {"type": "boolean"},
            "initial_funding_usdc": {"type": "string"},
            "payment_promised": {"type": "boolean"},
            "canonical_bounty_created": {"type": "boolean"},
            "public_url": {"type": "string"},
            "upgrade_url": {"type": "string"},
            "created_at": {"type": "string"},
            "expires_at": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "schema_version",
            "bounty_id",
            "bounty_kind",
            "funding_status",
            "status",
            "title",
            "goal",
            "acceptance_criteria",
            "source_url",
            "demo_agent_solution",
            "agent_solutions",
            "wallet_required",
            "initial_funding_usdc",
            "payment_promised",
            "canonical_bounty_created",
            "public_url",
            "upgrade_url",
            "created_at",
            "expires_at",
            "evidence_boundary"
        ],
        "additionalProperties": false
    })
}

fn network_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "chain_id": {"type": "integer"},
            "rpc_url_env": {"type": "string"},
            "native_usdc_token_address": {"type": "string"}
        },
        "required": [
            "name",
            "chain_id",
            "rpc_url_env",
            "native_usdc_token_address"
        ],
        "additionalProperties": false
    })
}

fn autonomous_event_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "log_key": {"type": "string"},
            "tx_hash": {"type": "string"},
            "block_number": {"type": "integer"},
            "log_index": {"type": "integer"},
            "contract_address": {"type": "string"},
            "bounty_id": {"type": "string"},
            "kind": {"type": "string"},
            "data": {},
            "occurred_at": {"type": "string"}
        },
        "required": [
            "id",
            "log_key",
            "tx_hash",
            "block_number",
            "log_index",
            "contract_address",
            "bounty_id",
            "kind",
            "data",
            "occurred_at"
        ],
        "additionalProperties": false
    })
}

fn autonomous_bounty_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "bounty_id": {"type": "string"},
            "bounty_contract": {"type": "string"},
            "creator": {"type": "string"},
            "status": {"type": "string"},
            "solver_reward": {"type": "string"},
            "verifier_reward": {"type": "string"},
            "claim_bond": {"type": "string"},
            "timeout_bond_pool": {"type": "string"},
            "target_amount": {"type": "string"},
            "funded_amount": {"type": "string"},
            "terms_hash": {"type": "string"},
            "terms": {"type": ["object", "null"]},
            "terms_valid": {"type": "boolean"},
            "verification_mode": {"type": "string"},
            "verifier_module": {"type": ["string", "null"]},
            "verification_ready": {"type": "boolean"},
            "verification_readiness_reason": {"type": "string"},
            "validation_errors": {"type": "array", "items": {"type": "string"}},
            "events": {"type": "array", "items": autonomous_event_output_schema()}
        },
        "required": [
            "bounty_id",
            "bounty_contract",
            "creator",
            "status",
            "solver_reward",
            "verifier_reward",
            "claim_bond",
            "timeout_bond_pool",
            "target_amount",
            "funded_amount",
            "terms_hash",
            "terms",
            "terms_valid",
            "verification_mode",
            "verifier_module",
            "verification_ready",
            "verification_readiness_reason",
            "validation_errors",
            "events"
        ],
        "additionalProperties": false
    })
}

fn autonomous_submission_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "protocol_version": {"type": "string"},
            "network": network_output_schema(),
            "bounty_contract": {"type": "string"},
            "bounty_id": {"type": "string"},
            "current_bounty_state": {"type": "string"},
            "expected_bounty_state": {"type": "string"},
            "expected_canonical_event": {"type": "string"},
            "solver": {"type": "string"},
            "round": {"type": "integer"},
            "claim_expires_at": {"type": "integer"},
            "authorization_deadline": {"type": "integer"},
            "artifact_reference": {"type": "string"},
            "submission_hash": {"type": "string"},
            "evidence_hash": {"type": "string"},
            "policy_hash": {"type": "string"},
            "signing_payload": {"type": "object"},
            "unsigned_relay_envelope": {"type": "object"},
            "evidence_publication": {"type": "object"},
            "relay_issue_url": {"type": ["string", "null"]},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "protocol_version",
            "network",
            "bounty_contract",
            "bounty_id",
            "current_bounty_state",
            "expected_bounty_state",
            "expected_canonical_event",
            "solver",
            "round",
            "claim_expires_at",
            "authorization_deadline",
            "artifact_reference",
            "submission_hash",
            "evidence_hash",
            "policy_hash",
            "signing_payload",
            "unsigned_relay_envelope",
            "evidence_publication",
            "relay_issue_url",
            "evidence_boundary"
        ],
        "additionalProperties": false
    })
}

fn submission_evidence_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "network": {"type": "string"},
            "bounty_contract": {"type": "string"},
            "bounty_id": {"type": "string"},
            "round": {"type": "integer"},
            "solver_wallet": {"type": "string"},
            "artifact_reference": {"type": "string"},
            "artifact_hash": {"type": "string"},
            "evidence": {},
            "evidence_hash": {"type": "string"},
            "created_at": {"type": "string"}
        },
        "required": [
            "network",
            "bounty_contract",
            "bounty_id",
            "round",
            "solver_wallet",
            "artifact_reference",
            "artifact_hash",
            "evidence",
            "evidence_hash",
            "created_at"
        ],
        "additionalProperties": false
    })
}

fn validate_output_schema_value(schema: &Value, value: &Value, path: &str) -> Result<(), String> {
    if let Some(expected) = schema.get("type") {
        let matches = match expected {
            Value::String(kind) => output_type_matches(kind, value),
            Value::Array(kinds) => kinds
                .iter()
                .filter_map(Value::as_str)
                .any(|kind| output_type_matches(kind, value)),
            _ => false,
        };
        if !matches {
            return Err(format!(
                "{path} has JSON type {}, expected {}",
                output_value_type(value),
                expected
            ));
        }
    }

    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for property in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(property) {
                    return Err(format!("{path}.{property} is required"));
                }
            }
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (property, property_value) in object {
                if let Some(property_schema) = properties.get(property) {
                    validate_output_schema_value(
                        property_schema,
                        property_value,
                        &format!("{path}.{property}"),
                    )?;
                } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                    return Err(format!("{path}.{property} is not declared"));
                }
            }
        }
    }

    if let Some(array) = value.as_array() {
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in array.iter().enumerate() {
                validate_output_schema_value(item_schema, item, &format!("{path}[{index}]"))?;
            }
        }
    }

    Ok(())
}

fn output_type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value
            .as_number()
            .is_some_and(|number| number.is_i64() || number.is_u64()),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn output_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn tool_title(name: &str) -> &'static str {
    match name {
        "publish_unfunded_bounty" => "Publish no-wallet bounty",
        "list_unfunded_bounties" => "List unfunded bounties",
        "submit_unfunded_bounty_solution" => "Submit unfunded bounty solution",
        "prepare_bounty_post" => "Prepare bounty for wallet review",
        "list_autonomous_bounties" => "Find paid AI work",
        "prepare_agent_to_earn" => "Check earning readiness",
        "agent_native_claim" => "Claim funded bounty",
        "prepare_autonomous_bounty_submission" => "Prepare completed work",
        "publish_autonomous_submission_evidence" => "Publish submission evidence",
        "list_autonomous_bounty_events" => "Confirm bounty lifecycle and payment",
        _ => "Agent Bounties tool",
    }
}

#[derive(Clone, Copy)]
struct AssistantToolImpact {
    read_only: bool,
    destructive: bool,
    open_world: bool,
}

fn assistant_tool_impact(name: &str) -> AssistantToolImpact {
    match name {
        "publish_unfunded_bounty"
        | "agent_native_claim"
        | "publish_autonomous_submission_evidence" => AssistantToolImpact {
            read_only: false,
            destructive: true,
            open_world: true,
        },
        "submit_unfunded_bounty_solution" => AssistantToolImpact {
            read_only: false,
            destructive: true,
            open_world: true,
        },
        _ => AssistantToolImpact {
            read_only: true,
            destructive: false,
            open_world: false,
        },
    }
}

fn assistant_tool_description(name: &str, fallback: &'static str) -> &'static str {
    match name {
        "prepare_bounty_post" => "Use this when a person wants something done, wants to achieve a goal with paid help, or asks to post an Agent Bounties task. Prepare a reviewable funded or crowdfunded draft from the current conversation; move no funds and request no wallet signature.",
        "publish_unfunded_bounty" => "Use this when a person explicitly wants to publish a public seven-day voluntary request with no wallet and zero USDC. It is not funded or claimable and promises no payment.",
        "list_unfunded_bounties" => "Use this when a person explicitly asks for voluntary or unpaid Agent Bounties work. Keep these requests separate from funded earning opportunities and never promise payment.",
        "submit_unfunded_bounty_solution" => "Use this when a registered agent explicitly wants to publish or replace its public solution to an unfunded voluntary request. This creates no payment claim.",
        "list_autonomous_bounties" => "Use this when a person asks to earn money with AI, find paid AI tasks, browse funded Agent Bounties work, or choose a bounty to complete. Set claimable_only=true for work that is currently funded and ready to claim.",
        "prepare_agent_to_earn" => "Use this when a person has chosen one funded canonical bounty and provides a public Base payout wallet. Check wallet, bond, policy, claimability, and verification readiness without requesting secrets or changing state.",
        "agent_native_claim" => "Use this when a person has chosen a funded verification-ready bounty and explicitly wants to claim it. Reuse one idempotency key, show any wallet_request for one bounded signature, and replay until confirmed BountyClaimed.",
        "prepare_autonomous_bounty_submission" => "Use this when the active solver has completed a claimed bounty and wants to submit the artifact and public evidence. Prepare deterministic commitments and a bounded signing/relay handoff; do not claim submission, verification, or payment yet.",
        "publish_autonomous_submission_evidence" => "Use this when confirmed SubmissionAdded exists and the solver wants to publish the exact public artifact and evidence preimages matching the canonical commitments. This public write is not verification or payout proof.",
        "list_autonomous_bounty_events" => "Use this when a person needs to check the confirmed lifecycle of a canonical bounty, including claim, submission, settlement, or reopening. Only a matching BountySettled event proves that the solver was paid.",
        _ => fallback,
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
            task_window_days: Some(14),
            source_url: Some("https://github.com/NSPG13/agent-bounties/issues/386".to_string()),
            crowdfund: false,
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
        assert_eq!(handoff["interface"], "mcp");
        assert_eq!(handoff["prepared_by"], "user_owned_ai");
        assert_eq!(handoff["task_window_days"], 14);
        assert_eq!(handoff["bounty_created"], false);
        assert_eq!(handoff["wallet_signature_requested"], false);
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "title" && value == "Fix the reconciliation regression"));
        assert_eq!(
            pairs.iter().filter(|(key, _)| key == "criterion").count(),
            2
        );
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "from" && value == "mcp-assistant"));
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "taskWindowDays" && value == "14"));
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

        args.solver_reward_usdc = "2".to_string();
        args.task_window_days = Some(31);
        assert!(build_bounty_post_handoff(&args)
            .unwrap_err()
            .contains("between 1 and 30"));
    }

    #[test]
    fn portable_markdown_card_contains_terms_and_review_boundary() {
        let handoff = build_bounty_post_handoff(&valid_args()).unwrap();
        let markdown = bounty_post_markdown(&handoff);
        assert!(markdown.contains("## Fix the reconciliation regression"));
        assert!(markdown.contains("**Done when**"));
        assert!(markdown.contains("[Review this draft on Agent Bounties]"));
        assert!(markdown.contains("nothing has been posted, funded, or signed"));
    }

    #[tokio::test]
    async fn app_tools_have_required_annotations_and_widget_metadata() {
        let tools = chatgpt_tools().await;
        assert_eq!(tools.len(), AI_ASSISTANT_TOOL_NAMES.len());
        for name in AI_ASSISTANT_TOOL_NAMES {
            let descriptor = tools
                .iter()
                .find(|tool| tool["name"] == *name)
                .unwrap_or_else(|| panic!("missing assistant tool {name}"));
            assert!(
                descriptor["description"]
                    .as_str()
                    .unwrap()
                    .starts_with("Use this when"),
                "assistant tool {name} has a non-discoverable description: {}",
                descriptor["description"]
            );
            assert_eq!(
                descriptor["outputSchema"]["type"], "object",
                "assistant tool {name} must declare an object outputSchema"
            );
        }
        let prepare = tools
            .iter()
            .find(|tool| tool["name"] == "prepare_bounty_post")
            .expect("prepare tool");
        let publish = tools
            .iter()
            .find(|tool| tool["name"] == "publish_unfunded_bounty")
            .expect("unfunded publication tool");

        assert_eq!(prepare["annotations"]["readOnlyHint"], true);
        assert_eq!(prepare["annotations"]["destructiveHint"], false);
        assert_eq!(prepare["annotations"]["openWorldHint"], false);
        assert_eq!(prepare["_meta"]["ui"]["resourceUri"], POST_WIDGET_URI);
        assert_eq!(prepare["_meta"]["openai/outputTemplate"], POST_WIDGET_URI);
        assert!(prepare["description"]
            .as_str()
            .unwrap()
            .starts_with("Use this when"));
        assert_eq!(publish["annotations"]["readOnlyHint"], false);
        assert_eq!(publish["annotations"]["destructiveHint"], true);
        assert_eq!(publish["annotations"]["openWorldHint"], true);

        let submit = tools
            .iter()
            .find(|tool| tool["name"] == "submit_unfunded_bounty_solution")
            .unwrap();
        assert_eq!(submit["annotations"]["readOnlyHint"], false);
        assert_eq!(submit["annotations"]["destructiveHint"], true);
        assert_eq!(submit["annotations"]["openWorldHint"], true);

        let claim = tools
            .iter()
            .find(|tool| tool["name"] == "agent_native_claim")
            .unwrap();
        assert_eq!(claim["title"], "Claim funded bounty");
        assert_eq!(claim["annotations"]["readOnlyHint"], false);
        assert_eq!(claim["annotations"]["destructiveHint"], true);
        assert_eq!(claim["annotations"]["openWorldHint"], true);

        let publish_evidence = tools
            .iter()
            .find(|tool| tool["name"] == "publish_autonomous_submission_evidence")
            .unwrap();
        assert_eq!(publish_evidence["annotations"]["readOnlyHint"], false);
        assert_eq!(publish_evidence["annotations"]["destructiveHint"], true);
        assert_eq!(publish_evidence["annotations"]["openWorldHint"], true);

        let prepare_submission = tools
            .iter()
            .find(|tool| tool["name"] == "prepare_autonomous_bounty_submission")
            .unwrap();
        assert_eq!(prepare_submission["annotations"]["readOnlyHint"], true);
        assert_eq!(prepare_submission["annotations"]["openWorldHint"], false);

        let settlement = tools
            .iter()
            .find(|tool| tool["name"] == "list_autonomous_bounty_events")
            .unwrap();
        assert_eq!(settlement["title"], "Confirm bounty lifecycle and payment");
        assert_eq!(settlement["annotations"]["readOnlyHint"], true);
        assert_eq!(
            settlement["outputSchema"]["properties"]["events"]["type"],
            "array"
        );
        let funded = tools
            .iter()
            .find(|tool| tool["name"] == "list_autonomous_bounties")
            .unwrap();
        assert_eq!(
            funded["outputSchema"]["properties"]["bounties"]["type"],
            "array"
        );
    }

    #[test]
    fn assistant_output_schemas_are_total_and_runtime_checked() {
        let handoff = build_bounty_post_handoff(&valid_args()).unwrap();
        assert!(normalize_assistant_tool_output("prepare_bounty_post", handoff).is_ok());

        for name in ["list_unfunded_bounties", "list_autonomous_bounties"] {
            let normalized = normalize_assistant_tool_output(name, json!([])).unwrap();
            assert_eq!(normalized, json!({"bounties": []}));
            assert!(validate_output_schema_value(
                &assistant_tool_output_schema(name),
                &json!([]),
                "structuredContent"
            )
            .is_err());
        }

        let normalized =
            normalize_assistant_tool_output("list_autonomous_bounty_events", json!([])).unwrap();
        assert_eq!(normalized, json!({"events": []}));

        let hosted = json!({"http_status": 200, "body": {"ready": true}});
        assert!(normalize_assistant_tool_output("prepare_agent_to_earn", hosted.clone()).is_ok());
        let invalid_hosted = json!({
            "http_status": 200,
            "body": {"ready": true},
            "unexpected": true
        });
        assert!(normalize_assistant_tool_output("prepare_agent_to_earn", invalid_hosted).is_err());

        let solution = json!({
            "solution_id": "5cfbdc18-6abc-4709-9fb8-1322e52aa84a",
            "agent_id": "913e03f2-4c07-4dd4-b28e-e83aff3d65a7",
            "summary": "Delivered",
            "deliverable_markdown": "Artifact",
            "evidence": {"commit": "abc"},
            "attribution_status": "registered_agent",
            "created_at": "2026-07-23T00:00:00Z",
            "updated_at": "2026-07-23T00:00:00Z"
        });
        assert!(
            normalize_assistant_tool_output("submit_unfunded_bounty_solution", solution).is_ok()
        );
    }

    #[test]
    fn widget_resource_has_mcp_apps_mime_and_exact_redirect_allowlist() {
        let contents = widget_resource_contents();
        assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
        assert_eq!(
            contents["_meta"]["ui"]["domain"],
            "840fc8c66eefe46904e7dd2c78e7fd12.claudemcpcontent.com"
        );
        assert_eq!(contents["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
        assert_eq!(contents["_meta"]["ui"]["csp"]["resourceDomains"], json!([]));
        assert_eq!(
            contents["_meta"]["openai/widgetCSP"]["redirect_domains"],
            json!(["https://agentbounties.app"])
        );
        assert_eq!(
            contents["_meta"]["openai/widgetDomain"],
            "https://mcp.agentbounties.app"
        );
        assert!(!contents["text"]
            .as_str()
            .unwrap()
            .contains("MCP_POST_WIDGET_SCRIPT"));
        assert!(contents["text"]
            .as_str()
            .unwrap()
            .contains("Agent Bounties bounty review"));
        assert!(contents["text"].as_str().unwrap().contains("openExternal"));
    }
}
