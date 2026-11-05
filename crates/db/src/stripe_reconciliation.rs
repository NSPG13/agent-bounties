// Transaction-capable persistence shared by the ordinary store and Stripe reconciliation.
use super::*;

/// Authoritative financial state, read while the transaction excludes writers.
pub struct StripeReconciliationSnapshot {
    pub agents: Vec<Agent>,
    pub bounties: Vec<Bounty>,
    pub funding_intents: Vec<FundingIntent>,
    pub funding_contributions: Vec<FundingContribution>,
    pub settlements: Vec<Settlement>,
    pub ledger_entries: Vec<LedgerEntry>,
    pub payment_events: Vec<PaymentEvent>,
}

/// Dropping this value rolls back. No cache update or external processor call
/// belongs inside this transaction. The bounded, conservative locks deliberately
/// favor correctness over throughput for the initial low-volume Stripe rollout.
pub struct StripeReconciliationTransaction {
    tx: Transaction<'static, Postgres>,
    event: PaymentEvent,
    event_is_replay: bool,
    object_kind: String,
    object_id: String,
    binding_hash: String,
    existing_outcome: Option<serde_json::Value>,
}

/// Eligibility observations share the payment lock and read current settlements.
/// They do not create payment receipts or assert that money has moved.
pub struct StripeConnectSnapshotTransaction {
    tx: Transaction<'static, Postgres>,
}

async fn begin_financial_transaction(
    store: &PostgresStore,
) -> DbResult<Transaction<'static, Postgres>> {
    let mut tx = store.pool.begin().await?;
    sqlx::query("SET LOCAL lock_timeout = '5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout = '15s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL idle_in_transaction_session_timeout = '15s'")
        .execute(&mut *tx)
        .await?;
    // Both eligibility and payment evidence must observe the state left by the
    // preceding writer, including writers in another API/MCP process.
    sqlx::query("LOCK TABLE agents, bounties, funding_intents, funding_contributions, settlements, ledger_entries, payment_events, stripe_reconciled_objects IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx).await?;
    Ok(tx)
}

async fn financial_snapshot(
    tx: &mut Transaction<'_, Postgres>,
) -> DbResult<StripeReconciliationSnapshot> {
    Ok(StripeReconciliationSnapshot {
        agents: list_agents_on(&mut **tx).await?,
        bounties: list_bounties_on(&mut **tx).await?,
        funding_intents: list_funding_intents_on(&mut **tx).await?,
        funding_contributions: list_funding_contributions_on(&mut **tx).await?,
        settlements: list_settlements_on(&mut **tx).await?,
        ledger_entries: list_ledger_entries_on(&mut **tx).await?,
        payment_events: list_payment_events_on(&mut **tx).await?,
    })
}

impl StripeConnectSnapshotTransaction {
    pub async fn snapshot(&mut self) -> DbResult<StripeReconciliationSnapshot> {
        financial_snapshot(&mut self.tx).await
    }

    pub async fn persist_settlements(&mut self, settlements: &[Settlement]) -> DbResult<()> {
        for settlement in settlements {
            upsert_settlement_on(&mut *self.tx, settlement).await?;
        }
        Ok(())
    }

    pub async fn commit_with_snapshot(mut self) -> DbResult<StripeReconciliationSnapshot> {
        let snapshot = self.snapshot().await?;
        self.tx.commit().await?;
        Ok(snapshot)
    }
}

impl PostgresStore {
    pub async fn begin_stripe_connect_snapshot(
        &self,
    ) -> DbResult<StripeConnectSnapshotTransaction> {
        Ok(StripeConnectSnapshotTransaction {
            tx: begin_financial_transaction(self).await?,
        })
    }

