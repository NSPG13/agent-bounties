//! Funding reservation and its durable execution journal. A paid order is reserved for funding
//! at most once, under the same order lock as refunds; the bank action keeps state `held` and a
//! row check makes a later refund reservation impossible. Only canonical events mark it funded.
use super::{stripe::locked_order, ManagedOrderStatus, ManagedOrderStoreError, Result};
use crate::PostgresStore;
use chain_base::AutonomousBountyEvent;
use managed_orders::funding::{FundingError, FundingPlan, FundingTarget};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedFundingOperation {
    pub order_id: Uuid,
    pub operation_id: Uuid,
    pub plan: FundingPlan,
    pub outcome: String,
    pub tx_hash: Option<String>,
    /// Hash of the signed bytes saved in the relay outbox, if any were ever signed.
    pub saved_tx_hash: Option<String>,
}

/// Every operation read carries whether a transaction was ever signed for it.
const OPERATION_SELECT: &str = "SELECT g.*, o.tx_hash AS saved_tx_hash
    FROM managed_bounty_funding_operations g
    LEFT JOIN relay_transaction_outbox o ON o.operation_id=g.operation_id";

/// One delegate key sends every funding transaction; one live lease at a time keeps its nonces
/// from racing.
const FUNDING_SENDER_LOCK: i64 = 0x6d61_6e61_6765_6466;

fn plan_digest(plan: &FundingPlan) -> Result<String> {
    let encoded = serde_json::to_vec(plan).map_err(|_| ManagedOrderStoreError::Integrity)?;
    let mut hash = Sha256::new();
    hash.update(b"agent-bounties:managed-funding-plan:v1\0");
    hash.update(encoded);
    Ok(hex::encode(hash.finalize()))
}

pub(super) fn operation_from_row(row: &sqlx::postgres::PgRow) -> Result<ManagedFundingOperation> {
    let plan: FundingPlan = serde_json::from_value(row.try_get("plan")?)
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    let order_id: Uuid = row.try_get("order_id")?;
    if plan.order_id != order_id
        || plan_digest(&plan)? != row.try_get::<String, _>("plan_digest")?
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(ManagedFundingOperation {
        order_id,
        operation_id: row.try_get("operation_id")?,
        plan,
        outcome: row.try_get("outcome")?,
        tx_hash: row.try_get("tx_hash")?,
        saved_tx_hash: row.try_get("saved_tx_hash")?,
    })
}

