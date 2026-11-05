use super::*;
use crate::managed_service_orders::{
    reviews::tests::{funded, signer, verdict, DEADLINE},
    tests::Fixture,
};
use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind as Kind};
use managed_orders::{
    recovery::{verify_recovered_principal, RecoveryBlock, RecoveryState},
    stripe_dispute::ManagedDisputeEvent,
};
use serde_json::json;
use std::sync::atomic::AtomicUsize;

async fn recovery_proof(f: &Fixture, id: Uuid) -> RecoveredPrincipal {
    let t = f.store.managed_recovery_target(id).await.unwrap();
    let p = &t.funding_plan;
    let mut events = vec![];
    for (kind, address, block, index, data) in [
        (
            Kind::CanonicalBountyCreated,
            p.factory.clone(),
            42,
            0,
            json!({"bounty_contract":p.predicted_bounty,"creator":p.funding_wallet,"creation_nonce":p.creation_nonce}),
        ),
        (
            Kind::FundingAdded,
            p.predicted_bounty.clone(),
            42,
            1,
            json!({"contributor":p.funding_wallet,"amount":p.target_micro_usdc,"funded_amount":p.target_micro_usdc,"target_amount":p.target_micro_usdc}),
        ),
        (
            Kind::BountyCancelled,
            p.predicted_bounty.clone(),
            50,
            0,
            json!({}),
        ),
        (
            Kind::RefundWithdrawn,
            p.predicted_bounty.clone(),
            50,
            1,
            json!({"contributor":p.funding_wallet,"principal":p.target_micro_usdc,"timeout_bond_bonus":0,"amount":p.target_micro_usdc}),
        ),
    ] {
        events.push(AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("{block}:{index}"),
            tx_hash: if block == 42 {
                t.funding_tx.clone()
            } else {
                format!("0x{block:064x}")
            },
            block_number: block,
            log_index: index,
            contract_address: address,
            bounty_id: format!("0x{}", "11".repeat(32)),
            kind,
            data,
            occurred_at: chrono::Utc::now(),
        });
    }
    verify_recovered_principal(
        &t.order.quote,
        p,
        &events,
        RecoveryBlock {
            number: 52,
            hash: format!("0x{:064x}", 52),
        },
        RecoveryBlock {
            number: 50,
            hash: format!("0x{:064x}", 50),
        },
        RecoveryState {
            cancelled: true,
            active_claim_bond: 0,
            pending_bond_refund_total: 0,
            funding_wallet_contribution: 0,
        },
        true,
    )
    .unwrap()
}
fn dispute(id: Uuid, created: i64, status: &str) -> ManagedDisputeEvent {
    ManagedDisputeEvent {
        event_id: format!("evt_{created}_{status}"),
        event_type: "charge.dispute.updated".into(),
        event_created: created,
        livemode: false,
        dispute_id: format!("dp_{}", id.simple()),
        payment_intent_id: format!("pi_{}", id.simple()),
        amount_minor: 100,
        currency: "mxn".into(),
        status: status.into(),
        reason: "fraudulent".into(),
    }
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn recovered_reservation_is_once_retains_funding_and_blocks_new_review() {
    let f = Fixture::with_deposit().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    let (a, b) = tokio::join!(
        f.store.reserve_managed_recovered_refund(&proof),
        f.store.reserve_managed_recovered_refund(&proof)
    );
    let a = a.unwrap();
    assert_eq!(a, b.unwrap());
    assert_eq!(a.kind(), RefundKind::RecoveredBounty);
    assert_eq!(
        a.amount_minor(),
        order.quote.lines()[0].amount_minor + order.quote.lines()[1].amount_minor
    );
    assert!(f
        .store
        .request_managed_refund("fixture-buyer", order.order_id)
        .await
        .is_err());
    let calls = AtomicUsize::new(0);
    assert!(f
        .store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &verdict(&order, true),
            DEADLINE,
            None,
            signer(&calls)
        )
        .await
        .is_err());
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    let reconnect = f.reconnect().await;
    assert_eq!(
        reconnect
            .reserve_managed_recovered_refund(&proof)
            .await
            .unwrap(),
        a
    );
    assert_eq!(
        reconnect
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ManagedOrderStatus::Funded
    );
    let lease = f
        .store
        .claim_managed_recovered_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    let merchant = a.merchant().clone();
    assert_eq!(
        f.store
            .begin_managed_refund_create(&lease, &merchant)
            .await
            .unwrap()
            .body["amount"],
        a.amount_minor()
    );
    sqlx::query("UPDATE managed_recovered_refund_evidence SET verified_at=clock_timestamp()-interval '60 seconds'").execute(&f.store.pool).await.unwrap();
    assert!(f
        .store
        .begin_managed_refund_create(&lease, &merchant)
        .await
        .is_err());
    reconnect.pool.close().await;
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn acceptance_and_recovery_reservation_serialize() {
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    let calls = AtomicUsize::new(0);
    let v = verdict(&order, true);
    let (a, b) = tokio::join!(
        f.store.reserve_managed_recovered_refund(&proof),
        f.store.record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &v,
            DEADLINE,
            None,
            signer(&calls)
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn disputes_and_appeals_stop_reservation_and_conflicts_survive_newer_won_event() {
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    f.store
        .record_managed_dispute(&dispute(order.order_id, 100, "needs_response"))
        .await
        .unwrap();
    assert!(f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .is_err());
    f.store
        .record_managed_dispute(&dispute(order.order_id, 101, "won"))
        .await
        .unwrap();
    let p = f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .unwrap();
    let lease = f
        .store
        .claim_managed_recovered_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .record_managed_dispute(&dispute(order.order_id, 101, "lost"))
        .await
        .unwrap();
    f.store
        .record_managed_dispute(&dispute(order.order_id, 102, "won"))
        .await
        .unwrap();
    assert!(f
        .store
        .begin_managed_refund_create(&lease, p.merchant())
        .await
        .is_err());
    f.cleanup().await;
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    let calls = AtomicUsize::new(0);
    f.store
        .record_managed_customer_review(
            "fixture-buyer",
            order.order_id,
            &verdict(&order, false),
            DEADLINE,
            Some(DEADLINE - 100),
            signer(&calls),
        )
        .await
        .unwrap();
    assert!(f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .is_err());
    f.cleanup().await;
}
#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn funded_queue_rotates_beyond_batch_and_claimers_do_not_duplicate() {
    let f = Fixture::new().await;
    let mut ids = std::collections::HashSet::new();
    for _ in 0..23 {
        ids.insert(funded(&f).await.order_id);
    }
    let (a, b) = tokio::join!(
        f.store.claim_managed_recovery_candidates(10),
        f.store.claim_managed_recovery_candidates(10)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.len(), 10);
    assert_eq!(b.len(), 10);
    assert!(!a.iter().any(|id| b.contains(id)));
    let c = f.store.claim_managed_recovery_candidates(10).await.unwrap();
    assert_eq!(c.len(), 3);
    assert_eq!(
        a.into_iter()
            .chain(b)
            .chain(c)
            .collect::<std::collections::HashSet<_>>(),
        ids
    );
    assert!(f
        .store
        .claim_managed_recovery_candidates(10)
        .await
        .unwrap()
        .is_empty());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn lost_response_and_later_dispute_allow_original_binding_and_signed_outcome_only() {
    use crate::managed_service_orders::refunds::tests::{object, signed};
    use managed_orders::stripe_refund::RefundOutcome;
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    let p = f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .unwrap();
    let lease = f
        .store
        .claim_managed_recovered_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .begin_managed_refund_create(&lease, p.merchant())
        .await
        .unwrap();
    // The provider accepted this exact intent but the response never reached the worker.
    f.store
        .release_managed_refund(&lease, ManagedCheckoutRunResult::Retry)
        .await
        .unwrap();
    f.store
        .record_managed_dispute(&dispute(order.order_id, 100, "needs_response"))
        .await
        .unwrap();
    assert!(f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .is_err());
    assert_eq!(
        f.store
            .managed_recovered_refund_plan(order.order_id)
            .await
            .unwrap(),
        Some(p.clone())
    );
    let value = object(&p, "succeeded");
    let event = signed(&p, "evt_recovered_lost_response", value.clone(), 1);
    f.store
        .reconcile_managed_refund_event(&event)
        .await
        .unwrap();
    assert_eq!(
        f.store
            .managed_recovered_refund_state(order.order_id)
            .await
            .unwrap()["status"],
        "pending_binding"
    );
    sqlx::query("UPDATE managed_recovered_refund_execution SET next_attempt_at=clock_timestamp()-interval '1 second'").execute(&f.store.pool).await.unwrap();
    let retry = f
        .store
        .claim_managed_recovered_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retry.known_refund(), value["id"].as_str());
    assert!(f
        .store
        .begin_managed_refund_create(&retry, p.merchant())
        .await
        .is_err());
    // Read-only provider retrieval binds the original operation; no second intent is allowed.
    f.store
        .complete_managed_refund(&retry, &p.binding(&value).unwrap())
        .await
        .unwrap();
    let state = f
        .store
        .managed_recovered_refund_state(order.order_id)
        .await
        .unwrap();
    assert_eq!(state["status"], "succeeded");
    let count: i64 = sqlx::query_scalar(
        "SELECT create_attempts FROM managed_recovered_refund_execution WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ManagedOrderStatus::Funded
    );
    let target = f
        .store
        .managed_refund_observation_target(order.order_id, RefundKind::RecoveredBounty)
        .await
        .unwrap();
    assert_eq!(target.signed, Some(RefundOutcome::Succeeded));
    f.store
        .record_managed_refund_observation(&target, RefundOutcome::Failed)
        .await
        .unwrap();
    // A provider read raises drift, never replaces signed payment evidence.
    assert_eq!(
        f.store
            .managed_recovered_refund_state(order.order_id)
            .await
            .unwrap()["status"],
        "succeeded"
    );
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM managed_refund_observations WHERE order_id=$1 AND kind='recovered_bounty'").bind(order.order_id).fetch_one(&f.store.pool).await.unwrap();
    assert_eq!(count, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn recovered_bound_without_signed_event_stays_awaiting_and_observation_queue_includes_it() {
    use crate::managed_service_orders::refunds::tests::object;
    let f = Fixture::new().await;
    let order = funded(&f).await;
    let proof = recovery_proof(&f, order.order_id).await;
    let p = f
        .store
        .reserve_managed_recovered_refund(&proof)
        .await
        .unwrap();
    let lease = f
        .store
        .claim_managed_recovered_refund(order.order_id)
        .await
        .unwrap()
        .unwrap();
    f.store
        .begin_managed_refund_create(&lease, p.merchant())
        .await
        .unwrap();
    f.store
        .complete_managed_refund(&lease, &p.binding(&object(&p, "succeeded")).unwrap())
        .await
        .unwrap();
    assert_eq!(
        f.store
            .managed_recovered_refund_state(order.order_id)
            .await
            .unwrap()["status"],
        "awaiting_signed_event"
    );
    assert!(f
        .store
        .claim_managed_refunds_to_observe(86400, 100, 60)
        .await
        .unwrap()
        .contains(&(order.order_id, RefundKind::RecoveredBounty)));
    f.cleanup().await;
}
