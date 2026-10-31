//! Per-device, opt-in browser notifications. Subscription endpoints are secrets.
use super::*;
use crate::review_deadline_source::{SOURCE, STAGES, WINDOW};
use base64::Engine;

// Fixed provider hosts keep browser-supplied subscriptions from becoming SSRF.
// No redirects are followed by the sender. Browser endpoints are private and
// must never appear in a public response, trace, or error message.
pub fn valid_push_subscription(endpoint: &str, p256dh: &str, auth: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    if endpoint.len() > 2048
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    let allowed = match host {
        "fcm.googleapis.com" => {
            url.path().starts_with("/fcm/send/") || url.path().starts_with("/wp/")
        }
        "updates.push.services.mozilla.com" => url.path().starts_with("/wpush/"),
        "web.push.apple.com" => url.path().starts_with("/"),
        _ => false,
    };
    let decode = |s: &str| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .ok()
    };
    allowed
        && (80..=100).contains(&p256dh.len())
        && (20..=30).contains(&auth.len())
        && decode(p256dh).is_some_and(|v| v.len() == 65 && v[0] == 4)
        && decode(auth).is_some_and(|v| v.len() == 16)
}

pub struct PushDelivery {
    pub id: Uuid,
    pub lease: Uuid,
    pub subscription_id: Uuid,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub bounty_contract: String,
    pub kind: String,
    pub stage: String,
    pub network: String,
    pub entry: i64,
    pub subscription_updated_at: DateTime<Utc>,
    pub deadline: Option<DateTime<Utc>>,
}

// Recheck the exact confirmed source at admission, including later lifecycle
// events. A previously accepted email is neither required nor push authority.
const CURRENT_SOURCE: &str = r#"j.submitted_at<=statement_timestamp()
 AND (j.review_deadline IS NULL OR j.review_deadline>statement_timestamp())
 AND ((j.notification_kind IN ('claim','submission') AND EXISTS(SELECT 1 FROM autonomous_bounty_events s WHERE s.network=j.network
  AND s.contract_address=j.bounty_contract AND s.bounty_id=j.bounty_id AND s.log_key=j.submission_log_key
  AND s.kind=CASE WHEN j.notification_kind='claim' THEN 'bounty_claimed' ELSE 'submission_added' END
  AND s.block_time_verified AND s.data->>'round'=j.round::TEXT AND s.occurred_at=j.submitted_at
  AND (j.review_deadline IS NULL OR s.data->>(CASE WHEN j.notification_kind='claim' THEN 'claim_expires_at' ELSE 'verification_expires_at' END)=floor(extract(epoch FROM j.review_deadline))::BIGINT::TEXT)
  AND NOT EXISTS(SELECT 1 FROM autonomous_bounty_events later WHERE later.network=s.network
   AND later.contract_address=s.contract_address AND later.bounty_id=s.bounty_id AND later.block_time_verified
   AND (later.block_number,later.log_index)>(s.block_number,s.log_index)
   AND later.kind IN ('bounty_became_claimable','bounty_claimed','submission_added','submission_rejected','bounty_settled','claim_expired','submission_expired','bounty_cancelled')))) OR (j.notification_kind='creator_open_submission' AND EXISTS(SELECT 1 FROM creator_open_review_sources s WHERE s.network=j.network AND s.bounty_contract=j.bounty_contract AND s.bounty_id=j.bounty_id AND s.round=j.round AND s.verifier_wallet=j.verifier_wallet AND s.submission_log_key=j.submission_log_key AND s.submitted_at=j.submitted_at AND s.review_deadline=j.review_deadline)))"#;

