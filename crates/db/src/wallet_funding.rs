use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct WalletTopupAttempt {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub wallet: String,
    pub provider: String,
    pub status: String,
    pub context: serde_json::Value,
    pub quote: Option<serde_json::Value>,
    pub expires_at: Option<DateTime<Utc>>,
}

fn topup(row: PgRow) -> DbResult<WalletTopupAttempt> {
    Ok(WalletTopupAttempt {
        id: row.try_get("id")?,
        operation_id: row.try_get("operation_id")?,
        wallet: row.try_get("wallet")?,
        provider: row.try_get("provider")?,
        status: row.try_get("status")?,
        context: row.try_get("context")?,
        quote: row.try_get("quote")?,
        expires_at: row.try_get("expires_at")?,
    })
}

impl PostgresStore {
    pub async fn has_posting_legal_acceptance(
        &self,
        id: Uuid,
        wallet: &str,
        terms: &str,
        privacy: &str,
        statement: &str,
    ) -> DbResult<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM legal_acceptances WHERE id=$1 AND wallet_address=lower($2) AND action='post_bounty' AND terms_version=$3 AND privacy_version=$4 AND statement_hash=$5)")
            .bind(id).bind(wallet).bind(terms).bind(privacy).bind(statement).fetch_one(&self.pool).await?)
    }
    #[allow(clippy::type_complexity)]
    pub async fn pending_posting_creations(
        &self,
    ) -> DbResult<Vec<(String, Uuid, String, serde_json::Value, Uuid)>> {
        let rows=sqlx::query("SELECT p.account_id,p.operation_id,p.draft_hash,p.create_request,p.relay_id FROM posting_creation_relays p JOIN x402_relay_attempts a ON a.id=p.relay_id WHERE (a.status IN ('prepared','broadcast') OR (a.status='failed' AND a.retryable AND a.updated_at < now() - interval '60 seconds') OR (a.status='relaying' AND a.lease_expires_at < now())) ORDER BY p.created_at LIMIT 20").fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|row| {
                Ok((
                    row.try_get("account_id")?,
                    row.try_get("operation_id")?,
                    row.try_get("draft_hash")?,
                    row.try_get("create_request")?,
                    row.try_get("relay_id")?,
                ))
            })
            .collect()
    }
    pub async fn repair_posting_transaction_marker(&self, relay: Uuid) -> DbResult<()> {
        // A crash after durable signing but before the broadcast marker cannot
        // allocate a new transaction. Recovery only reconciles the retained hash.
        sqlx::query("UPDATE x402_relay_attempts a SET status='broadcast', tx_hash=t.tx_hash, updated_at=now() FROM posting_relay_transactions t WHERE a.id=$1 AND t.relay_id=a.id AND a.tx_hash IS NULL AND a.status='relaying' AND a.lease_expires_at <= now()")
            .bind(relay).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn active_wallet_topup(
        &self,
        owner: &str,
        operation: Uuid,
    ) -> DbResult<Option<WalletTopupAttempt>> {
        sqlx::query("SELECT * FROM wallet_topup_attempts WHERE account_id = $1 AND operation_id = $2 ORDER BY created_at DESC LIMIT 1")
            .bind(owner).bind(operation).fetch_optional(&self.pool).await?.map(topup).transpose()
    }

    pub async fn reserve_wallet_topup(
        &self,
        owner: &str,
        operation: Uuid,
        wallet: &str,
        provider: &str,
        fingerprint: &str,
        context: &serde_json::Value,
    ) -> DbResult<(WalletTopupAttempt, bool)> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 3838))")
            .bind(owner)
            .execute(&mut *tx)
            .await?;
        let prior = sqlx::query("SELECT * FROM wallet_topup_attempts WHERE account_id = $1 AND (operation_id = $2 OR wallet = $3) AND status IN ('preparing','prepared','pending','unknown') ORDER BY created_at LIMIT 1")
            .bind(owner).bind(operation).bind(wallet.to_lowercase()).fetch_optional(&mut *tx).await?;
        if let Some(row) = prior {
            tx.commit().await?;
            return Ok((topup(row)?, false));
        }
        let recent: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wallet_topup_attempts WHERE account_id=$1 AND created_at >= now() - interval '1 hour'")
            .bind(owner).fetch_one(&mut *tx).await?;
        if recent >= 10 {
            return Err(DbError::X402RelayQuotaExceeded(
                "wallet top-up preparation limit; wait before requesting another quote".into(),
            ));
        }
        let row = sqlx::query("INSERT INTO wallet_topup_attempts (id,account_id,operation_id,wallet,provider,request_fingerprint,context,status) VALUES ($1,$2,$3,$4,$5,$6,$7,'preparing') RETURNING *")
            .bind(Uuid::new_v4()).bind(owner).bind(operation).bind(wallet.to_lowercase()).bind(provider).bind(fingerprint).bind(context).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok((topup(row)?, true))
    }

    pub async fn update_wallet_topup(
        &self,
        owner: &str,
        id: Uuid,
        expected: &str,
        status: &str,
        quote: Option<&serde_json::Value>,
        expires: Option<DateTime<Utc>>,
    ) -> DbResult<Option<WalletTopupAttempt>> {
        sqlx::query("UPDATE wallet_topup_attempts SET status=$4,quote=COALESCE($5,quote),expires_at=COALESCE($6,expires_at),updated_at=now() WHERE account_id=$1 AND id=$2 AND status=$3 RETURNING *")
            .bind(owner).bind(id).bind(expected).bind(status).bind(quote).bind(expires).fetch_optional(&self.pool).await?.map(topup).transpose()
    }

    pub async fn reserve_posting_creation(
        &self,
        owner: &str,
        operation: Uuid,
        draft_hash: &str,
        fingerprint: &str,
        create: &serde_json::Value,
    ) -> DbResult<Uuid> {
        sqlx::query("INSERT INTO posting_creation_relays (account_id,operation_id,relay_id,draft_hash,request_fingerprint,create_request) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT DO NOTHING")
            .bind(owner).bind(operation).bind(Uuid::new_v4()).bind(draft_hash).bind(fingerprint).bind(create).execute(&self.pool).await?;
        let row = sqlx::query("SELECT relay_id,draft_hash,request_fingerprint FROM posting_creation_relays WHERE account_id=$1 AND operation_id=$2")
            .bind(owner).bind(operation).fetch_one(&self.pool).await?;
        if row.try_get::<String, _>("draft_hash")? != draft_hash
            || row.try_get::<String, _>("request_fingerprint")? != fingerprint
        {
            return Err(DbError::X402RelayConflict(
                "This posting already has a different creation authorization".into(),
            ));
        }
        row.try_get("relay_id").map_err(Into::into)
    }

    pub async fn posting_creation_relay(
        &self,
        owner: &str,
        operation: Uuid,
    ) -> DbResult<Option<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT relay_id FROM posting_creation_relays WHERE account_id=$1 AND operation_id=$2",
        )
        .bind(owner)
        .bind(operation)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn persist_posting_transaction(
        &self,
        relay: Uuid,
        network: &str,
        prepared: &chain_base::PreparedBaseRelayTransaction,
    ) -> DbResult<()> {
        sqlx::query("INSERT INTO posting_relay_transactions (relay_id,network,relayer_address,nonce,tx_hash,signed_transaction,transaction_metadata) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (relay_id) DO NOTHING")
            .bind(relay).bind(network).bind(prepared.transaction.relayer.to_lowercase()).bind(i64_from_u64(prepared.nonce)?).bind(&prepared.transaction.tx_hash).bind(&prepared.signed_transaction).bind(serde_json::to_value(&prepared.transaction)?).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn posting_transaction(
        &self,
        relay: Uuid,
    ) -> DbResult<Option<chain_base::PreparedBaseRelayTransaction>> {
        let row = sqlx::query("SELECT nonce,signed_transaction,transaction_metadata FROM posting_relay_transactions WHERE relay_id=$1").bind(relay).fetch_optional(&self.pool).await?;
        row.map(|row| {
            Ok(chain_base::PreparedBaseRelayTransaction {
                nonce: row.try_get::<i64, _>("nonce")? as u64,
                signed_transaction: row.try_get("signed_transaction")?,
                transaction: serde_json::from_value(row.try_get("transaction_metadata")?)?,
            })
        })
        .transpose()
    }

    pub async fn x402_quota_available(
        &self,
        network: &str,
        contributor: &str,
        total: u32,
        per_wallet: u32,
    ) -> DbResult<bool> {
        let row = sqlx::query("SELECT COUNT(*) AS total, COUNT(*) FILTER (WHERE contributor=$2) AS contributor FROM x402_relay_attempts WHERE network=$1 AND created_at >= now() - interval '24 hours'")
            .bind(network).bind(contributor.to_lowercase()).fetch_one(&self.pool).await?;
        Ok(row.try_get::<i64, _>("total")? < i64::from(total)
            && row.try_get::<i64, _>("contributor")? < i64::from(per_wallet))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn wallet_topup_concurrent_providers_reload_and_account_isolation() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let owner = Uuid::new_v4().to_string();
        let operation = Uuid::new_v4();
        let wallet = "0x1111111111111111111111111111111111111111";
        let context = serde_json::json!({"shortfall_usdc_units":"2010000"});
        let (a, b) = tokio::join!(
            store.reserve_wallet_topup(&owner, operation, wallet, "moonpay", "a", &context),
            store.reserve_wallet_topup(&owner, operation, wallet, "coinbase", "b", &context)
        );
        let (a, first) = a.unwrap();
        let (b, second) = b.unwrap();
        assert_eq!(a.id, b.id);
        assert_ne!(first, second);
        assert!(store
            .active_wallet_topup("another-account", operation)
            .await
            .unwrap()
            .is_none());
        let prepared = store
            .update_wallet_topup(
                &owner,
                a.id,
                "preparing",
                "prepared",
                Some(&serde_json::json!({"received_usdc":"5.00"})),
                Some(Utc::now() + chrono::Duration::minutes(5)),
            )
            .await
            .unwrap()
            .unwrap();
        let (open_a, open_b) = tokio::join!(
            store.update_wallet_topup(&owner, a.id, "prepared", "pending", None, None),
            store.update_wallet_topup(&owner, a.id, "prepared", "pending", None, None)
        );
        assert_ne!(open_a.unwrap().is_some(), open_b.unwrap().is_some());
        let reloaded =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        assert_eq!(
            reloaded
                .active_wallet_topup(&owner, operation)
                .await
                .unwrap()
                .unwrap()
                .status,
            "pending"
        );
        let (other, new) = store
            .reserve_wallet_topup(
                &owner,
                Uuid::new_v4(),
                wallet,
                "coinbase",
                "switch",
                &context,
            )
            .await
            .unwrap();
        assert_eq!(other.id, prepared.id);
        assert!(!new);
        assert!(store
            .update_wallet_topup("another-account", a.id, "pending", "cancelled", None, None)
            .await
            .unwrap()
            .is_none());
        store
            .update_wallet_topup(&owner, a.id, "pending", "unknown", None, None)
            .await
            .unwrap();
        assert!(
            !store
                .reserve_wallet_topup(&owner, operation, wallet, "coinbase", "c", &context)
                .await
                .unwrap()
                .1
        );
        store
            .update_wallet_topup(&owner, a.id, "unknown", "cancelled", None, None)
            .await
            .unwrap();
        assert!(
            store
                .reserve_wallet_topup(&owner, operation, wallet, "coinbase", "d", &context)
                .await
                .unwrap()
                .1
        );
        store.migrate().await.unwrap(); // Additive migrations are safe after populated recovery data.
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn posting_relay_hash_survives_crash_and_blocks_nonce_reuse() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let owner = Uuid::new_v4().to_string();
        let operation = Uuid::new_v4();
        let create = serde_json::json!({"create":{"terms_hash":"exact"},"signature":"fixture"});
        let id = store
            .reserve_posting_creation(&owner, operation, "draft", "input", &create)
            .await
            .unwrap();
        assert_eq!(
            store
                .reserve_posting_creation(&owner, operation, "draft", "input", &create)
                .await
                .unwrap(),
            id
        );
        assert!(store
            .reserve_posting_creation(&owner, operation, "changed", "input", &create)
            .await
            .is_err());
        let network = format!("creation-test-{}", Uuid::new_v4());
        let relayer = "0x2222222222222222222222222222222222222222";
        let attempt = NewX402RelayAttempt {
            id,
            idempotency_key: format!("test:{operation}"),
            network: network.clone(),
            bounty_contract: "0x3333333333333333333333333333333333333333".into(),
            contributor: "0x4444444444444444444444444444444444444444".into(),
            amount: 2_010_000,
            authorization_nonce: format!("0x{:064x}", operation.as_u128()),
            authorization_valid_before: 2_000_000_000,
            request_fingerprint: "input".into(),
            relayer_address: relayer.into(),
        };
        store
            .reserve_x402_relay_attempt(&attempt, 1, 1)
            .await
            .unwrap();
        assert!(!store
            .x402_quota_available(&network, &attempt.contributor, 1, 1)
            .await
            .unwrap());
        let lease = store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        store
            .claim_x402_relay_attempt(id, lease, 45)
            .await
            .unwrap()
            .unwrap();
        let prepared = chain_base::PreparedBaseRelayTransaction {
            nonce: 7,
            signed_transaction: "0xdeadbeef".into(),
            transaction: chain_base::BaseRelayedTransaction {
                relayer: relayer.into(),
                tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                estimated_gas: 100_000,
                gas_limit: 120_000,
                max_fee_per_gas_wei: 10,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 1_200_000,
            },
        };
        store
            .persist_posting_transaction(id, &network, &prepared)
            .await
            .unwrap();
        store
            .release_x402_relayer_lease(&network, lease)
            .await
            .unwrap();
        assert!(store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .is_none());
        store.repair_posting_transaction_marker(id).await.unwrap();
        assert_eq!(
            store
                .get_x402_relay_attempt(id)
                .await
                .unwrap()
                .unwrap()
                .status,
            X402RelayStatus::Relaying
        );
        sqlx::query(
            "UPDATE x402_relay_attempts SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(id)
        .execute(&store.pool)
        .await
        .unwrap();
        store.repair_posting_transaction_marker(id).await.unwrap();
        let restored = store.get_x402_relay_attempt(id).await.unwrap().unwrap();
        assert_eq!(restored.status, X402RelayStatus::Broadcast);
        assert_eq!(
            restored.tx_hash.as_deref(),
            Some(prepared.transaction.tx_hash.as_str())
        );
        assert_eq!(
            store
                .posting_transaction(id)
                .await
                .unwrap()
                .unwrap()
                .signed_transaction,
            prepared.signed_transaction
        );
        store
            .mark_x402_relay_confirmed(id, Uuid::new_v4(), 123)
            .await
            .unwrap();
        assert!(store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .is_some());
    }
}
