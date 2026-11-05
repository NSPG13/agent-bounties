//! Original-payment refunds share the order lock with payment and funding transitions. The full
//! pre-funding refund and the review-deposit refund after payment run the same fenced execution
//! and signed-event reconciliation, each in its own tables.
use super::{stripe::locked_order, *};
use managed_orders::stripe_refund::{
    AuthenticatedRefundEvent, ManagedRefundBinding, ManagedRefundPlan, RefundEventData, RefundKind,
    RefundMerchant, RefundOutcome,
};
use payments_stripe::StripeRequestIntent;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

/// The tables of one refund kind. Names come only from this closed mapping.
struct Tables {
    operations: &'static str,
    events: &'static str,
    execution: &'static str,
}
fn tables(kind: RefundKind) -> Tables {
    match kind {
        RefundKind::Full => Tables {
            operations: "managed_refund_operations",
            events: "managed_refund_events",
            execution: "managed_refund_execution",
        },
        RefundKind::RecoveredBounty => Tables {
            operations: "managed_recovered_refund_operations",
            events: "managed_recovered_refund_events",
            execution: "managed_recovered_refund_execution",
        },
        RefundKind::ReviewDeposit => Tables {
            operations: "managed_deposit_refund_operations",
            events: "managed_deposit_refund_events",
            execution: "managed_deposit_refund_execution",
        },
    }
}
fn fingerprint(v: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(v).map_err(|_| ManagedOrderStoreError::Integrity)?,
    )))
}
fn conflict<T>() -> Result<T> {
    Err(OrderError::Conflict.into())
}
pub(super) fn plan(
    v: &Value,
    quote: &ManagedQuote,
    order_id: Uuid,
    accepted_at: i64,
    pi: &str,
    kind: RefundKind,
) -> Result<ManagedRefundPlan> {
    let p = ManagedRefundPlan::restore(v["plan"].clone(), quote, order_id, pi)
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    if p.kind() != kind
        || v["order_id"] != order_id.to_string()
        || v["operation_id"] != p.operation_id().to_string()
        || v["stripe_account_id"] != p.merchant().stripe_account_id
        || v["livemode"] != p.merchant().livemode
        || v["plan_digest"] != p.digest()
        || p.created_at() < accepted_at
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(p)
}
pub(super) fn event(v: &Value) -> Result<RefundEventData> {
    let e: RefundEventData = serde_json::from_value(v["event"].clone())
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    if v["event_fingerprint"] != fingerprint(&v["event"])?
        || v["event_id"] != e.event_id
        || v["stripe_account_id"] != e.stripe_account_id
        || v["livemode"] != e.livemode
        || v["order_id"] != e.order_id.to_string()
        || v["operation_id"] != e.operation_id.to_string()
        || v["event_created_at"].as_i64() != Some(e.event_created_at)
        || v["outcome"]
            != serde_json::to_value(e.outcome).map_err(|_| ManagedOrderStoreError::Integrity)?
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(e)
}
pub(super) fn status_from_row(row: &PgRow, quote: &ManagedQuote) -> Result<ManagedOrderStatus> {
    let bank: Option<String> = row.try_get("bank_action_state")?;
    let operation: Option<Value> = row.try_get("refund_operation")?;
    let evidence: Option<Value> = row.try_get("refund_evidence")?;
    let bank_funding: Option<Uuid> = row.try_get("bank_funding_operation_id")?;
    let funding: Option<Uuid> = row.try_get("funding_operation_id")?;
    let Some(v) = operation else {
        if bank.as_deref() != Some("held") || evidence.is_some() || bank_funding != funding {
            return Err(ManagedOrderStoreError::Integrity);
        }
        if funding.is_none() {
            return Ok(ManagedOrderStatus::AwaitingBankSettlement);
        }
        return match row
            .try_get::<Option<String>, _>("funding_outcome")?
            .as_deref()
        {
            Some("reserved") => Ok(ManagedOrderStatus::FundingReserved),
            Some("broadcast") => Ok(ManagedOrderStatus::FundingBroadcast),
            Some("funded") => Ok(ManagedOrderStatus::Funded),
            Some("reconciliation_required") => {
                Ok(ManagedOrderStatus::FundingReconciliationRequired)
            }
            _ => Err(ManagedOrderStoreError::Integrity),
        };
    };
    if bank.as_deref() != Some("refund_reserved") || bank_funding.is_some() || funding.is_some() {
        return Err(ManagedOrderStoreError::Integrity);
    }
    let pi: String = row.try_get("paid_intent")?;
    let p = plan(
        &v,
        quote,
        row.try_get("order_id")?,
        row.try_get("accepted_at")?,
        &pi,
        RefundKind::Full,
    )?;
    if v["refund_id"].is_null() {
        if evidence.is_some() || !v["charge_id"].is_null() {
            return Err(ManagedOrderStoreError::Integrity);
        }
        return Ok(ManagedOrderStatus::RefundRequested);
    }
    let Some(v_event) = evidence else {
        return Ok(ManagedOrderStatus::RefundPending);
    };
    let e = event(&v_event)?;
    if !p.matches_event(&e) || v["refund_id"] != e.refund_id || v["charge_id"] != e.charge_id {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(match e.outcome {
        RefundOutcome::Pending => ManagedOrderStatus::RefundPending,
        RefundOutcome::RequiresAction => ManagedOrderStatus::RefundActionRequired,
        RefundOutcome::Succeeded => ManagedOrderStatus::RefundSent,
        RefundOutcome::Failed | RefundOutcome::Canceled => ManagedOrderStatus::RefundFailed,
    })
}
pub(super) async fn stored(
    tx: &mut Transaction<'_, Postgres>,
    order: &ManagedOrder,
    kind: RefundKind,
) -> Result<(ManagedRefundPlan, Value)> {
    let v: Value = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(f) FROM {} f WHERE order_id=$1",
        tables(kind).operations
    ))
    .bind(order.order_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ManagedOrderStoreError::NotFound)?;
    let pi: String = sqlx::query_scalar(
        "SELECT payment_intent_id FROM managed_stripe_paid_receipts WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok((
        plan(
            &v,
            &order.quote,
            order.order_id,
            order.accepted_at,
            &pi,
            kind,
        )?,
        v,
    ))
}

