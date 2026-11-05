use super::*;
use chain_base::TEST_CUSTOMER_REVIEW_VERIFIER;
use managed_orders::stripe_checkout::CheckoutPolicy;

const SEPOLIA_V2_FACTORY: &str = "0x44de6074440fc006c20087991364ce65c87e6d88";
const SEPOLIA_V2_IMPLEMENTATION: &str = "0x9482786192e16ca4eb55f61787af664acd6b8e53";
const FEE_RECIPIENT: &str = "0x884834e884d6e93462655a2820140ad03e6747bc";
const FUNDING_WALLET: &str = "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";

fn config() -> ManagedBountyConfig {
    serde_json::from_value(json!({
        "network": "base-sepolia",
        "funding_wallet": FUNDING_WALLET,
        "customer_review": {
            "verifier": TEST_CUSTOMER_REVIEW_VERIFIER,
            "verifier_reward_micro_usdc": 500_000,
            "claim_window_seconds": 604_800,
            "verification_window_seconds": 604_800,
            "public_disclosure": "The paying customer reviews the delivery; the platform key signs only that recorded verdict."
        },
        "max_funding_window_seconds": 2_592_000,
        "quote_lifetime_seconds": 3_600,
        "minimum_open_seconds": 86_400,
        "terms_version": "managed-bounty-2026-10",
        "remedies": "Full refund before funding; after funding, refund of the unclaimed bounty on cancellation.",
        "pricing": {
            "conversion_buffer_bps": 100,
            "service_tax_bps": null,
            "processing_rate_bps": 410,
            "processing_fixed_minor": 300,
            "processing_fee_tax_bps": 1600
        },
        "rate_source": {"fixed": {"price": "18.70"}}
    }))
    .unwrap()
}

fn checkout_policy() -> CheckoutPolicy {
    CheckoutPolicy {
        stripe_account_id: "acct_card_bounty_fixture".into(),
        livemode: false,
        currency: FiatCurrency::Mxn,
        minimum_minor: 1_000,
        maximum_minor: 10_000_000,
        payment_method_configuration: None,
        adaptive_pricing: false,
    }
}

fn request(deadline: i64, reward: u64) -> BountyQuoteRequest {
    let document = json!({
        "title": "Redesign the pricing page",
        "goal": "Deliver a responsive pricing page matching the brief.",
        "acceptance_criteria": ["The page renders at 360px and 1440px", "All three plans show exact prices"],
        "benchmark": {"engine": CUSTOMER_REVIEW_ENGINE, "delivery_deadline": deadline},
        "evidence_schema": {
            "type": "object",
            "required": ["artifact_url", "artifact_sha256"],
            "properties": {
                "artifact_url": {"type": "string", "pattern": "^https://"},
                "artifact_sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}
            }
        },
        "source_url": null
    });
    BountyQuoteRequest {
        document: document.as_object().unwrap().clone(),
        solver_reward_micro_usdc: reward,
    }
}

fn price() -> ReferencePrice {
    ReferencePrice {
        price: "18.70".into(),
        reference: "fixed-rehearsal-rate".into(),
    }
}

fn prepared(
    req: BountyQuoteRequest,
    now: DateTime<Utc>,
) -> Result<PreparedBountyQuote, StatusCode> {
    let planner =
        AutonomousBountyTxPlanner::new(SEPOLIA_V2_FACTORY, SEPOLIA_V2_IMPLEMENTATION).unwrap();
    let fee = AutonomousV2FactoryFee {
        platform_fee_bps: 750,
        platform_fee_recipient: FEE_RECIPIENT.into(),
    };
    prepare(
        &config(),
        &format!("account:{}", "a".repeat(64)),
        &checkout_policy(),
        req,
        now,
        &price(),
        (&planner, &fee),
    )
}

