//! Bounded, merchant-scoped Checkout execution. Only signed webhooks prove payment.
//! This module has no environment defaults, public route, scheduler or live enable switch.
use async_trait::async_trait;
use db::{
    managed_service_orders::{
        ManagedCheckoutLease, ManagedCheckoutRunResult, ManagedOrderStoreError,
    },
    PostgresStore,
};
use managed_orders::stripe_checkout::{
    CheckoutError, CheckoutPolicy, ManagedCheckoutPlan, ManagedSessionBinding,
    MANAGED_STRIPE_API_VERSION,
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

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_LIST_PAGES: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum ManagedCheckoutExecutionError {
    #[error("managed Checkout merchant configuration is invalid or unavailable")]
    Configuration,
    #[error("managed Checkout provider transport failed; readback is required before retry")]
    Transport,
    #[error("managed Checkout provider returned HTTP {0}")]
    Http(u16),
    #[error("managed Checkout provider response failed validation")]
    Response,
    #[error("managed Checkout requires further reconciliation")]
    ReconciliationRequired,
    #[error("managed Checkout storage transition failed")]
    Store(#[from] ManagedOrderStoreError),
}
type Result<T> = std::result::Result<T, ManagedCheckoutExecutionError>;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ManagedCheckoutExecution {
    Busy,
    Bound {
        session_id: String,
        checkout_url: Option<String>,
    },
}

// The only constructible production client pins TLS origin and disables redirects/proxies.
// No Debug/Serialize implementation: credentials and raw provider bodies must not be logged.
pub struct ManagedStripeClient {
    policy: CheckoutPolicy,
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
        created_from: i64,
        created_to: i64,
        after: Option<String>,
    },
    Create(StripeRequestIntent),
}
#[async_trait]
trait Transport: Sync {
    async fn send(&self, operation: Operation) -> Result<Value>;
}

impl ManagedStripeClient {
    pub fn new(secret_key: &str, policy: CheckoutPolicy) -> Result<Self> {
        let mode = crate::stripe_secret_key_mode_from_secret(Some(secret_key));
        if mode != if policy.livemode { "live" } else { "test" }
            || !provider_id(&policy.stripe_account_id, "acct_")
        {
            return Err(ManagedCheckoutExecutionError::Configuration);
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {}", secret_key.trim()))
            .map_err(|_| ManagedCheckoutExecutionError::Configuration)?;
        authorization.set_sensitive(true);
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| ManagedCheckoutExecutionError::Configuration)?;
        Ok(Self {
            policy,
            transport: StripeTransport {
                client,
                authorization,
                base: "https://api.stripe.com".into(),
            },
        })
    }
}

fn provider_id(id: &str, prefix: &str) -> bool {
    id.starts_with(prefix)
        && id.len() > prefix.len()
        && id.len() <= 255
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

#[async_trait]
impl Transport for StripeTransport {
    async fn send(&self, operation: Operation) -> Result<Value> {
        let (method, path) = match &operation {
            Operation::Account => (Method::GET, "/v1/account".to_owned()),
            Operation::Retrieve(id) if provider_id(id, "cs_") => {
                (Method::GET, format!("/v1/checkout/sessions/{id}"))
            }
            Operation::List { after, .. }
                if after.as_ref().is_none_or(|id| provider_id(id, "cs_")) =>
            {
                (Method::GET, "/v1/checkout/sessions".to_owned())
            }
            Operation::Create(intent)
                if intent.method == "POST"
                    && intent.endpoint == "/v1/checkout/sessions"
                    && intent.api_version == MANAGED_STRIPE_API_VERSION =>
            {
                (Method::POST, intent.endpoint.clone())
            }
            _ => return Err(ManagedCheckoutExecutionError::Configuration),
        };
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header(AUTHORIZATION, self.authorization.clone())
            .header("Stripe-Version", MANAGED_STRIPE_API_VERSION);
        match operation {
            Operation::List {
                created_from,
                created_to,
                after,
            } => {
                request = request.query(&[
                    ("limit", "100".to_owned()),
                    ("created[gte]", created_from.to_string()),
                    ("created[lte]", created_to.to_string()),
                ]);
                if let Some(id) = after {
                    request = request.query(&[("starting_after", id)]);
                }
            }
            Operation::Create(intent) => {
                request = request
                    .header("Idempotency-Key", intent.idempotency_key)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(stripe_form_encode(&intent.body))
            }
            _ => {}
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| ManagedCheckoutExecutionError::Transport)?;
        if !response.status().is_success() {
            return Err(ManagedCheckoutExecutionError::Http(
                response.status().as_u16(),
            ));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(ManagedCheckoutExecutionError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ManagedCheckoutExecutionError::Transport)?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(ManagedCheckoutExecutionError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| ManagedCheckoutExecutionError::Response)
    }
}

/// Finds at most one exact operation. Exhaustion, ambiguity or corrupt pagination is not
/// evidence of absence and NEVER authorizes another create. No customer-wide unbounded search.
async fn discover(
    transport: &impl Transport,
    plan: &ManagedCheckoutPlan,
) -> Result<Option<String>> {
    let mut after = None;
    let mut found = None;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..MAX_LIST_PAGES {
        let page = transport
            .send(Operation::List {
                created_from: plan.created_at(),
                created_to: plan.expires_at(),
                after: after.clone(),
            })
            .await?;
        let data = page["data"]
            .as_array()
            .filter(|data| data.len() <= 100)
            .ok_or(ManagedCheckoutExecutionError::Response)?;
        let more = page["has_more"]
            .as_bool()
            .ok_or(ManagedCheckoutExecutionError::Response)?;
        if page["object"] != "list" || (more && data.is_empty()) {
            return Err(ManagedCheckoutExecutionError::Response);
        }
        for session in data {
            let id = session["id"]
                .as_str()
                .filter(|id| provider_id(id, "cs_"))
                .ok_or(ManagedCheckoutExecutionError::Response)?;
            if !seen.insert(id.to_owned()) {
                return Err(ManagedCheckoutExecutionError::Response);
            }
            if session["metadata"]["managed_operation_id"] == plan.operation_id().to_string() {
                plan.session_binding(session)
                    .map_err(|_| ManagedCheckoutExecutionError::Response)?;
                if found.is_some() {
                    return Err(ManagedCheckoutExecutionError::ReconciliationRequired);
                }
                found = Some(id.to_owned());
            }
            after = Some(id.to_owned());
        }
        if !more {
            return Ok(found);
        }
    }
    Err(ManagedCheckoutExecutionError::ReconciliationRequired)
}

async fn resolve(
    store: &PostgresStore,
    lease: &ManagedCheckoutLease,
    policy: &CheckoutPolicy,
    transport: &impl Transport,
    allow_create: bool,
) -> Result<ManagedSessionBinding> {
    let plan = lease.plan();
    if plan.policy().stripe_account_id != policy.stripe_account_id
        || plan.policy().livemode != policy.livemode
    {
        return Err(ManagedCheckoutExecutionError::Configuration);
    }
    // GET /v1/account verifies the credential's own merchant, not an accessible connected account.
    let merchant = transport.send(Operation::Account).await?;
    if merchant["object"] != "account" || merchant["id"] != policy.stripe_account_id {
        return Err(ManagedCheckoutExecutionError::Configuration);
    }
    let session = if let Some(id) = lease.known_session() {
        Some(id.to_owned())
    } else if lease.previously_attempted() {
        discover(transport, plan).await?
    } else {
        None
    };
    if let Some(id) = session {
        let response = transport.send(Operation::Retrieve(id.clone())).await?;
        if response["id"] != id {
            return Err(ManagedCheckoutExecutionError::Response);
        }
        return plan
            .session_binding(&response)
            .map_err(|_| ManagedCheckoutExecutionError::Response);
    }
    if !allow_create {
        return Err(ManagedCheckoutExecutionError::ReconciliationRequired);
    }
    // Stripe sandboxes can accept test payments while these live-readiness flags are
    // inactive. Credential mode and merchant identity are checked independently;
    // live creation still requires both capabilities explicitly enabled.
    if policy.livemode
        && (merchant["charges_enabled"] != true || merchant["payouts_enabled"] != true)
    {
        return Err(ManagedCheckoutExecutionError::Configuration);
    }
    let request = store
        .begin_managed_checkout_create(lease, policy)
        .await
        .map_err(|e| match e {
            ManagedOrderStoreError::Checkout(CheckoutError::ReconciliationRequired) => {
                ManagedCheckoutExecutionError::ReconciliationRequired
            }
            e => ManagedCheckoutExecutionError::Store(e),
        })?;
    let response = transport.send(Operation::Create(request)).await?;
    plan.session_binding(&response)
        .map_err(|_| ManagedCheckoutExecutionError::Response)
}

/// Internal shared API/MCP/worker boundary. `allow_create` must reflect trusted launch/intake
/// policy. False still permits read-only recovery; it never abandons accepted obligations.
pub async fn execute_managed_checkout(
    store: &PostgresStore,
    client: &ManagedStripeClient,
    order_id: Uuid,
    allow_create: bool,
) -> Result<ManagedCheckoutExecution> {
    run(
        store,
        &client.policy,
        &client.transport,
        order_id,
        allow_create,
    )
    .await
}

async fn run(
    store: &PostgresStore,
    policy: &CheckoutPolicy,
    transport: &impl Transport,
    order_id: Uuid,
    allow_create: bool,
) -> Result<ManagedCheckoutExecution> {
    let Some(lease) = store.claim_managed_checkout(order_id).await? else {
        return Ok(ManagedCheckoutExecution::Busy);
    };
    match resolve(store, &lease, policy, transport, allow_create).await {
        Ok(binding) => {
            store.complete_managed_checkout(&lease, &binding).await?;
            Ok(ManagedCheckoutExecution::Bound {
                session_id: binding.session_id().into(),
                checkout_url: binding.checkout_url().map(str::to_owned),
            })
        }
        Err(error) => {
            let disposition = match error {
                ManagedCheckoutExecutionError::Configuration
                | ManagedCheckoutExecutionError::Http(401 | 403) => {
                    ManagedCheckoutRunResult::ConfigurationError
                }
                ManagedCheckoutExecutionError::ReconciliationRequired
                | ManagedCheckoutExecutionError::Response => {
                    ManagedCheckoutRunResult::ReconciliationRequired
                }
                _ => ManagedCheckoutRunResult::Retry,
            };
            store.release_managed_checkout(&lease, disposition).await?;
            Err(error)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
