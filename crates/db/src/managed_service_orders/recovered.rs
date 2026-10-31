//! Recovery-qualified refunds keep the original funded order and funding journal intact.
use super::{refunds, stripe::locked_order, *};
use managed_orders::{
    funding::FundingPlan,
    recovery::RecoveredPrincipal,
    stripe_refund::{ManagedRefundPlan, RefundKind},
};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub(super) async fn reject_reserved(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
) -> Result<()> {
    let reserved: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_recovered_refund_operations WHERE order_id=$1)",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?;
    if reserved {
        return Err(OrderError::Conflict.into());
    }
    Ok(())
}
/// Under the common order lock, reject every payment or review obligation that can conflict.
async fn eligible(tx: &mut Transaction<'_, Postgres>, order: &ManagedOrder) -> Result<()> {
    if order.status != ManagedOrderStatus::Funded {
        return Err(OrderError::Conflict.into());
    }
    let blocked:bool=sqlx::query_scalar("SELECT
        EXISTS(SELECT 1 FROM managed_customer_reviews WHERE order_id=$1 AND
          ((verdict->>'passed')::boolean OR signed_passed IS TRUE OR (ruling->>'passed')::boolean
            OR (appeal IS NOT NULL AND ruling IS NULL)
            OR (signature IS NULL AND (appeal_ends_at IS NULL OR appeal_ends_at>extract(epoch FROM clock_timestamp())))))
        OR EXISTS(SELECT 1 FROM managed_deposit_payouts WHERE order_id=$1)
        OR EXISTS(SELECT 1 FROM managed_refund_operations WHERE order_id=$1)
        OR EXISTS(SELECT 1 FROM managed_deposit_refund_operations WHERE order_id=$1)
        OR EXISTS(SELECT 1 FROM managed_stripe_disputes WHERE order_id=$1 AND (status<>'won' OR reconciliation_required))
        OR EXISTS(SELECT 1 FROM autonomous_bounty_events WHERE network=$2 AND contract_address=lower($3) AND kind='bounty_settled')")
        .bind(order.order_id).bind(&order.quote.input().bounty.network).bind(&order.quote.input().bounty.predicted_bounty)
        .fetch_one(&mut **tx).await?;
    if blocked {
        return Err(OrderError::Conflict.into());
    }
    Ok(())
}
pub(super) async fn require_fresh(
    tx: &mut Transaction<'_, Postgres>,
    order: &ManagedOrder,
) -> Result<()> {
    eligible(tx, order).await?;
    let fresh: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_recovered_refund_evidence WHERE order_id=$1
       AND verified_at>clock_timestamp()-interval '30 seconds' AND NOT reconciliation_required)",
    )
    .bind(order.order_id)
    .fetch_one(&mut **tx)
    .await?;
    if !fresh {
        return Err(managed_orders::stripe_refund::RefundError::ReconciliationRequired.into());
    }
    Ok(())
}
#[derive(Debug, Clone)]
pub struct ManagedRecoveryTarget {
    pub order: ManagedOrder,
    pub funding_operation_id: Uuid,
    pub funding_plan: FundingPlan,
    pub funding_tx: String,
    pub funded_block: u64,
}
impl PostgresStore {
    pub async fn claim_managed_recovery_candidates(&self, limit: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar("WITH candidates AS (
            SELECT order_id FROM managed_bounty_funding_operations f WHERE outcome='funded'
            AND NOT EXISTS(SELECT 1 FROM managed_recovered_refund_operations r WHERE r.order_id=f.order_id AND r.refund_id IS NOT NULL)
            AND (last_recovery_attempt_at IS NULL OR last_recovery_attempt_at<statement_timestamp()-interval '60 seconds')
            ORDER BY last_recovery_attempt_at NULLS FIRST,order_id LIMIT $1 FOR UPDATE SKIP LOCKED
          ) UPDATE managed_bounty_funding_operations f SET last_recovery_attempt_at=statement_timestamp()
            FROM candidates c WHERE f.order_id=c.order_id RETURNING f.order_id")
            .bind(limit.clamp(1,20)).fetch_all(&self.pool).await?)
    }
    pub async fn managed_recovery_target(&self, id: Uuid) -> Result<ManagedRecoveryTarget> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, id).await?;
        let row=sqlx::query("SELECT g.*,o.tx_hash AS saved_tx_hash FROM managed_bounty_funding_operations g
            LEFT JOIN relay_transaction_outbox o ON o.operation_id=g.operation_id WHERE g.order_id=$1 AND g.outcome='funded'")
            .bind(id).fetch_one(&mut *tx).await?;
        let op = super::funding::operation_from_row(&row)?;
        let block: i64 = row.try_get("funded_block")?;
        let target = ManagedRecoveryTarget {
            order,
            funding_operation_id: op.operation_id,
            funding_plan: op.plan,
            funding_tx: row.try_get("funded_tx_hash")?,
            funded_block: u64::try_from(block).map_err(|_| ManagedOrderStoreError::Integrity)?,
        };
        tx.commit().await?;
        Ok(target)
    }
    pub async fn managed_recovered_refund_plan(
        &self,
        id: Uuid,
    ) -> Result<Option<ManagedRefundPlan>> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, id).await?;
        let result = match refunds::stored(&mut tx, &order, RefundKind::RecoveredBounty).await {
            Ok((p, _)) => Some(p),
            Err(ManagedOrderStoreError::NotFound) => None,
            Err(e) => return Err(e),
        };
        tx.commit().await?;
        Ok(result)
    }
    /// A fresh verified proof is required even for a retry. The immutable identity is compared,
    /// never replaced on drift. A changed identity leaves a persistent reconciliation hold.
    pub async fn reserve_managed_recovered_refund(
        &self,
        proof: &RecoveredPrincipal,
    ) -> Result<ManagedRefundPlan> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, proof.order_id()).await?;
        eligible(&mut tx, &order).await?;
        if order.quote_digest != proof.quote_digest() {
            return Err(ManagedOrderStoreError::Integrity);
        }
        let identity = proof.identity_digest();
        let old: Option<String> = sqlx::query_scalar(
            "SELECT identity_digest FROM managed_recovered_refund_evidence WHERE order_id=$1",
        )
        .bind(order.order_id)
        .fetch_optional(&mut *tx)
        .await?;
        if old.as_ref().is_some_and(|old| old != &identity) {
            sqlx::query("UPDATE managed_recovered_refund_evidence SET reconciliation_required=true WHERE order_id=$1")
                .bind(order.order_id).execute(&mut *tx).await?;
            tx.commit().await?;
            return Err(ManagedOrderStoreError::Integrity);
        }
        sqlx::query("INSERT INTO managed_recovered_refund_evidence(order_id,identity_digest,evidence)
          VALUES($1,$2,$3) ON CONFLICT(order_id) DO UPDATE SET evidence=excluded.evidence,verified_at=clock_timestamp()
          WHERE NOT managed_recovered_refund_evidence.reconciliation_required")
          .bind(order.order_id).bind(identity).bind(serde_json::to_value(proof).map_err(|_|ManagedOrderStoreError::Integrity)?)
          .execute(&mut *tx).await?;
        require_fresh(&mut tx, &order).await?;
        match refunds::stored(&mut tx, &order, RefundKind::RecoveredBounty).await {
            Ok((plan, _)) => {
                tx.commit().await?;
                return Ok(plan);
            }
            Err(ManagedOrderStoreError::NotFound) => {}
            Err(e) => return Err(e),
        }
        let pi: String = sqlx::query_scalar(
            "SELECT payment_intent_id FROM managed_stripe_paid_receipts WHERE order_id=$1",
        )
        .bind(order.order_id)
        .fetch_one(&mut *tx)
        .await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let p = ManagedRefundPlan::recovered_bounty(
            &order.quote,
            order.order_id,
            Uuid::new_v4(),
            pi,
            now,
        )?;
        sqlx::query("INSERT INTO managed_recovered_refund_operations(order_id,operation_id,stripe_account_id,livemode,plan,plan_digest)
            VALUES($1,$2,$3,$4,$5,$6)")
            .bind(order.order_id).bind(p.operation_id()).bind(&p.merchant().stripe_account_id).bind(p.merchant().livemode)
            .bind(serde_json::to_value(&p).map_err(|_|ManagedOrderStoreError::Integrity)?).bind(p.digest()).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(p)
    }
    /// Indexed rows locate receipts only. They are not authority for recovery or finality.
    pub async fn managed_recovery_transactions(
        &self,
        target: &ManagedRecoveryTarget,
    ) -> Result<Vec<String>> {
        let rows=sqlx::query("SELECT kind,tx_hash FROM autonomous_bounty_events WHERE network=$1 AND contract_address=lower($2)
          AND (kind='bounty_cancelled' OR (kind='refund_withdrawn' AND data->>'contributor'=lower($3)))
          ORDER BY block_number,log_index LIMIT 3")
          .bind(&target.funding_plan.network).bind(&target.funding_plan.predicted_bounty).bind(&target.funding_plan.funding_wallet)
          .fetch_all(&self.pool).await?;
        if rows.len() != 2
            || rows[0].try_get::<String, _>("kind")? != "bounty_cancelled"
            || rows[1].try_get::<String, _>("kind")? != "refund_withdrawn"
        {
            return Err(OrderError::Conflict.into());
        }
        let mut hashes = vec![target.funding_tx.clone()];
        for row in rows {
            let hash: String = row.try_get("tx_hash")?;
            if !hashes.contains(&hash) {
                hashes.push(hash)
            }
        }
        Ok(hashes)
    }
    pub async fn claim_managed_recovered_refund(
        &self,
        id: Uuid,
    ) -> Result<Option<ManagedRefundLease>> {
        self.claim_refund(id, RefundKind::RecoveredBounty).await
    }
    /// Distinct projection: the historical funding status stays Funded.
    pub async fn managed_recovered_refund_state(&self, id: Uuid) -> Result<Value> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, id).await?;
        let result = match refunds::stored(&mut tx, &order, RefundKind::RecoveredBounty).await {
            Err(ManagedOrderStoreError::NotFound) => {
                serde_json::json!({"status":"awaiting_eligible_recovery"})
            }
            Err(e) => return Err(e),
            Ok((p, v)) => {
                let signed = refunds::signed_outcome(&mut tx, &p, &v).await?;
                let hold: bool = sqlx::query_scalar("SELECT
                    EXISTS(SELECT 1 FROM managed_recovered_refund_evidence WHERE order_id=$1 AND reconciliation_required)
                    OR EXISTS(SELECT 1 FROM managed_recovered_refund_execution WHERE order_id=$1 AND last_result IN ('reconciliation_required','configuration_error'))")
                    .bind(id).fetch_one(&mut *tx).await?;
                let status = match signed {
                    Some(outcome) => serde_json::to_value(outcome)
                        .map_err(|_| ManagedOrderStoreError::Integrity)?,
                    None if hold => serde_json::json!("reconciliation_required"),
                    None if v["refund_id"].is_null() => serde_json::json!("pending_binding"),
                    None => serde_json::json!("awaiting_signed_event"),
                };
                serde_json::json!({"status":status,"reconciliation_required":hold,
                "operation_id":p.operation_id(),"refund_id":v["refund_id"],"amount_minor":p.amount_minor(),"currency":p.currency(),
                "evidence_boundary":"Only authenticated Stripe events prove the card refund outcome."})
            }
        };
        tx.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
