use domain::{FundingMode, Id, Money, PaymentRail, PrivacyLevel, RiskAction, RiskSurface};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const RISK_POLICY_VERSION: &str = "risk-policy-v0";

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RiskAssessment {
    pub surface: RiskSurface,
    pub action: RiskAction,
    pub score: u16,
    pub reasons: Vec<String>,
}

impl RiskAssessment {
    pub fn allow(surface: RiskSurface) -> Self {
        Self {
            surface,
            action: RiskAction::Allow,
            score: 0,
            reasons: Vec::new(),
        }
    }

    pub fn is_allowed(&self) -> bool {
        self.action == RiskAction::Allow
    }
}

#[derive(Debug, Clone)]
pub struct HelpRequestRiskInput {
    pub goal: String,
    pub context: String,
    pub budget: Money,
    pub privacy: PrivacyLevel,
}

#[derive(Debug, Clone)]
pub struct BountyRiskInput {
    pub title: String,
    pub template_slug: String,
    pub amount: Money,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
}

#[derive(Debug, Clone)]
pub struct SubmissionRiskInput {
    pub bounty_id: Id,
    pub solver_agent_id: Id,
    pub claimed_solver_agent_id: Option<Id>,
    pub artifact_uri: String,
    pub artifact_body: String,
}

#[derive(Debug, Clone)]
pub struct PayoutRiskInput {
    pub bounty_id: Id,
    pub rail: PaymentRail,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DirectBountyEvidenceChecklist {
    pub submission: Vec<String>,
    pub verification: Vec<String>,
    pub payment: Vec<String>,
    pub rejection_rules: Vec<String>,
}

impl Default for DirectBountyEvidenceChecklist {
    fn default() -> Self {
        Self {
            submission: vec![
                "exact source repository HTTPS URL".to_string(),
                "exact source commit SHA".to_string(),
                "repository subdirectory, or . when the repository root is the artifact"
                    .to_string(),
                "pull request HTTPS URL".to_string(),
                "immutable HTTPS artifact URL".to_string(),
                "artifact SHA-256 digest".to_string(),
            ],
            verification: vec![
                "check-run HTTPS URLs for the benchmark or CI runs used by the verifier"
                    .to_string(),
                "artifact digest that the verifier evaluated".to_string(),
            ],
            payment: vec![
                "canonical BountySettled event URL or transaction reference".to_string(),
                "submission, verification, PR, and test evidence are not payment evidence"
                    .to_string(),
            ],
            rejection_rules: vec![
                "empty evidence fields are rejected".to_string(),
                "non-HTTPS repository, pull request, check-run, artifact, or settlement references are rejected".to_string(),
                "mutable artifact references such as branch names, tags, releases, query refs, encoded lookalikes, or unpinned download URLs are rejected".to_string(),
                "evidence fields from mismatched repositories or mismatched source commits are rejected".to_string(),
                "payment evidence without a matching canonical BountySettled event is rejected".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DirectBountyEvidence {
    pub repository_url: String,
    pub source_commit: String,
    pub subdirectory: String,
    pub pull_request_url: String,
    pub check_run_urls: Vec<String>,
    pub artifact_url: String,
    pub artifact_sha256: String,
    pub bounty_settled_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DirectBountyEvidenceReport {
    pub checklist: DirectBountyEvidenceChecklist,
    pub accepted: bool,
    pub payment_settlement_status: String,
    pub errors: Vec<String>,
    pub evidence_boundary: String,
}

pub fn direct_bounty_evidence_checklist() -> DirectBountyEvidenceChecklist {
    DirectBountyEvidenceChecklist::default()
}

pub fn validate_direct_bounty_evidence(
    evidence: &DirectBountyEvidence,
) -> DirectBountyEvidenceReport {
    let mut errors = Vec::new();

    require_non_empty(&mut errors, "repository_url", &evidence.repository_url);
    require_non_empty(&mut errors, "source_commit", &evidence.source_commit);
    require_non_empty(&mut errors, "subdirectory", &evidence.subdirectory);
    require_non_empty(&mut errors, "pull_request_url", &evidence.pull_request_url);
    require_non_empty(&mut errors, "artifact_url", &evidence.artifact_url);
    require_non_empty(&mut errors, "artifact_sha256", &evidence.artifact_sha256);
    require_non_empty(&mut errors, "bounty_settled_url", &evidence.bounty_settled_url);

    require_https(&mut errors, "repository_url", &evidence.repository_url);
    require_https(&mut errors, "pull_request_url", &evidence.pull_request_url);
    require_https(&mut errors, "artifact_url", &evidence.artifact_url);
    require_https(&mut errors, "bounty_settled_url", &evidence.bounty_settled_url);

    if evidence.check_run_urls.is_empty() {
        errors.push("check_run_urls must contain at least one HTTPS check-run URL".to_string());
    }
    for (index, url) in evidence.check_run_urls.iter().enumerate() {
        require_non_empty(&mut errors, &format!("check_run_urls[{index}]"), url);
        require_https(&mut errors, &format!("check_run_urls[{index}]"), url);
    }

    let is_valid_source_commit = is_hex_sha(&evidence.source_commit, 40);
    if !is_valid_source_commit {
        errors.push("source_commit must be a full 40-character hexadecimal commit SHA".to_string());
    }
    if !is_hex_sha(&evidence.artifact_sha256, 64) {
        errors.push("artifact_sha256 must be a 64-character hexadecimal SHA-256 digest".to_string());
    }

    let repo_canonical = parse_github_repo(&evidence.repository_url);
    if repo_canonical.is_none() && evidence.repository_url.starts_with("https://") && !evidence.repository_url.trim().is_empty() {
        errors.push("repository_url must be a valid GitHub repository URL (https://github.com/owner/repo)".to_string());
    }

    if let Some((owner, repo)) = &repo_canonical {
        if let Some((pr_owner, pr_repo)) = parse_github_pr_repo(&evidence.pull_request_url) {
            if pr_owner != *owner || pr_repo != *repo {
                errors.push("pull_request_url must belong to the same repository as repository_url".to_string());
            }
        } else if evidence.pull_request_url.starts_with("https://") && !evidence.pull_request_url.trim().is_empty() {
            errors.push("pull_request_url must be a valid pull request URL for repository_url".to_string());
        }

        for (index, cr_url) in evidence.check_run_urls.iter().enumerate() {
            if let Some((cr_owner, cr_repo)) = parse_github_repo_from_any_url(cr_url) {
                if cr_owner != *owner || cr_repo != *repo {
                    errors.push(format!("check_run_urls[{index}] must belong to the same repository as repository_url"));
                }
            } else if cr_url.starts_with("https://") && !cr_url.trim().is_empty() {
                errors.push(format!("check_run_urls[{index}] must belong to the same repository as repository_url"));
            }
        }
    }

    match parse_immutable_artifact_url(&evidence.artifact_url) {
        Some(artifact) => {
            if let Some((owner, repo)) = &repo_canonical {
                if artifact.owner != *owner || artifact.repo != *repo {
                    errors.push("artifact_url repository must match repository_url".to_string());
                }
            }
            if is_valid_source_commit && !artifact.commit.eq_ignore_ascii_case(&evidence.source_commit) {
                errors.push("artifact_url commit SHA must match source_commit".to_string());
            }
            if artifact.path.trim().is_empty() {
                errors.push("artifact_url must specify a non-empty file path".to_string());
            }
        }
        None => {
            errors.push("artifact_url must be immutable by construction (pinned commit blob/raw HTTPS URL with non-empty path)".to_string());
        }
    }

    DirectBountyEvidenceReport {
        checklist: DirectBountyEvidenceChecklist::default(),
        accepted: errors.is_empty(),
        payment_settlement_status: "unverified: structural checklist references provided; requires canonical BountySettled event verification".to_string(),
        errors,
        evidence_boundary: "Submission, verification, and payment evidence are separate. Only canonical BountySettled proves payment.".to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct RiskPolicy {
    pub low_value_usdc_cap_minor: i64,
    pub max_artifact_body_bytes: usize,
    pub min_clarity_chars: usize,
    banned_terms: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RiskPolicyDescriptor {
    pub version: String,
    pub low_value_usdc_cap_minor: i64,
    pub low_value_usdc_cap_currency: String,
    pub max_artifact_body_bytes: usize,
    pub min_clarity_chars: usize,
    pub banned_terms: Vec<String>,
    pub automatic_review_rules: Vec<String>,
    pub blocked_rules: Vec<String>,
    pub settlement_invariants: Vec<String>,
    pub ai_judges_can_authorize_payment: bool,
}

impl Default for RiskPolicy {
    fn default() -> Self {
        Self {
            low_value_usdc_cap_minor: 10_000_000,
            max_artifact_body_bytes: 256_000,
            min_clarity_chars: 12,
            banned_terms: vec![
                "private key",
                "seed phrase",
                "password dump",
                "credential",
                "exfiltrate",
                "bypass captcha",
                "ssn",
                "social security",
                "malware",
            ],
        }
    }
}

impl RiskPolicy {
    pub fn descriptor(&self) -> RiskPolicyDescriptor {
        RiskPolicyDescriptor {
            version: RISK_POLICY_VERSION.to_string(),
            low_value_usdc_cap_minor: self.low_value_usdc_cap_minor,
            low_value_usdc_cap_currency: "usdc".to_string(),
            max_artifact_body_bytes: self.max_artifact_body_bytes,
            min_clarity_chars: self.min_clarity_chars,
            banned_terms: self
                .banned_terms
                .iter()
                .map(|term| (*term).to_string())
                .collect(),
            automatic_review_rules: vec![
                "Base USDC open-flow bounty above low_value_usdc_cap_minor requires operator review before it can become claimable.".to_string(),
                "Base USDC payout above low_value_usdc_cap_minor requires operator review before automatic release.".to_string(),
                "Private Base USDC escrow work requires operator review before automatic flows.".to_string(),
                "Artifact bodies above max_artifact_body_bytes require review before local verification.".to_string(),
                "Insecure http:// or local file:// artifact URIs require review.".to_string(),
                "Goals, context, titles, and template slugs shorter than min_clarity_chars require clarification.".to_string(),
            ],
            blocked_rules: vec![
                "Submissions from agents that do not own the active claim are blocked.".to_string(),
                "Inputs containing blocked unsafe terms are blocked from automatic flow.".to_string(),
            ],
            settlement_invariants: vec![
                "Paid bounties must be funded before claim.".to_string(),
                "Base USDC bounties become Paid, Refunded, or Disputed only after indexed escrow logs are reconciled.".to_string(),
                "Stripe ledger credits require verified webhook reconciliation.".to_string(),
                "Transaction broadcasts, hashes, planner outputs, and AI-judge decisions are not settlement.".to_string(),
            ],
            ai_judges_can_authorize_payment: false,
        }
    }

    pub fn evaluate_help_request(&self, input: &HelpRequestRiskInput) -> RiskAssessment {
        let mut assessment = RiskAssessment::allow(RiskSurface::HelpRequest);
        self.check_text_clarity(&mut assessment, &input.goal, "goal");
        self.check_text_clarity(&mut assessment, &input.context, "context");
        self.check_banned_terms(&mut assessment, &[&input.goal, &input.context]);
        if input.privacy == PrivacyLevel::Private
            && input.budget.currency == "usdc"
            && input.budget.amount > self.low_value_usdc_cap_minor
        {
            assessment.reasons.push(
                "private high-value USDC work requires operator review before funding".to_string(),
            );
            assessment.score += 30;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
        assessment
    }

    pub fn evaluate_bounty(&self, input: &BountyRiskInput) -> RiskAssessment {
        let mut assessment = RiskAssessment::allow(RiskSurface::Bounty);
        self.check_text_clarity(&mut assessment, &input.title, "title");
        self.check_text_clarity(&mut assessment, &input.template_slug, "template");
        self.check_banned_terms(&mut assessment, &[&input.title, &input.template_slug]);

        if input.funding_mode == FundingMode::BaseUsdcEscrow
            && input.amount.currency == "usdc"
            && input.amount.amount > self.low_value_usdc_cap_minor
        {
            assessment
                .reasons
                .push("Base USDC open-flow bounty exceeds low-value cap".to_string());
            assessment.score += 40;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
        if input.funding_mode == FundingMode::BaseUsdcEscrow
            && input.privacy == PrivacyLevel::Private
        {
            assessment
                .reasons
                .push("private bounty cannot use open public escrow without review".to_string());
            assessment.score += 40;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
        assessment
    }

    pub fn evaluate_submission(&self, input: &SubmissionRiskInput) -> RiskAssessment {
        let mut assessment = RiskAssessment::allow(RiskSurface::Submission);
        if input.claimed_solver_agent_id != Some(input.solver_agent_id) {
            assessment
                .reasons
                .push("submitting agent does not own the bounty claim".to_string());
            assessment.score += 100;
            assessment.action = RiskAction::Block;
        }
        if input.artifact_body.len() > self.max_artifact_body_bytes {
            assessment
                .reasons
                .push("artifact body exceeds local verification size limit".to_string());
            assessment.score += 30;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
        if input.artifact_uri.starts_with("http://") || input.artifact_uri.starts_with("file://") {
            assessment
                .reasons
                .push("artifact URI must not use insecure or local-only scheme".to_string());
            assessment.score += 20;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
        self.check_banned_terms(
            &mut assessment,
            &[&input.artifact_uri, &input.artifact_body],
        );
        assessment
    }

    pub fn evaluate_payout(&self, input: &PayoutRiskInput) -> RiskAssessment {
        let mut assessment = RiskAssessment::allow(RiskSurface::Payout);
        if input.rail == PaymentRail::BaseUsdc
            && input.amount.currency == "usdc"
            && input.amount.amount > self.low_value_usdc_cap_minor
        {
            assessment
                .reasons
                .push("Base USDC payout exceeds low-value automatic release cap".to_string());
            assessment.score += 50;
            assessment.action = RiskAction::NeedsReview;
        }
        assessment
    }

    fn check_text_clarity(&self, assessment: &mut RiskAssessment, value: &str, label: &str) {
        if value.trim().chars().count() < self.min_clarity_chars {
            assessment.reasons.push(format!(
                "{label} is too short for deterministic acceptance criteria"
            ));
            assessment.score += 10;
            assessment.action = strongest(assessment.action, RiskAction::NeedsReview);
        }
    }

    fn check_banned_terms(&self, assessment: &mut RiskAssessment, values: &[&str]) {
        let combined = values.join("\n").to_ascii_lowercase();
        for term in &self.banned_terms {
            if combined.contains(term) {
                assessment
                    .reasons
                    .push(format!("blocked unsafe term: {term}"));
                assessment.score += 100;
                assessment.action = RiskAction::Block;
            }
        }
    }
}

fn require_non_empty(errors: &mut Vec<String>, field: &str, value: &str) {
    if value.trim().is_empty() {
        errors.push(format!("{field} must not be empty"));
    }
}

fn require_https(errors: &mut Vec<String>, field: &str, value: &str) {
    if !value.starts_with("https://") {
        errors.push(format!("{field} must be an HTTPS URL"));
    }
}

fn is_hex_sha(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedArtifactUrl {
    owner: String,
    repo: String,
    commit: String,
    path: String,
}

fn sanitize_repo_name(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some(stripped) = lower.strip_suffix(".git") {
        stripped.to_string()
    } else {
        lower
    }
}

fn parse_github_repo(url: &str) -> Option<(String, String)> {
    if !url.starts_with("https://github.com/") {
        return None;
    }
    if url.contains('?') || url.contains('#') || url.contains('%') {
        return None;
    }
    let rest = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some((parts[0].to_ascii_lowercase(), sanitize_repo_name(parts[1])))
    } else {
        None
    }
}

fn parse_github_pr_repo(url: &str) -> Option<(String, String)> {
    if !url.starts_with("https://github.com/") {
        return None;
    }
    if url.contains('?') || url.contains('#') || url.contains('%') {
        return None;
    }
    let rest = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 4
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts[2] == "pull"
        && parts[3].chars().all(|c| c.is_ascii_digit())
    {
        Some((parts[0].to_ascii_lowercase(), sanitize_repo_name(parts[1])))
    } else {
        None
    }
}

fn parse_github_repo_from_any_url(url: &str) -> Option<(String, String)> {
    if !url.starts_with("https://github.com/") {
        return None;
    }
    if url.contains('?') || url.contains('#') || url.contains('%') {
        return None;
    }
    let rest = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some((parts[0].to_ascii_lowercase(), sanitize_repo_name(parts[1])))
    } else {
        None
    }
}

fn parse_immutable_artifact_url(url: &str) -> Option<ParsedArtifactUrl> {
    if !url.starts_with("https://") {
        return None;
    }

    // Reject query parameters, fragments, or percent-encoded lookalikes that could evade branch/tag checks
    if url.contains('?') || url.contains('#') || url.contains('%') {
        return None;
    }

    // Check for GitHub pinned commit SHA (raw or blob)
    // Format 1: https://github.com/{owner}/{repo}/blob/{40-hex-sha}/{path...}
    // Format 2: https://github.com/{owner}/{repo}/raw/{40-hex-sha}/{path...}
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 5
            && !parts[0].is_empty()
            && !parts[1].is_empty()
            && (parts[2] == "blob" || parts[2] == "raw")
        {
            let sha = parts[3];
            let path = parts[4..].join("/");
            if is_hex_sha(sha, 40) && !path.trim().is_empty() {
                return Some(ParsedArtifactUrl {
                    owner: parts[0].to_ascii_lowercase(),
                    repo: sanitize_repo_name(parts[1]),
                    commit: sha.to_ascii_lowercase(),
                    path,
                });
            }
        }
    }

    // Format 3: https://raw.githubusercontent.com/{owner}/{repo}/{40-hex-sha}/{path...}
    if let Some(rest) = url.strip_prefix("https://raw.githubusercontent.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 && !parts[0].is_empty() && !parts[1].is_empty() {
            let sha = parts[2];
            let path = parts[3..].join("/");
            if is_hex_sha(sha, 40) && !path.trim().is_empty() {
                return Some(ParsedArtifactUrl {
                    owner: parts[0].to_ascii_lowercase(),
                    repo: sanitize_repo_name(parts[1]),
                    commit: sha.to_ascii_lowercase(),
                    path,
                });
            }
        }
    }

    None
}

fn is_immutable_artifact_url(url: &str) -> bool {
    parse_immutable_artifact_url(url).is_some()
}

fn strongest(left: RiskAction, right: RiskAction) -> RiskAction {
    match (left, right) {
        (RiskAction::Block, _) | (_, RiskAction::Block) => RiskAction::Block,
        (RiskAction::NeedsReview, _) | (_, RiskAction::NeedsReview) => RiskAction::NeedsReview,
        _ => RiskAction::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{FundingMode, Money};
    use uuid::Uuid;

    #[test]
    fn blocks_submission_from_non_claim_owner() {
        let assessment = RiskPolicy::default().evaluate_submission(&SubmissionRiskInput {
            bounty_id: Uuid::new_v4(),
            solver_agent_id: Uuid::new_v4(),
            claimed_solver_agent_id: Some(Uuid::new_v4()),
            artifact_uri: "s3://bucket/artifact.json".to_string(),
            artifact_body: "{}".to_string(),
        });

        assert_eq!(assessment.action, RiskAction::Block);
    }

    #[test]
    fn high_value_base_bounty_requires_review() {
        let assessment = RiskPolicy::default().evaluate_bounty(&BountyRiskInput {
            title: "Fix deterministic payout reconciliation failure".to_string(),
            template_slug: "fix-ci-failure".to_string(),
            amount: Money::new(25_000_000, "usdc").unwrap(),
            funding_mode: FundingMode::BaseUsdcEscrow,
            privacy: PrivacyLevel::Public,
        });

        assert_eq!(assessment.action, RiskAction::NeedsReview);
    }

    #[test]
    fn descriptor_exposes_machine_readable_settlement_limits() {
        let descriptor = RiskPolicy::default().descriptor();

        assert_eq!(descriptor.version, RISK_POLICY_VERSION);
        assert_eq!(descriptor.low_value_usdc_cap_minor, 10_000_000);
        assert_eq!(descriptor.low_value_usdc_cap_currency, "usdc");
        assert!(!descriptor.ai_judges_can_authorize_payment);
        assert!(descriptor
            .settlement_invariants
            .iter()
            .any(|rule| rule.contains("indexed escrow logs")));
        assert!(descriptor
            .blocked_rules
            .iter()
            .any(|rule| rule.contains("active claim")));
    }

    #[test]
    fn direct_bounty_evidence_accepts_complete_pinned_https_record() {
        let report = validate_direct_bounty_evidence(&DirectBountyEvidence {
            repository_url: "https://github.com/agent-bounties/agent-bounties".to_string(),
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            subdirectory: "crates/risk".to_string(),
            pull_request_url: "https://github.com/agent-bounties/agent-bounties/pull/686"
                .to_string(),
            check_run_urls: vec![
                "https://github.com/agent-bounties/agent-bounties/actions/runs/123456789"
                    .to_string(),
            ],
            artifact_url:
                "https://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/crates/risk/tests/fixtures/report.json"
                    .to_string(),
            artifact_sha256:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            bounty_settled_url:
                "https://basescan.org/tx/0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
        });

        assert!(report.accepted, "{:?}", report.errors);
        assert!(report.errors.is_empty());
        assert!(report.payment_settlement_status.contains("unverified"));
        assert!(report
            .checklist
            .payment
            .iter()
            .any(|item| item.contains("BountySettled")));
        assert!(report.evidence_boundary.contains("Only canonical BountySettled"));
    }

    #[test]
    fn direct_bounty_evidence_rejects_missing_and_malformed_fields() {
        let report = validate_direct_bounty_evidence(&DirectBountyEvidence {
            repository_url: "".to_string(),
            source_commit: "abc".to_string(),
            subdirectory: "".to_string(),
            pull_request_url: "http://github.com/example/repo/pull/1".to_string(),
            check_run_urls: vec!["".to_string()],
            artifact_url: "https://example.com/artifacts/latest/report.json".to_string(),
            artifact_sha256: "not-a-digest".to_string(),
            bounty_settled_url: "ipfs://settled".to_string(),
        });

        assert!(!report.accepted);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("repository_url must not be empty")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("source_commit must be a full")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("pull_request_url must be an HTTPS URL")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("artifact_url must be immutable")));
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("bounty_settled_url must be an HTTPS URL")));
    }

    #[test]
    fn direct_bounty_evidence_allow_deny_table_tests() {
        // Pinned valid commit SHA raw URL -> Allowed
        assert!(is_immutable_artifact_url("https://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/report.json"));
        // Pinned valid commit SHA blob URL -> Allowed
        assert!(is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/blob/0123456789abcdef0123456789abcdef01234567/crates/risk/report.json"));
        // Pinned valid commit SHA github raw URL -> Allowed
        assert!(is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/raw/0123456789abcdef0123456789abcdef01234567/crates/risk/report.json"));

        // Mutable branch URLs -> Denied
        assert!(!is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/raw/main/report.json"));
        assert!(!is_immutable_artifact_url("https://raw.githubusercontent.com/agent-bounties/agent-bounties/master/report.json"));
        assert!(!is_immutable_artifact_url("https://raw.githubusercontent.com/agent-bounties/agent-bounties/feature-branch/report.json"));

        // Replaceable release assets and tags -> Denied
        assert!(!is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/releases/download/v1.0.0/report.json"));
        assert!(!is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/releases/tag/v1.0.0"));
        assert!(!is_immutable_artifact_url("https://github.com/agent-bounties/agent-bounties/tags/v1.0.0"));

        // Query parameters, fragments, and lookalikes -> Denied
        assert!(!is_immutable_artifact_url("https://example.com/report.json?commit=0123456789abcdef0123456789abcdef01234567"));
        assert!(!is_immutable_artifact_url("https://github.com/org/repo/raw/0123456789abcdef0123456789abcdef01234567/report.json?v=latest"));
        assert!(!is_immutable_artifact_url("https://github.com/org/repo/raw/0123456789abcdef0123456789abcdef01234567%2Freport.json"));

        // Generic unpinned URLs, IPFS, Arweave or other mutable paths -> Denied
        assert!(!is_immutable_artifact_url("https://example.com/report.json"));
        assert!(!is_immutable_artifact_url("https://example.com/ipfs/bafy"));
        assert!(!is_immutable_artifact_url("https://example.com/arweave/abcdef"));
        assert!(!is_immutable_artifact_url("https://ipfs.io/ipfs/QmT78zSuBKGvaFFB8JNjAYAkChFSFCZa17z5W33cyS2PHe/report.json"));
        assert!(!is_immutable_artifact_url("http://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/report.json"));
    }

    #[test]
    fn direct_bounty_evidence_requires_check_run_urls() {
        let report = validate_direct_bounty_evidence(&DirectBountyEvidence {
            repository_url: "https://github.com/agent-bounties/agent-bounties".to_string(),
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            subdirectory: ".".to_string(),
            pull_request_url: "https://github.com/agent-bounties/agent-bounties/pull/686"
                .to_string(),
            check_run_urls: Vec::new(),
            artifact_url:
                "https://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/crates/risk/tests/fixtures/report.json"
                    .to_string(),
            artifact_sha256:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            bounty_settled_url:
                "https://basescan.org/tx/0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
        });

        assert!(!report.accepted);
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("check_run_urls must contain at least one")));
    }
    #[test]
    fn direct_bounty_evidence_rejects_cross_binding_mismatches() {
        let base_evidence = DirectBountyEvidence {
            repository_url: "https://github.com/agent-bounties/agent-bounties".to_string(),
            source_commit: "0123456789abcdef0123456789abcdef01234567".to_string(),
            subdirectory: "crates/risk".to_string(),
            pull_request_url: "https://github.com/agent-bounties/agent-bounties/pull/686".to_string(),
            check_run_urls: vec![
                "https://github.com/agent-bounties/agent-bounties/actions/runs/123456789".to_string(),
            ],
            artifact_url: "https://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/crates/risk/tests/fixtures/report.json".to_string(),
            artifact_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            bounty_settled_url: "https://basescan.org/tx/0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        };

        // Mismatched PR repository -> Denied
        let mut pr_mismatch = base_evidence.clone();
        pr_mismatch.pull_request_url = "https://github.com/other-org/other-repo/pull/686".to_string();
        let report = validate_direct_bounty_evidence(&pr_mismatch);
        assert!(!report.accepted);
        assert!(report.errors.iter().any(|e| e.contains("pull_request_url must belong to the same repository")));

        // Mismatched artifact repository -> Denied
        let mut art_repo_mismatch = base_evidence.clone();
        art_repo_mismatch.artifact_url = "https://raw.githubusercontent.com/other-org/other-repo/0123456789abcdef0123456789abcdef01234567/report.json".to_string();
        let report = validate_direct_bounty_evidence(&art_repo_mismatch);
        assert!(!report.accepted);
        assert!(report.errors.iter().any(|e| e.contains("artifact_url repository must match repository_url")));

        // Mismatched artifact commit SHA -> Denied
        let mut art_commit_mismatch = base_evidence.clone();
        art_commit_mismatch.artifact_url = "https://raw.githubusercontent.com/agent-bounties/agent-bounties/fedcba9876543210fedcba9876543210fedcba98/crates/risk/tests/fixtures/report.json".to_string();
        let report = validate_direct_bounty_evidence(&art_commit_mismatch);
        assert!(!report.accepted);
        assert!(report.errors.iter().any(|e| e.contains("artifact_url commit SHA must match source_commit")));

        // Unrelated check-run URL -> Denied
        let mut check_mismatch = base_evidence.clone();
        check_mismatch.check_run_urls = vec!["https://github.com/unrelated-org/unrelated-repo/actions/runs/999".to_string()];
        let report = validate_direct_bounty_evidence(&check_mismatch);
        assert!(!report.accepted);
        assert!(report.errors.iter().any(|e| e.contains("check_run_urls[0] must belong to the same repository")));

        // Empty artifact path -> Denied
        let mut empty_path = base_evidence.clone();
        empty_path.artifact_url = "https://raw.githubusercontent.com/agent-bounties/agent-bounties/0123456789abcdef0123456789abcdef01234567/".to_string();
        let report = validate_direct_bounty_evidence(&empty_path);
        assert!(!report.accepted);
        assert!(report.errors.iter().any(|e| e.contains("artifact_url must specify a non-empty file path") || e.contains("artifact_url must be immutable")));
    }
}
