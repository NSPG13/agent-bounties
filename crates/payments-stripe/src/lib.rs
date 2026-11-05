pub mod invoicing;
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
const WEBHOOK_SIGNATURE_TOLERANCE_SECONDS: u64 = 5 * 60;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StripeIntegrationError {
    #[error("invalid webhook signature")]
    InvalidSignature,
    #[error("Stripe webhook does not match the configured endpoint scope")]
    UnexpectedWebhookScope,
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
pub const CONNECT_TRANSFERS_ENDPOINT: &str = "/v1/transfers";

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
        let builder = if request.method == "GET" {
            self.client.get(&request.url)
        } else {
            self.client
                .post(&request.url)
                .header("idempotency-key", request.idempotency_key)
                .header("content-type", request.content_type)
                .body(request.body)
        };
        let response = builder
            .header("authorization", request.authorization_header)
            .header("stripe-version", request.stripe_version)
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
    pub bounty_id: Option<Id>,
    pub funding_intent_id: Option<Id>,
    pub payment_event: PaymentEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Normalized event for internal reconciliation. HTTP handlers must use
/// [`parse_stripe_webhook_event`] after verifying the raw-body signature;
/// this internal representation is not Stripe's wire format.
pub struct StripeWebhookEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub payload: serde_json::Value,
}

/// Scope authenticated by the signature on a Stripe snapshot webhook.
/// Callers must compare `livemode` and `account` with their configured endpoint
/// scope before reconciling money. An absent account means the platform account,
/// not an arbitrary connected account. Neither field may be inferred from a URL.
#[derive(Debug, Clone)]
pub struct StripeWebhookEnvelope {
    pub event: StripeWebhookEvent,
    pub livemode: bool,
    pub account: Option<String>,
}

/// Authenticate the original bytes and require this endpoint's environment and
/// platform account scope. The signing secret must belong to that account's
/// destination; a connected-account event cannot authorize a platform credit.
pub fn verify_platform_webhook_event(
    body: &[u8],
    signature_header: &str,
    secret: &[u8],
    expected_livemode: bool,
) -> Result<StripeWebhookEvent, StripeIntegrationError> {
    verify_webhook_signature(body, signature_header, secret)?;
    let envelope = parse_stripe_webhook_event(body)?;
    if envelope.livemode != expected_livemode || envelope.account.is_some() {
        return Err(StripeIntegrationError::UnexpectedWebhookScope);
    }
    Ok(envelope.event)
}

