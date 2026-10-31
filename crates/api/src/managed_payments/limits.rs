//! Admission-only USD limit. Existing immutable records and provider recovery stay readable.
use managed_orders::{FiatCurrency, ManagedQuote};

pub(super) fn parse_limit(value: Option<&str>) -> anyhow::Result<Option<u64>> {
    value
        .map(|value| {
            anyhow::ensure!(
                !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
                "MANAGED_SERVICE_MAXIMUM_USD_MINOR must be positive USD cents"
            );
            let limit = value.parse::<u64>()?;
            anyhow::ensure!(
                (1..=99_999_999).contains(&limit),
                "MANAGED_SERVICE_MAXIMUM_USD_MINOR is outside its supported range"
            );
            Ok(limit)
        })
        .transpose()
}

pub(super) fn allows(limit: Option<u64>, quote: &ManagedQuote) -> bool {
    let Some(limit) = limit else { return true };
    if limit == 0 || quote.validate().is_err() {
        return false;
    }
    if quote.input().currency == FiatCurrency::Usd {
        return quote.total_minor() <= limit;
    }
    // The immutable offered rate is fiat minor units per one USDC (USD face value).
    // Cross multiplication includes every price line and rejects even a fraction of a cent
    // above the USD limit, without floating point or rounding down the charged total.
    let basis = &quote.input().price_basis;
    let charged = u128::from(quote.total_minor())
        .checked_mul(u128::from(basis.minor_units_per_usdc_denominator))
        .and_then(|value| value.checked_mul(100));
    let ceiling = u128::from(limit).checked_mul(u128::from(basis.minor_units_per_usdc_numerator));
    matches!((charged, ceiling), (Some(charged), Some(ceiling)) if charged <= ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;
    use managed_orders::AdditionalCost;

    fn quote(currency: FiatCurrency, total: u64, numerator: u64, denominator: u64) -> ManagedQuote {
        let mut input = crate::managed_payments::tests::input(&"a".repeat(64));
        input.currency = currency;
        input.solver_reward_micro_usdc = 100_000;
        input.verifier_reserve_micro_usdc = 100_000;
        input.price_basis.minor_units_per_usdc_numerator = numerator;
        input.price_basis.minor_units_per_usdc_denominator = denominator;
        let base = ManagedQuote::new(input.clone()).unwrap();
        input.additional_costs.push(AdditionalCost {
            code: "processing".into(),
            label: "Card processing".into(),
            amount_minor: total - base.total_minor(),
        });
        ManagedQuote::new(input).unwrap()
    }

    #[test]
    fn usd_limit_configuration_is_optional_but_malformed_values_fail_closed() {
        assert_eq!(parse_limit(None).unwrap(), None);
        assert_eq!(parse_limit(Some("600")).unwrap(), Some(600));
        for value in [
            "",
            "0",
            "-600",
            "+600",
            " 600",
            "600 ",
            "6.00",
            "100000000",
            "18446744073709551616",
        ] {
            assert!(parse_limit(Some(value)).is_err(), "{value}");
        }
    }

    #[test]
    fn usd_limit_includes_fees_and_rejects_a_fraction_above_the_limit() {
        assert!(allows(
            Some(600),
            &quote(FiatCurrency::Mxn, 12_000, 2_000, 1)
        ));
        assert!(!allows(
            Some(600),
            &quote(FiatCurrency::Mxn, 12_001, 2_000, 1)
        ));
        assert!(allows(
            Some(600),
            &quote(FiatCurrency::Mxn, 11_329, 9_441, 5)
        ));
        assert!(!allows(
            Some(600),
            &quote(FiatCurrency::Mxn, 11_330, 9_441, 5)
        ));
        assert!(allows(None, &quote(FiatCurrency::Mxn, 12_001, 2_000, 1)));
        assert!(!allows(
            Some(0),
            &quote(FiatCurrency::Mxn, 12_000, 2_000, 1)
        ));
    }

    #[test]
    fn usd_charge_uses_actual_cents_regardless_of_offered_usdc_rate() {
        assert!(allows(Some(600), &quote(FiatCurrency::Usd, 600, 100, 1)));
        assert!(!allows(Some(600), &quote(FiatCurrency::Usd, 601, 100, 1)));
        assert!(!allows(Some(600), &quote(FiatCurrency::Usd, 601, 1000, 1)));
    }
}
