use super::tests::{accept, Fixture};
use super::*;
use hmac::{Hmac, Mac};
use managed_orders::stripe_checkout::{
    verify_managed_checkout_event, AuthenticatedManagedEvent, CheckoutPolicy, ManagedCheckoutPlan,
};
use serde_json::{json, Value};
use sha2::Sha256;

async fn order(f: &Fixture) -> (ManagedOrder, CheckoutPolicy) {
    let mut input = f.quote().await.input().clone();
    input.expires_at = f.now().await + 7200;
    input.price_basis.valid_until = input.expires_at;
    let quote = ManagedQuote::new(input).unwrap();
    f.store.put_managed_service_quote(&quote).await.unwrap();
    let order = accept(&f.store, &quote, &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let policy = CheckoutPolicy {
        stripe_account_id: quote.input().stripe_account_id.clone(),
        livemode: false,
        currency: quote.input().currency,
        minimum_minor: 1000,
        maximum_minor: 1_000_000,
        payment_method_configuration: None,
        adaptive_pricing: false,
    };
    (order, policy)
}
pub(super) async fn prepared(f: &Fixture) -> (ManagedOrder, ManagedCheckoutPlan) {
    let (order, policy) = order(f).await;
    let plan = f
        .store
        .prepare_managed_checkout("fixture-buyer", order.order_id, policy)
        .await
        .unwrap();
    (order, plan)
}
pub(super) async fn paid(f: &Fixture) -> ManagedOrder {
    let (order, p) = prepared(f).await;
    let response = session(
        &p,
        &format!("cs_test_{}", order.order_id.simple()),
        &format!("pi_{}", order.order_id.simple()),
    );
    f.store
        .bind_managed_checkout(order.order_id, &p.session_binding(&response).unwrap())
        .await
        .unwrap();
    let e = event(
        &p,
        &format!("evt_{}", order.order_id.simple()),
        "checkout.session.completed",
        response,
        0,
    );
    f.store.reconcile_managed_checkout_event(&e).await.unwrap();
    f.store
        .get_managed_service_order("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap()
}
pub(super) fn session(plan: &ManagedCheckoutPlan, id: &str, pi: &str) -> Value {
    let request = plan.request(plan.created_at(), plan.policy()).unwrap();
    // The quoted total: every line Checkout itemizes, so deposit fixtures pay their total too.
    let total: u64 = request.body["line_items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            line["price_data"]["unit_amount"].as_u64().unwrap()
                * line["quantity"].as_u64().unwrap_or(1)
        })
        .sum();
    json!({"id":id,"object":"checkout.session","mode":"payment","livemode":plan.policy().livemode,
        "metadata":request.body["metadata"],"client_reference_id":plan.order_id(),"amount_total":total,
        "currency":"mxn","status":"complete","payment_status":"paid","payment_intent":pi,
        "expires_at":plan.expires_at(),"url":format!("https://checkout.stripe.com/c/pay/{id}")})
}
pub(super) fn event(
    plan: &ManagedCheckoutPlan,
    id: &str,
    kind: &str,
    payload: Value,
    time_offset: i64,
) -> AuthenticatedManagedEvent {
    let wire = json!({"id":id,"object":"event","type":kind,"livemode":plan.policy().livemode,
        "created":plan.created_at()+time_offset,"data":{"object":payload}});
    let body = serde_json::to_vec(&wire).unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"managed-fixture-secret").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&body);
    let signature = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
    verify_managed_checkout_event(
        &body,
        &signature,
        b"managed-fixture-secret",
        &plan.policy().stripe_account_id,
        plan.policy().livemode,
    )
    .unwrap()
}
async fn count(f: &Fixture, table: &str) -> i64 {
    assert!(matches!(
        table,
        "managed_stripe_checkout_operations"
            | "managed_stripe_event_inbox"
            | "managed_stripe_paid_receipts"
            | "managed_stripe_bank_actions"
    ));
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
        .fetch_one(&f.store.pool)
        .await
        .unwrap()
}
async fn state(store: &PostgresStore, order: &ManagedOrder) -> ManagedOrderStatus {
    store
        .get_managed_service_order("fixture-buyer", order.order_id)
        .await
        .unwrap()
        .unwrap()
        .status
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_preparation_and_restart_reuse_one_immutable_operation() {
    let f = Fixture::new().await;
    let (order, policy) = order(&f).await;
    let other = f.reconnect().await;
    let mut tasks = vec![];
    for i in 0..12 {
        let store = if i % 2 == 0 {
            f.store.clone()
        } else {
            other.clone()
        };
        let policy = policy.clone();
        let id = order.order_id;
        tasks.push(tokio::spawn(async move {
            store
                .prepare_managed_checkout("fixture-buyer", id, policy)
                .await
                .unwrap()
        }));
    }
    let expected = tasks.pop().unwrap().await.unwrap();
    for task in tasks {
        assert_eq!(task.await.unwrap(), expected);
    }
    assert_eq!(count(&f, "managed_stripe_checkout_operations").await, 1);
    assert!(matches!(
        other
            .prepare_managed_checkout("wrong-buyer", order.order_id, policy.clone())
            .await,
        Err(ManagedOrderStoreError::NotFound)
    ));
    let mut changed = policy;
    changed.maximum_minor -= 1;
    assert!(other
        .prepare_managed_checkout("fixture-buyer", order.order_id, changed)
        .await
        .is_err());
    assert_eq!(
        state(&other, &order).await,
        ManagedOrderStatus::AwaitingPayment
    );
    other.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn webhook_before_lost_response_is_durable_and_recovers_only_after_session_binding() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_early", "pi_early");
    let paid = event(
        &plan,
        "evt_early",
        "checkout.session.completed",
        response.clone(),
        0,
    );
    assert_eq!(
        f.store
            .reconcile_managed_checkout_event(&paid)
            .await
            .unwrap(),
        ManagedEventDisposition::PendingBinding
    );
    assert_eq!(count(&f, "managed_stripe_event_inbox").await, 1);
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    assert_eq!(count(&f, "managed_stripe_bank_actions").await, 0);
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::AwaitingPayment
    );
    let restarted = f.reconnect().await;
    let recovered = restarted
        .prepare_managed_checkout("fixture-buyer", order.order_id, plan.policy().clone())
        .await
        .unwrap();
    assert_eq!(recovered, plan);
    restarted
        .bind_managed_checkout(
            order.order_id,
            &recovered.session_binding(&response).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        state(&restarted, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 1);
    assert_eq!(count(&f, "managed_stripe_bank_actions").await, 1);
    restarted.migrate().await.unwrap();
    assert_eq!(
        state(&restarted, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_duplicates_and_distinct_paid_events_create_one_receipt_and_bank_action() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_once", "pi_once");
    f.store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    let paid = event(
        &plan,
        "evt_once",
        "checkout.session.completed",
        response.clone(),
        0,
    );
    let other = f.reconnect().await;
    let mut tasks = vec![];
    for i in 0..16 {
        let store = if i % 2 == 0 {
            f.store.clone()
        } else {
            other.clone()
        };
        let e = paid.clone();
        tasks.push(tokio::spawn(async move {
            store.reconcile_managed_checkout_event(&e).await.unwrap()
        }));
    }
    let mut processed = 0;
    for task in tasks {
        if task.await.unwrap() == ManagedEventDisposition::Processed {
            processed += 1;
        }
    }
    assert_eq!(processed, 1);
    let next = event(
        &plan,
        "evt_once_again",
        "checkout.session.async_payment_succeeded",
        response,
        1,
    );
    assert_eq!(
        other.reconcile_managed_checkout_event(&next).await.unwrap(),
        ManagedEventDisposition::Processed
    );
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 1);
    assert_eq!(count(&f, "managed_stripe_bank_actions").await, 1);
    assert_eq!(count(&f, "managed_stripe_event_inbox").await, 2);
    assert_eq!(
        state(&other, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    other.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn wrong_quote_money_operation_or_session_cannot_credit_an_order() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_right", "pi_right");
    f.store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    for (path, value) in [
        ("/amount_total", json!(224999)),
        ("/currency", json!("usd")),
        ("/metadata/managed_quote_digest", json!("0".repeat(64))),
        ("/metadata/managed_operation_id", json!(Uuid::new_v4())),
    ] {
        let mut altered = response.clone();
        *altered.pointer_mut(path).unwrap() = value;
        let e = event(
            &plan,
            &format!("evt_{}", Uuid::new_v4().simple()),
            "checkout.session.completed",
            altered,
            0,
        );
        assert!(
            f.store.reconcile_managed_checkout_event(&e).await.is_err(),
            "{path}"
        );
    }
    assert_eq!(count(&f, "managed_stripe_event_inbox").await, 0);
    for (id, pi) in [("cs_test_wrong", "pi_right"), ("cs_test_right", "pi_wrong")] {
        let e = event(
            &plan,
            &format!("evt_{}", Uuid::new_v4().simple()),
            "checkout.session.completed",
            session(&plan, id, pi),
            0,
        );
        assert_eq!(
            f.store.reconcile_managed_checkout_event(&e).await.unwrap(),
            ManagedEventDisposition::Quarantined
        );
    }
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    assert_eq!(count(&f, "managed_stripe_bank_actions").await, 0);
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::AwaitingPayment
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn delayed_failure_and_out_of_order_events_never_erase_paid_evidence() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_delay", "pi_delay");
    f.store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    let mut unpaid = response.clone();
    unpaid["payment_status"] = "unpaid".into();
    let pending = event(
        &plan,
        "evt_pending",
        "checkout.session.completed",
        unpaid.clone(),
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&pending)
        .await
        .unwrap();
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::PaymentPending
    );
    let failed = event(
        &plan,
        "evt_failed",
        "checkout.session.async_payment_failed",
        unpaid.clone(),
        1,
    );
    f.store
        .reconcile_managed_checkout_event(&failed)
        .await
        .unwrap();
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::PaymentFailed
    );
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    let paid = event(
        &plan,
        "evt_paid",
        "checkout.session.async_payment_succeeded",
        response,
        2,
    );
    f.store
        .reconcile_managed_checkout_event(&paid)
        .await
        .unwrap();
    let late = event(
        &plan,
        "evt_late_failure",
        "checkout.session.async_payment_failed",
        unpaid,
        3,
    );
    f.store
        .reconcile_managed_checkout_event(&late)
        .await
        .unwrap();
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn failure_after_receipt_insert_rolls_back_event_receipt_and_followup_then_recovers() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_rollback", "pi_rollback");
    f.store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    let paid = event(
        &plan,
        "evt_rollback",
        "checkout.session.completed",
        response,
        0,
    );
    sqlx::query(
        "ALTER TABLE managed_stripe_bank_actions ADD CONSTRAINT injected_failure CHECK(false)",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store.reconcile_managed_checkout_event(&paid).await,
        Err(ManagedOrderStoreError::Database(_))
    ));
    for table in [
        "managed_stripe_event_inbox",
        "managed_stripe_paid_receipts",
        "managed_stripe_bank_actions",
    ] {
        assert_eq!(count(&f, table).await, 0);
    }
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::AwaitingPayment
    );
    sqlx::query("ALTER TABLE managed_stripe_bank_actions DROP CONSTRAINT injected_failure")
        .execute(&f.store.pool)
        .await
        .unwrap();
    let restarted = f.reconnect().await;
    restarted
        .reconcile_managed_checkout_event(&paid)
        .await
        .unwrap();
    assert_eq!(
        state(&restarted, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn one_provider_session_or_payment_intent_cannot_pay_two_orders() {
    let f = Fixture::new().await;
    let (first, plan1) = prepared(&f).await;
    let (second, plan2) = prepared(&f).await;
    let response1 = session(&plan1, "cs_test_first", "pi_shared");
    f.store
        .bind_managed_checkout(first.order_id, &plan1.session_binding(&response1).unwrap())
        .await
        .unwrap();
    let same_session = session(&plan2, "cs_test_first", "pi_shared");
    assert!(f
        .store
        .bind_managed_checkout(
            second.order_id,
            &plan2.session_binding(&same_session).unwrap()
        )
        .await
        .is_err());
    let response2 = session(&plan2, "cs_test_second", "pi_shared");
    f.store
        .bind_managed_checkout(second.order_id, &plan2.session_binding(&response2).unwrap())
        .await
        .unwrap();
    let paid1 = event(
        &plan1,
        "evt_first",
        "checkout.session.completed",
        response1,
        0,
    );
    let paid2 = event(
        &plan2,
        "evt_second",
        "checkout.session.completed",
        response2,
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&paid1)
        .await
        .unwrap();
    assert!(f
        .store
        .reconcile_managed_checkout_event(&paid2)
        .await
        .is_err());
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 1);
    assert_eq!(count(&f, "managed_stripe_bank_actions").await, 1);
    assert_eq!(
        state(&f.store, &first).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    assert_eq!(
        state(&f.store, &second).await,
        ManagedOrderStatus::AwaitingPayment
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn corrupt_pending_event_cannot_be_promoted_when_the_session_response_arrives() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_corrupt", "pi_corrupt");
    let mut unpaid = response.clone();
    unpaid["payment_status"] = "unpaid".into();
    let pending = event(
        &plan,
        "evt_corrupt",
        "checkout.session.completed",
        unpaid,
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&pending)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE managed_stripe_event_inbox SET event=jsonb_set(event,'{outcome}','\"paid\"')",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store
            .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    let bound: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM managed_stripe_checkout_operations WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert!(bound.is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn expired_sessions_do_not_credit_money_and_conflicting_event_replays_do_not_replace_evidence(
) {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let mut response = session(&plan, "cs_test_expire", "pi_expire");
    response["status"] = "expired".into();
    response["payment_status"] = "unpaid".into();
    response["payment_intent"] = Value::Null;
    f.store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    let expired = event(
        &plan,
        "evt_expire",
        "checkout.session.expired",
        response.clone(),
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&expired)
        .await
        .unwrap();
    assert_eq!(
        state(&f.store, &order).await,
        ManagedOrderStatus::PaymentExpired
    );
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    response["payment_intent"] = "pi_other".into();
    let conflict = event(&plan, "evt_expire", "checkout.session.expired", response, 0);
    assert!(matches!(
        f.store.reconcile_managed_checkout_event(&conflict).await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert_eq!(count(&f, "managed_stripe_event_inbox").await, 1);
    for table in ["ledger_entries", "bounties", "funding_intents"] {
        let rows: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&f.store.pool)
            .await
            .unwrap();
        assert_eq!(
            rows, 0,
            "managed payment evidence must not credit legacy financial tables"
        );
    }
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn failed_early_webhook_recovery_rolls_back_session_binding_and_leaves_the_inbox_retryable() {
    let f = Fixture::new().await;
    let (order, plan) = prepared(&f).await;
    let response = session(&plan, "cs_test_early_fail", "pi_early_fail");
    let paid = event(
        &plan,
        "evt_early_fail",
        "checkout.session.completed",
        response.clone(),
        0,
    );
    f.store
        .reconcile_managed_checkout_event(&paid)
        .await
        .unwrap();
    sqlx::query(
        "ALTER TABLE managed_stripe_bank_actions ADD CONSTRAINT injected_failure CHECK(false)",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(f
        .store
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .is_err());
    let session: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM managed_stripe_checkout_operations WHERE order_id=$1",
    )
    .bind(order.order_id)
    .fetch_one(&f.store.pool)
    .await
    .unwrap();
    assert!(session.is_none());
    assert_eq!(count(&f, "managed_stripe_event_inbox").await, 1);
    assert_eq!(count(&f, "managed_stripe_paid_receipts").await, 0);
    sqlx::query("ALTER TABLE managed_stripe_bank_actions DROP CONSTRAINT injected_failure")
        .execute(&f.store.pool)
        .await
        .unwrap();
    let restarted = f.reconnect().await;
    restarted
        .bind_managed_checkout(order.order_id, &plan.session_binding(&response).unwrap())
        .await
        .unwrap();
    assert_eq!(
        state(&restarted, &order).await,
        ManagedOrderStatus::AwaitingBankSettlement
    );
    sqlx::query("DELETE FROM managed_stripe_bank_actions WHERE order_id=$1")
        .bind(order.order_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert!(matches!(
        restarted
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    restarted.pool.close().await;
    f.cleanup().await;
}
