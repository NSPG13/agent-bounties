use alloy::primitives::{keccak256, Address, Signature, B256};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{Id, Money};

pub const OBJECTIVE_SCHEMA_VERSION: &str = "agent-bounties/objective-v1";
const PERSONAL_SIGN_PREFIX: &[u8] = b"\x19Ethereum Signed Message:\n32";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ObjectiveError {
    #[error("{0} is required")]
    Required(&'static str),
    #[error("participant {0} is unknown")]
    UnknownParticipant(Id),
    #[error("participant {0} is duplicated")]
    DuplicateParticipant(Id),
    #[error("participant {0} requires a valid EVM wallet")]
    InvalidParticipantWallet(Id),
    #[error("objective authority is invalid: {0}")]
    InvalidAuthority(String),
    #[error("verification policy is invalid: {0}")]
    InvalidVerificationPolicy(String),
    #[error("access policy is invalid: {0}")]
    InvalidAccessPolicy(String),
    #[error("proposal {0} was not found")]
    ProposalNotFound(Id),
    #[error("contribution need {0} was not found")]
    ContributionNeedNotFound(Id),
    #[error("contribution offer {0} was not found")]
    ContributionOfferNotFound(Id),
    #[error("objective action is invalid in state {0:?}: {1}")]
    InvalidAction(ObjectiveStatus, String),
    #[error("proposal is no longer valid")]
    ProposalExpired,
    #[error("a provider proposal has already been accepted")]
    ProposalAlreadyAccepted,
    #[error("required contribution dependencies must form a directed acyclic graph")]
    CyclicDependency,
    #[error("required contribution dependency {0} is unknown")]
    UnknownDependency(Id),
    #[error("wallet approval is invalid: {0}")]
    InvalidApproval(String),
    #[error("wallet approval threshold was not met")]
    ApprovalThresholdNotMet,
    #[error("signed action does not match the current objective revision")]
    StaleAction,
    #[error("canonical bounty evidence does not match the committed binding")]
    CanonicalEvidenceMismatch,
    #[error("objective is not ready for final execution: {0}")]
    NotReady(String),
    #[error("unsupported privacy claim: {0}")]
    UnsupportedPrivacy(String),
    #[error("accepted value bundles are immutable; amendments are unavailable in objective-v1")]
    AmendmentsUnavailable,
    #[error("serialization failed while computing a commitment")]
    CommitmentSerialization,
}

pub type ObjectiveResult<T> = Result<T, ObjectiveError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantKind {
    Person,
    Agent,
    Organization,
    Company,
    PublicInstitution,
    Team,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentityDisclosure {
    Public,
    Pseudonymous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveParticipant {
    pub id: Id,
    pub kind: ParticipantKind,
    pub display_name: String,
    pub wallet: String,
    pub identity_disclosure: IdentityDisclosure,
    pub public_identity_reference: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedEffect {
    Positive,
    Negative,
    Mixed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AffectedPartyDeclaration {
    pub participant_id: Id,
    pub expected_effect: ExpectedEffect,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveAuthorityKind {
    SingleWallet,
    OrganizationWallet,
    WalletQuorum,
    DesignatedRepresentatives,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveAuthority {
    pub kind: ObjectiveAuthorityKind,
    pub member_ids: Vec<Id>,
    pub threshold: u16,
    pub public_statement: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Money,
    Work,
    Information,
    Research,
    Design,
    Software,
    Equipment,
    Access,
    Service,
    Verification,
    Organization,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ResourceDeclaration {
    pub id: Id,
    pub kind: ResourceKind,
    pub provider_id: Id,
    pub description: String,
    pub declared_quantity: Option<String>,
    pub monetary_value: Option<Money>,
    pub evidence_reference: Option<String>,
    pub evidence_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAccessEnforcement {
    NoneRequired,
    ExternalCustodian,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliverableAccessPolicy {
    Public,
    RequestingPartyOnly {
        custodian_id: Id,
        enforcement: ExternalAccessEnforcement,
    },
    NamedRecipients {
        recipient_ids: Vec<Id>,
        custodian_id: Id,
        enforcement: ExternalAccessEnforcement,
    },
    QualifyingFunders {
        custodian_id: Id,
        enforcement: ExternalAccessEnforcement,
    },
    TimeDelayedPublic {
        public_at: DateTime<Utc>,
        custodian_id: Id,
        enforcement: ExternalAccessEnforcement,
    },
    PublicSummaryRestrictedDeliverable {
        recipient_ids: Vec<Id>,
        custodian_id: Id,
        enforcement: ExternalAccessEnforcement,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct RightsPolicy {
    pub owner_ids: Vec<Id>,
    pub license_or_terms: String,
    pub restrictions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PublicEvidencePolicy {
    Public,
    RedactedPublicReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectivePrivacyDeclaration {
    pub blockchain_information_is_public: bool,
    pub evidence_policy: PublicEvidencePolicy,
    pub redaction_limits: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalBountyBinding {
    pub network: String,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub terms_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalPaymentRequirement {
    pub amount: Money,
    pub atomic_amount: u64,
    pub decimals: u8,
    pub bounty: CanonicalBountyBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AiJudgeCommitment {
    pub provider: String,
    pub model: String,
    pub model_version: String,
    pub system_prompt_hash: String,
    pub rubric_hash: String,
    pub benchmark_hash: String,
    pub decoding_parameters_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectiveVerificationMechanism {
    DeterministicSigner {
        verifier_id: Id,
        module_reference: String,
    },
    CommittedVerifier {
        verifier_id: Id,
    },
    WalletQuorum {
        verifier_ids: Vec<Id>,
        threshold: u16,
    },
    AiJudgeQuorum {
        verifier_ids: Vec<Id>,
        threshold: u16,
        commitment: AiJudgeCommitment,
    },
    ProviderAcceptance {
        provider_id: Id,
    },
    ObjectiveAuthority,
    CanonicalBounty {
        bounty: CanonicalBountyBinding,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveVerificationPolicy {
    pub mechanism: ObjectiveVerificationMechanism,
    pub acceptance_criteria: Vec<String>,
    pub evidence_schema: String,
    pub evidence_schema_hash: String,
    pub trust_assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContributionCompensation {
    InKind,
    Paid {
        payment: CanonicalPaymentRequirement,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContributionNeedDraft {
    pub id: Id,
    pub title: String,
    pub deliverable: String,
    pub purpose: String,
    pub recipient_ids: Vec<Id>,
    pub verification_policy: ObjectiveVerificationPolicy,
    pub mandatory: bool,
    pub deadline: DateTime<Utc>,
    pub access_policy: DeliverableAccessPolicy,
    pub rights_policy: RightsPolicy,
    pub compensation: ContributionCompensation,
    #[serde(default)]
    pub depends_on: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderProposalDraft {
    pub id: Id,
    pub provider_id: Id,
    pub outcome_commitment: String,
    pub monetary_payment: Option<CanonicalPaymentRequirement>,
    pub contribution_needs: Vec<ContributionNeedDraft>,
    pub delivery_deadline: DateTime<Utc>,
    pub final_verification_policy: ObjectiveVerificationPolicy,
    pub access_policy: DeliverableAccessPolicy,
    pub rights_policy: RightsPolicy,
    pub valid_until: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProviderProposal {
    pub draft: ProviderProposalDraft,
    pub terms_hash: String,
    pub provider_approval: ApprovalRecord,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BundleAmendmentPolicy {
    DisabledInObjectiveV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AcceptedValueBundle {
    pub version: u32,
    pub proposal_id: Id,
    pub provider_id: Id,
    pub terms_hash: String,
    pub outcome_commitment: String,
    pub monetary_payment: Option<CanonicalPaymentRequirement>,
    pub contribution_needs: Vec<ContributionNeedDraft>,
    pub delivery_deadline: DateTime<Utc>,
    pub final_verification_policy: ObjectiveVerificationPolicy,
    pub access_policy: DeliverableAccessPolicy,
    pub rights_policy: RightsPolicy,
    pub authority_approval: ApprovalRecord,
    pub amendment_policy: BundleAmendmentPolicy,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContributionWorkState {
    Offered,
    Selected,
    Submitted,
    Verified,
    Rejected,
    Withdrawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContributionCompensationState {
    InKind,
    PaymentPending,
    PaidCanonical {
        settlement: Box<CanonicalSettlementEvidence>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContributionOfferDraft {
    pub id: Id,
    pub need_id: Id,
    pub contributor_id: Id,
    pub role: ContributionRole,
    pub deliverable_commitment: String,
    pub expected_delivery_at: DateTime<Utc>,
    pub evidence_commitment: String,
    pub expects_monetary_compensation: bool,
    pub conditions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContributionSubmission {
    pub artifact_reference: String,
    pub artifact_hash: String,
    pub evidence_reference: String,
    pub evidence_hash: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VerificationStatement {
    pub passed: bool,
    pub evidence_reference: String,
    pub evidence_hash: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VerificationRecord {
    pub policy_hash: String,
    pub statement: VerificationStatement,
    pub approvals: ApprovalRecord,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ContributionOffer {
    pub draft: ContributionOfferDraft,
    pub state: ContributionWorkState,
    pub compensation_state: ContributionCompensationState,
    pub offer_approval: ApprovalRecord,
    pub selection_approval: Option<ApprovalRecord>,
    pub submission: Option<ContributionSubmission>,
    pub verification: Option<VerificationRecord>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContributionRole {
    Provider,
    Solver,
    Verifier,
    Organizer,
    Funder,
    Researcher,
    Designer,
    Developer,
    Contributor,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContributionRecordCompensation {
    InKind,
    PaymentPending,
    PaidCanonical {
        settlement: Box<CanonicalSettlementEvidence>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VerifiedContributionRecord {
    pub id: Id,
    pub contributor_id: Id,
    pub objective_id: Id,
    pub contribution_need_id: Id,
    pub contribution_offer_id: Id,
    pub role: ContributionRole,
    pub capability: String,
    pub beneficiary_categories: Vec<String>,
    pub evidence_reference: String,
    pub evidence_hash: String,
    pub verification_mechanism: String,
    pub verification_strength: String,
    pub compensation: ContributionRecordCompensation,
    pub completed_at: DateTime<Utc>,
    pub transferable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct VerifiedFinalOutcomeRecord {
    pub id: Id,
    pub provider_id: Id,
    pub objective_id: Id,
    pub role: ContributionRole,
    pub outcome_commitment: String,
    pub beneficiary_ids: Vec<Id>,
    pub artifact_reference: String,
    pub artifact_hash: String,
    pub submission_evidence_reference: String,
    pub submission_evidence_hash: String,
    pub verification_evidence_reference: String,
    pub verification_evidence_hash: String,
    pub verification_mechanism: String,
    pub verification_strength: String,
    pub compensation: ContributionRecordCompensation,
    pub completed_at: DateTime<Utc>,
    pub transferable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FinalSubmission {
    pub provider_id: Id,
    pub artifact_reference: String,
    pub artifact_hash: String,
    pub evidence_reference: String,
    pub evidence_hash: String,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    OpenForProposals,
    Coordinating,
    ReadyForFinalExecution,
    FinalSubmitted,
    FinalVerifiedAwaitingSettlement,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct WalletApproval {
    pub participant_id: Id,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ApprovalRecordEntry {
    pub participant_id: Id,
    pub recovered_wallet: String,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ApprovalRecord {
    pub commitment_hash: String,
    pub approvals: Vec<ApprovalRecordEntry>,
    pub threshold: u16,
    pub approved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ApprovalRequirement {
    pub participant_ids: Vec<Id>,
    pub threshold: u16,
    pub purpose: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveCreationDraft {
    pub id: Id,
    pub title: String,
    pub desired_outcome: String,
    pub human_purpose: String,
    pub participants: Vec<ObjectiveParticipant>,
    pub requesting_party_id: Id,
    pub beneficiary_ids: Vec<Id>,
    pub affected_parties: Vec<AffectedPartyDeclaration>,
    pub authority: ObjectiveAuthority,
    pub available_resources: Vec<ResourceDeclaration>,
    pub expected_final_deliverable: String,
    pub requested_access_policy: DeliverableAccessPolicy,
    pub requested_rights_policy: RightsPolicy,
    pub requested_final_verification: ObjectiveVerificationPolicy,
    pub privacy: ObjectivePrivacyDeclaration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveCreationPlan {
    pub schema_version: String,
    pub draft: ObjectiveCreationDraft,
    pub commitment_hash: String,
    pub required_approval: ApprovalRequirement,
    pub signing_instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SignedObjectiveCreation {
    pub plan: ObjectiveCreationPlan,
    pub approvals: Vec<WalletApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectiveAction {
    AddProviderProposal {
        proposal: Box<ProviderProposalDraft>,
    },
    AcceptProviderProposal {
        proposal_id: Id,
    },
    OfferContribution {
        offer: ContributionOfferDraft,
    },
    SelectContributionOffer {
        offer_id: Id,
    },
    SubmitContribution {
        offer_id: Id,
        submission: ContributionSubmission,
    },
    VerifyContribution {
        offer_id: Id,
        statement: VerificationStatement,
    },
    SubmitFinalOutcome {
        submission: FinalSubmission,
    },
    VerifyFinalOutcome {
        statement: VerificationStatement,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveActionPlan {
    pub schema_version: String,
    pub objective_id: Id,
    pub objective_revision: u64,
    pub action: ObjectiveAction,
    pub commitment_hash: String,
    pub required_approval: ApprovalRequirement,
    pub signing_instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SignedObjectiveAction {
    pub plan: ObjectiveActionPlan,
    pub approvals: Vec<WalletApproval>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveActionEvent {
    pub revision: u64,
    pub action_kind: String,
    pub commitment_hash: String,
    pub approval: ApprovalRecord,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalFundingEvidence {
    pub binding: CanonicalBountyBinding,
    pub funded_atomic_amount: u64,
    pub target_atomic_amount: u64,
    pub status: String,
    pub verification_ready: bool,
    pub verification_readiness_reason: String,
    pub confirming_event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalSettlementEvidence {
    pub binding: CanonicalBountyBinding,
    pub event_id: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
    pub recipient_wallet: String,
    pub solver_payout_atomic_amount: u64,
    pub submission_hash: String,
    pub evidence_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveCanonicalEvidence {
    pub funding: Vec<CanonicalFundingEvidence>,
    pub settlements: Vec<CanonicalSettlementEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Complete,
    Incomplete,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ReadinessCheck {
    pub key: String,
    pub label: String,
    pub state: ReadinessState,
    pub blocker: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveReadiness {
    pub ready: bool,
    pub checks: Vec<ReadinessCheck>,
    pub blockers: Vec<String>,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveGraphNodeKind {
    Objective,
    Funding,
    ContributionNeed,
    FinalExecution,
    FinalVerification,
    Settlement,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveGraphNode {
    pub id: String,
    pub kind: ObjectiveGraphNodeKind,
    pub label: String,
    pub required: bool,
    pub state: ReadinessState,
    pub evidence_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveGraphEdge {
    pub from: String,
    pub to: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveGraph {
    pub root_id: String,
    pub nodes: Vec<ObjectiveGraphNode>,
    pub edges: Vec<ObjectiveGraphEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ObjectiveView {
    pub objective: Objective,
    pub readiness: ObjectiveReadiness,
    pub graph: ObjectiveGraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Objective {
    pub schema_version: String,
    pub id: Id,
    pub revision: u64,
    pub status: ObjectiveStatus,
    pub title: String,
    pub desired_outcome: String,
    pub human_purpose: String,
    pub participants: BTreeMap<Id, ObjectiveParticipant>,
    pub requesting_party_id: Id,
    pub beneficiary_ids: Vec<Id>,
    pub affected_parties: Vec<AffectedPartyDeclaration>,
    pub authority: ObjectiveAuthority,
    pub available_resources: Vec<ResourceDeclaration>,
    pub expected_final_deliverable: String,
    pub requested_access_policy: DeliverableAccessPolicy,
    pub requested_rights_policy: RightsPolicy,
    pub requested_final_verification: ObjectiveVerificationPolicy,
    pub privacy: ObjectivePrivacyDeclaration,
    pub creation_approval: ApprovalRecord,
    pub proposals: BTreeMap<Id, ProviderProposal>,
    pub accepted_value_bundle: Option<AcceptedValueBundle>,
    pub contribution_offers: BTreeMap<Id, ContributionOffer>,
    pub contribution_records: Vec<VerifiedContributionRecord>,
    pub final_submission: Option<FinalSubmission>,
    pub final_verification: Option<VerificationRecord>,
    pub final_settlement: Option<CanonicalSettlementEvidence>,
    pub final_outcome_record: Option<VerifiedFinalOutcomeRecord>,
    pub action_events: Vec<ObjectiveActionEvent>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

impl Objective {
    pub fn plan_creation(draft: ObjectiveCreationDraft) -> ObjectiveResult<ObjectiveCreationPlan> {
        validate_creation_draft(&draft)?;
        let commitment_hash = commitment_hash(&CreationCommitment {
            schema_version: OBJECTIVE_SCHEMA_VERSION,
            draft: &draft,
        })?;
        Ok(ObjectiveCreationPlan {
            schema_version: OBJECTIVE_SCHEMA_VERSION.to_string(),
            required_approval: ApprovalRequirement {
                participant_ids: vec![draft.requesting_party_id],
                threshold: 1,
                purpose:
                    "authenticate the requesting party and bind the exact objective declaration"
                        .to_string(),
            },
            draft,
            commitment_hash,
            signing_instruction: signing_instruction(),
        })
    }

    pub fn create(signed: SignedObjectiveCreation, now: DateTime<Utc>) -> ObjectiveResult<Self> {
        if signed.plan.schema_version != OBJECTIVE_SCHEMA_VERSION {
            return Err(ObjectiveError::StaleAction);
        }
        let expected = Self::plan_creation(signed.plan.draft.clone())?;
        if expected.commitment_hash != signed.plan.commitment_hash
            || expected.required_approval != signed.plan.required_approval
        {
            return Err(ObjectiveError::StaleAction);
        }
        let participants = participant_map(&signed.plan.draft.participants)?;
        let approval = verify_approvals(
            &participants,
            &expected.required_approval,
            &expected.commitment_hash,
            &signed.approvals,
            now,
        )?;
        let draft = signed.plan.draft;
        Ok(Self {
            schema_version: OBJECTIVE_SCHEMA_VERSION.to_string(),
            id: draft.id,
            revision: 1,
            status: ObjectiveStatus::OpenForProposals,
            title: draft.title,
            desired_outcome: draft.desired_outcome,
            human_purpose: draft.human_purpose,
            participants,
            requesting_party_id: draft.requesting_party_id,
            beneficiary_ids: draft.beneficiary_ids,
            affected_parties: draft.affected_parties,
            authority: draft.authority,
            available_resources: draft.available_resources,
            expected_final_deliverable: draft.expected_final_deliverable,
            requested_access_policy: draft.requested_access_policy,
            requested_rights_policy: draft.requested_rights_policy,
            requested_final_verification: draft.requested_final_verification,
            privacy: draft.privacy,
            creation_approval: approval,
            proposals: BTreeMap::new(),
            accepted_value_bundle: None,
            contribution_offers: BTreeMap::new(),
            contribution_records: Vec::new(),
            final_submission: None,
            final_verification: None,
            final_settlement: None,
            final_outcome_record: None,
            action_events: Vec::new(),
            created_at: now,
            updated_at: now,
            completed_at: None,
        })
    }

    pub fn plan_action(
        &self,
        action: ObjectiveAction,
        now: DateTime<Utc>,
    ) -> ObjectiveResult<ObjectiveActionPlan> {
        let requirement = self.validate_action_and_requirement(&action, now)?;
        let commitment_hash = commitment_hash(&ActionCommitment {
            schema_version: OBJECTIVE_SCHEMA_VERSION,
            objective_id: self.id,
            objective_revision: self.revision,
            action: &action,
        })?;
        Ok(ObjectiveActionPlan {
            schema_version: OBJECTIVE_SCHEMA_VERSION.to_string(),
            objective_id: self.id,
            objective_revision: self.revision,
            action,
            commitment_hash,
            required_approval: requirement,
            signing_instruction: signing_instruction(),
        })
    }

    pub fn apply_action(
        &mut self,
        signed: SignedObjectiveAction,
        now: DateTime<Utc>,
        evidence: &ObjectiveCanonicalEvidence,
    ) -> ObjectiveResult<()> {
        if signed.plan.schema_version != OBJECTIVE_SCHEMA_VERSION
            || signed.plan.objective_id != self.id
            || signed.plan.objective_revision != self.revision
        {
            return Err(ObjectiveError::StaleAction);
        }
        let expected = self.plan_action(signed.plan.action.clone(), now)?;
        if expected.commitment_hash != signed.plan.commitment_hash
            || expected.required_approval != signed.plan.required_approval
        {
            return Err(ObjectiveError::StaleAction);
        }
        let approval = verify_approvals(
            &self.participants,
            &expected.required_approval,
            &expected.commitment_hash,
            &signed.approvals,
            now,
        )?;
        let action_kind = action_kind(&expected.action).to_string();
        self.apply_validated_action(expected.action, approval.clone(), now, evidence)?;
        self.revision += 1;
        self.updated_at = now;
        self.action_events.push(ObjectiveActionEvent {
            revision: self.revision,
            action_kind,
            commitment_hash: expected.commitment_hash,
            approval,
            occurred_at: now,
        });
        Ok(())
    }

    pub fn reconcile_canonical_evidence(
        &mut self,
        evidence: &ObjectiveCanonicalEvidence,
        now: DateTime<Utc>,
    ) -> ObjectiveResult<bool> {
        let mut changed = false;
        let bundle = match self.accepted_value_bundle.as_ref() {
            Some(bundle) => bundle.clone(),
            None => return Ok(false),
        };

        for need in &bundle.contribution_needs {
            let ContributionCompensation::Paid { payment } = &need.compensation else {
                continue;
            };
            let candidate_ids = self
                .contribution_offers
                .iter()
                .filter_map(|(id, offer)| {
                    (offer.draft.need_id == need.id
                        && offer.state == ContributionWorkState::Submitted)
                        .then_some(*id)
                })
                .collect::<Vec<_>>();
            for offer_id in candidate_ids {
                let candidate = &self.contribution_offers[&offer_id];
                let contributor_id = candidate.draft.contributor_id;
                let submission = candidate
                    .submission
                    .as_ref()
                    .expect("submitted offers have a submission");
                let contributor_wallet = self.participant_wallet(contributor_id)?.to_string();
                let Some(settlement) = matching_settlement(
                    evidence,
                    &payment.bounty,
                    &contributor_wallet,
                    payment.atomic_amount,
                    &submission.artifact_hash,
                    &submission.evidence_hash,
                ) else {
                    continue;
                };
                let offer = self
                    .contribution_offers
                    .get_mut(&offer_id)
                    .expect("offer exists");
                offer.state = ContributionWorkState::Verified;
                offer.compensation_state = ContributionCompensationState::PaidCanonical {
                    settlement: Box::new(settlement.clone()),
                };
                offer.updated_at = now;
                if !self
                    .contribution_records
                    .iter()
                    .any(|record| record.contribution_offer_id == offer_id)
                {
                    self.contribution_records.push(verified_contribution_record(
                        self.id,
                        need,
                        offer,
                        ContributionRecordCompensation::PaidCanonical {
                            settlement: Box::new(settlement.clone()),
                        },
                        now,
                    ));
                }
                changed = true;
            }
        }

        if let Some(submission) = self.final_submission.clone() {
            if let Some(payment) = &bundle.monetary_payment {
                let provider_wallet = self.participant_wallet(bundle.provider_id)?.to_string();
                if let Some(settlement) = matching_settlement(
                    evidence,
                    &payment.bounty,
                    &provider_wallet,
                    payment.atomic_amount,
                    &submission.artifact_hash,
                    &submission.evidence_hash,
                ) {
                    if self.final_settlement.as_ref() != Some(&settlement) {
                        self.final_settlement = Some(settlement.clone());
                        let verification = VerificationRecord {
                            policy_hash: policy_hash(&bundle.final_verification_policy)?,
                            statement: VerificationStatement {
                                passed: true,
                                evidence_reference: format!("{}#{}", settlement.tx_hash, settlement.log_index),
                                evidence_hash: settlement.evidence_hash.clone(),
                                summary: "Final outcome passed the immutable canonical bounty policy; confirmed BountySettled proves payment."
                                    .to_string(),
                            },
                            approvals: ApprovalRecord {
                                commitment_hash: settlement.event_id.clone(),
                                approvals: Vec::new(),
                                threshold: 0,
                                approved_at: now,
                            },
                            verified_at: now,
                        };
                        self.final_verification = Some(verification.clone());
                        self.final_outcome_record = Some(verified_final_outcome_record(
                            self.id,
                            &self.beneficiary_ids,
                            &bundle,
                            &submission,
                            &verification,
                            ContributionRecordCompensation::PaidCanonical {
                                settlement: Box::new(settlement.clone()),
                            },
                            now,
                        ));
                        self.status = ObjectiveStatus::Completed;
                        self.completed_at = Some(now);
                        changed = true;
                    }
                }
            }
        }

        if self.status == ObjectiveStatus::Coordinating && self.readiness(evidence, now)?.ready {
            self.status = ObjectiveStatus::ReadyForFinalExecution;
            changed = true;
        }

        if changed {
            self.revision += 1;
            self.updated_at = now;
        }
        Ok(changed)
    }

    pub fn view(
        &self,
        evidence: &ObjectiveCanonicalEvidence,
        now: DateTime<Utc>,
    ) -> ObjectiveResult<ObjectiveView> {
        let readiness = self.readiness(evidence, now)?;
        let graph = self.graph(&readiness, evidence);
        Ok(ObjectiveView {
            objective: self.clone(),
            readiness,
            graph,
        })
    }

    pub fn readiness(
        &self,
        evidence: &ObjectiveCanonicalEvidence,
        now: DateTime<Utc>,
    ) -> ObjectiveResult<ObjectiveReadiness> {
        let mut checks = Vec::new();
        let mut blockers = Vec::new();
        let Some(bundle) = self.accepted_value_bundle.as_ref() else {
            let blocker =
                "No provider proposal has been accepted by the declared objective authority."
                    .to_string();
            checks.push(readiness_check(
                "accepted_value_bundle",
                "Accepted provider agreement",
                false,
                Some(blocker.clone()),
            ));
            return Ok(ObjectiveReadiness {
                ready: false,
                checks,
                blockers: vec![blocker],
                next_actions: vec!["Receive a provider proposal, then accept it with the declared authority threshold.".to_string()],
            });
        };
        checks.push(readiness_check(
            "accepted_value_bundle",
            "Accepted provider agreement",
            true,
            None,
        ));

        let bundle_active = now <= bundle.delivery_deadline;
        let deadline_blocker = (!bundle_active).then(|| "The accepted provider delivery deadline has passed; objective-v1 does not permit silent amendments.".to_string());
        push_check(
            &mut checks,
            &mut blockers,
            "bundle_valid",
            "Accepted value bundle remains valid",
            bundle_active,
            deadline_blocker,
        );

        if let Some(payment) = &bundle.monetary_payment {
            let funded = matching_funding(evidence, &payment.bounty, payment.atomic_amount);
            let blocker = (!funded).then(|| format!(
                "Canonical bounty {} has not produced evidence of at least {} atomic units of committed funding.",
                payment.bounty.bounty_contract, payment.atomic_amount
            ));
            push_check(
                &mut checks,
                &mut blockers,
                "funding",
                "Required monetary funding",
                funded,
                blocker,
            );
        } else {
            checks.push(ReadinessCheck {
                key: "funding".to_string(),
                label: "Required monetary funding".to_string(),
                state: ReadinessState::NotApplicable,
                blocker: None,
            });
        }

        for need in bundle
            .contribution_needs
            .iter()
            .filter(|need| need.mandatory)
        {
            let work_complete = self.need_is_complete(need);
            let incomplete_dependencies = self.incomplete_dependency_titles(need);
            let completed = work_complete && incomplete_dependencies.is_empty();
            let blocker = if !work_complete {
                Some(format!(
                    "Mandatory contribution '{}' has not reached its required verified compensation state.",
                    need.title
                ))
            } else if !incomplete_dependencies.is_empty() {
                Some(format!(
                    "Mandatory contribution '{}' still depends on incomplete work: {}.",
                    need.title,
                    incomplete_dependencies.join(", ")
                ))
            } else {
                None
            };
            push_check(
                &mut checks,
                &mut blockers,
                &format!("contribution:{}", need.id),
                &need.title,
                completed,
                blocker,
            );
        }

        let policy_available = validate_verification_policy(
            &bundle.final_verification_policy,
            &self.participants,
            Some(bundle.provider_id),
        )
        .is_ok();
        let policy_blocker = (!policy_available).then(|| {
            "The final verification policy is not executable with the declared participants."
                .to_string()
        });
        push_check(
            &mut checks,
            &mut blockers,
            "final_verification",
            "Final verification policy available",
            policy_available,
            policy_blocker,
        );

        let ready = blockers.is_empty()
            && matches!(
                self.status,
                ObjectiveStatus::Coordinating | ObjectiveStatus::ReadyForFinalExecution
            );
        let next_actions = if ready {
            vec!["The selected provider can submit the final outcome using a revision-bound signed action.".to_string()]
        } else {
            blockers
                .iter()
                .map(|blocker| format!("Resolve: {blocker}"))
                .collect()
        };
        Ok(ObjectiveReadiness {
            ready,
            checks,
            blockers,
            next_actions,
        })
    }

    pub fn graph(
        &self,
        readiness: &ObjectiveReadiness,
        evidence: &ObjectiveCanonicalEvidence,
    ) -> ObjectiveGraph {
        let root = format!("objective:{}", self.id);
        let final_execution = format!("objective:{}:final-execution", self.id);
        let final_verification = format!("objective:{}:final-verification", self.id);
        let settlement = format!("objective:{}:settlement", self.id);
        let mut nodes = vec![ObjectiveGraphNode {
            id: root.clone(),
            kind: ObjectiveGraphNodeKind::Objective,
            label: self.title.clone(),
            required: true,
            state: if self.status == ObjectiveStatus::Completed {
                ReadinessState::Complete
            } else {
                ReadinessState::Incomplete
            },
            evidence_references: self
                .final_settlement
                .as_ref()
                .map(|e| vec![e.tx_hash.clone()])
                .or_else(|| {
                    self.final_verification
                        .as_ref()
                        .map(|verification| vec![verification.statement.evidence_reference.clone()])
                })
                .unwrap_or_default(),
        }];
        let mut edges = Vec::new();

        if let Some(bundle) = &self.accepted_value_bundle {
            if let Some(payment) = &bundle.monetary_payment {
                let id = format!("objective:{}:funding", self.id);
                let check = readiness.checks.iter().find(|check| check.key == "funding");
                let evidence_references =
                    matching_funding_evidence(evidence, &payment.bounty, payment.atomic_amount)
                        .map(|funding| vec![funding.confirming_event_id.clone()])
                        .unwrap_or_default();
                nodes.push(ObjectiveGraphNode {
                    id: id.clone(),
                    kind: ObjectiveGraphNodeKind::Funding,
                    label: "Pool required monetary funding".to_string(),
                    required: true,
                    state: check.map(|c| c.state).unwrap_or(ReadinessState::Incomplete),
                    evidence_references,
                });
                edges.push(ObjectiveGraphEdge {
                    from: id,
                    to: final_execution.clone(),
                    required: true,
                });
            }
            for need in &bundle.contribution_needs {
                let id = format!("contribution:{}", need.id);
                let check_key = format!("contribution:{}", need.id);
                let check = readiness.checks.iter().find(|check| check.key == check_key);
                let evidence_references = self
                    .contribution_records
                    .iter()
                    .filter(|record| record.contribution_need_id == need.id)
                    .map(|record| record.evidence_reference.clone())
                    .collect();
                nodes.push(ObjectiveGraphNode {
                    id: id.clone(),
                    kind: ObjectiveGraphNodeKind::ContributionNeed,
                    label: need.title.clone(),
                    required: need.mandatory,
                    state: check.map(|c| c.state).unwrap_or_else(|| {
                        if self.need_path_is_complete(need) {
                            ReadinessState::Complete
                        } else {
                            ReadinessState::Incomplete
                        }
                    }),
                    evidence_references,
                });
                edges.push(ObjectiveGraphEdge {
                    from: id.clone(),
                    to: final_execution.clone(),
                    required: need.mandatory,
                });
                for dependency in &need.depends_on {
                    edges.push(ObjectiveGraphEdge {
                        from: format!("contribution:{dependency}"),
                        to: id.clone(),
                        required: true,
                    });
                }
            }
        }

        nodes.extend([
            ObjectiveGraphNode {
                id: final_execution.clone(),
                kind: ObjectiveGraphNodeKind::FinalExecution,
                label: "Provider delivers the final outcome".to_string(),
                required: true,
                state: if self.final_submission.is_some() {
                    ReadinessState::Complete
                } else {
                    ReadinessState::Incomplete
                },
                evidence_references: self
                    .final_submission
                    .as_ref()
                    .map(|s| vec![s.artifact_reference.clone(), s.evidence_reference.clone()])
                    .unwrap_or_default(),
            },
            ObjectiveGraphNode {
                id: final_verification.clone(),
                kind: ObjectiveGraphNodeKind::FinalVerification,
                label: "Verify the final outcome".to_string(),
                required: true,
                state: if self
                    .final_verification
                    .as_ref()
                    .is_some_and(|v| v.statement.passed)
                {
                    ReadinessState::Complete
                } else {
                    ReadinessState::Incomplete
                },
                evidence_references: self
                    .final_verification
                    .as_ref()
                    .map(|v| vec![v.statement.evidence_reference.clone()])
                    .unwrap_or_default(),
            },
            ObjectiveGraphNode {
                id: settlement.clone(),
                kind: ObjectiveGraphNodeKind::Settlement,
                label: "Canonical settlement when payment is required".to_string(),
                required: self
                    .accepted_value_bundle
                    .as_ref()
                    .is_some_and(|b| b.monetary_payment.is_some()),
                state: if self.final_settlement.is_some() {
                    ReadinessState::Complete
                } else if self
                    .accepted_value_bundle
                    .as_ref()
                    .is_some_and(|b| b.monetary_payment.is_none())
                {
                    ReadinessState::NotApplicable
                } else {
                    ReadinessState::Incomplete
                },
                evidence_references: self
                    .final_settlement
                    .as_ref()
                    .map(|s| vec![s.tx_hash.clone()])
                    .unwrap_or_default(),
            },
        ]);
        edges.push(ObjectiveGraphEdge {
            from: final_execution,
            to: final_verification.clone(),
            required: true,
        });
        if self
            .accepted_value_bundle
            .as_ref()
            .is_some_and(|bundle| bundle.monetary_payment.is_some())
        {
            edges.extend([
                ObjectiveGraphEdge {
                    from: final_verification,
                    to: settlement.clone(),
                    required: true,
                },
                ObjectiveGraphEdge {
                    from: settlement,
                    to: root.clone(),
                    required: true,
                },
            ]);
        } else {
            edges.push(ObjectiveGraphEdge {
                from: final_verification,
                to: root.clone(),
                required: true,
            });
        }
        ObjectiveGraph {
            root_id: root,
            nodes,
            edges,
        }
    }

    pub fn amendments_supported(&self) -> ObjectiveResult<()> {
        Err(ObjectiveError::AmendmentsUnavailable)
    }

    fn validate_action_and_requirement(
        &self,
        action: &ObjectiveAction,
        now: DateTime<Utc>,
    ) -> ObjectiveResult<ApprovalRequirement> {
        if matches!(
            self.status,
            ObjectiveStatus::Completed | ObjectiveStatus::Cancelled
        ) {
            return Err(ObjectiveError::InvalidAction(
                self.status,
                "terminal objectives cannot be changed".to_string(),
            ));
        }
        match action {
            ObjectiveAction::AddProviderProposal { proposal } => {
                if self.status != ObjectiveStatus::OpenForProposals {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "provider proposals are accepted only before a value bundle is accepted"
                            .to_string(),
                    ));
                }
                if self.accepted_value_bundle.is_some() {
                    return Err(ObjectiveError::ProposalAlreadyAccepted);
                }
                validate_proposal(proposal, &self.participants, now)?;
                if self.proposals.contains_key(&proposal.id) {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "proposal id already exists".to_string(),
                    ));
                }
                Ok(single_approval(
                    proposal.provider_id,
                    "bind the provider to the exact proposed value bundle",
                ))
            }
            ObjectiveAction::AcceptProviderProposal { proposal_id } => {
                if self.status != ObjectiveStatus::OpenForProposals {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "a provider proposal can be accepted only while the objective is open"
                            .to_string(),
                    ));
                }
                if self.accepted_value_bundle.is_some() {
                    return Err(ObjectiveError::ProposalAlreadyAccepted);
                }
                let proposal = self
                    .proposals
                    .get(proposal_id)
                    .ok_or(ObjectiveError::ProposalNotFound(*proposal_id))?;
                if now > proposal.draft.valid_until {
                    return Err(ObjectiveError::ProposalExpired);
                }
                Ok(self.authority_requirement(
                    "accept the provider proposal as the immutable value bundle",
                ))
            }
            ObjectiveAction::OfferContribution { offer } => {
                self.require_coordination_state()?;
                let need = self.need(offer.need_id)?;
                validate_offer(offer, need, &self.participants, now)?;
                if self.contribution_offers.contains_key(&offer.id) {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "contribution offer id already exists".to_string(),
                    ));
                }
                Ok(single_approval(
                    offer.contributor_id,
                    "bind the contributor to the exact contribution offer",
                ))
            }
            ObjectiveAction::SelectContributionOffer { offer_id } => {
                self.require_coordination_state()?;
                let offer = self
                    .contribution_offers
                    .get(offer_id)
                    .ok_or(ObjectiveError::ContributionOfferNotFound(*offer_id))?;
                if offer.state != ContributionWorkState::Offered {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "only an offered contribution can be selected".to_string(),
                    ));
                }
                Ok(self.authority_requirement(
                    "select this contributor without changing the accepted contribution need",
                ))
            }
            ObjectiveAction::SubmitContribution {
                offer_id,
                submission,
            } => {
                self.require_coordination_state()?;
                let offer = self
                    .contribution_offers
                    .get(offer_id)
                    .ok_or(ObjectiveError::ContributionOfferNotFound(*offer_id))?;
                if offer.state != ContributionWorkState::Selected {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "only a selected contribution can be submitted".to_string(),
                    ));
                }
                validate_submission(submission)?;
                Ok(single_approval(
                    offer.draft.contributor_id,
                    "submit exact artifact and evidence commitments",
                ))
            }
            ObjectiveAction::VerifyContribution {
                offer_id,
                statement,
            } => {
                self.require_coordination_state()?;
                let offer = self
                    .contribution_offers
                    .get(offer_id)
                    .ok_or(ObjectiveError::ContributionOfferNotFound(*offer_id))?;
                if offer.state != ContributionWorkState::Submitted {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "only submitted work can be verified".to_string(),
                    ));
                }
                validate_statement(statement)?;
                let need = self.need(offer.draft.need_id)?;
                if matches!(
                    need.verification_policy.mechanism,
                    ObjectiveVerificationMechanism::CanonicalBounty { .. }
                ) {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "canonical paid work is reconciled only from BountySettled evidence"
                            .to_string(),
                    ));
                }
                self.verification_requirement(&need.verification_policy)
            }
            ObjectiveAction::SubmitFinalOutcome { submission } => {
                self.require_coordination_state()?;
                let bundle = self.bundle()?;
                if submission.provider_id != bundle.provider_id {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "only the accepted provider can submit the final outcome".to_string(),
                    ));
                }
                validate_final_submission(submission)?;
                Ok(single_approval(
                    bundle.provider_id,
                    "submit the final outcome against the immutable provider commitment",
                ))
            }
            ObjectiveAction::VerifyFinalOutcome { statement } => {
                if self.status != ObjectiveStatus::FinalSubmitted {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "final verification requires a submitted final outcome".to_string(),
                    ));
                }
                let bundle = self.bundle()?;
                if self.final_submission.is_none() {
                    return Err(ObjectiveError::InvalidAction(
                        self.status,
                        "the provider has not submitted the final outcome".to_string(),
                    ));
                }
                if bundle.monetary_payment.is_some()
                    || matches!(
                        bundle.final_verification_policy.mechanism,
                        ObjectiveVerificationMechanism::CanonicalBounty { .. }
                    )
                {
                    return Err(ObjectiveError::InvalidAction(self.status, "paid final outcomes are completed only by matching canonical BountySettled evidence".to_string()));
                }
                validate_statement(statement)?;
                self.verification_requirement(&bundle.final_verification_policy)
            }
        }
    }

    fn apply_validated_action(
        &mut self,
        action: ObjectiveAction,
        approval: ApprovalRecord,
        now: DateTime<Utc>,
        evidence: &ObjectiveCanonicalEvidence,
    ) -> ObjectiveResult<()> {
        match action {
            ObjectiveAction::AddProviderProposal { proposal } => {
                let proposal = *proposal;
                let terms_hash = commitment_hash(&proposal)?;
                self.proposals.insert(
                    proposal.id,
                    ProviderProposal {
                        draft: proposal,
                        terms_hash,
                        provider_approval: approval,
                        created_at: now,
                    },
                );
            }
            ObjectiveAction::AcceptProviderProposal { proposal_id } => {
                let proposal = self
                    .proposals
                    .get(&proposal_id)
                    .cloned()
                    .ok_or(ObjectiveError::ProposalNotFound(proposal_id))?;
                self.accepted_value_bundle = Some(AcceptedValueBundle {
                    version: 1,
                    proposal_id,
                    provider_id: proposal.draft.provider_id,
                    terms_hash: proposal.terms_hash,
                    outcome_commitment: proposal.draft.outcome_commitment,
                    monetary_payment: proposal.draft.monetary_payment,
                    contribution_needs: proposal.draft.contribution_needs,
                    delivery_deadline: proposal.draft.delivery_deadline,
                    final_verification_policy: proposal.draft.final_verification_policy,
                    access_policy: proposal.draft.access_policy,
                    rights_policy: proposal.draft.rights_policy,
                    authority_approval: approval,
                    amendment_policy: BundleAmendmentPolicy::DisabledInObjectiveV1,
                    accepted_at: now,
                });
                self.status = ObjectiveStatus::Coordinating;
            }
            ObjectiveAction::OfferContribution { offer } => {
                let need = self.need(offer.need_id)?;
                let compensation_state = match need.compensation {
                    ContributionCompensation::InKind => ContributionCompensationState::InKind,
                    ContributionCompensation::Paid { .. } => {
                        ContributionCompensationState::PaymentPending
                    }
                };
                self.contribution_offers.insert(
                    offer.id,
                    ContributionOffer {
                        draft: offer,
                        state: ContributionWorkState::Offered,
                        compensation_state,
                        offer_approval: approval,
                        selection_approval: None,
                        submission: None,
                        verification: None,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            ObjectiveAction::SelectContributionOffer { offer_id } => {
                let offer = self
                    .contribution_offers
                    .get_mut(&offer_id)
                    .expect("validated offer");
                offer.state = ContributionWorkState::Selected;
                offer.selection_approval = Some(approval);
                offer.updated_at = now;
            }
            ObjectiveAction::SubmitContribution {
                offer_id,
                submission,
            } => {
                let offer = self
                    .contribution_offers
                    .get_mut(&offer_id)
                    .expect("validated offer");
                offer.state = ContributionWorkState::Submitted;
                offer.submission = Some(submission);
                offer.updated_at = now;
            }
            ObjectiveAction::VerifyContribution {
                offer_id,
                statement,
            } => {
                let need_id = self.contribution_offers[&offer_id].draft.need_id;
                let need = self.need(need_id)?.clone();
                let policy_hash = policy_hash(&need.verification_policy)?;
                let offer = self
                    .contribution_offers
                    .get_mut(&offer_id)
                    .expect("validated offer");
                offer.state = if statement.passed {
                    ContributionWorkState::Verified
                } else {
                    ContributionWorkState::Rejected
                };
                offer.verification = Some(VerificationRecord {
                    policy_hash,
                    statement: statement.clone(),
                    approvals: approval,
                    verified_at: now,
                });
                offer.updated_at = now;
                if statement.passed {
                    self.contribution_records.push(verified_contribution_record(
                        self.id,
                        &need,
                        offer,
                        ContributionRecordCompensation::InKind,
                        now,
                    ));
                }
            }
            ObjectiveAction::SubmitFinalOutcome { submission } => {
                let readiness = self.readiness(evidence, now)?;
                if !readiness.ready {
                    return Err(ObjectiveError::NotReady(readiness.blockers.join("; ")));
                }
                self.final_submission = Some(submission);
                self.status = ObjectiveStatus::FinalSubmitted;
            }
            ObjectiveAction::VerifyFinalOutcome { statement } => {
                let policy = self.bundle()?.final_verification_policy.clone();
                let verification = VerificationRecord {
                    policy_hash: policy_hash(&policy)?,
                    statement: statement.clone(),
                    approvals: approval,
                    verified_at: now,
                };
                self.final_verification = Some(verification.clone());
                if statement.passed {
                    let bundle = self.bundle()?.clone();
                    let submission = self
                        .final_submission
                        .as_ref()
                        .expect("validated final submission")
                        .clone();
                    self.final_outcome_record = Some(verified_final_outcome_record(
                        self.id,
                        &self.beneficiary_ids,
                        &bundle,
                        &submission,
                        &verification,
                        ContributionRecordCompensation::InKind,
                        now,
                    ));
                    self.status = ObjectiveStatus::Completed;
                    self.completed_at = Some(now);
                } else {
                    self.status = ObjectiveStatus::FinalSubmitted;
                }
            }
        }
        if self.status == ObjectiveStatus::Coordinating && self.readiness(evidence, now)?.ready {
            self.status = ObjectiveStatus::ReadyForFinalExecution;
        }
        Ok(())
    }

    fn bundle(&self) -> ObjectiveResult<&AcceptedValueBundle> {
        self.accepted_value_bundle.as_ref().ok_or_else(|| {
            ObjectiveError::InvalidAction(
                self.status,
                "no accepted value bundle exists".to_string(),
            )
        })
    }

    fn need(&self, id: Id) -> ObjectiveResult<&ContributionNeedDraft> {
        self.bundle()?
            .contribution_needs
            .iter()
            .find(|need| need.id == id)
            .ok_or(ObjectiveError::ContributionNeedNotFound(id))
    }

    fn need_is_complete(&self, need: &ContributionNeedDraft) -> bool {
        self.contribution_offers.values().any(|offer| {
            if offer.draft.need_id != need.id || offer.state != ContributionWorkState::Verified {
                return false;
            }
            match need.compensation {
                ContributionCompensation::InKind => matches!(
                    offer.compensation_state,
                    ContributionCompensationState::InKind
                ),
                ContributionCompensation::Paid { .. } => matches!(
                    offer.compensation_state,
                    ContributionCompensationState::PaidCanonical { .. }
                ),
            }
        })
    }

    fn need_path_is_complete(&self, need: &ContributionNeedDraft) -> bool {
        self.need_is_complete(need)
            && need.depends_on.iter().all(|dependency_id| {
                self.need(*dependency_id)
                    .is_ok_and(|dependency| self.need_path_is_complete(dependency))
            })
    }

    fn incomplete_dependency_titles(&self, need: &ContributionNeedDraft) -> Vec<String> {
        need.depends_on
            .iter()
            .filter_map(|dependency_id| self.need(*dependency_id).ok())
            .filter(|dependency| !self.need_path_is_complete(dependency))
            .map(|dependency| dependency.title.clone())
            .collect()
    }

    fn require_coordination_state(&self) -> ObjectiveResult<()> {
        if matches!(
            self.status,
            ObjectiveStatus::Coordinating | ObjectiveStatus::ReadyForFinalExecution
        ) {
            Ok(())
        } else {
            Err(ObjectiveError::InvalidAction(
                self.status,
                "contribution and final-submission actions require an active accepted value bundle"
                    .to_string(),
            ))
        }
    }

    fn participant_wallet(&self, id: Id) -> ObjectiveResult<&str> {
        self.participants
            .get(&id)
            .map(|p| p.wallet.as_str())
            .ok_or(ObjectiveError::UnknownParticipant(id))
    }

    fn authority_requirement(&self, purpose: &str) -> ApprovalRequirement {
        ApprovalRequirement {
            participant_ids: self.authority.member_ids.clone(),
            threshold: self.authority.threshold,
            purpose: purpose.to_string(),
        }
    }

    fn verification_requirement(
        &self,
        policy: &ObjectiveVerificationPolicy,
    ) -> ObjectiveResult<ApprovalRequirement> {
        match &policy.mechanism {
            ObjectiveVerificationMechanism::DeterministicSigner { verifier_id, .. }
            | ObjectiveVerificationMechanism::CommittedVerifier { verifier_id } => {
                Ok(single_approval(*verifier_id, "attest the exact verification statement under the committed policy"))
            }
            ObjectiveVerificationMechanism::WalletQuorum { verifier_ids, threshold }
            | ObjectiveVerificationMechanism::AiJudgeQuorum { verifier_ids, threshold, .. } => Ok(ApprovalRequirement {
                participant_ids: verifier_ids.clone(),
                threshold: *threshold,
                purpose: "meet the precommitted verifier quorum for this exact statement".to_string(),
            }),
            ObjectiveVerificationMechanism::ProviderAcceptance { provider_id } => {
                Ok(single_approval(*provider_id, "provider acceptance under the precommitted criteria"))
            }
            ObjectiveVerificationMechanism::ObjectiveAuthority => Ok(self.authority_requirement("objective-authority verification under the precommitted criteria")),
            ObjectiveVerificationMechanism::CanonicalBounty { .. } => Err(ObjectiveError::InvalidVerificationPolicy("canonical bounty verification is reconciled from confirmed events, not wallet approvals submitted to this objective".to_string())),
        }
    }
}

#[derive(Serialize)]
struct CreationCommitment<'a> {
    schema_version: &'static str,
    draft: &'a ObjectiveCreationDraft,
}

#[derive(Serialize)]
struct ActionCommitment<'a> {
    schema_version: &'static str,
    objective_id: Id,
    objective_revision: u64,
    action: &'a ObjectiveAction,
}

fn validate_creation_draft(draft: &ObjectiveCreationDraft) -> ObjectiveResult<()> {
    required(&draft.title, "title")?;
    required(&draft.desired_outcome, "desired_outcome")?;
    required(&draft.human_purpose, "human_purpose")?;
    required(
        &draft.expected_final_deliverable,
        "expected_final_deliverable",
    )?;
    let participants = participant_map(&draft.participants)?;
    require_participant(&participants, draft.requesting_party_id)?;
    if draft.beneficiary_ids.is_empty()
        || draft
            .beneficiary_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != draft.beneficiary_ids.len()
    {
        return Err(ObjectiveError::InvalidAction(
            ObjectiveStatus::OpenForProposals,
            "beneficiaries must be non-empty and unique".to_string(),
        ));
    }
    require_participants(&participants, &draft.beneficiary_ids)?;
    for affected in &draft.affected_parties {
        require_participant(&participants, affected.participant_id)?;
        required(&affected.description, "affected_party.description")?;
    }
    validate_authority(&draft.authority, &participants)?;
    for resource in &draft.available_resources {
        require_participant(&participants, resource.provider_id)?;
        required(&resource.description, "resource.description")?;
        validate_optional_hash(resource.evidence_hash.as_deref())?;
    }
    validate_access_policy(&draft.requested_access_policy, &participants)?;
    validate_rights_policy(&draft.requested_rights_policy, &participants)?;
    validate_verification_policy(&draft.requested_final_verification, &participants, None)?;
    if !draft.privacy.blockchain_information_is_public {
        return Err(ObjectiveError::UnsupportedPrivacy("Base addresses, transactions, contract state, and canonical payment evidence are public".to_string()));
    }
    required(&draft.privacy.redaction_limits, "privacy.redaction_limits")?;
    Ok(())
}

fn validate_proposal(
    proposal: &ProviderProposalDraft,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    now: DateTime<Utc>,
) -> ObjectiveResult<()> {
    require_participant(participants, proposal.provider_id)?;
    required(&proposal.outcome_commitment, "proposal.outcome_commitment")?;
    if proposal.valid_until <= now
        || proposal.delivery_deadline <= now
        || proposal.valid_until > proposal.delivery_deadline
    {
        return Err(ObjectiveError::ProposalExpired);
    }
    validate_payment_requirement(proposal.monetary_payment.as_ref())?;
    validate_access_policy(&proposal.access_policy, participants)?;
    validate_rights_policy(&proposal.rights_policy, participants)?;
    validate_verification_policy(
        &proposal.final_verification_policy,
        participants,
        Some(proposal.provider_id),
    )?;
    match (&proposal.monetary_payment, &proposal.final_verification_policy.mechanism) {
        (Some(payment), ObjectiveVerificationMechanism::CanonicalBounty { bounty }) if binding_matches(&payment.bounty, bounty) => {}
        (Some(_), _) => return Err(ObjectiveError::InvalidVerificationPolicy("a paid final outcome must use the same canonical bounty binding for payment and verification".to_string())),
        (None, ObjectiveVerificationMechanism::CanonicalBounty { .. }) => return Err(ObjectiveError::InvalidVerificationPolicy("an unpaid final outcome cannot claim canonical paid-bounty verification".to_string())),
        (None, _) => {}
    }
    validate_needs(
        &proposal.contribution_needs,
        participants,
        proposal.provider_id,
        proposal.delivery_deadline,
        now,
    )?;
    Ok(())
}

fn validate_needs(
    needs: &[ContributionNeedDraft],
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    provider_id: Id,
    delivery_deadline: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ObjectiveResult<()> {
    let ids = needs.iter().map(|need| need.id).collect::<BTreeSet<_>>();
    if ids.len() != needs.len() {
        return Err(ObjectiveError::InvalidAction(
            ObjectiveStatus::OpenForProposals,
            "contribution need ids must be unique".to_string(),
        ));
    }
    for need in needs {
        required(&need.title, "contribution_need.title")?;
        required(&need.deliverable, "contribution_need.deliverable")?;
        required(&need.purpose, "contribution_need.purpose")?;
        if need.deadline <= now || need.deadline > delivery_deadline {
            return Err(ObjectiveError::InvalidAction(
                ObjectiveStatus::OpenForProposals,
                format!(
                    "contribution need {} deadline must be after now and no later than final delivery",
                    need.id
                ),
            ));
        }
        if need.recipient_ids.is_empty()
            || need
                .recipient_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len()
                != need.recipient_ids.len()
        {
            return Err(ObjectiveError::InvalidAction(
                ObjectiveStatus::OpenForProposals,
                format!(
                    "contribution need {} recipients must be non-empty and unique",
                    need.id
                ),
            ));
        }
        require_participants(participants, &need.recipient_ids)?;
        validate_access_policy(&need.access_policy, participants)?;
        validate_rights_policy(&need.rights_policy, participants)?;
        validate_verification_policy(&need.verification_policy, participants, Some(provider_id))?;
        match (&need.compensation, &need.verification_policy.mechanism) {
            (ContributionCompensation::Paid { payment }, ObjectiveVerificationMechanism::CanonicalBounty { bounty }) if binding_matches(&payment.bounty, bounty) => validate_payment_requirement(Some(payment))?,
            (ContributionCompensation::Paid { .. }, _) => return Err(ObjectiveError::InvalidVerificationPolicy("a paid contribution must be verified and paid by the same canonical bounty".to_string())),
            (ContributionCompensation::InKind, ObjectiveVerificationMechanism::CanonicalBounty { .. }) => return Err(ObjectiveError::InvalidVerificationPolicy("an in-kind contribution cannot use a paid-bounty settlement as its verification claim".to_string())),
            (ContributionCompensation::InKind, _) => {}
        }
        for dependency in &need.depends_on {
            if !ids.contains(dependency) {
                return Err(ObjectiveError::UnknownDependency(*dependency));
            }
        }
    }
    ensure_acyclic(needs)
}

fn ensure_acyclic(needs: &[ContributionNeedDraft]) -> ObjectiveResult<()> {
    fn visit(
        id: Id,
        dependencies: &BTreeMap<Id, Vec<Id>>,
        visiting: &mut BTreeSet<Id>,
        visited: &mut BTreeSet<Id>,
    ) -> ObjectiveResult<()> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(ObjectiveError::CyclicDependency);
        }
        for dependency in dependencies.get(&id).into_iter().flatten() {
            visit(*dependency, dependencies, visiting, visited)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }
    let dependencies = needs
        .iter()
        .map(|need| (need.id, need.depends_on.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for id in dependencies.keys() {
        visit(*id, &dependencies, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn validate_offer(
    offer: &ContributionOfferDraft,
    need: &ContributionNeedDraft,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    now: DateTime<Utc>,
) -> ObjectiveResult<()> {
    require_participant(participants, offer.contributor_id)?;
    required(
        &offer.deliverable_commitment,
        "offer.deliverable_commitment",
    )?;
    required(&offer.evidence_commitment, "offer.evidence_commitment")?;
    validate_hash(&offer.evidence_commitment)?;
    if offer.expected_delivery_at <= now || offer.expected_delivery_at > need.deadline {
        return Err(ObjectiveError::InvalidAction(
            ObjectiveStatus::Coordinating,
            "offer delivery must be after now and no later than the accepted need deadline"
                .to_string(),
        ));
    }
    match need.compensation {
        ContributionCompensation::InKind if offer.expects_monetary_compensation => {
            Err(ObjectiveError::InvalidAction(
                ObjectiveStatus::Coordinating,
                "an in-kind need cannot be offered as paid work".to_string(),
            ))
        }
        ContributionCompensation::Paid { .. } if !offer.expects_monetary_compensation => {
            Err(ObjectiveError::InvalidAction(
                ObjectiveStatus::Coordinating,
                "a paid need cannot be relabelled as in-kind work".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn validate_submission(submission: &ContributionSubmission) -> ObjectiveResult<()> {
    required(
        &submission.artifact_reference,
        "submission.artifact_reference",
    )?;
    required(
        &submission.evidence_reference,
        "submission.evidence_reference",
    )?;
    validate_hash(&submission.artifact_hash)?;
    validate_hash(&submission.evidence_hash)
}

fn validate_final_submission(submission: &FinalSubmission) -> ObjectiveResult<()> {
    required(
        &submission.artifact_reference,
        "final_submission.artifact_reference",
    )?;
    required(
        &submission.evidence_reference,
        "final_submission.evidence_reference",
    )?;
    validate_hash(&submission.artifact_hash)?;
    validate_hash(&submission.evidence_hash)
}

fn validate_statement(statement: &VerificationStatement) -> ObjectiveResult<()> {
    required(
        &statement.evidence_reference,
        "verification.evidence_reference",
    )?;
    required(&statement.summary, "verification.summary")?;
    validate_hash(&statement.evidence_hash)
}

fn validate_authority(
    authority: &ObjectiveAuthority,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
) -> ObjectiveResult<()> {
    required(&authority.public_statement, "authority.public_statement")?;
    let unique = authority
        .member_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if unique.len() != authority.member_ids.len() || unique.is_empty() {
        return Err(ObjectiveError::InvalidAuthority(
            "authority members must be non-empty and unique".to_string(),
        ));
    }
    require_participants(participants, &authority.member_ids)?;
    if authority.threshold == 0 || usize::from(authority.threshold) > unique.len() {
        return Err(ObjectiveError::InvalidAuthority(
            "threshold must be within the declared member set".to_string(),
        ));
    }
    match authority.kind {
        ObjectiveAuthorityKind::SingleWallet | ObjectiveAuthorityKind::OrganizationWallet
            if unique.len() != 1 || authority.threshold != 1 =>
        {
            Err(ObjectiveError::InvalidAuthority(
                "single-wallet authority must be one-of-one".to_string(),
            ))
        }
        _ => Ok(()),
    }
}

fn validate_verification_policy(
    policy: &ObjectiveVerificationPolicy,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    provider_id: Option<Id>,
) -> ObjectiveResult<()> {
    if policy.acceptance_criteria.is_empty()
        || policy
            .acceptance_criteria
            .iter()
            .any(|criterion| criterion.trim().is_empty())
    {
        return Err(ObjectiveError::InvalidVerificationPolicy(
            "at least one non-empty acceptance criterion is required".to_string(),
        ));
    }
    required(&policy.evidence_schema, "verification.evidence_schema")?;
    validate_hash(&policy.evidence_schema_hash)?;
    if policy.trust_assumptions.is_empty()
        || policy
            .trust_assumptions
            .iter()
            .any(|assumption| assumption.trim().is_empty())
    {
        return Err(ObjectiveError::InvalidVerificationPolicy(
            "trust assumptions must be explicit".to_string(),
        ));
    }
    match &policy.mechanism {
        ObjectiveVerificationMechanism::DeterministicSigner {
            verifier_id,
            module_reference,
        } => {
            require_participant(participants, *verifier_id)?;
            required(module_reference, "verification.module_reference")
        }
        ObjectiveVerificationMechanism::CommittedVerifier { verifier_id } => {
            require_participant(participants, *verifier_id)
        }
        ObjectiveVerificationMechanism::WalletQuorum {
            verifier_ids,
            threshold,
        } => validate_quorum(verifier_ids, *threshold, participants, 1),
        ObjectiveVerificationMechanism::AiJudgeQuorum {
            verifier_ids,
            threshold,
            commitment,
        } => {
            validate_quorum(verifier_ids, *threshold, participants, 2)?;
            required(&commitment.provider, "ai_judge.provider")?;
            required(&commitment.model, "ai_judge.model")?;
            required(&commitment.model_version, "ai_judge.model_version")?;
            for hash in [
                &commitment.system_prompt_hash,
                &commitment.rubric_hash,
                &commitment.benchmark_hash,
                &commitment.decoding_parameters_hash,
            ] {
                validate_hash(hash)?;
            }
            Ok(())
        }
        ObjectiveVerificationMechanism::ProviderAcceptance {
            provider_id: verifier,
        } => {
            require_participant(participants, *verifier)?;
            if provider_id.is_some_and(|provider| provider != *verifier) {
                return Err(ObjectiveError::InvalidVerificationPolicy("provider-acceptance policy names a different participant than the proposal provider".to_string()));
            }
            Ok(())
        }
        ObjectiveVerificationMechanism::ObjectiveAuthority => Ok(()),
        ObjectiveVerificationMechanism::CanonicalBounty { bounty } => validate_binding(bounty),
    }
}

fn validate_quorum(
    ids: &[Id],
    threshold: u16,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    minimum_threshold: u16,
) -> ObjectiveResult<()> {
    let unique = ids.iter().copied().collect::<BTreeSet<_>>();
    require_participants(participants, ids)?;
    if unique.len() != ids.len()
        || threshold < minimum_threshold
        || usize::from(threshold) > unique.len()
    {
        return Err(ObjectiveError::InvalidVerificationPolicy(
            "verifier set must be unique and meet its minimum threshold".to_string(),
        ));
    }
    Ok(())
}

fn validate_access_policy(
    policy: &DeliverableAccessPolicy,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
) -> ObjectiveResult<()> {
    match policy {
        DeliverableAccessPolicy::Public => Ok(()),
        DeliverableAccessPolicy::RequestingPartyOnly {
            custodian_id,
            enforcement,
        }
        | DeliverableAccessPolicy::QualifyingFunders {
            custodian_id,
            enforcement,
        }
        | DeliverableAccessPolicy::TimeDelayedPublic {
            custodian_id,
            enforcement,
            ..
        } => require_external_custodian(*custodian_id, *enforcement, participants),
        DeliverableAccessPolicy::NamedRecipients {
            recipient_ids,
            custodian_id,
            enforcement,
        }
        | DeliverableAccessPolicy::PublicSummaryRestrictedDeliverable {
            recipient_ids,
            custodian_id,
            enforcement,
        } => {
            if recipient_ids.is_empty() {
                return Err(ObjectiveError::InvalidAccessPolicy(
                    "restricted access needs at least one named recipient".to_string(),
                ));
            }
            require_participants(participants, recipient_ids)?;
            require_external_custodian(*custodian_id, *enforcement, participants)
        }
    }
}

fn require_external_custodian(
    custodian_id: Id,
    enforcement: ExternalAccessEnforcement,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
) -> ObjectiveResult<()> {
    require_participant(participants, custodian_id)?;
    if enforcement != ExternalAccessEnforcement::ExternalCustodian {
        return Err(ObjectiveError::UnsupportedPrivacy("restricted deliverable access is not enforced by the public protocol and must name an external custodian".to_string()));
    }
    Ok(())
}

fn validate_rights_policy(
    policy: &RightsPolicy,
    participants: &BTreeMap<Id, ObjectiveParticipant>,
) -> ObjectiveResult<()> {
    if policy.owner_ids.is_empty() {
        return Err(ObjectiveError::Required("rights.owner_ids"));
    }
    require_participants(participants, &policy.owner_ids)?;
    required(&policy.license_or_terms, "rights.license_or_terms")
}

fn validate_payment_requirement(
    payment: Option<&CanonicalPaymentRequirement>,
) -> ObjectiveResult<()> {
    let Some(payment) = payment else {
        return Ok(());
    };
    let expected_atomic_amount = u64::try_from(payment.amount.amount)
        .ok()
        .and_then(|amount| amount.checked_mul(10_000));
    if payment.amount.amount <= 0
        || !payment.amount.currency.eq_ignore_ascii_case("usdc")
        || payment.atomic_amount == 0
        || payment.decimals != 6
        || expected_atomic_amount != Some(payment.atomic_amount)
    {
        return Err(ObjectiveError::InvalidAction(
            ObjectiveStatus::OpenForProposals,
            "canonical payments must be positive native USDC with six decimals and match the amount expressed in USDC minor units".to_string(),
        ));
    }
    validate_binding(&payment.bounty)
}

fn validate_binding(binding: &CanonicalBountyBinding) -> ObjectiveResult<()> {
    required(&binding.network, "canonical_bounty.network")?;
    if Address::from_str(&binding.bounty_contract).is_err() {
        return Err(ObjectiveError::CanonicalEvidenceMismatch);
    }
    validate_hash(&binding.bounty_id)?;
    validate_hash(&binding.terms_hash)
}

fn participant_map(
    participants: &[ObjectiveParticipant],
) -> ObjectiveResult<BTreeMap<Id, ObjectiveParticipant>> {
    let mut map = BTreeMap::new();
    for participant in participants {
        required(&participant.display_name, "participant.display_name")?;
        if Address::from_str(&participant.wallet).is_err() {
            return Err(ObjectiveError::InvalidParticipantWallet(participant.id));
        }
        let mut normalized = participant.clone();
        normalized.wallet = normalize_wallet(&participant.wallet)?;
        if map.insert(participant.id, normalized).is_some() {
            return Err(ObjectiveError::DuplicateParticipant(participant.id));
        }
    }
    Ok(map)
}

fn verify_approvals(
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    requirement: &ApprovalRequirement,
    commitment: &str,
    approvals: &[WalletApproval],
    now: DateTime<Utc>,
) -> ObjectiveResult<ApprovalRecord> {
    validate_hash(commitment)?;
    let eligible = requirement
        .participant_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if requirement.threshold == 0 || usize::from(requirement.threshold) > eligible.len() {
        return Err(ObjectiveError::InvalidApproval(
            "invalid approval requirement".to_string(),
        ));
    }
    let mut seen = BTreeSet::new();
    let mut records = Vec::new();
    for approval in approvals {
        if !eligible.contains(&approval.participant_id) || !seen.insert(approval.participant_id) {
            return Err(ObjectiveError::InvalidApproval(
                "approval signer is ineligible or duplicated".to_string(),
            ));
        }
        let participant = participants
            .get(&approval.participant_id)
            .ok_or(ObjectiveError::UnknownParticipant(approval.participant_id))?;
        let recovered = recover_personal_sign_wallet(commitment, &approval.signature)?;
        if !recovered.eq_ignore_ascii_case(&participant.wallet) {
            return Err(ObjectiveError::InvalidApproval(format!(
                "signature does not recover participant {} wallet",
                participant.id
            )));
        }
        records.push(ApprovalRecordEntry {
            participant_id: participant.id,
            recovered_wallet: recovered,
            signature: approval.signature.clone(),
        });
    }
    if records.len() < usize::from(requirement.threshold) {
        return Err(ObjectiveError::ApprovalThresholdNotMet);
    }
    records.sort_by_key(|record| record.participant_id);
    Ok(ApprovalRecord {
        commitment_hash: commitment.to_ascii_lowercase(),
        approvals: records,
        threshold: requirement.threshold,
        approved_at: now,
    })
}

fn recover_personal_sign_wallet(commitment: &str, signature: &str) -> ObjectiveResult<String> {
    let hash = B256::from_str(commitment)
        .map_err(|_| ObjectiveError::InvalidApproval("commitment is not bytes32".to_string()))?;
    let mut message = Vec::with_capacity(PERSONAL_SIGN_PREFIX.len() + 32);
    message.extend_from_slice(PERSONAL_SIGN_PREFIX);
    message.extend_from_slice(hash.as_slice());
    let digest = keccak256(message);
    let signature = Signature::from_str(signature).map_err(|_| {
        ObjectiveError::InvalidApproval("signature is not a 65-byte EVM signature".to_string())
    })?;
    signature
        .recover_address_from_prehash(&digest)
        .map(|address| format!("{address:#x}"))
        .map_err(|_| ObjectiveError::InvalidApproval("signature recovery failed".to_string()))
}

fn matching_funding(
    evidence: &ObjectiveCanonicalEvidence,
    binding: &CanonicalBountyBinding,
    required: u64,
) -> bool {
    matching_funding_evidence(evidence, binding, required).is_some()
}

fn matching_funding_evidence<'a>(
    evidence: &'a ObjectiveCanonicalEvidence,
    binding: &CanonicalBountyBinding,
    required: u64,
) -> Option<&'a CanonicalFundingEvidence> {
    evidence.funding.iter().find(|item| {
        binding_matches(binding, &item.binding)
            && item.funded_atomic_amount >= required
            && item.target_atomic_amount >= required
            && item.verification_ready
            && matches!(
                item.status.as_str(),
                "claimable" | "claimed" | "submitted" | "paid"
            )
    })
}

fn matching_settlement(
    evidence: &ObjectiveCanonicalEvidence,
    binding: &CanonicalBountyBinding,
    recipient_wallet: &str,
    required: u64,
    submission_hash: &str,
    evidence_hash: &str,
) -> Option<CanonicalSettlementEvidence> {
    evidence
        .settlements
        .iter()
        .find(|settlement| {
            binding_matches(binding, &settlement.binding)
                && settlement
                    .recipient_wallet
                    .eq_ignore_ascii_case(recipient_wallet)
                && settlement.solver_payout_atomic_amount >= required
                && settlement
                    .submission_hash
                    .eq_ignore_ascii_case(submission_hash)
                && settlement.evidence_hash.eq_ignore_ascii_case(evidence_hash)
        })
        .cloned()
}

fn binding_matches(left: &CanonicalBountyBinding, right: &CanonicalBountyBinding) -> bool {
    left.network.eq_ignore_ascii_case(&right.network)
        && left
            .bounty_contract
            .eq_ignore_ascii_case(&right.bounty_contract)
        && left.bounty_id.eq_ignore_ascii_case(&right.bounty_id)
        && left.terms_hash.eq_ignore_ascii_case(&right.terms_hash)
}

fn verified_contribution_record(
    objective_id: Id,
    need: &ContributionNeedDraft,
    offer: &ContributionOffer,
    compensation: ContributionRecordCompensation,
    now: DateTime<Utc>,
) -> VerifiedContributionRecord {
    let (evidence_reference, evidence_hash) = offer
        .submission
        .as_ref()
        .map(|submission| {
            (
                submission.evidence_reference.clone(),
                submission.evidence_hash.clone(),
            )
        })
        .unwrap_or_else(|| match &compensation {
            ContributionRecordCompensation::PaidCanonical { settlement } => (
                format!("{}#{}", settlement.tx_hash, settlement.log_index),
                settlement.evidence_hash.clone(),
            ),
            _ => ("unavailable".to_string(), format!("0x{}", "0".repeat(64))),
        });
    VerifiedContributionRecord {
        id: Uuid::new_v5(&objective_id, offer.draft.id.as_bytes()),
        contributor_id: offer.draft.contributor_id,
        objective_id,
        contribution_need_id: need.id,
        contribution_offer_id: offer.draft.id,
        role: offer.draft.role,
        capability: need.title.clone(),
        beneficiary_categories: need.recipient_ids.iter().map(ToString::to_string).collect(),
        evidence_reference,
        evidence_hash,
        verification_mechanism: mechanism_name(&need.verification_policy.mechanism).to_string(),
        verification_strength: verification_strength(&need.verification_policy.mechanism),
        compensation,
        completed_at: now,
        transferable: false,
    }
}

fn verified_final_outcome_record(
    objective_id: Id,
    beneficiary_ids: &[Id],
    bundle: &AcceptedValueBundle,
    submission: &FinalSubmission,
    verification: &VerificationRecord,
    compensation: ContributionRecordCompensation,
    now: DateTime<Utc>,
) -> VerifiedFinalOutcomeRecord {
    VerifiedFinalOutcomeRecord {
        id: Uuid::new_v5(&objective_id, b"verified-final-outcome"),
        provider_id: bundle.provider_id,
        objective_id,
        role: ContributionRole::Provider,
        outcome_commitment: bundle.outcome_commitment.clone(),
        beneficiary_ids: beneficiary_ids.to_vec(),
        artifact_reference: submission.artifact_reference.clone(),
        artifact_hash: submission.artifact_hash.clone(),
        submission_evidence_reference: submission.evidence_reference.clone(),
        submission_evidence_hash: submission.evidence_hash.clone(),
        verification_evidence_reference: verification.statement.evidence_reference.clone(),
        verification_evidence_hash: verification.statement.evidence_hash.clone(),
        verification_mechanism: mechanism_name(&bundle.final_verification_policy.mechanism)
            .to_string(),
        verification_strength: verification_strength(&bundle.final_verification_policy.mechanism),
        compensation,
        completed_at: now,
        transferable: false,
    }
}

fn mechanism_name(mechanism: &ObjectiveVerificationMechanism) -> &'static str {
    match mechanism {
        ObjectiveVerificationMechanism::DeterministicSigner { .. } => "deterministic_signer",
        ObjectiveVerificationMechanism::CommittedVerifier { .. } => "committed_verifier",
        ObjectiveVerificationMechanism::WalletQuorum { .. } => "wallet_quorum",
        ObjectiveVerificationMechanism::AiJudgeQuorum { .. } => "ai_judge_quorum",
        ObjectiveVerificationMechanism::ProviderAcceptance { .. } => "provider_acceptance",
        ObjectiveVerificationMechanism::ObjectiveAuthority => "objective_authority",
        ObjectiveVerificationMechanism::CanonicalBounty { .. } => "canonical_bounty",
    }
}

fn verification_strength(mechanism: &ObjectiveVerificationMechanism) -> String {
    match mechanism {
        ObjectiveVerificationMechanism::WalletQuorum {
            threshold,
            verifier_ids,
        }
        | ObjectiveVerificationMechanism::AiJudgeQuorum {
            threshold,
            verifier_ids,
            ..
        } => format!("{threshold}-of-{} signed quorum", verifier_ids.len()),
        ObjectiveVerificationMechanism::CanonicalBounty { .. } => {
            "confirmed canonical BountySettled event".to_string()
        }
        _ => "one committed wallet signature".to_string(),
    }
}

fn action_kind(action: &ObjectiveAction) -> &'static str {
    match action {
        ObjectiveAction::AddProviderProposal { .. } => "add_provider_proposal",
        ObjectiveAction::AcceptProviderProposal { .. } => "accept_provider_proposal",
        ObjectiveAction::OfferContribution { .. } => "offer_contribution",
        ObjectiveAction::SelectContributionOffer { .. } => "select_contribution_offer",
        ObjectiveAction::SubmitContribution { .. } => "submit_contribution",
        ObjectiveAction::VerifyContribution { .. } => "verify_contribution",
        ObjectiveAction::SubmitFinalOutcome { .. } => "submit_final_outcome",
        ObjectiveAction::VerifyFinalOutcome { .. } => "verify_final_outcome",
    }
}

fn readiness_check(
    key: &str,
    label: &str,
    complete: bool,
    blocker: Option<String>,
) -> ReadinessCheck {
    ReadinessCheck {
        key: key.to_string(),
        label: label.to_string(),
        state: if complete {
            ReadinessState::Complete
        } else {
            ReadinessState::Incomplete
        },
        blocker,
    }
}

fn push_check(
    checks: &mut Vec<ReadinessCheck>,
    blockers: &mut Vec<String>,
    key: &str,
    label: &str,
    complete: bool,
    blocker: Option<String>,
) {
    if let Some(blocker) = blocker.clone() {
        blockers.push(blocker);
    }
    checks.push(readiness_check(key, label, complete, blocker));
}

fn policy_hash(policy: &ObjectiveVerificationPolicy) -> ObjectiveResult<String> {
    commitment_hash(policy)
}

fn commitment_hash<T: Serialize>(value: &T) -> ObjectiveResult<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| ObjectiveError::CommitmentSerialization)?;
    Ok(format!("{:#x}", keccak256(bytes)))
}

fn normalize_wallet(wallet: &str) -> ObjectiveResult<String> {
    Address::from_str(wallet)
        .map(|address| format!("{address:#x}"))
        .map_err(|_| {
            ObjectiveError::InvalidApproval("wallet is not a valid EVM address".to_string())
        })
}

fn validate_hash(value: &str) -> ObjectiveResult<()> {
    B256::from_str(value).map(|_| ()).map_err(|_| {
        ObjectiveError::InvalidVerificationPolicy("expected a 0x-prefixed bytes32 hash".to_string())
    })
}

fn validate_optional_hash(value: Option<&str>) -> ObjectiveResult<()> {
    value.map(validate_hash).transpose().map(|_| ())
}

fn required(value: &str, field: &'static str) -> ObjectiveResult<()> {
    if value.trim().is_empty() {
        Err(ObjectiveError::Required(field))
    } else {
        Ok(())
    }
}

fn require_participant(
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    id: Id,
) -> ObjectiveResult<()> {
    participants
        .contains_key(&id)
        .then_some(())
        .ok_or(ObjectiveError::UnknownParticipant(id))
}

fn require_participants(
    participants: &BTreeMap<Id, ObjectiveParticipant>,
    ids: &[Id],
) -> ObjectiveResult<()> {
    for id in ids {
        require_participant(participants, *id)?;
    }
    Ok(())
}

fn single_approval(participant_id: Id, purpose: &str) -> ApprovalRequirement {
    ApprovalRequirement {
        participant_ids: vec![participant_id],
        threshold: 1,
        purpose: purpose.to_string(),
    }
}

fn signing_instruction() -> String {
    "Sign the 32-byte commitment_hash with EIP-191 personal_sign. The service recovers every declared participant wallet and rejects stale revisions, duplicate signers, or changed action content."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    use chrono::Duration;

    fn hash(byte: char) -> String {
        format!("0x{}", byte.to_string().repeat(64))
    }

    fn signer(seed: &str) -> PrivateKeySigner {
        seed.parse().unwrap()
    }

    fn participant(
        id: Id,
        kind: ParticipantKind,
        name: &str,
        signer: &PrivateKeySigner,
    ) -> ObjectiveParticipant {
        ObjectiveParticipant {
            id,
            kind,
            display_name: name.to_string(),
            wallet: format!("{:#x}", signer.address()),
            identity_disclosure: IdentityDisclosure::Pseudonymous,
            public_identity_reference: None,
        }
    }

    fn approval(plan_hash: &str, id: Id, signer: &PrivateKeySigner) -> WalletApproval {
        let hash = B256::from_str(plan_hash).unwrap();
        let signature = signer.sign_message_sync(hash.as_slice()).unwrap();
        WalletApproval {
            participant_id: id,
            signature: signature.to_string(),
        }
    }

    struct Fixture {
        now: DateTime<Utc>,
        requester_id: Id,
        provider_id: Id,
        contributor_id: Id,
        verifier_a_id: Id,
        verifier_b_id: Id,
        requester: PrivateKeySigner,
        provider: PrivateKeySigner,
        contributor: PrivateKeySigner,
        verifier_a: PrivateKeySigner,
        verifier_b: PrivateKeySigner,
        binding: CanonicalBountyBinding,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                now: Utc::now(),
                requester_id: Uuid::new_v4(),
                provider_id: Uuid::new_v4(),
                contributor_id: Uuid::new_v4(),
                verifier_a_id: Uuid::new_v4(),
                verifier_b_id: Uuid::new_v4(),
                requester: signer(
                    "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
                ),
                provider: signer(
                    "0x8b3a350cf5c34c9194ca3a545d60ce3a46f87d7b32f1a4e2d88b9f9f79f7f2ab",
                ),
                contributor: signer(
                    "0x0f4b3f0f39c0d0e65c9f6c1edee6c7f5fd6e4d316121208f919f113b2f6f0f21",
                ),
                verifier_a: signer(
                    "0x5de4111afa1c4b3daadb1b5dbad9e1c58a5b55c44537e65ad8d7f7f5c3c3c3c3",
                ),
                verifier_b: signer(
                    "0x7c852118294e51e6533b16f3f841b9f597844f6f2f2176f8475b17e13f9c3a4f",
                ),
                binding: CanonicalBountyBinding {
                    network: "base-mainnet".to_string(),
                    bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
                    bounty_id: hash('1'),
                    terms_hash: hash('2'),
                },
            }
        }

        fn policy(&self) -> ObjectiveVerificationPolicy {
            ObjectiveVerificationPolicy {
                mechanism: ObjectiveVerificationMechanism::WalletQuorum {
                    verifier_ids: vec![self.verifier_a_id, self.verifier_b_id],
                    threshold: 2,
                },
                acceptance_criteria: vec!["The committed benchmark passes.".to_string()],
                evidence_schema: "https://example.test/evidence.schema.json".to_string(),
                evidence_schema_hash: hash('3'),
                trust_assumptions: vec![
                    "Both committed verifier wallets independently inspect the public evidence."
                        .to_string(),
                ],
            }
        }

        fn creation(&self) -> SignedObjectiveCreation {
            let draft = ObjectiveCreationDraft {
                id: Uuid::new_v4(),
                title: "Publish an auditable coordination report".to_string(),
                desired_outcome: "A verified report is publicly available.".to_string(),
                human_purpose: "Help affected people make an informed decision.".to_string(),
                participants: vec![
                    participant(self.requester_id, ParticipantKind::Organization, "Requester", &self.requester),
                    participant(self.provider_id, ParticipantKind::Agent, "Provider", &self.provider),
                    participant(self.contributor_id, ParticipantKind::Agent, "Researcher", &self.contributor),
                    participant(self.verifier_a_id, ParticipantKind::Agent, "Verifier A", &self.verifier_a),
                    participant(self.verifier_b_id, ParticipantKind::Agent, "Verifier B", &self.verifier_b),
                ],
                requesting_party_id: self.requester_id,
                beneficiary_ids: vec![self.requester_id],
                affected_parties: vec![AffectedPartyDeclaration {
                    participant_id: self.requester_id,
                    expected_effect: ExpectedEffect::Positive,
                    description: "Receives the report and bears decision risk.".to_string(),
                }],
                authority: ObjectiveAuthority {
                    kind: ObjectiveAuthorityKind::OrganizationWallet,
                    member_ids: vec![self.requester_id],
                    threshold: 1,
                    public_statement: "The requesting organization controls this objective through its declared wallet.".to_string(),
                },
                available_resources: Vec::new(),
                expected_final_deliverable: "Public report and evidence package".to_string(),
                requested_access_policy: DeliverableAccessPolicy::Public,
                requested_rights_policy: RightsPolicy {
                    owner_ids: vec![self.requester_id],
                    license_or_terms: "CC-BY-4.0".to_string(),
                    restrictions: Vec::new(),
                },
                requested_final_verification: self.policy(),
                privacy: ObjectivePrivacyDeclaration {
                    blockchain_information_is_public: true,
                    evidence_policy: PublicEvidencePolicy::Public,
                    redaction_limits: "No secrets or personal data may be placed in public evidence.".to_string(),
                },
            };
            let plan = Objective::plan_creation(draft).unwrap();
            let approvals = vec![approval(
                &plan.commitment_hash,
                self.requester_id,
                &self.requester,
            )];
            SignedObjectiveCreation { plan, approvals }
        }

        fn in_kind_need(&self) -> ContributionNeedDraft {
            ContributionNeedDraft {
                id: Uuid::new_v4(),
                title: "Regulatory research".to_string(),
                deliverable: "A source-linked permit research package".to_string(),
                purpose: "Establish the legal prerequisites for the final report".to_string(),
                recipient_ids: vec![self.provider_id],
                verification_policy: self.policy(),
                mandatory: true,
                deadline: self.now + Duration::days(5),
                access_policy: DeliverableAccessPolicy::Public,
                rights_policy: RightsPolicy {
                    owner_ids: vec![self.requester_id],
                    license_or_terms: "CC-BY-4.0".to_string(),
                    restrictions: Vec::new(),
                },
                compensation: ContributionCompensation::InKind,
                depends_on: Vec::new(),
            }
        }

        fn apply(
            &self,
            objective: &mut Objective,
            action: ObjectiveAction,
            signers: &[(Id, &PrivateKeySigner)],
            at: DateTime<Utc>,
            evidence: &ObjectiveCanonicalEvidence,
        ) {
            let plan = objective.plan_action(action, at).unwrap();
            let approvals = signers
                .iter()
                .map(|(id, signer)| approval(&plan.commitment_hash, *id, signer))
                .collect();
            objective
                .apply_action(SignedObjectiveAction { plan, approvals }, at, evidence)
                .unwrap();
        }
    }

    #[test]
    fn objective_loop_keeps_offers_verification_and_in_kind_compensation_distinct() {
        let f = Fixture::new();
        let mut objective = Objective::create(f.creation(), f.now).unwrap();
        let need = f.in_kind_need();
        let proposal = ProviderProposalDraft {
            id: Uuid::new_v4(),
            provider_id: f.provider_id,
            outcome_commitment: "Deliver the final public report".to_string(),
            monetary_payment: None,
            contribution_needs: vec![need.clone()],
            delivery_deadline: f.now + Duration::days(10),
            final_verification_policy: f.policy(),
            access_policy: DeliverableAccessPolicy::Public,
            rights_policy: need.rights_policy.clone(),
            valid_until: f.now + Duration::days(2),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::AddProviderProposal {
                proposal: Box::new(proposal.clone()),
            },
            &[(f.provider_id, &f.provider)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::AcceptProviderProposal {
                proposal_id: proposal.id,
            },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );

        let offer = ContributionOfferDraft {
            id: Uuid::new_v4(),
            need_id: need.id,
            contributor_id: f.contributor_id,
            role: ContributionRole::Researcher,
            deliverable_commitment: "Source-linked research package".to_string(),
            expected_delivery_at: f.now + Duration::days(3),
            evidence_commitment: hash('4'),
            expects_monetary_compensation: false,
            conditions: Vec::new(),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::OfferContribution {
                offer: offer.clone(),
            },
            &[(f.contributor_id, &f.contributor)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        assert_eq!(
            objective.contribution_offers[&offer.id].state,
            ContributionWorkState::Offered
        );
        f.apply(
            &mut objective,
            ObjectiveAction::SelectContributionOffer { offer_id: offer.id },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        assert_eq!(
            objective.contribution_offers[&offer.id].state,
            ContributionWorkState::Selected
        );
        f.apply(
            &mut objective,
            ObjectiveAction::SubmitContribution {
                offer_id: offer.id,
                submission: ContributionSubmission {
                    artifact_reference: "https://example.test/research".to_string(),
                    artifact_hash: hash('5'),
                    evidence_reference: "https://example.test/evidence".to_string(),
                    evidence_hash: hash('6'),
                    submitted_at: f.now,
                },
            },
            &[(f.contributor_id, &f.contributor)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        assert_eq!(
            objective.contribution_offers[&offer.id].state,
            ContributionWorkState::Submitted
        );
        f.apply(
            &mut objective,
            ObjectiveAction::VerifyContribution {
                offer_id: offer.id,
                statement: VerificationStatement {
                    passed: true,
                    evidence_reference: "https://example.test/verdict".to_string(),
                    evidence_hash: hash('7'),
                    summary: "Both verifiers reproduced the benchmark.".to_string(),
                },
            },
            &[
                (f.verifier_a_id, &f.verifier_a),
                (f.verifier_b_id, &f.verifier_b),
            ],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        assert_eq!(
            objective.contribution_offers[&offer.id].state,
            ContributionWorkState::Verified
        );
        assert!(matches!(
            objective.contribution_records[0].compensation,
            ContributionRecordCompensation::InKind
        ));
        assert!(!objective.contribution_records[0].transferable);
        assert_eq!(
            objective.contribution_records[0].role,
            ContributionRole::Researcher
        );
        assert!(
            objective
                .readiness(&ObjectiveCanonicalEvidence::default(), f.now)
                .unwrap()
                .ready
        );
        assert_eq!(objective.status, ObjectiveStatus::ReadyForFinalExecution);
        let view = objective
            .view(&ObjectiveCanonicalEvidence::default(), f.now)
            .unwrap();
        assert!(view.graph.edges.iter().any(|edge| {
            edge.from == format!("objective:{}:final-verification", objective.id)
                && edge.to == format!("objective:{}", objective.id)
                && edge.required
        }));
        f.apply(
            &mut objective,
            ObjectiveAction::SubmitFinalOutcome {
                submission: FinalSubmission {
                    provider_id: f.provider_id,
                    artifact_reference: "https://example.test/final-report".to_string(),
                    artifact_hash: hash('8'),
                    evidence_reference: "https://example.test/final-report-evidence".to_string(),
                    evidence_hash: hash('9'),
                    submitted_at: f.now,
                },
            },
            &[(f.provider_id, &f.provider)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::VerifyFinalOutcome {
                statement: VerificationStatement {
                    passed: true,
                    evidence_reference: "https://example.test/final-verdict".to_string(),
                    evidence_hash: hash('a'),
                    summary: "Both verifiers reproduced the final benchmark.".to_string(),
                },
            },
            &[
                (f.verifier_a_id, &f.verifier_a),
                (f.verifier_b_id, &f.verifier_b),
            ],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        assert_eq!(objective.status, ObjectiveStatus::Completed);
        let provider_record = objective.final_outcome_record.as_ref().unwrap();
        assert_eq!(provider_record.role, ContributionRole::Provider);
        assert!(!provider_record.transferable);
        assert!(matches!(
            provider_record.compensation,
            ContributionRecordCompensation::InKind
        ));
    }

    #[test]
    fn readiness_enforces_optional_needs_that_are_required_dependencies() {
        let f = Fixture::new();
        let mut objective = Objective::create(f.creation(), f.now).unwrap();
        let mut prerequisite = f.in_kind_need();
        prerequisite.title = "Source access".to_string();
        prerequisite.mandatory = false;
        let mut final_input = f.in_kind_need();
        final_input.title = "Regulatory report".to_string();
        final_input.depends_on = vec![prerequisite.id];
        let proposal = ProviderProposalDraft {
            id: Uuid::new_v4(),
            provider_id: f.provider_id,
            outcome_commitment: "Integrate the completed report".to_string(),
            monetary_payment: None,
            contribution_needs: vec![prerequisite.clone(), final_input.clone()],
            delivery_deadline: f.now + Duration::days(10),
            final_verification_policy: f.policy(),
            access_policy: DeliverableAccessPolicy::Public,
            rights_policy: final_input.rights_policy.clone(),
            valid_until: f.now + Duration::days(2),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::AddProviderProposal {
                proposal: Box::new(proposal.clone()),
            },
            &[(f.provider_id, &f.provider)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::AcceptProviderProposal {
                proposal_id: proposal.id,
            },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );

        let record_approval = objective.creation_approval.clone();
        let verified_offer = |need_id: Id, role: ContributionRole| {
            let offer_id = Uuid::new_v4();
            (
                offer_id,
                ContributionOffer {
                    draft: ContributionOfferDraft {
                        id: offer_id,
                        need_id,
                        contributor_id: f.contributor_id,
                        role,
                        deliverable_commitment: "Verified deliverable".to_string(),
                        expected_delivery_at: f.now + Duration::days(2),
                        evidence_commitment: hash('4'),
                        expects_monetary_compensation: false,
                        conditions: Vec::new(),
                    },
                    state: ContributionWorkState::Verified,
                    compensation_state: ContributionCompensationState::InKind,
                    offer_approval: record_approval.clone(),
                    selection_approval: None,
                    submission: None,
                    verification: None,
                    created_at: f.now,
                    updated_at: f.now,
                },
            )
        };
        let (report_offer_id, report_offer) =
            verified_offer(final_input.id, ContributionRole::Researcher);
        objective
            .contribution_offers
            .insert(report_offer_id, report_offer);

        let blocked = objective
            .readiness(&ObjectiveCanonicalEvidence::default(), f.now)
            .unwrap();
        assert!(blocked
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Source access")));

        let (access_offer_id, access_offer) =
            verified_offer(prerequisite.id, ContributionRole::Contributor);
        objective
            .contribution_offers
            .insert(access_offer_id, access_offer);
        assert!(
            objective
                .readiness(&ObjectiveCanonicalEvidence::default(), f.now)
                .unwrap()
                .ready
        );
    }

    #[test]
    fn paid_contribution_requires_the_exact_submitted_commitments() {
        let f = Fixture::new();
        let mut objective = Objective::create(f.creation(), f.now).unwrap();
        let payment = CanonicalPaymentRequirement {
            amount: Money::new(100, "usdc").unwrap(),
            atomic_amount: 1_000_000,
            decimals: 6,
            bounty: f.binding.clone(),
        };
        let mut need = f.in_kind_need();
        need.compensation = ContributionCompensation::Paid {
            payment: payment.clone(),
        };
        need.verification_policy = ObjectiveVerificationPolicy {
            mechanism: ObjectiveVerificationMechanism::CanonicalBounty {
                bounty: f.binding.clone(),
            },
            acceptance_criteria: vec!["The committed test suite passes.".to_string()],
            evidence_schema: "ipfs://paid-contribution-schema".to_string(),
            evidence_schema_hash: hash('4'),
            trust_assumptions: vec![
                "The canonical bounty enforces the immutable verifier policy.".to_string(),
            ],
        };
        let proposal = ProviderProposalDraft {
            id: Uuid::new_v4(),
            provider_id: f.provider_id,
            outcome_commitment: "Integrate the verified contribution".to_string(),
            monetary_payment: None,
            contribution_needs: vec![need.clone()],
            delivery_deadline: f.now + Duration::days(10),
            final_verification_policy: f.policy(),
            access_policy: DeliverableAccessPolicy::Public,
            rights_policy: need.rights_policy.clone(),
            valid_until: f.now + Duration::days(2),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::AddProviderProposal {
                proposal: Box::new(proposal.clone()),
            },
            &[(f.provider_id, &f.provider)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::AcceptProviderProposal {
                proposal_id: proposal.id,
            },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        let offer = ContributionOfferDraft {
            id: Uuid::new_v4(),
            need_id: need.id,
            contributor_id: f.contributor_id,
            role: ContributionRole::Developer,
            deliverable_commitment: "A tested implementation".to_string(),
            expected_delivery_at: f.now + Duration::days(3),
            evidence_commitment: hash('5'),
            expects_monetary_compensation: true,
            conditions: Vec::new(),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::OfferContribution {
                offer: offer.clone(),
            },
            &[(f.contributor_id, &f.contributor)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::SelectContributionOffer { offer_id: offer.id },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::SubmitContribution {
                offer_id: offer.id,
                submission: ContributionSubmission {
                    artifact_reference: "https://example.test/code".to_string(),
                    artifact_hash: hash('6'),
                    evidence_reference: "https://example.test/code-evidence".to_string(),
                    evidence_hash: hash('7'),
                    submitted_at: f.now,
                },
            },
            &[(f.contributor_id, &f.contributor)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );

        let settlement = |submission_hash: String| CanonicalSettlementEvidence {
            binding: f.binding.clone(),
            event_id: hash('8'),
            tx_hash: hash('9'),
            block_number: 100,
            log_index: 2,
            recipient_wallet: format!("{:#x}", f.contributor.address()),
            solver_payout_atomic_amount: payment.atomic_amount,
            submission_hash,
            evidence_hash: hash('7'),
        };
        let wrong = ObjectiveCanonicalEvidence {
            funding: Vec::new(),
            settlements: vec![settlement(hash('f'))],
        };
        assert!(!objective
            .reconcile_canonical_evidence(&wrong, f.now)
            .unwrap());
        assert_eq!(
            objective.contribution_offers[&offer.id].state,
            ContributionWorkState::Submitted
        );

        let exact = ObjectiveCanonicalEvidence {
            funding: Vec::new(),
            settlements: vec![settlement(hash('6'))],
        };
        assert!(objective
            .reconcile_canonical_evidence(&exact, f.now)
            .unwrap());
        let paid_offer = &objective.contribution_offers[&offer.id];
        assert_eq!(paid_offer.state, ContributionWorkState::Verified);
        assert!(matches!(
            paid_offer.compensation_state,
            ContributionCompensationState::PaidCanonical { .. }
        ));
        assert_eq!(objective.contribution_records.len(), 1);
        assert_eq!(objective.status, ObjectiveStatus::ReadyForFinalExecution);
    }

    #[test]
    fn paid_final_outcome_needs_funding_then_bounty_settlement_before_completion() {
        let f = Fixture::new();
        let mut objective = Objective::create(f.creation(), f.now).unwrap();
        let payment = CanonicalPaymentRequirement {
            amount: Money::new(100, "usdc").unwrap(),
            atomic_amount: 1_000_000,
            decimals: 6,
            bounty: f.binding.clone(),
        };
        let proposal = ProviderProposalDraft {
            id: Uuid::new_v4(),
            provider_id: f.provider_id,
            outcome_commitment: "Deliver a verified report".to_string(),
            monetary_payment: Some(payment.clone()),
            contribution_needs: Vec::new(),
            delivery_deadline: f.now + Duration::days(10),
            final_verification_policy: ObjectiveVerificationPolicy {
                mechanism: ObjectiveVerificationMechanism::CanonicalBounty {
                    bounty: f.binding.clone(),
                },
                acceptance_criteria: vec!["Canonical benchmark passes".to_string()],
                evidence_schema: "ipfs://schema".to_string(),
                evidence_schema_hash: hash('8'),
                trust_assumptions: vec![
                    "The immutable bounty verifier enforces the committed policy.".to_string(),
                ],
            },
            access_policy: DeliverableAccessPolicy::Public,
            rights_policy: RightsPolicy {
                owner_ids: vec![f.requester_id],
                license_or_terms: "CC-BY-4.0".to_string(),
                restrictions: Vec::new(),
            },
            valid_until: f.now + Duration::days(2),
        };
        f.apply(
            &mut objective,
            ObjectiveAction::AddProviderProposal {
                proposal: Box::new(proposal.clone()),
            },
            &[(f.provider_id, &f.provider)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        f.apply(
            &mut objective,
            ObjectiveAction::AcceptProviderProposal {
                proposal_id: proposal.id,
            },
            &[(f.requester_id, &f.requester)],
            f.now,
            &ObjectiveCanonicalEvidence::default(),
        );
        let not_ready = objective
            .readiness(&ObjectiveCanonicalEvidence::default(), f.now)
            .unwrap();
        assert!(not_ready
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Canonical bounty")));

        let funding = ObjectiveCanonicalEvidence {
            funding: vec![CanonicalFundingEvidence {
                binding: f.binding.clone(),
                funded_atomic_amount: 1_000_000,
                target_atomic_amount: 1_000_000,
                status: "claimable".to_string(),
                verification_ready: true,
                verification_readiness_reason: "Deterministic verifier is committed and available."
                    .to_string(),
                confirming_event_id: "event:funded".to_string(),
            }],
            settlements: Vec::new(),
        };
        assert!(objective
            .reconcile_canonical_evidence(&funding, f.now)
            .unwrap());
        assert_eq!(objective.status, ObjectiveStatus::ReadyForFinalExecution);
        let plan = objective
            .plan_action(
                ObjectiveAction::SubmitFinalOutcome {
                    submission: FinalSubmission {
                        provider_id: f.provider_id,
                        artifact_reference: "https://example.test/final".to_string(),
                        artifact_hash: hash('9'),
                        evidence_reference: "https://example.test/final-evidence".to_string(),
                        evidence_hash: hash('a'),
                        submitted_at: f.now,
                    },
                },
                f.now,
            )
            .unwrap();
        let signed = SignedObjectiveAction {
            approvals: vec![approval(&plan.commitment_hash, f.provider_id, &f.provider)],
            plan,
        };
        objective.apply_action(signed, f.now, &funding).unwrap();
        assert_eq!(objective.status, ObjectiveStatus::FinalSubmitted);
        assert!(objective.final_settlement.is_none());
        assert!(matches!(
            objective.plan_action(
                ObjectiveAction::OfferContribution {
                    offer: ContributionOfferDraft {
                        id: Uuid::new_v4(),
                        need_id: Uuid::new_v4(),
                        contributor_id: f.contributor_id,
                        role: ContributionRole::Contributor,
                        deliverable_commitment: "Late work".to_string(),
                        expected_delivery_at: f.now + Duration::days(1),
                        evidence_commitment: hash('d'),
                        expects_monetary_compensation: false,
                        conditions: Vec::new(),
                    },
                },
                f.now,
            ),
            Err(ObjectiveError::InvalidAction(
                ObjectiveStatus::FinalSubmitted,
                _
            ))
        ));

        let mut settled = funding;
        settled.settlements.push(CanonicalSettlementEvidence {
            binding: f.binding.clone(),
            event_id: hash('b'),
            tx_hash: hash('c'),
            block_number: 100,
            log_index: 2,
            recipient_wallet: format!("{:#x}", f.provider.address()),
            solver_payout_atomic_amount: 1_000_000,
            submission_hash: hash('9'),
            evidence_hash: hash('f'),
        });
        assert!(!objective
            .reconcile_canonical_evidence(&settled, f.now)
            .unwrap());
        assert_eq!(objective.status, ObjectiveStatus::FinalSubmitted);
        settled.settlements[0].evidence_hash = hash('a');
        assert!(objective
            .reconcile_canonical_evidence(&settled, f.now)
            .unwrap());
        assert_eq!(objective.status, ObjectiveStatus::Completed);
        assert!(
            objective
                .final_verification
                .as_ref()
                .unwrap()
                .statement
                .passed
        );
        assert!(objective.final_settlement.is_some());
        let provider_record = objective.final_outcome_record.as_ref().unwrap();
        assert_eq!(provider_record.provider_id, f.provider_id);
        assert!(matches!(
            provider_record.compensation,
            ContributionRecordCompensation::PaidCanonical { .. }
        ));
        let view = objective.view(&settled, f.now).unwrap();
        let funding_node = view
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == ObjectiveGraphNodeKind::Funding)
            .unwrap();
        assert_eq!(
            funding_node.evidence_references,
            vec!["event:funded".to_string()]
        );
        let root = view
            .graph
            .nodes
            .iter()
            .find(|node| node.kind == ObjectiveGraphNodeKind::Objective)
            .unwrap();
        assert_eq!(root.evidence_references, vec![hash('c')]);
    }

    #[test]
    fn cyclic_needs_and_fake_privacy_are_rejected() {
        let f = Fixture::new();
        let mut signed = f.creation();
        signed.plan.draft.privacy.blockchain_information_is_public = false;
        assert!(matches!(
            Objective::plan_creation(signed.plan.draft),
            Err(ObjectiveError::UnsupportedPrivacy(_))
        ));

        let mut first = f.in_kind_need();
        let mut second = f.in_kind_need();
        first.depends_on = vec![second.id];
        second.depends_on = vec![first.id];
        assert_eq!(
            ensure_acyclic(&[first, second]),
            Err(ObjectiveError::CyclicDependency)
        );
    }

    #[test]
    fn stale_or_wrong_wallet_actions_fail_closed() {
        let f = Fixture::new();
        let objective = Objective::create(f.creation(), f.now).unwrap();
        let proposal = ProviderProposalDraft {
            id: Uuid::new_v4(),
            provider_id: f.provider_id,
            outcome_commitment: "Outcome".to_string(),
            monetary_payment: None,
            contribution_needs: Vec::new(),
            delivery_deadline: f.now + Duration::days(2),
            final_verification_policy: f.policy(),
            access_policy: DeliverableAccessPolicy::Public,
            rights_policy: RightsPolicy {
                owner_ids: vec![f.requester_id],
                license_or_terms: "CC0".to_string(),
                restrictions: Vec::new(),
            },
            valid_until: f.now + Duration::days(1),
        };
        let plan = objective
            .plan_action(
                ObjectiveAction::AddProviderProposal {
                    proposal: Box::new(proposal),
                },
                f.now,
            )
            .unwrap();
        let wrong = SignedObjectiveAction {
            approvals: vec![approval(&plan.commitment_hash, f.provider_id, &f.requester)],
            plan,
        };
        let mut objective = objective;
        assert!(matches!(
            objective.apply_action(wrong, f.now, &ObjectiveCanonicalEvidence::default()),
            Err(ObjectiveError::InvalidApproval(_))
        ));
    }
}
