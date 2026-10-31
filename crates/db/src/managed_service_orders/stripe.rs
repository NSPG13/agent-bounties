use super::*;
use managed_orders::{
    stripe_checkout::{
        AuthenticatedManagedEvent, CheckoutPolicy, ManagedCheckoutPlan, ManagedEventData,
        ManagedPaymentOutcome, ManagedSessionBinding,
    },
    FiatCurrency,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Transaction};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedEventDisposition {
    PendingBinding,
    Processed,
    Quarantined,
    Duplicate,
}

fn currency(currency: FiatCurrency) -> &'static str {
    match currency {
        FiatCurrency::Mxn => "mxn",
        FiatCurrency::Usd => "usd",
    }
}
fn conflict<T>() -> Result<T> {
    Err(OrderError::Conflict.into())
}

fn outcome_name(outcome: ManagedPaymentOutcome) -> &'static str {
    match outcome {
        ManagedPaymentOutcome::Pending => "pending",
        ManagedPaymentOutcome::Paid => "paid",
        ManagedPaymentOutcome::Failed => "failed",
        ManagedPaymentOutcome::Expired => "expired",
    }
}
fn fingerprint(value: &Value) -> Result<String> {
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(value).map_err(|_| ManagedOrderStoreError::Integrity)?,
    )))
}
pub(super) fn stored_event(row: &PgRow) -> Result<ManagedEventData> {
    let value: Value = row.try_get("event")?;
    let e: ManagedEventData =
        serde_json::from_value(value.clone()).map_err(|_| ManagedOrderStoreError::Integrity)?;
    if fingerprint(&value)? != row.try_get::<String, _>("event_fingerprint")?
        || e.event_id != row.try_get::<String, _>("event_id")?
        || e.order_id != row.try_get::<Uuid, _>("order_id")?
        || e.operation_id != row.try_get::<Uuid, _>("operation_id")?
        || e.stripe_account_id != row.try_get::<String, _>("stripe_account_id")?
        || e.livemode != row.try_get::<bool, _>("livemode")?
        || e.body_sha256 != row.try_get::<String, _>("body_sha256")?
        || e.event_created_at != row.try_get::<i64, _>("event_created_at")?
        || outcome_name(e.outcome) != row.try_get::<String, _>("outcome")?
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(e)
}

pub(super) fn status_from_row(row: &PgRow, quote: &ManagedQuote) -> Result<ManagedOrderStatus> {
    if row.try_get::<Option<String>, _>("paid_intent")?.is_some() {
        if row.try_get::<Option<String>, _>("paid_intent")?
            != row.try_get::<Option<String>, _>("bound_intent")?
            || row.try_get::<Option<String>, _>("paid_session")?
                != row.try_get::<Option<String>, _>("stripe_session_id")?
            || row.try_get::<Option<String>, _>("paid_account")?.as_deref()
                != Some(&quote.input().stripe_account_id)
            || row.try_get::<Option<bool>, _>("paid_mode")? != Some(quote.input().livemode)
            || row.try_get::<Option<String>, _>("paid_digest")?.as_deref()
                != Some(quote.digest()?.as_str())
            || row.try_get::<Option<i64>, _>("paid_amount")? != Some(quote.total_minor() as i64)
            || row
                .try_get::<Option<String>, _>("paid_currency")?
                .as_deref()
                != Some(currency(quote.input().currency))
        {
            return Err(ManagedOrderStoreError::Integrity);
        }
        return super::refunds::status_from_row(row, quote);
    }
    if row
        .try_get::<Option<serde_json::Value>, _>("refund_operation")?
        .is_some()
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    match row
        .try_get::<Option<String>, _>("latest_outcome")?
        .as_deref()
    {
        None => Ok(ManagedOrderStatus::AwaitingPayment),
        Some("pending") => Ok(ManagedOrderStatus::PaymentPending),
        Some("failed") => Ok(ManagedOrderStatus::PaymentFailed),
        Some("expired") => Ok(ManagedOrderStatus::PaymentExpired),
        _ => Err(ManagedOrderStoreError::Integrity),
    }
}

