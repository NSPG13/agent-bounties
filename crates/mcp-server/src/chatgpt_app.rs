use super::{
    agent_native_claim, compile_objective_with_cloud_agent, fund_bounty_with_x402, get_paid_status,
    get_x402_relay_status, list_autonomous_bounties, list_autonomous_verification_jobs,
    list_opportunities, list_unfunded_bounties, mcp_base_url_from_env,
    plan_autonomous_attestation_settlement, plan_autonomous_bounty_claim,
    plan_autonomous_module_settlement, plan_autonomous_verification_attestation,
    prepare_agent_to_earn, prepare_autonomous_bounty_submission, proxy_hosted_json,
    public_base_url_from_env, publish_autonomous_submission_evidence, publish_unfunded_bounty,
    submit_unfunded_bounty_solution, tools, AgentNativeClaimArgs, AutonomousBountyFeedArgs,
    AutonomousVerificationJobsArgs, CompileObjectiveWithCloudAgentArgs, GetX402RelayStatusArgs,
    ListUnfundedBountiesArgs, ObservedInterface, ObservedProtocolEra, OpportunityListArgs,
    PaidStatusArgs, PlanAutonomousAttestationSettlementArgs, PlanAutonomousBountyClaimArgs,
    PlanAutonomousModuleSettlementArgs, PlanAutonomousVerificationAttestationArgs,
    PrepareAgentToEarnInput, PrepareAutonomousBountySubmissionArgs, PrepareBountyPostArgs,
    PublishAutonomousSubmissionEvidenceArgs, PublishUnfundedBountyArgs, SharedState,
    SubmitUnfundedBountySolutionArgs, ToolDescriptor, X402BountyFundingArgs,
};
#[cfg(test)]
use super::{AppState, ChatgptFileInput};
use axum::{
    extract::State,
    http::{header::ORIGIN, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use db::NewBountyImageAsset;
use domain::BountyImageReference;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::{env, net::IpAddr};
use url::Url;
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2026-07-28";
const MCP_LEGACY_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_CATALOG_TTL_MS: u64 = 300_000;
const MCP_PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const MCP_CLIENT_INFO_META: &str = "io.modelcontextprotocol/clientInfo";
const MCP_CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const MCP_SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";
const MCP_PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const MCP_METHOD_HEADER: &str = "mcp-method";
const MCP_NAME_HEADER: &str = "mcp-name";
const MCP_ALLOWED_ORIGINS_ENV: &str = "MCP_ALLOWED_ORIGINS";
const CHATGPT_SANDBOX_ENV: &str = "CHATGPT_APP_SANDBOX_MODE";
const FEED_WIDGET_URI: &str = "ui://agent-bounties/live-feed-v4.html";
const POST_PAGE_URL: &str = "https://agentbounties.app/post.html";
const FEED_WIDGET_HTML: &str = include_str!("../../../site/chatgpt-bounty-feed-widget.html");
const BOUNTY_CARD_PREVIEW_HTML: &str =
    include_str!("../../../site/chatgpt-bounty-card-preview.html");
const FEED_CARD_ART: &[u8] = include_bytes!("../../../site/assets/bounty-quest-agent-v1.webp");
const MAX_BOUNTY_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const CHATGPT_FULL_TOOL_NAMES: &[&str] = &[
    "get_bounty_feed",
    "render_bounty_feed",
    "prepare_moonpay_onramp",
    "prepare_bounty_action",
    "get_bounty_action_status",
    "compile_objective_with_cloud_agent",
    "list_bounty_comments",
    "add_bounty_comment",
    "create_share_bundle",
    "prepare_bounty_post",
    "list_autonomous_bounties",
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
    bounty_url: Option<String>,
    status: String,
    reward: Option<String>,
    payment_state: Option<String>,
    bounty_image_url: Option<String>,
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
struct PrepareMoonpayOnrampArgs {
    bounty_contract: String,
    amount_base_units: u64,
    intent_id: Option<String>,
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
                    "stage": {"type": "string", "minLength": 1, "maxLength": 40, "description": "Short factual stage label such as terms prepared, funding requested, solving, completed, verified, or commented."},
                    "bounty_url": {"type": "string", "minLength": 1, "maxLength": 12000},
                    "status": {"type": "string", "minLength": 1, "maxLength": 80},
                    "reward": {"type": ["string", "null"], "maxLength": 80},
                    "payment_state": {"type": ["string", "null"], "maxLength": 80},
                    "bounty_image_url": {"type": ["string", "null"], "description": "Optional first-party image URL from the selected bounty projection."}
                },
                "required": ["bounty_id", "title", "stage", "bounty_url", "status"],
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "prepare_moonpay_onramp",
            description: "Use this when a person funding one canonical Base bounty needs Base USDC. It prepares a first-party HTTPS handoff to the existing MoonPay onramp page without opening checkout, moving money, requesting card data, or claiming that the bounty was funded.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "bounty_contract": {"type": "string", "pattern": "^0x[0-9a-fA-F]{40}$"},
                    "amount_base_units": {"type": "integer", "minimum": 1, "maximum": 1_000_000_000_000_u64, "description": "Planned bounty contribution in 6-decimal USDC base units."},
                    "intent_id": {"type": ["string", "null"], "format": "uuid", "description": "Optional hosted funding-intent identifier to preserve the return boundary."}
                },
                "required": ["bounty_contract", "amount_base_units"],
                "additionalProperties": false
            }),
            authorization: None,
        },
        ToolDescriptor {
            name: "prepare_bounty_action",
            description: "Use this when the person wants to post, fund, solve, complete, or verify a bounty after ChatGPT has collected the details conversationally and received explicit confirmation. It creates one idempotent first-party review session and returns an HTTPS authorization URL. It never asks ChatGPT for a wallet signature, private key, seed phrase, payment authorization, or verifier signature, and it never claims the action is complete.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "idempotency_key": {"type": "string", "minLength": 8, "maxLength": 200, "pattern": "^[A-Za-z0-9:._-]+$"},
                    "action": {"type": "string", "enum": ["post", "fund", "solve", "complete", "verify"]},
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

pub(super) async fn prepare_bounty_post_handoff(
    state: &SharedState,
    args: &PrepareBountyPostArgs,
) -> Result<Value, String> {
    // Fail closed on every non-file field before downloading or persisting the
    // approved image.
    let validation_image = sandbox_bounty_image_reference(args)?;
    build_bounty_post_handoff(args, &validation_image)?;
    let image = persist_chatgpt_bounty_image(state, args).await?;
    build_bounty_post_handoff(args, &image)
}

pub(super) fn build_bounty_post_handoff(
    args: &PrepareBountyPostArgs,
    image: &BountyImageReference,
) -> Result<Value, String> {
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
    let task_window_days = args.task_window_days.unwrap_or(30);
    if !(1..=30).contains(&task_window_days) {
        return Err("task_window_days must be from 1 to 30".to_string());
    }
    let discovery_source = args
        .discovery_source
        .as_deref()
        .map(|value| bounded_text(value, "discovery_source", 500))
        .transpose()?;
    let image_prompt = bounded_text(&args.image_prompt, "image_prompt", 4_000)?;
    let image_alt_text = bounded_text(&args.image_alt_text, "image_alt_text", 500)?;
    if image.source != "chatgpt_user_generated"
        || image.prompt != image_prompt
        || image.alt_text != image_alt_text
    {
        return Err(
            "the stored bounty image must match the prompt and alt text approved in ChatGPT"
                .to_string(),
        );
    }

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
        query.append_pair("taskWindowDays", &task_window_days.to_string());
        query.append_pair("crowdfund", if args.crowdfund { "true" } else { "false" });
        if let Some(source_url) = &source_url {
            query.append_pair("sourceUrl", source_url);
        }
        query.append_pair(
            "discoverySource",
            discovery_source.as_deref().unwrap_or("ChatGPT app"),
        );
        query.append_pair("imageUrl", &image.asset_url);
        query.append_pair("imageSha256", &image.sha256);
        query.append_pair("imageMimeType", &image.mime_type);
        query.append_pair("imagePrompt", &image.prompt);
        query.append_pair("imageAlt", &image.alt_text);
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
        "task_window_days": task_window_days,
        "initial_funding_usdc": if args.crowdfund { "0".to_string() } else { format_usdc(target) },
        "crowdfund": args.crowdfund,
        "source_url": source_url,
        "image": image,
        "post_url": post_url.as_str(),
        "bounty_created": false,
        "wallet_signature_requested": false,
        "next_action": "Open the secure handoff, review every field, and choose whether to deposit 0 USDC now or fully fund. Then connect the creator wallet and approve only the exact Base transaction shown by that wallet.",
        "evidence_boundary": "No bounty id or contract exists yet. Only confirmed CanonicalBountyCreated proves creation; FundingAdded and BountyBecameClaimable prove funding and claimability."
    }))
}

async fn persist_chatgpt_bounty_image(
    state: &SharedState,
    args: &PrepareBountyPostArgs,
) -> Result<BountyImageReference, String> {
    let prompt = bounded_text(&args.image_prompt, "image_prompt", 4_000)?;
    let alt_text = bounded_text(&args.image_alt_text, "image_alt_text", 500)?;
    let file_id = bounded_text(&args.bounty_image.file_id, "bounty_image.file_id", 512)?;
    let download_url = validate_chatgpt_download_url(&args.bounty_image.download_url)?;
    let bytes = download_chatgpt_image(&download_url).await?;
    let mime_type = detect_bounty_image_mime(&bytes)
        .ok_or_else(|| "bounty_image must be a valid PNG, JPEG, or WebP file".to_string())?;
    if let Some(declared) = args
        .bounty_image
        .mime_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if declared != mime_type {
            return Err(format!(
                "bounty_image MIME mismatch: ChatGPT supplied {declared}, but the file is {mime_type}"
            ));
        }
    }
    if let Some(file_name) = args.bounty_image.file_name.as_deref() {
        bounded_text(file_name, "bounty_image.file_name", 255)?;
    }
    let sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let store = state.store.as_ref().ok_or_else(|| {
        "durable bounty image storage is unavailable; configure DATABASE_URL before posting"
            .to_string()
    })?;
    store
        .put_bounty_image_asset(&NewBountyImageAsset {
            sha256: sha256.clone(),
            mime_type: mime_type.to_string(),
            content: bytes,
        })
        .await
        .map_err(|error| format!("could not store the approved bounty image: {error}"))?;
    let asset_url = format!(
        "{}/public/bounty-images/{sha256}",
        mcp_base_url_from_env().trim_end_matches('/')
    );
    // file_id proves that ChatGPT supplied a file to this invocation, but it is
    // deliberately not persisted or exposed in public bounty terms.
    drop(file_id);
    Ok(BountyImageReference {
        source: "chatgpt_user_generated".to_string(),
        prompt,
        alt_text,
        asset_url,
        sha256,
        mime_type: mime_type.to_string(),
    })
}

fn validate_chatgpt_download_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "bounty_image.download_url is invalid".to_string())?;
    if !chatgpt_file_url_is_allowed(&url) {
        return Err(
            "bounty_image.download_url must be an HTTPS ChatGPT/OpenAI file URL".to_string(),
        );
    }
    Ok(url)
}

fn chatgpt_file_url_is_allowed(url: &Url) -> bool {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port_or_known_default() != Some(443)
    {
        return false;
    }
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if host.parse::<IpAddr>().is_ok() {
        return false;
    }
    host == "chatgpt.com"
        || host == "openai.com"
        || host.ends_with(".openai.com")
        || host == "oaiusercontent.com"
        || host.ends_with(".oaiusercontent.com")
}

async fn download_chatgpt_image(url: &Url) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("too many ChatGPT file redirects")
            } else if chatgpt_file_url_is_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("ChatGPT file redirect left the allowed HTTPS hosts")
            }
        }))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| format!("could not create the ChatGPT file client: {error}"))?;
    let mut response = client
        .get(url.clone())
        .send()
        .await
        .map_err(|error| format!("could not download the approved ChatGPT image: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "ChatGPT image download returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length == 0 || length > MAX_BOUNTY_IMAGE_BYTES as u64)
    {
        return Err("bounty_image must contain between 1 byte and 5 MiB".to_string());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("could not read the approved ChatGPT image: {error}"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BOUNTY_IMAGE_BYTES {
            return Err("bounty_image exceeds the 5 MiB limit".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("bounty_image is empty".to_string());
    }
    Ok(bytes)
}

fn detect_bounty_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn sandbox_bounty_image_reference(
    args: &PrepareBountyPostArgs,
) -> Result<BountyImageReference, String> {
    Ok(BountyImageReference {
        source: "chatgpt_user_generated".to_string(),
        prompt: bounded_text(&args.image_prompt, "image_prompt", 4_000)?,
        alt_text: bounded_text(&args.image_alt_text, "image_alt_text", 500)?,
        asset_url: "https://agentbounties.app/assets/bounty-quest-agent-v1.webp".to_string(),
        sha256: "0".repeat(64),
        mime_type: "image/webp".to_string(),
    })
}

pub(super) async fn mcp_post(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let era = mcp_protocol_era(&headers, &payload);
    let excluded = super::analytics_exclusion_is_authorized(&state, &headers);
    let mut response = handle_mcp_post(state.clone(), headers, payload, era).await;
    super::attest_analytics_exclusion(&mut response, excluded);
    if !excluded {
        if let Some(store) = state.store.clone() {
            let succeeded = response.status().is_success();
            let protocol_era = match era {
                McpProtocolEra::Legacy => ObservedProtocolEra::McpLegacy,
                McpProtocolEra::Modern => ObservedProtocolEra::McpModern,
            };
            tokio::spawn(async move {
                let _ = store
                    .record_interface_usage(
                        ObservedInterface::Mcp,
                        protocol_era,
                        succeeded,
                        chrono::Utc::now(),
                    )
                    .await;
            });
        }
    }
    response
}

async fn handle_mcp_post(
    state: SharedState,
    headers: HeaderMap,
    payload: Value,
    era: McpProtocolEra,
) -> Response {
    if !mcp_origin_is_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }

    if era == McpProtocolEra::Modern {
        if payload.is_array() {
            return mcp_error_response(
                StatusCode::BAD_REQUEST,
                payload_id(&payload),
                -32600,
                "MCP 2026-07-28 accepts one JSON-RPC request per HTTP POST",
                None,
            );
        }
        if let Err(error) = validate_modern_request(&headers, &payload) {
            return error.into_response(payload_id(&payload));
        }
        let Some((status, response)) = handle_request(state, payload, McpProtocolEra::Modern).await
        else {
            return StatusCode::ACCEPTED.into_response();
        };
        return (status, Json(response)).into_response();
    }

    let responses = if let Some(batch) = payload.as_array() {
        let mut responses = Vec::new();
        for request in batch {
            if let Some((_, response)) =
                handle_request(state.clone(), request.clone(), McpProtocolEra::Legacy).await
            {
                responses.push(response);
            }
        }
        if responses.is_empty() {
            return StatusCode::ACCEPTED.into_response();
        }
        Value::Array(responses)
    } else if let Some((_, response)) = handle_request(state, payload, McpProtocolEra::Legacy).await
    {
        response
    } else {
        return StatusCode::ACCEPTED.into_response();
    };

    (StatusCode::OK, Json(responses)).into_response()
}

pub(super) async fn mcp_get(headers: HeaderMap) -> Response {
    if !mcp_origin_is_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("allow", "POST")],
        "This stateless MCP endpoint accepts JSON-RPC over POST.",
    )
        .into_response()
}

