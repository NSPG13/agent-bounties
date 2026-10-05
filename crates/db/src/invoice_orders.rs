//! Durable storage for invoiced seller-of-record orders. Events are append-only with a gap-free
//! per-order sequence, and each external evidence id (Stripe event id or chain log key) is unique
//! across all orders. The API replays events through `invoicing::project_order`, which owns every
//! business rule; this module only stores what that projection accepted.

use super::{DbResult, PostgresStore};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{postgres::PgRow, Row};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InvoiceOrderRecord {
    pub order_id: String,
    pub buyer_name: String,
    pub buyer_email: String,
    pub stripe_customer_id: Option<String>,
    pub stripe_invoice_id: Option<String>,
    pub hosted_invoice_url: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InvoiceOrderEventRecord {
    pub sequence: i32,
    pub kind: String,
    pub event: Value,
    pub evidence_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceEventAppend {
    Appended,
    /// The evidence id was already recorded (a Stripe retry or a re-run reconciliation).
    DuplicateEvidence,
    /// Another writer appended first; reload and replay.
    SequenceConflict,
}

fn order_from_row(row: PgRow) -> DbResult<InvoiceOrderRecord> {
    Ok(InvoiceOrderRecord {
        order_id: row.try_get("order_id")?,
        buyer_name: row.try_get("buyer_name")?,
        buyer_email: row.try_get("buyer_email")?,
        stripe_customer_id: row.try_get("stripe_customer_id")?,
        stripe_invoice_id: row.try_get("stripe_invoice_id")?,
        hosted_invoice_url: row.try_get("hosted_invoice_url")?,
        created_at: row.try_get("created_at")?,
    })
}

fn is_unique_violation(error: &sqlx::Error, constraint_fragment: &str) -> bool {
    error
        .as_database_error()
        .filter(|error| error.code().as_deref() == Some("23505"))
        .and_then(|error| error.constraint())
        .is_some_and(|constraint| constraint.contains(constraint_fragment))
}

impl PostgresStore {
    /// Creates the order and its sequence-0 quote event atomically. Returns `false` when the order
    /// id already exists.
    pub async fn create_invoice_order(
        &self,
        order_id: &str,
        buyer_name: &str,
        buyer_email: &str,
        quoted_event: &Value,
    ) -> DbResult<bool> {
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query(
            r#"
            INSERT INTO invoice_orders (order_id, buyer_name, buyer_email)
            VALUES ($1, $2, $3)
            ON CONFLICT (order_id) DO NOTHING
            "#,
        )
        .bind(order_id)
        .bind(buyer_name)
        .bind(buyer_email)
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            == 1;
        if !inserted {
            transaction.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            r#"
            INSERT INTO invoice_order_events (order_id, sequence, kind, event)
            VALUES ($1, 0, 'quoted', $2)
            "#,
        )
        .bind(order_id)
        .bind(quoted_event)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    pub async fn get_invoice_order(&self, order_id: &str) -> DbResult<Option<InvoiceOrderRecord>> {
        sqlx::query(
            r#"
            SELECT order_id, buyer_name, buyer_email, stripe_customer_id, stripe_invoice_id,
                   hosted_invoice_url, created_at
            FROM invoice_orders WHERE order_id = $1
            "#,
        )
        .bind(order_id)
        .fetch_optional(&self.pool)
        .await?
        .map(order_from_row)
        .transpose()
    }

    pub async fn list_invoice_orders(&self, limit: i64) -> DbResult<Vec<InvoiceOrderRecord>> {
        sqlx::query(
            r#"
            SELECT order_id, buyer_name, buyer_email, stripe_customer_id, stripe_invoice_id,
                   hosted_invoice_url, created_at
            FROM invoice_orders ORDER BY created_at DESC, order_id LIMIT $1
            "#,
        )
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(order_from_row)
        .collect()
    }

    /// Every order id, oldest first, for reports that must not miss an order.
    pub async fn list_invoice_order_ids(&self) -> DbResult<Vec<String>> {
        Ok(
            sqlx::query_scalar("SELECT order_id FROM invoice_orders ORDER BY created_at, order_id")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    /// Records Stripe object ids as they are created so a retried invoice run resumes instead of
    /// creating duplicates. Ids already set are never overwritten.
    pub async fn set_invoice_order_stripe_refs(
        &self,
        order_id: &str,
        customer_id: Option<&str>,
        invoice_id: Option<&str>,
        hosted_invoice_url: Option<&str>,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            UPDATE invoice_orders SET
              stripe_customer_id = COALESCE(stripe_customer_id, $2),
              stripe_invoice_id = COALESCE(stripe_invoice_id, $3),
              hosted_invoice_url = COALESCE(hosted_invoice_url, $4),
              updated_at = now()
            WHERE order_id = $1
            "#,
        )
        .bind(order_id)
        .bind(customer_id)
        .bind(invoice_id)
        .bind(hosted_invoice_url)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_invoice_order_id_by_stripe_invoice(
        &self,
        invoice_id: &str,
    ) -> DbResult<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT order_id FROM invoice_orders WHERE stripe_invoice_id = $1")
                .bind(invoice_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn list_invoice_order_events(
        &self,
        order_id: &str,
    ) -> DbResult<Vec<InvoiceOrderEventRecord>> {
        sqlx::query(
            r#"
            SELECT sequence, kind, event, evidence_id, recorded_at
            FROM invoice_order_events WHERE order_id = $1 ORDER BY sequence
            "#,
        )
        .bind(order_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            Ok(InvoiceOrderEventRecord {
                sequence: row.try_get("sequence")?,
                kind: row.try_get("kind")?,
                event: row.try_get("event")?,
                evidence_id: row.try_get("evidence_id")?,
                recorded_at: row.try_get("recorded_at")?,
            })
        })
        .collect()
    }

    /// Appends at exactly `sequence`, the number of events the caller replayed. Optimistic
    /// concurrency: a concurrent append makes this one fail as `SequenceConflict`.
    pub async fn append_invoice_order_event(
        &self,
        order_id: &str,
        sequence: i32,
        kind: &str,
        event: &Value,
        evidence_id: Option<&str>,
    ) -> DbResult<InvoiceEventAppend> {
        let result = sqlx::query(
            r#"
            INSERT INTO invoice_order_events (order_id, sequence, kind, event, evidence_id)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(order_id)
        .bind(sequence)
        .bind(kind)
        .bind(event)
        .bind(evidence_id)
        .execute(&self.pool)
        .await;
        match result {
            Ok(_) => Ok(InvoiceEventAppend::Appended),
            Err(error) if is_unique_violation(&error, "evidence_id") => {
                Ok(InvoiceEventAppend::DuplicateEvidence)
            }
            Err(error) if is_unique_violation(&error, "pkey") => {
                Ok(InvoiceEventAppend::SequenceConflict)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn upsert_invoice_contractor(
        &self,
        contractor_id: &str,
        wallet: &str,
        record: &Value,
    ) -> DbResult<()> {
        sqlx::query(
            r#"
            INSERT INTO invoice_contractors (contractor_id, wallet, record)
            VALUES ($1, lower($2), $3)
            ON CONFLICT (contractor_id) DO UPDATE SET
              wallet = EXCLUDED.wallet,
              record = EXCLUDED.record,
              updated_at = now()
            "#,
        )
        .bind(contractor_id)
        .bind(wallet)
        .bind(record)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_invoice_contractor(&self, contractor_id: &str) -> DbResult<Option<Value>> {
        Ok(
            sqlx::query_scalar("SELECT record FROM invoice_contractors WHERE contractor_id = $1")
                .bind(contractor_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn list_invoice_contractors(&self) -> DbResult<Vec<Value>> {
        Ok(
            sqlx::query_scalar("SELECT record FROM invoice_contractors ORDER BY contractor_id")
                .fetch_all(&self.pool)
                .await?,
        )
    }
}
