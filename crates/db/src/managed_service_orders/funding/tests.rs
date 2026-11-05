use super::*;
use crate::managed_service_orders::{
    stripe_tests::paid,
    tests::{fixture_target, fixture_terms, Fixture, FIXTURE_FACTORY, FIXTURE_WALLET},
    ManagedOrder,
};
use chain_base::AutonomousBountyEventKind;
use managed_orders::OrderError;
use serde_json::json;

const TX: &str = "0x00000000000000000000000000000000000000000000000000000000000000aa";

/// A paid order whose committed public terms are stored, as the quote route stores them.
pub(in crate::managed_service_orders) async fn published(f: &Fixture) -> ManagedOrder {
    let order = paid(f).await;
    f.store
        .upsert_autonomous_bounty_terms(&fixture_terms(
            order.quote.input().bounty.funding_deadline,
            &order.quote.input().bounty.creation_nonce,
        ))
        .await
        .unwrap();
    order
}
async fn status(f: &Fixture, order_id: Uuid) -> ManagedOrderStatus {
    f.store
        .get_managed_service_order("fixture-buyer", order_id)
        .await
        .unwrap()
        .unwrap()
        .status
}
pub(in crate::managed_service_orders) async fn reserved(
    f: &Fixture,
) -> (ManagedOrder, ManagedFundingOperation) {
    let order = published(f).await;
    let op = f
        .store
        .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
        .await
        .unwrap();
    (order, op)
}
async fn index(f: &Fixture, plan: &FundingPlan, verified: bool, amount: u64) {
    let bounty_id = format!("0x{}", "b1".repeat(32));
    let events = [
        (
            AutonomousBountyEventKind::CanonicalBountyCreated,
            FIXTURE_FACTORY,
            json!({"bounty_contract": plan.predicted_bounty, "creator": FIXTURE_WALLET,
                "creation_nonce": plan.creation_nonce}),
        ),
        (
            AutonomousBountyEventKind::FundingAdded,
            plan.predicted_bounty.as_str(),
            json!({"contributor": FIXTURE_WALLET, "amount": amount,
                "funded_amount": amount, "target_amount": plan.target_micro_usdc}),
        ),
    ];
    for (index, (kind, contract, data)) in events.into_iter().enumerate() {
        let event = AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("{TX}:{index}"),
            tx_hash: TX.into(),
            block_number: 42,
            log_index: index as u64,
            contract_address: contract.into(),
            bounty_id: bounty_id.clone(),
            kind,
            data,
            occurred_at: chrono::Utc::now(),
        };
        f.store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();
    }
    sqlx::query("UPDATE autonomous_bounty_events SET block_time_verified=$1")
        .bind(verified)
        .execute(&f.store.pool)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_reservation_funds_once_and_blocks_refund() {
    let f = Fixture::new().await;
    let order = published(&f).await;
    assert_eq!(
        f.store
            .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
            .await
            .unwrap(),
        vec![order.order_id]
    );
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&fixture_target(), 3_600, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(
        f.store
            .list_managed_orders_ready_for_funding(&fixture_target(), 0, 86_400, 10)
            .await
            .unwrap()
            .is_empty(),
        "an order too close to its funding deadline is never a candidate"
    );
    let mut mainnet = fixture_target();
    mainnet.network = "base-mainnet".into();
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&mainnet, 0, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &mainnet, 3_600)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &fixture_target(), 86_400)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    let mut tasks = vec![];
    for _ in 0..8 {
        let store = f.store.clone();
        tasks.push(tokio::spawn(async move {
            store
                .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
                .await
                .unwrap()
        }));
    }
    let mut ops = vec![];
    for task in tasks {
        ops.push(task.await.unwrap());
    }
    let first = ops[0].clone();
    assert!(ops.iter().all(|op| op == &first));
    assert_eq!(first.plan.target_micro_usdc, 112_500_000);
    assert_eq!(first.plan.funding_wallet, FIXTURE_WALLET);
    assert_eq!(
        status(&f, order.order_id).await,
        ManagedOrderStatus::FundingReserved
    );
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        f.store
            .request_managed_refund("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    let bypass = sqlx::query(
        "UPDATE managed_stripe_bank_actions SET state='refund_reserved' WHERE order_id=$1",
    )
    .bind(order.order_id)
    .execute(&f.store.pool)
    .await;
    assert!(
        bypass.is_err(),
        "the row check must refuse refunding a funding order"
    );
    let replay = f.reconnect().await;
    replay.migrate().await.unwrap();
    assert_eq!(
        replay
            .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
            .await
            .unwrap(),
        first
    );
    replay.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn refund_reserved_or_unpaid_orders_are_never_funded() {
    let f = Fixture::new().await;
    let order = published(&f).await;
    f.store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    let (unpaid, _) = crate::managed_service_orders::stripe_tests::prepared(&f).await;
    assert!(matches!(
        f.store
            .reserve_managed_funding(unpaid.order_id, &fixture_target(), 3_600)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_bounty_funding_operations")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn unpublished_terms_or_another_fee_reserve_nothing() {
    let f = Fixture::new().await;
    let order = paid(&f).await;
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    f.store
        .upsert_autonomous_bounty_terms(&fixture_terms(
            order.quote.input().bounty.funding_deadline - 1,
            &order.quote.input().bounty.creation_nonce,
        ))
        .await
        .unwrap();
    let mut other_fee = fixture_target();
    other_fee.fee.platform_fee_bps = 500;
    for _ in 0..2 {
        assert!(matches!(
            f.store
                .reserve_managed_funding(order.order_id, &other_fee, 3_600)
                .await,
            Err(ManagedOrderStoreError::Integrity)
        ));
        f.store
            .upsert_autonomous_bounty_terms(&fixture_terms(
                order.quote.input().bounty.funding_deadline,
                &order.quote.input().bounty.creation_nonce,
            ))
            .await
            .unwrap();
    }
    assert_eq!(
        status(&f, order.order_id).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    f.store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .unwrap();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn leases_fence_one_worker_and_one_transaction() {
    let f = Fixture::new().await;
    let (order, _) = reserved(&f).await;
    let (second, second_op) = reserved(&f).await;
    let lease = f
        .store
        .claim_managed_funding_lease(order.order_id, 60)
        .await
        .unwrap()
        .unwrap();
    for id in [order.order_id, second.order_id] {
        assert!(
            f.store
                .claim_managed_funding_lease(id, 60)
                .await
                .unwrap()
                .is_none(),
            "one delegate sender, one live lease"
        );
    }
    assert!(f
        .store
        .managed_funding_lease_has_time(order.order_id, lease, 30)
        .await
        .unwrap());
    assert!(!f
        .store
        .managed_funding_lease_has_time(order.order_id, lease, 120)
        .await
        .unwrap());
    assert!(matches!(
        f.store
            .record_managed_funding_broadcast(order.order_id, Uuid::new_v4(), TX)
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    f.store
        .record_managed_funding_broadcast(
            order.order_id,
            lease,
            &TX.to_ascii_uppercase().replace("0X", "0x"),
        )
        .await
        .unwrap();
    f.store
        .record_managed_funding_broadcast(order.order_id, lease, TX)
        .await
        .unwrap();
    let other = format!("0x{}", "bb".repeat(32));
    assert!(matches!(
        f.store
            .record_managed_funding_broadcast(order.order_id, lease, &other)
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    assert_eq!(
        status(&f, order.order_id).await,
        ManagedOrderStatus::FundingBroadcast
    );
    sqlx::query("UPDATE managed_bounty_funding_operations SET lease_until=clock_timestamp()-interval '1 second' WHERE order_id=$1")
        .bind(order.order_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    let taken = f
        .store
        .claim_managed_funding_lease(order.order_id, 60)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(taken, lease);
    assert!(matches!(
        f.store
            .record_managed_funding_broadcast(order.order_id, lease, TX)
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    let open = f.store.list_open_managed_funding(10).await.unwrap();
    assert_eq!(open.len(), 2);
    assert_eq!(open[0].order_id, order.order_id);
    assert_eq!(open[0].tx_hash.as_deref(), Some(TX));
    assert_eq!(open[0].saved_tx_hash, None);
    f.store
        .release_managed_funding_lease(order.order_id, taken)
        .await
        .unwrap();
    assert!(f
        .store
        .claim_managed_funding_lease(second.order_id, 60)
        .await
        .unwrap()
        .is_some());
    // A later operation with saved signed bytes is listed first: it is never overlooked.
    let saved = chain_base::PreparedBaseRelayTransaction {
        nonce: 7,
        signed_transaction: "0x02".into(),
        transaction: chain_base::BaseRelayedTransaction {
            relayer: FIXTURE_WALLET.into(),
            tx_hash: format!("0x{}", "cc".repeat(32)),
            estimated_gas: 21_000,
            gas_limit: 25_200,
            max_fee_per_gas_wei: 1,
            max_priority_fee_per_gas_wei: 1,
            estimated_max_cost_wei: 25_200,
        },
    };
    f.store
        .save_relay_outbox(second_op.operation_id, "base-sepolia", &saved)
        .await
        .unwrap();
    let open = f.store.list_open_managed_funding(1).await.unwrap();
    assert_eq!(open[0].order_id, second.order_id);
    assert_eq!(open[0].saved_tx_hash, Some(saved.transaction.tx_hash));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn only_confirmed_exact_canonical_events_mark_funded() {
    let f = Fixture::new().await;
    let (order, op) = reserved(&f).await;
    index(&f, &op.plan, false, op.plan.target_micro_usdc).await;
    let unconfirmed = f.store.verified_funding_events(&op.plan, TX).await.unwrap();
    assert_eq!(
        managed_orders::funding::observe_funding(&op.plan, &unconfirmed),
        managed_orders::funding::FundingObservation::NotObserved
    );
    index(&f, &op.plan, true, op.plan.target_micro_usdc - 1).await;
    let short = f.store.verified_funding_events(&op.plan, TX).await.unwrap();
    assert_eq!(
        managed_orders::funding::observe_funding(&op.plan, &short),
        managed_orders::funding::FundingObservation::Conflict
    );
    index(&f, &op.plan, true, op.plan.target_micro_usdc).await;
    let events = f.store.verified_funding_events(&op.plan, TX).await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        managed_orders::funding::observe_funding(&op.plan, &events),
        managed_orders::funding::FundingObservation::Funded {
            tx_hash: TX.into(),
            block_number: 42
        }
    );
    f.store
        .record_managed_funding_funded(order.order_id, TX, 42)
        .await
        .unwrap();
    assert_eq!(status(&f, order.order_id).await, ManagedOrderStatus::Funded);
    f.store
        .record_managed_funding_reconciliation(order.order_id)
        .await
        .unwrap();
    assert_eq!(status(&f, order.order_id).await, ManagedOrderStatus::Funded);
    assert!(f
        .store
        .list_open_managed_funding(10)
        .await
        .unwrap()
        .is_empty());
    assert!(f
        .store
        .claim_managed_funding_lease(order.order_id, 60)
        .await
        .unwrap()
        .is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn reconciliation_stops_automation_and_keeps_the_money_reserved() {
    let f = Fixture::new().await;
    let (order, _) = reserved(&f).await;
    f.store
        .record_managed_funding_reconciliation(order.order_id)
        .await
        .unwrap();
    assert_eq!(
        status(&f, order.order_id).await,
        ManagedOrderStatus::FundingReconciliationRequired
    );
    assert!(f
        .store
        .list_open_managed_funding(10)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        f.store
            .request_managed_refund("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    sqlx::query("UPDATE managed_bounty_funding_operations SET plan=jsonb_set(plan,'{target_micro_usdc}','1')")
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    f.cleanup().await;
}
