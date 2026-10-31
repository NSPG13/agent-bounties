//! Private issue snapshots, bound to the webhook's numeric GitHub actor identity.
//! A snapshot is neither a public posting nor an authorization to spend.
use super::*;
impl PostgresStore {
    pub async fn set_github_origin_installation(
        &self,
        installation: i64,
        disabled: bool,
        retired: bool,
        updated_at: DateTime<Utc>,
    ) -> DbResult<()> {
        sqlx::query("INSERT INTO github_origin_installations(installation_id,disabled,retired,source_updated_at) VALUES($1,$2,$3,$4) ON CONFLICT(installation_id) DO UPDATE SET disabled=EXCLUDED.disabled OR github_origin_installations.retired OR EXCLUDED.retired,retired=github_origin_installations.retired OR EXCLUDED.retired,source_updated_at=GREATEST(github_origin_installations.source_updated_at,EXCLUDED.source_updated_at),updated_at=now() WHERE EXCLUDED.retired OR github_origin_installations.source_updated_at<EXCLUDED.source_updated_at OR (EXCLUDED.disabled AND github_origin_installations.source_updated_at=EXCLUDED.source_updated_at)")
            .bind(installation).bind(disabled).bind(retired).bind(updated_at).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn prune_github_origin_drafts(&self) -> DbResult<()> {
        sqlx::query("UPDATE github_origin_drafts SET plan='{}'::JSONB WHERE expires_at<=now() AND plan<>'{}'::JSONB").execute(&self.pool).await?;
        Ok(())
    }
    pub async fn save_github_origin_draft(
        &self,
        installation: i64,
        requester: &str,
        delivery: &str,
        event_key: &str,
        body_hash: &str,
        plan: &serde_json::Value,
    ) -> DbResult<Uuid> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,4053))")
            .bind(format!("github-installation:{installation}"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO github_origin_installations(installation_id) VALUES($1) ON CONFLICT DO NOTHING").bind(installation).execute(&mut *tx).await?;
        let disabled: bool = sqlx::query_scalar(
            "SELECT disabled OR retired FROM github_origin_installations WHERE installation_id=$1",
        )
        .bind(installation)
        .fetch_one(&mut *tx)
        .await?;
        if disabled {
            return Err(DbError::X402RelayQuotaExceeded(
                "github_installation_disabled".into(),
            ));
        }
        if let Some(row)=sqlx::query("SELECT id,body_sha256,event_key,requester_github_id FROM github_origin_drafts WHERE delivery_id=$1 OR event_key=$2 ORDER BY received_at LIMIT 1").bind(delivery).bind(event_key).fetch_optional(&mut *tx).await? {
            if row.try_get::<String,_>("event_key")?!=event_key || row.try_get::<String,_>("requester_github_id")?!=requester || row.try_get::<String,_>("body_sha256")?!=body_hash {
                return Err(DbError::X402RelayConflict("github_delivery_conflict".into()));
            }
            return Ok(row.try_get("id")?);
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,4053))")
            .bind(format!("github-requester:{requester}"))
            .execute(&mut *tx)
            .await?;
        let count:i64=sqlx::query_scalar("SELECT count(*) FROM github_origin_drafts WHERE installation_id=$1 AND received_at>now()-interval '1 day'").bind(installation).fetch_one(&mut *tx).await?;
        let personal:i64=sqlx::query_scalar("SELECT count(*) FROM github_origin_drafts WHERE requester_github_id=$1 AND received_at>now()-interval '1 day'").bind(requester).fetch_one(&mut *tx).await?;
        if count >= 100 || personal >= 20 {
            return Err(DbError::X402RelayQuotaExceeded("github_draft_limit".into()));
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO github_origin_drafts(id,installation_id,requester_github_id,delivery_id,event_key,body_sha256,plan) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(id).bind(installation).bind(requester).bind(delivery).bind(event_key).bind(body_hash).bind(plan).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    pub async fn github_origin_drafts(
        &self,
        requester: &str,
        id: Option<Uuid>,
    ) -> DbResult<Vec<serde_json::Value>> {
        let rows=sqlx::query("SELECT d.id,d.plan,d.received_at,d.expires_at FROM github_origin_drafts d JOIN github_origin_installations i USING(installation_id) WHERE d.requester_github_id=$1 AND ($2::UUID IS NULL OR d.id=$2) AND NOT i.disabled AND NOT i.retired AND d.expires_at>now() ORDER BY d.received_at DESC LIMIT 20")
            .bind(requester).bind(id).fetch_all(&self.pool).await?;
        rows.into_iter().map(|r|Ok(serde_json::json!({"id":r.try_get::<Uuid,_>("id")?,"plan":r.try_get::<serde_json::Value,_>("plan")?,"received_at":r.try_get::<DateTime<Utc>,_>("received_at")?,"expires_at":r.try_get::<DateTime<Utc>,_>("expires_at")?}))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL named *test*"]
    async fn quotas_are_shared_across_installations_and_expired_content_is_purged() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.path().contains("test"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let installation = (Uuid::new_v4().as_u128() % 1_000_000_000 + 2_000_000_000) as i64;
        let requester = format!(
            "{}",
            Uuid::new_v4().as_u128() % 1_000_000_000 + 3_000_000_000
        );
        let plan =
            serde_json::json!({"draft":{"title":"private-fixture"},"funding_authority":false});
        let hash = "11".repeat(32);
        for i in 0..19 {
            store
                .save_github_origin_draft(
                    installation,
                    &requester,
                    &Uuid::new_v4().to_string(),
                    &format!("{requester}:{i}"),
                    &hash,
                    &plan,
                )
                .await
                .unwrap();
        }
        let a = Uuid::new_v4().to_string();
        let b = Uuid::new_v4().to_string();
        let (a, b) = tokio::join!(
            store.save_github_origin_draft(installation, &requester, &a, &a, &hash, &plan),
            store.save_github_origin_draft(installation + 1, &requester, &b, &b, &hash, &plan)
        );
        assert_ne!(
            a.is_ok(),
            b.is_ok(),
            "concurrent installations share one requester quota"
        );
        assert_eq!(
            store
                .github_origin_drafts(&requester, None)
                .await
                .unwrap()
                .len(),
            20
        );
        sqlx::query("UPDATE github_origin_drafts SET expires_at=now()-interval '1 second' WHERE requester_github_id=$1").bind(&requester).execute(&store.pool).await.unwrap();
        store.prune_github_origin_drafts().await.unwrap();
        assert!(store
            .github_origin_drafts(&requester, None)
            .await
            .unwrap()
            .is_empty());
        let remaining:i64=sqlx::query_scalar("SELECT count(*) FROM github_origin_drafts WHERE requester_github_id=$1 AND plan<>'{}'::JSONB").bind(&requester).fetch_one(&store.pool).await.unwrap();
        assert_eq!(remaining, 0);
        sqlx::query("DELETE FROM github_origin_drafts WHERE requester_github_id=$1")
            .bind(&requester)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM github_origin_installations WHERE installation_id IN($1,$2)")
            .bind(installation)
            .bind(installation + 1)
            .execute(&store.pool)
            .await
            .unwrap();
    }
}
