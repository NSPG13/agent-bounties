use domain::{FundingMode, Id, Money, PrivacyLevel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubBountySource {
    Issue,
    PullRequest,
    CheckRun,
    Comment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubBountyRequest {
    pub id: Id,
    pub repository: String,
    pub source: GitHubBountySource,
    pub source_url: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GitHubBountyError {
    #[error("missing required GitHub issue form field: {0}")]
    MissingField(&'static str),
    #[error("missing GitHub funding comment issue context: {0}")]
    MissingIssueContext(&'static str),
    #[error("GitHub funding comments require a bounty issue: {0}")]
    NonBountyIssue(String),
    #[error("unknown bounty template: {0}")]
    UnknownTemplate(String),
    #[error("unknown funding mode: {0}")]
    UnknownFundingMode(String),
    #[error("unknown privacy level: {0}")]
    UnknownPrivacy(String),
    #[error("invalid suggested amount: {0}")]
    InvalidAmount(String),
    #[error("invalid GitHub funding comment: {0}")]
    InvalidFundingComment(String),
    #[error("duplicate GitHub funding comment signal: {0}")]
    DuplicateFundingSignal(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIssueFormBounty {
    pub request: GitHubBountyRequest,
    pub goal: String,
    pub acceptance_criteria: String,
    pub template_slug: String,
    pub amount: Money,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubCheckConclusion {
    Success,
    Neutral,
    Failure,
    ActionRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCheckRunOutput {
    pub title: String,
    pub summary: String,
    pub text: String,
    pub conclusion: GitHubCheckConclusion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubProofComment {
    pub bounty_id: Id,
    pub proof_url: String,
    pub verifier_summary: String,
    pub settlement_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubProofCommentPlan {
    pub comment: GitHubProofComment,
    pub markdown: String,
    pub fingerprint: String,
    pub check: GitHubCheckRunOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFundingCommentContext {
    pub repository: String,
    pub issue_url: String,
    pub issue_number: u64,
    pub issue_title: String,
    pub issue_labels: Vec<String>,
    pub existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFundingCommentSignal {
    pub issue_url: String,
    pub contributor_login: Option<String>,
    pub amount: i64,
    pub currency: String,
    pub rail: FundingMode,
    pub idempotency_key: String,
    pub requires_operator_reconciliation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFundingCommentPlan {
    pub signal: Option<GitHubFundingCommentSignal>,
    pub check: GitHubCheckRunOutput,
}

impl GitHubProofComment {
    pub fn markdown(&self) -> String {
        format!(
            "Agent bounty completed.\n\nProof: {}\n\nVerifier: {}\n\nBounty: `{}`{}",
            self.proof_url,
            self.verifier_summary,
            self.bounty_id,
            self.settlement_url
                .as_ref()
                .map(|url| format!("\n\nSettlement: {url}"))
                .unwrap_or_default()
        )
    }
}

pub fn proof_comment_fingerprint(comment: &GitHubProofComment) -> String {
    let mut hasher = Sha256::new();
    hasher.update(comment.markdown());
    hex::encode(hasher.finalize())
}

pub fn proof_comment_plan(comment: GitHubProofComment) -> GitHubProofCommentPlan {
    let markdown = comment.markdown();
    let fingerprint = proof_comment_fingerprint(&comment);
    let check = proof_check_output(&comment);
    GitHubProofCommentPlan {
        comment,
        markdown,
        fingerprint,
        check,
    }
}

pub fn funding_comment_idempotency_key(
    context: &GitHubFundingCommentContext,
    contributor_login: Option<&str>,
    amount: &Money,
    rail: &FundingMode,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(context.repository.trim().to_ascii_lowercase());
    hasher.update(context.issue_url.trim());
    hasher.update(context.issue_number.to_string());
    hasher.update(
        contributor_login
            .unwrap_or("anonymous")
            .trim()
            .to_ascii_lowercase(),
    );
    hasher.update(amount.amount.to_string());
    hasher.update(amount.currency.trim().to_ascii_lowercase());
    hasher.update(format!("{rail:?}"));
    hex::encode(hasher.finalize())
}

pub fn funding_comment_plan(
    context: GitHubFundingCommentContext,
    contributor_login: Option<String>,
    comment_body: &str,
) -> GitHubFundingCommentPlan {
    let parsed = parse_funding_comment(&context, contributor_login, comment_body);
    match parsed {
        Ok(signal) => GitHubFundingCommentPlan {
            check: funding_comment_check_output(Ok(&signal)),
            signal: Some(signal),
        },
        Err(error) => GitHubFundingCommentPlan {
            check: funding_comment_check_output(Err(&error)),
            signal: None,
        },
    }
}

pub fn issue_to_bounty_request(
    repository: &str,
    issue_url: &str,
    title: &str,
    body: &str,
) -> GitHubBountyRequest {
    GitHubBountyRequest {
        id: Uuid::new_v4(),
        repository: repository.to_string(),
        source: GitHubBountySource::Issue,
        source_url: issue_url.to_string(),
        title: title.to_string(),
        body: body.to_string(),
    }
}

pub fn parse_issue_form_bounty(
    repository: &str,
    issue_url: &str,
    title: &str,
    body: &str,
) -> Result<GitHubIssueFormBounty, GitHubBountyError> {
    let sections = parse_issue_form_sections(body);
    let goal = required_section(&sections, "goal")?;
    let acceptance_criteria = required_section(&sections, "acceptance criteria")?;
    let template_slug = required_section(&sections, "template")?;
    validate_template(&template_slug)?;
    let amount = parse_amount(&required_section(&sections, "suggested amount")?)?;
    let funding_mode = optional_section(&sections, "funding mode")
        .as_deref()
        .map(parse_funding_mode)
        .transpose()?
        .unwrap_or(FundingMode::BaseUsdcEscrow);
    let privacy = optional_section(&sections, "privacy")
        .as_deref()
        .map(parse_privacy)
        .transpose()?
        .unwrap_or(PrivacyLevel::Public);

    Ok(GitHubIssueFormBounty {
        request: GitHubBountyRequest {
            id: stable_bounty_id(repository, issue_url, title),
            repository: repository.to_string(),
            source: GitHubBountySource::Issue,
            source_url: issue_url.to_string(),
            title: title.to_string(),
            body: body.to_string(),
        },
        goal,
        acceptance_criteria,
        template_slug,
        amount,
        funding_mode,
        privacy,
    })
}

pub fn bounty_check_output(
    parsed: Result<&GitHubIssueFormBounty, &GitHubBountyError>,
) -> GitHubCheckRunOutput {
    match parsed {
        Ok(bounty) => GitHubCheckRunOutput {
            title: "Agent bounty ready".to_string(),
            summary: format!(
                "{} is ready for funding with template `{}`.",
                bounty.request.title, bounty.template_slug
            ),
            text: format!(
                "Goal:\n{}\n\nAcceptance criteria:\n{}\n\nAmount: {} {}\n\nFunding: {:?}\n\nPrivacy: {:?}",
                bounty.goal,
                bounty.acceptance_criteria,
                bounty.amount.amount,
                bounty.amount.currency,
                bounty.funding_mode,
                bounty.privacy
            ),
            conclusion: GitHubCheckConclusion::Success,
        },
        Err(error) => GitHubCheckRunOutput {
            title: "Agent bounty needs changes".to_string(),
            summary: error.to_string(),
            text: "Edit the paid bounty issue form so the bounty can be routed, funded, verified, and paid.".to_string(),
            conclusion: GitHubCheckConclusion::ActionRequired,
        },
    }
}

pub fn proof_check_output(comment: &GitHubProofComment) -> GitHubCheckRunOutput {
    GitHubCheckRunOutput {
        title: "Agent bounty proof accepted".to_string(),
        summary: format!("Proof recorded for bounty `{}`.", comment.bounty_id),
        text: comment.markdown(),
        conclusion: GitHubCheckConclusion::Success,
    }
}

pub fn funding_comment_check_output(
    parsed: Result<&GitHubFundingCommentSignal, &GitHubBountyError>,
) -> GitHubCheckRunOutput {
    match parsed {
        Ok(signal) => GitHubCheckRunOutput {
            title: "GitHub co-funding signal ready".to_string(),
            summary: format!(
                "{} {} via {:?} requires operator reconciliation.",
                signal.amount, signal.currency, signal.rail
            ),
            text: format!(
                "Issue: {}\nContributor: {}\nAmount: {} {}\nRail: {:?}\nIdempotency key: {}\nRequires operator reconciliation: {}",
                signal.issue_url,
                signal.contributor_login.as_deref().unwrap_or("unknown"),
                signal.amount,
                signal.currency,
                signal.rail,
                signal.idempotency_key,
                signal.requires_operator_reconciliation
            ),
            conclusion: GitHubCheckConclusion::Success,
        },
        Err(error) => GitHubCheckRunOutput {
            title: "GitHub co-funding signal needs action".to_string(),
            summary: error.to_string(),
            text: "A co-funding comment is only a public signal. Fix the comment or issue context before an operator reconciles it into platform funding.".to_string(),
            conclusion: GitHubCheckConclusion::ActionRequired,
        },
    }
}

fn parse_funding_comment(
    context: &GitHubFundingCommentContext,
    contributor_login: Option<String>,
    comment_body: &str,
) -> Result<GitHubFundingCommentSignal, GitHubBountyError> {
    validate_funding_comment_context(context)?;
    let parts = comment_body.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 6
        || parts[0] != "/agent-bounty"
        || parts[1] != "fund"
        || !parts[4].eq_ignore_ascii_case("via")
    {
        return Err(GitHubBountyError::InvalidFundingComment(
            "expected `/agent-bounty fund <amount> <currency> via <rail>`".to_string(),
        ));
    }

    let amount = parse_amount(&format!("{} {}", parts[2], parts[3]))?;
    let rail = parse_funding_mode(parts[5])?;
    let idempotency_key =
        funding_comment_idempotency_key(context, contributor_login.as_deref(), &amount, &rail);
    if context
        .existing_idempotency_keys
        .iter()
        .any(|key| key == &idempotency_key)
    {
        return Err(GitHubBountyError::DuplicateFundingSignal(idempotency_key));
    }

    Ok(GitHubFundingCommentSignal {
        issue_url: context.issue_url.clone(),
        contributor_login: contributor_login.and_then(|login| {
            let trimmed = login.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }),
        amount: amount.amount,
        currency: amount.currency,
        rail,
        idempotency_key,
        requires_operator_reconciliation: true,
    })
}

fn validate_funding_comment_context(
    context: &GitHubFundingCommentContext,
) -> Result<(), GitHubBountyError> {
    if context.repository.trim().is_empty() {
        return Err(GitHubBountyError::MissingIssueContext("repository"));
    }
    if context.issue_url.trim().is_empty() {
        return Err(GitHubBountyError::MissingIssueContext("issue_url"));
    }
    if context.issue_number == 0 {
        return Err(GitHubBountyError::MissingIssueContext("issue_number"));
    }
    let has_bounty_label = context
        .issue_labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("bounty"));
    let bounty_title = context
        .issue_title
        .to_ascii_lowercase()
        .contains("[bounty]");
    if !has_bounty_label && !bounty_title {
        return Err(GitHubBountyError::NonBountyIssue(context.issue_url.clone()));
    }
    Ok(())
}

fn parse_issue_form_sections(body: &str) -> HashMap<String, String> {
    let mut sections = HashMap::new();
    let mut current: Option<String> = None;
    let mut buffer = Vec::new();

    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            if let Some(key) = current.take() {
                sections.insert(key, clean_section(&buffer.join("\n")));
                buffer.clear();
            }
            current = Some(heading.trim().to_ascii_lowercase());
        } else {
            buffer.push(line);
        }
    }
    if let Some(key) = current {
        sections.insert(key, clean_section(&buffer.join("\n")));
    }

    sections
}

fn clean_section(value: &str) -> String {
    value
        .trim()
        .trim_matches('\r')
        .replace("_No response_", "")
        .trim()
        .to_string()
}

fn required_section(
    sections: &HashMap<String, String>,
    key: &'static str,
) -> Result<String, GitHubBountyError> {
    sections
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or(GitHubBountyError::MissingField(key))
}

fn optional_section(sections: &HashMap<String, String>, key: &'static str) -> Option<String> {
    sections
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
}

fn validate_template(template_slug: &str) -> Result<(), GitHubBountyError> {
    match template_slug.trim() {
        "fix-ci-failure"
        | "small-code-change"
        | "extract-data-to-schema"
        | "independent-claim-verification"
        | "write-docs-for-area"
        | "run-browser-workflow" => Ok(()),
        other => Err(GitHubBountyError::UnknownTemplate(other.to_string())),
    }
}

fn parse_amount(value: &str) -> Result<Money, GitHubBountyError> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(GitHubBountyError::InvalidAmount(value.to_string()));
    }
    let units = parts[0]
        .parse::<f64>()
        .map_err(|_| GitHubBountyError::InvalidAmount(value.to_string()))?;
    let currency = parts[1].to_ascii_lowercase();
    let scale = match currency.as_str() {
        "usdc" => 1_000_000_f64,
        "usd" => 100_f64,
        _ => return Err(GitHubBountyError::InvalidAmount(value.to_string())),
    };
    let minor = (units * scale).round();
    if minor <= 0.0 || !minor.is_finite() {
        return Err(GitHubBountyError::InvalidAmount(value.to_string()));
    }
    Money::new(minor as i64, currency).map_err(|_| GitHubBountyError::InvalidAmount(value.into()))
}

fn parse_funding_mode(value: &str) -> Result<FundingMode, GitHubBountyError> {
    let normalized = normalized_choice(value);
    match normalized.as_str() {
        "baseusdcescrow" | "baseusdc" | "base" => Ok(FundingMode::BaseUsdcEscrow),
        "stripefiatledger" | "stripefiat" | "stripe" => Ok(FundingMode::StripeFiatLedger),
        "simulated" | "localdemo" => Ok(FundingMode::Simulated),
        _ => Err(GitHubBountyError::UnknownFundingMode(value.to_string())),
    }
}

fn parse_privacy(value: &str) -> Result<PrivacyLevel, GitHubBountyError> {
    let normalized = normalized_choice(value);
    match normalized.as_str() {
        "public" => Ok(PrivacyLevel::Public),
        "redactedpublicproof" | "redacted" => Ok(PrivacyLevel::RedactedPublicProof),
        "private" => Ok(PrivacyLevel::Private),
        _ => Err(GitHubBountyError::UnknownPrivacy(value.to_string())),
    }
}

fn normalized_choice(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn stable_bounty_id(repository: &str, issue_url: &str, title: &str) -> Id {
    let mut hasher = Sha256::new();
    hasher.update(repository);
    hasher.update(issue_url);
    hasher.update(title);
    let hash = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_comment_contains_required_links() {
        let comment = GitHubProofComment {
            bounty_id: Uuid::new_v4(),
            proof_url: "https://agentbounties.dev/proofs/1".to_string(),
            verifier_summary: "GitHub CI passed".to_string(),
            settlement_url: Some("https://agentbounties.dev/settlements/1".to_string()),
        };

        let markdown = comment.markdown();
        assert!(markdown.contains("Proof:"));
        assert!(markdown.contains("GitHub CI passed"));
        assert!(markdown.contains("Settlement:"));
    }

    #[test]
    fn proof_comment_plan_builds_fingerprint_and_check() {
        let bounty_id = Uuid::new_v4();
        let plan = proof_comment_plan(GitHubProofComment {
            bounty_id,
            proof_url: "https://agentbounties.dev/proofs/1".to_string(),
            verifier_summary: "JsonSchema: artifact accepted".to_string(),
            settlement_url: None,
        });

        assert_eq!(plan.comment.bounty_id, bounty_id);
        assert_eq!(plan.fingerprint.len(), 64);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
        assert_eq!(plan.check.text, plan.markdown);
    }

    #[test]
    fn parses_paid_bounty_issue_form() {
        let body = r#"### Goal
Fix the failing CI check.

### Acceptance criteria
The test job is green and the patch explains the failure.

### Template
fix-ci-failure

### Suggested amount
10 USDC
"#;

        let bounty = parse_issue_form_bounty(
            "agent-bounties/agent-bounties",
            "https://github.com/agent-bounties/agent-bounties/issues/1",
            "[bounty]: Fix CI",
            body,
        )
        .unwrap();

        assert_eq!(bounty.template_slug, "fix-ci-failure");
        assert_eq!(bounty.amount, Money::new(10_000_000, "usdc").unwrap());
        assert_eq!(bounty.funding_mode, FundingMode::BaseUsdcEscrow);
        assert_eq!(bounty.privacy, PrivacyLevel::Public);
        assert_eq!(
            bounty.request.id,
            parse_issue_form_bounty(
                "agent-bounties/agent-bounties",
                "https://github.com/agent-bounties/agent-bounties/issues/1",
                "[bounty]: Fix CI",
                body,
            )
            .unwrap()
            .request
            .id
        );
    }

    #[test]
    fn parses_optional_funding_and_privacy_terms() {
        let body = r#"### Goal
Extract customer data into a redacted JSON proof.

### Acceptance criteria
JSON schema verifier accepts the artifact and the public proof excludes private fields.

### Template
extract-data-to-schema

### Suggested amount
2 USDC

### Funding mode
StripeFiatLedger

### Privacy
RedactedPublicProof
"#;

        let bounty = parse_issue_form_bounty(
            "agent-bounties/agent-bounties",
            "https://github.com/agent-bounties/agent-bounties/issues/3",
            "[bounty]: Redacted extraction",
            body,
        )
        .unwrap();

        assert_eq!(bounty.funding_mode, FundingMode::StripeFiatLedger);
        assert_eq!(bounty.privacy, PrivacyLevel::RedactedPublicProof);
    }

    #[test]
    fn malformed_issue_form_gets_action_required_check() {
        let error = parse_issue_form_bounty(
            "agent-bounties/agent-bounties",
            "https://github.com/agent-bounties/agent-bounties/issues/1",
            "[bounty]: Missing fields",
            "### Goal\nFix CI",
        )
        .unwrap_err();
        let output = bounty_check_output(Err(&error));

        assert_eq!(output.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(output.summary.contains("missing required"));
    }

    #[test]
    fn valid_issue_form_gets_success_check() {
        let body = r#"### Goal
Extract data into JSON.

### Acceptance criteria
Digest verifier accepts the JSON artifact.

### Template
extract-data-to-schema

### Suggested amount
1.5 USDC
"#;
        let bounty = parse_issue_form_bounty(
            "agent-bounties/agent-bounties",
            "https://github.com/agent-bounties/agent-bounties/issues/2",
            "[bounty]: Extract data",
            body,
        )
        .unwrap();
        let output = bounty_check_output(Ok(&bounty));

        assert_eq!(output.conclusion, GitHubCheckConclusion::Success);
        assert!(output.summary.contains("ready for funding"));
    }

    #[test]
    fn valid_funding_comment_gets_operator_reconciliation_plan() {
        let context = funding_context(Vec::new());
        let plan = funding_comment_plan(
            context,
            Some("octo-agent".to_string()),
            "/agent-bounty fund 5 USDC via BaseUsdcEscrow",
        );

        let signal = plan.signal.expect("funding signal");
        assert_eq!(
            signal.issue_url,
            "https://github.com/agent-bounties/agent-bounties/issues/20"
        );
        assert_eq!(signal.contributor_login.as_deref(), Some("octo-agent"));
        assert_eq!(signal.amount, 5_000_000);
        assert_eq!(signal.currency, "usdc");
        assert_eq!(signal.rail, FundingMode::BaseUsdcEscrow);
        assert_eq!(signal.idempotency_key.len(), 64);
        assert!(signal.requires_operator_reconciliation);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
    }

    #[test]
    fn invalid_funding_amount_requires_action() {
        let plan = funding_comment_plan(
            funding_context(Vec::new()),
            Some("octo-agent".to_string()),
            "/agent-bounty fund -5 USDC via BaseUsdcEscrow",
        );

        assert!(plan.signal.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(plan.check.summary.contains("invalid suggested amount"));
    }

    #[test]
    fn duplicate_funding_signal_requires_action() {
        let context = funding_context(Vec::new());
        let amount = Money::new(5_000_000, "usdc").unwrap();
        let key = funding_comment_idempotency_key(
            &context,
            Some("octo-agent"),
            &amount,
            &FundingMode::BaseUsdcEscrow,
        );
        let plan = funding_comment_plan(
            funding_context(vec![key.clone()]),
            Some("octo-agent".to_string()),
            "/agent-bounty fund 5 USDC via BaseUsdcEscrow",
        );

        assert!(plan.signal.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(plan.check.summary.contains(&key));
    }

    #[test]
    fn unsupported_funding_rail_requires_action() {
        let plan = funding_comment_plan(
            funding_context(Vec::new()),
            Some("octo-agent".to_string()),
            "/agent-bounty fund 5 USDC via UnknownRail",
        );

        assert!(plan.signal.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(plan.check.summary.contains("unknown funding mode"));
    }

    #[test]
    fn non_bounty_issue_requires_action() {
        let mut context = funding_context(Vec::new());
        context.issue_title = "Add funding comments".to_string();
        context.issue_labels.clear();
        let plan = funding_comment_plan(
            context,
            Some("octo-agent".to_string()),
            "/agent-bounty fund 5 USDC via BaseUsdcEscrow",
        );

        assert!(plan.signal.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(plan.check.summary.contains("require a bounty issue"));
    }

    fn funding_context(existing_idempotency_keys: Vec<String>) -> GitHubFundingCommentContext {
        GitHubFundingCommentContext {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            issue_number: 20,
            issue_title: "[bounty]: Add GitHub co-funding comment planner".to_string(),
            issue_labels: vec!["bounty".to_string(), "good-first-agent-bounty".to_string()],
            existing_idempotency_keys,
        }
    }
}