impl PostgresStore {
    pub async fn save_push_subscription(
        &self,
        account: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> DbResult<Uuid> {
        if !valid_push_subscription(endpoint, p256dh, auth) {
            return Err(DbError::InvalidPushNotification(
                "invalid push subscription".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,4047))")
            .bind(account)
            .execute(&mut *tx)
            .await?;
        let endpoint_hash = hex::encode(Sha256::digest(endpoint.as_bytes()));
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM creator_push_subscriptions WHERE account_id=$1 AND active AND expires_at>now() AND endpoint_hash<>$2").bind(account).bind(&endpoint_hash).fetch_one(&mut *tx).await?;
        if count >= 5 {
            return Err(DbError::InvalidPushNotification(
                "push device limit reached".into(),
            ));
        }
        let id=sqlx::query_scalar("INSERT INTO creator_push_subscriptions(id,account_id,endpoint_hash,endpoint,p256dh,auth) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(endpoint_hash) DO UPDATE SET p256dh=EXCLUDED.p256dh,auth=EXCLUDED.auth,active=TRUE,expires_at=now()+interval '90 days',updated_at=now() WHERE creator_push_subscriptions.account_id=EXCLUDED.account_id RETURNING id")
            .bind(Uuid::new_v4()).bind(account).bind(endpoint_hash).bind(endpoint).bind(p256dh).bind(auth).fetch_optional(&mut *tx).await?
            .ok_or_else(||DbError::InvalidPushNotification("push subscription belongs to another account".into()))?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn remove_push_subscription(&self, account: &str, id: Uuid) -> DbResult<()> {
        sqlx::query("UPDATE creator_push_subscriptions SET active=FALSE,updated_at=now() WHERE id=$1 AND account_id=$2").bind(id).bind(account).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn active_push_device_count(&self, account: &str) -> DbResult<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM creator_push_subscriptions WHERE account_id=$1 AND active AND expires_at>now()").bind(account).fetch_one(&self.pool).await?)
    }
    pub async fn enqueue_creator_push(&self, network: &str) -> DbResult<()> {
        sqlx::query(&format!(r#"INSERT INTO creator_push_deliveries(id,notification_id,subscription_id)
          SELECT gen_random_uuid(),j.id,p.id FROM verifier_review_notifications j
          JOIN site_auth_wallets w ON w.wallet_address=j.verifier_wallet AND w.chain_id=8453 AND w.proof_method='eip191_personal_sign'
          JOIN creator_push_subscriptions p ON p.account_id=w.account_key AND p.active AND p.expires_at>now()
          WHERE j.network=$1 AND {CURRENT_SOURCE} ON CONFLICT(notification_id,subscription_id) DO NOTHING"#)).bind(network).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn enqueue_review_deadline_push(&self, network: &str) -> DbResult<()> {
        sqlx::query(&format!(r#"INSERT INTO review_deadline_push_deliveries(id,notification_id,subscription_id,stage)
          SELECT gen_random_uuid(),j.id,p.id,v.stage FROM verifier_review_notifications j
          JOIN site_auth_wallets w ON w.wallet_address=j.verifier_wallet AND w.chain_id=8453 AND w.proof_method='eip191_personal_sign'
          JOIN creator_push_subscriptions p ON p.account_id=w.account_key AND p.active AND p.expires_at>statement_timestamp()
          CROSS JOIN {STAGES} v(stage,seconds)
          WHERE j.network=$1 AND j.review_deadline>statement_timestamp()-interval '24 hours'
          AND j.review_deadline-make_interval(secs=>v.seconds)>j.submitted_at
          AND j.review_deadline-make_interval(secs=>v.seconds)<=statement_timestamp()
          AND {SOURCE} ON CONFLICT(notification_id,subscription_id,stage) DO NOTHING"#))
          .bind(network).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn claim_review_deadline_push_for_protocol(
        &self,
        network: &str,
        creator_open: bool,
    ) -> DbResult<Option<PushDelivery>> {
        self.claim_push_inner(network, creator_open, true).await
    }
    pub async fn claim_creator_push(&self, network: &str) -> DbResult<Option<PushDelivery>> {
        self.claim_creator_push_for_protocol(network, false).await
    }
    pub async fn claim_creator_push_for_protocol(
        &self,
        network: &str,
        creator_open: bool,
    ) -> DbResult<Option<PushDelivery>> {
        self.claim_push_inner(network, creator_open, false).await
    }
    async fn claim_push_inner(
        &self,
        network: &str,
        creator_open: bool,
        reminder: bool,
    ) -> DbResult<Option<PushDelivery>> {
        let table = if reminder {
            "review_deadline_push_deliveries"
        } else {
            "creator_push_deliveries"
        };
        let stage = if reminder { "d.stage" } else { "'initial'" };
        let source = if reminder {
            format!("{SOURCE} AND {WINDOW}")
        } else {
            CURRENT_SOURCE.into()
        };
        let mut tx = self.pool.begin().await?;
        let row=sqlx::query(&format!(r#"SELECT {stage} AS stage,d.id,p.id AS subscription_id,p.endpoint,p.p256dh,p.auth,p.updated_at AS subscription_updated_at,j.bounty_contract,j.notification_kind,j.review_deadline,j.network,j.round
          FROM {table} d JOIN verifier_review_notifications j ON j.id=d.notification_id
          JOIN creator_push_subscriptions p ON p.id=d.subscription_id
          JOIN site_auth_wallets w ON w.wallet_address=j.verifier_wallet AND w.account_key=p.account_id AND w.chain_id=8453 AND w.proof_method='eip191_personal_sign'
          WHERE j.network=$1 AND (j.notification_kind='creator_open_submission')=$2 AND p.active AND p.expires_at>statement_timestamp() AND d.attempts<5
          AND (d.status='pending' OR (d.status='leased' AND d.lease_expires_at<=statement_timestamp())) AND d.next_attempt_at<=statement_timestamp()
          AND {source} ORDER BY j.review_deadline ASC NULLS LAST,d.id LIMIT 1 FOR UPDATE OF d SKIP LOCKED"#))
          .bind(network).bind(creator_open).fetch_optional(&mut *tx).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let lease = Uuid::new_v4();
        let id: Uuid = row.try_get("id")?;
        sqlx::query(&format!("UPDATE {table} SET status='leased',lease_token=$2,lease_expires_at=statement_timestamp()+interval '30 seconds',attempts=attempts+1,updated_at=now() WHERE id=$1")).bind(id).bind(lease).execute(&mut *tx).await?;
        let delivery = PushDelivery {
            id,
            lease,
            subscription_id: row.try_get("subscription_id")?,
            endpoint: row.try_get("endpoint")?,
            p256dh: row.try_get("p256dh")?,
            auth: row.try_get("auth")?,
            bounty_contract: row.try_get("bounty_contract")?,
            kind: row.try_get("notification_kind")?,
            stage: row.try_get("stage")?,
            network: row.try_get("network")?,
            entry: row.try_get("round")?,
            subscription_updated_at: row.try_get("subscription_updated_at")?,
            deadline: row.try_get("review_deadline")?,
        };
        tx.commit().await?;
        Ok(Some(delivery))
    }
    pub async fn creator_push_is_current(&self, delivery: &PushDelivery) -> DbResult<bool> {
        let table = delivery.table();
        let source = if delivery.stage == "initial" {
            CURRENT_SOURCE.into()
        } else {
            format!("{SOURCE} AND {WINDOW}")
        };
        Ok(sqlx::query_scalar(&format!(r#"SELECT EXISTS(SELECT 1 FROM {table} d
          JOIN verifier_review_notifications j ON j.id=d.notification_id JOIN creator_push_subscriptions p ON p.id=d.subscription_id
          JOIN site_auth_wallets w ON w.account_key=p.account_id AND w.wallet_address=j.verifier_wallet AND w.chain_id=8453 AND w.proof_method='eip191_personal_sign'
          WHERE d.id=$1 AND d.lease_token=$2 AND d.status='leased' AND d.lease_expires_at>statement_timestamp()+interval '10 seconds'
          AND p.active AND p.updated_at=$3 AND p.expires_at>statement_timestamp() AND {source})"#)).bind(delivery.id).bind(delivery.lease).bind(delivery.subscription_updated_at).fetch_one(&self.pool).await?)
    }
    pub async fn finish_creator_push(
        &self,
        delivery: &PushDelivery,
        accepted: bool,
        retry: bool,
        gone: bool,
    ) -> DbResult<()> {
        let mut tx = self.pool.begin().await?;
        let table = delivery.table();
        let updated=sqlx::query(&format!("UPDATE {table} SET status=CASE WHEN $3 THEN 'accepted' WHEN $4 AND attempts<5 THEN 'pending' ELSE 'failed' END,lease_token=NULL,lease_expires_at=NULL,next_attempt_at=now()+interval '60 seconds',updated_at=now() WHERE id=$1 AND lease_token=$2 AND status='leased'"))
            .bind(delivery.id).bind(delivery.lease).bind(accepted).bind(retry).execute(&mut *tx).await?;
        if gone && updated.rows_affected() == 1 {
            sqlx::query(
                "UPDATE creator_push_subscriptions SET active=FALSE,updated_at=now() WHERE id=$1 AND updated_at=$2",
            )
            .bind(delivery.subscription_id)
            .bind(delivery.subscription_updated_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

impl PushDelivery {
    fn table(&self) -> &'static str {
        if self.stage == "initial" {
            "creator_push_deliveries"
        } else {
            "review_deadline_push_deliveries"
        }
    }
}
