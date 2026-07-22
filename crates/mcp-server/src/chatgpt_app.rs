use super::{
    list_autonomous_bounties, list_unfunded_bounties, publish_unfunded_bounty,
    submit_unfunded_bounty_solution, tools, AutonomousBountyFeedArgs, ListUnfundedBountiesArgs,
    PrepareBountyPostArgs, PublishUnfundedBountyArgs, SharedState,
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
const POST_WIDGET_URI: &str = "ui://agent-bounties/post-bounty-v2.html";
const POST_PAGE_URL: &str = "https://agentbounties.app/post.html";
const POST_WIDGET_HTML: &str = include_str!("../../../site/chatgpt-post-widget.html");
const AI_ASSISTANT_TOOL_NAMES: &[&str] = &[
    "publish_unfunded_bounty",
    "list_unfunded_bounties",
    "submit_unfunded_bounty_solution",
    "prepare_bounty_post",
    "list_autonomous_bounties",
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
        "supported_hosts": ["chatgpt", "claude", "gemini", "other-mcp"],
        "rendering": {
            "mcp_app_widget": "chatgpt",
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
        "instructions": "Use publish_unfunded_bounty when a person wants to post with no wallet and no USDC. It creates a public seven-day bounty that agents can discover, while clearly reporting funding_status=unfunded and promising no payment. Use list_unfunded_bounties to discover those opportunities and submit_unfunded_bounty_solution for registered-agent work. Use prepare_bounty_post only for conversion to the on-chain flow. Never call an unfunded bounty canonical, funded, or claimable, and never request a private key or seed phrase."
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
    let mutates_public_state = matches!(
        descriptor.name,
        "publish_unfunded_bounty" | "submit_unfunded_bounty_solution"
    );
    let read_only = !mutates_public_state;
    let open_world = mutates_public_state;
    let mut value = Map::new();
    value.insert("name".to_string(), json!(descriptor.name));
    value.insert("title".to_string(), json!(tool_title(descriptor.name)));
    value.insert("description".to_string(), json!(descriptor.description));
    value.insert("inputSchema".to_string(), descriptor.input_schema);
    value.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": read_only,
            "destructiveHint": mutates_public_state,
            "openWorldHint": open_world,
            "idempotentHint": true
        }),
    );
    value.insert("securitySchemes".to_string(), json!([{"type": "noauth"}]));
    if descriptor.name == "prepare_bounty_post" {
        value.insert("outputSchema".to_string(), post_handoff_output_schema());
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
        _ => return Err(format!("unknown or unavailable AI assistant tool: {name}")),
    };
    match legacy_result(legacy) {
        Ok(value) => Ok(tool_result(value, narration, false)),
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
    json!({
        "uri": POST_WIDGET_URI,
        "mimeType": "text/html;profile=mcp-app",
        "text": POST_WIDGET_HTML,
        "_meta": {
            "ui": {
                "prefersBorder": true,
                "domain": "https://mcp.agentbounties.app",
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
            "rendering": {"type": "object"},
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
        "required": ["schema", "interface", "prepared_by", "supported_hosts", "rendering", "state", "title", "goal", "acceptance_criteria", "solver_reward_usdc", "verifier_reward_usdc", "task_window_days", "target_usdc", "initial_funding_usdc", "crowdfund", "post_url", "bounty_created", "wallet_signature_requested", "next_action", "evidence_boundary"],
        "additionalProperties": false
    })
}

fn tool_title(name: &str) -> &'static str {
    match name {
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
    }

    #[test]
    fn widget_resource_has_mcp_apps_mime_and_exact_redirect_allowlist() {
        let contents = widget_resource_contents();
        assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
        assert_eq!(
            contents["_meta"]["openai/widgetCSP"]["redirect_domains"],
            json!(["https://agentbounties.app"])
        );
        assert!(contents["text"].as_str().unwrap().contains("openExternal"));
    }
}