pub(super) async fn mcp_delete(headers: HeaderMap) -> Response {
    if !mcp_origin_is_allowed(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    (StatusCode::METHOD_NOT_ALLOWED, [("allow", "POST")]).into_response()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpProtocolEra {
    Legacy,
    Modern,
}

#[derive(Debug)]
struct McpProtocolError {
    status: StatusCode,
    code: i64,
    message: String,
    data: Option<Value>,
}

impl McpProtocolError {
    fn header_mismatch(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: -32020,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    fn unsupported_version(requested: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: -32022,
            message: format!("Unsupported MCP protocol version: {requested}"),
            data: Some(json!({
                "supported": [MCP_PROTOCOL_VERSION],
                "requested": requested
            })),
        }
    }

    fn into_response(self, id: Value) -> Response {
        mcp_error_response(self.status, id, self.code, &self.message, self.data)
    }
}

fn mcp_protocol_era(headers: &HeaderMap, payload: &Value) -> McpProtocolEra {
    let header_is_modern = headers
        .get(MCP_PROTOCOL_VERSION_HEADER)
        .is_some_and(|value| {
            value.to_str().map_or(true, |version| {
                !matches!(version, "2024-11-05" | "2025-03-26" | "2025-06-18")
            })
        });
    let body_has_request_metadata = payload
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|metadata| metadata.get(MCP_PROTOCOL_VERSION_META))
        .is_some();
    let is_discovery = payload
        .get("method")
        .and_then(Value::as_str)
        .is_some_and(|method| method == "server/discover");

    if header_is_modern || body_has_request_metadata || is_discovery {
        McpProtocolEra::Modern
    } else {
        McpProtocolEra::Legacy
    }
}

fn validate_modern_request(headers: &HeaderMap, request: &Value) -> Result<(), McpProtocolError> {
    let object = request
        .as_object()
        .ok_or_else(|| McpProtocolError::invalid_request("Invalid Request"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpProtocolError::invalid_request(
            "jsonrpc must be exactly '2.0'",
        ));
    }
    if !object
        .get("id")
        .is_some_and(|id| id.is_string() || id.is_number())
    {
        return Err(McpProtocolError::invalid_request(
            "MCP 2026-07-28 HTTP messages must be JSON-RPC requests with a string or number id",
        ));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| McpProtocolError::invalid_request("method must be a string"))?;
    let params = object
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| McpProtocolError::invalid_params("params must be an object"))?;
    let metadata = params
        .get("_meta")
        .and_then(Value::as_object)
        .ok_or_else(|| McpProtocolError::invalid_params("params._meta must be an object"))?;
    let body_version = metadata
        .get(MCP_PROTOCOL_VERSION_META)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpProtocolError::header_mismatch(format!(
                "params._meta.{MCP_PROTOCOL_VERSION_META} is required"
            ))
        })?;
    let header_version = required_header(headers, MCP_PROTOCOL_VERSION_HEADER)?;
    if header_version != body_version {
        return Err(McpProtocolError::header_mismatch(format!(
            "{MCP_PROTOCOL_VERSION_HEADER} header does not match request metadata"
        )));
    }
    if body_version != MCP_PROTOCOL_VERSION {
        return Err(McpProtocolError::unsupported_version(body_version));
    }

    let header_method = required_header(headers, MCP_METHOD_HEADER)?;
    if header_method != method {
        return Err(McpProtocolError::header_mismatch(format!(
            "{MCP_METHOD_HEADER} header does not match method"
        )));
    }

    if matches!(method, "tools/call" | "resources/read" | "prompts/get") {
        let source_field = if method == "resources/read" {
            "uri"
        } else {
            "name"
        };
        let body_name = params
            .get(source_field)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                McpProtocolError::invalid_params(format!(
                    "{method} requires a string params.{source_field}"
                ))
            })?;
        let header_name = decode_mcp_header_value(required_header(headers, MCP_NAME_HEADER)?)?;
        if header_name != body_name {
            return Err(McpProtocolError::header_mismatch(format!(
                "{MCP_NAME_HEADER} header does not match params.{source_field}"
            )));
        }
    }

    if !metadata
        .get(MCP_CLIENT_CAPABILITIES_META)
        .is_some_and(Value::is_object)
    {
        return Err(McpProtocolError::invalid_params(format!(
            "params._meta.{MCP_CLIENT_CAPABILITIES_META} must be an object"
        )));
    }
    if let Some(client_info) = metadata.get(MCP_CLIENT_INFO_META) {
        let valid = client_info.as_object().is_some_and(|client_info| {
            client_info.get("name").is_some_and(Value::is_string)
                && client_info.get("version").is_some_and(Value::is_string)
        });
        if !valid {
            return Err(McpProtocolError::invalid_params(format!(
                "params._meta.{MCP_CLIENT_INFO_META} must include string name and version fields"
            )));
        }
    }
    Ok(())
}

fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, McpProtocolError> {
    headers
        .get(name)
        .ok_or_else(|| McpProtocolError::header_mismatch(format!("missing {name} header")))?
        .to_str()
        .map_err(|_| McpProtocolError::header_mismatch(format!("malformed {name} header")))
}

fn decode_mcp_header_value(value: &str) -> Result<String, McpProtocolError> {
    const PREFIX: &str = "=?base64?";
    const SUFFIX: &str = "?=";
    if value.starts_with(PREFIX) {
        let encoded = value
            .strip_prefix(PREFIX)
            .and_then(|value| value.strip_suffix(SUFFIX))
            .ok_or_else(|| McpProtocolError::header_mismatch("malformed Base64 header sentinel"))?;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| McpProtocolError::header_mismatch("invalid Base64 header value"))?;
        return String::from_utf8(decoded)
            .map_err(|_| McpProtocolError::header_mismatch("Base64 header value is not UTF-8"));
    }
    Ok(value.to_string())
}

fn payload_id(payload: &Value) -> Value {
    payload.get("id").cloned().unwrap_or(Value::Null)
}

fn mcp_error_response(
    status: StatusCode,
    id: Value,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> Response {
    (
        status,
        Json(json_rpc_error_with_data(id, code, message, data)),
    )
        .into_response()
}

async fn handle_request(
    state: SharedState,
    request: Value,
    era: McpProtocolEra,
) -> Option<(StatusCode, Value)> {
    let Some(object) = request.as_object() else {
        return Some((
            StatusCode::OK,
            json_rpc_error(Value::Null, -32600, "Invalid Request"),
        ));
    };
    let id = object.get("id").cloned();
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some((
            StatusCode::OK,
            json_rpc_error(id.unwrap_or(Value::Null), -32600, "Invalid Request"),
        ));
    };
    let id = id?;
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));

    let result = match method {
        "server/discover" if era == McpProtocolEra::Modern => Ok(discover_result()),
        "initialize" if era == McpProtocolEra::Legacy => Ok(initialize_result(&params)),
        "ping" if era == McpProtocolEra::Legacy => Ok(json!({})),
        "tools/list" => {
            let mut tools = chatgpt_tools().await;
            if era == McpProtocolEra::Modern {
                tools.sort_by(|left, right| {
                    left.get("name")
                        .and_then(Value::as_str)
                        .cmp(&right.get("name").and_then(Value::as_str))
                });
            }
            Ok(json!({"tools": tools}))
        }
        "tools/call" => call_tool(state, &params).await,
        "resources/list" => Ok(json!({"resources": [feed_widget_resource_descriptor()]})),
        "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
        "resources/read" => read_resource(&params),
        _ => {
            return Some((
                if era == McpProtocolEra::Modern {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::OK
                },
                json_rpc_error(id, -32601, "Method not found"),
            ))
        }
    };

    Some(match result {
        Ok(result) => (
            StatusCode::OK,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": if era == McpProtocolEra::Modern {
                    modern_result(method, result)
                } else {
                    result
                }
            }),
        ),
        Err(error) => (
            if era == McpProtocolEra::Modern {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::OK
            },
            json_rpc_error(id, -32602, &error),
        ),
    })
}

fn initialize_result(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(MCP_LEGACY_PROTOCOL_VERSION);
    let protocol_version = match requested {
        "2024-11-05" | "2025-03-26" | "2025-06-18" => requested,
        _ => MCP_LEGACY_PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": protocol_version,
        "capabilities": mcp_server_capabilities(),
        "serverInfo": mcp_server_info(),
        "instructions": mcp_server_instructions()
    })
}

fn discover_result() -> Value {
    json!({
        "supportedVersions": [MCP_PROTOCOL_VERSION],
        "capabilities": mcp_server_capabilities(),
        "instructions": mcp_server_instructions()
    })
}

fn mcp_server_instructions() -> &'static str {
    let sandbox = chatgpt_sandbox_mode();
    let public_review = !sandbox && chatgpt_public_review_mode();
    if sandbox {
        "Sandbox mode is active. Use get_bounty_feed and render_bounty_feed to exercise the complete in-chat bounty UI. Use prepare_moonpay_onramp to exercise the external top-up handoff without opening MoonPay, and use prepare_bounty_action plus get_bounty_action_status to exercise the hosted bounty lifecycle without opening a wallet. Every tool returns deterministic fixture data and performs no network write, wallet action, public comment, publication, funding, claim, submission, verification, settlement, purchase, or payment. Never describe sandbox output as canonical evidence."
    } else if public_review {
        "Public review mode is active. Show only voluntary, unfunded community requests with no payment promise. Use render_bounty_feed for the compact in-chat work queue, publish_unfunded_bounty only when the user explicitly asks to publish a voluntary request, compile_objective_with_cloud_agent only for non-economic task decomposition, and the comment and share tools for public collaboration. Funding, claiming, completion, verification, wallet, settlement, token, and payment actions are unavailable in this app configuration. Never imply otherwise or direct a user around this boundary."
    } else {
        "Use get_bounty_feed to inspect fresh structured bounty data, then render_bounty_feed to show the mounted read-only feed in ChatGPT. The widget has only Post bounty, Comment, Share, and Solve actions; each action starts a conversation. For a new bounty, interview the person until the terms and image direction are complete, generate a unique bounty image in their ChatGPT account, show it for approval, summarize the complete bounty, and obtain explicit confirmation. Then call prepare_bounty_post with that approved ChatGPT image file; Agent Bounties stores the exact file and never generates a replacement. For fund, solve or claim, complete, or verify, call prepare_bounty_action and open only its first-party HTTPS authorization URL. If a funder needs Base USDC, call prepare_moonpay_onramp and open only its first-party HTTPS handoff; MoonPay purchase, wallet connection, identity checks, and card entry stay outside ChatGPT, and buying USDC is not bounty funding. Never request or accept a wallet signature, private key, seed phrase, payment authorization, verifier signature, or card data in ChatGPT. Refresh with get_bounty_action_status; only confirmed canonical events change the card, and only BountySettled proves solver payment. Use compile_objective_with_cloud_agent to break a broad objective into smaller reviewable child bounties. Use create_share_bundle after every meaningful step."
    }
}

fn mcp_server_capabilities() -> Value {
    json!({
        "tools": {"listChanged": false},
        "resources": {"subscribe": false, "listChanged": false}
    })
}

fn mcp_server_info() -> Value {
    let sandbox = chatgpt_sandbox_mode();
    let public_review = !sandbox && chatgpt_public_review_mode();
    json!({
        "name": if sandbox {
            "agent-bounties-sandbox"
        } else if public_review {
            "agent-bounties-community"
        } else {
            "agent-bounties"
        },
        "title": if sandbox {
            "Agent Bounties Sandbox"
        } else if public_review {
            "Agent Bounties Community"
        } else {
            "Agent Bounties"
        },
        "version": env!("CARGO_PKG_VERSION")
    })
}

fn modern_result(method: &str, mut result: Value) -> Value {
    let object = result
        .as_object_mut()
        .expect("every implemented MCP method returns an object result");
    object.insert("resultType".to_string(), json!("complete"));
    let metadata = object
        .entry("_meta".to_string())
        .or_insert_with(|| json!({}));
    if !metadata.is_object() {
        *metadata = json!({});
    }
    metadata[MCP_SERVER_INFO_META] = mcp_server_info();
    if matches!(
        method,
        "server/discover"
            | "tools/list"
            | "resources/list"
            | "resources/templates/list"
            | "resources/read"
    ) {
        object.insert("ttlMs".to_string(), json!(MCP_CATALOG_TTL_MS));
        object.insert("cacheScope".to_string(), json!("public"));
    }
    result
}

