//! One platform-wide daily gas allowance. Unused allowance never carries forward.
use super::*;
use chain_base::{ChainBaseError, RelayGasBudget};
use std::{sync::Arc, time::Duration};

pub const DEFAULT_GAS_BUDGET_MICRO_USDC: u64 = 1_000_000;
const PLATFORM_BUDGET: &str = "platform";

#[derive(Debug, Clone, Serialize)]
pub struct GasBudgetStatus {
    pub limit_micro_usdc: u64,
    pub reserved_micro_usdc: u64,
    pub remaining_micro_usdc: u64,
    pub resets_at: DateTime<Utc>,
    pub carries_forward: bool,
}

impl PostgresStore {
    pub fn gas_sponsor_budget(&self) -> Arc<dyn RelayGasBudget> {
        Arc::new(PostgresGasBudget {
            store: self.clone(),
        })
    }

    pub async fn gas_sponsor_budget_status(&self) -> DbResult<GasBudgetStatus> {
        let row = sqlx::query("SELECT COALESCE((SELECT reserved_micro_usdc FROM gas_sponsorship_daily_budget WHERE budget_key = $1 AND budget_day = (clock_timestamp() AT TIME ZONE 'UTC')::date), 0)::bigint AS reserved, ((clock_timestamp() AT TIME ZONE 'UTC')::date + 1)::timestamp AT TIME ZONE 'UTC' AS resets_at")
            .bind(PLATFORM_BUDGET).fetch_one(&self.pool).await?;
        let reserved = u64::try_from(row.try_get::<i64, _>("reserved")?).unwrap_or(u64::MAX);
        Ok(GasBudgetStatus {
            limit_micro_usdc: DEFAULT_GAS_BUDGET_MICRO_USDC,
            reserved_micro_usdc: reserved,
            remaining_micro_usdc: DEFAULT_GAS_BUDGET_MICRO_USDC.saturating_sub(reserved),
            resets_at: row.try_get("resets_at")?,
            carries_forward: false,
        })
    }

    async fn reserve_gas_budget(&self, key: &str, amount: u64, limit: u64) -> DbResult<bool> {
        // One atomic upsert serializes competing API/worker instances. The UTC
        // calendar date is chosen by PostgreSQL, never supplied by an API caller.
        let row = sqlx::query("INSERT INTO gas_sponsorship_daily_budget (budget_key, budget_day, reserved_micro_usdc, limit_micro_usdc) SELECT $1, (clock_timestamp() AT TIME ZONE 'UTC')::date, $2, $3 WHERE $2 > 0 AND $2 <= $3 ON CONFLICT (budget_key, budget_day) DO UPDATE SET reserved_micro_usdc = gas_sponsorship_daily_budget.reserved_micro_usdc + EXCLUDED.reserved_micro_usdc, limit_micro_usdc = LEAST(gas_sponsorship_daily_budget.limit_micro_usdc, EXCLUDED.limit_micro_usdc), updated_at = clock_timestamp() WHERE gas_sponsorship_daily_budget.reserved_micro_usdc + EXCLUDED.reserved_micro_usdc <= LEAST(gas_sponsorship_daily_budget.limit_micro_usdc, EXCLUDED.limit_micro_usdc) RETURNING reserved_micro_usdc")
            .bind(key).bind(i64_from_u64(amount)?).bind(i64_from_u64(limit)?).fetch_optional(&self.pool).await?;
        Ok(row.is_some())
    }
}

struct PostgresGasBudget {
    store: PostgresStore,
}
fn unavailable() -> ChainBaseError {
    ChainBaseError::RelayerProvider("gas sponsorship budget or fresh ETH/USDC quote unavailable; customer gas is never a fallback".into())
}

// Prices are decimal fixed point. Round upward, including beyond six places.
fn price_micro_usdc(value: &str) -> Option<u128> {
    if value.len() > 32 {
        return None;
    }
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|c| c.is_ascii_digit())
        || !fraction.bytes().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let mut decimals = fraction.chars().take(6).collect::<String>();
    while decimals.len() < 6 {
        decimals.push('0');
    }
    let price = whole
        .parse::<u128>()
        .ok()?
        .checked_mul(1_000_000)?
        .checked_add(decimals.parse::<u128>().ok()?)?
        .checked_add(u128::from(fraction.bytes().skip(6).any(|c| c != b'0')))?;
    (price > 0 && price <= 1_000_000_000_000).then_some(price)
}

fn reserve_micro_usdc(wei: u128, price: u128) -> Option<u64> {
    // 25% FX headroom, in addition to bounded execution gas and padded L1 fees.
    const DENOMINATOR: u128 = 4_000_000_000_000_000_000;
    u64::try_from(
        wei.checked_mul(price)?
            .checked_mul(5)?
            .checked_add(DENOMINATOR - 1)?
            / DENOMINATOR,
    )
    .ok()
}