impl PostgresStore {
    /// Paid, unreserved, undisputed orders for `target`, oldest payment first, whose first signed paid
    /// event is at least `min_paid_age_seconds` old and whose funding deadline is more than
    /// `deadline_margin_seconds` away, by database time. A candidate is not a reservation;
    /// `reserve_managed_funding` decides.
    pub async fn list_managed_orders_ready_for_funding(
        &self,
        target: &FundingTarget,
        min_paid_age_seconds: i64,
        deadline_margin_seconds: i64,
        limit: i64,
    ) -> Result<Vec<Uuid>> {
        let rows = sqlx::query(
            "SELECT b.order_id, (SELECT min(e.event_created_at) FROM managed_stripe_event_inbox e
                    WHERE e.order_id=b.order_id AND e.outcome='paid'
                      AND e.disposition='processed') AS paid_at
             FROM managed_stripe_bank_actions b
             JOIN managed_service_orders o ON o.order_id=b.order_id
             JOIN managed_service_quotes q ON q.quote_id=o.quote_id
             CROSS JOIN (SELECT floor(extract(epoch FROM clock_timestamp()))::bigint AS now) t
             WHERE b.state='held' AND b.funding_operation_id IS NULL
               AND NOT EXISTS (SELECT 1 FROM managed_refund_operations f WHERE f.order_id=b.order_id)
               AND NOT EXISTS (SELECT 1 FROM managed_stripe_disputes d WHERE d.order_id=b.order_id)
               AND q.quote->'input'->'bounty'->>'network'=$3
               AND q.quote->'input'->'bounty'->>'factory'=lower($4)
               AND q.quote->'input'->'bounty'->>'funding_wallet'=lower($5)
               AND jsonb_typeof(q.quote->'input'->'bounty'->'funding_deadline')='number'
               AND (q.quote->'input'->'bounty'->>'funding_deadline')::numeric > t.now + $6
               AND (SELECT min(e.event_created_at) FROM managed_stripe_event_inbox e
                    WHERE e.order_id=b.order_id AND e.outcome='paid' AND e.disposition='processed')
                   <= t.now - $1
             ORDER BY paid_at, b.order_id LIMIT $2",
        )
        .bind(min_paid_age_seconds.max(0))
        .bind(limit.clamp(1, 100))
        .bind(&target.network)
        .bind(&target.factory)
        .bind(&target.funding_wallet)
        .bind(deadline_margin_seconds.max(0))
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|row| row.try_get("order_id").map_err(Into::into))
            .collect()
    }

    /// Atomically reserve one held paid order for funding with the plan rebuilt from its own
    /// accepted quote and the stored public terms it commits to, for `target` only. A refund-reserved, already
    /// reserved or non-paid order returns its existing operation or a conflict; nothing is
    /// reserved twice.
    /// An order whose funding deadline is within `deadline_margin_seconds` of database time is
    /// refused and stays refundable.
    pub async fn reserve_managed_funding(
        &self,
        order_id: Uuid,
        target: &FundingTarget,
        deadline_margin_seconds: i64,
    ) -> Result<ManagedFundingOperation> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if let Some(existing) = sqlx::query(&format!("{OPERATION_SELECT} WHERE g.order_id=$1"))
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            tx.commit().await?;
            return operation_from_row(&existing);
        }
        let disputed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_stripe_disputes WHERE order_id=$1)",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        if order.status != ManagedOrderStatus::AwaitingBankSettlement || disputed {
            return Err(ManagedOrderStoreError::Quote(
                managed_orders::OrderError::Conflict,
            ));
        }
        // Read on the locked transaction's connection: a second pool connection here could
        // wait behind the very order lock this transaction holds.
        let terms = sqlx::query(
            "SELECT terms_hash, policy_hash, acceptance_criteria_hash, benchmark_hash,
                    evidence_schema_hash, creator_wallet, document, created_at
             FROM autonomous_bounty_terms WHERE terms_hash=lower($1)",
        )
        .bind(&order.quote.input().bounty.terms_hash)
        .fetch_optional(&mut *tx)
        .await?
        .map(crate::autonomous_terms_from_row)
        .transpose()
        .map_err(|_| ManagedOrderStoreError::Integrity)?
        .ok_or(ManagedOrderStoreError::Integrity)?;
        let plan = FundingPlan::new(order_id, &order.quote, &terms, target).map_err(|error| {
            if error == FundingError::OtherTarget {
                ManagedOrderStoreError::Quote(managed_orders::OrderError::Conflict)
            } else {
                ManagedOrderStoreError::Integrity
            }
        })?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        if plan.funding_deadline <= now.saturating_add(deadline_margin_seconds.max(0)) {
            return Err(ManagedOrderStoreError::Quote(
                managed_orders::OrderError::Conflict,
            ));
        }
        let operation_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO managed_bounty_funding_operations(order_id,operation_id,plan,plan_digest)
             VALUES($1,$2,$3,$4)",
        )
        .bind(order_id)
        .bind(operation_id)
        .bind(serde_json::to_value(&plan).map_err(|_| ManagedOrderStoreError::Integrity)?)
        .bind(plan_digest(&plan)?)
        .execute(&mut *tx)
        .await?;
        let reserved = sqlx::query(
            "UPDATE managed_stripe_bank_actions SET funding_operation_id=$2
             WHERE order_id=$1 AND state='held' AND funding_operation_id IS NULL",
        )
        .bind(order_id)
        .bind(operation_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if reserved != 1 {
            return Err(ManagedOrderStoreError::Quote(
                managed_orders::OrderError::Conflict,
            ));
        }
        tx.commit().await?;
        Ok(ManagedFundingOperation {
            order_id,
            operation_id,
            plan,
            outcome: "reserved".into(),
            tx_hash: None,
            saved_tx_hash: None,
        })
    }

    /// Open operations, signed ones first.
    pub async fn list_open_managed_funding(
        &self,
        limit: i64,
    ) -> Result<Vec<ManagedFundingOperation>> {
        // Signed operations first, so an unfinished transaction is always seen.
        let rows = sqlx::query(&format!(
            "{OPERATION_SELECT} WHERE g.outcome IN ('reserved','broadcast')
             ORDER BY o.tx_hash IS NULL, g.reserved_at, g.order_id LIMIT $1"
        ))
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(operation_from_row).collect()
    }

    /// One fenced worker per operation and one live lease across all operations, so the single
    /// delegate sender never signs two transactions at once. An expired lease may be taken
    /// over; its holder can no longer record a broadcast.
    pub async fn claim_managed_funding_lease(
        &self,
        order_id: Uuid,
        seconds: i64,
    ) -> Result<Option<Uuid>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(FUNDING_SENDER_LOCK)
            .execute(&mut *tx)
            .await?;
        let row = sqlx::query(
            "UPDATE managed_bounty_funding_operations
             SET lease_token=$2, lease_until=clock_timestamp()+make_interval(secs=>$3)
             WHERE order_id=$1 AND outcome IN ('reserved','broadcast')
               AND NOT EXISTS (SELECT 1 FROM managed_bounty_funding_operations
                               WHERE lease_until>clock_timestamp())",
        )
        .bind(order_id)
        .bind(Uuid::new_v4())
        .bind(seconds.clamp(10, 600) as f64)
        .execute(&mut *tx)
        .await?;
        let token = if row.rows_affected() == 1 {
            sqlx::query_scalar(
                "SELECT lease_token FROM managed_bounty_funding_operations WHERE order_id=$1",
            )
            .bind(order_id)
            .fetch_one(&mut *tx)
            .await?
        } else {
            None
        };
        tx.commit().await?;
        Ok(token)
    }

    /// The lease still has at least `seconds` left, re-read before each send.
    pub async fn managed_funding_lease_has_time(
        &self,
        order_id: Uuid,
        lease: Uuid,
        seconds: i64,
    ) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_bounty_funding_operations
             WHERE order_id=$1 AND lease_token=$2
               AND lease_until>clock_timestamp()+make_interval(secs=>$3))",
        )
        .bind(order_id)
        .bind(lease)
        .bind(seconds.max(0) as f64)
        .fetch_one(&self.pool)
        .await?)
    }

    /// Record the transaction hash of the persisted signed bytes. A different hash is refused:
    /// one operation never gets a second transaction.
    pub async fn record_managed_funding_broadcast(
        &self,
        order_id: Uuid,
        lease: Uuid,
        tx_hash: &str,
    ) -> Result<()> {
        let tx_hash = tx_hash.to_ascii_lowercase();
        let changed = sqlx::query(
            "UPDATE managed_bounty_funding_operations SET tx_hash=$3, outcome='broadcast'
             WHERE order_id=$1 AND lease_token=$2 AND lease_until>clock_timestamp()
               AND outcome IN ('reserved','broadcast') AND (tx_hash IS NULL OR tx_hash=$3)",
        )
        .bind(order_id)
        .bind(lease)
        .bind(&tx_hash)
        .execute(&self.pool)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(ManagedOrderStoreError::StaleLease);
        }
        Ok(())
    }

    /// Canonical confirmed events one transaction emitted from the plan's factory or predicted
    /// bounty, for `observe_funding`. Unconfirmed (block time unverified) rows are never read.
    pub async fn verified_funding_events(
        &self,
        plan: &FundingPlan,
        tx_hash: &str,
    ) -> Result<Vec<AutonomousBountyEvent>> {
        let rows = sqlx::query(
            "SELECT id, log_key, tx_hash, block_number, log_index, contract_address,
                    bounty_id, kind, data, occurred_at
             FROM autonomous_bounty_events
             WHERE network=$1 AND contract_address IN (lower($2), lower($3))
               AND tx_hash=lower($4) AND block_time_verified
             ORDER BY block_number, log_index",
        )
        .bind(&plan.network)
        .bind(&plan.factory)
        .bind(&plan.predicted_bounty)
        .bind(tx_hash)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                crate::autonomous_event_from_row(row).map_err(|_| ManagedOrderStoreError::Integrity)
            })
            .collect()
    }

    pub async fn record_managed_funding_funded(
        &self,
        order_id: Uuid,
        tx_hash: &str,
        block_number: u64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE managed_bounty_funding_operations
             SET outcome='funded', funded_tx_hash=lower($2), funded_block=$3,
                 lease_token=NULL, lease_until=NULL
             WHERE order_id=$1 AND outcome IN ('reserved','broadcast')",
        )
        .bind(order_id)
        .bind(tx_hash)
        .bind(i64::try_from(block_number).map_err(|_| ManagedOrderStoreError::Integrity)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Stops automatic handling; the money stays reserved for an operator to reconcile.
    pub async fn record_managed_funding_reconciliation(&self, order_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE managed_bounty_funding_operations
             SET outcome='reconciliation_required', lease_token=NULL, lease_until=NULL
             WHERE order_id=$1 AND outcome IN ('reserved','broadcast')",
        )
        .bind(order_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The saved transaction is older than the relay outbox's two hour send window.
    pub async fn managed_funding_send_window_closed(&self, operation_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM relay_transaction_outbox
             WHERE operation_id=$1 AND created_at<=now()-interval '2 hours')",
        )
        .bind(operation_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn release_managed_funding_lease(&self, order_id: Uuid, lease: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE managed_bounty_funding_operations SET lease_token=NULL, lease_until=NULL
             WHERE order_id=$1 AND lease_token=$2",
        )
        .bind(order_id)
        .bind(lease)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
pub(super) mod tests;
