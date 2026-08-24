use chain_base::{
    AutonomousBountyEvent, AutonomousBountyEventKind, OpenCompetitionEvent,
    OpenCompetitionEventKind, OpenCompetitionV2Event, OpenCompetitionV2EventKind,
    OpenCompetitionV2ProjectedState, OpenCompetitionV2Projection,
};
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
pub const OBJECTIVE_COORDINATION_MIGRATION: &str =
    include_str!("../../../migrations/0013_objective_coordination.sql");
pub const PUBLIC_COMPETITOR_INTELLIGENCE_REMOVAL_MIGRATION: &str =
    include_str!("../../../migrations/0014_remove_public_competitor_intelligence.sql");
pub const OPPORTUNITY_COMMENTS_MIGRATION: &str =
    include_str!("../../../migrations/0015_opportunity_comments.sql");
pub const CHATGPT_ACTION_INTENTS_MIGRATION: &str =
    include_str!("../../../migrations/0016_chatgpt_action_intents.sql");
pub const BOUNTY_IMAGE_ASSETS_MIGRATION: &str =
    include_str!("../../../migrations/0017_bounty_image_assets.sql");
pub const SOLVE_ACTION_RENAME_MIGRATION: &str =
    include_str!("../../../migrations/0018_rename_compete_action_to_solve.sql");
pub const OPEN_COMPETITION_V1_MIGRATION: &str =
    include_str!("../../../migrations/0019_open_competition_v1.sql");
pub const OPEN_COMPETITION_ENTRANT_RELAYS_MIGRATION: &str =
    include_str!("../../../migrations/0020_open_competition_entrant_relays.sql");
pub const INTERFACE_USAGE_MIGRATION: &str =
    include_str!("../../../migrations/0021_interface_usage.sql");
pub const EXTERNAL_INTERFACE_USAGE_MIGRATION: &str =
    include_str!("../../../migrations/0022_external_interface_usage.sql");
pub const OPEN_COMPETITION_V2_BETA2_MIGRATION: &str =
    include_str!("../../../migrations/0023_open_competition_v2_beta2.sql");
pub const OPEN_COMPETITION_V2_BETA3_MIGRATION: &str =
    include_str!("../../../migrations/0024_open_competition_v2_beta3.sql");
pub const OPPORTUNITY_FEEDBACK_MIGRATION: &str =
    include_str!("../../../migrations/0025_opportunity_feedback.sql");
pub const SITE_AUTH_ACCOUNTS_MIGRATION: &str =
    include_str!("../../../migrations/0026_site_auth_accounts.sql");
pub const COMPETITION_ACTIVATION_ANALYTICS_MIGRATION: &str =
    include_str!("../../../migrations/0027_competition_activation_analytics.sql");
