use crate::{Id, Money};
use chrono::{DateTime, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

/// Stable schema identifier for the additive guild-domain contract.
pub const GUILD_DOMAIN_SCHEMA_VERSION: &str = "agent-bounties/guild-domain-v1";
pub const DEFAULT_MISSION_MINIMUM_TRUST_REVIEW_COUNT: u32 = 3;
pub const DEFAULT_AFFILIATION_COMPLETED_TASK_THRESHOLD: u32 = 5;
pub const DEFAULT_AFFILIATION_MINIMUM_TRUST_REVIEW_COUNT: u32 = 3;

fn guild_domain_schema_version() -> String {
    GUILD_DOMAIN_SCHEMA_VERSION.to_string()
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GuildDomainError {
    #[error("trust score must be between 1 and 5")]
    InvalidTrustScore,
    #[error("quality/integrity review must be one non-empty line of at most 280 characters")]
    InvalidTrustReviewSentence,
    #[error("guild participant identifier must not be empty")]
    InvalidParticipantId,
    #[error("informal party name must not be empty and member identifiers must be unique")]
    InvalidInformalParty,
    #[error("trust review revision audit is inconsistent")]
    InvalidTrustReviewAudit,
    #[error("a trust review must be authored and revised by the same non-subject participant")]
    InvalidTrustReviewAuthor,
    #[error("trust review revision timestamp must not move backwards")]
    InvalidRevisionTimestamp,
    #[error("trust summary is inconsistent with 1-5 reviews")]
    InvalidTrustSummary,
    #[error("money promise requires a positive amount and non-empty single-line currency")]
    InvalidMoneyPromise,
    #[error("canonical Base USDC evidence requires a positive USDC money amount")]
    InvalidCanonicalMoneyPromise,
    #[error("other-asset identifier and quantity must be non-empty single-line values")]
    InvalidOtherAssetPromise,
    #[error("guild mission title must not be empty")]
    InvalidMissionTitle,
}

pub type GuildDomainResult<T> = Result<T, GuildDomainError>;

/// Reputation-derived adventurer class. Declaration order is its rank order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
pub enum AdventurerRank {
    #[serde(rename = "F")]
    F,
    #[serde(rename = "E")]
    E,
    #[serde(rename = "D")]
    D,
    #[serde(rename = "C")]
    C,
    #[serde(rename = "B")]
    B,
    #[serde(rename = "A")]
    A,
    #[serde(rename = "S")]
    S,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AdventurerRankThreshold {
    pub rank: AdventurerRank,
    pub minimum_reputation_points: u64,
}

/// Default, monotonic rank schedule. Applications may display another schedule,
/// but `AdventurerRank::from_reputation_points` always uses this versioned one.
pub const DEFAULT_ADVENTURER_RANK_THRESHOLDS: [AdventurerRankThreshold; 7] = [
    AdventurerRankThreshold {
        rank: AdventurerRank::F,
        minimum_reputation_points: 0,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::E,
        minimum_reputation_points: 100,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::D,
        minimum_reputation_points: 250,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::C,
        minimum_reputation_points: 500,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::B,
        minimum_reputation_points: 1_000,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::A,
        minimum_reputation_points: 2_500,
    },
    AdventurerRankThreshold {
        rank: AdventurerRank::S,
        minimum_reputation_points: 5_000,
    },
];

impl AdventurerRank {
    pub const fn from_reputation_points(points: u64) -> Self {
        if points >= 5_000 {
            Self::S
        } else if points >= 2_500 {
            Self::A
        } else if points >= 1_000 {
            Self::B
        } else if points >= 500 {
            Self::C
        } else if points >= 250 {
            Self::D
        } else if points >= 100 {
            Self::E
        } else {
            Self::F
        }
    }

    pub const fn minimum_reputation_points(self) -> u64 {
        match self {
            Self::F => 0,
            Self::E => 100,
            Self::D => 250,
            Self::C => 500,
            Self::B => 1_000,
            Self::A => 2_500,
            Self::S => 5_000,
        }
    }
}

/// Human-facing mission complexity. It deliberately has no conversion to
/// `AdventurerRank`, bounty price, eligibility, funding, or payment state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
pub enum MissionDifficulty {
    #[serde(rename = "F")]
    F,
    #[serde(rename = "E")]
    E,
    #[serde(rename = "D")]
    D,
    #[serde(rename = "C")]
    C,
    #[serde(rename = "B")]
    B,
    #[serde(rename = "A")]
    A,
    #[serde(rename = "S")]
    S,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct TrustScore(u8);

impl TrustScore {
    pub fn new(value: u8) -> GuildDomainResult<Self> {
        (1..=5)
            .contains(&value)
            .then_some(Self(value))
            .ok_or(GuildDomainError::InvalidTrustScore)
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for TrustScore {
    type Error = GuildDomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<TrustScore> for u8 {
    fn from(value: TrustScore) -> Self {
        value.get()
    }
}

impl<'de> Deserialize<'de> for TrustScore {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u8::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct TrustReviewSentence(String);

impl TrustReviewSentence {
    pub fn new(value: impl Into<String>) -> GuildDomainResult<Self> {
        let value = value.into().trim().to_string();
        let valid =
            !value.is_empty() && value.chars().count() <= 280 && !value.contains(['\r', '\n']);
        valid
            .then_some(Self(value))
            .ok_or(GuildDomainError::InvalidTrustReviewSentence)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TrustReviewSentence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum GuildParticipantKind {
    Human,
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GuildParticipantRef {
    pub participant_id: String,
    pub kind: GuildParticipantKind,
    pub display_name: Option<String>,
}

impl GuildParticipantRef {
    pub fn new(
        participant_id: impl Into<String>,
        kind: GuildParticipantKind,
        display_name: Option<String>,
    ) -> GuildDomainResult<Self> {
        let participant_id = participant_id.into().trim().to_string();
        if participant_id.is_empty() {
            return Err(GuildDomainError::InvalidParticipantId);
        }
        Ok(Self {
            participant_id,
            kind,
            display_name: display_name
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        })
    }
}

/// Exact aggregate of integer 1-5 reviews. Rational comparisons avoid floating
/// point drift in eligibility and affiliation decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrustSummary {
    pub score_total: u64,
    pub review_count: u32,
}

impl TrustSummary {
    pub fn new(score_total: u64, review_count: u32) -> GuildDomainResult<Self> {
        let summary = Self {
            score_total,
            review_count,
        };
        summary.validate()?;
        Ok(summary)
    }

    pub fn validate(self) -> GuildDomainResult<()> {
        let count = u64::from(self.review_count);
        let valid = if count == 0 {
            self.score_total == 0
        } else {
            self.score_total >= count && self.score_total <= 5 * count
        };
        valid
            .then_some(())
            .ok_or(GuildDomainError::InvalidTrustSummary)
    }

    pub fn average_at_least(self, minimum: TrustScore) -> bool {
        self.review_count > 0
            && self.score_total >= u64::from(minimum.get()) * u64::from(self.review_count)
    }

    pub fn average_strictly_exceeds_four(self) -> bool {
        self.review_count > 0 && self.score_total > 4 * u64::from(self.review_count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct TrustReviewAuditEntry {
    pub revision: u32,
    pub score: TrustScore,
    pub quality_integrity: TrustReviewSentence,
    pub revised_by: GuildParticipantRef,
    pub reason: Option<String>,
    pub revised_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TrustReviewWire {
    #[serde(default = "guild_domain_schema_version")]
    schema_version: String,
    id: Id,
    subject: GuildParticipantRef,
    reviewer: GuildParticipantRef,
    mission_id: Option<String>,
    score: TrustScore,
    quality_integrity: TrustReviewSentence,
    revision: u32,
    audit_log: Vec<TrustReviewAuditEntry>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Mutable trust review with an append-only revision audit. Scores and text can
/// change only through `revise`; each serialized revision preserves who changed
/// the current quality/integrity assessment and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "TrustReviewWire")]
pub struct TrustReview {
    schema_version: String,
    id: Id,
    subject: GuildParticipantRef,
    reviewer: GuildParticipantRef,
    mission_id: Option<String>,
    score: TrustScore,
    quality_integrity: TrustReviewSentence,
    revision: u32,
    audit_log: Vec<TrustReviewAuditEntry>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TrustReview {
    pub fn new(
        subject: GuildParticipantRef,
        reviewer: GuildParticipantRef,
        mission_id: Option<String>,
        score: u8,
        quality_integrity: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> GuildDomainResult<Self> {
        validate_participant(&subject)?;
        validate_participant(&reviewer)?;
        if same_participant(&subject, &reviewer) {
            return Err(GuildDomainError::InvalidTrustReviewAuthor);
        }
        let score = TrustScore::new(score)?;
        let quality_integrity = TrustReviewSentence::new(quality_integrity)?;
        let audit_log = vec![TrustReviewAuditEntry {
            revision: 1,
            score,
            quality_integrity: quality_integrity.clone(),
            revised_by: reviewer.clone(),
            reason: None,
            revised_at: created_at,
        }];
        Ok(Self {
            schema_version: guild_domain_schema_version(),
            id: Uuid::new_v4(),
            subject,
            reviewer,
            mission_id: normalize_optional_text(mission_id),
            score,
            quality_integrity,
            revision: 1,
            audit_log,
            created_at,
            updated_at: created_at,
        })
    }

    pub fn revise(
        &mut self,
        score: u8,
        quality_integrity: impl Into<String>,
        revised_by: GuildParticipantRef,
        reason: Option<String>,
        revised_at: DateTime<Utc>,
    ) -> GuildDomainResult<()> {
        validate_participant(&revised_by)?;
        if !same_participant(&self.reviewer, &revised_by) {
            return Err(GuildDomainError::InvalidTrustReviewAuthor);
        }
        if revised_at < self.updated_at {
            return Err(GuildDomainError::InvalidRevisionTimestamp);
        }
        let score = TrustScore::new(score)?;
        let quality_integrity = TrustReviewSentence::new(quality_integrity)?;
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(GuildDomainError::InvalidTrustReviewAudit)?;
        let entry = TrustReviewAuditEntry {
            revision,
            score,
            quality_integrity: quality_integrity.clone(),
            revised_by,
            reason: normalize_optional_text(reason),
            revised_at,
        };
        self.score = score;
        self.quality_integrity = quality_integrity;
        self.revision = revision;
        self.audit_log.push(entry);
        self.updated_at = revised_at;
        Ok(())
    }

    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub const fn id(&self) -> Id {
        self.id
    }

    pub fn subject(&self) -> &GuildParticipantRef {
        &self.subject
    }

    pub fn reviewer(&self) -> &GuildParticipantRef {
        &self.reviewer
    }

    pub fn mission_id(&self) -> Option<&str> {
        self.mission_id.as_deref()
    }

    pub const fn score(&self) -> TrustScore {
        self.score
    }

    pub fn quality_integrity(&self) -> &str {
        self.quality_integrity.as_str()
    }

    pub const fn revision(&self) -> u32 {
        self.revision
    }

    pub fn audit_log(&self) -> &[TrustReviewAuditEntry] {
        &self.audit_log
    }

    pub const fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    pub const fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    pub fn validate(&self) -> GuildDomainResult<()> {
        validate_participant(&self.subject)?;
        validate_participant(&self.reviewer)?;
        if same_participant(&self.subject, &self.reviewer) {
            return Err(GuildDomainError::InvalidTrustReviewAuthor);
        }
        if self.schema_version != GUILD_DOMAIN_SCHEMA_VERSION
            || self.revision == 0
            || self.audit_log.len() != self.revision as usize
            || self.audit_log.is_empty()
            || self.created_at > self.updated_at
        {
            return Err(GuildDomainError::InvalidTrustReviewAudit);
        }
        let mut previous_at = self.created_at;
        for (index, entry) in self.audit_log.iter().enumerate() {
            validate_participant(&entry.revised_by)?;
            if !same_participant(&self.reviewer, &entry.revised_by)
                || entry.revision != index as u32 + 1
                || entry.revised_at < previous_at
            {
                return Err(GuildDomainError::InvalidTrustReviewAudit);
            }
            previous_at = entry.revised_at;
        }
        let current = self
            .audit_log
            .last()
            .ok_or(GuildDomainError::InvalidTrustReviewAudit)?;
        if current.score != self.score
            || current.quality_integrity != self.quality_integrity
            || current.revised_at != self.updated_at
            || self.audit_log.first().map(|entry| entry.revised_at) != Some(self.created_at)
        {
            return Err(GuildDomainError::InvalidTrustReviewAudit);
        }
        Ok(())
    }
}

impl TryFrom<TrustReviewWire> for TrustReview {
    type Error = GuildDomainError;

    fn try_from(value: TrustReviewWire) -> Result<Self, Self::Error> {
        let review = Self {
            schema_version: value.schema_version,
            id: value.id,
            subject: value.subject,
            reviewer: value.reviewer,
            mission_id: normalize_optional_text(value.mission_id),
            score: value.score,
            quality_integrity: value.quality_integrity,
            revision: value.revision,
            audit_log: value.audit_log,
            created_at: value.created_at,
            updated_at: value.updated_at,
        };
        review.validate()?;
        Ok(review)
    }
}

/// Social coordination group only. Membership does not create legal identity,
/// KYC, mission eligibility, funding, verification, or payment authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct InformalParty {
    #[serde(default = "guild_domain_schema_version")]
    pub schema_version: String,
    pub id: Id,
    pub name: String,
    pub members: Vec<GuildParticipantRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl InformalParty {
    pub fn new(
        name: impl Into<String>,
        members: Vec<GuildParticipantRef>,
        created_at: DateTime<Utc>,
    ) -> GuildDomainResult<Self> {
        let name = name.into().trim().to_string();
        let mut identifiers = std::collections::BTreeSet::new();
        let members_valid = members.iter().all(|member| {
            validate_participant(member).is_ok()
                && identifiers.insert(member.participant_id.to_ascii_lowercase())
        });
        if name.is_empty() || members.is_empty() || !members_valid {
            return Err(GuildDomainError::InvalidInformalParty);
        }
        Ok(Self {
            schema_version: guild_domain_schema_version(),
            id: Uuid::new_v4(),
            name,
            members,
            created_at,
            updated_at: created_at,
        })
    }
}

fn validate_participant(participant: &GuildParticipantRef) -> GuildDomainResult<()> {
    (!participant.participant_id.trim().is_empty())
        .then_some(())
        .ok_or(GuildDomainError::InvalidParticipantId)
}

fn same_participant(left: &GuildParticipantRef, right: &GuildParticipantRef) -> bool {
    left.kind == right.kind
        && left
            .participant_id
            .eq_ignore_ascii_case(&right.participant_id)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct MissionEligibility {
    pub affiliated_only: bool,
    pub minimum_rank: Option<AdventurerRank>,
    pub minimum_trust_score: Option<TrustScore>,
    /// When omitted alongside a trust-score gate, three reviews are required.
    pub minimum_trust_review_count: Option<u32>,
}

impl MissionEligibility {
    pub fn effective_minimum_trust_review_count(&self) -> u32 {
        match (self.minimum_trust_score, self.minimum_trust_review_count) {
            (Some(_), Some(configured)) => configured.max(1),
            (Some(_), None) => DEFAULT_MISSION_MINIMUM_TRUST_REVIEW_COUNT,
            (None, Some(configured)) => configured,
            (None, None) => 0,
        }
    }

    pub fn evaluate(&self, evidence: &MissionEligibilityEvidence) -> MissionEligibilityDecision {
        let observed_rank = AdventurerRank::from_reputation_points(evidence.reputation_points);
        let mut reasons = Vec::new();
        if self.affiliated_only && !evidence.affiliated {
            reasons.push("mission is limited to affiliation-ready adventurers".to_string());
        }
        if self
            .minimum_rank
            .is_some_and(|minimum| observed_rank < minimum)
        {
            reasons.push(format!(
                "requires rank {} or higher",
                rank_label(self.minimum_rank.expect("checked"))
            ));
        }
        let minimum_reviews = self.effective_minimum_trust_review_count();
        let valid_trust = evidence.trust.validate().is_ok();
        if !valid_trust {
            reasons.push("trust summary is inconsistent with 1-5 reviews".to_string());
        } else {
            if evidence.trust.review_count < minimum_reviews {
                reasons.push(format!("requires at least {minimum_reviews} trust reviews"));
            }
            if let Some(minimum) = self.minimum_trust_score {
                if evidence.trust.review_count >= minimum_reviews
                    && !evidence.trust.average_at_least(minimum)
                {
                    reasons.push(format!(
                        "requires an average trust score of at least {}",
                        minimum.get()
                    ));
                }
            }
        }
        MissionEligibilityDecision {
            eligible: reasons.is_empty(),
            observed_rank,
            reasons,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MissionEligibilityEvidence {
    pub affiliated: bool,
    /// Unsigned by construction; negative JSON values fail deserialization.
    pub reputation_points: u64,
    pub trust: TrustSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MissionEligibilityDecision {
    pub eligible: bool,
    pub observed_rank: AdventurerRank,
    pub reasons: Vec<String>,
}

fn rank_label(rank: AdventurerRank) -> &'static str {
    match rank {
        AdventurerRank::F => "F",
        AdventurerRank::E => "E",
        AdventurerRank::D => "D",
        AdventurerRank::C => "C",
        AdventurerRank::B => "B",
        AdventurerRank::A => "A",
        AdventurerRank::S => "S",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BountyPromiseState {
    Promised,
    Funded,
    Paid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CanonicalFundingEventKind {
    FundingAdded,
    BountyBecameClaimable,
}

/// Exact confirmed canonical funding event. It proves funding, never payment.
/// Construct it only after the configured canonical indexer validates the
/// factory/clone emitter and block confirmation; a well-shaped hash alone is
/// not canonical evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalBaseUsdcFundingEvidence {
    pub network: String,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub event_kind: CanonicalFundingEventKind,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
}

/// Exact confirmed BountySettled event. This is the only variant in the guild
/// model that proves solver payment. Construct it only after the configured
/// canonical indexer validates the factory/clone emitter and confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CanonicalBaseUsdcSettlementEvidence {
    pub network: String,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: u64,
    pub solver: String,
    pub tx_hash: String,
    pub block_number: u64,
    pub log_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "evidence_type", content = "evidence", rename_all = "snake_case")]
pub enum MoneyBountyEvidence {
    PromiseOnly,
    CanonicalBaseUsdcFunding(CanonicalBaseUsdcFundingEvidence),
    CanonicalBaseUsdcSettlement(CanonicalBaseUsdcSettlementEvidence),
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct MoneyBountyPromiseWire {
    amount: Money,
    evidence: MoneyBountyEvidence,
}

/// Money-denominated promise. Its derived state is evidence-bound; canonical
/// funding is not payment, and canonical evidence is accepted only for USDC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "MoneyBountyPromiseWire")]
pub struct MoneyBountyPromise {
    amount: Money,
    evidence: MoneyBountyEvidence,
}

impl MoneyBountyPromise {
    pub fn new(amount: Money, evidence: MoneyBountyEvidence) -> GuildDomainResult<Self> {
        if amount.amount <= 0 || !nonempty_line(&amount.currency) {
            return Err(GuildDomainError::InvalidMoneyPromise);
        }
        let canonical = matches!(
            evidence,
            MoneyBountyEvidence::CanonicalBaseUsdcFunding(_)
                | MoneyBountyEvidence::CanonicalBaseUsdcSettlement(_)
        );
        if canonical && !amount.currency.eq_ignore_ascii_case("usdc") {
            return Err(GuildDomainError::InvalidCanonicalMoneyPromise);
        }
        validate_money_evidence(&evidence)?;
        Ok(Self { amount, evidence })
    }

    pub fn amount(&self) -> &Money {
        &self.amount
    }

    pub fn evidence(&self) -> &MoneyBountyEvidence {
        &self.evidence
    }

    pub const fn state(&self) -> BountyPromiseState {
        match self.evidence {
            MoneyBountyEvidence::PromiseOnly => BountyPromiseState::Promised,
            MoneyBountyEvidence::CanonicalBaseUsdcFunding(_) => BountyPromiseState::Funded,
            MoneyBountyEvidence::CanonicalBaseUsdcSettlement(_) => BountyPromiseState::Paid,
        }
    }
}

impl TryFrom<MoneyBountyPromiseWire> for MoneyBountyPromise {
    type Error = GuildDomainError;

    fn try_from(value: MoneyBountyPromiseWire) -> Result<Self, Self::Error> {
        Self::new(value.amount, value.evidence)
    }
}

fn validate_money_evidence(evidence: &MoneyBountyEvidence) -> GuildDomainResult<()> {
    let fields_valid = match evidence {
        MoneyBountyEvidence::PromiseOnly => true,
        MoneyBountyEvidence::CanonicalBaseUsdcFunding(value) => {
            canonical_base_network(&value.network)
                && fixed_hex(&value.bounty_contract, 40)
                && fixed_hex(&value.bounty_id, 64)
                && fixed_hex(&value.tx_hash, 64)
        }
        MoneyBountyEvidence::CanonicalBaseUsdcSettlement(value) => {
            value.round > 0
                && canonical_base_network(&value.network)
                && fixed_hex(&value.bounty_contract, 40)
                && fixed_hex(&value.bounty_id, 64)
                && fixed_hex(&value.solver, 40)
                && fixed_hex(&value.tx_hash, 64)
        }
    };
    fields_valid
        .then_some(())
        .ok_or(GuildDomainError::InvalidCanonicalMoneyPromise)
}

fn canonical_base_network(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "base-mainnet" | "base-sepolia"
    )
}

fn fixed_hex(value: &str, digits: usize) -> bool {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .is_some_and(|hex| hex.len() == digits && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct OtherAssetBountyPromiseWire {
    asset_identifier: String,
    quantity: String,
    note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(try_from = "OtherAssetBountyPromiseWire")]
pub struct OtherAssetBountyPromise {
    pub asset_identifier: String,
    pub quantity: String,
    pub note: Option<String>,
}

impl OtherAssetBountyPromise {
    pub fn new(
        asset_identifier: impl Into<String>,
        quantity: impl Into<String>,
        note: Option<String>,
    ) -> GuildDomainResult<Self> {
        let asset_identifier = asset_identifier.into().trim().to_string();
        let quantity = quantity.into().trim().to_string();
        if !nonempty_line(&asset_identifier) || !nonempty_line(&quantity) {
            return Err(GuildDomainError::InvalidOtherAssetPromise);
        }
        Ok(Self {
            asset_identifier,
            quantity,
            note: normalize_optional_text(note),
        })
    }

    /// Other assets are presentation-only promises in v1. They cannot become
    /// funded or paid through this model.
    pub const fn state(&self) -> BountyPromiseState {
        BountyPromiseState::Promised
    }
}

impl TryFrom<OtherAssetBountyPromiseWire> for OtherAssetBountyPromise {
    type Error = GuildDomainError;

    fn try_from(value: OtherAssetBountyPromiseWire) -> Result<Self, Self::Error> {
        Self::new(value.asset_identifier, value.quantity, value.note)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "asset_type", content = "details", rename_all = "snake_case")]
pub enum GuildBountyPromise {
    Money(MoneyBountyPromise),
    OtherAsset(OtherAssetBountyPromise),
}

impl GuildBountyPromise {
    pub const fn state(&self) -> BountyPromiseState {
        match self {
            Self::Money(promise) => promise.state(),
            Self::OtherAsset(promise) => promise.state(),
        }
    }
}

fn nonempty_line(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains(['\r', '\n'])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct GuildMission {
    #[serde(default = "guild_domain_schema_version")]
    pub schema_version: String,
    pub id: Id,
    pub title: String,
    pub difficulty: MissionDifficulty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligibility: Option<MissionEligibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounty_promise: Option<GuildBountyPromise>,
    pub created_at: DateTime<Utc>,
}

impl GuildMission {
    pub fn new(
        title: impl Into<String>,
        difficulty: MissionDifficulty,
        created_at: DateTime<Utc>,
    ) -> GuildDomainResult<Self> {
        let title = title.into().trim().to_string();
        if title.is_empty() {
            return Err(GuildDomainError::InvalidMissionTitle);
        }
        Ok(Self {
            schema_version: guild_domain_schema_version(),
            id: Uuid::new_v4(),
            title,
            difficulty,
            eligibility: None,
            bounty_promise: None,
            created_at,
        })
    }

    /// Difficulty is intentionally ignored. Only an explicit eligibility block
    /// can gate a mission.
    pub fn evaluate_eligibility(
        &self,
        evidence: &MissionEligibilityEvidence,
    ) -> MissionEligibilityDecision {
        self.eligibility.as_ref().map_or_else(
            || MissionEligibilityDecision {
                eligible: true,
                observed_rank: AdventurerRank::from_reputation_points(evidence.reputation_points),
                reasons: Vec::new(),
            },
            |eligibility| eligibility.evaluate(evidence),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HumanKycStatus {
    NotApplicable,
    NotStarted,
    Pending,
    Verified,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlatformSandboxStatus {
    NotEvaluated,
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct HarnessModelAnalysisEvidence {
    pub harness_reference: String,
    pub harness_passed: bool,
    pub model_reference: String,
    pub model_analysis_passed: bool,
    pub evidence_hash: String,
    pub observed_at: DateTime<Utc>,
}

impl HarnessModelAnalysisEvidence {
    fn harness_ready(&self) -> bool {
        self.harness_passed
            && nonempty_line(&self.harness_reference)
            && nonempty_line(&self.evidence_hash)
    }

    fn model_ready(&self) -> bool {
        self.model_analysis_passed
            && nonempty_line(&self.model_reference)
            && nonempty_line(&self.evidence_hash)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct AffiliationPolicy {
    pub completed_task_threshold: u32,
    pub minimum_trust_review_count: u32,
    pub platform_sandbox_required: bool,
    pub human_kyc_required_for_humans: bool,
}

impl Default for AffiliationPolicy {
    fn default() -> Self {
        Self {
            completed_task_threshold: DEFAULT_AFFILIATION_COMPLETED_TASK_THRESHOLD,
            minimum_trust_review_count: DEFAULT_AFFILIATION_MINIMUM_TRUST_REVIEW_COUNT,
            platform_sandbox_required: true,
            human_kyc_required_for_humans: true,
        }
    }
}

/// Evidence used only to decide guild affiliation readiness. It does not prove
/// identity beyond the stated KYC status and never authorizes mission payment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AffiliationEvidence {
    #[serde(default = "guild_domain_schema_version")]
    pub schema_version: String,
    pub subject: GuildParticipantRef,
    pub harness_model_analysis: Option<HarnessModelAnalysisEvidence>,
    pub human_kyc_status: HumanKycStatus,
    pub platform_sandbox_status: PlatformSandboxStatus,
    pub completed_task_count: u32,
    pub trust: TrustSummary,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct AffiliationReadiness {
    pub ready: bool,
    pub harness_ready: bool,
    pub model_analysis_ready: bool,
    pub human_kyc_ready: bool,
    pub platform_sandbox_ready: bool,
    pub completed_task_ready: bool,
    pub trust_ready: bool,
    pub completed_task_threshold: u32,
    pub minimum_trust_review_count: u32,
    pub trust_threshold: String,
    pub reasons: Vec<String>,
}

impl AffiliationPolicy {
    pub fn evaluate(&self, evidence: &AffiliationEvidence) -> AffiliationReadiness {
        let harness_ready = evidence
            .harness_model_analysis
            .as_ref()
            .is_some_and(HarnessModelAnalysisEvidence::harness_ready);
        let model_analysis_ready = evidence
            .harness_model_analysis
            .as_ref()
            .is_some_and(HarnessModelAnalysisEvidence::model_ready);
        let human_kyc_ready = evidence.subject.kind != GuildParticipantKind::Human
            || !self.human_kyc_required_for_humans
            || evidence.human_kyc_status == HumanKycStatus::Verified;
        let platform_sandbox_ready = !self.platform_sandbox_required
            || evidence.platform_sandbox_status == PlatformSandboxStatus::Passed;
        let completed_task_ready = evidence.completed_task_count >= self.completed_task_threshold;
        let trust_valid = evidence.trust.validate().is_ok();
        let trust_ready = trust_valid
            && evidence.trust.review_count >= self.minimum_trust_review_count.max(1)
            && evidence.trust.average_strictly_exceeds_four();
        let mut reasons = Vec::new();
        if !harness_ready {
            reasons.push("passing harness evidence is required".to_string());
        }
        if !model_analysis_ready {
            reasons.push("passing model analysis evidence is required".to_string());
        }
        if !human_kyc_ready {
            reasons.push("verified human KYC is required where applicable".to_string());
        }
        if !platform_sandbox_ready {
            reasons.push("the platform sandbox requirement is not satisfied".to_string());
        }
        if !completed_task_ready {
            reasons.push(format!(
                "requires at least {} completed tasks",
                self.completed_task_threshold
            ));
        }
        if !trust_valid {
            reasons.push("trust summary is inconsistent with 1-5 reviews".to_string());
        } else if evidence.trust.review_count < self.minimum_trust_review_count.max(1) {
            reasons.push(format!(
                "requires at least {} trust reviews",
                self.minimum_trust_review_count.max(1)
            ));
        } else if !evidence.trust.average_strictly_exceeds_four() {
            reasons.push("average trust score must be strictly greater than 4".to_string());
        }
        AffiliationReadiness {
            ready: reasons.is_empty(),
            harness_ready,
            model_analysis_ready,
            human_kyc_ready,
            platform_sandbox_ready,
            completed_task_ready,
            trust_ready,
            completed_task_threshold: self.completed_task_threshold,
            minimum_trust_review_count: self.minimum_trust_review_count.max(1),
            trust_threshold: "strictly_greater_than_4".to_string(),
            reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use serde_json::json;

    fn observed_at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0)
            .single()
            .expect("valid test timestamp")
    }

    fn participant(id: &str, kind: GuildParticipantKind) -> GuildParticipantRef {
        GuildParticipantRef::new(id, kind, None).expect("valid participant")
    }

    fn passing_analysis(at: DateTime<Utc>) -> HarnessModelAnalysisEvidence {
        HarnessModelAnalysisEvidence {
            harness_reference: "harness/run/42".to_string(),
            harness_passed: true,
            model_reference: "model/analysis/42".to_string(),
            model_analysis_passed: true,
            evidence_hash: "sha256:abc123".to_string(),
            observed_at: at,
        }
    }

    fn eligibility_evidence(
        affiliated: bool,
        reputation_points: u64,
        score_total: u64,
        review_count: u32,
    ) -> MissionEligibilityEvidence {
        MissionEligibilityEvidence {
            affiliated,
            reputation_points,
            trust: TrustSummary {
                score_total,
                review_count,
            },
        }
    }

    #[test]
    fn rank_schedule_has_explicit_stable_boundaries() {
        let cases = [
            (0, AdventurerRank::F),
            (99, AdventurerRank::F),
            (100, AdventurerRank::E),
            (249, AdventurerRank::E),
            (250, AdventurerRank::D),
            (499, AdventurerRank::D),
            (500, AdventurerRank::C),
            (999, AdventurerRank::C),
            (1_000, AdventurerRank::B),
            (2_499, AdventurerRank::B),
            (2_500, AdventurerRank::A),
            (4_999, AdventurerRank::A),
            (5_000, AdventurerRank::S),
            (u64::MAX, AdventurerRank::S),
        ];
        for (points, expected) in cases {
            assert_eq!(AdventurerRank::from_reputation_points(points), expected);
        }
        assert_eq!(
            DEFAULT_ADVENTURER_RANK_THRESHOLDS.map(|entry| entry.minimum_reputation_points),
            [0, 100, 250, 500, 1_000, 2_500, 5_000]
        );
        assert_eq!(serde_json::to_string(&AdventurerRank::S).unwrap(), "\"S\"");
        assert!(serde_json::from_value::<MissionEligibilityEvidence>(json!({
            "affiliated": false,
            "reputation_points": -1,
            "trust": { "score_total": 0, "review_count": 0 }
        }))
        .is_err());
    }

    #[test]
    fn mission_difficulty_never_creates_an_implicit_gate() {
        let at = observed_at();
        let novice = eligibility_evidence(false, 0, 0, 0);
        let mut mission = GuildMission::new("Document a small fix", MissionDifficulty::S, at)
            .expect("valid mission");

        let ungated = mission.evaluate_eligibility(&novice);
        assert!(ungated.eligible);
        assert_eq!(ungated.observed_rank, AdventurerRank::F);

        mission.difficulty = MissionDifficulty::F;
        assert_eq!(mission.evaluate_eligibility(&novice), ungated);

        mission.difficulty = MissionDifficulty::S;
        mission.eligibility = Some(MissionEligibility {
            minimum_rank: Some(AdventurerRank::E),
            ..MissionEligibility::default()
        });
        let explicitly_gated = mission.evaluate_eligibility(&novice);
        assert!(!explicitly_gated.eligible);
        assert_eq!(explicitly_gated.reasons, vec!["requires rank E or higher"]);
    }

    #[test]
    fn trust_review_mutation_is_validated_and_append_only() {
        let at = observed_at();
        let subject = participant("agent:solver", GuildParticipantKind::Agent);
        let reviewer = participant("human:reviewer", GuildParticipantKind::Human);

        assert_eq!(TrustScore::new(0), Err(GuildDomainError::InvalidTrustScore));
        assert_eq!(TrustScore::new(6), Err(GuildDomainError::InvalidTrustScore));
        assert_eq!(
            TrustReviewSentence::new("line one\nline two"),
            Err(GuildDomainError::InvalidTrustReviewSentence)
        );
        assert_eq!(
            TrustReviewSentence::new("x".repeat(281)),
            Err(GuildDomainError::InvalidTrustReviewSentence)
        );

        let mut review = TrustReview::new(
            subject,
            reviewer.clone(),
            Some("mission:7".to_string()),
            4,
            "Good work with clear evidence.",
            at,
        )
        .expect("valid review");
        assert_eq!(review.revision(), 1);
        assert_eq!(review.audit_log().len(), 1);
        assert_eq!(review.audit_log()[0].revised_by, reviewer);

        let revised_at = at + Duration::minutes(5);
        review
            .revise(
                5,
                "Excellent quality and complete evidence.",
                reviewer.clone(),
                Some("Evidence was expanded.".to_string()),
                revised_at,
            )
            .expect("valid revision");
        assert_eq!(review.revision(), 2);
        assert_eq!(review.score().get(), 5);
        assert_eq!(review.audit_log().len(), 2);
        assert_eq!(review.audit_log()[0].score.get(), 4);
        assert_eq!(review.audit_log()[1].revision, 2);
        review.validate().expect("consistent audit");

        let encoded = serde_json::to_value(&review).unwrap();
        let decoded: TrustReview = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded, review);

        let mut tampered = encoded;
        tampered["revision"] = json!(3);
        assert!(serde_json::from_value::<TrustReview>(tampered).is_err());
        assert_eq!(
            review.revise(
                5,
                "A different participant cannot rewrite this review.",
                participant("human:maintainer", GuildParticipantKind::Human),
                None,
                revised_at + Duration::seconds(1),
            ),
            Err(GuildDomainError::InvalidTrustReviewAuthor)
        );
        assert_eq!(
            review.revise(
                5,
                "Cannot predate the audit.",
                reviewer.clone(),
                None,
                at - Duration::seconds(1),
            ),
            Err(GuildDomainError::InvalidRevisionTimestamp)
        );
        assert_eq!(
            TrustReview::new(reviewer.clone(), reviewer, None, 5, "Self review.", at,),
            Err(GuildDomainError::InvalidTrustReviewAuthor)
        );
    }

    #[test]
    fn explicit_eligibility_uses_rank_affiliation_and_review_floor() {
        let policy = MissionEligibility {
            affiliated_only: true,
            minimum_rank: Some(AdventurerRank::C),
            minimum_trust_score: Some(TrustScore::new(4).unwrap()),
            minimum_trust_review_count: None,
        };
        assert_eq!(
            policy.effective_minimum_trust_review_count(),
            DEFAULT_MISSION_MINIMUM_TRUST_REVIEW_COUNT
        );

        let blocked = policy.evaluate(&eligibility_evidence(false, 499, 10, 2));
        assert!(!blocked.eligible);
        assert_eq!(blocked.observed_rank, AdventurerRank::D);
        assert_eq!(
            blocked.reasons,
            vec![
                "mission is limited to affiliation-ready adventurers",
                "requires rank C or higher",
                "requires at least 3 trust reviews",
            ]
        );

        let eligible = policy.evaluate(&eligibility_evidence(true, 500, 12, 3));
        assert!(eligible.eligible);
        assert!(eligible.reasons.is_empty());

        let low_average = policy.evaluate(&eligibility_evidence(true, 500, 11, 3));
        assert_eq!(
            low_average.reasons,
            vec!["requires an average trust score of at least 4"]
        );
    }

    #[test]
    fn informal_parties_are_versioned_social_groups() {
        let at = observed_at();
        let members = vec![
            participant("agent:one", GuildParticipantKind::Agent),
            participant("human:two", GuildParticipantKind::Human),
        ];
        let party = InformalParty::new("Night Shift", members, at).expect("valid party");
        assert_eq!(party.schema_version, GUILD_DOMAIN_SCHEMA_VERSION);
        assert_eq!(party.members.len(), 2);

        let duplicates = vec![
            participant("Agent:One", GuildParticipantKind::Agent),
            participant("agent:one", GuildParticipantKind::Agent),
        ];
        assert_eq!(
            InformalParty::new("Duplicate", duplicates, at),
            Err(GuildDomainError::InvalidInformalParty)
        );
        assert_eq!(
            InformalParty::new("Empty", Vec::new(), at),
            Err(GuildDomainError::InvalidInformalParty)
        );

        let encoded = serde_json::to_value(&party).unwrap();
        assert!(encoded.get("funded").is_none());
        assert!(encoded.get("paid").is_none());
        assert!(encoded.get("eligibility").is_none());
    }

    #[test]
    fn only_canonical_base_usdc_evidence_advances_money_state() {
        assert_eq!(
            MoneyBountyPromise::new(
                Money {
                    amount: 10,
                    currency: " ".to_string(),
                },
                MoneyBountyEvidence::PromiseOnly,
            ),
            Err(GuildDomainError::InvalidMoneyPromise)
        );
        let promised = MoneyBountyPromise::new(
            Money::new(2_500, "usd").unwrap(),
            MoneyBountyEvidence::PromiseOnly,
        )
        .expect("arbitrary money may be promised");
        assert_eq!(promised.state(), BountyPromiseState::Promised);

        let funding =
            MoneyBountyEvidence::CanonicalBaseUsdcFunding(CanonicalBaseUsdcFundingEvidence {
                network: "base-mainnet".to_string(),
                bounty_contract: format!("0x{}", "ab".repeat(20)),
                bounty_id: format!("0x{}", "cd".repeat(32)),
                event_kind: CanonicalFundingEventKind::BountyBecameClaimable,
                tx_hash: format!("0x{}", "ef".repeat(32)),
                block_number: 100,
                log_index: 2,
            });
        let funded = MoneyBountyPromise::new(Money::new(1_000_000, "USDC").unwrap(), funding)
            .expect("canonical USDC funding");
        assert_eq!(funded.state(), BountyPromiseState::Funded);

        let settlement =
            MoneyBountyEvidence::CanonicalBaseUsdcSettlement(CanonicalBaseUsdcSettlementEvidence {
                network: "base-mainnet".to_string(),
                bounty_contract: format!("0x{}", "ab".repeat(20)),
                bounty_id: format!("0x{}", "cd".repeat(32)),
                round: 1,
                solver: format!("0x{}", "12".repeat(20)),
                tx_hash: format!("0x{}", "34".repeat(32)),
                block_number: 120,
                log_index: 5,
            });
        let paid =
            MoneyBountyPromise::new(Money::new(1_000_000, "usdc").unwrap(), settlement.clone())
                .expect("canonical USDC settlement");
        assert_eq!(paid.state(), BountyPromiseState::Paid);

        assert_eq!(
            MoneyBountyPromise::new(Money::new(100, "usd").unwrap(), settlement),
            Err(GuildDomainError::InvalidCanonicalMoneyPromise)
        );
        let encoded_paid = serde_json::to_value(&paid).unwrap();
        let mut wrong_currency = encoded_paid.clone();
        wrong_currency["amount"]["currency"] = json!("eth");
        assert!(serde_json::from_value::<MoneyBountyPromise>(wrong_currency).is_err());
        let mut wrong_network = encoded_paid;
        wrong_network["evidence"]["evidence"]["network"] = json!("ethereum-mainnet");
        assert!(serde_json::from_value::<MoneyBountyPromise>(wrong_network).is_err());

        let other = OtherAssetBountyPromise::new(
            "conference-ticket",
            "1",
            Some("Subject to issuer terms".to_string()),
        )
        .expect("valid asset promise");
        assert_eq!(other.state(), BountyPromiseState::Promised);
        let encoded = serde_json::to_string(&GuildBountyPromise::OtherAsset(other)).unwrap();
        assert!(!encoded.contains("funded"));
        assert!(!encoded.contains("paid"));
        assert!(serde_json::from_value::<OtherAssetBountyPromise>(json!({
            "asset_identifier": "",
            "quantity": "1",
            "note": null
        }))
        .is_err());
    }

    #[test]
    fn affiliation_readiness_requires_all_default_evidence() {
        let at = observed_at();
        let policy = AffiliationPolicy::default();
        let mut evidence = AffiliationEvidence {
            schema_version: guild_domain_schema_version(),
            subject: participant("agent:ready", GuildParticipantKind::Agent),
            harness_model_analysis: Some(passing_analysis(at)),
            human_kyc_status: HumanKycStatus::NotApplicable,
            platform_sandbox_status: PlatformSandboxStatus::Passed,
            completed_task_count: DEFAULT_AFFILIATION_COMPLETED_TASK_THRESHOLD,
            trust: TrustSummary::new(13, 3).unwrap(),
            observed_at: at,
        };

        let ready = policy.evaluate(&evidence);
        assert!(ready.ready);
        assert!(ready.harness_ready);
        assert!(ready.model_analysis_ready);
        assert!(ready.human_kyc_ready);
        assert!(ready.platform_sandbox_ready);
        assert!(ready.completed_task_ready);
        assert!(ready.trust_ready);
        assert_eq!(ready.trust_threshold, "strictly_greater_than_4");

        evidence.trust = TrustSummary::new(12, 3).unwrap();
        let exactly_four = policy.evaluate(&evidence);
        assert!(!exactly_four.ready);
        assert!(!exactly_four.trust_ready);
        assert_eq!(
            exactly_four.reasons,
            vec!["average trust score must be strictly greater than 4"]
        );

        evidence.trust = TrustSummary::new(13, 3).unwrap();
        evidence.subject = participant("human:ready", GuildParticipantKind::Human);
        evidence.human_kyc_status = HumanKycStatus::Pending;
        let pending_kyc = policy.evaluate(&evidence);
        assert!(!pending_kyc.ready);
        assert!(!pending_kyc.human_kyc_ready);

        evidence.human_kyc_status = HumanKycStatus::Verified;
        assert!(policy.evaluate(&evidence).ready);

        evidence.platform_sandbox_status = PlatformSandboxStatus::Failed;
        evidence.completed_task_count =
            DEFAULT_AFFILIATION_COMPLETED_TASK_THRESHOLD.saturating_sub(1);
        evidence
            .harness_model_analysis
            .as_mut()
            .unwrap()
            .model_analysis_passed = false;
        let blocked = policy.evaluate(&evidence);
        assert!(!blocked.ready);
        assert!(!blocked.model_analysis_ready);
        assert!(!blocked.platform_sandbox_ready);
        assert!(!blocked.completed_task_ready);
    }

    #[test]
    fn version_defaults_make_new_mission_fields_backward_compatible() {
        let mission =
            GuildMission::new("Legacy-shaped mission", MissionDifficulty::D, observed_at())
                .expect("valid mission");
        let encoded = serde_json::to_value(&mission).unwrap();
        assert_eq!(
            encoded["schema_version"],
            json!(GUILD_DOMAIN_SCHEMA_VERSION)
        );
        assert!(encoded.get("eligibility").is_none());
        assert!(encoded.get("bounty_promise").is_none());

        let mut legacy_shaped = encoded;
        legacy_shaped
            .as_object_mut()
            .unwrap()
            .remove("schema_version");
        let decoded: GuildMission = serde_json::from_value(legacy_shaped).unwrap();
        assert_eq!(decoded.schema_version, GUILD_DOMAIN_SCHEMA_VERSION);
        assert_eq!(decoded.eligibility, None);
        assert_eq!(decoded.bounty_promise, None);
    }
}
