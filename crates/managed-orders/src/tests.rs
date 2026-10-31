use super::*;
use proptest::prelude::*;

type InputMutation = Box<dyn Fn(&mut QuoteInput)>;

#[test]
fn browser_quote_fixture_matches_rust_price_and_digest() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let input: QuoteInput = serde_json::from_value(fixture["quote"]["input"].clone()).unwrap();
    let quote = ManagedQuote::new(input).unwrap();
    assert_eq!(serde_json::to_value(&quote).unwrap(), fixture["quote"]);
    assert_eq!(quote.digest().unwrap(), fixture["quote_digest"]);
}

fn input() -> QuoteInput {
    QuoteInput {
        quote_id: Uuid::new_v4(),
        buyer_principal: "buyer-1".into(),
        stripe_account_id: "acct_test_fixture".into(),
        livemode: false,
        issued_at: 1_800_000_000,
        expires_at: 1_800_000_600,
        terms: ServiceTerms {
            deliverable: "Implement the accepted specification".into(),
            acceptance_criteria: "All specified regression cases pass".into(),
            delivery_deadline: 1_800_100_000,
            terms_version: "managed-1".into(),
            remedies: "Cancellation and redelivery terms accepted with this offer".into(),
        },
        currency: FiatCurrency::Usd,
        price_basis: PriceBasis {
            reference: "fixture-usd-offer".into(),
            minor_units_per_usdc_numerator: 100,
            minor_units_per_usdc_denominator: 1,
            valid_until: 1_800_000_600,
        },
        solver_reward_micro_usdc: 100_000_000,
        verifier_reserve_micro_usdc: 5_000_000,
        additional_costs: vec![],
        bounty: managed_orders_bounty(1_800_100_000),
    }
}

#[test]
fn creator_reward_is_preserved_with_separate_verification_and_seven_point_five_percent_fee() {
    let quote = ManagedQuote::new(input()).unwrap();
    assert_eq!(quote.input().solver_reward_micro_usdc, 100_000_000);
    assert_eq!(quote.platform_fee_micro_usdc(), 7_500_000);
    assert_eq!(quote.bounty_target_micro_usdc(), 112_500_000);
    assert_eq!(
        quote
            .lines()
            .iter()
            .map(|l| (l.code.as_str(), l.amount_minor))
            .collect::<Vec<_>>(),
        vec![
            ("solver_reward", 10_000),
            ("verification", 500),
            ("platform_fee", 750)
        ]
    );
    assert_eq!(quote.total_minor(), 11_250);
}

#[test]
fn mxn_offer_keeps_exact_usdc_target_and_discloses_additional_costs() {
    let mut request = input();
    request.currency = FiatCurrency::Mxn;
    // A synthetic test price, not a current exchange rate or guaranteed conversion.
    request.price_basis.minor_units_per_usdc_numerator = 2_000;
    request.additional_costs.push(AdditionalCost {
        code: "processing".into(),
        label: "Payment processing".into(),
        amount_minor: 250,
    });
    let quote = ManagedQuote::new(request).unwrap();
    assert_eq!(quote.total_minor(), 225_250);
    assert_eq!(quote.platform_fee_micro_usdc(), 7_500_000);
    assert_eq!(quote.bounty_target_micro_usdc(), 112_500_000);
    assert_eq!(quote.lines()[3].amount_micro_usdc, None);
    assert_eq!(quote.lines()[0].amount_minor, 200_000);
}

#[test]
fn smallest_units_round_up_without_a_fee_on_the_verifier_or_extras() {
    let mut request = input();
    request.solver_reward_micro_usdc = 1;
    request.verifier_reserve_micro_usdc = 1_000_000;
    let quote = ManagedQuote::new(request).unwrap();
    assert_eq!(quote.platform_fee_micro_usdc(), 1);
    assert_eq!(quote.bounty_target_micro_usdc(), 1_000_002);
    assert_eq!(quote.total_minor(), 102);
}

#[test]
fn fractional_fx_is_rounded_per_visible_line() {
    let mut request = input();
    request.price_basis.minor_units_per_usdc_numerator = 101;
    request.price_basis.minor_units_per_usdc_denominator = 3;
    let quote = ManagedQuote::new(request).unwrap();
    assert_eq!(
        quote
            .lines()
            .iter()
            .map(|l| l.amount_minor)
            .collect::<Vec<_>>(),
        vec![3367, 169, 253]
    );
    assert_eq!(quote.total_minor(), 3789);
}

