use crate::{DbError, DbResult, PostgresStore};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;

pub const PLATFORM_PAYOUT_PROOF_MAX_ROWS: usize = 5_000;

// Parameter positions are selected by trusted callers, never request text.
pub(crate) fn projection_sql(network: u8, start: u8, end: u8, excluded: u8) -> String {
    include_str!("platform_payouts.sql")
        .replace("$NETWORK", &format!("${network}"))
        .replace("$START", &format!("${start}"))
        .replace("$END", &format!("${end}"))
        .replace("$EXCLUDED", &format!("${excluded}"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformPayoutProofRow {
    pub protocol: String,
    pub network: String,
    pub factory_contract: Option<String>,
    pub contract_address: String,
    pub bounty_id: String,
    pub tx_hash: String,
    pub block_number: i64,
    pub log_index: i64,
    pub kind: String,
    pub occurred_at: DateTime<Utc>,
    pub is_settlement: bool,
    pub solver_base_units: String,
    pub verifier_base_units: String,
    pub keeper_base_units: String,
    pub bonus_base_units: String,
    pub total_base_units: String,
}

pub(crate) async fn read_rows(
    connection: &mut sqlx::PgConnection,
    network: &str,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    excluded_contracts: &[String],
) -> DbResult<Vec<PlatformPayoutProofRow>> {
    let query = format!(
        "WITH payouts AS ({}) SELECT protocol, network, factory_contract, \
             contract_address, bounty_id, tx_hash, block_number, log_index, kind, occurred_at, \
             settled, solver_amount::text AS solver, verifier_amount::text AS verifier, \
             keeper_amount::text AS keeper, bonus_amount::text AS bonus, \
             (solver_amount + verifier_amount + keeper_amount + bonus_amount)::text AS total \
             FROM payouts ORDER BY protocol, block_number, log_index, tx_hash, contract_address \
             LIMIT {}",
        projection_sql(1, 2, 3, 4),
        PLATFORM_PAYOUT_PROOF_MAX_ROWS + 1,
    );
    let rows = sqlx::query(&query)
        .bind(network)
        .bind(started_at)
        .bind(ended_at)
        .bind(excluded_contracts)
        .fetch_all(connection)
        .await?;
    rows.into_iter()
        .map(|row| {
            Ok(PlatformPayoutProofRow {
                protocol: row.try_get("protocol")?,
                network: row.try_get("network")?,
                factory_contract: row.try_get("factory_contract")?,
                contract_address: row.try_get("contract_address")?,
                bounty_id: row.try_get("bounty_id")?,
                tx_hash: row.try_get("tx_hash")?,
                block_number: row.try_get("block_number")?,
                log_index: row.try_get("log_index")?,
                kind: row.try_get("kind")?,
                occurred_at: row.try_get("occurred_at")?,
                is_settlement: row.try_get("settled")?,
                solver_base_units: row.try_get("solver")?,
                verifier_base_units: row.try_get("verifier")?,
                keeper_base_units: row.try_get("keeper")?,
                bonus_base_units: row.try_get("bonus")?,
                total_base_units: row.try_get("total")?,
            })
        })
        .collect()
}

pub fn platform_payout_snapshot_hash(
    rows: &[PlatformPayoutProofRow],
    network: &str,
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    excluded_contracts: &[String],
) -> DbResult<String> {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(&serde_json::json!({
        "version": "agent-bounties/historical-payout-selection-v1",
        "network": network, "started_at": started_at, "ended_at": ended_at,
        "excluded_contracts": excluded_contracts, "rows": rows,
    }))?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
}

impl PostgresStore {
    pub async fn platform_payout_proof_rows(
        &self,
        network: &str,
        started_at: DateTime<Utc>,
        ended_at: DateTime<Utc>,
        excluded_contracts: &[String],
    ) -> DbResult<Vec<PlatformPayoutProofRow>> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SET LOCAL statement_timeout = '5s'")
            .execute(&mut *transaction)
            .await?;
        let rows = read_rows(
            &mut transaction,
            network,
            started_at,
            ended_at,
            excluded_contracts,
        )
        .await?;
        transaction.commit().await?;
        if rows.len() > PLATFORM_PAYOUT_PROOF_MAX_ROWS {
            return Err(DbError::PayoutProofCapacity);
        }
        Ok(rows)
    }
}
