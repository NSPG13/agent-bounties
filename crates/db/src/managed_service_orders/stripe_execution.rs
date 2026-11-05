//! Internal worker claims. Callers must enforce execution policy before requesting a create.
use super::{stripe::*, *};
use managed_orders::stripe_checkout::{CheckoutPolicy, ManagedCheckoutPlan, ManagedSessionBinding};
use payments_stripe::StripeRequestIntent;
use sqlx::{Postgres, Transaction};

// A run makes at most eight requests, each with a ten-second total timeout.
const LEASE_SECONDS: i64 = 180;

/// Not deserializable or constructible from a client-supplied claim.
pub struct ManagedCheckoutLease {
    plan: ManagedCheckoutPlan,
    token: Uuid,
    known_session: Option<String>,
    previously_attempted: bool,
}
impl ManagedCheckoutLease {
    pub fn plan(&self) -> &ManagedCheckoutPlan {
        &self.plan
    }
    pub fn known_session(&self) -> Option<&str> {
        self.known_session.as_deref()
    }
    pub fn previously_attempted(&self) -> bool {
        self.previously_attempted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedCheckoutRunResult {
    Retry,
    ReconciliationRequired,
    ConfigurationError,
}

async fn fence(
    tx: &mut Transaction<'_, Postgres>,
    lease: &ManagedCheckoutLease,
) -> Result<ManagedOrder> {
    let order = locked_order(tx, lease.plan.order_id()).await?;
    let current: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM managed_stripe_checkout_execution
        WHERE order_id=$1 AND lease_token=$2 AND lease_until>clock_timestamp())",
    )
    .bind(order.order_id)
    .bind(lease.token)
    .fetch_one(&mut **tx)
    .await?;
    if !current {
        return Err(ManagedOrderStoreError::StaleLease);
    }
    let row = sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
        .bind(order.order_id)
        .fetch_one(&mut **tx)
        .await?;
    if plan_from_row(&row, &order)? != lease.plan {
        return Err(ManagedOrderStoreError::Integrity);
    }
    Ok(order)
}

impl PostgresStore {
    /// Claim an existing operation only. This does not issue a quote or authorize payment.
    /// Uses database time after acquiring the order lock, never a worker-supplied timestamp.
    pub async fn claim_managed_checkout(
        &self,
        order_id: Uuid,
    ) -> Result<Option<ManagedCheckoutLease>> {
        let mut tx = self.pool.begin().await?;
        let order = locked_order(&mut tx, order_id).await?;
        let row = sqlx::query("SELECT * FROM managed_stripe_checkout_operations WHERE order_id=$1")
            .bind(order_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ManagedOrderStoreError::NotFound)?;
        let plan = plan_from_row(&row, &order)?;
        let mut known_session: Option<String> = row.try_get("session_id")?;
        if known_session.is_none() {
            let pending = sqlx::query(
                "SELECT * FROM managed_stripe_event_inbox WHERE order_id=$1
                AND disposition='pending_binding' ORDER BY event_created_at,event_id LIMIT 101",
            )
            .bind(order_id)
            .fetch_all(&mut *tx)
            .await?;
            if pending.len() > 100 {
                return Err(ManagedOrderStoreError::Integrity);
            }
            for row in pending {
                let event = stored_event(&row)?;
                if event.order_id != order_id || event.operation_id != plan.operation_id() {
                    return Err(ManagedOrderStoreError::Integrity);
                }
                if known_session
                    .as_ref()
                    .is_some_and(|id| id != &event.session_id)
                {
                    return Err(OrderError::Conflict.into());
                }
                known_session = Some(event.session_id);
            }
        }
        sqlx::query("INSERT INTO managed_stripe_checkout_execution(order_id) VALUES($1) ON CONFLICT DO NOTHING")
            .bind(order_id).execute(&mut *tx).await?;
        let token = Uuid::new_v4();
        let claimed = sqlx::query(
            "UPDATE managed_stripe_checkout_execution SET lease_token=$2,
            lease_until=clock_timestamp()+make_interval(secs=>$3::double precision)
            WHERE order_id=$1 AND (lease_until IS NULL OR lease_until<=clock_timestamp())
              AND next_attempt_at<=clock_timestamp() RETURNING create_attempts",
        )
        .bind(order_id)
        .bind(token)
        .bind(LEASE_SECONDS as f64)
        .fetch_optional(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(claimed.map(|row| ManagedCheckoutLease {
            plan,
            token,
            known_session,
            previously_attempted: row.get::<i64, _>("create_attempts") > 0,
        }))
    }

    /// Persist the attempt before external I/O. A crash from here on requires readback or
    /// a replay of the SAME request within the provider idempotency/lifetime bounds.
    pub async fn begin_managed_checkout_create(
        &self,
        lease: &ManagedCheckoutLease,
        policy: &CheckoutPolicy,
    ) -> Result<StripeRequestIntent> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        let now: i64 =
            sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
                .fetch_one(&mut *tx)
                .await?;
        // Leave time for a bounded network request before Stripe's expiry constraint.
        let request = lease.plan.request(now + 15, policy)?;
        let bound: bool = sqlx::query_scalar("SELECT session_id IS NOT NULL FROM managed_stripe_checkout_operations WHERE order_id=$1")
            .bind(order.order_id).fetch_one(&mut *tx).await?;
        if bound {
            return Err(OrderError::Conflict.into());
        }
        sqlx::query(
            "UPDATE managed_stripe_checkout_execution SET create_attempts=create_attempts+1,
            last_create_at=clock_timestamp() WHERE order_id=$1",
        )
        .bind(order.order_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(request)
    }

    /// Session binding, early webhook processing and lease completion commit together.
    pub async fn complete_managed_checkout(
        &self,
        lease: &ManagedCheckoutLease,
        binding: &ManagedSessionBinding,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        bind_in_transaction(&mut tx, &order, binding).await?;
        sqlx::query("UPDATE managed_stripe_checkout_execution SET lease_token=NULL,lease_until=NULL,
            last_result='bound',next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE order_id=$1")
            .bind(order.order_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Intake may be disabled while these recovery results continue to be saved.
    /// A stale worker cannot clear a successor's lease or replace its result.
    pub async fn release_managed_checkout(
        &self,
        lease: &ManagedCheckoutLease,
        result: ManagedCheckoutRunResult,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let order = fence(&mut tx, lease).await?;
        let name = match result {
            ManagedCheckoutRunResult::Retry => "retry",
            ManagedCheckoutRunResult::ReconciliationRequired => "reconciliation_required",
            ManagedCheckoutRunResult::ConfigurationError => "configuration_error",
        };
        sqlx::query("UPDATE managed_stripe_checkout_execution SET lease_token=NULL,lease_until=NULL,last_result=$2,
            next_attempt_at=clock_timestamp()+interval '30 seconds' WHERE order_id=$1")
            .bind(order.order_id).bind(name).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
