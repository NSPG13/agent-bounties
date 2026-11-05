//! Durable transactional review mail. Canonical projections supply source keys,
//! never recipient addresses. Only a proven Base wallet link can select a contact.

use super::PostgresStore;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{postgres::PgRow, Row};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum ReviewNotificationError {
    // Do not retain SQL/provider errors that might contain an email address.
    #[error("review_notification_storage_unavailable")]
    Database,
    #[error("invalid_review_notification_input")]
    Invalid,
    #[error("review_notification_source_conflict")]
    SourceConflict,
    #[error("review_notification_account_unavailable")]
    AccountUnavailable,
}

impl From<sqlx::Error> for ReviewNotificationError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

type Result<T> = std::result::Result<T, ReviewNotificationError>;

// This is a private, signed-account response. Do not include it in telemetry.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct ReviewNotificationPreferences {
    pub verified_email: Option<String>,
    pub enabled: bool,
    pub verified_wallet_count: i64,
    pub verified_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReviewNotification {
    pub kind: String,
    pub network: String,
    pub bounty_contract: String,
    pub bounty_id: String,
    pub round: i64,
    pub verifier_wallet: String,
    pub submission_log_key: String,
    pub submitted_at: DateTime<Utc>,
    /// The separate on-chain review deadline, never the bounty delivery deadline.
    pub deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNotificationRound {
    pub bounty_contract: String,
    pub round: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewNotificationSync {
    pub active: u64,
    pub suspended: u64,
    pub cancelled: u64,
    pub waiting_contact: u64,
    pub opted_out: u64,
    pub accepted: u64,
    pub failed: u64,
}

// Deliberately neither Debug nor Serialize: these contain private recipient data.
pub struct LeasedReviewNotification {
    pub id: Uuid,
    pub lease_token: Uuid,
    pub notification: NewReviewNotification,
    pub account_id: String,
    pub recipient_email: String,
    pub provider_payload: Option<Value>,
}

pub struct ReviewNotificationAttempt {
    pub id: Uuid,
    pub lease_token: Uuid,
    pub idempotency_key: String,
    pub provider_payload: Value,
    pub first_attempt_at: DateTime<Utc>,
    pub retry_until: DateTime<Utc>,
}

pub enum ReviewNotificationOutcome {
    /// Provider acceptance is not evidence of mailbox delivery.
    Accepted {
        provider_id: String,
    },
    Retry {
        code: String,
        retry_after_seconds: u32,
    },
    Failed {
        code: String,
    },
}

fn valid_network(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

fn valid_hex(value: &str, bytes: usize) -> bool {
    value.len() == 2 + bytes * 2
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value[2..].bytes().any(|b| b != b'0')
}

fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.rsplit_once('@') else {
        return false;
    };
    (3..=320).contains(&value.len())
        && !local.is_empty()
        && !domain.is_empty()
        && !local.contains('@')
        && value
            .chars()
            .all(|ch| !ch.is_control() && !ch.is_whitespace())
}

fn valid_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn notification_from_row(row: &PgRow) -> std::result::Result<NewReviewNotification, sqlx::Error> {
    Ok(NewReviewNotification {
        kind: row.try_get("notification_kind")?,
        network: row.try_get("network")?,
        bounty_contract: row.try_get("bounty_contract")?,
        bounty_id: row.try_get("bounty_id")?,
        round: row.try_get("round")?,
        verifier_wallet: row.try_get("verifier_wallet")?,
        submission_log_key: row.try_get("submission_log_key")?,
        submitted_at: row.try_get("submitted_at")?,
        deadline: row.try_get("review_deadline")?,
    })
}

impl PostgresStore {
    /// Call only after fresh provider authentication. An old cookie's email is
    /// not verified contact evidence. Clearing a contact preserves its opt-out.
    pub async fn set_verified_review_email(
        &self,
        account_id: &str,
        provider: &str,
        email: Option<&str>,
    ) -> Result<()> {
        if email
            .is_some_and(|email| !matches!(provider, "google" | "github") || !valid_email(email))
        {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            r#"INSERT INTO verifier_review_contacts (account_id, provider, verified_email, verified_at)
               SELECT account_key, provider, $3, CASE WHEN $3::TEXT IS NULL THEN NULL ELSE now() END
               FROM site_auth_accounts WHERE account_key = $1 AND provider = $2
               ON CONFLICT (account_id) DO UPDATE SET provider = EXCLUDED.provider,
                 verified_email = EXCLUDED.verified_email, verified_at = EXCLUDED.verified_at,
                 updated_at = now()"#,
        ).bind(account_id).bind(provider).bind(email).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(ReviewNotificationError::AccountUnavailable);
        }
        // A retry must never reuse an idempotency key with a different address.
        sqlx::query(
            r#"UPDATE verifier_review_notifications SET status = 'cancelled', source_active = FALSE,
                 lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE,
                 last_code = 'verified_contact_changed', updated_at = now()
               WHERE recipient_account_id = $1 AND recipient_email IS DISTINCT FROM $2::TEXT
                 AND status IN ('pending', 'leased')"#,
        )
        .bind(account_id)
        .bind(email)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn review_notification_preferences(
        &self,
        account_id: &str,
    ) -> Result<ReviewNotificationPreferences> {
        let row = sqlx::query(
            r#"SELECT c.verified_email, COALESCE(c.enabled, TRUE) AS enabled, c.verified_at,
                 (SELECT count(*) FROM site_auth_wallets w WHERE w.account_key = a.account_key
                   AND w.chain_id = 8453 AND w.proof_method = 'eip191_personal_sign') AS verified_wallet_count
               FROM site_auth_accounts a LEFT JOIN verifier_review_contacts c ON c.account_id = a.account_key
               WHERE a.account_key = $1"#,
        ).bind(account_id).fetch_optional(&self.pool).await?
            .ok_or(ReviewNotificationError::AccountUnavailable)?;
        Ok(ReviewNotificationPreferences {
            verified_email: row.try_get("verified_email")?,
            enabled: row.try_get("enabled")?,
            verified_wallet_count: row.try_get("verified_wallet_count")?,
            verified_at: row.try_get("verified_at")?,
        })
    }

    pub async fn set_review_notifications_enabled(
        &self,
        account_id: &str,
        enabled: bool,
    ) -> Result<ReviewNotificationPreferences> {
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            r#"INSERT INTO verifier_review_contacts (account_id, provider, enabled)
               SELECT account_key, provider, $2 FROM site_auth_accounts WHERE account_key = $1
               ON CONFLICT (account_id) DO UPDATE SET enabled = EXCLUDED.enabled, updated_at = now()"#,
        ).bind(account_id).bind(enabled).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(ReviewNotificationError::AccountUnavailable);
        }
        if !enabled {
            sqlx::query(
                r#"UPDATE verifier_review_notifications j SET status = 'cancelled', source_active = FALSE,
                     lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE,
                     last_code = 'recipient_opted_out', updated_at = now()
                   WHERE j.status IN ('pending', 'leased') AND (j.recipient_account_id = $1
                     OR j.observed_account_id = $1 OR EXISTS (SELECT 1 FROM site_auth_wallets w
                       WHERE w.account_key = $1 AND w.wallet_address = j.verifier_wallet
                         AND w.chain_id = 8453 AND w.proof_method = 'eip191_personal_sign'))"#,
            ).bind(account_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        self.review_notification_preferences(account_id).await
    }

    /// Atomically reconcile one complete, fresh network projection. `retained_rounds`
    /// are still-current submissions with temporarily unavailable/invalid terms.
    /// Their existing jobs suspend, rather than irreversibly disappearing.
    pub async fn reconcile_review_notifications(
        &self,
        network: &str,
        active: &[NewReviewNotification],
        retained_rounds: &[ReviewNotificationRound],
    ) -> Result<ReviewNotificationSync> {
        if !valid_network(network) {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut keys = HashSet::new();
        for item in active {
            if !matches!(item.kind.as_str(), "submission" | "claim")
                || item.network != network
                || !valid_hex(&item.bounty_contract, 20)
                || !valid_hex(&item.bounty_id, 32)
                || !valid_hex(&item.verifier_wallet, 20)
                || item.round <= 0
                || item.submission_log_key.is_empty()
                || item.submission_log_key.len() > 256
                || item.submission_log_key.chars().any(char::is_control)
                || item
                    .deadline
                    .is_some_and(|deadline| deadline <= item.submitted_at)
                || !keys.insert((
                    &item.bounty_contract,
                    item.round,
                    &item.verifier_wallet,
                    &item.kind,
                ))
            {
                return Err(ReviewNotificationError::Invalid);
            }
        }
        if retained_rounds
            .iter()
            .any(|scope| !valid_hex(&scope.bounty_contract, 20) || scope.round <= 0)
        {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 4040))")
            .bind(network)
            .execute(&mut *tx)
            .await?;
        let sync_id = Uuid::new_v4();
        let mut result = ReviewNotificationSync::default();
        for item in active {
            let changed = sqlx::query(
                r#"INSERT INTO verifier_review_notifications AS j
                    (id, network, bounty_contract, bounty_id, round, verifier_wallet,
                     submission_log_key, submitted_at, review_deadline, last_seen_sync_id, notification_kind)
                   VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                   ON CONFLICT (network, bounty_contract, round, verifier_wallet, notification_kind) WHERE notification_kind IN ('submission','claim') DO UPDATE SET
                     source_active = CASE WHEN j.status IN ('pending', 'leased') THEN TRUE ELSE FALSE END,
                     last_seen_sync_id = EXCLUDED.last_seen_sync_id, updated_at = now()
                   WHERE j.bounty_id = EXCLUDED.bounty_id AND j.submission_log_key = EXCLUDED.submission_log_key
                     AND j.submitted_at = EXCLUDED.submitted_at
                     AND j.review_deadline IS NOT DISTINCT FROM EXCLUDED.review_deadline"#,
            ).bind(Uuid::new_v4()).bind(network).bind(&item.bounty_contract).bind(&item.bounty_id)
                .bind(item.round).bind(&item.verifier_wallet).bind(&item.submission_log_key)
                .bind(item.submitted_at).bind(item.deadline).bind(sync_id).bind(&item.kind).execute(&mut *tx).await?;
            if changed.rows_affected() != 1 {
                return Err(ReviewNotificationError::SourceConflict);
            }
            result.active += 1;
        }
        // Projection reads occur before this transaction. Absence from a stale
        // instance's input must never terminally cancel another instance's newer
        // submission. Require present canonical evidence or DB time for that.
        result.cancelled = sqlx::query(
            r#"UPDATE verifier_review_notifications j SET source_active = FALSE, status = 'cancelled',
                 lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE,
                 last_code = 'source_no_longer_active', updated_at = now()
               WHERE network = $1 AND notification_kind IN ('submission','claim') AND last_seen_sync_id <> $2 AND status IN ('pending', 'leased')
                 AND (j.review_deadline <= now() OR EXISTS (
                   SELECT 1 FROM autonomous_bounty_events s JOIN autonomous_bounty_events later
                     ON later.network = s.network AND later.contract_address = s.contract_address
                       AND later.bounty_id = s.bounty_id AND later.block_time_verified
                       AND (later.block_number, later.log_index) > (s.block_number, s.log_index)
                   WHERE s.network = j.network AND s.contract_address = j.bounty_contract
                     AND s.bounty_id = j.bounty_id AND s.log_key = j.submission_log_key
                     AND s.kind = CASE WHEN j.notification_kind = 'claim' THEN 'bounty_claimed' ELSE 'submission_added' END AND s.block_time_verified
                     AND later.kind IN ('bounty_became_claimable', 'bounty_claimed', 'submission_added',
                       'submission_rejected', 'bounty_settled', 'claim_expired', 'submission_expired', 'bounty_cancelled')))"#,
        ).bind(network).bind(sync_id).execute(&mut *tx).await?.rows_affected();
        // All remaining absent sources (including retained rounds and sources
        // omitted by a stale snapshot) suspend resumably and invalidate leases.
        result.suspended = sqlx::query(
            r#"UPDATE verifier_review_notifications SET source_active = FALSE, status = 'pending',
                 lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE, updated_at = now()
               WHERE network = $1 AND notification_kind IN ('submission','claim') AND last_seen_sync_id <> $2 AND status IN ('pending', 'leased')"#,
        ).bind(network).bind(sync_id).execute(&mut *tx).await?.rows_affected();
        let counts = sqlx::query(
            r#"SELECT
                 count(*) FILTER (WHERE j.source_active AND j.status = 'pending' AND NOT EXISTS (
                   SELECT 1 FROM site_auth_wallets w JOIN verifier_review_contacts c ON c.account_id = w.account_key
                   WHERE w.wallet_address = j.verifier_wallet AND w.chain_id = 8453
                     AND w.proof_method = 'eip191_personal_sign' AND c.verified_email IS NOT NULL)) AS waiting_contact,
                 count(*) FILTER (WHERE j.last_code = 'recipient_opted_out' OR EXISTS (
                   SELECT 1 FROM site_auth_wallets w JOIN verifier_review_contacts c ON c.account_id = w.account_key
                   WHERE w.wallet_address = j.verifier_wallet AND w.chain_id = 8453
                     AND w.proof_method = 'eip191_personal_sign' AND NOT c.enabled)) AS opted_out,
                 count(*) FILTER (WHERE j.status = 'accepted') AS accepted,
                 count(*) FILTER (WHERE j.status = 'failed') AS failed
               FROM verifier_review_notifications j WHERE j.network = $1"#,
        ).bind(network).fetch_one(&mut *tx).await?;
        result.waiting_contact = counts.try_get::<i64, _>("waiting_contact")? as u64;
        result.opted_out = counts.try_get::<i64, _>("opted_out")? as u64;
        result.accepted = counts.try_get::<i64, _>("accepted")? as u64;
        result.failed = counts.try_get::<i64, _>("failed")? as u64;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn claim_review_notifications(
        &self,
        network: &str,
        limit: u32,
        lease_seconds: u32,
    ) -> Result<Vec<LeasedReviewNotification>> {
        self.claim_review_notifications_for_protocol(network, limit, lease_seconds, false)
            .await
    }
    pub async fn claim_review_notifications_for_protocol(
        &self,
        network: &str,
        limit: u32,
        lease_seconds: u32,
        creator_open: bool,
    ) -> Result<Vec<LeasedReviewNotification>> {
        if !valid_network(network) || limit == 0 {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        // Missing contact is a waiting state. A known unlink, opt-out, expired
        // review, or exhausted provider idempotency window is terminal instead.
        sqlx::query(
            r#"UPDATE verifier_review_notifications j SET status = 'cancelled', source_active = FALSE,
                 lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE,
                 last_code = 'delivery_no_longer_authorized', updated_at = now()
               WHERE j.network = $1 AND j.status IN ('pending', 'leased') AND (
                 (j.review_deadline IS NOT NULL AND j.review_deadline <= now())
                 OR j.retry_until <= now() OR j.attempt_count >= 100
                 OR EXISTS (SELECT 1 FROM site_auth_wallets w JOIN verifier_review_contacts c ON c.account_id = w.account_key
                   WHERE w.wallet_address = j.verifier_wallet AND w.chain_id = 8453
                     AND w.proof_method = 'eip191_personal_sign' AND NOT c.enabled)
                 OR (j.observed_account_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM site_auth_wallets w
                   WHERE w.wallet_address = j.verifier_wallet AND w.account_key = j.observed_account_id
                     AND w.chain_id = 8453 AND w.proof_method = 'eip191_personal_sign'))
                 OR (j.first_attempt_at IS NOT NULL AND NOT EXISTS (SELECT 1 FROM site_auth_wallets w
                   JOIN verifier_review_contacts c ON c.account_id = w.account_key
                   WHERE w.wallet_address = j.verifier_wallet AND w.account_key = j.recipient_account_id
                     AND w.chain_id = 8453 AND w.proof_method = 'eip191_personal_sign'
                     AND c.enabled AND c.verified_email = j.recipient_email)))"#,
        ).bind(network).execute(&mut *tx).await?;
        let rows = sqlx::query(
            r#"SELECT j.*, w.account_key AS current_account_id, c.verified_email AS current_email
               FROM verifier_review_notifications j JOIN site_auth_wallets w
                 ON w.wallet_address = j.verifier_wallet AND w.chain_id = 8453 AND w.proof_method = 'eip191_personal_sign'
               JOIN verifier_review_contacts c ON c.account_id = w.account_key AND c.enabled AND c.verified_email IS NOT NULL
               WHERE j.network = $1 AND (j.notification_kind='creator_open_submission')=$3 AND j.source_active AND j.next_attempt_at <= now()
                 AND (j.status = 'pending' OR (j.status = 'leased' AND j.lease_expires_at <= now()))
                 AND (j.review_deadline IS NULL OR j.review_deadline > now())
                 AND (j.retry_until IS NULL OR j.retry_until > now()) AND j.attempt_count < 100
               ORDER BY j.review_deadline ASC NULLS LAST, j.next_attempt_at, j.created_at, j.id
               LIMIT $2 FOR UPDATE OF j SKIP LOCKED"#,
        ).bind(network).bind(i64::from(limit.min(100))).bind(creator_open).fetch_all(&mut *tx).await?;
        let mut leases = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get("id")?;
            let lease_token = Uuid::new_v4();
            let account_id: String = row.try_get("current_account_id")?;
            sqlx::query(
                r#"UPDATE verifier_review_notifications SET status = 'leased', lease_token = $2,
                     lease_expires_at = statement_timestamp() + make_interval(secs => $3), attempt_started = FALSE,
                     observed_account_id = COALESCE(observed_account_id, $4), updated_at = now() WHERE id = $1"#,
            ).bind(id).bind(lease_token).bind(f64::from(lease_seconds.clamp(30, 300)))
                .bind(&account_id).execute(&mut *tx).await?;
            leases.push(LeasedReviewNotification {
                id,
                lease_token,
                notification: notification_from_row(&row)?,
                account_id,
                recipient_email: row.try_get("current_email")?,
                provider_payload: row.try_get("provider_payload")?,
            });
        }
        tx.commit().await?;
        Ok(leases)
    }

    /// Admit exactly one provider attempt per lease. Call immediately before
    /// sending, with a provider timeout shorter than the remaining lease. The
    /// final database check cannot retract mail already accepted by a provider.
    pub async fn begin_review_notification_attempt(
        &self,
        lease: &LeasedReviewNotification,
        proposed_payload: &Value,
    ) -> Result<Option<ReviewNotificationAttempt>> {
        let payload = proposed_payload
            .as_object()
            .ok_or(ReviewNotificationError::Invalid)?;
        if payload.len() != 5
            || !["from", "subject", "text", "html"]
                .iter()
                .all(|key| payload.get(*key).is_some_and(Value::is_string))
            || payload
                .get("to")
                .and_then(Value::as_array)
                .is_none_or(|recipients| {
                    recipients.len() != 1 || recipients[0].as_str() != Some(&lease.recipient_email)
                })
            || serde_json::to_vec(proposed_payload)
                .map_err(|_| ReviewNotificationError::Invalid)?
                .len()
                > 65_536
        {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        // Lock contact/link before job, matching preference/contact invalidation
        // order. Deletion or opt-out that has committed cannot be bypassed here.
        let recipient = sqlx::query(
            r#"SELECT w.account_key FROM site_auth_wallets w JOIN verifier_review_contacts c ON c.account_id = w.account_key
               WHERE w.wallet_address = $1 AND w.account_key = $2 AND w.chain_id = 8453
                 AND w.proof_method = 'eip191_personal_sign' AND c.enabled AND c.verified_email = $3
               FOR SHARE OF w, c"#,
        ).bind(&lease.notification.verifier_wallet).bind(&lease.account_id).bind(&lease.recipient_email)
            .fetch_optional(&mut *tx).await?;
        if recipient.is_none() {
            tx.commit().await?;
            return Ok(None);
        }
        // Acquire the job before sampling admission time. PostgreSQL now() is
        // transaction-start time and would be stale after a contact/job lock wait.
        let locked = sqlx::query("SELECT id FROM verifier_review_notifications WHERE id = $1 AND lease_token = $2 FOR UPDATE")
            .bind(lease.id).bind(lease.lease_token).fetch_optional(&mut *tx).await?;
        if locked.is_none() {
            tx.commit().await?;
            return Ok(None);
        }
        let row = sqlx::query(
            r#"UPDATE verifier_review_notifications j SET
                 recipient_account_id = COALESCE(j.recipient_account_id, $3),
                 recipient_email = COALESCE(j.recipient_email, $4),
                 provider_payload = COALESCE(j.provider_payload, $5),
                 first_attempt_at = COALESCE(j.first_attempt_at, statement_timestamp()),
                 retry_until = COALESCE(j.retry_until, statement_timestamp() + interval '23 hours'),
                 attempt_count = j.attempt_count + 1, attempt_started = TRUE, updated_at = statement_timestamp()
               WHERE j.id = $1 AND j.lease_token = $2 AND j.status = 'leased' AND j.source_active
                 AND j.lease_expires_at > statement_timestamp() + interval '15 seconds' AND NOT j.attempt_started
                 AND j.next_attempt_at <= statement_timestamp() AND j.submitted_at <= statement_timestamp()
                 AND (j.review_deadline IS NULL OR j.review_deadline > statement_timestamp())
                 AND (j.retry_until IS NULL OR j.retry_until > statement_timestamp()) AND j.attempt_count < 100
                 AND j.verifier_wallet = $6 AND j.observed_account_id = $3
                 AND (j.recipient_account_id IS NULL OR (j.recipient_account_id = $3 AND j.recipient_email = $4))
                 AND ((j.notification_kind IN ('submission','claim') AND EXISTS (
                   SELECT 1 FROM autonomous_bounty_events s
                   WHERE s.network = j.network AND s.contract_address = j.bounty_contract
                     AND s.bounty_id = j.bounty_id AND s.log_key = j.submission_log_key
                     AND s.kind = CASE WHEN j.notification_kind = 'claim' THEN 'bounty_claimed' ELSE 'submission_added' END AND s.block_time_verified
                     AND s.data ->> 'round' = j.round::TEXT AND s.occurred_at = j.submitted_at
                     AND (j.review_deadline IS NULL OR s.data ->> (CASE WHEN j.notification_kind = 'claim' THEN 'claim_expires_at' ELSE 'verification_expires_at' END) =
                       floor(extract(epoch FROM j.review_deadline))::BIGINT::TEXT)
                     AND NOT EXISTS (SELECT 1 FROM autonomous_bounty_events later
                       WHERE later.network = s.network AND later.contract_address = s.contract_address
                         AND later.bounty_id = s.bounty_id AND later.block_time_verified
                         AND (later.block_number, later.log_index) > (s.block_number, s.log_index)
                         AND later.kind IN ('bounty_became_claimable', 'bounty_claimed', 'submission_added',
                           'submission_rejected', 'bounty_settled', 'claim_expired', 'submission_expired', 'bounty_cancelled')))) OR (j.notification_kind='creator_open_submission' AND EXISTS (
                   SELECT 1 FROM creator_open_review_sources s WHERE s.network=j.network AND s.bounty_contract=j.bounty_contract
                   AND s.bounty_id=j.bounty_id AND s.round=j.round AND s.verifier_wallet=j.verifier_wallet
                   AND s.submission_log_key=j.submission_log_key AND s.submitted_at=j.submitted_at AND s.review_deadline=j.review_deadline)))
               RETURNING id, provider_payload, first_attempt_at, retry_until"#,
        ).bind(lease.id).bind(lease.lease_token).bind(&lease.account_id).bind(&lease.recipient_email)
            .bind(proposed_payload).bind(&lease.notification.verifier_wallet).fetch_optional(&mut *tx).await?;
        let attempt = row
            .map(|row| -> Result<_> {
                Ok(ReviewNotificationAttempt {
                    id: row.try_get("id")?,
                    lease_token: lease.lease_token,
                    idempotency_key: format!("verifier-review/{}", lease.id),
                    provider_payload: row.try_get("provider_payload")?,
                    first_attempt_at: row.try_get("first_attempt_at")?,
                    retry_until: row.try_get("retry_until")?,
                })
            })
            .transpose()?;
        tx.commit().await?;
        Ok(attempt)
    }

    pub async fn finish_review_notification_attempt(
        &self,
        job_id: Uuid,
        lease_token: Uuid,
        outcome: ReviewNotificationOutcome,
    ) -> Result<bool> {
        let (status, code, delay, provider_id) = match outcome {
            ReviewNotificationOutcome::Accepted { provider_id } => {
                if provider_id.is_empty()
                    || provider_id.len() > 256
                    || provider_id.chars().any(char::is_control)
                {
                    return Err(ReviewNotificationError::Invalid);
                }
                (
                    "accepted",
                    "provider_accepted".to_string(),
                    0,
                    Some(provider_id),
                )
            }
            ReviewNotificationOutcome::Retry {
                code,
                retry_after_seconds,
            } => ("pending", code, retry_after_seconds.max(30), None),
            ReviewNotificationOutcome::Failed { code } => ("failed", code, 0, None),
        };
        if !valid_code(&code) {
            return Err(ReviewNotificationError::Invalid);
        }
        let mut tx = self.pool.begin().await?;
        let locked = sqlx::query("SELECT id FROM verifier_review_notifications WHERE id = $1 AND lease_token = $2 FOR UPDATE")
            .bind(job_id).bind(lease_token).fetch_optional(&mut *tx).await?;
        if locked.is_none() {
            tx.commit().await?;
            return Ok(false);
        }
        let result = sqlx::query(
            r#"UPDATE verifier_review_notifications SET
                 status = CASE WHEN $3 = 'pending' AND (statement_timestamp() + make_interval(secs => $5) >= retry_until
                   OR attempt_count >= 100 OR review_deadline <= statement_timestamp() + make_interval(secs => $5))
                   THEN 'failed' ELSE $3 END,
                 source_active = CASE WHEN $3 = 'accepted' OR $3 = 'failed' THEN FALSE ELSE source_active END,
                 next_attempt_at = statement_timestamp() + make_interval(secs => $5), last_code = $4,
                 provider_id = $6, accepted_at = CASE WHEN $3 = 'accepted' THEN statement_timestamp() ELSE NULL END,
                 lease_token = NULL, lease_expires_at = NULL, attempt_started = FALSE, updated_at = statement_timestamp()
               WHERE id = $1 AND lease_token = $2 AND status = 'leased' AND (attempt_started OR $3 = 'failed')
                 AND lease_expires_at > statement_timestamp()"#,
        ).bind(job_id).bind(lease_token).bind(status).bind(code).bind(f64::from(delay))
            .bind(provider_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use sqlx::postgres::PgConnectOptions;
    use std::str::FromStr;

    async fn fixture() -> PostgresStore {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("explicit local fixture URL required");
        let options = PgConnectOptions::from_str(&url).expect("fixture URL must parse");
        assert!(
            matches!(options.get_host(), "127.0.0.1" | "localhost" | "::1"),
            "review tests require loopback PostgreSQL"
        );
        assert!(
            options
                .get_database()
                .is_some_and(|name| name.contains("test")),
            "review tests require an explicitly named test database"
        );
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        store
    }

    fn network() -> String {
        format!("review-test-{}", Uuid::new_v4().simple())
    }

    fn address() -> String {
        format!("0x{}00000000", Uuid::new_v4().simple())
    }

    async fn account(store: &PostgresStore, provider: &str) -> String {
        let id = hex::encode(Sha256::digest(Uuid::new_v4().as_bytes()));
        store
            .upsert_site_auth_account(
                &id,
                provider,
                &id,
                "Review fixture",
                "unverified@example.test",
                "",
            )
            .await
            .unwrap();
        id
    }

    async fn linked_contact(store: &PostgresStore) -> (String, String, String) {
        let owner = account(store, "google").await;
        let wallet = address();
        let email = format!("{}@example.test", Uuid::new_v4().simple());
        store
            .link_site_auth_wallet(&owner, &wallet, 8453)
            .await
            .unwrap();
        store
            .set_verified_review_email(&owner, "google", Some(&email))
            .await
            .unwrap();
        (owner, wallet, email)
    }

    fn job(network: &str, wallet: &str, deadline: Option<DateTime<Utc>>) -> NewReviewNotification {
        NewReviewNotification {
            kind: "submission".into(),
            network: network.to_owned(),
            bounty_contract: address(),
            bounty_id: format!("0x{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            round: 1,
            verifier_wallet: wallet.to_owned(),
            submission_log_key: format!("review-fixture:{}", Uuid::new_v4()),
            submitted_at: DateTime::from_timestamp(Utc::now().timestamp() - 60, 0).unwrap(),
            deadline,
        }
    }

    async fn canonical(store: &PostgresStore, item: &NewReviewNotification) {
        sqlx::query(
            "INSERT INTO autonomous_bounty_events (id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,occurred_at,block_time_verified) VALUES ($1,$2,$3,'fixture',1,1,$4,$5,$8,$6,$7,TRUE)",
        ).bind(Uuid::new_v4()).bind(&item.submission_log_key).bind(&item.network)
            .bind(&item.bounty_contract).bind(&item.bounty_id)
            .bind(json!({"round":item.round,"verification_expires_at":item.deadline.map(|date| date.timestamp()),"claim_expires_at":item.deadline.map(|date| date.timestamp())}))
            .bind(item.submitted_at).bind(if item.kind=="claim" {"bounty_claimed"} else {"submission_added"}).execute(&store.pool).await.unwrap();
    }

    fn payload(email: &str, sender: &str) -> Value {
        json!({"from":sender,"to":[email],"subject":"A solution is ready for your review",
            "text":"Open your private review page.","html":"<p>Open your private review page.</p>"})
    }

    async fn enqueue(store: &PostgresStore, items: &[NewReviewNotification]) {
        for item in items {
            canonical(store, item).await;
        }
        store
            .reconcile_review_notifications(&items[0].network, items, &[])
            .await
            .unwrap();
    }

    async fn expire_lease(store: &PostgresStore, id: Uuid) {
        sqlx::query("UPDATE verifier_review_notifications SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
            .bind(id).execute(&store.pool).await.unwrap();
    }

    async fn due_now(store: &PostgresStore, id: Uuid) {
        sqlx::query("UPDATE verifier_review_notifications SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
            .bind(id).execute(&store.pool).await.unwrap();
    }

    #[test]
    fn notification_inputs_and_diagnostic_errors_do_not_accept_contact_injection() {
        assert!(valid_network("base-mainnet"));
        assert!(!valid_network("base mainnet"));
        assert!(!valid_hex("0x0000000000000000000000000000000000000000", 20));
        assert!(valid_email("reviewer+mail@example.test"));
        for invalid in [
            "x@y\r\nBcc:z@y",
            "two addresses@example.test",
            "a@@example.test",
            "@example.test",
        ] {
            assert!(!valid_email(invalid));
        }
        assert!(valid_code("provider_rate_limited"));
        assert!(!valid_code("recipient@example.test"));
        let error =
            ReviewNotificationError::from(sqlx::Error::Protocol("private@example.test".into()));
        assert!(!format!("{error:?}: {error}").contains("private@example.test"));
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn push_delivery_tests_bind_owner_deduplicate_and_limit_across_devices() {
        use crate::PushTestAdmission;
        use base64::Engine;
        let store = fixture().await;
        let owner = account(&store, "github").await;
        let other = account(&store, "github").await;
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([vec![4], vec![7; 64]].concat());
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3; 16]);
        let endpoint = format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4());
        let subscription = store
            .save_push_subscription(&owner, &endpoint, &key, &auth)
            .await
            .unwrap();
        let request = Uuid::new_v4();
        assert!(matches!(
            store
                .reserve_push_delivery_test(&other, subscription, request)
                .await
                .unwrap(),
            PushTestAdmission::Unavailable
        ));
        let (a, b) = tokio::join!(
            store.reserve_push_delivery_test(&owner, subscription, request),
            store.reserve_push_delivery_test(&owner, subscription, request)
        );
        let (delivery, prior) = match (a.unwrap(), b.unwrap()) {
            (PushTestAdmission::Send(d), PushTestAdmission::Recorded(s))
            | (PushTestAdmission::Recorded(s), PushTestAdmission::Send(d)) => (d, s),
            _ => panic!("duplicate request must reserve exactly one provider attempt"),
        };
        assert_eq!(prior, "attempting");
        assert!(store
            .push_delivery_test_is_current(&delivery)
            .await
            .unwrap());
        // Reconnecting cannot turn an interrupted attempt into a new send.
        let reopened =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        assert!(matches!(
            reopened
                .reserve_push_delivery_test(&owner, subscription, request)
                .await
                .unwrap(),
            PushTestAdmission::Recorded(_)
        ));
        assert!(matches!(
            store
                .reserve_push_delivery_test(&owner, subscription, Uuid::new_v4())
                .await
                .unwrap(),
            PushTestAdmission::RateLimited
        ));
        store
            .finish_push_delivery_test(&delivery, "accepted", false)
            .await
            .unwrap();
        assert!(!store
            .push_delivery_test_is_current(&delivery)
            .await
            .unwrap());
        assert!(
            matches!(store.reserve_push_delivery_test(&owner,subscription,request).await.unwrap(),PushTestAdmission::Recorded(s) if s=="accepted")
        );
        // Per-account quotas also apply when a different device is selected.
        let second = store
            .save_push_subscription(
                &owner,
                &format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4()),
                &key,
                &auth,
            )
            .await
            .unwrap();
        assert!(matches!(
            store
                .reserve_push_delivery_test(&owner, second, Uuid::new_v4())
                .await
                .unwrap(),
            PushTestAdmission::RateLimited
        ));
        for _ in 0..4 {
            sqlx::query("UPDATE push_delivery_tests SET created_at=now()-interval '2 minutes' WHERE account_id=$1").bind(&owner).execute(&store.pool).await.unwrap();
            assert!(matches!(
                store
                    .reserve_push_delivery_test(&owner, second, Uuid::new_v4())
                    .await
                    .unwrap(),
                PushTestAdmission::Send(_)
            ));
        }
        sqlx::query("UPDATE push_delivery_tests SET created_at=now()-interval '2 minutes' WHERE account_id=$1").bind(&owner).execute(&store.pool).await.unwrap();
        assert!(matches!(
            store
                .reserve_push_delivery_test(&owner, second, Uuid::new_v4())
                .await
                .unwrap(),
            PushTestAdmission::RateLimited
        ));
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn push_delivery_tests_cancel_after_opt_out_renewal_or_old_reservation() {
        use crate::PushTestAdmission;
        use base64::Engine;
        let store = fixture().await;
        let owner = account(&store, "github").await;
        let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([vec![4], vec![7; 64]].concat());
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3; 16]);
        let endpoint = format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4());
        let subscription = store
            .save_push_subscription(&owner, &endpoint, &key, &auth)
            .await
            .unwrap();
        let request = Uuid::new_v4();
        let PushTestAdmission::Send(d) = store
            .reserve_push_delivery_test(&owner, subscription, request)
            .await
            .unwrap()
        else {
            panic!("new attempt")
        };
        store
            .remove_push_subscription(&owner, subscription)
            .await
            .unwrap();
        assert!(!store.push_delivery_test_is_current(&d).await.unwrap());
        assert!(matches!(
            store
                .reserve_push_delivery_test(&owner, subscription, request)
                .await
                .unwrap(),
            PushTestAdmission::Unavailable
        ));
        store
            .save_push_subscription(&owner, &endpoint, &key, &auth)
            .await
            .unwrap();
        assert!(!store.push_delivery_test_is_current(&d).await.unwrap());
        store
            .finish_push_delivery_test(&d, "failed", true)
            .await
            .unwrap();
        assert_eq!(
            store.active_push_device_count(&owner).await.unwrap(),
            1,
            "old provider rejection cannot revoke a renewed device"
        );
        sqlx::query("UPDATE push_delivery_tests SET created_at=now()-interval '2 minutes' WHERE account_id=$1").bind(&owner).execute(&store.pool).await.unwrap();
        let PushTestAdmission::Send(fresh) = store
            .reserve_push_delivery_test(&owner, subscription, Uuid::new_v4())
            .await
            .unwrap()
        else {
            panic!("fresh attempt")
        };
        sqlx::query(
            "UPDATE push_delivery_tests SET created_at=now()-interval '21 seconds' WHERE id=$1",
        )
        .bind(fresh.id)
        .execute(&store.pool)
        .await
        .unwrap();
        assert!(!store.push_delivery_test_is_current(&fresh).await.unwrap());
        store
            .finish_push_delivery_test(&fresh, "unknown", false)
            .await
            .unwrap();
        assert!(
            matches!(store.reserve_push_delivery_test(&owner,subscription,fresh.id).await.unwrap(),PushTestAdmission::Recorded(s) if s=="unknown")
        );
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn creator_open_notices_follow_fenced_pending_sources_and_ignore_legacy_sync() {
        use base64::Engine;
        let store = fixture().await;
        let (owner, wallet, email) = linked_contact(&store).await;
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../site/fixtures/creator-open-preparations.json"
        ))
        .unwrap();
        let mut request: chain_base::creator_open_service::Request =
            serde_json::from_value(vectors["plans"][0]["request"].clone()).unwrap();
        request.original_terms.network = "base-mainnet".into();
        request.original_terms.creator = wallet.clone();
        request.original_terms.bounty_id =
            format!("0x{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let contract = address();
        store
            .save_creator_open_external_creation(
                "base-mainnet",
                &contract,
                &format!("0x{}", "11".repeat(32)),
                &request,
            )
            .await
            .unwrap();
        let index = store
            .creator_open_index_batch("base-mainnet", Some(&contract))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let block = chain_base::BaseBlockIdentity {
            number: 1,
            hash: format!("0x{}", "12".repeat(32)),
            timestamp: Utc::now().timestamp() as u64,
        };
        let mut projection: chain_base::creator_open::Projection=serde_json::from_value(json!({
            "protocol":"agent-bounties/creator-open-v1","contract":contract,"bounty_id":request.original_terms.bounty_id,
            "creator":wallet,"terms_hash":format!("0x{}","13".repeat(32)),"revision":1,"solver_reward":20000000,"review_reward":1000000,
            "submission_deadline":block.timestamp+86400,"review_window":3600,"max_pending":8,"status":"open","principal":21000000,"outstanding_bonds":1000000,
            "entries":{"1":{"solver":address(),"submission_hash":format!("0x{}","14".repeat(32)),"evidence_hash":format!("0x{}","15".repeat(32)),"review_deadline":block.timestamp+3540,"status":"pending"}},"winner":null,"settlement_transaction":null
        })).unwrap();
        assert!(store
            .save_creator_open_index(&index, &block, &[], &projection, true)
            .await
            .unwrap());
        store
            .reconcile_review_notifications("base-mainnet", &[], &[])
            .await
            .unwrap();
        assert!(store
            .claim_review_notifications("base-mainnet", 100, 60)
            .await
            .unwrap()
            .iter()
            .all(|j| j.notification.kind != "creator_open_submission"));
        let lease = store
            .claim_review_notifications_for_protocol("base-mainnet", 100, 60, true)
            .await
            .unwrap()
            .into_iter()
            .find(|j| j.notification.bounty_contract == contract)
            .unwrap();
        assert_eq!(lease.notification.verifier_wallet, wallet);
        assert_eq!(lease.notification.round, 1);
        let endpoint = format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4());
        let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([vec![4], vec![1; 64]].concat());
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2; 16]);
        store
            .save_push_subscription(&owner, &endpoint, &p256dh, &auth)
            .await
            .unwrap();
        store.enqueue_creator_push("base-mainnet").await.unwrap();
        let push = store
            .claim_creator_push_for_protocol("base-mainnet", true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(push.entry, 1);
        assert!(store.creator_push_is_current(&push).await.unwrap());
        let current = store
            .creator_open_index_batch("base-mainnet", Some(&contract))
            .await
            .unwrap()
            .pop()
            .unwrap();
        store
            .invalidate_creator_open_index(&current, true)
            .await
            .unwrap();
        assert!(!store.creator_push_is_current(&push).await.unwrap());
        assert!(store
            .begin_review_notification_attempt(&lease, &payload(&email, "reviews@example.test"))
            .await
            .unwrap()
            .is_none());
        assert!(!store
            .save_creator_open_index(&current, &block, &[], &projection, true)
            .await
            .unwrap());
        let fresh = store
            .creator_open_index_batch("base-mainnet", Some(&contract))
            .await
            .unwrap()
            .pop()
            .unwrap();
        projection.entries.get_mut(&1).unwrap().evidence_hash = format!("0x{}", "16".repeat(32));
        store
            .save_creator_open_index(&fresh, &block, &[], &projection, true)
            .await
            .unwrap();
        let replaced = store
            .claim_review_notifications_for_protocol("base-mainnet", 100, 60, true)
            .await
            .unwrap()
            .into_iter()
            .find(|j| j.notification.bounty_contract == contract)
            .unwrap();
        assert_ne!(
            lease.id, replaced.id,
            "a changed canonical entry has a new notice identity"
        );
        assert!(store
            .begin_review_notification_attempt(&lease, &payload(&email, "reviews@example.test"))
            .await
            .unwrap()
            .is_none());
        store
            .reconcile_review_notifications("base-mainnet", &[], &[])
            .await
            .unwrap();
        assert!(store
            .begin_review_notification_attempt(&replaced, &payload(&email, "reviews@example.test"))
            .await
            .unwrap()
            .is_some());
        // A provider's late gone response cannot disable a renewed subscription.
        store
            .save_push_subscription(&owner, &endpoint, &p256dh, &auth)
            .await
            .unwrap();
        store
            .finish_creator_push(&push, false, false, true)
            .await
            .unwrap();
        assert_eq!(store.active_push_device_count(&owner).await.unwrap(), 1);
        sqlx::query("UPDATE creator_open_indexes SET checked_at=now()-interval '121 seconds' WHERE network='base-mainnet' AND bounty_contract=$1").bind(&contract).execute(&store.pool).await.unwrap();
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM creator_open_review_sources WHERE bounty_contract=$1",
        )
        .bind(&contract)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
        // Migrations are replayed at startup and must preserve distinct reorg notices.
        store.migrate().await.unwrap();
        sqlx::query("DELETE FROM creator_push_deliveries WHERE notification_id IN(SELECT id FROM verifier_review_notifications WHERE bounty_contract=$1)").bind(&contract).execute(&store.pool).await.unwrap();
        sqlx::query("DELETE FROM verifier_review_notifications WHERE bounty_contract=$1")
            .bind(&contract)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM creator_open_indexes WHERE bounty_contract=$1")
            .bind(&contract)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM creator_open_external_creations WHERE bounty_contract=$1")
            .bind(&contract)
            .execute(&store.pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn push_devices_and_jobs_are_owned_opt_in_current_and_replay_safe() {
        use base64::Engine;
        let store = fixture().await;
        let owner = account(&store, "google").await;
        let stranger = account(&store, "google").await;
        let wallet = address();
        store
            .link_site_auth_wallet(&owner, &wallet, 8453)
            .await
            .unwrap();
        let endpoint = format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4());
        let p256dh = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode([vec![4], vec![1; 64]].concat());
        let auth = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2; 16]);
        let id = store
            .save_push_subscription(&owner, &endpoint, &p256dh, &auth)
            .await
            .unwrap();
        assert_eq!(
            store
                .save_push_subscription(&owner, &endpoint, &p256dh, &auth)
                .await
                .unwrap(),
            id
        );
        assert!(store
            .save_push_subscription(&stranger, &endpoint, &p256dh, &auth)
            .await
            .is_err());
        store.remove_push_subscription(&stranger, id).await.unwrap();
        assert_eq!(store.active_push_device_count(&owner).await.unwrap(), 1);
        let network = network();
        let mut source = job(&network, &wallet, Some(Utc::now() + Duration::hours(1)));
        source.kind = "claim".into();
        enqueue(&store, &[source.clone()]).await;
        store.enqueue_creator_push(&network).await.unwrap();
        store.enqueue_creator_push(&network).await.unwrap();
        let (a, b) = tokio::join!(
            store.claim_creator_push(&network),
            store.claim_creator_push(&network)
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a.is_some(), b.is_some());
        let delivery = a.or(b).unwrap();
        assert_eq!(delivery.subscription_id, id);
        assert_eq!(delivery.kind, "claim");
        assert!(store.creator_push_is_current(&delivery).await.unwrap());
        store.remove_push_subscription(&owner, id).await.unwrap();
        assert!(!store.creator_push_is_current(&delivery).await.unwrap());
        store
            .finish_creator_push(&delivery, false, false, false)
            .await
            .unwrap();
        assert!(store.claim_creator_push(&network).await.unwrap().is_none());
        // Resubscribing does not replay a terminated notification on this device.
        store
            .save_push_subscription(&owner, &endpoint, &p256dh, &auth)
            .await
            .unwrap();
        store.enqueue_creator_push(&network).await.unwrap();
        assert!(store.claim_creator_push(&network).await.unwrap().is_none());
        for n in 0..4 {
            store
                .save_push_subscription(&owner, &format!("{endpoint}/{n}"), &p256dh, &auth)
                .await
                .unwrap();
        }
        assert!(store
            .save_push_subscription(&owner, &format!("{endpoint}/overflow"), &p256dh, &auth)
            .await
            .is_err());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn creator_claim_and_submission_have_independent_delivery_identities() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let mut claim = job(&network, &wallet, Some(Utc::now() + Duration::hours(2)));
        claim.kind = "claim".into();
        enqueue(&store, &[claim.clone()]).await;
        let lease = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .remove(0);
        assert_eq!(lease.notification.kind, "claim");
        let mut notice = payload(&email, "review@example.test");
        notice["subject"] = json!("Work has started on your bounty");
        let attempt = store
            .begin_review_notification_attempt(&lease, &notice)
            .await
            .unwrap()
            .unwrap();
        store
            .finish_review_notification_attempt(
                attempt.id,
                attempt.lease_token,
                ReviewNotificationOutcome::Accepted {
                    provider_id: "claim-provider-fixture".into(),
                },
            )
            .await
            .unwrap();
        let mut submission = claim.clone();
        submission.kind = "submission".into();
        submission.submission_log_key = format!("review-fixture:{}", Uuid::new_v4());
        enqueue(&store, &[submission.clone()]).await;
        let second = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .remove(0);
        assert_ne!(second.id, lease.id);
        assert_eq!(second.notification.kind, "submission");
        assert!(store
            .begin_review_notification_attempt(&second, &payload(&email, "review@example.test"))
            .await
            .unwrap()
            .is_some());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM verifier_review_notifications WHERE network=$1",
        )
        .bind(&network)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 2);
        // An unconfirmed or mismatched claim source cannot authorize delivery.
        let mut absent = job(&network, &wallet, Some(Utc::now() + Duration::hours(1)));
        absent.kind = "claim".into();
        store
            .reconcile_review_notifications(&network, &[absent], &[])
            .await
            .unwrap();
        let unproven = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .remove(0);
        assert!(store
            .begin_review_notification_attempt(&unproven, &notice)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn preferences_require_fresh_contact_and_preserve_optout() {
        let store = fixture().await;
        let owner = account(&store, "google").await;
        let prefs = store.review_notification_preferences(&owner).await.unwrap();
        assert!(prefs.enabled);
        assert!(
            prefs.verified_email.is_none(),
            "old site account email is not verified evidence"
        );
        assert_eq!(prefs.verified_wallet_count, 0);
        store
            .set_review_notifications_enabled(&owner, false)
            .await
            .unwrap();
        store
            .set_verified_review_email(&owner, "google", Some("trusted@example.test"))
            .await
            .unwrap();
        let prefs = store.review_notification_preferences(&owner).await.unwrap();
        assert!(!prefs.enabled);
        assert_eq!(
            prefs.verified_email.as_deref(),
            Some("trusted@example.test")
        );
        assert!(prefs.verified_at.is_some());
        store
            .set_verified_review_email(&owner, "google", None)
            .await
            .unwrap();
        let prefs = store.review_notification_preferences(&owner).await.unwrap();
        assert!(!prefs.enabled);
        assert!(prefs.verified_email.is_none() && prefs.verified_at.is_none());
        assert!(matches!(
            store
                .set_verified_review_email(&owner, "github", Some("other@example.test"))
                .await,
            Err(ReviewNotificationError::AccountUnavailable)
        ));
        let microsoft = account(&store, "microsoft").await;
        store
            .set_verified_review_email(&microsoft, "microsoft", None)
            .await
            .unwrap();
        assert!(matches!(
            store
                .set_verified_review_email(
                    &microsoft,
                    "microsoft",
                    Some("unsupported@example.test")
                )
                .await,
            Err(ReviewNotificationError::Invalid)
        ));
        store
            .link_site_auth_wallet(&owner, &address(), 84532)
            .await
            .unwrap();
        assert_eq!(
            store
                .review_notification_preferences(&owner)
                .await
                .unwrap()
                .verified_wallet_count,
            0
        );
        store
            .link_site_auth_wallet(&owner, &address(), 8453)
            .await
            .unwrap();
        assert_eq!(
            store
                .review_notification_preferences(&owner)
                .await
                .unwrap()
                .verified_wallet_count,
            1
        );
        assert!(matches!(
            store
                .set_review_notifications_enabled(&"00".repeat(32), false)
                .await,
            Err(ReviewNotificationError::AccountUnavailable)
        ));
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn missing_contact_catches_up_with_optional_deadline_and_replays_once() {
        let store = fixture().await;
        let owner = account(&store, "github").await;
        let wallet = address();
        let network = network();
        let now = Utc::now();
        let items = vec![
            job(&network, &wallet, None),
            job(&network, &wallet, Some(now + Duration::hours(2))),
            job(&network, &wallet, Some(now - Duration::seconds(10))),
        ];
        enqueue(&store, &items).await;
        assert!(store
            .claim_review_notifications(&network, 10, 60)
            .await
            .unwrap()
            .is_empty());
        store
            .link_site_auth_wallet(&owner, &wallet, 8453)
            .await
            .unwrap();
        let sync = store
            .reconcile_review_notifications(&network, &items, &[])
            .await
            .unwrap();
        assert_eq!(sync.waiting_contact, 2);
        assert!(store
            .claim_review_notifications(&network, 10, 60)
            .await
            .unwrap()
            .is_empty());
        store
            .set_verified_review_email(&owner, "github", Some("catchup@example.test"))
            .await
            .unwrap();
        let leases = store
            .claim_review_notifications(&network, 10, 60)
            .await
            .unwrap();
        assert_eq!(leases.len(), 2);
        for lease in leases {
            let attempt = store
                .begin_review_notification_attempt(
                    &lease,
                    &payload("catchup@example.test", "review@example.test"),
                )
                .await
                .unwrap()
                .expect("confirmed source should admit");
            assert!(store
                .finish_review_notification_attempt(
                    attempt.id,
                    attempt.lease_token,
                    ReviewNotificationOutcome::Accepted {
                        provider_id: Uuid::new_v4().to_string()
                    }
                )
                .await
                .unwrap());
        }
        store.migrate().await.unwrap();
        let sync = store
            .reconcile_review_notifications(&network, &items, &[])
            .await
            .unwrap();
        assert_eq!(sync.accepted, 2);
        assert!(store
            .claim_review_notifications(&network, 100, 60)
            .await
            .unwrap()
            .is_empty());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM verifier_review_notifications WHERE network=$1",
        )
        .bind(&network)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(count, 3);
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn concurrent_leases_are_exclusive_fenced_and_freeze_provider_payload() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let items: Vec<_> = (0..6).map(|_| job(&network, &wallet, None)).collect();
        enqueue(&store, &items).await;
        let (first, second) = tokio::join!(
            store.claim_review_notifications(&network, 3, 60),
            store.claim_review_notifications(&network, 3, 60)
        );
        let mut leases = first.unwrap();
        leases.extend(second.unwrap());
        assert_eq!(leases.len(), 6);
        assert_eq!(
            leases
                .iter()
                .map(|job| job.id)
                .collect::<HashSet<_>>()
                .len(),
            6
        );
        assert_eq!(
            leases
                .iter()
                .map(|job| job.lease_token)
                .collect::<HashSet<_>>()
                .len(),
            6
        );
        assert!(store
            .claim_review_notifications(&network, 3, 60)
            .await
            .unwrap()
            .is_empty());
        let lease = leases.pop().unwrap();
        let original_payload = payload(&email, "original@example.test");
        let first = store
            .begin_review_notification_attempt(&lease, &original_payload)
            .await
            .unwrap()
            .unwrap();
        assert!(
            store
                .begin_review_notification_attempt(&lease, &original_payload)
                .await
                .unwrap()
                .is_none(),
            "one provider attempt per lease"
        );
        assert_eq!(
            first.retry_until - first.first_attempt_at,
            Duration::hours(23)
        );
        expire_lease(&store, lease.id).await;
        assert!(!store
            .finish_review_notification_attempt(
                lease.id,
                lease.lease_token,
                ReviewNotificationOutcome::Accepted {
                    provider_id: "stale-result".into()
                }
            )
            .await
            .unwrap());
        let retry = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(retry.id, first.id);
        assert_ne!(retry.lease_token, first.lease_token);
        assert_eq!(retry.provider_payload.as_ref(), Some(&original_payload));
        let second = store
            .begin_review_notification_attempt(
                &retry,
                &payload(&email, "changed-config@example.test"),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(first.provider_payload, second.provider_payload);
        assert_eq!(first.first_attempt_at, second.first_attempt_at);
        assert!(store
            .finish_review_notification_attempt(
                second.id,
                second.lease_token,
                ReviewNotificationOutcome::Accepted {
                    provider_id: "accepted-once".into()
                }
            )
            .await
            .unwrap());
        assert!(!store
            .finish_review_notification_attempt(
                second.id,
                second.lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "lost_response".into(),
                    retry_after_seconds: 30
                }
            )
            .await
            .unwrap());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn reconciliation_suspends_missing_terms_and_fences_new_lifecycle_events() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let item = job(&network, &wallet, None);
        enqueue(&store, std::slice::from_ref(&item)).await;
        let lease = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let scope = ReviewNotificationRound {
            bounty_contract: item.bounty_contract.clone(),
            round: item.round,
        };
        let sync = store
            .reconcile_review_notifications(&network, &[], &[scope])
            .await
            .unwrap();
        assert_eq!(sync.suspended, 1);
        assert!(store
            .begin_review_notification_attempt(&lease, &payload(&email, "review@example.test"))
            .await
            .unwrap()
            .is_none());
        assert!(store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .is_empty());
        store
            .reconcile_review_notifications(&network, std::slice::from_ref(&item), &[])
            .await
            .unwrap();
        let restored = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(restored.id, lease.id);
        assert_ne!(restored.lease_token, lease.lease_token);
        // Settlement arrives after the worker's earlier projection. Admission
        // must read the indexed source again rather than trust the active flag.
        sqlx::query("INSERT INTO autonomous_bounty_events (id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,block_time_verified) VALUES ($1,$2,$3,'fixture',2,0,$4,$5,'bounty_settled','{}',TRUE)")
            .bind(Uuid::new_v4()).bind(Uuid::new_v4().to_string()).bind(&network)
            .bind(&item.bounty_contract).bind(&item.bounty_id).execute(&store.pool).await.unwrap();
        assert!(store
            .begin_review_notification_attempt(&restored, &payload(&email, "review@example.test"))
            .await
            .unwrap()
            .is_none());
        store
            .reconcile_review_notifications(&network, &[], &[])
            .await
            .unwrap();
        store
            .reconcile_review_notifications(&network, std::slice::from_ref(&item), &[])
            .await
            .unwrap();
        assert!(
            store
                .claim_review_notifications(&network, 1, 60)
                .await
                .unwrap()
                .is_empty(),
            "terminal cancellation never resurrects"
        );
        let mut conflicting = item.clone();
        conflicting.submission_log_key = "changed-source".into();
        assert!(matches!(
            store
                .reconcile_review_notifications(&network, &[conflicting], &[])
                .await,
            Err(ReviewNotificationError::SourceConflict)
        ));
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn recipient_unlink_optout_and_contact_change_never_reassign_a_retry() {
        let store = fixture().await;
        let (owner, wallet, email) = linked_contact(&store).await;
        let network = network();
        let items: Vec<_> = (0..3).map(|_| job(&network, &wallet, None)).collect();
        enqueue(&store, &items).await;
        let leases = store
            .claim_review_notifications(&network, 3, 60)
            .await
            .unwrap();
        for lease in &leases {
            store
                .begin_review_notification_attempt(lease, &payload(&email, "review@example.test"))
                .await
                .unwrap()
                .unwrap();
        }
        store
            .set_review_notifications_enabled(&owner, false)
            .await
            .unwrap();
        assert!(!store
            .finish_review_notification_attempt(
                leases[0].id,
                leases[0].lease_token,
                ReviewNotificationOutcome::Accepted {
                    provider_id: "late".into()
                }
            )
            .await
            .unwrap());
        store
            .set_review_notifications_enabled(&owner, true)
            .await
            .unwrap();
        store
            .reconcile_review_notifications(&network, &items, &[])
            .await
            .unwrap();
        assert!(store
            .claim_review_notifications(&network, 3, 60)
            .await
            .unwrap()
            .is_empty());

        let transfer_network = super::tests::network();
        let item = job(&transfer_network, &wallet, None);
        enqueue(&store, std::slice::from_ref(&item)).await;
        let lease = store
            .claim_review_notifications(&transfer_network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let attempt = store
            .begin_review_notification_attempt(&lease, &payload(&email, "review@example.test"))
            .await
            .unwrap()
            .unwrap();
        store
            .finish_review_notification_attempt(
                attempt.id,
                attempt.lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "provider_timeout".into(),
                    retry_after_seconds: 30,
                },
            )
            .await
            .unwrap();
        store
            .unlink_site_auth_wallet(&owner, &wallet)
            .await
            .unwrap();
        let new_owner = account(&store, "google").await;
        store
            .set_verified_review_email(&new_owner, "google", Some("new-owner@example.test"))
            .await
            .unwrap();
        store
            .link_site_auth_wallet(&new_owner, &wallet, 8453)
            .await
            .unwrap();
        due_now(&store, lease.id).await;
        assert!(store
            .claim_review_notifications(&transfer_network, 1, 60)
            .await
            .unwrap()
            .is_empty());
        let original_recipient: String = sqlx::query_scalar(
            "SELECT recipient_account_id FROM verifier_review_notifications WHERE id=$1",
        )
        .bind(lease.id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(original_recipient, owner);

        let changed_network = super::tests::network();
        let item = job(&changed_network, &wallet, None);
        enqueue(&store, std::slice::from_ref(&item)).await;
        let lease = store
            .claim_review_notifications(&changed_network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        store
            .begin_review_notification_attempt(
                &lease,
                &payload("new-owner@example.test", "review@example.test"),
            )
            .await
            .unwrap()
            .unwrap();
        store
            .set_verified_review_email(&new_owner, "google", None)
            .await
            .unwrap();
        assert!(!store
            .finish_review_notification_attempt(
                lease.id,
                lease.lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "provider_timeout".into(),
                    retry_after_seconds: 30
                }
            )
            .await
            .unwrap());
        store
            .set_verified_review_email(&new_owner, "google", Some("replacement@example.test"))
            .await
            .unwrap();
        store
            .reconcile_review_notifications(&changed_network, &[item], &[])
            .await
            .unwrap();
        assert!(store
            .claim_review_notifications(&changed_network, 1, 60)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn provider_retry_window_and_retry_after_are_hard_bounds() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let items = vec![job(&network, &wallet, None), job(&network, &wallet, None)];
        enqueue(&store, &items).await;
        let leases = store
            .claim_review_notifications(&network, 2, 60)
            .await
            .unwrap();
        for lease in &leases {
            store
                .begin_review_notification_attempt(lease, &payload(&email, "review@example.test"))
                .await
                .unwrap()
                .unwrap();
        }
        assert!(store
            .finish_review_notification_attempt(
                leases[0].id,
                leases[0].lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "provider_rate_limit".into(),
                    retry_after_seconds: 7200
                }
            )
            .await
            .unwrap());
        let delay: f64 = sqlx::query_scalar("SELECT extract(epoch FROM next_attempt_at - now())::DOUBLE PRECISION FROM verifier_review_notifications WHERE id=$1")
            .bind(leases[0].id).fetch_one(&store.pool).await.unwrap();
        assert!(delay > 7190.0, "never shorten a provider Retry-After");
        assert!(store
            .finish_review_notification_attempt(
                leases[1].id,
                leases[1].lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "provider_rate_limit".into(),
                    retry_after_seconds: 86400
                }
            )
            .await
            .unwrap());
        let status: String =
            sqlx::query_scalar("SELECT status FROM verifier_review_notifications WHERE id=$1")
                .bind(leases[1].id)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(
            status, "failed",
            "cannot retry after provider deduplication expires"
        );
        sqlx::query("UPDATE verifier_review_notifications SET first_attempt_at=now()-interval '24 hours', retry_until=now()-interval '1 hour', next_attempt_at=now()-interval '1 second' WHERE id=$1")
            .bind(leases[0].id).execute(&store.pool).await.unwrap();
        assert!(store
            .claim_review_notifications(&network, 2, 60)
            .await
            .unwrap()
            .is_empty());
        store
            .reconcile_review_notifications(&network, &items, &[])
            .await
            .unwrap();
        assert!(store
            .claim_review_notifications(&network, 2, 60)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn admission_rejects_unconfirmed_source_wrong_round_and_extra_recipients() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let items = vec![job(&network, &wallet, None), job(&network, &wallet, None)];
        enqueue(&store, &items).await;
        sqlx::query(
            "UPDATE autonomous_bounty_events SET block_time_verified=FALSE WHERE log_key=$1",
        )
        .bind(&items[0].submission_log_key)
        .execute(&store.pool)
        .await
        .unwrap();
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{round}','2') WHERE log_key=$1")
            .bind(&items[1].submission_log_key).execute(&store.pool).await.unwrap();
        let leases = store
            .claim_review_notifications(&network, 2, 60)
            .await
            .unwrap();
        for lease in &leases {
            assert!(store
                .begin_review_notification_attempt(lease, &payload(&email, "review@example.test"))
                .await
                .unwrap()
                .is_none());
        }
        let mut extra = payload(&email, "review@example.test");
        extra["bcc"] = json!(["unrelated@example.test"]);
        assert!(matches!(
            store
                .begin_review_notification_attempt(&leases[0], &extra)
                .await,
            Err(ReviewNotificationError::Invalid)
        ));
        assert!(matches!(
            store
                .begin_review_notification_attempt(
                    &leases[0],
                    &payload("other@example.test", "review@example.test")
                )
                .await,
            Err(ReviewNotificationError::Invalid)
        ));
        assert!(store
            .claim_review_notifications("another-network", 2, 60)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn older_instance_snapshot_cannot_permanently_cancel_newer_round() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let old = job(&network, &wallet, None);
        enqueue(&store, std::slice::from_ref(&old)).await;
        // A has read round one. B observes the canonical next submission and
        // reconciles it before A finishes its older projection.
        let mut new = old.clone();
        new.round = 2;
        new.submission_log_key = Uuid::new_v4().to_string();
        canonical(&store, &new).await;
        sqlx::query("UPDATE autonomous_bounty_events SET block_number=2 WHERE log_key=$1")
            .bind(&new.submission_log_key)
            .execute(&store.pool)
            .await
            .unwrap();
        store
            .reconcile_review_notifications(&network, std::slice::from_ref(&new), &[])
            .await
            .unwrap();
        let new_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM verifier_review_notifications WHERE network=$1 AND round=2",
        )
        .bind(&network)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        let sync = store
            .reconcile_review_notifications(&network, &[old], &[])
            .await
            .unwrap();
        assert_eq!(
            sync.cancelled, 0,
            "A's absence is not canonical cancellation evidence"
        );
        let status: String =
            sqlx::query_scalar("SELECT status FROM verifier_review_notifications WHERE id=$1")
                .bind(new_id)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert_eq!(status, "pending");
        // B's next current projection resumes the same UUID/idempotency domain.
        store
            .reconcile_review_notifications(&network, &[new], &[])
            .await
            .unwrap();
        let lease = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(lease.id, new_id);
        assert!(store
            .begin_review_notification_attempt(&lease, &payload(&email, "review@example.test"))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn malformed_payload_can_fail_before_attempt_but_cannot_claim_acceptance() {
        let store = fixture().await;
        let (_, wallet, _) = linked_contact(&store).await;
        let network = network();
        let item = job(&network, &wallet, None);
        enqueue(&store, std::slice::from_ref(&item)).await;
        let lease = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert!(!store
            .finish_review_notification_attempt(
                lease.id,
                lease.lease_token,
                ReviewNotificationOutcome::Accepted {
                    provider_id: "not-attempted".into()
                }
            )
            .await
            .unwrap());
        assert!(!store
            .finish_review_notification_attempt(
                lease.id,
                lease.lease_token,
                ReviewNotificationOutcome::Retry {
                    code: "not_attempted".into(),
                    retry_after_seconds: 30
                }
            )
            .await
            .unwrap());
        assert!(store
            .finish_review_notification_attempt(
                lease.id,
                lease.lease_token,
                ReviewNotificationOutcome::Failed {
                    code: "invalid_provider_payload".into()
                }
            )
            .await
            .unwrap());
        store
            .reconcile_review_notifications(&network, &[item], &[])
            .await
            .unwrap();
        assert!(store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn claims_cap_batches_skip_locked_rows_and_require_base_wallet_proof() {
        let store = fixture().await;
        let (_, wallet, _) = linked_contact(&store).await;
        let network = network();
        let wrong_chain_owner = account(&store, "google").await;
        let wrong_chain_wallet = address();
        store
            .link_site_auth_wallet(&wrong_chain_owner, &wrong_chain_wallet, 84532)
            .await
            .unwrap();
        store
            .set_verified_review_email(
                &wrong_chain_owner,
                "google",
                Some("wrong-chain@example.test"),
            )
            .await
            .unwrap();
        let mut items: Vec<_> = (0..101).map(|_| job(&network, &wallet, None)).collect();
        items.push(job(&network, &wrong_chain_wallet, None));
        enqueue(&store, &items).await;
        let mut other_worker = store.pool.begin().await.unwrap();
        let locked_id: Uuid = sqlx::query_scalar("SELECT id FROM verifier_review_notifications WHERE network=$1 AND verifier_wallet=$2 ORDER BY id LIMIT 1 FOR UPDATE")
            .bind(&network).bind(&wallet).fetch_one(&mut *other_worker).await.unwrap();
        let leases = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            store.claim_review_notifications(&network, u32::MAX, u32::MAX),
        )
        .await
        .expect("a locked row must not block unrelated notifications")
        .unwrap();
        assert_eq!(leases.len(), 100);
        assert!(leases
            .iter()
            .all(|lease| lease.id != locked_id && lease.notification.verifier_wallet == wallet));
        let longest_lease: f64 = sqlx::query_scalar("SELECT max(extract(epoch FROM lease_expires_at-now()))::DOUBLE PRECISION FROM verifier_review_notifications WHERE network=$1")
            .bind(&network).fetch_one(&store.pool).await.unwrap();
        assert!((290.0..=300.0).contains(&longest_lease));
        other_worker.commit().await.unwrap();
        let remaining = store
            .claim_review_notifications(&network, 100, 60)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, locked_id);
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn claims_prioritize_due_reviews_without_bypassing_retry_backoff() {
        let store = fixture().await;
        let (_, wallet, _) = linked_contact(&store).await;
        let network = network();
        let now = Utc::now();
        let no_deadline = job(&network, &wallet, None);
        let later = job(&network, &wallet, Some(now + chrono::Duration::hours(2)));
        let urgent = job(&network, &wallet, Some(now + chrono::Duration::minutes(5)));
        let backing_off = job(&network, &wallet, Some(now + chrono::Duration::minutes(1)));
        enqueue(
            &store,
            &[
                no_deadline.clone(),
                later.clone(),
                urgent.clone(),
                backing_off.clone(),
            ],
        )
        .await;
        // Simulate older nonurgent work already waiting when the urgent review arrives.
        sqlx::query("UPDATE verifier_review_notifications SET next_attempt_at = now() - interval '1 hour' WHERE network=$1 AND bounty_contract=$2")
            .bind(&network).bind(&no_deadline.bounty_contract).execute(&store.pool).await.unwrap();
        sqlx::query("UPDATE verifier_review_notifications SET next_attempt_at = now() + interval '30 seconds' WHERE network=$1 AND bounty_contract=$2")
            .bind(&network).bind(&backing_off.bounty_contract).execute(&store.pool).await.unwrap();
        let leases = store
            .claim_review_notifications(&network, 10, 60)
            .await
            .unwrap();
        let contracts: Vec<_> = leases
            .iter()
            .map(|lease| lease.notification.bounty_contract.as_str())
            .collect();
        assert_eq!(
            contracts,
            vec![
                urgent.bounty_contract.as_str(),
                later.bounty_contract.as_str(),
                no_deadline.bounty_contract.as_str()
            ]
        );
    }

    #[tokio::test]
    #[ignore = "requires an explicit loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn admission_time_is_rechecked_after_waiting_for_a_job_lock() {
        let store = fixture().await;
        let (_, wallet, email) = linked_contact(&store).await;
        let network = network();
        let deadline = DateTime::from_timestamp(Utc::now().timestamp() + 2, 0).unwrap();
        let item = job(&network, &wallet, Some(deadline));
        enqueue(&store, &[item]).await;
        let lease = store
            .claim_review_notifications(&network, 1, 60)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let mut lock_holder = store.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM verifier_review_notifications WHERE id=$1 FOR UPDATE")
            .bind(lease.id)
            .fetch_one(&mut *lock_holder)
            .await
            .unwrap();
        let contender = store.clone();
        let admission = tokio::spawn(async move {
            contender
                .begin_review_notification_attempt(&lease, &payload(&email, "review@example.test"))
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(!admission.is_finished());
        tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
        lock_holder.commit().await.unwrap();
        assert!(
            admission.await.unwrap().unwrap().is_none(),
            "the review deadline passed while the transaction waited"
        );
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn broad_reminders_retry_exact_payload_skip_stale_stages_and_stop_after_completion() {
        let store = fixture().await;
        let (owner, wallet, email) = linked_contact(&store).await;
        let network = network();
        let mut item = job(
            &network,
            &wallet,
            Some(Utc::now() + chrono::Duration::minutes(45)),
        );
        item.submitted_at = Utc::now() - chrono::Duration::hours(48);
        enqueue(&store, &[item.clone()]).await;
        store.seed_review_reminders(&network).await.unwrap();
        let reminder = store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reminder.stage, "remaining_1h");
        assert_eq!(reminder.email, email);
        assert!(store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .is_none());
        let exact = payload(&email, "review@example.test");
        assert_eq!(
            store
                .begin_review_reminder(&reminder, &exact)
                .await
                .unwrap(),
            Some(exact.clone())
        );
        store
            .finish_review_reminder(&reminder, None, true)
            .await
            .unwrap();
        sqlx::query("UPDATE verifier_review_deliveries SET next_attempt_at=now()-INTERVAL '1 second' WHERE notification_id=$1").bind(reminder.id).execute(&store.pool).await.unwrap();
        let retry = store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry.payload, Some(exact.clone()));
        assert!(store
            .begin_review_reminder(&retry, &payload(&email, "changed@example.test"))
            .await
            .unwrap()
            .is_none());
        assert!(store
            .begin_review_reminder(&retry, &exact)
            .await
            .unwrap()
            .is_some());
        store
            .finish_review_reminder(&retry, Some("accepted-once"), false)
            .await
            .unwrap();
        assert!(store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .is_none());
        let agent = format!("account:{}", account(&store, "google").await);
        assert!(!store
            .assign_review_task(reminder.id, "unrelated", &agent)
            .await
            .unwrap());
        assert!(store
            .assign_review_task(reminder.id, &format!("account:{owner}"), &agent)
            .await
            .unwrap());
        assert!(!store
            .assigned_review_agent(&agent, &network, &item.bounty_contract, item.round, &wallet)
            .await
            .unwrap());
        assert!(store.accept_review_task(reminder.id, &agent).await.unwrap());
        assert!(store
            .assigned_review_agent(&agent, &network, &item.bounty_contract, item.round, &wallet)
            .await
            .unwrap());
        let tasks = store.review_tasks(&agent).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["signing_authority_granted"], false);
        assert!(!tasks[0].to_string().contains(&email));
        // Indexing settlement removes both reminders and delegated work, even with old rows present.
        sqlx::query("INSERT INTO autonomous_bounty_events (id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,block_time_verified) VALUES ($1,$2,$3,'fixture',2,0,$4,$5,'bounty_settled','{}',TRUE)").bind(Uuid::new_v4()).bind(Uuid::new_v4().to_string()).bind(&network).bind(&item.bounty_contract).bind(&item.bounty_id).execute(&store.pool).await.unwrap();
        assert!(store.review_tasks(&agent).await.unwrap().is_empty());
        assert!(!store.accept_review_task(reminder.id, &agent).await.unwrap());
        assert!(!store
            .assigned_review_agent(&agent, &network, &item.bounty_contract, item.round, &wallet)
            .await
            .unwrap());
        assert!(store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .is_none());
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn deadline_reminders_recheck_time_identity_contact_and_retry_age_at_send() {
        let store = fixture().await;
        let (owner, wallet, email) = linked_contact(&store).await;
        let network = network();
        let mut item = job(&network, &wallet, Some(Utc::now() + Duration::minutes(45)));
        item.submitted_at = Utc::now() - Duration::hours(48);
        enqueue(&store, &[item.clone()]).await;
        store.seed_review_reminders(&network).await.unwrap();
        let lease = store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .unwrap();
        let body = payload(&email, "reviews@example.test");
        store
            .set_review_notifications_enabled(&owner, false)
            .await
            .unwrap();
        assert!(store
            .begin_review_reminder(&lease, &body)
            .await
            .unwrap()
            .is_none());
        store
            .set_review_notifications_enabled(&owner, true)
            .await
            .unwrap();
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{round}','2') WHERE log_key=$1")
            .bind(&item.submission_log_key).execute(&store.pool).await.unwrap();
        assert!(store
            .begin_review_reminder(&lease, &body)
            .await
            .unwrap()
            .is_none());
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{round}','1') WHERE log_key=$1")
            .bind(&item.submission_log_key).execute(&store.pool).await.unwrap();
        assert!(store
            .begin_review_reminder(&lease, &body)
            .await
            .unwrap()
            .is_some());
        sqlx::query("UPDATE verifier_review_deliveries SET first_attempt_at=now()-interval '23 hours' WHERE notification_id=$1")
            .bind(lease.id).execute(&store.pool).await.unwrap();
        assert!(store
            .begin_review_reminder(&lease, &body)
            .await
            .unwrap()
            .is_none());
        sqlx::query(
            "UPDATE verifier_review_deliveries SET first_attempt_at=now() WHERE notification_id=$1",
        )
        .bind(lease.id)
        .execute(&store.pool)
        .await
        .unwrap();
        // The source remains unfinished, but the one-hour message must not send after expiry.
        let expired = Utc::now() - Duration::seconds(1);
        sqlx::query("UPDATE verifier_review_notifications SET review_deadline=$2 WHERE id=$1")
            .bind(lease.id)
            .bind(expired)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE autonomous_bounty_events SET data=jsonb_set(data,'{verification_expires_at}',$2) WHERE log_key=$1")
            .bind(&item.submission_log_key).bind(json!(expired.timestamp())).execute(&store.pool).await.unwrap();
        sqlx::query("UPDATE verifier_review_deliveries SET due_at=$2 WHERE notification_id=$1 AND stage='overdue'")
            .bind(lease.id).bind(expired).execute(&store.pool).await.unwrap();
        assert!(store
            .begin_review_reminder(&lease, &body)
            .await
            .unwrap()
            .is_none());
        let overdue = store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(overdue.stage, "overdue");
        assert!(store
            .begin_review_reminder(&overdue, &body)
            .await
            .unwrap()
            .is_some());
        store
            .finish_review_reminder(&overdue, Some("overdue-once"), false)
            .await
            .unwrap();
        assert!(store
            .claim_review_reminder(&network)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn deadline_push_is_independent_of_email_and_stops_on_unsubscribe_or_completion() {
        use base64::Engine;
        let store = fixture().await;
        let (owner, wallet, _) = linked_contact(&store).await;
        store
            .set_review_notifications_enabled(&owner, false)
            .await
            .unwrap();
        let network = network();
        let mut item = job(&network, &wallet, Some(Utc::now() + Duration::minutes(45)));
        item.submitted_at = Utc::now() - Duration::hours(48);
        enqueue(&store, &[item.clone()]).await;
        let device = store
            .save_push_subscription(
                &owner,
                &format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4()),
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode([vec![4], vec![1; 64]].concat()),
                &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2; 16]),
            )
            .await
            .unwrap();
        store.enqueue_review_deadline_push(&network).await.unwrap();
        store.enqueue_review_deadline_push(&network).await.unwrap();
        let push = store
            .claim_review_deadline_push_for_protocol(&network, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(push.stage, "remaining_1h");
        assert!(store.creator_push_is_current(&push).await.unwrap());
        assert!(store
            .claim_review_deadline_push_for_protocol(&network, false)
            .await
            .unwrap()
            .is_none());
        store
            .remove_push_subscription(&owner, device)
            .await
            .unwrap();
        assert!(!store.creator_push_is_current(&push).await.unwrap());
        // A late failure cannot retire a newer subscription generation.
        sqlx::query("UPDATE creator_push_subscriptions SET active=TRUE,updated_at=now()+interval '1 second' WHERE id=$1")
            .bind(device).execute(&store.pool).await.unwrap();
        store
            .finish_creator_push(&push, false, true, true)
            .await
            .unwrap();
        assert_eq!(store.active_push_device_count(&owner).await.unwrap(), 1);
        sqlx::query("UPDATE review_deadline_push_deliveries SET next_attempt_at=now()-interval '1 second' WHERE id=$1")
            .bind(push.id).execute(&store.pool).await.unwrap();
        let retry = store
            .claim_review_deadline_push_for_protocol(&network, false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry.id, push.id);
        assert!(store.creator_push_is_current(&retry).await.unwrap());
        sqlx::query("INSERT INTO autonomous_bounty_events(id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,block_time_verified) VALUES($1,$2,$3,'fixture',2,0,$4,$5,'bounty_settled','{}',TRUE)")
            .bind(Uuid::new_v4()).bind(Uuid::new_v4().to_string()).bind(&network).bind(&item.bounty_contract).bind(&item.bounty_id).execute(&store.pool).await.unwrap();
        assert!(!store.creator_push_is_current(&retry).await.unwrap());
        let snapshot = store.review_delivery_snapshot().await.unwrap();
        assert_eq!(
            snapshot["schema"],
            "agent-bounties/review-delivery-snapshot-v1"
        );
        let encoded = snapshot.to_string();
        assert!(!encoded.contains(&wallet));
        assert!(!encoded.contains(&owner));
        assert!(!encoded.contains("fcm.googleapis.com"));
    }

    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL"]
    async fn deadline_creator_open_reminders_cover_email_push_and_fence_reorgs() {
        use base64::Engine;
        let store = fixture().await;
        let (owner, wallet, email) = linked_contact(&store).await;
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../site/fixtures/creator-open-preparations.json"
        ))
        .unwrap();
        let mut request: chain_base::creator_open_service::Request =
            serde_json::from_value(vectors["plans"][0]["request"].clone()).unwrap();
        request.original_terms.network = "base-mainnet".into();
        request.original_terms.creator = wallet.clone();
        request.original_terms.bounty_id =
            format!("0x{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let contract = address();
        store
            .save_creator_open_external_creation(
                "base-mainnet",
                &contract,
                &format!("0x{}", "11".repeat(32)),
                &request,
            )
            .await
            .unwrap();
        let index = store
            .creator_open_index_batch("base-mainnet", Some(&contract))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let block = chain_base::BaseBlockIdentity {
            number: 99,
            hash: format!("0x{}", "12".repeat(32)),
            timestamp: Utc::now().timestamp() as u64,
        };
        let projection: chain_base::creator_open::Projection=serde_json::from_value(json!({
            "protocol":"agent-bounties/creator-open-v1","contract":contract,"bounty_id":request.original_terms.bounty_id,
            "creator":wallet,"terms_hash":format!("0x{}","13".repeat(32)),"revision":1,"solver_reward":20000000,"review_reward":1000000,
            "submission_deadline":block.timestamp+86400,"review_window":7200,"max_pending":8,"status":"open","principal":21000000,"outstanding_bonds":1000000,
            "entries":{"1":{"solver":address(),"submission_hash":format!("0x{}","14".repeat(32)),"evidence_hash":format!("0x{}","15".repeat(32)),"review_deadline":block.timestamp+2700,"status":"pending"}},"winner":null,"settlement_transaction":null
        })).unwrap();
        assert!(store
            .save_creator_open_index(&index, &block, &[], &projection, true)
            .await
            .unwrap());
        store.seed_review_reminders("base-mainnet").await.unwrap();
        let reminder = store
            .claim_review_reminder_for_protocol("base-mainnet", true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reminder.contract, contract);
        assert_eq!(reminder.stage, "remaining_1h");
        assert_eq!(reminder.entry, 1);
        let body = payload(&email, "reviews@example.test");
        assert!(store
            .begin_review_reminder(&reminder, &body)
            .await
            .unwrap()
            .is_some());
        store
            .save_push_subscription(
                &owner,
                &format!("https://fcm.googleapis.com/wp/{}", Uuid::new_v4()),
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode([vec![4], vec![1; 64]].concat()),
                &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([2; 16]),
            )
            .await
            .unwrap();
        store
            .enqueue_review_deadline_push("base-mainnet")
            .await
            .unwrap();
        let push = store
            .claim_review_deadline_push_for_protocol("base-mainnet", true)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(push.bounty_contract, contract);
        assert!(store.creator_push_is_current(&push).await.unwrap());
        let current = store
            .creator_open_index_batch("base-mainnet", Some(&contract))
            .await
            .unwrap()
            .pop()
            .unwrap();
        store
            .invalidate_creator_open_index(&current, true)
            .await
            .unwrap();
        assert!(store
            .begin_review_reminder(&reminder, &body)
            .await
            .unwrap()
            .is_none());
        assert!(!store.creator_push_is_current(&push).await.unwrap());
    }
}
