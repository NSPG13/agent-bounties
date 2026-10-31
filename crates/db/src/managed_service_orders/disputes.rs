//! Card disputes bound by PaymentIntent to one managed paid order. A dispute never moves money;
//! it stops automatic funding of the order and stays visible to operators.
use super::{ManagedOrderStoreError, Result};
use crate::PostgresStore;
use managed_orders::stripe_dispute::ManagedDisputeEvent;
use sqlx::Row;
use uuid::Uuid;

impl PostgresStore {
    /// Record a signed dispute event for the managed order paid by its PaymentIntent. Returns
    /// `None` when the payment is not a managed order. Amount above the paid amount, another
    /// currency, or a dispute ID already bound to another order is an integrity failure.
    pub async fn record_managed_dispute(
        &self,
        event: &ManagedDisputeEvent,
    ) -> Result<Option<Uuid>> {
        let receipts = sqlx::query(
            "SELECT order_id, amount_minor, currency FROM managed_stripe_paid_receipts
             WHERE livemode=$1 AND payment_intent_id=$2",
        )
        .bind(event.livemode)
        .bind(&event.payment_intent_id)
        .fetch_all(&self.pool)
        .await?;
        let [receipt] = receipts.as_slice() else {
            return if receipts.is_empty() {
                Ok(None)
            } else {
                Err(ManagedOrderStoreError::Integrity)
            };
        };
        let order_id: Uuid = receipt.try_get("order_id")?;
        let amount =
            i64::try_from(event.amount_minor).map_err(|_| ManagedOrderStoreError::Integrity)?;
        if amount > receipt.try_get::<i64, _>("amount_minor")?
            || event.currency != receipt.try_get::<String, _>("currency")?
        {
            return Err(ManagedOrderStoreError::Integrity);
        }
        let mut tx = self.pool.begin().await?;
        super::stripe::locked_order(&mut tx, order_id).await?;
        // An older event updates nothing; a dispute ID bound to another order is refused below.
        sqlx::query(
            "INSERT INTO managed_stripe_disputes
               (dispute_id, order_id, payment_intent_id, amount_minor, currency, status, reason,
                last_event_id, last_event_created)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT (dispute_id) DO UPDATE SET
               reconciliation_required=managed_stripe_disputes.reconciliation_required OR
                 (managed_stripe_disputes.last_event_created=excluded.last_event_created
                  AND managed_stripe_disputes.status<>excluded.status),
               status=excluded.status, reason=excluded.reason, amount_minor=excluded.amount_minor,
               last_event_id=excluded.last_event_id, last_event_created=excluded.last_event_created
             WHERE managed_stripe_disputes.order_id=excluded.order_id
               AND managed_stripe_disputes.last_event_created<=excluded.last_event_created",
        )
        .bind(&event.dispute_id)
        .bind(order_id)
        .bind(&event.payment_intent_id)
        .bind(amount)
        .bind(&event.currency)
        .bind(&event.status)
        .bind(&event.reason)
        .bind(&event.event_id)
        .bind(event.event_created)
        .execute(&mut *tx)
        .await?;
        let stored: Uuid =
            sqlx::query_scalar("SELECT order_id FROM managed_stripe_disputes WHERE dispute_id=$1")
                .bind(&event.dispute_id)
                .fetch_one(&mut *tx)
                .await?;
        if stored != order_id {
            return Err(ManagedOrderStoreError::Integrity);
        }
        tx.commit().await?;
        Ok(Some(order_id))
    }

    /// Whether any card dispute was recorded for this order.
    pub async fn managed_order_disputed(&self, order_id: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_stripe_disputes WHERE order_id=$1)",
        )
        .bind(order_id)
        .fetch_one(&self.pool)
        .await?)
    }
}

#[cfg(test)]
mod tests;