fn mcp_origin_is_allowed(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN) else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    mcp_origin_is_allowed_with_config(
        origin,
        env::var("MCP_BASE_URL").ok().as_deref(),
        env::var(MCP_ALLOWED_ORIGINS_ENV).ok().as_deref(),
    )
}

fn mcp_origin_is_allowed_with_config(
    origin: &str,
    mcp_base_url: Option<&str>,
    configured_origins: Option<&str>,
) -> bool {
    let Some(origin) = normalized_mcp_origin(origin) else {
        return false;
    };
    let defaults = [
        "https://chatgpt.com",
        "https://chat.openai.com",
        "https://agentbounties.app",
        "https://www.agentbounties.app",
        "https://mcp.agentbounties.app",
    ];
    if defaults.contains(&origin.as_str()) || is_loopback_http_origin(&origin) {
        return true;
    }
    if mcp_base_url
        .and_then(normalized_mcp_origin)
        .is_some_and(|allowed| allowed == origin)
    {
        return true;
    }
    configured_origins.is_some_and(|configured| {
        configured.split(',').any(|allowed| {
            normalized_mcp_origin(allowed.trim()).is_some_and(|allowed| allowed == origin)
        })
    })
}

fn normalized_mcp_origin(value: &str) -> Option<String> {
    let url = Url::parse(value).ok()?;
    if !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !matches!(url.scheme(), "http" | "https")
    {
        return None;
    }
    Some(url.origin().ascii_serialization())
}