/// Private execution fence, never constructible from customer JSON.
pub struct ManagedRefundLease {
    plan: ManagedRefundPlan,
    token: Uuid,
    known_refund: Option<String>,
    previously_attempted: bool,
}
impl ManagedRefundLease {
    pub fn plan(&self) -> &ManagedRefundPlan {
        &self.plan
    }
    pub fn known_refund(&self) -> Option<&str> {
        self.known_refund.as_deref()
    }
    pub fn previously_attempted(&self) -> bool {
        self.previously_attempted
    }
}
async fn fence(
    tx: &mut Transaction<'_, Postgres>,
    lease: &ManagedRefundLease,
) -> Result<ManagedOrder> {
    let order = locked_order(tx, lease.plan.order_id()).await?;
    let valid: bool = sqlx::query_scalar(&format!(
        "SELECT EXISTS(SELECT 1 FROM {} WHERE order_id=$1 AND lease_token=$2 AND lease_until>clock_timestamp())",
        tables(lease.plan.kind()).execution
    ))
    .bind(order.order_id)
    .bind(lease.token)
    .fetch_one(&mut **tx)
    .await?;
    if !valid {
        return Err(ManagedOrderStoreError::StaleLease);
    }
    if stored(tx, &order, lease.plan.kind()).await?.0 != lease.plan {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(order)
}
async fn process_event(
    tx: &mut Transaction<'_, Postgres>,
    p: &ManagedRefundPlan,
    v: &Value,
    e: &RefundEventData,
) -> Result<ManagedEventDisposition> {
    let disposition = if !p.matches_event(e) {
        ManagedEventDisposition::Quarantined
    } else if v["refund_id"].is_null() {
        ManagedEventDisposition::PendingBinding
    } else if v["refund_id"] == e.refund_id && v["charge_id"] == e.charge_id {
        ManagedEventDisposition::Processed
    } else {
        ManagedEventDisposition::Quarantined
    };
    let label = match disposition {
        ManagedEventDisposition::PendingBinding => "pending_binding",
        ManagedEventDisposition::Processed => "processed",
        _ => "quarantined",
    };
    sqlx::query(&format!(
        "UPDATE {} SET disposition=$4 WHERE stripe_account_id=$1 AND livemode=$2 AND event_id=$3",
        tables(p.kind()).events
    ))
    .bind(&e.stripe_account_id)
    .bind(e.livemode)
    .bind(&e.event_id)
    .bind(label)
    .execute(&mut **tx)
    .await?;
    Ok(disposition)
}

impl PostgresStore {
    /// Authenticated buyer principal only. New requests reserve the complete paid order before
    /// provider I/O. This reservation never returns to funding, even if the refund later fails.
    pub async fn request_managed_refund(
        &self,
        buyer: &str,
        order_id: Uuid,
    ) -> Result<ManagedOrder> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if order.quote.input().buyer_principal != buyer {
            return Err(ManagedOrderStoreError::NotFound);
        }
        let existing: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_refund_operations WHERE order_id=$1)",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        if existing {
            stored(&mut tx, &order, RefundKind::Full).await?;
            tx.commit().await?;
            return Ok(order);
        }
        if order.status != ManagedOrderStatus::AwaitingBankSettlement {
            return conflict();
        }
        let pi: String = sqlx::query_scalar(
            "SELECT payment_intent_id FROM managed_stripe_paid_receipts WHERE order_id=$1",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let p = ManagedRefundPlan::new(&order.quote, order_id, Uuid::new_v4(), pi, now)?;
        let changed=sqlx::query("UPDATE managed_stripe_bank_actions SET state='refund_reserved' WHERE order_id=$1 AND state='held'")
            .bind(order_id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return conflict();
        }
        sqlx::query("INSERT INTO managed_refund_operations(order_id,operation_id,stripe_account_id,livemode,plan,plan_digest) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(order_id).bind(p.operation_id()).bind(&p.merchant().stripe_account_id).bind(p.merchant().livemode)
            .bind(serde_json::to_value(&p).map_err(|_|ManagedOrderStoreError::Integrity)?).bind(p.digest()).execute(&mut *tx).await?;
        let order = locked_order(&mut tx, order_id).await?;
        tx.commit().await?;
        Ok(order)
    }
    pub async fn get_managed_refund_plan(
        &self,
        buyer: &str,
        order_id: Uuid,
    ) -> Result<Option<ManagedRefundPlan>> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if order.quote.input().buyer_principal != buyer {
            return Err(ManagedOrderStoreError::NotFound);
        }
        let result = match stored(&mut tx, &order, RefundKind::Full).await {
            Ok((p, _)) => Some(p),
            Err(ManagedOrderStoreError::NotFound) => None,
            Err(e) => return Err(e),
        };
        tx.commit().await?;
        Ok(result)
    }
    pub async fn claim_managed_refund(&self, order_id: Uuid) -> Result<Option<ManagedRefundLease>> {
        self.claim_refund(order_id, RefundKind::Full).await
    }
    pub async fn claim_managed_deposit_refund(
        &self,
        order_id: Uuid,
    ) -> Result<Option<ManagedRefundLease>> {
        self.claim_refund(order_id, RefundKind::ReviewDeposit).await
    }
    pub(super) async fn claim_refund(
        &self,
        order_id: Uuid,
        kind: RefundKind,
    ) -> Result<Option<ManagedRefundLease>> {
        let t = tables(kind);
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        let (p, v) = stored(&mut tx, &order, kind).await?;
        let mut known = v["refund_id"].as_str().map(str::to_owned);
        if known.is_none() {
            let pending: Vec<Value> = sqlx::query_scalar(&format!(
                "SELECT to_jsonb(e) FROM {} e WHERE order_id=$1 AND disposition='pending_binding' LIMIT 101",
                t.events
            ))
            .bind(order_id)
            .fetch_all(&mut *tx)
            .await?;
            if pending.len() > 100 {
                return Err(ManagedOrderStoreError::Integrity);
            }
            for v in pending {
                let e = event(&v)?;
                if !p.matches_event(&e) || known.as_ref().is_some_and(|id| id != &e.refund_id) {
                    return conflict();
                }
                known = Some(e.refund_id);
            }
        }
        sqlx::query(&format!(
            "INSERT INTO {}(order_id) VALUES($1) ON CONFLICT DO NOTHING",
            t.execution
        ))
        .bind(order_id)
        .execute(&mut *tx)
        .await?;
        let token = Uuid::new_v4();
        let attempts: Option<i64> = sqlx::query_scalar(&format!(
            "UPDATE {} SET lease_token=$2,lease_until=clock_timestamp()+interval '180 seconds'
            WHERE order_id=$1 AND (lease_until IS NULL OR lease_until<=clock_timestamp()) AND next_attempt_at<=clock_timestamp() RETURNING create_attempts",
            t.execution
        ))
        .bind(order_id)
        .bind(token)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(attempts.map(|a| ManagedRefundLease {
            plan: p,
            token,
            known_refund: known,
            previously_attempted: a > 0,
        }))
    }
    pub async fn begin_managed_refund_create(
        &self,
        lease: &ManagedRefundLease,
        merchant: &RefundMerchant,
    ) -> Result<StripeRequestIntent> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        if lease.plan.kind() == RefundKind::RecoveredBounty {
            super::recovered::require_fresh(&mut tx, &order).await?;
        }
        let t = tables(lease.plan.kind());
        let (_, v) = stored(&mut tx, &order, lease.plan.kind()).await?;
        if !v["refund_id"].is_null() {
            return conflict();
        }
        let pending: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS(SELECT 1 FROM {} WHERE order_id=$1 AND disposition='pending_binding')",
            t.events
        ))
        .bind(order.order_id)
        .fetch_one(&mut *tx)
        .await?;
        if lease.known_refund.is_some() || pending {
            return Err(managed_orders::stripe_refund::RefundError::ReconciliationRequired.into());
        }
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let request = lease.plan.request(now + 15, merchant)?;
        sqlx::query(&format!(
            "UPDATE {} SET create_attempts=create_attempts+1 WHERE order_id=$1",
            t.execution
        ))
        .bind(order.order_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }
    pub async fn complete_managed_refund(
        &self,
        lease: &ManagedRefundLease,
        binding: &ManagedRefundBinding,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        let t = tables(lease.plan.kind());
        let (p, mut v) = stored(&mut tx, &order, lease.plan.kind()).await?;
        if binding.operation_id() != p.operation_id()
            || binding.plan_digest() != p.digest()
            || v["refund_id"]
                .as_str()
                .is_some_and(|s| s != binding.refund_id())
            || v["charge_id"]
                .as_str()
                .is_some_and(|s| s != binding.charge_id())
        {
            return conflict();
        }
        sqlx::query(&format!(
            "UPDATE {} SET refund_id=$2,charge_id=$3 WHERE order_id=$1",
            t.operations
        ))
        .bind(order.order_id)
        .bind(binding.refund_id())
        .bind(binding.charge_id())
        .execute(&mut *tx)
        .await?;
        v["refund_id"] = binding.refund_id().into();
        v["charge_id"] = binding.charge_id().into();
        let pending: Vec<Value> = sqlx::query_scalar(&format!(
            "SELECT to_jsonb(e) FROM {} e WHERE order_id=$1 AND disposition='pending_binding' LIMIT 101",
            t.events
        ))
        .bind(order.order_id)
        .fetch_all(&mut *tx)
        .await?;
        if pending.len() > 100 {
            return Err(ManagedOrderStoreError::Integrity);
        }
        for raw in pending {
            process_event(&mut tx, &p, &v, &event(&raw)?).await?;
        }
        release(&mut tx, order.order_id, lease.plan.kind(), "bound").await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn release_managed_refund(
        &self,
        lease: &ManagedRefundLease,
        result: ManagedCheckoutRunResult,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        let label = match result {
            ManagedCheckoutRunResult::Retry => "retry",
            ManagedCheckoutRunResult::ReconciliationRequired => "reconciliation_required",
            ManagedCheckoutRunResult::ConfigurationError => "configuration_error",
        };
        release(&mut tx, order.order_id, lease.plan.kind(), label).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn reconcile_managed_refund_event(
        &self,
        authenticated: &AuthenticatedRefundEvent,
    ) -> Result<ManagedEventDisposition> {
        let e = authenticated.data();
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, e.order_id).await?;
        let t = tables(e.kind);
        let (p, v) = stored(&mut tx, &order, e.kind).await?;
        // Unrelated operation IDs cannot poison an order's inbox or satisfy its foreign keys.
        if e.operation_id != p.operation_id()
            || e.stripe_account_id != p.merchant().stripe_account_id
            || e.livemode != p.merchant().livemode
        {
            return conflict();
        }
        let body = serde_json::to_value(e).map_err(|_| ManagedOrderStoreError::Integrity)?;
        let outcome =
            serde_json::to_value(e.outcome).map_err(|_| ManagedOrderStoreError::Integrity)?;
        let inserted=sqlx::query(&format!("INSERT INTO {}(stripe_account_id,livemode,event_id,order_id,operation_id,event,event_fingerprint,event_created_at,outcome,disposition)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'pending_binding') ON CONFLICT DO NOTHING", t.events))
            .bind(&e.stripe_account_id).bind(e.livemode).bind(&e.event_id).bind(e.order_id).bind(e.operation_id)
            .bind(&body).bind(fingerprint(&body)?).bind(e.event_created_at).bind(outcome.as_str().ok_or(ManagedOrderStoreError::Integrity)?)
            .execute(&mut *tx).await?.rows_affected();
        if inserted == 0 {
            let old:Value=sqlx::query_scalar(&format!("SELECT to_jsonb(e) FROM {} e WHERE stripe_account_id=$1 AND livemode=$2 AND event_id=$3", t.events))
                .bind(&e.stripe_account_id).bind(e.livemode).bind(&e.event_id).fetch_one(&mut *tx).await?;
            if event(&old)? != *e {
                return conflict();
            }
            if old["disposition"] != "pending_binding" {
                tx.commit().await?;
                return Ok(ManagedEventDisposition::Duplicate);
            }
        }
        let result = process_event(&mut tx, &p, &v, e).await?;
        tx.commit().await?;
        Ok(result)
    }
}
/// Why a paid card bounty's review deposit is or is not being refunded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepositRefundReservation {
    /// The quote has no review deposit line.
    NoDeposit,
    /// The order is not funded, or no confirmed canonical `BountySettled` exists yet.
    NotSettled,
    /// The arbiter reversed a rejection: the deposit belongs to the solver.
    Forfeited,
    /// A card dispute exists: the money is already contested through the card network.
    Disputed,
    /// The refund of exactly the deposit line is reserved (possibly earlier).
    Reserved(Box<ManagedRefundPlan>),
}

