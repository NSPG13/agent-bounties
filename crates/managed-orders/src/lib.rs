//! Immutable, itemized managed-service offers. Acceptance is not payment or funding.
//! Quote inputs are assembled by trusted server pricing/configuration, not a public request body.

use chain_base::autonomous_v2_platform_fee;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use uuid::Uuid;

pub mod bridge_transfer;
pub mod deposit_payout;
pub mod funding;
pub mod pricing;
pub mod recovery;
pub mod review;
pub mod stripe_checkout;
pub mod stripe_dispute;
pub mod stripe_refund;

pub const PLATFORM_FEE_BPS: u16 = 750;
pub(crate) const MAX_EXACT_INTEGER: u64 = 9_007_199_254_740_991;
const SCHEMA: &str = "agent-bounties/managed-service-quote-v2";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OrderError {
    #[error("invalid managed-service quote")]
    InvalidQuote,
    #[error("managed-service amount exceeds its exact integer range")]
    InvalidAmount,
    #[error("managed-service quote has expired or is not yet valid")]
    Expired,
    #[error("managed-service acceptance conflicts with the issued quote")]
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FiatCurrency {
    Mxn,
    Usd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceTerms {
    pub deliverable: String,
    pub acceptance_criteria: String,
    pub delivery_deadline: i64,
    pub terms_version: String,
    pub remedies: String,
}

/// A disclosed offered-price basis, not an executed or guaranteed conversion.
/// `numerator / denominator` is fiat minor units per ONE USDC, not per micro-USDC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceBasis {
    pub reference: String,
    pub minor_units_per_usdc_numerator: u64,
    pub minor_units_per_usdc_denominator: u64,
    pub valid_until: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdditionalCost {
    pub code: String,
    pub label: String,
    pub amount_minor: u64,
}

/// The exact canonical autonomous-v2 bounty an order funds. The funding worker may submit only
/// the funding-wallet call whose Keccak-256 is `create_call_keccak256`. Live Stripe mode binds
/// Base mainnet and test mode binds Base Sepolia, so a test payment can never fund mainnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BountyCommitment {
    pub network: String,
    pub factory: String,
    pub funding_wallet: String,
    pub creation_nonce: String,
    pub terms_hash: String,
    pub verification_setup: String,
    pub funding_deadline: i64,
    pub predicted_bounty: String,
    pub create_call_keccak256: String,
}

fn address(value: &str) -> bool {
    value.len() == 42
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && value[2..].bytes().any(|b| b != b'0')
}

fn word(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && value[2..].bytes().any(|b| b != b'0')
}

impl BountyCommitment {
    fn valid_for(&self, input: &QuoteInput) -> bool {
        let network = if input.livemode {
            "base-mainnet"
        } else {
            "base-sepolia"
        };
        self.network == network
            && address(&self.factory)
            && address(&self.funding_wallet)
            && address(&self.predicted_bounty)
            && self.factory != self.funding_wallet
            && self.predicted_bounty != self.factory
            && self.predicted_bounty != self.funding_wallet
            && word(&self.creation_nonce)
            && word(&self.terms_hash)
            && word(&self.create_call_keccak256)
            && valid_key(&self.verification_setup)
            && self.funding_deadline > input.expires_at
            && self.funding_deadline <= input.terms.delivery_deadline
    }

    /// Structurally valid commitment for fixtures in this and dependent crates' tests.
    #[doc(hidden)]
    pub fn test_fixture(livemode: bool, funding_deadline: i64) -> Self {
        Self {
            network: if livemode {
                "base-mainnet"
            } else {
                "base-sepolia"
            }
            .into(),
            factory: format!("0x{}", "c3".repeat(20)),
            funding_wallet: format!("0x{}", "f0".repeat(20)),
            creation_nonce: format!("0x{}", "66".repeat(32)),
            terms_hash: format!("0x{}", "11".repeat(32)),
            verification_setup: "customer_review_v1".into(),
            funding_deadline,
            predicted_bounty: format!("0x{}", "b0".repeat(20)),
            create_call_keccak256: format!("0x{}", "ca".repeat(32)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuoteInput {
    pub quote_id: Uuid,
    pub buyer_principal: String,
    pub stripe_account_id: String,
    pub livemode: bool,
    pub issued_at: i64,
    pub expires_at: i64,
    pub terms: ServiceTerms,
    pub currency: FiatCurrency,
    pub price_basis: PriceBasis,
    pub solver_reward_micro_usdc: u64,
    pub verifier_reserve_micro_usdc: u64,
    pub additional_costs: Vec<AdditionalCost>,
    pub bounty: BountyCommitment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriceLine {
    pub code: String,
    pub label: String,
    pub amount_minor: u64,
    pub amount_micro_usdc: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedQuote {
    schema: String,
    input: QuoteInput,
    platform_fee_bps: u16,
    platform_fee_micro_usdc: u64,
    bounty_target_micro_usdc: u64,
    lines: Vec<PriceLine>,
    total_minor: u64,
}

impl<'de> Deserialize<'de> for ManagedQuote {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            schema: String,
            input: QuoteInput,
            platform_fee_bps: u16,
            platform_fee_micro_usdc: u64,
            bounty_target_micro_usdc: u64,
            lines: Vec<PriceLine>,
            total_minor: u64,
        }
        let document = Document::deserialize(deserializer)?;
        let quote = Self {
            schema: document.schema,
            input: document.input,
            platform_fee_bps: document.platform_fee_bps,
            platform_fee_micro_usdc: document.platform_fee_micro_usdc,
            bounty_target_micro_usdc: document.bounty_target_micro_usdc,
            lines: document.lines,
            total_minor: document.total_minor,
        };
        quote.validate().map_err(serde::de::Error::custom)?;
        Ok(quote)
    }
}

fn text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

pub fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.:".contains(&c))
}

fn bounded(value: u64) -> Result<u64, OrderError> {
    if value > MAX_EXACT_INTEGER {
        return Err(OrderError::InvalidAmount);
    }
    Ok(value)
}

/// USDC micro-units in fiat minor units at `basis`, rounded up to the customer.
pub fn fiat_amount(micro_usdc: u64, basis: &PriceBasis) -> Result<u64, OrderError> {
    let numerator = u128::from(micro_usdc)
        .checked_mul(u128::from(basis.minor_units_per_usdc_numerator))
        .ok_or(OrderError::InvalidAmount)?;
    let denominator = 1_000_000_u128
        .checked_mul(u128::from(basis.minor_units_per_usdc_denominator))
        .filter(|v| *v > 0)
        .ok_or(OrderError::InvalidAmount)?;
    let amount = numerator / denominator + u128::from(numerator % denominator != 0);
    bounded(u64::try_from(amount).map_err(|_| OrderError::InvalidAmount)?)
}

impl ManagedQuote {
    pub fn new(input: QuoteInput) -> Result<Self, OrderError> {
        if input.quote_id.is_nil()
            || !valid_key(&input.buyer_principal)
            || !input.stripe_account_id.starts_with("acct_")
            || input.stripe_account_id.len() <= 5
            || !valid_key(&input.stripe_account_id)
            || input.issued_at <= 0
            || input.expires_at <= input.issued_at
            || input.expires_at > input.price_basis.valid_until
            || input.terms.delivery_deadline <= input.expires_at
            || input.terms.delivery_deadline > MAX_EXACT_INTEGER as i64
            || !text(&input.terms.deliverable, 16_384)
            || !text(&input.terms.acceptance_criteria, 16_384)
            || !valid_key(&input.terms.terms_version)
            || !text(&input.terms.remedies, 8_192)
            || !valid_key(&input.price_basis.reference)
            || input.price_basis.valid_until > MAX_EXACT_INTEGER as i64
            || input.solver_reward_micro_usdc == 0
            || input.verifier_reserve_micro_usdc == 0
            || input.price_basis.minor_units_per_usdc_numerator == 0
            || input.price_basis.minor_units_per_usdc_denominator == 0
            || input.additional_costs.len() > 8
            || !input.bounty.valid_for(&input)
        {
            return Err(OrderError::InvalidQuote);
        }
        bounded(input.solver_reward_micro_usdc)?;
        bounded(input.verifier_reserve_micro_usdc)?;
        bounded(input.price_basis.minor_units_per_usdc_numerator)?;
        bounded(input.price_basis.minor_units_per_usdc_denominator)?;
        let fee =
            autonomous_v2_platform_fee(input.solver_reward_micro_usdc.into(), PLATFORM_FEE_BPS)
                .map_err(|_| OrderError::InvalidAmount)?;
        let fee = u64::try_from(fee).map_err(|_| OrderError::InvalidAmount)?;
        let target = input
            .solver_reward_micro_usdc
            .checked_add(input.verifier_reserve_micro_usdc)
            .and_then(|n| n.checked_add(fee))
            .ok_or(OrderError::InvalidAmount)?;
        bounded(target)?;
        let mut lines = Vec::new();
        for (code, label, amount) in [
            (
                "solver_reward",
                "Solver reward",
                input.solver_reward_micro_usdc,
            ),
            (
                "verification",
                "Verification reserve",
                input.verifier_reserve_micro_usdc,
            ),
            ("platform_fee", "Platform fee (7.5% of solver reward)", fee),
        ] {
            lines.push(PriceLine {
                code: code.into(),
                label: label.into(),
                amount_minor: fiat_amount(amount, &input.price_basis)?,
                amount_micro_usdc: Some(amount),
            });
        }
        let mut codes: HashSet<String> = lines.iter().map(|line| line.code.clone()).collect();
        for extra in &input.additional_costs {
            if !valid_key(&extra.code)
                || !text(&extra.label, 160)
                || extra.amount_minor == 0
                || !codes.insert(extra.code.clone())
            {
                return Err(OrderError::InvalidQuote);
            }
            bounded(extra.amount_minor)?;
            lines.push(PriceLine {
                code: extra.code.clone(),
                label: extra.label.clone(),
                amount_minor: extra.amount_minor,
                amount_micro_usdc: None,
            });
        }
        let total_minor = lines.iter().try_fold(0_u64, |sum, line| {
            bounded(
                sum.checked_add(line.amount_minor)
                    .ok_or(OrderError::InvalidAmount)?,
            )
        })?;
        Ok(Self {
            schema: SCHEMA.into(),
            input,
            platform_fee_bps: PLATFORM_FEE_BPS,
            platform_fee_micro_usdc: fee,
            bounty_target_micro_usdc: target,
            lines,
            total_minor,
        })
    }

    /// Recompute all derived amounts after deserialization; private Rust fields alone
    /// do not make serialized data trusted. Call before persistence or acceptance.
    pub fn validate(&self) -> Result<(), OrderError> {
        if *self != Self::new(self.input.clone())? {
            return Err(OrderError::InvalidQuote);
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, OrderError> {
        self.validate()?;
        let encoded = serde_json::to_vec(self).map_err(|_| OrderError::InvalidQuote)?;
        let mut hash = Sha256::new();
        hash.update(b"agent-bounties:managed-service-quote:v2\0");
        hash.update(encoded);
        Ok(hex::encode(hash.finalize()))
    }

    pub fn validate_acceptance(
        &self,
        buyer: &str,
        digest: &str,
        now: i64,
    ) -> Result<(), OrderError> {
        self.validate()?;
        if self.input.buyer_principal != buyer || self.digest()? != digest {
            return Err(OrderError::Conflict);
        }
        if now < self.input.issued_at || now >= self.input.expires_at {
            return Err(OrderError::Expired);
        }
        Ok(())
    }

    pub fn input(&self) -> &QuoteInput {
        &self.input
    }
    pub fn lines(&self) -> &[PriceLine] {
        &self.lines
    }
    pub fn total_minor(&self) -> u64 {
        self.total_minor
    }
    pub fn bounty_target_micro_usdc(&self) -> u64 {
        self.bounty_target_micro_usdc
    }
    pub fn platform_fee_micro_usdc(&self) -> u64 {
        self.platform_fee_micro_usdc
    }
}

#[cfg(test)]
mod tests;
