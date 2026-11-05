use super::*;
use serde_json::{json, Value};
fn fixture() -> (ManagedQuote, FundingPlan, Vec<AutonomousBountyEvent>) {
    let v: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let q: ManagedQuote = serde_json::from_value(v["quote"].clone()).unwrap();
    let b = &q.input().bounty;
    let p = FundingPlan {
        order_id: uuid::Uuid::new_v4(),
        network: b.network.clone(),
        factory: b.factory.clone(),
        funding_wallet: b.funding_wallet.clone(),
        predicted_bounty: b.predicted_bounty.clone(),
        creation_nonce: b.creation_nonce.clone(),
        funding_deadline: b.funding_deadline,
        target_micro_usdc: q.bounty_target_micro_usdc(),
        calldata: "0x1234".into(),
        create_call_keccak256: b.create_call_keccak256.clone(),
    };
    let specs = [
        (
            Kind::CanonicalBountyCreated,
            p.factory.clone(),
            100,
            0,
            json!({"bounty_contract":p.predicted_bounty,"creator":p.funding_wallet,"creation_nonce":p.creation_nonce}),
        ),
        (
            Kind::FundingAdded,
            p.predicted_bounty.clone(),
            100,
            1,
            json!({"contributor":p.funding_wallet,"amount":p.target_micro_usdc,"funded_amount":p.target_micro_usdc,"target_amount":p.target_micro_usdc}),
        ),
        (
            Kind::BountyCancelled,
            p.predicted_bounty.clone(),
            200,
            0,
            json!({"timeout_bond_refund_pool":5}),
        ),
        (
            Kind::RefundWithdrawn,
            p.predicted_bounty.clone(),
            200,
            2,
            json!({"contributor":p.funding_wallet,"principal":p.target_micro_usdc,"timeout_bond_bonus":5,"amount":p.target_micro_usdc+5}),
        ),
    ];
    let events = specs
        .into_iter()
        .map(
            |(kind, contract_address, block_number, log_index, data)| AutonomousBountyEvent {
                id: uuid::Uuid::new_v4(),
                log_key: format!("{block_number}:{log_index}"),
                tx_hash: format!("0x{block_number:064x}"),
                block_number,
                log_index,
                contract_address,
                bounty_id: format!("0x{}", "11".repeat(32)),
                kind,
                data,
                occurred_at: chrono::Utc::now(),
            },
        )
        .collect();
    (q, p, events)
}
fn verify(
    q: &ManagedQuote,
    p: &FundingPlan,
    e: &[AutonomousBountyEvent],
    safe: u64,
    pending: u64,
) -> Result<RecoveredPrincipal, RecoveryError> {
    verify_recovered_principal(
        q,
        p,
        e,
        RecoveryBlock {
            number: safe,
            hash: format!("0x{safe:064x}"),
        },
        RecoveryBlock {
            number: 200,
            hash: format!("0x{:064x}", 200),
        },
        RecoveryState {
            cancelled: true,
            active_claim_bond: 0,
            pending_bond_refund_total: pending,
            funding_wallet_contribution: 0,
        },
        true,
    )
}
#[test]
fn recovery_needs_full_principal_and_cancellation_and_keeps_identity_across_safe_checkpoints() {
    let (q, p, e) = fixture();
    let a = verify(&q, &p, &e, 202, 0).unwrap();
    let b = verify(&q, &p, &e, 220, 0).unwrap();
    assert_eq!(a.identity_digest(), b.identity_digest());
    assert_eq!(a.order_id(), p.order_id);
    assert!(verify(&q, &p, &e, 201, 0).is_err());
    assert!(verify(&q, &p, &e, 202, 1).is_err());
    for index in 0..e.len() {
        let mut bad = e.clone();
        bad.remove(index);
        assert!(verify(&q, &p, &bad, 202, 0).is_err());
    }
    let mut bad = e.clone();
    bad.push(e[3].clone());
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
    let mut reordered = e.clone();
    reordered.reverse();
    assert!(verify(&q, &p, &reordered, 202, 0).is_ok());
    for (key, value) in [
        ("principal", json!(p.target_micro_usdc - 1)),
        ("timeout_bond_bonus", json!(6)),
        ("contributor", json!(p.factory)),
    ] {
        let mut bad = e.clone();
        bad[3].data[key] = value;
        assert!(verify(&q, &p, &bad, 202, 0).is_err());
    }
    // A bonus never substitutes for a missing unit of the owner's original principal.
    let mut bad = e.clone();
    bad[3].data["principal"] = json!(p.target_micro_usdc - 1);
    bad[3].data["timeout_bond_bonus"] = json!(6);
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
}
#[test]
fn spoofed_or_conflicting_history_never_qualifies() {
    let (q, p, e) = fixture();
    for index in 0..e.len() {
        let mut bad = e.clone();
        bad[index].contract_address = format!("0x{}", "99".repeat(20));
        assert!(verify(&q, &p, &bad, 202, 0).is_err());
        let mut bad = e.clone();
        bad[index].tx_hash = "0xnotahash".into();
        assert!(verify(&q, &p, &bad, 202, 0).is_err());
    }
    let mut bad = e.clone();
    bad[2].kind = Kind::ClaimExpired;
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
    let mut bad = e.clone();
    bad[2].kind = Kind::SubmissionExpired;
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
    let mut bad = e.clone();
    bad[2].kind = Kind::BountySettled;
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
    let mut bad = e.clone();
    bad[3].bounty_id = format!("0x{}", "33".repeat(32));
    assert!(verify(&q, &p, &bad, 202, 0).is_err());
    let mut bad = p.clone();
    bad.network = "base-mainnet".into();
    assert!(verify(&q, &bad, &e, 202, 0).is_err());
    let mut bad = p.clone();
    bad.target_micro_usdc -= 1;
    assert!(verify(&q, &bad, &e, 202, 0).is_err());
    for state in [
        RecoveryState {
            cancelled: false,
            active_claim_bond: 0,
            pending_bond_refund_total: 0,
            funding_wallet_contribution: 0,
        },
        RecoveryState {
            cancelled: true,
            active_claim_bond: 1,
            pending_bond_refund_total: 0,
            funding_wallet_contribution: 0,
        },
        RecoveryState {
            cancelled: true,
            active_claim_bond: 0,
            pending_bond_refund_total: 0,
            funding_wallet_contribution: 1,
        },
    ] {
        assert!(verify_recovered_principal(
            &q,
            &p,
            &e,
            RecoveryBlock {
                number: 202,
                hash: format!("0x{:064x}", 202)
            },
            RecoveryBlock {
                number: 200,
                hash: format!("0x{:064x}", 200)
            },
            state,
            true
        )
        .is_err());
    }
    assert!(verify_recovered_principal(
        &q,
        &p,
        &e,
        RecoveryBlock {
            number: 202,
            hash: format!("0x{:064x}", 202)
        },
        RecoveryBlock {
            number: 200,
            hash: format!("0x{:064x}", 200)
        },
        RecoveryState {
            cancelled: true,
            active_claim_bond: 0,
            pending_bond_refund_total: 0,
            funding_wallet_contribution: 0
        },
        false
    )
    .is_err());
}
