use super::*;
use crate::managed_service_orders::{
    stripe_tests::{self, paid},
    tests::Fixture,
};
use hmac::{Hmac, Mac};
use managed_orders::stripe_refund::verify_managed_refund_event;
use serde_json::json;

async fn refund(f: &Fixture) -> (ManagedOrder, ManagedRefundPlan) {
    let order = paid(f).await;
    let order = f
        .store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .unwrap();
    let p = f
        .store
        .get_managed_refund_plan("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap();
    (order, p)
}
pub(in crate::managed_service_orders) fn object(p: &ManagedRefundPlan, status: &str) -> Value {
    json!({"id":format!("re_{}",p.operation_id().simple()),"object":"refund","charge":format!("ch_{}",p.order_id().simple()),
        "payment_intent":p.payment_intent_id(),"amount":p.amount_minor(),"currency":"mxn","status":status,"created":p.created_at(),
        "metadata":p.request(p.created_at(),p.merchant()).unwrap().body["metadata"]})
}
pub(in crate::managed_service_orders) fn signed(
    p: &ManagedRefundPlan,
    event_id: &str,
    v: Value,
    offset: i64,
) -> AuthenticatedRefundEvent {
    let wire = json!({"id":event_id,"object":"event","type":if v["status"]=="failed" {"refund.failed"}else{"refund.updated"},
        "livemode":p.merchant().livemode,"created":p.created_at()+offset,"data":{"object":v}});
    let body = serde_json::to_vec(&wire).unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"refund-fixture-secret").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&body);
    let signature = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
    verify_managed_refund_event(&body, &signature, b"refund-fixture-secret", p.merchant()).unwrap()
}
async fn state(f: &Fixture, id: Uuid) -> ManagedOrderStatus {
    f.store
        .get_managed_service_order("fixture-buyer", id)
        .await
        .unwrap()
        .unwrap()
        .status
}
async fn due(f: &Fixture, id: Uuid) {
    sqlx::query("UPDATE managed_refund_execution SET next_attempt_at=clock_timestamp()-interval '1 second',lease_until=CASE WHEN lease_token IS NULL THEN NULL ELSE clock_timestamp()-interval '1 second' END WHERE order_id=$1")
        .bind(id).execute(&f.store.pool).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_refund_requests_reserve_once_preserve_amount_and_survive_restart() {
    let f = Fixture::new().await;
    let order = paid(&f).await;
    assert!(matches!(
        f.store
            .request_managed_refund("other-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::NotFound)
    ));
    let mut tasks = vec![];
    for _ in 0..12 {
        let db = f.store.clone();
        let id = order.order_id;
        tasks.push(tokio::spawn(async move {
            db.request_managed_refund("fixture-buyer", id)
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(
            task.await.unwrap().status,
            ManagedOrderStatus::RefundRequested
        );
    }
    let p = f
        .store
        .get_managed_refund_plan("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p.amount_minor(), order.quote.total_minor());
    assert_eq!(p.amount_minor(), 225000);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_refund_operations")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let funding_available: bool = sqlx::query_scalar(
        "SELECT state='held' FROM managed_stripe_bank_actions WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert!(!funding_available);
    let restarted = f.reconnect().await;
    restarted.migrate().await.unwrap();
    assert_eq!(
        restarted
            .request_managed_refund("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .status,
        ManagedOrderStatus::RefundRequested
    );
    assert_eq!(
        restarted
            .get_managed_refund_plan("fixture-buyer", order.order_id)
            .await
            .unwrap(),
        Some(p)
    );
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn unsigned_api_success_waits_for_native_event_and_late_failure_wins() {
    let f = Fixture::new().await;
    let (order, p) = refund(&f).await;
    let lease = f
        .store
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .complete_managed_refund(&lease, &p.binding(&object(&p, "succeeded")).unwrap())
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundPending
    );
    let success = signed(&p, "evt_refund_sent", object(&p, "succeeded"), 2);
    assert_eq!(
        f.store
            .reconcile_managed_refund_event(&success)
            .await
            .unwrap(),
        ManagedEventDisposition::Processed
    );
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundSent
    );
    assert_eq!(
        f.store
            .reconcile_managed_refund_event(&success)
            .await
            .unwrap(),
        ManagedEventDisposition::Duplicate
    );
    f.store
        .reconcile_managed_refund_event(&signed(&p, "evt_refund_old", object(&p, "pending"), 1))
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundSent
    );
    f.store
        .reconcile_managed_refund_event(&signed(&p, "evt_refund_failed", object(&p, "failed"), 3))
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundFailed
    );
    f.store
        .reconcile_managed_refund_event(&signed(
            &p,
            "evt_refund_delayed_success",
            object(&p, "succeeded"),
            2,
        ))
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundFailed
    );
    assert_eq!(
        f.store
            .request_managed_refund("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .status,
        ManagedOrderStatus::RefundFailed
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn early_event_is_durable_until_exact_authenticated_binding() {
    let f = Fixture::new().await;
    let (order, p) = refund(&f).await;
    // An event can arrive after a worker takes its snapshot but before it creates.
    let stale = f
        .store
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stale.known_refund(), None);
    let e = signed(&p, "evt_refund_early", object(&p, "succeeded"), 1);
    assert_eq!(
        f.store.reconcile_managed_refund_event(&e).await.unwrap(),
        ManagedEventDisposition::PendingBinding
    );
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundRequested
    );
    assert!(f
        .store
        .begin_managed_refund_create(&stale, p.merchant())
        .await
        .is_err());
    f.store
        .release_managed_refund(&stale, ManagedCheckoutRunResult::ReconciliationRequired)
        .await
        .unwrap();
    due(&f, order.order_id).await;
    let restarted = f.reconnect().await;
    let lease = restarted
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(lease.known_refund(), Some(e.data().refund_id.as_str()));
    restarted
        .complete_managed_refund(&lease, &p.binding(&object(&p, "succeeded")).unwrap())
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundSent
    );
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn amount_substitution_duplicate_event_conflicts_and_corrupt_stored_evidence_fail_closed() {
    let f = Fixture::new().await;
    let (order, p) = refund(&f).await;
    let lease = f
        .store
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .complete_managed_refund(&lease, &p.binding(&object(&p, "pending")).unwrap())
        .await
        .unwrap();
    let mut bad = object(&p, "succeeded");
    bad["amount"] = json!(1);
    assert_eq!(
        f.store
            .reconcile_managed_refund_event(&signed(&p, "evt_wrong_amount", bad, 1))
            .await
            .unwrap(),
        ManagedEventDisposition::Quarantined
    );
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundPending
    );
    let e = signed(&p, "evt_good", object(&p, "succeeded"), 2);
    f.store.reconcile_managed_refund_event(&e).await.unwrap();
    assert!(f
        .store
        .reconcile_managed_refund_event(&signed(&p, "evt_good", object(&p, "failed"), 2))
        .await
        .is_err());
    sqlx::query("UPDATE managed_refund_events SET event=jsonb_set(event,'{amount_minor}','1') WHERE event_id='evt_good'").execute(&f.store.pool).await.unwrap();
    assert!(matches!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn fenced_workers_preserve_attempt_identity_and_cannot_steal_successor_lease() {
    let f = Fixture::new().await;
    let (order, p) = refund(&f).await;
    let mut tasks = vec![];
    for _ in 0..8 {
        let db = f.store.clone();
        let id = order.order_id;
        tasks.push(tokio::spawn(async move {
            db.claim_managed_refund(id).await.unwrap()
        }));
    }
    let mut leases = vec![];
    for task in tasks {
        if let Some(l) = task.await.unwrap() {
            leases.push(l);
        }
    }
    assert_eq!(leases.len(), 1);
    let old = leases.pop().unwrap();
    let r = f
        .store
        .begin_managed_refund_create(&old, p.merchant())
        .await
        .unwrap();
    due(&f, order.order_id).await;
    let new = f
        .store
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert!(new.previously_attempted());
    assert!(matches!(
        f.store
            .begin_managed_refund_create(&old, p.merchant())
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    assert!(matches!(
        f.store
            .complete_managed_refund(&old, &p.binding(&object(&p, "pending")).unwrap())
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    assert!(matches!(
        f.store
            .release_managed_refund(&old, ManagedCheckoutRunResult::Retry)
            .await,
        Err(ManagedOrderStoreError::StaleLease)
    ));
    let retry = f
        .store
        .begin_managed_refund_create(&new, p.merchant())
        .await
        .unwrap();
    assert_eq!(r.body, retry.body);
    assert_eq!(r.idempotency_key, retry.idempotency_key);
    f.store
        .complete_managed_refund(&new, &p.binding(&object(&p, "pending")).unwrap())
        .await
        .unwrap();
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn reservation_and_binding_failures_roll_back_without_partial_accounting() {
    let f = Fixture::new().await;
    let order = paid(&f).await;
    sqlx::query("ALTER TABLE managed_refund_operations ADD CONSTRAINT injected_failure CHECK(false) NOT VALID").execute(&f.store.pool).await.unwrap();
    assert!(f
        .store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .is_err());
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    sqlx::query("ALTER TABLE managed_refund_operations DROP CONSTRAINT injected_failure")
        .execute(&f.store.pool)
        .await
        .unwrap();
    f.store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .unwrap();
    let p = f
        .store
        .get_managed_refund_plan("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap();
    let lease = f
        .store
        .claim_managed_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .reconcile_managed_refund_event(&signed(
            &p,
            "evt_early_rollback",
            object(&p, "succeeded"),
            1,
        ))
        .await
        .unwrap();
    sqlx::query("ALTER TABLE managed_refund_execution ADD CONSTRAINT injected_failure CHECK(last_result IS DISTINCT FROM 'bound') NOT VALID").execute(&f.store.pool).await.unwrap();
    assert!(f
        .store
        .complete_managed_refund(&lease, &p.binding(&object(&p, "succeeded")).unwrap())
        .await
        .is_err());
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundRequested
    );
    let pending: String = sqlx::query_scalar(
        "SELECT disposition FROM managed_refund_events WHERE event_id='evt_early_rollback'",
    )
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert_eq!(pending, "pending_binding");
    sqlx::query("ALTER TABLE managed_refund_execution DROP CONSTRAINT injected_failure")
        .execute(&f.store.pool)
        .await
        .unwrap();
    f.store
        .complete_managed_refund(&lease, &p.binding(&object(&p, "succeeded")).unwrap())
        .await
        .unwrap();
    assert_eq!(
        state(&f, order.order_id).await,
        ManagedOrderStatus::RefundSent
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn unpaid_orders_cannot_manufacture_a_refund_and_refund_ids_cannot_fund_two_orders() {
    let f = Fixture::new().await;
    let (unpaid, _) = stripe_tests::prepared(&f).await;
    assert!(f
        .store
        .request_managed_refund("fixture-buyer", unpaid.order_id)
        .await
        .is_err());
    let (a, pa) = refund(&f).await;
    let (b, pb) = refund(&f).await;
    let la = f
        .store
        .claim_managed_refund(a.order_id)
        .await
        .unwrap()
        .unwrap();
    let lb = f
        .store
        .claim_managed_refund(b.order_id)
        .await
        .unwrap()
        .unwrap();
    let va = object(&pa, "pending");
    f.store
        .complete_managed_refund(&la, &pa.binding(&va).unwrap())
        .await
        .unwrap();
    let mut vb = object(&pb, "pending");
    vb["id"] = va["id"].clone();
    assert!(f
        .store
        .complete_managed_refund(&lb, &pb.binding(&vb).unwrap())
        .await
        .is_err());
    assert_eq!(
        state(&f, b.order_id).await,
        ManagedOrderStatus::RefundRequested
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn refund_scan_rotates_failures_and_restarts_without_inventing_observations() {
    use std::collections::HashSet;
    let f = Fixture::with_deposit().await;
    let mut full = HashSet::new();
    for _ in 0..101 {
        let (order, plan) = refund(&f).await;
        let lease = f
            .store
            .claim_managed_refund(order.order_id)
            .await
            .unwrap()
            .unwrap();
        f.store
            .complete_managed_refund(&lease, &plan.binding(&object(&plan, "pending")).unwrap())
            .await
            .unwrap();
        full.insert(order.order_id);
    }
    let mut deposits = HashSet::new();
    for _ in 0..101 {
        let order = crate::managed_service_orders::reviews::tests::funded(&f).await;
        crate::managed_service_orders::payouts::tests::settle(&f, &order, true).await;
        let DepositRefundReservation::Reserved(plan) = f
            .store
            .reserve_managed_deposit_refund(order.order_id)
            .await
            .unwrap()
        else {
            panic!("deposit reserved");
        };
        let lease = f
            .store
            .claim_managed_deposit_refund(order.order_id)
            .await
            .unwrap()
            .unwrap();
        f.store
            .complete_managed_refund(&lease, &plan.binding(&object(&plan, "pending")).unwrap())
            .await
            .unwrap();
        deposits.insert(order.order_id);
    }
    let corrupt = *deposits.iter().min().unwrap();
    sqlx::query(
        "UPDATE managed_deposit_refund_operations SET plan_digest=repeat('0',64) WHERE order_id=$1",
    )
    .bind(corrupt)
    .execute(&f.store.pool)
    .await
    .unwrap();
    let (a, b) = tokio::join!(
        f.store.claim_managed_refunds_to_observe(86400, 100, 900),
        f.store.claim_managed_refunds_to_observe(86400, 100, 900)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    let mut ids = HashSet::new();
    for (id, kind) in a.iter().chain(&b) {
        assert!(ids.insert((*id, kind_label(*kind))));
    }
    assert_eq!(ids.len(), 202);
    let restarted = f.reconnect().await;
    restarted.migrate().await.unwrap();
    assert!(restarted
        .claim_managed_refunds_to_observe(86400, 100, 900)
        .await
        .unwrap()
        .is_empty());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_refund_observations")
        .fetch_one(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "claim/crash is not a successful provider read");
    sqlx::query("UPDATE managed_refund_operations SET last_observation_attempt_at=last_observation_attempt_at-interval '901 seconds'")
        .execute(&f.store.pool).await.unwrap();
    sqlx::query("UPDATE managed_deposit_refund_operations SET last_observation_attempt_at=last_observation_attempt_at-interval '901 seconds'")
        .execute(&f.store.pool).await.unwrap();
    let batch = restarted
        .claim_managed_refunds_to_observe(86400, 100, 900)
        .await
        .unwrap();
    assert_eq!(batch.len(), 200);
    let tail = restarted
        .claim_managed_refunds_to_observe(86400, 100, 900)
        .await
        .unwrap();
    assert_eq!(tail.len(), 2);
    let mut checked = HashSet::new();
    let mut failures = 0;
    // Process malformed targets first, then valid targets, to model a permanently failing item.
    for (id, kind) in batch
        .iter()
        .chain(&tail)
        .filter(|(_, k)| *k == RefundKind::ReviewDeposit)
        .chain(
            batch
                .iter()
                .chain(&tail)
                .filter(|(_, k)| *k == RefundKind::Full),
        )
    {
        match restarted
            .managed_refund_observation_target(*id, *kind)
            .await
        {
            Err(_) => failures += 1,
            Ok(target) => {
                assert_eq!(target.signed, None);
                restarted
                    .record_managed_refund_observation(&target, RefundOutcome::Succeeded)
                    .await
                    .unwrap();
                if *kind == RefundKind::Full {
                    assert_eq!(state(&f, *id).await, ManagedOrderStatus::RefundPending);
                } else {
                    assert_eq!(
                        f.store.managed_deposit_state(*id).await.unwrap(),
                        DepositState::RefundPending
                    );
                }
                checked.insert(*id);
            }
        }
    }
    assert_eq!(failures, 1);
    full.extend(deposits);
    full.remove(&corrupt);
    assert_eq!(checked, full);
    sqlx::query("UPDATE managed_refund_observations SET refund_created_at=refund_created_at-7200")
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert_eq!(
        restarted
            .list_managed_refund_drift(3600)
            .await
            .unwrap()
            .len(),
        200
    );
    restarted.pool.close().await;
    f.cleanup().await;
}
