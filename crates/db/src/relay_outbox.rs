//! Private execution journal. Signed envelopes are never included in public responses.
use super::*;
impl PostgresStore {
    pub async fn acquire_saved_relayer_lease(
        &self,
        network: &str,
        id: Uuid,
        seconds: u64,
    ) -> DbResult<Option<Uuid>> {
        let token = Uuid::new_v4();
        let row = sqlx::query("INSERT INTO x402_relayer_leases(network,lease_token,lease_expires_at,updated_at) SELECT $1,$2,now()+make_interval(secs=>$3),now() WHERE NOT EXISTS(SELECT 1 FROM nonce_retirement_fences WHERE network=$1) AND EXISTS(SELECT 1 FROM relay_transaction_outbox WHERE operation_id=$4 AND network=$1 AND NOT (transaction_metadata ? 'timeout_scope')) ON CONFLICT(network) DO UPDATE SET lease_token=excluded.lease_token,lease_expires_at=excluded.lease_expires_at,updated_at=now() WHERE x402_relayer_leases.lease_expires_at<=now() RETURNING lease_token")
            .bind(network).bind(token).bind(i64_from_u64(seconds)?).bind(id).fetch_optional(&self.pool).await?;
        row.map(|r| r.try_get("lease_token"))
            .transpose()
            .map_err(Into::into)
    }
    pub async fn relayer_lease_has_time(
        &self,
        network: &str,
        token: Uuid,
        seconds: u64,
    ) -> DbResult<bool> {
        let r=sqlx::query("SELECT EXISTS(SELECT 1 FROM x402_relayer_leases WHERE network=$1 AND lease_token=$2 AND lease_expires_at>clock_timestamp()+make_interval(secs=>$3)) AS ready")
            .bind(network).bind(token).bind(i64_from_u64(seconds)?).fetch_one(&self.pool).await?;
        Ok(r.try_get("ready")?)
    }
    pub async fn relay_outbox(
        &self,
        id: Uuid,
        network: &str,
    ) -> DbResult<Option<chain_base::PreparedBaseRelayTransaction>> {
        sqlx::query("SELECT nonce,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1 AND network=$2 AND NOT (transaction_metadata ? 'timeout_scope')").bind(id).bind(network).fetch_optional(&self.pool).await?.map(|r|Ok(chain_base::PreparedBaseRelayTransaction{nonce:u64_from_i64(r.try_get("nonce")?)?,signed_transaction:r.try_get("signed_transaction")?,transaction:serde_json::from_value(r.try_get("transaction_metadata")?)?})).transpose()
    }
    pub async fn save_relay_outbox(
        &self,
        id: Uuid,
        network: &str,
        p: &chain_base::PreparedBaseRelayTransaction,
    ) -> DbResult<()> {
        // The network sender lease must be held through this insert and send.
        // Preserve compatibility with the already deployed private posting journal.
        let conflict=sqlx::query("SELECT relay_id FROM posting_relay_transactions WHERE network=$1 AND relayer_address=$2 AND nonce=$3 AND relay_id<>$4")
            .bind(network).bind(p.transaction.relayer.to_ascii_lowercase()).bind(i64_from_u64(p.nonce)?).bind(id).fetch_optional(&self.pool).await?;
        if conflict.is_some() {
            return Err(DbError::X402RelayQuotaExceeded(
                "sender nonce already belongs to a saved transaction".into(),
            ));
        }
        let r=sqlx::query("INSERT INTO relay_transaction_outbox(operation_id,network,relayer_address,nonce,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT(operation_id) DO UPDATE SET operation_id=excluded.operation_id RETURNING network,tx_hash,transaction_metadata")
            .bind(id).bind(network).bind(p.transaction.relayer.to_ascii_lowercase()).bind(i64_from_u64(p.nonce)?).bind(&p.transaction.tx_hash).bind(&p.signed_transaction).bind(serde_json::to_value(&p.transaction)?).fetch_one(&self.pool).await?;
        if r.try_get::<serde_json::Value, _>("transaction_metadata")?
            .get("timeout_scope")
            .is_some()
            || r.try_get::<String, _>("network")? != network
            || r.try_get::<String, _>("tx_hash")? != p.transaction.tx_hash
        {
            return Err(DbError::X402RelayQuotaExceeded(
                "saved transaction is immutable".into(),
            ));
        }
        Ok(())
    }
    pub async fn relay_outbox_send_ready(&self, id: Uuid) -> DbResult<bool> {
        let row=sqlx::query("SELECT EXISTS(SELECT 1 FROM relay_transaction_outbox WHERE operation_id=$1 AND NOT (transaction_metadata ? 'timeout_scope') AND send_attempts<3 AND next_send_at<=now() AND created_at>now()-interval '2 hours') AS ready").bind(id).fetch_one(&self.pool).await?;
        Ok(row.try_get("ready")?)
    }
    pub async fn claim_relay_outbox_send(&self, id: Uuid) -> DbResult<bool> {
        // A crash may consume an attempt without a send. Never allocate another
        // nonce or exceed three submissions of these exact bytes.
        let r=sqlx::query("UPDATE relay_transaction_outbox SET send_attempts=send_attempts+1,next_send_at=now()+interval '10 seconds' WHERE operation_id=$1 AND NOT (transaction_metadata ? 'timeout_scope') AND send_attempts<3 AND next_send_at<=now() AND created_at>now()-interval '2 hours'").bind(id).execute(&self.pool).await?;
        Ok(r.rows_affected() == 1)
    }
    pub async fn repair_x402_outbox_marker(&self, id: Uuid) -> DbResult<()> {
        sqlx::query("UPDATE x402_relay_attempts a SET tx_hash=o.tx_hash,status='broadcast',lease_token=NULL,lease_expires_at=NULL,updated_at=now() FROM relay_transaction_outbox o WHERE a.id=$1 AND o.operation_id=a.id AND NOT (o.transaction_metadata ? 'timeout_scope') AND a.network=o.network AND a.tx_hash IS NULL AND (a.lease_expires_at IS NULL OR a.lease_expires_at<=now())").bind(id).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn repair_sponsored_outbox_marker(&self, id: Uuid) -> DbResult<()> {
        sqlx::query("UPDATE sponsored_actions a SET transaction_hash=o.tx_hash,status='broadcast',lease_token=NULL,lease_expires_at=NULL,error_message=NULL,updated_at=now() FROM relay_transaction_outbox o WHERE a.id=$1 AND o.operation_id=a.id AND NOT (o.transaction_metadata ? 'timeout_scope') AND a.network=o.network AND a.transaction_hash IS NULL AND (a.lease_expires_at IS NULL OR a.lease_expires_at<=now())").bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires disposable loopback AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn outbox_survives_marker_crash_fences_nonces_and_bounds_sends() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        assert!(url.contains("@127.0.0.1:"));
        let store = PostgresStore::connect(&url).await.unwrap();
        store.migrate().await.unwrap();
        let network = "base-sepolia";
        let request = Uuid::new_v4().to_string();
        let operation = store
            .reserve_sponsored_action(network, &request, &request, u32::MAX, u32::MAX)
            .await
            .unwrap();
        let lease = Uuid::new_v4();
        assert!(store
            .claim_sponsored_action(operation.id, lease, 30)
            .await
            .unwrap());
        let mut saved = chain_base::PreparedBaseRelayTransaction {
            nonce: 0,
            signed_transaction: "0x010203".into(),
            transaction: chain_base::BaseRelayedTransaction {
                relayer: format!("0x{:040x}", Uuid::new_v4().as_u128()),
                tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                estimated_gas: 21_000,
                gas_limit: 25_200,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75_640,
            },
        };
        store
            .save_relay_outbox(operation.id, network, &saved)
            .await
            .unwrap();
        store
            .save_relay_outbox(operation.id, network, &saved)
            .await
            .unwrap();
        assert!(
            store
                .find_sponsored_action(network, &request)
                .await
                .unwrap()
                .unwrap()
                .transaction_hash
                .is_none(),
            "do not replace an active writer's lease"
        );
        sqlx::query(
            "UPDATE sponsored_actions SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(operation.id)
        .execute(&store.pool)
        .await
        .unwrap();
        let recovered = store
            .find_sponsored_action(network, &request)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(recovered.status, "broadcast");
        assert_eq!(
            recovered.transaction_hash.as_deref(),
            Some(saved.transaction.tx_hash.as_str())
        );
        assert!(!serde_json::to_string(&recovered)
            .unwrap()
            .contains("signed_transaction"));
        assert_eq!(
            store
                .relay_outbox(operation.id, network)
                .await
                .unwrap()
                .unwrap()
                .signed_transaction,
            saved.signed_transaction
        );
        let budget = store.gas_sponsor_budget();
        assert!(budget
            .ensure_sender_nonce_available(84532, &saved.transaction.relayer, 0)
            .await
            .is_err());
        assert!(budget
            .ensure_sender_nonce_available(84532, &saved.transaction.relayer, 1)
            .await
            .is_ok());
        assert!(store
            .save_relay_outbox(Uuid::new_v4(), network, &saved)
            .await
            .is_err());
        saved.transaction.tx_hash = format!("0x{:064x}", Uuid::new_v4().as_u128());
        assert!(store
            .save_relay_outbox(operation.id, network, &saved)
            .await
            .is_err());
        // X402 status and authorization lookup must also recover a crash
        // between the durable write and public hash marker, without a send.
        let x = NewX402RelayAttempt {
            id: Uuid::new_v4(),
            idempotency_key: Uuid::new_v4().to_string(),
            network: network.into(),
            bounty_contract: format!("0x{:040x}", Uuid::new_v4().as_u128()),
            contributor: format!("0x{:040x}", Uuid::new_v4().as_u128()),
            amount: 1,
            authorization_nonce: format!("0x{:064x}", Uuid::new_v4().as_u128()),
            authorization_valid_before: Utc::now().timestamp() as u64 + 600,
            request_fingerprint: Uuid::new_v4().to_string(),
            relayer_address: saved.transaction.relayer.clone(),
        };
        store
            .reserve_x402_relay_attempt(&x, u32::MAX, u32::MAX)
            .await
            .unwrap();
        store
            .claim_x402_relay_attempt(x.id, Uuid::new_v4(), 30)
            .await
            .unwrap()
            .unwrap();
        saved.nonce = 1;
        store
            .save_relay_outbox(x.id, network, &saved)
            .await
            .unwrap();
        assert!(store
            .get_x402_relay_attempt(x.id)
            .await
            .unwrap()
            .unwrap()
            .tx_hash
            .is_none());
        sqlx::query(
            "UPDATE x402_relay_attempts SET lease_expires_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(x.id)
        .execute(&store.pool)
        .await
        .unwrap();
        let recovered = store
            .get_x402_relay_attempt_by_authorization(
                network,
                &x.bounty_contract,
                &x.authorization_nonce,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            recovered.tx_hash.as_deref(),
            Some(saved.transaction.tx_hash.as_str())
        );
        assert_eq!(recovered.status, X402RelayStatus::Broadcast);
        let attempts: i32 = sqlx::query_scalar(
            "SELECT send_attempts FROM relay_transaction_outbox WHERE operation_id=$1",
        )
        .bind(x.id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 0, "repairing status cannot broadcast");
        for _ in 0..3 {
            assert!(store.claim_relay_outbox_send(operation.id).await.unwrap());
            assert!(!store.claim_relay_outbox_send(operation.id).await.unwrap());
            sqlx::query("UPDATE relay_transaction_outbox SET next_send_at=now()-interval '1 second' WHERE operation_id=$1").bind(operation.id).execute(&store.pool).await.unwrap();
        }
        assert!(!store.relay_outbox_send_ready(operation.id).await.unwrap());
        assert!(!store.claim_relay_outbox_send(operation.id).await.unwrap());
        sqlx::query("UPDATE relay_transaction_outbox SET send_attempts=0,created_at=now()-interval '3 hours' WHERE operation_id=$1").bind(operation.id).execute(&store.pool).await.unwrap();
        assert!(!store.relay_outbox_send_ready(operation.id).await.unwrap());
    }
}