/// The customer-facing state of the review deposit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepositState {
    NoDeposit,
    Held,
    RefundPending,
    Refunded,
    RefundFailed,
    ForfeitedToSolver,
}
impl DepositState {
    pub fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::NoDeposit => return None,
            Self::Held => "held",
            Self::RefundPending => "refund_pending",
            Self::Refunded => "refunded",
            Self::RefundFailed => "refund_failed",
            Self::ForfeitedToSolver => "forfeited_to_solver",
        })
    }
}

/// The refund status its signed events prove: a failure or cancellation outranks success,
/// which outranks earlier states, as for the order's own refund status.
pub(super) async fn signed_outcome(
    tx: &mut Transaction<'_, Postgres>,
    p: &ManagedRefundPlan,
    v: &Value,
) -> Result<Option<RefundOutcome>> {
    let evidence: Option<Value> = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(e) FROM {} e WHERE e.order_id=$1 AND e.disposition='processed'
          ORDER BY CASE e.outcome WHEN 'failed' THEN 4 WHEN 'canceled' THEN 4 WHEN 'succeeded' THEN 3 ELSE 0 END DESC,
                   e.event_created_at DESC, e.event_id DESC LIMIT 1",
        tables(p.kind()).events
    ))
    .bind(p.order_id())
    .fetch_optional(&mut **tx)
    .await?;
    let Some(raw) = evidence else {
        return Ok(None);
    };
    let e = event(&raw)?;
    if !p.matches_event(&e) || v["refund_id"] != e.refund_id {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(Some(e.outcome))
}

