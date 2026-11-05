//! Low-level company conversion/payout adapter. No routes, environment defaults, scheduler,
//! bank debit or signer. Durable authorization/reservation is a caller prerequisite.
use async_trait::async_trait;
use managed_orders::bridge_transfer::{
    external_account_snapshot_digest, BridgeCompany, BridgeEnvironment, BridgeError,
    BridgeObservation, BridgeRoute, BridgeTransferPlan,
};
use reqwest::{header::HeaderValue, Method};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BridgeExecutionError {
    #[error("company transfer provider configuration does not match the operation")]
    Configuration,
    #[error("new company provider transfers are disabled")]
    Disabled,
    #[error("company transfer exceeds its configured amount limit")]
    Limit,
    #[error("company or supplier provider account is unavailable for new transfers")]
    Ineligible,
    #[error("company transfer transport failed; reconcile the original operation before retry")]
    Transport,
    #[error("company transfer provider returned HTTP {0}; reconcile the original operation before retry")]
    Http(u16),
    #[error("company transfer provider response is invalid")]
    Response,
    #[error(transparent)]
    Contract(#[from] BridgeError),
}
type Result<T> = std::result::Result<T, BridgeExecutionError>;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeExecutionPolicy {
    pub company: BridgeCompany,
    pub maximum_conversion_minor: u64,
    pub maximum_payout_micro_usdc: u64,
    pub maximum_payout_minor: u64,
}
impl BridgeExecutionPolicy {
    fn check_identity(&self, plan: &BridgeTransferPlan) -> Result<()> {
        if plan.company() != &self.company {
            return Err(BridgeExecutionError::Configuration);
        }
        Ok(())
    }
    fn check_limit(&self, plan: &BridgeTransferPlan) -> Result<()> {
        self.check_identity(plan)?;
        let within = match plan.route() {
            BridgeRoute::MxnConversion { input_minor, .. } => {
                *input_minor <= self.maximum_conversion_minor
            }
            BridgeRoute::SupplierMxnPayout {
                input_micro_usdc,
                output_minor,
                ..
            } => {
                *input_micro_usdc <= self.maximum_payout_micro_usdc
                    && *output_minor <= self.maximum_payout_minor
            }
        };
        if !within {
            return Err(BridgeExecutionError::Limit);
        }
        Ok(())
    }
}

// Deliberately not Debug/Serialize: provider keys and private response bodies never enter logs.
pub struct ManagedBridgeClient {
    policy: BridgeExecutionPolicy,
    transport: BridgeTransport,
}
struct BridgeTransport {
    client: reqwest::Client,
    key: HeaderValue,
    base: String,
}
enum Operation {
    Customer(Uuid),
    ExternalAccount { customer: Uuid, account: Uuid },
    Retrieve(Uuid),
    Create { operation: Uuid, body: Value },
}
#[async_trait]
trait Transport: Sync {
    async fn send(&self, operation: Operation) -> Result<Value>;
}

impl ManagedBridgeClient {
    pub fn new(api_key: &str, policy: BridgeExecutionPolicy) -> Result<Self> {
        let sandbox = policy.company.environment == BridgeEnvironment::Sandbox;
        if api_key.len() < 16
            || api_key.len() > 512
            || !api_key.bytes().all(|b| b.is_ascii_graphic())
            || !api_key.starts_with(if sandbox { "sk-test-" } else { "sk-live-" })
            || policy.company.customer_id.is_nil()
        {
            return Err(BridgeExecutionError::Configuration);
        }
        let mut key =
            HeaderValue::from_str(api_key).map_err(|_| BridgeExecutionError::Configuration)?;
        key.set_sensitive(true);
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|_| BridgeExecutionError::Configuration)?;
        Ok(Self {
            policy,
            transport: BridgeTransport {
                client,
                key,
                base: if sandbox {
                    "https://api.sandbox.bridge.xyz/v0"
                } else {
                    "https://api.bridge.xyz/v0"
                }
                .into(),
            },
        })
    }

    /// Creates/replays ONLY this original request/key. The caller must durably persist the
    /// plan and attempted state, fence competing workers and reserve verified cash or an
    /// earned supplier obligation before calling. Never call directly from customer input.
    /// A timeout or validation failure is uncertain, not permission to choose another key.
    pub async fn create_transfer(
        &self,
        plan: &BridgeTransferPlan,
        now: i64,
        allow_new: bool,
    ) -> Result<BridgeObservation> {
        create(&self.transport, &self.policy, plan, now, allow_new).await
    }

    /// Readback stays available after containment, lowered caps, retry expiry or account
    /// suspension. This does not create a transfer or treat provider delivery as chain truth.
    pub async fn retrieve_transfer(
        &self,
        plan: &BridgeTransferPlan,
        transfer_id: Uuid,
    ) -> Result<BridgeObservation> {
        retrieve(&self.transport, &self.policy, plan, transfer_id).await
    }
}

