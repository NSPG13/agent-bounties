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
}