    pub async fn begin_stripe_reconciliation(
        &self,
        event: &PaymentEvent,
        object_kind: &str,
        object_id: &str,
        binding_hash: &str,
    ) -> DbResult<StripeReconciliationTransaction> {
        if event.rail != PaymentRail::StripeFiat
            || event.status != PaymentEventStatus::Applied
            || !matches!(object_kind, "checkout.session" | "transfer")
            || object_id.is_empty()
            || object_id.len() > 255
        {
            return Err(DbError::StripeReconciliationConflict);
        }
        let mut tx = begin_financial_transaction(self).await?;
        let previous_event =
            sqlx::query("SELECT rail, payload_hash FROM payment_events WHERE external_id = $1")
                .bind(&event.external_id)
                .fetch_optional(&mut *tx)
                .await?;
        if let Some(row) = &previous_event {
            if row.try_get::<String, _>("rail")? != "StripeFiat"
                || row.try_get::<String, _>("payload_hash")? != event.payload_hash
            {
                return Err(DbError::StripeReconciliationConflict);
            }
        }
        let existing = sqlx::query("SELECT binding_hash, outcome FROM stripe_reconciled_objects WHERE object_kind = $1 AND object_id = $2")
            .bind(object_kind).bind(object_id).fetch_optional(&mut *tx).await?;
        let existing_outcome = match existing {
            Some(row) => {
                if row.try_get::<String, _>("binding_hash")? != binding_hash {
                    return Err(DbError::StripeReconciliationConflict);
                }
                Some(row.try_get("outcome")?)
            }
            None => None,
        };
        Ok(StripeReconciliationTransaction {
            tx,
            event: event.clone(),
            event_is_replay: previous_event.is_some(),
            object_kind: object_kind.into(),
            object_id: object_id.into(),
            binding_hash: binding_hash.into(),
            existing_outcome,
        })
    }
}

impl StripeReconciliationTransaction {
    pub fn existing_outcome(&self) -> Option<&serde_json::Value> {
        self.existing_outcome.as_ref()
    }

    pub async fn snapshot(&mut self) -> DbResult<StripeReconciliationSnapshot> {
        financial_snapshot(&mut self.tx).await
    }

    pub async fn persist_funding(
        &mut self,
        intent: Option<&FundingIntent>,
        bounty: Option<&Bounty>,
        contribution: Option<&FundingContribution>,
        entries: &[LedgerEntry],
    ) -> DbResult<()> {
        if self.existing_outcome.is_some() {
            return Err(DbError::StripeReconciliationConflict);
        }
        if let Some(intent) = intent {
            upsert_funding_intent_on(&mut *self.tx, intent).await?;
        }
        if let Some(bounty) = bounty {
            upsert_bounty_on(&mut *self.tx, bounty).await?;
        }
        if let Some(contribution) = contribution {
            upsert_funding_contribution_on(&mut *self.tx, contribution).await?;
        }
        for entry in entries {
            insert_ledger_entry_on(&mut *self.tx, entry).await?;
        }
        Ok(())
    }

    pub async fn persist_transfer(
        &mut self,
        settlement: Option<&Settlement>,
        bounty: Option<&Bounty>,
        entries: &[LedgerEntry],
    ) -> DbResult<()> {
        if self.existing_outcome.is_some() {
            return Err(DbError::StripeReconciliationConflict);
        }
        if let Some(settlement) = settlement {
            upsert_settlement_on(&mut *self.tx, settlement).await?;
        }
        if let Some(bounty) = bounty {
            upsert_bounty_on(&mut *self.tx, bounty).await?;
        }
        for entry in entries {
            insert_ledger_entry_on(&mut *self.tx, entry).await?;
        }
        Ok(())
    }

    async fn prepare_commit(&mut self, outcome: &serde_json::Value) -> DbResult<()> {
        if let Some(existing) = &self.existing_outcome {
            if existing != outcome {
                return Err(DbError::StripeReconciliationConflict);
            }
        } else {
            sqlx::query("INSERT INTO stripe_reconciled_objects (object_kind, object_id, binding_hash, outcome) VALUES ($1, $2, $3, $4)")
                .bind(&self.object_kind).bind(&self.object_id).bind(&self.binding_hash)
                .bind(outcome).execute(&mut *self.tx).await?;
        }
        if !self.event_is_replay {
            let mut event = self.event.clone();
            if self.existing_outcome.is_some() {
                event.status = PaymentEventStatus::IgnoredDuplicate;
            }
            upsert_payment_event_on(&mut *self.tx, &event).await?;
        }
        Ok(())
    }