#[test]
fn expired_future_wrong_buyer_and_changed_digest_cannot_be_accepted() {
    let quote = ManagedQuote::new(input()).unwrap();
    let digest = quote.digest().unwrap();
    assert!(quote
        .validate_acceptance("buyer-1", &digest, 1_800_000_000)
        .is_ok());
    assert_eq!(
        quote.validate_acceptance("buyer-1", &digest, 1_800_000_600),
        Err(OrderError::Expired)
    );
    assert_eq!(
        quote.validate_acceptance("buyer-1", &digest, 1_799_999_999),
        Err(OrderError::Expired)
    );
    assert_eq!(
        quote.validate_acceptance("buyer-2", &digest, 1_800_000_001),
        Err(OrderError::Conflict)
    );
    assert_eq!(
        quote.validate_acceptance("buyer-1", &"0".repeat(64), 1_800_000_001),
        Err(OrderError::Conflict)
    );
}

#[test]
fn every_customer_and_economic_commitment_changes_the_digest() {
    let original = input();
    let digest = ManagedQuote::new(original.clone())
        .unwrap()
        .digest()
        .unwrap();
    let mutations: Vec<InputMutation> = vec![
        Box::new(|x| x.quote_id = Uuid::new_v4()),
        Box::new(|x| x.buyer_principal = "other-buyer".into()),
        Box::new(|x| x.stripe_account_id = "acct_other".into()),
        Box::new(|x| {
            x.livemode = true;
            x.bounty.network = "base-mainnet".into();
        }),
        Box::new(|x| x.bounty.factory = format!("0x{}", "c4".repeat(20))),
        Box::new(|x| x.bounty.funding_wallet = format!("0x{}", "f1".repeat(20))),
        Box::new(|x| x.bounty.creation_nonce = format!("0x{}", "67".repeat(32))),
        Box::new(|x| x.bounty.terms_hash = format!("0x{}", "12".repeat(32))),
        Box::new(|x| x.bounty.verification_setup = "sandboxed_regression_v1".into()),
        Box::new(|x| x.bounty.funding_deadline -= 1),
        Box::new(|x| x.bounty.predicted_bounty = format!("0x{}", "b1".repeat(20))),
        Box::new(|x| x.bounty.create_call_keccak256 = format!("0x{}", "cb".repeat(32))),
        Box::new(|x| x.issued_at += 1),
        Box::new(|x| x.expires_at -= 1),
        Box::new(|x| x.terms.deliverable.push_str(" with extra work")),
        Box::new(|x| x.terms.acceptance_criteria.push_str(" and review")),
        Box::new(|x| x.terms.delivery_deadline += 1),
        Box::new(|x| x.terms.terms_version = "managed-2".into()),
        Box::new(|x| x.terms.remedies.push_str(" with different remedies")),
        Box::new(|x| x.currency = FiatCurrency::Mxn),
        Box::new(|x| x.price_basis.reference.push_str("-new")),
        Box::new(|x| x.price_basis.minor_units_per_usdc_numerator += 1),
        Box::new(|x| x.price_basis.minor_units_per_usdc_denominator += 1),
        Box::new(|x| x.price_basis.valid_until += 1),
        Box::new(|x| x.solver_reward_micro_usdc += 1),
        Box::new(|x| x.verifier_reserve_micro_usdc += 1),
        Box::new(|x| {
            x.additional_costs.push(AdditionalCost {
                code: "other".into(),
                label: "Disclosed cost".into(),
                amount_minor: 1,
            })
        }),
    ];
    for mutate in mutations {
        let mut changed = original.clone();
        mutate(&mut changed);
        assert_ne!(
            ManagedQuote::new(changed).unwrap().digest().unwrap(),
            digest
        );
    }
}

#[test]
fn serialized_derived_amounts_are_revalidated_and_unknown_fields_rejected() {
    let quote = ManagedQuote::new(input()).unwrap();
    let serialized = serde_json::to_value(&quote).unwrap();
    for key in [
        "total_minor",
        "bounty_target_micro_usdc",
        "platform_fee_micro_usdc",
        "platform_fee_bps",
    ] {
        let mut altered = serialized.clone();
        altered[key] = 1.into();
        assert!(serde_json::from_value::<ManagedQuote>(altered).is_err());
    }
    let mut changed_line = serialized.clone();
    changed_line["lines"][0]["amount_minor"] = 1.into();
    assert!(serde_json::from_value::<ManagedQuote>(changed_line).is_err());
    let mut unknown = serialized.clone();
    unknown["paid"] = true.into();
    assert!(serde_json::from_value::<ManagedQuote>(unknown).is_err());
    let mut unknown_currency = serialized;
    unknown_currency["input"]["currency"] = "jpy".into();
    assert!(serde_json::from_value::<ManagedQuote>(unknown_currency).is_err());
    assert_eq!(
        serde_json::from_slice::<ManagedQuote>(&serde_json::to_vec(&quote).unwrap())
            .unwrap()
            .digest()
            .unwrap(),
        quote.digest().unwrap()
    );
}

