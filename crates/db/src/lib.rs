use chain_base::{BaseEscrowEvent, BaseEscrowEventKind};
use chrono::{DateTime, Utc};
use domain::{
    Agent, AgentStatus, AudienceInteraction, AudienceInteractionKind, AudienceLifecycleStage,
    AudienceMember, AudienceProvider, Bounty, BountyStatus, Capability, CapabilityClass, Claim,
    ContributorContact, DiscoveryResponse, Escrow, EscrowStatus, EvalRun, FundingContribution,
    FundingContributionStatus, FundingIntent, FundingIntentStatus, FundingMode, HelpRequest, Id,
    Money, OutreachAttempt, OutreachChannel, OutreachStatus, PaymentEvent, PaymentEventStatus,
    PaymentRail, PrivacyLevel, ProofRecord, Quote, ReputationEvent, RiskAction, RiskEvent,
    RiskReviewOutcome, RiskReviewRecord, RiskSurface, Settlement, Submission, TemplateSignal,
    VerificationDecision, VerifierKind, VerifierResult,
};
use ledger::{LedgerEntry, Posting};
use sqlx::{postgres::PgRow, PgPool, Row};
use std::collections::HashMap;
use thiserror::Error;

pub const CORE_MIGRATION: &str = include_str!("../../../migrations/0001_core.sql");
const MIGRATION_ADVISORY_LOCK_ID: i64 = 4_270_265_017;
const BASE_RELEASE_ATTESTATION_STATUS_LIMIT: i64 = 25;
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
    #[error("immutable attestation conflict: {0}")]
    ImmutableAttestationConflict(String),
    #[error("conflicting audience event replay: {0}")]
    AudienceConflict(String),
}

pub type DbResult<T> = Result<T, DbError>;

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

