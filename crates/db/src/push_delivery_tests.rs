//! Explicit, account-owned delivery checks. Never a bounty or verdict record.
use super::*;

// No Debug/Serialize: provider endpoints and encryption material are private.
pub struct PushTestDelivery {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub subscription_updated_at: DateTime<Utc>,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

pub enum PushTestAdmission {
    Send(PushTestDelivery),
    Recorded(String),
    Unavailable,
    RateLimited,
}

impl PostgresStore {
    /// Reserve before contacting the provider. Replays, including an ambiguous
    /// interrupted send, never send again. Quotas survive restarts and renewal.
    pub async fn reserve_push_delivery_test(
        &self,
        account: &str,
        subscription: Uuid,
        id: Uuid,
    ) -> DbResult<PushTestAdmission> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,4047))")
            .bind(account)
            .execute(&mut *tx)
            .await?;
        let device = sqlx::query("SELECT updated_at,endpoint,p256dh,auth FROM creator_push_subscriptions WHERE id=$1 AND account_id=$2 AND active AND expires_at>statement_timestamp()")
            .bind(subscription).bind(account).fetch_optional(&mut *tx).await?;
        let Some(device) = device else {
            return Ok(PushTestAdmission::Unavailable);
        };
        let prior = sqlx::query(
            "SELECT account_id,subscription_id,status FROM push_delivery_tests WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(prior) = prior {
            return Ok(
                if prior.try_get::<String, _>("account_id")? == account
                    && prior.try_get::<Uuid, _>("subscription_id")? == subscription
                {
                    PushTestAdmission::Recorded(prior.try_get("status")?)
                } else {
                    PushTestAdmission::Unavailable
                },
            );
        }
        let limited: bool = sqlx::query_scalar("SELECT count(*)>=5 OR count(*) FILTER(WHERE created_at>statement_timestamp()-interval '1 minute')>0 FROM push_delivery_tests WHERE account_id=$1 AND created_at>statement_timestamp()-interval '1 hour'")
            .bind(account).fetch_one(&mut *tx).await?;
        if limited {
            return Ok(PushTestAdmission::RateLimited);
        }
        let generation: DateTime<Utc> = device.try_get("updated_at")?;
        // A UUID collision across accounts must not produce an extra delivery.
        let inserted = sqlx::query("INSERT INTO push_delivery_tests(id,account_id,subscription_id,subscription_updated_at,status) VALUES($1,$2,$3,$4,'attempting') ON CONFLICT(id) DO NOTHING")
            .bind(id).bind(account).bind(subscription).bind(generation).execute(&mut *tx).await?;
        if inserted.rows_affected() != 1 {
            return Ok(PushTestAdmission::Unavailable);
        }
        let delivery = PushTestDelivery {
            id,
            subscription_id: subscription,
            subscription_updated_at: generation,
            endpoint: device.try_get("endpoint")?,
            p256dh: device.try_get("p256dh")?,
            auth: device.try_get("auth")?,
        };
        tx.commit().await?;
        Ok(PushTestAdmission::Send(delivery))
    }

    pub async fn push_delivery_test_is_current(&self, d: &PushTestDelivery) -> DbResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM push_delivery_tests t JOIN creator_push_subscriptions p ON p.id=t.subscription_id AND p.account_id=t.account_id WHERE t.id=$1 AND t.subscription_id=$2 AND t.subscription_updated_at=$3 AND p.updated_at=$3 AND p.active AND p.expires_at>statement_timestamp() AND t.status='attempting' AND t.created_at>statement_timestamp()-interval '20 seconds')")
            .bind(d.id).bind(d.subscription_id).bind(d.subscription_updated_at).fetch_one(&self.pool).await?)
    }

    pub async fn finish_push_delivery_test(
        &self,
        d: &PushTestDelivery,
        status: &str,
        gone: bool,
    ) -> DbResult<()> {
        if !matches!(status, "accepted" | "failed" | "unknown" | "cancelled") {
            return Err(DbError::InvalidPushNotification(
                "invalid delivery test outcome".into(),
            ));
        }
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE push_delivery_tests SET status=$4,updated_at=now() WHERE id=$1 AND subscription_id=$2 AND subscription_updated_at=$3 AND status='attempting'")
            .bind(d.id).bind(d.subscription_id).bind(d.subscription_updated_at).bind(status).execute(&mut *tx).await?;
        if gone && changed.rows_affected() == 1 {
            sqlx::query("UPDATE creator_push_subscriptions SET active=FALSE,updated_at=now() WHERE id=$1 AND updated_at=$2")
                .bind(d.subscription_id).bind(d.subscription_updated_at).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
