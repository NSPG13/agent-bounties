//! Ownership-scoped immutable acceptance, isolated from cached network snapshots.
//! All principal arguments MUST come from verified authentication at the caller.

use super::PostgresStore;
use managed_orders::{valid_key, ManagedQuote, OrderError};
use serde::Serialize;
use sqlx::{postgres::PgRow, Row};
use uuid::Uuid;

mod metrics;
mod stripe;
pub use stripe::ManagedEventDisposition;
mod stripe_execution;
pub use stripe_execution::{ManagedCheckoutLease, ManagedCheckoutRunResult};
mod recovered;
mod refunds;
pub use recovered::ManagedRecoveryTarget;
pub use refunds::{
    DepositRefundReservation, DepositState, ManagedRefundLease, RefundDrift,
    RefundObservationTarget,
};
mod funding;
pub use funding::ManagedFundingOperation;
mod disputes;
mod payouts;
pub use payouts::ManagedDepositPayout;
mod reviews;
pub use reviews::{
    ManagedCustomerReview, RecordedAppeal, RecordedRuling, ReviewDecider, ReviewSignature,
    ReviewSigner, SignedOutcome,
};

#[derive(Debug, thiserror::Error)]
pub enum ManagedOrderStoreError {
    #[error("managed-service order storage unavailable")]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Quote(#[from] OrderError),
    #[error(transparent)]
    Checkout(#[from] managed_orders::stripe_checkout::CheckoutError),
    #[error(transparent)]
    Refund(#[from] managed_orders::stripe_refund::RefundError),
    #[error("managed-service quote or order not found")]
    NotFound,
    #[error("managed-service stored record failed integrity validation")]
    Integrity,
    #[error("managed payment worker lease is no longer current")]
    StaleLease,
    #[error("new managed-service orders are disabled")]
    IntakeDisabled,
}

type Result<T> = std::result::Result<T, ManagedOrderStoreError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedOrderStatus {
    AwaitingPayment,
    PaymentPending,
    PaymentFailed,
    PaymentExpired,
    AwaitingBankSettlement,
    RefundRequested,
    RefundPending,
    RefundActionRequired,
    RefundSent,
    RefundFailed,
    FundingReserved,
    FundingBroadcast,
    Funded,
    FundingReconciliationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedOrder {
    pub order_id: Uuid,
    pub quote: ManagedQuote,
    pub quote_digest: String,
    pub accepted_at: i64,
    pub status: ManagedOrderStatus,
}

const ORDER_SELECT: &str = "SELECT q.*, o.order_id, o.accepted_at, o.idempotency_key,
    a.quote_digest AS action_digest, a.action AS action_kind, a.state AS action_state,
    c.session_id AS stripe_session_id, c.payment_intent_id AS bound_intent, p.payment_intent_id AS paid_intent,
    p.session_id AS paid_session, p.stripe_account_id AS paid_account, p.livemode AS paid_mode,
    p.quote_digest AS paid_digest, p.amount_minor AS paid_amount, p.currency AS paid_currency,
    b.state AS bank_action_state, to_jsonb(f) AS refund_operation,
    b.funding_operation_id AS bank_funding_operation_id, g.operation_id AS funding_operation_id,
    g.outcome AS funding_outcome,
    (SELECT to_jsonb(e) FROM managed_refund_events e WHERE e.order_id=o.order_id AND e.disposition='processed'
      ORDER BY CASE e.outcome WHEN 'failed' THEN 4 WHEN 'canceled' THEN 4 WHEN 'succeeded' THEN 3 ELSE 0 END DESC,
      e.event_created_at DESC, CASE e.outcome WHEN 'requires_action' THEN 1 ELSE 0 END DESC,e.event_id DESC LIMIT 1) AS refund_evidence,
    (SELECT e.outcome FROM managed_stripe_event_inbox e WHERE e.order_id=o.order_id
       AND e.disposition='processed' ORDER BY e.event_created_at DESC,e.event_id DESC LIMIT 1) AS latest_outcome
    FROM managed_service_orders o
    JOIN managed_service_quotes q ON q.quote_id=o.quote_id
    LEFT JOIN managed_service_actions a ON a.order_id=o.order_id
    LEFT JOIN managed_stripe_checkout_operations c ON c.order_id=o.order_id
    LEFT JOIN managed_stripe_paid_receipts p ON p.order_id=o.order_id
    LEFT JOIN managed_stripe_bank_actions b ON b.order_id=o.order_id
    LEFT JOIN managed_refund_operations f ON f.order_id=o.order_id
    LEFT JOIN managed_bounty_funding_operations g ON g.order_id=o.order_id";

fn quote_from_row(row: &PgRow) -> Result<ManagedQuote> {
    let json: serde_json::Value = row.try_get("quote")?;
    let quote: ManagedQuote =
        serde_json::from_value(json).map_err(|_| ManagedOrderStoreError::Integrity)?;
    let digest = quote
        .digest()
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    if quote.input().quote_id != row.try_get::<Uuid, _>("quote_id")?
        || quote.input().buyer_principal != row.try_get::<String, _>("buyer_principal")?
        || quote.input().issued_at != row.try_get::<i64, _>("issued_at")?
        || quote.input().expires_at != row.try_get::<i64, _>("expires_at")?
        || digest != row.try_get::<String, _>("quote_digest")?
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(quote)
}

fn order_from_row(row: &PgRow) -> Result<ManagedOrder> {
    let quote = quote_from_row(row)?;
    let digest: String = row.try_get("quote_digest")?;
    let accepted_at: i64 = row.try_get("accepted_at")?;
    quote
        .validate_acceptance(&quote.input().buyer_principal, &digest, accepted_at)
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    if row
        .try_get::<Option<String>, _>("action_digest")?
        .as_deref()
        != Some(digest.as_str())
        || row.try_get::<Option<String>, _>("action_kind")?.as_deref() != Some("prepare_checkout")
        || !matches!(
            row.try_get::<Option<String>, _>("action_state")?.as_deref(),
            Some("held" | "completed")
        )
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    let status = stripe::status_from_row(row, &quote)?;
    Ok(ManagedOrder {
        order_id: row.try_get("order_id")?,
        quote,
        quote_digest: digest,
        accepted_at,
        status,
    })
}

impl PostgresStore {
    /// Internal server-only issuance. Existing quote IDs cannot be revised.
    pub async fn put_managed_service_quote(&self, quote: &ManagedQuote) -> Result<()> {
        let digest = quote.digest()?;
        let json = serde_json::to_value(quote).map_err(|_| ManagedOrderStoreError::Integrity)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("INSERT INTO managed_service_quotes
            (quote_id,buyer_principal,quote_digest,quote,issued_at,expires_at) VALUES($1,$2,$3,$4,$5,$6)
            ON CONFLICT(quote_id) DO NOTHING")
            .bind(quote.input().quote_id).bind(&quote.input().buyer_principal).bind(&digest)
            .bind(json).bind(quote.input().issued_at).bind(quote.input().expires_at)
            .execute(&mut *tx).await?;
        let row = sqlx::query("SELECT * FROM managed_service_quotes WHERE quote_id=$1")
            .bind(quote.input().quote_id)
            .fetch_one(&mut *tx)
            .await?;
        if quote_from_row(&row)? != *quote {
            return Err(OrderError::Conflict.into());
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_managed_service_quote(
        &self,
        buyer: &str,
        quote_id: Uuid,
    ) -> Result<Option<ManagedQuote>> {
        let row = sqlx::query(
            "SELECT * FROM managed_service_quotes WHERE quote_id=$1 AND buyer_principal=$2",
        )
        .bind(quote_id)
        .bind(buyer)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(quote_from_row).transpose()
    }

    /// The principal is authenticated by the caller; the body supplies only quote ID,
    /// exact accepted digest and retry key. No amount or acceptance time comes from it.
    pub async fn accept_managed_service_quote(
        &self,
        buyer: &str,
        quote_id: Uuid,
        accepted_digest: &str,
        idempotency_key: &str,
    ) -> Result<ManagedOrder> {
        self.accept_managed_service_quote_with_intake(
            buyer,
            quote_id,
            accepted_digest,
            idempotency_key,
            true,
        )
        .await
    }

    /// Containment rejects new orders but still returns exact committed acceptances.
    pub async fn accept_managed_service_quote_with_intake(
        &self,
        buyer: &str,
        quote_id: Uuid,
        accepted_digest: &str,
        idempotency_key: &str,
        allow_new: bool,
    ) -> Result<ManagedOrder> {
        if !valid_key(buyer) || !valid_key(idempotency_key) {
            return Err(OrderError::Conflict.into());
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query("SELECT * FROM managed_service_quotes WHERE quote_id=$1 AND buyer_principal=$2 FOR UPDATE")
            .bind(quote_id).bind(buyer).fetch_optional(&mut *tx).await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        let quote = quote_from_row(&row)?;
        if quote.digest()? != accepted_digest {
            return Err(OrderError::Conflict.into());
        }
        // An exact replay remains valid after expiry, but cannot change key or content.
        let existing = sqlx::query(&format!("{ORDER_SELECT} WHERE o.quote_id=$1"))
            .bind(quote_id)
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(row) = existing {
            if row.try_get::<String, _>("idempotency_key")? != idempotency_key {
                return Err(OrderError::Conflict.into());
            }
            let order = order_from_row(&row)?;
            tx.commit().await?;
            return Ok(order);
        }
        if !allow_new {
            return Err(ManagedOrderStoreError::IntakeDisabled);
        }
        // clock_timestamp (not transaction-start now()) is sampled AFTER the lock.
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        quote.validate_acceptance(buyer, accepted_digest, now)?;
        let order_id = Uuid::new_v4();
        let inserted = sqlx::query(
            "INSERT INTO managed_service_orders
            (order_id,quote_id,buyer_principal,quote_digest,accepted_at,idempotency_key)
            VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING",
        )
        .bind(order_id)
        .bind(quote_id)
        .bind(buyer)
        .bind(accepted_digest)
        .bind(now)
        .bind(idempotency_key)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted != 1 {
            return Err(OrderError::Conflict.into());
        }
        sqlx::query(
            "INSERT INTO managed_service_actions(order_id,quote_digest,action,state)
            VALUES($1,$2,'prepare_checkout','held')",
        )
        .bind(order_id)
        .bind(accepted_digest)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ManagedOrder {
            order_id,
            quote,
            quote_digest: accepted_digest.into(),
            accepted_at: now,
            status: ManagedOrderStatus::AwaitingPayment,
        })
    }

    pub async fn get_managed_service_order(
        &self,
        buyer: &str,
        order_id: Uuid,
    ) -> Result<Option<ManagedOrder>> {
        let row = sqlx::query(&format!(
            "{ORDER_SELECT} WHERE o.order_id=$1 AND o.buyer_principal=$2"
        ))
        .bind(order_id)
        .bind(buyer)
        .fetch_optional(&self.pool)
        .await?;
        row.as_ref().map(order_from_row).transpose()
    }
}

#[cfg(test)]
mod stripe_execution_tests;
#[cfg(test)]
mod stripe_tests;
#[cfg(test)]
mod tests;
