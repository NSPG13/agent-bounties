//! Canonical checkpoints are separate from immutable decoded event history.
//! Divergence contains reads; this module never deletes events or rewinds a cursor.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousCanonicalCheckpoint {
    pub block_number: u64,
    pub block_hash: String,
    pub invalidated: bool,
}

/// Cursor and certification must describe the same database snapshot before RPC reads.
pub struct AutonomousIndexerPosition {
    pub cursor: Option<BaseLogScanCursor>,
    pub checkpoint: Option<AutonomousCanonicalCheckpoint>,
}

fn unavailable() -> DbError {
    DbError::InvalidEnum("autonomous canonical history requires reconciliation".into())
}

/// Call inside the same repeatable-read transaction as the public projection.
/// Unassociated event rows cannot establish paid evidence on their own.
pub(crate) async fn require_certified_history(
    connection: &mut sqlx::PgConnection,
    network: &str,
) -> DbResult<()> {
    let uncertified: bool = sqlx::query_scalar(r#"
        SELECT EXISTS (
            SELECT 1 FROM autonomous_bounty_events e
            WHERE e.network=$1 AND NOT EXISTS (
                SELECT 1 FROM autonomous_bounty_events created
                JOIN autonomous_canonical_checkpoints c
                  ON c.network=created.network AND c.factory_contract=created.contract_address
                WHERE created.network=e.network AND created.kind='canonical_bounty_created'
                  AND created.bounty_id=e.bounty_id
                  AND e.contract_address IN (created.contract_address, lower(created.data->>'bounty_contract'))
                  AND NOT c.invalidated AND e.block_number<=c.block_number
            )
        )
    "#).bind(network).fetch_one(connection).await?;
    if uncertified {
        return Err(unavailable());
    }
    Ok(())
}

fn same_event_payloads(a: &[AutonomousBountyEvent], b: &[AutonomousBountyEvent]) -> DbResult<bool> {
    let signatures =
        |events: &[AutonomousBountyEvent]| -> DbResult<std::collections::BTreeSet<String>> {
            events
                .iter()
                .map(|e| {
                    Ok(serde_json::to_string(&serde_json::json!([
                        e.log_key,
                        e.tx_hash,
                        e.block_number,
                        e.log_index,
                        e.contract_address,
                        e.bounty_id,
                        e.kind,
                        e.data
                    ]))?)
                })
                .collect()
        };
    let left = signatures(a)?;
    let right = signatures(b)?;
    Ok(left.len() == a.len() && right.len() == b.len() && left == right)
}

impl PostgresStore {
    pub async fn autonomous_indexer_position(
        &self,
        network: &str,
        factory: &str,
    ) -> DbResult<AutonomousIndexerPosition> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let cursor = Self::get_base_log_cursor_on(&mut *tx, network, factory).await?;
        let checkpoint =
            Self::autonomous_canonical_checkpoint_on(&mut *tx, network, factory).await?;
        tx.commit().await?;
        Ok(AutonomousIndexerPosition { cursor, checkpoint })
    }

    pub async fn autonomous_canonical_checkpoint(
        &self,
        network: &str,
        factory: &str,
    ) -> DbResult<Option<AutonomousCanonicalCheckpoint>> {
        Self::autonomous_canonical_checkpoint_on(&self.pool, network, factory).await
    }

    pub(crate) async fn autonomous_canonical_checkpoint_on<
        'e,
        E: sqlx::Executor<'e, Database = sqlx::Postgres>,
    >(
        executor: E,
        network: &str,
        factory: &str,
    ) -> DbResult<Option<AutonomousCanonicalCheckpoint>> {
        let row = sqlx::query("SELECT block_number,block_hash,invalidated FROM autonomous_canonical_checkpoints WHERE network=$1 AND factory_contract=$2")
            .bind(network).bind(normalize_key_address(factory)).fetch_optional(executor).await?;
        row.map(|r| {
            Ok(AutonomousCanonicalCheckpoint {
                block_number: u64_from_i64(r.try_get("block_number")?)?,
                block_hash: r.try_get("block_hash")?,
                invalidated: r.try_get("invalidated")?,
            })
        })
        .transpose()
    }

    pub async fn invalidate_autonomous_canonical_checkpoint(
        &self,
        network: &str,
        factory: &str,
    ) -> DbResult<()> {
        sqlx::query("UPDATE autonomous_canonical_checkpoints SET invalidated=true,updated_at=now() WHERE network=$1 AND factory_contract=$2")
            .bind(network).bind(normalize_key_address(factory)).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn require_autonomous_canonical_checkpoint(
        &self,
        network: &str,
        factory: &str,
        through: u64,
    ) -> DbResult<()> {
        match self
            .autonomous_canonical_checkpoint(network, factory)
            .await?
        {
            Some(c) if !c.invalidated && c.block_number >= through => Ok(()),
            _ => Err(unavailable()),
        }
    }

    /// Compare-and-swap with the read cursor and atomically certify the new cursor.
    /// None is allowed only for initial independently replayed history.
    pub async fn commit_autonomous_canonical_checkpoint(
        &self,
        network: &str,
        factory: &str,
        expected_cursor: Option<u64>,
        expected: Option<&AutonomousCanonicalCheckpoint>,
        block: &chain_base::BaseBlockIdentity,
        last_log_key: Option<&str>,
    ) -> DbResult<()> {
        self.commit_autonomous_checkpoint_inner(
            network,
            factory,
            expected_cursor,
            expected,
            block,
            last_log_key,
            None,
        )
        .await
    }

    /// Certify one completely replayed legacy snapshot. Never persist a prefix.
    /// The final short transaction freezes writes while checking the same exact
    /// event tuples replayed by the worker, then uses the existing cursor CAS.
    pub async fn commit_autonomous_bootstrap_checkpoint(
        &self,
        network: &str,
        factory: &str,
        cursor: u64,
        block: &chain_base::BaseBlockIdentity,
        events: &[AutonomousBountyEvent],
    ) -> DbResult<()> {
        if block.number != cursor {
            return Err(unavailable());
        }
        self.commit_autonomous_checkpoint_inner(
            network,
            factory,
            Some(cursor),
            None,
            block,
            None,
            Some(events),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn commit_autonomous_checkpoint_inner(
        &self,
        network: &str,
        factory: &str,
        expected_cursor: Option<u64>,
        expected: Option<&AutonomousCanonicalCheckpoint>,
        block: &chain_base::BaseBlockIdentity,
        last_log_key: Option<&str>,
        bootstrap_events: Option<&[AutonomousBountyEvent]>,
    ) -> DbResult<()> {
        if block.hash.len() != 66
            || !block.hash.starts_with("0x")
            || hex::decode(&block.hash[2..]).is_err()
            || expected_cursor.is_some_and(|n| n > block.number)
            || expected.is_some_and(|c| {
                c.invalidated
                    || (c.block_number == block.number && c.block_hash != block.hash.to_lowercase())
            })
        {
            return Err(unavailable());
        }
        let factory = normalize_key_address(factory);
        let mut tx = self.pool.begin().await?;
        if bootstrap_events.is_some() {
            // No network I/O in this transaction. Bound waits independently of
            // the worker's remaining absolute observation deadline.
            sqlx::query("SET LOCAL lock_timeout = '2s'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("SET LOCAL statement_timeout = '5s'")
                .execute(&mut *tx)
                .await?;
            sqlx::query("LOCK TABLE autonomous_bounty_events IN SHARE MODE")
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("autonomous-canonical:{network}:{factory}"))
            .execute(&mut *tx)
            .await?;
        let current: Option<i64>=sqlx::query_scalar("SELECT last_scanned_block FROM base_log_cursors WHERE network=$1 AND escrow_contract=$2 FOR UPDATE")
            .bind(network).bind(&factory).fetch_optional(&mut *tx).await?;
        let current = current.map(u64_from_i64).transpose()?;
        let row=sqlx::query("SELECT block_number,block_hash,invalidated FROM autonomous_canonical_checkpoints WHERE network=$1 AND factory_contract=$2 FOR UPDATE")
            .bind(network).bind(&factory).fetch_optional(&mut *tx).await?;
        let actual = row
            .map(|r| -> DbResult<_> {
                Ok(AutonomousCanonicalCheckpoint {
                    block_number: u64_from_i64(r.try_get("block_number")?)?,
                    block_hash: r.try_get("block_hash")?,
                    invalidated: r.try_get("invalidated")?,
                })
            })
            .transpose()?;
        if current != expected_cursor || actual.as_ref() != expected {
            // A rolling deployment can finish the exact same bounded scan twice.
            // The losing poll must re-read state and headers, not report success
            // or repeat downstream work. Broader divergence still needs review.
            if bootstrap_events.is_none()
                && current == Some(block.number)
                && actual.as_ref().is_some_and(|checkpoint| {
                    !checkpoint.invalidated
                        && checkpoint.block_number == block.number
                        && checkpoint.block_hash == block.hash.to_lowercase()
                })
            {
                return Err(DbError::AutonomousCheckpointAlreadyCommitted);
            }
            return Err(unavailable());
        }
        if let Some(events) = bootstrap_events {
            let current = Self::list_autonomous_bounty_events_for_factory_on(
                &mut *tx, network, &factory, false,
            )
            .await?;
            if !same_event_payloads(events, &current)? {
                return Err(unavailable());
            }
        }
        sqlx::query("INSERT INTO autonomous_canonical_checkpoints(network,factory_contract,block_number,block_hash) VALUES($1,$2,$3,$4) ON CONFLICT(network,factory_contract) DO UPDATE SET block_number=EXCLUDED.block_number,block_hash=EXCLUDED.block_hash,updated_at=now()")
            .bind(network).bind(&factory).bind(i64_from_u64(block.number)?).bind(block.hash.to_lowercase()).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO base_log_cursors(network,escrow_contract,last_scanned_block,last_log_key,updated_at) VALUES($1,$2,$3,$4,now()) ON CONFLICT(network,escrow_contract) DO UPDATE SET last_scanned_block=EXCLUDED.last_scanned_block,last_log_key=COALESCE(EXCLUDED.last_log_key,base_log_cursors.last_log_key),updated_at=now()")
            .bind(network).bind(&factory).bind(i64_from_u64(block.number)?).bind(last_log_key).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires isolated local PostgreSQL"]
    async fn bootstrap_commit_rejects_changed_and_concurrently_inserted_history() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("bootstrap_cas_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let store = PostgresStore::from_pool(PgPool::connect_with(options.clone()).await.unwrap());
        store.migrate().await.unwrap();
        let writer = PgPool::connect_with(options).await.unwrap();
        let factory = format!("0x{}", "71".repeat(20));
        let event = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: "bootstrap-original".into(),
            tx_hash: format!("0x{}", "72".repeat(32)),
            block_number: 50,
            log_index: 0,
            contract_address: factory.clone(),
            bounty_id: format!("0x{}", "73".repeat(32)),
            kind: AutonomousBountyEventKind::CanonicalBountyCreated,
            data: serde_json::json!({"bounty_contract":format!("0x{}", "74".repeat(20))}),
            occurred_at: Utc::now(),
        };
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();
        store
            .upsert_base_log_cursor("base-sepolia", &factory, 100, Some(&event.log_key))
            .await
            .unwrap();
        let events = store
            .list_autonomous_bounty_events_for_factory("base-sepolia", &factory, false)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        let block = chain_base::BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "75".repeat(32)),
            timestamp: 1,
        };
        let original = store
            .get_base_log_cursor("base-sepolia", &factory)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE autonomous_bounty_events SET data=data || '{\"changed\":true}'::jsonb WHERE log_key=$1")
            .bind(&event.log_key).execute(&writer).await.unwrap();
        assert!(store
            .commit_autonomous_bootstrap_checkpoint("base-sepolia", &factory, 100, &block, &events)
            .await
            .is_err());
        assert!(store
            .autonomous_canonical_checkpoint("base-sepolia", &factory)
            .await
            .unwrap()
            .is_none());
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();

        sqlx::query("DELETE FROM autonomous_bounty_events WHERE log_key=$1")
            .bind(&event.log_key)
            .execute(&writer)
            .await
            .unwrap();
        assert!(store
            .commit_autonomous_bootstrap_checkpoint("base-sepolia", &factory, 100, &block, &events)
            .await
            .is_err());
        assert!(store
            .autonomous_canonical_checkpoint("base-sepolia", &factory)
            .await
            .unwrap()
            .is_none());
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();

        // A writer which starts before the final SHARE lock must finish before
        // the snapshot comparison. Its newly committed row must not be missed.
        let mut transaction = writer.begin().await.unwrap();
        sqlx::query("INSERT INTO autonomous_bounty_events(id,log_key,network,tx_hash,block_number,log_index,contract_address,bounty_id,kind,data,occurred_at) SELECT $1,'bootstrap-new',network,tx_hash,51,1,contract_address,bounty_id,kind,data,occurred_at FROM autonomous_bounty_events WHERE log_key=$2")
            .bind(Uuid::new_v4()).bind(&event.log_key).execute(&mut *transaction).await.unwrap();
        let task_store = store.clone();
        let task_factory = factory.clone();
        let task_block = block.clone();
        let task_events = events.clone();
        let task = tokio::spawn(async move {
            task_store
                .commit_autonomous_bootstrap_checkpoint(
                    "base-sepolia",
                    &task_factory,
                    100,
                    &task_block,
                    &task_events,
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !task.is_finished(),
            "checkpoint must wait for concurrent writer"
        );
        transaction.commit().await.unwrap();
        assert!(task.await.unwrap().is_err());
        assert!(store
            .autonomous_canonical_checkpoint("base-sepolia", &factory)
            .await
            .unwrap()
            .is_none());
        let unchanged = store
            .get_base_log_cursor("base-sepolia", &factory)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.last_scanned_block, original.last_scanned_block);
        assert_eq!(unchanged.last_log_key, original.last_log_key);
        assert_eq!(unchanged.updated_at, original.updated_at);

        // Merely changing observation metadata does not change canonical tuples.
        sqlx::query("DELETE FROM autonomous_bounty_events WHERE log_key='bootstrap-new'")
            .execute(&writer)
            .await
            .unwrap();
        sqlx::query("UPDATE autonomous_bounty_events SET occurred_at=occurred_at + interval '1 hour' WHERE log_key=$1").bind(&event.log_key).execute(&writer).await.unwrap();
        store
            .commit_autonomous_bootstrap_checkpoint("base-sepolia", &factory, 100, &block, &events)
            .await
            .unwrap();
        assert_eq!(
            store
                .autonomous_canonical_checkpoint("base-sepolia", &factory)
                .await
                .unwrap()
                .unwrap()
                .block_hash,
            block.hash
        );
        store.pool.close().await;
        writer.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }

    #[tokio::test]
    #[ignore = "requires isolated local PostgreSQL"]
    async fn canonical_checkpoint_is_atomic_monotonic_and_cannot_clear_quarantine() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").expect("isolated database");
        crate::test_support::assert_isolated_security_database(&url);
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("canonical_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let first = PostgresStore::from_pool(PgPool::connect_with(options.clone()).await.unwrap());
        first.migrate().await.unwrap();
        let second = PostgresStore::from_pool(PgPool::connect_with(options).await.unwrap());
        let factory = "0x1111111111111111111111111111111111111111";
        let block = chain_base::BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "ab".repeat(32)),
            timestamp: 1,
        };
        assert!(first
            .require_autonomous_canonical_checkpoint("base-mainnet", factory, 100)
            .await
            .is_err());
        first
            .commit_autonomous_canonical_checkpoint(
                "base-mainnet",
                factory,
                None,
                None,
                &block,
                None,
            )
            .await
            .unwrap();
        let previous = second
            .autonomous_canonical_checkpoint("base-mainnet", factory)
            .await
            .unwrap()
            .unwrap();
        second
            .require_autonomous_canonical_checkpoint("base-mainnet", factory, 100)
            .await
            .unwrap();
        assert!(second
            .require_autonomous_canonical_checkpoint("base-mainnet", factory, 101)
            .await
            .is_err());
        let mut next = block.clone();
        next.number = 101;
        assert!(second
            .commit_autonomous_canonical_checkpoint(
                "base-mainnet",
                factory,
                None,
                None,
                &next,
                None
            )
            .await
            .is_err());
        let changed_hash = chain_base::BaseBlockIdentity {
            hash: format!("0x{}", "cd".repeat(32)),
            ..block.clone()
        };
        assert!(second
            .commit_autonomous_canonical_checkpoint(
                "base-mainnet",
                factory,
                Some(100),
                Some(&previous),
                &changed_hash,
                None
            )
            .await
            .is_err());
        first
            .invalidate_autonomous_canonical_checkpoint("base-mainnet", factory)
            .await
            .unwrap();
        assert!(second
            .require_autonomous_canonical_checkpoint("base-mainnet", factory, 100)
            .await
            .is_err());
        assert!(second
            .commit_autonomous_canonical_checkpoint(
                "base-mainnet",
                factory,
                Some(100),
                Some(&previous),
                &next,
                None
            )
            .await
            .is_err());
        assert_eq!(
            second
                .get_base_log_cursor("base-mainnet", factory)
                .await
                .unwrap()
                .unwrap()
                .last_scanned_block,
            100
        );
        first.pool.close().await;
        second.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
    }
}
