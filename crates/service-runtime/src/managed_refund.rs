//! Fenced original-payment refund execution. Signed native events remain authoritative.
use async_trait::async_trait;
use db::{
    managed_service_orders::{
        ManagedCheckoutRunResult, ManagedOrderStoreError, ManagedRefundLease,
    },
    PostgresStore,
};
use managed_orders::{
    stripe_checkout::MANAGED_STRIPE_API_VERSION,
    stripe_refund::{
        ManagedRefundBinding, ManagedRefundPlan, RefundError, RefundKind, RefundMerchant,
        RefundOutcome,
    },
};
use payments_stripe::{stripe_form_encode, StripeRequestIntent};
use reqwest::{
    header::{HeaderValue, AUTHORIZATION},
    Method,
};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const MAX_BYTES: usize = 2 * 1024 * 1024;
#[derive(Debug, thiserror::Error)]
pub enum ManagedRefundExecutionError {
    #[error("managed refund merchant configuration is invalid or unavailable")]
    Configuration,
    #[error("managed refund transport failed; reconcile the original operation before retry")]
    Transport,
    #[error("managed refund provider returned HTTP {0}")]
    Http(u16),
    #[error("managed refund provider response failed validation")]
    Response,
    #[error("managed refund requires reconciliation before any further request")]
    ReconciliationRequired,
    #[error("managed refund storage transition failed")]
    Store(#[from] ManagedOrderStoreError),
}
type Result<T> = std::result::Result<T, ManagedRefundExecutionError>;
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManagedRefundExecution {
    Busy,
    Bound { refund_id: String },
}

// No Debug or serialization; credentials never enter errors or stored plans.
pub struct ManagedRefundClient {
    merchant: RefundMerchant,
    transport: StripeTransport,
}
struct StripeTransport {
    client: reqwest::Client,
    authorization: HeaderValue,
    base: String,
}
enum Operation {
    Account,
    Retrieve(String),
    List {
        payment_intent: String,
        after: Option<String>,
    },
    Create(StripeRequestIntent),
}
#[async_trait]
trait Transport: Sync {
    async fn send(&self, operation: Operation) -> Result<Value>;
}
fn id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 255
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
impl ManagedRefundClient {
    pub fn new(secret: &str, merchant: RefundMerchant) -> Result<Self> {
        if crate::stripe_secret_key_mode_from_secret(Some(secret))
            != if merchant.livemode { "live" } else { "test" }
            || !id(&merchant.stripe_account_id, "acct_")
        {
            return Err(ManagedRefundExecutionError::Configuration);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", secret.trim()))
            .map_err(|_| ManagedRefundExecutionError::Configuration)?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| ManagedRefundExecutionError::Configuration)?;
        Ok(Self {
            merchant,
            transport: StripeTransport {
                client,
                authorization,
                base: "https://api.stripe.com".into(),
            },
        })
    }
}
#[async_trait]
impl Transport for StripeTransport {
    async fn send(&self, operation: Operation) -> Result<Value> {
        let (method, path) = match &operation {
            Operation::Account => (Method::GET, "/v1/account".into()),
            Operation::Retrieve(refund) if id(refund, "re_") => {
                (Method::GET, format!("/v1/refunds/{refund}"))
            }
            Operation::List {
                payment_intent,
                after,
            } if id(payment_intent, "pi_") && after.as_ref().is_none_or(|s| id(s, "re_")) => {
                (Method::GET, "/v1/refunds".into())
            }
            Operation::Create(r)
                if r.method == "POST"
                    && r.endpoint == "/v1/refunds"
                    && r.api_version == MANAGED_STRIPE_API_VERSION =>
            {
                (Method::POST, r.endpoint.clone())
            }
            _ => return Err(ManagedRefundExecutionError::Configuration),
        };
        let mut req = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Stripe-Version", MANAGED_STRIPE_API_VERSION);
        match operation {
            Operation::Create(r) => {
                req = req
                    .header("Idempotency-Key", r.idempotency_key)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(stripe_form_encode(&r.body))
            }
            Operation::List {
                payment_intent,
                after,
            } => {
                req = req.query(&[
                    ("limit", "100"),
                    ("payment_intent", payment_intent.as_str()),
                ]);
                if let Some(after) = after {
                    req = req.query(&[("starting_after", after)]);
                }
            }
            _ => {}
        }
        let mut response = req
            .send()
            .await
            .map_err(|_| ManagedRefundExecutionError::Transport)?;
        if !response.status().is_success() {
            return Err(ManagedRefundExecutionError::Http(
                response.status().as_u16(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
        {
            return Err(ManagedRefundExecutionError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ManagedRefundExecutionError::Transport)?
        {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(ManagedRefundExecutionError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| ManagedRefundExecutionError::Response)
    }
}

async fn discover(transport: &impl Transport, p: &ManagedRefundPlan) -> Result<Option<String>> {
    let mut after = None;
    let mut found = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..5 {
        let v = transport
            .send(Operation::List {
                payment_intent: p.payment_intent_id().into(),
                after: after.clone(),
            })
            .await?;
        let data = v["data"]
            .as_array()
            .filter(|a| a.len() <= 100)
            .ok_or(ManagedRefundExecutionError::Response)?;
        let more = v["has_more"]
            .as_bool()
            .ok_or(ManagedRefundExecutionError::Response)?;
        if v["object"] != "list" || (more && data.is_empty()) {
            return Err(ManagedRefundExecutionError::Response);
        }
        for refund in data {
            let rid = refund["id"]
                .as_str()
                .filter(|s| id(s, "re_"))
                .ok_or(ManagedRefundExecutionError::Response)?;
            if !seen.insert(rid.to_owned()) || refund["payment_intent"] != p.payment_intent_id() {
                return Err(ManagedRefundExecutionError::Response);
            }
            // An external/manual/partial refund cannot silently become a second full refund.
            if !p.matches_metadata(refund) {
                return Err(ManagedRefundExecutionError::ReconciliationRequired);
            }
            p.binding(refund)
                .map_err(|_| ManagedRefundExecutionError::Response)?;
            if found.is_some() {
                return Err(ManagedRefundExecutionError::ReconciliationRequired);
            }
            found = Some(rid.to_owned());
            after = Some(rid.to_owned());
        }
        if !more {
            return Ok(found);
        }
    }
    Err(ManagedRefundExecutionError::ReconciliationRequired)
}
async fn resolve(
    store: &PostgresStore,
    transport: &impl Transport,
    merchant: &RefundMerchant,
    lease: &ManagedRefundLease,
    allow_create: bool,
) -> Result<ManagedRefundBinding> {
    let p = lease.plan();
    if p.merchant() != merchant {
        return Err(ManagedRefundExecutionError::Configuration);
    }
    let account = transport.send(Operation::Account).await?;
    if account["object"] != "account" || account["id"] != merchant.stripe_account_id {
        return Err(ManagedRefundExecutionError::Configuration);
    }
    let known = match lease.known_refund() {
        Some(id) => Some(id.to_owned()),
        None => discover(transport, p).await?,
    };
    if let Some(rid) = known {
        let value = transport.send(Operation::Retrieve(rid.clone())).await?;
        if value["id"] != rid {
            return Err(ManagedRefundExecutionError::Response);
        }
        return p
            .binding(&value)
            .map_err(|_| ManagedRefundExecutionError::Response);
    }
    if !allow_create {
        return Err(ManagedRefundExecutionError::ReconciliationRequired);
    }
    let request = store
        .begin_managed_refund_create(lease, merchant)
        .await
        .map_err(|e| match e {
            ManagedOrderStoreError::Refund(RefundError::ReconciliationRequired) => {
                ManagedRefundExecutionError::ReconciliationRequired
            }
            other => ManagedRefundExecutionError::Store(other),
        })?;
    let value = transport.send(Operation::Create(request)).await?;
    p.binding(&value)
        .map_err(|_| ManagedRefundExecutionError::Response)
}
pub async fn execute_managed_refund(
    store: &PostgresStore,
    client: &ManagedRefundClient,
    order_id: Uuid,
    allow_create: bool,
) -> Result<ManagedRefundExecution> {
    run(
        store,
        &client.transport,
        &client.merchant,
        order_id,
        RefundKind::Full,
        allow_create,
    )
    .await
}
/// Execute a reserved review-deposit refund with the same fencing as a full refund.
pub async fn execute_managed_deposit_refund(
    store: &PostgresStore,
    client: &ManagedRefundClient,
    order_id: Uuid,
) -> Result<ManagedRefundExecution> {
    run(
        store,
        &client.transport,
        &client.merchant,
        order_id,
        RefundKind::ReviewDeposit,
        true,
    )
    .await
}
/// Read-only reconciliation stays available after creation is disabled or eligibility changes.
pub async fn reconcile_recovered_bounty_refund(
    store: &PostgresStore,
    client: &ManagedRefundClient,
    order_id: Uuid,
) -> Result<ManagedRefundExecution> {
    run(
        store,
        &client.transport,
        &client.merchant,
        order_id,
        RefundKind::RecoveredBounty,
        false,
    )
    .await
}
/// Fresh chain qualification is mandatory for creation; failures do not prevent reconciliation
/// of an existing immutable operation after a lost provider response.
pub async fn execute_recovered_bounty_refund(
    store: &PostgresStore,
    client: &ManagedRefundClient,
    rpc: &str,
    order_id: Uuid,
) -> Result<ManagedRefundExecution> {
    let qualification = tokio::time::timeout(Duration::from_secs(55), async {
        let target = store.managed_recovery_target(order_id).await?;
        let hashes = store.managed_recovery_transactions(&target).await?;
        let proof = crate::recovered_refund::qualify_recovery(rpc, &target, &hashes)
            .await
            .map_err(|_| ManagedRefundExecutionError::ReconciliationRequired)?;
        store.reserve_managed_recovered_refund(&proof).await?;
        Ok::<_, ManagedRefundExecutionError>(())
    })
    .await
    .unwrap_or(Err(ManagedRefundExecutionError::ReconciliationRequired));
    let allow_create = match qualification {
        Ok(()) => true,
        Err(error) => {
            if store
                .managed_recovered_refund_plan(order_id)
                .await?
                .is_none()
            {
                return Err(error);
            }
            false
        }
    };
    run(
        store,
        &client.transport,
        &client.merchant,
        order_id,
        RefundKind::RecoveredBounty,
        allow_create,
    )
    .await
}
/// Stripe's current status of a bound refund, read with the merchant key. This is a diagnostic:
/// refund status changes only through signed Stripe events, so a mismatch is reported to the
/// operator, who resends the event; a read never changes money state.
pub async fn read_managed_refund_status(
    client: &ManagedRefundClient,
    plan: &ManagedRefundPlan,
    refund_id: &str,
) -> Result<RefundOutcome> {
    if plan.merchant() != &client.merchant {
        return Err(ManagedRefundExecutionError::Configuration);
    }
    observe(&client.transport, plan, refund_id).await
}
async fn observe(
    transport: &impl Transport,
    plan: &ManagedRefundPlan,
    refund_id: &str,
) -> Result<RefundOutcome> {
    if !id(refund_id, "re_") {
        return Err(ManagedRefundExecutionError::Response);
    }
    let value = transport
        .send(Operation::Retrieve(refund_id.into()))
        .await?;
    // The read must be exactly this plan's refund: id, payment, amount, currency and metadata.
    if value["id"] != refund_id || plan.binding(&value).is_err() {
        return Err(ManagedRefundExecutionError::Response);
    }
    serde_json::from_value(value["status"].clone())
        .map_err(|_| ManagedRefundExecutionError::Response)
}
async fn run(
    store: &PostgresStore,
    transport: &impl Transport,
    merchant: &RefundMerchant,
    order_id: Uuid,
    kind: RefundKind,
    allow_create: bool,
) -> Result<ManagedRefundExecution> {
    let claimed = match kind {
        RefundKind::Full => store.claim_managed_refund(order_id).await?,
        RefundKind::ReviewDeposit => store.claim_managed_deposit_refund(order_id).await?,
        RefundKind::RecoveredBounty => store.claim_managed_recovered_refund(order_id).await?,
    };
    let Some(lease) = claimed else {
        return Ok(ManagedRefundExecution::Busy);
    };
    match resolve(store, transport, merchant, &lease, allow_create).await {
        Ok(binding) => {
            store.complete_managed_refund(&lease, &binding).await?;
            Ok(ManagedRefundExecution::Bound {
                refund_id: binding.refund_id().into(),
            })
        }
        Err(error) => {
            let result = match error {
                ManagedRefundExecutionError::Configuration
                | ManagedRefundExecutionError::Http(401 | 403) => {
                    ManagedCheckoutRunResult::ConfigurationError
                }
                ManagedRefundExecutionError::ReconciliationRequired
                | ManagedRefundExecutionError::Response
                | ManagedRefundExecutionError::Http(400 | 404) => {
                    ManagedCheckoutRunResult::ReconciliationRequired
                }
                _ => ManagedCheckoutRunResult::Retry,
            };
            store.release_managed_refund(&lease, result).await?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests;