#[test]
fn card_quote_binds_the_exact_funding_call_terms_and_predicted_bounty() {
    let now = Utc::now();
    let deadline = now.timestamp() + 7 * 86_400;
    let prepared = prepared(request(deadline, 100_000_000), now).unwrap();
    let quote = &prepared.quote;
    let input = quote.input();
    let bounty = &input.bounty;

    // The reward the creator entered is the solver reward; verification and fee are added.
    assert_eq!(input.solver_reward_micro_usdc, 100_000_000);
    assert_eq!(input.verifier_reserve_micro_usdc, 500_000);
    assert_eq!(quote.platform_fee_micro_usdc(), 7_500_000);
    assert_eq!(quote.bounty_target_micro_usdc(), 108_000_000);

    // Terms are public v2 terms created by the funding wallet with the pinned review signer.
    let terms = &prepared.terms;
    assert_eq!(terms.creator_wallet, FUNDING_WALLET);
    assert_eq!(bounty.terms_hash, terms.terms_hash.to_ascii_lowercase());
    assert_eq!(
        terms.document.contract_terms["initial_funding"]["amount"],
        108_000_000
    );
    assert_eq!(
        terms.document.verification_policy["verifiers"],
        json!([TEST_CUSTOMER_REVIEW_VERIFIER])
    );
    assert_eq!(bounty.creation_nonce, creation_nonce(input.quote_id));
    assert_eq!(bounty.funding_deadline, deadline);
    assert_eq!(bounty.network, "base-sepolia");
    assert_eq!(bounty.factory, SEPOLIA_V2_FACTORY);
    assert_eq!(bounty.funding_wallet, FUNDING_WALLET);

    // Rebuilding the call from the stored public terms reproduces the committed digest.
    let fee = AutonomousV2FactoryFee {
        platform_fee_bps: 750,
        platform_fee_recipient: FEE_RECIPIENT.into(),
    };
    let rebuilt = autonomous_v2_create_from_terms(terms, &fee).unwrap();
    let calldata = encode_bounded_v2_funding_wallet_create_call(&rebuilt).unwrap();
    assert_eq!(calldata, prepared.calldata);
    assert_eq!(
        call_keccak256(&calldata).unwrap(),
        bounty.create_call_keccak256
    );
    assert!(
        calldata.starts_with("0x261fef32"),
        "funding wallet createBounty selector"
    );

    // Itemized MXN: three USDC lines at 18.70, the 1% buffer, the refundable 10% review
    // deposit, and processing grossed up over all of it.
    let codes: Vec<_> = quote
        .lines()
        .iter()
        .map(|line| line.code.as_str())
        .collect();
    assert_eq!(
        codes,
        [
            "solver_reward",
            "verification",
            "platform_fee",
            "currency_conversion",
            "review_deposit",
            "payment_processing"
        ]
    );
    assert_eq!(quote.lines()[0].amount_minor, 187_000);
    assert_eq!(quote.lines()[3].amount_minor, 2_020);
    assert_eq!(
        quote.lines()[4].amount_minor,
        18_700,
        "10% of the 100 USDC solver reward at 18.70"
    );
    assert!(quote.digest().is_ok());
}

#[test]
fn equal_drafts_get_distinct_quotes_nonces_and_bounties() {
    let now = Utc::now();
    let deadline = now.timestamp() + 7 * 86_400;
    let first = prepared(request(deadline, 10_000_000), now).unwrap();
    let second = prepared(request(deadline, 10_000_000), now).unwrap();
    assert_ne!(first.quote.input().quote_id, second.quote.input().quote_id);
    assert_ne!(
        first.quote.input().bounty.creation_nonce,
        second.quote.input().bounty.creation_nonce
    );
    assert_ne!(
        first.quote.input().bounty.predicted_bounty,
        second.quote.input().bounty.predicted_bounty
    );
}

#[test]
fn drafts_cannot_supply_server_owned_fields_or_unsafe_deadlines() {
    let now = Utc::now();
    let deadline = now.timestamp() + 7 * 86_400;
    for key in [
        "contract_terms",
        "verification_policy",
        "schema_version",
        "discovery_source",
    ] {
        let mut req = request(deadline, 10_000_000);
        req.document.insert(key.into(), json!({}));
        assert_eq!(
            prepared(req, now).err(),
            Some(StatusCode::BAD_REQUEST),
            "{key}"
        );
    }
    let mut regression = request(deadline, 10_000_000);
    regression.document.insert(
        "benchmark".into(),
        json!({"engine": "sandboxed_regression_v1", "delivery_deadline": deadline}),
    );
    assert_eq!(
        prepared(regression, now).err(),
        Some(StatusCode::BAD_REQUEST)
    );
    // Too soon to fund and open, or beyond the configured funding window.
    for bad in [now.timestamp() + 3_600, now.timestamp() + 31 * 86_400] {
        assert_eq!(
            prepared(request(bad, 10_000_000), now).err(),
            Some(StatusCode::BAD_REQUEST)
        );
    }
    assert_eq!(
        prepared(request(deadline, 0), now).err(),
        Some(StatusCode::BAD_REQUEST)
    );
}

