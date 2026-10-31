//! Review deposit: refunded only after canonical settlement without a reversal, through its own
//! fenced refund; forfeited to the solver by a reversal and paid by one verified transfer.
use super::*;
use crate::managed_service_orders::{
    refunds::tests::{object, signed},
    reviews::tests::{appeal_of, funded, signer, verdict, DEADLINE, ENDS},
    tests::Fixture,
    DepositRefundReservation, DepositState, ManagedOrder, ManagedOrderStatus,
};
use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind};
use managed_orders::{
    stripe_refund::{RefundKind, RefundMerchant, RefundOutcome},
    OrderError,
};
use std::sync::atomic::AtomicUsize;

const TX_A: &str = "0x00000000000000000000000000000000000000000000000000000000000000a1";
const TX_B: &str = "0x00000000000000000000000000000000000000000000000000000000000000b2";

pub(in crate::managed_service_orders) async fn settle(
    f: &Fixture,
    order: &ManagedOrder,
    verified: bool,
) {
    let bounty = &order.quote.input().bounty.predicted_bounty;
    let event = AutonomousBountyEvent {
        id: Uuid::new_v4(),
        log_key: format!("0x{:0>64}:7", order.order_id.simple()),
        tx_hash: format!("0x{:0>64}", order.order_id.simple()),
        block_number: 99,
        log_index: 7,
        contract_address: bounty.clone(),
        bounty_id: format!("0x{}", "b1".repeat(32)),
        kind: AutonomousBountyEventKind::BountySettled,
        data: serde_json::json!({"round": 1}),
        occurred_at: chrono::Utc::now(),
    };
    f.store
        .upsert_autonomous_bounty_event("base-sepolia", &event)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE autonomous_bounty_events SET block_time_verified=$1 WHERE contract_address=$2",
    )
    .bind(verified)
    .bind(bounty)
    .execute(&f.store.pool)
    .await
    .unwrap();
}

