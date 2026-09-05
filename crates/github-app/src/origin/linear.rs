use super::{
    markdown_sections, parse_explicit_criteria, reward, stable_hash, NormalizedOriginBountyDraft,
    OriginAuthorityBoundary, OriginDraftPlan, OriginProvider, OriginSourceReference, OriginTrigger,
    OriginVerifierRequirement, PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR,
};
use domain::Money;
use serde::{Deserialize, Serialize};

const DEFAULT_VERIFIER_REWARD_USDC_MINOR: i64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinearWebhookTrigger {
    Command,
    Mention,
    Assignment,
    Delegation,
}

impl From<LinearWebhookTrigger> for OriginTrigger {
    fn from(value: LinearWebhookTrigger) -> Self {
        match value {
            LinearWebhookTrigger::Command => Self::Command,
            LinearWebhookTrigger::Mention => Self::Mention,
            LinearWebhookTrigger::Assignment => Self::Assignment,
            LinearWebhookTrigger::Delegation => Self::Delegation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearWebhookDraftInput {
    pub workspace_id: String,
    pub issue_id: String,
    pub identifier: String,
    pub issue_url: String,
    pub title: String,
    pub description: String,
    pub command_text: String,
    pub trigger: LinearWebhookTrigger,
    pub actor_id: Option<String>,
    pub event_id: Option<String>,
    #[serde(default)]
    pub existing_idempotency_keys: Vec<String>,
}

/// Plans a Linear issue-to-bounty draft from an already authenticated webhook
/// payload. The planner does not validate webhook signatures or call Linear.
pub fn plan_linear_origin_draft(input: LinearWebhookDraftInput) -> OriginDraftPlan {
    match parse_linear_origin_draft(&input) {
        Ok((draft, idempotency_key)) => {
            let duplicate = input.existing_idempotency_keys.contains(&idempotency_key);
            OriginDraftPlan {
                ready_for_human_review: !duplicate,
                draft: (!duplicate).then_some(draft),
                idempotency_key: Some(idempotency_key),
                duplicate,
                error: duplicate.then(|| "duplicate Linear origin event".to_string()),
                authority: OriginAuthorityBoundary::default(),
            }
        }
        Err(error) => OriginDraftPlan {
            ready_for_human_review: false,
            draft: None,
            idempotency_key: None,
            duplicate: false,
            error: Some(error),
            authority: OriginAuthorityBoundary::default(),
        },
    }
}

fn parse_linear_origin_draft(
    input: &LinearWebhookDraftInput,
) -> Result<(NormalizedOriginBountyDraft, String), String> {
    require_nonempty(&input.workspace_id, "Linear workspace id")?;
    require_nonempty(&input.issue_id, "Linear issue id")?;
    require_nonempty(&input.identifier, "Linear issue identifier")?;
    require_nonempty(&input.title, "Linear issue title")?;
    if !input.issue_url.starts_with("https://linear.app/")
        || !input
            .issue_url
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.contains("/issue/"))
    {
        return Err("Linear source must be an HTTPS linear.app issue URL".to_string());
    }

    let command = create_command_fragment(&input.command_text).ok_or_else(|| {
        "missing create command; use `/agent-bounty create <amount> USDC`".to_string()
    })?;
    let solver_reward = parse_create_reward(command)?;
    let sections = markdown_sections(&input.description);
    let goal = sections
        .iter()
        .find(|(heading, _)| heading == "goal")
        .map(|(_, body)| body.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| description_without_contract_sections(&input.description));
    if goal.trim().is_empty() {
        return Err("Linear issue needs an explicit goal or non-empty description".to_string());
    }

    let acceptance_criteria = sections
        .iter()
        .find(|(heading, _)| heading == "acceptance criteria")
        .map(|(_, body)| parse_explicit_criteria(body))
        .unwrap_or_default();
    let verifier_instructions = sections
        .iter()
        .find(|(heading, _)| heading == "verifier" || heading == "verification")
        .map(|(_, body)| body.trim().to_string())
        .filter(|value| !value.is_empty());
    let verifier = match verifier_instructions {
        Some(instructions) => OriginVerifierRequirement {
            kind: "explicit_origin_instructions".to_string(),
            instructions,
            requires_review: false,
        },
        None => OriginVerifierRequirement {
            kind: "review_required".to_string(),
            instructions: "Choose and precommit a verifier whose actual payout condition matches the acceptance criteria."
                .to_string(),
            requires_review: true,
        },
    };
    let mut fields_requiring_review = Vec::new();
    if acceptance_criteria.is_empty() {
        fields_requiring_review.push("acceptance criteria".to_string());
    }
    if verifier.requires_review {
        fields_requiring_review.push("verification mode and verifier scope".to_string());
    }
    fields_requiring_review
        .push("wallet, network, token, deadlines, and exact transaction".to_string());
    let ready_for_publish = fields_requiring_review.is_empty();

    let verifier_reward = Money::new(DEFAULT_VERIFIER_REWARD_USDC_MINOR, "usdc")
        .expect("static verifier reward is positive");
    let source = OriginSourceReference {
        provider: OriginProvider::Linear,
        workspace: input.workspace_id.clone(),
        external_id: input.issue_id.clone(),
        display_id: input.identifier.clone(),
        url: input.issue_url.clone(),
    };
    let idempotency_key = linear_idempotency_key(input, command);
    Ok((
        NormalizedOriginBountyDraft {
            source,
            trigger: input.trigger.into(),
            title: input.title.trim().to_string(),
            goal: goal.trim().to_string(),
            acceptance_criteria,
            reward: reward(solver_reward, verifier_reward),
            verifier,
            ready_for_publish,
            fields_requiring_review,
        },
        idempotency_key,
    ))
}

fn parse_create_reward(command: &str) -> Result<Money, String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != "/agent-bounty" || parts[1] != "create" {
        return Err("invalid create command; use `/agent-bounty create <amount> USDC`".to_string());
    }
    if !parts[3].eq_ignore_ascii_case("USDC") {
        return Err("only USDC is supported by `/agent-bounty create`".to_string());
    }
    let amount = decimal_usdc_minor(parts[2])?;
    if amount < PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR {
        return Err(format!(
            "public origin bounties require at least {} USDC",
            PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR / 1_000_000
        ));
    }
    Money::new(amount, "usdc").map_err(|_| "create amount must be positive".to_string())
}

fn decimal_usdc_minor(value: &str) -> Result<i64, String> {
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fractional.len() > 6
        || !fractional.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("invalid USDC amount; use at most six decimal places".to_string());
    }
    let whole = whole
        .parse::<i64>()
        .map_err(|_| "USDC amount is too large".to_string())?;
    let fractional = format!("{fractional:0<6}")
        .parse::<i64>()
        .map_err(|_| "invalid USDC amount".to_string())?;
    whole
        .checked_mul(1_000_000)
        .and_then(|amount| amount.checked_add(fractional))
        .ok_or_else(|| "USDC amount is too large".to_string())
}

fn create_command_fragment(value: &str) -> Option<&str> {
    value.lines().find_map(|line| {
        line.find("/agent-bounty create")
            .map(|start| line[start..].trim())
    })
}

fn description_without_contract_sections(value: &str) -> String {
    let mut lines = Vec::new();
    for line in value.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            break;
        }
        if !line.contains("/agent-bounty create") {
            lines.push(line);
        }
    }
    lines.join("\n").trim().to_string()
}

fn linear_idempotency_key(input: &LinearWebhookDraftInput, command: &str) -> String {
    if let Some(event_id) = input
        .event_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("linear-origin:{}:event:{event_id}", input.workspace_id);
    }
    let identity = format!(
        "{}\n{}\n{}\n{}\n{}",
        input.workspace_id,
        input.issue_id,
        input.actor_id.as_deref().unwrap_or_default(),
        command,
        input.title
    );
    format!("linear-origin:{}", stable_hash(&identity))
}

fn require_nonempty(value: &str, name: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("missing {name}"))
    } else {
        Ok(())
    }
}
