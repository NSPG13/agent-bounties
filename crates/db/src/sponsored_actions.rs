use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct SponsoredAction {
    pub id: Uuid,
    pub status: String,
    pub attempts: i32,
    pub transaction_hash: Option<String>,
    pub error_message: Option<String>,
}
fn action(row: PgRow) -> DbResult<SponsoredAction> {
    Ok(SponsoredAction {
        id: row.try_get("id")?,
        status: row.try_get("status")?,
        attempts: row.try_get("attempts")?,
        transaction_hash: row.try_get("transaction_hash")?,
        error_message: row.try_get("error_message")?,
    })
}
impl PostgresStore {
    /// Read-only guidance before requesting a signature. The atomic reservation
    /// below remains authoritative if another request uses the last slot.
    pub async fn check_sponsored_action_capacity(
        &self,
        network: &str,
        request_hash: &str,
        wallet: &str,
        max_network: u32,
        max_wallet: u32,
    ) -> DbResult<()> {
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sponsored_actions WHERE network=$1 AND request_hash=$2)",
        )
        .bind(network)
        .bind(request_hash)
        .fetch_one(&self.pool)
        .await?;
        if existing {
            return Ok(());
        }
        let row = sqlx::query("SELECT COUNT(*) AS total, COUNT(*) FILTER (WHERE wallet=$2) AS wallet_total FROM sponsored_actions WHERE network=$1 AND created_at > now() - interval '24 hours'")
            .bind(network).bind(wallet.to_ascii_lowercase()).fetch_one(&self.pool).await?;
        if row.try_get::<i64, _>("total")? >= i64::from(max_network)
            || row.try_get::<i64, _>("wallet_total")? >= i64::from(max_wallet)
        {
            return Err(DbError::X402RelayQuotaExceeded(
                "signed action quota reached".into(),
            ));
        }
        Ok(())
    }

    pub async fn find_sponsored_action(
        &self,
        network: &str,
        request_hash: &str,
    ) -> DbResult<Option<SponsoredAction>> {
        let found =
            sqlx::query("SELECT * FROM sponsored_actions WHERE network=$1 AND request_hash=$2")
                .bind(network)
                .bind(request_hash)
                .fetch_optional(&self.pool)
                .await?
                .map(action)
                .transpose()?;
        if let Some(item) = found {
            if item.transaction_hash.is_none() {
                self.repair_sponsored_outbox_marker(item.id).await?;
            }
            return self.sponsored_action(item.id).await.map(Some);
        }
        Ok(None)
    }
    /// Call only after verifying the action's exact wallet signature. Hash the
    /// semantic typed data, excluding its alternative ECDSA encodings.
    pub async fn reserve_sponsored_action(
        &self,
        network: &str,
        request_hash: &str,
        wallet: &str,
        max_network: u32,
        max_wallet: u32,
    ) -> DbResult<SponsoredAction> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(format!("sponsored-actions:{network}"))
            .execute(&mut *tx)
            .await?;
        if let Some(row) =
            sqlx::query("SELECT * FROM sponsored_actions WHERE network=$1 AND request_hash=$2")
                .bind(network)
                .bind(request_hash)
                .fetch_optional(&mut *tx)
                .await?
        {
            return action(row);
        }
        let wallet = wallet.to_ascii_lowercase();
        let row = sqlx::query("SELECT COUNT(*) AS total, COUNT(*) FILTER (WHERE wallet=$2) AS wallet_total FROM sponsored_actions WHERE network=$1 AND created_at > now() - interval '24 hours'")
            .bind(network).bind(&wallet).fetch_one(&mut *tx).await?;
        if row.try_get::<i64, _>("total")? >= i64::from(max_network)
            || row.try_get::<i64, _>("wallet_total")? >= i64::from(max_wallet)
        {
            return Err(DbError::X402RelayQuotaExceeded(
                "signed action quota reached".into(),
            ));
        }
        let row = sqlx::query("INSERT INTO sponsored_actions(id,network,request_hash,wallet) VALUES($1,$2,$3,$4) RETURNING *").bind(Uuid::new_v4()).bind(network).bind(request_hash).bind(wallet).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        action(row)
    }
    pub async fn claim_sponsored_action(
        &self,
        id: Uuid,
        lease: Uuid,
        seconds: u64,
    ) -> DbResult<bool> {
        let result = sqlx::query("UPDATE sponsored_actions SET status='relaying', attempts=attempts+1, lease_token=$2, lease_expires_at=now()+($3::bigint * interval '1 second'), updated_at=now() WHERE id=$1 AND transaction_hash IS NULL AND attempts < 3 AND (status IN ('prepared','failed') OR (status='relaying' AND lease_expires_at <= now()))")
            .bind(id).bind(lease).bind(i64_from_u64(seconds)?).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
    pub async fn sponsored_action(&self, id: Uuid) -> DbResult<SponsoredAction> {
        action(
            sqlx::query("SELECT * FROM sponsored_actions WHERE id=$1")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn finish_sponsored_action(
        &self,
        id: Uuid,
        lease: Option<Uuid>,
        status: &str,
        hash: Option<&str>,
        error: Option<&str>,
    ) -> DbResult<SponsoredAction> {
        sqlx::query("UPDATE sponsored_actions SET status=$3, transaction_hash=COALESCE($4,transaction_hash), error_message=$5, lease_token=NULL, lease_expires_at=NULL, updated_at=now() WHERE id=$1 AND (($2::uuid IS NOT NULL AND lease_token=$2 AND status='relaying') OR ($2 IS NULL AND status='broadcast' AND $3 IN ('confirmed','failed')))")
            .bind(id).bind(lease).bind(status).bind(hash).bind(error).execute(&self.pool).await?;
        self.sponsored_action(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn action_replay_claims_once_and_never_rebroadcasts_a_known_hash() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let network = format!("test-actions-{}", Uuid::new_v4());
        store
            .check_sponsored_action_capacity(&network, "same", "wallet", 1, 1)
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            store.reserve_sponsored_action(&network, "same", "wallet", 1, 1),
            store.reserve_sponsored_action(&network, "same", "wallet", 1, 1)
        );
        let a = a.unwrap();
        assert_eq!(a.id, b.unwrap().id);
        // A read cannot consume capacity, and an existing request remains
        // reconcilable while new network or wallet actions are rate-limited.
        store
            .check_sponsored_action_capacity(&network, "same", "wallet", 1, 1)
            .await
            .unwrap();
        assert!(matches!(
            store
                .check_sponsored_action_capacity(&network, "new", "wallet", 2, 1)
                .await,
            Err(DbError::X402RelayQuotaExceeded(_))
        ));
        assert!(matches!(
            store
                .check_sponsored_action_capacity(&network, "new", "another-wallet", 1, 2)
                .await,
            Err(DbError::X402RelayQuotaExceeded(_))
        ));
        store
            .check_sponsored_action_capacity(&network, "new", "another-wallet", 2, 1)
            .await
            .unwrap();
        store
            .check_sponsored_action_capacity(&network, "new", "another-wallet", 2, 1)
            .await
            .unwrap();
        assert!(store
            .reserve_sponsored_action(&network, "different", "wallet", 1, 1)
            .await
            .is_err());
        let lease = Uuid::new_v4();
        let (a, b) = tokio::join!(
            store.claim_sponsored_action(a.id, lease, 30),
            store.claim_sponsored_action(a.id, Uuid::new_v4(), 30)
        );
        assert_ne!(a.unwrap(), b.unwrap());
        // Separate operation gives this test a known lease owner.
        let operation = store
            .reserve_sponsored_action(&format!("{network}-2"), "same", "wallet", 1, 1)
            .await
            .unwrap();
        assert!(store
            .claim_sponsored_action(operation.id, lease, 30)
            .await
            .unwrap());
        let sent = store
            .finish_sponsored_action(
                operation.id,
                Some(lease),
                "broadcast",
                Some("0xfixture"),
                None,
            )
            .await
            .unwrap();
        assert_eq!(sent.transaction_hash.as_deref(), Some("0xfixture"));
        assert!(!store
            .claim_sponsored_action(operation.id, Uuid::new_v4(), 30)
            .await
            .unwrap());
        assert_eq!(
            store
                .finish_sponsored_action(
                    operation.id,
                    Some(Uuid::new_v4()),
                    "failed",
                    None,
                    Some("stale failure")
                )
                .await
                .unwrap()
                .status,
            "broadcast"
        );
        assert_eq!(
            store
                .finish_sponsored_action(operation.id, None, "confirmed", None, None)
                .await
                .unwrap()
                .status,
            "confirmed"
        );
        let retry = store
            .reserve_sponsored_action(&format!("{network}-3"), "retry", "wallet", 1, 1)
            .await
            .unwrap();
        for _ in 0..3 {
            assert!(store
                .claim_sponsored_action(retry.id, lease, 30)
                .await
                .unwrap());
            store
                .finish_sponsored_action(
                    retry.id,
                    Some(lease),
                    "failed",
                    None,
                    Some("fixture interruption"),
                )
                .await
                .unwrap();
        }
        assert!(!store
            .claim_sponsored_action(retry.id, lease, 30)
            .await
            .unwrap());
    }
}