#[derive(Debug, Clone)]
pub struct BountyStatusScope {
    pub bounty: Bounty,
    pub funding_intents: Vec<FundingIntent>,
    pub funding_contributions: Vec<FundingContribution>,
    pub escrows: Vec<Escrow>,
    pub base_escrow_events: Vec<BaseEscrowEvent>,
    pub base_release_attestations: Vec<BaseReleaseAttestationRecord>,
    pub claims: Vec<Claim>,
    pub submissions: Vec<Submission>,
    pub verifier_results: Vec<VerifierResult>,
    pub proofs: Vec<ProofRecord>,
    pub settlements: Vec<Settlement>,
    pub reputation_events: Vec<ReputationEvent>,
    pub template_signals: Vec<TemplateSignal>,
    pub risk_events: Vec<RiskEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BaseReleaseAttestationRecord {
    pub id: Id,
    pub network: String,
    pub tx_hash: String,
    pub log_key: String,
    pub bounty_id: Id,
    pub onchain_escrow_id: String,
    pub calldata_hash: Option<String>,
    pub proof_hash: Option<String>,
    pub recipients: serde_json::Value,
    pub escrow_contract: String,
    pub settlement_signer: String,
    pub platform_fee_wallet: Option<String>,
    pub verdict: String,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub struct BaseLogPersistenceCursor<'a> {
    pub network: &'a str,
    pub escrow_contract: &'a str,
    pub last_scanned_block: u64,
    pub last_log_key: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct BaseLogPersistenceBatch<'a> {
    pub bounties: &'a [Bounty],
    pub release_attestations: &'a [BaseReleaseAttestationRecord],
    pub funding_intents: &'a [FundingIntent],
    pub escrows: &'a [Escrow],
    pub settlements: &'a [Settlement],
    pub ledger_entries: &'a [LedgerEntry],
    pub base_escrow_events: &'a [BaseEscrowEvent],
    pub cursor: Option<BaseLogPersistenceCursor<'a>>,
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
            for statement in CORE_MIGRATION
                .split(';')
                .map(str::trim)
                .filter(|statement| !statement.is_empty())
            {
                sqlx::query(statement).execute(&mut *connection).await?;
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

        let base_escrow_events = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, onchain_escrow_id, bounty_id, kind, status, token, amount, currency, terms_hash, proof_hash, reason_hash, dispute_hash, occurred_at
            FROM base_escrow_events
            WHERE bounty_id = $1
            ORDER BY block_number, COALESCE(log_index, 0), occurred_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(base_escrow_event_from_row)
        .collect::<DbResult<Vec<_>>>()?;

        let base_release_attestations = sqlx::query(
            r#"
            SELECT id, network, tx_hash, log_key, bounty_id, onchain_escrow_id, calldata_hash, proof_hash, recipients, escrow_contract, settlement_signer, platform_fee_wallet, verdict, reason, created_at
            FROM base_release_attestations
            WHERE bounty_id = $1
            ORDER BY created_at DESC, tx_hash DESC, log_key DESC
            LIMIT $2
            "#,
        )
        .bind(bounty_id)
        .bind(BASE_RELEASE_ATTESTATION_STATUS_LIMIT)
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .map(base_release_attestation_from_row)
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
            base_escrow_events,
            base_release_attestations,
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

    pub async fn upsert_base_escrow_event(&self, event: &BaseEscrowEvent) -> DbResult<()> {
        let log_index = log_index_from_key(&event.log_key)
            .map(i64_from_u64)
            .transpose()?;
        sqlx::query(
            r#"
            INSERT INTO base_escrow_events
              (id, log_key, tx_hash, block_number, log_index, onchain_escrow_id, bounty_id, kind, status, token, amount, currency, terms_hash, proof_hash, reason_hash, dispute_hash, occurred_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
            ON CONFLICT (log_key) DO UPDATE SET
              tx_hash = EXCLUDED.tx_hash,
              block_number = EXCLUDED.block_number,
              log_index = EXCLUDED.log_index,
              onchain_escrow_id = EXCLUDED.onchain_escrow_id,
              bounty_id = EXCLUDED.bounty_id,
              kind = EXCLUDED.kind,
              status = EXCLUDED.status,
              token = EXCLUDED.token,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              terms_hash = EXCLUDED.terms_hash,
              proof_hash = EXCLUDED.proof_hash,
              reason_hash = EXCLUDED.reason_hash,
              dispute_hash = EXCLUDED.dispute_hash,
              occurred_at = EXCLUDED.occurred_at
            "#,
        )
        .bind(event.id)
        .bind(&event.log_key)
        .bind(&event.tx_hash)
        .bind(i64_from_u64(event.block_number)?)
        .bind(log_index)
        .bind(event.onchain_escrow_id.to_string())
        .bind(event.bounty_id)
        .bind(format!("{:?}", event.kind))
        .bind(format!("{:?}", event.status))
        .bind(&event.token)
        .bind(event.amount.as_ref().map(|amount| amount.amount))
        .bind(event.amount.as_ref().map(|amount| amount.currency.clone()))
        .bind(&event.terms_hash)
        .bind(&event.proof_hash)
        .bind(&event.reason_hash)
        .bind(&event.dispute_hash)
        .bind(event.occurred_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_base_escrow_events(&self) -> DbResult<Vec<BaseEscrowEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, onchain_escrow_id, bounty_id, kind, status, token, amount, currency, terms_hash, proof_hash, reason_hash, dispute_hash, occurred_at
            FROM base_escrow_events
            ORDER BY block_number, COALESCE(log_index, 0), occurred_at
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(base_escrow_event_from_row).collect()
    }

    pub async fn list_base_escrow_events_for_bounty(
        &self,
        bounty_id: Id,
    ) -> DbResult<Vec<BaseEscrowEvent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, log_key, tx_hash, block_number, onchain_escrow_id, bounty_id, kind, status, token, amount, currency, terms_hash, proof_hash, reason_hash, dispute_hash, occurred_at
            FROM base_escrow_events
            WHERE bounty_id = $1
            ORDER BY block_number, COALESCE(log_index, 0), occurred_at
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(base_escrow_event_from_row).collect()
    }

    pub async fn upsert_base_release_attestation(
        &self,
        record: &BaseReleaseAttestationRecord,
    ) -> DbResult<()> {
        let result = sqlx::query(
            r#"
            INSERT INTO base_release_attestations
              (id, network, tx_hash, log_key, bounty_id, onchain_escrow_id, calldata_hash, proof_hash, recipients, escrow_contract, settlement_signer, platform_fee_wallet, verdict, reason, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (network, tx_hash, log_key) DO UPDATE SET
              id = base_release_attestations.id
            WHERE base_release_attestations.bounty_id = EXCLUDED.bounty_id
              AND base_release_attestations.onchain_escrow_id = EXCLUDED.onchain_escrow_id
              AND base_release_attestations.calldata_hash IS NOT DISTINCT FROM EXCLUDED.calldata_hash
              AND base_release_attestations.proof_hash IS NOT DISTINCT FROM EXCLUDED.proof_hash
              AND base_release_attestations.recipients = EXCLUDED.recipients
              AND base_release_attestations.escrow_contract = EXCLUDED.escrow_contract
              AND base_release_attestations.settlement_signer = EXCLUDED.settlement_signer
              AND base_release_attestations.platform_fee_wallet IS NOT DISTINCT FROM EXCLUDED.platform_fee_wallet
              AND base_release_attestations.verdict = EXCLUDED.verdict
              AND base_release_attestations.reason = EXCLUDED.reason
            "#,
        )
        .bind(record.id)
        .bind(&record.network)
        .bind(&record.tx_hash)
        .bind(&record.log_key)
        .bind(record.bounty_id)
        .bind(&record.onchain_escrow_id)
        .bind(&record.calldata_hash)
        .bind(&record.proof_hash)
        .bind(&record.recipients)
        .bind(&record.escrow_contract)
        .bind(&record.settlement_signer)
        .bind(&record.platform_fee_wallet)
        .bind(&record.verdict)
        .bind(&record.reason)
        .bind(record.created_at)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(DbError::ImmutableAttestationConflict(format!(
                "{} {} {}",
                record.network, record.tx_hash, record.log_key
            )));
        }
        Ok(())
    }

    pub async fn list_base_release_attestations_for_bounty(
        &self,
        bounty_id: Id,
    ) -> DbResult<Vec<BaseReleaseAttestationRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, network, tx_hash, log_key, bounty_id, onchain_escrow_id, calldata_hash, proof_hash, recipients, escrow_contract, settlement_signer, platform_fee_wallet, verdict, reason, created_at
            FROM base_release_attestations
            WHERE bounty_id = $1
            ORDER BY created_at, tx_hash, log_key
            "#,
        )
        .bind(bounty_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(base_release_attestation_from_row)
            .collect()
    }

    pub async fn list_base_release_attestations_for_bounty_limited(
        &self,
        bounty_id: Id,
        limit: usize,
    ) -> DbResult<Vec<BaseReleaseAttestationRecord>> {
        let rows = sqlx::query(
            r#"
            SELECT id, network, tx_hash, log_key, bounty_id, onchain_escrow_id, calldata_hash, proof_hash, recipients, escrow_contract, settlement_signer, platform_fee_wallet, verdict, reason, created_at
            FROM base_release_attestations
            WHERE bounty_id = $1
            ORDER BY created_at DESC, tx_hash DESC, log_key DESC
            LIMIT $2
            "#,
        )
        .bind(bounty_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(base_release_attestation_from_row)
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

    pub async fn persist_base_log_pipeline(
        &self,
        batch: BaseLogPersistenceBatch<'_>,
    ) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;

        for bounty in batch.bounties {
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
            .execute(&mut *tx)
            .await?;
        }

        for record in batch.release_attestations {
            let result = sqlx::query(
                r#"
                INSERT INTO base_release_attestations
                  (id, network, tx_hash, log_key, bounty_id, onchain_escrow_id, calldata_hash, proof_hash, recipients, escrow_contract, settlement_signer, platform_fee_wallet, verdict, reason, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                ON CONFLICT (network, tx_hash, log_key) DO UPDATE SET
                  id = base_release_attestations.id
                WHERE base_release_attestations.bounty_id = EXCLUDED.bounty_id
                  AND base_release_attestations.onchain_escrow_id = EXCLUDED.onchain_escrow_id
                  AND base_release_attestations.calldata_hash IS NOT DISTINCT FROM EXCLUDED.calldata_hash
                  AND base_release_attestations.proof_hash IS NOT DISTINCT FROM EXCLUDED.proof_hash
                  AND base_release_attestations.recipients = EXCLUDED.recipients
                  AND base_release_attestations.escrow_contract = EXCLUDED.escrow_contract
                  AND base_release_attestations.settlement_signer = EXCLUDED.settlement_signer
                  AND base_release_attestations.platform_fee_wallet IS NOT DISTINCT FROM EXCLUDED.platform_fee_wallet
                  AND base_release_attestations.verdict = EXCLUDED.verdict
                  AND base_release_attestations.reason = EXCLUDED.reason
                "#,
            )
            .bind(record.id)
            .bind(&record.network)
            .bind(&record.tx_hash)
            .bind(&record.log_key)
            .bind(record.bounty_id)
            .bind(&record.onchain_escrow_id)
            .bind(&record.calldata_hash)
            .bind(&record.proof_hash)
            .bind(&record.recipients)
            .bind(&record.escrow_contract)
            .bind(&record.settlement_signer)
            .bind(&record.platform_fee_wallet)
            .bind(&record.verdict)
            .bind(&record.reason)
            .bind(record.created_at)
            .execute(&mut *tx)
            .await?;
            if result.rows_affected() == 0 {
                return Err(DbError::ImmutableAttestationConflict(format!(
                    "{} {} {}",
                    record.network, record.tx_hash, record.log_key
                )));
            }
        }

        for intent in batch.funding_intents {
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
            .execute(&mut *tx)
            .await?;
        }

        for escrow in batch.escrows {
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
            .execute(&mut *tx)
            .await?;
        }

        for settlement in batch.settlements {
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
            .execute(&mut *tx)
            .await?;
        }

        for entry in batch.ledger_entries {
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
            .execute(&mut *tx)
            .await?;
        }

        for event in batch.base_escrow_events {
            let log_index = log_index_from_key(&event.log_key)
                .map(i64_from_u64)
                .transpose()?;
            sqlx::query(
                r#"
                INSERT INTO base_escrow_events
                  (id, log_key, tx_hash, block_number, log_index, onchain_escrow_id, bounty_id, kind, status, token, amount, currency, terms_hash, proof_hash, reason_hash, dispute_hash, occurred_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
                ON CONFLICT (log_key) DO UPDATE SET
                  tx_hash = EXCLUDED.tx_hash,
                  block_number = EXCLUDED.block_number,
                  log_index = EXCLUDED.log_index,
                  onchain_escrow_id = EXCLUDED.onchain_escrow_id,
                  bounty_id = EXCLUDED.bounty_id,
                  kind = EXCLUDED.kind,
                  status = EXCLUDED.status,
                  token = EXCLUDED.token,
                  amount = EXCLUDED.amount,
                  currency = EXCLUDED.currency,
                  terms_hash = EXCLUDED.terms_hash,
                  proof_hash = EXCLUDED.proof_hash,
                  reason_hash = EXCLUDED.reason_hash,
                  dispute_hash = EXCLUDED.dispute_hash,
                  occurred_at = EXCLUDED.occurred_at
                "#,
            )
            .bind(event.id)
            .bind(&event.log_key)
            .bind(&event.tx_hash)
            .bind(i64_from_u64(event.block_number)?)
            .bind(log_index)
            .bind(event.onchain_escrow_id.to_string())
            .bind(event.bounty_id)
            .bind(format!("{:?}", event.kind))
            .bind(format!("{:?}", event.status))
            .bind(&event.token)
            .bind(event.amount.as_ref().map(|amount| amount.amount))
            .bind(event.amount.as_ref().map(|amount| amount.currency.clone()))
            .bind(&event.terms_hash)
            .bind(&event.proof_hash)
            .bind(&event.reason_hash)
            .bind(&event.dispute_hash)
            .bind(event.occurred_at)
            .execute(&mut *tx)
            .await?;
        }

        if let Some(cursor) = batch.cursor {
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
            .bind(cursor.network)
            .bind(normalize_key_address(cursor.escrow_contract))
            .bind(i64_from_u64(cursor.last_scanned_block)?)
            .bind(cursor.last_log_key)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
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

fn parse_agent_status(value: String) -> DbResult<AgentStatus> {
    match value.as_str() {
        "Active" => Ok(AgentStatus::Active),
        "Suspended" => Ok(AgentStatus::Suspended),
        _ => Err(DbError::InvalidEnum(value)),
    }
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

fn parse_base_escrow_event_kind(value: String) -> DbResult<BaseEscrowEventKind> {
    match value.as_str() {
        "Created" => Ok(BaseEscrowEventKind::Created),
        "Released" => Ok(BaseEscrowEventKind::Released),
        "Refunded" => Ok(BaseEscrowEventKind::Refunded),
        "Disputed" => Ok(BaseEscrowEventKind::Disputed),
        "Paused" => Ok(BaseEscrowEventKind::Paused),
        _ => Err(DbError::InvalidEnum(value)),
    }
}

fn base_escrow_event_from_row(row: PgRow) -> DbResult<BaseEscrowEvent> {
    let amount = match (
        row.try_get::<Option<i64>, _>("amount")?,
        row.try_get::<Option<String>, _>("currency")?,
    ) {
        (Some(amount), Some(currency)) => Some(Money::new(amount, currency)?),
        _ => None,
    };
    Ok(BaseEscrowEvent {
        id: row.try_get("id")?,
        log_key: row.try_get("log_key")?,
        tx_hash: row.try_get("tx_hash")?,
        block_number: u64_from_i64(row.try_get("block_number")?)?,
        onchain_escrow_id: row
            .try_get::<String, _>("onchain_escrow_id")?
            .parse()
            .map_err(|_| DbError::InvalidEnum("onchain_escrow_id".to_string()))?,
        bounty_id: row.try_get("bounty_id")?,
        kind: parse_base_escrow_event_kind(row.try_get::<String, _>("kind")?)?,
        status: parse_escrow_status(row.try_get::<String, _>("status")?)?,
        token: row.try_get("token")?,
        amount,
        terms_hash: row.try_get("terms_hash")?,
        proof_hash: row.try_get("proof_hash")?,
        reason_hash: row.try_get("reason_hash")?,
        dispute_hash: row.try_get("dispute_hash")?,
        occurred_at: row.try_get("occurred_at")?,
    })
}

fn base_release_attestation_from_row(row: PgRow) -> DbResult<BaseReleaseAttestationRecord> {
    Ok(BaseReleaseAttestationRecord {
        id: row.try_get("id")?,
        network: row.try_get("network")?,
        tx_hash: row.try_get("tx_hash")?,
        log_key: row.try_get("log_key")?,
        bounty_id: row.try_get("bounty_id")?,
        onchain_escrow_id: row.try_get("onchain_escrow_id")?,
        calldata_hash: row.try_get("calldata_hash")?,
        proof_hash: row.try_get("proof_hash")?,
        recipients: row.try_get("recipients")?,
        escrow_contract: row.try_get("escrow_contract")?,
        settlement_signer: row.try_get("settlement_signer")?,
        platform_fee_wallet: row.try_get("platform_fee_wallet")?,
        verdict: row.try_get("verdict")?,
        reason: row.try_get("reason")?,
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

fn log_index_from_key(log_key: &str) -> Option<u64> {
    log_key.rsplit_once(':')?.1.parse().ok()
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
    use domain::{FundingIntentStatus, FundingMode, Money, PaymentRail};
    use ledger::{credit, debit};

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
            "base_release_attestations",
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
    #[ignore]
    async fn base_log_pipeline_rolls_back_paid_state_when_cursor_commit_fails() {
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("AGENT_BOUNTIES_TEST_DATABASE_URL must be set for ignored Postgres tests");
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();

        let now = Utc::now();
        let amount = Money::new(1_000_000, "usdc").unwrap();
        let mut bounty = Bounty::new(
            "Atomic Base release persistence",
            "fix-ci-failure",
            amount.clone(),
            FundingMode::BaseUsdcEscrow,
            PrivacyLevel::Public,
        );
        bounty.status = BountyStatus::Payable;
        store.upsert_bounty(&bounty).await.unwrap();

        let mut paid_bounty = bounty.clone();
        paid_bounty.status = BountyStatus::Paid;
        let tx_hash = format!("0x{}aaaaaaaa", bounty.id.simple());
        let log_key = format!("{tx_hash}:7");
        let escrow_contract = format!("0x{}00000000", bounty.id.simple());
        let funding_intent = FundingIntent {
            id: Id::new_v4(),
            bounty_id: bounty.id,
            contributor_agent_id: None,
            source_organization_id: None,
            rail: PaymentRail::BaseUsdc,
            amount: amount.clone(),
            status: FundingIntentStatus::Applied,
            external_reference: Some(format!("atomic-release-test:{}", bounty.id)),
            stripe_success_url: None,
            stripe_cancel_url: None,
            created_at: now,
        };
        let escrow = Escrow {
            id: Id::new_v4(),
            bounty_id: bounty.id,
            rail: PaymentRail::BaseUsdc,
            token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913".to_string(),
            amount: amount.clone(),
            status: EscrowStatus::Released,
            external_reference: Some("base-escrow:7".to_string()),
        };
        let attestation = BaseReleaseAttestationRecord {
            id: Id::new_v4(),
            network: "base-mainnet".to_string(),
            tx_hash: tx_hash.clone(),
            log_key: log_key.clone(),
            bounty_id: bounty.id,
            onchain_escrow_id: "7".to_string(),
            calldata_hash: Some(
                "0x1111111111111111111111111111111111111111111111111111111111111111".to_string(),
            ),
            proof_hash: Some(
                "0x2222222222222222222222222222222222222222222222222222222222222222".to_string(),
            ),
            recipients: serde_json::json!([
                {
                    "address": "0x3333333333333333333333333333333333333333",
                    "amount": "1000000"
                }
            ]),
            escrow_contract: escrow_contract.clone(),
            settlement_signer: "0x5555555555555555555555555555555555555555".to_string(),
            platform_fee_wallet: None,
            verdict: "passed".to_string(),
            reason: "release calldata matches pending Base settlement plan".to_string(),
            created_at: now,
        };
        let ledger_entry = LedgerEntry::new(
            "Base release paid",
            Some(format!("base-release:{log_key}")),
            vec![
                debit("base_escrow", amount.clone()),
                credit("solver_payable", amount.clone()),
            ],
        )
        .unwrap();
        let event = BaseEscrowEvent {
            id: Id::new_v4(),
            log_key: log_key.clone(),
            tx_hash,
            block_number: 48426133,
            onchain_escrow_id: 7,
            bounty_id: bounty.id,
            kind: BaseEscrowEventKind::Released,
            status: EscrowStatus::Released,
            token: None,
            amount: None,
            terms_hash: None,
            proof_hash: attestation.proof_hash.clone(),
            reason_hash: None,
            dispute_hash: None,
            occurred_at: now,
        };
        let paid_bounties = [paid_bounty];
        let release_attestations = [attestation];
        let funding_intents = [funding_intent.clone()];
        let escrows = [escrow.clone()];
        let ledger_entries = [ledger_entry.clone()];
        let base_escrow_events = [event];

        let result = store
            .persist_base_log_pipeline(BaseLogPersistenceBatch {
                bounties: &paid_bounties,
                release_attestations: &release_attestations,
                funding_intents: &funding_intents,
                escrows: &escrows,
                settlements: &[],
                ledger_entries: &ledger_entries,
                base_escrow_events: &base_escrow_events,
                cursor: Some(BaseLogPersistenceCursor {
                    network: "base-mainnet",
                    escrow_contract: &escrow_contract,
                    last_scanned_block: u64::MAX,
                    last_log_key: Some(&log_key),
                }),
            })
            .await;
        assert!(matches!(result, Err(DbError::IntegerOverflow(_))));

        let stored_bounty = store
            .list_bounties()
            .await
            .unwrap()
            .into_iter()
            .find(|stored| stored.id == bounty.id)
            .unwrap();
        assert_eq!(stored_bounty.status, BountyStatus::Payable);
        assert!(store
            .list_base_release_attestations_for_bounty(bounty.id)
            .await
            .unwrap()
            .is_empty());
        assert!(store
            .list_base_escrow_events_for_bounty(bounty.id)
            .await
            .unwrap()
            .is_empty());
        assert!(!store
            .list_funding_intents()
            .await
            .unwrap()
            .iter()
            .any(|stored| stored.id == funding_intent.id));
        assert!(!store
            .list_escrows()
            .await
            .unwrap()
            .iter()
            .any(|stored| stored.id == escrow.id));
        assert!(!store
            .list_ledger_entries()
            .await
            .unwrap()
            .iter()
            .any(|stored| stored.external_event_id == ledger_entry.external_event_id));
        assert!(store
            .get_base_log_cursor("base-mainnet", &escrow_contract)
            .await
            .unwrap()
            .is_none());
    }
}