    pub async fn commit(mut self, outcome: &serde_json::Value) -> DbResult<()> {
        self.prepare_commit(outcome).await?;
        self.tx.commit().await?;
        Ok(())
    }

    pub async fn commit_with_snapshot(
        mut self,
        outcome: &serde_json::Value,
    ) -> DbResult<StripeReconciliationSnapshot> {
        self.prepare_commit(outcome).await?;
        let snapshot = self.snapshot().await?;
        self.tx.commit().await?;
        Ok(snapshot)
    }
}
pub(super) async fn list_agents_on<'e, E>(executor: E) -> DbResult<Vec<Agent>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        "SELECT id, handle, status, payout_wallet, created_at FROM agents ORDER BY created_at",
    )
    .fetch_all(executor)
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

pub(super) async fn list_bounties_on<'e, E>(executor: E) -> DbResult<Vec<Bounty>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
            r#"
            SELECT id, help_request_id, title, template_slug, amount, currency, funding_targets, funding_mode, privacy, status, terms_hash, created_at
            FROM bounties
            ORDER BY created_at
            "#,
        )
        .fetch_all(executor)
        .await?;

    rows.into_iter().map(|row| bounty_from_row(&row)).collect()
}

pub(super) async fn list_funding_intents_on<'e, E>(executor: E) -> DbResult<Vec<FundingIntent>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, external_reference, stripe_success_url, stripe_cancel_url, created_at
            FROM funding_intents
            ORDER BY created_at
            "#,
        )
        .fetch_all(executor)
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

pub(super) async fn list_funding_contributions_on<'e, E>(
    executor: E,
) -> DbResult<Vec<FundingContribution>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, contributor_agent_id, source_organization_id, rail, amount, currency, status, funding_ledger_entry_id, refund_ledger_entry_id, settlement_id, external_reference, created_at
            FROM funding_contributions
            ORDER BY created_at
            "#,
        )
        .fetch_all(executor)
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

pub(super) async fn list_settlements_on<'e, E>(executor: E) -> DbResult<Vec<Settlement>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
            r#"
            SELECT id, bounty_id, proof_record_id, rail, payout_intents, platform_fee, currency, created_at
            FROM settlements
            ORDER BY created_at
            "#,
        )
        .fetch_all(executor)
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

pub(super) async fn list_ledger_entries_on<'e, E>(executor: E) -> DbResult<Vec<LedgerEntry>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query("SELECT id, external_event_id, memo, postings, created_at FROM ledger_entries ORDER BY created_at")
            .fetch_all(executor)
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

pub(super) async fn list_payment_events_on<'e, E>(executor: E) -> DbResult<Vec<PaymentEvent>>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    let rows = sqlx::query(
        r#"
            SELECT id, rail, external_id, status, payload_hash, received_at
            FROM payment_events
            ORDER BY received_at
            "#,
    )
    .fetch_all(executor)
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

pub(super) async fn upsert_bounty_on<'e, E>(executor: E, bounty: &Bounty) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
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
        .execute(executor)
        .await?;
    Ok(())
}

pub(super) async fn upsert_funding_intent_on<'e, E>(
    executor: E,
    intent: &FundingIntent,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
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
        .execute(executor)
        .await?;
    Ok(())
}

pub(super) async fn upsert_funding_contribution_on<'e, E>(
    executor: E,
    contribution: &FundingContribution,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
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
        .execute(executor)
        .await?;
    Ok(())
}

pub(super) async fn upsert_settlement_on<'e, E>(
    executor: E,
    settlement: &Settlement,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
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
        .execute(executor)
        .await?;
    Ok(())
}

