use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

pub type Id = Uuid;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid amount")]
    InvalidAmount,
    #[error("invalid state transition from {from} to {to}")]
    InvalidTransition { from: String, to: String },
    #[error("bounty must be funded before it can be claimed")]
    UnfundedBounty,
    #[error("submission must be accepted before settlement can become payable")]
    UnacceptedSubmission,
    #[error("settlement is already terminal")]
    TerminalSettlement,
    #[error("proof record is required before settlement")]
    MissingProof,
}

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Money {
    pub amount: i64,
    pub currency: String,
}

impl Money {
    pub fn new(amount: i64, currency: impl Into<String>) -> DomainResult<Self> {
        if amount <= 0 {
            return Err(DomainError::InvalidAmount);
        }

        Ok(Self {
            amount,
            currency: currency.into().to_lowercase(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PrivacyLevel {
    Public,
    RedactedPublicProof,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum FundingMode {
    Simulated,
    BaseUsdcEscrow,
    StripeFiatLedger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum CapabilityClass {
    Coding,
    Research,
    Extraction,
    Verification,
    Documentation,
    Ci,
    BrowserWorkflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum VerifierKind {
    Manual,
    JsonSchema,
    DockerCommand,
    GitHubCi,
    HttpCallback,
    AiJudgeFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum AgentStatus {
    Active,
    Suspended,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Agent {
    pub id: Id,
    pub handle: String,
    pub status: AgentStatus,
    pub payout_wallet: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Agent {
    pub fn new(handle: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            handle: handle.into(),
            status: AgentStatus::Active,
            payout_wallet: None,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Capability {
    pub id: Id,
    pub agent_id: Id,
    pub class: CapabilityClass,
    pub template_slugs: Vec<String>,
    pub min_price: Money,
    pub max_price: Money,
    pub latency_seconds: u64,
    pub supported_verifiers: Vec<VerifierKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HelpRequest {
    pub id: Id,
    pub requester_agent_id: Id,
    pub goal: String,
    pub context: String,
    pub budget: Money,
    pub deadline: Option<DateTime<Utc>>,
    pub privacy: PrivacyLevel,
    pub required_confidence: f32,
    pub created_at: DateTime<Utc>,
}

impl HelpRequest {
    pub fn new(
        requester_agent_id: Id,
        goal: impl Into<String>,
        context: impl Into<String>,
        budget: Money,
        privacy: PrivacyLevel,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            requester_agent_id,
            goal: goal.into(),
            context: context.into(),
            budget,
            deadline: None,
            privacy,
            required_confidence: 0.8,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Quote {
    pub id: Id,
    pub help_request_id: Id,
    pub solver_agent_id: Id,
    pub price: Money,
    pub estimated_seconds: u64,
    pub verifier_kind: VerifierKind,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum BountyStatus {
    Unfunded,
    Funded,
    Claimable,
    Claimed,
    Submitted,
    Verifying,
    Accepted,
    Payable,
    Paid,
    Refunding,
    Refunded,
    Disputed,
    Expired,
}

impl BountyStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Paid | Self::Refunded | Self::Expired)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Bounty {
    pub id: Id,
    pub help_request_id: Option<Id>,
    pub title: String,
    pub template_slug: String,
    pub amount: Money,
    pub funding_mode: FundingMode,
    pub privacy: PrivacyLevel,
    pub status: BountyStatus,
    pub terms_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Bounty {
    pub fn new(
        title: impl Into<String>,
        template_slug: impl Into<String>,
        amount: Money,
        funding_mode: FundingMode,
        privacy: PrivacyLevel,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            help_request_id: None,
            title: title.into(),
            template_slug: template_slug.into(),
            amount,
            funding_mode,
            privacy,
            status: BountyStatus::Unfunded,
            terms_hash: None,
            created_at: Utc::now(),
        }
    }

    pub fn mark_funded(&mut self, terms_hash: impl Into<String>) -> DomainResult<()> {
        self.transition(BountyStatus::Funded)?;
        self.terms_hash = Some(terms_hash.into());
        Ok(())
    }

    pub fn make_claimable(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Claimable)
    }

    pub fn claim(&mut self) -> DomainResult<()> {
        if self.status == BountyStatus::Unfunded {
            return Err(DomainError::UnfundedBounty);
        }
        self.transition(BountyStatus::Claimed)
    }

    pub fn submit(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Submitted)
    }

    pub fn start_verification(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Verifying)
    }

    pub fn accept(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Accepted)
    }

    pub fn make_payable(&mut self, proof: &ProofRecord) -> DomainResult<()> {
        if proof.bounty_id != self.id {
            return Err(DomainError::MissingProof);
        }
        if self.status != BountyStatus::Accepted {
            return Err(DomainError::UnacceptedSubmission);
        }
        self.transition(BountyStatus::Payable)
    }

    pub fn mark_paid(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Paid)
    }

    pub fn refunding(&mut self) -> DomainResult<()> {
        if matches!(
            self.status,
            BountyStatus::Funded
                | BountyStatus::Claimable
                | BountyStatus::Claimed
                | BountyStatus::Submitted
                | BountyStatus::Disputed
        ) {
            self.status = BountyStatus::Refunding;
            return Ok(());
        }
        Err(DomainError::InvalidTransition {
            from: format!("{:?}", self.status),
            to: "Refunding".to_string(),
        })
    }

    pub fn mark_refunded(&mut self) -> DomainResult<()> {
        self.transition(BountyStatus::Refunded)
    }

    pub fn dispute(&mut self) -> DomainResult<()> {
        if matches!(
            self.status,
            BountyStatus::Submitted | BountyStatus::Verifying
        ) {
            self.status = BountyStatus::Disputed;
            return Ok(());
        }
        Err(DomainError::InvalidTransition {
            from: format!("{:?}", self.status),
            to: "Disputed".to_string(),
        })
    }

    fn transition(&mut self, to: BountyStatus) -> DomainResult<()> {
        if self.status.is_terminal() {
            return Err(DomainError::TerminalSettlement);
        }

        let allowed = matches!(
            (&self.status, &to),
            (BountyStatus::Unfunded, BountyStatus::Funded)
                | (BountyStatus::Funded, BountyStatus::Claimable)
                | (BountyStatus::Claimable, BountyStatus::Claimed)
                | (BountyStatus::Claimed, BountyStatus::Submitted)
                | (BountyStatus::Submitted, BountyStatus::Verifying)
                | (BountyStatus::Verifying, BountyStatus::Accepted)
                | (BountyStatus::Disputed, BountyStatus::Accepted)
                | (BountyStatus::Accepted, BountyStatus::Payable)
                | (BountyStatus::Payable, BountyStatus::Paid)
                | (BountyStatus::Refunding, BountyStatus::Refunded)
        );

        if !allowed {
            return Err(DomainError::InvalidTransition {
                from: format!("{:?}", self.status),
                to: format!("{:?}", to),
            });
        }

        self.status = to;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Claim {
    pub id: Id,
    pub bounty_id: Id,
    pub solver_agent_id: Id,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Submission {
    pub id: Id,
    pub bounty_id: Id,
    pub solver_agent_id: Id,
    pub artifact_digest: String,
    pub artifact_uri: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum VerificationDecision {
    Accepted,
    Rejected,
    NeedsReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VerifierResult {
    pub id: Id,
    pub bounty_id: Id,
    pub submission_id: Id,
    pub verifier_agent_id: Option<Id>,
    pub kind: VerifierKind,
    pub decision: VerificationDecision,
    pub summary: String,
    pub confidence: f32,
    pub signed_payload_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProofRecord {
    pub id: Id,
    pub bounty_id: Id,
    pub submission_id: Id,
    pub verifier_result_id: Id,
    pub proof_hash: String,
    pub public_summary: String,
    pub privacy: PrivacyLevel,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReputationEvent {
    pub id: Id,
    pub agent_id: Id,
    pub bounty_id: Id,
    pub capability_class: CapabilityClass,
    pub template_slug: String,
    pub delta: i32,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TemplateSignal {
    pub id: Id,
    pub bounty_id: Id,
    pub proof_record_id: Id,
    pub template_slug: String,
    pub capability_class: CapabilityClass,
    pub verifier_kind: VerifierKind,
    pub amount: Money,
    pub success: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RiskSurface {
    HelpRequest,
    Bounty,
    Submission,
    Verification,
    Payout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RiskAction {
    Allow,
    NeedsReview,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RiskEvent {
    pub id: Id,
    pub subject_id: Id,
    pub agent_id: Option<Id>,
    pub bounty_id: Option<Id>,
    pub surface: RiskSurface,
    pub action: RiskAction,
    pub score: u16,
    pub reasons: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RiskReviewOutcome {
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RiskReviewRecord {
    pub id: Id,
    pub risk_event_id: Id,
    pub subject_id: Id,
    pub bounty_id: Option<Id>,
    pub surface: RiskSurface,
    pub outcome: RiskReviewOutcome,
    pub operator_id: String,
    pub note: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PaymentRail {
    Simulated,
    BaseUsdc,
    StripeFiat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PaymentEventStatus {
    Received,
    Applied,
    IgnoredDuplicate,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FundingIntent {
    pub id: Id,
    pub bounty_id: Id,
    pub rail: PaymentRail,
    pub amount: Money,
    pub external_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum FundingContributionStatus {
    Applied,
    Refunded,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FundingContribution {
    pub id: Id,
    pub bounty_id: Id,
    pub contributor_agent_id: Option<Id>,
    pub source_organization_id: Option<Id>,
    pub rail: PaymentRail,
    pub amount: Money,
    pub status: FundingContributionStatus,
    pub funding_ledger_entry_id: Option<Id>,
    pub refund_ledger_entry_id: Option<Id>,
    pub settlement_id: Option<Id>,
    pub external_reference: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum EscrowStatus {
    Created,
    Funded,
    Disputed,
    Released,
    Refunded,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Escrow {
    pub id: Id,
    pub bounty_id: Id,
    pub rail: PaymentRail,
    pub token: String,
    pub amount: Money,
    pub status: EscrowStatus,
    pub external_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PayoutStatus {
    Pending,
    Blocked,
    Paying,
    Paid,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PayoutIntent {
    pub id: Id,
    pub bounty_id: Id,
    pub recipient_agent_id: Id,
    pub rail: PaymentRail,
    pub amount: Money,
    pub status: PayoutStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Settlement {
    pub id: Id,
    pub bounty_id: Id,
    pub proof_record_id: Id,
    pub rail: PaymentRail,
    pub payout_intents: Vec<PayoutIntent>,
    pub platform_fee: Money,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaymentEvent {
    pub id: Id,
    pub rail: PaymentRail,
    pub external_id: String,
    pub status: PaymentEventStatus,
    pub payload_hash: String,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EvalRun {
    pub id: Id,
    pub suite: String,
    pub score: f32,
    pub passed: bool,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounty_must_be_funded_before_claim() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );

        let err = bounty.claim().unwrap_err();
        assert_eq!(err, DomainError::UnfundedBounty);
    }

    #[test]
    fn happy_path_requires_proof_before_payable() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );
        bounty.mark_funded("terms").unwrap();
        bounty.make_claimable().unwrap();
        bounty.claim().unwrap();
        bounty.submit().unwrap();
        bounty.start_verification().unwrap();
        bounty.accept().unwrap();

        let proof = ProofRecord {
            id: Uuid::new_v4(),
            bounty_id: bounty.id,
            submission_id: Uuid::new_v4(),
            verifier_result_id: Uuid::new_v4(),
            proof_hash: "proof".to_string(),
            public_summary: "accepted".to_string(),
            privacy: PrivacyLevel::Public,
            created_at: Utc::now(),
        };

        bounty.make_payable(&proof).unwrap();
        bounty.mark_paid().unwrap();
        assert_eq!(bounty.status, BountyStatus::Paid);
    }

    #[test]
    fn cannot_skip_to_paid() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );
        bounty.mark_funded("terms").unwrap();
        assert!(bounty.mark_paid().is_err());
    }

    #[test]
    fn disputed_bounty_can_enter_refund_path() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );
        bounty.mark_funded("terms").unwrap();
        bounty.make_claimable().unwrap();
        bounty.claim().unwrap();
        bounty.submit().unwrap();
        bounty.dispute().unwrap();

        bounty.refunding().unwrap();
        bounty.mark_refunded().unwrap();

        assert_eq!(bounty.status, BountyStatus::Refunded);
    }
}
