//! Explicit operator revalidation of an existing, never-attempted timeout envelope.
//! This is not replay authority. The caller must independently verify chain state,
//! pending journals/providers, simulation and signed bytes before authorization and
//! again before consumption. Only a committed `true` result permits one first send.
use super::*;
use chain_base::timeout_recovery::TimeoutRelayScope;
use chrono::NaiveDate;

#[derive(Clone)]
pub struct TimeoutRevalidationRequest {
    pub scope: TimeoutRelayScope,
    pub expected_tx_hash: String,
    pub lease_token: Uuid,
    /// Derived by the authenticated operator handler, never from request JSON.
    pub operator_identity: String,
    /// Bounded, sanitized evidence. No credentials, raw URLs or signed bytes.
    pub observations: serde_json::Value,
    pub observed_at: DateTime<Utc>,
    /// Fresh quote's conservative total for this exact envelope, not a delta.
    pub required_budget_micro_usdc: u64,
    pub validity_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeoutRevalidationAuthorization {
    pub id: Uuid,
    pub operation_id: Uuid,
    pub tx_hash: String,
    pub expires_at: DateTime<Utc>,
    pub budget_day: NaiveDate,
    pub reserved_micro_usdc: u64,
}

fn denied() -> DbError {
    DbError::X402RelayQuotaExceeded("timeout operator revalidation denied".into())
}
fn valid_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lock order: network lease, exact outbox, receipt, global daily budget.
/// A network lease prevents normal cooperating writers from allocating another
/// nonce while these checks run. The unique journal constraints remain enabled.
pub(super) async fn lock_unsent(
    tx: &mut Transaction<'_, Postgres>,
    scope: &TimeoutRelayScope,
    hash: &str,
    lease: Uuid,
    margin_seconds: u64,
) -> DbResult<PgRow> {
    let id = scope.operation_id().map_err(|_| denied())?;
    if !valid_hash(hash) {
        return Err(denied());
    }
    let lease_row = sqlx::query("SELECT lease_token FROM x402_relayer_leases WHERE network=$1 AND lease_token=$2 AND lease_expires_at>clock_timestamp()+make_interval(secs=>$3) FOR UPDATE")
        .bind(&scope.network).bind(lease).bind(i64_from_u64(margin_seconds)?).fetch_optional(&mut **tx).await?;
    if lease_row.is_none() {
        return Err(denied());
    }
    let row = sqlx::query("SELECT network,relayer_address,nonce,tx_hash,transaction_metadata,send_attempts FROM relay_transaction_outbox WHERE operation_id=$1 FOR UPDATE")
        .bind(id).fetch_optional(&mut **tx).await?.ok_or_else(denied)?;
    let metadata: serde_json::Value = row.try_get("transaction_metadata")?;
    let relayer: String = row.try_get("relayer_address")?;
    let nonce: i64 = row.try_get("nonce")?;
    if row.try_get::<String, _>("network")? != scope.network
        || row.try_get::<String, _>("tx_hash")? != hash
        || row.try_get::<i32, _>("send_attempts")? != 0
        || metadata.get("timeout_scope") != Some(&serde_json::to_value(scope)?)
        || metadata.get("tx_hash").and_then(|v| v.as_str()) != Some(hash)
        || metadata.get("relayer").and_then(|v| v.as_str()) != Some(relayer.as_str())
    {
        return Err(denied());
    }
    // Historical lower nonces remain valid audit records. Any other current or
    // later occupied nonce, or active posting journal, requires reconciliation.
    let conflict: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM relay_transaction_outbox WHERE network=$1 AND relayer_address=$2 AND nonce >= $3 AND operation_id<>$4 UNION ALL SELECT 1 FROM posting_relay_transactions p JOIN x402_relay_attempts a ON a.id=p.relay_id WHERE p.network=$1 AND p.relayer_address=$2 AND (p.nonce >= $3 OR a.status NOT IN ('confirmed','failed')) UNION ALL SELECT 1 FROM x402_relay_attempts WHERE network=$1 AND relayer_address=$2 AND (status IN ('prepared','relaying','broadcast') OR (status='failed' AND retryable)) UNION ALL SELECT 1 FROM open_competition_entrant_relays WHERE network=$1 AND relayer_address=$2 AND (status IN ('prepared','relaying','broadcast') OR (status='failed' AND retryable)) UNION ALL SELECT 1 FROM sponsored_actions WHERE network=$1 AND status<>'confirmed')")
        .bind(&scope.network).bind(&relayer).bind(nonce).bind(id).fetch_one(&mut **tx).await?;
    let published: bool = sqlx::query_scalar("SELECT count(*) > 0 AND bool_and(COALESCE(contract_address=$3 AND bounty_id=$4 AND data->>'terms_hash'=$5,false)) FROM autonomous_bounty_events WHERE network=$1 AND kind='canonical_bounty_created' AND data->>'bounty_contract'=$2")
        .bind(&scope.network).bind(&scope.bounty_contract).bind(&scope.factory_contract).bind(&scope.bounty_id).bind(&scope.terms_hash).fetch_one(&mut **tx).await?;
    if conflict || !published {
        return Err(denied());
    }
    Ok(row)
}