fn is_loopback_http_origin(origin: &str) -> bool {
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    if url.scheme() != "http" {
        return false;
    }
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

async fn chatgpt_tools() -> Vec<Value> {
    let sandbox = chatgpt_sandbox_mode();
    let public_review = !sandbox && chatgpt_public_review_mode();
    let tool_names = chatgpt_tool_names(sandbox, public_review);
    let mut descriptors = tools().await.0;
    descriptors.extend(custom_tool_descriptors());
    descriptors
        .into_iter()
        .filter(|descriptor| tool_names.contains(&descriptor.name))
        .map(|descriptor| mcp_tool_descriptor_for_mode(descriptor, sandbox, public_review))
        .collect()
}

fn mcp_tool_descriptor_for_mode(
    descriptor: ToolDescriptor,
    sandbox: bool,
    _public_review: bool,
) -> Value {
    let public_review = false;
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
    } else if public_review {
        public_review_tool_description(descriptor.name, base_description).to_string()
    } else {
        base_description.to_string()
    };
    value.insert("description".to_string(), json!(description));
    value.insert(
        "inputSchema".to_string(),
        public_review_input_schema(descriptor.name, descriptor.input_schema, public_review),
    );
    value.insert(
        "annotations".to_string(),
        json!({
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "openWorldHint": open_world,
            "idempotentHint": idempotent
        }),
    );
    let security_schemes = analytics_security_schemes(
        std::env::var("ANALYTICS_EXCLUSION_TOKEN").is_ok_and(|value| !value.trim().is_empty()),
    );
    value.insert("securitySchemes".to_string(), security_schemes.clone());
    let mut metadata = json!({
        "securitySchemes": security_schemes,
        "ui": {"visibility": ["model", "app"]},
        "agentBountiesSandbox": sandbox,
        "agentBountiesPublicReview": public_review
    });
    if matches!(descriptor.name, "get_bounty_feed" | "render_bounty_feed") {
        value.insert(
            "outputSchema".to_string(),
            feed_output_schema(public_review),
        );
    }
    if descriptor.name == "list_autonomous_bounties" {
        value.insert(
            "outputSchema".to_string(),
            autonomous_bounty_feed_output_schema(),
        );
    }
    if matches!(
        descriptor.name,
        "prepare_bounty_action" | "get_bounty_action_status"
    ) {
        value.insert("outputSchema".to_string(), bounty_action_output_schema());
    }
    if descriptor.name == "prepare_moonpay_onramp" {
        value.insert("outputSchema".to_string(), moonpay_onramp_output_schema());
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
    if descriptor.name == "publish_unfunded_bounty" {
        value.insert(
            "outputSchema".to_string(),
            unfunded_bounty_output_schema(public_review),
        );
    }
    if descriptor.name == "compile_objective_with_cloud_agent" {
        value.insert(
            "outputSchema".to_string(),
            objective_plan_output_schema(public_review),
        );
    }
    if descriptor.name == "prepare_bounty_post" {
        metadata["openai/fileParams"] = json!(["bounty_image"]);
        value.insert("outputSchema".to_string(), post_handoff_output_schema());
    }
    let resource_uri = match descriptor.name {
        "render_bounty_feed" => Some(FEED_WIDGET_URI),
        _ => None,
    };
    if let Some(resource_uri) = resource_uri {
        metadata["ui"]["resourceUri"] = json!(resource_uri);
        metadata["openai/outputTemplate"] = json!(resource_uri);
        metadata["openai/toolInvocation/invoking"] = json!("Opening live bounty feed...");
        metadata["openai/toolInvocation/invoked"] = json!("Live feed ready");
    }
    value.insert("_meta".to_string(), metadata);
    Value::Object(value)
}

fn analytics_security_schemes(exclusion_link_enabled: bool) -> Value {
    if exclusion_link_enabled {
        json!([
            {"type": "noauth"},
            {"type": "oauth2", "scopes": [super::ANALYTICS_EXCLUSION_SCOPE]}
        ])
    } else {
        json!([{"type": "noauth"}])
    }
}

fn tool_impact(name: &str) -> (bool, bool, bool, bool) {
    match name {
        "prepare_moonpay_onramp" => (true, false, false, true),
        "prepare_bounty_post" => (false, true, true, true),
        "prepare_bounty_action" => (false, false, false, true),
        "get_bounty_action_status" => (false, false, false, true),
        "compile_objective_with_cloud_agent" => (false, false, true, false),
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

    let sandbox = chatgpt_sandbox_mode();
    let public_review = !sandbox && chatgpt_public_review_mode();
    if !chatgpt_tool_names(sandbox, public_review).contains(&name) {
        return Err(format!(
            "unknown or unavailable public ChatGPT app tool: {name}"
        ));
    }

    if sandbox {
        return sandbox_tool_result(name, &arguments).await;
    }
    if public_review {
        validate_public_review_tool_arguments(name, &arguments)?;
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
            if public_review {
                ensure_public_review_opportunity_id(&bounty_id)?;
            }
            let mut comments = fetch_comments(&bounty_id).await?;
            if public_review {
                constrain_public_review_comments(&mut comments);
            }
            return Ok(tool_result(
                comments,
                "Returned public in-chat comments for this bounty. Comments are conversation context, not payment or verification evidence.",
                false,
            ));
        }
        "add_bounty_comment" => {
            let args: AddCommentArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid add_bounty_comment arguments: {error}"))?;
            let bounty_id = bounded_opportunity_id(&args.bounty_id)?;
            if public_review {
                ensure_public_review_opportunity_id(&bounty_id)?;
            }
            let body = bounded_text(&args.body, "body", 500)?;
            if public_review {
                ensure_public_review_noncommercial_text(&body)?;
            }
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
            let mut value = legacy_result(result)?;
            if public_review {
                constrain_public_review_comments(&mut value);
            }
            return Ok(tool_result(
                value,
                "Comment published to the durable public bounty feed. Share the updated card when ready; comments remain conversation context, not payment or verification evidence.",
                false,
            ));
        }
        "create_share_bundle" => {
            let args: ShareBundleArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid create_share_bundle arguments: {error}"))?;
            return Ok(tool_result(build_share_bundle(&args, public_review)?, "Prepared a share-ready bounty card caption and social intents. Sharing is optional and does not change canonical payment state.", false));
        }
        "prepare_moonpay_onramp" => {
            let args: PrepareMoonpayOnrampArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_moonpay_onramp arguments: {error}"))?;
            return Ok(tool_result(
                build_moonpay_onramp_handoff(&args, false)?,
                "Prepared a first-party MoonPay top-up handoff. No checkout was opened, no purchase or wallet action occurred, and the bounty remains unfunded until a matching canonical FundingAdded event is indexed.",
                false,
            ));
        }
        "prepare_bounty_action" => {
            let args: PrepareBountyActionArgs = serde_json::from_value(arguments)
                .map_err(|error| format!("invalid prepare_bounty_action arguments: {error}"))?;
            let action = bounded_text(&args.action, "action", 16)?;
            if !matches!(
                action.as_str(),
                "post" | "fund" | "solve" | "complete" | "verify"
            ) {
                return Err("action must be post, fund, solve, complete, or verify".to_string());
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
                without_action_details(legacy_result(result)?),
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
                without_action_details(legacy_result(result)?),
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
            let mut args: CompileObjectiveWithCloudAgentArgs = serde_json::from_value(arguments)
                .map_err(|error| {
                    format!("invalid compile_objective_with_cloud_agent arguments: {error}")
                })?;
            if public_review {
                args.solver_budget_usdc = None;
            }
            (
                compile_objective_with_cloud_agent(State(state), Json(args))
                    .await
                    .0,
                if public_review {
                    "Compiled a bounded, non-economic task graph. The decomposition is advisory and creates no bounty, payment promise, claim, verification, settlement, or token transfer."
                } else {
                    "Compiled a bounded bounty graph. The decomposition is advisory until each child has independently reviewable terms, funding, verification, and evidence."
                },
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
            let value = prepare_bounty_post_handoff(&state, &args).await?;
            return Ok(tool_result(
                value,
                "Stored the image generated and approved in the poster's ChatGPT account, then prepared a reviewable wallet handoff. No bounty has been published or created yet.",
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
        Ok(mut value) => {
            if public_review && name == "compile_objective_with_cloud_agent" {
                constrain_public_review_objective_plan(&mut value);
            }
            if public_review && name == "publish_unfunded_bounty" {
                strip_public_unfunded_navigation(&mut value);
            }
            Ok(tool_result(value, narration, false))
        }
        Err(error) => Ok(tool_error(error)),
    }
}

fn chatgpt_sandbox_mode() -> bool {
    env_flag(CHATGPT_SANDBOX_ENV)
}

fn chatgpt_public_review_mode() -> bool {
    // The production and developer-installed apps intentionally expose one
    // full hosted-execution product. The only alternate runtime is the
    // deterministic no-write sandbox.
    false
}

fn env_flag(name: &str) -> bool {
    env::var(name).ok().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn chatgpt_tool_names(_sandbox: bool, _public_review: bool) -> &'static [&'static str] {
    CHATGPT_FULL_TOOL_NAMES
}

fn validate_public_review_tool_arguments(name: &str, arguments: &Value) -> Result<(), String> {
    let fields: &[&str] = match name {
        "publish_unfunded_bounty" => &["title", "goal", "acceptance_criteria"],
        "compile_objective_with_cloud_agent" => &["objective", "context", "constraints"],
        "add_bounty_comment" => &["body"],
        "create_share_bundle" => &["title", "stage", "status"],
        _ => &[],
    };
    for field in fields {
        match arguments.get(*field) {
            Some(Value::String(value)) => ensure_public_review_noncommercial_text(value)?,
            Some(Value::Array(values)) => {
                for value in values.iter().filter_map(Value::as_str) {
                    ensure_public_review_noncommercial_text(value)?;
                }
            }
            _ => {}
        }
    }
    for forbidden in ["source_url", "bounty_url", "reward", "payment_state"] {
        if arguments
            .get(forbidden)
            .is_some_and(|value| !value.is_null())
        {
            return Err(format!(
                "{forbidden} is unavailable in public review mode; use only voluntary non-economic collaboration fields"
            ));
        }
    }
    Ok(())
}

fn ensure_public_review_noncommercial_text(value: &str) -> Result<(), String> {
    if public_review_text_is_noncommercial(value) {
        Ok(())
    } else {
        Err("public review mode accepts voluntary non-economic collaboration only; remove payment, reward, funding, wallet, token-transfer, checkout, provider, address, and external-link language".to_string())
    }
}

fn ensure_public_review_opportunity_id(value: &str) -> Result<(), String> {
    if value.starts_with("unfunded:") {
        Ok(())
    } else {
        Err("public review comments are available only for voluntary community requests returned by the public feed".to_string())
    }
}

fn public_review_text_is_noncommercial(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    if normalized.contains("http://")
        || normalized.contains("https://")
        || normalized.contains('$')
        || normalized
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| {
                matches!(
                    token,
                    "pay"
                        | "paid"
                        | "paying"
                        | "payment"
                        | "payments"
                        | "payout"
                        | "payouts"
                        | "reward"
                        | "rewards"
                        | "fund"
                        | "funds"
                        | "funded"
                        | "funder"
                        | "funding"
                        | "crowdfund"
                        | "crowdfunding"
                        | "wallet"
                        | "wallets"
                        | "crypto"
                        | "cryptocurrency"
                        | "token"
                        | "tokens"
                        | "transfer"
                        | "transfers"
                        | "escrow"
                        | "usdc"
                        | "usd"
                        | "btc"
                        | "eth"
                        | "checkout"
                        | "purchase"
                        | "purchases"
                        | "buy"
                        | "sell"
                        | "tip"
                        | "tips"
                        | "donate"
                        | "donation"
                        | "compensation"
                        | "stripe"
                        | "paypal"
                        | "moonpay"
                        | "coinbase"
                        | "binance"
                ) || ((token.len() == 42 || token.len() == 66)
                    && token.starts_with("0x")
                    && token
                        .chars()
                        .skip(2)
                        .all(|character| character.is_ascii_hexdigit()))
            })
    {
        return false;
    }
    true
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
        "prepare_moonpay_onramp" => {
            let args: PrepareMoonpayOnrampArgs = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid prepare_moonpay_onramp arguments: {error}"))?;
            (
                build_moonpay_onramp_handoff(&args, true)?,
                "Prepared a sandbox-labeled MoonPay handoff without opening a provider page, connecting a wallet, creating checkout, or moving funds.",
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
            let mut value = build_share_bundle(&args, false)?;
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
            let image = sandbox_bounty_image_reference(&args)?;
            let mut value = build_bounty_post_handoff(&args, &image)?;
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

fn build_moonpay_onramp_handoff(
    args: &PrepareMoonpayOnrampArgs,
    sandbox: bool,
) -> Result<Value, String> {
    let bounty_contract = args.bounty_contract.trim();
    if bounty_contract.len() != 42
        || !bounty_contract.starts_with("0x")
        || !bounty_contract[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("bounty_contract must be a 20-byte 0x-prefixed address".to_string());
    }
    if args.amount_base_units == 0 || args.amount_base_units > 1_000_000_000_000 {
        return Err("amount_base_units must be between 1 and 1000000000000".to_string());
    }
    let intent_id = args
        .intent_id
        .as_deref()
        .map(|value| {
            Uuid::parse_str(value)
                .map(|id| id.to_string())
                .map_err(|_| "intent_id must be a UUID when provided".to_string())
        })
        .transpose()?;
    let mut onramp_url = Url::parse("https://agentbounties.app/onramp.html")
        .expect("static MoonPay handoff URL is valid");
    {
        let mut query = onramp_url.query_pairs_mut();
        query.append_pair("from", "chatgpt-app");
        query.append_pair("bountyContract", &bounty_contract.to_ascii_lowercase());
        query.append_pair("amount", &format_usdc(args.amount_base_units));
        query.append_pair(
            "return",
            "https://agentbounties.app/earn.html#fund-bounty-panel",
        );
        if let Some(intent_id) = &intent_id {
            query.append_pair("intent", intent_id);
        }
        if sandbox {
            query.append_pair("sandbox", "1");
        }
    }
    Ok(json!({
        "schema_version": "agent-bounties/moonpay-onramp-handoff-v1",
        "provider": "moonpay",
        "state": "review_required_not_opened",
        "network": "base-mainnet",
        "asset": "USDC",
        "bounty_contract": bounty_contract.to_ascii_lowercase(),
        "planned_amount_base_units": args.amount_base_units,
        "planned_amount_usdc": format_usdc(args.amount_base_units),
        "intent_id": intent_id,
        "onramp_url": onramp_url.as_str(),
        "checkout_created": false,
        "purchase_completed": false,
        "bounty_funded": false,
        "canonical_funding_event": null,
        "sandbox": sandbox,
        "next_action": if sandbox {
            "Sandbox proof only. Do not open MoonPay or represent this handoff as a purchase or bounty contribution."
        } else {
            "Open the first-party handoff, connect the destination Base wallet, review MoonPay's final quote and eligibility, complete the optional purchase outside ChatGPT, return, and separately authorize the exact bounty contribution."
        },
        "evidence_boundary": if sandbox {
            "Sandbox handoff only. No provider checkout, purchase, wallet top-up, bounty funding, canonical event, settlement, or payment exists."
        } else {
            "Preparing or opening this handoff is not a purchase and a MoonPay purchase is not bounty funding. Only a matching indexed canonical FundingAdded event changes the bounty's funded state."
        }
    }))
}

fn sandbox_action_intent_id(action: &str) -> Result<Uuid, String> {
    let suffix = match action {
        "post" => 1,
        "fund" => 2,
        "solve" => 3,
        "complete" => 4,
        "verify" => 5,
        _ => return Err("action must be post, fund, solve, complete, or verify".to_string()),
    };
    Uuid::parse_str(&format!("00000000-0000-4000-8000-{suffix:012}"))
        .map_err(|_| "failed to build sandbox action intent".to_string())
}

fn sandbox_action_from_intent_id(id: Uuid) -> Result<&'static str, String> {
    match id.as_bytes()[15] {
        1 => Ok("post"),
        2 => Ok("fund"),
        3 => Ok("solve"),
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
        "solve" => "bounty_claimed",
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
        let source_matches = args.source_type.as_deref().is_none_or(|expected| {
            item.get("source_type").and_then(Value::as_str) == Some(expected)
        });
        let work_matches = args.work_state.as_deref().is_none_or(|expected| {
            item.get("work_state").and_then(Value::as_str) == Some(expected)
        });
        let payment_matches = args.payment_state.as_deref().is_none_or(|expected| {
            item.get("payment_state").and_then(Value::as_str) == Some(expected)
        });
        let network_matches = args.network.as_deref().is_none_or(|expected| {
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
            "created_at": "2026-07-25T18:00:00Z",
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
    mut args: ChatgptFeedArgs,
    opportunity_ids: &[String],
) -> Result<Value, String> {
    let public_review = chatgpt_public_review_mode() && !chatgpt_sandbox_mode();
    if public_review {
        args.view = Some("recent".to_string());
        args.source_type = Some("unfunded_offchain".to_string());
        args.work_state = Some("open".to_string());
        args.payment_state = Some("none".to_string());
    }
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
    if public_review {
        constrain_public_review_feed(&mut value)?;
    }
    let mut value = attach_comments(value, public_review).await;
    let state_token = value
        .get("generated_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(object) = value.as_object_mut() {
        object.insert("state_token".to_string(), json!(state_token));
        object.insert(
            "app_mode".to_string(),
            json!(if public_review {
                "public_review"
            } else {
                "full"
            }),
        );
        object.insert("commerce_enabled".to_string(), json!(!public_review));
    }
    Ok(value)
}

fn constrain_public_review_feed(value: &mut Value) -> Result<(), String> {
    let source = value
        .as_object()
        .ok_or_else(|| "opportunity projection was not an object".to_string())?;
    let generated_at = source
        .get("generated_at")
        .cloned()
        .unwrap_or_else(|| json!(chrono::Utc::now().to_rfc3339()));
    let network = source
        .get("network")
        .cloned()
        .unwrap_or_else(|| json!("base-mainnet"));
    let degraded = source
        .get("degraded")
        .cloned()
        .unwrap_or(Value::Bool(false));
    let items = source
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| "opportunity projection did not contain items".to_string())?;
    let items = items
        .into_iter()
        .filter(|item| {
            item.get("source_type").and_then(Value::as_str) == Some("unfunded_offchain")
                && item.get("payment_state").and_then(Value::as_str) == Some("none")
                && item.get("work_state").and_then(Value::as_str) == Some("open")
        })
        .filter_map(|item| sanitize_public_review_item(&item))
        .collect::<Vec<_>>();
    let item_count = items.len();
    *value = json!({
        "schema_version": "agent-bounties/community-request-projection-v1",
        "generated_at": generated_at,
        "network": network,
        "applied_view": "recent",
        "degraded": degraded,
        "source_statuses": [{
            "source_type": "voluntary_request",
            "available": true,
            "authoritative_urls": [],
            "item_count": item_count,
            "error": null
        }],
        "items": items,
        "evidence_boundary": "Public review mode exposes voluntary community requests only. No reward, payment promise, wallet action, token transfer, paid-service checkout, claim, settlement, or payout is available."
    });
    Ok(())
}

fn sanitize_public_review_item(item: &Value) -> Option<Value> {
    let opportunity_id = item.get("opportunity_id")?.as_str()?;
    let title = item.get("title")?.as_str()?;
    let goal = item.get("goal").and_then(Value::as_str).unwrap_or_default();
    if !public_review_text_is_noncommercial(title) || !public_review_text_is_noncommercial(goal) {
        return None;
    }
    let acceptance_criteria = item
        .pointer("/evidence_requirements/acceptance_criteria")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if acceptance_criteria.iter().any(|criterion| {
        criterion
            .as_str()
            .is_some_and(|value| !public_review_text_is_noncommercial(value))
    }) {
        return None;
    }
    let public_strings = |field: &str| {
        item.get(field)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|value| public_review_text_is_noncommercial(value))
                    .map(|value| json!(value))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Some(json!({
        "opportunity_id": opportunity_id,
        "source_id": item.get("source_id").cloned().unwrap_or(Value::Null),
        "source_type": "voluntary_request",
        "request_status": "open",
        "title": title,
        "goal": goal,
        "categories": public_strings("categories"),
        "skills": public_strings("skills"),
        "public_url": public_review_card_url(opportunity_id, title, "open"),
        "work_state": "open",
        "access": "voluntary",
        "decision_authority": "The request poster may review public evidence.",
        "deadline": item.get("deadline").cloned().unwrap_or(Value::Null),
        "deadline_kind": item.get("deadline_kind").cloned().unwrap_or_else(|| json!("publication_expires_at")),
        "review_method": "public evidence review",
        "acceptance_criteria": acceptance_criteria,
        "created_at": item.get("created_at").cloned().unwrap_or(Value::Null),
        "updated_at": item.get("updated_at").cloned().unwrap_or(Value::Null),
        "evidence_boundary": "Voluntary community request only. No reward, payment promise, wallet action, token transfer, paid-service checkout, claim, settlement, or payout is available."
    }))
}

async fn attach_comments(mut value: Value, public_review: bool) -> Value {
    if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(opportunity_id) = item.get("opportunity_id").and_then(Value::as_str) {
                let mut comments = fetch_comments(opportunity_id)
                    .await
                    .unwrap_or_else(|_| json!({"comments": []}));
                if public_review {
                    constrain_public_review_comments(&mut comments);
                }
                let comments = comments
                    .get("comments")
                    .cloned()
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

fn constrain_public_review_comments(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let comments = object
        .get_mut("comments")
        .and_then(Value::as_array_mut)
        .map(|comments| {
            comments.retain(|comment| {
                comment
                    .get("body")
                    .and_then(Value::as_str)
                    .is_some_and(public_review_text_is_noncommercial)
            });
            comments.len()
        })
        .unwrap_or(0);
    object.insert("comment_count".to_string(), json!(comments));
    object.insert(
        "evidence_boundary".to_string(),
        json!("Public comments are collaboration context only. Payment, wallet, token-transfer, reward, and paid-service solicitation are unavailable in public review mode."),
    );
}

fn build_share_bundle(args: &ShareBundleArgs, public_review: bool) -> Result<Value, String> {
    let bounty_id = bounded_text(&args.bounty_id, "bounty_id", 200)?;
    let title = bounded_text(&args.title, "title", 200)?;
    let stage = bounded_text(&args.stage, "stage", 40)?;
    let status = bounded_text(&args.status, "status", 80)?;
    let bounty_url = if public_review {
        for value in [&title, &stage, &status] {
            ensure_public_review_noncommercial_text(value)?;
        }
        public_review_card_url(&bounty_id, &title, &status)
    } else {
        safe_share_url(
            args.bounty_url
                .as_deref()
                .ok_or_else(|| "bounty_url is required outside public review mode".to_string())?,
        )?
    };
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
    let bounty_image_url = args
        .bounty_image_url
        .as_deref()
        .map(safe_share_image_url)
        .transpose()?;
    let reward_copy = reward
        .as_deref()
        .map(|value| format!(" Reward target: {value}."))
        .unwrap_or_default();
    let caption = if public_review {
        format!(
            "{stage}: {title}. Status: {status}. Voluntary community request; no payment promise. View the public bounty: {bounty_url} #AgentBounties"
        )
    } else {
        format!(
            "{stage}: {title}. Status: {status}.{reward_copy} Payment state: {payment_state}. View the bounty: {bounty_url} #AgentBounties"
        )
    };
    let encoded_caption = encode_component(&caption);
    let encoded_url = encode_component(&bounty_url);
    Ok(json!({
        "schema": "agent-bounties/chatgpt-share-bundle-v1",
        "bounty_id": bounty_id,
        "stage": stage,
        "share_url": bounty_url,
        "bounty_image_url": bounty_image_url,
        "caption": caption,
        "hashtags": if public_review {
            json!(["#AgentBounties", "#OpenSource", "#CommunityCollaboration"])
        } else {
            json!(["#AgentBounties", "#BuildInPublic", "#AIWork"])
        },
        "intents": {
            "x": format!("https://x.com/intent/post?text={encoded_caption}&url={encoded_url}"),
            "linkedin": format!("https://www.linkedin.com/sharing/share-offsite/?url={encoded_url}"),
            "facebook": format!("https://www.facebook.com/sharer/sharer.php?u={encoded_url}"),
            "instagram": "https://www.instagram.com/".to_string()
        },
        "instagram_caption": caption,
        "evidence_boundary": if public_review {
            "This share bundle describes a voluntary community request only. It contains no reward, payment promise, wallet action, token transfer, or paid-service checkout."
        } else {
            "This share bundle describes the selected stage only. A transaction hash, planner response, comment, or individual AI output is not canonical funding, verification, settlement, or payment evidence."
        },
        "next_action": "Copy the caption or open a social share intent, then return to the feed."
    }))
}

fn public_review_card_url(bounty_id: &str, title: &str, status: &str) -> String {
    let origin = chatgpt_widget_domain_from_value(env::var("MCP_BASE_URL").ok().as_deref());
    let mut url = Url::parse(&format!("{origin}/chatgpt/bounty-card-preview"))
        .expect("validated widget origin must form a URL");
    url.query_pairs_mut()
        .append_pair("id", &truncate_chars(bounty_id, 96))
        .append_pair("title", &truncate_chars(title, 200))
        .append_pair("status", &truncate_chars(status, 80))
        .append_pair("public_review", "1");
    url.to_string()
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
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

fn safe_share_image_url(value: &str) -> Result<String, String> {
    let value = bounded_text(value, "bounty_image_url", 2_048)?;
    let parsed =
        Url::parse(&value).map_err(|_| "bounty_image_url must be a valid URL".to_string())?;
    let host = parsed.host_str().unwrap_or_default();
    let first_party_image = parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && ((host == "mcp.agentbounties.app"
            && parsed.path().starts_with("/public/bounty-images/"))
            || (host == "api.agentbounties.app"
                && parsed.path().starts_with("/public/opportunities/")
                && parsed.path().ends_with("/embed.svg"))
            || (host == "agentbounties.app" && parsed.path().starts_with("/assets/")));
    if !first_party_image {
        return Err("bounty_image_url must be a first-party Agent Bounties image URL".to_string());
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

fn without_action_details(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("details");
    }
    value
}

fn strip_public_review_economics(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for key in [
                "solver_budget_usdc",
                "suggested_solver_reward_usdc",
                "settlement_policy",
            ] {
                object.remove(key);
            }
            for child in object.values_mut() {
                strip_public_review_economics(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_public_review_economics(item);
            }
        }
        _ => {}
    }
}

fn constrain_public_review_objective_plan(value: &mut Value) {
    strip_public_review_economics(value);
    let Some(source) = value.as_object() else {
        return;
    };
    let tasks = source
        .get("tasks")
        .and_then(Value::as_array)
        .map(|tasks| {
            tasks
                .iter()
                .enumerate()
                .filter_map(|(index, task)| {
                    let task = task.as_object()?;
                    let title = safe_public_review_generated_text(
                        task.get("title").and_then(Value::as_str),
                        &format!("Review task {}", index + 1),
                    );
                    let goal = safe_public_review_generated_text(
                        task.get("goal").and_then(Value::as_str),
                        "Complete the public deliverable for this task.",
                    );
                    let mut acceptance_criteria = task
                        .get("acceptance_criteria")
                        .and_then(Value::as_array)
                        .map(|criteria| {
                            criteria
                                .iter()
                                .filter_map(Value::as_str)
                                .filter(|criterion| {
                                    public_review_text_is_noncommercial(criterion)
                                })
                                .map(|criterion| json!(criterion))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if acceptance_criteria.is_empty() {
                        acceptance_criteria.push(json!(
                            "Publish public evidence that the task goal is complete."
                        ));
                    }
                    Some(json!({
                        "task_id": task.get("task_id").cloned().unwrap_or_else(|| json!(format!("task-{}", index + 1))),
                        "title": title,
                        "goal": goal,
                        "depends_on": task.get("depends_on").cloned().unwrap_or_else(|| json!([])),
                        "acceptance_criteria": acceptance_criteria,
                        "effort_weight": task.get("effort_weight").cloned().unwrap_or_else(|| json!(1))
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let filtered_strings = |field: &str| {
        source
            .get(field)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|value| public_review_text_is_noncommercial(value))
                    .map(|value| json!(value))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    let mut sanitized = json!({
        "schema_version": source.get("schema_version").cloned().unwrap_or_else(|| json!("agent-bounties/public-objective-plan-v1")),
        "provider": source.get("provider").cloned().unwrap_or(Value::Null),
        "model": source.get("model").cloned().unwrap_or(Value::Null),
        "title": safe_public_review_generated_text(source.get("title").and_then(Value::as_str), "Public collaboration plan"),
        "objective": safe_public_review_generated_text(source.get("objective").and_then(Value::as_str), "Complete the stated public objective."),
        "success_definition": safe_public_review_generated_text(
            source.get("success_definition").and_then(Value::as_str),
            "All task drafts satisfy their public acceptance criteria and the combined objective is complete."
        ),
        "tasks": tasks,
        "parallel_layers": source.get("parallel_layers").cloned().unwrap_or_else(|| json!([])),
        "questions": filtered_strings("questions"),
        "risk_flags": filtered_strings("risk_flags"),
        "published": false,
        "evidence_boundary": "Advisory non-economic task drafts only. Nothing was published, funded, claimed, verified, settled, sold, or transferred."
    });
    if let Some(object) = sanitized.as_object_mut() {
        for optional_string in ["provider", "model"] {
            if object.get(optional_string).is_some_and(Value::is_null) {
                object.remove(optional_string);
            }
        }
    }
    *value = sanitized;
}

fn safe_public_review_generated_text(value: Option<&str>, fallback: &str) -> String {
    value
        .filter(|value| public_review_text_is_noncommercial(value))
        .map(ToString::to_string)
        .unwrap_or_else(|| fallback.to_string())
}

fn strip_public_unfunded_navigation(value: &mut Value) {
    let Some(source) = value.as_object() else {
        return;
    };
    let bounty_id = source
        .get("bounty_id")
        .and_then(Value::as_str)
        .unwrap_or("voluntary-request");
    let title = source
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Voluntary community request");
    *value = json!({
        "schema_version": "agent-bounties/voluntary-request-v1",
        "bounty_id": bounty_id,
        "status": source.get("status").cloned().unwrap_or_else(|| json!("open")),
        "title": title,
        "goal": source.get("goal").cloned().unwrap_or_else(|| json!("Complete the stated public request.")),
        "acceptance_criteria": source.get("acceptance_criteria").cloned().unwrap_or_else(|| json!([])),
        "public_url": public_review_card_url(bounty_id, title, "open"),
        "created_at": source.get("created_at").cloned().unwrap_or_else(|| json!("")),
        "expires_at": source.get("expires_at").cloned().unwrap_or_else(|| json!("")),
        "evidence_boundary": "Voluntary community request only. No reward, payment promise, wallet action, token transfer, paid-service checkout, claim, settlement, or payout is available."
    });
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
    feed_widget_resource_contents_for_mode(chatgpt_public_review_mode() && !chatgpt_sandbox_mode())
}

fn feed_widget_resource_contents_for_mode(_public_review: bool) -> Value {
    let public_review = false;
    let widget_domain = chatgpt_widget_domain_from_value(env::var("MCP_BASE_URL").ok().as_deref());
    let mut redirect_domains = vec![
        widget_domain.clone(),
        "https://x.com".to_string(),
        "https://www.linkedin.com".to_string(),
        "https://www.instagram.com".to_string(),
    ];
    if !public_review {
        redirect_domains.push("https://agentbounties.app".to_string());
    }
    let resource_domains = vec![
        widget_domain.clone(),
        "https://api.agentbounties.app".to_string(),
        "https://agentbounties.app".to_string(),
    ];
    let widget_description = if public_review {
        "A branded, read-only community-request feed. People use the visible conversation actions to discuss posting, commenting, sharing, or solving without filling out forms in the widget."
    } else {
        "A branded, read-only live bounty feed. People use Post bounty, Comment, Share, and Solve to continue in conversation; the widget contains no forms, wallet controls, or payment fields."
    };
    json!({
        "uri": FEED_WIDGET_URI,
        "mimeType": "text/html;profile=mcp-app",
        "text": feed_widget_html_for_mode(public_review),
        "_meta": {
            "ui": {
                "prefersBorder": false,
                "domain": widget_domain.clone(),
                "csp": {
                    "connectDomains": [],
                    "resourceDomains": resource_domains.clone(),
                    "redirectDomains": redirect_domains.clone()
                }
            },
            "openai/widgetDescription": widget_description,
            "openai/widgetPrefersBorder": false,
            "openai/widgetDomain": widget_domain,
            "openai/widgetCSP": {
                "connect_domains": [],
                "resource_domains": resource_domains,
                "redirect_domains": redirect_domains
            }
        }
    })
}

fn chatgpt_widget_domain_from_value(value: Option<&str>) -> String {
    value
        .and_then(|value| Url::parse(value.trim()).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .and_then(|url| {
            let host = url.host_str()?;
            Some(match url.port() {
                Some(port) => format!("https://{host}:{port}"),
                None => format!("https://{host}"),
            })
        })
        .unwrap_or_else(|| "https://mcp.agentbounties.app".to_string())
}

fn feed_widget_html_for_mode(public_review: bool) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(FEED_CARD_ART);
    let widget_domain = chatgpt_widget_domain_from_value(env::var("MCP_BASE_URL").ok().as_deref());
    let widget_domain_json = serde_json::to_string(&widget_domain)
        .unwrap_or_else(|_| "\"https://mcp.agentbounties.app\"".to_string());
    FEED_WIDGET_HTML
        .replace(
            "__BOUNTY_CARD_ART_DATA_URI__",
            &format!("data:image/webp;base64,{encoded}"),
        )
        .replace("__CHATGPT_APP_BASE_URL_JSON__", &widget_domain_json)
        .replace(
            "__CHATGPT_PUBLIC_REVIEW_MODE__",
            if public_review { "true" } else { "false" },
        )
}

pub(crate) fn bounty_card_preview_html() -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(FEED_CARD_ART);
    BOUNTY_CARD_PREVIEW_HTML.replace(
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
            "task_window_days": {"type": "integer"},
            "initial_funding_usdc": {"type": "string"},
            "crowdfund": {"type": "boolean"},
            "source_url": {"type": ["string", "null"]},
            "image": {
                "type": "object",
                "properties": {
                    "source": {"type": "string", "const": "chatgpt_user_generated"},
                    "prompt": {"type": "string"},
                    "alt_text": {"type": "string"},
                    "asset_url": {"type": "string"},
                    "sha256": {"type": "string"},
                    "mime_type": {"type": "string"}
                },
                "required": ["source", "prompt", "alt_text", "asset_url", "sha256", "mime_type"],
                "additionalProperties": false
            },
            "post_url": {"type": "string"},
            "bounty_created": {"type": "boolean"},
            "wallet_signature_requested": {"type": "boolean"},
            "next_action": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["schema", "state", "title", "goal", "acceptance_criteria", "solver_reward_usdc", "verifier_reward_usdc", "target_usdc", "task_window_days", "initial_funding_usdc", "crowdfund", "image", "post_url", "bounty_created", "wallet_signature_requested", "next_action", "evidence_boundary"],
        "additionalProperties": false
    })
}

fn feed_output_schema(public_review: bool) -> Value {
    if public_review {
        let source_status = json!({
            "type": "object",
            "properties": {
                "source_type": {"type": "string", "enum": ["voluntary_request"]},
                "available": {"type": "boolean"},
                "authoritative_urls": {"type": "array", "items": {"type": "string"}, "maxItems": 0},
                "item_count": {"type": "integer"},
                "error": {"type": ["string", "null"]}
            },
            "required": ["source_type", "available", "authoritative_urls", "item_count", "error"],
            "additionalProperties": false
        });
        let comment = json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "opportunity_id": {"type": "string"},
                "author": {"type": "string"},
                "body": {"type": "string"},
                "created_at": {"type": "string"}
            },
            "required": ["id", "author", "body", "created_at"],
            "additionalProperties": false
        });
        let item = json!({
            "type": "object",
            "properties": {
                "opportunity_id": {"type": "string"},
                "source_id": {"type": ["string", "null"]},
                "source_type": {"type": "string", "enum": ["voluntary_request"]},
                "request_status": {"type": "string", "enum": ["open"]},
                "title": {"type": "string"},
                "goal": {"type": "string"},
                "categories": {"type": "array", "items": {"type": "string"}},
                "skills": {"type": "array", "items": {"type": "string"}},
                "public_url": {"type": "string"},
                "work_state": {"type": "string", "enum": ["open"]},
                "access": {"type": "string", "enum": ["voluntary"]},
                "decision_authority": {"type": "string"},
                "deadline": {"type": ["string", "null"]},
                "deadline_kind": {"type": "string"},
                "review_method": {"type": "string"},
                "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
                "created_at": {"type": ["string", "null"]},
                "updated_at": {"type": ["string", "null"]},
                "comments": {"type": "array", "items": comment},
                "evidence_boundary": {"type": "string"}
            },
            "required": [
                "opportunity_id", "source_id", "source_type", "request_status",
                "title", "goal", "categories", "skills", "public_url", "work_state",
                "access", "decision_authority", "deadline", "deadline_kind",
                "review_method", "acceptance_criteria", "created_at", "updated_at",
                "comments", "evidence_boundary"
            ],
            "additionalProperties": false
        });
        return json!({
            "type": "object",
            "properties": {
                "schema_version": {"type": "string", "enum": ["agent-bounties/community-request-projection-v1"]},
                "generated_at": {"type": "string"},
                "state_token": {"type": "string"},
                "network": {"type": "string"},
                "applied_view": {"type": "string", "enum": ["recent"]},
                "degraded": {"type": "boolean"},
                "source_statuses": {
                    "type": "array",
                    "items": source_status
                },
                "items": {
                    "type": "array",
                    "items": item
                },
                "app_mode": {"type": "string", "enum": ["public_review"]},
                "commerce_enabled": {"type": "boolean", "enum": [false]},
                "evidence_boundary": {"type": "string"}
            },
            "required": [
                "schema_version", "generated_at", "state_token", "network", "applied_view",
                "degraded", "source_statuses", "items", "app_mode", "commerce_enabled",
                "evidence_boundary"
            ],
            "additionalProperties": false
        });
    }
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "generated_at": {"type": "string"},
            "network": {"type": "string"},
            "items": {"type": "array", "items": {"type": "object"}},
            "degraded": {"type": "boolean"},
            "app_mode": {"type": "string", "enum": ["full", "sandbox"]},
            "commerce_enabled": {"type": "boolean"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["schema_version", "generated_at", "network", "items", "degraded", "evidence_boundary"],
        "additionalProperties": true
    })
}

fn moonpay_onramp_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string", "enum": ["agent-bounties/moonpay-onramp-handoff-v1"]},
            "provider": {"type": "string", "enum": ["moonpay"]},
            "state": {"type": "string", "enum": ["review_required_not_opened"]},
            "network": {"type": "string", "enum": ["base-mainnet"]},
            "asset": {"type": "string", "enum": ["USDC"]},
            "bounty_contract": {"type": "string", "pattern": "^0x[0-9a-f]{40}$"},
            "planned_amount_base_units": {"type": "integer", "minimum": 1, "maximum": 1_000_000_000_000_u64},
            "planned_amount_usdc": {"type": "string"},
            "intent_id": {"type": ["string", "null"], "format": "uuid"},
            "onramp_url": {"type": "string"},
            "checkout_created": {"type": "boolean", "enum": [false]},
            "purchase_completed": {"type": "boolean", "enum": [false]},
            "bounty_funded": {"type": "boolean", "enum": [false]},
            "canonical_funding_event": {"type": "null"},
            "sandbox": {"type": "boolean"},
            "next_action": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "schema_version", "provider", "state", "network", "asset",
            "bounty_contract", "planned_amount_base_units", "planned_amount_usdc",
            "intent_id", "onramp_url", "checkout_created", "purchase_completed",
            "bounty_funded", "canonical_funding_event", "sandbox", "next_action",
            "evidence_boundary"
        ],
        "additionalProperties": false
    })
}

fn autonomous_bounty_feed_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "bounty_id": {"type": "string"},
                        "bounty_contract": {"type": "string"},
                        "status": {"type": "string"},
                        "solver_reward": {"type": "string"},
                        "claim_bond": {"type": "string"},
                        "required_external_spend": {"type": "string"},
                        "gross_cash_margin": {"type": "string"},
                        "terms_hash": {"type": "string"},
                        "verification_ready": {"type": "boolean"},
                        "verification_readiness_reason": {"type": "string"}
                    },
                    "required": [
                        "bounty_id", "bounty_contract", "status", "solver_reward",
                        "claim_bond", "required_external_spend", "gross_cash_margin",
                        "terms_hash", "verification_ready",
                        "verification_readiness_reason"
                    ],
                    "additionalProperties": true
                }
            },
            "sandbox": {"type": "boolean"},
            "evidence_boundary": {"type": "string"}
        },
        "required": ["items"],
        "additionalProperties": true
    })
}

fn bounty_action_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "intent_id": {"type": "string", "format": "uuid"},
            "action": {"type": "string", "enum": ["post", "fund", "solve", "complete", "verify"]},
            "status": {"type": "string", "enum": ["review_required", "pending_confirmation", "confirmed", "failed", "expired"]},
            "network": {"type": "string"},
            "opportunity_id": {"type": ["string", "null"]},
            "bounty_contract": {"type": ["string", "null"]},
            "bounty_id": {"type": ["string", "null"]},
            "actor_wallet": {"type": ["string", "null"]},
            "amount_base_units": {"type": ["integer", "null"]},
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
            "bounty_image_url": {"type": ["string", "null"]},
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
            "schema", "bounty_id", "stage", "share_url", "bounty_image_url", "caption", "hashtags",
            "intents", "instagram_caption", "evidence_boundary", "next_action"
        ],
        "additionalProperties": true
    })
}