#[test]
fn a_factory_with_another_fee_cannot_receive_card_quotes() {
    let now = Utc::now();
    let planner =
        AutonomousBountyTxPlanner::new(SEPOLIA_V2_FACTORY, SEPOLIA_V2_IMPLEMENTATION).unwrap();
    let fee = AutonomousV2FactoryFee {
        platform_fee_bps: 500,
        platform_fee_recipient: FEE_RECIPIENT.into(),
    };
    let result = prepare(
        &config(),
        &format!("account:{}", "a".repeat(64)),
        &checkout_policy(),
        request(now.timestamp() + 7 * 86_400, 10_000_000),
        now,
        &price(),
        (&planner, &fee),
    );
    assert_eq!(result.err(), Some(StatusCode::SERVICE_UNAVAILABLE));
}

#[test]
fn config_rejects_fixed_mainnet_rates_and_signer_reuse() {
    let base = |mutate: &dyn Fn(&mut Value)| {
        let mut raw = json!({
            "network": "base-mainnet", "funding_wallet": FUNDING_WALLET,
            "customer_review": {"verifier": TEST_CUSTOMER_REVIEW_VERIFIER, "verifier_reward_micro_usdc": 500_000,
                "claim_window_seconds": 604_800, "verification_window_seconds": 604_800, "public_disclosure": "x"},
            "max_funding_window_seconds": 2_592_000, "quote_lifetime_seconds": 3_600, "minimum_open_seconds": 86_400,
            "terms_version": "v1", "remedies": "r",
            "pricing": {"conversion_buffer_bps": 100, "service_tax_bps": null, "processing_rate_bps": 410,
                "processing_fixed_minor": 300, "processing_fee_tax_bps": 1600},
            "rate_source": "bitso_usd_mxn_ask"
        });
        mutate(&mut raw);
        serde_json::from_value::<ManagedBountyConfig>(raw)
            .unwrap()
            .validate()
    };
    assert!(base(&|_| {}).is_ok());
    assert!(base(&|v| v["rate_source"] = json!({"fixed": {"price": "18.7"}})).is_err());
    assert!(base(&|v| v["customer_review"]["verifier"] = json!(FUNDING_WALLET)).is_err());
    assert!(base(&|v| v["quote_lifetime_seconds"] = json!(600)).is_err());
    assert!(base(&|v| v["pricing"]["processing_rate_bps"] = json!(5000)).is_err());
}

#[test]
fn bitso_ticker_must_be_fresh_successful_usd_mxn() {
    let now = DateTime::parse_from_rfc3339("2026-10-22T19:53:00+00:00")
        .unwrap()
        .with_timezone(&Utc);
    let body = |success: bool, book: &str, created: &str, ask: &str| {
        serde_json::to_vec(&json!({"success": success, "payload": {"book": book, "created_at": created, "ask": ask, "bid": "18.69"}})).unwrap()
    };
    let ok = parse_bitso_ticker(
        &body(
            true,
            "usd_mxn",
            "2026-10-22T19:52:04+00:00",
            "18.700000000000",
        ),
        now,
    )
    .unwrap();
    assert_eq!(ok.price, "18.700000000000");
    assert!(ok.reference.starts_with("bitso:usd_mxn:ask:"));
    assert!(parse_bitso_ticker(
        &body(false, "usd_mxn", "2026-10-22T19:52:04+00:00", "18.7"),
        now
    )
    .is_none());
    assert!(parse_bitso_ticker(
        &body(true, "usdt_mxn", "2026-10-22T19:52:04+00:00", "18.7"),
        now
    )
    .is_none());
    assert!(parse_bitso_ticker(
        &body(true, "usd_mxn", "2026-10-22T19:40:00+00:00", "18.7"),
        now
    )
    .is_none());
    assert!(parse_bitso_ticker(
        &body(true, "usd_mxn", "2026-10-22T19:52:04+00:00", "-18.7"),
        now
    )
    .is_none());
    assert!(parse_bitso_ticker(b"not json", now).is_none());
}

