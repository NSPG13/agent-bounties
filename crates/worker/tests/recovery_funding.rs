//! Shipping worker process coverage. The only signing key is public scalar one and the RPC
//! listener is loopback-only. These fixtures do not execute EVM contracts or prove payment.
//! Opt-in because every case requires an isolated disposable Postgres database.
#[path = "support/funding.rs"]
mod support;

use serde_json::json;
use support::*;

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_migration_precedes_invalid_configuration() {
    let f = Fixture::new("startup", false).await;
    f.run("startup-invalid", "invalid", "config_error", &[], None)
        .await;
    assert!(!f.snapshot().await.is_empty());
    f.run("startup-idle", "silent", "idle", &[], None).await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_idle_restart_has_no_side_effects() {
    let f = Fixture::new("idle", true).await;
    f.run("idle-first", "silent", "idle", &[], None).await;
    f.run("idle-restart", "silent", "idle", &[], None).await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_live_lease_preserves_reservation() {
    let f = Fixture::new("busy", true).await;
    let (_, op) = f.reserved().await;
    f.store
        .claim_managed_funding_lease(op.order_id, 180)
        .await
        .unwrap()
        .unwrap();
    f.run("busy", "silent", "busy", &[], Some(&op)).await;
    f.assert_held(1, 0).await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_dispute_and_expiry_stop_before_signing() {
    for expired in [false, true] {
        let f = Fixture::new(if expired { "expired" } else { "disputed" }, true).await;
        let (order, mut op) = f.reserved().await;
        if expired {
            // Advance only the stored plan's clock boundary with its correct digest. This
            // isolates the worker expiry branch; it does not qualify quote/calldata expiry.
            op.plan.funding_deadline = chrono::Utc::now().timestamp() - 1;
            f.replace_plan(&op, false).await;
        } else {
            f.store
                .record_managed_dispute(&managed_orders::stripe_dispute::ManagedDisputeEvent {
                    event_id: "evt_fixture_dispute".into(),
                    event_type: "charge.dispute.created".into(),
                    event_created: chrono::Utc::now().timestamp(),
                    livemode: true,
                    dispute_id: format!("dp_{}", order.order_id.simple()),
                    payment_intent_id: format!("pi_{}", order.order_id.simple()),
                    amount_minor: order.quote.total_minor(),
                    currency: "mxn".into(),
                    status: "needs_response".into(),
                    reason: "fraudulent".into(),
                })
                .await
                .unwrap();
        }
        f.run(
            if expired { "expired" } else { "disputed" },
            "silent",
            "reconciliation_required",
            MUTABLE_OPERATION,
            Some(&op),
        )
        .await;
        f.assert_held(1, 0).await;
        assert_eq!(
            f.status(op.order_id).await,
            "funding_reconciliation_required"
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_saved_journal_precedes_unsigned_work_and_receipt_is_not_payment() {
    let f = Fixture::new("mined", true).await;
    let (_, op) = f.reserved().await;
    f.seed_journal(&op).await;
    let (_, second) = f.reserved().await;
    for name in ["mined-first", "mined-restart"] {
        let report = f
            .run(
                name,
                "mined",
                "awaiting_confirmation",
                MUTABLE_OPERATION,
                Some(&op),
            )
            .await;
        assert_eq!(report["order_id"], json!(op.order_id));
        assert_eq!(f.status(second.order_id).await, "funding_reserved");
        assert_eq!(f.status(op.order_id).await, "funding_broadcast");
        f.assert_held(2, 1).await;
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_old_saved_transaction_requires_reconciliation() {
    let f = Fixture::new("old", true).await;
    let (_, op) = f.reserved().await;
    f.seed_journal(&op).await;
    sqlx::query("UPDATE relay_transaction_outbox SET created_at=now()-interval '3 hours'")
        .execute(&f.pool)
        .await
        .unwrap();
    f.run(
        "old-journal",
        "null",
        "reconciliation_required",
        MUTABLE_OPERATION,
        Some(&op),
    )
    .await;
    f.assert_held(1, 1).await;
    assert_eq!(
        f.status(op.order_id).await,
        "funding_reconciliation_required"
    );
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_wrong_configuration_and_corrupt_plan_fail_without_signing() {
    for kind in ["network", "factory", "wallet", "digest"] {
        let f = Fixture::new(&format!("wrong_{kind}"), true).await;
        let (_, mut op) = f.reserved().await;
        match kind {
            "network" => op.plan.network = "base-sepolia".into(),
            "factory" => op.plan.factory = format!("0x{}", "e1".repeat(20)),
            "wallet" => op.plan.funding_wallet = format!("0x{}", "e0".repeat(20)),
            "digest" => {}
            _ => unreachable!(),
        }
        f.replace_plan(&op, kind == "digest").await;
        f.run(
            &format!("wrong-{kind}"),
            "silent",
            "poll_error",
            &[],
            Some(&op),
        )
        .await;
        f.assert_held(1, 0).await;
    }
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn shipping_simulation_fx_holds_and_core_poll_positive_reentry() {
    let f = Fixture::new("core", true).await;
    let (_, op) = f.reserved().await;
    f.run(
        "simulation-rejected",
        "reject",
        "not_signed",
        MUTABLE_OPERATION,
        Some(&op),
    )
    .await;
    f.assert_held(1, 0).await;
    assert_eq!(f.status(op.order_id).await, "funding_reserved");

    // The production core poll runs with an explicit synthetic FX budget. This is not a
    // shipping-process restart or successful real Coinbase/EVM integration claim.
    f.core_poll("core-sign", "signed", &op).await;
    let saved = f
        .store
        .relay_outbox(op.operation_id, "base-mainnet")
        .await
        .unwrap()
        .unwrap();
    assert_signed(&saved.signed_transaction, &op);
    assert_eq!(saved.nonce, 0);
    f.assert_held(1, 1).await;
    assert_eq!(f.status(op.order_id).await, "funding_broadcast");

    sqlx::query("UPDATE relay_transaction_outbox SET next_send_at=now()-interval '1 second'")
        .execute(&f.pool)
        .await
        .unwrap();
    f.core_poll("core-reentry", "rebroadcast", &op).await;
    let retried = f
        .store
        .relay_outbox(op.operation_id, "base-mainnet")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.nonce, retried.nonce);
    assert_eq!(saved.signed_transaction, retried.signed_transaction);
    assert_eq!(saved.transaction, retried.transaction);
    f.run(
        "core-journal-shipping-restart",
        "mined",
        "awaiting_confirmation",
        MUTABLE_OPERATION,
        Some(&op),
    )
    .await;
    assert_eq!(f.status(op.order_id).await, "funding_broadcast");
    f.assert_held(1, 1).await;

    // These are actual shipping subprocesses, confined by the driver to a network namespace
    // with no external interfaces. The fixed Coinbase dependency must stop both paths.
    let unsigned = Fixture::new("fx_unsigned", true).await;
    let (_, new_op) = unsigned.reserved().await;
    let report = unsigned
        .run(
            "fx-unavailable-unsigned",
            "prepare",
            "not_signed",
            MUTABLE_OPERATION,
            Some(&new_op),
        )
        .await;
    assert!(report["detail"]
        .as_str()
        .unwrap()
        .contains("fresh ETH/USDC quote unavailable"));
    unsigned.assert_held(1, 0).await;
    sqlx::query("UPDATE relay_transaction_outbox SET next_send_at=now()-interval '1 second'")
        .execute(&f.pool)
        .await
        .unwrap();
    let report = f
        .run(
            "fx-unavailable-retry",
            "prepare",
            "poll_error",
            MUTABLE_OPERATION,
            Some(&op),
        )
        .await;
    assert!(report["error"]
        .as_str()
        .unwrap()
        .contains("fresh ETH/USDC quote unavailable"));
    f.assert_held(1, 1).await;
}
