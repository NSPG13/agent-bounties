//! Explicit initial dispatch uses the existing operation-unique send latch.
use super::*;
use chain_base::nonce_retirement::retained_dispatch::RetainedNonceDispatch;

#[derive(Clone, Serialize)]
pub struct SavedRetainedNonceDispatch {
    pub id: Uuid,
    pub digest: String,
    pub proposal: RetainedNonceDispatch,
}

fn bound(saved: &SavedNonceRetirement, proposal: &RetainedNonceDispatch) -> DbResult<()> {
    proposal
        .bind(saved.plan.id, &saved.plan.plan, &saved.prepared)
        .map_err(|_| denied())?;
    if saved.send_attempted || saved.confirmed || saved.plan.digest != proposal.retirement_digest {
        return Err(denied());
    }
    Ok(())
}

async fn lock_retained(
    tx: &mut Transaction<'_, Postgres>,
    scope: &TimeoutRelayScope,
    saved: &SavedNonceRetirement,
    lease: Uuid,
    margin: u64,
) -> DbResult<()> {
    lock_original(tx, scope, &saved.plan.plan, lease, margin).await?;
    let valid: Option<Uuid> = sqlx::query_scalar(
        "SELECT e.operation_id FROM nonce_retirement_envelopes e
         JOIN nonce_retirement_plans p ON p.id=e.plan_id
         JOIN nonce_retirement_approvals a ON a.plan_id=e.plan_id
         WHERE e.operation_id=$1 AND e.plan_id=$2 AND e.tx_hash=$3
           AND e.signed_transaction=$4 AND e.transaction_metadata=$5
           AND p.digest=$6 AND p.plan=$7
           AND a.expires_at<=clock_timestamp()
           AND NOT EXISTS(SELECT 1 FROM nonce_retirement_send_events s WHERE s.operation_id=e.operation_id)
           AND NOT EXISTS(SELECT 1 FROM nonce_retirement_confirmations c WHERE c.operation_id=e.operation_id)
         FOR UPDATE OF e")
        .bind(saved.plan.plan.operation_id).bind(saved.plan.id)
        .bind(&saved.prepared.transaction.tx_hash).bind(&saved.prepared.signed_transaction)
        .bind(serde_json::to_value(&saved.prepared.transaction)?)
        .bind(&saved.plan.digest).bind(serde_json::to_value(&saved.plan.plan)?)
        .fetch_optional(&mut **tx).await?;
    if valid != Some(saved.plan.plan.operation_id) {
        return Err(denied());
    }
    Ok(())
}

impl PostgresStore {
    pub async fn save_retained_nonce_dispatch(
        &self,
        scope: &TimeoutRelayScope,
        saved: &SavedNonceRetirement,
        lease: Uuid,
        proposal: &RetainedNonceDispatch,
        observation: &NonceRetirementObservation,
    ) -> DbResult<SavedRetainedNonceDispatch> {
        bound(saved, proposal)?;
        evidence_valid(&saved.plan.plan, observation)?;
        if observation.provider_hashes != proposal.provider_hashes {
            return Err(denied());
        }
        let digest = proposal.digest().map_err(|_| denied())?;
        let mut tx = self.pool.begin().await?;
        lock_retained(&mut tx, scope, saved, lease, 3).await?;
        fresh(&mut tx, observation.observed_at).await?;
        let active: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM retained_nonce_dispatch_proposals WHERE operation_id=$1 AND expires_at>clock_timestamp())")
            .bind(proposal.operation_id).fetch_one(&mut *tx).await?;
        if active {
            return Err(denied());
        }
        let id = Uuid::new_v4();
        let rows = sqlx::query("INSERT INTO retained_nonce_dispatch_proposals(id,operation_id,retirement_plan_id,tx_hash,digest,proposal,observation,created_at,expires_at)
            SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9
            WHERE $8<=clock_timestamp() AND $8>=clock_timestamp()-interval '30 seconds'
              AND $9>clock_timestamp()+interval '30 seconds'")
            .bind(id).bind(proposal.operation_id).bind(saved.plan.id).bind(&proposal.transaction_hash)
            .bind(&digest).bind(serde_json::to_value(proposal)?).bind(serde_json::to_value(observation)?)
            .bind(proposal.created_at).bind(proposal.expires_at).execute(&mut *tx).await?;
        if rows.rows_affected() != 1 {
            return Err(denied());
        }
        tx.commit().await?;
        Ok(SavedRetainedNonceDispatch {
            id,
            digest,
            proposal: proposal.clone(),
        })
    }

    pub async fn retained_nonce_dispatch(
        &self,
        id: Uuid,
    ) -> DbResult<Option<SavedRetainedNonceDispatch>> {
        let row = sqlx::query(
            "SELECT id,digest,proposal FROM retained_nonce_dispatch_proposals WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|r| {
            let proposal: RetainedNonceDispatch = serde_json::from_value(r.try_get("proposal")?)?;
            let digest: String = r.try_get("digest")?;
            if proposal.digest().map_err(|_| denied())? != digest {
                return Err(denied());
            }
            Ok(SavedRetainedNonceDispatch {
                id: r.try_get("id")?,
                digest,
                proposal,
            })
        })
        .transpose()
    }

    /// One atomic authorization + shared send latch. Unknown commit => never send.
    #[allow(clippy::too_many_arguments)]
    pub async fn consume_retained_nonce_dispatch(
        &self,
        scope: &TimeoutRelayScope,
        saved: &SavedNonceRetirement,
        dispatch: &SavedRetainedNonceDispatch,
        approved_digest: &str,
        lease: Uuid,
        operator: &str,
        send_seconds: u64,
        observation: &NonceRetirementObservation,
        amount: u64,
    ) -> DbResult<()> {
        bound(saved, &dispatch.proposal)?;
        if dispatch.id.is_nil()
            || approved_digest != dispatch.digest
            || dispatch.proposal.digest().map_err(|_| denied())? != dispatch.digest
            || operator.is_empty()
            || operator.len() > 128
            || send_seconds == 0
            || send_seconds > 120
            || !(1..=DEFAULT_GAS_BUDGET_MICRO_USDC).contains(&amount)
            || observation.provider_hashes != dispatch.proposal.provider_hashes
        {
            return Err(denied());
        }
        evidence_valid(&saved.plan.plan, observation)?;
        let mut tx = self.pool.begin().await?;
        let margin = i64_from_u64(send_seconds + 2)?;
        lock_retained(&mut tx, scope, saved, lease, send_seconds + 2).await?;
        fresh(&mut tx, observation.observed_at).await?;
        let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM retained_nonce_dispatch_proposals
            WHERE id=$1 AND operation_id=$2 AND digest=$3 AND proposal=$4
              AND created_at<=clock_timestamp() AND expires_at>clock_timestamp()+make_interval(secs=>$5))")
            .bind(dispatch.id).bind(dispatch.proposal.operation_id).bind(approved_digest)
            .bind(serde_json::to_value(&dispatch.proposal)?).bind(margin).fetch_one(&mut *tx).await?;
        if !live {
            return Err(denied());
        }
        let day: chrono::NaiveDate =
            sqlx::query_scalar("SELECT (clock_timestamp() AT TIME ZONE 'UTC')::date")
                .fetch_one(&mut *tx)
                .await?;
        let previous: i64 = sqlx::query_scalar("SELECT COALESCE(max(reserved_micro_usdc),0) FROM (
            SELECT reserved_micro_usdc FROM nonce_retirement_approvals WHERE operation_id=$1 AND budget_day=$2
            UNION ALL SELECT reserved_micro_usdc FROM retained_nonce_dispatch_authorizations WHERE operation_id=$1 AND budget_day=$2
            ) reservations")
            .bind(dispatch.proposal.operation_id).bind(day).fetch_one(&mut *tx).await?;
        let reserved = previous.max(i64_from_u64(amount)?);
        let delta = reserved - previous;
        if delta > 0 {
            let budget: Option<String> = sqlx::query_scalar("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc)
                VALUES('platform',$1,$2,1000000) ON CONFLICT(budget_key,budget_day) DO UPDATE
                SET reserved_micro_usdc=gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc,
                    limit_micro_usdc=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc),updated_at=clock_timestamp()
                WHERE gas_sponsorship_daily_budget.reserved_micro_usdc+excluded.reserved_micro_usdc<=LEAST(gas_sponsorship_daily_budget.limit_micro_usdc,excluded.limit_micro_usdc)
                RETURNING budget_key")
                .bind(day).bind(delta).fetch_optional(&mut *tx).await?;
            if budget.is_none() {
                return Err(denied());
            }
        }
        let budget_valid: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM gas_sponsorship_daily_budget
            WHERE budget_key='platform' AND budget_day=$1 AND reserved_micro_usdc>=$2
              AND reserved_micro_usdc<=limit_micro_usdc AND limit_micro_usdc<=1000000)",
        )
        .bind(day)
        .bind(reserved)
        .fetch_one(&mut *tx)
        .await?;
        if !budget_valid {
            return Err(denied());
        }
        // Same operation PK as original first_send: no path can consume twice.
        sqlx::query(
            "INSERT INTO nonce_retirement_send_events(operation_id,observation) VALUES($1,$2)",
        )
        .bind(dispatch.proposal.operation_id)
        .bind(serde_json::to_value(observation)?)
        .execute(&mut *tx)
        .await?;
        let rows = sqlx::query("WITH admission_clock AS MATERIALIZED (SELECT clock_timestamp() AS at)
            INSERT INTO retained_nonce_dispatch_authorizations(proposal_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at)
            SELECT p.id,p.operation_id,p.digest,$4,$5,$6,$7,$8,c.at,
              LEAST(p.expires_at,l.lease_expires_at-interval '2 seconds',c.at+interval '120 seconds',(($7::date+1)::timestamp AT TIME ZONE 'UTC'))
            FROM retained_nonce_dispatch_proposals p JOIN x402_relayer_leases l ON l.network='base-mainnet' CROSS JOIN admission_clock c
            WHERE p.id=$1 AND p.digest=$2 AND p.proposal=$3 AND l.lease_token=$5
              AND LEAST(p.expires_at,l.lease_expires_at-interval '2 seconds',c.at+interval '120 seconds',(($7::date+1)::timestamp AT TIME ZONE 'UTC'))>clock_timestamp()+make_interval(secs=>$9)
              AND $7=(clock_timestamp() AT TIME ZONE 'UTC')::date
              AND $10<=clock_timestamp() AND $10>=clock_timestamp()-interval '30 seconds'")
            .bind(dispatch.id).bind(approved_digest).bind(serde_json::to_value(&dispatch.proposal)?)
            .bind(operator).bind(lease).bind(serde_json::to_value(observation)?).bind(day).bind(reserved)
            .bind(margin).bind(observation.observed_at).execute(&mut *tx).await?;
        if rows.rows_affected() != 1 {
            return Err(denied());
        }
        tx.commit().await?;
        Ok(())
    }
}
