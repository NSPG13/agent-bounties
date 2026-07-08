use domain::{FundingMode, Id, Money, PrivacyLevel};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

const STATIC_FUNDING_PAGE_URL: &str = "https://nspg13.github.io/agent-bounties/funding.html";
const DISTRIBUTION_FEEDBACK_REQUEST: &str = "Distribution feedback requested, separate from review or payout decisions:\n\n- How did you find Agent Bounties?\n- What made this bounty or project worth participating in?\n- If an AI agent helped you find or complete this work, what tool, prompt, link, label, scanner, or workflow led it here?\n- What would make the project easier or more trustworthy before you participate again?\n\nIf Agent Bounties is useful, please star the repository, react/upvote useful issues or bounties, share it with other AI-agent builders or bounty solvers, and invite collaborators who can improve task liquidity, verifier quality, payment trust, or agent distribution.\n\nThese answers help improve agent discovery, bounty templates, proof pages, and payment-trust messaging.";

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
    #[error("unknown bounty template: {0}")]
    UnknownTemplate(String),
    #[error("unknown funding mode: {0}")]
    UnknownFundingMode(String),
    #[error("unknown privacy level: {0}")]
    UnknownPrivacy(String),
    #[error("invalid suggested amount: {0}")]
    InvalidAmount(String),
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
    pub discovery_feedback: Option<String>,
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
pub struct GitHubFundingCommentInput {
    pub repository: String,
    pub issue_url: String,
    pub title: String,
    pub body: String,
    pub comment_body: String,
    pub contributor_login: Option<String>,
    pub comment_id: Option<String>,
    pub funding_api_base_url: Option<String>,
    #[serde(default)]
    pub existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubClaimCommentInput {
    pub repository: String,
    pub issue_url: String,
    pub title: String,
    pub body: String,
    pub comment_body: String,
    pub contributor_login: Option<String>,
    pub comment_id: Option<String>,
    pub claim_age_minutes: Option<u64>,
    pub progress_signal_count: u32,
    pub active_claim_login: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFundingSignal {
    pub issue_url: String,
    pub contributor_login: Option<String>,
    pub amount: Money,
    pub rail: FundingMode,
    pub idempotency_key: String,
    pub requires_operator_reconciliation: bool,
    pub operator_note: String,
    pub funding_handoff_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFundingCommentPlan {
    pub ready: bool,
    pub signal: Option<GitHubFundingSignal>,
    pub error: Option<String>,
    pub check: GitHubCheckRunOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitHubClaimDecision {
    Reserved,
    NeedsProgress,
    StaleReleaseRecommended,
    BlockedByActiveClaim,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubClaimSignal {
    pub issue_url: String,
    pub contributor_login: Option<String>,
    pub command: String,
    pub decision: GitHubClaimDecision,
    pub reservation_id: String,
    pub reservation_window_minutes: u64,
    pub progress_required_within_minutes: u64,
    pub progress_signal_count: u32,
    pub has_progress_signal: bool,
    pub settlement_authority: bool,
    pub operator_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubClaimCommentPlan {
    pub ready: bool,
    pub signal: Option<GitHubClaimSignal>,
    pub error: Option<String>,
    pub check: GitHubCheckRunOutput,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GitHubFundingCommentError {
    #[error("missing GitHub issue context")]
    MissingIssueContext,
    #[error("issue is not a valid paid bounty: {0}")]
    NonBountyIssue(String),
    #[error("missing funding command; use `/agent-bounty fund <amount> <currency> via <rail>`")]
    MissingCommand,
    #[error("invalid funding command; use `/agent-bounty fund <amount> <currency> via <rail>`")]
    InvalidCommand,
    #[error("invalid funding amount: {0}")]
    InvalidAmount(String),
    #[error("unsupported funding rail for public comments: {0}")]
    UnsupportedRail(String),
    #[error("currency {currency} does not match funding rail {rail}")]
    CurrencyRailMismatch { currency: String, rail: String },
    #[error("duplicate funding signal idempotency key: {0}")]
    DuplicateSignal(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GitHubClaimCommentError {
    #[error("missing GitHub issue context")]
    MissingIssueContext,
    #[error("issue is not a valid paid bounty: {0}")]
    NonBountyIssue(String),
    #[error("missing claim command; use `/agent-bounty claim` or `/agent-bounty attempt`")]
    MissingCommand,
    #[error("active claim is held by {0}; wait for progress, stale release, or maintainer review")]
    ActiveClaimHeld(String),
    #[error("claim comment needs a concrete progress signal such as `plan:`, `branch:`, `draft pr:`, `tests:`, or a pull request URL")]
    MissingProgressSignal,
}

impl GitHubProofComment {
    pub fn markdown(&self) -> String {
        format!(
            "Agent bounty completed.\n\nProof: {}\n\nVerifier: {}\n\nBounty: `{}`{}\n\n{}",
            self.proof_url,
            self.verifier_summary,
            self.bounty_id,
            self.settlement_url
                .as_ref()
                .map(|url| format!("\n\nSettlement: {url}"))
                .unwrap_or_default(),
            DISTRIBUTION_FEEDBACK_REQUEST
        )
    }
}

pub const CLAIM_RESERVATION_WINDOW_MINUTES: u64 = 120;

pub fn funding_comment_plan(input: GitHubFundingCommentInput) -> GitHubFundingCommentPlan {
    match parse_funding_comment_signal(&input) {
        Ok(signal) => {
            let check = funding_comment_check_output(Ok(&signal));
            GitHubFundingCommentPlan {
                ready: true,
                signal: Some(signal),
                error: None,
                check,
            }
        }
        Err(error) => {
            let message = error.to_string();
            let check = funding_comment_check_output(Err(&error));
            GitHubFundingCommentPlan {
                ready: false,
                signal: None,
                error: Some(message),
                check,
            }
        }
    }
}

pub fn claim_comment_plan(input: GitHubClaimCommentInput) -> GitHubClaimCommentPlan {
    match parse_claim_comment_signal(&input) {
        Ok(signal) => {
            let check = claim_comment_check_output(Ok(&signal));
            GitHubClaimCommentPlan {
                ready: matches!(
                    signal.decision,
                    GitHubClaimDecision::Reserved | GitHubClaimDecision::StaleReleaseRecommended
                ),
                signal: Some(signal),
                error: None,
                check,
            }
        }
        Err(error) => {
            let message = error.to_string();
            let check = claim_comment_check_output(Err(&error));
            GitHubClaimCommentPlan {
                ready: false,
                signal: None,
                error: Some(message),
                check,
            }
        }
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
    let discovery_feedback = optional_section(&sections, "discovery feedback");

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
        discovery_feedback,
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
                "Goal:\n{}\n\nAcceptance criteria:\n{}\n\nAmount: {} {}\n\nFunding: {:?}\n\nPrivacy: {:?}\n\nDistribution feedback:\n{}",
                bounty.goal,
                bounty.acceptance_criteria,
                bounty.amount.amount,
                bounty.amount.currency,
                bounty.funding_mode,
                bounty.privacy,
                bounty
                    .discovery_feedback
                    .as_deref()
                    .unwrap_or(DISTRIBUTION_FEEDBACK_REQUEST)
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

pub fn claim_comment_check_output(
    signal: Result<&GitHubClaimSignal, &GitHubClaimCommentError>,
) -> GitHubCheckRunOutput {
    match signal {
        Ok(signal) => {
            let (title, summary) = match signal.decision {
                GitHubClaimDecision::Reserved => (
                    "Agent bounty claim reserved",
                    format!(
                        "Reserved for {} minutes; progress is required before settlement review.",
                        signal.reservation_window_minutes
                    ),
                ),
                GitHubClaimDecision::NeedsProgress => (
                    "Agent bounty claim needs progress",
                    "Claim comment did not include enough concrete progress evidence.".to_string(),
                ),
                GitHubClaimDecision::StaleReleaseRecommended => (
                    "Agent bounty stale claim release recommended",
                    "Reservation window expired without progress; maintainers can release the claim."
                        .to_string(),
                ),
                GitHubClaimDecision::BlockedByActiveClaim => (
                    "Agent bounty claim blocked",
                    "Another active claim is still inside the reservation window.".to_string(),
                ),
            };
            GitHubCheckRunOutput {
                title: title.to_string(),
                summary,
                text: format!(
                    "Issue: {}\nContributor: {}\nCommand: {}\nDecision: {:?}\nReservation id: {}\nReservation window minutes: {}\nProgress required within minutes: {}\nProgress signal count: {}\nHas progress signal: {}\nSettlement authority: false\n\nThis GitHub claim signal is coordination evidence only. It does not claim platform funds, approve work, accept a bounty, release escrow, or authorize payment.\n\nOperator note: {}\n\n{}",
                    signal.issue_url,
                    signal
                        .contributor_login
                        .as_deref()
                        .unwrap_or("unknown"),
                    signal.command,
                    signal.decision,
                    signal.reservation_id,
                    signal.reservation_window_minutes,
                    signal.progress_required_within_minutes,
                    signal.progress_signal_count,
                    signal.has_progress_signal,
                    signal.operator_note,
                    DISTRIBUTION_FEEDBACK_REQUEST
                ),
                conclusion: match signal.decision {
                    GitHubClaimDecision::Reserved
                    | GitHubClaimDecision::StaleReleaseRecommended => {
                        GitHubCheckConclusion::Success
                    }
                    GitHubClaimDecision::NeedsProgress
                    | GitHubClaimDecision::BlockedByActiveClaim => {
                        GitHubCheckConclusion::ActionRequired
                    }
                },
            }
        }
        Err(error) => GitHubCheckRunOutput {
            title: "Agent bounty claim needs review".to_string(),
            summary: error.to_string(),
            text: "The claim comment was not converted into an active claim signal. Use `/agent-bounty claim` or `/agent-bounty attempt` with a concrete `plan:`, `branch:`, `draft pr:`, `tests:`, or pull request URL. Claim comments never authorize payment.".to_string(),
            conclusion: GitHubCheckConclusion::ActionRequired,
        },
    }
}

pub fn funding_comment_check_output(
    signal: Result<&GitHubFundingSignal, &GitHubFundingCommentError>,
) -> GitHubCheckRunOutput {
    match signal {
        Ok(signal) => {
            let handoff_text = signal
                .funding_handoff_url
                .as_ref()
                .map(|url| {
                    format!(
                        "\nStripe Checkout funding handoff: {url}\nHandoff boundary: opens the public funding form with UI defaults only; funding still requires verified Stripe webhook reconciliation.\n"
                    )
                })
                .unwrap_or_default();
            GitHubCheckRunOutput {
                title: "Agent bounty funding signal ready".to_string(),
                summary: format!(
                    "{} {} via {:?} requires operator reconciliation.",
                    signal.amount.amount, signal.amount.currency, signal.rail
                ),
                text: format!(
                    "Issue: {}\nContributor: {}\nAmount: {} {}\nRail: {:?}\nIdempotency key: {}\nRequires operator reconciliation: true{handoff_text}\nThis GitHub comment is a public funding signal only. It does not credit the ledger, create a Stripe balance, or mark Base escrow funded.\n\n{}",
                    signal.issue_url,
                    signal
                        .contributor_login
                        .as_deref()
                        .unwrap_or("unknown"),
                    signal.amount.amount,
                    signal.amount.currency,
                    signal.rail,
                    signal.idempotency_key,
                    DISTRIBUTION_FEEDBACK_REQUEST
                ),
                conclusion: GitHubCheckConclusion::Success,
            }
        }
        Err(error) => GitHubCheckRunOutput {
            title: "Agent bounty funding signal needs review".to_string(),
            summary: error.to_string(),
            text: "The funding comment was not converted into a funding signal. Edit the comment to use `/agent-bounty fund <amount> <currency> via <rail>` on a valid paid bounty issue, or reconcile funding manually in the platform.".to_string(),
            conclusion: GitHubCheckConclusion::ActionRequired,
        },
    }
}

fn parse_claim_comment_signal(
    input: &GitHubClaimCommentInput,
) -> Result<GitHubClaimSignal, GitHubClaimCommentError> {
    if input.repository.trim().is_empty()
        || input.issue_url.trim().is_empty()
        || input.title.trim().is_empty()
        || input.body.trim().is_empty()
    {
        return Err(GitHubClaimCommentError::MissingIssueContext);
    }
    parse_issue_form_bounty(
        &input.repository,
        &input.issue_url,
        &input.title,
        &input.body,
    )
    .map_err(|error| GitHubClaimCommentError::NonBountyIssue(error.to_string()))?;

    let command =
        claim_command_line(&input.comment_body).ok_or(GitHubClaimCommentError::MissingCommand)?;
    let contributor = input
        .contributor_login
        .as_ref()
        .map(|login| login.trim().to_string())
        .filter(|login| !login.is_empty());
    let has_progress_signal =
        claim_has_progress_signal(&input.comment_body) || input.progress_signal_count > 0;
    let claim_age = input.claim_age_minutes.unwrap_or(0);
    let is_stale =
        claim_age >= CLAIM_RESERVATION_WINDOW_MINUTES && input.progress_signal_count == 0;

    if let Some(active_claim_login) = input
        .active_claim_login
        .as_ref()
        .map(|login| login.trim())
        .filter(|login| !login.is_empty())
    {
        let same_claimant = contributor
            .as_deref()
            .map(|login| login.eq_ignore_ascii_case(active_claim_login))
            .unwrap_or(false);
        if !same_claimant && !is_stale {
            return Err(GitHubClaimCommentError::ActiveClaimHeld(
                active_claim_login.to_string(),
            ));
        }
    }

    let decision = if is_stale {
        GitHubClaimDecision::StaleReleaseRecommended
    } else if has_progress_signal {
        GitHubClaimDecision::Reserved
    } else if input.active_claim_login.is_some() {
        GitHubClaimDecision::BlockedByActiveClaim
    } else {
        return Err(GitHubClaimCommentError::MissingProgressSignal);
    };

    Ok(GitHubClaimSignal {
        issue_url: input.issue_url.clone(),
        contributor_login: contributor,
        command: claim_command_name(command).to_string(),
        decision,
        reservation_id: claim_reservation_id(input, command),
        reservation_window_minutes: CLAIM_RESERVATION_WINDOW_MINUTES,
        progress_required_within_minutes: CLAIM_RESERVATION_WINDOW_MINUTES,
        progress_signal_count: input.progress_signal_count,
        has_progress_signal,
        settlement_authority: false,
        operator_note:
            "Use this as a public reservation signal only. Release stale claims manually or through a future operator workflow; payment still requires funding, verification, and settlement evidence."
                .to_string(),
    })
}

fn parse_funding_comment_signal(
    input: &GitHubFundingCommentInput,
) -> Result<GitHubFundingSignal, GitHubFundingCommentError> {
    if input.repository.trim().is_empty()
        || input.issue_url.trim().is_empty()
        || input.title.trim().is_empty()
        || input.body.trim().is_empty()
    {
        return Err(GitHubFundingCommentError::MissingIssueContext);
    }
    let bounty = parse_issue_form_bounty(
        &input.repository,
        &input.issue_url,
        &input.title,
        &input.body,
    )
    .map_err(|error| GitHubFundingCommentError::NonBountyIssue(error.to_string()))?;

    let command = funding_command_line(&input.comment_body)
        .ok_or(GitHubFundingCommentError::MissingCommand)?;
    let (amount, rail) = parse_funding_command(command)?;
    validate_comment_funding_rail(&amount, &rail)?;
    let idempotency_key = funding_signal_idempotency_key(input, command, &amount, &rail);
    if input
        .existing_idempotency_keys
        .iter()
        .any(|key| key == &idempotency_key)
    {
        return Err(GitHubFundingCommentError::DuplicateSignal(idempotency_key));
    }

    let funding_handoff_url = funding_handoff_url(input, &bounty, &amount, &rail, &idempotency_key);

    Ok(GitHubFundingSignal {
        issue_url: input.issue_url.clone(),
        contributor_login: input
            .contributor_login
            .as_ref()
            .map(|login| login.trim().to_string())
            .filter(|login| !login.is_empty()),
        amount,
        rail,
        idempotency_key,
        requires_operator_reconciliation: true,
        operator_note:
            "Verify actual Stripe Checkout credit or indexed Base escrow funding before applying this contribution."
                .to_string(),
        funding_handoff_url,
    })
}

fn funding_handoff_url(
    input: &GitHubFundingCommentInput,
    bounty: &GitHubIssueFormBounty,
    amount: &Money,
    rail: &FundingMode,
    idempotency_key: &str,
) -> Option<String> {
    if !matches!(rail, FundingMode::StripeFiatLedger) {
        return None;
    }

    let mut query = Vec::new();
    if let Some(api_base_url) = input
        .funding_api_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        query.push(("apiBaseUrl", api_base_url.to_string()));
    }
    query.extend([
        ("bountyId", bounty.request.id.to_string()),
        ("amountMinor", amount.amount.to_string()),
        ("currency", amount.currency.clone()),
        ("rail", "StripeFiat".to_string()),
        ("source", "github-funding-comment".to_string()),
        ("externalReference", idempotency_key.to_string()),
    ]);

    Some(format!(
        "{STATIC_FUNDING_PAGE_URL}?{}",
        query
            .into_iter()
            .map(|(key, value)| format!("{key}={}", url_query_encode(&value)))
            .collect::<Vec<_>>()
            .join("&")
    ))
}

fn url_query_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn claim_command_line(comment_body: &str) -> Option<&str> {
    comment_body.lines().map(str::trim).find(|line| {
        line.starts_with("/agent-bounty claim") || line.starts_with("/agent-bounty attempt")
    })
}

fn claim_command_name(command: &str) -> &str {
    if command.starts_with("/agent-bounty attempt") {
        "attempt"
    } else {
        "claim"
    }
}

fn claim_has_progress_signal(comment_body: &str) -> bool {
    let lower = comment_body.to_ascii_lowercase();
    if lower.contains("https://github.com/") && lower.contains("/pull/") {
        return true;
    }
    comment_body.lines().any(|line| {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        matches!(
            key.trim().to_ascii_lowercase().as_str(),
            "plan" | "approach" | "branch" | "draft pr" | "pr" | "tests" | "progress"
        ) && value.split_whitespace().count() >= 3
    })
}

fn claim_reservation_id(input: &GitHubClaimCommentInput, command: &str) -> String {
    if let Some(comment_id) = input
        .comment_id
        .as_ref()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    {
        return format!(
            "github-claim-comment:{}:{}:comment:{}",
            input.repository, input.issue_url, comment_id
        );
    }
    let mut hasher = Sha256::new();
    hasher.update(input.repository.as_bytes());
    hasher.update(input.issue_url.as_bytes());
    if let Some(login) = input.contributor_login.as_deref() {
        hasher.update(login.as_bytes());
    }
    hasher.update(command.as_bytes());
    format!("github-claim-comment:{}", hex::encode(hasher.finalize()))
}

fn funding_command_line(comment_body: &str) -> Option<&str> {
    comment_body
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("/agent-bounty fund"))
}

fn parse_funding_command(command: &str) -> Result<(Money, FundingMode), GitHubFundingCommentError> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 6
        || parts[0] != "/agent-bounty"
        || parts[1] != "fund"
        || !parts[4].eq_ignore_ascii_case("via")
    {
        return Err(GitHubFundingCommentError::InvalidCommand);
    }
    let amount_text = format!("{} {}", parts[2], parts[3]);
    let amount = parse_amount(&amount_text)
        .map_err(|_| GitHubFundingCommentError::InvalidAmount(amount_text))?;
    let rail = parse_funding_mode(parts[5])
        .map_err(|_| GitHubFundingCommentError::UnsupportedRail(parts[5].to_string()))?;
    match rail {
        FundingMode::BaseUsdcEscrow | FundingMode::StripeFiatLedger => Ok((amount, rail)),
        FundingMode::Simulated | FundingMode::MixedRails => Err(
            GitHubFundingCommentError::UnsupportedRail(parts[5].to_string()),
        ),
    }
}

fn validate_comment_funding_rail(
    amount: &Money,
    rail: &FundingMode,
) -> Result<(), GitHubFundingCommentError> {
    let expected_currency = match rail {
        FundingMode::BaseUsdcEscrow => "usdc",
        FundingMode::StripeFiatLedger => "usd",
        FundingMode::Simulated | FundingMode::MixedRails => {
            return Err(GitHubFundingCommentError::UnsupportedRail(format!(
                "{rail:?}"
            )));
        }
    };
    if amount.currency != expected_currency {
        return Err(GitHubFundingCommentError::CurrencyRailMismatch {
            currency: amount.currency.clone(),
            rail: format!("{rail:?}"),
        });
    }
    Ok(())
}

fn funding_signal_idempotency_key(
    input: &GitHubFundingCommentInput,
    command: &str,
    amount: &Money,
    rail: &FundingMode,
) -> String {
    if let Some(comment_id) = input
        .comment_id
        .as_ref()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
    {
        return format!(
            "github-funding-comment:{}:{}:comment:{}",
            input.repository, input.issue_url, comment_id
        );
    }
    let mut hasher = Sha256::new();
    hasher.update(input.repository.as_bytes());
    hasher.update(input.issue_url.as_bytes());
    if let Some(login) = input.contributor_login.as_deref() {
        hasher.update(login.as_bytes());
    }
    hasher.update(command.as_bytes());
    hasher.update(amount.amount.to_string().as_bytes());
    hasher.update(amount.currency.as_bytes());
    hasher.update(format!("{rail:?}").as_bytes());
    format!("github-funding-comment:{}", hex::encode(hasher.finalize()))
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
        | "payment-state-machine"
        | "small-web-public-change"
        | "docs-and-cli-report"
        | "extract-data-to-schema"
        | "primary-source-research"
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
        assert!(markdown.contains("Distribution feedback requested"));
        assert!(markdown.contains("what tool, prompt, link, label, scanner, or workflow"));
        assert!(markdown.contains("star the repository"));
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
    fn ignores_optional_cofunding_note_section() {
        let body = r#"### Goal
Improve the public bounty discovery page.

### Acceptance criteria
The page links to claim, status, funding, and proof actions without private data.

### Template
write-docs-for-area

### Suggested amount
5 USDC

### Funding mode
BaseUsdcEscrow

### Co-funding note
Supporters can add funds after the platform bounty URL is linked.

### Discovery feedback
Found it from a proof page and posted because the payment path is explicit.

### Privacy
Public
"#;

        let bounty = parse_issue_form_bounty(
            "agent-bounties/agent-bounties",
            "https://github.com/agent-bounties/agent-bounties/issues/4",
            "[bounty]: Improve public bounty discovery",
            body,
        )
        .unwrap();

        assert_eq!(bounty.template_slug, "write-docs-for-area");
        assert_eq!(bounty.amount, Money::new(5_000_000, "usdc").unwrap());
        assert_eq!(bounty.funding_mode, FundingMode::BaseUsdcEscrow);
        assert_eq!(bounty.privacy, PrivacyLevel::Public);
        assert_eq!(
            bounty.discovery_feedback.as_deref(),
            Some("Found it from a proof page and posted because the payment path is explicit.")
        );
    }

    #[test]
    fn validates_public_launch_template_slugs() {
        for slug in [
            "payment-state-machine",
            "small-web-public-change",
            "docs-and-cli-report",
            "primary-source-research",
        ] {
            let body = format!(
                r#"### Goal
Complete a focused project task for {slug}.

### Acceptance criteria
The change has deterministic evidence and a clear proof record.

### Template
{slug}

### Suggested amount
5 USDC
"#
            );

            let bounty = parse_issue_form_bounty(
                "agent-bounties/agent-bounties",
                &format!("https://github.com/agent-bounties/agent-bounties/issues/{slug}"),
                "[bounty]: Validate template",
                &body,
            )
            .unwrap();

            assert_eq!(bounty.template_slug, slug);
        }
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
        assert!(output.text.contains("Distribution feedback"));
        assert!(output.text.contains("How did you find Agent Bounties?"));
        assert!(output.text.contains("star the repository"));
    }

    #[test]
    fn funding_comment_plan_accepts_base_usdc_signal() {
        let input = GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        };

        let plan = funding_comment_plan(input);

        assert!(plan.ready);
        let signal = plan.signal.unwrap();
        assert_eq!(signal.amount, Money::new(5_000_000, "usdc").unwrap());
        assert_eq!(signal.rail, FundingMode::BaseUsdcEscrow);
        assert!(signal.requires_operator_reconciliation);
        assert!(signal.funding_handoff_url.is_none());
        assert!(signal.idempotency_key.ends_with(":comment:123"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
        assert!(!plan.check.text.contains("Stripe Checkout funding handoff"));
        assert!(plan.check.text.contains("Distribution feedback requested"));
        assert!(plan
            .check
            .text
            .contains("what tool, prompt, link, label, scanner, or workflow"));
        assert!(plan.check.text.contains("star the repository"));
    }

    #[test]
    fn funding_comment_plan_adds_stripe_checkout_handoff_url() {
        let input = GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("StripeFiatLedger"),
            comment_body: "/agent-bounty fund 5 USD via StripeFiatLedger".to_string(),
            contributor_login: Some("human-funder".to_string()),
            comment_id: Some("124".to_string()),
            funding_api_base_url: Some("https://api.agentbounties.example/".to_string()),
            existing_idempotency_keys: vec![],
        };

        let plan = funding_comment_plan(input);

        assert!(plan.ready);
        let signal = plan.signal.unwrap();
        let handoff = signal.funding_handoff_url.expect("handoff url");
        assert_eq!(signal.amount, Money::new(500, "usd").unwrap());
        assert_eq!(signal.rail, FundingMode::StripeFiatLedger);
        assert!(handoff.starts_with(STATIC_FUNDING_PAGE_URL));
        assert!(handoff.contains("apiBaseUrl=https%3A%2F%2Fapi.agentbounties.example"));
        assert!(handoff.contains("amountMinor=500"));
        assert!(handoff.contains("currency=usd"));
        assert!(handoff.contains("rail=StripeFiat"));
        assert!(handoff.contains("source=github-funding-comment"));
        assert!(handoff.contains("externalReference=github-funding-comment%3A"));
        assert!(plan.check.text.contains("Stripe Checkout funding handoff"));
        assert!(plan
            .check
            .text
            .contains("verified Stripe webhook reconciliation"));
    }

    #[test]
    fn claim_comment_rejects_instant_templated_no_progress_claim() {
        let plan = claim_comment_plan(GitHubClaimCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body:
                "/agent-bounty claim\nI'm reviewing the codebase and will open a PR shortly."
                    .to_string(),
            contributor_login: Some("claim-bot".to_string()),
            comment_id: Some("501".to_string()),
            claim_age_minutes: Some(1),
            progress_signal_count: 0,
            active_claim_login: None,
        });

        assert!(!plan.ready);
        assert!(plan.signal.is_none());
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
        assert!(plan.error.unwrap().contains("concrete progress signal"));
    }

    #[test]
    fn claim_comment_reserves_when_progress_signal_is_present() {
        let plan = claim_comment_plan(GitHubClaimCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty claim\nPlan: add a deterministic planner and tests."
                .to_string(),
            contributor_login: Some("solver-agent".to_string()),
            comment_id: Some("502".to_string()),
            claim_age_minutes: Some(5),
            progress_signal_count: 0,
            active_claim_login: None,
        });

        assert!(plan.ready);
        let signal = plan.signal.unwrap();
        assert_eq!(signal.decision, GitHubClaimDecision::Reserved);
        assert!(signal.has_progress_signal);
        assert_eq!(
            signal.reservation_window_minutes,
            CLAIM_RESERVATION_WINDOW_MINUTES
        );
        assert!(!signal.settlement_authority);
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::Success);
        assert!(plan.check.text.contains("Settlement authority: false"));
    }

    #[test]
    fn claim_comment_recommends_release_for_stale_claim_without_progress() {
        let plan = claim_comment_plan(GitHubClaimCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty claim\nStill looking.".to_string(),
            contributor_login: Some("stale-agent".to_string()),
            comment_id: Some("503".to_string()),
            claim_age_minutes: Some(CLAIM_RESERVATION_WINDOW_MINUTES + 1),
            progress_signal_count: 0,
            active_claim_login: Some("stale-agent".to_string()),
        });

        assert!(plan.ready);
        let signal = plan.signal.unwrap();
        assert_eq!(
            signal.decision,
            GitHubClaimDecision::StaleReleaseRecommended
        );
        assert!(!signal.settlement_authority);
        assert!(plan.check.summary.contains("expired without progress"));
    }

    #[test]
    fn claim_comment_blocks_other_solver_inside_active_reservation_window() {
        let plan = claim_comment_plan(GitHubClaimCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/58".to_string(),
            title: "[bounty]: Add stale-claim controls".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty attempt\nPlan: open an alternative PR with tests."
                .to_string(),
            contributor_login: Some("second-agent".to_string()),
            comment_id: Some("504".to_string()),
            claim_age_minutes: Some(30),
            progress_signal_count: 0,
            active_claim_login: Some("first-agent".to_string()),
        });

        assert!(!plan.ready);
        assert!(plan.signal.is_none());
        assert!(plan.error.unwrap().contains("active claim is held"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[test]
    fn funding_comment_plan_rejects_invalid_amount() {
        let plan = funding_comment_plan(GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty fund nope USDC via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: None,
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        });

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("invalid funding amount"));
        assert_eq!(plan.check.conclusion, GitHubCheckConclusion::ActionRequired);
    }

    #[test]
    fn funding_comment_plan_rejects_unsupported_rail() {
        let plan = funding_comment_plan(GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty fund 5 USDC via Simulated".to_string(),
            contributor_login: None,
            comment_id: None,
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        });

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("unsupported funding rail"));
    }

    #[test]
    fn funding_comment_plan_rejects_duplicate_signal() {
        let existing_key =
            "github-funding-comment:agent-bounties/agent-bounties:https://github.com/agent-bounties/agent-bounties/issues/20:comment:123";
        let plan = funding_comment_plan(GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: Some("123".to_string()),
            funding_api_base_url: None,
            existing_idempotency_keys: vec![existing_key.to_string()],
        });

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("duplicate funding signal"));
    }