pub(super) async fn insert_ledger_entry_on<'e, E>(executor: E, entry: &LedgerEntry) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
            INSERT INTO ledger_entries (id, external_event_id, memo, postings, created_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
    )
    .bind(entry.id)
    .bind(&entry.external_event_id)
    .bind(&entry.memo)
    .bind(serde_json::to_value(&entry.postings)?)
    .bind(entry.created_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub(super) async fn upsert_payment_event_on<'e, E>(
    executor: E,
    event: &PaymentEvent,
) -> DbResult<()>
where
    E: sqlx::Executor<'e, Database = Postgres>,
{
    sqlx::query(UPSERT_PAYMENT_EVENT_SQL)
        .bind(event.id)
        .bind(format!("{:?}", event.rail))
        .bind(&event.external_id)
        .bind(format!("{:?}", event.status))
        .bind(&event.payload_hash)
        .bind(event.received_at)
        .execute(executor)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{PayoutIntent, PayoutStatus};
    use ledger::{AccountCode, PostingSide};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;

    async fn test_store() -> PostgresStore {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("AGENT_BOUNTIES_TEST_DATABASE_URL must identify disposable test Postgres");
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        store
    }

    fn event() -> PaymentEvent {
        PaymentEvent {
            id: Uuid::new_v4(),
            rail: PaymentRail::StripeFiat,
            external_id: format!("evt_test_{}", Uuid::new_v4()),
            status: PaymentEventStatus::Applied,
            payload_hash: format!("sha256:{}", "10".repeat(32)),
            received_at: Utc::now(),
        }
    }

    fn binding_hash() -> String {
        format!("sha256:{}", "20".repeat(32))
    }

    fn entry(external_event_id: &str) -> LedgerEntry {
        LedgerEntry::new(
            "synthetic reconciliation fixture",
            Some(external_event_id.to_owned()),
            vec![
                Posting {
                    account: AccountCode::new("stripe-clearing"),
                    side: PostingSide::Debit,
                    amount: Money::new(500, "USD").unwrap(),
                },
                Posting {
                    account: AccountCode::new("bounty-funding"),
                    side: PostingSide::Credit,
                    amount: Money::new(500, "USD").unwrap(),
                },
            ],
        )
        .unwrap()
    }

    struct FundingFixture {
        bounty: Bounty,
        intent: FundingIntent,
        contribution: FundingContribution,
        entry: LedgerEntry,
        object_id: String,
        outcome: serde_json::Value,
    }

    impl FundingFixture {
        async fn seed(store: &PostgresStore) -> Self {
            let object_id = format!("cs_test_{}", Uuid::new_v4());
            let mut bounty = Bounty::new(
                "synthetic transaction test",
                "synthetic",
                Money::new(500, "USD").unwrap(),
                FundingMode::StripeFiatLedger,
                PrivacyLevel::Private,
            );
            store.upsert_bounty(&bounty).await.unwrap();
            let mut intent = FundingIntent {
                id: Uuid::new_v4(),
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: Some(Uuid::new_v4()),
                rail: PaymentRail::StripeFiat,
                amount: bounty.amount.clone(),
                status: FundingIntentStatus::AwaitingEvidence,
                external_reference: Some(object_id.clone()),
                stripe_success_url: None,
                stripe_cancel_url: None,
                created_at: Utc::now(),
            };
            store.upsert_funding_intent(&intent).await.unwrap();
            bounty.status = BountyStatus::Funded;
            intent.status = FundingIntentStatus::Applied;
            let entry = entry(&format!("stripe:{object_id}"));
            let contribution = FundingContribution {
                id: Uuid::new_v4(),
                bounty_id: bounty.id,
                contributor_agent_id: None,
                source_organization_id: intent.source_organization_id,
                rail: PaymentRail::StripeFiat,
                amount: bounty.amount.clone(),
                status: FundingContributionStatus::Applied,
                funding_ledger_entry_id: Some(entry.id),
                refund_ledger_entry_id: None,
                settlement_id: None,
                external_reference: Some(object_id.clone()),
                created_at: Utc::now(),
            };
            let outcome = serde_json::json!({
                "kind": "funding_applied",
                "bounty_id": bounty.id,
                "contribution_id": contribution.id,
            });
            Self {
                bounty,
                intent,
                contribution,
                entry,
                object_id,
                outcome,
            }
        }

        async fn persist(&self, transaction: &mut StripeReconciliationTransaction) -> DbResult<()> {
            transaction
                .persist_funding(
                    Some(&self.intent),
                    Some(&self.bounty),
                    Some(&self.contribution),
                    std::slice::from_ref(&self.entry),
                )
                .await
        }

        async fn assert_state(&self, store: &PostgresStore, applied: bool) {
            let bounty = store
                .list_bounties()
                .await
                .unwrap()
                .into_iter()
                .find(|value| value.id == self.bounty.id)
                .unwrap();
            assert_eq!(
                bounty.status,
                if applied {
                    BountyStatus::Funded
                } else {
                    BountyStatus::Unfunded
                }
            );
            let intent = store
                .list_funding_intents()
                .await
                .unwrap()
                .into_iter()
                .find(|value| value.id == self.intent.id)
                .unwrap();
            assert_eq!(
                intent.status,
                if applied {
                    FundingIntentStatus::Applied
                } else {
                    FundingIntentStatus::AwaitingEvidence
                }
            );
            assert_eq!(
                store
                    .list_funding_contributions()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|value| value.bounty_id == self.bounty.id)
                    .count(),
                usize::from(applied)
            );
            assert_eq!(
                store
                    .list_ledger_entries()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|value| value.external_event_id == self.entry.external_event_id)
                    .count(),
                usize::from(applied)
            );
            assert_eq!(sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM stripe_reconciled_objects WHERE object_kind = 'checkout.session' AND object_id = $1"
            ).bind(&self.object_id).fetch_one(&store.pool).await.unwrap(), i64::from(applied));
        }
    }

    async fn assert_event_count(store: &PostgresStore, payment: &PaymentEvent, count: i64) {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM payment_events WHERE external_id = $1"
            )
            .bind(&payment.external_id)
            .fetch_one(&store.pool)
            .await
            .unwrap(),
            count
        );
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn funding_drop_rolls_back_every_write_and_retry_can_commit() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let payment = event();
        let mut transaction = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut transaction).await.unwrap();
        let snapshot = transaction.snapshot().await.unwrap();
        assert_eq!(
            snapshot
                .funding_intents
                .iter()
                .find(|value| value.id == fixture.intent.id)
                .unwrap()
                .status,
            FundingIntentStatus::Applied
        );
        assert!(snapshot
            .ledger_entries
            .iter()
            .any(|value| value.id == fixture.entry.id));
        // Ordinary readers cannot observe the uncommitted financial changes.
        fixture.assert_state(&store, false).await;
        assert_event_count(&store, &payment, 0).await;
        drop(transaction);
        fixture.assert_state(&store, false).await;
        let mut retry = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        assert!(retry.existing_outcome().is_none());
        fixture.persist(&mut retry).await.unwrap();
        retry.commit(&fixture.outcome).await.unwrap();
        fixture.assert_state(&store, true).await;
        assert_event_count(&store, &payment, 1).await;
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn committed_funding_replays_after_lost_response_and_new_delivery_id() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let payment = event();
        let mut transaction = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut transaction).await.unwrap();
        transaction.commit(&fixture.outcome).await.unwrap();
        // Discard the acknowledged result and all connections to model a worker
        // restart after a commit whose HTTP response never reached the sender.
        store.pool.close().await;
        let restarted = test_store().await;
        let mut retry = restarted
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        assert_eq!(retry.existing_outcome(), Some(&fixture.outcome));
        assert!(matches!(
            fixture.persist(&mut retry).await,
            Err(DbError::StripeReconciliationConflict)
        ));
        retry.commit(&fixture.outcome).await.unwrap();
        let another_delivery = event();
        let replay = restarted
            .begin_stripe_reconciliation(
                &another_delivery,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        assert_eq!(replay.existing_outcome(), Some(&fixture.outcome));
        replay.commit(&fixture.outcome).await.unwrap();
        fixture.assert_state(&restarted, true).await;
        assert_event_count(&restarted, &payment, 1).await;
        assert_event_count(&restarted, &another_delivery, 1).await;
        for delivery in [&payment, &another_delivery] {
            // Retrying either delivery preserves its original audit status and identity.
            let replay = restarted
                .begin_stripe_reconciliation(
                    delivery,
                    "checkout.session",
                    &fixture.object_id,
                    &binding_hash(),
                )
                .await
                .unwrap();
            replay.commit(&fixture.outcome).await.unwrap();
        }
        let events = restarted.list_payment_events().await.unwrap();
        for (delivery, status) in [
            (&payment, PaymentEventStatus::Applied),
            (&another_delivery, PaymentEventStatus::IgnoredDuplicate),
        ] {
            let stored = events
                .iter()
                .find(|e| e.external_id == delivery.external_id)
                .unwrap();
            assert_eq!(stored.status, status);
            assert_eq!(stored.id, delivery.id);
            assert_eq!(
                stored.received_at.timestamp_micros(),
                delivery.received_at.timestamp_micros()
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn object_binding_payload_and_outcome_conflicts_cannot_mutate_applied_funding() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let payment = event();
        let mut transaction = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut transaction).await.unwrap();
        transaction.commit(&fixture.outcome).await.unwrap();
        let another_delivery = event();
        assert!(matches!(
            store
                .begin_stripe_reconciliation(
                    &another_delivery,
                    "checkout.session",
                    &fixture.object_id,
                    &format!("sha256:{}", "99".repeat(32)),
                )
                .await,
            Err(DbError::StripeReconciliationConflict)
        ));
        let mut changed_payload = payment.clone();
        changed_payload.payload_hash = format!("sha256:{}", "88".repeat(32));
        assert!(matches!(
            store
                .begin_stripe_reconciliation(
                    &changed_payload,
                    "checkout.session",
                    &fixture.object_id,
                    &binding_hash(),
                )
                .await,
            Err(DbError::StripeReconciliationConflict)
        ));
        let replay = store
            .begin_stripe_reconciliation(
                &another_delivery,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        assert!(matches!(
            replay
                .commit(&serde_json::json!({"kind": "different"}))
                .await,
            Err(DbError::StripeReconciliationConflict)
        ));
        fixture.assert_state(&store, true).await;
        assert_event_count(&store, &another_delivery, 0).await;
        let stored = store
            .list_payment_events()
            .await
            .unwrap()
            .into_iter()
            .find(|value| value.external_id == payment.external_id)
            .unwrap();
        assert_eq!(stored.payload_hash, payment.payload_hash);
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn independent_concurrent_sessions_apply_one_object_once() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let first_event = event();
        let second_event = event();
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let second = PostgresStore::from_pool(
            PgPoolOptions::new()
                .max_connections(1)
                .connect(&url)
                .await
                .unwrap(),
        );
        let second_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&second.pool)
            .await
            .unwrap();
        let mut first = store
            .begin_stripe_reconciliation(
                &first_event,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        let first_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *first.tx)
            .await
            .unwrap();
        assert_ne!(first_pid, second_pid);
        fixture.persist(&mut first).await.unwrap();
        let object_id = fixture.object_id.clone();
        let expected = fixture.outcome.clone();
        let event_copy = second_event.clone();
        let waiting = tokio::spawn(async move {
            let transaction = second
                .begin_stripe_reconciliation(
                    &event_copy,
                    "checkout.session",
                    &object_id,
                    &binding_hash(),
                )
                .await
                .unwrap();
            assert_eq!(transaction.existing_outcome(), Some(&expected));
            transaction.commit(&expected).await.unwrap();
        });
        // Prove real PostgreSQL contention before releasing the first writer;
        // scheduling two futures alone would not prove overlapping sessions.
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let blocked: bool = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')"
                ).bind(second_pid).fetch_one(&store.pool).await.unwrap();
                if blocked { break; }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("second database session must wait on the first transaction");
        first.commit(&fixture.outcome).await.unwrap();
        waiting.await.unwrap();
        fixture.assert_state(&store, true).await;
        assert_event_count(&store, &first_event, 1).await;
        assert_event_count(&store, &second_event, 1).await;
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn database_error_rolls_back_prior_funding_and_ledger_writes() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let payment = event();
        let mut transaction = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        let mut duplicate_primary_key = fixture.entry.clone();
        duplicate_primary_key.external_event_id = Some(format!("collision_{}", Uuid::new_v4()));
        let failure = transaction
            .persist_funding(
                Some(&fixture.intent),
                Some(&fixture.bounty),
                Some(&fixture.contribution),
                &[fixture.entry.clone(), duplicate_primary_key],
            )
            .await;
        assert!(
            matches!(failure, Err(DbError::Sqlx(sqlx::Error::Database(ref error)))
            if error.code().as_deref() == Some("23505"))
        );
        assert!(transaction.commit(&fixture.outcome).await.is_err());
        fixture.assert_state(&store, false).await;
        assert_event_count(&store, &payment, 0).await;
        let mut retry = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut retry).await.unwrap();
        retry.commit(&fixture.outcome).await.unwrap();
        fixture.assert_state(&store, true).await;
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn late_event_insert_failure_rolls_back_object_and_financial_writes() {
        let store = test_store().await;
        let fixture = FundingFixture::seed(&store).await;
        let existing = event();
        store.upsert_payment_event(&existing).await.unwrap();
        let mut payment = event();
        payment.id = existing.id;
        let mut transaction = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut transaction).await.unwrap();
        let result = transaction.commit(&fixture.outcome).await;
        assert!(
            matches!(result, Err(DbError::Sqlx(sqlx::Error::Database(ref error)))
            if error.code().as_deref() == Some("23505"))
        );
        fixture.assert_state(&store, false).await;
        assert_event_count(&store, &payment, 0).await;
        assert_event_count(&store, &existing, 1).await;
        payment.id = Uuid::new_v4();
        let mut retry = store
            .begin_stripe_reconciliation(
                &payment,
                "checkout.session",
                &fixture.object_id,
                &binding_hash(),
            )
            .await
            .unwrap();
        fixture.persist(&mut retry).await.unwrap();
        retry.commit(&fixture.outcome).await.unwrap();
        fixture.assert_state(&store, true).await;
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn transfer_settlement_ledger_and_bounty_commit_together_and_replay_once() {
        let store = test_store().await;
        let mut fixture = FundingFixture::seed(&store).await;
        fixture.bounty.status = BountyStatus::Payable;
        store.upsert_bounty(&fixture.bounty).await.unwrap();
        let agent = Agent::new(format!("stripe-transfer-test-{}", Uuid::new_v4()));
        store.upsert_agent(&agent).await.unwrap();
        let submission = Uuid::new_v4();
        let verification = Uuid::new_v4();
        let proof = Uuid::new_v4();
        sqlx::query("INSERT INTO submissions (id, bounty_id, solver_agent_id, artifact_digest, artifact_uri) VALUES ($1, $2, $3, 'fixture', 'private://fixture')")
            .bind(submission).bind(fixture.bounty.id).bind(agent.id).execute(&store.pool).await.unwrap();
        store
            .upsert_verifier_result(&domain::VerifierResult {
                id: verification,
                bounty_id: fixture.bounty.id,
                submission_id: submission,
                verifier_agent_id: None,
                kind: domain::VerifierKind::Manual,
                decision: domain::VerificationDecision::Accepted,
                summary: "synthetic test".into(),
                confidence: 1.0,
                signed_payload_hash: "fixture".into(),
                created_at: Utc::now(),
            })
            .await
            .unwrap();
        sqlx::query("INSERT INTO proof_records (id, bounty_id, submission_id, verifier_result_id, proof_hash, public_summary, privacy) VALUES ($1, $2, $3, $4, 'fixture', '', 'Private')")
            .bind(proof).bind(fixture.bounty.id).bind(submission).bind(verification).execute(&store.pool).await.unwrap();
        let mut settlement = Settlement {
            id: Uuid::new_v4(),
            bounty_id: fixture.bounty.id,
            proof_record_id: proof,
            rail: PaymentRail::StripeFiat,
            payout_intents: vec![PayoutIntent {
                id: Uuid::new_v4(),
                bounty_id: fixture.bounty.id,
                recipient_agent_id: agent.id,
                rail: PaymentRail::StripeFiat,
                amount: Money::new(500, "USD").unwrap(),
                status: PayoutStatus::Paying,
            }],
            platform_fee: Money::new(1, "USD").unwrap(),
            created_at: Utc::now(),
        };
        store.upsert_settlement(&settlement).await.unwrap();
        settlement.payout_intents[0].status = PayoutStatus::Paid;
        fixture.bounty.status = BountyStatus::Paid;
        let payment = event();
        let object_id = format!("tr_test_{}", Uuid::new_v4());
        let payout_entry = entry(&format!("stripe:{object_id}"));
        let outcome =
            serde_json::json!({"kind": "transfer_applied", "settlement_id": settlement.id});
        for commit in [false, true] {
            let mut transaction = store
                .begin_stripe_reconciliation(&payment, "transfer", &object_id, &binding_hash())
                .await
                .unwrap();
            transaction
                .persist_transfer(
                    Some(&settlement),
                    Some(&fixture.bounty),
                    std::slice::from_ref(&payout_entry),
                )
                .await
                .unwrap();
            if commit {
                transaction.commit(&outcome).await.unwrap();
            } else {
                drop(transaction);
            }
            let stored = store
                .list_settlements()
                .await
                .unwrap()
                .into_iter()
                .find(|value| value.id == settlement.id)
                .unwrap();
            assert_eq!(
                stored.payout_intents[0].status,
                if commit {
                    PayoutStatus::Paid
                } else {
                    PayoutStatus::Paying
                }
            );
            let bounty = store
                .list_bounties()
                .await
                .unwrap()
                .into_iter()
                .find(|value| value.id == fixture.bounty.id)
                .unwrap();
            assert_eq!(
                bounty.status,
                if commit {
                    BountyStatus::Paid
                } else {
                    BountyStatus::Payable
                }
            );
            assert_eq!(
                store
                    .list_ledger_entries()
                    .await
                    .unwrap()
                    .iter()
                    .filter(|value| value.external_event_id == payout_entry.external_event_id)
                    .count(),
                usize::from(commit)
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT count(*) FROM stripe_reconciled_objects WHERE object_kind = 'transfer' AND object_id = $1"
                )
                .bind(&object_id)
                .fetch_one(&store.pool)
                .await
                .unwrap(),
                i64::from(commit)
            );
            assert_event_count(&store, &payment, i64::from(commit)).await;
        }
        let mut replay = store
            .begin_stripe_reconciliation(&payment, "transfer", &object_id, &binding_hash())
            .await
            .unwrap();
        assert_eq!(replay.existing_outcome(), Some(&outcome));
        assert!(matches!(
            replay
                .persist_transfer(
                    Some(&settlement),
                    Some(&fixture.bounty),
                    std::slice::from_ref(&payout_entry)
                )
                .await,
            Err(DbError::StripeReconciliationConflict)
        ));
        replay.commit(&outcome).await.unwrap();
        assert_eq!(
            store
                .list_ledger_entries()
                .await
                .unwrap()
                .iter()
                .filter(|value| value.external_event_id == payout_entry.external_event_id)
                .count(),
            1
        );
        assert_event_count(&store, &payment, 1).await;
    }
}