pub(super) fn plan_from_row(row: &PgRow, order: &ManagedOrder) -> Result<ManagedCheckoutPlan> {
    let plan = ManagedCheckoutPlan::restore(row.try_get("plan")?, &order.quote, order.order_id)
        .map_err(|_| ManagedOrderStoreError::Integrity)?;
    if plan.digest() != row.try_get::<String, _>("plan_digest")?
        || plan.created_at() < order.accepted_at
        || plan.operation_id() != row.try_get::<Uuid, _>("operation_id")?
        || plan.policy().stripe_account_id != row.try_get::<String, _>("stripe_account_id")?
        || plan.policy().livemode != row.try_get::<bool, _>("livemode")?
    {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(plan)
}

pub(super) async fn locked_order(
    tx: &mut Transaction<'_, Postgres>,
    order_id: Uuid,
) -> Result<ManagedOrder> {
    // A joined SELECT begun before a lock wait can retain stale sibling-table rows even
    // after acquiring the order lock. Lock first, then hydrate under a fresh READ COMMITTED
    // statement snapshot. All money transitions serialize on this same order row.
    sqlx::query("SELECT order_id FROM managed_service_orders WHERE order_id=$1 FOR UPDATE")
        .bind(order_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ManagedOrderStoreError::NotFound)?;
    let row = sqlx::query(&format!("{ORDER_SELECT} WHERE o.order_id=$1"))
        .bind(order_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ManagedOrderStoreError::NotFound)?;
    order_from_row(&row)
}

fn event_matches(
    event: &ManagedEventData,
    order: &ManagedOrder,
    plan: &ManagedCheckoutPlan,
) -> bool {
    event.order_id == order.order_id
        && event.operation_id == plan.operation_id()
        && event.quote_digest == order.quote_digest
        && event.stripe_account_id == plan.policy().stripe_account_id
        && event.livemode == plan.policy().livemode
        && event.amount_minor == order.quote.total_minor()
        && event.currency == order.quote.input().currency
        && event.event_created_at >= plan.created_at()
}

async fn process_event(
    tx: &mut Transaction<'_, Postgres>,
    event: &ManagedEventData,
    order: &ManagedOrder,
    plan: &ManagedCheckoutPlan,
) -> Result<ManagedEventDisposition> {
    let row = sqlx::query("SELECT session_id,payment_intent_id FROM managed_stripe_checkout_operations WHERE order_id=$1")
        .bind(order.order_id).fetch_one(&mut **tx).await?;
    let Some(session_id) = row.try_get::<Option<String>, _>("session_id")? else {
        return Ok(ManagedEventDisposition::PendingBinding);
    };
    let bound_intent: Option<String> = row.try_get("payment_intent_id")?;
    let conflicting_intent =
        matches!((&bound_intent, &event.payment_intent_id), (Some(a), Some(b)) if a != b);
    let disposition = if !event_matches(event, order, plan)
        || event.session_id != session_id
        || conflicting_intent
    {
        ManagedEventDisposition::Quarantined
    } else {
        if let Some(intent) = &event.payment_intent_id {
            sqlx::query("UPDATE managed_stripe_checkout_operations SET payment_intent_id=COALESCE(payment_intent_id,$2) WHERE order_id=$1")
                .bind(order.order_id).bind(intent).execute(&mut **tx).await?;
        }
        if event.outcome == ManagedPaymentOutcome::Paid {
            let intent = event
                .payment_intent_id
                .as_ref()
                .ok_or(ManagedOrderStoreError::Integrity)?;
            let inserted = sqlx::query("INSERT INTO managed_stripe_paid_receipts
                (order_id,stripe_account_id,livemode,payment_intent_id,session_id,quote_digest,amount_minor,currency)
                VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT DO NOTHING")
                .bind(order.order_id).bind(&event.stripe_account_id).bind(event.livemode).bind(intent)
                .bind(&event.session_id).bind(&event.quote_digest).bind(event.amount_minor as i64).bind(currency(event.currency))
                .execute(&mut **tx).await?.rows_affected();
            if inserted == 0 {
                let same: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM managed_stripe_paid_receipts
                    WHERE order_id=$1 AND stripe_account_id=$2 AND livemode=$3 AND payment_intent_id=$4
                    AND session_id=$5 AND quote_digest=$6 AND amount_minor=$7 AND currency=$8)")
                    .bind(order.order_id).bind(&event.stripe_account_id).bind(event.livemode).bind(intent)
                    .bind(&event.session_id).bind(&event.quote_digest).bind(event.amount_minor as i64).bind(currency(event.currency))
                    .fetch_one(&mut **tx).await?;
                if !same {
                    return conflict();
                }
            }
            sqlx::query("INSERT INTO managed_stripe_bank_actions(order_id,action,state) VALUES($1,'reconcile_bank_cash','held') ON CONFLICT DO NOTHING")
                .bind(order.order_id).execute(&mut **tx).await?;
        }
        ManagedEventDisposition::Processed
    };
    sqlx::query("UPDATE managed_stripe_event_inbox SET disposition=$4 WHERE stripe_account_id=$1 AND livemode=$2 AND event_id=$3")
        .bind(&event.stripe_account_id).bind(event.livemode).bind(&event.event_id)
        .bind(if disposition == ManagedEventDisposition::Processed { "processed" } else { "quarantined" })
        .execute(&mut **tx).await?;
    Ok(disposition)
}

impl PostgresStore {
    /// Owner-scoped lookup; never creates or changes an operation during readback.
    pub async fn get_managed_checkout_plan(
        &self,
        buyer: &str,
        order_id: Uuid,
    ) -> Result<Option<ManagedCheckoutPlan>> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if order.quote.input().buyer_principal != buyer {
            return Err(ManagedOrderStoreError::NotFound);
        }
        let row = sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await?;
        let plan = row
            .as_ref()
            .map(|row| plan_from_row(row, &order))
            .transpose()?;
        tx.commit().await?;
        Ok(plan)
    }

    /// Persist before any Stripe request. This internal operation grants no execution authority.
    pub async fn prepare_managed_checkout(
        &self,
        buyer: &str,
        order_id: Uuid,
        policy: CheckoutPolicy,
    ) -> Result<ManagedCheckoutPlan> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        if order.quote.input().buyer_principal != buyer {
            return Err(ManagedOrderStoreError::NotFound);
        }
        if let Some(row) =
            sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
                .bind(order_id)
                .fetch_optional(&mut *tx)
                .await?
        {
            let plan = plan_from_row(&row, &order)?;
            if plan.policy() != &policy {
                return conflict();
            }
            tx.commit().await?;
            return Ok(plan);
        }
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        let plan = ManagedCheckoutPlan::new(&order.quote, order_id, Uuid::new_v4(), policy, now)?;
        sqlx::query("INSERT INTO managed_stripe_checkout_operations(order_id,operation_id,stripe_account_id,livemode,plan,plan_digest)
            VALUES($1,$2,$3,$4,$5,$6)")
            .bind(order_id).bind(plan.operation_id()).bind(&plan.policy().stripe_account_id).bind(plan.policy().livemode)
            .bind(serde_json::to_value(&plan).map_err(|_| ManagedOrderStoreError::Integrity)?).bind(plan.digest())
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(plan)
    }

    // Old reconciliation fixtures exercise the atomic primitive; production must hold a lease.
    #[cfg(test)]
    pub async fn bind_managed_checkout(
        &self,
        order_id: Uuid,
        binding: &ManagedSessionBinding,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        bind_in_transaction(&mut tx, &order, binding).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn reconcile_managed_checkout_event(
        &self,
        authenticated: &AuthenticatedManagedEvent,
    ) -> Result<ManagedEventDisposition> {
        let event = authenticated.data();
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, event.order_id).await?;
        let row = sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
            .bind(order.order_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        let plan = plan_from_row(&row, &order)?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        if !event_matches(event, &order, &plan) || event.event_created_at > now + 300 {
            return conflict();
        }
        let json = serde_json::to_value(event).map_err(|_| ManagedOrderStoreError::Integrity)?;
        let outcome = outcome_name(event.outcome);
        let inserted = sqlx::query("INSERT INTO managed_stripe_event_inbox
            (stripe_account_id,livemode,event_id,order_id,operation_id,body_sha256,event,event_created_at,outcome,disposition,event_fingerprint)
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'pending_binding',$10) ON CONFLICT DO NOTHING")
            .bind(&event.stripe_account_id).bind(event.livemode).bind(&event.event_id).bind(event.order_id).bind(event.operation_id)
            .bind(&event.body_sha256).bind(&json).bind(event.event_created_at).bind(outcome).bind(fingerprint(&json)?)
            .execute(&mut *tx).await?.rows_affected();
        if inserted == 0 {
            let prior = sqlx::query("SELECT * FROM managed_stripe_event_inbox WHERE stripe_account_id=$1 AND livemode=$2 AND event_id=$3")
                .bind(&event.stripe_account_id).bind(event.livemode).bind(&event.event_id).fetch_one(&mut *tx).await?;
            if stored_event(&prior)? != *event {
                return conflict();
            }
            if prior.try_get::<String, _>("disposition")? != "pending_binding" {
                tx.commit().await?;
                return Ok(ManagedEventDisposition::Duplicate);
            }
        }
        let disposition = process_event(&mut tx, event, &order, &plan).await?;
        tx.commit().await?;
        Ok(disposition)
    }
}

// Caller holds the order row lock and, in production, the current execution fence.
pub(super) async fn bind_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    order: &ManagedOrder,
    binding: &ManagedSessionBinding,
) -> Result<()> {
    let order_id = order.order_id;
    let row = sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
        .bind(order_id)
        .fetch_one(&mut **tx)
        .await?;
    let plan = plan_from_row(&row, order)?;
    let existing: Option<String> = row.try_get("session_id")?;
    let intent: Option<String> = row.try_get("payment_intent_id")?;
    if binding.operation_id() != plan.operation_id()
        || binding.plan_digest() != plan.digest()
        || existing
            .as_deref()
            .is_some_and(|id| id != binding.session_id())
        || matches!((intent.as_deref(), binding.payment_intent_id()), (Some(a),Some(b)) if a != b)
    {
        return conflict();
    }
    sqlx::query("UPDATE managed_stripe_checkout_operations SET session_id=$2,checkout_url=COALESCE(checkout_url,$3),
            payment_intent_id=COALESCE(payment_intent_id,$4) WHERE order_id=$1")
            .bind(order_id).bind(binding.session_id()).bind(binding.checkout_url()).bind(binding.payment_intent_id())
            .execute(&mut **tx).await?;
    sqlx::query("UPDATE managed_service_actions SET state='completed' WHERE order_id=$1")
        .bind(order_id)
        .execute(&mut **tx)
        .await?;
    let pending = sqlx::query("SELECT * FROM managed_stripe_event_inbox WHERE order_id=$1 AND disposition='pending_binding' ORDER BY event_created_at,event_id")
            .bind(order_id).fetch_all(&mut **tx).await?;
    for row in pending {
        let event = stored_event(&row)?;
        process_event(tx, &event, order, &plan).await?;
    }
    Ok(())
}
