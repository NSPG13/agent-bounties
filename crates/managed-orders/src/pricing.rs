//! Server-owned itemized pricing for card-paid bounties. Integer arithmetic only: every line
//! rounds up, so a disclosed price never undercounts the conversion, tax or processor fee.
use crate::{AdditionalCost, OrderError, PriceBasis, MAX_EXACT_INTEGER};

const MAX_PRICE_DECIMALS: usize = 6;
const BPS: u128 = 10_000;

/// Pricing parameters configured by the merchant. Rates are basis points.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PricingPolicy {
    /// Disclosed buffer over the reference rate for currency movement and USDC conversion.
    pub conversion_buffer_bps: u16,
    /// Optional tax on the service (e.g. IVA). `None` until the merchant confirms it applies.
    pub service_tax_bps: Option<u16>,
    /// Processor percentage fee, e.g. 410 for 3.6% domestic plus 0.5% international cards.
    pub processing_rate_bps: u16,
    /// Processor fixed fee per charge in fiat minor units.
    pub processing_fixed_minor: u64,
    /// Tax the processor charges on its own fee, e.g. 1600 for IVA on Stripe Mexico fees.
    pub processing_fee_tax_bps: u16,
}

impl PricingPolicy {
    pub fn validate(&self) -> Result<(), OrderError> {
        let rate =
            u128::from(self.processing_rate_bps) * (BPS + u128::from(self.processing_fee_tax_bps));
        if self.conversion_buffer_bps > 1_000
            || self
                .service_tax_bps
                .is_some_and(|bps| bps == 0 || bps > 3_000)
            || self.processing_rate_bps > 1_000
            || self.processing_fee_tax_bps > 3_000
            || self.processing_fixed_minor > 100_000
            || rate >= BPS * BPS / 2
        {
            return Err(OrderError::InvalidQuote);
        }
        Ok(())
    }
}

/// Builds the price basis from a decimal fiat-per-USD quote such as `"18.700000000000"`.
/// Digits beyond six decimals round up. The basis is minor fiat units per ONE USDC.
pub fn price_basis_from_decimal(
    price: &str,
    reference: &str,
    valid_until: i64,
) -> Result<PriceBasis, OrderError> {
    let (whole, fraction) = price.split_once('.').unwrap_or((price, ""));
    if whole.is_empty()
        || whole.len() > 7
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 32
    {
        return Err(OrderError::InvalidQuote);
    }
    let kept = &fraction[..fraction.len().min(MAX_PRICE_DECIMALS)];
    let dropped_nonzero = fraction.len() > MAX_PRICE_DECIMALS
        && fraction[MAX_PRICE_DECIMALS..].bytes().any(|b| b != b'0');
    let mut micro_price: u128 = whole
        .parse::<u128>()
        .map_err(|_| OrderError::InvalidQuote)?
        * 10u128.pow(MAX_PRICE_DECIMALS as u32);
    if !kept.is_empty() {
        micro_price += kept.parse::<u128>().map_err(|_| OrderError::InvalidQuote)?
            * 10u128.pow((MAX_PRICE_DECIMALS - kept.len()) as u32);
    }
    micro_price += u128::from(dropped_nonzero);
    if micro_price == 0 {
        return Err(OrderError::InvalidQuote);
    }
    // Minor units per USDC = price * 100 = micro_price / 10_000.
    let mut numerator = micro_price;
    let mut denominator = 10_000u128;
    let divisor = gcd(numerator, denominator);
    numerator /= divisor;
    denominator /= divisor;
    let exact = |value: u128| {
        u64::try_from(value)
            .ok()
            .filter(|v| *v <= MAX_EXACT_INTEGER)
            .ok_or(OrderError::InvalidAmount)
    };
    Ok(PriceBasis {
        reference: reference.to_string(),
        minor_units_per_usdc_numerator: exact(numerator)?,
        minor_units_per_usdc_denominator: exact(denominator)?,
        valid_until,
    })
}

pub const REVIEW_DEPOSIT_CODE: &str = "review_deposit";

