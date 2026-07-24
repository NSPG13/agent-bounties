use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind};
use chrono::{DateTime, Utc};
use domain::{
    Agent, AgentEligibilityDecision, AgentEligibilityEvidence, AgentStatus, AgentWebhookEventType,
    AudienceInteraction, AudienceInteractionKind, AudienceLifecycleStage, AudienceMember,
    AudienceProvider, AutonomousBountyTermsDocument, AutonomousBountyTermsRecord,
    AutonomousSubmissionEvidenceRecord, BondSponsorship, BondSponsorshipStatus, Bounty,
    BountyStatus, CanonicalSolverCompletion, Capability, CapabilityClass, Claim, ClaimCandidate,
    ClaimCandidateStatus, ContributorContact, DiscoveryResponse, Escrow, EscrowStatus, EvalRun,
    FundingContribution, FundingContributionStatus, FundingIntent, FundingIntentStatus,
    FundingMode, HelpRequest, Id, Money, Objective, ObjectiveStatus, OutreachAttempt,
    OutreachChannel, OutreachStatus, PaymentEvent, PaymentEventStatus, PaymentRail, PrivacyLevel,
    ProofRecord, Quote, ReputationEvent, RiskAction, RiskEvent, RiskReviewOutcome,
    RiskReviewRecord, RiskSurface, Settlement, Submission, TemplateSignal, VerificationDecision,
    VerifierKind, VerifierResult,
};
use ledger::{LedgerEntry, Posting};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;
use uuid::Uuid;

pub const CORE_MIGRATION: &str = include_str!("../../../migrations/0001_core.sql");
pub const AUTONOMOUS_PROTOCOL_MIGRATION: &str =
    include_str!("../../../migrations/0002_autonomous_protocol.sql");
pub const X402_RELAYER_MIGRATION: &str = include_str!("../../../migrations/0003_x402_relayer.sql");
pub const AGENT_COORDINATION_MIGRATION: &str =
    include_str!("../../../migrations/0004_agent_coordination.sql");
pub const TRIAL_BOUNTIES_MIGRATION: &str =
    include_str!("../../../migrations/0005_trial_bounties.sql");
pub const SOLVER_LEADERBOARD_MIGRATION: &str =
    include_str!("../../../migrations/0006_solver_leaderboard.sql");
pub const DISCOVERY_SUBSCRIPTIONS_MIGRATION: &str =
    include_str!("../../../migrations/0007_discovery_subscriptions.sql");
pub const OPPORTUNITY_CONVERSION_MIGRATION: &str =
    include_str!("../../../migrations/0008_opportunity_conversion.sql");
pub const LEGAL_ACCEPTANCES_MIGRATION: &str =
    include_str!("../../../migrations/0009_legal_acceptances.sql");
pub const SITE_ANALYTICS_MIGRATION: &str =
    include_str!("../../../migrations/0010_site_analytics.sql");
pub const SOCIAL_MENTION_INGESTION_MIGRATION: &str =
    include_str!("../../../migrations/0011_social_mention_ingestion.sql");
pub const COMPETITOR_INTELLIGENCE_MIGRATION: &str =
    include_str!("../../../migrations/0012_competitor_intelligence.sql");
pub const OBJECTIVE_COORDINATION_MIGRATION: &str =
    include_str!("../../../migrations/0013_objective_coordination.sql");