async fn state(f: &Fixture, order: &ManagedOrder) -> DepositState {
    f.store.managed_deposit_state(order.order_id).await.unwrap()
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn the_deposit_is_refunded_once_after_canonical_settlement_through_signed_events() {
    let f = Fixture::with_deposit().await;
    let order = funded(&f).await;
    assert_eq!(state(&f, &order).await, DepositState::Held);
    let reserve = || f.store.reserve_managed_deposit_refund(order.order_id);
    assert_eq!(
        reserve().await.unwrap(),
        DepositRefundReservation::NotSettled
    );
    // An unverified settlement is not evidence.
    settle(&f, &order, false).await;
    assert_eq!(
        reserve().await.unwrap(),
        DepositRefundReservation::NotSettled
    );
    settle(&f, &order, true).await;
    let DepositRefundReservation::Reserved(plan) = reserve().await.unwrap() else {
        panic!("settled bounty reserves its deposit refund");
    };
    assert_eq!(
        (plan.kind(), plan.amount_minor()),
        (RefundKind::ReviewDeposit, 20_000)
    );
    assert_eq!(
        reserve().await.unwrap(),
        DepositRefundReservation::Reserved(plan.clone())
    );
    assert_eq!(state(&f, &order).await, DepositState::RefundPending);
    assert_eq!(
        f.store
            .list_unbound_managed_deposit_refunds(10)
            .await
            .unwrap(),
        vec![order.order_id]
    );
    // The full pre-funding refund stays unavailable for the funded order.
    assert!(f
        .store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .is_err());

    let merchant = RefundMerchant {
        stripe_account_id: "acct_fixture_only".into(),
        livemode: false,
    };
    let lease = f
        .store
        .claim_managed_deposit_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert!(f
        .store
        .claim_managed_deposit_refund(order.order_id)
        .await
        .unwrap()
        .is_none());
    let request = f
        .store
        .begin_managed_refund_create(&lease, &merchant)
        .await
        .unwrap();
    assert_eq!(request.body["amount"], 20_000);
    assert!(request.idempotency_key.contains(":deposit-refund:"));
    let binding = plan.binding(&object(&plan, "pending")).unwrap();
    f.store
        .complete_managed_refund(&lease, &binding)
        .await
        .unwrap();
    assert!(f
        .store
        .list_unbound_managed_deposit_refunds(10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(state(&f, &order).await, DepositState::RefundPending);

    // Stripe says succeeded but no signed event arrived: the read is recorded and reported as
    // drift once the refund is old enough, and the deposit state does not move.
    let targets = f
        .store
        .claim_managed_refunds_to_observe(86_400, 10, 0)
        .await
        .unwrap();
    assert_eq!(targets.len(), 1);
    let targets = vec![f
        .store
        .managed_refund_observation_target(targets[0].0, targets[0].1)
        .await
        .unwrap()];
    assert_eq!(
        (targets[0].plan.kind(), targets[0].signed),
        (RefundKind::ReviewDeposit, None)
    );
    f.store
        .record_managed_refund_observation(&targets[0], RefundOutcome::Succeeded)
        .await
        .unwrap();
    assert!(f
        .store
        .list_managed_refund_drift(3_600)
        .await
        .unwrap()
        .is_empty());
    sqlx::query("UPDATE managed_refund_observations SET refund_created_at=refund_created_at-7200")
        .execute(&f.store.pool)
        .await
        .unwrap();
    let drift = f.store.list_managed_refund_drift(3_600).await.unwrap();
    assert_eq!(
        (
            drift.len(),
            drift[0].order_id,
            drift[0].stripe_status,
            drift[0].signed_status
        ),
        (1, order.order_id, RefundOutcome::Succeeded, None)
    );
    assert_eq!(state(&f, &order).await, DepositState::RefundPending);

    f.store
        .reconcile_managed_refund_event(&signed(
            &plan,
            "evt_deposit_1",
            object(&plan, "succeeded"),
            5,
        ))
        .await
        .unwrap();
    assert_eq!(state(&f, &order).await, DepositState::Refunded);
    // The resent signed event clears the drift on the next read.
    let targets = f
        .store
        .claim_managed_refunds_to_observe(86_400, 10, 0)
        .await
        .unwrap();
    let targets = vec![f
        .store
        .managed_refund_observation_target(targets[0].0, targets[0].1)
        .await
        .unwrap()];
    assert_eq!(targets[0].signed, Some(RefundOutcome::Succeeded));
    f.store
        .record_managed_refund_observation(&targets[0], RefundOutcome::Succeeded)
        .await
        .unwrap();
    assert!(f
        .store
        .list_managed_refund_drift(0)
        .await
        .unwrap()
        .is_empty());
    // The order itself is still funded: a deposit refund is not an order refund.
    assert_eq!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ManagedOrderStatus::Funded
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn a_reversal_forfeits_the_deposit_and_owes_one_verified_payout() {
    let f = Fixture::with_deposit().await;
    let calls = AtomicUsize::new(0);
    let mut paid_orders = vec![];
    for _ in 0..2 {
        let order = funded(&f).await;
        let review = f
            .store
            .record_managed_customer_review(
                "fixture-buyer",
                order.order_id,
                &verdict(&order, false),
                DEADLINE,
                Some(ENDS),
                signer(&calls),
            )
            .await
            .unwrap();
        let appeal = appeal_of(&review, "Section 3 of the report contains the benchmark.");
        f.store
            .record_managed_review_appeal(order.order_id, &appeal, "0xab", ENDS - 1)
            .await
            .unwrap();
        f.store
            .record_managed_review_ruling(
                order.order_id,
                1,
                true,
                "Section 3 contains the requested benchmark.",
                "operator",
                signer(&calls),
            )
            .await
            .unwrap();
        paid_orders.push((order, appeal.solver));
    }
    let (order, solver) = &paid_orders[0];
    assert_eq!(state(&f, order).await, DepositState::ForfeitedToSolver);
    // A reversal creates the deposit obligation before a settlement relay. If that relay
    // fails, the signed ruling must remain discoverable on subsequent finalizer passes.
    let retryable = f
        .store
        .claim_managed_funded_bounties("base-sepolia", 10, 0)
        .await
        .unwrap();
    assert_eq!(retryable.len(), 2);
    assert!(retryable.iter().any(|(id, _)| *id == order.order_id));
    settle(&f, order, false).await;
    assert!(f
        .store
        .claim_managed_funded_bounties("base-sepolia", 10, 0)
        .await
        .unwrap()
        .iter()
        .any(|(id, _)| *id == order.order_id));
    settle(&f, order, true).await;
    let remaining = f
        .store
        .claim_managed_funded_bounties("base-sepolia", 10, 0)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].0, paid_orders[1].0.order_id);
    assert_eq!(
        f.store
            .reserve_managed_deposit_refund(order.order_id)
            .await
            .unwrap(),
        DepositRefundReservation::Forfeited
    );
    let owed = f
        .store
        .managed_deposit_payout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            owed.round,
            owed.solver.as_str(),
            owed.amount_micro_usdc,
            owed.tx_hash.as_deref()
        ),
        (1, solver.as_str(), 10_000_000, None)
    );
    assert_eq!(
        owed.bounty_contract,
        order.quote.input().bounty.predicted_bounty
    );
    assert_eq!(
        f.store
            .list_managed_deposit_payouts(true, 10)
            .await
            .unwrap()
            .len(),
        2
    );
    let paid = f
        .store
        .record_managed_deposit_payout(order.order_id, TX_A, 1234)
        .await
        .unwrap();
    assert_eq!(
        (paid.tx_hash.as_deref(), paid.block_number),
        (Some(TX_A), Some(1234))
    );
    assert_eq!(
        f.store
            .record_managed_deposit_payout(order.order_id, TX_A, 1234)
            .await
            .unwrap(),
        paid
    );
    for (id, tx) in [(order.order_id, TX_B), (paid_orders[1].0.order_id, TX_A)] {
        assert!(matches!(
            f.store.record_managed_deposit_payout(id, tx, 1).await,
            Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
        ));
    }
    assert_eq!(
        f.store
            .list_managed_deposit_payouts(true, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn no_deposit_or_a_card_dispute_reserves_nothing() {
    let plain = Fixture::new().await;
    let order = funded(&plain).await;
    settle(&plain, &order, true).await;
    assert_eq!(
        plain
            .store
            .reserve_managed_deposit_refund(order.order_id)
            .await
            .unwrap(),
        DepositRefundReservation::NoDeposit
    );
    assert_eq!(state(&plain, &order).await, DepositState::NoDeposit);
    plain.cleanup().await;

    let f = Fixture::with_deposit().await;
    let order = funded(&f).await;
    settle(&f, &order, true).await;
    f.store
        .record_managed_dispute(&managed_orders::stripe_dispute::ManagedDisputeEvent {
            event_id: "evt_dispute_deposit".into(),
            event_type: "charge.dispute.created".into(),
            event_created: 20,
            livemode: false,
            dispute_id: format!("dp_{}", order.order_id.simple()),
            payment_intent_id: format!("pi_{}", order.order_id.simple()),
            amount_minor: order.quote.total_minor(),
            currency: "mxn".into(),
            status: "needs_response".into(),
            reason: "fraudulent".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        f.store
            .reserve_managed_deposit_refund(order.order_id)
            .await
            .unwrap(),
        DepositRefundReservation::Disputed
    );
    f.cleanup().await;
}