pub const SITE_AUTH_PASSWORD_ACCOUNTS_MIGRATION: &str =
    include_str!("../../../migrations/0028_site_auth_password_accounts.sql");
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
    #[error("conflicting open-competition event replay: {0}")]
    OpenCompetitionEventConflict(String),
    #[error("conflicting Open Competition V2 canonical replay: {0}")]
    OpenCompetitionV2Conflict(String),
    #[error("conflicting x402 relay replay: {0}")]
    X402RelayConflict(String),
    #[error("x402 hosted relay quota exceeded: {0}")]
    X402RelayQuotaExceeded(String),
    #[error("conflicting open-competition entrant relay replay: {0}")]
    OpenCompetitionEntrantRelayConflict(String),
    #[error("open-competition entrant relay quota exceeded: {0}")]
    OpenCompetitionEntrantRelayQuotaExceeded(String),
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
    #[error("opportunity comment idempotency conflict")]
    OpportunityCommentConflict,
    #[error("ChatGPT action intent conflict: {0}")]
    ChatgptActionIntentConflict(String),
    #[error("ChatGPT action intent is unavailable")]
    ChatgptActionIntentUnavailable,
    #[error("bounty image asset conflict: {0}")]
    BountyImageAssetConflict(String),
    #[error("site account wallet conflict: {0}")]
    SiteAuthConflict(String),
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
pub struct NewOpportunityComment {
    pub id: Uuid,
    pub opportunity_id: String,
    pub author: String,
    pub body: String,
    pub feedback: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunityComment {
    pub id: Uuid,
    pub opportunity_id: String,
    pub author: String,
    pub body: String,
    pub feedback: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewChatgptActionIntent {
    pub id: Uuid,
    pub idempotency_key: String,
    pub action: String,
    pub network: String,
    pub opportunity_id: Option<String>,
    pub bounty_contract: Option<String>,
    pub bounty_id: Option<String>,
    pub actor_wallet: Option<String>,
    pub amount_base_units: Option<u64>,
    pub details: serde_json::Value,
    pub request_fingerprint: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatgptActionIntent {
    pub id: Uuid,
    pub idempotency_key: String,
    pub action: String,
    pub network: String,
    pub opportunity_id: Option<String>,
    pub bounty_contract: Option<String>,
    pub bounty_id: Option<String>,
    pub actor_wallet: Option<String>,
    pub amount_base_units: Option<u64>,
    pub details: serde_json::Value,
    pub request_fingerprint: String,
    pub status: String,
    pub transaction_hash: Option<String>,
    pub canonical_event_id: Option<Uuid>,
    pub canonical_event_kind: Option<String>,
    pub confirmed_block: Option<u64>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ChatgptActionObservation {
    pub transaction_hash: String,
    pub bounty_contract: Option<String>,
    pub bounty_id: Option<String>,
    pub actor_wallet: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewBountyImageAsset {
    pub sha256: String,
    pub mime_type: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BountyImageAsset {
    pub sha256: String,
    pub mime_type: String,
    pub content: Vec<u8>,
    pub created_at: DateTime<Utc>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedInterface {
    Api,
    Cli,
    Mcp,
}

impl ObservedInterface {
    fn as_str(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedProtocolEra {
    NotApplicable,
    McpLegacy,
    McpModern,
    McpHttpAdapter,
}

impl ObservedProtocolEra {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::McpLegacy => "legacy",
            Self::McpModern => "modern",
            Self::McpHttpAdapter => "http_adapter",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InterfaceUsageStats {
    pub interface: String,
    pub protocol_era: String,
    pub request_count: u64,
    pub successful_request_count: u64,
    pub first_observed_at: DateTime<Utc>,
    pub last_observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SiteAnalyticsStats {
    pub overview: SiteAnalyticsOverview,
    pub event_counts: Vec<SiteAnalyticsEventCount>,
    pub daily: Vec<SiteAnalyticsDailyStats>,
    pub channels: Vec<SiteAnalyticsChannelStats>,
    pub interfaces: Vec<InterfaceUsageStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformIdentityStats {
    pub selected: u64,
    pub previous: u64,
    pub latest_week: u64,
    pub previous_week: u64,
    pub first_month: u64,
    pub lifetime: u64,
    pub posters: u64,
    pub funders: u64,
    pub solvers: u64,
    pub verifiers: u64,
    pub commenters: u64,
    pub marketplace_wallets: u64,
    pub opportunity_comment_authors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformPayoutStats {
    pub selected_total_base_units: String,
    pub previous_total_base_units: String,
    pub first_month_total_base_units: String,
    pub lifetime_total_base_units: String,
    pub selected_solver_base_units: String,
    pub selected_verifier_base_units: String,
    pub selected_keeper_base_units: String,
    pub selected_bonus_base_units: String,
    pub selected_settled_rounds: u64,
    pub previous_settled_rounds: u64,
    pub first_month_settled_rounds: u64,
    pub lifetime_settled_rounds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformClaimCohortStats {
    pub settled: u64,
    pub mature: u64,
    pub immature: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformDailyStats {
    pub day: String,
    pub active_identities: u64,
    pub payout_base_units: String,
    pub settled_rounds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformMetricsCoverageStats {
    pub verified_canonical_events: u64,
    pub awaiting_block_time_events: u64,
    pub opportunity_comments: u64,
    pub latest_verified_event_at: Option<DateTime<Utc>>,
    pub latest_comment_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformMetricsStats {
    pub generated_at: DateTime<Utc>,
    pub identities: PlatformIdentityStats,
    pub payouts: PlatformPayoutStats,
    pub claim_cohort: PlatformClaimCohortStats,
    pub daily: Vec<PlatformDailyStats>,
    pub coverage: PlatformMetricsCoverageStats,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformDemandGrowthStats {
    pub gmv_7d_base_units: String,
    pub gmv_28d_base_units: String,
    pub lifetime_gmv_base_units: String,
    pub new_poster_funder_wallets_28d: u64,
    pub active_poster_funder_wallets_28d: u64,
    pub repeat_poster_funder_wallets_28d: u64,
    pub non_operator_attributed_gmv_28d_base_units: String,
    pub attributed_gmv_28d_base_units: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionEntrantRelayStatus {
    Prepared,
    Relaying,
    Broadcast,
    Confirmed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOpenCompetitionEntrantRelay {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub wallet: String,
    pub bounty_contract: String,
    pub delegate: String,
    pub action: u8,
    pub wallet_nonce: u64,
    pub deadline: u64,
    pub payload_hash: String,
    pub request_fingerprint: String,
    pub relayer_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionEntrantRelay {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub wallet: String,
    pub bounty_contract: String,
    pub delegate: String,
    pub action: u8,
    pub wallet_nonce: u64,
    pub deadline: u64,
    pub payload_hash: String,
    pub request_fingerprint: String,
    pub relayer_address: String,
    pub status: OpenCompetitionEntrantRelayStatus,
    pub retryable: bool,
    pub attempt_count: u32,
    pub tx_hash: Option<String>,
    pub estimated_gas: Option<u64>,
    pub gas_limit: Option<u64>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub receipt_block: Option<u64>,
    pub receipt_block_hash: Option<String>,
    pub canonical_safe_block: Option<u64>,
    pub canonical_safe_block_hash: Option<String>,
    pub canonical_event: Option<String>,
    pub payment_proven: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCompetitionV2SafeContext {
    pub block_hash: String,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2StoredProjection {
    pub network: String,
    pub factory_contract: String,
    pub projection: OpenCompetitionV2Projection,
    pub safe_block_number: u64,
    pub safe_block_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCompetitionV2ProofJobState {
    Quoted,
    PaymentPending,
    Paid,
    Proving,
    Proved,
    Relaying,
    Confirmed,
    RefundDue,
    Refunded,
    LostCompetition,
}

impl OpenCompetitionV2ProofJobState {
    fn storage_name(self) -> &'static str {
        match self {
            Self::Quoted => "quoted",
            Self::PaymentPending => "payment_pending",
            Self::Paid => "paid",
            Self::Proving => "proving",
            Self::Proved => "proved",
            Self::Relaying => "relaying",
            Self::Confirmed => "confirmed",
            Self::RefundDue => "refund_due",
            Self::Refunded => "refunded",
            Self::LostCompetition => "lost_competition",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ProofJob {
    pub id: Uuid,
    pub idempotency_key: String,
    pub network: String,
    pub competition_contract: String,
    pub solver: String,
    pub solver_nonce: String,
    pub artifact_hash: String,
    pub program_input: serde_json::Value,
    pub expected_public_values: String,
    pub requested_relay: bool,
    pub proof_system: String,
    pub state: OpenCompetitionV2ProofJobState,
    pub gross_prize: String,
    pub proof_fee_quote: String,
    pub relay_fee_quote: String,
    pub net_prize_if_win: String,
    pub maximum_charge: String,
    pub winner_mode: String,
    pub competition_risk: String,
    pub quote_expires_at: DateTime<Utc>,
    pub proof_sla_deadline: DateTime<Utc>,
    pub payer: Option<String>,
    pub payment_authorization_nonce: Option<String>,
    pub payment_authorization: Option<serde_json::Value>,
    pub payment_tx_hash: Option<String>,
    pub payment_block_number: Option<u64>,
    pub payment_evidence: Option<serde_json::Value>,
    pub proof_hash: Option<String>,
    pub public_values_hash: Option<String>,
    pub proof: Option<String>,
    pub public_values: Option<String>,
    pub proof_provider_job_id: Option<String>,
    pub solver_authorization_deadline: Option<u64>,
    pub solver_signature: Option<String>,
    pub relay_tx_hash: Option<String>,
    pub settlement_event_id: Option<Uuid>,
    pub refund_evidence: Option<serde_json::Value>,
    pub refund_tx_hash: Option<String>,
    pub refund_block_number: Option<u64>,
    pub refund_due_at: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub attempt_count: u32,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OpenCompetitionV2ProofJobUpdate {
    pub payer: Option<String>,
    pub payment_authorization_nonce: Option<String>,
    pub payment_authorization: Option<serde_json::Value>,
    pub payment_tx_hash: Option<String>,
    pub payment_block_number: Option<u64>,
    pub payment_evidence: Option<serde_json::Value>,
    pub proof_hash: Option<String>,
    pub public_values_hash: Option<String>,
    pub proof: Option<String>,
    pub public_values: Option<String>,
    pub proof_provider_job_id: Option<String>,
    pub solver_authorization_deadline: Option<u64>,
    pub solver_signature: Option<String>,
    pub relay_tx_hash: Option<String>,
    pub settlement_event_id: Option<Uuid>,
    pub refund_evidence: Option<serde_json::Value>,
    pub refund_tx_hash: Option<String>,
    pub refund_block_number: Option<u64>,
    pub refund_due_at: Option<DateTime<Utc>>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteAuthWallet {
    pub address: String,
    pub chain_id: i64,
    pub linked_at: DateTime<Utc>,
    pub proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteAuthPrincipal {
    pub account_key: String,
    pub display_name: String,
    pub email: String,
    pub avatar_url: String,
    pub sign_in_method: String,
    pub linked_methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteAuthPasswordRecord {
    pub account_key: String,
    pub password_phc: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteAuthEmailAction {
    pub token_hash: String,
    pub purpose: String,
    pub email: String,
    pub email_key: String,
    pub account_key: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2IndexerAgreement {
    pub network: String,
    pub factory_contract: String,
    pub protocol_version: String,
    pub common_safe_block: u64,
    pub primary_safe_head: u64,
    pub shadow_safe_head: u64,
    pub primary_block_hash: String,
    pub shadow_block_hash: String,
    pub canonical_event_count: u64,
    pub canonical_event_set_hash: String,
    pub agrees: bool,
    pub failure_code: Option<String>,
    pub observed_at: DateTime<Utc>,
}

fn site_auth_email_action_from_row(row: PgRow) -> DbResult<SiteAuthEmailAction> {
    Ok(SiteAuthEmailAction {
        token_hash: row.try_get("token_hash")?,
        purpose: row.try_get("purpose")?,
        email: row.try_get("email")?,
        email_key: row.try_get("email_key")?,
        account_key: row.try_get("account_key")?,
        expires_at: row.try_get("expires_at")?,
        verified_at: row.try_get("verified_at")?,
        consumed_at: row.try_get("consumed_at")?,
    })
}

async fn merge_site_auth_accounts(
    transaction: &mut Transaction<'_, Postgres>,
    source: &str,
    target: &str,
) -> DbResult<()> {
    sqlx::query(
        "DELETE FROM site_auth_password_credentials WHERE account_key = $1 AND EXISTS (SELECT 1 FROM site_auth_password_credentials WHERE account_key = $2)",
    )
    .bind(source)
    .bind(target)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "UPDATE site_auth_password_credentials SET account_key = $2 WHERE account_key = $1",
    )
    .bind(source)
    .bind(target)
    .execute(&mut **transaction)
    .await?;
    sqlx::query("UPDATE site_auth_wallets SET account_key = $2 WHERE account_key = $1")
        .bind(source)
        .bind(target)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE site_auth_sessions SET account_key = $2 WHERE account_key = $1")
        .bind(source)
        .bind(target)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE site_auth_email_actions SET account_key = $2 WHERE account_key = $1")
        .bind(source)
        .bind(target)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE site_auth_verified_emails SET account_key = $2 WHERE account_key = $1")
        .bind(source)
        .bind(target)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("UPDATE site_auth_identities SET account_key = $2 WHERE account_key = $1")
        .bind(source)
        .bind(target)
        .execute(&mut **transaction)
        .await?;
    sqlx::query("DELETE FROM site_auth_accounts WHERE account_key = $1")
        .bind(source)
        .execute(&mut **transaction)
        .await?;
    Ok(())
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
                OBJECTIVE_COORDINATION_MIGRATION,
                PUBLIC_COMPETITOR_INTELLIGENCE_REMOVAL_MIGRATION,
                OPPORTUNITY_COMMENTS_MIGRATION,
                CHATGPT_ACTION_INTENTS_MIGRATION,
                BOUNTY_IMAGE_ASSETS_MIGRATION,
                SOLVE_ACTION_RENAME_MIGRATION,
                OPEN_COMPETITION_V1_MIGRATION,
                OPEN_COMPETITION_ENTRANT_RELAYS_MIGRATION,
                INTERFACE_USAGE_MIGRATION,
                EXTERNAL_INTERFACE_USAGE_MIGRATION,
                OPEN_COMPETITION_V2_BETA2_MIGRATION,
                OPEN_COMPETITION_V2_BETA3_MIGRATION,
                OPPORTUNITY_FEEDBACK_MIGRATION,
                SITE_AUTH_ACCOUNTS_MIGRATION,
                COMPETITION_ACTIVATION_ANALYTICS_MIGRATION,
                SITE_AUTH_PASSWORD_ACCOUNTS_MIGRATION,
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

    pub async fn upsert_site_auth_account(
        &self,
        account_key: &str,
        provider: &str,
        provider_subject: &str,
        display_name: &str,
        email: &str,
        avatar_url: &str,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO site_auth_accounts
              (account_key, provider, provider_subject, display_name, email, avatar_url)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (account_key) DO UPDATE SET
              display_name = EXCLUDED.display_name,
              email = EXCLUDED.email,
              avatar_url = EXCLUDED.avatar_url,
              last_signed_in_at = NOW()
            "#,
        )
        .bind(account_key)
        .bind(provider)
        .bind(provider_subject)
        .bind(display_name)
        .bind(email)
        .bind(avatar_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_site_auth_identity(
        &self,
        proposed_account_key: &str,
        provider: &str,
        provider_subject: &str,
        display_name: &str,
        email: &str,
        avatar_url: &str,
        verified_email: Option<(&str, &str)>,
    ) -> DbResult<String> {
        let mut transaction = self.pool.begin().await?;
        let existing_account: Option<String> = sqlx::query_scalar(
            "SELECT account_key FROM site_auth_identities WHERE provider = $1 AND provider_subject = $2 FOR UPDATE",
        )
        .bind(provider)
        .bind(provider_subject)
        .fetch_optional(&mut *transaction)
        .await?;
        let verified_owner = if let Some((_, email_key)) = verified_email {
            sqlx::query_scalar::<_, String>(
                "SELECT account_key FROM site_auth_verified_emails WHERE email_key = $1 FOR UPDATE",
            )
            .bind(email_key)
            .fetch_optional(&mut *transaction)
            .await?
        } else {
            None
        };
        let target = verified_owner
            .or_else(|| existing_account.clone())
            .unwrap_or_else(|| proposed_account_key.to_string());

        if let Some(source) = existing_account
            .as_ref()
            .filter(|source| *source != &target)
        {
            merge_site_auth_accounts(&mut transaction, source, &target).await?;
        }

        sqlx::query(
            r#"
            INSERT INTO site_auth_accounts
              (account_key, provider, provider_subject, display_name, email, avatar_url)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (account_key) DO UPDATE SET
              display_name = EXCLUDED.display_name,
              email = CASE WHEN EXCLUDED.email = '' THEN site_auth_accounts.email ELSE EXCLUDED.email END,
              avatar_url = CASE WHEN EXCLUDED.avatar_url = '' THEN site_auth_accounts.avatar_url ELSE EXCLUDED.avatar_url END,
              last_signed_in_at = NOW()
            "#,
        )
        .bind(&target)
        .bind(provider)
        .bind(provider_subject)
        .bind(display_name)
        .bind(email)
        .bind(avatar_url)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO site_auth_identities
              (provider, provider_subject, account_key, verified_email, verified_email_key, email_verified_at)
            VALUES ($1, $2, $3, $4, $5, CASE WHEN $5 IS NULL THEN NULL ELSE NOW() END)
            ON CONFLICT (provider, provider_subject) DO UPDATE SET
              account_key = EXCLUDED.account_key,
              verified_email = COALESCE(EXCLUDED.verified_email, site_auth_identities.verified_email),
              verified_email_key = COALESCE(EXCLUDED.verified_email_key, site_auth_identities.verified_email_key),
              email_verified_at = COALESCE(EXCLUDED.email_verified_at, site_auth_identities.email_verified_at),
              last_signed_in_at = NOW()
            "#,
        )
        .bind(provider)
        .bind(provider_subject)
        .bind(&target)
        .bind(verified_email.map(|value| value.0))
        .bind(verified_email.map(|value| value.1))
        .execute(&mut *transaction)
        .await?;
        if let Some((email, email_key)) = verified_email {
            sqlx::query(
                r#"
                INSERT INTO site_auth_verified_emails (email_key, email, account_key)
                VALUES ($1, $2, $3)
                ON CONFLICT (email_key) DO UPDATE SET email = EXCLUDED.email
                "#,
            )
            .bind(email_key)
            .bind(email)
            .bind(&target)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(target)
    }

    pub async fn create_site_auth_session(
        &self,
        token_hash: &str,
        account_key: &str,
        sign_in_method: &str,
        expires_at: DateTime<Utc>,
    ) -> DbResult<()> {
        sqlx::query(
            "INSERT INTO site_auth_sessions (token_hash, account_key, sign_in_method, expires_at) VALUES ($1, $2, $3, $4)",
        )
        .bind(token_hash)
        .bind(account_key)
        .bind(sign_in_method)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn site_auth_principal_for_session(
        &self,
        token_hash: &str,
    ) -> DbResult<Option<SiteAuthPrincipal>> {
        let row = sqlx::query(
            r#"
            SELECT s.account_key, a.display_name, a.email, a.avatar_url, s.sign_in_method
            FROM site_auth_sessions s
            JOIN site_auth_accounts a ON a.account_key = s.account_key
            WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > NOW()
            "#,
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let account_key: String = row.try_get("account_key")?;
        sqlx::query(
            "UPDATE site_auth_sessions SET last_seen_at = NOW() WHERE token_hash = $1 AND last_seen_at < NOW() - INTERVAL '5 minutes'",
        )
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        let linked_methods = sqlx::query_scalar::<_, String>(
            "SELECT provider FROM site_auth_identities WHERE account_key = $1 UNION SELECT 'password' FROM site_auth_password_credentials WHERE account_key = $1 AND enabled ORDER BY 1",
        )
        .bind(&account_key)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(SiteAuthPrincipal {
            account_key,
            display_name: row.try_get("display_name")?,
            email: row.try_get("email")?,
            avatar_url: row.try_get("avatar_url")?,
            sign_in_method: row.try_get("sign_in_method")?,
            linked_methods,
        }))
    }

    pub async fn revoke_site_auth_session(&self, token_hash: &str) -> DbResult<()> {
        sqlx::query("UPDATE site_auth_sessions SET revoked_at = NOW() WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn site_auth_attempt_allowed(
        &self,
        scope: &str,
        subject_hash: &str,
        window_seconds: i64,
        maximum_attempts: i32,
        block_seconds: i64,
    ) -> DbResult<bool> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT window_started_at, attempts, blocked_until FROM site_auth_attempts WHERE scope = $1 AND subject_hash = $2 FOR UPDATE",
        )
        .bind(scope)
        .bind(subject_hash)
        .fetch_optional(&mut *transaction)
        .await?;
        let now = Utc::now();
        let allowed = match row {
            None => {
                sqlx::query("INSERT INTO site_auth_attempts (scope, subject_hash, window_started_at, attempts) VALUES ($1, $2, $3, 1)")
                    .bind(scope)
                    .bind(subject_hash)
                    .bind(now)
                    .execute(&mut *transaction)
                    .await?;
                true
            }
            Some(row) => {
                let window_started_at: DateTime<Utc> = row.try_get("window_started_at")?;
                let attempts: i32 = row.try_get("attempts")?;
                let blocked_until: Option<DateTime<Utc>> = row.try_get("blocked_until")?;
                if blocked_until.is_some_and(|until| until > now) {
                    false
                } else if window_started_at + chrono::Duration::seconds(window_seconds) <= now {
                    sqlx::query("UPDATE site_auth_attempts SET window_started_at = $3, attempts = 1, blocked_until = NULL, updated_at = $3 WHERE scope = $1 AND subject_hash = $2")
                        .bind(scope)
                        .bind(subject_hash)
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                    true
                } else if attempts >= maximum_attempts {
                    sqlx::query("UPDATE site_auth_attempts SET attempts = attempts + 1, blocked_until = $3, updated_at = $4 WHERE scope = $1 AND subject_hash = $2")
                        .bind(scope)
                        .bind(subject_hash)
                        .bind(now + chrono::Duration::seconds(block_seconds))
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                    false
                } else {
                    sqlx::query("UPDATE site_auth_attempts SET attempts = attempts + 1, updated_at = $3 WHERE scope = $1 AND subject_hash = $2")
                        .bind(scope)
                        .bind(subject_hash)
                        .bind(now)
                        .execute(&mut *transaction)
                        .await?;
                    true
                }
            }
        };
        transaction.commit().await?;
        Ok(allowed)
    }

    pub async fn insert_site_auth_email_action(
        &self,
        token_hash: &str,
        purpose: &str,
        email: &str,
        email_key: &str,
        account_key: Option<&str>,
        expires_at: DateTime<Utc>,
        idempotency_key: &str,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO site_auth_email_actions
              (token_hash, purpose, email, email_key, account_key, expires_at, idempotency_key)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(token_hash)
        .bind(purpose)
        .bind(email)
        .bind(email_key)
        .bind(account_key)
        .bind(expires_at)
        .bind(idempotency_key)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_site_auth_email_delivered(
        &self,
        token_hash: &str,
        delivery_id: &str,
    ) -> DbResult<()> {
        sqlx::query("UPDATE site_auth_email_actions SET delivery_id = $2 WHERE token_hash = $1")
            .bind(token_hash)
            .bind(delivery_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn verify_site_auth_email_action(
        &self,
        token_hash: &str,
        purpose: &str,
        setup_hash: &str,
    ) -> DbResult<Option<SiteAuthEmailAction>> {
        let row = sqlx::query(
            r#"
            UPDATE site_auth_email_actions SET verified_at = NOW(), setup_hash = $3
            WHERE token_hash = $1 AND purpose = $2 AND expires_at > NOW()
              AND verified_at IS NULL AND consumed_at IS NULL
            RETURNING token_hash, purpose, email, email_key, account_key, expires_at, verified_at, consumed_at
            "#,
        )
        .bind(token_hash)
        .bind(purpose)
        .bind(setup_hash)
        .fetch_optional(&self.pool)
        .await?;
        row.map(site_auth_email_action_from_row).transpose()
    }

    pub async fn site_auth_password_record(
        &self,
        email_key: &str,
    ) -> DbResult<Option<SiteAuthPasswordRecord>> {
        let row = sqlx::query(
            r#"
            SELECT c.account_key, c.password_phc, c.enabled
            FROM site_auth_verified_emails e
            JOIN site_auth_password_credentials c ON c.account_key = e.account_key
            WHERE e.email_key = $1
            "#,
        )
        .bind(email_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(SiteAuthPasswordRecord {
                account_key: row.try_get("account_key")?,
                password_phc: row.try_get("password_phc")?,
                enabled: row.try_get("enabled")?,
            })
        })
        .transpose()
    }

    pub async fn site_auth_account_for_verified_email(
        &self,
        email_key: &str,
    ) -> DbResult<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT account_key FROM site_auth_verified_emails WHERE email_key = $1",
        )
        .bind(email_key)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn complete_site_auth_password_action(
        &self,
        setup_hash: &str,
        purpose: &str,
        proposed_account_key: &str,
        display_name: &str,
        password_phc: &str,
    ) -> DbResult<Option<String>> {
        let mut transaction = self.pool.begin().await?;
        let action = sqlx::query(
            r#"
            SELECT email, email_key, account_key
            FROM site_auth_email_actions
            WHERE setup_hash = $1 AND purpose = $2 AND verified_at IS NOT NULL
              AND consumed_at IS NULL AND expires_at > NOW()
            FOR UPDATE
            "#,
        )
        .bind(setup_hash)
        .bind(purpose)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(action) = action else {
            transaction.rollback().await?;
            return Ok(None);
        };
        let email: String = action.try_get("email")?;
        let email_key: String = action.try_get("email_key")?;
        let action_account: Option<String> = action.try_get("account_key")?;
        let verified_owner: Option<String> = sqlx::query_scalar(
            "SELECT account_key FROM site_auth_verified_emails WHERE email_key = $1 FOR UPDATE",
        )
        .bind(&email_key)
        .fetch_optional(&mut *transaction)
        .await?;
        let account_key = action_account
            .or(verified_owner)
            .unwrap_or_else(|| proposed_account_key.to_string());
        let insert_display_name = if display_name.is_empty() {
            "Agent Bounties account"
        } else {
            display_name
        };

        sqlx::query(
            r#"
            INSERT INTO site_auth_accounts
              (account_key, provider, provider_subject, display_name, email, avatar_url)
            VALUES ($1, 'password', $2, $3, $4, '')
            ON CONFLICT (account_key) DO UPDATE SET
              display_name = CASE WHEN $5 THEN site_auth_accounts.display_name ELSE EXCLUDED.display_name END,
              email = $4,
              last_signed_in_at = NOW()
            "#,
        )
        .bind(&account_key)
        .bind(&email_key)
        .bind(insert_display_name)
        .bind(&email)
        .bind(display_name.is_empty())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO site_auth_verified_emails (email_key, email, account_key)
            VALUES ($1, $2, $3)
            ON CONFLICT (email_key) DO UPDATE SET email = EXCLUDED.email
            "#,
        )
        .bind(&email_key)
        .bind(&email)
        .bind(&account_key)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO site_auth_password_credentials (account_key, password_phc)
            VALUES ($1, $2)
            ON CONFLICT (account_key) DO UPDATE SET
              password_phc = EXCLUDED.password_phc, enabled = TRUE, changed_at = NOW()
            "#,
        )
        .bind(&account_key)
        .bind(password_phc)
        .execute(&mut *transaction)
        .await?;
        if purpose == "registration" {
            sqlx::query(
                r#"
                INSERT INTO site_auth_identities
                  (provider, provider_subject, account_key, verified_email, verified_email_key, email_verified_at)
                VALUES ('password', $1, $2, $3, $1, NOW())
                ON CONFLICT (provider, provider_subject) DO UPDATE SET
                  account_key = EXCLUDED.account_key, verified_email = EXCLUDED.verified_email,
                  verified_email_key = EXCLUDED.verified_email_key,
                  email_verified_at = EXCLUDED.email_verified_at, last_signed_in_at = NOW()
                "#,
            )
            .bind(&email_key)
            .bind(&account_key)
            .bind(&email)
            .execute(&mut *transaction)
            .await?;
        } else {
            sqlx::query(
                "UPDATE site_auth_sessions SET revoked_at = NOW() WHERE account_key = $1 AND revoked_at IS NULL",
            )
            .bind(&account_key)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query(
            "UPDATE site_auth_email_actions SET consumed_at = NOW(), account_key = $2 WHERE setup_hash = $1",
        )
        .bind(setup_hash)
        .bind(&account_key)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Some(account_key))
    }

    pub async fn list_site_auth_wallets(&self, account_key: &str) -> DbResult<Vec<SiteAuthWallet>> {
        let rows = sqlx::query(
            r#"
            SELECT wallet_address, chain_id, linked_at, proof_method
            FROM site_auth_wallets
            WHERE account_key = $1
            ORDER BY linked_at ASC
            "#,
        )
        .bind(account_key)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(SiteAuthWallet {
                    address: row.try_get("wallet_address")?,
                    chain_id: row.try_get("chain_id")?,
                    linked_at: row.try_get("linked_at")?,
                    proof: row.try_get("proof_method")?,
                })
            })
            .collect()
    }

    pub async fn link_site_auth_wallet(
        &self,
        account_key: &str,
        wallet_address: &str,
        chain_id: i64,
    ) -> DbResult<Vec<SiteAuthWallet>> {
        let mut transaction = self.pool.begin().await?;
        let account_exists: Option<String> = sqlx::query_scalar(
            "SELECT account_key FROM site_auth_accounts WHERE account_key = $1 FOR UPDATE",
        )
        .bind(account_key)
        .fetch_optional(&mut *transaction)
        .await?;
        if account_exists.is_none() {
            return Err(DbError::SiteAuthConflict(
                "site_auth_account_unavailable".to_string(),
            ));
        }
        let linked_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM site_auth_wallets WHERE account_key = $1")
                .bind(account_key)
                .fetch_one(&mut *transaction)
                .await?;
        let current_owner: Option<String> = sqlx::query_scalar(
            "SELECT account_key FROM site_auth_wallets WHERE wallet_address = lower($1)",
        )
        .bind(wallet_address)
        .fetch_optional(&mut *transaction)
        .await?;
        if current_owner
            .as_deref()
            .is_some_and(|owner| owner != account_key)
        {
            return Err(DbError::SiteAuthConflict(
                "wallet_linked_to_another_account".to_string(),
            ));
        }
        if current_owner.is_none() && linked_count >= 8 {
            return Err(DbError::SiteAuthConflict(
                "wallet_limit_reached".to_string(),
            ));
        }
        let inserted_owner: String = sqlx::query_scalar(
            r#"
            INSERT INTO site_auth_wallets (wallet_address, account_key, chain_id)
            VALUES (lower($1), $2, $3)
            ON CONFLICT (wallet_address) DO UPDATE SET
              wallet_address = EXCLUDED.wallet_address
            RETURNING account_key
            "#,
        )
        .bind(wallet_address)
        .bind(account_key)
        .bind(chain_id)
        .fetch_one(&mut *transaction)
        .await?;
        if inserted_owner != account_key {
            return Err(DbError::SiteAuthConflict(
                "wallet_linked_to_another_account".to_string(),
            ));
        }
        transaction.commit().await?;
        self.list_site_auth_wallets(account_key).await
    }

    pub async fn unlink_site_auth_wallet(
        &self,
        account_key: &str,
        wallet_address: &str,
    ) -> DbResult<Vec<SiteAuthWallet>> {
        sqlx::query(
            "DELETE FROM site_auth_wallets WHERE account_key = $1 AND wallet_address = lower($2)",
        )
        .bind(account_key)
        .bind(wallet_address)
        .execute(&self.pool)
        .await?;
        self.list_site_auth_wallets(account_key).await
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

    pub async fn record_interface_usage(
        &self,
        interface: ObservedInterface,
        protocol_era: ObservedProtocolEra,
        succeeded: bool,
        observed_at: DateTime<Utc>,
    ) -> DbResult<()> {
        let valid_pair = matches!(
            (interface, protocol_era),
            (
                ObservedInterface::Api | ObservedInterface::Cli,
                ObservedProtocolEra::NotApplicable
            ) | (
                ObservedInterface::Mcp,
                ObservedProtocolEra::McpLegacy
                    | ObservedProtocolEra::McpModern
                    | ObservedProtocolEra::McpHttpAdapter
            )
        );
        if !valid_pair {
            return Err(DbError::InvalidEnum(
                "interface and protocol era do not form a valid attribution pair".to_string(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO external_interface_usage_hourly
              (bucket_started_at, interface, protocol_era, request_count,
               successful_request_count, first_observed_at, last_observed_at)
            VALUES (
              date_trunc('hour', $1 AT TIME ZONE 'UTC') AT TIME ZONE 'UTC',
              $2, $3, 1, CASE WHEN $4 THEN 1 ELSE 0 END, $1, $1
            )
            ON CONFLICT (bucket_started_at, interface, protocol_era) DO UPDATE SET
              request_count = external_interface_usage_hourly.request_count + 1,
              successful_request_count = external_interface_usage_hourly.successful_request_count
                + CASE WHEN $4 THEN 1 ELSE 0 END,
              first_observed_at = LEAST(external_interface_usage_hourly.first_observed_at, EXCLUDED.first_observed_at),
              last_observed_at = GREATEST(external_interface_usage_hourly.last_observed_at, EXCLUDED.last_observed_at)
            "#,
        )
        .bind(observed_at)
        .bind(interface.as_str())
        .bind(protocol_era.as_str())
        .bind(succeeded)
        .execute(&self.pool)
        .await?;
        Ok(())
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

        let interface_rows = sqlx::query(
            r#"
            SELECT interface, protocol_era,
                   SUM(request_count)::BIGINT AS request_count,
                   SUM(successful_request_count)::BIGINT AS successful_request_count,
                   MIN(first_observed_at) AS first_observed_at,
                   MAX(last_observed_at) AS last_observed_at
            FROM external_interface_usage_hourly
            WHERE bucket_started_at >= (
                    date_trunc('hour', $1 AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                  )
              AND bucket_started_at <= (
                    date_trunc('hour', NOW() AT TIME ZONE 'UTC') AT TIME ZONE 'UTC'
                  )
            GROUP BY interface, protocol_era
            ORDER BY request_count DESC, interface, protocol_era
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
            interfaces: interface_rows
                .into_iter()
                .map(|row| {
                    Ok(InterfaceUsageStats {
                        interface: row.try_get("interface")?,
                        protocol_era: row.try_get("protocol_era")?,
                        request_count: u64_from_i64(row.try_get("request_count")?)?,
                        successful_request_count: u64_from_i64(
                            row.try_get("successful_request_count")?,
                        )?,
                        first_observed_at: row.try_get("first_observed_at")?,
                        last_observed_at: row.try_get("last_observed_at")?,
                    })
                })
                .collect::<DbResult<Vec<_>>>()?,
        })
    }

    pub async fn platform_demand_growth_stats(
        &self,
        network: &str,
        ended_at: DateTime<Utc>,
        launch_at: DateTime<Utc>,
        excluded_wallets: &[String],
        excluded_bounty_contracts: &[String],
    ) -> DbResult<PlatformDemandGrowthStats> {
        let row = sqlx::query(
            r#"
            WITH supply_actions AS (
              SELECT event.occurred_at,
                     lower(CASE
                       WHEN event.kind = 'canonical_bounty_created' THEN event.data->>'creator'
                       WHEN event.kind = 'external_bounty_submitted' THEN event.data->>'submitter'
                       ELSE event.data->>'contributor'
                     END) AS identity,
                     'autonomous:' || event.id::text AS action_key
              FROM autonomous_bounty_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($5)
                AND event.kind IN ('canonical_bounty_created', 'external_bounty_submitted', 'funding_added')
              UNION ALL
              SELECT event.occurred_at,
                     lower(CASE WHEN event.kind = 'canonical_competition_created'
                       THEN event.data->>'creator' ELSE event.data->>'contributor' END) AS identity,
                     'open-v1:' || event.id::text AS action_key
              FROM open_competition_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($5)
                AND event.kind IN ('canonical_competition_created', 'funding_added')
              UNION ALL
              SELECT event.occurred_at,
                     lower(CASE WHEN event.kind = 'canonical_competition_created'
                       THEN event.data->>'creator' ELSE event.data->>'contributor' END) AS identity,
                     'open-v2:' || event.id::text AS action_key
              FROM open_competition_v2_events AS event
              WHERE event.network = $1
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND NOT lower(event.contract_address) = ANY($5)
                AND event.kind IN ('canonical_competition_created', 'funding_added')
            ), external_supply_actions AS (
              SELECT * FROM supply_actions
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND identity <> '0x0000000000000000000000000000000000000000'
                AND NOT identity = ANY($4)
            ), wallet_rollup AS (
              SELECT identity,
                     MIN(occurred_at) AS first_action_at,
                     COUNT(DISTINCT action_key) FILTER (
                       WHERE occurred_at >= $2 - INTERVAL '28 days'
                     ) AS actions_28d
              FROM external_supply_actions
              GROUP BY identity
            ), funding AS (
              SELECT 'autonomous'::text AS protocol, event.contract_address,
                     event.bounty_id, event.occurred_at,
                     lower(event.data->>'contributor') AS contributor,
                     COALESCE((event.data->>'amount')::numeric, 0) AS amount
              FROM autonomous_bounty_events AS event
              WHERE event.network = $1 AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'funding_added'
                AND NOT lower(event.contract_address) = ANY($5)
              UNION ALL
              SELECT 'open-v1', event.contract_address, event.bounty_id, event.occurred_at,
                     lower(event.data->>'contributor'), COALESCE((event.data->>'amount')::numeric, 0)
              FROM open_competition_events AS event
              WHERE event.network = $1 AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'funding_added'
                AND NOT lower(event.contract_address) = ANY($5)
              UNION ALL
              SELECT 'open-v2', event.contract_address, event.bounty_id, event.occurred_at,
                     lower(event.data->>'contributor'), COALESCE((event.data->>'amount')::numeric, 0)
              FROM open_competition_v2_events AS event
              WHERE event.network = $1
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'funding_added'
                AND NOT lower(event.contract_address) = ANY($5)
            ), settlements AS (
              SELECT 'autonomous'::text AS protocol, event.contract_address,
                     event.bounty_id, event.occurred_at,
                     COALESCE((event.data->>'solver_reward')::numeric, 0)
                     + COALESCE((event.data->>'verifier_reward')::numeric, 0)
                     + COALESCE((event.data->>'timeout_bond_bonus')::numeric, 0) AS gmv
              FROM autonomous_bounty_events AS event
              WHERE event.network = $1 AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'bounty_settled'
                AND NOT lower(event.contract_address) = ANY($5)
              UNION ALL
              SELECT 'open-v1', event.contract_address, event.bounty_id, event.occurred_at,
                     COALESCE((event.data->>'solver_reward')::numeric, 0)
                     + COALESCE((event.data->>'verifier_reward')::numeric, 0)
                     + COALESCE((event.data->>'timeout_bond_bonus')::numeric, 0)
              FROM open_competition_events AS event
              WHERE event.network = $1 AND event.block_time_verified = TRUE
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'bounty_settled'
                AND NOT lower(event.contract_address) = ANY($5)
              UNION ALL
              SELECT 'open-v2', event.contract_address, event.bounty_id, event.occurred_at,
                     COALESCE((event.data->>'solver_reward')::numeric, 0)
                     + COALESCE((event.data->>'keeper_reward')::numeric, 0)
              FROM open_competition_v2_events AS event
              WHERE event.network = $1
                AND event.occurred_at >= $3 AND event.occurred_at < $2
                AND event.kind = 'competition_settled'
                AND NOT lower(event.contract_address) = ANY($5)
            ), attributed AS (
              SELECT settlement.*,
                     COALESCE(SUM(funding.amount), 0) AS total_funding,
                     COALESCE(SUM(funding.amount) FILTER (
                       WHERE funding.contributor ~ '^0x[0-9a-f]{40}$'
                         AND funding.contributor <> '0x0000000000000000000000000000000000000000'
                         AND NOT funding.contributor = ANY($4)
                     ), 0) AS non_operator_funding
              FROM settlements AS settlement
              LEFT JOIN funding
                ON funding.protocol = settlement.protocol
               AND lower(funding.contract_address) = lower(settlement.contract_address)
               AND funding.bounty_id = settlement.bounty_id
               AND funding.occurred_at <= settlement.occurred_at
              GROUP BY settlement.protocol, settlement.contract_address,
                       settlement.bounty_id, settlement.occurred_at, settlement.gmv
            )
            SELECT
              COALESCE((SELECT SUM(gmv) FROM attributed
                WHERE occurred_at >= $2 - INTERVAL '7 days'), 0)::text AS gmv_7d,
              COALESCE((SELECT SUM(gmv) FROM attributed
                WHERE occurred_at >= $2 - INTERVAL '28 days'), 0)::text AS gmv_28d,
              COALESCE((SELECT SUM(gmv) FROM attributed), 0)::text AS lifetime_gmv,
              (SELECT COUNT(*) FROM wallet_rollup
                WHERE actions_28d > 0 AND first_action_at >= $2 - INTERVAL '28 days') AS new_wallets_28d,
              (SELECT COUNT(*) FROM wallet_rollup WHERE actions_28d > 0) AS active_wallets_28d,
              (SELECT COUNT(*) FROM wallet_rollup WHERE actions_28d >= 2) AS repeat_wallets_28d,
              TRUNC(COALESCE((SELECT SUM(gmv * non_operator_funding / total_funding)
                FROM attributed
                WHERE occurred_at >= $2 - INTERVAL '28 days' AND total_funding > 0), 0))::text
                AS non_operator_attributed_gmv_28d,
              COALESCE((SELECT SUM(gmv)
                FROM attributed
                WHERE occurred_at >= $2 - INTERVAL '28 days' AND total_funding > 0), 0)::text
                AS attributed_gmv_28d
            "#,
        )
        .bind(network)
        .bind(ended_at)
        .bind(launch_at)
        .bind(excluded_wallets)
        .bind(excluded_bounty_contracts)
        .fetch_one(&self.pool)
        .await?;
        Ok(PlatformDemandGrowthStats {
            gmv_7d_base_units: row.try_get("gmv_7d")?,
            gmv_28d_base_units: row.try_get("gmv_28d")?,
            lifetime_gmv_base_units: row.try_get("lifetime_gmv")?,
            new_poster_funder_wallets_28d: u64_from_i64(row.try_get("new_wallets_28d")?)?,
            active_poster_funder_wallets_28d: u64_from_i64(row.try_get("active_wallets_28d")?)?,
            repeat_poster_funder_wallets_28d: u64_from_i64(row.try_get("repeat_wallets_28d")?)?,
            non_operator_attributed_gmv_28d_base_units: row
                .try_get("non_operator_attributed_gmv_28d")?,
            attributed_gmv_28d_base_units: row.try_get("attributed_gmv_28d")?,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn platform_metrics_stats(
        &self,
        network: &str,
        selected_started_at: DateTime<Utc>,
        selected_ended_at: DateTime<Utc>,
        previous_started_at: DateTime<Utc>,
        launch_at: DateTime<Utc>,
        first_month_ended_at: DateTime<Utc>,
        excluded_wallets: &[String],
        excluded_comment_authors: &[String],
        excluded_bounty_contracts: &[String],
    ) -> DbResult<PlatformMetricsStats> {
        let identity_row = sqlx::query(
            r#"
            WITH event_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_bounty_created' THEN event.data->>'creator'
                       WHEN event.kind = 'external_bounty_submitted' THEN event.data->>'submitter'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity,
                     CASE
                       WHEN event.kind IN ('canonical_bounty_created', 'external_bounty_submitted') THEN 'poster'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn') THEN 'funder'
                       ELSE 'solver'
                     END AS role
              FROM autonomous_bounty_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $5
                AND event.occurred_at < $3
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($9)
                AND event.kind IN (
                  'canonical_bounty_created', 'external_bounty_submitted', 'funding_added',
                  'bounty_claimed', 'submission_added', 'submission_rejected',
                  'bounty_settled', 'claim_expired', 'submission_expired',
                  'refund_withdrawn'
                )
            ), competition_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_competition_created'
                         THEN event.data->>'creator'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity,
                     CASE
                       WHEN event.kind = 'canonical_competition_created' THEN 'poster'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn') THEN 'funder'
                       ELSE 'solver'
                     END AS role
              FROM open_competition_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $5
                AND event.occurred_at < $3
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($9)
                AND event.kind IN (
                  'canonical_competition_created', 'funding_added',
                  'solution_committed', 'solution_revealed',
                  'competition_submission_rejected', 'commitment_expired',
                  'bounty_settled', 'entry_bond_withdrawn', 'refund_withdrawn'
                )
            ), competition_v2_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_competition_created'
                         THEN event.data->>'creator'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity,
                     CASE
                       WHEN event.kind = 'canonical_competition_created' THEN 'poster'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn') THEN 'funder'
                       ELSE 'solver'
                     END AS role
              FROM open_competition_v2_events AS event
              WHERE event.network = $1
                AND event.occurred_at >= $5
                AND event.occurred_at < $3
                AND NOT lower(event.contract_address) = ANY($9)
                AND event.kind IN (
                  'canonical_competition_created', 'funding_added',
                  'entry_qualified', 'competition_settled', 'refund_withdrawn'
                )
            ), verifier_actors AS (
              SELECT payout.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(configuration.data->>'verifier_reward_recipient') AS identity,
                     'verifier'::text AS role
              FROM autonomous_bounty_events AS payout
              JOIN LATERAL (
                SELECT configured.data
                FROM autonomous_bounty_events AS configured
                WHERE configured.network = payout.network
                  AND configured.bounty_id = payout.bounty_id
                  AND configured.contract_address = payout.contract_address
                  AND configured.block_time_verified = TRUE
                  AND configured.kind = 'canonical_bounty_verification_configured'
                ORDER BY configured.block_number, configured.log_index
                LIMIT 1
              ) AS configuration ON TRUE
              WHERE payout.network = $1
                AND payout.block_time_verified = TRUE
                AND payout.occurred_at >= $5
                AND payout.occurred_at < $3
                AND NOT lower(payout.contract_address) = ANY($9)
                AND payout.kind IN ('submission_rejected', 'bounty_settled')
                AND COALESCE((payout.data->>'verifier_reward')::numeric, 0) > 0
            ), competition_verifier_actors AS (
              SELECT payout.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(configuration.data->>'verifier_reward_recipient') AS identity,
                     'verifier'::text AS role
              FROM open_competition_events AS payout
              JOIN LATERAL (
                SELECT configured.data
                FROM open_competition_events AS configured
                WHERE configured.network = payout.network
                  AND configured.bounty_id = payout.bounty_id
                  AND configured.contract_address = payout.contract_address
                  AND configured.block_time_verified = TRUE
                  AND configured.kind = 'canonical_competition_verification_configured'
                ORDER BY configured.block_number, configured.log_index
                LIMIT 1
              ) AS configuration ON TRUE
              WHERE payout.network = $1
                AND payout.block_time_verified = TRUE
                AND payout.occurred_at >= $5
                AND payout.occurred_at < $3
                AND NOT lower(payout.contract_address) = ANY($9)
                AND payout.kind IN ('competition_submission_rejected', 'bounty_settled')
                AND CASE WHEN payout.kind = 'bounty_settled'
                      THEN COALESCE((payout.data->>'verifier_reward')::numeric, 0)
                      ELSE COALESCE((payout.data->>'bond_paid_to_verifier')::numeric, 0)
                    END > 0
            ), comment_actors AS (
              SELECT comment.created_at AS occurred_at,
                     'opportunity_comment_author'::text AS namespace,
                     lower(regexp_replace(trim(comment.author), '\s+', ' ', 'g')) AS identity,
                     'commenter'::text AS role
              FROM opportunity_comments AS comment
              WHERE comment.created_at >= $5 AND comment.created_at < $3
            ), actors AS (
              SELECT occurred_at, namespace, identity, role
              FROM event_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND NOT identity = ANY($7)
              UNION ALL
              SELECT occurred_at, namespace, identity, role
              FROM competition_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND NOT identity = ANY($7)
              UNION ALL
              SELECT occurred_at, namespace, identity, role
              FROM competition_v2_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND NOT identity = ANY($7)
              UNION ALL
              SELECT occurred_at, namespace, identity, role
              FROM verifier_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND identity <> '0x0000000000000000000000000000000000000000'
                AND NOT identity = ANY($7)
              UNION ALL
              SELECT occurred_at, namespace, identity, role
              FROM competition_verifier_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND identity <> '0x0000000000000000000000000000000000000000'
                AND NOT identity = ANY($7)
              UNION ALL
              SELECT occurred_at, namespace, identity, role
              FROM comment_actors
              WHERE identity <> '' AND NOT identity = ANY($8)
            )
            SELECT
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ) AS selected,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $4 AND occurred_at < $2
              ) AS previous,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $3 - INTERVAL '7 days' AND occurred_at < $3
              ) AS latest_week,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $3 - INTERVAL '14 days'
                  AND occurred_at < $3 - INTERVAL '7 days'
              ) AS previous_week,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $5 AND occurred_at < $6
              ) AS first_month,
              COUNT(DISTINCT (namespace, identity)) AS lifetime,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND role = 'poster'
              ) AS posters,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND role = 'funder'
              ) AS funders,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND role = 'solver'
              ) AS solvers,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND role = 'verifier'
              ) AS verifiers,
              COUNT(DISTINCT (namespace, identity)) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND role = 'commenter'
              ) AS commenters,
              COUNT(DISTINCT identity) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND namespace = 'base_wallet'
              ) AS marketplace_wallets,
              COUNT(DISTINCT identity) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
                  AND namespace = 'opportunity_comment_author'
              ) AS opportunity_comment_authors
            FROM actors
            "#,
        )
        .bind(network)
        .bind(selected_started_at)
        .bind(selected_ended_at)
        .bind(previous_started_at)
        .bind(launch_at)
        .bind(first_month_ended_at)
        .bind(excluded_wallets)
        .bind(excluded_comment_authors)
        .bind(excluded_bounty_contracts)
        .fetch_one(&self.pool)
        .await?;

        let payout_row = sqlx::query(
            r#"
            WITH payouts AS (
              SELECT occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0)
                       ELSE 0 END AS solver_amount,
                     COALESCE((data->>'verifier_reward')::numeric, 0) AS verifier_amount,
                     0::numeric AS keeper_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0)
                       ELSE 0 END AS bonus_amount
              FROM autonomous_bounty_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND occurred_at >= $5
                AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($7)
                AND kind IN ('bounty_settled', 'submission_rejected')
              UNION ALL
              SELECT occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0)
                       ELSE 0 END AS solver_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'verifier_reward')::numeric, 0)
                       ELSE COALESCE((data->>'bond_paid_to_verifier')::numeric, 0)
                     END AS verifier_amount,
                     0::numeric AS keeper_amount,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0)
                       ELSE 0 END AS bonus_amount
              FROM open_competition_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND occurred_at >= $5
                AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($7)
                AND kind IN ('bounty_settled', 'competition_submission_rejected')
              UNION ALL
              SELECT occurred_at, TRUE AS settled,
                     COALESCE((data->>'solver_reward')::numeric, 0) AS solver_amount,
                     0::numeric AS verifier_amount,
                     COALESCE((data->>'keeper_reward')::numeric, 0) AS keeper_amount,
                     0::numeric AS bonus_amount
              FROM open_competition_v2_events
              WHERE network = $1
                AND occurred_at >= $5
                AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($7)
                AND kind = 'competition_settled'
            ), normalized AS (
              SELECT *, solver_amount + verifier_amount + keeper_amount + bonus_amount AS total_amount
              FROM payouts
            )
            SELECT
              COALESCE(SUM(total_amount) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ), 0)::text AS selected_total,
              COALESCE(SUM(total_amount) FILTER (
                WHERE occurred_at >= $4 AND occurred_at < $2
              ), 0)::text AS previous_total,
              COALESCE(SUM(total_amount) FILTER (
                WHERE occurred_at >= $5 AND occurred_at < $6
              ), 0)::text AS first_month_total,
              COALESCE(SUM(total_amount), 0)::text AS lifetime_total,
              COALESCE(SUM(solver_amount) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ), 0)::text AS selected_solver,
              COALESCE(SUM(verifier_amount) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ), 0)::text AS selected_verifier,
              COALESCE(SUM(keeper_amount) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ), 0)::text AS selected_keeper,
              COALESCE(SUM(bonus_amount) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3
              ), 0)::text AS selected_bonus,
              COUNT(*) FILTER (
                WHERE occurred_at >= $2 AND occurred_at < $3 AND settled
              ) AS selected_settled,
              COUNT(*) FILTER (
                WHERE occurred_at >= $4 AND occurred_at < $2 AND settled
              ) AS previous_settled,
              COUNT(*) FILTER (
                WHERE occurred_at >= $5 AND occurred_at < $6 AND settled
              ) AS first_month_settled,
              COUNT(*) FILTER (WHERE settled) AS lifetime_settled
            FROM normalized
            "#,
        )
        .bind(network)
        .bind(selected_started_at)
        .bind(selected_ended_at)
        .bind(previous_started_at)
        .bind(launch_at)
        .bind(first_month_ended_at)
        .bind(excluded_bounty_contracts)
        .fetch_one(&self.pool)
        .await?;

        let cohort_row = sqlx::query(
            r#"
            WITH claim_events AS (
              SELECT network, bounty_id, (data->>'round')::bigint AS round,
                     (data->>'claim_expires_at')::bigint AS claim_expires_at
              FROM autonomous_bounty_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND kind = 'bounty_claimed'
                AND occurred_at >= $2
                AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($4)
            ), claims AS (
              SELECT network, bounty_id, round, MAX(claim_expires_at) AS claim_expires_at
              FROM claim_events
              GROUP BY network, bounty_id, round
            ), terminal AS (
              SELECT network, bounty_id, (data->>'round')::bigint AS round,
                     BOOL_OR(kind = 'bounty_settled') AS settled
              FROM autonomous_bounty_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($4)
                AND kind IN (
                  'bounty_settled', 'submission_rejected',
                  'claim_expired', 'submission_expired'
                )
              GROUP BY network, bounty_id, (data->>'round')::bigint
            ), evaluated AS (
              SELECT claims.*,
                     terminal.round IS NOT NULL AS has_terminal,
                     COALESCE(terminal.settled, FALSE) AS settled,
                     claims.claim_expires_at <= EXTRACT(EPOCH FROM $3::timestamptz)::bigint
                       OR terminal.round IS NOT NULL AS mature
              FROM claims
              LEFT JOIN terminal USING (network, bounty_id, round)
            )
            SELECT
              COUNT(*) FILTER (WHERE mature AND settled) AS settled,
              COUNT(*) FILTER (WHERE mature) AS mature,
              COUNT(*) FILTER (WHERE NOT mature) AS immature
            FROM evaluated
            "#,
        )
        .bind(network)
        .bind(selected_started_at)
        .bind(selected_ended_at)
        .bind(excluded_bounty_contracts)
        .fetch_one(&self.pool)
        .await?;

        let daily_rows = sqlx::query(
            r#"
            WITH event_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_bounty_created' THEN event.data->>'creator'
                       WHEN event.kind = 'external_bounty_submitted' THEN event.data->>'submitter'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity
              FROM autonomous_bounty_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $2 AND event.occurred_at < $3
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($6)
                AND event.kind IN (
                  'canonical_bounty_created', 'external_bounty_submitted', 'funding_added',
                  'bounty_claimed', 'submission_added', 'submission_rejected',
                  'bounty_settled', 'claim_expired', 'submission_expired',
                  'refund_withdrawn'
                )
            ), competition_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_competition_created'
                         THEN event.data->>'creator'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity
              FROM open_competition_events AS event
              WHERE event.network = $1
                AND event.block_time_verified = TRUE
                AND event.occurred_at >= $2 AND event.occurred_at < $3
                AND NOT lower(COALESCE(event.data->>'bounty_contract', event.contract_address)) = ANY($6)
                AND event.kind IN (
                  'canonical_competition_created', 'funding_added',
                  'solution_committed', 'solution_revealed',
                  'competition_submission_rejected', 'commitment_expired',
                  'bounty_settled', 'entry_bond_withdrawn', 'refund_withdrawn'
                )
            ), competition_v2_actors AS (
              SELECT event.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(CASE
                       WHEN event.kind = 'canonical_competition_created'
                         THEN event.data->>'creator'
                       WHEN event.kind IN ('funding_added', 'refund_withdrawn')
                         THEN event.data->>'contributor'
                       ELSE event.data->>'solver'
                     END) AS identity
              FROM open_competition_v2_events AS event
              WHERE event.network = $1
                AND event.occurred_at >= $2 AND event.occurred_at < $3
                AND NOT lower(event.contract_address) = ANY($6)
                AND event.kind IN (
                  'canonical_competition_created', 'funding_added',
                  'entry_qualified', 'competition_settled', 'refund_withdrawn'
                )
            ), verifier_actors AS (
              SELECT payout.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(configuration.data->>'verifier_reward_recipient') AS identity
              FROM autonomous_bounty_events AS payout
              JOIN LATERAL (
                SELECT configured.data
                FROM autonomous_bounty_events AS configured
                WHERE configured.network = payout.network
                  AND configured.bounty_id = payout.bounty_id
                  AND configured.contract_address = payout.contract_address
                  AND configured.block_time_verified = TRUE
                  AND configured.kind = 'canonical_bounty_verification_configured'
                ORDER BY configured.block_number, configured.log_index
                LIMIT 1
              ) AS configuration ON TRUE
              WHERE payout.network = $1
                AND payout.block_time_verified = TRUE
                AND payout.occurred_at >= $2 AND payout.occurred_at < $3
                AND NOT lower(payout.contract_address) = ANY($6)
                AND payout.kind IN ('submission_rejected', 'bounty_settled')
                AND COALESCE((payout.data->>'verifier_reward')::numeric, 0) > 0
            ), competition_verifier_actors AS (
              SELECT payout.occurred_at,
                     'base_wallet'::text AS namespace,
                     lower(configuration.data->>'verifier_reward_recipient') AS identity
              FROM open_competition_events AS payout
              JOIN LATERAL (
                SELECT configured.data
                FROM open_competition_events AS configured
                WHERE configured.network = payout.network
                  AND configured.bounty_id = payout.bounty_id
                  AND configured.contract_address = payout.contract_address
                  AND configured.block_time_verified = TRUE
                  AND configured.kind = 'canonical_competition_verification_configured'
                ORDER BY configured.block_number, configured.log_index
                LIMIT 1
              ) AS configuration ON TRUE
              WHERE payout.network = $1
                AND payout.block_time_verified = TRUE
                AND payout.occurred_at >= $2 AND payout.occurred_at < $3
                AND NOT lower(payout.contract_address) = ANY($6)
                AND payout.kind IN ('competition_submission_rejected', 'bounty_settled')
                AND CASE WHEN payout.kind = 'bounty_settled'
                      THEN COALESCE((payout.data->>'verifier_reward')::numeric, 0)
                      ELSE COALESCE((payout.data->>'bond_paid_to_verifier')::numeric, 0)
                    END > 0
            ), comment_actors AS (
              SELECT created_at AS occurred_at,
                     'opportunity_comment_author'::text AS namespace,
                     lower(regexp_replace(trim(author), '\s+', ' ', 'g')) AS identity
              FROM opportunity_comments
              WHERE created_at >= $2 AND created_at < $3
            ), actors AS (
              SELECT occurred_at, namespace, identity FROM event_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$' AND NOT identity = ANY($4)
              UNION ALL
              SELECT occurred_at, namespace, identity FROM competition_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$' AND NOT identity = ANY($4)
              UNION ALL
              SELECT occurred_at, namespace, identity FROM competition_v2_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$' AND NOT identity = ANY($4)
              UNION ALL
              SELECT occurred_at, namespace, identity FROM verifier_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND identity <> '0x0000000000000000000000000000000000000000'
                AND NOT identity = ANY($4)
              UNION ALL
              SELECT occurred_at, namespace, identity FROM competition_verifier_actors
              WHERE identity ~ '^0x[0-9a-f]{40}$'
                AND identity <> '0x0000000000000000000000000000000000000000'
                AND NOT identity = ANY($4)
              UNION ALL
              SELECT occurred_at, namespace, identity FROM comment_actors
              WHERE identity <> '' AND NOT identity = ANY($5)
            ), payouts AS (
              SELECT occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0) ELSE 0 END
                     + COALESCE((data->>'verifier_reward')::numeric, 0)
                     + CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0) ELSE 0 END
                       AS total_amount
              FROM autonomous_bounty_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND occurred_at >= $2 AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($6)
                AND kind IN ('bounty_settled', 'submission_rejected')
              UNION ALL
              SELECT occurred_at, kind = 'bounty_settled' AS settled,
                     CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'solver_reward')::numeric, 0) ELSE 0 END
                     + CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'verifier_reward')::numeric, 0)
                       ELSE COALESCE((data->>'bond_paid_to_verifier')::numeric, 0) END
                     + CASE WHEN kind = 'bounty_settled'
                       THEN COALESCE((data->>'timeout_bond_bonus')::numeric, 0) ELSE 0 END
                       AS total_amount
              FROM open_competition_events
              WHERE network = $1
                AND block_time_verified = TRUE
                AND occurred_at >= $2 AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($6)
                AND kind IN ('bounty_settled', 'competition_submission_rejected')
              UNION ALL
              SELECT occurred_at, TRUE AS settled,
                     COALESCE((data->>'solver_reward')::numeric, 0)
                     + COALESCE((data->>'keeper_reward')::numeric, 0) AS total_amount
              FROM open_competition_v2_events
              WHERE network = $1
                AND occurred_at >= $2 AND occurred_at < $3
                AND NOT lower(contract_address) = ANY($6)
                AND kind = 'competition_settled'
            ), days AS (
              SELECT generate_series(
                date_trunc('day', $2::timestamptz),
                date_trunc('day', $3::timestamptz - INTERVAL '1 microsecond'),
                INTERVAL '1 day'
              ) AS day
            )
            SELECT to_char(days.day AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS day,
                   (SELECT COUNT(DISTINCT (namespace, identity))
                    FROM actors
                    WHERE (occurred_at AT TIME ZONE 'UTC')::date =
                          (days.day AT TIME ZONE 'UTC')::date) AS active_identities,
                   COALESCE((SELECT SUM(total_amount)
                    FROM payouts
                    WHERE (occurred_at AT TIME ZONE 'UTC')::date =
                          (days.day AT TIME ZONE 'UTC')::date), 0)::text AS payout_base_units,
                   (SELECT COUNT(*) FROM payouts
                    WHERE settled
                      AND (occurred_at AT TIME ZONE 'UTC')::date =
                          (days.day AT TIME ZONE 'UTC')::date) AS settled_rounds
            FROM days
            ORDER BY days.day
            "#,
        )
        .bind(network)
        .bind(selected_started_at)
        .bind(selected_ended_at)
        .bind(excluded_wallets)
        .bind(excluded_comment_authors)
        .bind(excluded_bounty_contracts)
        .fetch_all(&self.pool)
        .await?;

        let coverage_row = sqlx::query(
            r#"
            WITH protocol_events AS (
              SELECT block_time_verified, occurred_at
              FROM autonomous_bounty_events
              WHERE network = $1
                AND occurred_at >= $2
              UNION ALL
              SELECT block_time_verified, occurred_at
              FROM open_competition_events
              WHERE network = $1
                AND occurred_at >= $2
              UNION ALL
              SELECT TRUE AS block_time_verified, occurred_at
              FROM open_competition_v2_events
              WHERE network = $1
                AND occurred_at >= $2
            )
            SELECT
              COUNT(*) FILTER (WHERE block_time_verified = TRUE) AS verified_events,
              COUNT(*) FILTER (WHERE block_time_verified = FALSE) AS awaiting_events,
              MAX(occurred_at) FILTER (WHERE block_time_verified = TRUE) AS latest_verified_event_at,
              (SELECT COUNT(*) FROM opportunity_comments WHERE created_at >= $2) AS comments,
              (SELECT MAX(created_at) FROM opportunity_comments WHERE created_at >= $2)
                AS latest_comment_at
            FROM protocol_events
            "#,
        )
        .bind(network)
        .bind(launch_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(PlatformMetricsStats {
            generated_at: selected_ended_at,
            identities: PlatformIdentityStats {
                selected: u64_from_i64(identity_row.try_get("selected")?)?,
                previous: u64_from_i64(identity_row.try_get("previous")?)?,
                latest_week: u64_from_i64(identity_row.try_get("latest_week")?)?,
                previous_week: u64_from_i64(identity_row.try_get("previous_week")?)?,
                first_month: u64_from_i64(identity_row.try_get("first_month")?)?,
                lifetime: u64_from_i64(identity_row.try_get("lifetime")?)?,
                posters: u64_from_i64(identity_row.try_get("posters")?)?,
                funders: u64_from_i64(identity_row.try_get("funders")?)?,
                solvers: u64_from_i64(identity_row.try_get("solvers")?)?,
                verifiers: u64_from_i64(identity_row.try_get("verifiers")?)?,
                commenters: u64_from_i64(identity_row.try_get("commenters")?)?,
                marketplace_wallets: u64_from_i64(identity_row.try_get("marketplace_wallets")?)?,
                opportunity_comment_authors: u64_from_i64(
                    identity_row.try_get("opportunity_comment_authors")?,
                )?,
            },
            payouts: PlatformPayoutStats {
                selected_total_base_units: payout_row.try_get("selected_total")?,
                previous_total_base_units: payout_row.try_get("previous_total")?,
                first_month_total_base_units: payout_row.try_get("first_month_total")?,
                lifetime_total_base_units: payout_row.try_get("lifetime_total")?,
                selected_solver_base_units: payout_row.try_get("selected_solver")?,
                selected_verifier_base_units: payout_row.try_get("selected_verifier")?,
                selected_keeper_base_units: payout_row.try_get("selected_keeper")?,
                selected_bonus_base_units: payout_row.try_get("selected_bonus")?,
                selected_settled_rounds: u64_from_i64(payout_row.try_get("selected_settled")?)?,
                previous_settled_rounds: u64_from_i64(payout_row.try_get("previous_settled")?)?,
                first_month_settled_rounds: u64_from_i64(
                    payout_row.try_get("first_month_settled")?,
                )?,
                lifetime_settled_rounds: u64_from_i64(payout_row.try_get("lifetime_settled")?)?,
            },
            claim_cohort: PlatformClaimCohortStats {
                settled: u64_from_i64(cohort_row.try_get("settled")?)?,
                mature: u64_from_i64(cohort_row.try_get("mature")?)?,
                immature: u64_from_i64(cohort_row.try_get("immature")?)?,
            },
            daily: daily_rows
                .into_iter()
                .map(|row| {
                    Ok(PlatformDailyStats {
                        day: row.try_get("day")?,
                        active_identities: u64_from_i64(row.try_get("active_identities")?)?,
                        payout_base_units: row.try_get("payout_base_units")?,
                        settled_rounds: u64_from_i64(row.try_get("settled_rounds")?)?,
                    })
                })
                .collect::<DbResult<Vec<_>>>()?,
            coverage: PlatformMetricsCoverageStats {
                verified_canonical_events: u64_from_i64(coverage_row.try_get("verified_events")?)?,
                awaiting_block_time_events: u64_from_i64(coverage_row.try_get("awaiting_events")?)?,
                opportunity_comments: u64_from_i64(coverage_row.try_get("comments")?)?,
                latest_verified_event_at: coverage_row.try_get("latest_verified_event_at")?,
                latest_comment_at: coverage_row.try_get("latest_comment_at")?,
            },
        })
    }

    pub async fn opportunity_lifecycle_stats(
        &self,
        window_started_at: DateTime<Utc>,
        excluded_bounty_contracts: &[String],
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
               AND NOT lower(created.contract_address) = ANY($2)
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
        .bind(excluded_bounty_contracts)
        .fetch_one(&self.pool)
        .await?;

        let canonical = sqlx::query(
            r#"
            WITH created AS (
              SELECT network, bounty_id, MIN(occurred_at) AS created_at
              FROM autonomous_bounty_events
              WHERE kind = 'canonical_bounty_created' AND occurred_at >= $1
                AND NOT lower(contract_address) = ANY($2)
              GROUP BY network, bounty_id
            ), settled AS (
              SELECT network, bounty_id, MIN(occurred_at) AS settled_at
              FROM autonomous_bounty_events
              WHERE kind = 'bounty_settled'
                AND NOT lower(contract_address) = ANY($2)
              GROUP BY network, bounty_id
            ), posters AS (
              SELECT lower(data->>'creator') AS wallet, COUNT(DISTINCT bounty_id) AS bounties
              FROM autonomous_bounty_events
              WHERE kind = 'canonical_bounty_created' AND occurred_at >= $1
                AND data ? 'creator'
                AND NOT lower(contract_address) = ANY($2)
              GROUP BY lower(data->>'creator')
            ), paid_solvers AS (
              SELECT lower(data->>'solver') AS wallet, COUNT(DISTINCT bounty_id) AS bounties
              FROM autonomous_bounty_events
              WHERE kind = 'bounty_settled' AND occurred_at >= $1
                AND data ? 'solver'
                AND NOT lower(contract_address) = ANY($2)
              GROUP BY lower(data->>'solver')
            )
            SELECT
              (SELECT COUNT(*) FROM created) AS canonical_created_in_window,
              (SELECT COUNT(DISTINCT (network, bounty_id)) FROM autonomous_bounty_events
                WHERE kind = 'bounty_claimed' AND occurred_at >= $1
                  AND NOT lower(contract_address) = ANY($2)) AS canonical_claimed_in_window,
              (SELECT COUNT(DISTINCT (network, bounty_id)) FROM autonomous_bounty_events
                WHERE kind = 'bounty_settled' AND occurred_at >= $1
                  AND NOT lower(contract_address) = ANY($2)) AS canonical_settled_in_window,
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
        .bind(excluded_bounty_contracts)
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

    pub async fn create_or_get_opportunity_comment(
        &self,
        comment: &NewOpportunityComment,
    ) -> DbResult<OpportunityComment> {
        let inserted = sqlx::query(
            r#"
            INSERT INTO opportunity_comments (id, opportunity_id, author, body, feedback)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO NOTHING
            RETURNING id, opportunity_id, author, body, feedback, created_at
            "#,
        )
        .bind(comment.id)
        .bind(&comment.opportunity_id)
        .bind(&comment.author)
        .bind(&comment.body)
        .bind(&comment.feedback)
        .fetch_optional(&self.pool)
        .await?;

        let row = match inserted {
            Some(row) => row,
            None => {
                sqlx::query(
                    r#"
                    SELECT id, opportunity_id, author, body, feedback, created_at
                    FROM opportunity_comments
                    WHERE id = $1
                    "#,
                )
                .bind(comment.id)
                .fetch_one(&self.pool)
                .await?
            }
        };
        let persisted = opportunity_comment_from_row(row)?;
        if persisted.opportunity_id != comment.opportunity_id
            || persisted.author != comment.author
            || persisted.body != comment.body
            || persisted.feedback != comment.feedback
        {
            return Err(DbError::OpportunityCommentConflict);
        }
        Ok(persisted)
    }

    pub async fn list_opportunity_comments(
        &self,
        opportunity_id: &str,
        limit: u32,
    ) -> DbResult<Vec<OpportunityComment>> {
        let limit = i64::from(limit.clamp(1, 100));
        sqlx::query(
            r#"
            SELECT id, opportunity_id, author, body, feedback, created_at
            FROM opportunity_comments
            WHERE opportunity_id = $1
            ORDER BY created_at DESC, id DESC
            LIMIT $2
            "#,
        )
        .bind(opportunity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(opportunity_comment_from_row)
        .collect()
    }

    pub async fn reserve_chatgpt_action_intent(
        &self,
        intent: &NewChatgptActionIntent,
    ) -> DbResult<ChatgptActionIntent> {
        let inserted = sqlx::query(
            r#"
            INSERT INTO chatgpt_action_intents
              (id, idempotency_key, action, network, opportunity_id,
               bounty_contract, bounty_id, actor_wallet, amount_base_units,
               details, request_fingerprint, expires_at)
            VALUES (
              $1, $2, $3, $4, $5, lower($6), lower($7), lower($8), $9,
              $10, $11, $12
            )
            ON CONFLICT (idempotency_key) DO NOTHING
            RETURNING id, idempotency_key, action, network, opportunity_id,
                      bounty_contract, bounty_id, actor_wallet, amount_base_units,
                      details, request_fingerprint, status, transaction_hash,
                      canonical_event_id, canonical_event_kind, confirmed_block,
                      expires_at, created_at, updated_at
            "#,
        )
        .bind(intent.id)
        .bind(&intent.idempotency_key)
        .bind(&intent.action)
        .bind(&intent.network)
        .bind(&intent.opportunity_id)
        .bind(&intent.bounty_contract)
        .bind(&intent.bounty_id)
        .bind(&intent.actor_wallet)
        .bind(intent.amount_base_units.map(i64_from_u64).transpose()?)
        .bind(&intent.details)
        .bind(&intent.request_fingerprint)
        .bind(intent.expires_at)
        .fetch_optional(&self.pool)
        .await?;

        let row = match inserted {
            Some(row) => row,
            None => {
                sqlx::query(
                    r#"
                    SELECT id, idempotency_key, action, network, opportunity_id,
                           bounty_contract, bounty_id, actor_wallet, amount_base_units,
                           details, request_fingerprint, status, transaction_hash,
                           canonical_event_id, canonical_event_kind, confirmed_block,
                           expires_at, created_at, updated_at
                    FROM chatgpt_action_intents
                    WHERE idempotency_key = $1
                    "#,
                )
                .bind(&intent.idempotency_key)
                .fetch_one(&self.pool)
                .await?
            }
        };
        let persisted = chatgpt_action_intent_from_row(row)?;
        if persisted.request_fingerprint != intent.request_fingerprint {
            return Err(DbError::ChatgptActionIntentConflict(
                "idempotency key is already bound to a different action".to_string(),
            ));
        }
        Ok(persisted)
    }

    pub async fn get_chatgpt_action_intent(
        &self,
        id: Uuid,
    ) -> DbResult<Option<ChatgptActionIntent>> {
        sqlx::query(
            r#"
            SELECT id, idempotency_key, action, network, opportunity_id,
                   bounty_contract, bounty_id, actor_wallet, amount_base_units,
                   details, request_fingerprint, status, transaction_hash,
                   canonical_event_id, canonical_event_kind, confirmed_block,
                   expires_at, created_at, updated_at
            FROM chatgpt_action_intents
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(chatgpt_action_intent_from_row)
        .transpose()
    }

    pub async fn observe_chatgpt_action_transaction(
        &self,
        id: Uuid,
        observation: &ChatgptActionObservation,
    ) -> DbResult<ChatgptActionIntent> {
        let row = sqlx::query(
            r#"
            UPDATE chatgpt_action_intents
            SET transaction_hash = lower($2),
                bounty_contract = COALESCE(bounty_contract, lower($3)),
                bounty_id = COALESCE(bounty_id, lower($4)),
                actor_wallet = COALESCE(actor_wallet, lower($5)),
                status = 'pending_confirmation',
                updated_at = now()
            WHERE id = $1
              AND status IN ('review_required', 'pending_confirmation')
              AND expires_at > now()
              AND (
                transaction_hash IS NULL
                OR transaction_hash = lower($2)
              )
              AND (
                bounty_contract IS NULL
                OR $3 IS NULL
                OR bounty_contract = lower($3)
              )
              AND (
                bounty_id IS NULL
                OR $4 IS NULL
                OR bounty_id = lower($4)
              )
              AND (
                actor_wallet IS NULL
                OR $5 IS NULL
                OR actor_wallet = lower($5)
              )
            RETURNING id, idempotency_key, action, network, opportunity_id,
                      bounty_contract, bounty_id, actor_wallet, amount_base_units,
                      details, request_fingerprint, status, transaction_hash,
                      canonical_event_id, canonical_event_kind, confirmed_block,
                      expires_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(&observation.transaction_hash)
        .bind(&observation.bounty_contract)
        .bind(&observation.bounty_id)
        .bind(&observation.actor_wallet)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(row) = row {
            return chatgpt_action_intent_from_row(row);
        }
        match self.get_chatgpt_action_intent(id).await? {
            None => Err(DbError::ChatgptActionIntentUnavailable),
            Some(intent) if intent.status == "confirmed" => Ok(intent),
            Some(intent) if intent.expires_at <= Utc::now() => {
                Err(DbError::ChatgptActionIntentUnavailable)
            }
            Some(_) => Err(DbError::ChatgptActionIntentConflict(
                "transaction observation does not match the original action".to_string(),
            )),
        }
    }

    pub async fn confirm_chatgpt_action_intent(
        &self,
        id: Uuid,
        event: &AutonomousBountyEvent,
    ) -> DbResult<ChatgptActionIntent> {
        sqlx::query(
            r#"
            UPDATE chatgpt_action_intents
            SET status = 'confirmed',
                canonical_event_id = $2,
                canonical_event_kind = $3,
                confirmed_block = $4,
                updated_at = now()
            WHERE id = $1
              AND status IN ('review_required', 'pending_confirmation', 'confirmed')
              AND transaction_hash = lower($5)
              AND (
                canonical_event_id IS NULL
                OR canonical_event_id = $2
              )
            RETURNING id, idempotency_key, action, network, opportunity_id,
                      bounty_contract, bounty_id, actor_wallet, amount_base_units,
                      details, request_fingerprint, status, transaction_hash,
                      canonical_event_id, canonical_event_kind, confirmed_block,
                      expires_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(event.id)
        .bind(autonomous_event_kind_storage_name(event.kind))
        .bind(i64_from_u64(event.block_number)?)
        .bind(&event.tx_hash)
        .fetch_optional(&self.pool)
        .await?
        .map(chatgpt_action_intent_from_row)
        .transpose()?
        .ok_or_else(|| {
            DbError::ChatgptActionIntentConflict(
                "canonical event does not match the observed transaction".to_string(),
            )
        })
    }

    pub async fn expire_chatgpt_action_intent(
        &self,
        id: Uuid,
    ) -> DbResult<Option<ChatgptActionIntent>> {
        sqlx::query(
            r#"
            UPDATE chatgpt_action_intents
            SET status = 'expired', updated_at = now()
            WHERE id = $1
              AND status IN ('review_required', 'pending_confirmation')
              AND expires_at <= now()
            RETURNING id, idempotency_key, action, network, opportunity_id,
                      bounty_contract, bounty_id, actor_wallet, amount_base_units,
                      details, request_fingerprint, status, transaction_hash,
                      canonical_event_id, canonical_event_kind, confirmed_block,
                      expires_at, created_at, updated_at
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .map(chatgpt_action_intent_from_row)
        .transpose()
    }

    pub async fn put_bounty_image_asset(
        &self,
        asset: &NewBountyImageAsset,
    ) -> DbResult<BountyImageAsset> {
        let row = sqlx::query(
            r#"
            INSERT INTO bounty_image_assets (sha256, mime_type, content)
            VALUES ($1, $2, $3)
            ON CONFLICT (sha256) DO UPDATE SET
              mime_type = bounty_image_assets.mime_type
            RETURNING sha256, mime_type, content, created_at
            "#,
        )
        .bind(&asset.sha256)
        .bind(&asset.mime_type)
        .bind(&asset.content)
        .fetch_one(&self.pool)
        .await?;
        let stored = bounty_image_asset_from_row(&row)?;
        if stored.mime_type != asset.mime_type || stored.content != asset.content {
            return Err(DbError::BountyImageAssetConflict(
                "bounty image hash already exists with different content metadata".to_string(),
            ));
        }
        Ok(stored)
    }

    pub async fn get_bounty_image_asset(&self, sha256: &str) -> DbResult<Option<BountyImageAsset>> {
        let row = sqlx::query(
            r#"
            SELECT sha256, mime_type, content, created_at
            FROM bounty_image_assets
            WHERE sha256 = $1
            "#,
        )
        .bind(sha256)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| bounty_image_asset_from_row(&row)).transpose()
    }

    pub async fn delete_expired_chatgpt_action_intents_before(
        &self,
        cutoff: DateTime<Utc>,
    ) -> DbResult<u64> {
        let result = sqlx::query(
            r#"
            DELETE FROM chatgpt_action_intents
            WHERE expires_at < $1
            "#,
        )
        .bind(cutoff)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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

    pub async fn reserve_open_competition_entrant_relay(
        &self,
        relay: &NewOpenCompetitionEntrantRelay,
        max_network_attempts: u32,
        max_wallet_attempts: u32,
    ) -> DbResult<OpenCompetitionEntrantRelay> {
        if max_network_attempts == 0
            || max_wallet_attempts == 0
            || max_wallet_attempts > max_network_attempts
        {
            return Err(DbError::OpenCompetitionEntrantRelayQuotaExceeded(
                "configured quota is invalid".to_string(),
            ));
        }
        if relay.action > 2 || relay.deadline == 0 || relay.idempotency_key.trim().is_empty() {
            return Err(DbError::OpenCompetitionEntrantRelayConflict(
                "relay action, deadline, or idempotency key is invalid".to_string(),
            ));
        }
        let normalized_wallet = normalize_key_address(&relay.wallet);
        let normalized_bounty = normalize_key_address(&relay.bounty_contract);
        let normalized_delegate = normalize_key_address(&relay.delegate);
        let normalized_payload_hash = relay.payload_hash.to_ascii_lowercase();
        let normalized_relayer = normalize_key_address(&relay.relayer_address);
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(format!(
                "open-competition-entrant-relay-quota:{}",
                relay.network
            ))
            .execute(&mut *transaction)
            .await?;

        let existing_idempotency = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, wallet, bounty_contract, delegate,
                   action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message, receipt_block,
                   receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                   canonical_event, payment_proven, created_at, updated_at
            FROM open_competition_entrant_relays
            WHERE idempotency_key = $1
            "#,
        )
        .bind(&relay.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        .map(open_competition_entrant_relay_from_row)
        .transpose()?;
        if let Some(existing) = existing_idempotency {
            validate_open_competition_entrant_relay_replay(&existing, relay)?;
            transaction.commit().await?;
            return Ok(existing);
        }

        let existing_live_nonce = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, wallet, bounty_contract, delegate,
                   action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message, receipt_block,
                   receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                   canonical_event, payment_proven, created_at, updated_at
            FROM open_competition_entrant_relays
            WHERE network = $1 AND wallet = $2 AND wallet_nonce = $3
              AND (status <> 'failed' OR retryable)
            "#,
        )
        .bind(&relay.network)
        .bind(&normalized_wallet)
        .bind(i64_from_u64(relay.wallet_nonce)?)
        .fetch_optional(&mut *transaction)
        .await?
        .map(open_competition_entrant_relay_from_row)
        .transpose()?;
        if let Some(existing) = existing_live_nonce {
            validate_open_competition_entrant_relay_replay(&existing, relay)?;
            transaction.commit().await?;
            return Ok(existing);
        }

        let quota = sqlx::query(
            r#"
            SELECT COUNT(*) AS network_count,
                   COUNT(*) FILTER (WHERE wallet = $2) AS wallet_count
            FROM open_competition_entrant_relays
            WHERE network = $1 AND created_at >= now() - interval '24 hours'
            "#,
        )
        .bind(&relay.network)
        .bind(&normalized_wallet)
        .fetch_one(&mut *transaction)
        .await?;
        let network_count: i64 = quota.try_get("network_count")?;
        let wallet_count: i64 = quota.try_get("wallet_count")?;
        if network_count >= i64::from(max_network_attempts) {
            return Err(DbError::OpenCompetitionEntrantRelayQuotaExceeded(
                "network rolling-24-hour relay limit reached".to_string(),
            ));
        }
        if wallet_count >= i64::from(max_wallet_attempts) {
            return Err(DbError::OpenCompetitionEntrantRelayQuotaExceeded(
                "wallet rolling-24-hour relay limit reached".to_string(),
            ));
        }

        sqlx::query(
            r#"
            INSERT INTO open_competition_entrant_relays
              (id, idempotency_key, network, wallet, bounty_contract, delegate,
               action, wallet_nonce, deadline, payload_hash, request_fingerprint,
               relayer_address, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'prepared')
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(relay.id)
        .bind(&relay.idempotency_key)
        .bind(&relay.network)
        .bind(&normalized_wallet)
        .bind(&normalized_bounty)
        .bind(&normalized_delegate)
        .bind(i16::from(relay.action))
        .bind(i64_from_u64(relay.wallet_nonce)?)
        .bind(i64_from_u64(relay.deadline)?)
        .bind(&normalized_payload_hash)
        .bind(&relay.request_fingerprint)
        .bind(&normalized_relayer)
        .execute(&mut *transaction)
        .await?;

        let persisted = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, wallet, bounty_contract, delegate,
                   action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message, receipt_block,
                   receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                   canonical_event, payment_proven, created_at, updated_at
            FROM open_competition_entrant_relays
            WHERE idempotency_key = $1
            "#,
        )
        .bind(&relay.idempotency_key)
        .fetch_optional(&mut *transaction)
        .await?
        .map(open_competition_entrant_relay_from_row)
        .transpose()?
        .ok_or_else(|| {
            DbError::OpenCompetitionEntrantRelayConflict(
                "idempotency key is already bound to another relay".to_string(),
            )
        })?;
        validate_open_competition_entrant_relay_replay(&persisted, relay)?;
        transaction.commit().await?;
        Ok(persisted)
    }

    pub async fn get_open_competition_entrant_relay(
        &self,
        id: Uuid,
    ) -> DbResult<Option<OpenCompetitionEntrantRelay>> {
        let row = sqlx::query(
            r#"
            SELECT id, idempotency_key, network, wallet, bounty_contract, delegate,
                   action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                   relayer_address, status, retryable, attempt_count, tx_hash,
                   estimated_gas, gas_limit, error_code, error_message, receipt_block,
                   receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                   canonical_event, payment_proven, created_at, updated_at
            FROM open_competition_entrant_relays WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(open_competition_entrant_relay_from_row).transpose()
    }

    pub async fn claim_open_competition_entrant_relay(
        &self,
        id: Uuid,
        lease_token: Uuid,
        lease_seconds: u64,
    ) -> DbResult<Option<OpenCompetitionEntrantRelay>> {
        let row = sqlx::query(
            r#"
            UPDATE open_competition_entrant_relays
            SET status = 'relaying', retryable = true,
                attempt_count = attempt_count + 1, lease_token = $2,
                lease_expires_at = now() + make_interval(secs => $3),
                error_code = NULL, error_message = NULL, updated_at = now()
            WHERE id = $1
              AND (
                status = 'prepared'
                OR (status = 'failed' AND retryable)
                OR (status = 'relaying' AND lease_expires_at <= now())
              )
            RETURNING id, idempotency_key, network, wallet, bounty_contract, delegate,
                      action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message, receipt_block,
                      receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                      canonical_event, payment_proven, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(i64_from_u64(lease_seconds)?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(open_competition_entrant_relay_from_row).transpose()
    }

    pub async fn mark_open_competition_entrant_relay_broadcast(
        &self,
        id: Uuid,
        lease_token: Uuid,
        tx_hash: &str,
        estimated_gas: u64,
        gas_limit: u64,
    ) -> DbResult<OpenCompetitionEntrantRelay> {
        let row = sqlx::query(
            r#"
            UPDATE open_competition_entrant_relays
            SET status = 'broadcast', retryable = true, tx_hash = $3,
                estimated_gas = $4, gas_limit = $5,
                lease_token = NULL, lease_expires_at = NULL, updated_at = now()
            WHERE id = $1 AND lease_token = $2 AND status = 'relaying'
            RETURNING id, idempotency_key, network, wallet, bounty_contract, delegate,
                      action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message, receipt_block,
                      receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                      canonical_event, payment_proven, created_at, updated_at
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
            DbError::OpenCompetitionEntrantRelayConflict(
                "relay lease was lost before broadcast persisted".to_string(),
            )
        })?;
        open_competition_entrant_relay_from_row(row)
    }

    pub async fn mark_open_competition_entrant_relay_failed(
        &self,
        id: Uuid,
        lease_token: Option<Uuid>,
        retryable: bool,
        error_code: &str,
        error_message: &str,
    ) -> DbResult<OpenCompetitionEntrantRelay> {
        let row = sqlx::query(
            r#"
            UPDATE open_competition_entrant_relays
            SET status = 'failed', retryable = $3, error_code = $4, error_message = $5,
                lease_token = NULL, lease_expires_at = NULL, updated_at = now()
            WHERE id = $1
              AND status IN ('relaying', 'broadcast', 'failed')
              AND ($2::uuid IS NULL OR lease_token = $2)
            RETURNING id, idempotency_key, network, wallet, bounty_contract, delegate,
                      action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message, receipt_block,
                      receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                      canonical_event, payment_proven, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .bind(retryable)
        .bind(error_code)
        .bind(error_message.chars().take(500).collect::<String>())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::OpenCompetitionEntrantRelayConflict("relay failure lease mismatch".to_string())
        })?;
        open_competition_entrant_relay_from_row(row)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn mark_open_competition_entrant_relay_confirmed(
        &self,
        id: Uuid,
        receipt_block: u64,
        receipt_block_hash: &str,
        canonical_safe_block: u64,
        canonical_safe_block_hash: &str,
        canonical_event: &str,
        payment_proven: bool,
    ) -> DbResult<OpenCompetitionEntrantRelay> {
        let row = sqlx::query(
            r#"
            UPDATE open_competition_entrant_relays
            SET status = 'confirmed', retryable = false,
                receipt_block = $2, receipt_block_hash = $3,
                canonical_safe_block = $4, canonical_safe_block_hash = $5,
                canonical_event = $6, payment_proven = $7,
                lease_token = NULL, lease_expires_at = NULL,
                error_code = NULL, error_message = NULL, updated_at = now()
            WHERE id = $1 AND status IN ('broadcast', 'confirmed')
            RETURNING id, idempotency_key, network, wallet, bounty_contract, delegate,
                      action, wallet_nonce, deadline, payload_hash, request_fingerprint,
                      relayer_address, status, retryable, attempt_count, tx_hash,
                      estimated_gas, gas_limit, error_code, error_message, receipt_block,
                      receipt_block_hash, canonical_safe_block, canonical_safe_block_hash,
                      canonical_event, payment_proven, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(i64_from_u64(receipt_block)?)
        .bind(receipt_block_hash.to_ascii_lowercase())
        .bind(i64_from_u64(canonical_safe_block)?)
        .bind(canonical_safe_block_hash.to_ascii_lowercase())
        .bind(canonical_event)
        .bind(payment_proven)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            DbError::OpenCompetitionEntrantRelayConflict(
                "relay was not broadcast before confirmation".to_string(),
            )
        })?;
        open_competition_entrant_relay_from_row(row)
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

    pub async fn claim_funnel_stats(
        &self,
        window_hours: u32,
        excluded_bounty_contracts: &[String],
    ) -> DbResult<ClaimFunnelStats> {
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
              AND NOT lower(bounty_contract) = ANY($2)
            "#,
        )
        .bind(window_started_at)
        .bind(excluded_bounty_contracts)
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
              AND NOT lower(candidate.bounty_contract) = ANY($2)
            "#,
        )
        .bind(window_started_at)
        .bind(excluded_bounty_contracts)
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
                AND NOT lower(contract_address) = ANY($2)
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
        .bind(excluded_bounty_contracts)
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
              AND NOT lower(bounty_contract) = ANY($2)
            GROUP BY failure_code
            ORDER BY failure_code
            "#,
        )
        .bind(window_started_at)
        .bind(excluded_bounty_contracts)
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

    pub async fn upsert_open_competition_event(
        &self,
        network: &str,
        factory_contract: &str,
        event: &OpenCompetitionEvent,
    ) -> DbResult<()> {
        let kind = serde_json::to_value(event.kind)?
            .as_str()
            .ok_or_else(|| DbError::InvalidEnum("open competition event kind".to_string()))?
            .to_string();
        let result = sqlx::query(
            r#"
            INSERT INTO open_competition_events
              (id, protocol_version, log_key, network, factory_contract, tx_hash,
               block_number, log_index, contract_address, bounty_id, kind, data, occurred_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (log_key) DO UPDATE SET
              tx_hash = EXCLUDED.tx_hash,
              block_number = EXCLUDED.block_number,
              log_index = EXCLUDED.log_index,
              contract_address = EXCLUDED.contract_address,
              bounty_id = EXCLUDED.bounty_id,
              kind = EXCLUDED.kind,
              data = EXCLUDED.data,
              occurred_at = CASE
                WHEN open_competition_events.block_time_verified
                  THEN open_competition_events.occurred_at
                ELSE EXCLUDED.occurred_at
              END
            WHERE open_competition_events.protocol_version = EXCLUDED.protocol_version
              AND open_competition_events.network = EXCLUDED.network
              AND open_competition_events.factory_contract = EXCLUDED.factory_contract
            "#,
        )
        .bind(event.id)
        .bind(&event.protocol_version)
        .bind(&event.log_key)
        .bind(network)
        .bind(normalize_key_address(factory_contract))
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
        if result.rows_affected() != 1 {
            return Err(DbError::OpenCompetitionEventConflict(event.log_key.clone()));
        }
        Ok(())
    }

    pub async fn list_open_competition_events(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, protocol_version, log_key, tx_hash, block_number, log_index,
                   contract_address, bounty_id, kind, data, occurred_at
            FROM open_competition_events
            WHERE network = $1 AND factory_contract = $2
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_event_from_row)
            .collect()
    }

    pub async fn list_verified_open_competition_events(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, protocol_version, log_key, tx_hash, block_number, log_index,
                   contract_address, bounty_id, kind, data, occurred_at
            FROM open_competition_events
            WHERE network = $1 AND factory_contract = $2 AND block_time_verified = TRUE
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_event_from_row)
            .collect()
    }

    pub async fn list_canonical_open_competition_contracts(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<String>> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT data->>'bounty_contract' AS bounty_contract
            FROM open_competition_events
            WHERE network = $1 AND factory_contract = $2
              AND kind = 'canonical_competition_created'
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

    pub async fn upsert_open_competition_v2_event(
        &self,
        network: &str,
        factory_contract: &str,
        event: &OpenCompetitionV2Event,
        safe: &OpenCompetitionV2SafeContext,
    ) -> DbResult<()> {
        if event.protocol_version != chain_base::OPEN_COMPETITION_V2_PROTOCOL_VERSION {
            return Err(DbError::OpenCompetitionV2Conflict(format!(
                "unsupported protocol version {}",
                event.protocol_version
            )));
        }
        if event.block_number > safe.safe_block_number {
            return Err(DbError::OpenCompetitionV2Conflict(format!(
                "event block {} is newer than safe block {}",
                event.block_number, safe.safe_block_number
            )));
        }
        let kind = serde_json::to_value(event.kind)?
            .as_str()
            .ok_or_else(|| DbError::InvalidEnum("Open Competition V2 event kind".to_string()))?
            .to_string();
        let result = sqlx::query(
            r#"
            INSERT INTO open_competition_v2_events
              (id, protocol_version, log_key, network, factory_contract, tx_hash,
               block_number, block_hash, log_index, contract_address, bounty_id,
               kind, data, occurred_at, safe_block_number, safe_block_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
            ON CONFLICT (network, factory_contract, log_key) DO UPDATE SET
              safe_block_number = EXCLUDED.safe_block_number,
              safe_block_hash = EXCLUDED.safe_block_hash
            WHERE open_competition_v2_events.id = EXCLUDED.id
              AND open_competition_v2_events.protocol_version = EXCLUDED.protocol_version
              AND open_competition_v2_events.tx_hash = EXCLUDED.tx_hash
              AND open_competition_v2_events.block_number = EXCLUDED.block_number
              AND open_competition_v2_events.block_hash = EXCLUDED.block_hash
              AND open_competition_v2_events.log_index = EXCLUDED.log_index
              AND open_competition_v2_events.contract_address = EXCLUDED.contract_address
              AND open_competition_v2_events.bounty_id = EXCLUDED.bounty_id
              AND open_competition_v2_events.kind = EXCLUDED.kind
              AND open_competition_v2_events.data = EXCLUDED.data
              AND EXCLUDED.safe_block_number >= open_competition_v2_events.safe_block_number
              AND (
                EXCLUDED.safe_block_number > open_competition_v2_events.safe_block_number
                OR EXCLUDED.safe_block_hash = open_competition_v2_events.safe_block_hash
              )
            "#,
        )
        .bind(event.id)
        .bind(&event.protocol_version)
        .bind(&event.log_key)
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .bind(normalize_key_address(&event.tx_hash))
        .bind(i64_from_u64(event.block_number)?)
        .bind(normalize_key_address(&safe.block_hash))
        .bind(i64_from_u64(event.log_index)?)
        .bind(normalize_key_address(&event.contract_address))
        .bind(&event.bounty_id)
        .bind(kind)
        .bind(&event.data)
        .bind(event.occurred_at)
        .bind(i64_from_u64(safe.safe_block_number)?)
        .bind(normalize_key_address(&safe.safe_block_hash))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(DbError::OpenCompetitionV2Conflict(event.log_key.clone()));
        }
        Ok(())
    }

    pub async fn list_open_competition_v2_events(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionV2Event>> {
        let rows = sqlx::query(
            r#"
            SELECT id, protocol_version, log_key, tx_hash, block_number, log_index,
                   contract_address, bounty_id, kind, data, occurred_at
            FROM open_competition_v2_events
            WHERE network = $1 AND factory_contract = $2
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_v2_event_from_row)
            .collect()
    }

    pub async fn upsert_open_competition_v2_indexer_agreement(
        &self,
        agreement: &OpenCompetitionV2IndexerAgreement,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO open_competition_v2_indexer_agreements
              (network, factory_contract, protocol_version, common_safe_block,
               primary_safe_head, shadow_safe_head,
               primary_block_hash, shadow_block_hash, canonical_event_count,
               canonical_event_set_hash, agrees, failure_code, observed_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            ON CONFLICT (network, factory_contract) DO UPDATE SET
              protocol_version = EXCLUDED.protocol_version,
              common_safe_block = EXCLUDED.common_safe_block,
              primary_safe_head = EXCLUDED.primary_safe_head,
              shadow_safe_head = EXCLUDED.shadow_safe_head,
              primary_block_hash = EXCLUDED.primary_block_hash,
              shadow_block_hash = EXCLUDED.shadow_block_hash,
              canonical_event_count = EXCLUDED.canonical_event_count,
              canonical_event_set_hash = EXCLUDED.canonical_event_set_hash,
              agrees = EXCLUDED.agrees,
              failure_code = EXCLUDED.failure_code,
              observed_at = EXCLUDED.observed_at
            "#,
        )
        .bind(&agreement.network)
        .bind(normalize_key_address(&agreement.factory_contract))
        .bind(&agreement.protocol_version)
        .bind(i64_from_u64(agreement.common_safe_block)?)
        .bind(i64_from_u64(agreement.primary_safe_head)?)
        .bind(i64_from_u64(agreement.shadow_safe_head)?)
        .bind(normalize_key_address(&agreement.primary_block_hash))
        .bind(normalize_key_address(&agreement.shadow_block_hash))
        .bind(i64_from_u64(agreement.canonical_event_count)?)
        .bind(normalize_key_address(&agreement.canonical_event_set_hash))
        .bind(agreement.agrees)
        .bind(&agreement.failure_code)
        .bind(agreement.observed_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_open_competition_v2_indexer_agreement(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Option<OpenCompetitionV2IndexerAgreement>> {
        let row = sqlx::query(
            r#"
            SELECT network, factory_contract, protocol_version, common_safe_block,
                   primary_safe_head, shadow_safe_head,
                   primary_block_hash, shadow_block_hash, canonical_event_count,
                   canonical_event_set_hash, agrees, failure_code, observed_at
            FROM open_competition_v2_indexer_agreements
            WHERE network = $1 AND factory_contract = $2
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(OpenCompetitionV2IndexerAgreement {
                network: row.try_get("network")?,
                factory_contract: row.try_get("factory_contract")?,
                protocol_version: row.try_get("protocol_version")?,
                common_safe_block: u64_from_i64(row.try_get("common_safe_block")?)?,
                primary_safe_head: u64_from_i64(row.try_get("primary_safe_head")?)?,
                shadow_safe_head: u64_from_i64(row.try_get("shadow_safe_head")?)?,
                primary_block_hash: row.try_get("primary_block_hash")?,
                shadow_block_hash: row.try_get("shadow_block_hash")?,
                canonical_event_count: u64_from_i64(row.try_get("canonical_event_count")?)?,
                canonical_event_set_hash: row.try_get("canonical_event_set_hash")?,
                agrees: row.try_get("agrees")?,
                failure_code: row.try_get("failure_code")?,
                observed_at: row.try_get("observed_at")?,
            })
        })
        .transpose()
    }

    pub async fn list_open_competition_v2_events_for_contract(
        &self,
        network: &str,
        competition_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionV2Event>> {
        let rows = sqlx::query(
            r#"
            SELECT id, protocol_version, log_key, tx_hash, block_number, log_index,
                   contract_address, bounty_id, kind, data, occurred_at
            FROM open_competition_v2_events
            WHERE network = $1 AND contract_address = $2
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(competition_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_v2_event_from_row)
            .collect()
    }

    pub async fn upsert_open_competition_v2_projection(
        &self,
        network: &str,
        factory_contract: &str,
        projection: &OpenCompetitionV2Projection,
        safe_block_number: u64,
        safe_block_hash: &str,
    ) -> DbResult<()> {
        if projection.last_block > safe_block_number {
            return Err(DbError::OpenCompetitionV2Conflict(format!(
                "projection {} is newer than its safe block",
                projection.bounty_id
            )));
        }
        let result = sqlx::query(
            r#"
            INSERT INTO open_competition_v2_projections
              (network, factory_contract, bounty_id, competition_contract, creator,
               creation_nonce, beta_risk_hash, state, solver_reward, keeper_reward,
               funding_deadline, proof_window_seconds, winner_mode, score_direction,
               score_threshold, proof_system, verifier_adapter, program_vkey,
               source_hash, elf_hash, journal_schema_hash, metric_program_hash,
               execution_policy_hash, verification_policy_hash, settlement_policy_hash,
               funded_amount, proof_deadline,
               accepted_entries, leader, winner, refund_pool_remaining, last_block,
               last_log_index, safe_block_number, safe_block_hash)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                    $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24,
                    $25, $26, $27, $28, $29, $30, $31, $32, $33, $34, $35)
            ON CONFLICT (network, factory_contract, bounty_id) DO UPDATE SET
              state = EXCLUDED.state,
              solver_reward = EXCLUDED.solver_reward,
              keeper_reward = EXCLUDED.keeper_reward,
              winner_mode = EXCLUDED.winner_mode,
              proof_system = EXCLUDED.proof_system,
              funded_amount = EXCLUDED.funded_amount,
              proof_deadline = EXCLUDED.proof_deadline,
              accepted_entries = EXCLUDED.accepted_entries,
              leader = EXCLUDED.leader,
              winner = EXCLUDED.winner,
              refund_pool_remaining = EXCLUDED.refund_pool_remaining,
              last_block = EXCLUDED.last_block,
              last_log_index = EXCLUDED.last_log_index,
              safe_block_number = EXCLUDED.safe_block_number,
              safe_block_hash = EXCLUDED.safe_block_hash,
              updated_at = now()
            WHERE open_competition_v2_projections.competition_contract = EXCLUDED.competition_contract
              AND open_competition_v2_projections.creator = EXCLUDED.creator
              AND open_competition_v2_projections.creation_nonce IS NOT DISTINCT FROM EXCLUDED.creation_nonce
              AND open_competition_v2_projections.beta_risk_hash IS NOT DISTINCT FROM EXCLUDED.beta_risk_hash
              AND open_competition_v2_projections.program_vkey IS NOT DISTINCT FROM EXCLUDED.program_vkey
              AND open_competition_v2_projections.elf_hash IS NOT DISTINCT FROM EXCLUDED.elf_hash
              AND open_competition_v2_projections.verification_policy_hash IS NOT DISTINCT FROM EXCLUDED.verification_policy_hash
              AND (EXCLUDED.last_block, EXCLUDED.last_log_index)
                    >= (open_competition_v2_projections.last_block,
                        open_competition_v2_projections.last_log_index)
              AND EXCLUDED.safe_block_number >= open_competition_v2_projections.safe_block_number
              AND (
                EXCLUDED.safe_block_number > open_competition_v2_projections.safe_block_number
                OR EXCLUDED.safe_block_hash = open_competition_v2_projections.safe_block_hash
              )
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .bind(&projection.bounty_id)
        .bind(normalize_key_address(&projection.competition))
        .bind(normalize_key_address(&projection.creator))
        .bind(&projection.creation_nonce)
        .bind(&projection.beta_risk_hash)
        .bind(open_competition_v2_state_storage_name(projection.state))
        .bind(projection.solver_reward.to_string())
        .bind(projection.keeper_reward.to_string())
        .bind(optional_i64_from_u64(projection.funding_deadline)?)
        .bind(optional_i64_from_u64(projection.proof_window_seconds)?)
        .bind(&projection.winner_mode)
        .bind(&projection.score_direction)
        .bind(&projection.score_threshold)
        .bind(&projection.proof_system)
        .bind(&projection.verifier_adapter)
        .bind(&projection.program_vkey)
        .bind(&projection.source_hash)
        .bind(&projection.elf_hash)
        .bind(&projection.journal_schema_hash)
        .bind(&projection.metric_program_hash)
        .bind(&projection.execution_policy_hash)
        .bind(&projection.verification_policy_hash)
        .bind(&projection.settlement_policy_hash)
        .bind(projection.funded_amount.to_string())
        .bind(optional_i64_from_u64(projection.proof_deadline)?)
        .bind(i64_from_u64(projection.accepted_entries)?)
        .bind(projection.leader.as_deref().map(normalize_key_address))
        .bind(projection.winner.as_deref().map(normalize_key_address))
        .bind(projection.refund_pool_remaining.to_string())
        .bind(i64_from_u64(projection.last_block)?)
        .bind(i64_from_u64(projection.last_log_index)?)
        .bind(i64_from_u64(safe_block_number)?)
        .bind(normalize_key_address(safe_block_hash))
        .execute(&self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(DbError::OpenCompetitionV2Conflict(
                projection.bounty_id.clone(),
            ));
        }
        Ok(())
    }

    pub async fn list_open_competition_v2_projections(
        &self,
        network: &str,
        factory_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionV2StoredProjection>> {
        let rows = sqlx::query(
            r#"
            SELECT network, factory_contract, bounty_id, competition_contract,
                   creator, creation_nonce, beta_risk_hash, state, solver_reward,
                   keeper_reward, funding_deadline, proof_window_seconds, winner_mode,
                   score_direction, score_threshold, proof_system, verifier_adapter,
                   program_vkey, source_hash, elf_hash, journal_schema_hash,
                   metric_program_hash, execution_policy_hash,
                   verification_policy_hash, settlement_policy_hash, funded_amount,
                   proof_deadline, accepted_entries, leader, winner,
                   refund_pool_remaining, last_block, last_log_index,
                   safe_block_number, safe_block_hash
            FROM open_competition_v2_projections
            WHERE network = $1 AND factory_contract = $2
            ORDER BY last_block DESC, last_log_index DESC
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_v2_projection_from_row)
            .collect()
    }

    pub async fn insert_open_competition_v2_proof_job(
        &self,
        job: &OpenCompetitionV2ProofJob,
    ) -> DbResult<OpenCompetitionV2ProofJob> {
        if job.state != OpenCompetitionV2ProofJobState::Quoted {
            return Err(DbError::OpenCompetitionV2Conflict(
                "new proof job must start in quoted state".to_string(),
            ));
        }
        let result = sqlx::query(
            r#"
            INSERT INTO open_competition_v2_proof_jobs
              (id, idempotency_key, network, competition_contract, solver,
               solver_nonce, artifact_hash, program_input, expected_public_values,
               requested_relay, proof_system, state, gross_prize,
               proof_fee_quote, relay_fee_quote, net_prize_if_win,
               maximum_charge, winner_mode, competition_risk,
               quote_expires_at, proof_sla_deadline, payer,
               payment_authorization_nonce, payment_authorization, payment_tx_hash,
               payment_block_number, payment_evidence, proof_hash,
               public_values_hash, proof, public_values, proof_provider_job_id,
               solver_authorization_deadline, solver_signature, relay_tx_hash,
               settlement_event_id, refund_evidence, refund_tx_hash,
               refund_block_number, refund_due_at, failure_code, failure_message,
               created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
                    $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23,
                    $24, $25, $26, $27, $28, $29, $30, $31, $32, $33, $34,
                    $35, $36, $37, $38, $39, $40, $41, $42, $43, $44)
            ON CONFLICT (idempotency_key) DO NOTHING
            "#,
        )
        .bind(job.id)
        .bind(&job.idempotency_key)
        .bind(&job.network)
        .bind(normalize_key_address(&job.competition_contract))
        .bind(normalize_key_address(&job.solver))
        .bind(&job.solver_nonce)
        .bind(normalize_key_address(&job.artifact_hash))
        .bind(&job.program_input)
        .bind(&job.expected_public_values)
        .bind(job.requested_relay)
        .bind(&job.proof_system)
        .bind(job.state.storage_name())
        .bind(&job.gross_prize)
        .bind(&job.proof_fee_quote)
        .bind(&job.relay_fee_quote)
        .bind(&job.net_prize_if_win)
        .bind(&job.maximum_charge)
        .bind(&job.winner_mode)
        .bind(&job.competition_risk)
        .bind(job.quote_expires_at)
        .bind(job.proof_sla_deadline)
        .bind(&job.payer)
        .bind(&job.payment_authorization_nonce)
        .bind(&job.payment_authorization)
        .bind(&job.payment_tx_hash)
        .bind(job.payment_block_number.map(i64_from_u64).transpose()?)
        .bind(&job.payment_evidence)
        .bind(&job.proof_hash)
        .bind(&job.public_values_hash)
        .bind(&job.proof)
        .bind(&job.public_values)
        .bind(&job.proof_provider_job_id)
        .bind(
            job.solver_authorization_deadline
                .map(i64_from_u64)
                .transpose()?,
        )
        .bind(&job.solver_signature)
        .bind(&job.relay_tx_hash)
        .bind(job.settlement_event_id)
        .bind(&job.refund_evidence)
        .bind(&job.refund_tx_hash)
        .bind(job.refund_block_number.map(i64_from_u64).transpose()?)
        .bind(job.refund_due_at)
        .bind(&job.failure_code)
        .bind(&job.failure_message)
        .bind(job.created_at)
        .bind(job.updated_at)
        .execute(&self.pool)
        .await?;
        let stored = self
            .get_open_competition_v2_proof_job_by_idempotency(&job.idempotency_key)
            .await?
            .ok_or_else(|| DbError::OpenCompetitionV2Conflict(job.idempotency_key.clone()))?;
        if result.rows_affected() == 0 && !same_open_competition_v2_quote(job, &stored) {
            return Err(DbError::OpenCompetitionV2Conflict(
                job.idempotency_key.clone(),
            ));
        }
        Ok(stored)
    }

    pub async fn get_open_competition_v2_proof_job(
        &self,
        id: Uuid,
    ) -> DbResult<Option<OpenCompetitionV2ProofJob>> {
        let row = sqlx::query(
            r#"
            SELECT * FROM open_competition_v2_proof_jobs WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(open_competition_v2_proof_job_from_row).transpose()
    }

    pub async fn get_open_competition_v2_proof_job_by_idempotency(
        &self,
        idempotency_key: &str,
    ) -> DbResult<Option<OpenCompetitionV2ProofJob>> {
        let row = sqlx::query(
            r#"
            SELECT * FROM open_competition_v2_proof_jobs WHERE idempotency_key = $1
            "#,
        )
        .bind(idempotency_key)
        .fetch_optional(&self.pool)
        .await?;
        row.map(open_competition_v2_proof_job_from_row).transpose()
    }

    pub async fn list_open_competition_v2_proof_jobs_for_contract(
        &self,
        network: &str,
        competition_contract: &str,
    ) -> DbResult<Vec<OpenCompetitionV2ProofJob>> {
        let rows = sqlx::query(
            r#"
            SELECT * FROM open_competition_v2_proof_jobs
            WHERE network = $1 AND competition_contract = $2
            ORDER BY created_at, id
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(competition_contract))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(open_competition_v2_proof_job_from_row)
            .collect()
    }

    pub async fn lease_next_open_competition_v2_proof_job(
        &self,
        lease_token: Uuid,
        lease_seconds: u32,
    ) -> DbResult<Option<OpenCompetitionV2ProofJob>> {
        if lease_seconds == 0 {
            return Err(DbError::OpenCompetitionV2Conflict(
                "proof job lease must be positive".to_string(),
            ));
        }
        let row = sqlx::query(
            r#"
            WITH candidate AS (
              SELECT id
              FROM open_competition_v2_proof_jobs
              WHERE state IN ('paid', 'proving', 'relaying', 'refund_due')
                AND (lease_expires_at IS NULL OR lease_expires_at <= now())
              ORDER BY
                updated_at,
                CASE state
                  WHEN 'refund_due' THEN 0
                  WHEN 'relaying' THEN 1
                  WHEN 'proving' THEN 2
                  ELSE 3
                END,
                updated_at,
                id
              FOR UPDATE SKIP LOCKED
              LIMIT 1
            )
            UPDATE open_competition_v2_proof_jobs AS job SET
              lease_token = $1,
              lease_expires_at = now() + make_interval(secs => $2),
              attempt_count = attempt_count + 1,
              updated_at = now()
            FROM candidate
            WHERE job.id = candidate.id
            RETURNING job.*
            "#,
        )
        .bind(lease_token)
        .bind(i32::try_from(lease_seconds).map_err(|_| {
            DbError::IntegerOverflow(format!("proof job lease seconds {lease_seconds}"))
        })?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(open_competition_v2_proof_job_from_row).transpose()
    }

    pub async fn release_open_competition_v2_proof_job_lease(
        &self,
        id: Uuid,
        lease_token: Uuid,
    ) -> DbResult<bool> {
        let result = sqlx::query(
            r#"
            UPDATE open_competition_v2_proof_jobs SET
              lease_token = NULL,
              lease_expires_at = NULL,
              updated_at = now()
            WHERE id = $1 AND lease_token = $2
            "#,
        )
        .bind(id)
        .bind(lease_token)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn transition_open_competition_v2_proof_job(
        &self,
        id: Uuid,
        expected: OpenCompetitionV2ProofJobState,
        next: OpenCompetitionV2ProofJobState,
        update: &OpenCompetitionV2ProofJobUpdate,
    ) -> DbResult<OpenCompetitionV2ProofJob> {
        validate_open_competition_v2_proof_transition(expected, next, update)?;
        let row = sqlx::query(
            r#"
            UPDATE open_competition_v2_proof_jobs SET
              state = $3,
              payer = COALESCE($4, payer),
              payment_authorization_nonce = COALESCE($5, payment_authorization_nonce),
              payment_authorization = COALESCE($6, payment_authorization),
              payment_tx_hash = COALESCE($7, payment_tx_hash),
              payment_block_number = COALESCE($8, payment_block_number),
              payment_evidence = COALESCE($9, payment_evidence),
              proof_hash = COALESCE($10, proof_hash),
              public_values_hash = COALESCE($11, public_values_hash),
              proof = COALESCE($12, proof),
              public_values = COALESCE($13, public_values),
              proof_provider_job_id = COALESCE($14, proof_provider_job_id),
              solver_authorization_deadline = COALESCE($15, solver_authorization_deadline),
              solver_signature = COALESCE($16, solver_signature),
              relay_tx_hash = COALESCE($17, relay_tx_hash),
              settlement_event_id = COALESCE($18, settlement_event_id),
              refund_evidence = COALESCE($19, refund_evidence),
              refund_tx_hash = COALESCE($20, refund_tx_hash),
              refund_block_number = COALESCE($21, refund_block_number),
              refund_due_at = COALESCE($22, refund_due_at),
              failure_code = COALESCE($23, failure_code),
              failure_message = COALESCE($24, failure_message),
              updated_at = now()
            WHERE id = $1 AND state = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(expected.storage_name())
        .bind(next.storage_name())
        .bind(update.payer.as_deref().map(normalize_key_address))
        .bind(&update.payment_authorization_nonce)
        .bind(&update.payment_authorization)
        .bind(&update.payment_tx_hash)
        .bind(update.payment_block_number.map(i64_from_u64).transpose()?)
        .bind(&update.payment_evidence)
        .bind(&update.proof_hash)
        .bind(&update.public_values_hash)
        .bind(&update.proof)
        .bind(&update.public_values)
        .bind(&update.proof_provider_job_id)
        .bind(
            update
                .solver_authorization_deadline
                .map(i64_from_u64)
                .transpose()?,
        )
        .bind(&update.solver_signature)
        .bind(&update.relay_tx_hash)
        .bind(update.settlement_event_id)
        .bind(&update.refund_evidence)
        .bind(&update.refund_tx_hash)
        .bind(update.refund_block_number.map(i64_from_u64).transpose()?)
        .bind(update.refund_due_at)
        .bind(&update.failure_code)
        .bind(&update.failure_message)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::OpenCompetitionV2Conflict(id.to_string()))?;
        open_competition_v2_proof_job_from_row(row)
    }

    pub async fn list_unverified_open_competition_event_blocks(
        &self,
        network: &str,
        factory_contract: &str,
        limit: u32,
    ) -> DbResult<Vec<u64>> {
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT block_number
            FROM open_competition_events
            WHERE network = $1 AND factory_contract = $2 AND block_time_verified = FALSE
            ORDER BY block_number
            LIMIT $3
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| u64_from_i64(row.try_get("block_number")?))
            .collect()
    }

    pub async fn confirm_open_competition_event_block_time(
        &self,
        network: &str,
        factory_contract: &str,
        block_number: u64,
        occurred_at: DateTime<Utc>,
    ) -> DbResult<u64> {
        let result = sqlx::query(
            r#"
            UPDATE open_competition_events
            SET occurred_at = $4, block_time_verified = TRUE
            WHERE network = $1 AND factory_contract = $2 AND block_number = $3
            "#,
        )
        .bind(network)
        .bind(normalize_key_address(factory_contract))
        .bind(i64_from_u64(block_number)?)
        .bind(occurred_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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

    pub async fn list_verified_autonomous_bounty_events(
        &self,
        network: &str,
    ) -> DbResult<Vec<AutonomousBountyEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, log_index, contract_address,
                   bounty_id, kind, data, occurred_at
            FROM autonomous_bounty_events
            WHERE network = $1 AND block_time_verified = TRUE
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(autonomous_event_from_row).collect()
    }

    pub async fn list_autonomous_bounty_events_by_transaction(
        &self,
        network: &str,
        transaction_hash: &str,
    ) -> DbResult<Vec<AutonomousBountyEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, log_index, contract_address,
                   bounty_id, kind, data, occurred_at
            FROM autonomous_bounty_events
            WHERE network = $1 AND tx_hash = lower($2)
            ORDER BY block_number, log_index
            "#,
        )
        .bind(network)
        .bind(transaction_hash)
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

fn opportunity_comment_from_row(row: PgRow) -> DbResult<OpportunityComment> {
    Ok(OpportunityComment {
        id: row.try_get("id")?,
        opportunity_id: row.try_get("opportunity_id")?,
        author: row.try_get("author")?,
        body: row.try_get("body")?,
        feedback: row.try_get("feedback")?,
        created_at: row.try_get("created_at")?,
    })
}

fn chatgpt_action_intent_from_row(row: PgRow) -> DbResult<ChatgptActionIntent> {
    Ok(ChatgptActionIntent {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        action: row.try_get("action")?,
        network: row.try_get("network")?,
        opportunity_id: row.try_get("opportunity_id")?,
        bounty_contract: row.try_get("bounty_contract")?,
        bounty_id: row.try_get("bounty_id")?,
        actor_wallet: row.try_get("actor_wallet")?,
        amount_base_units: row
            .try_get::<Option<i64>, _>("amount_base_units")?
            .map(u64_from_i64)
            .transpose()?,
        details: row.try_get("details")?,
        request_fingerprint: row.try_get("request_fingerprint")?,
        status: row.try_get("status")?,
        transaction_hash: row.try_get("transaction_hash")?,
        canonical_event_id: row.try_get("canonical_event_id")?,
        canonical_event_kind: row.try_get("canonical_event_kind")?,
        confirmed_block: row
            .try_get::<Option<i64>, _>("confirmed_block")?
            .map(u64_from_i64)
            .transpose()?,
        expires_at: row.try_get("expires_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn bounty_image_asset_from_row(row: &PgRow) -> DbResult<BountyImageAsset> {
    Ok(BountyImageAsset {
        sha256: row.try_get("sha256")?,
        mime_type: row.try_get("mime_type")?,
        content: row.try_get("content")?,
        created_at: row.try_get("created_at")?,
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

fn parse_open_competition_entrant_relay_status(
    value: String,
) -> DbResult<OpenCompetitionEntrantRelayStatus> {
    match value.as_str() {
        "prepared" => Ok(OpenCompetitionEntrantRelayStatus::Prepared),
        "relaying" => Ok(OpenCompetitionEntrantRelayStatus::Relaying),
        "broadcast" => Ok(OpenCompetitionEntrantRelayStatus::Broadcast),
        "confirmed" => Ok(OpenCompetitionEntrantRelayStatus::Confirmed),
        "failed" => Ok(OpenCompetitionEntrantRelayStatus::Failed),
        other => Err(DbError::InvalidEnum(format!(
            "open-competition entrant relay status {other}"
        ))),
    }
}

fn open_competition_entrant_relay_from_row(row: PgRow) -> DbResult<OpenCompetitionEntrantRelay> {
    Ok(OpenCompetitionEntrantRelay {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        network: row.try_get("network")?,
        wallet: row.try_get("wallet")?,
        bounty_contract: row.try_get("bounty_contract")?,
        delegate: row.try_get("delegate")?,
        action: u8::try_from(row.try_get::<i16, _>("action")?)
            .map_err(|_| DbError::IntegerOverflow("entrant relay action".to_string()))?,
        wallet_nonce: u64_from_i64(row.try_get("wallet_nonce")?)?,
        deadline: u64_from_i64(row.try_get("deadline")?)?,
        payload_hash: row.try_get("payload_hash")?,
        request_fingerprint: row.try_get("request_fingerprint")?,
        relayer_address: row.try_get("relayer_address")?,
        status: parse_open_competition_entrant_relay_status(row.try_get("status")?)?,
        retryable: row.try_get("retryable")?,
        attempt_count: u32::try_from(row.try_get::<i32, _>("attempt_count")?)
            .map_err(|_| DbError::IntegerOverflow("entrant relay attempt count".to_string()))?,
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
        receipt_block: row
            .try_get::<Option<i64>, _>("receipt_block")?
            .map(u64_from_i64)
            .transpose()?,
        receipt_block_hash: row.try_get("receipt_block_hash")?,
        canonical_safe_block: row
            .try_get::<Option<i64>, _>("canonical_safe_block")?
            .map(u64_from_i64)
            .transpose()?,
        canonical_safe_block_hash: row.try_get("canonical_safe_block_hash")?,
        canonical_event: row.try_get("canonical_event")?,
        payment_proven: row.try_get("payment_proven")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_open_competition_entrant_relay_replay(
    persisted: &OpenCompetitionEntrantRelay,
    requested: &NewOpenCompetitionEntrantRelay,
) -> DbResult<()> {
    if persisted.idempotency_key != requested.idempotency_key
        || !persisted.wallet.eq_ignore_ascii_case(&requested.wallet)
        || !persisted
            .bounty_contract
            .eq_ignore_ascii_case(&requested.bounty_contract)
        || !persisted.delegate.eq_ignore_ascii_case(&requested.delegate)
        || persisted.action != requested.action
        || persisted.wallet_nonce != requested.wallet_nonce
        || persisted.deadline != requested.deadline
        || !persisted
            .payload_hash
            .eq_ignore_ascii_case(&requested.payload_hash)
        || persisted.request_fingerprint != requested.request_fingerprint
        || !persisted
            .relayer_address
            .eq_ignore_ascii_case(&requested.relayer_address)
    {
        return Err(DbError::OpenCompetitionEntrantRelayConflict(
            "wallet nonce replay does not match the original request".to_string(),
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

fn open_competition_event_from_row(row: PgRow) -> DbResult<OpenCompetitionEvent> {
    let kind_value = serde_json::Value::String(row.try_get::<String, _>("kind")?);
    let kind: OpenCompetitionEventKind = serde_json::from_value(kind_value)?;
    Ok(OpenCompetitionEvent {
        id: row.try_get("id")?,
        protocol_version: row.try_get("protocol_version")?,
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

fn open_competition_v2_event_from_row(row: PgRow) -> DbResult<OpenCompetitionV2Event> {
    let kind: OpenCompetitionV2EventKind =
        serde_json::from_value(serde_json::Value::String(row.try_get::<String, _>("kind")?))?;
    Ok(OpenCompetitionV2Event {
        id: row.try_get("id")?,
        protocol_version: row.try_get("protocol_version")?,
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

fn open_competition_v2_projection_from_row(
    row: PgRow,
) -> DbResult<OpenCompetitionV2StoredProjection> {
    let parse_amount = |field: &str| -> DbResult<u128> {
        row.try_get::<String, _>(field)?
            .parse::<u128>()
            .map_err(|_| DbError::IntegerOverflow(field.to_string()))
    };
    Ok(OpenCompetitionV2StoredProjection {
        network: row.try_get("network")?,
        factory_contract: row.try_get("factory_contract")?,
        projection: OpenCompetitionV2Projection {
            bounty_id: row.try_get("bounty_id")?,
            competition: row.try_get("competition_contract")?,
            creator: row.try_get("creator")?,
            creation_nonce: row.try_get("creation_nonce")?,
            beta_risk_hash: row.try_get("beta_risk_hash")?,
            state: parse_open_competition_v2_state(row.try_get("state")?)?,
            solver_reward: parse_amount("solver_reward")?,
            keeper_reward: parse_amount("keeper_reward")?,
            funding_deadline: optional_u64_from_i64(row.try_get("funding_deadline")?)?,
            proof_window_seconds: optional_u64_from_i64(row.try_get("proof_window_seconds")?)?,
            winner_mode: row.try_get("winner_mode")?,
            score_direction: row.try_get("score_direction")?,
            score_threshold: row.try_get("score_threshold")?,
            proof_system: row.try_get("proof_system")?,
            verifier_adapter: row.try_get("verifier_adapter")?,
            program_vkey: row.try_get("program_vkey")?,
            source_hash: row.try_get("source_hash")?,
            elf_hash: row.try_get("elf_hash")?,
            journal_schema_hash: row.try_get("journal_schema_hash")?,
            metric_program_hash: row.try_get("metric_program_hash")?,
            execution_policy_hash: row.try_get("execution_policy_hash")?,
            verification_policy_hash: row.try_get("verification_policy_hash")?,
            settlement_policy_hash: row.try_get("settlement_policy_hash")?,
            funded_amount: parse_amount("funded_amount")?,
            proof_deadline: optional_u64_from_i64(row.try_get("proof_deadline")?)?,
            accepted_entries: u64_from_i64(row.try_get("accepted_entries")?)?,
            leader: row.try_get("leader")?,
            winner: row.try_get("winner")?,
            refund_pool_remaining: parse_amount("refund_pool_remaining")?,
            last_block: u64_from_i64(row.try_get("last_block")?)?,
            last_log_index: u64_from_i64(row.try_get("last_log_index")?)?,
        },
        safe_block_number: u64_from_i64(row.try_get("safe_block_number")?)?,
        safe_block_hash: row.try_get("safe_block_hash")?,
    })
}

fn open_competition_v2_state_storage_name(state: OpenCompetitionV2ProjectedState) -> &'static str {
    match state {
        OpenCompetitionV2ProjectedState::Announced => "announced",
        OpenCompetitionV2ProjectedState::Funding => "funding",
        OpenCompetitionV2ProjectedState::Active => "active",
        OpenCompetitionV2ProjectedState::Settled => "settled",
        OpenCompetitionV2ProjectedState::Cancelled => "cancelled",
    }
}

fn parse_open_competition_v2_state(value: String) -> DbResult<OpenCompetitionV2ProjectedState> {
    match value.as_str() {
        "announced" => Ok(OpenCompetitionV2ProjectedState::Announced),
        "funding" => Ok(OpenCompetitionV2ProjectedState::Funding),
        "active" => Ok(OpenCompetitionV2ProjectedState::Active),
        "settled" => Ok(OpenCompetitionV2ProjectedState::Settled),
        "cancelled" => Ok(OpenCompetitionV2ProjectedState::Cancelled),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn open_competition_v2_proof_job_from_row(row: PgRow) -> DbResult<OpenCompetitionV2ProofJob> {
    Ok(OpenCompetitionV2ProofJob {
        id: row.try_get("id")?,
        idempotency_key: row.try_get("idempotency_key")?,
        network: row.try_get("network")?,
        competition_contract: row.try_get("competition_contract")?,
        solver: row.try_get("solver")?,
        solver_nonce: row.try_get("solver_nonce")?,
        artifact_hash: row.try_get("artifact_hash")?,
        program_input: row.try_get("program_input")?,
        expected_public_values: row.try_get("expected_public_values")?,
        requested_relay: row.try_get("requested_relay")?,
        proof_system: row.try_get("proof_system")?,
        state: parse_open_competition_v2_proof_job_state(row.try_get("state")?)?,
        gross_prize: row.try_get("gross_prize")?,
        proof_fee_quote: row.try_get("proof_fee_quote")?,
        relay_fee_quote: row.try_get("relay_fee_quote")?,
        net_prize_if_win: row.try_get("net_prize_if_win")?,
        maximum_charge: row.try_get("maximum_charge")?,
        winner_mode: row.try_get("winner_mode")?,
        competition_risk: row.try_get("competition_risk")?,
        quote_expires_at: row.try_get("quote_expires_at")?,
        proof_sla_deadline: row.try_get("proof_sla_deadline")?,
        payer: row.try_get("payer")?,
        payment_authorization_nonce: row.try_get("payment_authorization_nonce")?,
        payment_authorization: row.try_get("payment_authorization")?,
        payment_tx_hash: row.try_get("payment_tx_hash")?,
        payment_block_number: row
            .try_get::<Option<i64>, _>("payment_block_number")?
            .map(u64_from_i64)
            .transpose()?,
        payment_evidence: row.try_get("payment_evidence")?,
        proof_hash: row.try_get("proof_hash")?,
        public_values_hash: row.try_get("public_values_hash")?,
        proof: row.try_get("proof")?,
        public_values: row.try_get("public_values")?,
        proof_provider_job_id: row.try_get("proof_provider_job_id")?,
        solver_authorization_deadline: row
            .try_get::<Option<i64>, _>("solver_authorization_deadline")?
            .map(u64_from_i64)
            .transpose()?,
        solver_signature: row.try_get("solver_signature")?,
        relay_tx_hash: row.try_get("relay_tx_hash")?,
        settlement_event_id: row.try_get("settlement_event_id")?,
        refund_evidence: row.try_get("refund_evidence")?,
        refund_tx_hash: row.try_get("refund_tx_hash")?,
        refund_block_number: row
            .try_get::<Option<i64>, _>("refund_block_number")?
            .map(u64_from_i64)
            .transpose()?,
        refund_due_at: row.try_get("refund_due_at")?,
        failure_code: row.try_get("failure_code")?,
        failure_message: row.try_get("failure_message")?,
        attempt_count: u32::try_from(row.try_get::<i32, _>("attempt_count")?)
            .map_err(|_| DbError::IntegerOverflow("attempt_count".to_string()))?,
        lease_token: row.try_get("lease_token")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_open_competition_v2_proof_job_state(
    value: String,
) -> DbResult<OpenCompetitionV2ProofJobState> {
    match value.as_str() {
        "quoted" => Ok(OpenCompetitionV2ProofJobState::Quoted),
        "payment_pending" => Ok(OpenCompetitionV2ProofJobState::PaymentPending),
        "paid" => Ok(OpenCompetitionV2ProofJobState::Paid),
        "proving" => Ok(OpenCompetitionV2ProofJobState::Proving),
        "proved" => Ok(OpenCompetitionV2ProofJobState::Proved),
        "relaying" => Ok(OpenCompetitionV2ProofJobState::Relaying),
        "confirmed" => Ok(OpenCompetitionV2ProofJobState::Confirmed),
        "refund_due" => Ok(OpenCompetitionV2ProofJobState::RefundDue),
        "refunded" => Ok(OpenCompetitionV2ProofJobState::Refunded),
        "lost_competition" => Ok(OpenCompetitionV2ProofJobState::LostCompetition),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn open_competition_v2_proof_transition_allowed(
    expected: OpenCompetitionV2ProofJobState,
    next: OpenCompetitionV2ProofJobState,
) -> bool {
    matches!(
        (expected, next),
        (
            OpenCompetitionV2ProofJobState::Quoted,
            OpenCompetitionV2ProofJobState::PaymentPending
        ) | (
            OpenCompetitionV2ProofJobState::PaymentPending,
            OpenCompetitionV2ProofJobState::PaymentPending
        ) | (
            OpenCompetitionV2ProofJobState::PaymentPending,
            OpenCompetitionV2ProofJobState::Paid
        ) | (
            OpenCompetitionV2ProofJobState::PaymentPending,
            OpenCompetitionV2ProofJobState::Quoted
        ) | (
            OpenCompetitionV2ProofJobState::Paid,
            OpenCompetitionV2ProofJobState::Proving
        ) | (
            OpenCompetitionV2ProofJobState::Paid,
            OpenCompetitionV2ProofJobState::RefundDue
        ) | (
            OpenCompetitionV2ProofJobState::Proving,
            OpenCompetitionV2ProofJobState::Proving
        ) | (
            OpenCompetitionV2ProofJobState::Proving,
            OpenCompetitionV2ProofJobState::Proved
        ) | (
            OpenCompetitionV2ProofJobState::Proving,
            OpenCompetitionV2ProofJobState::RefundDue
        ) | (
            OpenCompetitionV2ProofJobState::Proved,
            OpenCompetitionV2ProofJobState::Relaying
        ) | (
            OpenCompetitionV2ProofJobState::Proved,
            OpenCompetitionV2ProofJobState::Confirmed
        ) | (
            OpenCompetitionV2ProofJobState::Proved,
            OpenCompetitionV2ProofJobState::LostCompetition
        ) | (
            OpenCompetitionV2ProofJobState::Proved,
            OpenCompetitionV2ProofJobState::RefundDue
        ) | (
            OpenCompetitionV2ProofJobState::Relaying,
            OpenCompetitionV2ProofJobState::Relaying
        ) | (
            OpenCompetitionV2ProofJobState::Relaying,
            OpenCompetitionV2ProofJobState::Confirmed
        ) | (
            OpenCompetitionV2ProofJobState::Relaying,
            OpenCompetitionV2ProofJobState::RefundDue
        ) | (
            OpenCompetitionV2ProofJobState::Relaying,
            OpenCompetitionV2ProofJobState::LostCompetition
        ) | (
            OpenCompetitionV2ProofJobState::RefundDue,
            OpenCompetitionV2ProofJobState::RefundDue
        ) | (
            OpenCompetitionV2ProofJobState::RefundDue,
            OpenCompetitionV2ProofJobState::Refunded
        )
    )
}

fn validate_open_competition_v2_proof_transition(
    expected: OpenCompetitionV2ProofJobState,
    next: OpenCompetitionV2ProofJobState,
    update: &OpenCompetitionV2ProofJobUpdate,
) -> DbResult<()> {
    if !open_competition_v2_proof_transition_allowed(expected, next) {
        return Err(DbError::OpenCompetitionV2Conflict(format!(
            "illegal proof job transition {} -> {}",
            expected.storage_name(),
            next.storage_name()
        )));
    }
    if next == OpenCompetitionV2ProofJobState::Confirmed && update.settlement_event_id.is_none() {
        return Err(DbError::OpenCompetitionV2Conflict(
            "confirmed proof job requires canonical settlement event".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::RefundDue
        && expected != OpenCompetitionV2ProofJobState::RefundDue
        && update.refund_due_at.is_none()
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "refund_due proof job requires a refund deadline".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::Refunded
        && (update.refund_evidence.is_none()
            || update.refund_tx_hash.is_none()
            || update.refund_block_number.is_none())
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "refunded proof job requires canonical refund evidence".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::PaymentPending
        && (update.payer.is_none() || update.payment_authorization_nonce.is_none())
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "payment_pending proof job requires payer and authorization nonce".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::Paid
        && (update.payment_evidence.is_none()
            || update.payment_tx_hash.is_none()
            || update.payment_block_number.is_none())
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "paid proof job requires canonical payment evidence".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::Proved
        && (update.proof_hash.is_none()
            || update.public_values_hash.is_none()
            || update.proof.is_none()
            || update.public_values.is_none()
            || update.proof_provider_job_id.is_none())
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "proved proof job requires the complete provider-bound proof".to_string(),
        ));
    }
    if next == OpenCompetitionV2ProofJobState::Relaying
        && expected == OpenCompetitionV2ProofJobState::Proved
        && (update.solver_authorization_deadline.is_none() || update.solver_signature.is_none())
    {
        return Err(DbError::OpenCompetitionV2Conflict(
            "relaying proof job requires scoped solver authorization".to_string(),
        ));
    }
    Ok(())
}

fn same_open_competition_v2_quote(
    expected: &OpenCompetitionV2ProofJob,
    actual: &OpenCompetitionV2ProofJob,
) -> bool {
    expected.id == actual.id
        && expected.idempotency_key == actual.idempotency_key
        && expected.network == actual.network
        && expected
            .competition_contract
            .eq_ignore_ascii_case(&actual.competition_contract)
        && expected.solver.eq_ignore_ascii_case(&actual.solver)
        && expected.solver_nonce == actual.solver_nonce
        && expected
            .artifact_hash
            .eq_ignore_ascii_case(&actual.artifact_hash)
        && expected.program_input == actual.program_input
        && expected
            .expected_public_values
            .eq_ignore_ascii_case(&actual.expected_public_values)
        && expected.requested_relay == actual.requested_relay
        && expected.proof_system == actual.proof_system
        && expected.gross_prize == actual.gross_prize
        && expected.proof_fee_quote == actual.proof_fee_quote
        && expected.relay_fee_quote == actual.relay_fee_quote
        && expected.net_prize_if_win == actual.net_prize_if_win
        && expected.maximum_charge == actual.maximum_charge
        && expected.winner_mode == actual.winner_mode
        && expected.quote_expires_at == actual.quote_expires_at
        && expected.proof_sla_deadline == actual.proof_sla_deadline
}

fn autonomous_event_kind_storage_name(kind: AutonomousBountyEventKind) -> &'static str {
    match kind {
        AutonomousBountyEventKind::CanonicalBountyCreated => "canonical_bounty_created",
        AutonomousBountyEventKind::CanonicalBountyTermsCommitted => {
            "canonical_bounty_terms_committed"
        }
        AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured => {
            "canonical_bounty_economics_configured"
        }
        AutonomousBountyEventKind::CanonicalBountyVerificationConfigured => {
            "canonical_bounty_verification_configured"
        }
        AutonomousBountyEventKind::ExternalBountySubmitted => "external_bounty_submitted",
        AutonomousBountyEventKind::FundingAdded => "funding_added",
        AutonomousBountyEventKind::BountyBecameClaimable => "bounty_became_claimable",
        AutonomousBountyEventKind::BountyClaimed => "bounty_claimed",
        AutonomousBountyEventKind::SubmissionAdded => "submission_added",
        AutonomousBountyEventKind::SubmissionRejected => "submission_rejected",
        AutonomousBountyEventKind::BountySettled => "bounty_settled",
        AutonomousBountyEventKind::ClaimExpired => "claim_expired",
        AutonomousBountyEventKind::SubmissionExpired => "submission_expired",
        AutonomousBountyEventKind::BountyCancelled => "bounty_cancelled",
        AutonomousBountyEventKind::RefundWithdrawn => "refund_withdrawn",
    }
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
    fn open_competition_v2_proof_job_graph_is_closed_and_evidence_gated() {
        use OpenCompetitionV2ProofJobState as State;

        let allowed = [
            (State::Quoted, State::PaymentPending),
            (State::PaymentPending, State::PaymentPending),
            (State::PaymentPending, State::Paid),
            (State::PaymentPending, State::Quoted),
            (State::Paid, State::Proving),
            (State::Paid, State::RefundDue),
            (State::Proving, State::Proving),
            (State::Proving, State::Proved),
            (State::Proving, State::RefundDue),
            (State::Proved, State::Relaying),
            (State::Proved, State::Confirmed),
            (State::Proved, State::LostCompetition),
            (State::Proved, State::RefundDue),
            (State::Relaying, State::Relaying),
            (State::Relaying, State::Confirmed),
            (State::Relaying, State::RefundDue),
            (State::Relaying, State::LostCompetition),
            (State::RefundDue, State::RefundDue),
            (State::RefundDue, State::Refunded),
        ];
        for expected in [
            State::Quoted,
            State::PaymentPending,
            State::Paid,
            State::Proving,
            State::Proved,
            State::Relaying,
            State::Confirmed,
            State::RefundDue,
            State::Refunded,
            State::LostCompetition,
        ] {
            for next in [
                State::Quoted,
                State::PaymentPending,
                State::Paid,
                State::Proving,
                State::Proved,
                State::Relaying,
                State::Confirmed,
                State::RefundDue,
                State::Refunded,
                State::LostCompetition,
            ] {
                assert_eq!(
                    open_competition_v2_proof_transition_allowed(expected, next),
                    allowed.contains(&(expected, next)),
                    "unexpected transition {expected:?} -> {next:?}"
                );
            }
        }

        assert!(validate_open_competition_v2_proof_transition(
            State::Quoted,
            State::PaymentPending,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let pending = OpenCompetitionV2ProofJobUpdate {
            payer: Some(format!("0x{}", "11".repeat(20))),
            payment_authorization_nonce: Some(format!("0x{}", "22".repeat(32))),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(
            State::Quoted,
            State::PaymentPending,
            &pending,
        )
        .unwrap();

        assert!(validate_open_competition_v2_proof_transition(
            State::PaymentPending,
            State::Paid,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let paid = OpenCompetitionV2ProofJobUpdate {
            payment_tx_hash: Some(format!("0x{}", "33".repeat(32))),
            payment_block_number: Some(100),
            payment_evidence: Some(serde_json::json!({"log_index": 0})),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(State::PaymentPending, State::Paid, &paid)
            .unwrap();

        assert!(validate_open_competition_v2_proof_transition(
            State::Proving,
            State::Proved,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let proved = OpenCompetitionV2ProofJobUpdate {
            proof_hash: Some(format!("0x{}", "44".repeat(32))),
            public_values_hash: Some(format!("0x{}", "55".repeat(32))),
            proof: Some("0x1234".to_string()),
            public_values: Some(format!("0x{}", "66".repeat(640))),
            proof_provider_job_id: Some("provider-job-1".to_string()),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(State::Proving, State::Proved, &proved)
            .unwrap();

        assert!(validate_open_competition_v2_proof_transition(
            State::Proved,
            State::Relaying,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let relaying = OpenCompetitionV2ProofJobUpdate {
            solver_authorization_deadline: Some(1_900_000_000),
            solver_signature: Some(format!("0x{}", "77".repeat(65))),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(State::Proved, State::Relaying, &relaying)
            .unwrap();

        assert!(validate_open_competition_v2_proof_transition(
            State::Relaying,
            State::Confirmed,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let confirmed = OpenCompetitionV2ProofJobUpdate {
            settlement_event_id: Some(Uuid::new_v4()),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(
            State::Relaying,
            State::Confirmed,
            &confirmed,
        )
        .unwrap();

        let refund_due = OpenCompetitionV2ProofJobUpdate {
            refund_due_at: Some(Utc::now()),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(State::Paid, State::RefundDue, &refund_due)
            .unwrap();
        assert!(validate_open_competition_v2_proof_transition(
            State::Paid,
            State::RefundDue,
            &OpenCompetitionV2ProofJobUpdate::default(),
        )
        .is_err());
        let refund_broadcast = OpenCompetitionV2ProofJobUpdate {
            refund_tx_hash: Some(format!("0x{}", "33".repeat(32))),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(
            State::RefundDue,
            State::RefundDue,
            &refund_broadcast,
        )
        .unwrap();
        let refunded = OpenCompetitionV2ProofJobUpdate {
            refund_evidence: Some(
                serde_json::json!({"transaction_hash": format!("0x{}", "44".repeat(32))}),
            ),
            refund_tx_hash: Some(format!("0x{}", "44".repeat(32))),
            refund_block_number: Some(101),
            ..Default::default()
        };
        validate_open_competition_v2_proof_transition(State::RefundDue, State::Refunded, &refunded)
            .unwrap();
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
    fn competition_activation_analytics_migration_matches_the_api_allowlist() {
        for invariant in [
            "DROP CONSTRAINT IF EXISTS site_analytics_event_name_check",
            "competition_view",
            "competition_instructions_copied",
            "competition_template_copied",
            "competition_child_post_started",
            "competition_feedback_started",
            "competition_feedback_submitted",
            "competition_entry_confirmed",
        ] {
            assert!(
                COMPETITION_ACTIVATION_ANALYTICS_MIGRATION.contains(invariant),
                "missing competition activation analytics invariant {invariant}"
            );
        }
    }

    #[test]
    fn interface_usage_migration_stores_only_hourly_aggregate_attribution() {
        for invariant in [
            "interface_usage_hourly",
            "PRIMARY KEY (bucket_started_at, interface, protocol_era)",
            "interface IN ('api', 'cli', 'mcp')",
            "protocol_era IN ('not_applicable', 'legacy', 'modern', 'http_adapter')",
            "request_count BIGINT NOT NULL",
            "successful_request_count BIGINT NOT NULL",
            "interface_usage_hourly_recent_idx",
        ] {
            assert!(
                INTERFACE_USAGE_MIGRATION.contains(invariant),
                "missing aggregate interface-usage invariant {invariant}"
            );
        }
        for forbidden in [
            "ip_address",
            "user_agent",
            "wallet_address",
            "visitor_id",
            "session_id",
            "client_id",
            "request_body",
            "tool_arguments",
        ] {
            assert!(
                !INTERFACE_USAGE_MIGRATION.contains(forbidden),
                "interface usage must not persist {forbidden}"
            );
        }
    }

    #[test]
    fn external_interface_usage_migration_starts_a_clean_privacy_minimized_epoch() {
        for invariant in [
            "external_interface_usage_hourly",
            "PRIMARY KEY (bucket_started_at, interface, protocol_era)",
            "interface IN ('api', 'cli', 'mcp')",
            "protocol_era IN ('not_applicable', 'legacy', 'modern', 'http_adapter')",
            "request_count BIGINT NOT NULL",
            "successful_request_count BIGINT NOT NULL",
            "external_interface_usage_hourly_recent_idx",
            "verified analytics-exclusion credential",
        ] {
            assert!(
                EXTERNAL_INTERFACE_USAGE_MIGRATION.contains(invariant),
                "missing external interface-usage invariant {invariant}"
            );
        }
        for forbidden in [
            "INSERT INTO external_interface_usage_hourly",
            "SELECT * FROM interface_usage_hourly",
            "ip_address",
            "user_agent",
            "wallet_address",
            "visitor_id",
            "session_id",
            "client_id",
            "request_body",
            "tool_arguments",
        ] {
            assert!(
                !EXTERNAL_INTERFACE_USAGE_MIGRATION.contains(forbidden),
                "external interface usage must not contain or backfill {forbidden}"
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
    fn public_competitor_intelligence_cleanup_drops_only_retired_tables() {
        for table in [
            "competitors",
            "competitor_links",
            "competitor_capabilities",
            "competitor_intelligence_runs",
            "competitor_source_observations",
            "competitor_metric_observations",
            "competitor_intelligence_changes",
        ] {
            assert!(
                PUBLIC_COMPETITOR_INTELLIGENCE_REMOVAL_MIGRATION
                    .contains(&format!("DROP TABLE IF EXISTS {table}")),
                "cleanup must retire {table}"
            );
        }
        assert!(
            !PUBLIC_COMPETITOR_INTELLIGENCE_REMOVAL_MIGRATION.contains("bounties"),
            "cleanup must not affect platform bounty data"
        );
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

    #[test]
    fn opportunity_comments_migration_is_public_bounded_and_idempotent() {
        for invariant in [
            "opportunity_comments",
            "id UUID PRIMARY KEY",
            "opportunity_id TEXT NOT NULL",
            "author TEXT NOT NULL",
            "body TEXT NOT NULL",
            "opportunity_comments_recent_idx",
            "opportunity_id ~ '^[A-Za-z0-9:._-]+$'",
            "length(body) BETWEEN 1 AND 500",
        ] {
            assert!(
                OPPORTUNITY_COMMENTS_MIGRATION.contains(invariant),
                "missing opportunity comment invariant {invariant}"
            );
        }
    }

    #[test]
    fn opportunity_feedback_migration_is_bounded_and_private_evidence_capable() {
        for invariant in [
            "ADD COLUMN IF NOT EXISTS feedback JSONB",
            "jsonb_typeof(feedback) = 'object'",
            "pg_column_size(feedback) <= 4096",
            "'wallet_signature'",
            "'evidence_reference'",
        ] {
            assert!(
                OPPORTUNITY_FEEDBACK_MIGRATION.contains(invariant),
                "missing opportunity feedback invariant {invariant}"
            );
        }
    }

    #[test]
    fn chatgpt_action_intents_are_bounded_idempotent_and_canonical_event_backed() {
        for invariant in [
            "chatgpt_action_intents",
            "idempotency_key TEXT NOT NULL UNIQUE",
            "action IN ('post', 'fund', 'compete', 'complete', 'verify')",
            "status = 'confirmed'",
            "transaction_hash IS NOT NULL",
            "canonical_event_id IS NOT NULL",
            "confirmed_block IS NOT NULL",
            "pg_column_size(details) <= 16384",
            "chatgpt_action_intents_transaction_idx",
        ] {
            assert!(
                CHATGPT_ACTION_INTENTS_MIGRATION.contains(invariant),
                "missing ChatGPT action intent invariant {invariant}"
            );
        }
    }

    #[test]
    fn bounty_image_assets_are_content_addressed_and_bounded() {
        for invariant in [
            "CREATE TABLE IF NOT EXISTS bounty_image_assets",
            "sha256 TEXT PRIMARY KEY",
            "mime_type TEXT NOT NULL",
            "content BYTEA NOT NULL",
            "octet_length(content) BETWEEN 1 AND 5242880",
            "bounty_image_assets_created_idx",
        ] {
            assert!(
                BOUNTY_IMAGE_ASSETS_MIGRATION.contains(invariant),
                "missing bounty image asset invariant {invariant}"
            );
        }
    }

    #[test]
    fn chatgpt_solve_action_migration_renames_legacy_compete_intents() {
        for invariant in [
            "DROP CONSTRAINT IF EXISTS chatgpt_action_intents_action_check",
            "SET action = 'solve'",
            "WHERE action = 'compete'",
            "action IN ('post', 'fund', 'solve', 'complete', 'verify')",
        ] {
            assert!(
                SOLVE_ACTION_RENAME_MIGRATION.contains(invariant),
                "missing ChatGPT solve-action migration invariant {invariant}"
            );
        }
    }

    #[test]
    fn open_competition_v2_migration_is_safe_block_and_refund_complete() {
        for invariant in [
            "protocol_version = 'agent-bounties/open-competition-v2-beta2'",
            "safe_block_number BIGINT NOT NULL CHECK (safe_block_number >= block_number)",
            "UNIQUE (network, factory_contract, log_key)",
            "UNIQUE (network, factory_contract, tx_hash, log_index)",
            "open_competition_v2_projections",
            "open_competition_v2_programs",
            "open_competition_v2_proof_jobs",
            "'refund_due'",
            "refund_due_at TIMESTAMPTZ",
        ] {
            assert!(
                OPEN_COMPETITION_V2_BETA2_MIGRATION.contains(invariant),
                "missing Open Competition V2 persistence invariant {invariant}"
            );
        }

        for protocol in [
            "agent-bounties/open-competition-v2-beta2",
            "agent-bounties/open-competition-v2-beta3",
        ] {
            assert!(OPEN_COMPETITION_V2_BETA3_MIGRATION.contains(protocol));
        }
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn objective_aggregate_compare_and_swap_is_durable() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
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
    async fn chatgpt_action_intent_replays_and_confirms_only_observed_transaction() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let intent_id = Uuid::new_v4();
        let contract = "0x1111111111111111111111111111111111111111";
        let actor = "0x2222222222222222222222222222222222222222";
        let transaction_hash = format!("0x{}", "33".repeat(32));
        let bounty_id = format!("0x{}", "44".repeat(32));
        let request = NewChatgptActionIntent {
            id: intent_id,
            idempotency_key: format!("chatgpt-action-{intent_id}"),
            action: "fund".to_string(),
            network: "base-mainnet".to_string(),
            opportunity_id: Some(format!("canonical_base:base-mainnet:{contract}")),
            bounty_contract: Some(contract.to_string()),
            bounty_id: Some(bounty_id.clone()),
            actor_wallet: None,
            amount_base_units: Some(1_000_000),
            details: serde_json::json!({"title": "Durable action test"}),
            request_fingerprint: "55".repeat(32),
            expires_at: Utc::now() + chrono::Duration::hours(1),
        };
        let created = store.reserve_chatgpt_action_intent(&request).await.unwrap();
        let replay = store.reserve_chatgpt_action_intent(&request).await.unwrap();
        assert_eq!(created, replay);
        assert_eq!(created.status, "review_required");

        let observed = store
            .observe_chatgpt_action_transaction(
                intent_id,
                &ChatgptActionObservation {
                    transaction_hash: transaction_hash.clone(),
                    bounty_contract: Some(contract.to_string()),
                    bounty_id: Some(bounty_id.clone()),
                    actor_wallet: Some(actor.to_string()),
                },
            )
            .await
            .unwrap();
        assert_eq!(observed.status, "pending_confirmation");

        let wrong_event = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("base-mainnet:{}:0", Uuid::new_v4()),
            tx_hash: format!("0x{}", "66".repeat(32)),
            block_number: 100,
            log_index: 0,
            contract_address: contract.to_string(),
            bounty_id: bounty_id.clone(),
            kind: AutonomousBountyEventKind::FundingAdded,
            data: serde_json::json!({"contributor": actor, "amount": 1_000_000}),
            occurred_at: Utc::now(),
        };
        store
            .upsert_autonomous_bounty_event("base-mainnet", &wrong_event)
            .await
            .unwrap();
        assert!(matches!(
            store
                .confirm_chatgpt_action_intent(intent_id, &wrong_event)
                .await,
            Err(DbError::ChatgptActionIntentConflict(_))
        ));

        let matching_event = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("base-mainnet:{}:0", Uuid::new_v4()),
            tx_hash: transaction_hash,
            block_number: 101,
            log_index: 0,
            contract_address: contract.to_string(),
            bounty_id,
            kind: AutonomousBountyEventKind::FundingAdded,
            data: serde_json::json!({"contributor": actor, "amount": 1_000_000}),
            occurred_at: Utc::now(),
        };
        store
            .upsert_autonomous_bounty_event("base-mainnet", &matching_event)
            .await
            .unwrap();
        let confirmed = store
            .confirm_chatgpt_action_intent(intent_id, &matching_event)
            .await
            .unwrap();
        assert_eq!(confirmed.status, "confirmed");
        assert_eq!(confirmed.canonical_event_id, Some(matching_event.id));
        assert_eq!(
            confirmed.canonical_event_kind.as_deref(),
            Some("funding_added")
        );
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn opportunity_comment_round_trip_is_durable_and_idempotent() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let id = Uuid::new_v4();
        let comment = NewOpportunityComment {
            id,
            opportunity_id: "canonical:base-mainnet:0xabc".to_string(),
            author: "Ada".to_string(),
            body: "The acceptance criteria are clear.".to_string(),
            feedback: Some(serde_json::json!({
                "stage": "posting",
                "friction": "The exact funding sequence was hard to find."
            })),
        };
        let created = store
            .create_or_get_opportunity_comment(&comment)
            .await
            .unwrap();
        let replay = store
            .create_or_get_opportunity_comment(&comment)
            .await
            .unwrap();
        assert_eq!(created, replay);
        let conflict = store
            .create_or_get_opportunity_comment(&NewOpportunityComment {
                body: "different content".to_string(),
                ..comment.clone()
            })
            .await;
        assert!(matches!(conflict, Err(DbError::OpportunityCommentConflict)));
        let comments = store
            .list_opportunity_comments(&comment.opportunity_id, 100)
            .await
            .unwrap();
        assert!(comments.iter().any(|item| item.id == id));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn social_mention_ingestion_round_trip_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let id = Uuid::new_v4();
        let mention_seed = id.simple().to_string();
        let mention_id = format!("0x{}{}", mention_seed, &mention_seed[..8]);
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
        let before = store
            .site_analytics_stats(now - chrono::Duration::minutes(1))
            .await
            .unwrap();
        let api_requests_before = before
            .interfaces
            .iter()
            .find(|usage| usage.interface == "api")
            .map(|usage| usage.request_count)
            .unwrap_or(0);
        store
            .record_interface_usage(
                ObservedInterface::Api,
                ObservedProtocolEra::NotApplicable,
                true,
                now,
            )
            .await
            .unwrap();
        store
            .record_interface_usage(
                ObservedInterface::Cli,
                ObservedProtocolEra::NotApplicable,
                false,
                now,
            )
            .await
            .unwrap();
        store
            .record_interface_usage(
                ObservedInterface::Mcp,
                ObservedProtocolEra::McpModern,
                true,
                now,
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .record_interface_usage(
                    ObservedInterface::Api,
                    ObservedProtocolEra::McpModern,
                    true,
                    now,
                )
                .await,
            Err(DbError::InvalidEnum(_))
        ));
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
        let api = stats
            .interfaces
            .iter()
            .find(|usage| usage.interface == "api")
            .expect("API usage is aggregated");
        assert_eq!(api.protocol_era, "not_applicable");
        assert!(api.request_count > api_requests_before);
        assert!(stats.interfaces.iter().any(|usage| {
            usage.interface == "mcp"
                && usage.protocol_era == "modern"
                && usage.successful_request_count >= 1
        }));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn platform_metrics_query_separates_history_from_identity_and_cohort_boundaries() {
        async fn add_event(
            store: &PostgresStore,
            network: &str,
            block_number: u64,
            contract_address: &str,
            bounty_id: &str,
            kind: AutonomousBountyEventKind,
            data: serde_json::Value,
            occurred_at: DateTime<Utc>,
            verified: bool,
        ) {
            store
                .upsert_autonomous_bounty_event(
                    network,
                    &AutonomousBountyEvent {
                        id: Uuid::new_v4(),
                        log_key: format!("{network}:{block_number}:0"),
                        tx_hash: format!("0x{block_number:064x}"),
                        block_number,
                        log_index: 0,
                        contract_address: contract_address.to_string(),
                        bounty_id: bounty_id.to_string(),
                        kind,
                        data,
                        occurred_at,
                    },
                )
                .await
                .unwrap();
            if verified {
                assert_eq!(
                    store
                        .confirm_autonomous_event_block_time(network, block_number, occurred_at)
                        .await
                        .unwrap(),
                    1
                );
            }
        }

        async fn add_competition_event(
            store: &PostgresStore,
            network: &str,
            factory_contract: &str,
            block_number: u64,
            contract_address: &str,
            bounty_id: &str,
            kind: OpenCompetitionEventKind,
            data: serde_json::Value,
            occurred_at: DateTime<Utc>,
            verified: bool,
        ) {
            store
                .upsert_open_competition_event(
                    network,
                    factory_contract,
                    &OpenCompetitionEvent {
                        id: Uuid::new_v4(),
                        protocol_version: "agent-bounties/open-competition-v1".to_string(),
                        log_key: format!("open:{network}:{block_number}:0"),
                        tx_hash: format!("0x{block_number:064x}"),
                        block_number,
                        log_index: 0,
                        contract_address: contract_address.to_string(),
                        bounty_id: bounty_id.to_string(),
                        kind,
                        data,
                        occurred_at,
                    },
                )
                .await
                .unwrap();
            if verified {
                assert_eq!(
                    store
                        .confirm_open_competition_event_block_time(
                            network,
                            factory_contract,
                            block_number,
                            occurred_at,
                        )
                        .await
                        .unwrap(),
                    1
                );
            }
        }

        async fn add_competition_v2_settlement(
            store: &PostgresStore,
            network: &str,
            factory_contract: &str,
            block_number: u64,
            contract_address: &str,
            bounty_id: &str,
            solver: &str,
            occurred_at: DateTime<Utc>,
        ) {
            let block_hash = format!("0x{:064x}", block_number + 1);
            store
                .upsert_open_competition_v2_event(
                    network,
                    factory_contract,
                    &OpenCompetitionV2Event {
                        id: Uuid::new_v4(),
                        protocol_version: chain_base::OPEN_COMPETITION_V2_PROTOCOL_VERSION
                            .to_string(),
                        log_key: format!("v2:{network}:{block_number}:0"),
                        tx_hash: format!("0x{block_number:064x}"),
                        block_number,
                        log_index: 0,
                        contract_address: contract_address.to_string(),
                        bounty_id: bounty_id.to_string(),
                        kind: OpenCompetitionV2EventKind::CompetitionSettled,
                        data: serde_json::json!({
                            "solver": solver,
                            "solver_reward": 3_000_000,
                            "keeper": "0x6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f6f",
                            "keeper_reward": 40_000
                        }),
                        occurred_at,
                    },
                    &OpenCompetitionV2SafeContext {
                        block_hash: block_hash.clone(),
                        safe_block_number: block_number + 10,
                        safe_block_hash: format!("0x{:064x}", block_number + 10),
                    },
                )
                .await
                .unwrap();
        }

        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let test_id = Uuid::new_v4();
        let network = format!("platform-metrics-test-{test_id}");
        let opportunity_id = format!("metrics-test-{test_id}");
        let at = |value: &str| {
            DateTime::parse_from_rfc3339(value)
                .unwrap()
                .with_timezone(&Utc)
        };
        let launch_at = at("2099-01-01T00:00:00Z");
        let previous_started_at = launch_at;
        let selected_started_at = at("2099-01-08T00:00:00Z");
        let selected_ended_at = at("2099-01-15T00:00:00Z");
        let first_month_ended_at = at("2099-02-01T00:00:00Z");
        let contract_one = "0x1111111111111111111111111111111111111111";
        let contract_two = "0x2222222222222222222222222222222222222222";
        let competition_contract = "0x3333333333333333333333333333333333333333";
        let competition_refund_contract = "0x3434343434343434343434343434343434343434";
        let competition_factory = "0x4444444444444444444444444444444444444444";
        let recovery_contract = "0x9999999999999999999999999999999999999999";
        let policy_excluded_contract = "0x9898989898989898989898989898989898989898";
        let funder = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let competition_funder = "0x4545454545454545454545454545454545454545";
        let refunded_funder = "0x4646464646464646464646464646464646464646";
        let competition_solver_one = "0x5555555555555555555555555555555555555555";
        let competition_solver_two = "0x5656565656565656565656565656565656565656";
        let solver_one = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let verifier = "0xcccccccccccccccccccccccccccccccccccccccc";
        let solver_two = "0xdddddddddddddddddddddddddddddddddddddddd";
        let immature_solver = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        let expired_solver = "0xffffffffffffffffffffffffffffffffffffffff";
        let maintainer_wallet = "0x1234512345123451234512345123451234512345";
        let future_expiry = selected_ended_at.timestamp() + 3_600;
        let past_expiry = selected_ended_at.timestamp() - 3_600;
        let mut block = 9_000_000_u64;

        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::CanonicalBountyCreated,
            serde_json::json!({"creator": funder}),
            at("2099-01-04T00:00:00Z"),
            true,
        )
        .await;
        for (offset, solver) in [
            "0x6868686868686868686868686868686868686868",
            "0x7979797979797979797979797979797979797979",
            "0x6868686868686868686868686868686868686868",
            "0x6868686868686868686868686868686868686868",
            "0x6868686868686868686868686868686868686868",
        ]
        .into_iter()
        .enumerate()
        {
            block += 1;
            add_competition_v2_settlement(
                &store,
                &network,
                competition_factory,
                block,
                &format!("0x{:040x}", 0x600_u64 + offset as u64),
                &format!("beta3-{offset}"),
                solver,
                at("2099-01-14T05:00:00Z") + chrono::Duration::minutes(offset as i64),
            )
            .await;
        }

        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_factory,
            "competition-one",
            OpenCompetitionEventKind::CanonicalCompetitionCreated,
            serde_json::json!({
                "bounty_contract": competition_contract,
                "creator": funder
            }),
            at("2099-01-09T03:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_factory,
            "competition-refund",
            OpenCompetitionEventKind::CanonicalCompetitionCreated,
            serde_json::json!({
                "bounty_contract": competition_refund_contract,
                "creator": funder
            }),
            at("2099-01-09T03:00:30Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_contract,
            "competition-one",
            OpenCompetitionEventKind::CanonicalCompetitionVerificationConfigured,
            serde_json::json!({"verifier_reward_recipient": verifier}),
            at("2099-01-09T03:01:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_contract,
            "competition-one",
            OpenCompetitionEventKind::FundingAdded,
            serde_json::json!({"contributor": competition_funder, "amount": 2_400_000}),
            at("2099-01-10T03:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_contract,
            "competition-one",
            OpenCompetitionEventKind::CompetitionSubmissionRejected,
            serde_json::json!({
                "solver": competition_solver_one,
                "bond_paid_to_verifier": 200_000,
                "entry_bond_returned": 8_000_000,
                "refund": 9_000_000
            }),
            at("2099-01-12T03:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_contract,
            "competition-one",
            OpenCompetitionEventKind::BountySettled,
            serde_json::json!({
                "solver": competition_solver_two,
                "solver_reward": 2_000_000,
                "verifier_reward": 200_000,
                "timeout_bond_bonus": 75_000,
                "entry_bond_returned": 200_000,
                "refund": 9_000_000
            }),
            at("2099-01-13T03:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_refund_contract,
            "competition-refund",
            OpenCompetitionEventKind::BountyCancelled,
            serde_json::json!({
                "principal": 500_000_000,
                "expired_entry_bonus": 25_000_000
            }),
            at("2099-01-13T03:30:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_refund_contract,
            "competition-refund",
            OpenCompetitionEventKind::RefundWithdrawn,
            serde_json::json!({
                "contributor": refunded_funder,
                "principal": 500_000_000,
                "expired_entry_bonus": 25_000_000,
                "amount": 525_000_000
            }),
            at("2099-01-13T04:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            competition_contract,
            "competition-unverified",
            OpenCompetitionEventKind::BountySettled,
            serde_json::json!({
                "solver": "0x5757575757575757575757575757575757575757",
                "solver_reward": 70_000_000,
                "verifier_reward": 7_000_000,
                "timeout_bond_bonus": 0
            }),
            at("2099-01-14T03:00:00Z"),
            false,
        )
        .await;
        block += 1;
        add_competition_event(
            &store,
            &network,
            competition_factory,
            block,
            recovery_contract,
            "competition-recovery",
            OpenCompetitionEventKind::BountySettled,
            serde_json::json!({
                "solver": "0x5858585858585858585858585858585858585858",
                "solver_reward": 60_000_000,
                "verifier_reward": 6_000_000,
                "timeout_bond_bonus": 0
            }),
            at("2099-01-14T04:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
            serde_json::json!({"verifier_reward_recipient": verifier}),
            at("2099-01-04T00:01:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::FundingAdded,
            serde_json::json!({"contributor": funder, "amount": 9_999_999}),
            at("2099-01-09T00:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::BountyClaimed,
            serde_json::json!({
                "round": 1,
                "solver": solver_one,
                "claim_expires_at": future_expiry
            }),
            at("2099-01-10T00:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::SubmissionAdded,
            serde_json::json!({"round": 1, "solver": solver_one}),
            at("2099-01-10T01:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_one,
            "bounty-one",
            AutonomousBountyEventKind::BountySettled,
            serde_json::json!({
                "round": 1,
                "solver": solver_one,
                "solver_reward": 1_000_000,
                "verifier_reward": 100_000,
                "timeout_bond_bonus": 50_000,
                "solver_payout": 1_050_000,
                "returned_claim_bond": 100_000,
                "refund": 4_000_000,
                "leaderboard_prize": 2_000_000
            }),
            at("2099-01-11T00:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "bounty-two",
            AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
            serde_json::json!({"verifier_reward_recipient": verifier}),
            at("2099-01-09T00:01:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "bounty-two",
            AutonomousBountyEventKind::BountyClaimed,
            serde_json::json!({
                "round": 2,
                "solver": solver_two,
                "claim_expires_at": future_expiry
            }),
            at("2099-01-10T02:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "bounty-two",
            AutonomousBountyEventKind::SubmissionRejected,
            serde_json::json!({
                "round": 2,
                "solver": solver_two,
                "verifier_reward": 100_000,
                "forfeited_bond": 100_000,
                "refund": 3_000_000
            }),
            at("2099-01-12T00:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "bounty-three",
            AutonomousBountyEventKind::BountyClaimed,
            serde_json::json!({
                "round": 3,
                "solver": immature_solver,
                "claim_expires_at": future_expiry
            }),
            at("2099-01-13T00:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "bounty-four",
            AutonomousBountyEventKind::BountyClaimed,
            serde_json::json!({
                "round": 4,
                "solver": expired_solver,
                "claim_expires_at": past_expiry
            }),
            at("2099-01-13T01:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "maintainer-bounty",
            AutonomousBountyEventKind::CanonicalBountyCreated,
            serde_json::json!({"creator": maintainer_wallet}),
            at("2099-01-13T02:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            contract_two,
            "unverified-bounty",
            AutonomousBountyEventKind::BountySettled,
            serde_json::json!({
                "round": 1,
                "solver": "0x7777777777777777777777777777777777777777",
                "solver_reward": 90_000_000,
                "verifier_reward": 10_000_000,
                "timeout_bond_bonus": 0
            }),
            at("2099-01-14T00:00:00Z"),
            false,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            recovery_contract,
            "recovery-bounty",
            AutonomousBountyEventKind::BountySettled,
            serde_json::json!({
                "round": 1,
                "solver": "0x6666666666666666666666666666666666666666",
                "solver_reward": 80_000_000,
                "verifier_reward": 10_000_000,
                "timeout_bond_bonus": 0
            }),
            at("2099-01-14T01:00:00Z"),
            true,
        )
        .await;
        block += 1;
        add_event(
            &store,
            &network,
            block,
            policy_excluded_contract,
            "policy-excluded-bounty",
            AutonomousBountyEventKind::BountySettled,
            serde_json::json!({
                "round": 1,
                "solver": "0x6767676767676767676767676767676767676767",
                "solver_reward": 45_000_000,
                "verifier_reward": 5_000_000,
                "timeout_bond_bonus": 0
            }),
            at("2099-01-14T01:30:00Z"),
            true,
        )
        .await;

        for (author, minute) in [
            ("  Alice   Agent  ", 0_i64),
            ("alice agent", 1_i64),
            ("Maintainer", 2_i64),
        ] {
            sqlx::query(
                "INSERT INTO opportunity_comments (id, opportunity_id, author, body, created_at) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(Uuid::new_v4())
            .bind(&opportunity_id)
            .bind(author)
            .bind("metrics test")
            .bind(at("2099-01-14T02:00:00Z") + chrono::Duration::minutes(minute))
            .execute(&store.pool)
            .await
            .unwrap();
        }

        let stats = store
            .platform_metrics_stats(
                &network,
                selected_started_at,
                selected_ended_at,
                previous_started_at,
                launch_at,
                first_month_ended_at,
                &[maintainer_wallet.to_string()],
                &["maintainer".to_string()],
                &[policy_excluded_contract.to_string()],
            )
            .await
            .unwrap();

        assert_eq!(stats.identities.selected, 15);
        assert_eq!(stats.identities.previous, 1);
        assert_eq!(stats.identities.lifetime, 15);
        assert_eq!(stats.identities.posters, 1);
        assert_eq!(stats.identities.funders, 3);
        assert_eq!(stats.identities.solvers, 10);
        assert_eq!(stats.identities.verifiers, 1);
        assert_eq!(stats.identities.commenters, 1);
        assert_eq!(stats.identities.marketplace_wallets, 14);
        assert_eq!(stats.identities.opportunity_comment_authors, 1);
        // The recovery-reserved contract remains in immutable history, while the
        // separately declared policy contract is excluded from every series.
        assert_eq!(stats.payouts.selected_total_base_units, "174925000");
        assert_eq!(stats.payouts.selected_solver_base_units, "158000000");
        assert_eq!(stats.payouts.selected_verifier_base_units, "16600000");
        assert_eq!(stats.payouts.selected_keeper_base_units, "200000");
        assert_eq!(stats.payouts.selected_bonus_base_units, "125000");
        assert_eq!(stats.payouts.selected_settled_rounds, 9);
        assert_eq!(stats.claim_cohort.settled, 1);
        assert_eq!(stats.claim_cohort.mature, 3);
        assert_eq!(stats.claim_cohort.immature, 1);
        assert_eq!(stats.coverage.awaiting_block_time_events, 2);
        assert_eq!(
            stats
                .daily
                .iter()
                .map(|day| day.payout_base_units.parse::<u128>().unwrap())
                .sum::<u128>(),
            174_925_000
        );

        let growth = store
            .platform_demand_growth_stats(
                &network,
                selected_ended_at,
                launch_at,
                &[maintainer_wallet.to_string()],
                &[policy_excluded_contract.to_string()],
            )
            .await
            .unwrap();
        assert_eq!(growth.gmv_7d_base_units, "174625000");
        assert_eq!(growth.gmv_28d_base_units, "174625000");
        assert_eq!(growth.lifetime_gmv_base_units, "174625000");
        assert_eq!(growth.new_poster_funder_wallets_28d, 2);
        assert_eq!(growth.active_poster_funder_wallets_28d, 2);
        assert_eq!(growth.repeat_poster_funder_wallets_28d, 1);
        // The five V2 settlement fixtures intentionally omit FundingAddedV2.
        // GMV remains canonical, while the external-funding share must be
        // withheld by the API because only 3.425 USDC is attributable.
        assert_eq!(growth.non_operator_attributed_gmv_28d_base_units, "3425000");
        assert_eq!(growth.attributed_gmv_28d_base_units, "3425000");

        sqlx::query("DELETE FROM open_competition_v2_events WHERE network = $1")
            .bind(&network)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM open_competition_events WHERE network = $1")
            .bind(&network)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM autonomous_bounty_events WHERE network = $1")
            .bind(&network)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM opportunity_comments WHERE opportunity_id = $1")
            .bind(&opportunity_id)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn opportunity_lifecycle_query_executes_against_migrated_postgres() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let stats = store
            .opportunity_lifecycle_stats(Utc::now() - chrono::Duration::hours(1), &[])
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
    async fn open_competition_entrant_relay_is_secret_free_idempotent_and_quota_bounded() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let network = format!("entrant-relay-test-{}", Uuid::new_v4());
        let relay = NewOpenCompetitionEntrantRelay {
            id: Uuid::new_v4(),
            idempotency_key: format!("entrant-relay-{}", Uuid::new_v4()),
            network: network.clone(),
            wallet: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_contract: "0x2222222222222222222222222222222222222222".to_string(),
            delegate: "0x3333333333333333333333333333333333333333".to_string(),
            action: 0,
            wallet_nonce: 7,
            deadline: 2_000_000_000,
            payload_hash: format!("0x{}", "44".repeat(32)),
            request_fingerprint: "fingerprint-a".to_string(),
            relayer_address: "0x5555555555555555555555555555555555555555".to_string(),
        };
        let first = store
            .reserve_open_competition_entrant_relay(&relay, 2, 1)
            .await
            .unwrap();
        let replay = store
            .reserve_open_competition_entrant_relay(&relay, 2, 1)
            .await
            .unwrap();
        assert_eq!(first.id, replay.id);

        let mut conflict = relay.clone();
        conflict.id = Uuid::new_v4();
        conflict.request_fingerprint = "fingerprint-b".to_string();
        assert!(matches!(
            store
                .reserve_open_competition_entrant_relay(&conflict, 2, 1)
                .await,
            Err(DbError::OpenCompetitionEntrantRelayConflict(_))
        ));

        let mut wallet_quota = relay.clone();
        wallet_quota.id = Uuid::new_v4();
        wallet_quota.idempotency_key = format!("entrant-relay-{}", Uuid::new_v4());
        wallet_quota.wallet_nonce = 8;
        wallet_quota.payload_hash = format!("0x{}", "66".repeat(32));
        wallet_quota.request_fingerprint = "fingerprint-wallet-quota".to_string();
        assert!(matches!(
            store
                .reserve_open_competition_entrant_relay(&wallet_quota, 2, 1)
                .await,
            Err(DbError::OpenCompetitionEntrantRelayQuotaExceeded(_))
        ));

        let mut second = wallet_quota;
        second.wallet = "0x7777777777777777777777777777777777777777".to_string();
        second.request_fingerprint = "fingerprint-second".to_string();
        store
            .reserve_open_competition_entrant_relay(&second, 2, 1)
            .await
            .unwrap();

        let lease = store
            .acquire_x402_relayer_lease(&network, 30)
            .await
            .unwrap()
            .unwrap();
        let claimed = store
            .claim_open_competition_entrant_relay(first.id, lease, 30)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, OpenCompetitionEntrantRelayStatus::Relaying);
        let broadcast = store
            .mark_open_competition_entrant_relay_broadcast(
                first.id,
                lease,
                &format!("0x{}", "88".repeat(32)),
                90_000,
                120_000,
            )
            .await
            .unwrap();
        assert_eq!(
            broadcast.status,
            OpenCompetitionEntrantRelayStatus::Broadcast
        );
        store
            .release_x402_relayer_lease(&network, lease)
            .await
            .unwrap();
        let confirmed = store
            .mark_open_competition_entrant_relay_confirmed(
                first.id,
                123,
                &format!("0x{}", "99".repeat(32)),
                130,
                &format!("0x{}", "aa".repeat(32)),
                "SolutionCommitted",
                false,
            )
            .await
            .unwrap();
        assert_eq!(
            confirmed.status,
            OpenCompetitionEntrantRelayStatus::Confirmed
        );
        assert!(!confirmed.retryable);
        assert!(!confirmed.payment_proven);

        let recovery_network = format!("entrant-relay-recovery-{}", Uuid::new_v4());
        let mut failed = relay.clone();
        failed.id = Uuid::new_v4();
        failed.idempotency_key = format!("entrant-relay-{}", Uuid::new_v4());
        failed.network = recovery_network.clone();
        failed.request_fingerprint = "fingerprint-retryable-failure".to_string();
        let failed = store
            .reserve_open_competition_entrant_relay(&failed, 3, 3)
            .await
            .unwrap();
        let recovery_lease = store
            .acquire_x402_relayer_lease(&recovery_network, 30)
            .await
            .unwrap()
            .unwrap();
        store
            .claim_open_competition_entrant_relay(failed.id, recovery_lease, 30)
            .await
            .unwrap()
            .unwrap();
        store
            .mark_open_competition_entrant_relay_failed(
                failed.id,
                Some(recovery_lease),
                true,
                "temporary_provider_failure",
                "retry the exact request",
            )
            .await
            .unwrap();
        store
            .release_x402_relayer_lease(&recovery_network, recovery_lease)
            .await
            .unwrap();

        let mut replacement = relay.clone();
        replacement.id = Uuid::new_v4();
        replacement.idempotency_key = format!("entrant-relay-{}", Uuid::new_v4());
        replacement.network = recovery_network;
        replacement.payload_hash = format!("0x{}", "77".repeat(32));
        replacement.request_fingerprint = "fingerprint-corrected-action".to_string();
        assert!(matches!(
            store
                .reserve_open_competition_entrant_relay(&replacement, 3, 3)
                .await,
            Err(DbError::OpenCompetitionEntrantRelayConflict(_))
        ));
        store
            .mark_open_competition_entrant_relay_failed(
                failed.id,
                None,
                false,
                "transaction_reverted",
                "wallet nonce was not consumed",
            )
            .await
            .unwrap();
        let recovered = store
            .reserve_open_competition_entrant_relay(&replacement, 3, 3)
            .await
            .unwrap();
        assert_eq!(recovered.wallet_nonce, failed.wallet_nonce);
        assert_ne!(recovered.id, failed.id);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn claim_funnel_counts_direct_and_atomic_sponsored_confirmations() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let baseline = store.claim_funnel_stats(1, &[]).await.unwrap();
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

        let observed = store.claim_funnel_stats(1, &[]).await.unwrap();
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

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn site_auth_verified_merge_actions_and_reset_are_transactional() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let unique = Uuid::new_v4().simple().to_string();
        let email = format!("auth-{unique}@example.com");
        let account_a = hex::encode(Sha256::digest(format!("a-{unique}").as_bytes()));
        let account_b = hex::encode(Sha256::digest(format!("b-{unique}").as_bytes()));
        let account_c = hex::encode(Sha256::digest(format!("c-{unique}").as_bytes()));
        let account_d = hex::encode(Sha256::digest(format!("d-{unique}").as_bytes()));

        let google_account = store
            .upsert_site_auth_identity(
                &account_a,
                "google",
                &format!("google-{unique}"),
                "Verified Owner",
                &email,
                "",
                Some((&email, &email)),
            )
            .await
            .unwrap();
        assert_eq!(google_account, account_a);
        let wallet = format!("0x{:0>40}", unique);
        store
            .link_site_auth_wallet(&account_a, &wallet, 8453)
            .await
            .unwrap();

        let microsoft_account = store
            .upsert_site_auth_identity(
                &account_b,
                "microsoft",
                &format!("microsoft-{unique}"),
                "Unverified Claim",
                &email,
                "",
                None,
            )
            .await
            .unwrap();
        assert_eq!(microsoft_account, account_b);
        let github_subject = format!("github-{unique}");
        let unverified_github = store
            .upsert_site_auth_identity(
                &account_c,
                "github",
                &github_subject,
                "Pending GitHub Owner",
                &email,
                "",
                None,
            )
            .await
            .unwrap();
        assert_eq!(unverified_github, account_c);
        let merged_wallet = format!("0x{:0>40}", format!("c{unique}"));
        store
            .link_site_auth_wallet(&account_c, &merged_wallet, 8453)
            .await
            .unwrap();
        let merged_session = hex::encode(Sha256::digest(format!("merged-session-{unique}")));
        store
            .create_site_auth_session(
                &merged_session,
                &account_c,
                "github",
                Utc::now() + chrono::Duration::hours(8),
            )
            .await
            .unwrap();

        let merged = store
            .upsert_site_auth_identity(
                &account_c,
                "github",
                &github_subject,
                "Verified Owner",
                &email,
                "",
                Some((&email, &email)),
            )
            .await
            .unwrap();
        assert_eq!(merged, account_a);
        assert_eq!(
            store
                .list_site_auth_wallets(&account_a)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store
                .site_auth_principal_for_session(&merged_session)
                .await
                .unwrap()
                .unwrap()
                .account_key,
            account_a
        );
        assert_eq!(
            store
                .upsert_site_auth_identity(
                    &account_c,
                    "github",
                    &github_subject,
                    "Verified Owner",
                    &email,
                    "",
                    Some((&email, &email)),
                )
                .await
                .unwrap(),
            account_a
        );

        let expired_token = hex::encode(Sha256::digest(format!("expired-{unique}")));
        store
            .insert_site_auth_email_action(
                &expired_token,
                "registration",
                &email,
                &email,
                Some(&account_a),
                Utc::now() + chrono::Duration::minutes(1),
                &format!("expired-{unique}"),
            )
            .await
            .unwrap();
        sqlx::query(
            "UPDATE site_auth_email_actions SET created_at = NOW() - INTERVAL '2 seconds', expires_at = NOW() - INTERVAL '1 second' WHERE token_hash = $1",
        )
        .bind(&expired_token)
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(store
            .verify_site_auth_email_action(
                &expired_token,
                "registration",
                &hex::encode(Sha256::digest(format!("expired-setup-{unique}"))),
            )
            .await
            .unwrap()
            .is_none());

        let expired_session = hex::encode(Sha256::digest(format!("expired-session-{unique}")));
        store
            .create_site_auth_session(
                &expired_session,
                &account_a,
                "password",
                Utc::now() + chrono::Duration::minutes(1),
            )
            .await
            .unwrap();
        sqlx::query(
            "UPDATE site_auth_sessions SET created_at = NOW() - INTERVAL '2 seconds', expires_at = NOW() - INTERVAL '1 second', last_seen_at = NOW() - INTERVAL '2 seconds' WHERE token_hash = $1",
        )
        .bind(&expired_session)
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(store
            .site_auth_principal_for_session(&expired_session)
            .await
            .unwrap()
            .is_none());

        let rollback_subject = format!("rollback-{unique}");
        store
            .upsert_site_auth_identity(
                &account_d,
                "microsoft",
                &rollback_subject,
                "Rollback Owner",
                "rollback@example.com",
                "",
                None,
            )
            .await
            .unwrap();
        let mut failed_merge = store.pool.begin().await.unwrap();
        assert!(
            merge_site_auth_accounts(&mut failed_merge, &account_d, &"f".repeat(64),)
                .await
                .is_err()
        );
        failed_merge.rollback().await.unwrap();
        let rollback_owner: String = sqlx::query_scalar(
            "SELECT account_key FROM site_auth_identities WHERE provider = 'microsoft' AND provider_subject = $1",
        )
        .bind(&rollback_subject)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(rollback_owner, account_d);

        let registration_token = hex::encode(Sha256::digest(format!("registration-{unique}")));
        store
            .insert_site_auth_email_action(
                &registration_token,
                "registration",
                &email,
                &email,
                Some(&account_a),
                Utc::now() + chrono::Duration::hours(8),
                &format!("registration-{unique}"),
            )
            .await
            .unwrap();
        let registration_setup = hex::encode(Sha256::digest(format!("setup-{unique}")));
        assert!(
            store
                .verify_site_auth_email_action(
                    &registration_token,
                    "registration",
                    &registration_setup,
                )
                .await
                .unwrap()
                .is_some()
        );
        assert!(store
            .verify_site_auth_email_action(
                &registration_token,
                "registration",
                &hex::encode(Sha256::digest(format!("replay-{unique}"))),
            )
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            store
                .complete_site_auth_password_action(
                    &registration_setup,
                    "registration",
                    &account_c,
                    "Verified Owner",
                    "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                )
                .await
                .unwrap(),
            Some(account_a.clone())
        );

        let old_session = hex::encode(Sha256::digest(format!("session-{unique}")));
        store
            .create_site_auth_session(
                &old_session,
                &account_a,
                "password",
                Utc::now() + chrono::Duration::hours(8),
            )
            .await
            .unwrap();
        assert!(store
            .site_auth_principal_for_session(&old_session)
            .await
            .unwrap()
            .is_some());

        let reset_token = hex::encode(Sha256::digest(format!("reset-{unique}")));
        let reset_setup = hex::encode(Sha256::digest(format!("reset-setup-{unique}")));
        store
            .insert_site_auth_email_action(
                &reset_token,
                "reset",
                &email,
                &email,
                Some(&account_a),
                Utc::now() + chrono::Duration::minutes(30),
                &format!("reset-{unique}"),
            )
            .await
            .unwrap();
        store
            .verify_site_auth_email_action(&reset_token, "reset", &reset_setup)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store
                .complete_site_auth_password_action(
                    &reset_setup,
                    "reset",
                    &account_c,
                    "",
                    "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                )
                .await
                .unwrap(),
            Some(account_a)
        );
        assert!(store
            .site_auth_principal_for_session(&old_session)
            .await
            .unwrap()
            .is_none());
    }
}
