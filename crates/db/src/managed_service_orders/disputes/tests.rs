use super::*;
use crate::managed_service_orders::{
    funding::tests::{published, reserved},
    tests::{fixture_target, Fixture},
    ManagedOrderStatus,
};
use managed_orders::OrderError;

fn dispute(order_id: Uuid, created: i64, status: &str) -> ManagedDisputeEvent {
    ManagedDisputeEvent {
        event_id: format!("evt_{created}"),
        event_type: "charge.dispute.created".into(),
        event_created: created,
        livemode: false,
        dispute_id: format!("dp_{}", order_id.simple()),
        payment_intent_id: format!("pi_{}", order_id.simple()),
        amount_minor: 225_000,
        currency: "mxn".into(),
        status: status.into(),
        reason: "fraudulent".into(),
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn a_dispute_stops_candidacy_and_reservation_and_keeps_its_latest_status() {
    let f = Fixture::new().await;
    let order = published(&f).await;
    assert_eq!(
        f.store
            .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
            .await
            .unwrap(),
        vec![order.order_id]
    );
    assert_eq!(
        f.store
            .record_managed_dispute(&dispute(order.order_id, 20, "needs_response"))
            .await
            .unwrap(),
        Some(order.order_id)
    );
    assert!(f
        .store
        .managed_order_disputed(order.order_id)
        .await
        .unwrap());
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        f.store
            .reserve_managed_funding(order.order_id, &fixture_target(), 3_600)
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    // An older event never overwrites a newer status; a won dispute still blocks funding.
    f.store
        .record_managed_dispute(&dispute(order.order_id, 30, "won"))
        .await
        .unwrap();
    f.store
        .record_managed_dispute(&dispute(order.order_id, 10, "under_review"))
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT status FROM managed_stripe_disputes")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(status, "won");
    assert!(f
        .store
        .list_managed_orders_ready_for_funding(&fixture_target(), 0, 3_600, 10)
        .await
        .unwrap()
        .is_empty());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn unknown_or_inconsistent_disputes_bind_nothing() {
    let f = Fixture::new().await;
    let (order, _) = reserved(&f).await;
    let mut unknown = dispute(Uuid::new_v4(), 10, "needs_response");
    unknown.dispute_id = "dp_unknown".into();
    assert_eq!(
        f.store.record_managed_dispute(&unknown).await.unwrap(),
        None
    );
    for change in [
        |d: &mut ManagedDisputeEvent| d.amount_minor = 225_001,
        |d: &mut ManagedDisputeEvent| d.currency = "usd".into(),
    ] {
        let mut bad = dispute(order.order_id, 10, "needs_response");
        change(&mut bad);
        assert!(matches!(
            f.store.record_managed_dispute(&bad).await,
            Err(ManagedOrderStoreError::Integrity)
        ));
    }
    assert!(!f
        .store
        .managed_order_disputed(order.order_id)
        .await
        .unwrap());
    // After reservation a dispute is still recorded, for the worker and operators to see.
    f.store
        .record_managed_dispute(&dispute(order.order_id, 10, "needs_response"))
        .await
        .unwrap();
    assert!(f
        .store
        .managed_order_disputed(order.order_id)
        .await
        .unwrap());
    let other = published(&f).await;
    let mut stolen = dispute(other.order_id, 11, "needs_response");
    stolen.dispute_id = format!("dp_{}", order.order_id.simple());
    assert!(matches!(
        f.store.record_managed_dispute(&stolen).await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    assert_eq!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ManagedOrderStatus::FundingReserved
    );
    f.cleanup().await;
}
