use super::{
    stripe_tests::{event, prepared, session},
    tests::Fixture,
    *,
};

async fn expire(f: &Fixture, id: Uuid) {
    sqlx::query("UPDATE managed_stripe_checkout_execution SET lease_until=clock_timestamp()-interval '1 second',next_attempt_at=clock_timestamp()-interval '1 second' WHERE order_id=$1")
        .bind(id).execute(&f.store.pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_workers_claim_once_and_restart_preserves_attempt_identity() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let mut tasks = vec![];
    for _ in 0..12 {
        let store = f.store.clone();
        tasks.push(tokio::spawn(async move {
            store.claim_managed_checkout(order.order_id).await.unwrap()
        }));
    }
    let mut winners = vec![];
    for task in tasks {
        if let Some(lease) = task.await.unwrap() {
            winners.push(lease);
        }
    }
    assert_eq!(winners.len(), 1);
    let first = winners.pop().unwrap();
    assert!(!first.previously_attempted());
    let request = f
        .store
        .begin_managed_checkout_create(&first, plan.policy())
        .await
        .unwrap();
    expire(&f, order.order_id).await;
    let restarted = f.reconnect().await;
    restarted.migrate().await.unwrap();
    let second = restarted
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert!(second.previously_attempted());
    assert_eq!(second.plan(), &plan);
    let replay = restarted
        .begin_managed_checkout_create(&second, plan.policy())
        .await
        .unwrap();
    assert_eq!(request.idempotency_key, replay.idempotency_key);
    assert_eq!(request.body, replay.body);
    let attempts: i64 = sqlx::query_scalar(
        "SELECT create_attempts FROM managed_stripe_checkout_execution WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert_eq!(attempts, 2);
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn expired_and_superseded_workers_cannot_create_bind_or_clear_successor() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let old = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    let binding = plan
        .session_binding(&session(&plan, "cs_test_fenced", "pi_fenced"))
        .unwrap();
    expire(&f, order.order_id).await;
    assert!(matches!(
        f.store.complete_managed_checkout(&old, &binding).await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    let new = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        f.store
            .begin_managed_checkout_create(&old, plan.policy())
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    assert!(matches!(
        f.store
            .release_managed_checkout(&old, ManagedCheckoutRunResult::Retry)
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    f.store
        .complete_managed_checkout(&new, &binding)
        .await
        .unwrap();
    assert!(f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        f.store.complete_managed_checkout(&new, &binding).await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    let status = f
        .store
        .get_managed_service_order("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap()
        .status;
    assert_eq!(status, ManagedOrderStatus::AwaitingPayment); // response alone never marks payment
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn failed_completion_rolls_back_binding_payment_followup_and_lease_release() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_atomic_worker", "pi_atomic_worker");
    let paid = event(
        &plan,
        "evt_atomic_worker",
        "checkout.session.completed",
        response.clone(),
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&paid)
        .await
        .unwrap();
    let lease = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.known_session(), Some("cs_test_atomic_worker"));
    sqlx::query("ALTER TABLE managed_stripe_checkout_execution ADD CONSTRAINT fixture_fail_release CHECK(last_result IS DISTINCT FROM 'bound')")
        .execute(&f.store.pool).await.unwrap();
    let binding = plan.session_binding(&response).unwrap();
    assert!(f
        .store
        .complete_managed_checkout(&lease, &binding)
        .await
        .is_err());
    let bound: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM managed_stripe_checkout_operations WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert_eq!(bound, None);
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_stripe_paid_receipts")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(receipts, 0);
    sqlx::query(
        "ALTER TABLE managed_stripe_checkout_execution DROP CONSTRAINT fixture_fail_release",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    f.store
        .complete_managed_checkout(&lease, &binding)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn conflicting_or_corrupt_early_candidates_do_not_authorize_a_claim() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    for (id, session_id, pi) in [
        ("evt_candidate1", "cs_test_candidate1", "pi_candidate1"),
        ("evt_candidate2", "cs_test_candidate2", "pi_candidate2"),
    ] {
        f.store
            .reconcile_managed_checkout_event(&event(
                &plan,
                id,
                "checkout.session.completed",
                session(&plan, session_id, pi),
                0,
            ))
            .await
            .unwrap();
    }
    assert!(f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .is_err());
    sqlx::query("UPDATE managed_stripe_event_inbox SET event_fingerprint=repeat('0',64) WHERE event_id='evt_candidate1'").execute(&f.store.pool).await.unwrap();
    assert!(matches!(
        f.store.claim_managed_checkout(order.order_id).await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    let claims: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_stripe_checkout_execution")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(claims, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn policy_conflict_and_bound_session_cannot_start_another_create() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let lease = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    let mut changed = plan.policy().clone();
    changed.maximum_minor += 1;
    assert!(f
        .store
        .begin_managed_checkout_create(&lease, &changed)
        .await
        .is_err());
    let binding = plan
        .session_binding(&session(&plan, "cs_test_bound", "pi_bound"))
        .unwrap();
    f.store
        .complete_managed_checkout(&lease, &binding)
        .await
        .unwrap();
    sqlx::query("UPDATE managed_stripe_checkout_execution SET next_attempt_at=clock_timestamp()-interval '1 second'").execute(&f.store.pool).await.unwrap();
    let lease = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.known_session(), Some("cs_test_bound"));
    assert!(f
        .store
        .begin_managed_checkout_create(&lease, plan.policy())
        .await
        .is_err());
    let attempts: i64 =
        sqlx::query_scalar("SELECT create_attempts FROM managed_stripe_checkout_execution")
            .fetch_one(&f.store.pool)
            .await
            .unwrap();
    assert_eq!(attempts, 0);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn lease_expiry_is_rechecked_after_waiting_for_the_order_lock() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let lease = f
        .store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    let mut held = f.store.pool.begin().await.unwrap();
    sqlx::query("SELECT order_id FROM managed_service_orders WHERE order_id=$1 FOR UPDATE")
        .bind(order.order_id)
        .fetch_one(&mut *held)
        .await
        .unwrap();
    sqlx::query("UPDATE managed_stripe_checkout_execution SET lease_until=clock_timestamp()+interval '0.5 seconds' WHERE order_id=$1")
        .bind(order.order_id).execute(&f.store.pool).await.unwrap();
    let other = f.reconnect().await;
    let worker = other.clone();
    let binding = plan
        .session_binding(&session(&plan, "cs_test_lock_wait", "pi_lock_wait"))
        .unwrap();
    let result =
        tokio::spawn(async move { worker.complete_managed_checkout(&lease, &binding).await });
    sqlx::query("SELECT pg_sleep(1)")
        .execute(&mut *held)
        .await
        .unwrap();
    held.commit().await.unwrap();
    assert!(matches!(
        result.await.unwrap(),
        Err(ManagedOrderStoreError::StaleLease)
    ));
    other.pool.close().await;
    f.cleanup().await;
}
