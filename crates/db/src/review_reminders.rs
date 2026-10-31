//! Bounded reminders use verified contacts and exact unfinished submissions.
use crate::{
    review_deadline_source::{SOURCE, STAGES, WINDOW},
    PostgresStore,
};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

pub struct Reminder {
    pub id: Uuid,
    pub stage: String,
    pub lease: Uuid,
    pub email: String,
    pub contract: String,
    pub kind: String,
    pub entry: i64,
    pub deadline: DateTime<Utc>,
    pub payload: Option<Value>,
}
const LIVE:&str="EXISTS(SELECT 1 FROM autonomous_bounty_events source WHERE source.log_key=j.submission_log_key AND source.network=j.network AND source.contract_address=j.bounty_contract AND source.bounty_id=j.bounty_id AND source.kind='submission_added' AND source.block_time_verified AND NOT EXISTS(SELECT 1 FROM autonomous_bounty_events later WHERE later.network=source.network AND later.contract_address=source.contract_address AND later.bounty_id=source.bounty_id AND later.block_time_verified AND (later.block_number,later.log_index)>(source.block_number,source.log_index) AND later.kind IN ('bounty_became_claimable','bounty_claimed','submission_added','submission_rejected','bounty_settled','claim_expired','submission_expired','bounty_cancelled')))";
impl PostgresStore {
    /// Operator-shell diagnostic only. Aggregate data, no recipient or endpoint
    /// details, and a read-only transaction: never starts workers or seeds jobs.
    pub async fn review_delivery_snapshot(&self) -> Result<Value, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await?;
        let result = sqlx::query_scalar(&format!(r#"SELECT jsonb_build_object(
          'schema','agent-bounties/review-delivery-snapshot-v1',
          'observed_at',statement_timestamp(),
          'enabled_verified_contacts',(SELECT count(*) FROM verifier_review_contacts WHERE enabled AND verified_email IS NOT NULL),
          'active_push_devices',(SELECT count(*) FROM creator_push_subscriptions WHERE active AND expires_at>statement_timestamp()),
          'stored_notices',(SELECT count(*) FROM verifier_review_notifications WHERE network='base-mainnet'),
          'current_stored_reviews',(SELECT count(*) FROM verifier_review_notifications j WHERE j.network='base-mainnet' AND j.review_deadline>statement_timestamp()-interval '24 hours' AND {SOURCE}),
          'email_initial_provider_accepted',(SELECT count(*) FROM verifier_review_notifications WHERE network='base-mainnet' AND status='accepted'),
          'email_reminder_provider_accepted',(SELECT count(*) FROM verifier_review_deliveries d JOIN verifier_review_notifications j ON j.id=d.notification_id WHERE j.network='base-mainnet' AND d.stage<>'initial' AND d.accepted_at IS NOT NULL),
          'email_reminder_attempts_exhausted',(SELECT count(*) FROM verifier_review_deliveries d JOIN verifier_review_notifications j ON j.id=d.notification_id WHERE j.network='base-mainnet' AND d.stage<>'initial' AND d.attempts>=20 AND d.accepted_at IS NULL),
          'push_initial_provider_accepted',(SELECT count(*) FROM creator_push_deliveries WHERE status='accepted'),
          'push_reminder_provider_accepted',(SELECT count(*) FROM review_deadline_push_deliveries WHERE status='accepted'),
          'push_reminder_failed',(SELECT count(*) FROM review_deadline_push_deliveries WHERE status='failed'),
          'boundary','Stored notices only; no backfill, provider request, inbox receipt or verdict evidence.')"#))
          .fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn seed_review_reminders(&self, network: &str) -> Result<(), sqlx::Error> {
        let query = format!(
            r#"INSERT INTO verifier_review_deliveries(notification_id,stage,due_at)
          SELECT j.id,v.stage,j.review_deadline-make_interval(secs=>v.seconds)
          FROM verifier_review_notifications j CROSS JOIN {STAGES} v(stage,seconds)
          WHERE j.network=$1 AND j.review_deadline>statement_timestamp()-interval '24 hours'
          AND j.review_deadline-make_interval(secs=>v.seconds)>j.submitted_at
          AND {SOURCE} ON CONFLICT DO NOTHING"#
        );
        sqlx::query(&query)
            .bind(network)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn claim_review_reminder(
        &self,
        network: &str,
    ) -> Result<Option<Reminder>, sqlx::Error> {
        self.claim_review_reminder_for_protocol(network, false)
            .await
    }
    pub async fn claim_review_reminder_for_protocol(
        &self,
        network: &str,
        creator_open: bool,
    ) -> Result<Option<Reminder>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let query = format!(
            r#"SELECT d.*,j.bounty_contract,j.notification_kind,j.round,j.review_deadline,c.verified_email
          FROM verifier_review_deliveries d JOIN verifier_review_notifications j ON j.id=d.notification_id
          JOIN site_auth_wallets w ON w.wallet_address=j.verifier_wallet
            AND w.chain_id=CASE WHEN j.network='base-sepolia' THEN 84532 ELSE 8453 END
            AND w.proof_method='eip191_personal_sign'
          JOIN verifier_review_contacts c ON c.account_id=w.account_key AND c.enabled AND c.verified_email IS NOT NULL
          WHERE j.network=$1 AND (j.notification_kind='creator_open_submission')=$2
          AND d.accepted_at IS NULL AND d.attempts<20 AND d.due_at<=statement_timestamp()
          AND d.next_attempt_at<=statement_timestamp()
          AND (d.lease_expires_at IS NULL OR d.lease_expires_at<=statement_timestamp())
          AND (d.first_attempt_at IS NULL OR d.first_attempt_at>statement_timestamp()-interval '23 hours')
          AND (d.recipient_email IS NULL OR d.recipient_email=c.verified_email)
          AND {WINDOW} AND {SOURCE}
          ORDER BY j.review_deadline,d.due_at,d.notification_id LIMIT 1 FOR UPDATE OF d SKIP LOCKED"#
        );
        let Some(row) = sqlx::query(&query)
            .bind(network)
            .bind(creator_open)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(None);
        };
        let id = row.try_get("notification_id")?;
        let stage: String = row.try_get("stage")?;
        let lease = Uuid::new_v4();
        sqlx::query("UPDATE verifier_review_deliveries SET lease_token=$3,lease_expires_at=statement_timestamp()+interval '60 seconds' WHERE notification_id=$1 AND stage=$2")
          .bind(id).bind(&stage).bind(lease).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(Reminder {
            id,
            stage,
            lease,
            email: row.try_get("verified_email")?,
            contract: row.try_get("bounty_contract")?,
            kind: row.try_get("notification_kind")?,
            entry: row.try_get("round")?,
            deadline: row.try_get("review_deadline")?,
            payload: row.try_get("provider_payload")?,
        }))
    }
    pub async fn begin_review_reminder(
        &self,
        r: &Reminder,
        payload: &Value,
    ) -> Result<Option<Value>, sqlx::Error> {
        let query = format!(
            r#"UPDATE verifier_review_deliveries d
          SET provider_payload=COALESCE(d.provider_payload,$4),recipient_email=COALESCE(d.recipient_email,$5),
            first_attempt_at=COALESCE(d.first_attempt_at,statement_timestamp()),attempts=d.attempts+1
          FROM verifier_review_notifications j WHERE j.id=d.notification_id
          AND d.notification_id=$1 AND d.stage=$2 AND d.lease_token=$3
          AND d.lease_expires_at>statement_timestamp()+interval '10 seconds'
          AND d.accepted_at IS NULL AND d.attempts<20 AND d.next_attempt_at<=statement_timestamp()
          AND (d.first_attempt_at IS NULL OR d.first_attempt_at>statement_timestamp()-interval '23 hours')
          AND (d.recipient_email IS NULL OR d.recipient_email=$5)
          AND (d.provider_payload IS NULL OR d.provider_payload=$4)
          AND EXISTS(SELECT 1 FROM site_auth_wallets w JOIN verifier_review_contacts c ON c.account_id=w.account_key
            WHERE w.wallet_address=j.verifier_wallet
            AND w.chain_id=CASE WHEN j.network='base-sepolia' THEN 84532 ELSE 8453 END
            AND w.proof_method='eip191_personal_sign' AND c.enabled AND c.verified_email=$5)
          AND {WINDOW} AND {SOURCE} RETURNING d.provider_payload"#
        );
        sqlx::query_scalar(&query)
            .bind(r.id)
            .bind(&r.stage)
            .bind(r.lease)
            .bind(payload)
            .bind(&r.email)
            .fetch_optional(&self.pool)
            .await
    }
    pub async fn finish_review_reminder(
        &self,
        r: &Reminder,
        provider_id: Option<&str>,
        retry: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE verifier_review_deliveries SET accepted_at=CASE WHEN $4::text IS NOT NULL THEN now() ELSE NULL END,provider_id=$4,attempts=CASE WHEN $4 IS NULL AND NOT $5 THEN 20 ELSE attempts END,lease_token=NULL,lease_expires_at=NULL,next_attempt_at=now()+interval '5 minutes' WHERE notification_id=$1 AND stage=$2 AND lease_token=$3")
            .bind(r.id).bind(&r.stage).bind(r.lease).bind(provider_id).bind(retry).execute(&self.pool).await?;
        Ok(())
    }
}