const MIGRATION_ADVISORY_LOCK_ID: i64 = 4_270_265_017;
const UPSERT_PAYMENT_EVENT_SQL: &str = r#"
            INSERT INTO payment_events (id, rail, external_id, status, payload_hash, received_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (external_id) DO UPDATE SET
              rail = CASE
                WHEN payment_events.status = 'Applied' THEN payment_events.rail
                ELSE EXCLUDED.rail
              END,
              status = CASE
                WHEN payment_events.status = 'Applied' THEN payment_events.status
                ELSE EXCLUDED.status
              END,
              payload_hash = CASE
                WHEN payment_events.status = 'Applied' THEN payment_events.payload_hash
                ELSE EXCLUDED.payload_hash
              END,
              received_at = CASE
                WHEN payment_events.status = 'Applied' THEN payment_events.received_at
                ELSE EXCLUDED.received_at
              END
            "#;
const UPSERT_AUDIENCE_MEMBER_SQL: &str = r#"
            INSERT INTO audience_members
              (id, provider, external_id, external_id_normalized, handle, public_profile_url, roles, lifecycle_stage, first_seen_at, last_seen_at)
            VALUES ($1, $2, $3, lower($3), $4, $5, $6, $7, $8, $9)
            ON CONFLICT (provider, external_id_normalized) DO UPDATE SET
              external_id = EXCLUDED.external_id,
              handle = EXCLUDED.handle,
              public_profile_url = COALESCE(EXCLUDED.public_profile_url, audience_members.public_profile_url),
              roles = (
                SELECT COALESCE(jsonb_agg(role ORDER BY role::text), '[]'::jsonb)
                FROM (
                  SELECT DISTINCT role
                  FROM jsonb_array_elements(audience_members.roles || EXCLUDED.roles) AS merged(role)
                ) AS unique_roles
              ),
              lifecycle_stage = CASE
                WHEN audience_members.lifecycle_stage = 'Retained' OR EXCLUDED.lifecycle_stage = 'Retained' THEN 'Retained'
                WHEN audience_members.lifecycle_stage = 'Converted' OR EXCLUDED.lifecycle_stage = 'Converted' THEN 'Converted'
                WHEN audience_members.lifecycle_stage = 'Engaged' OR EXCLUDED.lifecycle_stage = 'Engaged' THEN 'Engaged'
                ELSE 'Observed'
              END,
              first_seen_at = LEAST(audience_members.first_seen_at, EXCLUDED.first_seen_at),
              last_seen_at = GREATEST(audience_members.last_seen_at, EXCLUDED.last_seen_at)
            "#;
const INSERT_AUDIENCE_INTERACTION_SQL: &str = r#"
            INSERT INTO audience_interactions
              (id, audience_member_id, provider_event_id, kind, public_url, occurred_at, referrer_url, campaign, source_interaction_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (audience_member_id, provider_event_id) DO NOTHING
            "#;
const CLAIM_CANDIDATE_SELECT_BY_IDEMPOTENCY_SQL: &str = r#"
            SELECT id, idempotency_key, network, bounty_contract, solver_wallet,
                   agent_id, eligibility_evidence, eligibility_decision, status,
                   exclusive_until, authorization_nonce, authorization_valid_before,
                   claim_transaction_hash, canonical_event_id, failure_code,
                   failure_message, created_at, updated_at
            FROM claim_candidates
            WHERE idempotency_key = $1
            "#;
const ACTIVE_CLAIM_CANDIDATE_SELECT_SQL: &str = r#"
            SELECT id, idempotency_key, network, bounty_contract, solver_wallet,
                   agent_id, eligibility_evidence, eligibility_decision, status,
                   exclusive_until, authorization_nonce, authorization_valid_before,
                   claim_transaction_hash, canonical_event_id, failure_code,
                   failure_message, created_at, updated_at
            FROM claim_candidates
            WHERE network = $1 AND bounty_contract = $2 AND solver_wallet = $3
              AND status IN (
                'waitlisted', 'exclusive', 'sponsoring', 'authorization_ready', 'relaying'
              )
            "#;
const BOND_SPONSORSHIP_SELECT_BY_CANDIDATE_SQL: &str = r#"
            SELECT id, claim_candidate_id, network, bounty_contract, solver_wallet,
                   sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                   failure_code, failure_message, created_at, updated_at
            FROM bond_sponsorships WHERE claim_candidate_id = $1
            "#;

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Domain(#[from] domain::DomainError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("invalid persisted enum value: {0}")]
    InvalidEnum(String),
    #[error("integer value cannot fit target type: {0}")]
    IntegerOverflow(String),
    #[error("conflicting audience event replay: {0}")]
    AudienceConflict(String),
    #[error("conflicting autonomous submission evidence replay: {0}")]
    AutonomousEvidenceConflict(String),
    #[error("conflicting x402 relay replay: {0}")]
    X402RelayConflict(String),
    #[error("x402 hosted relay quota exceeded: {0}")]
    X402RelayQuotaExceeded(String),
    #[error("objective {0} already exists")]
    ObjectiveAlreadyExists(Id),
    #[error("objective {0} was not found")]
    ObjectiveNotFound(Id),
    #[error("objective {objective_id} revision conflict: expected {expected_revision}")]
    ObjectiveRevisionConflict {
        objective_id: Id,
        expected_revision: u64,
    },
    #[error("claim candidate conflict: {0}")]
    ClaimCandidateConflict(String),
    #[error("claim waitlist is full")]
    ClaimWaitlistFull,
    #[error("trial bounty idempotency conflict")]
    TrialBountyConflict,
    #[error("unfunded bounty is unavailable for solutions")]
    UnfundedBountyUnavailable,
    #[error("bond sponsorship quota exceeded: {0}")]
    BondSponsorshipQuotaExceeded(String),
    #[error("opportunity conversion correlation conflict: {0}")]
    OpportunityConversionConflict(String),
}

pub type DbResult<T> = Result<T, DbError>;

#[derive(Debug, Clone)]
pub struct NewLegalAcceptance {
    pub id: Uuid,
    pub terms_version: String,
    pub privacy_version: String,
    pub action: String,
    pub wallet_address: String,
    pub statement_hash: String,
    pub acceptance_method: String,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalAcceptance {
    pub id: Uuid,
    pub terms_version: String,
    pub privacy_version: String,
    pub action: String,
    pub wallet_address: String,
    pub statement_hash: String,
    pub acceptance_method: String,
    pub accepted_at: DateTime<Utc>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewTrialBounty {
    pub id: Uuid,
    pub idempotency_key: String,
    pub request_fingerprint: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub source_url: Option<String>,
    pub discovery_source: String,
    pub status: String,
    pub demo_agent_solution: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrialBounty {
    pub id: Uuid,
    pub idempotency_key: String,
    pub request_fingerprint: String,
    pub title: String,
    pub goal: String,
    pub acceptance_criteria: Vec<String>,
    pub source_url: Option<String>,
    pub discovery_source: String,
    pub status: String,
    pub demo_agent_solution: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUnfundedBountySolution {
    pub id: Uuid,
    pub trial_bounty_id: Uuid,
    pub agent_id: Uuid,
    pub summary: String,
    pub deliverable_markdown: String,
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnfundedBountySolution {
    pub id: Uuid,
    pub trial_bounty_id: Uuid,
    pub agent_id: Uuid,
    pub summary: String,
    pub deliverable_markdown: String,
    pub evidence: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimFunnelStageCounts {
    pub observed: u64,
    pub unique_solver_wallets: u64,
    pub waitlisted_current: u64,
    pub exclusive_current: u64,
    pub authorization_ready_current: u64,
    pub relaying_current: u64,
    pub authorization_prepared: u64,
    pub transaction_broadcast: u64,
    pub claimed_canonical: u64,
    pub superseded: u64,
    pub withdrawn: u64,
    pub failed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimSponsorshipFunnelCounts {
    pub reserved: u64,
    pub broadcast: u64,
    pub confirmed: u64,
    pub failed: u64,
    pub sponsored_claims_confirmed: u64,
    pub direct_claims_confirmed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CanonicalClaimOutcomeCounts {
    pub claims_confirmed: u64,
    pub unique_claimed_solver_wallets: u64,
    pub hosted_claims_confirmed: u64,
    pub unattributed_claims_confirmed: u64,
    pub submissions_confirmed: u64,
    pub settlements_confirmed: u64,
    pub unique_paid_solver_wallets: u64,
    pub repeat_paid_solver_wallets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClaimFunnelStats {
    pub schema_version: String,
    pub window_hours: u32,
    pub window_started_at: DateTime<Utc>,
    pub generated_at: DateTime<Utc>,
    pub stages: ClaimFunnelStageCounts,
    pub sponsorship: ClaimSponsorshipFunnelCounts,
    pub canonical_outcomes: CanonicalClaimOutcomeCounts,
    pub failure_codes: BTreeMap<String, u64>,
    pub evidence_boundary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpportunityLifecycleStats {
    pub published: u64,
    pub solution_received: u64,
    pub funding_prepared: u64,
    pub wallet_signed_observed: u64,
    pub canonical_created: u64,
    pub funded: u64,
    pub claimed: u64,
    pub submitted: u64,
    pub settled: u64,
    pub average_seconds_to_first_solution: Option<f64>,
    pub median_seconds_to_first_solution: Option<f64>,
    pub average_seconds_creation_to_settlement: Option<f64>,
    pub canonical_created_in_window: u64,
    pub canonical_claimed_in_window: u64,
    pub canonical_settled_in_window: u64,
    pub unique_canonical_poster_wallets: u64,
    pub repeat_canonical_poster_wallets: u64,
    pub unique_paid_solver_wallets: u64,
    pub repeat_paid_solver_wallets: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSiteAnalyticsEvent {
    pub event_id: Uuid,
    pub visitor_id: Uuid,
    pub session_id: Uuid,
    pub event_name: String,
    pub page_path: String,
    pub source: Option<String>,
    pub campaign: Option<String>,
    pub referrer_host: Option<String>,
    pub opportunity_id: Option<String>,
    pub bounty_contract: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsOverview {
    pub unique_visitors: u64,
    pub returning_visitors: u64,
    pub sessions: u64,
    pub page_views: u64,
    pub first_event_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsEventCount {
    pub event_name: String,
    pub events: u64,
    pub sessions: u64,
    pub visitors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsDailyStats {
    pub day: String,
    pub visitors: u64,
    pub sessions: u64,
    pub page_views: u64,
    pub market_views: u64,
    pub funded_bounty_clicks: u64,
    pub canonical_posts_confirmed: u64,
    pub funding_starts: u64,
    pub claims_confirmed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsChannelStats {
    pub source: String,
    pub campaign: Option<String>,
    pub visitors: u64,
    pub sessions: u64,
    pub page_views: u64,
    pub funded_bounty_clicks: u64,
    pub canonical_posts_confirmed: u64,
    pub funding_starts: u64,
    pub claims_confirmed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsStats {
    pub overview: SiteAnalyticsOverview,
    pub event_counts: Vec<SiteAnalyticsEventCount>,
    pub daily: Vec<SiteAnalyticsDailyStats>,
    pub channels: Vec<SiteAnalyticsChannelStats>,
}

#[derive(Debug, Clone)]
pub struct NewSocialMentionIngestion {
    pub id: Uuid,
    pub provider: String,
    pub provider_event_id: String,
    pub source_network: String,
    pub mention_id: String,
    pub mention_url: String,
    pub author_fid: i64,
    pub author_handle: Option<String>,
    pub mention_text: String,
    pub status: String,
    pub draft: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocialMentionIngestion {
    pub id: Uuid,
    pub provider: String,
    pub provider_event_id: String,
    pub source_network: String,
    pub mention_id: String,
    pub mention_url: String,
    pub author_fid: i64,
    pub author_handle: Option<String>,
    pub mention_text: String,
    pub status: String,
    pub draft: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
    pub reply_cast_hash: Option<String>,
    pub last_error: Option<String>,
    pub reply_attempt_count: u32,
    pub reply_lease_token: Option<Uuid>,
    pub reply_lease_expires_at: Option<DateTime<Utc>>,
    pub received_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SocialMentionIngestionReservation {
    pub record: SocialMentionIngestion,
    pub inserted: bool,
}

const SELECT_GITHUB_ISSUE_SYNC_BOUNTY_FOR_UPDATE_SQL: &str = r#"
            SELECT id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            FROM bounties
            WHERE id = $1
            FOR UPDATE
            "#;
const LOCK_GITHUB_ISSUE_SYNC_BOUNTY_SQL: &str = r#"
            SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))
            "#;
const GITHUB_ISSUE_SYNC_ACTIVITY_SQL: &str = r#"
            SELECT
              EXISTS(SELECT 1 FROM funding_intents WHERE bounty_id = $1)
              OR EXISTS(SELECT 1 FROM funding_contributions WHERE bounty_id = $1)
              OR EXISTS(SELECT 1 FROM claims WHERE bounty_id = $1)
              OR EXISTS(SELECT 1 FROM submissions WHERE bounty_id = $1)
              AS has_activity
            "#;
const INSERT_GITHUB_ISSUE_SYNC_BOUNTY_SQL: &str = r#"
            INSERT INTO bounties
              (id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            "#;
const UPDATE_GITHUB_ISSUE_SYNC_BOUNTY_SQL: &str = r#"
            UPDATE bounties
            SET help_request_id = $2,
                title = $3,
                template_slug = $4,
                amount = $5,
                currency = $6,
                funding_targets = $7,
                funding_mode = $8,
                privacy = $9,
                status = $10,
                terms_hash = $11
            WHERE id = $1
            RETURNING id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            "#;

#[derive(Debug, Clone)]
pub enum GitHubIssueSyncBountyUpsert {
    Upserted(Bounty),
    BlockedByActivity(Bounty),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseLogScanCursor {
    pub network: String,
    pub escrow_contract: String,
    pub last_scanned_block: u64,
    pub last_log_key: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseIndexerHeartbeat {
    pub network: String,
    pub escrow_contract: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latest_block: Option<u64>,
    pub confirmed_to_block: Option<u64>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub fetched_logs: u64,
    pub persisted_cursor_block: Option<u64>,
    pub skipped_reason: Option<String>,
    pub error_message: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum X402RelayStatus {
    Prepared,
    Relaying,
    Broadcast,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewX402RelayAttempt {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub bounty_contract: String,
    pub contributor: String,
    pub amount: u64,
    pub authorization_nonce: String,
    pub authorization_valid_before: u64,
    pub request_fingerprint: String,
    pub relayer_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X402RelayAttempt {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub bounty_contract: String,
    pub contributor: String,
    pub amount: u64,
    pub authorization_nonce: String,
    pub authorization_valid_before: u64,
    pub request_fingerprint: String,
    pub relayer_address: String,
    pub status: X402RelayStatus,
    pub retryable: bool,
    pub attempt_count: u32,
    pub tx_hash: Option<String>,
    pub estimated_gas: Option<u64>,
    pub gas_limit: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub canonical_event_id: Option<Uuid>,
    pub confirmed_block: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewClaimCandidate {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub bounty_contract: String,
    pub solver_wallet: String,
    pub agent_id: Option<Uuid>,
    pub eligibility_evidence: AgentEligibilityEvidence,
    pub eligibility_decision: AgentEligibilityDecision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimCandidateReservation {
    pub candidate: ClaimCandidate,
    pub waitlist_position: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct NewBondSponsorship {
    pub id: Uuid,
    pub claim_candidate_id: Uuid,
    pub network: String,
    pub bounty_contract: String,
    pub solver_wallet: String,
    pub sponsor_wallet: String,
    pub amount: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: Uuid,
    pub owner_wallet: String,
    pub endpoint_url: String,
    pub event_types: Vec<AgentWebhookEventType>,
    pub subscription_kind: String,
    pub filters: domain::DiscoverySubscriptionFilters,
    pub management_token_hash: Option<String>,
    pub secret_version: u32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewDiscoveryWebhookSubscription {
    pub id: Uuid,
    pub endpoint_url: String,
    pub filters: domain::DiscoverySubscriptionFilters,
    pub management_token_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub event_id: Uuid,
    pub event_type: AgentWebhookEventType,
    pub payload: serde_json::Value,
    pub status: String,
    pub attempt_count: u32,
    pub next_attempt_at: DateTime<Utc>,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub response_status: Option<u16>,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryObligation {
    pub id: Uuid,
    pub issue_number: u64,
    pub source_contract: String,
    pub recipient_wallet: String,
    pub amount: u64,
    pub status: String,
    pub transaction_hash: Option<String>,
    pub evidence_url: Option<String>,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct BountyStatusScope {
    pub bounty: Bounty,
    pub funding_intents: Vec<FundingIntent>,
    pub funding_contributions: Vec<FundingContribution>,
    pub escrows: Vec<Escrow>,
    pub claims: Vec<Claim>,
    pub submissions: Vec<Submission>,
    pub verifier_results: Vec<VerifierResult>,
    pub proofs: Vec<ProofRecord>,
    pub settlements: Vec<Settlement>,
    pub reputation_events: Vec<ReputationEvent>,
    pub template_signals: Vec<TemplateSignal>,
    pub risk_events: Vec<RiskEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableTableSnapshot {
    pub rows: u64,
    pub canonical_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableDataSnapshot {
    pub schema_version: String,
    pub tables: BTreeMap<String, DurableTableSnapshot>,
}

#[derive(Debug, Default)]
pub struct InMemoryStore {
    pub agents: HashMap<Id, Agent>,
    pub bounties: HashMap<Id, Bounty>,
}

impl InMemoryStore {
    pub fn insert_agent(&mut self, agent: Agent) {
        self.agents.insert(agent.id, agent);
    }

    pub fn insert_bounty(&mut self, bounty: Bounty) {
        self.bounties.insert(bounty.id, bounty);
    }
}

#[derive(Debug, Clone)]
pub struct PostgresStore {
    pool: PgPool,
}

impl PostgresStore {
    pub async fn connect(database_url: &str) -> DbResult<Self> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn migrate(&self) -> DbResult<()> {
        let mut connection = self.pool.acquire().await?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(MIGRATION_ADVISORY_LOCK_ID)
            .execute(&mut *connection)
            .await?;

        let migration_result = async {
            for migration in [
                CORE_MIGRATION,
                AUTONOMOUS_PROTOCOL_MIGRATION,
                X402_RELAYER_MIGRATION,
                AGENT_COORDINATION_MIGRATION,
                TRIAL_BOUNTIES_MIGRATION,
                SOLVER_LEADERBOARD_MIGRATION,
                DISCOVERY_SUBSCRIPTIONS_MIGRATION,
                OPPORTUNITY_CONVERSION_MIGRATION,
                LEGAL_ACCEPTANCES_MIGRATION,
                SITE_ANALYTICS_MIGRATION,
                SOCIAL_MENTION_INGESTION_MIGRATION,
                COMPETITOR_INTELLIGENCE_MIGRATION,
                OBJECTIVE_COORDINATION_MIGRATION,
            ] {
                for statement in migration
                    .split(';')
                    .map(str::trim)
                    .filter(|statement| !statement.is_empty())
                {
                    sqlx::query(statement).execute(&mut *connection).await?;
                }
            }
            Ok::<(), sqlx::Error>(())
        }
        .await;

        let unlock_result = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(MIGRATION_ADVISORY_LOCK_ID)
            .execute(&mut *connection)
            .await;

        match (migration_result, unlock_result) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(error), Ok(_)) => Err(error.into()),
            (Ok(()), Err(error)) | (Err(_), Err(error)) => Err(error.into()),
        }
    }

    pub async fn durable_data_snapshot(&self) -> DbResult<DurableDataSnapshot> {
        let table_names: Vec<String> = sqlx::query_scalar(
            "SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut tables = BTreeMap::new();
        for table_name in table_names {
            let identifier = format!("\"{}\"", table_name.replace('"', "\"\""));
            let rows: Vec<String> = sqlx::query_scalar(&format!(
                "SELECT to_jsonb(snapshot_row)::text FROM public.{identifier} AS snapshot_row \
                 ORDER BY to_jsonb(snapshot_row)::text"
            ))
            .fetch_all(&self.pool)
            .await?;
            let mut hasher = Sha256::new();
            for row in &rows {
                hasher.update((row.len() as u64).to_be_bytes());
                hasher.update(row.as_bytes());
            }
            tables.insert(
                table_name,
                DurableTableSnapshot {
                    rows: rows.len() as u64,
                    canonical_sha256: hex::encode(hasher.finalize()),
                },
            );
        }
        Ok(DurableDataSnapshot {
            schema_version: "agent-bounties/durable-data-snapshot-v1".to_string(),
            tables,
        })
    }

    pub async fn record_legal_acceptance(
        &self,
        acceptance: &NewLegalAcceptance,
    ) -> DbResult<LegalAcceptance> {
        let row = sqlx::query(
            r#"
            INSERT INTO legal_acceptances
              (id, terms_version, privacy_version, action, wallet_address,
               statement_hash, acceptance_method, accepted_at)
            VALUES ($1, $2, $3, $4, lower($5), $6, $7, $8)
            RETURNING id, terms_version, privacy_version, action, wallet_address,
                      statement_hash, acceptance_method, accepted_at, recorded_at
            "#,
        )
        .bind(acceptance.id)
        .bind(&acceptance.terms_version)
        .bind(&acceptance.privacy_version)
        .bind(&acceptance.action)
        .bind(&acceptance.wallet_address)
        .bind(&acceptance.statement_hash)
        .bind(&acceptance.acceptance_method)
        .bind(acceptance.accepted_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(LegalAcceptance {
            id: row.try_get("id")?,
            terms_version: row.try_get("terms_version")?,
            privacy_version: row.try_get("privacy_version")?,
            action: row.try_get("action")?,
            wallet_address: row.try_get("wallet_address")?,
            statement_hash: row.try_get("statement_hash")?,
            acceptance_method: row.try_get("acceptance_method")?,
            accepted_at: row.try_get("accepted_at")?,
            recorded_at: row.try_get("recorded_at")?,
        })
    }

    pub async fn create_discovery_webhook_subscription(
        &self,
        subscription: &NewDiscoveryWebhookSubscription,
    ) -> DbResult<WebhookSubscription> {
        let event_types = vec![
            AgentWebhookEventType::OpportunityPublished,
            AgentWebhookEventType::OpportunityStateChanged,
        ];
        let row = sqlx::query(
            r#"
            INSERT INTO webhook_subscriptions
              (id, owner_wallet, endpoint_url, event_types, subscription_kind,
               filters, management_token_hash)
            VALUES ($1, $2, $3, $4, 'public_discovery', $5, $6)
            RETURNING id, owner_wallet, endpoint_url, event_types, subscription_kind,
                      filters, management_token_hash, secret_version, enabled,
                      created_at, updated_at
            "#,
        )
        .bind(subscription.id)
        .bind(format!("discovery:{}", subscription.id))
        .bind(&subscription.endpoint_url)
        .bind(serde_json::to_value(&event_types)?)
        .bind(serde_json::to_value(&subscription.filters)?)
        .bind(&subscription.management_token_hash)
        .fetch_one(&self.pool)
        .await?;
        webhook_subscription_from_row(row)
    }

    pub async fn get_webhook_subscription(
        &self,
        id: Uuid,
    ) -> DbResult<Option<WebhookSubscription>> {
        sqlx::query(
            r#"
            SELECT id, owner_wallet, endpoint_url, event_types, subscription_kind,
                   filters, management_token_hash, secret_version, enabled,
                   created_at, updated_at
            FROM webhook_subscriptions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(webhook_subscription_from_row)
        .transpose()
    }

    pub async fn list_enabled_discovery_webhook_subscriptions(
        &self,
    ) -> DbResult<Vec<WebhookSubscription>> {
        let rows = sqlx::query(
            r#"
            SELECT id, owner_wallet, endpoint_url, event_types, subscription_kind,
                   filters, management_token_hash, secret_version, enabled,
                   created_at, updated_at
            FROM webhook_subscriptions
            WHERE subscription_kind = 'public_discovery' AND enabled
            ORDER BY created_at, id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(webhook_subscription_from_row)
            .collect()
    }

    pub async fn delete_discovery_webhook_subscription(
        &self,
        id: Uuid,
        management_token_hash: &str,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            r#"
            DELETE FROM webhook_subscriptions
            WHERE id = $1 AND subscription_kind = 'public_discovery'
              AND management_token_hash = $2
            "#,
        )
        .bind(id)
        .bind(management_token_hash)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn enqueue_webhook_delivery(
        &self,
        subscription_id: Uuid,
        event_id: Uuid,
        event_type: AgentWebhookEventType,
        payload: &serde_json::Value,
    ) -> DbResult<bool> {
        let event_type = serde_json::to_value(event_type)?
            .as_str()
            .ok_or_else(|| DbError::InvalidEnum("agent webhook event type".to_string()))?
            .to_string();
        let result = sqlx::query(
            r#"
            INSERT INTO webhook_deliveries
              (id, subscription_id, event_id, event_type, payload, status)
            VALUES ($1, $2, $3, $4, $5, 'pending')
            ON CONFLICT (subscription_id, event_id) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(subscription_id)
        .bind(event_id)
        .bind(event_type)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn lease_webhook_deliveries(
        &self,
        limit: u32,
        lease_token: Uuid,
        lease_seconds: u64,
    ) -> DbResult<Vec<WebhookDelivery>> {
        let rows = sqlx::query(
            r#"
            WITH ready AS (
              SELECT id
              FROM webhook_deliveries
              WHERE next_attempt_at <= now()
                AND (status = 'pending'
                     OR (status = 'delivering' AND lease_expires_at <= now()))
              ORDER BY next_attempt_at, created_at, id
              FOR UPDATE SKIP LOCKED
              LIMIT $1
            )
            UPDATE webhook_deliveries AS delivery
            SET status = 'delivering', attempt_count = attempt_count + 1,
                lease_token = $2,
                lease_expires_at = now() + make_interval(secs => $3),
                updated_at = now()
            FROM ready
            WHERE delivery.id = ready.id
            RETURNING delivery.id, delivery.subscription_id, delivery.event_id,
                      delivery.event_type, delivery.payload, delivery.status,
                      delivery.attempt_count, delivery.next_attempt_at,
                      delivery.lease_token, delivery.lease_expires_at,
                      delivery.response_status, delivery.last_error,
                      delivery.delivered_at, delivery.created_at, delivery.updated_at
            "#,
        )
        .bind(i64::from(limit.clamp(1, 100)))
        .bind(lease_token)
        .bind(i64_from_u64(lease_seconds)?)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(webhook_delivery_from_row).collect()
    }

    pub async fn mark_webhook_delivery_delivered(
        &self,
        id: Uuid,
        lease_token: Uuid,
        response_status: u16,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE webhook_deliveries
            SET status = 'delivered', response_status = $3, last_error = NULL,
                delivered_at = now(), lease_token = NULL, lease_expires_at = NULL,
                updated_at = now()
            WHERE id = $1 AND lease_token = $2 AND status = 'delivering'
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(i32::from(response_status))
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn reschedule_webhook_delivery(
        &self,
        id: Uuid,
        lease_token: Uuid,
        dead: bool,
        delay_seconds: u64,
        response_status: Option<u16>,
        error: &str,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE webhook_deliveries
            SET status = CASE WHEN $3 THEN 'dead' ELSE 'pending' END,
                next_attempt_at = CASE WHEN $3 THEN next_attempt_at
                                       ELSE now() + make_interval(secs => $4) END,
                response_status = $5, last_error = $6,
                lease_token = NULL, lease_expires_at = NULL, updated_at = now()
            WHERE id = $1 AND lease_token = $2 AND status = 'delivering'
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(dead)
        .bind(i64_from_u64(delay_seconds)?)
        .bind(response_status.map(i32::from))
        .bind(error.chars().take(500).collect::<String>())
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn record_opportunity_creation_progress(
        &self,
        terms_hash: &str,
        unfunded_bounty_id: Option<Uuid>,
        network: &str,
        stage: &str,
        observed_at: DateTime<Utc>,
    ) -> DbResult<()> {
        if !matches!(stage, "funding_prepared" | "wallet_signed") {
            return Err(DbError::InvalidEnum(format!(
                "opportunity creation stage {stage}"
            )));
        }
        let funding_prepared_at = (stage == "funding_prepared").then_some(observed_at);
        let wallet_signed_at = (stage == "wallet_signed").then_some(observed_at);
        let row = sqlx::query(
            r#"
            INSERT INTO opportunity_creation_progress
              (terms_hash, unfunded_bounty_id, network, funding_prepared_at,
               wallet_signed_at, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $6)
            ON CONFLICT (terms_hash) DO UPDATE SET
              unfunded_bounty_id = COALESCE(
                opportunity_creation_progress.unfunded_bounty_id,
                EXCLUDED.unfunded_bounty_id
              ),
              network = EXCLUDED.network,
              funding_prepared_at = CASE
                WHEN opportunity_creation_progress.funding_prepared_at IS NULL
                  THEN EXCLUDED.funding_prepared_at
                WHEN EXCLUDED.funding_prepared_at IS NULL
                  THEN opportunity_creation_progress.funding_prepared_at
                ELSE LEAST(
                  opportunity_creation_progress.funding_prepared_at,
                  EXCLUDED.funding_prepared_at
                )
              END,
              wallet_signed_at = CASE
                WHEN opportunity_creation_progress.wallet_signed_at IS NULL
                  THEN EXCLUDED.wallet_signed_at
                WHEN EXCLUDED.wallet_signed_at IS NULL
                  THEN opportunity_creation_progress.wallet_signed_at
                ELSE LEAST(
                  opportunity_creation_progress.wallet_signed_at,
                  EXCLUDED.wallet_signed_at
                )
              END,
              updated_at = GREATEST(opportunity_creation_progress.updated_at, EXCLUDED.updated_at)
            WHERE opportunity_creation_progress.unfunded_bounty_id IS NULL
               OR EXCLUDED.unfunded_bounty_id IS NULL
               OR opportunity_creation_progress.unfunded_bounty_id = EXCLUDED.unfunded_bounty_id
            RETURNING terms_hash
            "#,
        )
        .bind(terms_hash.to_ascii_lowercase())
        .bind(unfunded_bounty_id)
        .bind(network.to_ascii_lowercase())
        .bind(funding_prepared_at)
        .bind(wallet_signed_at)
        .bind(observed_at)
        .fetch_optional(&self.pool)
        .await?;
        if row.is_none() {
            return Err(DbError::OpportunityConversionConflict(
                "one terms hash cannot refer to different unfunded bounties".to_string(),
            ));
        }
        Ok(())
    }

    pub async fn record_site_analytics_event(
        &self,
        event: &NewSiteAnalyticsEvent,
    ) -> DbResult<bool> {
        let inserted = sqlx::query(
            r#"
            INSERT INTO site_analytics_events
              (event_id, visitor_id, session_id, event_name, page_path, source,
               campaign, referrer_host, opportunity_id, bounty_contract, occurred_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (event_id) DO NOTHING
            RETURNING event_id
            "#,
        )
        .bind(event.event_id)
        .bind(event.visitor_id)
        .bind(event.session_id)
        .bind(&event.event_name)
        .bind(&event.page_path)
        .bind(&event.source)
        .bind(&event.campaign)
        .bind(&event.referrer_host)
        .bind(&event.opportunity_id)
        .bind(&event.bounty_contract)
        .bind(event.occurred_at)
        .fetch_optional(&self.pool)
        .await?;
        Ok(inserted.is_some())
    }

    pub async fn reserve_social_mention_ingestion(
        &self,
        ingestion: &NewSocialMentionIngestion,
    ) -> DbResult<SocialMentionIngestionReservation> {
        let row = sqlx::query(
            r#"
            INSERT INTO social_mention_ingestions
              (id, provider, provider_event_id, source_network, mention_id,
               mention_url, author_fid, author_handle, mention_text, status,
               draft, idempotency_key, received_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
            ON CONFLICT DO NOTHING
            RETURNING id, provider, provider_event_id, source_network, mention_id,
                      mention_url, author_fid, author_handle, mention_text, status,
                      draft, idempotency_key, reply_cast_hash, last_error,
                      reply_attempt_count, reply_lease_token, reply_lease_expires_at,
                      received_at, updated_at
            "#,
        )
        .bind(ingestion.id)
        .bind(&ingestion.provider)
        .bind(&ingestion.provider_event_id)
        .bind(&ingestion.source_network)
        .bind(&ingestion.mention_id)
        .bind(&ingestion.mention_url)
        .bind(ingestion.author_fid)
        .bind(&ingestion.author_handle)
        .bind(&ingestion.mention_text)
        .bind(&ingestion.status)
        .bind(&ingestion.draft)
        .bind(&ingestion.idempotency_key)
        .bind(ingestion.received_at)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            return Ok(SocialMentionIngestionReservation {
                record: social_mention_ingestion_from_row(row)?,
                inserted: true,
            });
        }

        let row = sqlx::query(
            r#"
            SELECT id, provider, provider_event_id, source_network, mention_id,
                   mention_url, author_fid, author_handle, mention_text, status,
                   draft, idempotency_key, reply_cast_hash, last_error,
                   reply_attempt_count, reply_lease_token, reply_lease_expires_at,
                   received_at, updated_at
            FROM social_mention_ingestions
            WHERE (provider = $1 AND provider_event_id = $2)
               OR (source_network = $3 AND mention_id = $4)
            ORDER BY received_at ASC
            LIMIT 1
            "#,
        )
        .bind(&ingestion.provider)
        .bind(&ingestion.provider_event_id)
        .bind(&ingestion.source_network)
        .bind(&ingestion.mention_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::InvalidEnum("social mention replay disappeared after conflict".to_string())
        })?;
        Ok(SocialMentionIngestionReservation {
            record: social_mention_ingestion_from_row(row)?,
            inserted: false,
        })
    }

    pub async fn get_social_mention_ingestion(
        &self,
        id: Uuid,
    ) -> DbResult<Option<SocialMentionIngestion>> {
        let row = sqlx::query(
            r#"
            SELECT id, provider, provider_event_id, source_network, mention_id,
                   mention_url, author_fid, author_handle, mention_text, status,
                   draft, idempotency_key, reply_cast_hash, last_error,
                   reply_attempt_count, reply_lease_token, reply_lease_expires_at,
                   received_at, updated_at
            FROM social_mention_ingestions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(social_mention_ingestion_from_row).transpose()
    }

    pub async fn claim_social_mention_reply(
        &self,
        id: Uuid,
        lease_token: Uuid,
        lease_seconds: u64,
    ) -> DbResult<Option<SocialMentionIngestion>> {
        let row = sqlx::query(
            r#"
            UPDATE social_mention_ingestions
            SET status = 'replying', reply_attempt_count = reply_attempt_count + 1,
                reply_lease_token = $2,
                reply_lease_expires_at = now() + make_interval(secs => $3),
                last_error = NULL, updated_at = now()
            WHERE id = $1
              AND (
                status IN ('reply_pending', 'reply_failed')
                OR (status = 'replying' AND reply_lease_expires_at < now())
              )
            RETURNING id, provider, provider_event_id, source_network, mention_id,
                      mention_url, author_fid, author_handle, mention_text, status,
                      draft, idempotency_key, reply_cast_hash, last_error,
                      reply_attempt_count, reply_lease_token, reply_lease_expires_at,
                      received_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(i64_from_u64(lease_seconds)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(social_mention_ingestion_from_row).transpose()
    }

    pub async fn complete_social_mention_reply(
        &self,
        id: Uuid,
        lease_token: Uuid,
        reply_cast_hash: Option<&str>,
        error: Option<&str>,
    ) -> DbResult<Option<SocialMentionIngestion>> {
        let succeeded = reply_cast_hash.is_some();
        let row = sqlx::query(
            r#"
            UPDATE social_mention_ingestions
            SET status = CASE WHEN $3 THEN 'replied' ELSE 'reply_failed' END,
                reply_cast_hash = CASE WHEN $3 THEN $4 ELSE reply_cast_hash END,
                last_error = CASE WHEN $3 THEN NULL ELSE $5 END,
                reply_lease_token = NULL, reply_lease_expires_at = NULL,
                updated_at = now()
            WHERE id = $1 AND status = 'replying' AND reply_lease_token = $2
            RETURNING id, provider, provider_event_id, source_network, mention_id,
                      mention_url, author_fid, author_handle, mention_text, status,
                      draft, idempotency_key, reply_cast_hash, last_error,
                      reply_attempt_count, reply_lease_token, reply_lease_expires_at,
                      received_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(succeeded)
        .bind(reply_cast_hash)
        .bind(error.map(|value| value.chars().take(500).collect::<String>()))
        .fetch_optional(&self.pool)
        .await?;
        row.map(social_mention_ingestion_from_row).transpose()
    }

    pub async fn site_analytics_stats(
        &self,
        window_started_at: DateTime<Utc>,
    ) -> DbResult<SiteAnalyticsStats> {
        let overview = sqlx::query(
            r#"
            WITH window_events AS (
              SELECT * FROM site_analytics_events
              WHERE occurred_at >= $1 AND occurred_at <= NOW()
            ), visitor_days AS (
              SELECT visitor_id, COUNT(DISTINCT occurred_at::date) AS active_days
              FROM window_events
              GROUP BY visitor_id
            )
            SELECT
              (SELECT COUNT(*) FROM visitor_days) AS unique_visitors,
              (SELECT COUNT(*) FROM visitor_days WHERE active_days >= 2) AS returning_visitors,
              COUNT(DISTINCT session_id) AS sessions,
              COUNT(*) FILTER (WHERE event_name = 'page_view') AS page_views,
              MIN(occurred_at) AS first_event_at,
              MAX(occurred_at) AS last_event_at
            FROM window_events
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;

        let event_rows = sqlx::query(
            r#"
            SELECT event_name, COUNT(*) AS events,
                   COUNT(DISTINCT session_id) AS sessions,
                   COUNT(DISTINCT visitor_id) AS visitors
            FROM site_analytics_events
            WHERE occurred_at >= $1 AND occurred_at <= NOW()
            GROUP BY event_name
            ORDER BY event_name
            "#,
        )
        .bind(window_started_at)
        .fetch_all(&self.pool)
        .await?;

        let daily_rows = sqlx::query(
            r#"
            SELECT to_char(occurred_at::date, 'YYYY-MM-DD') AS day,
                   COUNT(DISTINCT visitor_id) AS visitors,
                   COUNT(DISTINCT session_id) AS sessions,
                   COUNT(*) FILTER (WHERE event_name = 'page_view') AS page_views,
                   COUNT(*) FILTER (WHERE event_name = 'market_view') AS market_views,
                   COUNT(*) FILTER (WHERE event_name = 'funded_bounty_click') AS funded_bounty_clicks,
                   COUNT(*) FILTER (WHERE event_name = 'canonical_post_confirmed') AS canonical_posts_confirmed,
                   COUNT(*) FILTER (WHERE event_name = 'funding_started') AS funding_starts,
                   COUNT(*) FILTER (WHERE event_name = 'claim_confirmed') AS claims_confirmed
            FROM site_analytics_events
            WHERE occurred_at >= $1 AND occurred_at <= NOW()
            GROUP BY occurred_at::date
            ORDER BY occurred_at::date
            "#,
        )
        .bind(window_started_at)
        .fetch_all(&self.pool)
        .await?;

        let channel_rows = sqlx::query(
            r#"
            WITH window_events AS (
              SELECT * FROM site_analytics_events
              WHERE occurred_at >= $1 AND occurred_at <= NOW()
            ), active_visitors AS (
              SELECT DISTINCT visitor_id FROM window_events
            ), first_touch AS (
              SELECT DISTINCT ON (event.visitor_id)
                     event.visitor_id, COALESCE(event.source, 'direct') AS source,
                     event.campaign
              FROM site_analytics_events AS event
              JOIN active_visitors USING (visitor_id)
              ORDER BY event.visitor_id, event.occurred_at, event.received_at, event.event_id
            )
            SELECT first_touch.source, first_touch.campaign,
                   COUNT(DISTINCT window_events.visitor_id) AS visitors,
                   COUNT(DISTINCT window_events.session_id) AS sessions,
                   COUNT(*) FILTER (WHERE window_events.event_name = 'page_view') AS page_views,
                   COUNT(*) FILTER (WHERE window_events.event_name = 'funded_bounty_click') AS funded_bounty_clicks,
                   COUNT(*) FILTER (WHERE window_events.event_name = 'canonical_post_confirmed') AS canonical_posts_confirmed,
                   COUNT(*) FILTER (WHERE window_events.event_name = 'funding_started') AS funding_starts,
                   COUNT(*) FILTER (WHERE window_events.event_name = 'claim_confirmed') AS claims_confirmed
            FROM window_events
            JOIN first_touch USING (visitor_id)
            GROUP BY first_touch.source, first_touch.campaign
            ORDER BY visitors DESC, first_touch.source, first_touch.campaign
            "#,
        )
        .bind(window_started_at)
        .fetch_all(&self.pool)
        .await?;

        Ok(SiteAnalyticsStats {
            overview: SiteAnalyticsOverview {
                unique_visitors: u64_from_i64(overview.try_get("unique_visitors")?)?,
                returning_visitors: u64_from_i64(overview.try_get("returning_visitors")?)?,
                sessions: u64_from_i64(overview.try_get("sessions")?)?,
                page_views: u64_from_i64(overview.try_get("page_views")?)?,
                first_event_at: overview.try_get("first_event_at")?,
                last_event_at: overview.try_get("last_event_at")?,
            },
            event_counts: event_rows
                .into_iter()
                .map(|row| {
                    Ok(SiteAnalyticsEventCount {
                        event_name: row.try_get("event_name")?,
                        events: u64_from_i64(row.try_get("events")?)?,
                        sessions: u64_from_i64(row.try_get("sessions")?)?,
                        visitors: u64_from_i64(row.try_get("visitors")?)?,
                    })
                })
                .collect::<DbResult<Vec<_>>>()?,
            daily: daily_rows
                .into_iter()
                .map(|row| {
                    Ok(SiteAnalyticsDailyStats {
                        day: row.try_get("day")?,
                        visitors: u64_from_i64(row.try_get("visitors")?)?,
                        sessions: u64_from_i64(row.try_get("sessions")?)?,
                        page_views: u64_from_i64(row.try_get("page_views")?)?,
                        market_views: u64_from_i64(row.try_get("market_views")?)?,
                        funded_bounty_clicks: u64_from_i64(row.try_get("funded_bounty_clicks")?)?,
                        canonical_posts_confirmed: u64_from_i64(
                            row.try_get("canonical_posts_confirmed")?,
                        )?,
                        funding_starts: u64_from_i64(row.try_get("funding_starts")?)?,
                        claims_confirmed: u64_from_i64(row.try_get("claims_confirmed")?)?,
                    })
                })
                .collect::<DbResult<Vec<_>>>()?,
            channels: channel_rows
                .into_iter()
                .map(|row| {
                    Ok(SiteAnalyticsChannelStats {
                        source: row.try_get("source")?,
                        campaign: row.try_get("campaign")?,
                        visitors: u64_from_i64(row.try_get("visitors")?)?,
                        sessions: u64_from_i64(row.try_get("sessions")?)?,
                        page_views: u64_from_i64(row.try_get("page_views")?)?,
                        funded_bounty_clicks: u64_from_i64(row.try_get("funded_bounty_clicks")?)?,
                        canonical_posts_confirmed: u64_from_i64(
                            row.try_get("canonical_posts_confirmed")?,
                        )?,
                        funding_starts: u64_from_i64(row.try_get("funding_starts")?)?,
                        claims_confirmed: u64_from_i64(row.try_get("claims_confirmed")?)?,
                    })
                })
                .collect::<DbResult<Vec<_>>>()?,
        })
    }

    pub async fn opportunity_lifecycle_stats(
        &self,
        window_started_at: DateTime<Utc>,
    ) -> DbResult<OpportunityLifecycleStats> {
        let cohort = sqlx::query(
            r#"
            WITH cohort AS (
              SELECT id, created_at FROM trial_bounties WHERE created_at >= $1
            ), first_solutions AS (
              SELECT cohort.id,
                     MIN(solution.created_at) AS first_solution_at,
                     cohort.created_at AS published_at
              FROM cohort
              JOIN unfunded_bounty_solutions AS solution
                ON solution.trial_bounty_id = cohort.id
              GROUP BY cohort.id, cohort.created_at
            ), progress AS (
              SELECT progress.*
              FROM opportunity_creation_progress AS progress
              JOIN cohort ON cohort.id = progress.unfunded_bounty_id
            ), roots AS (
              SELECT DISTINCT progress.unfunded_bounty_id, progress.network,
                     created.bounty_id
              FROM progress
              JOIN autonomous_bounty_events AS created
                ON created.network = progress.network
               AND created.kind = 'canonical_bounty_created'
               AND lower(created.data->>'terms_hash') = lower(progress.terms_hash)
            ), root_flags AS (
              SELECT roots.unfunded_bounty_id,
                     BOOL_OR(event.kind = 'bounty_became_claimable') AS funded,
                     BOOL_OR(event.kind = 'bounty_claimed') AS claimed,
                     BOOL_OR(event.kind = 'submission_added') AS submitted,
                     BOOL_OR(event.kind = 'bounty_settled') AS settled
              FROM roots
              LEFT JOIN autonomous_bounty_events AS event
                ON event.network = roots.network AND event.bounty_id = roots.bounty_id
              GROUP BY roots.unfunded_bounty_id
            )
            SELECT
              (SELECT COUNT(*) FROM cohort) AS published,
              (SELECT COUNT(*) FROM first_solutions) AS solution_received,
              (SELECT COUNT(DISTINCT unfunded_bounty_id) FROM progress
                WHERE funding_prepared_at IS NOT NULL) AS funding_prepared,
              (SELECT COUNT(DISTINCT unfunded_bounty_id) FROM progress
                WHERE wallet_signed_at IS NOT NULL) AS wallet_signed_observed,
              (SELECT COUNT(DISTINCT unfunded_bounty_id) FROM roots) AS canonical_created,
              (SELECT COUNT(*) FROM root_flags WHERE funded) AS funded,
              (SELECT COUNT(*) FROM root_flags WHERE claimed) AS claimed,
              (SELECT COUNT(*) FROM root_flags WHERE submitted) AS submitted,
              (SELECT COUNT(*) FROM root_flags WHERE settled) AS settled,
              (SELECT AVG(EXTRACT(EPOCH FROM (first_solution_at - published_at)))::double precision
                FROM first_solutions) AS average_seconds_to_first_solution,
              (SELECT percentile_cont(0.5) WITHIN GROUP (
                  ORDER BY EXTRACT(EPOCH FROM (first_solution_at - published_at))
                )::double precision FROM first_solutions) AS median_seconds_to_first_solution
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;

        let canonical = sqlx::query(
            r#"
            WITH created AS (
              SELECT network, bounty_id, MIN(occurred_at) AS created_at
              FROM autonomous_bounty_events
              WHERE kind = 'canonical_bounty_created' AND occurred_at >= $1
              GROUP BY network, bounty_id
            ), settled AS (
              SELECT network, bounty_id, MIN(occurred_at) AS settled_at
              FROM autonomous_bounty_events
              WHERE kind = 'bounty_settled'
              GROUP BY network, bounty_id
            ), posters AS (
              SELECT lower(data->>'creator') AS wallet, COUNT(DISTINCT bounty_id) AS bounties
              FROM autonomous_bounty_events
              WHERE kind = 'canonical_bounty_created' AND occurred_at >= $1
                AND data ? 'creator'
              GROUP BY lower(data->>'creator')
            ), paid_solvers AS (
              SELECT lower(data->>'solver') AS wallet, COUNT(DISTINCT bounty_id) AS bounties
              FROM autonomous_bounty_events
              WHERE kind = 'bounty_settled' AND occurred_at >= $1
                AND data ? 'solver'
              GROUP BY lower(data->>'solver')
            )
            SELECT
              (SELECT COUNT(*) FROM created) AS canonical_created_in_window,
              (SELECT COUNT(DISTINCT (network, bounty_id)) FROM autonomous_bounty_events
                WHERE kind = 'bounty_claimed' AND occurred_at >= $1) AS canonical_claimed_in_window,
              (SELECT COUNT(DISTINCT (network, bounty_id)) FROM autonomous_bounty_events
                WHERE kind = 'bounty_settled' AND occurred_at >= $1) AS canonical_settled_in_window,
              (SELECT COUNT(*) FROM posters) AS unique_canonical_poster_wallets,
              (SELECT COUNT(*) FROM posters WHERE bounties > 1) AS repeat_canonical_poster_wallets,
              (SELECT COUNT(*) FROM paid_solvers) AS unique_paid_solver_wallets,
              (SELECT COUNT(*) FROM paid_solvers WHERE bounties > 1) AS repeat_paid_solver_wallets,
              (SELECT AVG(EXTRACT(EPOCH FROM (settled.settled_at - created.created_at)))::double precision
                FROM created JOIN settled USING (network, bounty_id))
                AS average_seconds_creation_to_settlement
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(OpportunityLifecycleStats {
            published: u64_from_i64(cohort.try_get("published")?)?,
            solution_received: u64_from_i64(cohort.try_get("solution_received")?)?,
            funding_prepared: u64_from_i64(cohort.try_get("funding_prepared")?)?,
            wallet_signed_observed: u64_from_i64(cohort.try_get("wallet_signed_observed")?)?,
            canonical_created: u64_from_i64(cohort.try_get("canonical_created")?)?,
            funded: u64_from_i64(cohort.try_get("funded")?)?,
            claimed: u64_from_i64(cohort.try_get("claimed")?)?,
            submitted: u64_from_i64(cohort.try_get("submitted")?)?,
            settled: u64_from_i64(cohort.try_get("settled")?)?,
            average_seconds_to_first_solution: cohort
                .try_get("average_seconds_to_first_solution")?,
            median_seconds_to_first_solution: cohort.try_get("median_seconds_to_first_solution")?,
            average_seconds_creation_to_settlement: canonical
                .try_get("average_seconds_creation_to_settlement")?,
            canonical_created_in_window: u64_from_i64(
                canonical.try_get("canonical_created_in_window")?,
            )?,
            canonical_claimed_in_window: u64_from_i64(
                canonical.try_get("canonical_claimed_in_window")?,
            )?,
            canonical_settled_in_window: u64_from_i64(
                canonical.try_get("canonical_settled_in_window")?,
            )?,
            unique_canonical_poster_wallets: u64_from_i64(
                canonical.try_get("unique_canonical_poster_wallets")?,
            )?,
            repeat_canonical_poster_wallets: u64_from_i64(
                canonical.try_get("repeat_canonical_poster_wallets")?,
            )?,
            unique_paid_solver_wallets: u64_from_i64(
                canonical.try_get("unique_paid_solver_wallets")?,
            )?,
            repeat_paid_solver_wallets: u64_from_i64(
                canonical.try_get("repeat_paid_solver_wallets")?,
            )?,
        })
    }

    pub async fn create_or_get_trial_bounty(
        &self,
        trial: &NewTrialBounty,
    ) -> DbResult<TrialBounty> {
        let inserted = sqlx::query(
            r#"
            INSERT INTO trial_bounties
              (id, idempotency_key, request_fingerprint, title, goal,
               acceptance_criteria, source_url, discovery_source, status,
               demo_agent_solution, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (idempotency_key) DO NOTHING
            RETURNING id, idempotency_key, request_fingerprint, title, goal,
                      acceptance_criteria, source_url, discovery_source, status,
                      demo_agent_solution, created_at, expires_at
            "#,
        )
        .bind(trial.id)
        .bind(&trial.idempotency_key)
        .bind(&trial.request_fingerprint)
        .bind(&trial.title)
        .bind(&trial.goal)
        .bind(serde_json::to_value(&trial.acceptance_criteria)?)
        .bind(&trial.source_url)
        .bind(&trial.discovery_source)
        .bind(&trial.status)
        .bind(&trial.demo_agent_solution)
        .bind(trial.expires_at)
        .fetch_optional(&self.pool)
        .await?;

        let row = match inserted {
            Some(row) => row,
            None => {
                sqlx::query(
                    r#"
                SELECT id, idempotency_key, request_fingerprint, title, goal,
                       acceptance_criteria, source_url, discovery_source, status,
                       demo_agent_solution, created_at, expires_at
                FROM trial_bounties
                WHERE idempotency_key = $1
                "#,
                )
                .bind(&trial.idempotency_key)
                .fetch_one(&self.pool)
                .await?
            }
        };
        let persisted = trial_bounty_from_row(row)?;
        if persisted.request_fingerprint != trial.request_fingerprint {
            return Err(DbError::TrialBountyConflict);
        }
        Ok(persisted)
    }

    pub async fn get_trial_bounty(&self, id: Uuid) -> DbResult<Option<TrialBounty>> {
        sqlx::query(
            r#"
            SELECT id, idempotency_key, request_fingerprint, title, goal,
                   acceptance_criteria, source_url, discovery_source, status,
                   demo_agent_solution, created_at, expires_at
            FROM trial_bounties
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(trial_bounty_from_row)
        .transpose()
    }

    pub async fn get_trial_bounty_by_idempotency(
        &self,
        idempotency_key: &str,
    ) -> DbResult<Option<TrialBounty>> {
        sqlx::query(
            r#"
            SELECT id, idempotency_key, request_fingerprint, title, goal,
                   acceptance_criteria, source_url, discovery_source, status,
                   demo_agent_solution, created_at, expires_at
            FROM trial_bounties
            WHERE idempotency_key = $1
            "#,
        )
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?
        .map(trial_bounty_from_row)
        .transpose()
    }

    pub async fn list_trial_bounties(&self, limit: u32) -> DbResult<Vec<TrialBounty>> {
        let limit = i64::from(limit.clamp(1, 100));
        sqlx::query(
            r#"
            SELECT id, idempotency_key, request_fingerprint, title, goal,
                   acceptance_criteria, source_url, discovery_source, status,
                   demo_agent_solution, created_at, expires_at
            FROM trial_bounties
            WHERE status = 'open' AND expires_at > now()
            ORDER BY created_at DESC, id
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(trial_bounty_from_row)
        .collect()
    }

    pub async fn upsert_unfunded_bounty_solution(
        &self,
        solution: &NewUnfundedBountySolution,
    ) -> DbResult<UnfundedBountySolution> {
        let row = sqlx::query(
            r#"
            INSERT INTO unfunded_bounty_solutions
              (id, trial_bounty_id, agent_id, summary, deliverable_markdown, evidence)
            SELECT $1, $2, $3, $4, $5, $6
            FROM trial_bounties
            WHERE id = $2 AND status = 'open' AND expires_at > now()
            ON CONFLICT (trial_bounty_id, agent_id) DO UPDATE SET
              summary = EXCLUDED.summary,
              deliverable_markdown = EXCLUDED.deliverable_markdown,
              evidence = EXCLUDED.evidence,
              updated_at = now()
            RETURNING id, trial_bounty_id, agent_id, summary,
                      deliverable_markdown, evidence, created_at, updated_at
            "#,
        )
        .bind(solution.id)
        .bind(solution.trial_bounty_id)
        .bind(solution.agent_id)
        .bind(&solution.summary)
        .bind(&solution.deliverable_markdown)
        .bind(&solution.evidence)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(DbError::UnfundedBountyUnavailable)?;
        unfunded_bounty_solution_from_row(row)
    }

    pub async fn list_unfunded_bounty_solutions(
        &self,
        trial_bounty_id: Uuid,
    ) -> DbResult<Vec<UnfundedBountySolution>> {
        sqlx::query(
            r#"
            SELECT id, trial_bounty_id, agent_id, summary,
                   deliverable_markdown, evidence, created_at, updated_at
            FROM unfunded_bounty_solutions
            WHERE trial_bounty_id = $1
            ORDER BY created_at, id
            "#,
        )
        .bind(trial_bounty_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(unfunded_bounty_solution_from_row)
        .collect()
    }

    pub async fn create_objective(&self, objective: &Objective) -> DbResult<()> {
        let status = objective_status_value(objective.status)?;
        let result = sqlx::query(
            r#"
            INSERT INTO objective_aggregates
              (id, schema_version, revision, status, requesting_party_id, record, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(objective.id)
        .bind(&objective.schema_version)
        .bind(i64_from_u64(objective.revision)?)
        .bind(status)
        .bind(objective.requesting_party_id)
        .bind(serde_json::to_value(objective)?)
        .bind(objective.created_at)
        .bind(objective.updated_at)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::ObjectiveAlreadyExists(objective.id));
        }
        Ok(())
    }

    pub async fn replace_objective(
        &self,
        objective: &Objective,
        expected_revision: u64,
    ) -> DbResult<()> {
        if objective.revision <= expected_revision {
            return Err(DbError::ObjectiveRevisionConflict {
                objective_id: objective.id,
                expected_revision,
            });
        }
        let status = objective_status_value(objective.status)?;
        let result = sqlx::query(
            r#"
            UPDATE objective_aggregates
            SET schema_version = $2,
                revision = $3,
                status = $4,
                requesting_party_id = $5,
                record = $6,
                updated_at = $7
            WHERE id = $1 AND revision = $8
            "#,
        )
        .bind(objective.id)
        .bind(&objective.schema_version)
        .bind(i64_from_u64(objective.revision)?)
        .bind(status)
        .bind(objective.requesting_party_id)
        .bind(serde_json::to_value(objective)?)
        .bind(objective.updated_at)
        .bind(i64_from_u64(expected_revision)?)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM objective_aggregates WHERE id = $1)")
                .bind(objective.id)
                .fetch_one(&self.pool)
                .await?;
        if exists {
            Err(DbError::ObjectiveRevisionConflict {
                objective_id: objective.id,
                expected_revision,
            })
        } else {
            Err(DbError::ObjectiveNotFound(objective.id))
        }
    }

    pub async fn get_objective(&self, id: Id) -> DbResult<Option<Objective>> {
        let record = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT record FROM objective_aggregates WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        record
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }

    pub async fn list_objectives(&self) -> DbResult<Vec<Objective>> {
        let records = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT record FROM objective_aggregates ORDER BY created_at DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        records
            .into_iter()
            .map(serde_json::from_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub async fn reserve_x402_relay_attempt(
        &self,
        attempt: &NewX402RelayAttempt,
        max_network_attempts: u32,
        max_contributor_attempts: u32,
    ) -> DbResult<X402RelayAttempt> {
        if max_network_attempts == 0 || max_contributor_attempts == 0 {
            return Err(DbError::X402RelayQuotaExceeded(
                "configured quota must be positive".to_string(),
            ));
        }
        let normalized_bounty = normalize_key_address(&attempt.bounty_contract);
        let normalized_contributor = normalize_key_address(&attempt.contributor);
        let normalized_nonce = attempt.authorization_nonce.to_ascii_lowercase();
        let normalized_relayer = normalize_key_address(&attempt.relayer_address);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("x402-relay-quota:{}", attempt.network))
            .execute(&mut *transaction)
            .await?;

        let existing = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, contributor, amount,
                   authorization_nonce, authorization_valid_before, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message,
                   canonical_event_id, confirmed_block, created_at, updated_at
            FROM x402_relay_attempts
            WHERE network = $1 AND bounty_contract = $2 AND authorization_nonce = $3
            "#,
        )
        .bind(&attempt.network)
        .bind(&normalized_bounty)
        .bind(&normalized_nonce)
        .fetch_optional(&mut *transaction)
        .await?
        .map(x402_relay_attempt_from_row)
        .transpose()?;
        if let Some(existing) = existing {
            validate_x402_relay_replay(&existing, attempt)?;
            transaction.commit().await?;
            return Ok(existing);
        }

        let quota = sqlx::query(
            r#"
            SELECT COUNT(*) AS network_count,
                   COUNT(*) FILTER (WHERE contributor = $2) AS contributor_count
            FROM x402_relay_attempts
            WHERE network = $1 AND created_at >= now() - interval '24 hours'
            "#,
        )
        .bind(&attempt.network)
        .bind(&normalized_contributor)
        .fetch_one(&mut *transaction)
        .await?;
        let network_count: i64 = quota.try_get("network_count")?;
        let contributor_count: i64 = quota.try_get("contributor_count")?;
        if network_count >= i64::from(max_network_attempts) {
            return Err(DbError::X402RelayQuotaExceeded(
                "network rolling-24-hour authorization limit reached".to_string(),
            ));
        }
        if contributor_count >= i64::from(max_contributor_attempts) {
            return Err(DbError::X402RelayQuotaExceeded(
                "contributor rolling-24-hour authorization limit reached".to_string(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO x402_relay_attempts
              (id, idempotency_key, network, bounty_contract, contributor, amount,
               authorization_nonce, authorization_valid_before, request_fingerprint,
               relayer_address, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'prepared')
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(attempt.id)
        .bind(&attempt.idempotency_key)
        .bind(&attempt.network)
        .bind(&normalized_bounty)
        .bind(&normalized_contributor)
        .bind(i64_from_u64(attempt.amount)?)
        .bind(&normalized_nonce)
        .bind(i64_from_u64(attempt.authorization_valid_before)?)
        .bind(&attempt.request_fingerprint)
        .bind(&normalized_relayer)
        .execute(&mut *transaction)
        .await?;

        let persisted = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, contributor, amount,
                   authorization_nonce, authorization_valid_before, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message,
                   canonical_event_id, confirmed_block, created_at, updated_at
            FROM x402_relay_attempts
            WHERE network = $1 AND bounty_contract = $2 AND authorization_nonce = $3
            "#,
        )
        .bind(&attempt.network)
        .bind(&normalized_bounty)
        .bind(&normalized_nonce)
        .fetch_optional(&mut *transaction)
        .await?
        .map(x402_relay_attempt_from_row)
        .transpose()?
        .ok_or_else(|| {
            DbError::X402RelayConflict(
                "idempotency key is already bound to another authorization".to_string(),
            )
        })?;
        validate_x402_relay_replay(&persisted, attempt)?;
        transaction.commit().await?;
        Ok(persisted)
    }

    pub async fn get_x402_relay_attempt(&self, id: Uuid) -> DbResult<Option<X402RelayAttempt>> {
        let row = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, contributor, amount,
                   authorization_nonce, authorization_valid_before, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message,
                   canonical_event_id, confirmed_block, created_at, updated_at
            FROM x402_relay_attempts
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(x402_relay_attempt_from_row).transpose()
    }

    pub async fn get_x402_relay_attempt_by_authorization(
        &self,
        network: &str,
        bounty_contract: &str,
        authorization_nonce: &str,
    ) -> DbResult<Option<X402RelayAttempt>> {
        let row = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, contributor, amount,
                   authorization_nonce, authorization_valid_before, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message,
                   canonical_event_id, confirmed_block, created_at, updated_at
            FROM x402_relay_attempts
            WHERE network = $1 AND bounty_contract = $2 AND authorization_nonce = $3
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(bounty_contract))
        .bind(authorization_nonce.to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await?;
        row.map(x402_relay_attempt_from_row).transpose()
    }

    pub async fn acquire_x402_relayer_lease(
        &self,
        network: &str,
        lease_seconds: u64,
    ) -> DbResult<Option<Uuid>> {
        let lease_token = Uuid::new_v4();
        let lease_seconds = i64_from_u64(lease_seconds)?;
        let row = sqlx::query(
            r#"
            INSERT INTO x402_relayer_leases
              (network, lease_token, lease_expires_at, updated_at)
            VALUES ($1, $2, now() + make_interval(secs => $3), now())
            ON CONFLICT (network) DO UPDATE SET
              lease_token = EXCLUDED.lease_token,
              lease_expires_at = EXCLUDED.lease_expires_at,
              updated_at = now()
            WHERE x402_relayer_leases.lease_expires_at <= now()
            RETURNING lease_token
            "#,
        )
        .bind(network)
        .bind(lease_token)
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| row.try_get("lease_token"))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn release_x402_relayer_lease(
        &self,
        network: &str,
        lease_token: Uuid,
    ) -> DbResult<()> {
        sqlx::query("DELETE FROM x402_relayer_leases WHERE network = $1 AND lease_token = $2")
            .bind(network)
            .bind(lease_token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn claim_x402_relay_attempt(
        &self,
        id: Uuid,
        lease_token: Uuid,
        lease_seconds: u64,
    ) -> DbResult<Option<X402RelayAttempt>> {
        let lease_seconds = i64_from_u64(lease_seconds)?;
        let row = sqlx::query(
            r#"
            UPDATE x402_relay_attempts
            SET status = 'relaying',
                retryable = true,
                attempt_count = attempt_count + 1,
                lease_token = $2,
                lease_expires_at = now() + make_interval(secs => $3),
                error_code = NULL,
                error_message = NULL,
                updated_at = now()
            WHERE id = $1
              AND (
                status = 'prepared'
                OR (status = 'failed' AND retryable)
                OR (status = 'relaying' AND lease_expires_at <= now())
              )
            RETURNING id, idempotency_key, network, bounty_contract, contributor, amount,
                      authorization_nonce, authorization_valid_before, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message,
                      canonical_event_id, confirmed_block, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await?;
        row.map(x402_relay_attempt_from_row).transpose()
    }

    pub async fn mark_x402_relay_broadcast(
        &self,
        id: Uuid,
        lease_token: Uuid,
        tx_hash: &str,
        estimated_gas: u64,
        gas_limit: u64,
    ) -> DbResult<X402RelayAttempt> {
        let row = sqlx::query(
            r#"
            UPDATE x402_relay_attempts
            SET status = 'broadcast', retryable = true, tx_hash = $3,
                estimated_gas = $4, gas_limit = $5,
                lease_token = NULL, lease_expires_at = NULL, updated_at = now()
            WHERE id = $1 AND lease_token = $2 AND status = 'relaying'
            RETURNING id, idempotency_key, network, bounty_contract, contributor, amount,
                      authorization_nonce, authorization_valid_before, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message,
                      canonical_event_id, confirmed_block, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(tx_hash.to_ascii_lowercase())
        .bind(i64_from_u64(estimated_gas)?)
        .bind(i64_from_u64(gas_limit)?)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::X402RelayConflict(
                "relay lease was lost before broadcast persisted".to_string(),
            )
        })?;
        x402_relay_attempt_from_row(row)
    }

    pub async fn mark_x402_relay_failed(
        &self,
        id: Uuid,
        lease_token: Option<Uuid>,
        retryable: bool,
        error_code: &str,
        error_message: &str,
    ) -> DbResult<X402RelayAttempt> {
        let row = sqlx::query(
            r#"
            UPDATE x402_relay_attempts
            SET status = 'failed', retryable = $3, error_code = $4, error_message = $5,
                lease_token = NULL, lease_expires_at = NULL, updated_at = now()
            WHERE id = $1 AND ($2::uuid IS NULL OR lease_token = $2)
            RETURNING id, idempotency_key, network, bounty_contract, contributor, amount,
                      authorization_nonce, authorization_valid_before, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message,
                      canonical_event_id, confirmed_block, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(retryable)
        .bind(error_code)
        .bind(error_message.chars().take(500).collect::<String>())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::X402RelayConflict("relay failure lease mismatch".to_string()))?;
        x402_relay_attempt_from_row(row)
    }

    pub async fn mark_x402_relay_confirmed(
        &self,
        id: Uuid,
        canonical_event_id: Uuid,
        confirmed_block: u64,
    ) -> DbResult<X402RelayAttempt> {
        let row = sqlx::query(
            r#"
            UPDATE x402_relay_attempts
            SET status = 'confirmed', retryable = false,
                canonical_event_id = $2, confirmed_block = $3,
                lease_token = NULL, lease_expires_at = NULL,
                error_code = NULL, error_message = NULL, updated_at = now()
            WHERE id = $1 AND status IN ('broadcast', 'confirmed')
            RETURNING id, idempotency_key, network, bounty_contract, contributor, amount,
                      authorization_nonce, authorization_valid_before, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message,
                      canonical_event_id, confirmed_block, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(canonical_event_id)
        .bind(i64_from_u64(confirmed_block)?)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::X402RelayConflict("relay was not broadcast before confirmation".to_string())
        })?;
        x402_relay_attempt_from_row(row)
    }

    pub async fn reserve_claim_candidate(
        &self,
        candidate: &NewClaimCandidate,
        exclusive_seconds: u64,
        waitlist_capacity: u16,
    ) -> DbResult<ClaimCandidateReservation> {
        if !candidate.eligibility_decision.eligible {
            return Err(DbError::ClaimCandidateConflict(
                "ineligible candidates cannot enter the claim queue".to_string(),
            ));
        }
        if exclusive_seconds == 0 || waitlist_capacity == 0 {
            return Err(DbError::ClaimCandidateConflict(
                "claim queue bounds must be positive".to_string(),
            ));
        }
        let network = candidate.network.trim().to_ascii_lowercase();
        let bounty = normalize_key_address(&candidate.bounty_contract);
        let solver = normalize_key_address(&candidate.solver_wallet);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("claim-queue:{network}:{bounty}"))
            .execute(&mut *transaction)
            .await?;

        let existing = sqlx::query(CLAIM_CANDIDATE_SELECT_BY_IDEMPOTENCY_SQL)
            .bind(&candidate.idempotency_key)
            .fetch_optional(&mut *transaction)
            .await?
            .map(claim_candidate_from_row)
            .transpose()?;
        if let Some(existing) = existing {
            if existing.network != network
                || existing.bounty_contract != bounty
                || existing.solver_wallet != solver
                || existing.eligibility_evidence != candidate.eligibility_evidence
            {
                return Err(DbError::ClaimCandidateConflict(
                    "idempotency key was already used for different claim inputs".to_string(),
                ));
            }
            let position = waitlist_position(&mut transaction, &existing).await?;
            transaction.commit().await?;
            return Ok(ClaimCandidateReservation {
                candidate: existing,
                waitlist_position: position,
            });
        }

        if sqlx::query(ACTIVE_CLAIM_CANDIDATE_SELECT_SQL)
            .bind(&network)
            .bind(&bounty)
            .bind(&solver)
            .fetch_optional(&mut *transaction)
            .await?
            .is_some()
        {
            return Err(DbError::ClaimCandidateConflict(
                "solver already has an active request for this bounty; replay its original idempotency key"
                    .to_string(),
            ));
        }

        let active_count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM claim_candidates
            WHERE network = $1 AND bounty_contract = $2
              AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying')
            "#,
        )
        .bind(&network)
        .bind(&bounty)
        .fetch_one(&mut *transaction)
        .await?;
        let status = if active_count == 0 {
            "exclusive"
        } else {
            let waitlisted: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM claim_candidates WHERE network = $1 AND bounty_contract = $2 AND status = 'waitlisted'",
            )
            .bind(&network)
            .bind(&bounty)
            .fetch_one(&mut *transaction)
            .await?;
            if waitlisted >= i64::from(waitlist_capacity) {
                return Err(DbError::ClaimWaitlistFull);
            }
            "waitlisted"
        };
        let exclusive_seconds = i64_from_u64(exclusive_seconds)?;
        let row = sqlx::query(
            r#"
            INSERT INTO claim_candidates
              (id, idempotency_key, network, bounty_contract, solver_wallet, agent_id,
               eligibility_evidence, eligibility_decision, status, exclusive_until)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9,
                    CASE WHEN $9 = 'exclusive'
                         THEN now() + make_interval(secs => $10) ELSE NULL END)
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(candidate.id)
        .bind(&candidate.idempotency_key)
        .bind(&network)
        .bind(&bounty)
        .bind(&solver)
        .bind(candidate.agent_id)
        .bind(serde_json::to_value(&candidate.eligibility_evidence)?)
        .bind(serde_json::to_value(&candidate.eligibility_decision)?)
        .bind(status)
        .bind(exclusive_seconds)
        .fetch_one(&mut *transaction)
        .await?;
        let candidate = claim_candidate_from_row(row)?;
        let position = waitlist_position(&mut transaction, &candidate).await?;
        transaction.commit().await?;
        Ok(ClaimCandidateReservation {
            candidate,
            waitlist_position: position,
        })
    }

    pub async fn get_claim_candidate(&self, id: Uuid) -> DbResult<Option<ClaimCandidate>> {
        sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, solver_wallet,
                   agent_id, eligibility_evidence, eligibility_decision, status,
                   exclusive_until, authorization_nonce, authorization_valid_before,
                   claim_transaction_hash, canonical_event_id, failure_code,
                   failure_message, created_at, updated_at
            FROM claim_candidates WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(claim_candidate_from_row)
        .transpose()
    }

    pub async fn set_claim_candidate_authorization(
        &self,
        id: Uuid,
        nonce: &str,
        valid_before: u64,
    ) -> DbResult<ClaimCandidate> {
        let row = sqlx::query(
            r#"
            UPDATE claim_candidates
            SET status = 'authorization_ready', authorization_nonce = $2,
                authorization_valid_before = $3, updated_at = now()
            WHERE id = $1 AND status IN ('exclusive', 'sponsoring', 'authorization_ready')
              AND exclusive_until > now()
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(nonce.to_ascii_lowercase())
        .bind(i64_from_u64(valid_before)?)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "candidate is not the live exclusive claimant".to_string(),
            )
        })?;
        claim_candidate_from_row(row)
    }

    pub async fn mark_claim_candidate_relaying(
        &self,
        id: Uuid,
        tx_hash: &str,
    ) -> DbResult<ClaimCandidate> {
        update_claim_candidate_status(&self.pool, id, "relaying", Some(tx_hash), None, None).await
    }

    pub async fn mark_claim_candidate_claimed(
        &self,
        id: Uuid,
        canonical_event_id: Uuid,
    ) -> DbResult<ClaimCandidate> {
        update_claim_candidate_status(
            &self.pool,
            id,
            "claimed",
            None,
            Some(canonical_event_id),
            None,
        )
        .await
    }

    pub async fn mark_claim_candidate_failed(
        &self,
        id: Uuid,
        code: &str,
        message: &str,
    ) -> DbResult<ClaimCandidate> {
        update_claim_candidate_status(&self.pool, id, "failed", None, None, Some((code, message)))
            .await
    }

    pub async fn promote_waitlisted_claimant_after_canonical_reopen(
        &self,
        network: &str,
        bounty_contract: &str,
        exclusive_seconds: u64,
    ) -> DbResult<Option<ClaimCandidate>> {
        let network = network.trim().to_ascii_lowercase();
        let bounty = normalize_key_address(bounty_contract);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("claim-queue:{network}:{bounty}"))
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            r#"
            UPDATE claim_candidates SET status = 'superseded', updated_at = now()
            WHERE network = $1 AND bounty_contract = $2
              AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying')
              AND exclusive_until <= now()
            "#,
        )
        .bind(&network)
        .bind(&bounty)
        .execute(&mut *transaction)
        .await?;
        let active = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, bounty_contract, solver_wallet,
                   agent_id, eligibility_evidence, eligibility_decision, status,
                   exclusive_until, authorization_nonce, authorization_valid_before,
                   claim_transaction_hash, canonical_event_id, failure_code,
                   failure_message, created_at, updated_at
            FROM claim_candidates
            WHERE network = $1 AND bounty_contract = $2
              AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying')
            "#,
        )
        .bind(&network)
        .bind(&bounty)
        .fetch_optional(&mut *transaction)
        .await?
        .map(claim_candidate_from_row)
        .transpose()?;
        if active.is_some() {
            transaction.commit().await?;
            return Ok(active);
        }
        let row = sqlx::query(
            r#"
            UPDATE claim_candidates
            SET status = 'exclusive', exclusive_until = now() + make_interval(secs => $3),
                updated_at = now()
            WHERE id = (
              SELECT id FROM claim_candidates
              WHERE network = $1 AND bounty_contract = $2 AND status = 'waitlisted'
              ORDER BY created_at, id LIMIT 1 FOR UPDATE SKIP LOCKED
            )
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(&network)
        .bind(&bounty)
        .bind(i64_from_u64(exclusive_seconds)?)
        .fetch_optional(&mut *transaction)
        .await?
        .map(claim_candidate_from_row)
        .transpose()?;
        transaction.commit().await?;
        Ok(row)
    }

    pub async fn reserve_bond_sponsorship(
        &self,
        sponsorship: &NewBondSponsorship,
        max_network_amount_24h: u64,
        max_solver_amount_24h: u64,
    ) -> DbResult<BondSponsorship> {
        if sponsorship.amount == 0
            || sponsorship.amount > max_solver_amount_24h
            || max_solver_amount_24h > max_network_amount_24h
        {
            return Err(DbError::BondSponsorshipQuotaExceeded(
                "requested grant exceeds configured bounds".to_string(),
            ));
        }
        let network = sponsorship.network.trim().to_ascii_lowercase();
        let solver = normalize_key_address(&sponsorship.solver_wallet);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!("bond-sponsorship:{network}"))
            .execute(&mut *transaction)
            .await?;
        if let Some(existing) = sqlx::query(BOND_SPONSORSHIP_SELECT_BY_CANDIDATE_SQL)
            .bind(sponsorship.claim_candidate_id)
            .fetch_optional(&mut *transaction)
            .await?
            .map(bond_sponsorship_from_row)
            .transpose()?
        {
            transaction.commit().await?;
            return Ok(existing);
        }
        let usage = sqlx::query(
            r#"
            SELECT COALESCE(SUM(amount), 0)::bigint AS network_amount,
                   COALESCE(SUM(amount) FILTER (WHERE solver_wallet = $2), 0)::bigint AS solver_amount
            FROM bond_sponsorships
            WHERE network = $1
              AND (status <> 'failed' OR failure_code = 'broadcast_unknown')
              AND created_at >= now() - interval '24 hours'
            "#,
        )
        .bind(&network)
        .bind(&solver)
        .fetch_one(&mut *transaction)
        .await?;
        let network_amount = u64_from_i64(usage.try_get("network_amount")?)?;
        let solver_amount = u64_from_i64(usage.try_get("solver_amount")?)?;
        if network_amount.saturating_add(sponsorship.amount) > max_network_amount_24h
            || solver_amount.saturating_add(sponsorship.amount) > max_solver_amount_24h
        {
            return Err(DbError::BondSponsorshipQuotaExceeded(
                "rolling 24-hour grant cap reached".to_string(),
            ));
        }
        let row = sqlx::query(
            r#"
            INSERT INTO bond_sponsorships
              (id, claim_candidate_id, network, bounty_contract, solver_wallet,
               sponsor_wallet, amount, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'reserved')
            RETURNING id, claim_candidate_id, network, bounty_contract, solver_wallet,
                      sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                      failure_code, failure_message, created_at, updated_at
            "#,
        )
        .bind(sponsorship.id)
        .bind(sponsorship.claim_candidate_id)
        .bind(&network)
        .bind(normalize_key_address(&sponsorship.bounty_contract))
        .bind(&solver)
        .bind(normalize_key_address(&sponsorship.sponsor_wallet))
        .bind(i64_from_u64(sponsorship.amount)?)
        .fetch_one(&mut *transaction)
        .await?;
        let sponsorship = bond_sponsorship_from_row(row)?;
        transaction.commit().await?;
        Ok(sponsorship)
    }

    pub async fn get_bond_sponsorship_for_candidate(
        &self,
        claim_candidate_id: Uuid,
    ) -> DbResult<Option<BondSponsorship>> {
        sqlx::query(BOND_SPONSORSHIP_SELECT_BY_CANDIDATE_SQL)
            .bind(claim_candidate_id)
            .fetch_optional(&self.pool)
            .await?
            .map(bond_sponsorship_from_row)
            .transpose()
    }

    pub async fn get_claim_candidate_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> DbResult<Option<ClaimCandidate>> {
        sqlx::query(CLAIM_CANDIDATE_SELECT_BY_IDEMPOTENCY_SQL)
            .bind(idempotency_key.trim())
            .fetch_optional(&self.pool)
            .await?
            .map(claim_candidate_from_row)
            .transpose()
    }

    pub async fn claim_funnel_stats(&self, window_hours: u32) -> DbResult<ClaimFunnelStats> {
        let window_hours = window_hours.clamp(1, 720);
        let generated_at = Utc::now();
        let window_started_at = generated_at - chrono::Duration::hours(i64::from(window_hours));
        let row = sqlx::query(
            r#"
            SELECT
              COUNT(*) AS observed,
              COUNT(DISTINCT solver_wallet) AS unique_solver_wallets,
              COUNT(*) FILTER (WHERE status = 'waitlisted') AS waitlisted_current,
              COUNT(*) FILTER (WHERE status IN ('exclusive', 'sponsoring')) AS exclusive_current,
              COUNT(*) FILTER (WHERE status = 'authorization_ready') AS authorization_ready_current,
              COUNT(*) FILTER (WHERE status = 'relaying') AS relaying_current,
              COUNT(*) FILTER (WHERE authorization_nonce IS NOT NULL) AS authorization_prepared,
              COUNT(*) FILTER (WHERE claim_transaction_hash IS NOT NULL) AS transaction_broadcast,
              COUNT(*) FILTER (WHERE status = 'claimed' AND canonical_event_id IS NOT NULL) AS claimed_canonical,
              COUNT(*) FILTER (WHERE status = 'superseded') AS superseded,
              COUNT(*) FILTER (WHERE status = 'withdrawn') AS withdrawn,
              COUNT(*) FILTER (WHERE status = 'failed') AS failed
            FROM claim_candidates
            WHERE created_at >= $1
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;
        let stages = ClaimFunnelStageCounts {
            observed: u64_from_i64(row.try_get("observed")?)?,
            unique_solver_wallets: u64_from_i64(row.try_get("unique_solver_wallets")?)?,
            waitlisted_current: u64_from_i64(row.try_get("waitlisted_current")?)?,
            exclusive_current: u64_from_i64(row.try_get("exclusive_current")?)?,
            authorization_ready_current: u64_from_i64(row.try_get("authorization_ready_current")?)?,
            relaying_current: u64_from_i64(row.try_get("relaying_current")?)?,
            authorization_prepared: u64_from_i64(row.try_get("authorization_prepared")?)?,
            transaction_broadcast: u64_from_i64(row.try_get("transaction_broadcast")?)?,
            claimed_canonical: u64_from_i64(row.try_get("claimed_canonical")?)?,
            superseded: u64_from_i64(row.try_get("superseded")?)?,
            withdrawn: u64_from_i64(row.try_get("withdrawn")?)?,
            failed: u64_from_i64(row.try_get("failed")?)?,
        };
        let sponsorship_row = sqlx::query(
            r#"
            SELECT
              COUNT(*) FILTER (WHERE sponsorship.status = 'reserved') AS reserved,
              COUNT(*) FILTER (WHERE sponsorship.status = 'broadcast') AS broadcast,
              COUNT(*) FILTER (WHERE sponsorship.status = 'confirmed') AS confirmed,
              COUNT(*) FILTER (WHERE sponsorship.status = 'failed') AS failed,
              COUNT(*) FILTER (
                WHERE sponsorship.status = 'confirmed'
                  AND candidate.status = 'claimed'
                  AND candidate.canonical_event_id IS NOT NULL
              ) AS sponsored_claims_confirmed
            FROM bond_sponsorships sponsorship
            JOIN claim_candidates candidate ON candidate.id = sponsorship.claim_candidate_id
            WHERE sponsorship.created_at >= $1
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;
        let sponsored_claims_confirmed =
            u64_from_i64(sponsorship_row.try_get("sponsored_claims_confirmed")?)?;
        let sponsorship = ClaimSponsorshipFunnelCounts {
            reserved: u64_from_i64(sponsorship_row.try_get("reserved")?)?,
            broadcast: u64_from_i64(sponsorship_row.try_get("broadcast")?)?,
            confirmed: u64_from_i64(sponsorship_row.try_get("confirmed")?)?,
            failed: u64_from_i64(sponsorship_row.try_get("failed")?)?,
            sponsored_claims_confirmed,
            direct_claims_confirmed: stages
                .claimed_canonical
                .saturating_sub(sponsored_claims_confirmed),
        };
        let canonical_row = sqlx::query(
            r#"
            WITH window_events AS (
              SELECT id, kind, NULLIF(lower(data->>'solver'), '') AS solver_wallet
              FROM autonomous_bounty_events
              WHERE occurred_at >= $1
                AND kind IN ('bounty_claimed', 'submission_added', 'bounty_settled')
            ), paid_solvers AS (
              SELECT solver_wallet, COUNT(*) AS settlement_count
              FROM window_events
              WHERE kind = 'bounty_settled' AND solver_wallet IS NOT NULL
              GROUP BY solver_wallet
            )
            SELECT
              COUNT(*) FILTER (WHERE event.kind = 'bounty_claimed') AS claims_confirmed,
              COUNT(DISTINCT event.solver_wallet) FILTER (
                WHERE event.kind = 'bounty_claimed'
              ) AS unique_claimed_solver_wallets,
              COUNT(*) FILTER (
                WHERE event.kind = 'bounty_claimed'
                  AND EXISTS (
                    SELECT 1 FROM claim_candidates candidate
                    WHERE candidate.canonical_event_id = event.id
                  )
              ) AS hosted_claims_confirmed,
              COUNT(*) FILTER (
                WHERE event.kind = 'bounty_claimed'
                  AND NOT EXISTS (
                    SELECT 1 FROM claim_candidates candidate
                    WHERE candidate.canonical_event_id = event.id
                  )
              ) AS unattributed_claims_confirmed,
              COUNT(*) FILTER (WHERE event.kind = 'submission_added') AS submissions_confirmed,
              COUNT(*) FILTER (WHERE event.kind = 'bounty_settled') AS settlements_confirmed,
              COUNT(DISTINCT event.solver_wallet) FILTER (
                WHERE event.kind = 'bounty_settled'
              ) AS unique_paid_solver_wallets,
              (SELECT COUNT(*) FROM paid_solvers WHERE settlement_count > 1)
                AS repeat_paid_solver_wallets
            FROM window_events event
            "#,
        )
        .bind(window_started_at)
        .fetch_one(&self.pool)
        .await?;
        let canonical_outcomes = CanonicalClaimOutcomeCounts {
            claims_confirmed: u64_from_i64(canonical_row.try_get("claims_confirmed")?)?,
            unique_claimed_solver_wallets: u64_from_i64(
                canonical_row.try_get("unique_claimed_solver_wallets")?,
            )?,
            hosted_claims_confirmed: u64_from_i64(
                canonical_row.try_get("hosted_claims_confirmed")?,
            )?,
            unattributed_claims_confirmed: u64_from_i64(
                canonical_row.try_get("unattributed_claims_confirmed")?,
            )?,
            submissions_confirmed: u64_from_i64(canonical_row.try_get("submissions_confirmed")?)?,
            settlements_confirmed: u64_from_i64(canonical_row.try_get("settlements_confirmed")?)?,
            unique_paid_solver_wallets: u64_from_i64(
                canonical_row.try_get("unique_paid_solver_wallets")?,
            )?,
            repeat_paid_solver_wallets: u64_from_i64(
                canonical_row.try_get("repeat_paid_solver_wallets")?,
            )?,
        };
        let failure_rows = sqlx::query(
            r#"
            SELECT failure_code, COUNT(*) AS count
            FROM claim_candidates
            WHERE created_at >= $1 AND status = 'failed' AND failure_code IS NOT NULL
            GROUP BY failure_code
            ORDER BY failure_code
            "#,
        )
        .bind(window_started_at)
        .fetch_all(&self.pool)
        .await?;
        let mut failure_codes = BTreeMap::new();
        for failure in failure_rows {
            failure_codes.insert(
                failure.try_get::<String, _>("failure_code")?,
                u64_from_i64(failure.try_get("count")?)?,
            );
        }
        Ok(ClaimFunnelStats {
            schema_version: "agent-bounties/claim-funnel-v2".to_string(),
            window_hours,
            window_started_at,
            generated_at,
            stages,
            sponsorship,
            canonical_outcomes,
            failure_codes,
            evidence_boundary: "Stages and sponsorship measure hosted coordination. Canonical outcomes count indexed contract events across every path; an unattributed claim is not proof of a specific client. Only confirmed canonical BountyClaimed events count as claims, and only canonical BountySettled events prove payout.".to_string(),
        })
    }

    pub async fn mark_bond_sponsorship_broadcast(
        &self,
        id: Uuid,
        tx_hash: &str,
    ) -> DbResult<BondSponsorship> {
        update_bond_sponsorship(&self.pool, id, "broadcast", Some(tx_hash), None, None).await
    }

    pub async fn mark_bond_sponsorship_confirmed(
        &self,
        id: Uuid,
        confirmed_block: u64,
    ) -> DbResult<BondSponsorship> {
        update_bond_sponsorship(
            &self.pool,
            id,
            "confirmed",
            None,
            Some(confirmed_block),
            None,
        )
        .await
    }

    pub async fn mark_bond_sponsorship_failed(
        &self,
        id: Uuid,
        code: &str,
        message: &str,
    ) -> DbResult<BondSponsorship> {
        update_bond_sponsorship(&self.pool, id, "failed", None, None, Some((code, message))).await
    }

    pub async fn mark_atomic_sponsored_claim_broadcast(
        &self,
        candidate_id: Uuid,
        sponsorship_id: Uuid,
        tx_hash: &str,
    ) -> DbResult<(ClaimCandidate, BondSponsorship)> {
        let tx_hash = tx_hash.trim().to_ascii_lowercase();
        let mut transaction = self.pool.begin().await?;
        let candidate = sqlx::query(
            r#"
            UPDATE claim_candidates
            SET status = 'relaying', claim_transaction_hash = $2, updated_at = now()
            WHERE id = $1 AND (
              status IN ('exclusive', 'sponsoring', 'authorization_ready')
              OR (status = 'relaying' AND claim_transaction_hash = $2)
            )
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(candidate_id)
        .bind(&tx_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic sponsored claim candidate cannot transition to relaying".to_string(),
            )
        })
        .and_then(claim_candidate_from_row)?;
        let sponsorship = sqlx::query(
            r#"
            UPDATE bond_sponsorships
            SET status = 'broadcast', transaction_hash = $2, updated_at = now()
            WHERE id = $1 AND claim_candidate_id = $3 AND (
              status = 'reserved' OR (status = 'broadcast' AND transaction_hash = $2)
            )
            RETURNING id, claim_candidate_id, network, bounty_contract, solver_wallet,
                      sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                      failure_code, failure_message, created_at, updated_at
            "#,
        )
        .bind(sponsorship_id)
        .bind(&tx_hash)
        .bind(candidate_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic bond sponsorship cannot transition to broadcast".to_string(),
            )
        })
        .and_then(bond_sponsorship_from_row)?;
        transaction.commit().await?;
        Ok((candidate, sponsorship))
    }

    pub async fn mark_atomic_sponsored_claim_confirmed(
        &self,
        candidate_id: Uuid,
        sponsorship_id: Uuid,
        canonical_event_id: Uuid,
        confirmed_block: u64,
    ) -> DbResult<(ClaimCandidate, BondSponsorship)> {
        let mut transaction = self.pool.begin().await?;
        let candidate = sqlx::query(
            r#"
            UPDATE claim_candidates
            SET status = 'claimed', canonical_event_id = $2, updated_at = now()
            WHERE id = $1 AND status IN ('relaying', 'claimed')
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(candidate_id)
        .bind(canonical_event_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic sponsored claim candidate cannot transition to claimed".to_string(),
            )
        })
        .and_then(claim_candidate_from_row)?;
        let sponsorship = sqlx::query(
            r#"
            UPDATE bond_sponsorships
            SET status = 'confirmed', confirmed_block = $2, updated_at = now()
            WHERE id = $1 AND claim_candidate_id = $3
              AND status IN ('broadcast', 'confirmed')
            RETURNING id, claim_candidate_id, network, bounty_contract, solver_wallet,
                      sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                      failure_code, failure_message, created_at, updated_at
            "#,
        )
        .bind(sponsorship_id)
        .bind(i64_from_u64(confirmed_block)?)
        .bind(candidate_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic bond sponsorship cannot transition to confirmed".to_string(),
            )
        })
        .and_then(bond_sponsorship_from_row)?;
        transaction.commit().await?;
        Ok((candidate, sponsorship))
    }

    pub async fn mark_atomic_sponsored_claim_failed(
        &self,
        candidate_id: Uuid,
        sponsorship_id: Uuid,
        code: &str,
        message: &str,
    ) -> DbResult<(ClaimCandidate, BondSponsorship)> {
        let message = message.chars().take(500).collect::<String>();
        let mut transaction = self.pool.begin().await?;
        let candidate = sqlx::query(
            r#"
            UPDATE claim_candidates
            SET status = 'failed', failure_code = $2, failure_message = $3,
                updated_at = now()
            WHERE id = $1 AND status IN (
              'exclusive', 'sponsoring', 'authorization_ready', 'relaying', 'failed'
            )
            RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                      agent_id, eligibility_evidence, eligibility_decision, status,
                      exclusive_until, authorization_nonce, authorization_valid_before,
                      claim_transaction_hash, canonical_event_id, failure_code,
                      failure_message, created_at, updated_at
            "#,
        )
        .bind(candidate_id)
        .bind(code)
        .bind(&message)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic sponsored claim candidate cannot transition to failed".to_string(),
            )
        })
        .and_then(claim_candidate_from_row)?;
        let sponsorship = sqlx::query(
            r#"
            UPDATE bond_sponsorships
            SET status = 'failed', failure_code = $2, failure_message = $3,
                updated_at = now()
            WHERE id = $1 AND claim_candidate_id = $4
              AND status IN ('reserved', 'broadcast', 'failed')
            RETURNING id, claim_candidate_id, network, bounty_contract, solver_wallet,
                      sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                      failure_code, failure_message, created_at, updated_at
            "#,
        )
        .bind(sponsorship_id)
        .bind(code)
        .bind(&message)
        .bind(candidate_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| {
            DbError::ClaimCandidateConflict(
                "atomic bond sponsorship cannot transition to failed".to_string(),
            )
        })
        .and_then(bond_sponsorship_from_row)?;
        transaction.commit().await?;
        Ok((candidate, sponsorship))
    }

    pub async fn upsert_agent(&self, agent: &Agent) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO agents (id, handle, status, payout_wallet, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
              handle = EXCLUDED.handle,
              status = EXCLUDED.status,
              payout_wallet = EXCLUDED.payout_wallet
            "#,
        )
        .bind(agent.id)
        .bind(&agent.handle)
        .bind(format!("{:?}", agent.status))
        .bind(&agent.payout_wallet)
        .bind(agent.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_agents(&self) -> DbResult<Vec<Agent>> {
        let rows = sqlx::query(
            "SELECT id, handle, status, payout_wallet, created_at FROM agents ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Agent {
                    id: row.try_get("id")?,
                    handle: row.try_get("handle")?,
                    status: parse_agent_status(row.try_get::<String, _>("status")?)?,
                    payout_wallet: row.try_get("payout_wallet")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_contributor_contact(&self, contact: &ContributorContact) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO contributor_contacts
              (id, github_login, github_login_normalized, email, payout_wallet, associated_prs, contact_consent, wallet_consent, outreach_allowed, source, notes, created_at, updated_at)
            VALUES ($1, $2, lower($2), $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (github_login_normalized) DO UPDATE SET
              github_login = EXCLUDED.github_login,
              email = EXCLUDED.email,
              payout_wallet = EXCLUDED.payout_wallet,
              associated_prs = EXCLUDED.associated_prs,
              contact_consent = EXCLUDED.contact_consent,
              wallet_consent = EXCLUDED.wallet_consent,
              outreach_allowed = EXCLUDED.outreach_allowed,
              source = EXCLUDED.source,
              notes = EXCLUDED.notes,
              updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(contact.id)
        .bind(&contact.github_login)
        .bind(&contact.email)
        .bind(&contact.payout_wallet)
        .bind(serde_json::to_value(&contact.associated_prs)?)
        .bind(contact.contact_consent)
        .bind(contact.wallet_consent)
        .bind(contact.outreach_allowed)
        .bind(&contact.source)
        .bind(&contact.notes)
        .bind(contact.created_at)
        .bind(contact.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_contributor_contacts(&self) -> DbResult<Vec<ContributorContact>> {
        let rows = sqlx::query(
            r#"
            SELECT id, github_login, email, payout_wallet, associated_prs, contact_consent, wallet_consent, outreach_allowed, source, notes, created_at, updated_at
            FROM contributor_contacts
            ORDER BY created_at, github_login
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(ContributorContact {
                    id: row.try_get("id")?,
                    github_login: row.try_get("github_login")?,
                    email: row.try_get("email")?,
                    payout_wallet: row.try_get("payout_wallet")?,
                    associated_prs: serde_json::from_value(row.try_get("associated_prs")?)?,
                    contact_consent: row.try_get("contact_consent")?,
                    wallet_consent: row.try_get("wallet_consent")?,
                    outreach_allowed: row.try_get("outreach_allowed")?,
                    source: row.try_get("source")?,
                    notes: row.try_get("notes")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_audience_member(&self, member: &AudienceMember) -> DbResult<()> {
        sqlx::query(UPSERT_AUDIENCE_MEMBER_SQL)
            .bind(member.id)
            .bind(format!("{:?}", member.provider))
            .bind(&member.external_id)
            .bind(&member.handle)
            .bind(&member.public_profile_url)
            .bind(serde_json::to_value(&member.roles)?)
            .bind(format!("{:?}", member.lifecycle_stage))
            .bind(member.first_seen_at)
            .bind(member.last_seen_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_audience_members(&self) -> DbResult<Vec<AudienceMember>> {
        let rows = sqlx::query(
            r#"
            SELECT id, provider, external_id, handle, public_profile_url, roles, lifecycle_stage, first_seen_at, last_seen_at
            FROM audience_members
            ORDER BY first_seen_at, handle
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(AudienceMember {
                    id: row.try_get("id")?,
                    provider: parse_audience_provider(row.try_get::<String, _>("provider")?)?,
                    external_id: row.try_get("external_id")?,
                    handle: row.try_get("handle")?,
                    public_profile_url: row.try_get("public_profile_url")?,
                    roles: serde_json::from_value(row.try_get("roles")?)?,
                    lifecycle_stage: parse_audience_lifecycle_stage(
                        row.try_get::<String, _>("lifecycle_stage")?,
                    )?,
                    first_seen_at: row.try_get("first_seen_at")?,
                    last_seen_at: row.try_get("last_seen_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_audience_interaction(
        &self,
        interaction: &AudienceInteraction,
    ) -> DbResult<()> {
        sqlx::query(INSERT_AUDIENCE_INTERACTION_SQL)
            .bind(interaction.id)
            .bind(interaction.audience_member_id)
            .bind(&interaction.provider_event_id)
            .bind(format!("{:?}", interaction.kind))
            .bind(&interaction.public_url)
            .bind(interaction.occurred_at)
            .bind(&interaction.referrer_url)
            .bind(&interaction.campaign)
            .bind(interaction.source_interaction_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_audience_interaction_with_member(
        &self,
        member: &AudienceMember,
        interaction: &AudienceInteraction,
    ) -> DbResult<()> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(UPSERT_AUDIENCE_MEMBER_SQL)
            .bind(member.id)
            .bind(format!("{:?}", member.provider))
            .bind(&member.external_id)
            .bind(&member.handle)
            .bind(&member.public_profile_url)
            .bind(serde_json::to_value(&member.roles)?)
            .bind(format!("{:?}", member.lifecycle_stage))
            .bind(member.first_seen_at)
            .bind(member.last_seen_at)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(INSERT_AUDIENCE_INTERACTION_SQL)
            .bind(interaction.id)
            .bind(interaction.audience_member_id)
            .bind(&interaction.provider_event_id)
            .bind(format!("{:?}", interaction.kind))
            .bind(&interaction.public_url)
            .bind(interaction.occurred_at)
            .bind(&interaction.referrer_url)
            .bind(&interaction.campaign)
            .bind(interaction.source_interaction_id)
            .execute(&mut *transaction)
            .await?;

        let persisted = sqlx::query(
            r#"
            SELECT id, kind, public_url, referrer_url, campaign, source_interaction_id
            FROM audience_interactions
            WHERE audience_member_id = $1 AND provider_event_id = $2
            "#,
        )
        .bind(interaction.audience_member_id)
        .bind(&interaction.provider_event_id)
        .fetch_one(&mut *transaction)
        .await?;
        let persisted_id: Id = persisted.try_get("id")?;
        let persisted_kind: String = persisted.try_get("kind")?;
        let persisted_public_url: Option<String> = persisted.try_get("public_url")?;
        let persisted_referrer_url: Option<String> = persisted.try_get("referrer_url")?;
        let persisted_campaign: Option<String> = persisted.try_get("campaign")?;
        let persisted_source_interaction_id: Option<Id> =
            persisted.try_get("source_interaction_id")?;
        if persisted_id != interaction.id
            || persisted_kind != format!("{:?}", interaction.kind)
            || persisted_public_url != interaction.public_url
            || persisted_referrer_url != interaction.referrer_url
            || persisted_campaign != interaction.campaign
            || persisted_source_interaction_id != interaction.source_interaction_id
        {
            return Err(DbError::AudienceConflict(format!(
                "member={} provider_event_id={}",
                interaction.audience_member_id, interaction.provider_event_id
            )));
        }

        sqlx::query(
            r#"
            UPDATE audience_members
            SET lifecycle_stage = 'Retained'
            WHERE id = $1
              AND (SELECT COUNT(*) FROM audience_interactions WHERE audience_member_id = $1) >= 2
            "#,
        )
        .bind(member.id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn list_audience_interactions(&self) -> DbResult<Vec<AudienceInteraction>> {
        let rows = sqlx::query(
            r#"
            SELECT id, audience_member_id, provider_event_id, kind, public_url, occurred_at, referrer_url, campaign, source_interaction_id
            FROM audience_interactions
            ORDER BY occurred_at, id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(AudienceInteraction {
                    id: row.try_get("id")?,
                    audience_member_id: row.try_get("audience_member_id")?,
                    provider_event_id: row.try_get("provider_event_id")?,
                    kind: parse_audience_interaction_kind(row.try_get::<String, _>("kind")?)?,
                    public_url: row.try_get("public_url")?,
                    occurred_at: row.try_get("occurred_at")?,
                    referrer_url: row.try_get("referrer_url")?,
                    campaign: row.try_get("campaign")?,
                    source_interaction_id: row.try_get("source_interaction_id")?,
                })
            })
            .collect()
    }

    pub async fn upsert_discovery_response(&self, response: &DiscoveryResponse) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO discovery_responses
              (id, audience_member_id, interaction_id, provider_response_id, public_source_url, found_via, motivation, improvement_suggestion, agent_or_tool, private_storage_consent, captured_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (audience_member_id, provider_response_id) DO NOTHING
            "#,
        )
        .bind(response.id)
        .bind(response.audience_member_id)
        .bind(response.interaction_id)
        .bind(&response.provider_response_id)
        .bind(&response.public_source_url)
        .bind(&response.found_via)
        .bind(&response.motivation)
        .bind(&response.improvement_suggestion)
        .bind(&response.agent_or_tool)
        .bind(response.private_storage_consent)
        .bind(response.captured_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_discovery_responses(&self) -> DbResult<Vec<DiscoveryResponse>> {
        let rows = sqlx::query(
            r#"
            SELECT id, audience_member_id, interaction_id, provider_response_id, public_source_url, found_via, motivation, improvement_suggestion, agent_or_tool, private_storage_consent, captured_at
            FROM discovery_responses
            ORDER BY captured_at, id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(DiscoveryResponse {
                    id: row.try_get("id")?,
                    audience_member_id: row.try_get("audience_member_id")?,
                    interaction_id: row.try_get("interaction_id")?,
                    provider_response_id: row.try_get("provider_response_id")?,
                    public_source_url: row.try_get("public_source_url")?,
                    found_via: row.try_get("found_via")?,
                    motivation: row.try_get("motivation")?,
                    improvement_suggestion: row.try_get("improvement_suggestion")?,
                    agent_or_tool: row.try_get("agent_or_tool")?,
                    private_storage_consent: row.try_get("private_storage_consent")?,
                    captured_at: row.try_get("captured_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_outreach_attempt(&self, attempt: &OutreachAttempt) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO outreach_attempts
              (id, audience_member_id, provider_event_id, channel, public_url, prompt_version, status, consent_contact_id, sent_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (audience_member_id, provider_event_id) DO UPDATE SET
              status = CASE
                WHEN outreach_attempts.status IN ('Responded', 'Declined', 'Unreachable') THEN outreach_attempts.status
                ELSE EXCLUDED.status
              END
            "#,
        )
        .bind(attempt.id)
        .bind(attempt.audience_member_id)
        .bind(&attempt.provider_event_id)
        .bind(format!("{:?}", attempt.channel))
        .bind(&attempt.public_url)
        .bind(&attempt.prompt_version)
        .bind(format!("{:?}", attempt.status))
        .bind(attempt.consent_contact_id)
        .bind(attempt.sent_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_outreach_attempts(&self) -> DbResult<Vec<OutreachAttempt>> {
        let rows = sqlx::query(
            r#"
            SELECT id, audience_member_id, provider_event_id, channel, public_url, prompt_version, status, consent_contact_id, sent_at
            FROM outreach_attempts
            ORDER BY sent_at, id
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(OutreachAttempt {
                    id: row.try_get("id")?,
                    audience_member_id: row.try_get("audience_member_id")?,
                    provider_event_id: row.try_get("provider_event_id")?,
                    channel: parse_outreach_channel(row.try_get::<String, _>("channel")?)?,
                    public_url: row.try_get("public_url")?,
                    prompt_version: row.try_get("prompt_version")?,
                    status: parse_outreach_status(row.try_get::<String, _>("status")?)?,
                    consent_contact_id: row.try_get("consent_contact_id")?,
                    sent_at: row.try_get("sent_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_capability(&self, capability: &Capability) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO capabilities
              (id, agent_id, class, template_slugs, min_price, max_price, currency, latency_seconds, supported_verifiers)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE SET
              class = EXCLUDED.class,
              template_slugs = EXCLUDED.template_slugs,
              min_price = EXCLUDED.min_price,
              max_price = EXCLUDED.max_price,
              currency = EXCLUDED.currency,
              latency_seconds = EXCLUDED.latency_seconds,
              supported_verifiers = EXCLUDED.supported_verifiers
            "#,
        )
        .bind(capability.id)
        .bind(capability.agent_id)
        .bind(format!("{:?}", capability.class))
        .bind(serde_json::to_value(&capability.template_slugs)?)
        .bind(capability.min_price.amount)
        .bind(capability.max_price.amount)
        .bind(&capability.min_price.currency)
        .bind(i64::try_from(capability.latency_seconds).map_err(|_| {
            DbError::IntegerOverflow("capability.latency_seconds".to_string())
        })?)
        .bind(serde_json::to_value(&capability.supported_verifiers)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_capabilities(&self) -> DbResult<Vec<Capability>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, class, template_slugs, min_price, max_price, currency, latency_seconds, supported_verifiers
            FROM capabilities
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let currency: String = row.try_get("currency")?;
                Ok(Capability {
                    id: row.try_get("id")?,
                    agent_id: row.try_get("agent_id")?,
                    class: parse_capability_class(row.try_get::<String, _>("class")?)?,
                    template_slugs: serde_json::from_value(row.try_get("template_slugs")?)?,
                    min_price: Money::new(row.try_get::<i64, _>("min_price")?, currency.clone())?,
                    max_price: Money::new(row.try_get::<i64, _>("max_price")?, currency)?,
                    latency_seconds: u64::try_from(row.try_get::<i64, _>("latency_seconds")?)
                        .map_err(|_| DbError::IntegerOverflow("latency_seconds".to_string()))?,
                    supported_verifiers: serde_json::from_value(
                        row.try_get("supported_verifiers")?,
                    )?,
                })
            })
            .collect()
    }

    pub async fn upsert_help_request(&self, request: &HelpRequest) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO help_requests
              (id, requester_agent_id, goal, context, budget, currency, privacy, required_confidence, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE SET
              goal = EXCLUDED.goal,
              context = EXCLUDED.context,
              budget = EXCLUDED.budget,
              currency = EXCLUDED.currency,
              privacy = EXCLUDED.privacy,
              required_confidence = EXCLUDED.required_confidence
            "#,
        )
        .bind(request.id)
        .bind(request.requester_agent_id)
        .bind(&request.goal)
        .bind(&request.context)
        .bind(request.budget.amount)
        .bind(&request.budget.currency)
        .bind(format!("{:?}", request.privacy))
        .bind(request.required_confidence)
        .bind(request.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_help_requests(&self) -> DbResult<Vec<HelpRequest>> {
        let rows = sqlx::query(
            r#"
            SELECT id, requester_agent_id, goal, context, budget, currency, privacy, required_confidence, created_at
            FROM help_requests
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(HelpRequest {
                    id: row.try_get("id")?,
                    requester_agent_id: row.try_get("requester_agent_id")?,
                    goal: row.try_get("goal")?,
                    context: row.try_get("context")?,
                    budget: Money::new(
                        row.try_get::<i64, _>("budget")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    deadline: None,
                    privacy: parse_privacy(row.try_get::<String, _>("privacy")?)?,
                    required_confidence: row.try_get("required_confidence")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_quote(&self, quote: &Quote) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO quotes
              (id, help_request_id, solver_agent_id, price, currency, estimated_seconds, verifier_kind, confidence)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
              price = EXCLUDED.price,
              currency = EXCLUDED.currency,
              estimated_seconds = EXCLUDED.estimated_seconds,
              verifier_kind = EXCLUDED.verifier_kind,
              confidence = EXCLUDED.confidence
            "#,
        )
        .bind(quote.id)
        .bind(quote.help_request_id)
        .bind(quote.solver_agent_id)
        .bind(quote.price.amount)
        .bind(&quote.price.currency)
        .bind(i64::try_from(quote.estimated_seconds).map_err(|_| {
            DbError::IntegerOverflow("quote.estimated_seconds".to_string())
        })?)
        .bind(format!("{:?}", quote.verifier_kind))
        .bind(quote.confidence)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_quotes(&self) -> DbResult<Vec<Quote>> {
        let rows = sqlx::query(
            r#"
            SELECT id, help_request_id, solver_agent_id, price, currency, estimated_seconds, verifier_kind, confidence
            FROM quotes
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Quote {
                    id: row.try_get("id")?,
                    help_request_id: row.try_get("help_request_id")?,
                    solver_agent_id: row.try_get("solver_agent_id")?,
                    price: Money::new(
                        row.try_get::<i64, _>("price")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    estimated_seconds: u64::try_from(row.try_get::<i64, _>("estimated_seconds")?)
                        .map_err(|_| {
                        DbError::IntegerOverflow("estimated_seconds".to_string())
                    })?,
                    verifier_kind: parse_verifier_kind(row.try_get::<String, _>("verifier_kind")?)?,
                    confidence: row.try_get("confidence")?,
                })
            })
            .collect()
    }

    pub async fn upsert_bounty(&self, bounty: &Bounty) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO bounties
              (id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
              help_request_id = EXCLUDED.help_request_id,
              title = EXCLUDED.title,
              template_slug = EXCLUDED.template_slug,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              funding_targets = EXCLUDED.funding_targets,
              funding_mode = EXCLUDED.funding_mode,
              privacy = EXCLUDED.privacy,
              status = EXCLUDED.status,
              terms_hash = EXCLUDED.terms_hash
            "#,
        )
        .bind(bounty.id)
        .bind(bounty.help_request_id)
        .bind(&bounty.title)
        .bind(&bounty.template_slug)
        .bind(bounty.amount.amount)
        .bind(&bounty.amount.currency)
        .bind(serde_json::to_value(&bounty.funding_targets)?)
        .bind(format!("{:?}", bounty.funding_mode))
        .bind(format!("{:?}", bounty.privacy))
        .bind(format!("{:?}", bounty.status))
        .bind(&bounty.terms_hash)
        .bind(bounty.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_github_issue_sync_bounty(
        &self,
        bounty: &Bounty,
    ) -> DbResult<GitHubIssueSyncBountyUpsert> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(LOCK_GITHUB_ISSUE_SYNC_BOUNTY_SQL)
            .bind(bounty.id)
            .fetch_one(&mut *tx)
            .await?;
        let existing = sqlx::query(SELECT_GITHUB_ISSUE_SYNC_BOUNTY_FOR_UPDATE_SQL)
            .bind(bounty.id)
            .fetch_optional(&mut *tx)
            .await?;

        if let Some(row) = existing {
            let existing_bounty = bounty_from_row(&row)?;
            let has_activity: bool = sqlx::query(GITHUB_ISSUE_SYNC_ACTIVITY_SQL)
                .bind(bounty.id)
                .fetch_one(&mut *tx)
                .await?
                .try_get("has_activity")?;

            if existing_bounty.status != BountyStatus::Unfunded || has_activity {
                tx.commit().await?;
                return Ok(GitHubIssueSyncBountyUpsert::BlockedByActivity(
                    existing_bounty,
                ));
            }

            let updated = sqlx::query(UPDATE_GITHUB_ISSUE_SYNC_BOUNTY_SQL)
                .bind(bounty.id)
                .bind(bounty.help_request_id)
                .bind(&bounty.title)
                .bind(&bounty.template_slug)
                .bind(bounty.amount.amount)
                .bind(&bounty.amount.currency)
                .bind(serde_json::to_value(&bounty.funding_targets)?)
                .bind(format!("{:?}", bounty.funding_mode))
                .bind(format!("{:?}", bounty.privacy))
                .bind(format!("{:?}", bounty.status))
                .bind(&bounty.terms_hash)
                .fetch_one(&mut *tx)
                .await?;
            let updated = bounty_from_row(&updated)?;
            tx.commit().await?;
            return Ok(GitHubIssueSyncBountyUpsert::Upserted(updated));
        }

        let inserted = sqlx::query(INSERT_GITHUB_ISSUE_SYNC_BOUNTY_SQL)
            .bind(bounty.id)
            .bind(bounty.help_request_id)
            .bind(&bounty.title)
            .bind(&bounty.template_slug)
            .bind(bounty.amount.amount)
            .bind(&bounty.amount.currency)
            .bind(serde_json::to_value(&bounty.funding_targets)?)
            .bind(format!("{:?}", bounty.funding_mode))
            .bind(format!("{:?}", bounty.privacy))
            .bind(format!("{:?}", bounty.status))
            .bind(&bounty.terms_hash)
            .bind(bounty.created_at)
            .fetch_one(&mut *tx)
            .await?;
        let inserted = bounty_from_row(&inserted)?;
        tx.commit().await?;
        Ok(GitHubIssueSyncBountyUpsert::Upserted(inserted))
    }

    pub async fn list_bounties(&self) -> DbResult<Vec<Bounty>> {
        let rows = sqlx::query(
            r#"
            SELECT id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            FROM bounties
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(|row| bounty_from_row(&row)).collect()
    }

    pub async fn load_bounty_status_scope(
        &self,
        bounty_id: Id,
    ) -> DbResult<Option<BountyStatusScope>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;

        let bounty = sqlx::query(
            r#"
            SELECT id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            FROM bounties
            WHERE id = $1
            "#,
        )
        .bind(bounty_id)
        .fetch_optional(&mut *tx)
        .await?
        .map(|row| bounty_from_row(&row))
        .transpose()?;

        let Some(bounty) = bounty else {
            tx.commit().await?;
            return Ok(None);
        };

        let funding_intents = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, stripe_success_url, stripe_cancel_url, created_at
            FROM funding_intents
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(FundingIntent {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                contributor_agent_id: row.try_get("contributor_agent_id")?,
                source_organization_id: row.try_get("source_organization_id")?,
                rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                amount: Money::new(
                    row.try_get::<i64, _>("amount")?,
                    row.try_get::<String, _>("currency")?,
                )?,
                status: parse_funding_intent_status(row.try_get::<String, _>("status")?)?,
                external_reference: row.try_get("external_reference")?,
                stripe_success_url: row.try_get("stripe_success_url")?,
                stripe_cancel_url: row.try_get("stripe_cancel_url")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let funding_contributions = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, funding_ledger_entry_id, refund_ledger_entry_id, settlement_id, external_reference, created_at
            FROM funding_contributions
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(FundingContribution {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                contributor_agent_id: row.try_get("contributor_agent_id")?,
                source_organization_id: row.try_get("source_organization_id")?,
                rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                amount: Money::new(
                    row.try_get::<i64, _>("amount")?,
                    row.try_get::<String, _>("currency")?,
                )?,
                status: parse_funding_contribution_status(row.try_get::<String, _>("status")?)?,
                funding_ledger_entry_id: row.try_get("funding_ledger_entry_id")?,
                refund_ledger_entry_id: row.try_get("refund_ledger_entry_id")?,
                settlement_id: row.try_get("settlement_id")?,
                external_reference: row.try_get("external_reference")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let escrows = sqlx::query(
            r#"
            SELECT id, bounty_id, rail, token, amount, currency, status, external_reference
            FROM escrows
            WHERE bounty_id = $1
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(Escrow {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                token: row.try_get("token")?,
                amount: Money::new(
                    row.try_get::<i64, _>("amount")?,
                    row.try_get::<String, _>("currency")?,
                )?,
                status: parse_escrow_status(row.try_get::<String, _>("status")?)?,
                external_reference: row.try_get("external_reference")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let claims = sqlx::query(
            r#"
            SELECT id, bounty_id, solver_agent_id, claimed_at
            FROM claims
            WHERE bounty_id = $1
            ORDER BY claimed_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(Claim {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                solver_agent_id: row.try_get("solver_agent_id")?,
                claimed_at: row.try_get("claimed_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let submissions = sqlx::query(
            r#"
            SELECT id, bounty_id, solver_agent_id, artifact_digest, artifact_uri, submitted_at
            FROM submissions
            WHERE bounty_id = $1
            ORDER BY submitted_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(Submission {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                solver_agent_id: row.try_get("solver_agent_id")?,
                artifact_digest: row.try_get("artifact_digest")?,
                artifact_uri: row.try_get("artifact_uri")?,
                submitted_at: row.try_get("submitted_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let verifier_results = sqlx::query(
            r#"
            SELECT id, bounty_id, submission_id, verifier_agent_id, kind, decision, summary, confidence, signed_payload_hash, created_at
            FROM verifier_results
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(VerifierResult {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                submission_id: row.try_get("submission_id")?,
                verifier_agent_id: row.try_get("verifier_agent_id")?,
                kind: parse_verifier_kind(row.try_get::<String, _>("kind")?)?,
                decision: parse_verification_decision(row.try_get::<String, _>("decision")?)?,
                summary: row.try_get("summary")?,
                confidence: row.try_get("confidence")?,
                signed_payload_hash: row.try_get("signed_payload_hash")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let proofs = sqlx::query(
            r#"
            SELECT id, bounty_id, submission_id, verifier_result_id, proof_hash, public_summary, privacy, created_at
            FROM proof_records
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ProofRecord {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                submission_id: row.try_get("submission_id")?,
                verifier_result_id: row.try_get("verifier_result_id")?,
                proof_hash: row.try_get("proof_hash")?,
                public_summary: row.try_get("public_summary")?,
                privacy: parse_privacy(row.try_get::<String, _>("privacy")?)?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let settlements = sqlx::query(
            r#"
            SELECT id, bounty_id, proof_record_id, rail, payout_intents, platform_fee, currency, created_at
            FROM settlements
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            let platform_fee_amount = row.try_get::<i64, _>("platform_fee")?;
            let currency = row.try_get::<String, _>("currency")?;
            let platform_fee = persisted_nonnegative_money(platform_fee_amount, currency)?;
            Ok(Settlement {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                proof_record_id: row.try_get("proof_record_id")?,
                rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                payout_intents: serde_json::from_value(row.try_get("payout_intents")?)?,
                platform_fee,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let reputation_events = sqlx::query(
            r#"
            SELECT id, agent_id, bounty_id, capability_class, template_slug, delta, reason, created_at
            FROM reputation_events
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(ReputationEvent {
                id: row.try_get("id")?,
                agent_id: row.try_get("agent_id")?,
                bounty_id: row.try_get("bounty_id")?,
                capability_class: parse_capability_class(
                    row.try_get::<String, _>("capability_class")?,
                )?,
                template_slug: row.try_get("template_slug")?,
                delta: row.try_get("delta")?,
                reason: row.try_get("reason")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let template_signals = sqlx::query(
            r#"
            SELECT id, bounty_id, proof_record_id, template_slug, capability_class, verifier_kind, amount, currency, success, created_at
            FROM template_signals
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            Ok(TemplateSignal {
                id: row.try_get("id")?,
                bounty_id: row.try_get("bounty_id")?,
                proof_record_id: row.try_get("proof_record_id")?,
                template_slug: row.try_get("template_slug")?,
                capability_class: parse_capability_class(
                    row.try_get::<String, _>("capability_class")?,
                )?,
                verifier_kind: parse_verifier_kind(row.try_get::<String, _>("verifier_kind")?)?,
                amount: Money::new(
                    row.try_get::<i64, _>("amount")?,
                    row.try_get::<String, _>("currency")?,
                )?,
                success: row.try_get("success")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        let risk_events = sqlx::query(
            r#"
            SELECT id, subject_id, agent_id, bounty_id, surface, action, score, reasons, created_at
            FROM risk_events
            WHERE bounty_id = $1
            ORDER BY created_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(|row| {
            let score: i32 = row.try_get("score")?;
            Ok(RiskEvent {
                id: row.try_get("id")?,
                subject_id: row.try_get("subject_id")?,
                agent_id: row.try_get("agent_id")?,
                bounty_id: row.try_get("bounty_id")?,
                surface: parse_risk_surface(row.try_get::<String, _>("surface")?)?,
                action: parse_risk_action(row.try_get::<String, _>("action")?)?,
                score: u16::try_from(score)
                    .map_err(|_| DbError::IntegerOverflow("risk_event.score".to_string()))?,
                reasons: serde_json::from_value(row.try_get("reasons")?)?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<DbResult<Vec<_>>>()?;

        tx.commit().await?;
        Ok(Some(BountyStatusScope {
            bounty,
            funding_intents,
            funding_contributions,
            escrows,
            claims,
            submissions,
            verifier_results,
            proofs,
            settlements,
            reputation_events,
            template_signals,
            risk_events,
        }))
    }

    pub async fn upsert_funding_contribution(
        &self,
        contribution: &FundingContribution,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO funding_contributions
              (id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, funding_ledger_entry_id, refund_ledger_entry_id, settlement_id, external_reference, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (id) DO UPDATE SET
              contributor_agent_id = EXCLUDED.contributor_agent_id,
              source_organization_id = EXCLUDED.source_organization_id,
              rail = EXCLUDED.rail,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              status = EXCLUDED.status,
              funding_ledger_entry_id = EXCLUDED.funding_ledger_entry_id,
              refund_ledger_entry_id = EXCLUDED.refund_ledger_entry_id,
              settlement_id = EXCLUDED.settlement_id,
              external_reference = EXCLUDED.external_reference
            "#,
        )
        .bind(contribution.id)
        .bind(contribution.bounty_id)
        .bind(contribution.contributor_agent_id)
        .bind(contribution.source_organization_id)
        .bind(format!("{:?}", contribution.rail))
        .bind(contribution.amount.amount)
        .bind(&contribution.amount.currency)
        .bind(format!("{:?}", contribution.status))
        .bind(contribution.funding_ledger_entry_id)
        .bind(contribution.refund_ledger_entry_id)
        .bind(contribution.settlement_id)
        .bind(&contribution.external_reference)
        .bind(contribution.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_funding_contributions(&self) -> DbResult<Vec<FundingContribution>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, funding_ledger_entry_id, refund_ledger_entry_id, settlement_id, external_reference, created_at
            FROM funding_contributions
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(FundingContribution {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    contributor_agent_id: row.try_get("contributor_agent_id")?,
                    source_organization_id: row.try_get("source_organization_id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    amount: Money::new(
                        row.try_get::<i64, _>("amount")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    status: parse_funding_contribution_status(row.try_get::<String, _>("status")?)?,
                    funding_ledger_entry_id: row.try_get("funding_ledger_entry_id")?,
                    refund_ledger_entry_id: row.try_get("refund_ledger_entry_id")?,
                    settlement_id: row.try_get("settlement_id")?,
                    external_reference: row.try_get("external_reference")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_funding_intent(&self, intent: &FundingIntent) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO funding_intents
              (id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, stripe_success_url, stripe_cancel_url, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            ON CONFLICT (id) DO UPDATE SET
              contributor_agent_id = EXCLUDED.contributor_agent_id,
              source_organization_id = EXCLUDED.source_organization_id,
              rail = EXCLUDED.rail,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              status = EXCLUDED.status,
              external_reference = EXCLUDED.external_reference,
              stripe_success_url = EXCLUDED.stripe_success_url,
              stripe_cancel_url = EXCLUDED.stripe_cancel_url
            "#,
        )
        .bind(intent.id)
        .bind(intent.bounty_id)
        .bind(intent.contributor_agent_id)
        .bind(intent.source_organization_id)
        .bind(format!("{:?}", intent.rail))
        .bind(intent.amount.amount)
        .bind(&intent.amount.currency)
        .bind(format!("{:?}", intent.status))
        .bind(&intent.external_reference)
        .bind(&intent.stripe_success_url)
        .bind(&intent.stripe_cancel_url)
        .bind(intent.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_funding_intents(&self) -> DbResult<Vec<FundingIntent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, stripe_success_url, stripe_cancel_url, created_at
            FROM funding_intents
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(FundingIntent {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    contributor_agent_id: row.try_get("contributor_agent_id")?,
                    source_organization_id: row.try_get("source_organization_id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    amount: Money::new(
                        row.try_get::<i64, _>("amount")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    status: parse_funding_intent_status(row.try_get::<String, _>("status")?)?,
                    external_reference: row.try_get("external_reference")?,
                    stripe_success_url: row.try_get("stripe_success_url")?,
                    stripe_cancel_url: row.try_get("stripe_cancel_url")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_escrow(&self, escrow: &Escrow) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO escrows
              (id, bounty_id, rail, token, amount, currency, status, external_reference)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
              bounty_id = EXCLUDED.bounty_id,
              rail = EXCLUDED.rail,
              token = EXCLUDED.token,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              status = EXCLUDED.status,
              external_reference = EXCLUDED.external_reference
            "#,
        )
        .bind(escrow.id)
        .bind(escrow.bounty_id)
        .bind(format!("{:?}", escrow.rail))
        .bind(&escrow.token)
        .bind(escrow.amount.amount)
        .bind(&escrow.amount.currency)
        .bind(format!("{:?}", escrow.status))
        .bind(&escrow.external_reference)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_escrows(&self) -> DbResult<Vec<Escrow>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, rail, token, amount, currency, status, external_reference
            FROM escrows
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Escrow {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    token: row.try_get("token")?,
                    amount: Money::new(
                        row.try_get::<i64, _>("amount")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    status: parse_escrow_status(row.try_get::<String, _>("status")?)?,
                    external_reference: row.try_get("external_reference")?,
                })
            })
            .collect()
    }

    pub async fn upsert_autonomous_bounty_event(
        &self,
        network: &str,
        event: &AutonomousBountyEvent,
    ) -> DbResult<()> {
        let kind = serde_json::to_value(event.kind)?
            .as_str()
            .ok_or_else(|| DbError::InvalidEnum("autonomous bounty event kind".to_string()))?
            .to_string();
        sqlx::query(
            r#"
            INSERT INTO autonomous_bounty_events
              (id, log_key, network, tx_hash, block_number, log_index, contract_address, bounty_id, kind, data, occurred_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (log_key) DO UPDATE SET
              network = EXCLUDED.network,
              tx_hash = EXCLUDED.tx_hash,
              block_number = EXCLUDED.block_number,
              log_index = EXCLUDED.log_index,
              contract_address = EXCLUDED.contract_address,
              bounty_id = EXCLUDED.bounty_id,
              kind = EXCLUDED.kind,
              data = EXCLUDED.data,
              occurred_at = CASE
                WHEN autonomous_bounty_events.block_time_verified
                  THEN autonomous_bounty_events.occurred_at
                ELSE EXCLUDED.occurred_at
              END
            "#,
        )
        .bind(event.id)
        .bind(&event.log_key)
        .bind(network)
        .bind(&event.tx_hash)
        .bind(i64_from_u64(event.block_number)?)
        .bind(i64_from_u64(event.log_index)?)
        .bind(normalize_key_address(&event.contract_address))
        .bind(&event.bounty_id)
        .bind(kind)
        .bind(&event.data)
        .bind(event.occurred_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_autonomous_bounty_events(
        &self,
        network: &str,
    ) -> DbResult<Vec<AutonomousBountyEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, log_index, contract_address,
                   bounty_id, kind, data, occurred_at
            FROM autonomous_bounty_events
            WHERE network = $1
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(autonomous_event_from_row).collect()
    }

    pub async fn list_unverified_autonomous_event_blocks(
        &self,
        network: &str,
        limit: u32,
    ) -> DbResult<Vec<u64>> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT block_number
            FROM autonomous_bounty_events
            WHERE network = $1 AND block_time_verified = FALSE
            ORDER BY block_number
            LIMIT $2
            "#,
        )
        .bind(network)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| u64_from_i64(row.try_get("block_number")?))
            .collect()
    }

    pub async fn confirm_autonomous_event_block_time(
        &self,
        network: &str,
        block_number: u64,
        occurred_at: DateTime<Utc>,
    ) -> DbResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE autonomous_bounty_events
            SET occurred_at = $3, block_time_verified = TRUE
            WHERE network = $1 AND block_number = $2
            "#,
        )
        .bind(network)
        .bind(i64_from_u64(block_number)?)
        .bind(occurred_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_canonical_solver_completions(
        &self,
        network: &str,
        starts_at: DateTime<Utc>,
        ends_at: DateTime<Utc>,
    ) -> DbResult<Vec<CanonicalSolverCompletion>> {
        let rows = sqlx::query(
            r#"
            SELECT settled.bounty_id,
                   settled.contract_address AS bounty_contract,
                   lower(settled.data->>'solver') AS solver_wallet,
                   lower(created.data->>'creator') AS creator_wallet,
                   settled.data->>'solver_reward' AS solver_reward,
                   settled.occurred_at,
                   settled.block_number,
                   settled.log_index,
                   COALESCE(
                     terms.document->'benchmark'->>'engine' = 'standing_meta_v2_parent',
                     FALSE
                   ) AS standing_meta_bounty
            FROM autonomous_bounty_events settled
            JOIN LATERAL (
              SELECT event.data
              FROM autonomous_bounty_events event
              WHERE event.network = settled.network
                AND event.bounty_id = settled.bounty_id
                AND event.kind = 'canonical_bounty_created'
              ORDER BY event.block_number, event.log_index
              LIMIT 1
            ) created ON TRUE
            LEFT JOIN autonomous_bounty_terms terms
              ON lower(terms.terms_hash) = lower(created.data->>'terms_hash')
            WHERE settled.network = $1
              AND settled.kind = 'bounty_settled'
              AND settled.block_time_verified = TRUE
              AND settled.occurred_at >= $2
              AND settled.occurred_at < $3
            ORDER BY settled.occurred_at, settled.block_number, settled.log_index
            "#,
        )
        .bind(network)
        .bind(starts_at)
        .bind(ends_at)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let reward = row.try_get::<String, _>("solver_reward")?;
                Ok(CanonicalSolverCompletion {
                    bounty_id: row.try_get("bounty_id")?,
                    bounty_contract: row.try_get("bounty_contract")?,
                    solver_wallet: row.try_get("solver_wallet")?,
                    creator_wallet: row.try_get("creator_wallet")?,
                    solver_reward_usdc_base_units: reward
                        .parse::<u64>()
                        .map_err(|_| DbError::IntegerOverflow(format!("solver reward {reward}")))?,
                    occurred_at: row.try_get("occurred_at")?,
                    block_number: u64_from_i64(row.try_get("block_number")?)?,
                    log_index: u64_from_i64(row.try_get("log_index")?)?,
                    standing_meta_bounty: row.try_get("standing_meta_bounty")?,
                })
            })
            .collect()
    }

    pub async fn list_canonical_autonomous_bounty_contracts(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<String>> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT data->>'bounty_contract' AS bounty_contract
            FROM autonomous_bounty_events
            WHERE network = $1
              AND contract_address = $2
              AND kind = 'canonical_bounty_created'
              AND data ? 'bounty_contract'
            ORDER BY bounty_contract
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let address: String = row.try_get("bounty_contract")?;
                Ok(normalize_key_address(&address))
            })
            .collect()
    }

    pub async fn upsert_autonomous_bounty_terms(
        &self,
        record: &AutonomousBountyTermsRecord,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO autonomous_bounty_terms
              (terms_hash, policy_hash, acceptance_criteria_hash, benchmark_hash,
               evidence_schema_hash, creator_wallet, document, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (terms_hash) DO UPDATE SET
              policy_hash = EXCLUDED.policy_hash,
              acceptance_criteria_hash = EXCLUDED.acceptance_criteria_hash,
              benchmark_hash = EXCLUDED.benchmark_hash,
              evidence_schema_hash = EXCLUDED.evidence_schema_hash,
              creator_wallet = EXCLUDED.creator_wallet,
              document = EXCLUDED.document,
              created_at = LEAST(autonomous_bounty_terms.created_at, EXCLUDED.created_at)
            "#,
        )
        .bind(&record.terms_hash)
        .bind(&record.policy_hash)
        .bind(&record.acceptance_criteria_hash)
        .bind(&record.benchmark_hash)
        .bind(&record.evidence_schema_hash)
        .bind(normalize_key_address(&record.creator_wallet))
        .bind(serde_json::to_value(&record.document)?)
        .bind(record.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_autonomous_bounty_terms(
        &self,
        terms_hash: &str,
    ) -> DbResult<Option<AutonomousBountyTermsRecord>> {
        let row = sqlx::query(
            r#"
            SELECT terms_hash, policy_hash, acceptance_criteria_hash, benchmark_hash,
                   evidence_schema_hash, creator_wallet, document, created_at
            FROM autonomous_bounty_terms
            WHERE terms_hash = $1
            "#,
        )
        .bind(terms_hash.to_ascii_lowercase())
        .fetch_optional(&self.pool)
        .await?;
        row.map(autonomous_terms_from_row).transpose()
    }

    pub async fn list_autonomous_bounty_terms(&self) -> DbResult<Vec<AutonomousBountyTermsRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT terms_hash, policy_hash, acceptance_criteria_hash, benchmark_hash,
                   evidence_schema_hash, creator_wallet, document, created_at
            FROM autonomous_bounty_terms
            ORDER BY created_at DESC, terms_hash
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(autonomous_terms_from_row).collect()
    }

    pub async fn upsert_autonomous_submission_evidence(
        &self,
        record: &AutonomousSubmissionEvidenceRecord,
    ) -> DbResult<AutonomousSubmissionEvidenceRecord> {
        sqlx::query(
            r#"
            INSERT INTO autonomous_submission_evidence
              (network, bounty_contract, bounty_id, round, solver_wallet,
               artifact_reference, artifact_hash, evidence, evidence_hash, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (network, bounty_contract, round) DO NOTHING
            "#,
        )
        .bind(&record.network)
        .bind(normalize_key_address(&record.bounty_contract))
        .bind(record.bounty_id.to_ascii_lowercase())
        .bind(i64_from_u64(record.round)?)
        .bind(normalize_key_address(&record.solver_wallet))
        .bind(&record.artifact_reference)
        .bind(record.artifact_hash.to_ascii_lowercase())
        .bind(&record.evidence)
        .bind(record.evidence_hash.to_ascii_lowercase())
        .bind(record.created_at)
        .execute(&self.pool)
        .await?;
        let persisted = self
            .get_autonomous_submission_evidence(
                &record.network,
                &record.bounty_contract,
                record.round,
            )
            .await?
            .ok_or_else(|| {
                DbError::AutonomousEvidenceConflict(
                    "record disappeared after immutable upsert".to_string(),
                )
            })?;
        if !persisted.bounty_id.eq_ignore_ascii_case(&record.bounty_id)
            || !persisted
                .solver_wallet
                .eq_ignore_ascii_case(&record.solver_wallet)
            || persisted.artifact_reference != record.artifact_reference
            || !persisted
                .artifact_hash
                .eq_ignore_ascii_case(&record.artifact_hash)
            || persisted.evidence != record.evidence
            || !persisted
                .evidence_hash
                .eq_ignore_ascii_case(&record.evidence_hash)
        {
            return Err(DbError::AutonomousEvidenceConflict(format!(
                "{} round {}",
                record.bounty_contract, record.round
            )));
        }
        Ok(persisted)
    }

    pub async fn get_autonomous_submission_evidence(
        &self,
        network: &str,
        bounty_contract: &str,
        round: u64,
    ) -> DbResult<Option<AutonomousSubmissionEvidenceRecord>> {
        let row = sqlx::query(
            r#"
            SELECT network, bounty_contract, bounty_id, round, solver_wallet,
                   artifact_reference, artifact_hash, evidence, evidence_hash, created_at
            FROM autonomous_submission_evidence
            WHERE network = $1 AND bounty_contract = $2 AND round = $3
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(bounty_contract))
        .bind(i64_from_u64(round)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(autonomous_submission_evidence_from_row).transpose()
    }

    pub async fn list_autonomous_submission_evidence(
        &self,
        network: &str,
    ) -> DbResult<Vec<AutonomousSubmissionEvidenceRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT network, bounty_contract, bounty_id, round, solver_wallet,
                   artifact_reference, artifact_hash, evidence, evidence_hash, created_at
            FROM autonomous_submission_evidence
            WHERE network = $1
            ORDER BY created_at, bounty_contract, round
            "#,
        )
        .bind(network)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(autonomous_submission_evidence_from_row)
            .collect()
    }

    pub async fn get_base_log_cursor(
        &self,
        network: &str,
        escrow_contract: &str,
    ) -> DbResult<Option<BaseLogScanCursor>> {
        let row = sqlx::query(
            r#"
            SELECT network, escrow_contract, last_scanned_block, last_log_key, updated_at
            FROM base_log_cursors
            WHERE network = $1 AND escrow_contract = $2
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(escrow_contract))
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(BaseLogScanCursor {
                network: row.try_get("network")?,
                escrow_contract: row.try_get("escrow_contract")?,
                last_scanned_block: u64_from_i64(row.try_get("last_scanned_block")?)?,
                last_log_key: row.try_get("last_log_key")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .transpose()
    }

    pub async fn upsert_base_log_cursor(
        &self,
        network: &str,
        escrow_contract: &str,
        last_scanned_block: u64,
        last_log_key: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO base_log_cursors
              (network, escrow_contract, last_scanned_block, last_log_key, updated_at)
            VALUES ($1, $2, $3, $4, now())
            ON CONFLICT (network, escrow_contract) DO UPDATE SET
              last_scanned_block = GREATEST(base_log_cursors.last_scanned_block, EXCLUDED.last_scanned_block),
              last_log_key = COALESCE(EXCLUDED.last_log_key, base_log_cursors.last_log_key),
              updated_at = now()
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(escrow_contract))
        .bind(i64_from_u64(last_scanned_block)?)
        .bind(last_log_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_base_indexer_heartbeat(
        &self,
        network: &str,
        escrow_contract: &str,
    ) -> DbResult<Option<BaseIndexerHeartbeat>> {
        let row = sqlx::query(
            r#"
            SELECT network, escrow_contract, status, started_at, completed_at,
                   latest_block, confirmed_to_block, from_block, to_block,
                   fetched_logs, persisted_cursor_block, skipped_reason,
                   error_message, updated_at
            FROM base_indexer_heartbeats
            WHERE network = $1 AND escrow_contract = $2
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(escrow_contract))
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            Ok(BaseIndexerHeartbeat {
                network: row.try_get("network")?,
                escrow_contract: row.try_get("escrow_contract")?,
                status: row.try_get("status")?,
                started_at: row.try_get("started_at")?,
                completed_at: row.try_get("completed_at")?,
                latest_block: optional_u64_from_i64(row.try_get("latest_block")?)?,
                confirmed_to_block: optional_u64_from_i64(row.try_get("confirmed_to_block")?)?,
                from_block: optional_u64_from_i64(row.try_get("from_block")?)?,
                to_block: optional_u64_from_i64(row.try_get("to_block")?)?,
                fetched_logs: u64_from_i64(row.try_get("fetched_logs")?)?,
                persisted_cursor_block: optional_u64_from_i64(
                    row.try_get("persisted_cursor_block")?,
                )?,
                skipped_reason: row.try_get("skipped_reason")?,
                error_message: row.try_get("error_message")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .transpose()
    }

    pub async fn upsert_base_indexer_heartbeat(
        &self,
        heartbeat: &BaseIndexerHeartbeat,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO base_indexer_heartbeats
              (network, escrow_contract, status, started_at, completed_at,
               latest_block, confirmed_to_block, from_block, to_block,
               fetched_logs, persisted_cursor_block, skipped_reason,
               error_message, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, now())
            ON CONFLICT (network, escrow_contract) DO UPDATE SET
              status = EXCLUDED.status,
              started_at = EXCLUDED.started_at,
              completed_at = EXCLUDED.completed_at,
              latest_block = EXCLUDED.latest_block,
              confirmed_to_block = EXCLUDED.confirmed_to_block,
              from_block = EXCLUDED.from_block,
              to_block = EXCLUDED.to_block,
              fetched_logs = EXCLUDED.fetched_logs,
              persisted_cursor_block = EXCLUDED.persisted_cursor_block,
              skipped_reason = EXCLUDED.skipped_reason,
              error_message = EXCLUDED.error_message,
              updated_at = now()
            "#,
        )
        .bind(&heartbeat.network)
        .bind(normalize_key_address(&heartbeat.escrow_contract))
        .bind(&heartbeat.status)
        .bind(heartbeat.started_at)
        .bind(heartbeat.completed_at)
        .bind(optional_i64_from_u64(heartbeat.latest_block)?)
        .bind(optional_i64_from_u64(heartbeat.confirmed_to_block)?)
        .bind(optional_i64_from_u64(heartbeat.from_block)?)
        .bind(optional_i64_from_u64(heartbeat.to_block)?)
        .bind(i64_from_u64(heartbeat.fetched_logs)?)
        .bind(optional_i64_from_u64(heartbeat.persisted_cursor_block)?)
        .bind(&heartbeat.skipped_reason)
        .bind(&heartbeat.error_message)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_claim(&self, claim: &Claim) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO claims (id, bounty_id, solver_agent_id, claimed_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (bounty_id) DO UPDATE SET
              solver_agent_id = EXCLUDED.solver_agent_id
            "#,
        )
        .bind(claim.id)
        .bind(claim.bounty_id)
        .bind(claim.solver_agent_id)
        .bind(claim.claimed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_claims(&self) -> DbResult<Vec<Claim>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, solver_agent_id, claimed_at
            FROM claims
            ORDER BY claimed_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Claim {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    solver_agent_id: row.try_get("solver_agent_id")?,
                    claimed_at: row.try_get("claimed_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_submission(&self, submission: &Submission) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO submissions (id, bounty_id, solver_agent_id, artifact_digest, artifact_uri, submitted_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (id) DO UPDATE SET
              artifact_digest = EXCLUDED.artifact_digest,
              artifact_uri = EXCLUDED.artifact_uri
            "#,
        )
        .bind(submission.id)
        .bind(submission.bounty_id)
        .bind(submission.solver_agent_id)
        .bind(&submission.artifact_digest)
        .bind(&submission.artifact_uri)
        .bind(submission.submitted_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_submissions(&self) -> DbResult<Vec<Submission>> {
        let rows = sqlx::query(
            "SELECT id, bounty_id, solver_agent_id, artifact_digest, artifact_uri, submitted_at FROM submissions",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Submission {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    solver_agent_id: row.try_get("solver_agent_id")?,
                    artifact_digest: row.try_get("artifact_digest")?,
                    artifact_uri: row.try_get("artifact_uri")?,
                    submitted_at: row.try_get("submitted_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_verifier_result(&self, result: &VerifierResult) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO verifier_results
              (id, bounty_id, submission_id, verifier_agent_id, kind, decision, summary, confidence, signed_payload_hash, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
              kind = EXCLUDED.kind,
              decision = EXCLUDED.decision,
              summary = EXCLUDED.summary,
              confidence = EXCLUDED.confidence,
              signed_payload_hash = EXCLUDED.signed_payload_hash
            "#,
        )
        .bind(result.id)
        .bind(result.bounty_id)
        .bind(result.submission_id)
        .bind(result.verifier_agent_id)
        .bind(format!("{:?}", result.kind))
        .bind(format!("{:?}", result.decision))
        .bind(&result.summary)
        .bind(result.confidence)
        .bind(&result.signed_payload_hash)
        .bind(result.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_verifier_results(&self) -> DbResult<Vec<VerifierResult>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, submission_id, verifier_agent_id, kind, decision, summary, confidence, signed_payload_hash, created_at
            FROM verifier_results
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(VerifierResult {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    submission_id: row.try_get("submission_id")?,
                    verifier_agent_id: row.try_get("verifier_agent_id")?,
                    kind: parse_verifier_kind(row.try_get::<String, _>("kind")?)?,
                    decision: parse_verification_decision(row.try_get::<String, _>("decision")?)?,
                    summary: row.try_get("summary")?,
                    confidence: row.try_get("confidence")?,
                    signed_payload_hash: row.try_get("signed_payload_hash")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_proof_record(&self, proof: &ProofRecord) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO proof_records
              (id, bounty_id, submission_id, verifier_result_id, proof_hash, public_summary, privacy, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
              proof_hash = EXCLUDED.proof_hash,
              public_summary = EXCLUDED.public_summary,
              privacy = EXCLUDED.privacy
            "#,
        )
        .bind(proof.id)
        .bind(proof.bounty_id)
        .bind(proof.submission_id)
        .bind(proof.verifier_result_id)
        .bind(&proof.proof_hash)
        .bind(&proof.public_summary)
        .bind(format!("{:?}", proof.privacy))
        .bind(proof.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_proof_records(&self) -> DbResult<Vec<ProofRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, submission_id, verifier_result_id, proof_hash, public_summary, privacy, created_at
            FROM proof_records
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(ProofRecord {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    submission_id: row.try_get("submission_id")?,
                    verifier_result_id: row.try_get("verifier_result_id")?,
                    proof_hash: row.try_get("proof_hash")?,
                    public_summary: row.try_get("public_summary")?,
                    privacy: parse_privacy(row.try_get::<String, _>("privacy")?)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_settlement(&self, settlement: &Settlement) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO settlements
              (id, bounty_id, proof_record_id, rail, payout_intents, platform_fee, currency, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
              rail = EXCLUDED.rail,
              payout_intents = EXCLUDED.payout_intents,
              platform_fee = EXCLUDED.platform_fee,
              currency = EXCLUDED.currency
            "#,
        )
        .bind(settlement.id)
        .bind(settlement.bounty_id)
        .bind(settlement.proof_record_id)
        .bind(format!("{:?}", settlement.rail))
        .bind(serde_json::to_value(&settlement.payout_intents)?)
        .bind(settlement.platform_fee.amount)
        .bind(&settlement.platform_fee.currency)
        .bind(settlement.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_settlements(&self) -> DbResult<Vec<Settlement>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, proof_record_id, rail, payout_intents, platform_fee, currency, created_at
            FROM settlements
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let platform_fee_amount = row.try_get::<i64, _>("platform_fee")?;
                let currency = row.try_get::<String, _>("currency")?;
                let platform_fee = persisted_nonnegative_money(platform_fee_amount, currency)?;
                Ok(Settlement {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    proof_record_id: row.try_get("proof_record_id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    payout_intents: serde_json::from_value(row.try_get("payout_intents")?)?,
                    platform_fee,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_reputation_event(&self, event: &ReputationEvent) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO reputation_events
              (id, agent_id, bounty_id, capability_class, template_slug, delta, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
              capability_class = EXCLUDED.capability_class,
              template_slug = EXCLUDED.template_slug,
              delta = EXCLUDED.delta,
              reason = EXCLUDED.reason
            "#,
        )
        .bind(event.id)
        .bind(event.agent_id)
        .bind(event.bounty_id)
        .bind(format!("{:?}", event.capability_class))
        .bind(&event.template_slug)
        .bind(event.delta)
        .bind(&event.reason)
        .bind(event.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_reputation_events(&self) -> DbResult<Vec<ReputationEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, agent_id, bounty_id, capability_class, template_slug, delta, reason, created_at
            FROM reputation_events
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(ReputationEvent {
                    id: row.try_get("id")?,
                    agent_id: row.try_get("agent_id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    capability_class: parse_capability_class(
                        row.try_get::<String, _>("capability_class")?,
                    )?,
                    template_slug: row.try_get("template_slug")?,
                    delta: row.try_get("delta")?,
                    reason: row.try_get("reason")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_template_signal(&self, signal: &TemplateSignal) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO template_signals
              (id, bounty_id, proof_record_id, template_slug, capability_class, verifier_kind, amount, currency, success, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
              template_slug = EXCLUDED.template_slug,
              capability_class = EXCLUDED.capability_class,
              verifier_kind = EXCLUDED.verifier_kind,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              success = EXCLUDED.success
            "#,
        )
        .bind(signal.id)
        .bind(signal.bounty_id)
        .bind(signal.proof_record_id)
        .bind(&signal.template_slug)
        .bind(format!("{:?}", signal.capability_class))
        .bind(format!("{:?}", signal.verifier_kind))
        .bind(signal.amount.amount)
        .bind(&signal.amount.currency)
        .bind(signal.success)
        .bind(signal.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_template_signals(&self) -> DbResult<Vec<TemplateSignal>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, proof_record_id, template_slug, capability_class, verifier_kind, amount, currency, success, created_at
            FROM template_signals
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(TemplateSignal {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    proof_record_id: row.try_get("proof_record_id")?,
                    template_slug: row.try_get("template_slug")?,
                    capability_class: parse_capability_class(
                        row.try_get::<String, _>("capability_class")?,
                    )?,
                    verifier_kind: parse_verifier_kind(row.try_get::<String, _>("verifier_kind")?)?,
                    amount: Money::new(
                        row.try_get::<i64, _>("amount")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
                    success: row.try_get("success")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_risk_event(&self, event: &RiskEvent) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO risk_events
              (id, subject_id, agent_id, bounty_id, surface, action, score, reasons, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE SET
              action = EXCLUDED.action,
              score = EXCLUDED.score,
              reasons = EXCLUDED.reasons
            "#,
        )
        .bind(event.id)
        .bind(event.subject_id)
        .bind(event.agent_id)
        .bind(event.bounty_id)
        .bind(format!("{:?}", event.surface))
        .bind(format!("{:?}", event.action))
        .bind(i32::from(event.score))
        .bind(serde_json::to_value(&event.reasons)?)
        .bind(event.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_risk_events(&self) -> DbResult<Vec<RiskEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, subject_id, agent_id, bounty_id, surface, action, score, reasons, created_at
            FROM risk_events
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let score: i32 = row.try_get("score")?;
                Ok(RiskEvent {
                    id: row.try_get("id")?,
                    subject_id: row.try_get("subject_id")?,
                    agent_id: row.try_get("agent_id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    surface: parse_risk_surface(row.try_get::<String, _>("surface")?)?,
                    action: parse_risk_action(row.try_get::<String, _>("action")?)?,
                    score: u16::try_from(score)
                        .map_err(|_| DbError::IntegerOverflow("risk_event.score".to_string()))?,
                    reasons: serde_json::from_value(row.try_get("reasons")?)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_risk_review(&self, review: &RiskReviewRecord) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO risk_reviews
              (id, risk_event_id, subject_id, bounty_id, surface, outcome, operator_id, note, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (risk_event_id) DO UPDATE SET
              outcome = EXCLUDED.outcome,
              operator_id = EXCLUDED.operator_id,
              note = EXCLUDED.note
            "#,
        )
        .bind(review.id)
        .bind(review.risk_event_id)
        .bind(review.subject_id)
        .bind(review.bounty_id)
        .bind(format!("{:?}", review.surface))
        .bind(format!("{:?}", review.outcome))
        .bind(&review.operator_id)
        .bind(&review.note)
        .bind(review.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_risk_reviews(&self) -> DbResult<Vec<RiskReviewRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, risk_event_id, subject_id, bounty_id, surface, outcome, operator_id, note, created_at
            FROM risk_reviews
            ORDER BY created_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(RiskReviewRecord {
                    id: row.try_get("id")?,
                    risk_event_id: row.try_get("risk_event_id")?,
                    subject_id: row.try_get("subject_id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    surface: parse_risk_surface(row.try_get::<String, _>("surface")?)?,
                    outcome: parse_risk_review_outcome(row.try_get::<String, _>("outcome")?)?,
                    operator_id: row.try_get("operator_id")?,
                    note: row.try_get("note")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn insert_ledger_entry(&self, entry: &LedgerEntry) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ledger_entries (id, external_event_id, memo, postings, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (external_event_id) DO NOTHING
            "#,
        )
        .bind(entry.id)
        .bind(&entry.external_event_id)
        .bind(&entry.memo)
        .bind(serde_json::to_value(&entry.postings)?)
        .bind(entry.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_ledger_entries(&self) -> DbResult<Vec<LedgerEntry>> {
        let rows = sqlx::query("SELECT id, external_event_id, memo, postings, created_at FROM ledger_entries ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;

        rows.into_iter()
            .map(|row| {
                Ok(LedgerEntry {
                    id: row.try_get("id")?,
                    external_event_id: row.try_get("external_event_id")?,
                    memo: row.try_get("memo")?,
                    postings: serde_json::from_value::<Vec<Posting>>(row.try_get("postings")?)?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_payment_event(&self, event: &PaymentEvent) -> DbResult<()> {
        sqlx::query(UPSERT_PAYMENT_EVENT_SQL)
            .bind(event.id)
            .bind(format!("{:?}", event.rail))
            .bind(&event.external_id)
            .bind(format!("{:?}", event.status))
            .bind(&event.payload_hash)
            .bind(event.received_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_payment_events(&self) -> DbResult<Vec<PaymentEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, rail, external_id, status, payload_hash, received_at
            FROM payment_events
            ORDER BY received_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(PaymentEvent {
                    id: row.try_get("id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    external_id: row.try_get("external_id")?,
                    status: parse_payment_event_status(row.try_get::<String, _>("status")?)?,
                    payload_hash: row.try_get("payload_hash")?,
                    received_at: row.try_get("received_at")?,
                })
            })
            .collect()
    }

    pub async fn upsert_eval_run(&self, run: &EvalRun) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO eval_runs (id, suite, score, passed, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
              suite = EXCLUDED.suite,
              score = EXCLUDED.score,
              passed = EXCLUDED.passed,
              created_at = EXCLUDED.created_at
            "#,
        )
        .bind(run.id)
        .bind(&run.suite)
        .bind(run.score)
        .bind(run.passed)
        .bind(run.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_eval_runs(&self) -> DbResult<Vec<EvalRun>> {
        let rows = sqlx::query(
            r#"
            SELECT id, suite, score, passed, created_at
            FROM eval_runs
            ORDER BY created_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(EvalRun {
                    id: row.try_get("id")?,
                    suite: row.try_get("suite")?,
                    score: row.try_get("score")?,
                    passed: row.try_get("passed")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }
}

fn trial_bounty_from_row(row: PgRow) -> DbResult<TrialBounty> {
    Ok(TrialBounty {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        request_fingerprint: row.try_get("request_fingerprint")?,
        title: row.try_get("title")?,
        goal: row.try_get("goal")?,
        acceptance_criteria: serde_json::from_value(row.try_get("acceptance_criteria")?)?,
        source_url: row.try_get("source_url")?,
        discovery_source: row.try_get("discovery_source")?,
        status: row.try_get("status")?,
        demo_agent_solution: row.try_get("demo_agent_solution")?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
    })
}

fn unfunded_bounty_solution_from_row(row: PgRow) -> DbResult<UnfundedBountySolution> {
    Ok(UnfundedBountySolution {
        id: row.try_get("id")?,
        trial_bounty_id: row.try_get("trial_bounty_id")?,
        agent_id: row.try_get("agent_id")?,
        summary: row.try_get("summary")?,
        deliverable_markdown: row.try_get("deliverable_markdown")?,
        evidence: row.try_get("evidence")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_agent_status(value: String) -> DbResult<AgentStatus> {
    match value.as_str() {
        "Active" => Ok(AgentStatus::Active),
        "Suspended" => Ok(AgentStatus::Suspended),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_x402_relay_status(value: String) -> DbResult<X402RelayStatus> {
    match value.as_str() {
        "prepared" => Ok(X402RelayStatus::Prepared),
        "relaying" => Ok(X402RelayStatus::Relaying),
        "broadcast" => Ok(X402RelayStatus::Broadcast),
        "confirmed" => Ok(X402RelayStatus::Confirmed),
        "failed" => Ok(X402RelayStatus::Failed),
        other => Err(DbError::InvalidEnum(format!("x402 relay status {other}"))),
    }
}

fn x402_relay_attempt_from_row(row: PgRow) -> DbResult<X402RelayAttempt> {
    Ok(X402RelayAttempt {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        contributor: row.try_get("contributor")?,
        amount: u64_from_i64(row.try_get("amount")?)?,
        authorization_nonce: row.try_get("authorization_nonce")?,
        authorization_valid_before: u64_from_i64(row.try_get("authorization_valid_before")?)?,
        request_fingerprint: row.try_get("request_fingerprint")?,
        relayer_address: row.try_get("relayer_address")?,
        status: parse_x402_relay_status(row.try_get("status")?)?,
        retryable: row.try_get("retryable")?,
        attempt_count: u32::try_from(row.try_get::<i32, _>("attempt_count")?)
            .map_err(|_| DbError::IntegerOverflow("x402 relay attempt count".to_string()))?,
        tx_hash: row.try_get("tx_hash")?,
        estimated_gas: row
            .try_get::<Option<i64>, _>("estimated_gas")?
            .map(u64_from_i64)
            .transpose()?,
        gas_limit: row
            .try_get::<Option<i64>, _>("gas_limit")?
            .map(u64_from_i64)
            .transpose()?,
        error_code: row.try_get("error_code")?,
        error_message: row.try_get("error_message")?,
        canonical_event_id: row.try_get("canonical_event_id")?,
        confirmed_block: row
            .try_get::<Option<i64>, _>("confirmed_block")?
            .map(u64_from_i64)
            .transpose()?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_x402_relay_replay(
    persisted: &X402RelayAttempt,
    requested: &NewX402RelayAttempt,
) -> DbResult<()> {
    if persisted.idempotency_key != requested.idempotency_key
        || !persisted
            .bounty_contract
            .eq_ignore_ascii_case(&requested.bounty_contract)
        || !persisted
            .contributor
            .eq_ignore_ascii_case(&requested.contributor)
        || persisted.amount != requested.amount
        || persisted.authorization_valid_before != requested.authorization_valid_before
        || persisted.request_fingerprint != requested.request_fingerprint
        || !persisted
            .relayer_address
            .eq_ignore_ascii_case(&requested.relayer_address)
    {
        return Err(DbError::X402RelayConflict(
            "authorization nonce replay does not match the original request".to_string(),
        ));
    }
    Ok(())
}

async fn waitlist_position(
    transaction: &mut Transaction<'_, Postgres>,
    candidate: &ClaimCandidate,
) -> DbResult<Option<u32>> {
    if candidate.status != ClaimCandidateStatus::Waitlisted {
        return Ok(None);
    }
    let position: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM claim_candidates
        WHERE network = $1 AND bounty_contract = $2 AND status = 'waitlisted'
          AND (created_at, id) <= ($3, $4)
        "#,
    )
    .bind(&candidate.network)
    .bind(&candidate.bounty_contract)
    .bind(candidate.created_at)
    .bind(candidate.id)
    .fetch_one(&mut **transaction)
    .await?;
    Ok(Some(u32::try_from(position).map_err(|_| {
        DbError::IntegerOverflow("claim waitlist position".to_string())
    })?))
}

fn parse_claim_candidate_status(value: String) -> DbResult<ClaimCandidateStatus> {
    match value.as_str() {
        "waitlisted" => Ok(ClaimCandidateStatus::Waitlisted),
        "exclusive" => Ok(ClaimCandidateStatus::Exclusive),
        "sponsoring" => Ok(ClaimCandidateStatus::Sponsoring),
        "authorization_ready" => Ok(ClaimCandidateStatus::AuthorizationReady),
        "relaying" => Ok(ClaimCandidateStatus::Relaying),
        "claimed" => Ok(ClaimCandidateStatus::Claimed),
        "superseded" => Ok(ClaimCandidateStatus::Superseded),
        "withdrawn" => Ok(ClaimCandidateStatus::Withdrawn),
        "failed" => Ok(ClaimCandidateStatus::Failed),
        other => Err(DbError::InvalidEnum(format!(
            "claim candidate status {other}"
        ))),
    }
}

fn claim_candidate_from_row(row: PgRow) -> DbResult<ClaimCandidate> {
    Ok(ClaimCandidate {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        solver_wallet: row.try_get("solver_wallet")?,
        agent_id: row.try_get("agent_id")?,
        eligibility_evidence: serde_json::from_value(row.try_get("eligibility_evidence")?)?,
        eligibility_decision: serde_json::from_value(row.try_get("eligibility_decision")?)?,
        status: parse_claim_candidate_status(row.try_get("status")?)?,
        exclusive_until: row.try_get("exclusive_until")?,
        authorization_nonce: row.try_get("authorization_nonce")?,
        authorization_valid_before: row
            .try_get::<Option<i64>, _>("authorization_valid_before")?
            .map(u64_from_i64)
            .transpose()?,
        claim_transaction_hash: row.try_get("claim_transaction_hash")?,
        canonical_event_id: row.try_get("canonical_event_id")?,
        failure_code: row.try_get("failure_code")?,
        failure_message: row.try_get("failure_message")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn update_claim_candidate_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    tx_hash: Option<&str>,
    canonical_event_id: Option<Uuid>,
    failure: Option<(&str, &str)>,
) -> DbResult<ClaimCandidate> {
    let row = sqlx::query(
        r#"
        UPDATE claim_candidates
        SET status = $2,
            claim_transaction_hash = COALESCE($3, claim_transaction_hash),
            canonical_event_id = COALESCE($4, canonical_event_id),
            failure_code = $5,
            failure_message = $6,
            updated_at = now()
        WHERE id = $1 AND (
          ($2 = 'relaying' AND status IN ('exclusive', 'sponsoring', 'authorization_ready'))
          OR ($2 = 'claimed' AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying', 'claimed'))
          OR ($2 = 'failed' AND status IN ('exclusive', 'sponsoring', 'authorization_ready', 'relaying'))
        )
        RETURNING id, idempotency_key, network, bounty_contract, solver_wallet,
                  agent_id, eligibility_evidence, eligibility_decision, status,
                  exclusive_until, authorization_nonce, authorization_valid_before,
                  claim_transaction_hash, canonical_event_id, failure_code,
                  failure_message, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(tx_hash.map(str::to_ascii_lowercase))
    .bind(canonical_event_id)
    .bind(failure.map(|(code, _)| code))
    .bind(failure.map(|(_, message)| message.chars().take(500).collect::<String>()))
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DbError::ClaimCandidateConflict(format!(
            "candidate cannot transition to {status} from its current state"
        ))
    })?;
    claim_candidate_from_row(row)
}

fn parse_bond_sponsorship_status(value: String) -> DbResult<BondSponsorshipStatus> {
    match value.as_str() {
        "reserved" => Ok(BondSponsorshipStatus::Reserved),
        "broadcast" => Ok(BondSponsorshipStatus::Broadcast),
        "confirmed" => Ok(BondSponsorshipStatus::Confirmed),
        "failed" => Ok(BondSponsorshipStatus::Failed),
        other => Err(DbError::InvalidEnum(format!(
            "bond sponsorship status {other}"
        ))),
    }
}

fn bond_sponsorship_from_row(row: PgRow) -> DbResult<BondSponsorship> {
    Ok(BondSponsorship {
        id: row.try_get("id")?,
        claim_candidate_id: row.try_get("claim_candidate_id")?,
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        solver_wallet: row.try_get("solver_wallet")?,
        sponsor_wallet: row.try_get("sponsor_wallet")?,
        amount: u64_from_i64(row.try_get("amount")?)?,
        status: parse_bond_sponsorship_status(row.try_get("status")?)?,
        transaction_hash: row.try_get("transaction_hash")?,
        confirmed_block: row
            .try_get::<Option<i64>, _>("confirmed_block")?
            .map(u64_from_i64)
            .transpose()?,
        failure_code: row.try_get("failure_code")?,
        failure_message: row.try_get("failure_message")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn update_bond_sponsorship(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    tx_hash: Option<&str>,
    confirmed_block: Option<u64>,
    failure: Option<(&str, &str)>,
) -> DbResult<BondSponsorship> {
    let row = sqlx::query(
        r#"
        UPDATE bond_sponsorships
        SET status = $2, transaction_hash = COALESCE($3, transaction_hash),
            confirmed_block = COALESCE($4, confirmed_block),
            failure_code = $5, failure_message = $6, updated_at = now()
        WHERE id = $1 AND (
          ($2 = 'broadcast' AND status = 'reserved')
          OR ($2 = 'confirmed' AND status IN ('broadcast', 'confirmed'))
          OR ($2 = 'failed' AND status IN ('reserved', 'broadcast'))
        )
        RETURNING id, claim_candidate_id, network, bounty_contract, solver_wallet,
                  sponsor_wallet, amount, status, transaction_hash, confirmed_block,
                  failure_code, failure_message, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(status)
    .bind(tx_hash.map(str::to_ascii_lowercase))
    .bind(confirmed_block.map(i64_from_u64).transpose()?)
    .bind(failure.map(|(code, _)| code))
    .bind(failure.map(|(_, message)| message.chars().take(500).collect::<String>()))
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        DbError::ClaimCandidateConflict(format!("bond sponsorship cannot transition to {status}"))
    })?;
    bond_sponsorship_from_row(row)
}

fn parse_audience_provider(value: String) -> DbResult<AudienceProvider> {
    match value.as_str() {
        "Github" => Ok(AudienceProvider::Github),
        "HostedApi" => Ok(AudienceProvider::HostedApi),
        "Mcp" => Ok(AudienceProvider::Mcp),
        "BaseWallet" => Ok(AudienceProvider::BaseWallet),
        "Stripe" => Ok(AudienceProvider::Stripe),
        "Other" => Ok(AudienceProvider::Other),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_audience_lifecycle_stage(value: String) -> DbResult<AudienceLifecycleStage> {
    match value.as_str() {
        "Observed" => Ok(AudienceLifecycleStage::Observed),
        "Engaged" => Ok(AudienceLifecycleStage::Engaged),
        "Converted" => Ok(AudienceLifecycleStage::Converted),
        "Retained" => Ok(AudienceLifecycleStage::Retained),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_audience_interaction_kind(value: String) -> DbResult<AudienceInteractionKind> {
    match value.as_str() {
        "IssueOpened" => Ok(AudienceInteractionKind::IssueOpened),
        "PullRequestOpened" => Ok(AudienceInteractionKind::PullRequestOpened),
        "IssueCommented" => Ok(AudienceInteractionKind::IssueCommented),
        "PullRequestReviewed" => Ok(AudienceInteractionKind::PullRequestReviewed),
        "BountyPosted" => Ok(AudienceInteractionKind::BountyPosted),
        "FundingSignaled" => Ok(AudienceInteractionKind::FundingSignaled),
        "BountyFunded" => Ok(AudienceInteractionKind::BountyFunded),
        "ClaimSignaled" => Ok(AudienceInteractionKind::ClaimSignaled),
        "BountyClaimed" => Ok(AudienceInteractionKind::BountyClaimed),
        "SubmissionMade" => Ok(AudienceInteractionKind::SubmissionMade),
        "SubmissionAccepted" => Ok(AudienceInteractionKind::SubmissionAccepted),
        "VerificationSubmitted" => Ok(AudienceInteractionKind::VerificationSubmitted),
        "PayoutReceived" => Ok(AudienceInteractionKind::PayoutReceived),
        "RepoStarred" => Ok(AudienceInteractionKind::RepoStarred),
        "BountyUpvoted" => Ok(AudienceInteractionKind::BountyUpvoted),
        "ProofShared" => Ok(AudienceInteractionKind::ProofShared),
        "ReferralCreated" => Ok(AudienceInteractionKind::ReferralCreated),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_outreach_channel(value: String) -> DbResult<OutreachChannel> {
    match value.as_str() {
        "GithubPublic" => Ok(OutreachChannel::GithubPublic),
        "OtherPublic" => Ok(OutreachChannel::OtherPublic),
        "EmailPrivate" => Ok(OutreachChannel::EmailPrivate),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_outreach_status(value: String) -> DbResult<OutreachStatus> {
    match value.as_str() {
        "Pending" => Ok(OutreachStatus::Pending),
        "Responded" => Ok(OutreachStatus::Responded),
        "Declined" => Ok(OutreachStatus::Declined),
        "Unreachable" => Ok(OutreachStatus::Unreachable),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn persisted_nonnegative_money(amount: i64, currency: String) -> DbResult<Money> {
    if amount == 0 {
        Ok(Money::zero(currency))
    } else {
        Ok(Money::new(amount, currency)?)
    }
}

fn objective_status_value(status: ObjectiveStatus) -> DbResult<String> {
    serde_json::to_value(status)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| DbError::InvalidEnum("objective status".to_string()))
}

fn parse_capability_class(value: String) -> DbResult<CapabilityClass> {
    match value.as_str() {
        "Coding" => Ok(CapabilityClass::Coding),
        "Research" => Ok(CapabilityClass::Research),
        "Extraction" => Ok(CapabilityClass::Extraction),
        "Verification" => Ok(CapabilityClass::Verification),
        "Documentation" => Ok(CapabilityClass::Documentation),
        "Ci" => Ok(CapabilityClass::Ci),
        "BrowserWorkflow" => Ok(CapabilityClass::BrowserWorkflow),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_privacy(value: String) -> DbResult<PrivacyLevel> {
    match value.as_str() {
        "Public" => Ok(PrivacyLevel::Public),
        "RedactedPublicProof" => Ok(PrivacyLevel::RedactedPublicProof),
        "Private" => Ok(PrivacyLevel::Private),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_verifier_kind(value: String) -> DbResult<VerifierKind> {
    match value.as_str() {
        "Manual" => Ok(VerifierKind::Manual),
        "JsonSchema" => Ok(VerifierKind::JsonSchema),
        "DockerCommand" => Ok(VerifierKind::DockerCommand),
        "GitHubCi" => Ok(VerifierKind::GitHubCi),
        "HttpCallback" => Ok(VerifierKind::HttpCallback),
        "AiJudgeFilter" => Ok(VerifierKind::AiJudgeFilter),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_funding_mode(value: String) -> DbResult<FundingMode> {
    match value.as_str() {
        "Simulated" => Ok(FundingMode::Simulated),
        "BaseUsdcEscrow" => Ok(FundingMode::BaseUsdcEscrow),
        "StripeFiatLedger" => Ok(FundingMode::StripeFiatLedger),
        "MixedRails" => Ok(FundingMode::MixedRails),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_payment_rail(value: String) -> DbResult<PaymentRail> {
    match value.as_str() {
        "Simulated" => Ok(PaymentRail::Simulated),
        "BaseUsdc" => Ok(PaymentRail::BaseUsdc),
        "StripeFiat" => Ok(PaymentRail::StripeFiat),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_escrow_status(value: String) -> DbResult<EscrowStatus> {
    match value.as_str() {
        "Created" => Ok(EscrowStatus::Created),
        "Funded" => Ok(EscrowStatus::Funded),
        "Disputed" => Ok(EscrowStatus::Disputed),
        "Released" => Ok(EscrowStatus::Released),
        "Refunded" => Ok(EscrowStatus::Refunded),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_payment_event_status(value: String) -> DbResult<PaymentEventStatus> {
    match value.as_str() {
        "Received" => Ok(PaymentEventStatus::Received),
        "Applied" => Ok(PaymentEventStatus::Applied),
        "IgnoredDuplicate" => Ok(PaymentEventStatus::IgnoredDuplicate),
        "Failed" => Ok(PaymentEventStatus::Failed),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_funding_contribution_status(value: String) -> DbResult<FundingContributionStatus> {
    match value.as_str() {
        "Applied" => Ok(FundingContributionStatus::Applied),
        "Refunded" => Ok(FundingContributionStatus::Refunded),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_funding_intent_status(value: String) -> DbResult<FundingIntentStatus> {
    match value.as_str() {
        "AwaitingEvidence" => Ok(FundingIntentStatus::AwaitingEvidence),
        "Applied" => Ok(FundingIntentStatus::Applied),
        "Rejected" => Ok(FundingIntentStatus::Rejected),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn webhook_subscription_from_row(row: PgRow) -> DbResult<WebhookSubscription> {
    Ok(WebhookSubscription {
        id: row.try_get("id")?,
        owner_wallet: row.try_get("owner_wallet")?,
        endpoint_url: row.try_get("endpoint_url")?,
        event_types: serde_json::from_value(row.try_get("event_types")?)?,
        subscription_kind: row.try_get("subscription_kind")?,
        filters: serde_json::from_value(row.try_get("filters")?)?,
        management_token_hash: row.try_get("management_token_hash")?,
        secret_version: u32::try_from(row.try_get::<i32, _>("secret_version")?)
            .map_err(|_| DbError::IntegerOverflow("secret_version".to_string()))?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn webhook_delivery_from_row(row: PgRow) -> DbResult<WebhookDelivery> {
    Ok(WebhookDelivery {
        id: row.try_get("id")?,
        subscription_id: row.try_get("subscription_id")?,
        event_id: row.try_get("event_id")?,
        event_type: serde_json::from_value(serde_json::Value::String(row.try_get("event_type")?))?,
        payload: row.try_get("payload")?,
        status: row.try_get("status")?,
        attempt_count: u32::try_from(row.try_get::<i32, _>("attempt_count")?)
            .map_err(|_| DbError::IntegerOverflow("attempt_count".to_string()))?,
        next_attempt_at: row.try_get("next_attempt_at")?,
        lease_token: row.try_get("lease_token")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        response_status: row
            .try_get::<Option<i32>, _>("response_status")?
            .map(u16::try_from)
            .transpose()
            .map_err(|_| DbError::IntegerOverflow("response_status".to_string()))?,
        last_error: row.try_get("last_error")?,
        delivered_at: row.try_get("delivered_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn social_mention_ingestion_from_row(row: PgRow) -> DbResult<SocialMentionIngestion> {
    Ok(SocialMentionIngestion {
        id: row.try_get("id")?,
        provider: row.try_get("provider")?,
        provider_event_id: row.try_get("provider_event_id")?,
        source_network: row.try_get("source_network")?,
        mention_id: row.try_get("mention_id")?,
        mention_url: row.try_get("mention_url")?,
        author_fid: row.try_get("author_fid")?,
        author_handle: row.try_get("author_handle")?,
        mention_text: row.try_get("mention_text")?,
        status: row.try_get("status")?,
        draft: row.try_get("draft")?,
        idempotency_key: row.try_get("idempotency_key")?,
        reply_cast_hash: row.try_get("reply_cast_hash")?,
        last_error: row.try_get("last_error")?,
        reply_attempt_count: u32::try_from(row.try_get::<i32, _>("reply_attempt_count")?)
            .map_err(|_| DbError::IntegerOverflow("reply_attempt_count".to_string()))?,
        reply_lease_token: row.try_get("reply_lease_token")?,
        reply_lease_expires_at: row.try_get("reply_lease_expires_at")?,
        received_at: row.try_get("received_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn autonomous_event_from_row(row: PgRow) -> DbResult<AutonomousBountyEvent> {
    let kind_value = serde_json::Value::String(row.try_get::<String, _>("kind")?);
    let kind: AutonomousBountyEventKind = serde_json::from_value(kind_value)?;
    Ok(AutonomousBountyEvent {
        id: row.try_get("id")?,
        log_key: row.try_get("log_key")?,
        tx_hash: row.try_get("tx_hash")?,
        block_number: u64_from_i64(row.try_get("block_number")?)?,
        log_index: u64_from_i64(row.try_get("log_index")?)?,
        contract_address: row.try_get("contract_address")?,
        bounty_id: row.try_get("bounty_id")?,
        kind,
        data: row.try_get("data")?,
        occurred_at: row.try_get("occurred_at")?,
    })
}

fn autonomous_terms_from_row(row: PgRow) -> DbResult<AutonomousBountyTermsRecord> {
    let document: serde_json::Value = row.try_get("document")?;
    Ok(AutonomousBountyTermsRecord {
        terms_hash: row.try_get("terms_hash")?,
        policy_hash: row.try_get("policy_hash")?,
        acceptance_criteria_hash: row.try_get("acceptance_criteria_hash")?,
        benchmark_hash: row.try_get("benchmark_hash")?,
        evidence_schema_hash: row.try_get("evidence_schema_hash")?,
        creator_wallet: row.try_get("creator_wallet")?,
        document: serde_json::from_value::<AutonomousBountyTermsDocument>(document)?,
        created_at: row.try_get("created_at")?,
    })
}

fn autonomous_submission_evidence_from_row(
    row: PgRow,
) -> DbResult<AutonomousSubmissionEvidenceRecord> {
    Ok(AutonomousSubmissionEvidenceRecord {
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        bounty_id: row.try_get("bounty_id")?,
        round: u64_from_i64(row.try_get("round")?)?,
        solver_wallet: row.try_get("solver_wallet")?,
        artifact_reference: row.try_get("artifact_reference")?,
        artifact_hash: row.try_get("artifact_hash")?,
        evidence: row.try_get("evidence")?,
        evidence_hash: row.try_get("evidence_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

fn parse_risk_surface(value: String) -> DbResult<RiskSurface> {
    match value.as_str() {
        "HelpRequest" => Ok(RiskSurface::HelpRequest),
        "Bounty" => Ok(RiskSurface::Bounty),
        "Submission" => Ok(RiskSurface::Submission),
        "Verification" => Ok(RiskSurface::Verification),
        "Payout" => Ok(RiskSurface::Payout),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn i64_from_u64(value: u64) -> DbResult<i64> {
    i64::try_from(value).map_err(|_| DbError::IntegerOverflow(value.to_string()))
}

fn u64_from_i64(value: i64) -> DbResult<u64> {
    u64::try_from(value).map_err(|_| DbError::IntegerOverflow(value.to_string()))
}

fn optional_i64_from_u64(value: Option<u64>) -> DbResult<Option<i64>> {
    value.map(i64_from_u64).transpose()
}

fn optional_u64_from_i64(value: Option<i64>) -> DbResult<Option<u64>> {
    value.map(u64_from_i64).transpose()
}

fn normalize_key_address(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn parse_risk_action(value: String) -> DbResult<RiskAction> {
    match value.as_str() {
        "Allow" => Ok(RiskAction::Allow),
        "NeedsReview" => Ok(RiskAction::NeedsReview),
        "Block" => Ok(RiskAction::Block),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_risk_review_outcome(value: String) -> DbResult<RiskReviewOutcome> {
    match value.as_str() {
        "Approved" => Ok(RiskReviewOutcome::Approved),
        "Rejected" => Ok(RiskReviewOutcome::Rejected),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_verification_decision(value: String) -> DbResult<VerificationDecision> {
    match value.as_str() {
        "Accepted" => Ok(VerificationDecision::Accepted),
        "Rejected" => Ok(VerificationDecision::Rejected),
        "NeedsReview" => Ok(VerificationDecision::NeedsReview),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn parse_bounty_status(value: String) -> DbResult<BountyStatus> {
    match value.as_str() {
        "Unfunded" => Ok(BountyStatus::Unfunded),
        "Funded" => Ok(BountyStatus::Funded),
        "Claimable" => Ok(BountyStatus::Claimable),
        "Claimed" => Ok(BountyStatus::Claimed),
        "Submitted" => Ok(BountyStatus::Submitted),
        "Verifying" => Ok(BountyStatus::Verifying),
        "Accepted" => Ok(BountyStatus::Accepted),
        "Payable" => Ok(BountyStatus::Payable),
        "Paid" => Ok(BountyStatus::Paid),
        "Refunding" => Ok(BountyStatus::Refunding),
        "Refunded" => Ok(BountyStatus::Refunded),
        "Disputed" => Ok(BountyStatus::Disputed),
        "Expired" => Ok(BountyStatus::Expired),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn bounty_from_row(row: &PgRow) -> DbResult<Bounty> {
    Ok(Bounty {
        id: row.try_get("id")?,
        help_request_id: row.try_get("help_request_id")?,
        title: row.try_get("title")?,
        template_slug: row.try_get("template_slug")?,
        amount: Money::new(
            row.try_get::<i64, _>("amount")?,
            row.try_get::<String, _>("currency")?,
        )?,
        funding_targets: serde_json::from_value(row.try_get("funding_targets")?)?,
        funding_mode: parse_funding_mode(row.try_get::<String, _>("funding_mode")?)?,
        privacy: parse_privacy(row.try_get::<String, _>("privacy")?)?,
        status: parse_bounty_status(row.try_get::<String, _>("status")?)?,
        terms_hash: row.try_get("terms_hash")?,
        created_at: row.try_get("created_at")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::B256,
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use domain::{
        DeliverableAccessPolicy, FundingMode, IdentityDisclosure, Money, Objective,
        ObjectiveAuthority, ObjectiveAuthorityKind, ObjectiveCreationDraft, ObjectiveParticipant,
        ObjectivePrivacyDeclaration, ObjectiveVerificationMechanism, ObjectiveVerificationPolicy,
        ParticipantKind, PublicEvidencePolicy, RightsPolicy, SignedObjectiveCreation,
        WalletApproval,
    };

    #[test]
    fn store_tracks_agents_and_bounties() {
        let mut store = InMemoryStore::default();
        let agent = Agent::new("solver");
        let bounty = Bounty::new(
            "Fix test",
            "fix-ci-failure",
            Money::new(1000, "usdc").unwrap(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );

        store.insert_agent(agent.clone());
        store.insert_bounty(bounty.clone());

        assert!(store.agents.contains_key(&agent.id));
        assert!(store.bounties.contains_key(&bounty.id));
    }

    #[test]
    fn migration_contains_durable_market_tables() {
        for table in [
            "agents",
            "contributor_contacts",
            "audience_members",
            "audience_interactions",
            "discovery_responses",
            "outreach_attempts",
            "capabilities",
            "help_requests",
            "quotes",
            "bounties",
            "funding_intents",
            "funding_contributions",
            "escrows",
            "base_escrow_events",
            "claims",
            "submissions",
            "verifier_results",
            "proof_records",
            "settlements",
            "reputation_events",
            "template_signals",
            "risk_events",
            "risk_reviews",
            "ledger_entries",
            "payment_events",
            "eval_runs",
            "base_indexer_heartbeats",
        ] {
            assert!(CORE_MIGRATION.contains(table), "missing {table}");
        }
        assert!(CORE_MIGRATION.contains("idx_funding_contributions_external_reference"));
        assert!(CORE_MIGRATION.contains("source_organization_id UUID"));
        assert!(CORE_MIGRATION.contains("funding_targets JSONB"));
        assert!(CORE_MIGRATION.contains("funding_ledger_entry_id UUID"));
        assert!(CORE_MIGRATION.contains("refund_ledger_entry_id UUID"));
        assert!(CORE_MIGRATION.contains("settlement_id UUID"));
        assert!(CORE_MIGRATION.contains("stripe_success_url TEXT"));
        assert!(CORE_MIGRATION.contains("stripe_cancel_url TEXT"));
        assert!(CORE_MIGRATION.contains("github_login_normalized TEXT"));
        assert!(CORE_MIGRATION.contains("outreach_allowed BOOLEAN"));
        assert!(CORE_MIGRATION.contains("private_storage_consent BOOLEAN"));
        assert!(CORE_MIGRATION.contains("consent_contact_id UUID"));
        assert!(CORE_MIGRATION.contains("REFERENCES audience_members(id) ON DELETE CASCADE"));
        assert!(CORE_MIGRATION.contains("idx_audience_interactions_kind_occurred"));
        assert!(CORE_MIGRATION.contains("fund-contribution:"));
        assert!(CORE_MIGRATION.contains("CHECK (platform_fee >= 0)"));
        assert!(CORE_MIGRATION.contains("DROP CONSTRAINT IF EXISTS settlements_platform_fee_check"));
    }

    #[test]
    fn autonomous_migration_contains_protocol_tables_and_indexes() {
        for table in [
            "autonomous_bounty_events",
            "autonomous_bounty_terms",
            "autonomous_submission_evidence",
        ] {
            assert!(
                AUTONOMOUS_PROTOCOL_MIGRATION.contains(table),
                "missing {table}"
            );
        }
        for index in [
            "idx_autonomous_bounty_events_bounty",
            "idx_autonomous_bounty_events_contract",
            "idx_autonomous_bounty_terms_creator",
            "idx_autonomous_submission_evidence_bounty",
        ] {
            assert!(
                AUTONOMOUS_PROTOCOL_MIGRATION.contains(index),
                "missing {index}"
            );
        }
    }

    #[test]
    fn x402_migration_contains_idempotency_and_relayer_leases() {
        for table in ["x402_relay_attempts", "x402_relayer_leases"] {
            assert!(X402_RELAYER_MIGRATION.contains(table), "missing {table}");
        }
        for invariant in [
            "idempotency_key TEXT NOT NULL UNIQUE",
            "UNIQUE (network, bounty_contract, authorization_nonce)",
            "request_fingerprint TEXT NOT NULL",
            "lease_expires_at TIMESTAMPTZ",
            "canonical_event_id UUID",
        ] {
            assert!(
                X402_RELAYER_MIGRATION.contains(invariant),
                "missing x402 invariant {invariant}"
            );
        }
    }

    #[test]
    fn coordination_migration_bounds_claims_sponsorship_and_delivery() {
        for table in [
            "recovery_obligations",
            "claim_candidates",
            "bond_sponsorships",
            "webhook_subscriptions",
            "webhook_deliveries",
            "regression_verification_runs",
        ] {
            assert!(
                AGENT_COORDINATION_MIGRATION.contains(table),
                "missing {table}"
            );
        }
        for invariant in [
            "idempotency_key TEXT NOT NULL UNIQUE",
            "idx_claim_candidates_one_exclusive",
            "idx_claim_candidates_one_active_per_solver",
            "claim_candidate_id UUID NOT NULL UNIQUE",
            "idx_bond_sponsorships_rolling_caps",
            "UNIQUE (subscription_id, event_id)",
        ] {
            assert!(
                AGENT_COORDINATION_MIGRATION.contains(invariant),
                "missing coordination invariant {invariant}"
            );
        }
    }

    #[test]
    fn unfunded_bounty_migration_keeps_public_work_open_and_attribution_bounded() {
        for table in ["trial_bounties", "unfunded_bounty_solutions"] {
            assert!(TRIAL_BOUNTIES_MIGRATION.contains(table), "missing {table}");
        }
        for invariant in [
            "idempotency_key TEXT NOT NULL UNIQUE",
            "status IN ('open', 'closed')",
            "UNIQUE (trial_bounty_id, agent_id)",
            "expires_at > created_at",
        ] {
            assert!(
                TRIAL_BOUNTIES_MIGRATION.contains(invariant),
                "missing unfunded bounty invariant {invariant}"
            );
        }
    }

    #[test]
    fn leaderboard_migration_requires_verified_block_time_indexes() {
        for invariant in [
            "block_time_verified BOOLEAN NOT NULL DEFAULT FALSE",
            "idx_autonomous_bounty_events_unverified_blocks",
            "idx_autonomous_bounty_events_solver_leaderboard",
            "block_time_verified = TRUE AND kind = 'bounty_settled'",
        ] {
            assert!(
                SOLVER_LEADERBOARD_MIGRATION.contains(invariant),
                "missing leaderboard invariant {invariant}"
            );
        }
    }

    #[test]
    fn discovery_subscription_migration_extends_existing_delivery_tables() {
        for invariant in [
            "subscription_kind TEXT NOT NULL DEFAULT 'agent_wallet'",
            "filters JSONB NOT NULL DEFAULT '{}'::jsonb",
            "management_token_hash TEXT",
            "idx_webhook_subscriptions_public_discovery",
            "idx_webhook_deliveries_subscription_created",
        ] {
            assert!(
                DISCOVERY_SUBSCRIPTIONS_MIGRATION.contains(invariant),
                "missing discovery subscription invariant {invariant}"
            );
        }
    }

    #[test]
    fn opportunity_conversion_migration_records_only_missing_observable_stages() {
        for invariant in [
            "opportunity_creation_progress",
            "terms_hash TEXT PRIMARY KEY",
            "unfunded_bounty_id UUID REFERENCES trial_bounties(id) ON DELETE SET NULL",
            "funding_prepared_at TIMESTAMPTZ",
            "wallet_signed_at TIMESTAMPTZ",
            "idx_opportunity_creation_progress_unfunded",
        ] {
            assert!(
                OPPORTUNITY_CONVERSION_MIGRATION.contains(invariant),
                "missing opportunity conversion invariant {invariant}"
            );
        }
    }

    #[test]
    fn legal_acceptance_migration_preserves_versioned_action_evidence() {
        for invariant in [
            "legal_acceptances",
            "terms_version TEXT NOT NULL",
            "privacy_version TEXT NOT NULL",
            "wallet_address TEXT NOT NULL",
            "statement_hash TEXT NOT NULL",
            "accepted_at TIMESTAMPTZ NOT NULL",
            "acceptance_method IN ('web_clickwrap', 'api_explicit')",
            "legal_acceptances_wallet_recorded_idx",
            "legal_acceptances_action_recorded_idx",
        ] {
            assert!(
                LEGAL_ACCEPTANCES_MIGRATION.contains(invariant),
                "missing legal acceptance invariant {invariant}"
            );
        }
    }

    #[test]
    fn site_analytics_migration_is_privacy_minimized_and_idempotent() {
        for invariant in [
            "site_analytics_events",
            "event_id UUID PRIMARY KEY",
            "visitor_id UUID NOT NULL",
            "session_id UUID NOT NULL",
            "event_name TEXT NOT NULL",
            "site_analytics_event_name_check",
            "site_analytics_page_path_check",
            "site_analytics_event_time_check",
            "site_analytics_events_visitor_idx",
            "site_analytics_events_source_idx",
        ] {
            assert!(
                SITE_ANALYTICS_MIGRATION.contains(invariant),
                "missing site analytics invariant {invariant}"
            );
        }
        for forbidden in ["ip_address", "user_agent", "referrer_url", "wallet_address"] {
            assert!(
                !SITE_ANALYTICS_MIGRATION.contains(forbidden),
                "site analytics must not persist {forbidden}"
            );
        }
    }

    #[test]
    fn social_mention_migration_is_durable_idempotent_and_lease_bounded() {
        for invariant in [
            "social_mention_ingestions",
            "UNIQUE (provider, provider_event_id)",
            "UNIQUE (source_network, mention_id)",
            "reply_lease_token UUID",
            "reply_lease_expires_at TIMESTAMPTZ",
            "reply_attempt_count INTEGER NOT NULL DEFAULT 0",
            "'reply_failed'",
            "'replied'",
        ] {
            assert!(
                SOCIAL_MENTION_INGESTION_MIGRATION.contains(invariant),
                "missing social mention invariant {invariant}"
            );
        }
    }

    #[test]
    fn competitor_intelligence_migration_is_evidence_bound_and_additive() {
        for invariant in [
            "competitors",
            "competitor_links",
            "competitor_capabilities",
            "competitor_intelligence_runs",
            "competitor_source_observations",
            "competitor_metric_observations",
            "competitor_intelligence_changes",
            "content_sha256",
            "source_changed",
            "source_failed",
            "CREATE TABLE IF NOT EXISTS",
        ] {
            assert!(
                COMPETITOR_INTELLIGENCE_MIGRATION.contains(invariant),
                "missing competitor intelligence invariant {invariant}"
            );
        }
        for forbidden in [
            "cookie TEXT",
            "authorization TEXT",
            "password TEXT",
            "private_key TEXT",
        ] {
            assert!(
                !COMPETITOR_INTELLIGENCE_MIGRATION
                    .to_ascii_lowercase()
                    .contains(forbidden),
                "competitor intelligence must not persist {forbidden}"
            );
        }
    }

    #[test]
    fn objective_migration_keeps_one_versioned_aggregate_with_cas_fields() {
        for invariant in [
            "objective_aggregates",
            "schema_version TEXT NOT NULL",
            "revision BIGINT NOT NULL CHECK (revision > 0)",
            "requesting_party_id UUID NOT NULL",
            "record JSONB NOT NULL",
            "agent-bounties/objective-v1",
            "idx_objective_aggregates_status_updated",
        ] {
            assert!(
                OBJECTIVE_COORDINATION_MIGRATION.contains(invariant),
                "missing objective persistence invariant {invariant}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn objective_aggregate_compare_and_swap_is_durable() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let signer: PrivateKeySigner =
            "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"
                .parse()
                .unwrap();
        let requester_id = Uuid::new_v4();
        let draft = ObjectiveCreationDraft {
            id: Uuid::new_v4(),
            title: "Durable objective".to_string(),
            desired_outcome: "A revisioned aggregate survives process boundaries.".to_string(),
            human_purpose: "Prevent lost signed actions.".to_string(),
            participants: vec![ObjectiveParticipant {
                id: requester_id,
                kind: ParticipantKind::Organization,
                display_name: "Persistence test requester".to_string(),
                wallet: format!("{:#x}", signer.address()),
                identity_disclosure: IdentityDisclosure::Pseudonymous,
                public_identity_reference: None,
            }],
            requesting_party_id: requester_id,
            beneficiary_ids: vec![requester_id],
            affected_parties: Vec::new(),
            authority: ObjectiveAuthority {
                kind: ObjectiveAuthorityKind::SingleWallet,
                member_ids: vec![requester_id],
                threshold: 1,
                public_statement: "One declared organization wallet controls this test objective."
                    .to_string(),
            },
            available_resources: Vec::new(),
            expected_final_deliverable: "Durable revision evidence".to_string(),
            requested_access_policy: DeliverableAccessPolicy::Public,
            requested_rights_policy: RightsPolicy {
                owner_ids: vec![requester_id],
                license_or_terms: "CC0-1.0".to_string(),
                restrictions: Vec::new(),
            },
            requested_final_verification: ObjectiveVerificationPolicy {
                mechanism: ObjectiveVerificationMechanism::CommittedVerifier {
                    verifier_id: requester_id,
                },
                acceptance_criteria: vec!["The stored revision can be read back.".to_string()],
                evidence_schema: "https://example.test/objective-cas.schema.json".to_string(),
                evidence_schema_hash: format!("0x{}", "11".repeat(32)),
                trust_assumptions: vec![
                    "The declared test wallet signs the verification statement.".to_string(),
                ],
            },
            privacy: ObjectivePrivacyDeclaration {
                blockchain_information_is_public: true,
                evidence_policy: PublicEvidencePolicy::Public,
                redaction_limits: "No private data is used in this test.".to_string(),
            },
        };
        let plan = Objective::plan_creation(draft).unwrap();
        let commitment = plan.commitment_hash.parse::<B256>().unwrap();
        let signature = signer.sign_message_sync(commitment.as_slice()).unwrap();
        let objective = Objective::create(
            SignedObjectiveCreation {
                approvals: vec![WalletApproval {
                    participant_id: requester_id,
                    signature: signature.to_string(),
                }],
                plan,
            },
            Utc::now(),
        )
        .unwrap();

        store.create_objective(&objective).await.unwrap();
        assert_eq!(
            store.get_objective(objective.id).await.unwrap(),
            Some(objective.clone())
        );

        let mut next = objective.clone();
        next.revision += 1;
        next.title = "Durable objective, revision two".to_string();
        next.updated_at = Utc::now();
        store
            .replace_objective(&next, objective.revision)
            .await
            .unwrap();

        let mut stale = next.clone();
        stale.revision += 1;
        assert!(matches!(
            store.replace_objective(&stale, objective.revision).await,
            Err(DbError::ObjectiveRevisionConflict { .. })
        ));
        assert_eq!(store.get_objective(objective.id).await.unwrap(), Some(next));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn social_mention_ingestion_round_trip_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let id = Uuid::new_v4();
        let mention_id = format!("0x{}", "42".repeat(20));
        let new_ingestion = NewSocialMentionIngestion {
            id,
            provider: "neynar".to_string(),
            provider_event_id: format!("cast.created:{mention_id}"),
            source_network: "farcaster".to_string(),
            mention_id: mention_id.clone(),
            mention_url: format!("https://farcaster.xyz/tester/{mention_id}"),
            author_fid: 42,
            author_handle: Some("tester".to_string()),
            mention_text: "@bountyboard /agent-bounty create 10 USDC fix it".to_string(),
            status: "reply_pending".to_string(),
            draft: Some(serde_json::json!({"draft_objective": "fix it"})),
            idempotency_key: Some(format!("social-{id}")),
            received_at: Utc::now(),
        };
        let first = store
            .reserve_social_mention_ingestion(&new_ingestion)
            .await
            .unwrap();
        assert!(first.inserted);
        let replay = store
            .reserve_social_mention_ingestion(&new_ingestion)
            .await
            .unwrap();
        assert!(!replay.inserted);
        assert_eq!(first.record.id, replay.record.id);

        let lease = Uuid::new_v4();
        let claimed = store
            .claim_social_mention_reply(id, lease, 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, "replying");
        assert_eq!(claimed.reply_attempt_count, 1);
        assert!(store
            .claim_social_mention_reply(id, Uuid::new_v4(), 30)
            .await
            .unwrap()
            .is_none());
        let failed = store
            .complete_social_mention_reply(id, lease, None, Some("provider unavailable"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed.status, "reply_failed");

        let retry_lease = Uuid::new_v4();
        let retried = store
            .claim_social_mention_reply(id, retry_lease, 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retried.reply_attempt_count, 2);
        let reply_hash = format!("0x{}", "24".repeat(20));
        let replied = store
            .complete_social_mention_reply(id, retry_lease, Some(&reply_hash), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replied.status, "replied");
        assert_eq!(
            replied.reply_cast_hash.as_deref(),
            Some(reply_hash.as_str())
        );
        assert_eq!(
            store
                .get_social_mention_ingestion(id)
                .await
                .unwrap()
                .unwrap()
                .status,
            "replied"
        );
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn site_analytics_round_trip_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let now = Utc::now();
        let event = NewSiteAnalyticsEvent {
            event_id: Uuid::new_v4(),
            visitor_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            event_name: "page_view".to_string(),
            page_path: "/".to_string(),
            source: Some("postgres-test".to_string()),
            campaign: None,
            referrer_host: None,
            opportunity_id: None,
            bounty_contract: None,
            occurred_at: now,
        };
        assert!(store.record_site_analytics_event(&event).await.unwrap());
        assert!(!store.record_site_analytics_event(&event).await.unwrap());
        let stats = store
            .site_analytics_stats(now - chrono::Duration::minutes(1))
            .await
            .unwrap();
        assert!(stats.overview.unique_visitors >= 1);
        assert!(stats.overview.page_views >= 1);
        assert!(stats
            .channels
            .iter()
            .any(|channel| channel.source == "postgres-test"));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn competitor_intelligence_migration_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let table: Option<String> = sqlx::query_scalar(
            "SELECT to_regclass('public.competitor_intelligence_changes')::text",
        )
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(table.as_deref(), Some("competitor_intelligence_changes"));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn opportunity_lifecycle_query_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let stats = store
            .opportunity_lifecycle_stats(Utc::now() - chrono::Duration::hours(1))
            .await
            .unwrap();
        assert!(stats.solution_received <= stats.published);
        assert!(stats.wallet_signed_observed <= stats.funding_prepared);
        assert!(stats.settled <= stats.canonical_created);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn discovery_webhook_round_trip_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let subscription_id = Uuid::new_v4();
        let management_token_hash = format!("token-{}", Uuid::new_v4());
        let subscription = store
            .create_discovery_webhook_subscription(&NewDiscoveryWebhookSubscription {
                id: subscription_id,
                endpoint_url: "https://agent.example/bountyboard".to_string(),
                filters: domain::DiscoverySubscriptionFilters {
                    work_states: vec!["open".to_string()],
                    ..domain::DiscoverySubscriptionFilters::default()
                },
                management_token_hash: management_token_hash.clone(),
            })
            .await
            .unwrap();
        assert_eq!(subscription.subscription_kind, "public_discovery");
        assert_eq!(
            store
                .get_webhook_subscription(subscription_id)
                .await
                .unwrap()
                .unwrap()
                .filters
                .work_states,
            vec!["open"]
        );
        assert!(store
            .list_enabled_discovery_webhook_subscriptions()
            .await
            .unwrap()
            .iter()
            .any(|item| item.id == subscription_id));

        let event_id = Uuid::new_v4();
        assert!(store
            .enqueue_webhook_delivery(
                subscription_id,
                event_id,
                AgentWebhookEventType::OpportunityPublished,
                &serde_json::json!({"opportunity_id": "unfunded:test"}),
            )
            .await
            .unwrap());
        assert!(!store
            .enqueue_webhook_delivery(
                subscription_id,
                event_id,
                AgentWebhookEventType::OpportunityPublished,
                &serde_json::json!({"opportunity_id": "unfunded:test"}),
            )
            .await
            .unwrap());

        let lease_token = Uuid::new_v4();
        let delivery = store
            .lease_webhook_deliveries(100, lease_token, 30)
            .await
            .unwrap()
            .into_iter()
            .find(|delivery| delivery.subscription_id == subscription_id)
            .unwrap();
        assert_eq!(delivery.attempt_count, 1);
        assert!(store
            .mark_webhook_delivery_delivered(delivery.id, lease_token, 204)
            .await
            .unwrap());
        assert!(!store
            .delete_discovery_webhook_subscription(subscription_id, "wrong-token")
            .await
            .unwrap());
        assert!(store
            .delete_discovery_webhook_subscription(subscription_id, &management_token_hash)
            .await
            .unwrap());
        assert!(store
            .get_webhook_subscription(subscription_id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn x402_relay_attempt_is_idempotent_and_lease_bounded() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
        let network = format!("x402-test-{}", Uuid::new_v4());
        let attempt = NewX402RelayAttempt {
            id: Uuid::new_v4(),
            idempotency_key: format!("x402-test-{}", Uuid::new_v4()),
            network: network.clone(),
            bounty_contract: "0x1111111111111111111111111111111111111111".to_string(),
            contributor: "0x2222222222222222222222222222222222222222".to_string(),
            amount: 150_000,
            authorization_nonce: nonce,
            authorization_valid_before: 2_000_000_000,
            request_fingerprint: "fingerprint-a".to_string(),
            relayer_address: "0x3333333333333333333333333333333333333333".to_string(),
        };
        let first = store
            .reserve_x402_relay_attempt(&attempt, 2, 1)
            .await
            .unwrap();
        let replay = store
            .reserve_x402_relay_attempt(&attempt, 2, 1)
            .await
            .unwrap();
        assert_eq!(first.id, replay.id);

        let mut conflict = attempt.clone();
        conflict.id = Uuid::new_v4();
        conflict.request_fingerprint = "fingerprint-b".to_string();
        assert!(matches!(
            store.reserve_x402_relay_attempt(&conflict, 2, 1).await,
            Err(DbError::X402RelayConflict(_))
        ));

        let mut contributor_quota = attempt.clone();
        contributor_quota.id = Uuid::new_v4();
        contributor_quota.idempotency_key = format!("x402-test-{}", Uuid::new_v4());
        contributor_quota.authorization_nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
        contributor_quota.request_fingerprint = "fingerprint-contributor-quota".to_string();
        assert!(matches!(
            store
                .reserve_x402_relay_attempt(&contributor_quota, 2, 1)
                .await,
            Err(DbError::X402RelayQuotaExceeded(_))
        ));

        let mut second = contributor_quota.clone();
        second.contributor = "0x4444444444444444444444444444444444444444".to_string();
        second.request_fingerprint = "fingerprint-second".to_string();
        let second = store
            .reserve_x402_relay_attempt(&second, 2, 1)
            .await
            .unwrap();
        assert_ne!(second.id, first.id);

        let mut network_quota = contributor_quota;
        network_quota.id = Uuid::new_v4();
        network_quota.idempotency_key = format!("x402-test-{}", Uuid::new_v4());
        network_quota.authorization_nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
        network_quota.contributor = "0x5555555555555555555555555555555555555555".to_string();
        network_quota.request_fingerprint = "fingerprint-network-quota".to_string();
        assert!(matches!(
            store.reserve_x402_relay_attempt(&network_quota, 2, 1).await,
            Err(DbError::X402RelayQuotaExceeded(_))
        ));

        let lease = store
            .acquire_x402_relayer_lease(&network, 30)
            .await
            .unwrap()
            .unwrap();
        assert!(store
            .acquire_x402_relayer_lease(&network, 30)
            .await
            .unwrap()
            .is_none());
        let claimed = store
            .claim_x402_relay_attempt(first.id, lease, 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, X402RelayStatus::Relaying);
        sqlx::query(
            "UPDATE x402_relay_attempts SET lease_expires_at = now() - interval '1 second' WHERE id = $1",
        )
        .bind(first.id)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE x402_relayer_leases SET lease_expires_at = now() - interval '1 second' WHERE network = $1",
        )
        .bind(&network)
        .execute(&store.pool)
        .await
        .unwrap();
        let recovered_lease = store
            .acquire_x402_relayer_lease(&network, 30)
            .await
            .unwrap()
            .unwrap();
        let recovered = store
            .claim_x402_relay_attempt(first.id, recovered_lease, 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, X402RelayStatus::Relaying);
        assert_eq!(recovered.attempt_count, 2);
        let broadcast = store
            .mark_x402_relay_broadcast(
                first.id,
                recovered_lease,
                &format!("0x{}", "44".repeat(32)),
                100_000,
                120_000,
            )
            .await
            .unwrap();
        assert_eq!(broadcast.status, X402RelayStatus::Broadcast);
        store
            .release_x402_relayer_lease(&network, recovered_lease)
            .await
            .unwrap();
        let confirmed = store
            .mark_x402_relay_confirmed(first.id, Uuid::new_v4(), 123)
            .await
            .unwrap();
        assert_eq!(confirmed.status, X402RelayStatus::Confirmed);
        assert!(!confirmed.retryable);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn claim_funnel_counts_direct_and_atomic_sponsored_confirmations() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let baseline = store.claim_funnel_stats(1).await.unwrap();
        let network = format!("funnel-test-{}", Uuid::new_v4());
        let address = |id: Uuid| {
            let value = id.simple().to_string();
            format!("0x{value}{}", &value[..8])
        };
        let reserve = |bounty_contract: String, solver_wallet: String| NewClaimCandidate {
            id: Uuid::new_v4(),
            idempotency_key: format!("claim-funnel-{}", Uuid::new_v4()),
            network: network.clone(),
            bounty_contract,
            solver_wallet: solver_wallet.clone(),
            agent_id: None,
            eligibility_evidence: AgentEligibilityEvidence {
                agent_id: None,
                solver_wallet,
                capabilities: Vec::new(),
                paid_completions: 0,
                paid_usdc_base_units: 0,
            },
            eligibility_decision: AgentEligibilityDecision {
                eligible: true,
                reasons: Vec::new(),
            },
        };
        let valid_before = u64::try_from(Utc::now().timestamp()).unwrap() + 600;

        let direct_input = reserve(address(Uuid::new_v4()), address(Uuid::new_v4()));
        let direct = store
            .reserve_claim_candidate(&direct_input, 600, 5)
            .await
            .unwrap()
            .candidate;
        store
            .set_claim_candidate_authorization(
                direct.id,
                &format!("0x{}", "11".repeat(32)),
                valid_before,
            )
            .await
            .unwrap();
        store
            .mark_claim_candidate_relaying(direct.id, &format!("0x{}", "22".repeat(32)))
            .await
            .unwrap();
        let direct_claim_event_id = Uuid::new_v4();
        store
            .mark_claim_candidate_claimed(direct.id, direct_claim_event_id)
            .await
            .unwrap();

        let sponsored_input = reserve(address(Uuid::new_v4()), address(Uuid::new_v4()));
        let sponsored = store
            .reserve_claim_candidate(&sponsored_input, 600, 5)
            .await
            .unwrap()
            .candidate;
        store
            .set_claim_candidate_authorization(
                sponsored.id,
                &format!("0x{}", "33".repeat(32)),
                valid_before,
            )
            .await
            .unwrap();
        let sponsorship = store
            .reserve_bond_sponsorship(
                &NewBondSponsorship {
                    id: Uuid::new_v4(),
                    claim_candidate_id: sponsored.id,
                    network: network.clone(),
                    bounty_contract: sponsored.bounty_contract.clone(),
                    solver_wallet: sponsored.solver_wallet.clone(),
                    sponsor_wallet: address(Uuid::new_v4()),
                    amount: 10_000,
                },
                100_000,
                10_000,
            )
            .await
            .unwrap();
        store
            .mark_atomic_sponsored_claim_broadcast(
                sponsored.id,
                sponsorship.id,
                &format!("0x{}", "44".repeat(32)),
            )
            .await
            .unwrap();
        let sponsored_claim_event_id = Uuid::new_v4();
        store
            .mark_atomic_sponsored_claim_confirmed(
                sponsored.id,
                sponsorship.id,
                sponsored_claim_event_id,
                1,
            )
            .await
            .unwrap();

        let event = |id: Uuid,
                     kind: AutonomousBountyEventKind,
                     bounty_contract: &str,
                     solver_wallet: &str,
                     block_number: u64| {
            let tx_hash = format!("0x{}", Uuid::new_v4().simple().to_string().repeat(2));
            AutonomousBountyEvent {
                id,
                log_key: format!("{tx_hash}:0"),
                tx_hash,
                block_number,
                log_index: 0,
                contract_address: bounty_contract.to_string(),
                bounty_id: format!("0x{}", Uuid::new_v4().simple().to_string().repeat(2)),
                kind,
                data: serde_json::json!({"round": 1, "solver": solver_wallet}),
                occurred_at: Utc::now(),
            }
        };
        let mut events = Vec::new();
        {
            let mut add_loop =
                |claim_id: Uuid, bounty_contract: &str, solver_wallet: &str, first_block: u64| {
                    events.extend([
                        event(
                            claim_id,
                            AutonomousBountyEventKind::BountyClaimed,
                            bounty_contract,
                            solver_wallet,
                            first_block,
                        ),
                        event(
                            Uuid::new_v4(),
                            AutonomousBountyEventKind::SubmissionAdded,
                            bounty_contract,
                            solver_wallet,
                            first_block + 1,
                        ),
                        event(
                            Uuid::new_v4(),
                            AutonomousBountyEventKind::BountySettled,
                            bounty_contract,
                            solver_wallet,
                            first_block + 2,
                        ),
                    ]);
                };
            add_loop(
                direct_claim_event_id,
                &direct.bounty_contract,
                &direct.solver_wallet,
                1,
            );
            add_loop(
                sponsored_claim_event_id,
                &sponsored.bounty_contract,
                &sponsored.solver_wallet,
                4,
            );
            let unattributed_solver = address(Uuid::new_v4());
            for offset in 0..2_u64 {
                let bounty_contract = address(Uuid::new_v4());
                add_loop(
                    Uuid::new_v4(),
                    &bounty_contract,
                    &unattributed_solver,
                    7 + offset * 3,
                );
            }
        }
        for event in events {
            store
                .upsert_autonomous_bounty_event(&network, &event)
                .await
                .unwrap();
        }

        let observed = store.claim_funnel_stats(1).await.unwrap();
        assert_eq!(observed.stages.observed, baseline.stages.observed + 2);
        assert_eq!(
            observed.stages.unique_solver_wallets,
            baseline.stages.unique_solver_wallets + 2
        );
        assert_eq!(
            observed.stages.authorization_prepared,
            baseline.stages.authorization_prepared + 2
        );
        assert_eq!(
            observed.stages.transaction_broadcast,
            baseline.stages.transaction_broadcast + 2
        );
        assert_eq!(
            observed.stages.claimed_canonical,
            baseline.stages.claimed_canonical + 2
        );
        assert_eq!(
            observed.sponsorship.sponsored_claims_confirmed,
            baseline.sponsorship.sponsored_claims_confirmed + 1
        );
        assert_eq!(
            observed.sponsorship.direct_claims_confirmed,
            baseline.sponsorship.direct_claims_confirmed + 1
        );
        assert_eq!(
            observed.canonical_outcomes.claims_confirmed,
            baseline.canonical_outcomes.claims_confirmed + 4
        );
        assert_eq!(
            observed.canonical_outcomes.unique_claimed_solver_wallets,
            baseline.canonical_outcomes.unique_claimed_solver_wallets + 3
        );
        assert_eq!(
            observed.canonical_outcomes.hosted_claims_confirmed,
            baseline.canonical_outcomes.hosted_claims_confirmed + 2
        );
        assert_eq!(
            observed.canonical_outcomes.unattributed_claims_confirmed,
            baseline.canonical_outcomes.unattributed_claims_confirmed + 2
        );
        assert_eq!(
            observed.canonical_outcomes.submissions_confirmed,
            baseline.canonical_outcomes.submissions_confirmed + 4
        );
        assert_eq!(
            observed.canonical_outcomes.settlements_confirmed,
            baseline.canonical_outcomes.settlements_confirmed + 4
        );
        assert_eq!(
            observed.canonical_outcomes.unique_paid_solver_wallets,
            baseline.canonical_outcomes.unique_paid_solver_wallets + 3
        );
        assert_eq!(
            observed.canonical_outcomes.repeat_paid_solver_wallets,
            baseline.canonical_outcomes.repeat_paid_solver_wallets + 1
        );
    }

    #[test]
    fn persisted_platform_fee_allows_zero_but_rejects_negative_amounts() {
        assert_eq!(
            persisted_nonnegative_money(0, "USDC".to_string()).unwrap(),
            Money::zero("usdc")
        );
        assert!(persisted_nonnegative_money(-1, "usdc".to_string()).is_err());
    }

    #[test]
    fn migration_lock_id_is_stable() {
        assert_eq!(MIGRATION_ADVISORY_LOCK_ID, 4_270_265_017);
    }

    #[test]
    fn payment_event_upsert_preserves_applied_events() {
        assert!(UPSERT_PAYMENT_EVENT_SQL.contains("ON CONFLICT (external_id) DO UPDATE SET"));
        assert!(UPSERT_PAYMENT_EVENT_SQL.contains("WHEN payment_events.status = 'Applied'"));
        assert!(UPSERT_PAYMENT_EVENT_SQL.contains("THEN payment_events.status"));
        assert!(UPSERT_PAYMENT_EVENT_SQL.contains("THEN payment_events.payload_hash"));
        assert!(UPSERT_PAYMENT_EVENT_SQL.contains("THEN payment_events.received_at"));
    }

    #[test]
    fn github_issue_sync_upsert_locks_bounty_before_activity_check() {
        assert!(LOCK_GITHUB_ISSUE_SYNC_BOUNTY_SQL.contains("pg_advisory_xact_lock"));
        assert!(LOCK_GITHUB_ISSUE_SYNC_BOUNTY_SQL.contains("hashtextextended($1::text"));
        assert!(SELECT_GITHUB_ISSUE_SYNC_BOUNTY_FOR_UPDATE_SQL.contains("FOR UPDATE"));
        for table in [
            "funding_intents",
            "funding_contributions",
            "claims",
            "submissions",
        ] {
            assert!(
                GITHUB_ISSUE_SYNC_ACTIVITY_SQL.contains(table),
                "missing persisted activity table {table}"
            );
        }
        assert!(UPDATE_GITHUB_ISSUE_SYNC_BOUNTY_SQL.contains("WHERE id = $1"));
        assert!(UPDATE_GITHUB_ISSUE_SYNC_BOUNTY_SQL.contains("RETURNING id"));
        assert!(!UPDATE_GITHUB_ISSUE_SYNC_BOUNTY_SQL.contains("created_at ="));
    }
}