#[test]
fn invalid_prices_oversized_amounts_and_duplicate_reserved_costs_are_rejected() {
    let mutations: Vec<InputMutation> = vec![
        Box::new(|x| x.solver_reward_micro_usdc = 0),
        Box::new(|x| x.verifier_reserve_micro_usdc = 0),
        Box::new(|x| x.solver_reward_micro_usdc = u64::MAX),
        Box::new(|x| x.verifier_reserve_micro_usdc = MAX_EXACT_INTEGER),
        Box::new(|x| x.price_basis.minor_units_per_usdc_denominator = 0),
        Box::new(|x| x.price_basis.minor_units_per_usdc_numerator = 0),
        Box::new(|x| x.price_basis.minor_units_per_usdc_numerator = MAX_EXACT_INTEGER),
        Box::new(|x| x.price_basis.valid_until -= 1),
        Box::new(|x| x.terms.delivery_deadline = x.expires_at),
        Box::new(|x| x.terms.deliverable = " ".into()),
        Box::new(|x| x.buyer_principal = "buyer\n1".into()),
        Box::new(|x| x.quote_id = Uuid::nil()),
        Box::new(|x| {
            x.additional_costs.push(AdditionalCost {
                code: "platform_fee".into(),
                label: "Second fee".into(),
                amount_minor: 1,
            })
        }),
        Box::new(|x| {
            x.additional_costs = vec![
                AdditionalCost {
                    code: "processing".into(),
                    label: "Processing".into(),
                    amount_minor: 1
                };
                2
            ]
        }),
        Box::new(|x| {
            x.additional_costs.push(AdditionalCost {
                code: "processing".into(),
                label: "Processing".into(),
                amount_minor: MAX_EXACT_INTEGER,
            })
        }),
    ];
    for mutate in mutations {
        let mut changed = input();
        mutate(&mut changed);
        assert!(ManagedQuote::new(changed).is_err());
    }
}

proptest! {
    #[test]
    fn fee_target_and_visible_total_hold_across_rewards(reward in 1_u64..1_000_000_000_000, verifier in 1_u64..1_000_000_000) {
        let mut request = input();
        request.solver_reward_micro_usdc = reward; request.verifier_reserve_micro_usdc = verifier;
        let quote = ManagedQuote::new(request).unwrap();
        // Independent integer formula; this assertion must not call the production fee helper.
        let expected_fee = (reward * 750).div_ceil(10_000);
        prop_assert_eq!(quote.platform_fee_micro_usdc(), expected_fee);
        prop_assert_eq!(quote.bounty_target_micro_usdc(), reward + verifier + expected_fee);
        prop_assert_eq!(quote.lines()[0].amount_micro_usdc, Some(reward));
        prop_assert_eq!(quote.total_minor(), quote.lines().iter().map(|l| l.amount_minor).sum::<u64>());
    }
}

fn managed_orders_bounty(funding_deadline: i64) -> crate::BountyCommitment {
    crate::BountyCommitment::test_fixture(false, funding_deadline)
}

#[test]
fn bounty_commitment_binds_network_to_mode_and_rejects_malformed_or_late_fields() {
    let valid = input();
    ManagedQuote::new(valid.clone()).unwrap();
    let mutations: Vec<InputMutation> = vec![
        Box::new(|x| x.livemode = true),
        Box::new(|x| x.bounty.network = "base-mainnet".into()),
        Box::new(|x| x.bounty.factory = format!("0x{}", "C3".repeat(20))),
        Box::new(|x| x.bounty.factory = format!("0x{}", "00".repeat(20))),
        Box::new(|x| x.bounty.funding_wallet = x.bounty.factory.clone()),
        Box::new(|x| x.bounty.predicted_bounty = x.bounty.funding_wallet.clone()),
        Box::new(|x| x.bounty.creation_nonce = format!("0x{}", "00".repeat(32))),
        Box::new(|x| x.bounty.terms_hash = "0x11".into()),
        Box::new(|x| x.bounty.create_call_keccak256 = format!("{}0", "ca".repeat(32))),
        Box::new(|x| x.bounty.verification_setup = "bad setup".into()),
        Box::new(|x| x.bounty.funding_deadline = x.expires_at),
        Box::new(|x| x.bounty.funding_deadline = x.terms.delivery_deadline + 1),
    ];
    for mutate in mutations {
        let mut changed = valid.clone();
        mutate(&mut changed);
        assert!(ManagedQuote::new(changed).is_err());
    }
}