fn unfunded_bounty_output_schema(public_review: bool) -> Value {
    if public_review {
        return json!({
            "type": "object",
            "properties": {
                "schema_version": {"type": "string"},
                "bounty_id": {"type": "string"},
                "status": {"type": "string", "enum": ["open"]},
                "title": {"type": "string"},
                "goal": {"type": "string"},
                "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
                "public_url": {"type": "string"},
                "created_at": {"type": "string"},
                "expires_at": {"type": "string"},
                "evidence_boundary": {"type": "string"}
            },
            "required": [
                "schema_version", "bounty_id", "status", "title", "goal",
                "acceptance_criteria", "public_url", "created_at", "expires_at",
                "evidence_boundary"
            ],
            "additionalProperties": false
        });
    }
    json!({
        "type": "object",
        "properties": {
            "schema_version": {"type": "string"},
            "bounty_id": {"type": "string"},
            "bounty_kind": {"type": "string", "enum": ["unfunded_offchain"]},
            "funding_status": {"type": "string", "enum": ["unfunded"]},
            "status": {"type": "string", "enum": ["open"]},
            "title": {"type": "string"},
            "goal": {"type": "string"},
            "acceptance_criteria": {"type": "array", "items": {"type": "string"}},
            "source_url": {"type": ["string", "null"]},
            "wallet_required": {"type": "boolean", "enum": [false]},
            "initial_funding_usdc": {"type": "string", "enum": ["0"]},
            "payment_promised": {"type": "boolean", "enum": [false]},
            "canonical_bounty_created": {"type": "boolean", "enum": [false]},
            "public_url": {"type": "string"},
            "created_at": {"type": "string"},
            "expires_at": {"type": "string"},
            "evidence_boundary": {"type": "string"}
        },
        "required": [
            "schema_version", "bounty_id", "bounty_kind", "funding_status", "status",
            "title", "goal", "acceptance_criteria", "wallet_required",
            "initial_funding_usdc", "payment_promised", "canonical_bounty_created",
            "public_url", "created_at", "expires_at", "evidence_boundary"
        ],
        "additionalProperties": true
    })
}

