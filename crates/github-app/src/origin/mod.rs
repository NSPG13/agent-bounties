//! Provider-neutral, deterministic plans for turning origin issues into bounty
//! drafts and returning lifecycle evidence to the issue tracker.
//!
//! This module performs no provider, wallet, verifier, or chain writes. An
//! authorized worker may translate the returned write intents into provider
//! API calls after independently authenticating the webhook and evidence.

use domain::{Id, Money};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod github;
pub mod linear;
pub mod runtime;

pub const PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR: i64 = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginProvider {
    #[serde(rename = "github")]
    GitHub,
    #[serde(rename = "linear")]
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginTrigger {
    Command,
    Mention,
    Assignment,
    Delegation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginSourceReference {
    pub provider: OriginProvider,
    pub workspace: String,
    pub external_id: String,
    pub display_id: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginVerifierRequirement {
    pub kind: String,
    pub instructions: String,
    pub requires_review: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginReward {
    pub solver: Money,
    pub verifier: Money,
    pub target: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedOriginBountyDraft {
    pub source: OriginSourceReference,
    pub trigger: OriginTrigger,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub reward: OriginReward,
    pub verifier: OriginVerifierRequirement,
    pub ready_for_publish: bool,
    pub fields_requiring_review: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OriginAuthorityBoundary {
    pub holds_wallet_keys: bool,
    pub requests_wallet_signature: bool,
    pub funding_authority: bool,
    pub verification_authority: bool,
    pub settlement_authority: bool,
    pub provider_write_performed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginDraftPlan {
    pub ready_for_human_review: bool,
    pub draft: Option<NormalizedOriginBountyDraft>,
    pub idempotency_key: Option<String>,
    pub duplicate: bool,
    pub error: Option<String>,
    pub authority: OriginAuthorityBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginProgressStatus {
    DraftPrepared,
    CanonicalFundingConfirmed,
    CanonicalClaimConfirmed,
    CanonicalSubmissionConfirmed,
}

impl OriginProgressStatus {
    fn display(self) -> &'static str {
        match self {
            Self::DraftPrepared => "bounty draft prepared for review",
            Self::CanonicalFundingConfirmed => "canonical funding confirmed",
            Self::CanonicalClaimConfirmed => "canonical solver claim confirmed",
            Self::CanonicalSubmissionConfirmed => "canonical submission confirmed",
        }
    }

    fn requires_canonical_evidence(self) -> bool {
        self != Self::DraftPrepared
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginProgressInput {
    pub source: OriginSourceReference,
    pub bounty_id: Id,
    pub status: OriginProgressStatus,
    pub status_url: String,
    pub canonical_evidence_url: Option<String>,
    #[serde(default)]
    pub existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginProgressCallbackPlan {
    pub ready: bool,
    pub status: OriginProgressStatus,
    pub operations: Vec<OriginWriteOperation>,
    pub error: Option<String>,
    pub authority: OriginAuthorityBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalSettlementEvent {
    BountySettled,
    CompetitionSettledV2,
}

impl CanonicalSettlementEvent {
    fn display(self) -> &'static str {
        match self {
            Self::BountySettled => "BountySettled",
            Self::CompetitionSettledV2 => "CompetitionSettledV2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginVerificationEvidence {
    pub passed: bool,
    pub committed_policy_matched: bool,
    pub summary: String,
    pub evidence_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginSettlementReceipt {
    pub event: CanonicalSettlementEvent,
    pub canonical_contract_verified: bool,
    pub confirmed: bool,
    pub chain_id: u64,
    pub transaction_hash: String,
    pub log_index: u64,
    pub receipt_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginResultInput {
    pub source: OriginSourceReference,
    pub bounty_id: Id,
    pub status_url: String,
    pub artifact_url: Option<String>,
    pub verification: Option<OriginVerificationEvidence>,
    pub settlement: Option<OriginSettlementReceipt>,
    #[serde(default)]
    pub existing_idempotency_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginCompletionStatus {
    InProgress,
    SubmittedAwaitingVerification,
    VerificationFailed,
    VerifiedAwaitingSettlement,
    Settled,
}

impl OriginCompletionStatus {
    fn display(self) -> &'static str {
        match self {
            Self::InProgress => "in progress",
            Self::SubmittedAwaitingVerification => "submitted; awaiting verification",
            Self::VerificationFailed => "verification failed",
            Self::VerifiedAwaitingSettlement => "verified; awaiting canonical settlement",
            Self::Settled => "verified and settled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OriginWriteOperation {
    UpsertStatusComment {
        idempotency_key: String,
        stable_marker: String,
        markdown: String,
        already_applied: bool,
    },
    CloseIssue {
        idempotency_key: String,
        depends_on_idempotency_key: String,
        completion_reason: String,
        already_applied: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginResultCallbackPlan {
    pub ready: bool,
    pub status: OriginCompletionStatus,
    pub close_origin: bool,
    pub operations: Vec<OriginWriteOperation>,
    pub blocked_reasons: Vec<String>,
    pub error: Option<String>,
    pub authority: OriginAuthorityBoundary,
}

pub fn plan_origin_progress_callback(input: OriginProgressInput) -> OriginProgressCallbackPlan {
    if let Err(error) = validate_source(&input.source) {
        return invalid_progress_plan(input.status, error);
    }
    if !is_https_url(&input.status_url) {
        return invalid_progress_plan(input.status, "status URL must use HTTPS".to_string());
    }
    let evidence_url = input
        .canonical_evidence_url
        .as_deref()
        .filter(|url| is_https_url(url));
    if input.status.requires_canonical_evidence() && evidence_url.is_none() {
        return invalid_progress_plan(
            input.status,
            "canonical lifecycle status requires an HTTPS indexed-evidence URL".to_string(),
        );
    }

    let status_key = progress_idempotency_key(&input);
    let stable_marker = status_comment_marker(&input.source, input.bounty_id);
    let mut markdown = format!(
        "Agent Bounties status: **{}**\n\nBounty: `{}`\nStatus: {}",
        input.status.display(),
        input.bounty_id,
        input.status_url
    );
    if let Some(url) = evidence_url {
        markdown.push_str(&format!("\nCanonical evidence: {url}"));
    }
    markdown.push_str(
        "\n\nThis callback reports reconciled status only. It cannot hold keys, sign or fund transactions, verify work, authorize payout, settle a bounty, or close the origin issue.\n\n",
    );
    markdown.push_str(&stable_marker);
    let already_applied = input.existing_idempotency_keys.contains(&status_key);

    OriginProgressCallbackPlan {
        ready: true,
        status: input.status,
        operations: vec![OriginWriteOperation::UpsertStatusComment {
            idempotency_key: status_key,
            stable_marker,
            markdown,
            already_applied,
        }],
        error: None,
        authority: OriginAuthorityBoundary::default(),
    }
}

pub fn plan_origin_result_callback(input: OriginResultInput) -> OriginResultCallbackPlan {
    if let Err(error) = validate_source(&input.source) {
        return invalid_result_plan(error);
    }
    if !is_https_url(&input.status_url) {
        return invalid_result_plan("status URL must use HTTPS".to_string());
    }

    let artifact_valid = input.artifact_url.as_deref().is_some_and(is_https_url);
    let verification_valid = input.verification.as_ref().is_some_and(|evidence| {
        evidence.passed
            && evidence.committed_policy_matched
            && !evidence.summary.trim().is_empty()
            && is_https_url(&evidence.evidence_url)
    });
    let settlement_valid = input
        .settlement
        .as_ref()
        .is_some_and(valid_settlement_receipt);

    let mut blocked_reasons = Vec::new();
    if input.artifact_url.is_some() && !artifact_valid {
        blocked_reasons.push("artifact URL must use HTTPS".to_string());
    }
    if let Some(evidence) = &input.verification {
        if !evidence.passed {
            blocked_reasons.push("precommitted verification did not pass".to_string());
        } else if !evidence.committed_policy_matched {
            blocked_reasons
                .push("verification evidence does not match the committed policy".to_string());
        } else if evidence.summary.trim().is_empty() || !is_https_url(&evidence.evidence_url) {
            blocked_reasons
                .push("verification evidence needs a summary and HTTPS evidence URL".to_string());
        }
    }
    if input.settlement.is_some() && !settlement_valid {
        blocked_reasons.push(
            "settlement receipt must identify a confirmed canonical contract event with a valid chain transaction and HTTPS receipt"
                .to_string(),
        );
    }

    let status = if artifact_valid && verification_valid && settlement_valid {
        OriginCompletionStatus::Settled
    } else if verification_valid {
        OriginCompletionStatus::VerifiedAwaitingSettlement
    } else if input
        .verification
        .as_ref()
        .is_some_and(|evidence| !evidence.passed)
    {
        OriginCompletionStatus::VerificationFailed
    } else if artifact_valid {
        OriginCompletionStatus::SubmittedAwaitingVerification
    } else {
        OriginCompletionStatus::InProgress
    };
    let close_origin = status == OriginCompletionStatus::Settled;
    if !close_origin && blocked_reasons.is_empty() {
        blocked_reasons.push(match status {
            OriginCompletionStatus::InProgress => {
                "no valid artifact has been submitted".to_string()
            }
            OriginCompletionStatus::SubmittedAwaitingVerification => {
                "valid precommitted verification evidence is pending".to_string()
            }
            OriginCompletionStatus::VerifiedAwaitingSettlement => {
                "confirmed canonical settlement is pending".to_string()
            }
            OriginCompletionStatus::VerificationFailed => {
                "precommitted verification did not pass".to_string()
            }
            OriginCompletionStatus::Settled => unreachable!(),
        });
    }

    let status_key = result_idempotency_key(&input, status);
    let stable_marker = status_comment_marker(&input.source, input.bounty_id);
    let markdown = render_status_markdown(&input, status, &blocked_reasons, &stable_marker);
    let mut operations = vec![OriginWriteOperation::UpsertStatusComment {
        already_applied: input.existing_idempotency_keys.contains(&status_key),
        idempotency_key: status_key.clone(),
        stable_marker,
        markdown,
    }];

    if close_origin {
        let settlement = input
            .settlement
            .as_ref()
            .expect("settled status requires settlement evidence");
        let close_key = close_idempotency_key(&input.source, input.bounty_id, settlement);
        operations.push(OriginWriteOperation::CloseIssue {
            already_applied: input.existing_idempotency_keys.contains(&close_key),
            idempotency_key: close_key,
            depends_on_idempotency_key: status_key,
            completion_reason: "verified artifact returned and canonical settlement confirmed"
                .to_string(),
        });
    }

    OriginResultCallbackPlan {
        ready: true,
        status,
        close_origin,
        operations,
        blocked_reasons,
        error: None,
        authority: OriginAuthorityBoundary::default(),
    }
}

pub(crate) fn reward(solver: Money, verifier: Money) -> OriginReward {
    let target = Money::new(
        solver
            .amount
            .checked_add(verifier.amount)
            .expect("bounded bounty rewards fit in i64"),
        solver.currency.clone(),
    )
    .expect("positive matching rewards produce a valid target");
    OriginReward {
        solver,
        verifier,
        target,
    }
}

pub(crate) fn valid_public_solver_reward(reward: &Money) -> bool {
    reward.currency == "usdc" && reward.amount >= PUBLIC_MINIMUM_SOLVER_REWARD_USDC_MINOR
}

pub(crate) fn stable_hash(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

pub(crate) fn is_https_url(value: &str) -> bool {
    let value = value.trim();
    let Some(authority) = value.strip_prefix("https://") else {
        return false;
    };
    !authority.is_empty() && !authority.starts_with('/') && !value.chars().any(char::is_whitespace)
}

pub(crate) fn markdown_sections(value: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut heading: Option<String> = None;
    let mut body = Vec::new();
    for line in value.lines() {
        if let Some(found) = line
            .strip_prefix("### ")
            .or_else(|| line.strip_prefix("## "))
        {
            if let Some(previous) = heading.take() {
                sections.push((previous, body.join("\n").trim().to_string()));
                body.clear();
            }
            heading = Some(found.trim().to_ascii_lowercase());
        } else if heading.is_some() {
            body.push(line);
        }
    }
    if let Some(previous) = heading {
        sections.push((previous, body.join("\n").trim().to_string()));
    }
    sections
}

pub(crate) fn parse_explicit_criteria(value: &str) -> Vec<String> {
    let criteria = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(strip_list_marker)
        .map(str::to_string)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if criteria.is_empty() && !value.trim().is_empty() {
        vec![value.trim().to_string()]
    } else {
        criteria
    }
}

fn strip_list_marker(value: &str) -> &str {
    if let Some(value) = value
        .strip_prefix("- ")
        .or_else(|| value.strip_prefix("* "))
    {
        return value.trim();
    }
    if let Some((number, value)) = value.split_once(". ") {
        if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
            return value.trim();
        }
    }
    value
}

pub(crate) fn validate_source(source: &OriginSourceReference) -> Result<(), String> {
    if source.workspace.trim().is_empty()
        || source.external_id.trim().is_empty()
        || source.display_id.trim().is_empty()
    {
        return Err("origin source requires workspace, external id, and display id".to_string());
    }
    if !is_https_url(&source.url) {
        return Err("origin source URL must use HTTPS".to_string());
    }
    match source.provider {
        OriginProvider::GitHub if !is_github_issue_url(&source.url) => {
            Err("GitHub source must be an HTTPS issue URL".to_string())
        }
        OriginProvider::Linear if !is_linear_issue_url(&source.url) => {
            Err("Linear source must be an HTTPS Linear issue URL".to_string())
        }
        _ => Ok(()),
    }
}

fn is_github_issue_url(value: &str) -> bool {
    value.starts_with("https://github.com/")
        && value
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.contains("/issues/"))
}

fn is_linear_issue_url(value: &str) -> bool {
    value.starts_with("https://linear.app/")
        && value
            .split(['?', '#'])
            .next()
            .is_some_and(|path| path.contains("/issue/"))
}

fn valid_settlement_receipt(receipt: &OriginSettlementReceipt) -> bool {
    receipt.canonical_contract_verified
        && receipt.confirmed
        && receipt.chain_id > 0
        && is_transaction_hash(&receipt.transaction_hash)
        && is_https_url(&receipt.receipt_url)
}

fn is_transaction_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn source_bounty_identity(source: &OriginSourceReference, bounty_id: Id) -> String {
    format!(
        "{:?}:{}:{}:{}",
        source.provider, source.workspace, source.external_id, bounty_id
    )
    .to_ascii_lowercase()
}

fn status_comment_marker(source: &OriginSourceReference, bounty_id: Id) -> String {
    format!(
        "<!-- agent-bounties-origin-status:{} -->",
        stable_hash(&source_bounty_identity(source, bounty_id))
    )
}

fn progress_idempotency_key(input: &OriginProgressInput) -> String {
    let identity = format!(
        "{}\n{:?}\n{}",
        source_bounty_identity(&input.source, input.bounty_id),
        input.status,
        input.canonical_evidence_url.as_deref().unwrap_or_default()
    );
    format!("origin-progress-v1:{}", stable_hash(&identity))
}

fn result_idempotency_key(input: &OriginResultInput, status: OriginCompletionStatus) -> String {
    let verification_identity = input
        .verification
        .as_ref()
        .map(|evidence| {
            format!(
                "{}:{}:{}:{}",
                evidence.passed,
                evidence.committed_policy_matched,
                evidence.summary,
                evidence.evidence_url
            )
        })
        .unwrap_or_default();
    let settlement_identity = input
        .settlement
        .as_ref()
        .map(|receipt| {
            format!(
                "{:?}:{}:{}:{}:{}:{}",
                receipt.event,
                receipt.canonical_contract_verified,
                receipt.confirmed,
                receipt.chain_id,
                receipt.transaction_hash,
                receipt.log_index
            )
        })
        .unwrap_or_default();
    let identity = format!(
        "{}\n{:?}\n{}\n{}\n{}",
        source_bounty_identity(&input.source, input.bounty_id),
        status,
        input.artifact_url.as_deref().unwrap_or_default(),
        verification_identity,
        settlement_identity
    );
    format!("origin-result-v1:{}", stable_hash(&identity))
}

fn close_idempotency_key(
    source: &OriginSourceReference,
    bounty_id: Id,
    settlement: &OriginSettlementReceipt,
) -> String {
    let identity = format!(
        "{}:{}:{}",
        source_bounty_identity(source, bounty_id),
        settlement.transaction_hash.to_ascii_lowercase(),
        settlement.log_index
    );
    format!("origin-close-v1:{}", stable_hash(&identity))
}

fn render_status_markdown(
    input: &OriginResultInput,
    status: OriginCompletionStatus,
    blocked_reasons: &[String],
    stable_marker: &str,
) -> String {
    let mut lines = vec![
        format!("Agent Bounties status: **{}**", status.display()),
        String::new(),
        format!("Bounty: `{}`", input.bounty_id),
        format!("Status: {}", input.status_url),
    ];
    if let Some(url) = input
        .artifact_url
        .as_deref()
        .filter(|url| is_https_url(url))
    {
        lines.push(format!("Artifact: {url}"));
    }
    if let Some(evidence) = &input.verification {
        if is_https_url(&evidence.evidence_url) {
            lines.push(format!(
                "Verification: {} ({})",
                sanitized_summary(&evidence.summary),
                evidence.evidence_url
            ));
        }
    }
    if let Some(receipt) = &input.settlement {
        if valid_settlement_receipt(receipt) {
            lines.push(format!(
                "Settlement: confirmed canonical `{}` on chain {} at {} (transaction `{}`, log {})",
                receipt.event.display(),
                receipt.chain_id,
                receipt.receipt_url,
                receipt.transaction_hash,
                receipt.log_index
            ));
        }
    }
    if !blocked_reasons.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "Origin remains open: {}.",
            blocked_reasons.join("; ")
        ));
    }
    lines.extend([
        String::new(),
        "This callback reports reconciled evidence only. It cannot hold keys, sign or fund transactions, verify work, authorize payout, or settle a bounty.".to_string(),
        String::new(),
        stable_marker.to_string(),
    ]);
    lines.join("\n")
}

fn sanitized_summary(value: &str) -> String {
    value
        .trim()
        .replace(['<', '>'], "")
        .chars()
        .take(500)
        .collect()
}

fn invalid_result_plan(error: String) -> OriginResultCallbackPlan {
    OriginResultCallbackPlan {
        ready: false,
        status: OriginCompletionStatus::InProgress,
        close_origin: false,
        operations: vec![],
        blocked_reasons: vec![],
        error: Some(error),
        authority: OriginAuthorityBoundary::default(),
    }
}

fn invalid_progress_plan(
    status: OriginProgressStatus,
    error: String,
) -> OriginProgressCallbackPlan {
    OriginProgressCallbackPlan {
        ready: false,
        status,
        operations: vec![],
        error: Some(error),
        authority: OriginAuthorityBoundary::default(),
    }
}
