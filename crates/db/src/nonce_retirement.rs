//! One explicitly approved self-transfer, linked to an immutable obsolete timeout.
//! Unknown commits and any saved replacement permanently revoke execution retries.
use super::*;
use chain_base::{
    nonce_retirement::{
        validate_nonce_retirement_envelope, NonceRetirementConfirmation,
        NonceRetirementObservation, NonceRetirementPlan,
    },
    timeout_recovery::TimeoutRelayScope,
    PreparedBaseRelayTransaction,
};

mod retained_dispatch;
pub use retained_dispatch::SavedRetainedNonceDispatch;

#[derive(Clone, Serialize)]
pub struct SavedNonceRetirementPlan {
    pub id: Uuid,
    pub digest: String,
    pub plan: NonceRetirementPlan,
}
#[derive(Clone)]
pub struct SavedNonceRetirement {
    pub plan: SavedNonceRetirementPlan,
    pub prepared: PreparedBaseRelayTransaction,
    pub send_attempted: bool,
    pub confirmed: bool,
}
fn denied() -> DbError {
    DbError::X402RelayQuotaExceeded("nonce retirement denied; reconcile retained evidence".into())
}
fn evidence_valid(plan: &NonceRetirementPlan, o: &NonceRetirementObservation) -> DbResult<()> {
    if o.maximum_cost_wei > plan.maximum_cost_wei
        || o.maximum_cost_wei == 0
        || o.finalized_block.number > o.safe_block.number
        || o.provider_hashes[0] == o.provider_hashes[1]
    {
        return Err(denied());
    }
    Ok(())
}
async fn lock_original(
    tx: &mut Transaction<'_, Postgres>,
    scope: &TimeoutRelayScope,
    plan: &NonceRetirementPlan,
    lease: Uuid,
    margin: u64,
) -> DbResult<()> {
    plan.validate().map_err(|_| denied())?;
    let row =
        super::timeout_revalidation::lock_unsent(tx, scope, &plan.original_tx_hash, lease, margin)
            .await?;
    if scope.network != "base-mainnet"
        || scope.operation_id().map_err(|_| denied())? != plan.operation_id
        || row.try_get::<String, _>("relayer_address")? != plan.sender
        || u64_from_i64(row.try_get("nonce")?)? != plan.nonce
    {
        return Err(denied());
    }
    let bytes: String = sqlx::query_scalar(
        "SELECT signed_transaction FROM relay_transaction_outbox WHERE operation_id=$1",
    )
    .bind(plan.operation_id)
    .fetch_one(&mut **tx)
    .await?;
    let raw = hex::decode(bytes.strip_prefix("0x").ok_or_else(denied)?).map_err(|_| denied())?;
    if hex::encode(Sha256::digest(raw)) != plan.original_envelope_sha256 {
        return Err(denied());
    }
    let active:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM timeout_revalidation_authorizations a WHERE operation_id=$1 AND expires_at>clock_timestamp() AND NOT EXISTS(SELECT 1 FROM timeout_revalidation_events e WHERE e.authorization_id=a.id))").bind(plan.operation_id).fetch_one(&mut **tx).await?;
    if active {
        return Err(denied());
    }
    Ok(())
}
async fn fresh(tx: &mut Transaction<'_, Postgres>, at: DateTime<Utc>) -> DbResult<()> {
    let good: bool = sqlx::query_scalar(
        "SELECT $1<=clock_timestamp() AND $1>=clock_timestamp()-interval '30 seconds'",
    )
    .bind(at)
    .fetch_one(&mut **tx)
    .await?;
    if !good {
        return Err(denied());
    }
    Ok(())
}
async fn approval_live(
    tx: &mut Transaction<'_, Postgres>,
    saved: &SavedNonceRetirementPlan,
    lease: Uuid,
    margin: u64,
) -> DbResult<()> {
    let good:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nonce_retirement_approvals a JOIN nonce_retirement_plans p ON p.id=a.plan_id JOIN gas_sponsorship_daily_budget b ON b.budget_key='platform' AND b.budget_day=a.budget_day WHERE a.plan_id=$1 AND a.approved_digest=$2 AND a.lease_token=$3 AND a.budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date AND a.expires_at>clock_timestamp()+make_interval(secs=>$4) AND p.expires_at>clock_timestamp()+make_interval(secs=>$4) AND b.reserved_micro_usdc>=a.reserved_micro_usdc AND b.reserved_micro_usdc<=b.limit_micro_usdc AND b.limit_micro_usdc<=1000000)")
        .bind(saved.id).bind(&saved.digest).bind(lease).bind(i64_from_u64(margin)?).fetch_one(&mut **tx).await?;
    if !good {
        return Err(denied());
    }
    Ok(())
}
impl PostgresStore {
    /// Same lease and duration as ordinary relays; only this exact retained
    /// unsent operation may acquire it while a retirement fence exists.
    pub async fn acquire_nonce_retirement_lease(
        &self,
        id: Uuid,
        seconds: u64,
    ) -> DbResult<Option<Uuid>> {
        if seconds == 0 || seconds > 180 {
            return Err(denied());
        }
        let token = Uuid::new_v4();
        let row:Option<Uuid>=sqlx::query_scalar("INSERT INTO x402_relayer_leases(network,lease_token,lease_expires_at,updated_at) SELECT o.network,$2,clock_timestamp()+make_interval(secs=>$3),clock_timestamp() FROM relay_transaction_outbox o WHERE o.operation_id=$1 AND o.network='base-mainnet' AND o.send_attempts=0 AND o.transaction_metadata ? 'timeout_scope' ON CONFLICT(network) DO UPDATE SET lease_token=excluded.lease_token,lease_expires_at=excluded.lease_expires_at,updated_at=clock_timestamp() WHERE x402_relayer_leases.lease_expires_at<=clock_timestamp() RETURNING lease_token")
            .bind(id).bind(token).bind(i64_from_u64(seconds)?).fetch_optional(&self.pool).await?;
        Ok(row)
    }
    pub async fn save_nonce_retirement_plan(
        &self,
        scope: &TimeoutRelayScope,
        lease: Uuid,
        plan: &NonceRetirementPlan,
        o: &NonceRetirementObservation,
    ) -> DbResult<SavedNonceRetirementPlan> {
        evidence_valid(plan, o)?;
        let digest = plan.digest().map_err(|_| denied())?;
        let mut tx = self.pool.begin().await?;
        lock_original(&mut tx, scope, plan, lease, 3).await?;
        fresh(&mut tx, o.observed_at).await?;
        let conflict:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nonce_retirement_envelopes WHERE operation_id=$1 UNION ALL SELECT 1 FROM nonce_retirement_plans WHERE network=$2 AND expires_at>clock_timestamp())").bind(plan.operation_id).bind(&scope.network).fetch_one(&mut *tx).await?;
        if conflict {
            return Err(denied());
        }
        let id = Uuid::new_v4();
        let rows=sqlx::query("INSERT INTO nonce_retirement_plans(id,operation_id,digest,network,relayer_address,nonce,plan,observation,created_at,expires_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10 WHERE $9<=clock_timestamp() AND $10>clock_timestamp()+interval '30 seconds' AND $9>=clock_timestamp()-interval '30 seconds'")
            .bind(id).bind(plan.operation_id).bind(&digest).bind(&scope.network).bind(&plan.sender).bind(i64_from_u64(plan.nonce)?).bind(serde_json::to_value(plan)?).bind(serde_json::to_value(o)?).bind(plan.created_at).bind(plan.expires_at).execute(&mut *tx).await?;
        if rows.rows_affected() != 1 {
            return Err(denied());
        }
        tx.commit().await?;
        Ok(SavedNonceRetirementPlan {
            id,
            digest,
            plan: plan.clone(),
        })
    }
    pub async fn nonce_retirement_plan(
        &self,
        id: Uuid,
    ) -> DbResult<Option<SavedNonceRetirementPlan>> {
        sqlx::query("SELECT id,digest,plan FROM nonce_retirement_plans WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .map(|r| {
                Ok(SavedNonceRetirementPlan {
                    id: r.try_get("id")?,
                    digest: r.try_get("digest")?,
                    plan: serde_json::from_value(r.try_get("plan")?)?,
                })
            })
            .transpose()
    }
    pub async fn nonce_retirement(
        &self,
        operation: Uuid,
    ) -> DbResult<Option<SavedNonceRetirement>> {
        let row=sqlx::query("SELECT p.id,p.digest,p.plan,e.signed_transaction,e.transaction_metadata,EXISTS(SELECT 1 FROM nonce_retirement_send_events s WHERE s.operation_id=e.operation_id) AS sent,EXISTS(SELECT 1 FROM nonce_retirement_confirmations c WHERE c.operation_id=e.operation_id) AS confirmed FROM nonce_retirement_envelopes e JOIN nonce_retirement_plans p ON p.id=e.plan_id WHERE e.operation_id=$1").bind(operation).fetch_optional(&self.pool).await?;
        row.map(|r| {
            let plan: NonceRetirementPlan = serde_json::from_value(r.try_get("plan")?)?;
            let prepared = PreparedBaseRelayTransaction {
                nonce: plan.nonce,
                signed_transaction: r.try_get("signed_transaction")?,
                transaction: serde_json::from_value(r.try_get("transaction_metadata")?)?,
            };
            validate_nonce_retirement_envelope(&plan, &prepared).map_err(|_| denied())?;
            Ok(SavedNonceRetirement {
                plan: SavedNonceRetirementPlan {
                    id: r.try_get("id")?,
                    digest: r.try_get("digest")?,
                    plan,
                },
                prepared,
                send_attempted: r.try_get("sent")?,
                confirmed: r.try_get("confirmed")?,
            })
        })
        .transpose()
    }
    /// Explicit human approval is represented by the exact stored digest. It is
    /// never inferred from possession of an operator credential or an RPC result.
    #[allow(clippy::too_many_arguments)]
    pub async fn approve_nonce_retirement(
        &self,
        scope: &TimeoutRelayScope,
        saved: &SavedNonceRetirementPlan,
        approved_digest: &str,
        lease: Uuid,
        operator: &str,
        o: &NonceRetirementObservation,
        amount: u64,
    ) -> DbResult<()> {
        if approved_digest != saved.digest
            || saved.plan.digest().map_err(|_| denied())? != saved.digest
            || operator.is_empty()
            || operator.len() > 128
            || !(1..=DEFAULT_GAS_BUDGET_MICRO_USDC).contains(&amount)
        {
            return Err(denied());
        }
        evidence_valid(&saved.plan, o)?;
        let mut tx = self.pool.begin().await?;
        lock_original(&mut tx, scope, &saved.plan, lease, 3).await?;
        fresh(&mut tx, o.observed_at).await?;
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM nonce_retirement_envelopes WHERE operation_id=$1 UNION ALL SELECT 1 FROM nonce_retirement_approvals WHERE plan_id=$2)").bind(saved.plan.operation_id).bind(saved.id).fetch_one(&mut *tx).await?;
        if exists {
            return Err(denied());
        }
        let day: chrono::NaiveDate =
            sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'UTC')::date")
                .fetch_one(&mut *tx)
                .await?;
        // Same operation's unused approvals reserve only the maximum on a day.
        // Once bytes exist a different approval/plan is impossible.
        let previous:i64=sqlx::query_scalar("SELECT COALESCE(max(reserved_micro_usdc),0) FROM nonce_retirement_approvals WHERE operation_id=$1 AND budget_day=$2").bind(saved.plan.operation_id).bind(day).fetch_one(&mut *tx).await?;
        let reserved = previous.max(i64_from_u64(amount)?);
        let delta = reserved - previous;
        if delta > 0 {
            let row=sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES('platform',$1,$2,1000000) ON CONFLICT(budget_key,budget_day) DO UPDATE SET reserved_micro_usdc=gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc,limit_micro_usdc=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc),updated_at=clock_timestamp() WHERE gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc<=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc) RETURNING budget_key").bind(day).bind(delta).fetch_optional(&mut *tx).await?;
            if row.is_none() {
                return Err(denied());
            }
        }
        let rows=sqlx::query("WITH admission_clock AS MATERIALIZED (SELECT clock_timestamp() AS at) INSERT INTO nonce_retirement_approvals(plan_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at) SELECT p.id,p.operation_id,p.digest,$4,$5,$6,$7,$8,admission_clock.at,LEAST(p.expires_at,l.lease_expires_at-interval '2 seconds',admission_clock.at+interval '120 seconds',(($7::date+1)::timestamp AT TIME ZONE 'UTC')) FROM nonce_retirement_plans p JOIN x402_relayer_leases l ON l.network=p.network CROSS JOIN admission_clock WHERE p.id=$1 AND p.digest=$2 AND p.plan=$3 AND l.lease_token=$5 AND l.lease_expires_at>clock_timestamp()+interval '3 seconds' AND p.expires_at>clock_timestamp()+interval '3 seconds' AND $7=(clock_timestamp() AT TIME ZONE 'UTC')::date")
            .bind(saved.id).bind(approved_digest).bind(serde_json::to_value(&saved.plan)?).bind(operator).bind(lease).bind(serde_json::to_value(o)?).bind(day).bind(reserved).execute(&mut *tx).await?;
        if rows.rows_affected() != 1 {
            return Err(denied());
        }
        approval_live(&mut tx, saved, lease, 2).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn save_nonce_retirement_envelope(
        &self,
        scope: &TimeoutRelayScope,
        saved: &SavedNonceRetirementPlan,
        lease: Uuid,
        prepared: &PreparedBaseRelayTransaction,
    ) -> DbResult<()> {
        validate_nonce_retirement_envelope(&saved.plan, prepared).map_err(|_| denied())?;
        let mut tx = self.pool.begin().await?;
        lock_original(&mut tx, scope, &saved.plan, lease, 3).await?;
        approval_live(&mut tx, saved, lease, 3).await?;
        // No upsert: even an identical existing envelope is reconciliation-only.
        sqlx::query("INSERT INTO nonce_retirement_envelopes(operation_id,plan_id,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,$4,$5)")
            .bind(saved.plan.operation_id).bind(saved.id).bind(&prepared.transaction.tx_hash).bind(&prepared.signed_transaction).bind(serde_json::to_value(&prepared.transaction)?).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn consume_nonce_retirement_send(
        &self,
        scope: &TimeoutRelayScope,
        saved: &SavedNonceRetirement,
        lease: Uuid,
        send_seconds: u64,
        o: &NonceRetirementObservation,
    ) -> DbResult<bool> {
        if send_seconds == 0 || send_seconds > 120 {
            return Err(denied());
        }
        evidence_valid(&saved.plan.plan, o)?;
        let mut tx = self.pool.begin().await?;
        lock_original(&mut tx, scope, &saved.plan.plan, lease, send_seconds + 2).await?;
        approval_live(&mut tx, &saved.plan, lease, send_seconds + 2).await?;
        fresh(&mut tx, o.observed_at).await?;
        let rows=sqlx::query("INSERT INTO nonce_retirement_send_events(operation_id,observation) SELECT e.operation_id,$4 FROM nonce_retirement_envelopes e JOIN nonce_retirement_approvals a ON a.plan_id=e.plan_id JOIN x402_relayer_leases l ON l.lease_token=a.lease_token WHERE e.operation_id=$1 AND e.plan_id=$2 AND e.tx_hash=$3 AND e.signed_transaction=$5 AND a.expires_at>clock_timestamp()+make_interval(secs=>$6) AND l.lease_expires_at>clock_timestamp()+make_interval(secs=>$6) AND a.budget_day=(clock_timestamp() AT TIME ZONE 'UTC')::date AND $7>=clock_timestamp()-interval '30 seconds' ON CONFLICT(operation_id) DO NOTHING")
            .bind(saved.plan.plan.operation_id).bind(saved.plan.id).bind(&saved.prepared.transaction.tx_hash).bind(serde_json::to_value(o)?).bind(&saved.prepared.signed_transaction).bind(i64_from_u64(send_seconds+2)?).bind(o.observed_at).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(rows.rows_affected() == 1)
    }
    pub async fn confirm_nonce_retirement(
        &self,
        saved: &SavedNonceRetirement,
        o: &NonceRetirementConfirmation,
    ) -> DbResult<()> {
        if o.transaction_hash != saved.prepared.transaction.tx_hash
            || o.finalized_nonce <= saved.plan.plan.nonce
            || o.included_block.number > o.finalized_block.number
            || o.provider_hashes[0] == o.provider_hashes[1]
        {
            return Err(denied());
        }
        let mut tx = self.pool.begin().await?;
        fresh(&mut tx, o.observed_at).await?;
        let rows=sqlx::query("INSERT INTO nonce_retirement_confirmations(operation_id,tx_hash,observation) SELECT operation_id,tx_hash,$3 FROM nonce_retirement_envelopes WHERE operation_id=$1 AND tx_hash=$2 AND signed_transaction=$4 ON CONFLICT(operation_id) DO NOTHING")
            .bind(saved.plan.plan.operation_id).bind(&o.transaction_hash).bind(serde_json::to_value(o)?).bind(&saved.prepared.signed_transaction).execute(&mut *tx).await?;
        let _ = rows;
        tx.commit().await?;
        Ok(())
    }
}
