//! Canonical checkpoints are separate from immutable decoded event history.
//! Divergence contains reads; this module never deletes events or rewinds a cursor.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutonomousCanonicalCheckpoint {
    pub block_number: u64,
    pub block_hash: String,
    pub invalidated: bool,
}

fn unavailable() -> DbError {
    DbError::InvalidEnum("autonomous canonical history requires reconciliation".into())
}

impl PostgresStore {
    pub async fn autonomous_canonical_checkpoint(
        &self,
        network: &str,
        factory: &str,
    ) -> DbResult<Option<AutonomousCanonicalCheckpoint>> {
        let row = sqlx::query("SELECT block_number,block_hash,invalidated FROM autonomous_canonical_checkpoints WHERE network=$1 AND factory_contract=$2")
            .bind(network).bind(normalize_key_address(factory)).fetch_optional(&self.pool).await?;
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
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("autonomous-canonical:{network}:{factory}"))
            .execute(&mut *tx)
            .await?;
        let current: Option<i64>=sqlx::query_scalar("SELECT last_scanned_block FROM base_log_cursors WHERE network=$1 AND escrow_contract=$2 FOR UPDATE")
            .bind(network).bind(&factory).fetch_optional(&mut *tx).await?;
        if current.map(u64_from_i64).transpose()? != expected_cursor {
            return Err(unavailable());
        }
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
        if actual.as_ref() != expected {
            return Err(unavailable());
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