    #[test]
    fn funding_comment_plan_rejects_non_bounty_issue() {
        let plan = funding_comment_plan(GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "Plain issue".to_string(),
            body: "not an issue form".to_string(),
            comment_body: "/agent-bounty fund 5 USDC via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: None,
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        });

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("not a valid paid bounty"));
    }

    #[test]
    fn funding_comment_plan_rejects_currency_rail_mismatch() {
        let plan = funding_comment_plan(GitHubFundingCommentInput {
            repository: "agent-bounties/agent-bounties".to_string(),
            issue_url: "https://github.com/agent-bounties/agent-bounties/issues/20".to_string(),
            title: "[bounty]: Add co-funding".to_string(),
            body: valid_issue_body("BaseUsdcEscrow"),
            comment_body: "/agent-bounty fund 5 USD via BaseUsdcEscrow".to_string(),
            contributor_login: None,
            comment_id: None,
            funding_api_base_url: None,
            existing_idempotency_keys: vec![],
        });

        assert!(!plan.ready);
        assert!(plan.error.unwrap().contains("does not match funding rail"));
    }

    fn valid_issue_body(funding_mode: &str) -> String {
        format!(
            r#"### Goal
Improve co-funding.

### Acceptance criteria
The public signal is deterministic and cannot credit the ledger directly.

### Template
write-docs-for-area

### Suggested amount
5 USDC

### Funding mode
{funding_mode}
"#
        )
    }
}
