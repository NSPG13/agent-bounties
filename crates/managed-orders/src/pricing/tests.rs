use super::*;

fn policy() -> PricingPolicy {
    PricingPolicy {
        conversion_buffer_bps: 100,
        service_tax_bps: None,
        processing_rate_bps: 410,
        processing_fixed_minor: 300,
        processing_fee_tax_bps: 1_600,
    }
}

#[test]
fn reference_price_becomes_exact_minor_units_per_usdc() {
    let basis = price_basis_from_decimal("18.700000000000", "bitso:usd_mxn:ask", 99).unwrap();
    assert_eq!(
        (
            basis.minor_units_per_usdc_numerator,
            basis.minor_units_per_usdc_denominator
        ),
        (1_870, 1)
    );
    let basis = price_basis_from_decimal("18.7123", "r", 99).unwrap();
    assert_eq!(
        (
            basis.minor_units_per_usdc_numerator,
            basis.minor_units_per_usdc_denominator
        ),
        (187_123, 100)
    );
    // A seventh nonzero decimal rounds the offered price up, never down.
    let basis = price_basis_from_decimal("18.7000001", "r", 99).unwrap();
    assert_eq!(
        (
            basis.minor_units_per_usdc_numerator,
            basis.minor_units_per_usdc_denominator
        ),
        (18_700_001, 10_000)
    );
    for invalid in [
        "",
        "0",
        "0.000",
        "-18.7",
        "18,7",
        "1e3",
        "18.7.1",
        "12345678.0",
        " 18.7",
    ] {
        assert!(
            price_basis_from_decimal(invalid, "r", 99).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn worked_mxn_example_discloses_buffer_and_covers_the_processor() {
    // 100 USDC reward + 5 USDC verification + 7.5 USDC fee at 18.70 MXN = MXN 2,103.75.
    let lines = priced_additional_costs(210_375, 0, &policy()).unwrap();
    assert_eq!(lines[0].code, "currency_conversion");
    assert_eq!(lines[0].label, "Currency conversion buffer (1.00%)");
    assert_eq!(lines[0].amount_minor, 2_104);
    assert_eq!(lines[1].code, "payment_processing");
    let subtotal = 210_375 + 2_104;
    let charge = subtotal + lines[1].amount_minor;
    assert!(u128::from(charge - subtotal) >= processing_cost(charge.into(), &policy()));
    assert!(
        u128::from(charge - 1 - subtotal) < processing_cost((charge - 1).into(), &policy()),
        "processing line must be the smallest that covers the fee"
    );
    assert_eq!(lines.len(), 2, "IVA stays off until configured");
}

#[test]
fn optional_service_tax_is_itemized_before_processing() {
    let mut taxed = policy();
    taxed.service_tax_bps = Some(1_600);
    let lines = priced_additional_costs(10_000, 0, &taxed).unwrap();
    let codes: Vec<_> = lines.iter().map(|line| line.code.as_str()).collect();
    assert_eq!(
        codes,
        ["currency_conversion", "service_tax", "payment_processing"]
    );
    assert_eq!(lines[0].amount_minor, 100);
    assert_eq!(lines[1].amount_minor, 1_616);
    assert_eq!(lines[1].label, "IVA (16.00%)");
}

#[test]
fn review_deposit_is_itemized_before_processing_and_covered_by_it() {
    let lines = priced_additional_costs(210_375, 18_700, &policy()).unwrap();
    let codes: Vec<_> = lines.iter().map(|line| line.code.as_str()).collect();
    assert_eq!(
        codes,
        [
            "currency_conversion",
            "review_deposit",
            "payment_processing"
        ]
    );
    assert_eq!(
        lines[0].amount_minor, 2_104,
        "the buffer excludes the deposit"
    );
    assert_eq!(lines[1].amount_minor, 18_700);
    let subtotal = 210_375 + 2_104 + 18_700;
    let charge = subtotal + lines[2].amount_minor;
    assert!(u128::from(charge - subtotal) >= processing_cost(charge.into(), &policy()));
}

#[test]
fn processing_gross_up_is_minimal_and_sufficient_across_amounts() {
    let p = policy();
    let mut subtotal = 1_000u128;
    while subtotal < 5_000_000_000 {
        let charge = gross_up_for_processing(subtotal, &p).unwrap();
        assert!(
            charge - subtotal >= processing_cost(charge, &p),
            "{subtotal}"
        );
        assert!(
            charge - 1 - subtotal < processing_cost(charge - 1, &p),
            "{subtotal}"
        );
        subtotal = subtotal * 7 / 3 + 13;
    }
}

#[test]
fn unsafe_policies_and_amounts_are_rejected() {
    let mut p = policy();
    p.processing_rate_bps = 5_000;
    assert!(priced_additional_costs(10_000, 0, &p).is_err());
    let mut p = policy();
    p.conversion_buffer_bps = 1_001;
    assert!(priced_additional_costs(10_000, 0, &p).is_err());
    let mut p = policy();
    p.service_tax_bps = Some(0);
    assert!(priced_additional_costs(10_000, 0, &p).is_err());
    assert!(priced_additional_costs(0, 0, &policy()).is_err());
    assert!(priced_additional_costs(MAX_EXACT_INTEGER, 0, &policy()).is_err());
}