#[test]
fn long_public_terms_are_referenced_by_hash_in_the_quote() {
    let now = Utc::now();
    let mut req = request(now.timestamp() + 7 * 86_400, 10_000_000);
    req.document
        .insert("goal".into(), json!("detail ".repeat(5_000).trim_end()));
    let prepared = prepared(req, now).unwrap();
    let deliverable = &prepared.quote.input().terms.deliverable;
    assert!(deliverable.starts_with("Redesign the pricing page\n\nThe complete text"));
    assert!(deliverable.ends_with(&format!("{}.", prepared.terms.terms_hash)));
    assert_eq!(prepared.terms.document.goal.len(), 34_999);
}

fn caps_words(valid_until: u64, per_bounty: u128, lifetime: alloy::primitives::U256) -> String {
    let mut words = vec![alloy::primitives::U256::ZERO; 13];
    words[2] = alloy::primitives::U256::from(valid_until);
    words[8] = alloy::primitives::U256::from(per_bounty);
    words[9] = alloy::primitives::U256::from(per_bounty);
    words[10] = lifetime;
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes::<32>()).collect();
    format!("0x{}", hex::encode(bytes))
}

fn word(value: u64) -> String {
    format!("0x{:064x}", value)
}

#[test]
fn card_quotes_follow_the_funding_wallets_live_caps() {
    assert_eq!(selector("policy()"), "0x0505c8c9");
    assert_eq!(selector("revoked()"), "0x63d256ce");
    assert_eq!(selector("lifetimeSpent()"), "0xb80762dd");
    let no_lifetime_cap = alloy::primitives::U256::MAX;
    let caps = WalletCaps::decode(
        &caps_words(2_000_000_000, 200_000_000, no_lifetime_cap),
        &word(0),
        &word(5_000_000_000),
    )
    .unwrap();
    assert_eq!(caps.max_per_bounty, 200_000_000);
    assert_eq!(caps.lifetime_room, u128::MAX);
    let expires = 1_900_000_000;
    assert_eq!(caps.admit(200_000_000, expires), Ok(()));
    assert_eq!(
        caps.admit(200_000_001, expires),
        Err(StatusCode::UNPROCESSABLE_ENTITY)
    );
    // A wallet whose policy ends before the quote does cannot fund it.
    assert_eq!(
        caps.admit(1, 2_000_000_000),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );

    let revoked = WalletCaps::decode(
        &caps_words(2_000_000_000, 200_000_000, no_lifetime_cap),
        &word(1),
        &word(0),
    )
    .unwrap();
    assert_eq!(
        revoked.admit(1, expires),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    );

    // A finite lifetime cap admits only what is left of it.
    let limited = alloy::primitives::U256::from(1_000_000_000u64);
    let nearly_spent = WalletCaps::decode(
        &caps_words(2_000_000_000, 200_000_000, limited),
        &word(0),
        &word(900_000_000),
    )
    .unwrap();
    assert_eq!(nearly_spent.admit(100_000_000, expires), Ok(()));
    assert_eq!(
        nearly_spent.admit(100_000_001, expires),
        Err(StatusCode::UNPROCESSABLE_ENTITY)
    );

    // Malformed reads are unavailable, never permissive.
    let good = caps_words(2_000_000_000, 200_000_000, no_lifetime_cap);
    for (policy, revoked, spent) in [
        ("0x".to_string(), word(0), word(0)),
        (good[..good.len() - 64].to_string(), word(0), word(0)),
        (good.clone(), word(2), word(0)),
        (good.clone(), word(0), "0x1234".to_string()),
        (
            caps_words(2_000_000_000, 200_000_000, limited),
            word(0),
            word(1_000_000_001),
        ),
    ] {
        assert_eq!(WalletCaps::decode(&policy, &revoked, &spent), None);
    }
}
