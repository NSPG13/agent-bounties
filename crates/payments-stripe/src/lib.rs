use async_trait::async_trait;
use chrono::Utc;
use domain::{Id, Money, PaymentEvent, PaymentEventStatus, PaymentRail, PayoutStatus};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StripeIntegrationError {
    #[error("invalid webhook signature")]
    InvalidSignature,
    #[error("duplicate Stripe event")]
    DuplicateEvent,
    #[error("unsupported Stripe event type: {0}")]
    UnsupportedEvent(String),
    #[error("amount is below Stripe minimum charge for {currency}: {minimum_minor}")]
    BelowMinimumCharge {
        currency: String,
        minimum_minor: i64,
    },
    #[error("checkout session was not paid")]
    CheckoutSessionNotPaid,
    #[error("missing field in Stripe payload: {0}")]
    MissingField(String),
    #[error("invalid field in Stripe payload: {0}")]
    InvalidField(String),
    #[error("unsupported Stripe request method: {0}")]
    UnsupportedMethod(String),
    #[error("invalid Stripe endpoint: {0}")]
    InvalidEndpoint(String),
    #[error("Stripe API request failed with status {status}: {body}")]
    RequestFailed { status: u16, body: String },
    #[error("Stripe HTTP transport failed: {0}")]
    HttpTransport(String),
}