#[derive(Deserialize)]
struct StripeSnapshotEvent {
    id: String,
    object: String,
    #[serde(rename = "type")]
    event_type: String,
    livemode: bool,
    account: Option<String>,
    data: StripeSnapshotData,
    #[serde(flatten)]
    additional_fields: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct StripeSnapshotData {
    object: serde_json::Map<String, serde_json::Value>,
}

/// Parse Stripe's snapshot event shape, never the legacy `payload` shape.
/// This only validates schema: the caller must first authenticate the exact raw
/// bytes with [`verify_webhook_signature`] and then enforce endpoint scope.
pub fn parse_stripe_webhook_event(
    body: &[u8],
) -> Result<StripeWebhookEnvelope, StripeIntegrationError> {
    let wire: StripeSnapshotEvent = serde_json::from_slice(body)
        .map_err(|_| StripeIntegrationError::InvalidField("event".to_string()))?;
    if wire.object != "event" || wire.additional_fields.contains_key("payload") {
        return Err(StripeIntegrationError::InvalidField("event".to_string()));
    }
    validate_stripe_identifier(&wire.id, "id")?;
    if wire.event_type.is_empty()
        || wire.event_type.len() > 255
        || !wire
            .event_type
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_')
    {
        return Err(StripeIntegrationError::InvalidField("type".to_string()));
    }
    if let Some(account) = &wire.account {
        validate_stripe_identifier(account, "account")?;
        if !account.starts_with("acct_") {
            return Err(StripeIntegrationError::InvalidField("account".to_string()));
        }
    }
    // Organization destinations use a different routing context. They require
    // an explicit handler; silently treating one as a platform event is unsafe.
    if wire
        .additional_fields
        .get("context")
        .is_some_and(|context| !context.is_null())
    {
        return Err(StripeIntegrationError::InvalidField("context".to_string()));
    }
    let payload = serde_json::Value::Object(wire.data.object);
    let object_id = get_string(&payload, "id")?;
    validate_stripe_identifier(&object_id, "data.object.id")?;
    let object_kind = get_string(&payload, "object")?;
    let expected_kind = match wire.event_type.as_str() {
        "checkout.session.completed"
        | "checkout.session.async_payment_succeeded"
        | "checkout.session.async_payment_failed"
        | "checkout.session.expired" => Some("checkout.session"),
        "transfer.created" => Some("transfer"),
        "payment_intent.succeeded" => Some("payment_intent"),
        _ => None,
    };
    if object_kind.is_empty() || expected_kind.is_some_and(|kind| kind != object_kind) {
        return Err(StripeIntegrationError::InvalidField(
            "data.object.object".to_string(),
        ));
    }
    if let Some(livemode) = payload.get("livemode") {
        if livemode.as_bool() != Some(wire.livemode) {
            return Err(StripeIntegrationError::InvalidField(
                "data.object.livemode".to_string(),
            ));
        }
    }
    Ok(StripeWebhookEnvelope {
        event: StripeWebhookEvent {
            id: wire.id,
            event_type: wire.event_type,
            payload,
        },
        livemode: wire.livemode,
        account: wire.account,
    })
}

fn validate_stripe_identifier(value: &str, field: &str) -> Result<(), StripeIntegrationError> {
    if value.is_empty()
        || value.len() > 255
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(StripeIntegrationError::InvalidField(field.to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectPayoutState {
    pub agent_id: Id,
    pub connected_account_id: Option<String>,
    pub eligible: bool,
    pub status: PayoutStatus,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ConnectAccountSnapshot {
    pub agent_id: Id,
    pub connected_account_id: Option<String>,
    pub payouts_enabled: bool,
    pub disabled_reason: Option<String>,
    pub currently_due: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectTransferRequest {
    pub bounty_id: Id,
    pub proof_record_id: Id,
    pub settlement_id: Id,
    pub payout_intent_id: Id,
    pub agent_id: Id,
    pub connected_account_id: String,
    pub amount: Money,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectTransferEvidence {
    pub transfer_id: String,
    pub connected_account_id: String,
    pub bounty_id: Id,
    pub proof_record_id: Id,
    pub settlement_id: Id,
    pub payout_intent_id: Id,
    pub agent_id: Id,
    pub amount: Money,
    pub payment_event: PaymentEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectAccountV2CreateIntent {
    pub agent_id: Id,
    pub request: StripeRequestIntent,
}

#[derive(Debug, Clone)]
pub struct StripePlanner {
    pub platform_base_url: String,
    pub payment_method_configuration: Option<String>,
}

impl StripePlanner {
    pub fn new(platform_base_url: impl Into<String>) -> Self {
        Self {
            platform_base_url: platform_base_url.into(),
            payment_method_configuration: None,
        }
    }

    pub fn with_payment_method_configuration(
        mut self,
        payment_method_configuration: impl Into<String>,
    ) -> Self {
        let payment_method_configuration = payment_method_configuration.into();
        self.payment_method_configuration =
            normalized_payment_method_configuration(Some(&payment_method_configuration));
        self
    }

    pub fn checkout_top_up(
        &self,
        request: &CheckoutTopUpRequest,
    ) -> Result<StripeRequestIntent, StripeIntegrationError> {
        validate_minimum_charge(&request.amount)?;
        let mut intent = StripeRequestIntent {
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
                            "name": "Agentizable balance top-up"
                        }
                    }
                }],
                "metadata": {
                    "organization_id": request.organization_id.to_string(),
                    "rail": "stripe_fiat_ledger",
                    "purpose": "platform_balance_top_up"
                }
            }),
        };
        apply_checkout_payment_method_configuration(
            &mut intent,
            self.payment_method_configuration.as_deref(),
        )?;
        Ok(intent)
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

    pub fn connect_transfer(
        &self,
        request: &ConnectTransferRequest,
    ) -> Result<StripeRequestIntent, StripeIntegrationError> {
        validate_minimum_charge(&request.amount)?;
        if request.connected_account_id.trim().is_empty() {
            return Err(StripeIntegrationError::MissingField(
                "connected_account_id".to_string(),
            ));
        }
        Ok(StripeRequestIntent {
            method: "POST".to_string(),
            endpoint: CONNECT_TRANSFERS_ENDPOINT.to_string(),
            api_version: STRIPE_API_VERSION.to_string(),
            idempotency_key: format!("connect_transfer:{}", request.payout_intent_id),
            body: serde_json::json!({
                "amount": request.amount.amount,
                "currency": request.amount.currency,
                "destination": request.connected_account_id,
                "transfer_group": format!("bounty:{}", request.bounty_id),
                "metadata": {
                    "purpose": "agent_bounty_fiat_payout",
                    "bounty_id": request.bounty_id.to_string(),
                    "proof_record_id": request.proof_record_id.to_string(),
                    "settlement_id": request.settlement_id.to_string(),
                    "payout_intent_id": request.payout_intent_id.to_string(),
                    "agent_id": request.agent_id.to_string()
                }
            }),
        })
    }
}

pub fn apply_checkout_payment_method_configuration(
    intent: &mut StripeRequestIntent,
    payment_method_configuration: Option<&str>,
) -> Result<(), StripeIntegrationError> {
    let Some(payment_method_configuration) =
        normalized_payment_method_configuration(payment_method_configuration)
    else {
        return Ok(());
    };
    if intent.endpoint != CHECKOUT_SESSIONS_ENDPOINT {
        return Err(StripeIntegrationError::InvalidEndpoint(
            intent.endpoint.clone(),
        ));
    }
    let body = intent
        .body
        .as_object_mut()
        .ok_or_else(|| StripeIntegrationError::InvalidField("body".to_string()))?;
    body.insert(
        "payment_method_configuration".to_string(),
        serde_json::Value::String(payment_method_configuration),
    );
    Ok(())
}

fn normalized_payment_method_configuration(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
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
    // GET only reads an object; every write is a POST with an idempotency key.
    let get = intent.method.eq_ignore_ascii_case("GET");
    if !get && !intent.method.eq_ignore_ascii_case("POST") {
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
    let body = if get {
        String::new()
    } else if intent.endpoint.starts_with("/v2/") {
        serde_json::to_string(&intent.body)
            .map_err(|_| StripeIntegrationError::InvalidField("body".to_string()))?
    } else {
        stripe_form_encode(&intent.body)
    };

    Ok(StripeHttpRequest {
        method: if get { "GET" } else { "POST" }.to_string(),
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
        validate_stripe_identifier(&event.id, "id")?;
        if self.applied.contains(&event.id) {
            return Err(StripeIntegrationError::DuplicateEvent);
        }

        if event.event_type != "checkout.session.completed"
            && event.event_type != "checkout.session.async_payment_succeeded"
            && event.event_type != "payment_intent.succeeded"
            && event.event_type != "transfer.created"
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
        if event.event_type != "checkout.session.completed"
            && event.event_type != "checkout.session.async_payment_succeeded"
        {
            return Err(StripeIntegrationError::UnsupportedEvent(
                event.event_type.clone(),
            ));
        }
        if event
            .payload
            .get("payment_status")
            .and_then(serde_json::Value::as_str)
            != Some("paid")
        {
            return Err(StripeIntegrationError::CheckoutSessionNotPaid);
        }

        let organization_id = get_string(&event.payload, "client_reference_id")?
            .parse()
            .map_err(|_| StripeIntegrationError::InvalidField("client_reference_id".to_string()))?;
        // One account can serve several products. A caller-controlled reference
        // alone is not evidence that this payment belongs to Agentizable.
        if get_string(&event.payload, "mode")? != "payment" {
            return Err(StripeIntegrationError::InvalidField("mode".to_string()));
        }
        let metadata = event
            .payload
            .get("metadata")
            .filter(|value| value.is_object())
            .ok_or_else(|| StripeIntegrationError::InvalidField("metadata".to_string()))?;
        if get_string(metadata, "rail")? != "stripe_fiat_ledger" {
            return Err(StripeIntegrationError::InvalidField(
                "metadata.rail".to_string(),
            ));
        }
        if required_metadata_uuid(&event.payload, "organization_id")? != organization_id {
            return Err(StripeIntegrationError::InvalidField(
                "metadata.organization_id".to_string(),
            ));
        }
        let amount = get_i64(&event.payload, "amount_total")?;
        let currency = get_string(&event.payload, "currency")?;
        let checkout_session_id = get_string(&event.payload, "id")?;
        validate_stripe_identifier(&checkout_session_id, "id")?;
        let payment_intent_id = match event.payload.get("payment_intent") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(id)) => {
                validate_stripe_identifier(id, "payment_intent")?;
                Some(id.clone())
            }
            Some(_) => {
                return Err(StripeIntegrationError::InvalidField(
                    "payment_intent".to_string(),
                ));
            }
        };
        let bounty_id = metadata_uuid(&event.payload, "bounty_id")?;
        let funding_intent_id = metadata_uuid(&event.payload, "funding_intent_id")?;
        match (
            get_string(metadata, "purpose")?.as_str(),
            bounty_id.is_some(),
            funding_intent_id.is_some(),
        ) {
            ("platform_balance_top_up", false, false) | ("bounty_funding_intent", true, true) => {}
            _ => {
                return Err(StripeIntegrationError::InvalidField(
                    "metadata.purpose".to_string(),
                ));
            }
        }
        let amount = stripe_money(amount, currency, "amount_total")?;
        let payment_event = self.apply(event)?;

        Ok(StripeFundingCredit {
            organization_id,
            amount,
            checkout_session_id,
            payment_intent_id,
            bounty_id,
            funding_intent_id,
            payment_event,
        })
    }

    pub fn apply_connect_transfer(
        &mut self,
        event: &StripeWebhookEvent,
    ) -> Result<ConnectTransferEvidence, StripeIntegrationError> {
        if event.event_type != "transfer.created" {
            return Err(StripeIntegrationError::UnsupportedEvent(
                event.event_type.clone(),
            ));
        }
        let object = &event.payload;
        let transfer_id = get_string(object, "id")?;
        let connected_account_id = get_string(object, "destination")?;
        validate_stripe_identifier(&transfer_id, "id")?;
        validate_stripe_identifier(&connected_account_id, "destination")?;
        let amount = get_i64(object, "amount")?;
        let currency = get_string(object, "currency")?;
        let bounty_id = required_metadata_uuid(object, "bounty_id")?;
        let proof_record_id = required_metadata_uuid(object, "proof_record_id")?;
        let settlement_id = required_metadata_uuid(object, "settlement_id")?;
        let payout_intent_id = required_metadata_uuid(object, "payout_intent_id")?;
        let agent_id = required_metadata_uuid(object, "agent_id")?;
        let amount = stripe_money(amount, currency, "amount")?;
        let payment_event = self.apply(event)?;

        Ok(ConnectTransferEvidence {
            transfer_id,
            connected_account_id,
            bounty_id,
            proof_record_id,
            settlement_id,
            payout_intent_id,
            agent_id,
            amount,
            payment_event,
        })
    }
}

fn stripe_money(
    amount: i64,
    currency: String,
    amount_field: &str,
) -> Result<Money, StripeIntegrationError> {
    if currency.len() != 3 || !currency.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Err(StripeIntegrationError::InvalidField("currency".to_string()));
    }
    Money::new(amount, currency)
        .map_err(|_| StripeIntegrationError::InvalidField(amount_field.to_string()))
}

fn required_metadata_uuid(
    payload: &serde_json::Value,
    key: &str,
) -> Result<Id, StripeIntegrationError> {
    metadata_uuid(payload, key)?
        .ok_or_else(|| StripeIntegrationError::MissingField(format!("metadata.{key}")))
}

fn metadata_uuid(
    payload: &serde_json::Value,
    key: &str,
) -> Result<Option<Id>, StripeIntegrationError> {
    let metadata = match payload.get("metadata") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Object(metadata)) => metadata,
        Some(_) => {
            return Err(StripeIntegrationError::InvalidField("metadata".to_string()));
        }
    };
    let Some(value) = metadata.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| StripeIntegrationError::InvalidField(format!("metadata.{key}")))?;
    value
        .parse()
        .map(Some)
        .map_err(|_| StripeIntegrationError::InvalidField(format!("metadata.{key}")))
}

