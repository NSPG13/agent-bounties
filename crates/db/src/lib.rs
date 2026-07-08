use chain_base::{BaseEscrowEvent, BaseEscrowEventKind};
use domain::{
    Agent, AgentStatus, Bounty, BountyStatus, Capability, CapabilityClass, Claim, Escrow,
    EscrowStatus, EvalRun, FundingContribution, FundingContributionStatus, FundingIntent,
    FundingIntentStatus, FundingMode, HelpRequest, Id, Money, PaymentEvent, PaymentEventStatus,
    PaymentRail, PrivacyLevel, ProofRecord, Quote, ReputationEvent, RiskAction, RiskEvent,
    RiskReviewOutcome, RiskReviewRecord, RiskSurface, Settlement, Submission, TemplateSignal,
    VerificationDecision, VerifierKind, VerifierResult,
};
use ledger::{LedgerEntry, Posting};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use thiserror::Error;

pub const CORE_MIGRATION: &str = include_str!("../../../migrations/0001_core.sql");
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
}

pub type DbResult<T> = Result<T, DbError>;

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

        rows.into_iter()
            .map(|row| {
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
            })
            .collect()
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
              (id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
              contributor_agent_id = EXCLUDED.contributor_agent_id,
              source_organization_id = EXCLUDED.source_organization_id,
              rail = EXCLUDED.rail,
              amount = EXCLUDED.amount,
              currency = EXCLUDED.currency,
              status = EXCLUDED.status,
              external_reference = EXCLUDED.external_reference
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
        .bind(intent.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_funding_intents(&self) -> DbResult<Vec<FundingIntent>> {
        let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, created_at
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

        rows.into_iter()
            .map(|row| {
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
            })
            .collect()
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
                Ok(Settlement {
                    id: row.try_get("id")?,
                    bounty_id: row.try_get("bounty_id")?,
                    proof_record_id: row.try_get("proof_record_id")?,
                    rail: parse_payment_rail(row.try_get::<String, _>("rail")?)?,
                    payout_intents: serde_json::from_value(row.try_get("payout_intents")?)?,
                    platform_fee: Money::new(
                        row.try_get::<i64, _>("platform_fee")?,
                        row.try_get::<String, _>("currency")?,
                    )?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{FundingMode, Money};

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
        ] {
            assert!(CORE_MIGRATION.contains(table), "missing {table}");
        }
        assert!(CORE_MIGRATION.contains("idx_funding_contributions_external_reference"));
        assert!(CORE_MIGRATION.contains("source_organization_id UUID"));
        assert!(CORE_MIGRATION.contains("funding_targets JSONB"));
        assert!(CORE_MIGRATION.contains("funding_ledger_entry_id UUID"));
        assert!(CORE_MIGRATION.contains("refund_ledger_entry_id UUID"));
        assert!(CORE_MIGRATION.contains("settlement_id UUID"));
        assert!(CORE_MIGRATION.contains("fund-contribution:"));
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
}