fn kind_label(kind: RefundKind) -> &'static str {
    match kind {
        RefundKind::Full => "full",
        RefundKind::ReviewDeposit => "review_deposit",
        RefundKind::RecoveredBounty => "recovered_bounty",
    }
}

fn outcome_label(outcome: RefundOutcome) -> Result<String> {
    serde_json::to_value(outcome)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(ManagedOrderStoreError::Integrity)
}

/// A card refund bound to Stripe, and the status its signed events prove.
#[derive(Debug, Clone)]
pub struct RefundObservationTarget {
    pub plan: ManagedRefundPlan,
    pub refund_id: String,
    pub signed: Option<RefundOutcome>,
}

/// A refund whose latest Stripe read differs from what its signed events prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundDrift {
    pub kind: RefundKind,
    pub order_id: Uuid,
    pub refund_id: String,
    pub stripe_status: RefundOutcome,
    pub signed_status: Option<RefundOutcome>,
    pub observed_at: i64,
}

async fn deposit_forfeited(tx: &mut Transaction<'_, Postgres>, order_id: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_customer_reviews
          WHERE order_id=$1 AND ruling IS NOT NULL AND (ruling->>'passed')::boolean)",
    )
    .bind(order_id)
    .fetch_one(&mut **tx)
    .await?)
}