async fn create(
    transport: &impl Transport,
    policy: &BridgeExecutionPolicy,
    plan: &BridgeTransferPlan,
    now: i64,
    allow_new: bool,
) -> Result<BridgeObservation> {
    if !allow_new {
        return Err(BridgeExecutionError::Disabled);
    }
    policy.check_limit(plan)?;
    plan.request(now)?;
    // Three bounded requests at most; reserve the whole 30-second transport budget.
    let body = plan.request(
        now.checked_add(30)
            .ok_or(BridgeExecutionError::Configuration)?,
    )?;
    let company = transport
        .send(Operation::Customer(plan.company().customer_id))
        .await?;
    if company["id"] != plan.company().customer_id.to_string() || company["status"] != "active" {
        return Err(BridgeExecutionError::Ineligible);
    }
    if let BridgeRoute::SupplierMxnPayout {
        external_account_id,
        external_account_snapshot_digest: expected_snapshot,
        ..
    } = plan.route()
    {
        let external = transport
            .send(Operation::ExternalAccount {
                customer: plan.company().customer_id,
                account: *external_account_id,
            })
            .await?;
        // Onboarding must additionally bind the verified bank owner to the supplier obligation.
        // These preflight reads alone cannot establish that relationship or approved third-party use.
        if external["id"] != external_account_id.to_string()
            || external["customer_id"] != plan.company().customer_id.to_string()
            || external["currency"] != "mxn"
            || external["account_type"] != "clabe"
            || external["active"] != true
            || external["account_verification"]["clabe"]["matched"] != true
            || external_account_snapshot_digest(&external)? != *expected_snapshot
        {
            return Err(BridgeExecutionError::Ineligible);
        }
    }
    let response = transport
        .send(Operation::Create {
            operation: plan.operation_id(),
            body,
        })
        .await?;
    Ok(plan.observe(&response, None)?)
}
async fn retrieve(
    transport: &impl Transport,
    policy: &BridgeExecutionPolicy,
    plan: &BridgeTransferPlan,
    transfer_id: Uuid,
) -> Result<BridgeObservation> {
    policy.check_identity(plan)?;
    if transfer_id.is_nil() {
        return Err(BridgeExecutionError::Configuration);
    }
    let response = transport.send(Operation::Retrieve(transfer_id)).await?;
    Ok(plan.observe(&response, Some(transfer_id))?)
}

#[async_trait]
impl Transport for BridgeTransport {
    async fn send(&self, operation: Operation) -> Result<Value> {
        let (method, path) = match &operation {
            Operation::Customer(id) => (Method::GET, format!("/customers/{id}")),
            Operation::ExternalAccount { customer, account } => (
                Method::GET,
                format!("/customers/{customer}/external_accounts/{account}"),
            ),
            Operation::Retrieve(id) => (Method::GET, format!("/transfers/{id}")),
            Operation::Create { .. } => (Method::POST, "/transfers".into()),
        };
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .header("Api-Key", self.key.clone());
        if let Operation::Create { operation, body } = operation {
            request = request
                .header("Idempotency-Key", operation.to_string())
                .json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| BridgeExecutionError::Transport)?;
        if !response.status().is_success() {
            return Err(BridgeExecutionError::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(BridgeExecutionError::Response);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| BridgeExecutionError::Transport)?
        {
            if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(BridgeExecutionError::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| BridgeExecutionError::Response)
    }
}

#[cfg(test)]
mod tests;
