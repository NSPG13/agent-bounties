use super::{
    markdown_sections, parse_explicit_criteria, reward, stable_hash, valid_public_solver_reward,
    NormalizedOriginBountyDraft, OriginAuthorityBoundary, OriginDraftPlan, OriginProvider,
    OriginSourceReference, OriginTrigger, OriginVerifierRequirement,
    PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR,
};
use crate::{create_comment_plan, GitHubCreateCommentInput};
use domain::Money;
use serde::{Deserialize, Serialize};

const DEFAULT_VERIFIER_REWARD_USDC_MINOR: i64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubWebhookTrigger {
    Mention,
    Assignment,
}

impl From<GitHubWebhookTrigger> for OriginTrigger {
    fn from(value: GitHubWebhookTrigger) -> Self {
        match value {
            GitHubWebhookTrigger::Mention => Self::Mention,
            GitHubWebhookTrigger::Assignment => Self::Assignment,
        }
    }
}

/// Projection produced by a provider worker after it has authenticated the
/// GitHub webhook and authorized the actor/install context. This pure planner
/// intentionally does neither of those jobs itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubWebhookDraftInput {
    pub repository: String,
    pub issue_number: String,
    pub issue_url: String,
    pub title: String,
    pub body: String,
    pub solver_reward: Money,
    pub trigger: GitHubWebhookTrigger,
    pub actor_login: Option<String>,
    pub event_id: String,
    #[serde(default)]
    pub existing_idempotency_keys: Vec<String>,
}

/// Reuses the established GitHub create-comment planner, then translates its
/// review-only output into the provider-neutral origin contract.
pub fn plan_github_origin_draft(input: GitHubCreateCommentInput) -> OriginDraftPlan {
    let repository = input.repository.clone();
    let issue_url = input.issue_url.clone();
    let title = input.title.clone();
    let body = input.body.clone();
    let plan = create_comment_plan(input);
    let Some(signal) = plan.signal else {
        return OriginDraftPlan {
            ready_for_human_review: false,
            draft: None,
            idempotency_key: None,
            duplicate: plan
                .error
                .as_deref()
                .is_some_and(|error| error.contains("duplicate create signal")),
            error: plan.error,
            authority: OriginAuthorityBoundary::default(),
        };
    };

    if !valid_public_solver_reward(&signal.draft.solver_reward) {
        return OriginDraftPlan {
            ready_for_human_review: false,
            draft: None,
            idempotency_key: Some(signal.idempotency_key),
            duplicate: false,
            error: Some(format!(
                "public origin bounties require at least {} USDC",
                PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR / 1_000_000
            )),
            authority: OriginAuthorityBoundary::default(),
        };
    }

    let external_id = github_issue_number(&issue_url).unwrap_or_else(|| issue_url.clone());
    let source = OriginSourceReference {
        provider: OriginProvider::GitHub,
        workspace: repository,
        external_id: external_id.clone(),
        display_id: format!("#{external_id}"),
        url: issue_url,
    };
    let sections = markdown_sections(&body);
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
    let mut fields_requiring_review = signal.draft.fields_requiring_review.clone();
    if !acceptance_criteria.is_empty() {
        fields_requiring_review.retain(|field| field != "acceptance criteria");
    }
    if !verifier.requires_review {
        fields_requiring_review.retain(|field| field != "verification mode and verifier scope");
    }
    let draft = NormalizedOriginBountyDraft {
        source,
        trigger: OriginTrigger::Command,
        title,
        goal: signal.draft.draft_objective,
        acceptance_criteria,
        reward: reward(signal.draft.solver_reward, signal.draft.verifier_reward),
        verifier,
        ready_for_publish: false,
        fields_requiring_review,
    };

    OriginDraftPlan {
        ready_for_human_review: true,
        draft: Some(draft),
        idempotency_key: Some(signal.idempotency_key),
        duplicate: false,
        error: None,
        authority: OriginAuthorityBoundary::default(),
    }
}

/// Plans assignment- and mention-triggered GitHub drafts from an authenticated
/// webhook projection. It performs no provider or wallet writes.
pub fn plan_github_webhook_origin_draft(input: GitHubWebhookDraftInput) -> OriginDraftPlan {
    match parse_github_webhook_origin_draft(&input) {
        Ok((draft, idempotency_key)) => {
            let duplicate = input.existing_idempotency_keys.contains(&idempotency_key);
            OriginDraftPlan {
                ready_for_human_review: !duplicate,
                draft: (!duplicate).then_some(draft),
                idempotency_key: Some(idempotency_key),
                duplicate,
                error: duplicate.then(|| "duplicate GitHub origin event".to_string()),
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

fn parse_github_webhook_origin_draft(
    input: &GitHubWebhookDraftInput,
) -> Result<(NormalizedOriginBountyDraft, String), String> {
    if input.repository.trim().is_empty() {
        return Err("missing GitHub repository".to_string());
    }
    if input.issue_number.trim().is_empty()
        || !input.issue_number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("GitHub issue number must contain only digits".to_string());
    }
    if github_issue_number(&input.issue_url).as_deref() != Some(input.issue_number.trim()) {
        return Err(
            "GitHub issue URL must use HTTPS and match the projected issue number".to_string(),
        );
    }
    if input.title.trim().is_empty() || input.body.trim().is_empty() {
        return Err("GitHub issue needs a non-empty title and goal context".to_string());
    }
    if input.event_id.trim().is_empty() {
        return Err("GitHub webhook projection requires a stable provider event id".to_string());
    }
    if !valid_public_solver_reward(&input.solver_reward) {
        return Err(format!(
            "public origin bounties require at least {} USDC",
            PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR / 1_000_000
        ));
    }

    let sections = markdown_sections(&input.body);
    let goal = sections
        .iter()
        .find(|(heading, _)| heading == "goal")
        .map(|(_, body)| body.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| input.body.trim().to_string());
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

    let verifier_reward = Money::new(DEFAULT_VERIFIER_REWARD_USDC_MINOR, "usdc")
        .expect("static verifier reward is positive");
    let draft = NormalizedOriginBountyDraft {
        source: OriginSourceReference {
            provider: OriginProvider::GitHub,
            workspace: input.repository.trim().to_string(),
            external_id: input.issue_number.trim().to_string(),
            display_id: format!("#{}", input.issue_number.trim()),
            url: input.issue_url.trim().to_string(),
        },
        trigger: input.trigger.into(),
        title: input.title.trim().to_string(),
        goal,
        acceptance_criteria,
        reward: reward(input.solver_reward.clone(), verifier_reward),
        verifier,
        ready_for_publish: false,
        fields_requiring_review,
    };
    let identity = format!(
        "{}\n{}\n{}\n{}\n{:?}",
        input.repository.trim().to_ascii_lowercase(),
        input.issue_number.trim(),
        input.event_id.trim(),
        input.actor_login.as_deref().unwrap_or_default(),
        input.trigger
    );
    let idempotency_key = format!("github-origin-v1:{}", stable_hash(&identity));
    Ok((draft, idempotency_key))
}

fn github_issue_number(issue_url: &str) -> Option<String> {
    let path = issue_url.split(['?', '#']).next()?.trim_end_matches('/');
    let mut parts = path.rsplit('/');
    let number = parts.next()?;
    (parts.next()? == "issues" && number.chars().all(|character| character.is_ascii_digit()))
        .then(|| number.to_string())
}