fn has_deposit(order: &ManagedOrder) -> bool {
    order
        .quote
        .input()
        .additional_costs
        .iter()
        .any(|cost| cost.code == managed_orders::pricing::REVIEW_DEPOSIT_CODE)
}

impl PostgresStore {
    /// Reserve the refund of a card bounty's review deposit once the bounty is paid through a
    /// confirmed canonical `BountySettled` and no rejection was reversed. Idempotent.
    pub async fn reserve_managed_deposit_refund(
        &self,
        order_id: Uuid,
    ) -> Result<DepositRefundReservation> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        match stored(&mut tx, &order, RefundKind::ReviewDeposit).await {
            Ok((p, _)) => {
                tx.commit().await?;
                return Ok(DepositRefundReservation::Reserved(Box::new(p)));
            }
            Err(ManagedOrderStoreError::NotFound) => {}
            Err(e) => return Err(e),
        }
        if !has_deposit(&order) {
            return Ok(DepositRefundReservation::NoDeposit);
        }
        if order.status != ManagedOrderStatus::Funded {
            return Ok(DepositRefundReservation::NotSettled);
        }
        if deposit_forfeited(&mut tx, order_id).await? {
            return Ok(DepositRefundReservation::Forfeited);
        }
        let disputed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM managed_stripe_disputes WHERE order_id=$1)",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        if disputed {
            return Ok(DepositRefundReservation::Disputed);
        }
        let bounty = &order.quote.input().bounty;
        let settled: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM autonomous_bounty_events
              WHERE network=$1 AND contract_address=lower($2) AND kind='bounty_settled'
                AND block_time_verified)",
        )
        .bind(&bounty.network)
        .bind(&bounty.predicted_bounty)
        .fetch_one(&mut *tx)
        .await?;
        if !settled {
            return Ok(DepositRefundReservation::NotSettled);
        }
        let pi: String = sqlx::query_scalar(
            "SELECT payment_intent_id FROM managed_stripe_paid_receipts WHERE order_id=$1",
        )
        .bind(order_id)
        .fetch_one(&mut *tx)
        .await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let p = ManagedRefundPlan::review_deposit(&order.quote, order_id, Uuid::new_v4(), pi, now)?;
        sqlx::query("INSERT INTO managed_deposit_refund_operations(order_id,operation_id,stripe_account_id,livemode,plan,plan_digest) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(order_id).bind(p.operation_id()).bind(&p.merchant().stripe_account_id).bind(p.merchant().livemode)
            .bind(serde_json::to_value(&p).map_err(|_| ManagedOrderStoreError::Integrity)?).bind(p.digest())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(DepositRefundReservation::Reserved(Box::new(p)))
    }

    /// The review deposit's state, from the order, its rulings and signed refund events only.
    pub async fn managed_deposit_state(&self, order_id: Uuid) -> Result<DepositState> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        let state = if !has_deposit(&order) {
            DepositState::NoDeposit
        } else if deposit_forfeited(&mut tx, order_id).await? {
            DepositState::ForfeitedToSolver
        } else {
            match stored(&mut tx, &order, RefundKind::ReviewDeposit).await {
                Err(ManagedOrderStoreError::NotFound) => DepositState::Held,
                Err(e) => return Err(e),
                Ok((_, v)) if v["refund_id"].is_null() => DepositState::RefundPending,
                Ok((p, v)) => match signed_outcome(&mut tx, &p, &v).await? {
                    Some(RefundOutcome::Succeeded) => DepositState::Refunded,
                    Some(RefundOutcome::Failed | RefundOutcome::Canceled) => {
                        DepositState::RefundFailed
                    }
                    _ => DepositState::RefundPending,
                },
            }
        };
        tx.commit().await?;
        Ok(state)
    }

    /// Claim a bounded, durable rotation per refund kind. Attempts are separate from successful
    /// observations and signed refund state. No lock is held during provider I/O.
    pub async fn claim_managed_refunds_to_observe(
        &self,
        max_age_seconds: i64,
        limit: i64,
        retry_after_seconds: i64,
    ) -> Result<Vec<(Uuid, RefundKind)>> {
        let mut targets = vec![];
        for kind in [
            RefundKind::Full,
            RefundKind::ReviewDeposit,
            RefundKind::RecoveredBounty,
        ] {
            let ids: Vec<Uuid> = sqlx::query_scalar(&format!(
                "WITH candidates AS (
                   SELECT order_id, last_observation_attempt_at FROM {table}
                   WHERE refund_id IS NOT NULL
                     AND (plan->>'created_at')::bigint
                         > floor(extract(epoch FROM statement_timestamp()))::bigint - $1
                     AND (last_observation_attempt_at IS NULL
                       OR last_observation_attempt_at <= statement_timestamp() - make_interval(secs => $3))
                   ORDER BY last_observation_attempt_at NULLS FIRST, order_id
                   LIMIT $2 FOR UPDATE SKIP LOCKED
                 ), claimed AS (
                   UPDATE {table} r SET last_observation_attempt_at=statement_timestamp()
                   FROM candidates c WHERE r.order_id=c.order_id RETURNING r.order_id
                 ) SELECT claimed.order_id FROM claimed JOIN candidates c USING (order_id)
                   ORDER BY c.last_observation_attempt_at NULLS FIRST, c.order_id",
                table = tables(kind).operations
            ))
            .bind(max_age_seconds.max(0))
            .bind(limit.clamp(1, 200))
            .bind(retry_after_seconds.max(0) as f64)
            .fetch_all(&self.pool)
            .await?;
            targets.extend(ids.into_iter().map(|id| (id, kind)));
        }
        Ok(targets)
    }

    /// Load one claimed target independently, so a malformed record cannot abort its batch.
    pub async fn managed_refund_observation_target(
        &self,
        order_id: Uuid,
        kind: RefundKind,
    ) -> Result<RefundObservationTarget> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        let (plan, v) = stored(&mut tx, &order, kind).await?;
        let signed = signed_outcome(&mut tx, &plan, &v).await?;
        tx.commit().await?;
        let refund_id = v["refund_id"]
            .as_str()
            .ok_or(ManagedOrderStoreError::Integrity)?
            .to_owned();
        Ok(RefundObservationTarget {
            plan,
            refund_id,
            signed,
        })
    }

    /// Record the latest Stripe read of a refund beside its signed status. Never changes it.
    pub async fn record_managed_refund_observation(
        &self,
        target: &RefundObservationTarget,
        stripe_status: RefundOutcome,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO managed_refund_observations
               (kind, order_id, refund_id, stripe_status, signed_status, refund_created_at)
             VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (kind, order_id) DO UPDATE SET refund_id=EXCLUDED.refund_id,
               stripe_status=EXCLUDED.stripe_status, signed_status=EXCLUDED.signed_status,
               refund_created_at=EXCLUDED.refund_created_at, observed_at=clock_timestamp()",
        )
        .bind(kind_label(target.plan.kind()))
        .bind(target.plan.order_id())
        .bind(&target.refund_id)
        .bind(outcome_label(stripe_status)?)
        .bind(target.signed.map(outcome_label).transpose()?)
        .bind(target.plan.created_at())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Refunds whose latest Stripe read differs from their signed status, once a refund is old
    /// enough that its events should have arrived.
    pub async fn list_managed_refund_drift(&self, settle_seconds: i64) -> Result<Vec<RefundDrift>> {
        let rows = sqlx::query(
            "SELECT kind, order_id, refund_id, stripe_status, signed_status,
                    floor(extract(epoch FROM observed_at))::bigint AS observed
               FROM managed_refund_observations
              WHERE signed_status IS DISTINCT FROM stripe_status
                AND refund_created_at < floor(extract(epoch FROM clock_timestamp()))::bigint - $1
              ORDER BY observed_at, order_id LIMIT 200",
        )
        .bind(settle_seconds.max(0))
        .fetch_all(&self.pool)
        .await?;
        let outcome = |value: String| {
            serde_json::from_value::<RefundOutcome>(Value::String(value))
                .map_err(|_| ManagedOrderStoreError::Integrity)
        };
        rows.iter()
            .map(|row| {
                Ok(RefundDrift {
                    kind: match row.try_get::<String, _>("kind")?.as_str() {
                        "full" => RefundKind::Full,
                        "review_deposit" => RefundKind::ReviewDeposit,
                        "recovered_bounty" => RefundKind::RecoveredBounty,
                        _ => return Err(ManagedOrderStoreError::Integrity),
                    },
                    order_id: row.try_get("order_id")?,
                    refund_id: row.try_get("refund_id")?,
                    stripe_status: outcome(row.try_get("stripe_status")?)?,
                    signed_status: row
                        .try_get::<Option<String>, _>("signed_status")?
                        .map(outcome)
                        .transpose()?,
                    observed_at: row.try_get("observed")?,
                })
            })
            .collect()
    }

    /// Reserved deposit refunds not yet bound to a Stripe refund, for the executor.
    pub async fn list_unbound_managed_deposit_refunds(&self, limit: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar(
            "SELECT f.order_id FROM managed_deposit_refund_operations f
               LEFT JOIN managed_deposit_refund_execution x ON x.order_id=f.order_id
              WHERE f.refund_id IS NULL
                AND (x.order_id IS NULL OR (x.next_attempt_at<=clock_timestamp()
                     AND (x.last_result IS NULL OR x.last_result='retry')))
              ORDER BY f.order_id LIMIT $1",
        )
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?)
    }
}

async fn release(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
    kind: RefundKind,
    result: &str,
) -> Result<()> {
    sqlx::query(&format!("UPDATE {} SET lease_token=NULL,lease_until=NULL,last_result=$2,next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE order_id=$1", tables(kind).execution))
        .bind(order_id).bind(result).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
pub(super) mod tests;