/// Fetch a fresh conservative ETH/USDC quote without reserving funds. The caller
/// must reserve this amount atomically with its durable authorization.
pub async fn quote_gas_budget_micro_usdc(
    chain_id: u64,
    maximum_fee_wei: u128,
) -> Result<u64, ChainBaseError> {
    if ![8453, 84532].contains(&chain_id) {
        return Err(unavailable());
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable())?;
    let response = client
        .get("https://api.coinbase.com/v2/prices/ETH-USDC/spot")
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success()
        || response
            .headers()
            .get("age")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|age| age > 60)
    {
        return Err(unavailable());
    }
    let quote: serde_json::Value = response.json().await.map_err(|_| unavailable())?;
    if quote["data"]["base"] != "ETH" || quote["data"]["currency"] != "USDC" {
        return Err(unavailable());
    }
    let price = quote["data"]["amount"]
        .as_str()
        .and_then(price_micro_usdc)
        .ok_or_else(unavailable)?;
    let amount = reserve_micro_usdc(maximum_fee_wei, price)
        .filter(|amount| *amount > 0)
        .ok_or_else(unavailable)?;
    Ok(amount)
}

#[async_trait::async_trait]
impl RelayGasBudget for PostgresGasBudget {
    async fn ensure_sender_nonce_available(
        &self,
        chain_id: u64,
        sender: &str,
        nonce: u64,
    ) -> Result<(), ChainBaseError> {
        let network = match chain_id {
            8453 => "base-mainnet",
            84532 => "base-sepolia",
            _ => return Err(unavailable()),
        };
        let nonce = i64::try_from(nonce).map_err(|_| unavailable())?;
        let row=sqlx::query("SELECT EXISTS(SELECT 1 FROM nonce_retirement_fences WHERE network=$1 AND relayer_address=$2 UNION ALL SELECT 1 FROM relay_transaction_outbox WHERE network=$1 AND relayer_address=$2 AND nonce=$3 UNION ALL SELECT 1 FROM posting_relay_transactions WHERE network=$1 AND relayer_address=$2 AND nonce=$3) AS occupied").bind(network).bind(sender.to_ascii_lowercase()).bind(nonce).fetch_one(&self.store.pool).await.map_err(|_|unavailable())?;
        if row
            .try_get::<bool, _>("occupied")
            .map_err(|_| unavailable())?
        {
            return Err(ChainBaseError::RelayerProvider("sender nonce belongs to a saved transaction; reconcile or retransmit that exact transaction first".into()));
        }
        Ok(())
    }
    async fn reserve(&self, chain_id: u64, maximum_fee_wei: u128) -> Result<(), ChainBaseError> {
        let amount = quote_gas_budget_micro_usdc(chain_id, maximum_fee_wei).await?;
        if !self
            .store
            .reserve_gas_budget(PLATFORM_BUDGET, amount, DEFAULT_GAS_BUDGET_MICRO_USDC)
            .await
            .map_err(|_| unavailable())?
        {
            return Err(ChainBaseError::RelayerProvider("daily 1 USDC gas sponsorship allowance reached; resets at 00:00 UTC, without carryover".into()));
        }
        // Retain reservations on an uncertain broadcast. Never reclaim assumed savings.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fx_and_fee_round_up_without_float_or_carryover() {
        assert_eq!(price_micro_usdc("2674.565"), Some(2_674_565_000));
        assert_eq!(price_micro_usdc("1.0000001"), Some(1_000_001));
        for invalid in ["0", "-1", "1e3", "NaN", "1000001", "1.2.3"] {
            assert!(price_micro_usdc(invalid).is_none());
        }
        assert_eq!(reserve_micro_usdc(1, 3_000_000_000), Some(1));
        assert_eq!(
            reserve_micro_usdc(1_000_000_000_000, 3_000_000_000),
            Some(3750)
        );
        assert!(reserve_micro_usdc(u128::MAX, u128::MAX).is_none());
    }
    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn daily_budget_is_atomic_shared_and_has_no_carryover() {
        let store =
            PostgresStore::connect(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap();
        store.migrate().await.unwrap();
        let key = format!("gas-test-{}", Uuid::new_v4());
        // Yesterday's unused 0.75 USDC cannot augment today's 1 USDC.
        sqlx::query("INSERT INTO gas_sponsorship_daily_budget VALUES ($1, (clock_timestamp() AT TIME ZONE 'UTC')::date - 1, 250000, 1000000, now())").bind(&key).execute(&store.pool).await.unwrap();
        let (a, b) = tokio::join!(
            store.reserve_gas_budget(&key, 600_000, 1_000_000),
            store.reserve_gas_budget(&key, 600_000, 1_000_000)
        );
        assert_ne!(a.unwrap(), b.unwrap());
        assert!(store
            .reserve_gas_budget(&key, 400_000, 1_000_000)
            .await
            .unwrap());
        assert!(!store.reserve_gas_budget(&key, 1, 1_000_000).await.unwrap());
        assert!(!store.reserve_gas_budget(&key, 0, 1_000_000).await.unwrap());
        // Moving the completed prior day out of today's slot resets to exactly 1 USDC.
        sqlx::query("UPDATE gas_sponsorship_daily_budget SET budget_day = budget_day - 2 WHERE budget_key = $1 AND budget_day = (clock_timestamp() AT TIME ZONE 'UTC')::date").bind(&key).execute(&store.pool).await.unwrap();
        assert!(store
            .reserve_gas_budget(&key, 1_000_000, 1_000_000)
            .await
            .unwrap());
        assert!(!store.reserve_gas_budget(&key, 1, 1_000_000).await.unwrap());
    }
}
