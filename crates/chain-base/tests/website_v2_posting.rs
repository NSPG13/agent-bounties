//! The website posting page builds v2 terms and a v2 creation in the browser. This checks that
//! exact output, kept current by scripts/test-website-v2-posting-fixture.cjs, against the API's
//! terms publication, public-earning gate and v2 planner, and against the page's own encoder.

use chain_base::{
    build_autonomous_bounty_terms_record, validate_autonomous_v2_creation_for_public_earning,
    AutonomousBountyTxPlanner, AutonomousBountyV2Create, AutonomousV2FactoryFee,
};
use chrono::{DateTime, Utc};
use domain::AutonomousBountyTermsDocument;
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/website-v2-posting.json"))
        .expect("website v2 posting fixture")
}

fn fee(fixture: &Value) -> AutonomousV2FactoryFee {
    AutonomousV2FactoryFee {
        platform_fee_bps: u16::try_from(fixture["platform_fee_bps"].as_u64().unwrap()).unwrap(),
        platform_fee_recipient: fixture["platform_fee_recipient"]
            .as_str()
            .unwrap()
            .to_string(),
    }
}

#[test]
fn website_v2_terms_publish_and_its_creation_passes_public_earning() {
    let fixture = fixture();
    let document: AutonomousBountyTermsDocument =
        serde_json::from_value(fixture["publication"]["document"].clone()).unwrap();
    let created_at: DateTime<Utc> = fixture["published_at"].as_str().unwrap().parse().unwrap();
    let record = build_autonomous_bounty_terms_record(
        fixture["publication"]["creator_wallet"].as_str().unwrap(),
        document,
        created_at,
    )
    .expect("the API accepts the website's v2 terms document");
    // The page builds its creation from the published record's hashes.
    for key in [
        "terms_hash",
        "policy_hash",
        "acceptance_criteria_hash",
        "benchmark_hash",
        "evidence_schema_hash",
    ] {
        let published = serde_json::to_value(&record).unwrap()[key].clone();
        assert_eq!(published, fixture["terms"][key], "{key}");
    }
    let create: AutonomousBountyV2Create =
        serde_json::from_value(fixture["create"].clone()).unwrap();
    let platform_fee = validate_autonomous_v2_creation_for_public_earning(
        fixture["network"].as_str().unwrap(),
        &create,
        &record,
        &fee(&fixture),
    )
    .expect("the website's v2 creation passes the public-earning gate");
    assert_eq!(
        platform_fee.to_string(),
        fixture["expected"]["platform_fee"].as_str().unwrap()
    );
}

#[test]
fn website_v2_wallet_calls_match_the_api_planner() {
    let fixture = fixture();
    let create: AutonomousBountyV2Create =
        serde_json::from_value(fixture["create"].clone()).unwrap();
    let planner = AutonomousBountyTxPlanner::new(
        fixture["factory"].as_str().unwrap(),
        fixture["implementation"].as_str().unwrap(),
    )
    .unwrap();
    let plan = planner
        .plan_v2_creation(
            fixture["network"].as_str().unwrap(),
            &create,
            &fee(&fixture),
        )
        .unwrap();
    let expected = &fixture["expected"];
    assert_eq!(plan.plan.bounty_id, expected["bounty_id"].as_str().unwrap());
    assert_eq!(
        plan.plan.predicted_bounty_contract,
        expected["predicted_bounty_contract"].as_str().unwrap()
    );
    assert_eq!(
        plan.quote.target_amount,
        expected["target_amount"].as_str().unwrap()
    );
    let calls = expected["wallet_calls"].as_array().unwrap();
    assert_eq!(plan.plan.wallet_calls.len(), calls.len());
    for (planned, page) in plan.plan.wallet_calls.iter().zip(calls) {
        // The planner returns checksummed USDC; the page compares addresses case-insensitively.
        assert_eq!(
            planned.to.to_ascii_lowercase(),
            page["to"].as_str().unwrap()
        );
        assert_eq!(planned.data, page["data"].as_str().unwrap());
        assert_eq!(planned.value_wei, 0);
        assert_eq!(
            planned.from.as_deref(),
            fixture["create"]["creator"].as_str()
        );
    }
}
