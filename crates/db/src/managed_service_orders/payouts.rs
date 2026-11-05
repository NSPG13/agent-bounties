//! The customer's review deposit owed to a solver after the arbiter reversed a rejection. The
//! obligation is written with the ruling. It is paid only by a USDC transfer the caller has
//! verified on chain, and one transaction hash can settle at most one obligation.
use super::{stripe::locked_order, ManagedOrderStoreError, Result};
use crate::PostgresStore;
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedDepositPayout {
    pub order_id: Uuid,
    pub round: u64,
    pub network: String,
    pub bounty_contract: String,
    pub solver: String,
    pub amount_micro_usdc: u64,
    /// The verified transfer, once paid.
    pub tx_hash: Option<String>,
    pub block_number: Option<u64>,
}

const SELECT: &str = "SELECT p.*, f.plan->>'predicted_bounty' AS bounty_contract
    FROM managed_deposit_payouts p JOIN managed_bounty_funding_operations f ON f.order_id=p.order_id";

fn payout_from_row(row: &sqlx::postgres::PgRow) -> Result<ManagedDepositPayout> {
    let unsigned = |value: i64| u64::try_from(value).map_err(|_| ManagedOrderStoreError::Integrity);
    Ok(ManagedDepositPayout {
        order_id: row.try_get("order_id")?,
        round: unsigned(row.try_get("round")?)?,
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        solver: row.try_get("solver")?,
        amount_micro_usdc: unsigned(row.try_get("amount_micro_usdc")?)?,
        tx_hash: row.try_get("tx_hash")?,
        block_number: row
            .try_get::<Option<i64>, _>("block_number")?
            .map(unsigned)
            .transpose()?,
    })
}

impl PostgresStore {
    /// Deposit payouts owed to solvers, unpaid first and oldest first.
    pub async fn list_managed_deposit_payouts(
        &self,
        unpaid_only: bool,
        limit: i64,
    ) -> Result<Vec<ManagedDepositPayout>> {
        let rows = sqlx::query(&format!(
            "{SELECT} WHERE (NOT $1 OR p.tx_hash IS NULL)
             ORDER BY (p.tx_hash IS NOT NULL), p.created_at, p.order_id LIMIT $2"
        ))
        .bind(unpaid_only)
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(payout_from_row).collect()
    }

    /// The deposit payout of one card bounty, if its rejection was reversed.
    pub async fn managed_deposit_payout(
        &self,
        order_id: Uuid,
    ) -> Result<Option<ManagedDepositPayout>> {
        sqlx::query(&format!("{SELECT} WHERE p.order_id=$1"))
            .bind(order_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(payout_from_row)
            .transpose()
    }

    /// Record the verified transfer that paid the obligation. The same hash is idempotent; a
    /// different hash for a paid obligation, or a hash already used elsewhere, is refused.
    pub async fn record_managed_deposit_payout(
        &self,
        order_id: Uuid,
        tx_hash: &str,
        block_number: u64,
    ) -> Result<ManagedDepositPayout> {
        let mut tx = self.pool.begin().await?;
        locked_order(&mut tx, order_id).await?;
        let current = sqlx::query(&format!("{SELECT} WHERE p.order_id=$1"))
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await?
            .as_ref()
            .map(payout_from_row)
            .transpose()?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        let conflict = ManagedOrderStoreError::Quote(managed_orders::OrderError::Conflict);
        if let Some(paid) = &current.tx_hash {
            tx.commit().await?;
            return if paid.eq_ignore_ascii_case(tx_hash) {
                Ok(current)
            } else {
                Err(conflict)
            };
        }
        let used: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_deposit_payouts WHERE tx_hash=lower($1))",
        )
        .bind(tx_hash)
        .fetch_one(&mut *tx)
        .await?;
        if used {
            return Err(conflict);
        }
        sqlx::query(
            "UPDATE managed_deposit_payouts SET tx_hash=lower($2), block_number=$3, paid_at=clock_timestamp()
              WHERE order_id=$1 AND tx_hash IS NULL",
        )
        .bind(order_id)
        .bind(tx_hash)
        .bind(i64::try_from(block_number).map_err(|_| ManagedOrderStoreError::Integrity)?)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.managed_deposit_payout(order_id)
            .await?
            .ok_or(ManagedOrderStoreError::Integrity)
    }
}

#[cfg(test)]
pub(super) mod tests;
