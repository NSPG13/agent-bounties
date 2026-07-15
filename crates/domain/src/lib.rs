use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

pub mod objective;
pub use objective::*;

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

    pub fn zero(currency: impl Into<String>) -> Self {
        Self {
            amount: 0,
            currency: currency.into().to_lowercase(),
        }
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
    MixedRails,
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

pub const AUTONOMOUS_BOUNTY_PROTOCOL_VERSION: &str = "agent-bounties/autonomous-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutonomousBountyTermsDocument {
    pub schema_version: String,
    pub contract_terms: Value,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub benchmark: Value,
    pub evidence_schema: Value,
    pub verification_policy: Value,
    pub source_url: Option<String>,
    pub discovery_source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutonomousBountyTermsRecord {
    pub terms_hash: String,
    pub policy_hash: String,
    pub acceptance_criteria_hash: String,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub creator_wallet: String,
    pub document: AutonomousBountyTermsDocument,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutonomousSubmissionEvidenceRecord {
    pub network: String,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: u64,
    pub solver_wallet: String,
    pub artifact_reference: String,
    pub artifact_hash: String,
    pub evidence: Value,
    pub evidence_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum VerificationMechanism {
    DeterministicModule,
    SignedQuorum,
    AiJudgeQuorum,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum VerificationEngine {
    JsonSchema,
    DockerCommand,
    GitHubCi,
    HttpCallback,
    AiJudge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AiJudgePolicyCommitment {
    pub provider: String,
    pub model: String,
    pub model_version: String,
    pub system_prompt_hash: String,
    pub rubric_hash: String,
    pub benchmark_hash: String,
    pub decoding_parameters_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AutomaticVerificationPolicy {
    pub protocol_version: String,
    pub mechanism: VerificationMechanism,
    pub engine: VerificationEngine,
    pub terms_hash: String,
    pub policy_hash: String,
    pub acceptance_criteria_hash: String,
    pub benchmark_hash: String,
    pub evidence_schema_hash: String,
    pub verifier_set_hash: Option<String>,
    pub verifier_count: u8,
    pub threshold: u8,
    pub max_automatic_payout: Money,
    pub ai_judge: Option<AiJudgePolicyCommitment>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerificationPolicyError {
    #[error("unsupported autonomous bounty protocol version")]
    UnsupportedVersion,
    #[error("verification policy contains an invalid hash commitment")]
    InvalidHash,
    #[error("automatic settlement payout must be positive USDC")]
    InvalidAutomaticPayout,
    #[error("verification quorum must contain one to eight verifiers and meet threshold")]
    InvalidQuorum,
    #[error("deterministic module policy must use a one-of-one verifier and no AI commitment")]
    InvalidDeterministicPolicy,
    #[error("signed quorum policy requires a verifier-set commitment and no AI commitment")]
    InvalidSignedQuorumPolicy,
    #[error("AI judge settlement requires a committed model policy and at least two verifier signatures")]
    InvalidAiJudgePolicy,
}

impl AutomaticVerificationPolicy {
    pub fn validate(&self) -> Result<(), VerificationPolicyError> {
        if self.protocol_version != AUTONOMOUS_BOUNTY_PROTOCOL_VERSION {
            return Err(VerificationPolicyError::UnsupportedVersion);
        }
        if !is_bytes32_hash(&self.terms_hash)
            || !is_bytes32_hash(&self.policy_hash)
            || !is_bytes32_hash(&self.acceptance_criteria_hash)
            || !is_bytes32_hash(&self.benchmark_hash)
            || !is_bytes32_hash(&self.evidence_schema_hash)
            || self
                .verifier_set_hash
                .as_ref()
                .is_some_and(|value| !is_bytes32_hash(value))
        {
            return Err(VerificationPolicyError::InvalidHash);
        }
        if self.max_automatic_payout.amount <= 0
            || !self
                .max_automatic_payout
                .currency
                .eq_ignore_ascii_case("usdc")
        {
            return Err(VerificationPolicyError::InvalidAutomaticPayout);
        }
        if self.verifier_count == 0
            || self.verifier_count > 8
            || self.threshold == 0
            || self.threshold > self.verifier_count
        {
            return Err(VerificationPolicyError::InvalidQuorum);
        }

        match self.mechanism {
            VerificationMechanism::DeterministicModule => {
                if self.threshold != 1
                    || self.verifier_count != 1
                    || self.ai_judge.is_some()
                    || self.engine == VerificationEngine::AiJudge
                {
                    return Err(VerificationPolicyError::InvalidDeterministicPolicy);
                }
            }
            VerificationMechanism::SignedQuorum => {
                if self.verifier_set_hash.is_none()
                    || self.ai_judge.is_some()
                    || self.engine == VerificationEngine::AiJudge
                {
                    return Err(VerificationPolicyError::InvalidSignedQuorumPolicy);
                }
            }
            VerificationMechanism::AiJudgeQuorum => {
                let Some(ai_judge) = self.ai_judge.as_ref() else {
                    return Err(VerificationPolicyError::InvalidAiJudgePolicy);
                };
                if self.engine != VerificationEngine::AiJudge
                    || self.verifier_set_hash.is_none()
                    || self.threshold < 2
                    || ai_judge.provider.trim().is_empty()
                    || ai_judge.model.trim().is_empty()
                    || ai_judge.model_version.trim().is_empty()
                    || !is_bytes32_hash(&ai_judge.system_prompt_hash)
                    || !is_bytes32_hash(&ai_judge.rubric_hash)
                    || !is_bytes32_hash(&ai_judge.benchmark_hash)
                    || !is_bytes32_hash(&ai_judge.decoding_parameters_hash)
                    || ai_judge.benchmark_hash != self.benchmark_hash
                {
                    return Err(VerificationPolicyError::InvalidAiJudgePolicy);
                }
            }
        }
        Ok(())
    }

    pub fn permits_automatic_settlement(&self) -> bool {
        self.validate().is_ok()
    }
}

fn is_bytes32_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
        && value[2..].bytes().any(|byte| byte != b'0')
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
pub struct ContributorContact {
    pub id: Id,
    pub github_login: String,
    pub email: Option<String>,
    pub payout_wallet: Option<String>,
    pub associated_prs: Vec<String>,
    pub contact_consent: bool,
    pub wallet_consent: bool,
    pub outreach_allowed: bool,
    pub source: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ContributorContact {
    pub fn new(github_login: impl Into<String>, source: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            github_login: github_login.into(),
            email: None,
            payout_wallet: None,
            associated_prs: Vec::new(),
            contact_consent: false,
            wallet_consent: false,
            outreach_allowed: false,
            source: source.into(),
            notes: None,
            created_at: now,
            updated_at: now,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceProvider {
    Github,
    HostedApi,
    Mcp,
    BaseWallet,
    Stripe,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceRole {
    Observer,
    Contributor,
    BountyPoster,
    ProspectiveFunder,
    Funder,
    ProspectiveSolver,
    Claimer,
    Solver,
    Verifier,
    Recipient,
    Promoter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceLifecycleStage {
    Observed,
    Engaged,
    Converted,
    Retained,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudienceMember {
    pub id: Id,
    pub provider: AudienceProvider,
    pub external_id: String,
    pub handle: String,
    pub public_profile_url: Option<String>,
    pub roles: Vec<AudienceRole>,
    pub lifecycle_stage: AudienceLifecycleStage,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceInteractionKind {
    IssueOpened,
    PullRequestOpened,
    IssueCommented,
    PullRequestReviewed,
    BountyPosted,
    FundingSignaled,
    BountyFunded,
    ClaimSignaled,
    BountyClaimed,
    SubmissionMade,
    SubmissionAccepted,
    VerificationSubmitted,
    PayoutReceived,
    RepoStarred,
    BountyUpvoted,
    ProofShared,
    ReferralCreated,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudienceInteraction {
    pub id: Id,
    pub audience_member_id: Id,
    pub provider_event_id: String,
    pub kind: AudienceInteractionKind,
    pub public_url: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub referrer_url: Option<String>,
    pub campaign: Option<String>,
    pub source_interaction_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DiscoveryResponse {
    pub id: Id,
    pub audience_member_id: Id,
    pub interaction_id: Option<Id>,
    pub provider_response_id: String,
    pub public_source_url: Option<String>,
    pub found_via: String,
    pub motivation: String,
    pub improvement_suggestion: String,
    pub agent_or_tool: Option<String>,
    pub private_storage_consent: bool,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutreachChannel {
    GithubPublic,
    OtherPublic,
    EmailPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutreachStatus {
    Pending,
    Responded,
    Declined,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OutreachAttempt {
    pub id: Id,
    pub audience_member_id: Id,
    pub provider_event_id: String,
    pub channel: OutreachChannel,
    pub public_url: Option<String>,
    pub prompt_version: String,
    pub status: OutreachStatus,
    pub consent_contact_id: Option<Id>,
    pub sent_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudienceMetric {
    pub key: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AudienceReport {
    pub total_members: u64,
    pub total_interactions: u64,
    pub members_asked_for_discovery_feedback: u64,
    pub members_with_discovery_responses: u64,
    pub repeat_participants: u64,
    pub external_bounty_posters: u64,
    pub external_funders: u64,
    pub external_solvers: u64,
    pub paid_participants: u64,
    pub repo_stars_attributed: u64,
    pub shares_attributed: u64,
    pub not_asked_or_answered_handles: Vec<String>,
    pub asked_without_response_handles: Vec<String>,
    pub interactions_by_kind: Vec<AudienceMetric>,
    pub generated_at: DateTime<Utc>,
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
    #[serde(default)]
    pub funding_targets: Vec<FundingPartitionTarget>,
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
            funding_targets: Vec::new(),
            funding_mode,
            privacy,
            status: BountyStatus::Unfunded,
            terms_hash: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_funding_targets(mut self, funding_targets: Vec<FundingPartitionTarget>) -> Self {
        self.funding_targets = funding_targets;
        self
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

    pub fn reopen_for_funding(&mut self) -> DomainResult<()> {
        if matches!(
            self.status,
            BountyStatus::Unfunded | BountyStatus::Funded | BountyStatus::Claimable
        ) {
            self.status = BountyStatus::Unfunded;
            return Ok(());
        }
        Err(DomainError::InvalidTransition {
            from: format!("{:?}", self.status),
            to: "Unfunded".to_string(),
        })
    }

    pub fn mark_payment_disputed(&mut self) -> DomainResult<()> {
        if matches!(
            self.status,
            BountyStatus::Claimed
                | BountyStatus::Submitted
                | BountyStatus::Verifying
                | BountyStatus::Disputed
        ) {
            self.status = BountyStatus::Disputed;
            return Ok(());
        }
        Err(DomainError::InvalidTransition {
            from: format!("{:?}", self.status),
            to: "Disputed".to_string(),
        })
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
                to: format!("{to:?}"),
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
pub struct FundingPartitionTarget {
    pub rail: PaymentRail,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PaymentEventStatus {
    Received,
    Applied,
    IgnoredDuplicate,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum FundingIntentStatus {
    AwaitingEvidence,
    Applied,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FundingIntent {
    pub id: Id,
    pub bounty_id: Id,
    pub contributor_agent_id: Option<Id>,
    pub source_organization_id: Option<Id>,
    pub rail: PaymentRail,
    pub amount: Money,
    pub status: FundingIntentStatus,
    pub external_reference: Option<String>,
    #[serde(default)]
    pub stripe_success_url: Option<String>,
    #[serde(default)]
    pub stripe_cancel_url: Option<String>,
    pub created_at: DateTime<Utc>,
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
    fn zero_money_is_explicit_and_does_not_allow_zero_value_bounties() {
        assert_eq!(
            Money::zero("USDC"),
            Money {
                amount: 0,
                currency: "usdc".to_string()
            }
        );
        assert_eq!(Money::new(0, "usdc"), Err(DomainError::InvalidAmount));
    }

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

    #[test]
    fn funded_bounty_can_reopen_after_partition_refund_before_claim() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::MixedRails,
            PrivacyLevel::Public,
        );
        bounty.mark_funded("terms").unwrap();
        bounty.make_claimable().unwrap();

        bounty.reopen_for_funding().unwrap();

        assert_eq!(bounty.status, BountyStatus::Unfunded);
    }

    #[test]
    fn claimed_bounty_can_be_marked_payment_disputed() {
        let mut bounty = Bounty::new(
            "Fix CI",
            "fix-ci",
            Money::new(1_000_000, "usdc").unwrap(),
            FundingMode::MixedRails,
            PrivacyLevel::Public,
        );
        bounty.mark_funded("terms").unwrap();
        bounty.make_claimable().unwrap();
        bounty.claim().unwrap();

        bounty.mark_payment_disputed().unwrap();

        assert_eq!(bounty.status, BountyStatus::Disputed);
    }

    #[test]
    fn deterministic_policy_allows_one_committed_verifier() {
        let policy = deterministic_policy();

        assert_eq!(policy.validate(), Ok(()));
        assert!(policy.permits_automatic_settlement());
    }

    #[test]
    fn ai_judge_requires_model_benchmark_and_two_signature_quorum() {
        let mut policy = ai_judge_policy();
        assert_eq!(policy.validate(), Ok(()));

        policy.threshold = 1;
        assert_eq!(
            policy.validate(),
            Err(VerificationPolicyError::InvalidAiJudgePolicy)
        );

        policy.threshold = 2;
        policy.ai_judge.as_mut().unwrap().benchmark_hash = "0x00".to_string();
        assert_eq!(
            policy.validate(),
            Err(VerificationPolicyError::InvalidAiJudgePolicy)
        );
    }

    #[test]
    fn policy_rejects_non_usdc_or_uncommitted_hashes() {
        let mut policy = deterministic_policy();
        policy.max_automatic_payout = Money::new(100, "usd").unwrap();
        assert_eq!(
            policy.validate(),
            Err(VerificationPolicyError::InvalidAutomaticPayout)
        );

        policy.max_automatic_payout = Money::new(100, "usdc").unwrap();
        policy.policy_hash = format!("0x{}", "0".repeat(64));
        assert_eq!(policy.validate(), Err(VerificationPolicyError::InvalidHash));
    }

    fn deterministic_policy() -> AutomaticVerificationPolicy {
        AutomaticVerificationPolicy {
            protocol_version: AUTONOMOUS_BOUNTY_PROTOCOL_VERSION.to_string(),
            mechanism: VerificationMechanism::DeterministicModule,
            engine: VerificationEngine::DockerCommand,
            terms_hash: test_hash('1'),
            policy_hash: test_hash('2'),
            acceptance_criteria_hash: test_hash('3'),
            benchmark_hash: test_hash('5'),
            evidence_schema_hash: test_hash('4'),
            verifier_set_hash: None,
            verifier_count: 1,
            threshold: 1,
            max_automatic_payout: Money::new(1_000_000, "usdc").unwrap(),
            ai_judge: None,
        }
    }

    fn ai_judge_policy() -> AutomaticVerificationPolicy {
        AutomaticVerificationPolicy {
            protocol_version: AUTONOMOUS_BOUNTY_PROTOCOL_VERSION.to_string(),
            mechanism: VerificationMechanism::AiJudgeQuorum,
            engine: VerificationEngine::AiJudge,
            terms_hash: test_hash('1'),
            policy_hash: test_hash('2'),
            acceptance_criteria_hash: test_hash('3'),
            benchmark_hash: test_hash('8'),
            evidence_schema_hash: test_hash('4'),
            verifier_set_hash: Some(test_hash('5')),
            verifier_count: 3,
            threshold: 2,
            max_automatic_payout: Money::new(1_000_000, "usdc").unwrap(),
            ai_judge: Some(AiJudgePolicyCommitment {
                provider: "provider".to_string(),
                model: "judge-model".to_string(),
                model_version: "2026-07-10".to_string(),
                system_prompt_hash: test_hash('6'),
                rubric_hash: test_hash('7'),
                benchmark_hash: test_hash('8'),
                decoding_parameters_hash: test_hash('9'),
            }),
        }
    }

    fn test_hash(character: char) -> String {
        format!("0x{}", character.to_string().repeat(64))
    }
}
