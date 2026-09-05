//! Bounded provider runtime for authenticated webhook-to-draft and
//! origin-write-to-HTTP-request planning.
//!
//! The runtime verifies HMACs over the exact raw request body and emits
//! allowlisted request plans. It never stores credentials or performs network,
//! wallet, payment, verifier, or settlement actions.

use super::{
    github::{plan_github_webhook_origin_draft, GitHubWebhookDraftInput, GitHubWebhookTrigger},
    linear::{plan_linear_origin_draft, LinearWebhookDraftInput, LinearWebhookTrigger},
    stable_hash, validate_source, OriginAuthorityBoundary, OriginDraftPlan, OriginProvider,
    OriginSourceReference, OriginWriteOperation,
};
use domain::Money;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const MAX_WEBHOOK_BODY_BYTES: usize = 1_048_576;
const MAX_PROVIDER_COMMENT_BYTES: usize = 65_536;
const GITHUB_API_ROOT: &str = "https://api.github.com";
const LINEAR_GRAPHQL_URL: &str = "https://api.linear.app/graphql";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRuntimeConfig {
    pub app_login: String,
    pub assignment_solver_reward: Money,
    pub mention_solver_reward: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearRuntimeConfig {
    pub agent_id: String,
    pub agent_mention: String,
    pub delegation_solver_reward: Money,
    pub mention_solver_reward: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthenticatedOriginDraftPlan {
    pub authenticated: bool,
    pub provider: OriginProvider,
    pub event_id: Option<String>,
    pub draft_plan: Option<OriginDraftPlan>,
    pub error: Option<String>,
    pub authority: OriginAuthorityBoundary,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubRepositoryProjection {
    full_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubActorProjection {
    login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubInstallationProjection {
    id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubIssueProjection {
    number: u64,
    html_url: String,
    title: String,
    body: Option<String>,
    #[serde(default)]
    pull_request: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubCommentProjection {
    id: u64,
    body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct GitHubWebhookProjection {
    action: String,
    repository: GitHubRepositoryProjection,
    sender: GitHubActorProjection,
    installation: GitHubInstallationProjection,
    issue: GitHubIssueProjection,
    assignee: Option<GitHubActorProjection>,
    comment: Option<GitHubCommentProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LinearActorProjection {
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LinearAssigneeProjection {
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearIssueProjection {
    id: String,
    identifier: String,
    url: String,
    title: String,
    description: Option<String>,
    assignee: Option<LinearAssigneeProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct LinearCommentProjection {
    id: String,
    body: String,
    issue: LinearIssueProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinearWebhookProjection {
    action: String,
    #[serde(rename = "type")]
    event_type: String,
    organization_id: String,
    webhook_id: String,
    actor: LinearActorProjection,
    data: Value,
}

pub fn authenticate_and_plan_github_webhook(
    raw_body: &[u8],
    signature_header: &str,
    event_header: &str,
    delivery_id: &str,
    webhook_secret: &[u8],
    config: &GitHubRuntimeConfig,
    existing_idempotency_keys: Vec<String>,
) -> AuthenticatedOriginDraftPlan {
    let provider = OriginProvider::GitHub;
    if let Err(error) = verify_github_signature(raw_body, signature_header, webhook_secret) {
        return invalid_authenticated_plan(provider, None, error);
    }
    if delivery_id.trim().is_empty() {
        return invalid_authenticated_plan(
            provider,
            None,
            "missing GitHub delivery id".to_string(),
        );
    }
    if config.app_login.trim().is_empty() {
        return invalid_authenticated_plan(
            provider,
            Some(delivery_id.to_string()),
            "GitHub runtime requires the configured app login".to_string(),
        );
    }
    let projection: GitHubWebhookProjection = match parse_bounded_json(raw_body) {
        Ok(projection) => projection,
        Err(error) => return invalid_authenticated_plan(provider, None, error),
    };
    if projection.installation.id == 0 {
        return invalid_authenticated_plan(
            provider,
            Some(delivery_id.to_string()),
            "GitHub event is missing a valid app installation".to_string(),
        );
    }
    if projection.issue.pull_request.is_some() {
        return invalid_authenticated_plan(
            provider,
            Some(delivery_id.to_string()),
            "pull-request events are not supported by the issue origin runtime".to_string(),
        );
    }
    let expected_issue_url = format!(
        "https://github.com/{}/issues/{}",
        projection.repository.full_name, projection.issue.number
    );
    if projection.issue.html_url.split(['?', '#']).next() != Some(expected_issue_url.as_str()) {
        return invalid_authenticated_plan(
            provider,
            Some(delivery_id.to_string()),
            "GitHub event repository, issue number, and issue URL do not match".to_string(),
        );
    }

    let (trigger, solver_reward, signed_event_id) = match event_header.trim() {
        "issues" if projection.action == "assigned" => {
            let Some(assignee) = projection.assignee.as_ref() else {
                return invalid_authenticated_plan(
                    provider,
                    Some(delivery_id.to_string()),
                    "assigned event is missing its assignee".to_string(),
                );
            };
            if !assignee.login.eq_ignore_ascii_case(config.app_login.trim()) {
                return invalid_authenticated_plan(
                    provider,
                    Some(delivery_id.to_string()),
                    "issue assignment does not target the configured GitHub App".to_string(),
                );
            }
            (
                GitHubWebhookTrigger::Assignment,
                config.assignment_solver_reward.clone(),
                format!(
                    "github:issues:assigned:{}:{}:{}",
                    projection.repository.full_name.to_ascii_lowercase(),
                    projection.issue.number,
                    assignee.login.to_ascii_lowercase()
                ),
            )
        }
        "issue_comment"
            if matches!(projection.action.as_str(), "created" | "edited") =>
        {
            let Some(comment) = projection.comment.as_ref() else {
                return invalid_authenticated_plan(
                    provider,
                    Some(delivery_id.to_string()),
                    "issue_comment event is missing comment data".to_string(),
                );
            };
            if comment.id == 0 || !contains_mention(&comment.body, &config.app_login) {
                return invalid_authenticated_plan(
                    provider,
                    Some(delivery_id.to_string()),
                    "issue comment does not contain a valid configured app mention".to_string(),
                );
            }
            (
                GitHubWebhookTrigger::Mention,
                config.mention_solver_reward.clone(),
                format!(
                    "github:issue_comment:{}:{}:{}:{}",
                    projection.action,
                    projection.repository.full_name.to_ascii_lowercase(),
                    comment.id,
                    stable_hash(&comment.body)
                ),
            )
        }
        _ => {
            return invalid_authenticated_plan(
                provider,
                Some(delivery_id.to_string()),
                "unsupported GitHub event; allowlisted events are issues/assigned and issue_comment/created|edited"
                    .to_string(),
            )
        }
    };

    let input = GitHubWebhookDraftInput {
        repository: projection.repository.full_name,
        issue_number: projection.issue.number.to_string(),
        issue_url: projection.issue.html_url,
        title: projection.issue.title,
        body: projection.issue.body.unwrap_or_default(),
        solver_reward,
        trigger,
        actor_login: Some(projection.sender.login),
        event_id: signed_event_id.clone(),
        existing_idempotency_keys,
    };
    AuthenticatedOriginDraftPlan {
        authenticated: true,
        provider,
        event_id: Some(signed_event_id),
        draft_plan: Some(plan_github_webhook_origin_draft(input)),
        error: None,
        authority: OriginAuthorityBoundary::default(),
    }
}

pub fn authenticate_and_plan_linear_webhook(
    raw_body: &[u8],
    signature_header: &str,
    webhook_secret: &[u8],
    config: &LinearRuntimeConfig,
    existing_idempotency_keys: Vec<String>,
) -> AuthenticatedOriginDraftPlan {
    let provider = OriginProvider::Linear;
    if let Err(error) = verify_linear_signature(raw_body, signature_header, webhook_secret) {
        return invalid_authenticated_plan(provider, None, error);
    }
    if config.agent_id.trim().is_empty() || config.agent_mention.trim().is_empty() {
        return invalid_authenticated_plan(
            provider,
            None,
            "Linear runtime requires configured agent id and mention".to_string(),
        );
    }
    let projection: LinearWebhookProjection = match parse_bounded_json(raw_body) {
        Ok(projection) => projection,
        Err(error) => return invalid_authenticated_plan(provider, None, error),
    };
    if projection.organization_id.trim().is_empty()
        || projection.webhook_id.trim().is_empty()
        || projection.actor.id.trim().is_empty()
    {
        return invalid_authenticated_plan(
            provider,
            None,
            "Linear event is missing organization, webhook, or actor identity".to_string(),
        );
    }

    let (issue, command_text, trigger, solver_reward, event_subject_id) =
        match (projection.event_type.as_str(), projection.action.as_str()) {
            ("Issue", "update") => {
                let issue: LinearIssueProjection = match serde_json::from_value(projection.data) {
                    Ok(issue) => issue,
                    Err(_) => {
                        return invalid_authenticated_plan(
                            provider,
                            Some(projection.webhook_id),
                            "Linear Issue/update projection is malformed".to_string(),
                        )
                    }
                };
                if issue.assignee.as_ref().map(|assignee| assignee.id.as_str())
                    != Some(config.agent_id.trim())
                {
                    return invalid_authenticated_plan(
                        provider,
                        Some(projection.webhook_id),
                        "Linear issue delegation does not target the configured agent".to_string(),
                    );
                }
                let command = format!(
                    "/agent-bounty create {} USDC",
                    usdc_major(&config.delegation_solver_reward)
                );
                let subject_id = issue.id.clone();
                (
                    issue,
                    command,
                    LinearWebhookTrigger::Delegation,
                    config.delegation_solver_reward.clone(),
                    subject_id,
                )
            }
            ("Comment", "create" | "update") => {
                let comment: LinearCommentProjection =
                    match serde_json::from_value(projection.data) {
                        Ok(comment) => comment,
                        Err(_) => {
                            return invalid_authenticated_plan(
                                provider,
                                Some(projection.webhook_id),
                                "Linear Comment projection is malformed".to_string(),
                            )
                        }
                    };
                if !contains_mention(&comment.body, &config.agent_mention) {
                    return invalid_authenticated_plan(
                        provider,
                        Some(projection.webhook_id),
                        "Linear comment does not contain the configured agent mention".to_string(),
                    );
                }
                let command = format!(
                    "/agent-bounty create {} USDC",
                    usdc_major(&config.mention_solver_reward)
                );
                (
                    comment.issue,
                    command,
                    LinearWebhookTrigger::Mention,
                    config.mention_solver_reward.clone(),
                    comment.id,
                )
            }
            _ => {
                return invalid_authenticated_plan(
                    provider,
                    Some(projection.webhook_id),
                    "unsupported Linear event; allowlisted events are Issue/update delegation and Comment/create|update mention"
                        .to_string(),
                )
            }
        };

    let event_id = format!(
        "{}:{}:{}:{}",
        projection.webhook_id, projection.event_type, projection.action, event_subject_id
    );
    let input = LinearWebhookDraftInput {
        workspace_id: projection.organization_id,
        issue_id: issue.id,
        identifier: issue.identifier,
        issue_url: issue.url,
        title: issue.title,
        description: issue.description.unwrap_or_default(),
        command_text,
        trigger,
        actor_id: Some(projection.actor.id),
        event_id: Some(event_id.clone()),
        existing_idempotency_keys,
    };
    let mut draft_plan = plan_linear_origin_draft(input);
    if let Some(draft) = draft_plan.draft.as_ref() {
        if draft.reward.solver != solver_reward {
            draft_plan = OriginDraftPlan {
                ready_for_human_review: false,
                draft: None,
                idempotency_key: draft_plan.idempotency_key,
                duplicate: false,
                error: Some("Linear runtime reward projection mismatch".to_string()),
                authority: OriginAuthorityBoundary::default(),
            };
        }
    }
    AuthenticatedOriginDraftPlan {
        authenticated: true,
        provider,
        event_id: Some(event_id),
        draft_plan: Some(draft_plan),
        error: None,
        authority: OriginAuthorityBoundary::default(),
    }
}

pub fn verify_github_signature(
    raw_body: &[u8],
    signature_header: &str,
    webhook_secret: &[u8],
) -> Result<(), String> {
    let signature = signature_header
        .trim()
        .strip_prefix("sha256=")
        .ok_or_else(|| "GitHub signature must use the sha256= scheme".to_string())?;
    verify_hmac_hex(raw_body, signature, webhook_secret)
        .map_err(|_| "invalid GitHub webhook signature".to_string())
}

pub fn verify_linear_signature(
    raw_body: &[u8],
    signature_header: &str,
    webhook_secret: &[u8],
) -> Result<(), String> {
    if signature_header.trim().starts_with("sha256=") {
        return Err("Linear signature must be an unprefixed SHA-256 HMAC hex digest".to_string());
    }
    verify_hmac_hex(raw_body, signature_header.trim(), webhook_secret)
        .map_err(|_| "invalid Linear webhook signature".to_string())
}

fn verify_hmac_hex(raw_body: &[u8], signature: &str, secret: &[u8]) -> Result<(), ()> {
    if raw_body.len() > MAX_WEBHOOK_BODY_BYTES || secret.is_empty() {
        return Err(());
    }
    let signature = hex::decode(signature).map_err(|_| ())?;
    if signature.len() != 32 {
        return Err(());
    }
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| ())?;
    mac.update(raw_body);
    mac.verify_slice(&signature).map_err(|_| ())
}

fn parse_bounded_json<T>(raw_body: &[u8]) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if raw_body.is_empty() || raw_body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err("webhook body is empty or exceeds the 1 MiB limit".to_string());
    }
    serde_json::from_slice(raw_body)
        .map_err(|_| "webhook body is not a supported JSON projection".to_string())
}

fn contains_mention(text: &str, login: &str) -> bool {
    let login = login.trim().trim_start_matches('@').to_ascii_lowercase();
    if login.is_empty() {
        return false;
    }
    let needle = format!("@{login}");
    let lower = text.to_ascii_lowercase();
    lower.match_indices(&needle).any(|(index, _)| {
        let following = lower.as_bytes().get(index + needle.len()).copied();
        following.is_none_or(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
    })
}

fn usdc_major(reward: &Money) -> String {
    let whole = reward.amount / 1_000_000;
    let fractional = reward.amount.rem_euclid(1_000_000);
    if fractional == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fractional:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

fn invalid_authenticated_plan(
    provider: OriginProvider,
    event_id: Option<String>,
    error: String,
) -> AuthenticatedOriginDraftPlan {
    AuthenticatedOriginDraftPlan {
        authenticated: false,
        provider,
        event_id,
        draft_plan: None,
        error: Some(error),
        authority: OriginAuthorityBoundary::default(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCredentialKind {
    GitHubInstallationToken,
    LinearOAuthAccessToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHttpMethod {
    Post,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHttpRequestPlan {
    pub sequence: u32,
    pub operation_idempotency_key: String,
    pub depends_on_idempotency_key: Option<String>,
    pub method: ProviderHttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
    pub credential_kind: ProviderCredentialKind,
    pub already_applied: bool,
    pub network_write_performed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum ProviderRequestBinding {
    GitHub {
        existing_status_comment_id: Option<u64>,
    },
    Linear {
        existing_status_comment_id: Option<String>,
        completed_state_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHttpRequestBatchPlan {
    pub ready: bool,
    pub provider: OriginProvider,
    pub requests: Vec<ProviderHttpRequestPlan>,
    pub error: Option<String>,
    pub authority: OriginAuthorityBoundary,
}

pub fn plan_provider_http_requests(
    source: &OriginSourceReference,
    operations: &[OriginWriteOperation],
    binding: ProviderRequestBinding,
) -> ProviderHttpRequestBatchPlan {
    let result = validate_source(source)
        .and_then(|_| validate_operation_shape(operations))
        .and_then(|_| match (&source.provider, &binding) {
            (OriginProvider::GitHub, ProviderRequestBinding::GitHub { .. }) => {
                plan_github_requests(source, operations, &binding)
            }
            (OriginProvider::Linear, ProviderRequestBinding::Linear { .. }) => {
                plan_linear_requests(source, operations, &binding)
            }
            _ => Err("provider request binding does not match the source provider".to_string()),
        });
    match result {
        Ok(requests) => ProviderHttpRequestBatchPlan {
            ready: true,
            provider: source.provider,
            requests,
            error: None,
            authority: OriginAuthorityBoundary::default(),
        },
        Err(error) => ProviderHttpRequestBatchPlan {
            ready: false,
            provider: source.provider,
            requests: vec![],
            error: Some(error),
            authority: OriginAuthorityBoundary::default(),
        },
    }
}

fn plan_github_requests(
    source: &OriginSourceReference,
    operations: &[OriginWriteOperation],
    binding: &ProviderRequestBinding,
) -> Result<Vec<ProviderHttpRequestPlan>, String> {
    let ProviderRequestBinding::GitHub {
        existing_status_comment_id,
    } = binding
    else {
        unreachable!();
    };
    let (owner, repository) = github_repository_parts(&source.workspace)?;
    let issue_number = source
        .external_id
        .parse::<u64>()
        .map_err(|_| "GitHub source external id must be an issue number".to_string())?;
    let expected_source_url =
        format!("https://github.com/{owner}/{repository}/issues/{issue_number}");
    if source.url.split(['?', '#']).next() != Some(expected_source_url.as_str()) {
        return Err("GitHub source URL does not match its workspace and issue id".to_string());
    }
    let issue_url = format!("{GITHUB_API_ROOT}/repos/{owner}/{repository}/issues/{issue_number}");
    let mut requests = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            OriginWriteOperation::UpsertStatusComment {
                idempotency_key,
                stable_marker,
                markdown,
                already_applied,
            } => {
                validate_status_comment(stable_marker, markdown)?;
                let (method, url) = match existing_status_comment_id {
                    Some(comment_id) if *comment_id > 0 => (
                        ProviderHttpMethod::Patch,
                        format!(
                            "{GITHUB_API_ROOT}/repos/{owner}/{repository}/issues/comments/{comment_id}"
                        ),
                    ),
                    Some(_) => return Err("GitHub comment id must be positive".to_string()),
                    None => (
                        ProviderHttpMethod::Post,
                        format!("{issue_url}/comments"),
                    ),
                };
                requests.push(ProviderHttpRequestPlan {
                    sequence: index as u32,
                    operation_idempotency_key: idempotency_key.clone(),
                    depends_on_idempotency_key: None,
                    method,
                    url,
                    headers: vec![
                        (
                            "accept".to_string(),
                            "application/vnd.github+json".to_string(),
                        ),
                        ("content-type".to_string(), "application/json".to_string()),
                        (
                            "user-agent".to_string(),
                            "AgentBounties-GitHub-App".to_string(),
                        ),
                        ("x-github-api-version".to_string(), "2022-11-28".to_string()),
                    ],
                    body: json!({ "body": markdown }),
                    credential_kind: ProviderCredentialKind::GitHubInstallationToken,
                    already_applied: *already_applied,
                    network_write_performed: false,
                });
            }
            OriginWriteOperation::CloseIssue {
                idempotency_key,
                depends_on_idempotency_key,
                completion_reason,
                already_applied,
            } => {
                validate_close_dependency(
                    &requests,
                    depends_on_idempotency_key,
                    completion_reason,
                )?;
                requests.push(ProviderHttpRequestPlan {
                    sequence: index as u32,
                    operation_idempotency_key: idempotency_key.clone(),
                    depends_on_idempotency_key: Some(depends_on_idempotency_key.clone()),
                    method: ProviderHttpMethod::Patch,
                    url: issue_url.clone(),
                    headers: vec![
                        (
                            "accept".to_string(),
                            "application/vnd.github+json".to_string(),
                        ),
                        ("content-type".to_string(), "application/json".to_string()),
                        (
                            "user-agent".to_string(),
                            "AgentBounties-GitHub-App".to_string(),
                        ),
                        ("x-github-api-version".to_string(), "2022-11-28".to_string()),
                    ],
                    body: json!({ "state": "closed", "state_reason": "completed" }),
                    credential_kind: ProviderCredentialKind::GitHubInstallationToken,
                    already_applied: *already_applied,
                    network_write_performed: false,
                });
            }
        }
    }
    Ok(requests)
}

fn plan_linear_requests(
    source: &OriginSourceReference,
    operations: &[OriginWriteOperation],
    binding: &ProviderRequestBinding,
) -> Result<Vec<ProviderHttpRequestPlan>, String> {
    let ProviderRequestBinding::Linear {
        existing_status_comment_id,
        completed_state_id,
    } = binding
    else {
        unreachable!();
    };
    validate_opaque_provider_id(&source.external_id, "Linear issue id")?;
    validate_opaque_provider_id(completed_state_id, "Linear completed state id")?;
    if let Some(comment_id) = existing_status_comment_id {
        validate_opaque_provider_id(comment_id, "Linear comment id")?;
    }
    let mut requests = Vec::new();
    for (index, operation) in operations.iter().enumerate() {
        match operation {
            OriginWriteOperation::UpsertStatusComment {
                idempotency_key,
                stable_marker,
                markdown,
                already_applied,
            } => {
                validate_status_comment(stable_marker, markdown)?;
                let body = match existing_status_comment_id {
                    Some(comment_id) => json!({
                        "query": "mutation AgentBountiesCommentUpdate($id: String!, $body: String!) { commentUpdate(id: $id, input: { body: $body }) { success comment { id } } }",
                        "variables": { "id": comment_id, "body": markdown }
                    }),
                    None => json!({
                        "query": "mutation AgentBountiesCommentCreate($issueId: String!, $body: String!) { commentCreate(input: { issueId: $issueId, body: $body }) { success comment { id } } }",
                        "variables": { "issueId": source.external_id, "body": markdown }
                    }),
                };
                requests.push(ProviderHttpRequestPlan {
                    sequence: index as u32,
                    operation_idempotency_key: idempotency_key.clone(),
                    depends_on_idempotency_key: None,
                    method: ProviderHttpMethod::Post,
                    url: LINEAR_GRAPHQL_URL.to_string(),
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body,
                    credential_kind: ProviderCredentialKind::LinearOAuthAccessToken,
                    already_applied: *already_applied,
                    network_write_performed: false,
                });
            }
            OriginWriteOperation::CloseIssue {
                idempotency_key,
                depends_on_idempotency_key,
                completion_reason,
                already_applied,
            } => {
                validate_close_dependency(
                    &requests,
                    depends_on_idempotency_key,
                    completion_reason,
                )?;
                requests.push(ProviderHttpRequestPlan {
                    sequence: index as u32,
                    operation_idempotency_key: idempotency_key.clone(),
                    depends_on_idempotency_key: Some(depends_on_idempotency_key.clone()),
                    method: ProviderHttpMethod::Post,
                    url: LINEAR_GRAPHQL_URL.to_string(),
                    headers: vec![("content-type".to_string(), "application/json".to_string())],
                    body: json!({
                        "query": "mutation AgentBountiesIssueComplete($id: String!, $stateId: String!) { issueUpdate(id: $id, input: { stateId: $stateId }) { success issue { id } } }",
                        "variables": { "id": source.external_id, "stateId": completed_state_id }
                    }),
                    credential_kind: ProviderCredentialKind::LinearOAuthAccessToken,
                    already_applied: *already_applied,
                    network_write_performed: false,
                });
            }
        }
    }
    Ok(requests)
}

fn github_repository_parts(repository: &str) -> Result<(&str, &str), String> {
    let (owner, repository) = repository
        .split_once('/')
        .ok_or_else(|| "GitHub workspace must be exactly owner/repository".to_string())?;
    if repository.contains('/') || !valid_github_slug(owner) || !valid_github_slug(repository) {
        return Err("GitHub workspace must be exactly owner/repository".to_string());
    }
    Ok((owner, repository))
}

fn valid_github_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn validate_opaque_provider_id(value: &str, name: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(format!("{name} contains unsupported characters"));
    }
    Ok(())
}

fn validate_status_comment(stable_marker: &str, markdown: &str) -> Result<(), String> {
    if !stable_marker.starts_with("<!-- agent-bounties-origin-status:")
        || !stable_marker.ends_with(" -->")
        || stable_marker.len() > 128
        || !markdown.ends_with(stable_marker)
    {
        return Err("status comment is missing its valid stable Agent Bounties marker".to_string());
    }
    if markdown.is_empty() || markdown.len() > MAX_PROVIDER_COMMENT_BYTES {
        return Err("status comment is empty or exceeds the provider limit".to_string());
    }
    Ok(())
}

fn validate_operation_shape(operations: &[OriginWriteOperation]) -> Result<(), String> {
    if operations.is_empty() || operations.len() > 2 {
        return Err(
            "provider batch requires one status upsert and at most one dependent close".to_string(),
        );
    }
    if !matches!(
        operations[0],
        OriginWriteOperation::UpsertStatusComment { .. }
    ) {
        return Err("provider batch must start with a status/proof upsert".to_string());
    }
    if operations
        .get(1)
        .is_some_and(|operation| !matches!(operation, OriginWriteOperation::CloseIssue { .. }))
    {
        return Err("only a dependent issue close may follow the status upsert".to_string());
    }
    Ok(())
}

fn validate_close_dependency(
    requests: &[ProviderHttpRequestPlan],
    dependency: &str,
    completion_reason: &str,
) -> Result<(), String> {
    if completion_reason != "verified artifact returned and canonical settlement confirmed" {
        return Err(
            "issue close intent does not carry the canonical completion reason".to_string(),
        );
    }
    if !requests
        .iter()
        .any(|request| request.operation_idempotency_key == dependency)
    {
        return Err(
            "issue close must follow its status/proof comment in the same batch".to_string(),
        );
    }
    Ok(())
}