/// Additional cost lines for a quote whose converted USDC lines total `base_minor`: the
/// conversion buffer, the optional service tax, the refundable review deposit, and card
/// processing grossed up so the
/// processor's fee and the tax on that fee are covered by the amount charged.
pub fn priced_additional_costs(
    base_minor: u64,
    review_deposit_minor: u64,
    policy: &PricingPolicy,
) -> Result<Vec<AdditionalCost>, OrderError> {
    policy.validate()?;
    if base_minor == 0 || base_minor > MAX_EXACT_INTEGER {
        return Err(OrderError::InvalidAmount);
    }
    let mut lines = Vec::new();
    let mut subtotal = u128::from(base_minor);
    if policy.conversion_buffer_bps > 0 {
        let amount = ceil_div(subtotal * u128::from(policy.conversion_buffer_bps), BPS);
        lines.push(line(
            "currency_conversion",
            format!(
                "Currency conversion buffer ({})",
                percent(policy.conversion_buffer_bps)
            ),
            amount,
        )?);
        subtotal += amount;
    }
    if let Some(bps) = policy.service_tax_bps {
        let amount = ceil_div(subtotal * u128::from(bps), BPS);
        lines.push(line(
            "service_tax",
            format!("IVA ({})", percent(bps)),
            amount,
        )?);
        subtotal += amount;
    }
    if review_deposit_minor > 0 {
        // Refundable; card processing below also covers it.
        lines.push(line(
            REVIEW_DEPOSIT_CODE,
            "Review deposit (refundable, 10% of solver reward)".into(),
            u128::from(review_deposit_minor),
        )?);
        subtotal += u128::from(review_deposit_minor);
    }
    let charge = gross_up_for_processing(subtotal, policy)?;
    lines.push(line(
        "payment_processing",
        format!(
            "Card processing ({} + fixed fee, with tax on the fee)",
            percent(policy.processing_rate_bps)
        ),
        charge - subtotal,
    )?);
    Ok(lines)
}

/// Processor fee and the tax on it for a charge, each rounded up to a minor unit.
pub fn processing_cost(charge: u128, policy: &PricingPolicy) -> u128 {
    let fee = ceil_div(charge * u128::from(policy.processing_rate_bps), BPS)
        + u128::from(policy.processing_fixed_minor);
    fee + ceil_div(fee * u128::from(policy.processing_fee_tax_bps), BPS)
}

/// The smallest charge that still leaves `subtotal` after `processing_cost`.
fn gross_up_for_processing(subtotal: u128, policy: &PricingPolicy) -> Result<u128, OrderError> {
    let tax = BPS + u128::from(policy.processing_fee_tax_bps);
    let denominator = BPS * BPS - u128::from(policy.processing_rate_bps) * tax;
    let numerator = subtotal * BPS * BPS + u128::from(policy.processing_fixed_minor) * tax * BPS;
    let mut charge = ceil_div(numerator, denominator).max(subtotal);
    while charge > subtotal && charge - 1 - subtotal >= processing_cost(charge - 1, policy) {
        charge -= 1;
    }
    for _ in 0..16 {
        if charge - subtotal >= processing_cost(charge, policy) {
            return if charge <= u128::from(MAX_EXACT_INTEGER) {
                Ok(charge)
            } else {
                Err(OrderError::InvalidAmount)
            };
        }
        charge += 1;
    }
    Err(OrderError::InvalidAmount)
}

fn line(code: &str, label: String, amount: u128) -> Result<AdditionalCost, OrderError> {
    let amount_minor = u64::try_from(amount)
        .ok()
        .filter(|v| *v > 0 && *v <= MAX_EXACT_INTEGER)
        .ok_or(OrderError::InvalidAmount)?;
    Ok(AdditionalCost {
        code: code.to_string(),
        label,
        amount_minor,
    })
}

fn percent(bps: u16) -> String {
    format!("{}.{:02}%", bps / 100, bps % 100)
}

fn ceil_div(numerator: u128, denominator: u128) -> u128 {
    numerator / denominator + u128::from(!numerator.is_multiple_of(denominator))
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests;
