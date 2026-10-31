//! Immutable company transfer plans and authenticated-readback validation.
//! These types do not authorize bank deposits, signing, bounty funding or payouts.
use crate::MAX_EXACT_INTEGER;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SCHEMA: &str = "agent-bounties/managed-bridge-transfer-v1";
pub const CREATE_WINDOW_SECONDS: i64 = 23 * 60 * 60;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BridgeError {
    #[error("invalid company transfer plan")]
    Plan,
    #[error("company transfer create window expired; reconcile the original operation")]
    Expired,
    #[error("provider transfer does not match the committed operation")]
    Binding,
    #[error("provider amount is not an exact supported decimal")]
    Amount,
    #[error("provider transfer observation is invalid or unsupported")]
    Observation,
    #[error("provider transfer observation conflicts with recorded history")]
    History,
}
type Result<T> = std::result::Result<T, BridgeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeEnvironment {
    Sandbox,
    Live,
}

// Company identity/configuration only. No secrets and no user-controlled provider URLs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BridgeCompany {
    pub environment: BridgeEnvironment,
    pub customer_id: Uuid,
    pub base_treasury_address: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BridgeRoute {
    MxnConversion {
        company_source_clabe: String,
        input_minor: u64,
        minimum_output_micro_usdc: u64,
    },
    SupplierMxnPayout {
        obligation_id: Uuid,
        external_account_id: Uuid,
        external_account_snapshot_digest: String,
        input_micro_usdc: u64,
        output_minor: u64,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BridgeTransferPlan {
    schema: String,
    operation_id: Uuid,
    order_id: Uuid,
    quote_digest: String,
    company: BridgeCompany,
    route: BridgeRoute,
    created_at: i64,
}

fn positive(n: u64) -> bool {
    n > 0 && n <= MAX_EXACT_INTEGER
}
fn hex_value(s: &str, bytes: usize) -> bool {
    s.len() == bytes * 2 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn address(s: &str) -> bool {
    s.strip_prefix("0x")
        .is_some_and(|v| hex_value(v, 20) && v.bytes().any(|b| b != b'0'))
}
fn same_address(value: &Value, expected: &str) -> bool {
    value
        .as_str()
        .is_some_and(|s| address(s) && s.eq_ignore_ascii_case(expected))
}
pub fn valid_clabe(s: &str) -> bool {
    if s.len() != 18 || !s.bytes().all(|b| b.is_ascii_digit()) || s.bytes().all(|b| b == b'0') {
        return false;
    }
    let sum: u32 = s
        .bytes()
        .take(17)
        .enumerate()
        .map(|(i, b)| u32::from(b - b'0') * [3, 7, 1][i % 3] % 10)
        .sum();
    (10 - sum % 10) % 10 == u32::from(s.as_bytes()[17] - b'0')
}

/// Reject signs, exponent syntax, whitespace, excess nonzero precision and integer overflow.
/// Extra trailing zeroes are safe and commonly returned by providers. Never round money.
pub fn parse_decimal(value: &str, scale: u32) -> Result<u64> {
    if value.is_empty() || value.len() > 64 || scale > 6 {
        return Err(BridgeError::Amount);
    }
    let (whole, frac) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || (value.contains('.') && frac.is_empty())
        || frac
            .as_bytes()
            .iter()
            .skip(scale as usize)
            .any(|b| *b != b'0')
    {
        return Err(BridgeError::Amount);
    }
    let factor = 10_u64.pow(scale);
    let whole: u64 = whole.parse().map_err(|_| BridgeError::Amount)?;
    let mut fraction = 0_u64;
    for i in 0..scale as usize {
        fraction =
            fraction * 10 + u64::from(frac.as_bytes().get(i).copied().unwrap_or(b'0') - b'0');
    }
    whole
        .checked_mul(factor)
        .and_then(|n| n.checked_add(fraction))
        .filter(|n| *n <= MAX_EXACT_INTEGER)
        .ok_or(BridgeError::Amount)
}
fn decimal(n: u64, scale: u32) -> String {
    let factor = 10_u64.pow(scale);
    format!(
        "{}.{:0width$}",
        n / factor,
        n % factor,
        width = scale as usize
    )
}
fn amount(value: &Value, scale: u32) -> Result<u64> {
    parse_decimal(value.as_str().ok_or(BridgeError::Amount)?, scale)
}
fn timestamp(value: &Value) -> Result<i64> {
    let s = value
        .as_str()
        .filter(|s| s.len() <= 40)
        .ok_or(BridgeError::Observation)?;
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp_millis())
        .map_err(|_| BridgeError::Observation)
}

/// Bind the complete authenticated, private beneficiary snapshot used when approving a
/// supplier offer. Any provider change requires revalidation before creating a payout.
/// Hashing an arbitrary value is not authentication or supplier ownership verification.
pub fn external_account_snapshot_digest(value: &Value) -> Result<String> {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut pairs: Vec<_> = map.iter().collect();
                pairs.sort_by(|a, b| a.0.cmp(b.0));
                Value::Object(
                    pairs
                        .into_iter()
                        .map(|(k, v)| (k.clone(), canonical(v)))
                        .collect(),
                )
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    if !value.is_object() {
        return Err(BridgeError::Binding);
    }
    let bytes = serde_json::to_vec(&canonical(value)).map_err(|_| BridgeError::Binding)?;
    if bytes.len() > 256 * 1024 {
        return Err(BridgeError::Binding);
    }
    let mut hash = Sha256::new();
    hash.update(b"agent-bounties:bridge-beneficiary-snapshot:v1\0");
    hash.update(bytes);
    Ok(hex::encode(hash.finalize()))
}

impl BridgeTransferPlan {
    /// Caller must subsequently persist this exact plan and validate/reserve the authoritative
    /// funding/earned-obligation evidence. Constructing a plan grants no spending authority.
    pub fn new(
        operation_id: Uuid,
        order_id: Uuid,
        quote_digest: String,
        company: BridgeCompany,
        route: BridgeRoute,
        created_at: i64,
    ) -> Result<Self> {
        let plan = Self {
            schema: SCHEMA.into(),
            operation_id,
            order_id,
            quote_digest,
            company,
            route,
            created_at,
        };
        plan.validate()?;
        Ok(plan)
    }
    pub fn restore(value: Value) -> Result<Self> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Stored {
            schema: String,
            operation_id: Uuid,
            order_id: Uuid,
            quote_digest: String,
            company: BridgeCompany,
            route: BridgeRoute,
            created_at: i64,
        }
        let s: Stored = serde_json::from_value(value).map_err(|_| BridgeError::Plan)?;
        let p = Self {
            schema: s.schema,
            operation_id: s.operation_id,
            order_id: s.order_id,
            quote_digest: s.quote_digest,
            company: s.company,
            route: s.route,
            created_at: s.created_at,
        };
        p.validate()?;
        Ok(p)
    }
    fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA
            || self.operation_id.is_nil()
            || self.order_id.is_nil()
            || !hex_value(&self.quote_digest, 32)
            || self.quote_digest != self.quote_digest.to_ascii_lowercase()
            || self.company.customer_id.is_nil()
            || !address(&self.company.base_treasury_address)
            || self.company.base_treasury_address
                != self.company.base_treasury_address.to_ascii_lowercase()
            || self.created_at <= 0
            || self.created_at > 253_402_214_400
        {
            return Err(BridgeError::Plan);
        }
        let valid = match &self.route {
            BridgeRoute::MxnConversion {
                company_source_clabe,
                input_minor,
                minimum_output_micro_usdc,
            } => {
                valid_clabe(company_source_clabe)
                    && positive(*input_minor)
                    && positive(*minimum_output_micro_usdc)
            }
            BridgeRoute::SupplierMxnPayout {
                obligation_id,
                external_account_id,
                external_account_snapshot_digest,
                input_micro_usdc,
                output_minor,
            } => {
                !obligation_id.is_nil()
                    && !external_account_id.is_nil()
                    && hex_value(external_account_snapshot_digest, 32)
                    && *external_account_snapshot_digest
                        == external_account_snapshot_digest.to_ascii_lowercase()
                    && positive(*input_micro_usdc)
                    && positive(*output_minor)
            }
        };
        if !valid {
            return Err(BridgeError::Plan);
        }
        Ok(())
    }
    pub fn operation_id(&self) -> Uuid {
        self.operation_id
    }
    pub fn order_id(&self) -> Uuid {
        self.order_id
    }
    pub fn company(&self) -> &BridgeCompany {
        &self.company
    }
    pub fn route(&self) -> &BridgeRoute {
        &self.route
    }
    pub fn created_at(&self) -> i64 {
        self.created_at
    }
    pub fn digest(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(b"agent-bounties:managed-bridge-transfer:v1\0");
        // This struct has only infallibly serializable fields.
        hash.update(serde_json::to_vec(self).expect("serializable transfer plan"));
        hex::encode(hash.finalize())
    }
    pub fn request(&self, now: i64) -> Result<Value> {
        self.validate()?;
        // Conservatively anchored to plan creation, which precedes the first durable attempt.
        if now < self.created_at || now >= self.created_at + CREATE_WINDOW_SECONDS {
            return Err(BridgeError::Expired);
        }
        let mut body = json!({"client_reference_id":self.operation_id,
            "on_behalf_of":self.company.customer_id});
        match &self.route {
            BridgeRoute::MxnConversion { input_minor, .. } => {
                body["amount"] = json!(decimal(*input_minor, 2));
                body["source"] = json!({"currency":"mxn","payment_rail":"spei"});
                body["destination"] = json!({"currency":"usdc","payment_rail":"base",
                    "to_address":self.company.base_treasury_address});
            }
            BridgeRoute::SupplierMxnPayout {
                external_account_id,
                input_micro_usdc,
                output_minor,
                ..
            } => {
                body["source"] = json!({"currency":"usdc","payment_rail":"base",
                    "from_address":self.company.base_treasury_address,"amount":decimal(*input_micro_usdc,6)});
                body["destination"] = json!({"currency":"mxn","payment_rail":"spei",
                    "external_account_id":external_account_id,"amount":decimal(*output_minor,2)});
                body["return_instructions"] = json!({"address":self.company.base_treasury_address});
            }
        }
        Ok(body)
    }

    /// Only call with an authenticated provider GET/POST response. This is not webhook
    /// authentication. No buyer-controlled payload may enter the accounting path here.
    pub fn observe(&self, value: &Value, known_id: Option<Uuid>) -> Result<BridgeObservation> {
        let id = value["id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .filter(|id| !id.is_nil())
            .ok_or(BridgeError::Binding)?;
        if known_id.is_some_and(|expected| id != expected)
            || value["client_reference_id"] != self.operation_id.to_string()
            || value["on_behalf_of"] != self.company.customer_id.to_string()
        {
            return Err(BridgeError::Binding);
        }
        let state: BridgeTransferState =
            serde_json::from_value(value["state"].clone()).map_err(|_| BridgeError::Observation)?;
        let created = timestamp(&value["created_at"])?;
        let updated = timestamp(&value["updated_at"])?;
        if created < self.created_at * 1000
            || created >= (self.created_at + CREATE_WINDOW_SECONDS) * 1000
            || updated < created
        {
            return Err(BridgeError::Observation);
        }
        let (
            source_currency,
            source_scale,
            source_amount,
            output_currency,
            output_scale,
            required_output,
        ) = match &self.route {
            BridgeRoute::MxnConversion {
                company_source_clabe,
                input_minor,
                minimum_output_micro_usdc,
            } => {
                if value["source"]["currency"] != "mxn"
                    || value["source"]["payment_rail"] != "spei"
                    || value["destination"]["currency"] != "usdc"
                    || value["destination"]["payment_rail"] != "base"
                    || !same_address(
                        &value["destination"]["to_address"],
                        &self.company.base_treasury_address,
                    )
                    || (!value["source"]["clabe"].is_null()
                        && value["source"]["clabe"] != *company_source_clabe)
                    || (state.received_funds() && value["source"]["clabe"] != *company_source_clabe)
                {
                    return Err(BridgeError::Binding);
                }
                (
                    "mxn",
                    2,
                    *input_minor,
                    "usdc",
                    6,
                    *minimum_output_micro_usdc,
                )
            }
            BridgeRoute::SupplierMxnPayout {
                external_account_id,
                input_micro_usdc,
                output_minor,
                ..
            } => {
                if value["source"]["currency"] != "usdc"
                    || value["source"]["payment_rail"] != "base"
                    || !same_address(
                        &value["source"]["from_address"],
                        &self.company.base_treasury_address,
                    )
                    || value["destination"]["currency"] != "mxn"
                    || value["destination"]["payment_rail"] != "spei"
                    || value["destination"]["external_account_id"]
                        != external_account_id.to_string()
                    || amount(&value["destination"]["amount"], 2)? != *output_minor
                {
                    return Err(BridgeError::Binding);
                }
                ("usdc", 6, *input_micro_usdc, "mxn", 2, *output_minor)
            }
        };
        // Bridge expresses crypto source transfer currency in its pegged fiat (USDC -> USD).
        if value["currency"]
            != if source_currency == "usdc" {
                "usd"
            } else {
                source_currency
            }
            || amount(&value["amount"], source_scale)? != source_amount
            || amount(&value["developer_fee"], source_scale)? != 0
        {
            return Err(BridgeError::Binding);
        }
        let mut output = None;
        let mut final_tx_hash = None;
        let mut output_matches_commitment = false;
        if state == BridgeTransferState::PaymentProcessed {
            let r = &value["receipt"];
            let initial = amount(&r["initial_amount"], source_scale)?;
            if r["initial_amount_currency"] != source_currency
                || r["final_amount_currency"] != output_currency
                || initial == 0
                || initial > source_amount
                || (matches!(self.route, BridgeRoute::MxnConversion { .. })
                    && initial != source_amount)
            {
                return Err(BridgeError::Binding);
            }
            let n = amount(&r["final_amount"], output_scale)?;
            output_matches_commitment = n >= required_output
                && (!matches!(self.route, BridgeRoute::SupplierMxnPayout { .. })
                    || n == required_output);
            output = Some(n);
            if matches!(self.route, BridgeRoute::MxnConversion { .. }) {
                let hash = r["destination_tx_hash"]
                    .as_str()
                    .filter(|s| {
                        s.strip_prefix("0x")
                            .is_some_and(|s| hex_value(s, 32) && s.bytes().any(|b| b != b'0'))
                    })
                    .ok_or(BridgeError::Observation)?;
                final_tx_hash = Some(hash.to_ascii_lowercase());
            }
        }
        let deposit = self.deposit(value)?;
        Ok(BridgeObservation {
            plan_digest: self.digest(),
            transfer_id: id,
            state,
            provider_created_at_ms: created,
            provider_updated_at_ms: updated,
            output,
            output_matches_commitment,
            final_tx_hash,
            deposit,
        })
    }

    fn deposit(&self, value: &Value) -> Result<Option<BridgeDeposit>> {
        let d = &value["source_deposit_instructions"];
        if d.is_null() {
            return Ok(None);
        }
        match &self.route {
            BridgeRoute::MxnConversion { input_minor, .. } => {
                let clabe = d["clabe"]
                    .as_str()
                    .filter(|s| valid_clabe(s))
                    .ok_or(BridgeError::Observation)?;
                let message = d["deposit_message"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
                    .ok_or(BridgeError::Observation)?;
                if d["payment_rail"] != "spei"
                    || d["currency"] != "mxn"
                    || amount(&d["amount"], 2)? != *input_minor
                {
                    return Err(BridgeError::Binding);
                }
                Ok(Some(BridgeDeposit::Spei {
                    clabe: clabe.into(),
                    message: message.into(),
                    amount_minor: *input_minor,
                }))
            }
            BridgeRoute::SupplierMxnPayout {
                input_micro_usdc, ..
            } => {
                let to = d["to_address"]
                    .as_str()
                    .filter(|s| address(s))
                    .ok_or(BridgeError::Observation)?;
                if d["payment_rail"] != "base"
                    || d["currency"] != "usdc"
                    || !same_address(&d["from_address"], &self.company.base_treasury_address)
                    || amount(&d["amount"], 6)? != *input_micro_usdc
                {
                    return Err(BridgeError::Binding);
                }
                Ok(Some(BridgeDeposit::BaseUsdc {
                    to_address: to.to_ascii_lowercase(),
                    amount_micro_usdc: *input_micro_usdc,
                }))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeTransferState {
    AwaitingFunds,
    InReview,
    FundsReceived,
    PaymentSubmitted,
    PaymentProcessed,
    Undeliverable,
    Returned,
    MissingReturnPolicy,
    Refunded,
    RefundInFlight,
    RefundFailed,
    Canceled,
    Error,
    // Fixed-output documentation additionally uses these states.
    Pending,
    Underfunded,
}
impl BridgeTransferState {
    fn received_funds(self) -> bool {
        matches!(
            self,
            Self::FundsReceived | Self::PaymentSubmitted | Self::PaymentProcessed
        )
    }
    pub fn exception(self) -> bool {
        matches!(
            self,
            Self::Undeliverable
                | Self::Returned
                | Self::MissingReturnPolicy
                | Self::Refunded
                | Self::RefundInFlight
                | Self::RefundFailed
                | Self::Canceled
                | Self::Error
                | Self::Underfunded
        )
    }
    fn rank(self) -> Option<u8> {
        match self {
            Self::Pending | Self::AwaitingFunds | Self::InReview => Some(0),
            Self::FundsReceived => Some(1),
            Self::PaymentSubmitted => Some(2),
            Self::PaymentProcessed => Some(3),
            _ => None,
        }
    }
}

// Sensitive instructions: deliberately no Debug. Persist privately; never include in buyer status.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "rail", rename_all = "snake_case")]
pub enum BridgeDeposit {
    Spei {
        clabe: String,
        message: String,
        amount_minor: u64,
    },
    BaseUsdc {
        to_address: String,
        amount_micro_usdc: u64,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct BridgeObservation {
    plan_digest: String,
    transfer_id: Uuid,
    state: BridgeTransferState,
    provider_created_at_ms: i64,
    provider_updated_at_ms: i64,
    output: Option<u64>,
    output_matches_commitment: bool,
    final_tx_hash: Option<String>,
    deposit: Option<BridgeDeposit>,
}
impl BridgeObservation {
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn transfer_id(&self) -> Uuid {
        self.transfer_id
    }
    pub fn state(&self) -> BridgeTransferState {
        self.state
    }
    pub fn output(&self) -> Option<u64> {
        self.output
    }
    /// Provider-delivered output only. Conversion still requires canonical treasury evidence;
    /// payout still needs durable obligation reconciliation and monitoring for late returns.
    pub fn output_matches_commitment(&self) -> bool {
        self.output_matches_commitment
    }
    pub fn final_tx_hash(&self) -> Option<&str> {
        self.final_tx_hash.as_deref()
    }
    pub fn deposit(&self) -> Option<&BridgeDeposit> {
        self.deposit.as_ref()
    }
    /// History is for this exact plan/transfer only. Exceptions after delivery are observable;
    /// stale/ambiguous readbacks cannot erase them or imply a replacement transfer is safe.
    pub fn validate_successor(&self, next: &Self) -> Result<()> {
        if self.plan_digest != next.plan_digest
            || self.transfer_id != next.transfer_id
            || self.provider_created_at_ms != next.provider_created_at_ms
            || next.provider_updated_at_ms < self.provider_updated_at_ms
            || (next.provider_updated_at_ms == self.provider_updated_at_ms && self != next)
            || self
                .deposit
                .as_ref()
                .zip(next.deposit.as_ref())
                .is_some_and(|(a, b)| a != b)
            || self
                .state
                .rank()
                .zip(next.state.rank())
                .is_some_and(|(a, b)| b < a)
            || (self.state.exception() && !next.state.exception())
            || (self.state == BridgeTransferState::PaymentProcessed
                && next.state == BridgeTransferState::PaymentProcessed
                && (self.output != next.output || self.final_tx_hash != next.final_tx_hash))
        {
            return Err(BridgeError::History);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