fn objective_plan_output_schema(public_review: bool) -> Value {
    let mut schema = json!({
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
    });
    if public_review {
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            for field in [
                "solver_budget_usdc",
                "execution_policy",
                "verification_policy",
                "settlement_policy",
                "source_url",
                "next_action",
            ] {
                properties.remove(field);
            }
        }
        schema["additionalProperties"] = json!(false);
    }
    schema
}

fn public_review_input_schema(name: &str, mut schema: Value, public_review: bool) -> Value {
    if !public_review {
        return schema;
    }
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    match name {
        "get_bounty_feed" | "render_bounty_feed" => {
            properties.remove("source_type");
            properties.remove("payment_state");
            if let Some(view) = properties.get_mut("view") {
                view["enum"] = json!(["recent", "engineering", "creative", "urgent", null]);
            }
            if let Some(work_state) = properties.get_mut("work_state") {
                work_state["enum"] = json!(["open", null]);
            }
        }
        "compile_objective_with_cloud_agent" => {
            properties.remove("solver_budget_usdc");
            properties.remove("source_url");
        }
        "create_share_bundle" => {
            properties.remove("bounty_url");
            properties.remove("bounty_image_url");
            properties.remove("reward");
            properties.remove("payment_state");
        }
        "publish_unfunded_bounty" => {
            properties.remove("source_url");
        }
        _ => {}
    }
    if let Some(required) = schema.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|field| {
            !matches!(
                field.as_str(),
                Some("source_url" | "bounty_url" | "reward" | "payment_state")
            )
        });
    }
    schema
}

fn public_review_tool_description<'a>(name: &str, fallback: &'a str) -> &'a str {
    match name {
        "get_bounty_feed" => "Use this when the user needs fresh structured data for voluntary, unfunded community requests. It returns no funded inventory, payment promise, claim, token action, or settlement action.",
        "render_bounty_feed" => "Use this when the user wants the interactive voluntary-request work queue rendered inside ChatGPT. This public configuration contains comments, sharing, and non-economic planning only.",
        "compile_objective_with_cloud_agent" => "Use this when the user wants a broad objective broken into smaller, independently reviewable non-economic task drafts. It creates and funds nothing and allocates no token budget.",
        "publish_unfunded_bounty" => "Use this when the user explicitly asks to publish a voluntary public digital-work request with no wallet and no payment promise. This public write expires from active discovery after seven days and creates no canonical or funded bounty.",
        "list_bounty_comments" => "Use this when the user wants to read public comments on one voluntary community request. Comments are conversation context only.",
        "add_bounty_comment" => "Use this when the user explicitly wants to publish one bounded public comment on a voluntary community request. Do not include secrets or restricted personal data.",
        "create_share_bundle" => "Use this when the user wants a factual social caption and share image for a voluntary community request. Sharing is optional and creates no payment promise.",
        _ => fallback,
    }
}

