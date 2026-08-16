use alloy::{
    primitives::{keccak256, Address, Signature, B256, U256},
    sol_types::SolValue,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use thiserror::Error;

pub const X402_VERSION: u8 = 2;
pub const PAYMENT_REQUIRED_HEADER: &str = "payment-required";
pub const PAYMENT_SIGNATURE_HEADER: &str = "payment-signature";
pub const PAYMENT_RESPONSE_HEADER: &str = "payment-response";
pub const AGENT_BOUNTY_FUND_SCHEME: &str = "agent-bounty-fund";
pub const STANDARD_EXACT_SCHEME: &str = "exact";
pub const MAX_HEADER_LENGTH: usize = 32 * 1024;
const MIN_SETTLEMENT_BUFFER_SECONDS: u64 = 6;
const MAX_CLOCK_SKEW_SECONDS: u64 = 30;
const EIP712_DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const TRANSFER_WITH_AUTHORIZATION_TYPE: &str = "TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum X402Error {
    #[error("x402 header is empty")]
    EmptyHeader,
    #[error("x402 header exceeds the {MAX_HEADER_LENGTH}-byte limit")]
    HeaderTooLarge,
    #[error("x402 header is not valid base64")]
    InvalidBase64,
    #[error("x402 header is not valid UTF-8 JSON")]
    InvalidJson,
    #[error("unsupported x402 version {0}")]
    UnsupportedVersion(u8),
    #[error("payment requirements must contain exactly one funding option")]
    AmbiguousRequirements,
    #[error("standard exact transfers cannot fund an Agent Bounties contract; use agent-bounty-fund so FundingAdded is emitted")]
    UnsafeExactFunding,
    #[error("payment requirements do not exactly match the issued challenge")]
    RequirementsMismatch,
    #[error("payment resource does not match the issued challenge")]
    ResourceMismatch,
    #[error("payment extensions do not exactly echo the issued challenge")]
    ExtensionsMismatch,
    #[error("invalid EIP-3009 authorization payload")]
    InvalidAuthorization,
    #[error("invalid EVM address")]
    InvalidAddress,
    #[error("invalid USDC base-unit amount")]
    InvalidAmount,
    #[error("authorization recipient does not match the canonical bounty contract")]
    RecipientMismatch,
    #[error("authorization amount does not match the x402 requirement")]
    AmountMismatch,
    #[error("authorization validAfter must be zero for the autonomous-v1 contribution call")]
    InvalidValidAfter,
    #[error("authorization expires too soon to settle safely")]
    AuthorizationExpired,
    #[error("authorization lasts longer than the issued x402 timeout")]
    AuthorizationTooLong,
    #[error("invalid EIP-3009 nonce")]
    InvalidNonce,
    #[error("invalid EIP-3009 signature")]
    InvalidSignature,
    #[error("x402 funding challenge is invalid: {0}")]
    InvalidChallenge(&'static str),
    #[error("proof SLA does not fit before the competition deadline")]
    ProofSlaMiss,
    #[error("proof quote arithmetic exceeds supported bounds")]
    QuoteOverflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResourceInfo {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaymentRequirements {
    pub scheme: String,
    pub network: String,
    pub asset: String,
    pub amount: String,
    pub pay_to: String,
    pub max_timeout_seconds: u64,
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaymentRequired {
    pub x402_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub resource: ResourceInfo,
    pub accepts: Vec<PaymentRequirements>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaymentPayload {
    pub x402_version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceInfo>,
    pub accepted: PaymentRequirements,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SettlementResponse {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    pub transaction: String,
    pub network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extensions: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Eip3009Authorization {
    pub from: String,
    pub to: String,
    pub value: String,
    pub valid_after: String,
    pub valid_before: String,
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Eip3009Payload {
    pub signature: String,
    pub authorization: Eip3009Authorization,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedFundingAuthorization {
    pub contributor: String,
    pub bounty_contract: String,
    pub amount: u64,
    pub valid_before: u64,
    pub nonce: String,
    pub v: u8,
    pub r: String,
    pub s: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedExactAuthorization {
    pub payer: String,
    pub recipient: String,
    pub amount: u64,
    pub valid_before: u64,
    pub nonce: String,
    pub v: u8,
    pub r: String,
    pub s: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompetitionWinnerMode {
    FirstProven,
    BestScore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofBrokerQuoteRequest {
    pub network: String,
    pub competition_contract: String,
    pub solver: String,
    pub solver_nonce: String,
    pub artifact_hash: String,
    pub proof_system: String,
    pub gross_prize: String,
    pub proof_fee_quote: String,
    pub relay_fee_quote: String,
    pub winner_mode: CompetitionWinnerMode,
    pub proof_deadline: u64,
    pub measured_proof_sla_seconds: u64,
    pub now_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProofBrokerQuote {
    pub schema_version: String,
    pub quote_id: String,
    pub network: String,
    pub competition_contract: String,
    pub solver: String,
    pub solver_nonce: String,
    pub artifact_hash: String,
    pub proof_system: String,
    pub gross_prize: String,
    pub proof_fee_quote: String,
    pub relay_fee_quote: String,
    pub net_prize_if_win: String,
    pub maximum_charge: String,
    pub profitable_if_win: bool,
    pub winner_mode: CompetitionWinnerMode,
    pub competition_risk: String,
    pub quote_expiration: u64,
    pub proof_sla_deadline: u64,
    pub evidence_boundary: String,
}

pub fn quote_proof_broker_job(
    request: ProofBrokerQuoteRequest,
) -> Result<ProofBrokerQuote, X402Error> {
    let competition = normalize_address(&request.competition_contract)?;
    let solver = normalize_address(&request.solver)?;
    let artifact_hash = normalize_word(&request.artifact_hash)?;
    if !matches!(request.proof_system.as_str(), "groth16" | "plonk") {
        return Err(X402Error::InvalidChallenge("unsupported proof system"));
    }
    let gross = parse_u128(&request.gross_prize)?;
    let proof_fee = parse_u128(&request.proof_fee_quote)?;
    let relay_fee = parse_u128(&request.relay_fee_quote)?;
    let maximum_charge = proof_fee
        .checked_add(relay_fee)
        .ok_or(X402Error::QuoteOverflow)?;
    let gross_signed = i128::try_from(gross).map_err(|_| X402Error::QuoteOverflow)?;
    let charge_signed = i128::try_from(maximum_charge).map_err(|_| X402Error::QuoteOverflow)?;
    let net = gross_signed
        .checked_sub(charge_signed)
        .ok_or(X402Error::QuoteOverflow)?;
    let proof_sla_deadline = request
        .now_unix_seconds
        .checked_add(request.measured_proof_sla_seconds)
        .ok_or(X402Error::QuoteOverflow)?;
    if request.measured_proof_sla_seconds == 0 || proof_sla_deadline >= request.proof_deadline {
        return Err(X402Error::ProofSlaMiss);
    }
    let quote_expiration = request.now_unix_seconds.saturating_add(300).min(
        request
            .proof_deadline
            .saturating_sub(request.measured_proof_sla_seconds),
    );
    if quote_expiration <= request.now_unix_seconds {
        return Err(X402Error::ProofSlaMiss);
    }
    let quote_material = json!({
        "network": request.network,
        "competition_contract": competition,
        "solver": solver,
        "solver_nonce": request.solver_nonce,
        "artifact_hash": artifact_hash,
        "proof_system": request.proof_system,
        "gross_prize": gross.to_string(),
        "proof_fee_quote": proof_fee.to_string(),
        "relay_fee_quote": relay_fee.to_string(),
        "maximum_charge": maximum_charge.to_string(),
        "winner_mode": request.winner_mode,
        "proof_deadline": request.proof_deadline,
        "quote_expiration": quote_expiration,
    });
    let encoded = serde_json::to_vec(&quote_material).map_err(|_| X402Error::InvalidJson)?;
    let competition_risk = match request.winner_mode {
        CompetitionWinnerMode::FirstProven => {
            "Another qualifying proof can settle first before this transaction confirms."
        }
        CompetitionWinnerMode::BestScore => {
            "A later qualifying entry can win with a better score; positive net prize is not guaranteed profit."
        }
    };
    Ok(ProofBrokerQuote {
        schema_version: "agent-bounties/open-competition-v2-proof-quote-v1".to_string(),
        quote_id: format!("{:#x}", keccak256(encoded)),
        network: request.network,
        competition_contract: competition,
        solver,
        solver_nonce: request.solver_nonce,
        artifact_hash,
        proof_system: request.proof_system,
        gross_prize: gross.to_string(),
        proof_fee_quote: proof_fee.to_string(),
        relay_fee_quote: relay_fee.to_string(),
        net_prize_if_win: net.to_string(),
        maximum_charge: maximum_charge.to_string(),
        profitable_if_win: net > 0,
        winner_mode: request.winner_mode,
        competition_risk: competition_risk.to_string(),
        quote_expiration,
        proof_sla_deadline,
        evidence_boundary: "This quote is not a proof, entry, settlement, refund, or guarantee of winning. A paid broker failure is refundable; losing a valid competition entry is not a broker failure.".to_string(),
    })
}

pub fn base_usdc_exact_service_challenge(
    resource_url: impl Into<String>,
    network: impl Into<String>,
    asset: &str,
    broker_payment_address: &str,
    quote: &ProofBrokerQuote,
) -> Result<PaymentRequired, X402Error> {
    let network = network.into();
    let asset = normalize_address(asset)?;
    let pay_to = normalize_address(broker_payment_address)?;
    if quote.maximum_charge == "0" {
        return Err(X402Error::InvalidChallenge(
            "proof service charge must be positive",
        ));
    }
    let eip712_name = match network.as_str() {
        "eip155:8453" => "USD Coin",
        "eip155:84532" => "USDC",
        _ => {
            return Err(X402Error::InvalidChallenge(
                "network must be Base mainnet or Base Sepolia",
            ));
        }
    };
    let mut extra = BTreeMap::new();
    extra.insert("assetTransferMethod".to_string(), json!("eip3009"));
    extra.insert("name".to_string(), json!(eip712_name));
    extra.insert("version".to_string(), json!("2"));
    extra.insert("quoteId".to_string(), json!(quote.quote_id));
    extra.insert("competition".to_string(), json!(quote.competition_contract));
    extra.insert("solver".to_string(), json!(quote.solver));
    extra.insert("artifactHash".to_string(), json!(quote.artifact_hash));
    extra.insert("proofSystem".to_string(), json!(quote.proof_system));
    extra.insert("maximumCharge".to_string(), json!(quote.maximum_charge));
    extra.insert("refundSlaSeconds".to_string(), json!(1800));
    Ok(PaymentRequired {
        x402_version: X402_VERSION,
        error: Some("Pay the exact bounded proof-service quote.".to_string()),
        resource: ResourceInfo {
            url: resource_url.into(),
            description: Some(
                "Generate and optionally relay one SP1 competition proof.".to_string(),
            ),
            mime_type: Some("application/json".to_string()),
            service_name: Some("Agent Bounties V2 Proof Broker".to_string()),
            tags: Some(vec![
                "sp1".to_string(),
                "proof".to_string(),
                "x402".to_string(),
            ]),
            icon_url: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: STANDARD_EXACT_SCHEME.to_string(),
            network,
            asset,
            amount: quote.maximum_charge.clone(),
            pay_to,
            max_timeout_seconds: 300,
            extra,
        }],
        extensions: None,
    })
}

pub fn base_usdc_funding_challenge(
    resource_url: impl Into<String>,
    network: impl Into<String>,
    asset: &str,
    bounty_contract: &str,
    amount: u64,
    max_timeout_seconds: u64,
) -> Result<PaymentRequired, X402Error> {
    if amount == 0 {
        return Err(X402Error::InvalidChallenge("amount must be positive"));
    }
    if max_timeout_seconds < MIN_SETTLEMENT_BUFFER_SECONDS {
        return Err(X402Error::InvalidChallenge(
            "maxTimeoutSeconds must allow the settlement buffer",
        ));
    }
    let asset = normalize_address(asset)?;
    let bounty_contract = normalize_address(bounty_contract)?;
    let network = network.into();
    let eip712_name = match network.as_str() {
        "eip155:8453" => "USD Coin",
        "eip155:84532" => "USDC",
        _ => {
            return Err(X402Error::InvalidChallenge(
                "network must be Base mainnet or Base Sepolia",
            ));
        }
    };
    let mut extra = BTreeMap::new();
    extra.insert("assetTransferMethod".to_string(), json!("eip3009"));
    extra.insert("name".to_string(), json!(eip712_name));
    extra.insert("version".to_string(), json!("2"));
    extra.insert("fundingMethod".to_string(), json!("fundWithAuthorization"));
    extra.insert("fundingEvent".to_string(), json!("FundingAdded"));
    extra.insert(
        "protocol".to_string(),
        json!("agent-bounties/autonomous-v1"),
    );

    Ok(PaymentRequired {
        x402_version: X402_VERSION,
        error: Some(
            "Authorize Base USDC funding; only confirmed canonical FundingAdded is funding evidence"
                .to_string(),
        ),
        resource: ResourceInfo {
            url: resource_url.into(),
            description: Some(
                "Fund a canonical Agent Bounties contract with a bounded USDC authorization"
                    .to_string(),
            ),
            mime_type: Some("application/json".to_string()),
            service_name: Some("Agent Bounties".to_string()),
            tags: Some(vec![
                "ai-agents".to_string(),
                "bounties".to_string(),
                "funding".to_string(),
                "base".to_string(),
                "usdc".to_string(),
            ]),
            icon_url: None,
        },
        accepts: vec![PaymentRequirements {
            scheme: AGENT_BOUNTY_FUND_SCHEME.to_string(),
            network,
            asset,
            amount: amount.to_string(),
            pay_to: bounty_contract,
            max_timeout_seconds,
            extra,
        }],
        extensions: None,
    })
}

pub fn encode_payment_required_header(required: &PaymentRequired) -> Result<String, X402Error> {
    encode_header(required)
}

pub fn encode_payment_signature_header(payload: &PaymentPayload) -> Result<String, X402Error> {
    encode_header(payload)
}

pub fn encode_payment_response_header(response: &SettlementResponse) -> Result<String, X402Error> {
    encode_header(response)
}

pub fn decode_payment_required_header(header: &str) -> Result<PaymentRequired, X402Error> {
    decode_header(header)
}

pub fn decode_payment_signature_header(header: &str) -> Result<PaymentPayload, X402Error> {
    decode_header(header)
}

pub fn decode_payment_response_header(header: &str) -> Result<SettlementResponse, X402Error> {
    decode_header(header)
}

pub fn validate_funding_payload(
    payload: &PaymentPayload,
    required: &PaymentRequired,
    now_unix_seconds: u64,
) -> Result<ValidatedFundingAuthorization, X402Error> {
    if payload.x402_version != X402_VERSION {
        return Err(X402Error::UnsupportedVersion(payload.x402_version));
    }
    if required.x402_version != X402_VERSION {
        return Err(X402Error::UnsupportedVersion(required.x402_version));
    }
    if required.accepts.len() != 1 {
        return Err(X402Error::AmbiguousRequirements);
    }
    let expected = &required.accepts[0];
    if expected.scheme == STANDARD_EXACT_SCHEME {
        return Err(X402Error::UnsafeExactFunding);
    }
    if expected.scheme != AGENT_BOUNTY_FUND_SCHEME || payload.accepted != *expected {
        return Err(X402Error::RequirementsMismatch);
    }
    if payload
        .resource
        .as_ref()
        .is_some_and(|resource| resource != &required.resource)
    {
        return Err(X402Error::ResourceMismatch);
    }
    if payload.extensions != required.extensions {
        return Err(X402Error::ExtensionsMismatch);
    }

    let authorization_payload: Eip3009Payload = serde_json::from_value(payload.payload.clone())
        .map_err(|_| X402Error::InvalidAuthorization)?;
    let contributor = normalize_address(&authorization_payload.authorization.from)?;
    let recipient = normalize_address(&authorization_payload.authorization.to)?;
    let expected_recipient = normalize_address(&expected.pay_to)?;
    if recipient != expected_recipient {
        return Err(X402Error::RecipientMismatch);
    }
    let amount = parse_positive_u64(&authorization_payload.authorization.value)?;
    let expected_amount = parse_positive_u64(&expected.amount)?;
    if amount != expected_amount {
        return Err(X402Error::AmountMismatch);
    }
    let valid_after = authorization_payload
        .authorization
        .valid_after
        .parse::<u64>()
        .map_err(|_| X402Error::InvalidValidAfter)?;
    if valid_after != 0 {
        return Err(X402Error::InvalidValidAfter);
    }
    let valid_before = authorization_payload
        .authorization
        .valid_before
        .parse::<u64>()
        .map_err(|_| X402Error::AuthorizationExpired)?;
    if valid_before < now_unix_seconds.saturating_add(MIN_SETTLEMENT_BUFFER_SECONDS) {
        return Err(X402Error::AuthorizationExpired);
    }
    if valid_before
        > now_unix_seconds
            .saturating_add(expected.max_timeout_seconds)
            .saturating_add(MAX_CLOCK_SKEW_SECONDS)
    {
        return Err(X402Error::AuthorizationTooLong);
    }
    let nonce = normalize_word(&authorization_payload.authorization.nonce)
        .map_err(|_| X402Error::InvalidNonce)?;
    let (v, r, s) = split_signature(&authorization_payload.signature)?;
    let digest = eip3009_authorization_digest(expected, &authorization_payload.authorization)?;
    let signature = authorization_payload
        .signature
        .parse::<Signature>()
        .map_err(|_| X402Error::InvalidSignature)?;
    let recovered = signature
        .recover_address_from_prehash(&digest)
        .map_err(|_| X402Error::InvalidSignature)?;
    let expected_signer = contributor
        .parse::<Address>()
        .map_err(|_| X402Error::InvalidSignature)?;
    if recovered != expected_signer {
        return Err(X402Error::InvalidSignature);
    }

    Ok(ValidatedFundingAuthorization {
        contributor,
        bounty_contract: expected_recipient,
        amount,
        valid_before,
        nonce,
        v,
        r,
        s,
    })
}

pub fn validate_exact_service_payload(
    payload: &PaymentPayload,
    required: &PaymentRequired,
    now_unix_seconds: u64,
) -> Result<ValidatedExactAuthorization, X402Error> {
    if payload.x402_version != X402_VERSION {
        return Err(X402Error::UnsupportedVersion(payload.x402_version));
    }
    if required.x402_version != X402_VERSION {
        return Err(X402Error::UnsupportedVersion(required.x402_version));
    }
    if required.accepts.len() != 1 {
        return Err(X402Error::AmbiguousRequirements);
    }
    let expected = &required.accepts[0];
    if expected.scheme != STANDARD_EXACT_SCHEME || payload.accepted != *expected {
        return Err(X402Error::RequirementsMismatch);
    }
    if payload
        .resource
        .as_ref()
        .is_some_and(|resource| resource != &required.resource)
    {
        return Err(X402Error::ResourceMismatch);
    }
    if payload.extensions != required.extensions {
        return Err(X402Error::ExtensionsMismatch);
    }

    let authorization_payload: Eip3009Payload = serde_json::from_value(payload.payload.clone())
        .map_err(|_| X402Error::InvalidAuthorization)?;
    let payer = normalize_address(&authorization_payload.authorization.from)?;
    let recipient = normalize_address(&authorization_payload.authorization.to)?;
    if recipient != normalize_address(&expected.pay_to)? {
        return Err(X402Error::RecipientMismatch);
    }
    let amount = parse_positive_u64(&authorization_payload.authorization.value)?;
    if amount != parse_positive_u64(&expected.amount)? {
        return Err(X402Error::AmountMismatch);
    }
    let valid_after = authorization_payload
        .authorization
        .valid_after
        .parse::<u64>()
        .map_err(|_| X402Error::InvalidValidAfter)?;
    if valid_after != 0 {
        return Err(X402Error::InvalidValidAfter);
    }
    let valid_before = authorization_payload
        .authorization
        .valid_before
        .parse::<u64>()
        .map_err(|_| X402Error::AuthorizationExpired)?;
    if valid_before < now_unix_seconds.saturating_add(MIN_SETTLEMENT_BUFFER_SECONDS) {
        return Err(X402Error::AuthorizationExpired);
    }
    if valid_before
        > now_unix_seconds
            .saturating_add(expected.max_timeout_seconds)
            .saturating_add(MAX_CLOCK_SKEW_SECONDS)
    {
        return Err(X402Error::AuthorizationTooLong);
    }
    let nonce = normalize_word(&authorization_payload.authorization.nonce)
        .map_err(|_| X402Error::InvalidNonce)?;
    let (v, r, s) = split_signature(&authorization_payload.signature)?;
    let digest = eip3009_authorization_digest(expected, &authorization_payload.authorization)?;
    let recovered = authorization_payload
        .signature
        .parse::<Signature>()
        .map_err(|_| X402Error::InvalidSignature)?
        .recover_address_from_prehash(&digest)
        .map_err(|_| X402Error::InvalidSignature)?;
    if recovered
        != payer
            .parse::<Address>()
            .map_err(|_| X402Error::InvalidSignature)?
    {
        return Err(X402Error::InvalidSignature);
    }
    Ok(ValidatedExactAuthorization {
        payer,
        recipient,
        amount,
        valid_before,
        nonce,
        v,
        r,
        s,
    })
}

fn eip3009_authorization_digest(
    requirements: &PaymentRequirements,
    authorization: &Eip3009Authorization,
) -> Result<B256, X402Error> {
    let chain_id = requirements
        .network
        .strip_prefix("eip155:")
        .ok_or(X402Error::InvalidChallenge("network must be eip155"))?
        .parse::<u64>()
        .map_err(|_| X402Error::InvalidChallenge("network chain ID is invalid"))?;
    let verifying_contract = requirements
        .asset
        .parse::<Address>()
        .map_err(|_| X402Error::InvalidChallenge("asset address is invalid"))?;
    let name = requirements
        .extra
        .get("name")
        .and_then(Value::as_str)
        .ok_or(X402Error::InvalidChallenge("USDC EIP-712 name is missing"))?;
    let version = requirements
        .extra
        .get("version")
        .and_then(Value::as_str)
        .ok_or(X402Error::InvalidChallenge(
            "USDC EIP-712 version is missing",
        ))?;
    let from = authorization
        .from
        .parse::<Address>()
        .map_err(|_| X402Error::InvalidAddress)?;
    let to = authorization
        .to
        .parse::<Address>()
        .map_err(|_| X402Error::InvalidAddress)?;
    let value = authorization
        .value
        .parse::<u64>()
        .map(U256::from)
        .map_err(|_| X402Error::InvalidAmount)?;
    let valid_after = authorization
        .valid_after
        .parse::<u64>()
        .map(U256::from)
        .map_err(|_| X402Error::InvalidValidAfter)?;
    let valid_before = authorization
        .valid_before
        .parse::<u64>()
        .map(U256::from)
        .map_err(|_| X402Error::AuthorizationExpired)?;
    let nonce = authorization
        .nonce
        .parse::<B256>()
        .map_err(|_| X402Error::InvalidNonce)?;

    let domain_separator = keccak256(
        (
            keccak256(EIP712_DOMAIN_TYPE),
            keccak256(name),
            keccak256(version),
            U256::from(chain_id),
            verifying_contract,
        )
            .abi_encode(),
    );
    let authorization_hash = keccak256(
        (
            keccak256(TRANSFER_WITH_AUTHORIZATION_TYPE),
            from,
            to,
            value,
            valid_after,
            valid_before,
            nonce,
        )
            .abi_encode(),
    );
    let mut preimage = Vec::with_capacity(66);
    preimage.extend_from_slice(&[0x19, 0x01]);
    preimage.extend_from_slice(domain_separator.as_slice());
    preimage.extend_from_slice(authorization_hash.as_slice());
    Ok(keccak256(preimage))
}

fn encode_header<T: Serialize>(value: &T) -> Result<String, X402Error> {
    let json = serde_json::to_vec(value).map_err(|_| X402Error::InvalidJson)?;
    let encoded = STANDARD.encode(json);
    if encoded.len() > MAX_HEADER_LENGTH {
        return Err(X402Error::HeaderTooLarge);
    }
    Ok(encoded)
}

fn decode_header<T: DeserializeOwned>(header: &str) -> Result<T, X402Error> {
    if header.is_empty() {
        return Err(X402Error::EmptyHeader);
    }
    if header.len() > MAX_HEADER_LENGTH {
        return Err(X402Error::HeaderTooLarge);
    }
    let decoded = STANDARD
        .decode(header)
        .map_err(|_| X402Error::InvalidBase64)?;
    serde_json::from_slice(&decoded).map_err(|_| X402Error::InvalidJson)
}

fn normalize_address(value: &str) -> Result<String, X402Error> {
    let raw = value.strip_prefix("0x").ok_or(X402Error::InvalidAddress)?;
    if raw.len() != 40 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(X402Error::InvalidAddress);
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn normalize_word(value: &str) -> Result<String, X402Error> {
    let raw = value.strip_prefix("0x").ok_or(X402Error::InvalidNonce)?;
    if raw.len() != 64 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(X402Error::InvalidNonce);
    }
    Ok(format!("0x{}", raw.to_ascii_lowercase()))
}

fn parse_positive_u64(value: &str) -> Result<u64, X402Error> {
    let parsed = value.parse::<u64>().map_err(|_| X402Error::InvalidAmount)?;
    if parsed == 0 {
        return Err(X402Error::InvalidAmount);
    }
    Ok(parsed)
}

fn parse_u128(value: &str) -> Result<u128, X402Error> {
    value.parse::<u128>().map_err(|_| X402Error::InvalidAmount)
}

fn split_signature(signature: &str) -> Result<(u8, String, String), X402Error> {
    let raw = signature
        .strip_prefix("0x")
        .ok_or(X402Error::InvalidSignature)?;
    if raw.len() != 130 || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(X402Error::InvalidSignature);
    }
    let bytes = hex::decode(raw).map_err(|_| X402Error::InvalidSignature)?;
    let v = match bytes[64] {
        0 | 27 => 27,
        1 | 28 => 28,
        _ => return Err(X402Error::InvalidSignature),
    };
    Ok((
        v,
        format!("0x{}", raw[..64].to_ascii_lowercase()),
        format!("0x{}", raw[64..128].to_ascii_lowercase()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};

    const ASSET: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
    const SEPOLIA_ASSET: &str = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
    const BOUNTY: &str = "0x1111111111111111111111111111111111111111";
    const FUNDER: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
    const TEST_PRIVATE_KEY: &str =
        "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const FOUNDRY_TYPED_DATA_SIGNATURE: &str = "0x525987b32d816adbfab6840381acfb64549b4f00e7d2ec2229e67182f676a80c768b69e91496161b6039cafe5a71925101fd751f06c29cd34ef3ae5cf91674601c";
    const SEPOLIA_FOUNDRY_TYPED_DATA_SIGNATURE: &str = "0x8551589ae4ba572d312a887e4efedc8cc5ce3519642e9a97c64afdd2a3e4be6b2de86e7c95e6990737efc5bac5b8cb820c5129f543a21b516c6dd6595c2e80241b";
    const NONCE: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn challenge() -> PaymentRequired {
        base_usdc_funding_challenge(
            "https://api.example/v1/x402/base/bounties/0x111/funding?amount=150000",
            "eip155:8453",
            ASSET,
            BOUNTY,
            150_000,
            300,
        )
        .unwrap()
    }

    fn authorization() -> Eip3009Authorization {
        Eip3009Authorization {
            from: FUNDER.to_string(),
            to: BOUNTY.to_string(),
            value: "150000".to_string(),
            valid_after: "0".to_string(),
            valid_before: "1300".to_string(),
            nonce: NONCE.to_string(),
        }
    }

    fn payment(required: &PaymentRequired) -> PaymentPayload {
        let authorization = authorization();
        let digest = eip3009_authorization_digest(&required.accepts[0], &authorization).unwrap();
        let signer = TEST_PRIVATE_KEY.parse::<PrivateKeySigner>().unwrap();
        assert_eq!(format!("{:#x}", signer.address()), FUNDER);
        let signature = signer.sign_hash_sync(&digest).unwrap().to_string();
        PaymentPayload {
            x402_version: X402_VERSION,
            resource: Some(required.resource.clone()),
            accepted: required.accepts[0].clone(),
            payload: json!({
                "signature": signature,
                "authorization": authorization
            }),
            extensions: required.extensions.clone(),
        }
    }

    fn proof_service_challenge() -> PaymentRequired {
        let quote = quote_proof_broker_job(ProofBrokerQuoteRequest {
            network: "eip155:8453".to_string(),
            competition_contract: "0x2222222222222222222222222222222222222222".to_string(),
            solver: FUNDER.to_string(),
            solver_nonce: "7".to_string(),
            artifact_hash: format!("0x{}", "bb".repeat(32)),
            proof_system: "groth16".to_string(),
            gross_prize: "2000000".to_string(),
            proof_fee_quote: "140000".to_string(),
            relay_fee_quote: "10000".to_string(),
            winner_mode: CompetitionWinnerMode::FirstProven,
            proof_deadline: 2_000,
            measured_proof_sla_seconds: 60,
            now_unix_seconds: 1_000,
        })
        .unwrap();
        base_usdc_exact_service_challenge(
            "https://api.example/v1/base/open-competition-v2-beta3/proof-jobs/1/payment",
            "eip155:8453",
            ASSET,
            BOUNTY,
            &quote,
        )
        .unwrap()
    }

    #[test]
    fn eip3009_digest_matches_foundry_typed_data_vector() {
        let required = challenge();
        let digest = eip3009_authorization_digest(&required.accepts[0], &authorization()).unwrap();
        let signer = TEST_PRIVATE_KEY.parse::<PrivateKeySigner>().unwrap();
        assert_eq!(
            signer.sign_hash_sync(&digest).unwrap().to_string(),
            FOUNDRY_TYPED_DATA_SIGNATURE
        );

        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/base-usdc-eip3009.json")).unwrap();
        assert_eq!(fixture["primaryType"], "TransferWithAuthorization");
        assert_eq!(fixture["domain"]["chainId"], 8453);
        assert_eq!(fixture["domain"]["verifyingContract"], ASSET);
        assert_eq!(fixture["message"]["from"], FUNDER);
        assert_eq!(fixture["message"]["to"], BOUNTY);
        assert_eq!(fixture["message"]["nonce"], NONCE);
    }

    #[test]
    fn base_sepolia_challenge_uses_the_live_usdc_domain_name() {
        let required = base_usdc_funding_challenge(
            "https://api.example/v1/x402/base/bounties/0x111/funding?network=base-sepolia&amount=150000",
            "eip155:84532",
            SEPOLIA_ASSET,
            BOUNTY,
            150_000,
            300,
        )
        .unwrap();
        assert_eq!(required.accepts[0].extra["name"], "USDC");
        assert_eq!(required.accepts[0].extra["version"], "2");
        let digest = eip3009_authorization_digest(&required.accepts[0], &authorization()).unwrap();
        let signer = TEST_PRIVATE_KEY.parse::<PrivateKeySigner>().unwrap();
        assert_eq!(
            signer.sign_hash_sync(&digest).unwrap().to_string(),
            SEPOLIA_FOUNDRY_TYPED_DATA_SIGNATURE
        );
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/base-sepolia-usdc-eip3009.json"))
                .unwrap();
        assert_eq!(fixture["domain"]["name"], "USDC");
        assert_eq!(fixture["domain"]["chainId"], 84532);
        assert!(validate_funding_payload(&payment(&required), &required, 1_000).is_ok());
    }

    #[test]
    fn payment_required_and_signature_headers_round_trip() {
        let required = challenge();
        let required_header = encode_payment_required_header(&required).unwrap();
        assert_eq!(
            decode_payment_required_header(&required_header).unwrap(),
            required
        );

        let payload = payment(&required);
        let signature_header = encode_payment_signature_header(&payload).unwrap();
        assert_eq!(
            decode_payment_signature_header(&signature_header).unwrap(),
            payload
        );

        let response = SettlementResponse {
            success: true,
            error_reason: None,
            error_message: None,
            payer: Some(FUNDER.to_string()),
            transaction: "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .to_string(),
            network: "eip155:8453".to_string(),
            amount: Some("150000".to_string()),
            extensions: Some(BTreeMap::from([(
                "agent-bounties".to_string(),
                json!({"canonicalEvent": "FundingAdded"}),
            )])),
            extra: None,
        };
        let response_header = encode_payment_response_header(&response).unwrap();
        assert_eq!(
            decode_payment_response_header(&response_header).unwrap(),
            response
        );
    }

    #[test]
    fn validates_exact_bounty_authorization() {
        let required = challenge();
        let validated = validate_funding_payload(&payment(&required), &required, 1_000).unwrap();
        assert_eq!(validated.contributor, FUNDER);
        assert_eq!(validated.bounty_contract, BOUNTY);
        assert_eq!(validated.amount, 150_000);
        assert_eq!(validated.valid_before, 1_300);
        assert_eq!(validated.nonce, NONCE);
        assert!(matches!(validated.v, 27 | 28));
        assert_eq!(validated.r.len(), 66);
        assert_eq!(validated.s.len(), 66);
    }

    #[test]
    fn validates_standard_exact_proof_service_authorization() {
        let required = proof_service_challenge();
        assert_eq!(required.accepts[0].scheme, STANDARD_EXACT_SCHEME);
        assert_eq!(required.accepts[0].extra["name"], "USD Coin");
        let validated =
            validate_exact_service_payload(&payment(&required), &required, 1_000).unwrap();
        assert_eq!(validated.payer, FUNDER);
        assert_eq!(validated.recipient, BOUNTY);
        assert_eq!(validated.amount, 150_000);
        assert_eq!(validated.nonce, NONCE);
    }

    #[test]
    fn published_compatibility_vector_is_executable() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../../site/x402-test-vectors.json")).unwrap();
        for id in [
            "valid_custom_bounty_funding",
            "valid_base_sepolia_custom_bounty_funding",
        ] {
            let vector = fixture["vectors"]
                .as_array()
                .unwrap()
                .iter()
                .find(|vector| vector["id"] == id)
                .unwrap();
            let required: PaymentRequired =
                serde_json::from_value(vector["payment_required"].clone()).unwrap();
            let payload: PaymentPayload =
                serde_json::from_value(vector["payment_payload"].clone()).unwrap();
            let now = vector["now_unix_seconds"].as_u64().unwrap();

            let validated = validate_funding_payload(&payload, &required, now).unwrap();
            assert_eq!(validated.contributor, vector["expected"]["contributor"]);
            assert_eq!(
                validated.bounty_contract,
                vector["expected"]["bounty_contract"]
            );
            assert_eq!(validated.amount.to_string(), vector["expected"]["amount"]);
        }
    }

    #[test]
    fn rejects_standard_exact_funding_to_avoid_stranded_usdc() {
        let mut required = challenge();
        required.accepts[0].scheme = STANDARD_EXACT_SCHEME.to_string();
        let payload = payment(&required);
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::UnsafeExactFunding
        );
    }

    #[test]
    fn rejects_tampered_requirements_resource_and_extensions() {
        let required = challenge();
        let mut payload = payment(&required);
        payload.accepted.amount = "1".to_string();
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::RequirementsMismatch
        );

        let mut payload = payment(&required);
        payload.resource.as_mut().unwrap().url.push_str("/redirect");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::ResourceMismatch
        );

        let mut payload = payment(&required);
        payload.extensions = Some(BTreeMap::from([("unexpected".to_string(), json!(true))]));
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::ExtensionsMismatch
        );
    }

    #[test]
    fn rejects_wrong_recipient_amount_window_nonce_and_signature() {
        let required = challenge();
        let mut payload = payment(&required);
        payload.payload["authorization"]["to"] = json!(FUNDER);
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::RecipientMismatch
        );

        let mut payload = payment(&required);
        payload.payload["authorization"]["value"] = json!("149999");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::AmountMismatch
        );

        let mut payload = payment(&required);
        payload.payload["authorization"]["validBefore"] = json!("1005");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::AuthorizationExpired
        );

        let mut payload = payment(&required);
        payload.payload["authorization"]["validBefore"] = json!("1331");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::AuthorizationTooLong
        );

        let mut payload = payment(&required);
        payload.payload["authorization"]["nonce"] = json!("0x01");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::InvalidNonce
        );

        let mut payload = payment(&required);
        payload.payload["authorization"]["from"] =
            json!("0x2222222222222222222222222222222222222222");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::InvalidSignature
        );

        let mut payload = payment(&required);
        payload.payload["signature"] = json!("0x01");
        assert_eq!(
            validate_funding_payload(&payload, &required, 1_000).unwrap_err(),
            X402Error::InvalidSignature
        );
    }

    #[test]
    fn rejects_malformed_and_oversized_headers() {
        assert_eq!(
            decode_payment_signature_header("not base64!").unwrap_err(),
            X402Error::InvalidBase64
        );
        assert_eq!(
            decode_payment_signature_header(&"A".repeat(MAX_HEADER_LENGTH + 1)).unwrap_err(),
            X402Error::HeaderTooLarge
        );
    }

    #[test]
    fn proof_quote_binds_solver_artifact_cost_and_competition_risk() {
        let request = ProofBrokerQuoteRequest {
            network: "eip155:8453".to_string(),
            competition_contract: BOUNTY.to_string(),
            solver: FUNDER.to_string(),
            solver_nonce: "7".to_string(),
            artifact_hash: NONCE.to_string(),
            proof_system: "groth16".to_string(),
            gross_prize: "2000000".to_string(),
            proof_fee_quote: "100000".to_string(),
            relay_fee_quote: "10000".to_string(),
            winner_mode: CompetitionWinnerMode::BestScore,
            proof_deadline: 2_000,
            measured_proof_sla_seconds: 60,
            now_unix_seconds: 1_000,
        };
        let quote = quote_proof_broker_job(request.clone()).unwrap();
        assert_eq!(quote.net_prize_if_win, "1890000");
        assert_eq!(quote.maximum_charge, "110000");
        assert!(quote.profitable_if_win);
        assert_eq!(quote.quote_expiration, 1_300);
        assert!(quote.competition_risk.contains("not guaranteed profit"));

        let challenge = base_usdc_exact_service_challenge(
            "https://api.example/v1/base/open-competition-v2/proof-jobs",
            "eip155:8453",
            ASSET,
            FUNDER,
            &quote,
        )
        .unwrap();
        assert_eq!(challenge.accepts[0].scheme, STANDARD_EXACT_SCHEME);
        assert_eq!(challenge.accepts[0].amount, "110000");

        let mut late = request;
        late.proof_deadline = 1_059;
        assert_eq!(
            quote_proof_broker_job(late).unwrap_err(),
            X402Error::ProofSlaMiss
        );
    }
}