pub fn verify_webhook_signature(
    payload: &[u8],
    signature_header: &str,
    secret: &[u8],
) -> Result<(), StripeIntegrationError> {
    verify_webhook_signature_at(payload, signature_header, secret, Utc::now().timestamp())
}

fn verify_webhook_signature_at(
    payload: &[u8],
    signature_header: &str,
    secret: &[u8],
    now_timestamp: i64,
) -> Result<(), StripeIntegrationError> {
    if secret.is_empty() || secret.iter().all(u8::is_ascii_whitespace) {
        return Err(StripeIntegrationError::InvalidSignature);
    }
    let timestamp = signature_timestamp(signature_header)?;
    if timestamp.abs_diff(now_timestamp) > WEBHOOK_SIGNATURE_TOLERANCE_SECONDS {
        return Err(StripeIntegrationError::InvalidSignature);
    }

    let signed_payload = stripe_signed_payload(timestamp, payload);
    signature_candidates(signature_header)
        .into_iter()
        .any(|candidate| verify_signature_candidate(&signed_payload, candidate, secret))
        .then_some(())
        .ok_or(StripeIntegrationError::InvalidSignature)
}

fn verify_signature_candidate(signed_payload: &[u8], candidate: &str, secret: &[u8]) -> bool {
    let signature = match hex::decode(candidate) {
        Ok(signature) => signature,
        Err(_) => return false,
    };
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts secrets of any size");
    mac.update(signed_payload);
    mac.verify_slice(&signature).is_ok()
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

fn signature_timestamp(header: &str) -> Result<i64, StripeIntegrationError> {
    header
        .split(',')
        .filter_map(|part| part.trim().strip_prefix("t="))
        .find_map(|timestamp| timestamp.parse::<i64>().ok())
        .ok_or(StripeIntegrationError::InvalidSignature)
}

fn signature_candidates(header: &str) -> Vec<&str> {
    header
        .split(',')
        .filter_map(|part| part.trim().strip_prefix("v1="))
        .collect()
}

fn stripe_signed_payload(timestamp: i64, payload: &[u8]) -> Vec<u8> {
    let timestamp = timestamp.to_string();
    let mut signed = Vec::with_capacity(timestamp.len() + 1 + payload.len());
    signed.extend_from_slice(timestamp.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(payload);
    signed
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
        let now_timestamp = 1_700_000_000;
        let signature = stripe_signature_header(payload, secret, now_timestamp);

        verify_webhook_signature_at(payload, &signature, secret, now_timestamp).unwrap();
        assert_eq!(
            verify_webhook_signature_at(payload, "00", secret, now_timestamp).unwrap_err(),
            StripeIntegrationError::InvalidSignature
        );
        assert_eq!(
            verify_webhook_signature_at(
                payload,
                &raw_payload_signature_header(payload, secret, now_timestamp),
                secret,
                now_timestamp
            )
            .unwrap_err(),
            StripeIntegrationError::InvalidSignature
        );
        assert_eq!(
            verify_webhook_signature_at(payload, &signature, secret, now_timestamp + 301)
                .unwrap_err(),
            StripeIntegrationError::InvalidSignature
        );
        for empty_secret in [b"".as_slice(), b" \t\n".as_slice()] {
            let signature = stripe_signature_header(payload, empty_secret, now_timestamp);
            assert_eq!(
                verify_webhook_signature_at(payload, &signature, empty_secret, now_timestamp),
                Err(StripeIntegrationError::InvalidSignature)
            );
        }
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
        assert!(intent.body.get("payment_method_types").is_none());
        assert!(intent.body.get("payment_method_configuration").is_none());
    }

    #[test]
    fn checkout_top_up_can_target_payment_method_configuration() {
        let organization_id = Uuid::new_v4();
        let planner = StripePlanner::new("https://agentbounties.test")
            .with_payment_method_configuration(" pmc_paypal_enabled ");
        let intent = planner
            .checkout_top_up(&CheckoutTopUpRequest {
                organization_id,
                amount: Money::new(5_000, "usd").unwrap(),
                success_url: "https://agentbounties.test/success".to_string(),
                cancel_url: "https://agentbounties.test/cancel".to_string(),
            })
            .unwrap();

        assert_eq!(
            intent.body["payment_method_configuration"],
            "pmc_paypal_enabled"
        );
        assert!(intent.body.get("payment_method_types").is_none());
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
                "payment_intent": "pi_test_paid",
                "mode": "payment",
                "metadata": {
                    "organization_id": organization_id.to_string(),
                    "rail": "stripe_fiat_ledger",
                    "purpose": "platform_balance_top_up"
                }
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

    fn checkout_snapshot(event_type: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "evt_checkout_wire",
            "object": "event",
            "api_version": "2026-02-25.clover",
            "created": 1_700_000_000,
            "livemode": false,
            "type": event_type,
            "data": {
                "object": {
                    "id": "cs_test_paid",
                    "object": "checkout.session",
                    "livemode": false,
                    "client_reference_id": "00000000-0000-4000-8000-000000000001",
                    "amount_total": 5_000,
                    "currency": "usd",
                    "payment_status": "paid",
                    "payment_intent": "pi_test_paid",
                    "mode": "payment",
                    "metadata": {
                        "organization_id": "00000000-0000-4000-8000-000000000001",
                        "rail": "stripe_fiat_ledger",
                        "purpose": "platform_balance_top_up"
                    }
                }
            },
            "pending_webhooks": 1,
            "request": { "id": null, "idempotency_key": null }
        })
    }

    #[test]
    fn platform_webhook_authenticates_raw_bytes_before_parsing() {
        let secret = b"whsec_local_fixture";
        let timestamp = Utc::now().timestamp();
        let snapshot = checkout_snapshot("checkout.session.completed");
        let body = serde_json::to_vec(&snapshot).unwrap();
        let signature = stripe_signature_header(&body, secret, timestamp);
        assert!(verify_platform_webhook_event(&body, &signature, secret, false).is_ok());
        let mut tampered = snapshot;
        tampered["data"]["object"]["amount_total"] = serde_json::json!(50_000);
        for invalid in [serde_json::to_vec(&tampered).unwrap(), b"not json".to_vec()] {
            assert_eq!(
                verify_platform_webhook_event(&invalid, &signature, secret, false).unwrap_err(),
                StripeIntegrationError::InvalidSignature
            );
        }
        assert_eq!(
            verify_platform_webhook_event(&body, &signature, b"wrong_destination", false)
                .unwrap_err(),
            StripeIntegrationError::InvalidSignature
        );
    }

    #[test]
    fn platform_webhook_rejects_wrong_environment_and_connected_account() {
        let secret = b"whsec_local_fixture";
        for live in [false, true] {
            let mut snapshot = checkout_snapshot("checkout.session.completed");
            snapshot["livemode"] = serde_json::json!(live);
            snapshot["data"]["object"]["livemode"] = serde_json::json!(live);
            let body = serde_json::to_vec(&snapshot).unwrap();
            let signature = stripe_signature_header(&body, secret, Utc::now().timestamp());
            assert!(verify_platform_webhook_event(&body, &signature, secret, live).is_ok());
            assert_eq!(
                verify_platform_webhook_event(&body, &signature, secret, !live).unwrap_err(),
                StripeIntegrationError::UnexpectedWebhookScope
            );
            snapshot["account"] = serde_json::json!("acct_other_account");
            let body = serde_json::to_vec(&snapshot).unwrap();
            let signature = stripe_signature_header(&body, secret, Utc::now().timestamp());
            assert_eq!(
                verify_platform_webhook_event(&body, &signature, secret, live).unwrap_err(),
                StripeIntegrationError::UnexpectedWebhookScope
            );
        }
    }

    #[test]
    fn failure_and_expiration_events_require_a_checkout_session_object() {
        for event_type in [
            "checkout.session.async_payment_failed",
            "checkout.session.expired",
        ] {
            let mut snapshot = checkout_snapshot(event_type);
            assert!(parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap()).is_ok());
            snapshot["data"]["object"]["object"] = serde_json::json!("payment_intent");
            assert!(parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap()).is_err());
        }
    }

    #[test]
    fn signed_stripe_snapshot_normalizes_checkout_object_and_preserves_scope() {
        let body = serde_json::to_vec(&checkout_snapshot("checkout.session.completed")).unwrap();
        let secret = b"whsec_local_fixture";
        let timestamp = 1_700_000_000;
        let signature = stripe_signature_header(&body, secret, timestamp);
        verify_webhook_signature_at(&body, &signature, secret, timestamp).unwrap();
        let envelope = parse_stripe_webhook_event(&body).unwrap();
        assert!(!envelope.livemode);
        assert!(envelope.account.is_none());
        assert_eq!(envelope.event.id, "evt_checkout_wire");
        assert_eq!(envelope.event.payload["id"], "cs_test_paid");
        assert!(envelope.event.payload.get("data").is_none());
        let credit = StripeEventDeduper::default()
            .apply_checkout_top_up(&envelope.event)
            .unwrap();
        assert_eq!(credit.amount, Money::new(5_000, "usd").unwrap());
        assert_eq!(credit.payment_event.external_id, "evt_checkout_wire");

        let mut changed = checkout_snapshot("checkout.session.completed");
        changed["data"]["object"]["amount_total"] = serde_json::json!(50_000);
        let changed_body = serde_json::to_vec(&changed).unwrap();
        assert_eq!(
            verify_webhook_signature_at(&changed_body, &signature, secret, timestamp),
            Err(StripeIntegrationError::InvalidSignature)
        );

        changed["livemode"] = serde_json::json!(true);
        changed["data"]["object"]["livemode"] = serde_json::json!(true);
        changed["account"] = serde_json::json!("acct_connected_scope");
        let envelope = parse_stripe_webhook_event(&serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(envelope.livemode);
        assert_eq!(envelope.account.as_deref(), Some("acct_connected_scope"));
    }

    #[test]
    fn snapshot_parser_rejects_legacy_ambiguous_and_malformed_envelopes() {
        let valid = checkout_snapshot("checkout.session.completed");
        let mut invalid = vec![
            serde_json::json!({
                "id": "evt_legacy", "type": "checkout.session.completed",
                "payload": valid["data"]["object"]
            }),
            serde_json::json!(null),
            serde_json::json!([]),
        ];
        for field in ["id", "object", "type", "livemode", "data"] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(field);
            invalid.push(missing);
            let mut null = valid.clone();
            null[field] = serde_json::Value::Null;
            invalid.push(null);
        }
        for (field, value) in [
            ("id", serde_json::json!("")),
            ("id", serde_json::json!("evt_contains whitespace")),
            ("id", serde_json::json!("x".repeat(256))),
            ("object", serde_json::json!("checkout.session")),
            ("type", serde_json::json!("")),
            ("livemode", serde_json::json!("false")),
            ("livemode", serde_json::json!(0)),
            ("account", serde_json::json!(true)),
            ("account", serde_json::json!("")),
            ("account", serde_json::json!("unexpected_scope")),
            ("context", serde_json::json!("acct_organization_scope")),
            ("payload", serde_json::Value::Null),
            ("data", serde_json::json!({"object": []})),
            ("data", serde_json::json!({"object": null})),
            ("data", serde_json::json!({})),
        ] {
            let mut malformed = valid.clone();
            malformed[field] = value;
            invalid.push(malformed);
        }
        for (field, value) in [
            ("id", serde_json::json!("")),
            ("object", serde_json::json!("transfer")),
            ("livemode", serde_json::json!(true)),
            ("livemode", serde_json::json!("false")),
        ] {
            let mut malformed = valid.clone();
            malformed["data"]["object"][field] = value;
            invalid.push(malformed);
        }
        for (index, body) in invalid.iter().enumerate() {
            assert!(
                parse_stripe_webhook_event(&serde_json::to_vec(body).unwrap()).is_err(),
                "invalid envelope case {index} was accepted"
            );
        }
        // Duplicate routing fields must not be accepted with last-value wins.
        let duplicate_mode = format!(
            "{},\"livemode\":true}}",
            valid.to_string().trim_end_matches('}')
        );
        assert!(parse_stripe_webhook_event(duplicate_mode.as_bytes()).is_err());
    }

    #[test]
    fn checkout_status_must_be_explicitly_paid_for_both_success_event_types() {
        for event_type in [
            "checkout.session.completed",
            "checkout.session.async_payment_succeeded",
        ] {
            for status in [
                None,
                Some(serde_json::Value::Null),
                Some(serde_json::json!(false)),
                Some(serde_json::json!(1)),
                Some(serde_json::json!({"status": "paid"})),
                Some(serde_json::json!("unpaid")),
                Some(serde_json::json!("no_payment_required")),
                Some(serde_json::json!("")),
            ] {
                let mut snapshot = checkout_snapshot(event_type);
                let object = snapshot["data"]["object"].as_object_mut().unwrap();
                if let Some(status) = status {
                    object.insert("payment_status".to_string(), status);
                } else {
                    object.remove("payment_status");
                }
                let mut event = parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap())
                    .unwrap()
                    .event;
                let mut deduper = StripeEventDeduper::default();
                assert_eq!(
                    deduper.apply_checkout_top_up(&event).unwrap_err(),
                    StripeIntegrationError::CheckoutSessionNotPaid
                );
                // Rejection must not poison a later valid delivery.
                event.payload["payment_status"] = serde_json::json!("paid");
                assert!(deduper.apply_checkout_top_up(&event).is_ok());
            }
        }
    }

    #[test]
    fn asynchronous_checkout_only_credits_confirmed_success() {
        let mut snapshot = checkout_snapshot("checkout.session.completed");
        snapshot["data"]["object"]["payment_status"] = serde_json::json!("unpaid");
        let mut deduper = StripeEventDeduper::default();
        let unpaid = parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert_eq!(
            deduper.apply_checkout_top_up(&unpaid.event).unwrap_err(),
            StripeIntegrationError::CheckoutSessionNotPaid
        );
        snapshot["id"] = serde_json::json!("evt_async_payment_succeeded");
        snapshot["type"] = serde_json::json!("checkout.session.async_payment_succeeded");
        snapshot["data"]["object"]["payment_status"] = serde_json::json!("paid");
        let paid = parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(deduper.apply_checkout_top_up(&paid.event).is_ok());
        assert_eq!(
            deduper.apply_checkout_top_up(&paid.event).unwrap_err(),
            StripeIntegrationError::DuplicateEvent
        );
        snapshot["id"] = serde_json::json!("evt_async_payment_failed");
        snapshot["type"] = serde_json::json!("checkout.session.async_payment_failed");
        let failed = parse_stripe_webhook_event(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
        assert!(matches!(
            deduper.apply_checkout_top_up(&failed.event),
            Err(StripeIntegrationError::UnsupportedEvent(_))
        ));
    }

    #[test]
    fn invalid_financial_fields_do_not_consume_checkout_event() {
        let envelope = parse_stripe_webhook_event(
            &serde_json::to_vec(&checkout_snapshot("checkout.session.completed")).unwrap(),
        )
        .unwrap();
        for (field, value) in [
            ("amount_total", serde_json::json!(-1)),
            ("amount_total", serde_json::json!(0)),
            ("amount_total", serde_json::json!("5000")),
            ("currency", serde_json::json!("")),
            ("currency", serde_json::json!("dollars")),
            ("id", serde_json::json!("")),
            ("id", serde_json::json!("x".repeat(256))),
            ("payment_intent", serde_json::json!(true)),
            ("payment_intent", serde_json::json!("")),
            ("metadata", serde_json::json!("not-an-object")),
            ("metadata", serde_json::json!({"funding_intent_id": false})),
            ("metadata", serde_json::json!({"bounty_id": null})),
        ] {
            let mut event = envelope.event.clone();
            event.payload[field] = value;
            let mut deduper = StripeEventDeduper::default();
            assert!(deduper.apply_checkout_top_up(&event).is_err(), "{field}");
            assert!(deduper.apply_checkout_top_up(&envelope.event).is_ok());
        }
    }

    #[test]
    fn checkout_credit_requires_the_platforms_payment_namespace() {
        for event_type in [
            "checkout.session.completed",
            "checkout.session.async_payment_succeeded",
        ] {
            let valid = parse_stripe_webhook_event(
                &serde_json::to_vec(&checkout_snapshot(event_type)).unwrap(),
            )
            .unwrap()
            .event;
            for (pointer, replacement) in [
                ("/mode", serde_json::json!("subscription")),
                ("/mode", serde_json::Value::Null),
                ("/metadata", serde_json::Value::Null),
                ("/metadata", serde_json::json!({})),
                ("/metadata/purpose", serde_json::json!("unrelated_product")),
                (
                    "/metadata/purpose",
                    serde_json::json!("bounty_funding_intent"),
                ),
                ("/metadata/purpose", serde_json::Value::Null),
                ("/metadata/rail", serde_json::json!("another_rail")),
                ("/metadata/rail", serde_json::Value::Null),
                (
                    "/metadata/organization_id",
                    serde_json::json!(Uuid::new_v4()),
                ),
                ("/metadata/organization_id", serde_json::Value::Null),
            ] {
                let mut event = valid.clone();
                *event.payload.pointer_mut(pointer).unwrap() = replacement;
                let mut deduper = StripeEventDeduper::default();
                assert!(deduper.apply_checkout_top_up(&event).is_err(), "{pointer}");
                // Invalid evidence must not consume the event's retry identity.
                assert!(deduper.apply_checkout_top_up(&valid).is_ok());
            }
            for key in ["mode", "metadata"] {
                let mut event = valid.clone();
                event.payload.as_object_mut().unwrap().remove(key);
                assert!(StripeEventDeduper::default()
                    .apply_checkout_top_up(&event)
                    .is_err());
            }
        }
    }

    #[test]
    fn checkout_funding_purpose_requires_both_bound_identifiers() {
        let mut event = parse_stripe_webhook_event(
            &serde_json::to_vec(&checkout_snapshot("checkout.session.completed")).unwrap(),
        )
        .unwrap()
        .event;
        let bounty = Uuid::new_v4();
        let intent = Uuid::new_v4();
        event.payload["metadata"]["bounty_id"] = serde_json::json!(bounty);
        event.payload["metadata"]["funding_intent_id"] = serde_json::json!(intent);
        // A top-up cannot quietly acquire bounty-funding semantics.
        assert!(StripeEventDeduper::default()
            .apply_checkout_top_up(&event)
            .is_err());
        event.payload["metadata"]["purpose"] = serde_json::json!("bounty_funding_intent");
        for key in ["bounty_id", "funding_intent_id"] {
            let mut incomplete = event.clone();
            incomplete.payload["metadata"]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(StripeEventDeduper::default()
                .apply_checkout_top_up(&incomplete)
                .is_err());
        }
        let credit = StripeEventDeduper::default()
            .apply_checkout_top_up(&event)
            .unwrap();
        assert_eq!(credit.bounty_id, Some(bounty));
        assert_eq!(credit.funding_intent_id, Some(intent));
    }

    fn stripe_signature_header(payload: &[u8], secret: &[u8], timestamp: i64) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(&stripe_signed_payload(timestamp, payload));
        format!(
            "t={},v1={}",
            timestamp,
            hex::encode(mac.finalize().into_bytes())
        )
    }

    fn raw_payload_signature_header(payload: &[u8], secret: &[u8], timestamp: i64) -> String {
        let mut mac = HmacSha256::new_from_slice(secret).unwrap();
        mac.update(payload);
        format!(
            "t={},v1={}",
            timestamp,
            hex::encode(mac.finalize().into_bytes())
        )
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
    fn connect_transfer_intent_carries_payout_metadata() {
        let bounty_id = Uuid::new_v4();
        let proof_record_id = Uuid::new_v4();
        let settlement_id = Uuid::new_v4();
        let payout_intent_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let planner = StripePlanner::new("https://agentbounties.test");

        let intent = planner
            .connect_transfer(&ConnectTransferRequest {
                bounty_id,
                proof_record_id,
                settlement_id,
                payout_intent_id,
                agent_id,
                connected_account_id: "acct_test_transfer".to_string(),
                amount: Money::new(4_500, "usd").unwrap(),
            })
            .unwrap();

        assert_eq!(intent.endpoint, CONNECT_TRANSFERS_ENDPOINT);
        assert_eq!(
            intent.idempotency_key,
            format!("connect_transfer:{payout_intent_id}")
        );
        assert_eq!(intent.body["amount"], 4_500);
        assert_eq!(intent.body["destination"], "acct_test_transfer");
        assert_eq!(intent.body["metadata"]["bounty_id"], bounty_id.to_string());
        assert_eq!(
            intent.body["metadata"]["payout_intent_id"],
            payout_intent_id.to_string()
        );
    }

    #[test]
    fn transfer_event_creates_payout_evidence_once() {
        let bounty_id = Uuid::new_v4();
        let proof_record_id = Uuid::new_v4();
        let settlement_id = Uuid::new_v4();
        let payout_intent_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let event = StripeWebhookEvent {
            id: "evt_transfer_created".to_string(),
            event_type: "transfer.created".to_string(),
            payload: serde_json::json!({
                "id": "tr_test_paid",
                "destination": "acct_test_transfer",
                "amount": 4_500,
                "currency": "usd",
                "metadata": {
                    "bounty_id": bounty_id.to_string(),
                    "proof_record_id": proof_record_id.to_string(),
                    "settlement_id": settlement_id.to_string(),
                    "payout_intent_id": payout_intent_id.to_string(),
                    "agent_id": agent_id.to_string()
                }
            }),
        };
        let mut deduper = StripeEventDeduper::default();

        let evidence = deduper.apply_connect_transfer(&event).unwrap();

        assert_eq!(evidence.transfer_id, "tr_test_paid");
        assert_eq!(evidence.connected_account_id, "acct_test_transfer");
        assert_eq!(evidence.bounty_id, bounty_id);
        assert_eq!(evidence.proof_record_id, proof_record_id);
        assert_eq!(evidence.settlement_id, settlement_id);
        assert_eq!(evidence.payout_intent_id, payout_intent_id);
        assert_eq!(evidence.agent_id, agent_id);
        assert_eq!(evidence.amount, Money::new(4_500, "usd").unwrap());
        assert_eq!(
            deduper.apply_connect_transfer(&event).unwrap_err(),
            StripeIntegrationError::DuplicateEvent
        );
    }

    #[test]
    fn wire_transfer_normalizes_once_and_invalid_amount_does_not_consume_event() {
        let body = serde_json::json!({
            "id": "evt_transfer_snapshot",
            "object": "event",
            "type": "transfer.created",
            "livemode": false,
            "data": {
                "object": {
                    "id": "tr_test_snapshot",
                    "object": "transfer",
                    "destination": "acct_recipient",
                    "amount": 4_500,
                    "currency": "usd",
                    "metadata": {
                        "bounty_id": "00000000-0000-4000-8000-000000000001",
                        "proof_record_id": "00000000-0000-4000-8000-000000000002",
                        "settlement_id": "00000000-0000-4000-8000-000000000003",
                        "payout_intent_id": "00000000-0000-4000-8000-000000000004",
                        "agent_id": "00000000-0000-4000-8000-000000000005"
                    }
                }
            }
        });
        let valid = parse_stripe_webhook_event(&serde_json::to_vec(&body).unwrap())
            .unwrap()
            .event;
        for (field, value) in [
            ("amount", serde_json::json!(-1)),
            ("amount", serde_json::json!(0)),
            ("currency", serde_json::json!("")),
            ("id", serde_json::json!("")),
            ("destination", serde_json::json!("")),
            ("destination", serde_json::json!("x".repeat(256))),
        ] {
            let mut invalid = valid.clone();
            invalid.payload[field] = value;
            let mut deduper = StripeEventDeduper::default();
            assert!(deduper.apply_connect_transfer(&invalid).is_err(), "{field}");
            let evidence = deduper.apply_connect_transfer(&valid).unwrap();
            assert_eq!(evidence.amount, Money::new(4_500, "usd").unwrap());
            assert_eq!(evidence.transfer_id, "tr_test_snapshot");
            assert_eq!(evidence.connected_account_id, "acct_recipient");
        }
        let mut nested = valid.clone();
        nested.payload = serde_json::json!({"data": {"object": valid.payload}});
        assert!(StripeEventDeduper::default()
            .apply_connect_transfer(&nested)
            .is_err());
    }

    #[test]
    fn checkout_execution_uses_form_encoded_stripe_request() {
        let organization_id = Uuid::new_v4();
        let intent = StripePlanner::new("https://agentbounties.test")
            .with_payment_method_configuration("pmc_paypal_enabled")
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
            .contains(&format!("client_reference_id={organization_id}")));
        assert!(request
            .body
            .contains("payment_method_configuration=pmc_paypal_enabled"));
        assert!(!request.body.contains("payment_method_types"));
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
