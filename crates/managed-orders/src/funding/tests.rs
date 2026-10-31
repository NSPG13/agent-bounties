use super::*;
use crate::{BountyCommitment, FiatCurrency, PriceBasis, QuoteInput, ServiceTerms};
use chain_base::{
    build_autonomous_bounty_terms_record, AutonomousBountyTxPlanner,
    AUTONOMOUS_V2_PROTOCOL_VERSION, TEST_CUSTOMER_REVIEW_VERIFIER,
};
use chrono::Utc;
use serde_json::json;

const FACTORY: &str = "0x44de6074440fc006c20087991364ce65c87e6d88";
const IMPLEMENTATION: &str = "0x9482786192e16ca4eb55f61787af664acd6b8e53";
const FEE_RECIPIENT: &str = "0x884834e884d6e93462655a2820140ad03e6747bc";
const WALLET: &str = "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";

fn fee() -> AutonomousV2FactoryFee {
    AutonomousV2FactoryFee {
        platform_fee_bps: 750,
        platform_fee_recipient: FEE_RECIPIENT.into(),
    }
}

fn target() -> FundingTarget {
    FundingTarget {
        network: "base-sepolia".into(),
        factory: FACTORY.into(),
        funding_wallet: WALLET.into(),
        fee: fee(),
    }
}