fn chatgpt_tool_description(name: &str, fallback: &'static str) -> &'static str {
    match name {
        "get_bounty_feed" => "Use this when the model or mounted feed needs fresh structured bounty data without rendering another widget. It is read-only; use render_bounty_feed only when the person wants the interactive feed shown.",
        "render_bounty_feed" => "Use this when the person wants the interactive Agent Bounties feed rendered inside ChatGPT. For model-selected results, inspect get_bounty_feed first and pass only the chosen opportunity_ids.",
        "prepare_moonpay_onramp" => "Use this when a person funding one canonical Base bounty needs Base USDC. Prepare a first-party MoonPay handoff only; never request card data or identity documents in ChatGPT, never treat a MoonPay purchase as bounty funding, and require a separate canonical funding authorization.",
        "prepare_bounty_action" => "Use this when ChatGPT has gathered the required details conversationally and the person has explicitly confirmed a post, fund, solve or claim, complete, or verify action. Create one idempotent first-party authorization session; never request a wallet or verifier signature in ChatGPT and never describe prepared status as completion.",
        "get_bounty_action_status" => "Use this when the person returns from first-party authorization and the card needs canonical status. Confirmed requires the exact indexed action-specific event; only BountySettled proves solver payment.",
        "fund_bounty_with_x402" => "Use this when the person explicitly wants to fund one canonical Base bounty. Request the x402 challenge first, replay only with the exact wallet-signed authorization, and do not call a challenge, signature, relay, or transaction hash funding evidence.",
        "get_x402_relay_status" => "Use this when an earlier x402 funding response returned a relay_id that still needs canonical confirmation. Only a matching confirmed FundingAdded event changes funding state.",
        "prepare_agent_to_earn" => "Use this when the person has selected one funded claimable bounty and supplied a public Base wallet. Check wallet policy, bond, claimability, and verification readiness without requesting a secret or changing state.",
        "agent_native_claim" => "Use this when the person explicitly wants to solve one funded verification-ready bounty. Reuse one idempotency key, request at most the exact bounded wallet signature returned by the tool, and replay until BountyClaimed is confirmed.",
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
        "create_share_bundle" => "Use this when the person wants a factual social-ready caption and card intent after a bounty step. Pass the selected projection's first-party bounty_image_url when available so the original approved image remains attached to the share package. Sharing is optional and changes no funding, claim, verification, settlement, or payment state.",
        "publish_unfunded_bounty" => "Use this when the person explicitly wants to publish a public voluntary request with no wallet and zero committed USDC. It is not canonical, funded, claimable, or guaranteed to pay.",
        "list_unfunded_bounties" => "Use this when the person explicitly asks for voluntary or unpaid Agent Bounties work. Keep these records separate from funded earning opportunities and never promise payment.",
        "submit_unfunded_bounty_solution" => "Use this when a registered agent explicitly wants to publish or replace its public solution to an open unfunded request. This public write creates no payment claim.",
        "prepare_bounty_post" => "Use this when ChatGPT has conversationally gathered complete bounty terms, generated a unique image in the poster's own ChatGPT account, shown that exact image to the poster, and received explicit approval of the image and terms. Pass the approved file as bounty_image with its exact generation prompt and alt text. Agent Bounties stores that file and prepares a reviewable wallet handoff; it does not generate an image, move funds, request a secret, or prove that a bounty exists.",
        "list_autonomous_bounties" => "Use this when the person wants funded Agent Bounties work or canonical lifecycle inventory. Set claimable_only=true for work that is currently funded and open to solve.",
        _ => fallback,
    }
}

fn tool_title(name: &str) -> &'static str {
    match name {
        "get_bounty_feed" => "Refresh bounty feed data",
        "render_bounty_feed" => "Open live bounty feed",
        "prepare_moonpay_onramp" => "Prepare MoonPay top-up",
        "prepare_bounty_action" => "Prepare secure bounty action",
        "get_bounty_action_status" => "Check canonical action status",
        "fund_bounty_with_x402" => "Request funding challenge",
        "get_x402_relay_status" => "Check funding relay",
        "prepare_agent_to_earn" => "Check claim readiness",
        "agent_native_claim" => "Solve bounty",
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
    json_rpc_error_with_data(id, code, message, None)
}

fn json_rpc_error_with_data(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": error
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::BountyNetwork;
    use chain_base::{AutonomousBountyRecoveryReservations, BaseRpcUrlConfig};
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

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
            image_prompt: "Minimal editorial illustration of a reconciliation test becoming green."
                .to_string(),
            image_alt_text: "A clean code diff with a passing reconciliation check.".to_string(),
            bounty_image: super::ChatgptFileInput {
                download_url: "https://files.oaiusercontent.com/example".to_string(),
                file_id: "file-example".to_string(),
                mime_type: Some("image/webp".to_string()),
                file_name: Some("reconciliation-bounty.webp".to_string()),
            },
        }
    }

    #[test]
    fn handoff_is_prefilled_but_never_claims_creation_or_signature() {
        let args = valid_args();
        let image = sandbox_bounty_image_reference(&args).unwrap();
        let handoff = build_bounty_post_handoff(&args, &image).unwrap();
        let post_url = Url::parse(handoff["post_url"].as_str().unwrap()).unwrap();
        let pairs = post_url.query_pairs().collect::<Vec<_>>();

        assert_eq!(handoff["state"], "review_required_not_published");
        assert_eq!(handoff["target_usdc"], "2.1");
        assert_eq!(handoff["initial_funding_usdc"], "2.1");
        assert_eq!(handoff["bounty_created"], false);
        assert_eq!(handoff["wallet_signature_requested"], false);
        assert_eq!(handoff["image"]["source"], "chatgpt_user_generated");
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "title" && value == "Fix the reconciliation regression"));
        assert_eq!(
            pairs.iter().filter(|(key, _)| key == "criterion").count(),
            2
        );
    }

    #[test]
    fn production_runtime_has_one_full_product_profile() {
        assert!(!chatgpt_public_review_mode());
        assert_eq!(
            chatgpt_tool_names(false, chatgpt_public_review_mode()),
            CHATGPT_FULL_TOOL_NAMES
        );
        assert_eq!(
            chatgpt_tool_names(false, true),
            CHATGPT_FULL_TOOL_NAMES,
            "the removed public-review flag cannot reduce the product"
        );
        assert!(CHATGPT_FULL_TOOL_NAMES.contains(&"prepare_moonpay_onramp"));
        assert!(CHATGPT_FULL_TOOL_NAMES.contains(&"prepare_bounty_action"));
        assert!(CHATGPT_FULL_TOOL_NAMES.contains(&"get_bounty_action_status"));
    }

    #[test]
    fn moonpay_handoff_is_first_party_and_never_claims_purchase_or_funding() {
        let handoff = build_moonpay_onramp_handoff(
            &PrepareMoonpayOnrampArgs {
                bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
                amount_base_units: 3_500_000,
                intent_id: Some("00000000-0000-4000-8000-000000000002".to_string()),
            },
            false,
        )
        .unwrap();
        let onramp_url = Url::parse(handoff["onramp_url"].as_str().unwrap()).unwrap();
        assert_eq!(
            onramp_url.origin().ascii_serialization(),
            "https://agentbounties.app"
        );
        assert_eq!(onramp_url.path(), "/onramp.html");
        assert_eq!(handoff["provider"], "moonpay");
        assert_eq!(handoff["planned_amount_usdc"], "3.5");
        assert_eq!(handoff["checkout_created"], false);
        assert_eq!(handoff["purchase_completed"], false);
        assert_eq!(handoff["bounty_funded"], false);
        assert!(handoff["canonical_funding_event"].is_null());
        assert!(handoff["evidence_boundary"]
            .as_str()
            .unwrap()
            .contains("FundingAdded"));
    }

    #[test]
    fn handoff_rejects_non_https_sources_and_invalid_money() {
        let mut args = valid_args();
        let image = sandbox_bounty_image_reference(&args).unwrap();
        args.source_url = Some("http://example.com/private".to_string());
        assert!(build_bounty_post_handoff(&args, &image)
            .unwrap_err()
            .contains("HTTPS"));

        args.source_url = None;
        args.solver_reward_usdc = "0".to_string();
        assert!(build_bounty_post_handoff(&args, &image)
            .unwrap_err()
            .contains("greater than zero"));
    }

    #[test]
    fn chatgpt_image_downloads_are_host_and_magic_bounded() {
        for allowed in [
            "https://files.oaiusercontent.com/file.png?sig=opaque",
            "https://chatgpt.com/backend-api/files/example",
            "https://files.openai.com/example",
        ] {
            assert!(
                validate_chatgpt_download_url(allowed).is_ok(),
                "expected allowed ChatGPT file URL: {allowed}"
            );
        }
        for rejected in [
            "http://files.oaiusercontent.com/file.png",
            "https://openai.com.evil.example/file.png",
            "https://user:password@files.openai.com/file.png",
            "https://127.0.0.1/file.png",
            "https://example.com/file.png",
        ] {
            assert!(
                validate_chatgpt_download_url(rejected).is_err(),
                "expected rejected ChatGPT file URL: {rejected}"
            );
        }
        assert_eq!(
            detect_bounty_image_mime(b"\x89PNG\r\n\x1a\npayload"),
            Some("image/png")
        );
        assert_eq!(
            detect_bounty_image_mime(b"\xff\xd8\xffpayload"),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_bounty_image_mime(b"RIFF\x00\x00\x00\x00WEBPpayload"),
            Some("image/webp")
        );
        assert_eq!(detect_bounty_image_mime(b"<svg></svg>"), None);
    }

    #[test]
    fn public_resource_catalog_exposes_only_the_live_feed_widget() {
        let descriptor = feed_widget_resource_descriptor();
        assert_eq!(descriptor["uri"], FEED_WIDGET_URI);
        assert_eq!(
            read_resource(&json!({"uri": FEED_WIDGET_URI})).unwrap()["contents"][0]["uri"],
            FEED_WIDGET_URI
        );
        assert_eq!(
            chatgpt_widget_domain_from_value(Some("https://community-mcp.example/path")),
            "https://community-mcp.example"
        );
        assert_eq!(
            chatgpt_widget_domain_from_value(Some("https://user:secret@example.com")),
            "https://mcp.agentbounties.app"
        );
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
        let autonomous_feed = tools
            .iter()
            .find(|tool| tool["name"] == "list_autonomous_bounties")
            .expect("canonical autonomous feed tool");
        assert!(
            autonomous_feed["outputSchema"]["properties"]["items"]["items"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("gross_cash_margin"))
        );

        assert_eq!(prepare["annotations"]["readOnlyHint"], false);
        assert_eq!(prepare["annotations"]["destructiveHint"], false);
        assert_eq!(prepare["annotations"]["openWorldHint"], false);
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
        assert_eq!(status["annotations"]["readOnlyHint"], false);
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
            "prepare_bounty_post",
            "list_autonomous_bounties",
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

        let post = tools
            .iter()
            .find(|tool| tool["name"] == "prepare_bounty_post")
            .expect("ChatGPT-account image handoff tool");
        assert_eq!(post["_meta"]["openai/fileParams"], json!(["bounty_image"]));
        assert!(post["_meta"]["ui"].get("resourceUri").is_none());
        assert!(post["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "bounty_image"));
    }

    #[test]
    fn optional_analytics_oauth_keeps_public_tools_anonymous() {
        assert_eq!(
            analytics_security_schemes(false),
            json!([{"type": "noauth"}])
        );
        assert_eq!(
            analytics_security_schemes(true),
            json!([
                {"type": "noauth"},
                {"type": "oauth2", "scopes": ["analytics:exclude-owner"]}
            ])
        );
    }

    fn public_tool_test_state() -> SharedState {
        Arc::new(AppState {
            network: Mutex::new(BountyNetwork::default()),
            eval_runs: Mutex::new(Vec::new()),
            base_rpc_urls: BaseRpcUrlConfig::default(),
            base_broadcast_enabled: false,
            stripe_secret_key: None,
            stripe_live_execution_enabled: false,
            stripe_api_base_url: "https://api.stripe.com".to_string(),
            stripe_payment_method_configuration: None,
            operator_api_token: None,
            analytics_exclusion_token: None,
            mcp_base_url: "http://127.0.0.1:8090".to_string(),
            oauth_authorizations: Mutex::new(HashMap::new()),
            oauth_codes: Mutex::new(HashMap::new()),
            store: None,
            recovery_reservations: AutonomousBountyRecoveryReservations::default(),
        })
    }

    fn modern_request(
        method: &'static str,
        mut params: Value,
        name: Option<&str>,
    ) -> (HeaderMap, Value) {
        params["_meta"] = json!({
            (MCP_PROTOCOL_VERSION_META): MCP_PROTOCOL_VERSION,
            (MCP_CLIENT_INFO_META): {
                "name": "agent-bounties-protocol-test",
                "version": "1.0.0"
            },
            (MCP_CLIENT_CAPABILITIES_META): {}
        });
        let mut headers = HeaderMap::new();
        headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            MCP_PROTOCOL_VERSION.parse().unwrap(),
        );
        headers.insert(MCP_METHOD_HEADER, method.parse().unwrap());
        if let Some(name) = name {
            headers.insert(MCP_NAME_HEADER, name.parse().unwrap());
        }
        (
            headers,
            json!({
                "jsonrpc": "2.0",
                "id": "protocol-test",
                "method": method,
                "params": params
            }),
        )
    }

    #[tokio::test]
    async fn modern_discovery_is_stateless_typed_and_cacheable() {
        let (headers, request) = modern_request("server/discover", json!({}), None);
        assert_eq!(mcp_protocol_era(&headers, &request), McpProtocolEra::Modern);
        validate_modern_request(&headers, &request).unwrap();

        let (status, response) =
            handle_request(public_tool_test_state(), request, McpProtocolEra::Modern)
                .await
                .unwrap();
        let result = &response["result"];
        assert_eq!(status, StatusCode::OK);
        assert_eq!(result["supportedVersions"], json!([MCP_PROTOCOL_VERSION]));
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["ttlMs"], MCP_CATALOG_TTL_MS);
        assert_eq!(result["cacheScope"], "public");
        assert_eq!(
            result["_meta"][MCP_SERVER_INFO_META]["version"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(result["capabilities"]["tools"]["listChanged"], false);
        assert_eq!(result["capabilities"]["resources"]["subscribe"], false);
        assert!(result.get("protocolVersion").is_none());
    }

    #[tokio::test]
    async fn modern_tool_catalog_is_deterministic_typed_and_cacheable() {
        let (headers, request) = modern_request("tools/list", json!({}), None);
        validate_modern_request(&headers, &request).unwrap();
        let (_, response) =
            handle_request(public_tool_test_state(), request, McpProtocolEra::Modern)
                .await
                .unwrap();
        let result = &response["result"];
        let names = result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        let mut sorted_names = names.clone();
        sorted_names.sort_unstable();

        assert_eq!(names, sorted_names);
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["ttlMs"], MCP_CATALOG_TTL_MS);
        assert_eq!(result["cacheScope"], "public");
        assert!(result["_meta"][MCP_SERVER_INFO_META].is_object());
    }

    #[test]
    fn modern_headers_must_match_the_request_body() {
        let (mut headers, request) = modern_request("tools/list", json!({}), None);
        headers.insert(MCP_METHOD_HEADER, "resources/list".parse().unwrap());
        let error = validate_modern_request(&headers, &request).unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, -32020);

        let (mut headers, mut request) = modern_request("tools/list", json!({}), None);
        headers.insert(MCP_PROTOCOL_VERSION_HEADER, "2099-01-01".parse().unwrap());
        request["params"]["_meta"][MCP_PROTOCOL_VERSION_META] = json!("2099-01-01");
        let error = validate_modern_request(&headers, &request).unwrap_err();
        assert_eq!(error.code, -32022);
        assert_eq!(
            error.data.unwrap()["supported"],
            json!([MCP_PROTOCOL_VERSION])
        );
    }

    #[test]
    fn modern_name_header_supports_the_required_base64_sentinel() {
        let resource_uri = "ui://agent-bounties/世界.html";
        let encoded = format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(resource_uri)
        );
        let (mut headers, request) =
            modern_request("resources/read", json!({"uri": resource_uri}), None);
        headers.insert(MCP_NAME_HEADER, encoded.parse().unwrap());

        validate_modern_request(&headers, &request).unwrap();
        assert_eq!(decode_mcp_header_value(&encoded).unwrap(), resource_uri);
        assert_eq!(decode_mcp_header_value("ordinary?=").unwrap(), "ordinary?=");
        assert!(decode_mcp_header_value("=?base64?not-valid?=").is_err());
    }

    #[tokio::test]
    async fn legacy_initialize_remains_available_without_modern_metadata() {
        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {"protocolVersion": MCP_LEGACY_PROTOCOL_VERSION}
        });
        assert_eq!(
            mcp_protocol_era(&HeaderMap::new(), &request),
            McpProtocolEra::Legacy
        );
        let (status, response) =
            handle_request(public_tool_test_state(), request, McpProtocolEra::Legacy)
                .await
                .unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response["result"]["protocolVersion"],
            MCP_LEGACY_PROTOCOL_VERSION
        );
        assert!(response["result"].get("resultType").is_none());

        let mut legacy_headers = HeaderMap::new();
        legacy_headers.insert(
            MCP_PROTOCOL_VERSION_HEADER,
            MCP_LEGACY_PROTOCOL_VERSION.parse().unwrap(),
        );
        assert_eq!(
            mcp_protocol_era(
                &legacy_headers,
                &json!({"method": "tools/list", "params": {}})
            ),
            McpProtocolEra::Legacy
        );
        legacy_headers.insert(MCP_PROTOCOL_VERSION_HEADER, "2099-01-01".parse().unwrap());
        assert_eq!(
            mcp_protocol_era(
                &legacy_headers,
                &json!({"method": "tools/list", "params": {}})
            ),
            McpProtocolEra::Modern
        );
    }

    #[tokio::test]
    async fn removed_handshake_methods_are_not_exposed_to_modern_clients() {
        let (headers, request) = modern_request("initialize", json!({}), None);
        validate_modern_request(&headers, &request).unwrap();
        let (status, response) =
            handle_request(public_tool_test_state(), request, McpProtocolEra::Modern)
                .await
                .unwrap();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn origin_validation_is_exact_and_configurable() {
        assert!(mcp_origin_is_allowed_with_config(
            "https://chatgpt.com",
            None,
            None
        ));
        assert!(mcp_origin_is_allowed_with_config(
            "http://127.0.0.1:3000",
            None,
            None
        ));
        assert!(mcp_origin_is_allowed_with_config(
            "https://tenant.example",
            Some("https://tenant.example"),
            None
        ));
        assert!(mcp_origin_is_allowed_with_config(
            "https://client.example",
            None,
            Some("https://one.example, https://client.example")
        ));
        for rejected in [
            "null",
            "http://chatgpt.com",
            "https://chatgpt.com.evil.example",
            "https://user:secret@chatgpt.com",
            "https://chatgpt.com/path",
        ] {
            assert!(
                !mcp_origin_is_allowed_with_config(rejected, None, None),
                "expected rejected Origin: {rejected}"
            );
        }
    }

    #[tokio::test]
    async fn mounted_public_inventory_tool_is_callable_and_fails_closed() {
        let params = json!({
            "name": "list_autonomous_bounties",
            "arguments": {"network": "base-mainnet", "claimable_only": true}
        });
        let result = call_tool(public_tool_test_state(), &params).await.unwrap();
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("unknown or unavailable public ChatGPT app tool"));
        assert!(encoded.contains("DATABASE_URL"));
        assert!(!encoded.contains("\"paid\":true"));

        let error = call_tool(
            public_tool_test_state(),
            &json!({"name": "not_a_real_tool", "arguments": {}}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("unknown or unavailable public ChatGPT app tool"));
    }

    #[test]
    fn feed_resource_is_embedded_as_an_interactive_mcp_app() {
        let contents = feed_widget_resource_contents_for_mode(false);
        let html = contents["text"].as_str().unwrap();
        assert_eq!(contents["mimeType"], "text/html;profile=mcp-app");
        assert_eq!(contents["uri"], FEED_WIDGET_URI);
        let redirect_domains = contents["_meta"]["openai/widgetCSP"]["redirect_domains"]
            .as_array()
            .expect("redirect domains");
        for expected in [
            "https://mcp.agentbounties.app",
            "https://agentbounties.app",
            "https://x.com",
            "https://www.linkedin.com",
            "https://www.instagram.com",
        ] {
            assert!(
                redirect_domains.iter().any(|value| value == expected),
                "missing redirect domain {expected}"
            );
        }
        assert!(html.contains("class=\"project-thumb\""));
        assert!(html.contains("bridgeNotify(\"ui/message\", message)"));
        assert!(html.contains("openai()?.sendFollowUpMessage"));
        assert!(!html.contains("ui/download-file"));
        assert!(!html.contains("__CHATGPT_PUBLIC_REVIEW_MODE__"));
        assert!(!contents["_meta"]["openai/widgetDescription"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .contains("pokémon"));
        for outdated_term in ["collectible", "quest card", "instagram-inspired"] {
            assert!(
                !contents["_meta"]["openai/widgetDescription"]
                    .as_str()
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains(outdated_term),
                "widget description contains outdated visual term {outdated_term}"
            );
        }
        assert!(html.contains("callTool(\"get_bounty_feed\""));
        for forbidden in [
            "callTool(\"add_bounty_comment\"",
            "callTool(\"create_share_bundle\"",
            "callTool(\"prepare_moonpay_onramp\"",
            "callTool(\"publish_unfunded_bounty\"",
            "callTool(\"compile_objective_with_cloud_agent\"",
            "callTool(\"prepare_bounty_action\"",
            "callTool(\"get_bounty_action_status\"",
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
            "data-action=\"post-bounty\"",
            "data-action=\"comment\"",
            "data-action=\"share\"",
            "data-action=\"solve\"",
            ">Post bounty<",
            ">Comment<",
            ">Share<",
            ">Solve<",
        ] {
            assert!(
                html.contains(visible_control),
                "feed widget must render {visible_control}"
            );
        }
        for forbidden_element in ["<input", "<textarea", "<select", "<form"] {
            assert!(
                !html.to_ascii_lowercase().contains(forbidden_element),
                "feed widget must not render {forbidden_element}"
            );
        }
        assert_eq!(html.matches("<button").count(), 4);
        assert!(html.contains("data:image/webp;base64,"));
        assert!(!html.contains("__BOUNTY_CARD_ART_DATA_URI__"));
        assert!(!html.contains("__CHATGPT_APP_BASE_URL_JSON__"));
        assert!(html.contains("bridgeRequest(\"tools/call\""));
        assert!(!html.contains("window.location.replace"));
        assert!(html.contains("Preview data · no writes"));
        assert!(html.contains("Safe fixture data"));
        assert!(!html.contains("open for competition"));
        assert!(!html.contains("ready to compete"));
        assert!(!html.contains("#2563eb"));
        assert!(html.contains("#020b08"));
        assert!(html.contains("#b9ef37"));
        assert!(html.contains("#18d9ac"));
        assert!(html.contains("#e8bd26"));
    }

    #[test]
    fn legacy_public_review_argument_cannot_reduce_the_widget() {
        let contents = feed_widget_resource_contents_for_mode(true);
        let redirect_domains = contents["_meta"]["ui"]["csp"]["redirectDomains"]
            .as_array()
            .expect("redirect domains");
        for expected in [
            "https://mcp.agentbounties.app",
            "https://agentbounties.app",
            "https://x.com",
            "https://www.linkedin.com",
            "https://www.instagram.com",
        ] {
            assert!(redirect_domains.iter().any(|domain| domain == expected));
        }
        assert!(contents["text"]
            .as_str()
            .unwrap()
            .contains("const APP_PUBLIC_REVIEW = false;"));
        assert!(contents["_meta"]["openai/widgetDescription"]
            .as_str()
            .unwrap()
            .contains("Post bounty, Comment, Share, and Solve"));
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
            bounty_url: Some("https://agentbounties.app/bounties/bounty-42".to_string()),
            status: "submitted; awaiting verifier".to_string(),
            reward: Some("12 USDC".to_string()),
            payment_state: Some("escrowed".to_string()),
            bounty_image_url: Some(format!(
                "https://mcp.agentbounties.app/public/bounty-images/{}",
                "ab".repeat(32)
            )),
        };
        let bundle = build_share_bundle(&args, false).unwrap();
        assert_eq!(bundle["stage"], "verification");
        assert!(bundle["caption"]
            .as_str()
            .unwrap()
            .contains("#AgentBounties"));
        assert!(bundle["intents"]["x"]
            .as_str()
            .unwrap()
            .contains("intent/post"));
        assert!(bundle["bounty_image_url"]
            .as_str()
            .unwrap()
            .contains("/public/bounty-images/"));
        assert!(bundle["evidence_boundary"]
            .as_str()
            .unwrap()
            .contains("not canonical"));

        let mut unsafe_args = args;
        unsafe_args.bounty_url = Some("javascript:alert(1)".to_string());
        assert!(build_share_bundle(&unsafe_args, false).is_err());
    }

    #[tokio::test]
    async fn sandbox_descriptors_are_read_only_and_explicit() {
        let mut descriptors = tools().await.0;
        descriptors.extend(custom_tool_descriptors());
        let sandbox_tools = descriptors
            .into_iter()
            .filter(|descriptor| CHATGPT_FULL_TOOL_NAMES.contains(&descriptor.name))
            .map(|descriptor| mcp_tool_descriptor_for_mode(descriptor, true, false))
            .collect::<Vec<_>>();

        assert_eq!(sandbox_tools.len(), CHATGPT_FULL_TOOL_NAMES.len());
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
    async fn legacy_public_review_argument_cannot_reduce_the_tool_surface() {
        let mut descriptors = tools().await.0;
        descriptors.extend(custom_tool_descriptors());
        let public_tools = descriptors
            .into_iter()
            .filter(|descriptor| chatgpt_tool_names(false, true).contains(&descriptor.name))
            .map(|descriptor| mcp_tool_descriptor_for_mode(descriptor, false, true))
            .collect::<Vec<_>>();

        assert_eq!(public_tools.len(), CHATGPT_FULL_TOOL_NAMES.len());
        for required in [
            "prepare_moonpay_onramp",
            "prepare_bounty_action",
            "get_bounty_action_status",
        ] {
            assert!(public_tools.iter().any(|tool| tool["name"] == required));
        }
        let feed = public_tools
            .iter()
            .find(|tool| tool["name"] == "get_bounty_feed")
            .unwrap();
        assert!(feed["inputSchema"]["properties"]
            .get("payment_state")
            .is_some());
        assert!(feed["inputSchema"]["properties"]
            .get("source_type")
            .is_some());
        assert_eq!(feed["_meta"]["agentBountiesPublicReview"], false);

        let compiler = public_tools
            .iter()
            .find(|tool| tool["name"] == "compile_objective_with_cloud_agent")
            .unwrap();
        assert!(compiler["inputSchema"]["properties"]
            .get("solver_budget_usdc")
            .is_some());
        assert!(compiler["inputSchema"]["properties"]
            .get("source_url")
            .is_some());
        assert_eq!(compiler["annotations"]["readOnlyHint"], false);
        assert_eq!(compiler["annotations"]["openWorldHint"], true);
        assert_eq!(compiler["annotations"]["idempotentHint"], false);

        let share = public_tools
            .iter()
            .find(|tool| tool["name"] == "create_share_bundle")
            .unwrap();
        assert!(share["inputSchema"]["properties"]
            .get("bounty_url")
            .is_some());
        assert!(share["inputSchema"]["properties"].get("reward").is_some());
        assert!(share["inputSchema"]["properties"]
            .get("payment_state")
            .is_some());
    }

    #[test]
    fn public_review_feed_filter_is_fail_closed() {
        let mut projection = json!({
            "source_statuses": [
                {"source_type": "canonical_base", "authoritative_urls": ["https://agentbounties.app/earn.html"]}
            ],
            "items": [
                {
                    "opportunity_id": "unfunded:volunteer-docs",
                    "source_id": "volunteer-docs",
                    "source_type": "unfunded_offchain",
                    "payment_state": "none",
                    "work_state": "open",
                    "title": "Publish an accessibility checklist",
                    "goal": "Write a concise public checklist.",
                    "public_url": "https://agentbounties.app/earn.html",
                    "next_action": {"action": "submit_unfunded_bounty_solution", "url": "https://agentbounties.app/paid"},
                    "reward": {"amount": "0", "currency": "USDC"},
                    "evidence_requirements": {"acceptance_criteria": ["Include keyboard-only checks."]}
                },
                {"source_type": "canonical_base", "payment_state": "escrowed", "work_state": "claimable"},
                {"source_type": "unfunded_offchain", "payment_state": "seeking_funding", "work_state": "open"},
                {
                    "opportunity_id": "unfunded:economic",
                    "source_type": "unfunded_offchain",
                    "payment_state": "none",
                    "work_state": "open",
                    "title": "Pay 10 USDC for this task",
                    "goal": "Transfer a token reward.",
                    "evidence_requirements": {"acceptance_criteria": []}
                }
            ]
        });
        constrain_public_review_feed(&mut projection).unwrap();
        assert_eq!(projection["items"].as_array().unwrap().len(), 1);
        let item = &projection["items"][0];
        assert!(item.get("reward").is_none());
        assert!(item.get("payment_state").is_none());
        assert!(item.get("payment_committed").is_none());
        assert!(item.get("payment_authority").is_none());
        assert!(item.get("competition_mode").is_none());
        assert!(item.get("verification_method").is_none());
        assert!(item.get("verification_ready").is_none());
        assert!(item.get("next_action").is_none());
        assert!(item.get("embeds").is_none());
        assert_eq!(item["source_type"], "voluntary_request");
        assert_eq!(item["request_status"], "open");
        assert_eq!(item["access"], "voluntary");
        assert!(item["public_url"]
            .as_str()
            .unwrap()
            .contains("/chatgpt/bounty-card-preview?"));
        assert!(item["public_url"]
            .as_str()
            .unwrap()
            .contains("public_review=1"));
        assert_eq!(projection["source_statuses"].as_array().unwrap().len(), 1);
        assert_eq!(
            projection["source_statuses"][0]["source_type"],
            "voluntary_request"
        );
        assert_eq!(
            projection["source_statuses"][0]["authoritative_urls"],
            json!([])
        );

        let mut plan = json!({
            "title": "Release plan",
            "objective": "Publish an onboarding guide",
            "success_definition": "The solver receives canonical payment.",
            "solver_budget_usdc": "10.00",
            "settlement_policy": {"asset": "USDC"},
            "source_url": "https://agentbounties.app/earn.html",
            "next_action": "Fund the child tasks",
            "tasks": [{
                "task_id": "task-1",
                "title": "Pay the writer",
                "goal": "Create the guide",
                "acceptance_criteria": [
                    "Publish a concise guide.",
                    "Send 10 USDC to the wallet."
                ],
                "suggested_solver_reward_usdc": "10.00"
            }]
        });
        constrain_public_review_objective_plan(&mut plan);
        assert!(plan.get("solver_budget_usdc").is_none());
        assert!(plan.get("settlement_policy").is_none());
        assert!(plan.get("source_url").is_none());
        assert!(plan.get("next_action").is_none());
        assert!(plan["tasks"][0]
            .get("suggested_solver_reward_usdc")
            .is_none());
        assert_eq!(plan["tasks"][0]["title"], "Review task 1");
        assert_eq!(
            plan["tasks"][0]["acceptance_criteria"],
            json!(["Publish a concise guide."])
        );
        assert!(plan["success_definition"]
            .as_str()
            .unwrap()
            .starts_with("All task drafts"));

        let mut request = json!({
            "bounty_kind": "unfunded_offchain",
            "payment_promised": false,
            "upgrade_url": "https://agentbounties.app/post.html"
        });
        strip_public_unfunded_navigation(&mut request);
        assert!(request.get("upgrade_url").is_none());
        assert!(request.get("payment_promised").is_none());
        assert!(request["public_url"]
            .as_str()
            .unwrap()
            .contains("public_review=1"));
    }

    #[test]
    fn public_review_rejects_commerce_language_and_external_links() {
        for blocked in [
            "Pay 10 USDC",
            "Connect a wallet",
            "Use MoonPay checkout",
            "Send the reward to 0x0000000000000000000000000000000000000000",
            "See https://example.com/offer",
        ] {
            assert!(
                ensure_public_review_noncommercial_text(blocked).is_err(),
                "{blocked}"
            );
        }
        for allowed in [
            "Publish an accessible onboarding checklist",
            "Review the public evidence and leave a constructive comment",
        ] {
            ensure_public_review_noncommercial_text(allowed).unwrap();
        }
        ensure_public_review_opportunity_id("unfunded:request-1").unwrap();
        assert!(ensure_public_review_opportunity_id(
            "canonical:base-mainnet:0x0000000000000000000000000000000000000000"
        )
        .is_err());
    }

    #[test]
    fn public_review_share_bundle_uses_only_the_noncommercial_card_origin() {
        let bundle = build_share_bundle(
            &ShareBundleArgs {
                bounty_id: "unfunded:community-request".to_string(),
                title: "Publish an accessibility checklist".to_string(),
                stage: "request shared".to_string(),
                bounty_url: None,
                status: "open".to_string(),
                reward: None,
                payment_state: None,
                bounty_image_url: None,
            },
            true,
        )
        .unwrap();
        assert!(bundle["share_url"]
            .as_str()
            .unwrap()
            .contains("/chatgpt/bounty-card-preview?"));
        assert!(!bundle["share_url"].as_str().unwrap().contains("reward="));
        assert!(bundle["caption"]
            .as_str()
            .unwrap()
            .contains("no payment promise"));
        assert!(!bundle["caption"].as_str().unwrap().contains("USDC"));
        assert!(bundle["evidence_boundary"]
            .as_str()
            .unwrap()
            .contains("paid-service checkout"));
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
        let comments = comment["structuredContent"]["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 2);
        for item in comments {
            assert!(item["id"].is_string(), "{item}");
            assert!(item["author"].is_string(), "{item}");
            assert!(item["body"].is_string(), "{item}");
            assert!(item["created_at"].is_string(), "{item}");
        }

        for action in ["post", "fund", "solve", "complete", "verify"] {
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
