//! Timeout operations share nonce occupancy with payment outboxes, but never
//! share their replay allowance or wallet/payment identity.
use super::*;
use chain_base::{timeout_recovery::TimeoutRelayScope, PreparedBaseRelayTransaction};

#[derive(Clone)]
pub struct SavedTimeoutRelay {
    pub scope: TimeoutRelayScope,
    pub prepared: PreparedBaseRelayTransaction,
    pub send_attempted: bool,
    /// In-memory only: the first-send marker commit returned an error.
    pub marker_commit_unknown: bool,
}
fn invalid_timeout() -> DbError {
    DbError::X402RelayQuotaExceeded("saved timeout scope or envelope is immutable".into())
}
impl PostgresStore {
    /// Public canonical creation evidence is a separate namespace from private
    /// UUID bounties and retained artifacts. No cached readiness is required.
    pub async fn timeout_publication_matches(&self, scope: &TimeoutRelayScope) -> DbResult<bool> {
        scope.validate().map_err(|_| invalid_timeout())?;
        Ok(sqlx::query_scalar("SELECT count(*) > 0 AND bool_and(COALESCE(contract_address=$3 AND bounty_id=$4 AND data->>'terms_hash'=$5,false)) FROM autonomous_bounty_events WHERE network=$1 AND kind='canonical_bounty_created' AND data->>'bounty_contract'=$2")
            .bind(&scope.network).bind(&scope.bounty_contract).bind(&scope.factory_contract).bind(&scope.bounty_id).bind(&scope.terms_hash).fetch_one(&self.pool).await?)
    }
    pub async fn timeout_budget_day(&self) -> DbResult<chrono::NaiveDate> {
        Ok(
            sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'UTC')::date")
                .fetch_one(&self.pool)
                .await?,
        )
    }
    pub async fn timeout_relay(
        &self,
        id: Uuid,
        network: &str,
    ) -> DbResult<Option<SavedTimeoutRelay>> {
        sqlx::query("SELECT nonce,signed_transaction,transaction_metadata,send_attempts FROM relay_transaction_outbox WHERE operation_id=$1 AND network=$2")
            .bind(id).bind(network).fetch_optional(&self.pool).await?.map(|row| {
                let metadata: serde_json::Value = row.try_get("transaction_metadata")?;
                let scope: TimeoutRelayScope = serde_json::from_value(metadata.get("timeout_scope").cloned().ok_or_else(invalid_timeout)?)?;
                if scope.network != network || scope.operation_id().map_err(|_| invalid_timeout())? != id { return Err(invalid_timeout()); }
                Ok(SavedTimeoutRelay { scope, prepared: PreparedBaseRelayTransaction {
                    nonce: u64_from_i64(row.try_get("nonce")?)?, signed_transaction: row.try_get("signed_transaction")?,
                    transaction: serde_json::from_value(metadata)?,
                }, send_attempted: row.try_get::<i32,_>("send_attempts")? > 0, marker_commit_unknown: false })
            }).transpose()
    }
    pub async fn save_timeout_relay(
        &self,
        scope: &TimeoutRelayScope,
        lease: Uuid,
        prepared: &PreparedBaseRelayTransaction,
        preparation_day: chrono::NaiveDate,
    ) -> DbResult<()> {
        let id = scope.operation_id().map_err(|_| invalid_timeout())?;
        let mut metadata = serde_json::to_value(&prepared.transaction)?;
        metadata["timeout_scope"] = serde_json::to_value(scope)?;
        let mut tx = self.pool.begin().await?;
        // Serialize saves under the existing live network lease before checking
        // any outbox. ON CONFLICT(operation_id) alone cannot arbitrate a
        // concurrent collision on the separate transaction-hash unique index.
        // The INSERT rechecks lease expiry and budget day after the lock is acquired.
        let held: Option<Uuid> = sqlx::query_scalar("SELECT lease_token FROM x402_relayer_leases WHERE network=$1 AND lease_token=$2 AND lease_expires_at>clock_timestamp() FOR UPDATE")
            .bind(&scope.network).bind(lease).fetch_optional(&mut *tx).await?;
        if held.is_none() {
            return Err(invalid_timeout());
        }
        let conflict: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM posting_relay_transactions WHERE network=$1 AND relayer_address=$2 AND nonce=$3)")
            .bind(&scope.network).bind(prepared.transaction.relayer.to_ascii_lowercase()).bind(i64_from_u64(prepared.nonce)?).fetch_one(&mut *tx).await?;
        if conflict {
            return Err(invalid_timeout());
        }
        let row = sqlx::query("INSERT INTO relay_transaction_outbox(operation_id,network,relayer_address,nonce,tx_hash,signed_transaction,transaction_metadata) SELECT $1,$2,$3,$4,$5,$6,$7 WHERE EXISTS(SELECT 1 FROM x402_relayer_leases WHERE network=$2 AND lease_token=$8 AND lease_expires_at>clock_timestamp()) AND $9::date=(clock_timestamp() AT TIME ZONE 'UTC')::date ON CONFLICT(operation_id) DO UPDATE SET operation_id=excluded.operation_id RETURNING network,nonce,signed_transaction,transaction_metadata")
            .bind(id).bind(&scope.network).bind(prepared.transaction.relayer.to_ascii_lowercase()).bind(i64_from_u64(prepared.nonce)?)
            .bind(&prepared.transaction.tx_hash).bind(&prepared.signed_transaction).bind(&metadata).bind(lease).bind(preparation_day).fetch_optional(&mut *tx).await?.ok_or_else(invalid_timeout)?;
        if row.try_get::<String, _>("network")? != scope.network
            || row.try_get::<i64, _>("nonce")? != i64_from_u64(prepared.nonce)?
            || row.try_get::<String, _>("signed_transaction")? != prepared.signed_transaction
            || row.try_get::<serde_json::Value, _>("transaction_metadata")? != metadata
        {
            return Err(invalid_timeout());
        }
        tx.commit().await?;
        Ok(())
    }
    /// Mark uncertainty BEFORE the only permitted send. A crash here sacrifices
    /// liveness, never safety: subsequent requests can only read the exact hash.
    pub async fn claim_timeout_relay_send(
        &self,
        scope: &TimeoutRelayScope,
        lease: Uuid,
        send_seconds: u64,
    ) -> DbResult<bool> {
        let id = scope.operation_id().map_err(|_| invalid_timeout())?;
        let seconds = send_seconds.checked_add(2).ok_or_else(invalid_timeout)?;
        let result = sqlx::query("UPDATE relay_transaction_outbox o SET send_attempts=1 WHERE operation_id=$1 AND network=$2 AND transaction_metadata->'timeout_scope'=$3 AND send_attempts=0 AND created_at>clock_timestamp()-interval '120 seconds' AND (created_at AT TIME ZONE 'UTC')::date=(clock_timestamp() AT TIME ZONE 'UTC')::date AND EXISTS(SELECT 1 FROM x402_relayer_leases WHERE network=$2 AND lease_token=$4 AND lease_expires_at>clock_timestamp()+make_interval(secs=>$5))")
            .bind(id).bind(&scope.network).bind(serde_json::to_value(scope)?).bind(lease).bind(i64_from_u64(seconds)?).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> TimeoutRelayScope {
        TimeoutRelayScope {
            network: "base-sepolia".into(),
            bounty_contract: format!("0x{:040x}", Uuid::new_v4().as_u128()),
            action: "expire_claim".into(),
            round: 7,
            factory_contract: format!("0x{}", "11".repeat(20)),
            bounty_id: format!("0x{}", "22".repeat(32)),
            terms_hash: format!("0x{}", "33".repeat(32)),
        }
    }
    fn prepared() -> PreparedBaseRelayTransaction {
        PreparedBaseRelayTransaction {
            nonce: 10,
            signed_transaction: "0x010203".into(),
            transaction: chain_base::BaseRelayedTransaction {
                relayer: format!("0x{:040x}", Uuid::new_v4().as_u128()),
                tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                estimated_gas: 21000,
                gas_limit: 25000,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75000,
            },
        }
    }
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn concurrent_timeout_saves_wait_for_live_lease_and_preserve_original_envelope() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let scope = scope();
        let prepared = prepared();
        let day = store.timeout_budget_day().await.unwrap();
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 90)
            .await
            .unwrap()
            .unwrap();
        let mut blocker = store.pool.begin().await.unwrap();
        sqlx::query("SELECT lease_token FROM x402_relayer_leases WHERE network=$1 FOR UPDATE")
            .bind(&scope.network)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let mut first = Box::pin(store.save_timeout_relay(&scope, lease, &prepared, day));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(75), &mut first)
                .await
                .is_err(),
            "save must wait for the lease row lock before inserting"
        );
        blocker.rollback().await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), first)
            .await
            .unwrap()
            .unwrap();
        let id = scope.operation_id().unwrap();
        let original: (DateTime<Utc>, String, serde_json::Value) = sqlx::query_as(
            "SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1",
        )
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
        let mut attempts = tokio::task::JoinSet::new();
        for _ in 0..16 {
            let (store, scope, prepared, barrier) = (
                store.clone(),
                scope.clone(),
                prepared.clone(),
                barrier.clone(),
            );
            attempts.spawn(async move {
                barrier.wait().await;
                store
                    .save_timeout_relay(&scope, lease, &prepared, day)
                    .await
            });
        }
        while let Some(result) = attempts.join_next().await {
            result.unwrap().unwrap();
        }
        let unchanged: (DateTime<Utc>, String, serde_json::Value) = sqlx::query_as(
            "SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1",
        )
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(unchanged, original);
        let mut wrong = prepared.clone();
        wrong.signed_transaction = "0x040506".into();
        wrong.transaction.tx_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
        assert!(store
            .save_timeout_relay(&scope, lease, &wrong, day)
            .await
            .is_err());
        let mut next_scope = scope.clone();
        next_scope.round += 1;
        assert!(
            store
                .save_timeout_relay(&next_scope, lease, &prepared, day)
                .await
                .is_err(),
            "a distinct operation cannot reuse the nonce/hash"
        );

        // Expiry while waiting does not revive a lease or publish a new row.
        let mut blocker = store.pool.begin().await.unwrap();
        sqlx::query("SELECT lease_token FROM x402_relayer_leases WHERE network=$1 FOR UPDATE")
            .bind(&scope.network)
            .fetch_one(&mut *blocker)
            .await
            .unwrap();
        let mut next = prepared.clone();
        next.nonce += 1;
        next.transaction.tx_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
        let mut waiting = Box::pin(store.save_timeout_relay(&next_scope, lease, &next, day));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(75), &mut waiting)
                .await
                .is_err()
        );
        sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE network=$1")
            .bind(&scope.network).execute(&mut *blocker).await.unwrap();
        blocker.commit().await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .is_err()
        );
        assert!(store
            .timeout_relay(next_scope.operation_id().unwrap(), &scope.network)
            .await
            .unwrap()
            .is_none());
        let final_row: (DateTime<Utc>, String, serde_json::Value) = sqlx::query_as(
            "SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1",
        ).bind(id).fetch_one(&store.pool).await.unwrap();
        assert_eq!(final_row, original);
        assert!(
            !store
                .timeout_relay(id, &scope.network)
                .await
                .unwrap()
                .unwrap()
                .send_attempted
        );
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
    }
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn timeout_outbox_is_one_send_durable_and_isolated_from_payment_replays() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        assert!(url.contains("@127.0.0.1:"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let scope = scope();
        let id = scope.operation_id().unwrap();
        let p = prepared();
        let mut legacy = Bounty::new(
            "Private fixture",
            "fix-ci-failure",
            Money::new(1, "usd").unwrap(),
            FundingMode::StripeFiatLedger,
            PrivacyLevel::Private,
        );
        legacy.terms_hash = Some(scope.terms_hash.clone());
        store.upsert_bounty(&legacy).await.unwrap();
        assert!(
            !store.timeout_publication_matches(&scope).await.unwrap(),
            "private legacy records cannot publish canonical identities"
        );
        assert!(store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .is_none()); // preparation failure has no envelope/send marker
        assert!(store
            .save_timeout_relay(
                &scope,
                Uuid::new_v4(),
                &p,
                store.timeout_budget_day().await.unwrap()
            )
            .await
            .is_err());
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 90)
            .await
            .unwrap()
            .unwrap();
        let day = store.timeout_budget_day().await.unwrap();
        assert!(store
            .save_timeout_relay(&scope, lease, &p, day.pred_opt().unwrap())
            .await
            .is_err());
        let (a, b) = tokio::join!(
            store.save_timeout_relay(&scope, lease, &p, day),
            store.save_timeout_relay(&scope, lease, &p, day)
        );
        a.unwrap();
        b.unwrap();
        let record = store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .unwrap();
        assert!(!record.send_attempted);
        assert_eq!(record.scope, scope);
        assert!(store
            .timeout_relay(id, "base-mainnet")
            .await
            .unwrap()
            .is_none());
        let mut changed = scope.clone();
        changed.terms_hash = format!("0x{}", "44".repeat(32));
        assert_eq!(changed.operation_id().unwrap(), id);
        assert!(store
            .save_timeout_relay(&changed, lease, &p, day)
            .await
            .is_err());
        changed = scope.clone();
        changed.round += 1;
        assert_ne!(changed.operation_id().unwrap(), id);
        assert!(store
            .save_timeout_relay(&changed, lease, &p, day)
            .await
            .is_err()); // cross-operation nonce fence
        assert!(store
            .save_relay_outbox(id, &scope.network, &p)
            .await
            .is_err());
        assert!(store
            .save_relay_outbox(Uuid::new_v4(), &scope.network, &p)
            .await
            .is_err());
        assert!(store
            .relay_outbox(id, &scope.network)
            .await
            .unwrap()
            .is_none());
        assert!(!store.relay_outbox_send_ready(id).await.unwrap());
        assert!(!store.claim_relay_outbox_send(id).await.unwrap());
        assert!(store
            .gas_sponsor_budget()
            .ensure_sender_nonce_available(84532, &p.transaction.relayer, p.nonce)
            .await
            .is_err());
        assert!(store
            .gas_sponsor_budget()
            .ensure_sender_nonce_available(8453, &p.transaction.relayer, p.nonce)
            .await
            .is_ok());
        assert!(!store
            .claim_timeout_relay_send(&scope, Uuid::new_v4(), 15)
            .await
            .unwrap());
        assert!(!store
            .claim_timeout_relay_send(&changed, lease, 15)
            .await
            .unwrap());
        let (a, b) = tokio::join!(
            store.claim_timeout_relay_send(&scope, lease, 15),
            store.claim_timeout_relay_send(&scope, lease, 15)
        );
        assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
        // A lost send reply, a crash before send, and an accepted send all have
        // this same durable state. None can be resent by any generic path.
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
        assert!(store
            .acquire_saved_relayer_lease(&scope.network, id, 30)
            .await
            .unwrap()
            .is_none());
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 90)
            .await
            .unwrap()
            .unwrap();
        assert!(!store
            .claim_timeout_relay_send(&scope, lease, 15)
            .await
            .unwrap());
        assert!(!store.claim_relay_outbox_send(id).await.unwrap());
        let record = store
            .timeout_relay(id, &scope.network)
            .await
            .unwrap()
            .unwrap();
        assert!(record.send_attempted);
        assert_eq!(record.prepared.transaction.tx_hash, p.transaction.tx_hash);
        // Even an accidental cross-table UUID collision cannot publish a
        // timeout as a payment or grant its generic replay allowance.
        sqlx::query(
            "INSERT INTO sponsored_actions(id,network,request_hash,wallet) VALUES($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(&scope.network)
        .bind(id.to_string())
        .bind(&p.transaction.relayer)
        .execute(&store.pool)
        .await
        .unwrap();
        let request = NewX402RelayAttempt {
            id,
            idempotency_key: id.to_string(),
            network: scope.network.clone(),
            bounty_contract: scope.bounty_contract.clone(),
            contributor: p.transaction.relayer.clone(),
            amount: 1,
            authorization_nonce: scope.terms_hash.clone(),
            authorization_valid_before: Utc::now().timestamp() as u64 + 60,
            request_fingerprint: id.to_string(),
            relayer_address: p.transaction.relayer.clone(),
        };
        store
            .reserve_x402_relay_attempt(&request, u32::MAX, u32::MAX)
            .await
            .unwrap();
        store.repair_x402_outbox_marker(id).await.unwrap();
        store.repair_sponsored_outbox_marker(id).await.unwrap();
        let sponsored: Option<String> =
            sqlx::query_scalar("SELECT transaction_hash FROM sponsored_actions WHERE id=$1")
                .bind(id)
                .fetch_one(&store.pool)
                .await
                .unwrap();
        assert!(sponsored.is_none());
        assert!(store
            .get_x402_relay_attempt(id)
            .await
            .unwrap()
            .unwrap()
            .tx_hash
            .is_none());
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
    }
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn timeout_first_send_requires_unexpired_lease_and_recent_same_day_preparation() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        assert!(url.contains("@127.0.0.1:"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let scope = scope();
        let id = scope.operation_id().unwrap();
        let p = prepared();
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 90)
            .await
            .unwrap()
            .unwrap();
        store
            .save_timeout_relay(&scope, lease, &p, store.timeout_budget_day().await.unwrap())
            .await
            .unwrap();
        assert!(store
            .claim_timeout_relay_send(&scope, lease, u64::MAX)
            .await
            .is_err());
        assert!(!store
            .claim_timeout_relay_send(&scope, lease, 100)
            .await
            .unwrap());
        sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp()-interval '121 seconds' WHERE operation_id=$1").bind(id).execute(&store.pool).await.unwrap();
        assert!(!store
            .claim_timeout_relay_send(&scope, lease, 15)
            .await
            .unwrap());
        sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp()-interval '1 day' WHERE operation_id=$1").bind(id).execute(&store.pool).await.unwrap();
        assert!(!store
            .claim_timeout_relay_send(&scope, lease, 15)
            .await
            .unwrap());
        sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp() WHERE operation_id=$1").bind(id).execute(&store.pool).await.unwrap();
        store
            .release_x402_relayer_lease(&scope.network, lease)
            .await
            .unwrap();
        assert!(!store
            .claim_timeout_relay_send(&scope, lease, 15)
            .await
            .unwrap());
        assert!(
            !store
                .timeout_relay(id, &scope.network)
                .await
                .unwrap()
                .unwrap()
                .send_attempted
        );
    }
}