/// Terms and an accepted quote built the way the card-quote route builds them.
fn fixture() -> (ManagedQuote, AutonomousBountyTermsRecord) {
    let now = Utc::now();
    let deadline = now.timestamp() + 7 * 86_400;
    let nonce = format!("0x{}", "5a".repeat(32));
    let usdc = |amount: u64| json!({"amount": amount, "currency": "usdc"});
    let document = json!({
        "schema_version": "agent-bounties/terms-v1",
        "title": "Fixture task", "goal": "Deliver the fixture.",
        "acceptance_criteria": ["It is delivered"],
        "benchmark": {"engine": "customer_review_v1", "reviewer": "paying_customer",
            "acceptance": "all_published_criteria", "delivery_deadline": deadline},
        "verification_policy": {"engine": "customer_review_v1", "mechanism": "signed_quorum", "threshold": 1,
            "verifiers": [TEST_CUSTOMER_REVIEW_VERIFIER], "public_disclosure": "Customer review."},
        "evidence_schema": {"type": "object", "required": ["artifact_url", "artifact_sha256"], "properties": {
            "artifact_url": {"type": "string", "pattern": "^https://"},
            "artifact_sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}}},
        "source_url": null, "discovery_source": "managed_card_quote",
        "contract_terms": {
            "protocol_version": AUTONOMOUS_V2_PROTOCOL_VERSION, "network": "base-sepolia",
            "settlement_token": chain_base::base_network_descriptor("base-sepolia").unwrap().native_usdc_token_address,
            "creator_wallet": WALLET, "solver_reward": usdc(10_000_000), "verifier_reward": usdc(500_000),
            "claim_bond": usdc(500_000), "initial_funding": usdc(11_250_000), "platform_fee_bps": 750,
            "platform_fee": usdc(750_000), "platform_fee_recipient": FEE_RECIPIENT, "funding_deadline": deadline,
            "claim_window_seconds": 604_800, "verification_window_seconds": 259_200, "creation_nonce": nonce
        }
    });
    let terms = build_autonomous_bounty_terms_record(
        WALLET,
        serde_json::from_value(document).unwrap(),
        now,
    )
    .unwrap();
    let create = autonomous_v2_create_from_terms(&terms, &fee()).unwrap();
    let plan = AutonomousBountyTxPlanner::new(FACTORY, IMPLEMENTATION)
        .unwrap()
        .plan_v2_creation("base-sepolia", &create, &fee())
        .unwrap();
    let calldata = encode_bounded_v2_funding_wallet_create_call(&create).unwrap();
    let issued_at = now.timestamp();
    let quote = ManagedQuote::new(QuoteInput {
        quote_id: Uuid::new_v4(),
        buyer_principal: format!("account:{}", "a".repeat(64)),
        stripe_account_id: "acct_funding_fixture".into(),
        livemode: false,
        issued_at,
        expires_at: issued_at + 3_600,
        terms: ServiceTerms {
            deliverable: "Fixture task".into(),
            acceptance_criteria: "- It is delivered".into(),
            delivery_deadline: deadline,
            terms_version: "fixture".into(),
            remedies: "Fixture remedies".into(),
        },
        currency: FiatCurrency::Mxn,
        price_basis: PriceBasis {
            reference: "fixture".into(),
            minor_units_per_usdc_numerator: 1_870,
            minor_units_per_usdc_denominator: 1,
            valid_until: issued_at + 3_600,
        },
        solver_reward_micro_usdc: 10_000_000,
        verifier_reserve_micro_usdc: 500_000,
        additional_costs: vec![],
        bounty: BountyCommitment {
            network: "base-sepolia".into(),
            factory: FACTORY.into(),
            funding_wallet: WALLET.into(),
            creation_nonce: format!("0x{}", "5a".repeat(32)),
            terms_hash: terms.terms_hash.to_ascii_lowercase(),
            verification_setup: "customer_review_v1".into(),
            funding_deadline: deadline,
            predicted_bounty: plan.plan.predicted_bounty_contract.to_ascii_lowercase(),
            create_call_keccak256: calldata_keccak256(&calldata).unwrap(),
        },
    })
    .unwrap();
    (quote, terms)
}

fn event(
    kind: AutonomousBountyEventKind,
    contract: &str,
    tx: &str,
    data: serde_json::Value,
) -> AutonomousBountyEvent {
    AutonomousBountyEvent {
        id: Uuid::new_v4(),
        log_key: Uuid::new_v4().to_string(),
        tx_hash: tx.into(),
        block_number: 100,
        log_index: 1,
        contract_address: contract.into(),
        bounty_id: "0xbounty".into(),
        kind,
        data,
        occurred_at: Utc::now(),
    }
}

fn canonical_events(plan: &FundingPlan, amount: u64) -> Vec<AutonomousBountyEvent> {
    vec![
        event(
            AutonomousBountyEventKind::CanonicalBountyCreated,
            &plan.factory,
            "0xtx",
            json!({"bounty_contract": plan.predicted_bounty, "creator": plan.funding_wallet,
                   "creation_nonce": plan.creation_nonce}),
        ),
        event(
            AutonomousBountyEventKind::FundingAdded,
            &plan.predicted_bounty,
            "0xtx",
            json!({"contributor": plan.funding_wallet, "amount": amount, "funded_amount": amount,
                   "target_amount": plan.target_micro_usdc}),
        ),
    ]
}

#[test]
fn plan_rebuilds_exactly_the_accepted_call_from_public_terms() {
    let (quote, terms) = fixture();
    let plan = FundingPlan::new(Uuid::new_v4(), &quote, &terms, &target()).unwrap();
    assert_eq!(plan.target_micro_usdc, 11_250_000);
    assert!(plan.calldata.starts_with("0x261fef32"));
    assert_eq!(
        plan.create_call_keccak256,
        quote.input().bounty.create_call_keccak256
    );
    assert_eq!(plan.predicted_bounty, quote.input().bounty.predicted_bounty);
}

#[test]
fn any_divergence_between_quote_terms_and_fee_is_refused() {
    let (quote, terms) = fixture();
    let mut other_terms = terms.clone();
    other_terms.terms_hash = format!("0x{}", "00".repeat(31) + "01");
    assert_eq!(
        FundingPlan::new(Uuid::new_v4(), &quote, &other_terms, &target()),
        Err(FundingError::Mismatch)
    );
    let mut other_creator = terms.clone();
    other_creator.creator_wallet = "0x1111111111111111111111111111111111111111".into();
    assert_eq!(
        FundingPlan::new(Uuid::new_v4(), &quote, &other_creator, &target()),
        Err(FundingError::Mismatch)
    );
    let mut other_fee = target();
    other_fee.fee.platform_fee_bps = 500;
    assert!(FundingPlan::new(Uuid::new_v4(), &quote, &terms, &other_fee).is_err());
    for change in [
        |t: &mut FundingTarget| t.network = "base-mainnet".into(),
        |t: &mut FundingTarget| t.factory = FEE_RECIPIENT.into(),
        |t: &mut FundingTarget| t.funding_wallet = FEE_RECIPIENT.into(),
    ] {
        let mut other = target();
        change(&mut other);
        assert_eq!(
            FundingPlan::new(Uuid::new_v4(), &quote, &terms, &other),
            Err(FundingError::OtherTarget)
        );
    }

    let mut input = quote.input().clone();
    input.bounty.create_call_keccak256 = format!("0x{}", "cb".repeat(32));
    let swapped = ManagedQuote::new(input).unwrap();
    assert_eq!(
        FundingPlan::new(Uuid::new_v4(), &swapped, &terms, &target()),
        Err(FundingError::Mismatch)
    );
    let mut input = quote.input().clone();
    input.solver_reward_micro_usdc += 1;
    let repriced = ManagedQuote::new(input).unwrap();
    assert_eq!(
        FundingPlan::new(Uuid::new_v4(), &repriced, &terms, &target()),
        Err(FundingError::Mismatch)
    );
}

#[test]
fn only_the_exact_canonical_creation_and_full_funding_count() {
    let (quote, terms) = fixture();
    let plan = FundingPlan::new(Uuid::new_v4(), &quote, &terms, &target()).unwrap();
    assert_eq!(observe_funding(&plan, &[]), FundingObservation::NotObserved);
    assert_eq!(
        observe_funding(&plan, &canonical_events(&plan, plan.target_micro_usdc)),
        FundingObservation::Funded {
            tx_hash: "0xtx".into(),
            block_number: 100
        }
    );
    // Creation alone is not funding.
    assert_eq!(
        observe_funding(&plan, &canonical_events(&plan, plan.target_micro_usdc)[..1]),
        FundingObservation::NotObserved
    );
    // Partial funding, another creator, another nonce or another factory never prove funding.
    let partial = canonical_events(&plan, plan.target_micro_usdc - 1);
    assert_eq!(
        observe_funding(&plan, &partial),
        FundingObservation::Conflict
    );
    let mut other_creator = canonical_events(&plan, plan.target_micro_usdc);
    other_creator[0].data["creator"] = json!("0x1111111111111111111111111111111111111111");
    assert_eq!(
        observe_funding(&plan, &other_creator),
        FundingObservation::Conflict
    );
    let mut other_nonce = canonical_events(&plan, plan.target_micro_usdc);
    other_nonce[0].data["creation_nonce"] = json!(format!("0x{}", "00".repeat(32)));
    assert_eq!(
        observe_funding(&plan, &other_nonce),
        FundingObservation::Conflict
    );
    let mut other_factory = canonical_events(&plan, plan.target_micro_usdc);
    other_factory[0].contract_address = "0x2222222222222222222222222222222222222222".into();
    assert_eq!(
        observe_funding(&plan, &other_factory),
        FundingObservation::NotObserved
    );
    // Funding from a later, separate transaction is not the committed creation.
    let mut later = canonical_events(&plan, plan.target_micro_usdc);
    later[1].tx_hash = "0xother".into();
    assert_eq!(observe_funding(&plan, &later), FundingObservation::Conflict);
}
