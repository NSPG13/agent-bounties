//! Original-payment refunds: the full pre-funding refund and the partial refund of the review
//! deposit after the bounty is paid. Request/API binding is not signed refund evidence.
use crate::{stripe_checkout::MANAGED_STRIPE_API_VERSION, FiatCurrency, ManagedQuote};
use payments_stripe::{verify_platform_webhook_event, StripeRequestIntent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const MANAGED_REFUND_PURPOSE: &str = "managed_service_refund";
pub const MANAGED_RECOVERED_REFUND_PURPOSE: &str = "managed_recovered_bounty_refund";
pub const MANAGED_DEPOSIT_REFUND_PURPOSE: &str = "managed_review_deposit_refund";

/// What a refund returns. Each kind has its own operation, so a paid bounty's deposit refund
/// never shares state with the full pre-funding refund.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundKind {
    /// The whole original payment, before funding.
    #[default]
    Full,
    /// Only the quote's `review_deposit` line, after the bounty is paid without a reversal.
    ReviewDeposit,
    /// Original solver and verification amounts only, after complete principal recovery.
    RecoveredBounty,
}

impl RefundKind {
    pub fn is_full(&self) -> bool {
        *self == Self::Full
    }
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Full => MANAGED_REFUND_PURPOSE,
            Self::ReviewDeposit => MANAGED_DEPOSIT_REFUND_PURPOSE,
            Self::RecoveredBounty => MANAGED_RECOVERED_REFUND_PURPOSE,
        }
    }
    fn from_purpose(purpose: &str) -> Option<Self> {
        [Self::Full, Self::ReviewDeposit, Self::RecoveredBounty]
            .into_iter()
            .find(|kind| kind.purpose() == purpose)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RefundError {
    #[error("managed refund does not match its original paid order")]
    Binding,
    #[error("managed refund requires reconciliation before any further request")]
    ReconciliationRequired,
    #[error("managed refund webhook authentication failed")]
    Authentication,
    #[error("managed refund event is unsupported or inconsistent")]
    Event,
}
type Result<T> = std::result::Result<T, RefundError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefundMerchant {
    pub stripe_account_id: String,
    pub livemode: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    // Omitted for a full refund, so plans stored before deposit refunds keep their digest.
    #[serde(default, skip_serializing_if = "RefundKind::is_full")]
    kind: RefundKind,
    order_id: Uuid,
    operation_id: Uuid,
    quote_digest: String,
    merchant: RefundMerchant,
    payment_intent_id: String,
    amount_minor: u64,
    currency: FiatCurrency,
    created_at: i64,
    retry_before: i64,
    idempotency_key: String,
    body: Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ManagedRefundPlan(Document);

fn id(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 255
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn currency(c: FiatCurrency) -> &'static str {
    match c {
        FiatCurrency::Mxn => "mxn",
        FiatCurrency::Usd => "usd",
    }
}

impl ManagedRefundPlan {
    /// The store supplies the original PaymentIntent from its signed paid receipt. No caller
    /// amount or alternate destination is accepted; quote expiry does not forfeit remedies.
    pub fn new(
        quote: &ManagedQuote,
        order_id: Uuid,
        operation_id: Uuid,
        payment_intent_id: String,
        now: i64,
    ) -> Result<Self> {
        Self::of_kind(
            RefundKind::Full,
            quote,
            order_id,
            operation_id,
            payment_intent_id,
            now,
        )
    }

    /// The refund of exactly the quote's review deposit line to the original payment.
    pub fn review_deposit(
        quote: &ManagedQuote,
        order_id: Uuid,
        operation_id: Uuid,
        payment_intent_id: String,
        now: i64,
    ) -> Result<Self> {
        Self::of_kind(
            RefundKind::ReviewDeposit,
            quote,
            order_id,
            operation_id,
            payment_intent_id,
            now,
        )
    }

    /// Constructs an immutable amount plan, not recovery or execution authority. The store
    /// must separately prove complete canonical recovery and absence of obligations.
    pub fn recovered_bounty(
        quote: &ManagedQuote,
        order_id: Uuid,
        operation_id: Uuid,
        payment_intent_id: String,
        now: i64,
    ) -> Result<Self> {
        Self::of_kind(
            RefundKind::RecoveredBounty,
            quote,
            order_id,
            operation_id,
            payment_intent_id,
            now,
        )
    }

    fn of_kind(
        kind: RefundKind,
        quote: &ManagedQuote,
        order_id: Uuid,
        operation_id: Uuid,
        payment_intent_id: String,
        now: i64,
    ) -> Result<Self> {
        let quote_digest = quote.digest().map_err(|_| RefundError::Binding)?;
        let amount_minor = match kind {
            RefundKind::Full => quote.total_minor(),
            RefundKind::RecoveredBounty => {
                // Preserve each original quote line's rounding and currency. In particular,
                // the escrow target includes a platform fee that this card refund excludes.
                let mut amount = 0u64;
                for code in ["solver_reward", "verification"] {
                    let line = quote
                        .lines()
                        .iter()
                        .find(|line| line.code == code)
                        .ok_or(RefundError::Binding)?;
                    amount = amount
                        .checked_add(line.amount_minor)
                        .ok_or(RefundError::Binding)?;
                }
                if amount == 0 || amount >= quote.total_minor() {
                    return Err(RefundError::Binding);
                }
                amount
            }
            RefundKind::ReviewDeposit => quote
                .input()
                .additional_costs
                .iter()
                .find(|cost| cost.code == crate::pricing::REVIEW_DEPOSIT_CODE)
                .map(|cost| cost.amount_minor)
                .filter(|amount| *amount > 0 && *amount < quote.total_minor())
                .ok_or(RefundError::Binding)?,
        };
        if order_id.is_nil()
            || operation_id.is_nil()
            || !id(&payment_intent_id, "pi_")
            || !id(&quote.input().stripe_account_id, "acct_")
            || quote.total_minor() > 99_999_999
            || now < quote.input().issued_at
            || now > 253_402_214_400
        {
            return Err(RefundError::Binding);
        }
        let merchant = RefundMerchant {
            stripe_account_id: quote.input().stripe_account_id.clone(),
            livemode: quote.input().livemode,
        };
        let body = json!({"payment_intent":payment_intent_id,"amount":amount_minor,"reason":"requested_by_customer",
            "metadata":{"purpose":kind.purpose(),"managed_order_id":order_id,"managed_operation_id":operation_id,
            "managed_quote_digest":quote_digest,"merchant_account_id":merchant.stripe_account_id}});
        let key = match kind {
            RefundKind::Full => "refund",
            RefundKind::ReviewDeposit => "deposit-refund",
            RefundKind::RecoveredBounty => "recovered-bounty-refund",
        };
        Ok(Self(Document {
            kind,
            order_id,
            operation_id,
            quote_digest,
            merchant,
            payment_intent_id,
            amount_minor,
            currency: quote.input().currency,
            created_at: now,
            retry_before: now + 23 * 3600,
            idempotency_key: format!("managed-order:{order_id}:{key}:{operation_id}"),
            body,
        }))
    }
    pub fn restore(
        value: Value,
        quote: &ManagedQuote,
        order_id: Uuid,
        payment_intent_id: &str,
    ) -> Result<Self> {
        let d: Document = serde_json::from_value(value).map_err(|_| RefundError::Binding)?;
        let expected = Self::of_kind(
            d.kind,
            quote,
            order_id,
            d.operation_id,
            payment_intent_id.into(),
            d.created_at,
        )?;
        if expected.0 != d {
            return Err(RefundError::Binding);
        }
        Ok(expected)
    }
    pub fn request(&self, now: i64, merchant: &RefundMerchant) -> Result<StripeRequestIntent> {
        if merchant != &self.0.merchant {
            return Err(RefundError::Binding);
        }
        if now < self.0.created_at || now >= self.0.retry_before {
            return Err(RefundError::ReconciliationRequired);
        }
        Ok(StripeRequestIntent {
            method: "POST".into(),
            endpoint: "/v1/refunds".into(),
            api_version: MANAGED_STRIPE_API_VERSION.into(),
            idempotency_key: self.0.idempotency_key.clone(),
            body: self.0.body.clone(),
        })
    }
    pub fn kind(&self) -> RefundKind {
        self.0.kind
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
    pub fn merchant(&self) -> &RefundMerchant {
        &self.0.merchant
    }
    pub fn payment_intent_id(&self) -> &str {
        &self.0.payment_intent_id
    }
    pub fn amount_minor(&self) -> u64 {
        self.0.amount_minor
    }
    pub fn currency(&self) -> FiatCurrency {
        self.0.currency
    }
    pub fn created_at(&self) -> i64 {
        self.0.created_at
    }
    pub fn digest(&self) -> String {
        hex::encode(Sha256::digest(
            serde_json::to_vec(self).expect("typed refund plan serializes"),
        ))
    }
    pub fn matches_metadata(&self, v: &Value) -> bool {
        v["metadata"] == self.0.body["metadata"]
    }
    /// Call only with the exact merchant/mode-authenticated provider API response. Status
    /// does not enter this binding; only native signed events can advance refund status.
    pub fn binding(&self, v: &Value) -> Result<ManagedRefundBinding> {
        let refund_id = v["id"]
            .as_str()
            .filter(|s| id(s, "re_"))
            .ok_or(RefundError::Binding)?;
        let charge_id = v["charge"]
            .as_str()
            .filter(|s| id(s, "ch_"))
            .ok_or(RefundError::Binding)?;
        let created = v["created"].as_i64().ok_or(RefundError::Binding)?;
        if v["object"] != "refund"
            || v["payment_intent"] != self.0.payment_intent_id
            || !self.matches_metadata(v)
            || v["amount"].as_u64() != Some(self.0.amount_minor)
            || v["currency"] != currency(self.0.currency)
            || created < self.0.created_at
            || created >= self.0.retry_before
            || serde_json::from_value::<RefundOutcome>(v["status"].clone()).is_err()
            || v.get("livemode")
                .is_some_and(|value| value.as_bool() != Some(self.0.merchant.livemode))
        {
            return Err(RefundError::Binding);
        }
        Ok(ManagedRefundBinding {
            operation_id: self.0.operation_id,
            plan_digest: self.digest(),
            refund_id: refund_id.into(),
            charge_id: charge_id.into(),
        })
    }
    pub fn matches_event(&self, e: &RefundEventData) -> bool {
        e.kind == self.kind()
            && e.order_id == self.order_id()
            && e.operation_id == self.operation_id()
            && e.quote_digest == self.quote_digest()
            && e.stripe_account_id == self.merchant().stripe_account_id
            && e.livemode == self.merchant().livemode
            && e.payment_intent_id == self.payment_intent_id()
            && e.amount_minor == self.amount_minor()
            && e.currency == self.currency()
            && e.event_created_at >= e.refund_created_at
            && e.refund_created_at >= self.created_at()
            && e.refund_created_at < self.0.retry_before
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedRefundBinding {
    operation_id: Uuid,
    plan_digest: String,
    refund_id: String,
    charge_id: String,
}
impl ManagedRefundBinding {
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn refund_id(&self) -> &str {
        &self.refund_id
    }
    pub fn charge_id(&self) -> &str {
        &self.charge_id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefundOutcome {
    Pending,
    RequiresAction,
    Succeeded,
    Failed,
    Canceled,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefundEventData {
    #[serde(default, skip_serializing_if = "RefundKind::is_full")]
    pub kind: RefundKind,
    pub event_id: String,
    pub body_sha256: String,
    pub stripe_account_id: String,
    pub livemode: bool,
    pub order_id: Uuid,
    pub operation_id: Uuid,
    pub quote_digest: String,
    pub refund_id: String,
    pub charge_id: String,
    pub payment_intent_id: String,
    pub amount_minor: u64,
    pub currency: FiatCurrency,
    pub outcome: RefundOutcome,
    pub event_created_at: i64,
    pub refund_created_at: i64,
}
#[derive(Debug, Clone)]
pub struct AuthenticatedRefundEvent(RefundEventData);
impl AuthenticatedRefundEvent {
    pub fn data(&self) -> &RefundEventData {
        &self.0
    }
}

pub fn verify_managed_refund_event(
    body: &[u8],
    signature: &str,
    secret: &[u8],
    merchant: &RefundMerchant,
) -> Result<AuthenticatedRefundEvent> {
    let event = verify_platform_webhook_event(body, signature, secret, merchant.livemode)
        .map_err(|_| RefundError::Authentication)?;
    let wire: Value = serde_json::from_slice(body).map_err(|_| RefundError::Event)?;
    let v = &event.payload;
    let m = &v["metadata"];
    let get = |key: &str| v[key].as_str().ok_or(RefundError::Event);
    let meta = |key: &str| m[key].as_str().ok_or(RefundError::Event);
    let outcome: RefundOutcome =
        serde_json::from_value(v["status"].clone()).map_err(|_| RefundError::Event)?;
    let event_created_at = wire["created"]
        .as_i64()
        .filter(|n| *n > 0)
        .ok_or(RefundError::Event)?;
    let refund_created_at = v["created"]
        .as_i64()
        .filter(|n| *n > 0 && *n <= event_created_at)
        .ok_or(RefundError::Event)?;
    let order_id = Uuid::parse_str(meta("managed_order_id")?).map_err(|_| RefundError::Event)?;
    let operation_id =
        Uuid::parse_str(meta("managed_operation_id")?).map_err(|_| RefundError::Event)?;
    let digest = meta("managed_quote_digest")?;
    let kind = RefundKind::from_purpose(meta("purpose")?).ok_or(RefundError::Event)?;
    if !matches!(
        event.event_type.as_str(),
        "refund.created" | "refund.updated" | "refund.failed"
    ) || (event.event_type == "refund.failed" && outcome != RefundOutcome::Failed)
        || !id(&event.id, "evt_")
        || !id(&merchant.stripe_account_id, "acct_")
        || get("object")? != "refund"
        || meta("merchant_account_id")? != merchant.stripe_account_id
        || order_id.is_nil()
        || operation_id.is_nil()
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !id(get("id")?, "re_")
        || !id(get("charge")?, "ch_")
        || !id(get("payment_intent")?, "pi_")
        || v.get("livemode")
            .is_some_and(|value| value.as_bool() != Some(merchant.livemode))
    {
        return Err(RefundError::Event);
    }
    let amount_minor = v["amount"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= 99_999_999)
        .ok_or(RefundError::Event)?;
    let currency = match get("currency")? {
        "mxn" => FiatCurrency::Mxn,
        "usd" => FiatCurrency::Usd,
        _ => return Err(RefundError::Event),
    };
    Ok(AuthenticatedRefundEvent(RefundEventData {
        kind,
        event_id: event.id,
        body_sha256: hex::encode(Sha256::digest(body)),
        stripe_account_id: merchant.stripe_account_id.clone(),
        livemode: merchant.livemode,
        order_id,
        operation_id,
        quote_digest: digest.into(),
        refund_id: get("id")?.into(),
        charge_id: get("charge")?.into(),
        payment_intent_id: get("payment_intent")?.into(),
        amount_minor,
        currency,
        outcome,
        event_created_at,
        refund_created_at,
    }))
}

#[cfg(test)]
mod tests;