pub const STRIPE_API_VERSION: &str = "2026-02-25.clover";
pub const STRIPE_API_BASE_URL: &str = "https://api.stripe.com";
pub const CHECKOUT_SESSIONS_ENDPOINT: &str = "/v1/checkout/sessions";
pub const CONNECT_ACCOUNTS_V2_ENDPOINT: &str = "/v2/core/accounts";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutTopUpRequest {
    pub organization_id: Id,
    pub amount: Money,
    pub success_url: String,
    pub cancel_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutTopUpSession {
    pub id: String,
    pub url: String,
    pub amount: Money,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeRequestIntent {
    pub method: String,
    pub endpoint: String,
    pub api_version: String,
    pub idempotency_key: String,
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeExecutionReport {
    pub request: StripeRequestIntent,
    pub status: u16,
    pub stripe_id: Option<String>,
    pub object: Option<String>,
    pub url: Option<String>,
    pub livemode: Option<bool>,
    pub response: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct StripeHttpRequest {
    pub method: String,
    pub url: String,
    pub authorization_header: String,
    pub stripe_version: String,
    pub idempotency_key: String,
    pub content_type: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct StripeHttpResponse {
    pub status: u16,
    pub body: String,
}

#[async_trait]
pub trait StripeHttpTransport {
    async fn post(
        &self,
        request: StripeHttpRequest,
    ) -> Result<StripeHttpResponse, StripeIntegrationError>;
}

#[derive(Debug, Default, Clone)]
pub struct ReqwestStripeHttpTransport {
    client: reqwest::Client,
}

impl ReqwestStripeHttpTransport {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl StripeHttpTransport for ReqwestStripeHttpTransport {
    async fn post(
        &self,
        request: StripeHttpRequest,
    ) -> Result<StripeHttpResponse, StripeIntegrationError> {
        let response = self
            .client
            .post(&request.url)
            .header("authorization", request.authorization_header)
            .header("stripe-version", request.stripe_version)
            .header("idempotency-key", request.idempotency_key)
            .header("content-type", request.content_type)
            .body(request.body)
            .send()
            .await
            .map_err(|error| StripeIntegrationError::HttpTransport(error.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|error| StripeIntegrationError::HttpTransport(error.to_string()))?;
        Ok(StripeHttpResponse { status, body })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeFundingCredit {
    pub organization_id: Id,
    pub amount: Money,
    pub checkout_session_id: String,
    pub payment_intent_id: Option<String>,
    pub payment_event: PaymentEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeWebhookEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectPayoutState {
    pub agent_id: Id,
    pub connected_account_id: Option<String>,
    pub eligible: bool,
    pub status: PayoutStatus,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAccountSnapshot {
    pub agent_id: Id,
    pub connected_account_id: Option<String>,
    pub payouts_enabled: bool,
    pub disabled_reason: Option<String>,
    pub currently_due: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAccountV2CreateIntent {
    pub agent_id: Id,
    pub request: StripeRequestIntent,
}

#[derive(Debug, Clone)]
pub struct StripePlanner {
    pub platform_base_url: String,
}

impl StripePlanner {
    pub fn new(platform_base_url: impl Into<String>) -> Self {
        Self {
            platform_base_url: platform_base_url.into(),
        }
    }

    pub fn checkout_top_up(
        &self,
        request: &CheckoutTopUpRequest,
    ) -> Result<StripeRequestIntent, StripeIntegrationError> {
        validate_minimum_charge(&request.amount)?;
        Ok(StripeRequestIntent {
            method: "POST".to_string(),
            endpoint: CHECKOUT_SESSIONS_ENDPOINT.to_string(),
            api_version: STRIPE_API_VERSION.to_string(),
            idempotency_key: format!(
                "checkout_top_up:{}:{}:{}",
                request.organization_id, request.amount.currency, request.amount.amount
            ),
            body: serde_json::json!({
                "mode": "payment",
                "success_url": request.success_url,
                "cancel_url": request.cancel_url,
                "client_reference_id": request.organization_id.to_string(),
                "line_items": [{
                    "quantity": 1,
                    "price_data": {
                        "currency": request.amount.currency,
                        "unit_amount": request.amount.amount,
                        "product_data": {
                            "name": "Agent Bounties balance top-up"
                        }
                    }
                }],
                "metadata": {
                    "organization_id": request.organization_id.to_string(),
                    "rail": "stripe_fiat_ledger",
                    "purpose": "platform_balance_top_up"
                }
            }),
        })
    }

    pub fn connect_account_v2(
        &self,
        agent_id: Id,
    ) -> Result<ConnectAccountV2CreateIntent, StripeIntegrationError> {
        Ok(ConnectAccountV2CreateIntent {
            agent_id,
            request: StripeRequestIntent {
                method: "POST".to_string(),
                endpoint: CONNECT_ACCOUNTS_V2_ENDPOINT.to_string(),
                api_version: STRIPE_API_VERSION.to_string(),
                idempotency_key: format!("connect_account_v2:{agent_id}"),
                body: serde_json::json!({
                    "contact_email": null,
                    "metadata": {
                        "agent_id": agent_id.to_string(),
                        "purpose": "agent_bounty_fiat_payouts"
                    },
                    "identity": {
                        "business_details": {
                            "registered_name": null
                        }
                    },
                    "configuration": {
                        "merchant": {
                            "capabilities": {
                                "card_payments": { "requested": true },
                                "transfers": { "requested": true }
                            }
                        },
                        "recipient": {
                            "capabilities": {
                                "stripe_balance": { "requested": true }
                            }
                        }
                    },
                    "dashboard": {
                        "type": "express"
                    },
                    "defaults": {
                        "responsibilities": {
                            "fees_collector": "application",
                            "losses_collector": "application"
                        }
                    }
                }),
            },
        })
    }
}

pub async fn execute_stripe_request(
    intent: &StripeRequestIntent,
    secret_key: &str,
    api_base_url: &str,
) -> Result<StripeExecutionReport, StripeIntegrationError> {
    execute_stripe_request_with_transport(
        intent,
        secret_key,
        api_base_url,
        &ReqwestStripeHttpTransport::new(),
    )
    .await
}

pub async fn execute_stripe_request_with_transport<T: StripeHttpTransport + Sync>(
    intent: &StripeRequestIntent,
    secret_key: &str,
    api_base_url: &str,
    transport: &T,
) -> Result<StripeExecutionReport, StripeIntegrationError> {
    let http_request = build_stripe_http_request(intent, secret_key, api_base_url)?;
    let response = transport.post(http_request).await?;
    parse_stripe_execution_response(intent, response)
}

pub fn build_stripe_http_request(
    intent: &StripeRequestIntent,
    secret_key: &str,
    api_base_url: &str,
) -> Result<StripeHttpRequest, StripeIntegrationError> {
    if !intent.method.eq_ignore_ascii_case("POST") {
        return Err(StripeIntegrationError::UnsupportedMethod(
            intent.method.clone(),
        ));
    }
    if !intent.endpoint.starts_with('/') {
        return Err(StripeIntegrationError::InvalidEndpoint(
            intent.endpoint.clone(),
        ));
    }
    if secret_key.trim().is_empty() {
        return Err(StripeIntegrationError::InvalidField(
            "secret_key".to_string(),
        ));
    }

    let api_base_url = api_base_url.trim_end_matches('/');
    let url = format!("{api_base_url}{}", intent.endpoint);
    let content_type = if intent.endpoint.starts_with("/v2/") {
        "application/json"
    } else {
        "application/x-www-form-urlencoded"
    };
    let body = if intent.endpoint.starts_with("/v2/") {
        serde_json::to_string(&intent.body)
            .map_err(|_| StripeIntegrationError::InvalidField("body".to_string()))?
    } else {
        stripe_form_encode(&intent.body)
    };

    Ok(StripeHttpRequest {
        method: "POST".to_string(),
        url,
        authorization_header: format!("Bearer {}", secret_key.trim()),
        stripe_version: intent.api_version.clone(),
        idempotency_key: intent.idempotency_key.clone(),
        content_type: content_type.to_string(),
        body,
    })
}

pub fn stripe_form_encode(body: &serde_json::Value) -> String {
    let mut pairs = Vec::new();
    flatten_form_value(None, body, &mut pairs);
    pairs
        .into_iter()
        .map(|(key, value)| format!("{}={}", form_escape(&key), form_escape(&value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn parse_stripe_execution_response(
    intent: &StripeRequestIntent,
    response: StripeHttpResponse,
) -> Result<StripeExecutionReport, StripeIntegrationError> {
    if !(200..300).contains(&response.status) {
        return Err(StripeIntegrationError::RequestFailed {
            status: response.status,
            body: response.body,
        });
    }
    let value: serde_json::Value = serde_json::from_str(&response.body)
        .map_err(|_| StripeIntegrationError::InvalidField("stripe_response".to_string()))?;
    Ok(StripeExecutionReport {
        request: intent.clone(),
        status: response.status,
        stripe_id: value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        object: value
            .get("object")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        url: value
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string),
        livemode: value.get("livemode").and_then(serde_json::Value::as_bool),
        response: value,
    })
}

fn flatten_form_value(
    key: Option<String>,
    value: &serde_json::Value,
    pairs: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Null => {}
        serde_json::Value::Bool(value) => {
            if let Some(key) = key {
                pairs.push((key, value.to_string()));
            }
        }
        serde_json::Value::Number(value) => {
            if let Some(key) = key {
                pairs.push((key, value.to_string()));
            }
        }
        serde_json::Value::String(value) => {
            if let Some(key) = key {
                pairs.push((key, value.clone()));
            }
        }
        serde_json::Value::Array(values) => {
            if let Some(key) = key {
                for (index, value) in values.iter().enumerate() {
                    flatten_form_value(Some(format!("{key}[{index}]")), value, pairs);
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (child_key, child_value) in map {
                let next_key = match &key {
                    Some(parent) => format!("{parent}[{child_key}]"),
                    None => child_key.clone(),
                };
                flatten_form_value(Some(next_key), child_value, pairs);
            }
        }
    }
}

fn form_escape(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[derive(Debug, Default)]
pub struct StripeEventDeduper {
    applied: HashSet<String>,
}

impl StripeEventDeduper {
    pub fn apply(
        &mut self,
        event: &StripeWebhookEvent,
    ) -> Result<PaymentEvent, StripeIntegrationError> {
        if self.applied.contains(&event.id) {
            return Err(StripeIntegrationError::DuplicateEvent);
        }

        if event.event_type != "checkout.session.completed"
            && event.event_type != "payment_intent.succeeded"
        {
            return Err(StripeIntegrationError::UnsupportedEvent(
                event.event_type.clone(),
            ));
        }

        self.applied.insert(event.id.clone());
        Ok(PaymentEvent {
            id: Uuid::new_v4(),
            rail: PaymentRail::StripeFiat,
            external_id: event.id.clone(),
            status: PaymentEventStatus::Applied,
            payload_hash: hash_payload(&event.payload),
            received_at: Utc::now(),
        })
    }

    pub fn apply_checkout_top_up(
        &mut self,
        event: &StripeWebhookEvent,
    ) -> Result<StripeFundingCredit, StripeIntegrationError> {
        if event.event_type != "checkout.session.completed" {
            return Err(StripeIntegrationError::UnsupportedEvent(
                event.event_type.clone(),
            ));
        }
        let payment_status = event
            .payload
            .get("payment_status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("paid");
        if payment_status != "paid" {
            return Err(StripeIntegrationError::CheckoutSessionNotPaid);
        }

        let organization_id = get_string(&event.payload, "client_reference_id")?
            .parse()
            .map_err(|_| StripeIntegrationError::InvalidField("client_reference_id".to_string()))?;
        let amount = get_i64(&event.payload, "amount_total")?;
        let currency = get_string(&event.payload, "currency")?;
        let checkout_session_id = get_string(&event.payload, "id")?;
        let payment_intent_id = event
            .payload
            .get("payment_intent")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let payment_event = self.apply(event)?;

        Ok(StripeFundingCredit {
            organization_id,
            amount: Money::new(amount, currency)
                .map_err(|_| StripeIntegrationError::InvalidField("amount_total".to_string()))?,
            checkout_session_id,
            payment_intent_id,
            payment_event,
        })
    }
}

pub fn verify_webhook_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &[u8],
) -> Result<(), StripeIntegrationError> {
    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|_| StripeIntegrationError::InvalidSignature)?;
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    signature_candidates(signature_header)
        .into_iter()
        .any(|candidate| {
            hex::decode(candidate)
                .map(|signature| signature.as_slice() == expected.as_slice())
                .unwrap_or(false)
        })
        .then_some(())
        .ok_or(StripeIntegrationError::InvalidSignature)
}

pub fn hash_payload(payload: &serde_json::Value) -> String {
    let mut hasher = Sha256::new();
    hasher.update(payload.to_string());
    hex::encode(hasher.finalize())
}

pub fn validate_minimum_charge(amount: &Money) -> Result<(), StripeIntegrationError> {
    let minimum = minimum_charge_minor(&amount.currency);
    if amount.amount < minimum {
        return Err(StripeIntegrationError::BelowMinimumCharge {
            currency: amount.currency.clone(),
            minimum_minor: minimum,
        });
    }
    Ok(())
}

pub fn minimum_charge_minor(currency: &str) -> i64 {
    match currency.to_ascii_lowercase().as_str() {
        "usd" => 50,
        _ => 1,
    }
}

pub fn evaluate_connect_payout(snapshot: &ConnectAccountSnapshot) -> ConnectPayoutState {
    let blocked_reason = if snapshot.connected_account_id.is_none() {
        Some("connected account not created".to_string())
    } else if let Some(reason) = &snapshot.disabled_reason {
        Some(reason.clone())
    } else if !snapshot.currently_due.is_empty() {
        Some(format!(
            "requirements due: {}",
            snapshot.currently_due.join(",")
        ))
    } else if !snapshot.payouts_enabled {
        Some("payouts are not enabled".to_string())
    } else {
        None
    };

    ConnectPayoutState {
        agent_id: snapshot.agent_id,
        connected_account_id: snapshot.connected_account_id.clone(),
        eligible: blocked_reason.is_none(),
        status: if blocked_reason.is_none() {
            PayoutStatus::Pending
        } else {
            PayoutStatus::Blocked
        },
        blocked_reason,
    }
}

fn get_string(payload: &serde_json::Value, key: &str) -> Result<String, StripeIntegrationError> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| StripeIntegrationError::MissingField(key.to_string()))
}

fn get_i64(payload: &serde_json::Value, key: &str) -> Result<i64, StripeIntegrationError> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| StripeIntegrationError::MissingField(key.to_string()))
}

fn signature_candidates(header: &str) -> Vec<&str> {
    header
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            part.strip_prefix("v1=")
                .or_else(|| if part.contains('=') { None } else { Some(part) })
        })
        .collect()
}

pub trait StripeClient {
    fn create_checkout_top_up(&self, request: CheckoutTopUpRequest) -> CheckoutTopUpSession;
}

#[derive(Debug, Default)]
pub struct StubStripeClient;

impl StripeClient for StubStripeClient {
    fn create_checkout_top_up(&self, request: CheckoutTopUpRequest) -> CheckoutTopUpSession {
        CheckoutTopUpSession {
            id: format!("cs_test_{}", Uuid::new_v4().simple()),
            url: format!(
                "https://checkout.stripe.com/c/pay/{}",
                request.organization_id.simple()
            ),
            amount: request.amount,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    struct MockStripeTransport {
        request: Arc<Mutex<Option<StripeHttpRequest>>>,
        response: StripeHttpResponse,
    }

    #[async_trait]
    impl StripeHttpTransport for MockStripeTransport {
        async fn post(
            &self,
            request: StripeHttpRequest,
        ) -> Result<StripeHttpResponse, StripeIntegrationError> {
            *self.request.lock().expect("request lock") = Some(request);
            Ok(self.response.clone())
        }
    }

    #[test]
    fn duplicate_events_are_rejected() {
        let mut deduper = StripeEventDeduper::default();
        let event = StripeWebhookEvent {
            id: "evt_1".to_string(),
            event_type: "checkout.session.completed".to_string(),
            payload: serde_json::json!({"amount_total": 5000}),
        };

        deduper.apply(&event).unwrap();
        assert_eq!(
            deduper.apply(&event).unwrap_err(),
            StripeIntegrationError::DuplicateEvent
        );
    }

    #[test]
    fn webhook_signature_is_checked() {
        let payload = br#"{"id":"evt_1"}"#;
        let secret = b"whsec_test";
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload);
        let signature = hex::encode(mac.finalize().into_bytes());

        verify_webhook_signature(payload, &signature, secret).unwrap();
        verify_webhook_signature(payload, &format!("t=123,v1={signature}"), secret).unwrap();
        assert_eq!(
            verify_webhook_signature(payload, "00", secret).unwrap_err(),
            StripeIntegrationError::InvalidSignature
        );
    }

    #[test]
    fn checkout_top_up_uses_checkout_sessions_and_metadata() {
        let organization_id = Uuid::new_v4();
        let planner = StripePlanner::new("https://agentbounties.test");
        let intent = planner
            .checkout_top_up(&CheckoutTopUpRequest {
                organization_id,
                amount: Money::new(5_000, "usd").unwrap(),
                success_url: "https://agentbounties.test/success".to_string(),
                cancel_url: "https://agentbounties.test/cancel".to_string(),
            })
            .unwrap();

        assert_eq!(intent.method, "POST");
        assert_eq!(intent.endpoint, CHECKOUT_SESSIONS_ENDPOINT);
        assert_eq!(intent.api_version, STRIPE_API_VERSION);
        assert_eq!(intent.body["mode"], "payment");
        assert_eq!(
            intent.body["client_reference_id"],
            organization_id.to_string()
        );
        assert_eq!(intent.body["metadata"]["rail"], "stripe_fiat_ledger");
        assert_eq!(
            intent.body["line_items"][0]["price_data"]["unit_amount"],
            5_000
        );
    }

    #[test]
    fn checkout_top_up_rejects_below_minimum_usd_charge() {
        let planner = StripePlanner::new("https://agentbounties.test");
        let err = planner
            .checkout_top_up(&CheckoutTopUpRequest {
                organization_id: Uuid::new_v4(),
                amount: Money::new(49, "usd").unwrap(),
                success_url: "https://agentbounties.test/success".to_string(),
                cancel_url: "https://agentbounties.test/cancel".to_string(),
            })
            .unwrap_err();

        assert_eq!(
            err,
            StripeIntegrationError::BelowMinimumCharge {
                currency: "usd".to_string(),
                minimum_minor: 50
            }
        );
    }

    #[test]
    fn paid_checkout_webhook_creates_funding_credit_once() {
        let organization_id = Uuid::new_v4();
        let event = StripeWebhookEvent {
            id: "evt_checkout_paid".to_string(),
            event_type: "checkout.session.completed".to_string(),
            payload: serde_json::json!({
                "id": "cs_test_paid",
                "client_reference_id": organization_id.to_string(),
                "amount_total": 5_000,
                "currency": "usd",
                "payment_status": "paid",
                "payment_intent": "pi_test_paid"
            }),
        };
        let mut deduper = StripeEventDeduper::default();

        let credit = deduper.apply_checkout_top_up(&event).unwrap();

        assert_eq!(credit.organization_id, organization_id);
        assert_eq!(credit.amount, Money::new(5_000, "usd").unwrap());
        assert_eq!(credit.checkout_session_id, "cs_test_paid");
        assert_eq!(credit.payment_intent_id.as_deref(), Some("pi_test_paid"));
        assert_eq!(credit.payment_event.rail, PaymentRail::StripeFiat);
        assert_eq!(
            deduper.apply_checkout_top_up(&event).unwrap_err(),
            StripeIntegrationError::DuplicateEvent
        );
    }

    #[test]
    fn unpaid_checkout_webhook_does_not_credit_funds() {
        let event = StripeWebhookEvent {
            id: "evt_checkout_unpaid".to_string(),
            event_type: "checkout.session.completed".to_string(),
            payload: serde_json::json!({
                "id": "cs_test_unpaid",
                "client_reference_id": Uuid::new_v4().to_string(),
                "amount_total": 5_000,
                "currency": "usd",
                "payment_status": "unpaid"
            }),
        };
        let mut deduper = StripeEventDeduper::default();

        assert_eq!(
            deduper.apply_checkout_top_up(&event).unwrap_err(),
            StripeIntegrationError::CheckoutSessionNotPaid
        );
    }

    #[test]
    fn connect_payout_requires_enabled_clear_account() {
        let agent_id = Uuid::new_v4();
        let blocked = evaluate_connect_payout(&ConnectAccountSnapshot {
            agent_id,
            connected_account_id: Some("acct_test".to_string()),
            payouts_enabled: false,
            disabled_reason: None,
            currently_due: vec![],
        });

        assert!(!blocked.eligible);
        assert_eq!(blocked.status, PayoutStatus::Blocked);
        assert_eq!(
            blocked.blocked_reason.as_deref(),
            Some("payouts are not enabled")
        );

        let eligible = evaluate_connect_payout(&ConnectAccountSnapshot {
            agent_id,
            connected_account_id: Some("acct_test".to_string()),
            payouts_enabled: true,
            disabled_reason: None,
            currently_due: vec![],
        });

        assert!(eligible.eligible);
        assert_eq!(eligible.status, PayoutStatus::Pending);
        assert_eq!(eligible.blocked_reason, None);
    }

    #[test]
    fn connect_account_intent_uses_accounts_v2() {
        let agent_id = Uuid::new_v4();
        let planner = StripePlanner::new("https://agentbounties.test");

        let intent = planner.connect_account_v2(agent_id).unwrap();

        assert_eq!(intent.request.endpoint, CONNECT_ACCOUNTS_V2_ENDPOINT);
        assert_eq!(intent.request.api_version, STRIPE_API_VERSION);
        assert_eq!(
            intent.request.body["metadata"]["agent_id"],
            agent_id.to_string()
        );
        assert_eq!(intent.request.body["dashboard"]["type"], "express");
    }

    #[test]
    fn checkout_execution_uses_form_encoded_stripe_request() {
        let organization_id = Uuid::new_v4();
        let intent = StripePlanner::new("https://agentbounties.test")
            .checkout_top_up(&CheckoutTopUpRequest {
                organization_id,
                amount: Money::new(5_000, "usd").unwrap(),
                success_url: "https://agentbounties.test/success".to_string(),
                cancel_url: "https://agentbounties.test/cancel".to_string(),
            })
            .unwrap();

        let request =
            build_stripe_http_request(&intent, "sk_test_mock", STRIPE_API_BASE_URL).unwrap();

        assert_eq!(request.url, "https://api.stripe.com/v1/checkout/sessions");
        assert_eq!(request.content_type, "application/x-www-form-urlencoded");
        assert_eq!(request.authorization_header, "Bearer sk_test_mock");
        assert_eq!(request.stripe_version, STRIPE_API_VERSION);
        assert!(request.body.contains("mode=payment"));
        assert!(request
            .body
            .contains("line_items%5B0%5D%5Bprice_data%5D%5Bunit_amount%5D=5000"));
        assert!(request
            .body
            .contains(&format!("client_reference_id={}", organization_id)));
    }

    #[test]
    fn connect_execution_uses_json_stripe_request() {
        let agent_id = Uuid::new_v4();
        let intent = StripePlanner::new("https://agentbounties.test")
            .connect_account_v2(agent_id)
            .unwrap()
            .request;

        let request =
            build_stripe_http_request(&intent, "sk_test_mock", "https://api.stripe.test").unwrap();

        assert_eq!(request.url, "https://api.stripe.test/v2/core/accounts");
        assert_eq!(request.content_type, "application/json");
        assert!(request.body.contains("\"configuration\""));
        assert!(request.body.contains(&agent_id.to_string()));
    }

    #[tokio::test]
    async fn executes_stripe_request_through_transport_and_parses_report() {
        let intent = StripePlanner::new("https://agentbounties.test")
            .checkout_top_up(&CheckoutTopUpRequest {
                organization_id: Uuid::new_v4(),
                amount: Money::new(5_000, "usd").unwrap(),
                success_url: "https://agentbounties.test/success".to_string(),
                cancel_url: "https://agentbounties.test/cancel".to_string(),
            })
            .unwrap();
        let captured = Arc::new(Mutex::new(None));
        let transport = MockStripeTransport {
            request: captured.clone(),
            response: StripeHttpResponse {
                status: 200,
                body: serde_json::json!({
                    "id": "cs_test_123",
                    "object": "checkout.session",
                    "url": "https://checkout.stripe.com/c/pay/cs_test_123",
                    "livemode": false
                })
                .to_string(),
            },
        };

        let report = execute_stripe_request_with_transport(
            &intent,
            "sk_test_mock",
            "https://api.stripe.test",
            &transport,
        )
        .await
        .unwrap();

        assert_eq!(report.status, 200);
        assert_eq!(report.stripe_id.as_deref(), Some("cs_test_123"));
        assert_eq!(report.object.as_deref(), Some("checkout.session"));
        assert_eq!(
            report.url.as_deref(),
            Some("https://checkout.stripe.com/c/pay/cs_test_123")
        );
        let captured = captured.lock().expect("request lock").clone().unwrap();
        assert_eq!(captured.url, "https://api.stripe.test/v1/checkout/sessions");
        assert_eq!(captured.idempotency_key, intent.idempotency_key);
    }

    #[tokio::test]
    async fn non_successful_stripe_execution_is_an_error() {
        let intent = StripePlanner::new("https://agentbounties.test")
            .connect_account_v2(Uuid::new_v4())
            .unwrap()
            .request;
        let transport = MockStripeTransport {
            request: Arc::new(Mutex::new(None)),
            response: StripeHttpResponse {
                status: 402,
                body: "{\"error\":{\"message\":\"blocked\"}}".to_string(),
            },
        };

        let error = execute_stripe_request_with_transport(
            &intent,
            "sk_test_mock",
            "https://api.stripe.test",
            &transport,
        )
        .await
        .unwrap_err();

        assert_eq!(
            error,
            StripeIntegrationError::RequestFailed {
                status: 402,
                body: "{\"error\":{\"message\":\"blocked\"}}".to_string()
            }
        );
    }
}