impl PostgresStore {
    pub async fn authorize_timeout_revalidation(
        &self,
        request: &TimeoutRevalidationRequest,
    ) -> DbResult<TimeoutRevalidationAuthorization> {
        if !(1..=120).contains(&request.validity_seconds)
            || !(1..=DEFAULT_GAS_BUDGET_MICRO_USDC).contains(&request.required_budget_micro_usdc)
            || request.operator_identity.is_empty()
            || request.operator_identity.len() > 128
            || !request
                .operator_identity
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
            || request
                .observations
                .as_object()
                .is_none_or(|o| o.is_empty())
            || request.observations.to_string().len() > 16_384
        {
            return Err(denied());
        }
        let scope = &request.scope;
        let operation = scope.operation_id().map_err(|_| denied())?;
        let mut tx = self.pool.begin().await?;
        let row = lock_unsent(
            &mut tx,
            scope,
            &request.expected_tx_hash,
            request.lease_token,
            3,
        )
        .await?;
        let clock: (DateTime<Utc>, NaiveDate) =
            sqlx::query_as("SELECT clock_timestamp(),(clock_timestamp() AT TIME ZONE 'UTC')::date")
                .fetch_one(&mut *tx)
                .await?;
        if request.observed_at > clock.0
            || clock.0 - request.observed_at > chrono::Duration::seconds(30)
        {
            return Err(denied());
        }
        let previous: Option<i64> = sqlx::query_scalar("SELECT reserved_micro_usdc FROM timeout_revalidation_gas_receipts WHERE operation_id=$1 AND tx_hash=$2 AND budget_day=$3 FOR UPDATE")
            .bind(operation).bind(&request.expected_tx_hash).bind(clock.1).fetch_optional(&mut *tx).await?;
        let required = i64_from_u64(request.required_budget_micro_usdc)?;
        let reserved = previous.unwrap_or(0).max(required);
        let delta = reserved - previous.unwrap_or(0);
        if delta > 0 {
            let budget = sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES ('platform',$1,$2,$3) ON CONFLICT(budget_key,budget_day) DO UPDATE SET reserved_micro_usdc=gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc,limit_micro_usdc=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc),updated_at=clock_timestamp() WHERE gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc<=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc) RETURNING reserved_micro_usdc")
                .bind(clock.1).bind(delta).bind(i64_from_u64(DEFAULT_GAS_BUDGET_MICRO_USDC)?).fetch_optional(&mut *tx).await?;
            if budget.is_none() {
                return Err(denied());
            }
            sqlx::query("INSERT INTO timeout_revalidation_gas_receipts(operation_id,tx_hash,budget_day,reserved_micro_usdc) VALUES ($1,$2,$3,$4) ON CONFLICT(operation_id,tx_hash,budget_day) DO UPDATE SET reserved_micro_usdc=excluded.reserved_micro_usdc,updated_at=clock_timestamp()")
                .bind(operation).bind(&request.expected_tx_hash).bind(clock.1).bind(reserved).execute(&mut *tx).await?;
        }
        let budget_backed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM gas_sponsorship_daily_budget WHERE budget_key='platform' AND budget_day=$1 AND reserved_micro_usdc >= $2 AND reserved_micro_usdc<=limit_micro_usdc AND limit_micro_usdc<=$3)")
            .bind(clock.1).bind(reserved).bind(i64_from_u64(DEFAULT_GAS_BUDGET_MICRO_USDC)?).fetch_one(&mut *tx).await?;
        if !budget_backed {
            return Err(denied());
        }
        // Old unused authorizations cannot race a newer authorization; audit
        // records are retained and only a terminal event is appended.
        sqlx::query("INSERT INTO timeout_revalidation_events(authorization_id,status) SELECT id,CASE WHEN expires_at<=clock_timestamp() THEN 'expired' ELSE 'superseded' END FROM timeout_revalidation_authorizations a WHERE operation_id=$1 AND NOT EXISTS(SELECT 1 FROM timeout_revalidation_events e WHERE e.authorization_id=a.id)")
            .bind(operation).execute(&mut *tx).await?;
        let id = Uuid::new_v4();
        let expires_at: DateTime<Utc> = sqlx::query_scalar("INSERT INTO timeout_revalidation_authorizations(id,operation_id,network,tx_hash,relayer_address,nonce,timeout_scope,lease_token,operator_identity,observations,observed_at,budget_day,required_budget_micro_usdc,created_at,expires_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$15,LEAST($15::timestamptz+make_interval(secs=>$14),l.lease_expires_at-interval '2 seconds',(($12::date+1)::timestamp AT TIME ZONE 'UTC')) FROM x402_relayer_leases l WHERE l.network=$3 AND l.lease_token=$8 AND l.lease_expires_at>clock_timestamp()+interval '3 seconds' AND $12=(clock_timestamp() AT TIME ZONE 'UTC')::date AND $11>=clock_timestamp()-interval '30 seconds' RETURNING expires_at")
            .bind(id).bind(operation).bind(&scope.network).bind(&request.expected_tx_hash).bind(row.try_get::<String,_>("relayer_address")?).bind(row.try_get::<i64,_>("nonce")?).bind(serde_json::to_value(scope)?).bind(request.lease_token).bind(&request.operator_identity).bind(&request.observations).bind(request.observed_at).bind(clock.1).bind(required).bind(i64_from_u64(request.validity_seconds)?).bind(clock.0).fetch_optional(&mut *tx).await?.ok_or_else(denied)?;
        tx.commit().await?;
        Ok(TimeoutRevalidationAuthorization {
            id,
            operation_id: operation,
            tx_hash: request.expected_tx_hash.clone(),
            expires_at,
            budget_day: clock.1,
            reserved_micro_usdc: u64_from_i64(reserved)?,
        })
    }

    /// Never retry an uncertain commit as permission to broadcast. Reloading the
    /// outbox may prove consumption, but can never authorize a second send.
    // Keep every authority binding explicit at this narrow atomic boundary.
    #[allow(clippy::too_many_arguments)]
    pub async fn consume_timeout_revalidation(
        &self,
        scope: &TimeoutRelayScope,
        expected_tx_hash: &str,
        authorization_id: Uuid,
        lease_token: Uuid,
        send_seconds: u64,
        final_observations: &serde_json::Value,
        observed_at: DateTime<Utc>,
        final_required_budget_micro_usdc: u64,
    ) -> DbResult<bool> {
        if send_seconds == 0
            || send_seconds > 120
            || !(1..=DEFAULT_GAS_BUDGET_MICRO_USDC).contains(&final_required_budget_micro_usdc)
            || final_observations.as_object().is_none_or(|o| o.is_empty())
            || final_observations.to_string().len() > 16_384
        {
            return Err(denied());
        }
        let mut tx = self.pool.begin().await?;
        let row = lock_unsent(
            &mut tx,
            scope,
            expected_tx_hash,
            lease_token,
            send_seconds + 2,
        )
        .await?;
        let fresh: bool = sqlx::query_scalar(
            "SELECT $1<=clock_timestamp() AND $1>=clock_timestamp()-interval '30 seconds'",
        )
        .bind(observed_at)
        .fetch_one(&mut *tx)
        .await?;
        if !fresh {
            return Err(denied());
        }
        let operation = scope.operation_id().map_err(|_| denied())?;
        let authorization = sqlx::query("SELECT id FROM timeout_revalidation_authorizations a JOIN timeout_revalidation_gas_receipts r USING(operation_id,tx_hash,budget_day) JOIN gas_sponsorship_daily_budget b ON b.budget_key='platform' AND b.budget_day=a.budget_day WHERE a.id=$1 AND a.operation_id=$2 AND a.network=$3 AND a.tx_hash=$4 AND a.timeout_scope=$5 AND a.lease_token=$6 AND a.relayer_address=$7 AND a.nonce=$8 AND a.budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date AND a.expires_at>clock_timestamp()+make_interval(secs=>$9) AND r.reserved_micro_usdc>=a.required_budget_micro_usdc AND a.required_budget_micro_usdc>=$11 AND b.reserved_micro_usdc>=r.reserved_micro_usdc AND b.reserved_micro_usdc<=b.limit_micro_usdc AND b.limit_micro_usdc<=$10 AND NOT EXISTS(SELECT 1 FROM timeout_revalidation_events e WHERE e.authorization_id=a.id)")
            .bind(authorization_id).bind(operation).bind(&scope.network).bind(expected_tx_hash).bind(serde_json::to_value(scope)?).bind(lease_token).bind(row.try_get::<String,_>("relayer_address")?).bind(row.try_get::<i64,_>("nonce")?).bind(i64_from_u64(send_seconds+2)?).bind(i64_from_u64(DEFAULT_GAS_BUDGET_MICRO_USDC)?).bind(i64_from_u64(final_required_budget_micro_usdc)?).fetch_optional(&mut *tx).await?;
        if authorization.is_none() {
            sqlx::query("INSERT INTO timeout_revalidation_events(authorization_id,status) SELECT id,'expired' FROM timeout_revalidation_authorizations a WHERE id=$1 AND operation_id=$2 AND expires_at<=clock_timestamp() AND NOT EXISTS(SELECT 1 FROM timeout_revalidation_events e WHERE e.authorization_id=a.id)")
                .bind(authorization_id).bind(operation).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(false);
        }
        // Recheck time inside the CAS, after any preceding database waits.
        let changed = sqlx::query("UPDATE relay_transaction_outbox o SET send_attempts=1 WHERE operation_id=$1 AND network=$2 AND tx_hash=$3 AND transaction_metadata->'timeout_scope'=$4 AND send_attempts=0 AND $8<=clock_timestamp() AND $8>=clock_timestamp()-interval '30 seconds' AND EXISTS(SELECT 1 FROM x402_relayer_leases l JOIN timeout_revalidation_authorizations a ON a.lease_token=l.lease_token AND a.network=l.network WHERE l.network=$2 AND l.lease_token=$5 AND a.id=$6 AND l.lease_expires_at>clock_timestamp()+make_interval(secs=>$7) AND a.expires_at>clock_timestamp()+make_interval(secs=>$7) AND a.budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date)")
            .bind(operation).bind(&scope.network).bind(expected_tx_hash).bind(serde_json::to_value(scope)?).bind(lease_token).bind(authorization_id).bind(i64_from_u64(send_seconds+2)?).bind(observed_at).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(denied());
        }
        sqlx::query("INSERT INTO timeout_revalidation_events(authorization_id,status,observations,observed_at,required_budget_micro_usdc) VALUES ($1,'consumed',$2,$3,$4)")
            .bind(authorization_id).bind(final_observations).bind(observed_at).bind(i64_from_u64(final_required_budget_micro_usdc)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chain_base::{BaseRelayedTransaction, PreparedBaseRelayTransaction};
    use serde_json::json;

    async fn fixture(
        store: &PostgresStore,
    ) -> (TimeoutRevalidationRequest, PreparedBaseRelayTransaction) {
        let scope = TimeoutRelayScope {
            network: "base-sepolia".into(),
            bounty_contract: format!("0x{:040x}", Uuid::new_v4().as_u128()),
            action: "expire_claim".into(),
            round: 7,
            factory_contract: format!("0x{}", "11".repeat(20)),
            bounty_id: format!("0x{}", "22".repeat(32)),
            terms_hash: format!("0x{}", "33".repeat(32)),
        };
        let prepared = PreparedBaseRelayTransaction {
            nonce: 77,
            signed_transaction: "0x010203".into(),
            transaction: BaseRelayedTransaction {
                relayer: format!("0x{:040x}", Uuid::new_v4().as_u128()),
                tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                estimated_gas: 21_000,
                gas_limit: 25_000,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75_000,
            },
        };
        let lease = store
            .acquire_x402_relayer_lease(&scope.network, 180)
            .await
            .unwrap()
            .unwrap();
        store
            .save_timeout_relay(
                &scope,
                lease,
                &prepared,
                store.timeout_budget_day().await.unwrap(),
            )
            .await
            .unwrap();
        store.upsert_autonomous_bounty_event(&scope.network,&AutonomousBountyEvent {
            id:Uuid::new_v4(), log_key:Uuid::new_v4().to_string(), tx_hash:format!("0x{}","44".repeat(32)), block_number:1,log_index:0,contract_address:scope.factory_contract.clone(),bounty_id:scope.bounty_id.clone(),kind:AutonomousBountyEventKind::CanonicalBountyCreated,
            data:json!({"bounty_contract":scope.bounty_contract,"terms_hash":scope.terms_hash}), occurred_at:Utc::now(),
        }).await.unwrap();
        sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp()-interval '1 day' WHERE operation_id=$1").bind(scope.operation_id().unwrap()).execute(&store.pool).await.unwrap();
        (
            TimeoutRevalidationRequest {
                scope,
                expected_tx_hash: prepared.transaction.tx_hash.clone(),
                lease_token: lease,
                operator_identity: "operator:fixture".into(),
                observations: json!({"providers":["a","b"],"state":"exact_round_expired","pending":"none","simulation":"pass"}),
                observed_at: Utc::now(),
                required_budget_micro_usdc: 100,
                validity_seconds: 120,
            },
            prepared,
        )
    }
    async fn snapshot(
        store: &PostgresStore,
        id: Uuid,
    ) -> (DateTime<Utc>, String, serde_json::Value, i32) {
        sqlx::query_as("SELECT created_at,signed_transaction,transaction_metadata,send_attempts FROM relay_transaction_outbox WHERE operation_id=$1").bind(id).fetch_one(&store.pool).await.unwrap()
    }
    async fn consume(
        store: &PostgresStore,
        r: &TimeoutRevalidationRequest,
        id: Uuid,
    ) -> DbResult<bool> {
        store
            .consume_timeout_revalidation(
                &r.scope,
                &r.expected_tx_hash,
                id,
                r.lease_token,
                1,
                &json!({"final":"checked"}),
                Utc::now(),
                50,
            )
            .await
    }
    async fn terminal(store: &PostgresStore, id: Uuid) -> String {
        sqlx::query_scalar(
            "SELECT status FROM timeout_revalidation_events WHERE authorization_id=$1",
        )
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "requires explicitly isolated disposable PostgreSQL"]
    async fn timeout_revalidation_postgres_invariants() {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        crate::test_support::assert_isolated_security_database(&url);
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("timeout_revalidation_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let store = PostgresStore::from_pool(PgPool::connect_with(options).await.unwrap());
        store.migrate().await.unwrap();
        store.migrate().await.unwrap();
        let (mut r, p) = fixture(&store).await;
        let op = r.scope.operation_id().unwrap();
        let original = snapshot(&store, op).await;
        assert!(
            !store
                .claim_timeout_relay_send(&r.scope, r.lease_token, 1)
                .await
                .unwrap(),
            "ordinary stale-envelope guard must remain closed"
        );
        let baseline = store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc;
        // The public age guard remains closed even after operator authorization.
        let a = store.authorize_timeout_revalidation(&r).await.unwrap();
        assert_eq!(a.reserved_micro_usdc, 100);
        assert!(
            a.expires_at
                <= (a
                    .budget_day
                    .succ_opt()
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()),
            "send authorization cannot cross the reserved UTC budget day"
        );
        assert!(!store
            .claim_timeout_relay_send(&r.scope, r.lease_token, 1)
            .await
            .unwrap());
        // Concurrent renewals retain one receipt; only the latest authorization survives.
        let (b, c) = tokio::join!(
            store.authorize_timeout_revalidation(&r),
            store.authorize_timeout_revalidation(&r)
        );
        let b = b.unwrap();
        let c = c.unwrap();
        assert_eq!(
            store
                .gas_sponsor_budget_status()
                .await
                .unwrap()
                .reserved_micro_usdc,
            baseline + 100
        );
        assert_eq!(terminal(&store, a.id).await, "superseded");
        assert!(!consume(&store, &r, a.id).await.unwrap());
        let events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM timeout_revalidation_events WHERE authorization_id IN ($1,$2)",
        )
        .bind(b.id)
        .bind(c.id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(events, 1);
        r.required_budget_micro_usdc = 150;
        let d = store.authorize_timeout_revalidation(&r).await.unwrap();
        r.required_budget_micro_usdc = 50;
        let e = store.authorize_timeout_revalidation(&r).await.unwrap();
        assert_eq!(e.reserved_micro_usdc, 150);
        assert_eq!(
            store
                .gas_sponsor_budget_status()
                .await
                .unwrap()
                .reserved_micro_usdc,
            baseline + 150
        );
        assert!(!consume(&store, &r, d.id).await.unwrap());
        // Neither audit records nor original signed fields can be rewritten by this path.
        assert_eq!(sqlx::query("UPDATE timeout_revalidation_authorizations SET operator_identity='changed' WHERE id=$1").bind(e.id).execute(&store.pool).await.unwrap().rows_affected(),0);
        assert_eq!(
            sqlx::query("DELETE FROM timeout_revalidation_events WHERE authorization_id=$1")
                .bind(a.id)
                .execute(&store.pool)
                .await
                .unwrap()
                .rows_affected(),
            0
        );
        assert_eq!(snapshot(&store, op).await, original);
        // Invalid scope/hash/lease/observations are denied without a marker or allowance.
        for bad in [0, 1, 2, 3, 4, 5, 6, 7] {
            let mut q = r.clone();
            match bad {
                0 => q.expected_tx_hash = format!("0x{}", "55".repeat(32)),
                1 => q.scope.terms_hash = format!("0x{}", "66".repeat(32)),
                2 => q.lease_token = Uuid::new_v4(),
                3 => q.observed_at = Utc::now() - chrono::Duration::seconds(31),
                4 => q.observed_at = Utc::now() + chrono::Duration::seconds(60),
                5 => q.operator_identity.clear(),
                6 => q.observations = json!({"oversized":"x".repeat(16_384)}),
                _ => q.validity_seconds = 121,
            };
            assert!(
                store.authorize_timeout_revalidation(&q).await.is_err(),
                "case {bad}"
            );
        }
        assert!(store
            .consume_timeout_revalidation(
                &r.scope,
                &r.expected_tx_hash,
                e.id,
                r.lease_token,
                u64::MAX,
                &json!({"final":"checked"}),
                Utc::now(),
                50
            )
            .await
            .is_err());
        let mut bad_scope = r.scope.clone();
        bad_scope.terms_hash = format!("0x{}", "66".repeat(32));
        assert!(store
            .consume_timeout_revalidation(
                &bad_scope,
                &r.expected_tx_hash,
                e.id,
                r.lease_token,
                1,
                &json!({"final":"checked"}),
                Utc::now(),
                50
            )
            .await
            .is_err());
        for (evidence, when) in [
            (
                json!({"final":"checked"}),
                Utc::now() - chrono::Duration::seconds(31),
            ),
            (
                json!({"final":"checked"}),
                Utc::now() + chrono::Duration::seconds(60),
            ),
            (json!({}), Utc::now()),
        ] {
            assert!(store
                .consume_timeout_revalidation(
                    &r.scope,
                    &r.expected_tx_hash,
                    e.id,
                    r.lease_token,
                    1,
                    &evidence,
                    when,
                    50
                )
                .await
                .is_err());
        }
        assert!(!store
            .consume_timeout_revalidation(
                &r.scope,
                &r.expected_tx_hash,
                Uuid::new_v4(),
                r.lease_token,
                1,
                &json!({"final":"checked"}),
                Utc::now(),
                50
            )
            .await
            .unwrap());
        assert!(store
            .consume_timeout_revalidation(
                &r.scope,
                &format!("0x{}", "99".repeat(32)),
                e.id,
                r.lease_token,
                1,
                &json!({"final":"checked"}),
                Utc::now(),
                50
            )
            .await
            .is_err());
        // A current or later occupied nonce cannot be exempted by operation identity.
        let competing = Uuid::new_v4();
        sqlx::query("INSERT INTO relay_transaction_outbox(operation_id,network,relayer_address,nonce,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,78,$4,'0x9999','{}')").bind(competing).bind(&r.scope.network).bind(&p.transaction.relayer).bind(format!("0x{:064x}",Uuid::new_v4().as_u128())).execute(&store.pool).await.unwrap();
        assert!(store.authorize_timeout_revalidation(&r).await.is_err());
        assert!(consume(&store, &r, e.id).await.is_err());
        sqlx::query("DELETE FROM relay_transaction_outbox WHERE operation_id=$1")
            .bind(competing)
            .execute(&store.pool)
            .await
            .unwrap();
        // A different pending posting journal blocks even at an older nonce.
        let posting = Uuid::new_v4();
        sqlx::query("INSERT INTO x402_relay_attempts(id,idempotency_key,network,bounty_contract,contributor,amount,authorization_nonce,authorization_valid_before,request_fingerprint,relayer_address,status) VALUES($1,$2,$3,$4,$5,1,$2,1800000000,$2,$5,'prepared')")
            .bind(posting).bind(posting.to_string()).bind(&r.scope.network).bind(&r.scope.bounty_contract).bind(&p.transaction.relayer).execute(&store.pool).await.unwrap();
        sqlx::query("INSERT INTO posting_relay_transactions(relay_id,network,relayer_address,nonce,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,76,$4,'0x9999','{}')")
            .bind(posting).bind(&r.scope.network).bind(&p.transaction.relayer).bind(format!("0x{:064x}",Uuid::new_v4().as_u128())).execute(&store.pool).await.unwrap();
        assert!(store.authorize_timeout_revalidation(&r).await.is_err());
        assert!(consume(&store, &r, e.id).await.is_err());
        sqlx::query("UPDATE x402_relay_attempts SET status='confirmed' WHERE id=$1")
            .bind(posting)
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(
            store.authorize_timeout_revalidation(&r).await.is_ok(),
            "confirmed older nonce is historical, not pending"
        );
        sqlx::query("UPDATE posting_relay_transactions SET nonce=77 WHERE relay_id=$1")
            .bind(posting)
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(
            store.authorize_timeout_revalidation(&r).await.is_err(),
            "same nonce in another journal never receives the exact-row exemption"
        );
        sqlx::query("DELETE FROM posting_relay_transactions WHERE relay_id=$1")
            .bind(posting)
            .execute(&store.pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM x402_relay_attempts WHERE id=$1")
            .bind(posting)
            .execute(&store.pool)
            .await
            .unwrap();
        // Historical action journals without any outbox still fence a possibly pending send.
        for family in ["x402", "entrant", "sponsored"] {
            let id = Uuid::new_v4();
            match family {
                "x402" => {
                    sqlx::query("INSERT INTO x402_relay_attempts(id,idempotency_key,network,bounty_contract,contributor,amount,authorization_nonce,authorization_valid_before,request_fingerprint,relayer_address,status) VALUES($1,$2,$3,$4,$5,1,$2,1800000000,$2,$5,'prepared')").bind(id).bind(id.to_string()).bind(&r.scope.network).bind(&r.scope.bounty_contract).bind(&p.transaction.relayer).execute(&store.pool).await.unwrap();
                }
                "entrant" => {
                    sqlx::query("INSERT INTO open_competition_entrant_relays(id,idempotency_key,network,wallet,bounty_contract,delegate,action,wallet_nonce,deadline,payload_hash,request_fingerprint,relayer_address,status) VALUES($1,$2,$3,$4,$5,$4,0,1,1800000000,$2,$2,$4,'prepared')").bind(id).bind(id.to_string()).bind(&r.scope.network).bind(&p.transaction.relayer).bind(&r.scope.bounty_contract).execute(&store.pool).await.unwrap();
                }
                _ => {
                    sqlx::query("INSERT INTO sponsored_actions(id,network,request_hash,wallet,status) VALUES($1,$2,$3,$4,'prepared')").bind(id).bind(&r.scope.network).bind(id.to_string()).bind(&p.transaction.relayer).execute(&store.pool).await.unwrap();
                }
            }
            let table = match family {
                "x402" => "x402_relay_attempts",
                "entrant" => "open_competition_entrant_relays",
                _ => "sponsored_actions",
            };
            for status in ["prepared", "relaying", "broadcast", "failed"] {
                sqlx::query(&format!("UPDATE {table} SET status=$2 WHERE id=$1"))
                    .bind(id)
                    .bind(status)
                    .execute(&store.pool)
                    .await
                    .unwrap();
                assert!(
                    store.authorize_timeout_revalidation(&r).await.is_err(),
                    "{family}/{status} without outbox must block"
                );
            }
            sqlx::query(&format!(
                "UPDATE {table} SET status='confirmed' WHERE id=$1"
            ))
            .bind(id)
            .execute(&store.pool)
            .await
            .unwrap();
            assert!(
                store.authorize_timeout_revalidation(&r).await.is_ok(),
                "confirmed {family} is history"
            );
            sqlx::query(&format!("DELETE FROM {table} WHERE id=$1"))
                .bind(id)
                .execute(&store.pool)
                .await
                .unwrap();
        }
        let reserved_check = store.authorize_timeout_revalidation(&r).await.unwrap();
        assert!(
            !store
                .consume_timeout_revalidation(
                    &r.scope,
                    &r.expected_tx_hash,
                    reserved_check.id,
                    r.lease_token,
                    1,
                    &json!({"final":"checked"}),
                    Utc::now(),
                    r.required_budget_micro_usdc + 1
                )
                .await
                .unwrap(),
            "an unreserved larger final quote cannot consume"
        );
        // No reservation increment survives a denied cap transaction.
        let prior = store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc;
        let mut over = r.clone();
        over.required_budget_micro_usdc = 1_000_000;
        sqlx::query("UPDATE gas_sponsorship_daily_budget SET reserved_micro_usdc=1000000 WHERE budget_key='platform' AND budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date").execute(&store.pool).await.unwrap();
        assert!(store.authorize_timeout_revalidation(&over).await.is_err());
        sqlx::query("UPDATE gas_sponsorship_daily_budget SET reserved_micro_usdc=$1 WHERE budget_key='platform' AND budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date").bind(i64_from_u64(prior).unwrap()).execute(&store.pool).await.unwrap();
        // Expiry uses a real clock, not a mutable authorization fixture.
        r.validity_seconds = 1;
        let expired = store.authorize_timeout_revalidation(&r).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        assert!(!consume(&store, &r, expired.id).await.unwrap());
        assert_eq!(terminal(&store, expired.id).await, "expired");
        r.validity_seconds = 120;
        r.observed_at = Utc::now();
        let fresh = store.authorize_timeout_revalidation(&r).await.unwrap();
        // An old budget day cannot authorize a current send, even if timestamps look fresh.
        let old = Uuid::new_v4();
        sqlx::query("INSERT INTO timeout_revalidation_gas_receipts SELECT operation_id,tx_hash,budget_day-1,reserved_micro_usdc,created_at,updated_at FROM timeout_revalidation_gas_receipts WHERE operation_id=$1 AND budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date").bind(op).execute(&store.pool).await.unwrap();
        sqlx::query("INSERT INTO timeout_revalidation_authorizations SELECT $1,operation_id,network,tx_hash,relayer_address,nonce,timeout_scope,lease_token,operator_identity,observations,observed_at,budget_day-1,required_budget_micro_usdc,created_at,expires_at FROM timeout_revalidation_authorizations WHERE id=$2").bind(old).bind(fresh.id).execute(&store.pool).await.unwrap();
        assert!(!consume(&store, &r, old).await.unwrap());
        // Lease expiration cannot be repaired by a still-valid authorization.
        sqlx::query("UPDATE x402_relayer_leases SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE network=$1").bind(&r.scope.network).execute(&store.pool).await.unwrap();
        assert!(consume(&store, &r, fresh.id).await.is_err());
        assert!(store.authorize_timeout_revalidation(&r).await.is_err());
        r.lease_token = store
            .acquire_x402_relayer_lease(&r.scope.network, 180)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !consume(&store, &r, fresh.id).await.unwrap(),
            "new lease cannot inherit old authorization"
        );
        r.observed_at = Utc::now();
        let final_auth = store.authorize_timeout_revalidation(&r).await.unwrap();
        let (one, two) = tokio::join!(
            consume(&store, &r, final_auth.id),
            consume(&store, &r, final_auth.id)
        );
        assert_eq!(
            usize::from(matches!(one, Ok(true))) + usize::from(matches!(two, Ok(true))),
            1
        );
        assert_eq!(terminal(&store, final_auth.id).await, "consumed");
        let final_evidence: serde_json::Value = sqlx::query_scalar(
            "SELECT observations FROM timeout_revalidation_events WHERE authorization_id=$1",
        )
        .bind(final_auth.id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
        assert_eq!(final_evidence, json!({"final":"checked"}));
        let after = snapshot(&store, op).await;
        assert_eq!(
            (after.0, after.1, after.2),
            (original.0, original.1, original.2)
        );
        assert_eq!(after.3, 1);
        // Lost commit acknowledgement: a durable marker prevents every new authorization/send.
        assert!(store.authorize_timeout_revalidation(&r).await.is_err());
        assert!(consume(&store, &r, final_auth.id).await.is_err());
        assert!(!store
            .claim_timeout_relay_send(&r.scope, r.lease_token, 1)
            .await
            .unwrap());
        assert!(store
            .gas_sponsor_budget()
            .ensure_sender_nonce_available(84532, &p.transaction.relayer, p.nonce)
            .await
            .is_err());
        store
            .release_x402_relayer_lease(&r.scope.network, r.lease_token)
            .await
            .unwrap();
        // A previous day's receipt is never borrowed by a new current-day authorization.
        let (next, _) = fixture(&store).await;
        let next_op = next.scope.operation_id().unwrap();
        sqlx::query("INSERT INTO timeout_revalidation_gas_receipts(operation_id,tx_hash,budget_day,reserved_micro_usdc) VALUES($1,$2,(clock_timestamp() AT TIME ZONE 'UTC')::date-1,1000000)").bind(next_op).bind(&next.expected_tx_hash).execute(&store.pool).await.unwrap();
        let before = store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc;
        let today = store.authorize_timeout_revalidation(&next).await.unwrap();
        assert_eq!(today.reserved_micro_usdc, 100);
        assert_eq!(
            store
                .gas_sponsor_budget_status()
                .await
                .unwrap()
                .reserved_micro_usdc,
            before + 100
        );
        store
            .release_x402_relayer_lease(&next.scope.network, next.lease_token)
            .await
            .unwrap();
    }
}