impl PostgresStore {
    pub async fn review_tasks(&self, principal: &str) -> Result<Vec<Value>, sqlx::Error> {
        let query=format!("SELECT jsonb_build_object('id',j.id,'network',j.network,'bounty_contract',j.bounty_contract,'round',j.round,'reviewer_wallet',j.verifier_wallet,'review_deadline',j.review_deadline,'assigned_to',a.assignee_key,'agent_ready',a.accepted_at IS NOT NULL,'overdue',j.review_deadline<=now(),'signing_authority_granted',false) FROM verifier_review_notifications j LEFT JOIN verification_review_assignments a ON a.notification_id=j.id WHERE (a.assignee_key=$1 OR 'wallet:'||CASE WHEN j.network='base-sepolia' THEN '84532' ELSE '8453' END||':'||j.verifier_wallet=$1 OR EXISTS(SELECT 1 FROM site_auth_wallets w WHERE 'account:'||w.account_key=$1 AND w.wallet_address=j.verifier_wallet AND w.chain_id=CASE WHEN j.network='base-sepolia' THEN 84532 ELSE 8453 END AND w.proof_method='eip191_personal_sign')) AND {LIVE} ORDER BY j.review_deadline NULLS LAST LIMIT 100");
        sqlx::query_scalar(&query)
            .bind(principal)
            .fetch_all(&self.pool)
            .await
    }
    pub async fn assign_review_task(
        &self,
        id: Uuid,
        owner: &str,
        assignee: &str,
    ) -> Result<bool, sqlx::Error> {
        // Authorization is derived here as well as in the API; an arbitrary task
        // ID or agent name cannot grant authority over someone else's review.
        let query=format!("INSERT INTO verification_review_assignments(notification_id,assigned_by,assignee_key) SELECT j.id,$2,$3 FROM verifier_review_notifications j WHERE j.id=$1 AND ('wallet:'||CASE WHEN j.network='base-sepolia' THEN '84532' ELSE '8453' END||':'||j.verifier_wallet=$2 OR EXISTS(SELECT 1 FROM site_auth_wallets w WHERE 'account:'||w.account_key=$2 AND w.wallet_address=j.verifier_wallet AND w.chain_id=CASE WHEN j.network='base-sepolia' THEN 84532 ELSE 8453 END AND w.proof_method='eip191_personal_sign')) AND {LIVE} ON CONFLICT(notification_id) DO UPDATE SET assignee_key=EXCLUDED.assignee_key,assigned_by=EXCLUDED.assigned_by,accepted_at=CASE WHEN verification_review_assignments.assignee_key=EXCLUDED.assignee_key THEN verification_review_assignments.accepted_at ELSE NULL END");
        Ok(sqlx::query(&query)
            .bind(id)
            .bind(owner)
            .bind(assignee)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }
    pub async fn accept_review_task(&self, id: Uuid, principal: &str) -> Result<bool, sqlx::Error> {
        let query=format!("UPDATE verification_review_assignments a SET accepted_at=COALESCE(a.accepted_at,now()) FROM verifier_review_notifications j WHERE j.id=a.notification_id AND a.notification_id=$1 AND a.assignee_key=$2 AND {LIVE}");
        Ok(sqlx::query(&query)
            .bind(id)
            .bind(principal)
            .execute(&self.pool)
            .await?
            .rows_affected()
            == 1)
    }
    pub async fn assigned_review_agent(
        &self,
        principal: &str,
        network: &str,
        contract: &str,
        round: i64,
        reviewer: &str,
    ) -> Result<bool, sqlx::Error> {
        let query=format!("SELECT EXISTS(SELECT 1 FROM verification_review_assignments a JOIN verifier_review_notifications j ON j.id=a.notification_id WHERE a.assignee_key=$1 AND a.accepted_at IS NOT NULL AND j.network=$2 AND j.bounty_contract=$3 AND j.round=$4 AND j.verifier_wallet=$5 AND {LIVE})");
        sqlx::query_scalar(&query)
            .bind(principal)
            .bind(network)
            .bind(contract.to_lowercase())
            .bind(round)
            .bind(reviewer.to_lowercase())
            .fetch_one(&self.pool)
            .await
    }
}
