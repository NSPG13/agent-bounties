//! Order-bound Checkout requests and authenticated native events. Neither is funding evidence.

use crate::{FiatCurrency, ManagedQuote};
use payments_stripe::{
    verify_platform_webhook_event, StripeRequestIntent, CHECKOUT_SESSIONS_ENDPOINT,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MANAGED_STRIPE_API_VERSION: &str = "2026-09-30.endive";
pub const MANAGED_CHECKOUT_PURPOSE: &str = "managed_service_order";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CheckoutError {
    #[error("managed Checkout configuration or quote binding is invalid")]
    Binding,
    #[error("managed Checkout offer cannot fit the provider session lifetime")]
    Expired,
    #[error("managed Checkout requires reconciliation before any further request")]
    ReconciliationRequired,
    #[error("managed Checkout webhook authentication failed")]
    Authentication,
    #[error("managed Checkout event is unsupported or inconsistent")]
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutPolicy {
    pub stripe_account_id: String,
    pub livemode: bool,
    pub currency: FiatCurrency,
    pub minimum_minor: u64,
    pub maximum_minor: u64,
    pub payment_method_configuration: Option<String>,
    /// Stripe Adaptive Pricing: customers may pay in their local currency at Stripe's rate
    /// and fee. The session, PaymentIntent, receipt and refund stay in `currency`.
    #[serde(default)]
    pub adaptive_pricing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDocument {
    order_id: Uuid,
    operation_id: Uuid,
    quote_digest: String,
    policy: CheckoutPolicy,
    created_at: i64,
    expires_at: i64,
    retry_before: i64,
    api_version: String,
    idempotency_key: String,
    body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ManagedCheckoutPlan(PlanDocument);

fn currency_name(currency: FiatCurrency) -> &'static str {
    match currency {
        FiatCurrency::Mxn => "mxn",
        FiatCurrency::Usd => "usd",
    }
}

fn identifier(value: &str, prefix: &str) -> bool {
    value.starts_with(prefix)
        && value.len() > prefix.len()
        && value.len() <= 255
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn digest_string(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl ManagedCheckoutPlan {
    pub fn new(
        quote: &ManagedQuote,
        order_id: Uuid,
        operation_id: Uuid,
        policy: CheckoutPolicy,
        now: i64,
    ) -> Result<Self, CheckoutError> {
        let digest = quote.digest().map_err(|_| CheckoutError::Binding)?;
        quote
            .validate_acceptance(&quote.input().buyer_principal, &digest, now)
            .map_err(|_| CheckoutError::Expired)?;
        let q = quote.input();
        let documented_minimum = match policy.currency {
            FiatCurrency::Mxn => 1_000,
            FiatCurrency::Usd => 50,
        };
        if order_id.is_nil()
            || operation_id.is_nil()
            || !identifier(&policy.stripe_account_id, "acct_")
            || policy.stripe_account_id != q.stripe_account_id
            || policy.livemode != q.livemode
            || policy.currency != q.currency
            || policy.minimum_minor < documented_minimum
            || policy.maximum_minor < policy.minimum_minor
            || policy.maximum_minor > 99_999_999
            || quote.total_minor() < policy.minimum_minor
            || quote.total_minor() > policy.maximum_minor
            || policy
                .payment_method_configuration
                .as_ref()
                .is_some_and(|id| !identifier(id, "pmc_"))
        {
            return Err(CheckoutError::Binding);
        }
        let retry_before = now.checked_add(23 * 3600).ok_or(CheckoutError::Expired)?;
        let expires_at = q
            .expires_at
            .min(now.checked_add(3600).ok_or(CheckoutError::Expired)?);
        // Include one minute for transport latency before Stripe enforces its 30-minute minimum.
        if expires_at - now < 1860 {
            return Err(CheckoutError::Expired);
        }
        let metadata = json!({ "purpose": MANAGED_CHECKOUT_PURPOSE,
            "managed_order_id": order_id, "managed_operation_id": operation_id,
            "managed_quote_digest": digest, "merchant_account_id": policy.stripe_account_id });
        let lines: Vec<Value> = quote
            .lines()
            .iter()
            .map(|line| {
                json!({
                    "quantity": 1, "price_data": { "currency": currency_name(q.currency),
                    "unit_amount": line.amount_minor, "product_data": { "name": line.label } }
                })
            })
            .collect();
        // A stable random suffix: the operation UUID is generated once and durably persisted.
        let suffix: String = operation_id.as_bytes()[..8]
            .iter()
            .map(|v| char::from(b'a' + v % 26))
            .collect();
        let mut body = json!({ "mode": "payment", "ui_mode": "hosted_page",
            "client_reference_id": order_id, "line_items": lines,
            "metadata": metadata, "payment_intent_data": { "metadata": metadata },
            "adaptive_pricing": { "enabled": policy.adaptive_pricing }, "expires_at": expires_at,
            "integration_identifier": format!("agentbounties-managed-{suffix}"),
            "success_url": format!("https://agentizable.ai/account.html?managed_order={order_id}&checkout_session_id={{CHECKOUT_SESSION_ID}}"),
            "cancel_url": format!("https://agentizable.ai/account.html?managed_order={order_id}") });
        if let Some(id) = &policy.payment_method_configuration {
            body["payment_method_configuration"] = id.clone().into();
        }
        Ok(Self(PlanDocument {
            order_id,
            operation_id,
            quote_digest: digest,
            policy,
            created_at: now,
            expires_at,
            retry_before,
            api_version: MANAGED_STRIPE_API_VERSION.into(),
            idempotency_key: format!("managed-order:{order_id}:checkout:{operation_id}"),
            body,
        }))
    }

    pub fn restore(
        value: Value,
        quote: &ManagedQuote,
        order_id: Uuid,
    ) -> Result<Self, CheckoutError> {
        let document: PlanDocument =
            serde_json::from_value(value).map_err(|_| CheckoutError::Binding)?;
        let expected = Self::new(
            quote,
            order_id,
            document.operation_id,
            document.policy.clone(),
            document.created_at,
        )?;
        if expected.0 != document {
            return Err(CheckoutError::Binding);
        }
        Ok(expected)
    }

    pub fn request(
        &self,
        now: i64,
        policy: &CheckoutPolicy,
    ) -> Result<StripeRequestIntent, CheckoutError> {
        if policy != &self.0.policy {
            return Err(CheckoutError::Binding);
        }
        if now < self.0.created_at || now >= self.0.retry_before || now >= self.0.expires_at - 1800
        {
            // Even a retry might be the first request Stripe sees. Never extend expires_at.
            return Err(CheckoutError::ReconciliationRequired);
        }
        Ok(StripeRequestIntent {
            method: "POST".into(),
            endpoint: CHECKOUT_SESSIONS_ENDPOINT.into(),
            api_version: self.0.api_version.clone(),
            idempotency_key: self.0.idempotency_key.clone(),
            body: self.0.body.clone(),
        })
    }

    pub fn order_id(&self) -> Uuid {
        self.0.order_id
    }
    pub fn operation_id(&self) -> Uuid {
        self.0.operation_id
    }
    pub fn quote_digest(&self) -> &str {
        &self.0.quote_digest
    }
    pub fn policy(&self) -> &CheckoutPolicy {
        &self.0.policy
    }
    pub fn created_at(&self) -> i64 {
        self.0.created_at
    }
    pub fn expires_at(&self) -> i64 {
        self.0.expires_at
    }
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::to_vec(self).expect("typed Checkout plan serializes"),
        ))
    }

    /// Validate the authenticated Stripe create/retrieve response before saving the binding.
    /// The caller must obtain it using credentials read-verified for `policy`'s merchant account.
    pub fn session_binding(
        &self,
        response: &Value,
    ) -> Result<ManagedSessionBinding, CheckoutError> {
        let id = response
            .get("id")
            .and_then(Value::as_str)
            .ok_or(CheckoutError::Binding)?;
        if !identifier(
            id,
            if self.0.policy.livemode {
                "cs_live_"
            } else {
                "cs_test_"
            },
        ) || response["object"] != "checkout.session"
            || response["mode"] != "payment"
            || response["livemode"] != self.0.policy.livemode
            || response["client_reference_id"] != self.0.order_id.to_string()
            || response["metadata"] != self.0.body["metadata"]
            || response["currency"] != currency_name(self.0.policy.currency)
            || response["amount_total"].as_u64() != Some(self.total_minor())
            || response["expires_at"].as_i64() != Some(self.0.expires_at)
            || !matches!(
                response["status"].as_str(),
                Some("open" | "complete" | "expired")
            )
        {
            return Err(CheckoutError::Binding);
        }
        let checkout_url = match &response["url"] {
            Value::String(url)
                if url.starts_with("https://checkout.stripe.com/c/pay/")
                    && url.len() <= 8192
                    && !url.chars().any(char::is_control) =>
            {
                Some(url.clone())
            }
            Value::Null if response["status"] != "open" => None,
            _ => return Err(CheckoutError::Binding),
        };
        let payment_intent_id = match &response["payment_intent"] {
            Value::Null => None,
            Value::String(id) if identifier(id, "pi_") => Some(id.clone()),
            _ => return Err(CheckoutError::Binding),
        };
        Ok(ManagedSessionBinding {
            operation_id: self.0.operation_id,
            session_id: id.into(),
            plan_digest: self.digest(),
            checkout_url,
            payment_intent_id,
        })
    }

    fn total_minor(&self) -> u64 {
        self.0.body["line_items"]
            .as_array()
            .expect("validated lines")
            .iter()
            .map(|line| {
                line["price_data"]["unit_amount"]
                    .as_u64()
                    .expect("validated price")
            })
            .sum()
    }
}

#[derive(Debug, Clone)]
pub struct ManagedSessionBinding {
    operation_id: Uuid,
    session_id: String,
    plan_digest: String,
    checkout_url: Option<String>,
    payment_intent_id: Option<String>,
}
impl ManagedSessionBinding {
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn checkout_url(&self) -> Option<&str> {
        self.checkout_url.as_deref()
    }
    pub fn payment_intent_id(&self) -> Option<&str> {
        self.payment_intent_id.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedPaymentOutcome {
    Pending,
    Paid,
    Failed,
    Expired,
}

/// Minimal normalized data, with no customer's card/contact/specification contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedEventData {
    pub event_id: String,
    pub body_sha256: String,
    pub stripe_account_id: String,
    pub livemode: bool,
    pub order_id: Uuid,
    pub operation_id: Uuid,
    pub quote_digest: String,
    pub session_id: String,
    pub payment_intent_id: Option<String>,
    pub amount_minor: u64,
    pub currency: FiatCurrency,
    pub outcome: ManagedPaymentOutcome,
    pub event_created_at: i64,
}

/// Cannot be constructed from client JSON or an operator-provided paid boolean.
#[derive(Debug, Clone)]
pub struct AuthenticatedManagedEvent(ManagedEventData);
impl AuthenticatedManagedEvent {
    pub fn data(&self) -> &ManagedEventData {
        &self.0
    }
}

pub fn verify_managed_checkout_event(
    body: &[u8],
    signature: &str,
    signing_secret: &[u8],
    stripe_account_id: &str,
    livemode: bool,
) -> Result<AuthenticatedManagedEvent, CheckoutError> {
    let event = verify_platform_webhook_event(body, signature, signing_secret, livemode)
        .map_err(|_| CheckoutError::Authentication)?;
    let wire: Value = serde_json::from_slice(body).map_err(|_| CheckoutError::Event)?;
    let created_at = wire["created"]
        .as_i64()
        .filter(|v| *v > 0)
        .ok_or(CheckoutError::Event)?;
    let p = &event.payload;
    let metadata = &p["metadata"];
    let s = |key: &str| p[key].as_str().ok_or(CheckoutError::Event);
    let m = |key: &str| metadata[key].as_str().ok_or(CheckoutError::Event);
    if !identifier(stripe_account_id, "acct_")
        || !identifier(&event.id, "evt_")
        || s("object")? != "checkout.session"
        || s("mode")? != "payment"
        || p["livemode"].as_bool() != Some(livemode)
        || m("purpose")? != MANAGED_CHECKOUT_PURPOSE
        || m("merchant_account_id")? != stripe_account_id
        || !digest_string(m("managed_quote_digest")?)
        || !identifier(s("id")?, if livemode { "cs_live_" } else { "cs_test_" })
    {
        return Err(CheckoutError::Event);
    }
    let order_id = Uuid::parse_str(m("managed_order_id")?).map_err(|_| CheckoutError::Event)?;
    let operation_id =
        Uuid::parse_str(m("managed_operation_id")?).map_err(|_| CheckoutError::Event)?;
    if order_id.is_nil()
        || operation_id.is_nil()
        || s("client_reference_id")? != order_id.to_string()
    {
        return Err(CheckoutError::Event);
    }
    let outcome = match (
        event.event_type.as_str(),
        s("status")?,
        s("payment_status")?,
    ) {
        (
            "checkout.session.completed" | "checkout.session.async_payment_succeeded",
            "complete",
            "paid",
        ) => ManagedPaymentOutcome::Paid,
        ("checkout.session.completed", "complete", "unpaid") => ManagedPaymentOutcome::Pending,
        ("checkout.session.async_payment_failed", "complete", "unpaid") => {
            ManagedPaymentOutcome::Failed
        }
        ("checkout.session.expired", "expired", "unpaid") => ManagedPaymentOutcome::Expired,
        _ => return Err(CheckoutError::Event),
    };
    let payment_intent_id = match &p["payment_intent"] {
        Value::String(id) if identifier(id, "pi_") => Some(id.clone()),
        Value::Null if outcome == ManagedPaymentOutcome::Expired => None,
        _ => return Err(CheckoutError::Event),
    };
    let amount_minor = p["amount_total"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 99_999_999)
        .ok_or(CheckoutError::Event)?;
    let currency = match s("currency")? {
        "usd" => FiatCurrency::Usd,
        "mxn" => FiatCurrency::Mxn,
        _ => return Err(CheckoutError::Event),
    };
    Ok(AuthenticatedManagedEvent(ManagedEventData {
        event_id: event.id,
        body_sha256: hex::encode(Sha256::digest(body)),
        stripe_account_id: stripe_account_id.into(),
        livemode,
        order_id,
        operation_id,
        quote_digest: m("managed_quote_digest")?.into(),
        session_id: s("id")?.into(),
        payment_intent_id,
        amount_minor,
        currency,
        outcome,
        event_created_at: created_at,
    }))
}

#[cfg(test)]
mod tests;
