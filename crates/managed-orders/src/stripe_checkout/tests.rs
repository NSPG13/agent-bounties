use super::*;
use crate::{PriceBasis, QuoteInput, ServiceTerms};
use hmac::{Hmac, Mac};

fn setup() -> (ManagedQuote, CheckoutPolicy, ManagedCheckoutPlan) {
    let now = chrono::Utc::now().timestamp();
    let quote = ManagedQuote::new(QuoteInput {
        quote_id: Uuid::new_v4(),
        buyer_principal: "account:fixture".into(),
        stripe_account_id: "acct_fixture".into(),
        livemode: false,
        issued_at: now - 1,
        expires_at: now + 7200,
        terms: ServiceTerms {
            deliverable: "Fixture deliverable".into(),
            acceptance_criteria: "Fixture criteria".into(),
            delivery_deadline: now + 86400,
            terms_version: "fixture-v1".into(),
            remedies: "Fixture remedies".into(),
        },
        currency: FiatCurrency::Mxn,
        price_basis: PriceBasis {
            reference: "fixture-price".into(),
            minor_units_per_usdc_numerator: 2000,
            minor_units_per_usdc_denominator: 1,
            valid_until: now + 7200,
        },
        solver_reward_micro_usdc: 100_000_000,
        verifier_reserve_micro_usdc: 5_000_000,
        additional_costs: vec![],
        bounty: managed_orders_bounty(now + 86400),
    })
    .unwrap();
    let policy = CheckoutPolicy {
        stripe_account_id: "acct_fixture".into(),
        livemode: false,
        currency: FiatCurrency::Mxn,
        minimum_minor: 1000,
        maximum_minor: 1_000_000,
        payment_method_configuration: Some("pmc_fixture".into()),
        adaptive_pricing: false,
    };
    let plan =
        ManagedCheckoutPlan::new(&quote, Uuid::new_v4(), Uuid::new_v4(), policy.clone(), now)
            .unwrap();
    (quote, policy, plan)
}

fn session(plan: &ManagedCheckoutPlan) -> Value {
    let request = plan.request(plan.created_at(), plan.policy()).unwrap();
    json!({ "id":"cs_test_fixture", "object":"checkout.session", "mode":"payment", "livemode":false,
        "metadata":request.body["metadata"], "client_reference_id":plan.order_id(), "amount_total":225000,
        "currency":"mxn", "status":"complete", "payment_status":"paid", "payment_intent":"pi_fixture",
        "expires_at": plan.expires_at(), "url":"https://checkout.stripe.com/c/pay/cs_test_fixture" })
}
fn wire(session: Value) -> Value {
    json!({ "id":"evt_fixture", "object":"event", "type":"checkout.session.completed", "livemode":false,
        "created":chrono::Utc::now().timestamp(), "data":{"object":session} })
}
fn signed(wire: &Value) -> (Vec<u8>, String) {
    let body = serde_json::to_vec(wire).unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&body);
    (
        body,
        format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes())),
    )
}
fn verify(wire: &Value) -> Result<AuthenticatedManagedEvent, CheckoutError> {
    let (body, signature) = signed(wire);
    verify_managed_checkout_event(&body, &signature, b"fixture-secret", "acct_fixture", false)
}

#[test]
fn checkout_is_itemized_fixed_price_and_retries_reuse_identical_bytes_and_key() {
    let (quote, policy, plan) = setup();
    let first = plan.request(plan.created_at(), &policy).unwrap();
    let second = plan.request(plan.created_at() + 10, &policy).unwrap();
    assert_eq!(first.body, second.body);
    assert_eq!(first.idempotency_key, second.idempotency_key);
    assert_eq!(first.api_version, "2026-09-30.endive");
    assert_eq!(
        first.body["line_items"][0]["price_data"]["unit_amount"],
        200000
    );
    assert_eq!(
        first.body["line_items"][1]["price_data"]["unit_amount"],
        10000
    );
    assert_eq!(
        first.body["line_items"][2]["price_data"]["unit_amount"],
        15000
    );
    assert_eq!(first.body["adaptive_pricing"]["enabled"], false);
    assert_eq!(first.body["payment_method_configuration"], "pmc_fixture");
    for key in [
        "payment_method_types",
        "automatic_tax",
        "customer",
        "setup_future_usage",
    ] {
        assert!(first.body.get(key).is_none());
    }
    assert_eq!(quote.input().solver_reward_micro_usdc, 100_000_000);
    assert_eq!(
        first.body["payment_intent_data"]["metadata"],
        first.body["metadata"]
    );
    let restored = ManagedCheckoutPlan::restore(
        serde_json::to_value(&plan).unwrap(),
        &quote,
        plan.order_id(),
    )
    .unwrap();
    assert_eq!(restored, plan);
}

#[test]
fn adaptive_pricing_is_explicit_and_binding_stays_in_the_settlement_currency() {
    let (quote, policy, plan) = setup();
    // A plan stored before the flag existed restores to the same bytes, disabled.
    let mut stored = serde_json::to_value(&plan).unwrap();
    stored["policy"]
        .as_object_mut()
        .unwrap()
        .remove("adaptive_pricing");
    assert_eq!(
        ManagedCheckoutPlan::restore(stored, &quote, plan.order_id()).unwrap(),
        plan
    );
    let adaptive = CheckoutPolicy {
        adaptive_pricing: true,
        ..policy
    };
    let plan = ManagedCheckoutPlan::new(
        &quote,
        plan.order_id(),
        plan.operation_id(),
        adaptive.clone(),
        plan.created_at(),
    )
    .unwrap();
    let request = plan.request(plan.created_at(), &adaptive).unwrap();
    assert_eq!(request.body["adaptive_pricing"]["enabled"], true);
    // Stripe keeps the session in MXN and adds the customer's currency separately.
    let mut response = session(&plan);
    response["presentment_details"] =
        json!({"presentment_amount": 12_345, "presentment_currency": "usd"});
    plan.session_binding(&response).unwrap();
    response["currency"] = "usd".into();
    assert!(plan.session_binding(&response).is_err());
    let restored = ManagedCheckoutPlan::restore(
        serde_json::to_value(&plan).unwrap(),
        &quote,
        plan.order_id(),
    )
    .unwrap();
    assert_eq!(restored, plan);
}

#[test]
fn stale_requests_and_changed_policy_need_reconciliation_and_cannot_extend_the_offer() {
    let (quote, mut policy, plan) = setup();
    for now in [
        plan.created_at() - 1,
        plan.expires_at() - 1800,
        plan.created_at() + 86400,
    ] {
        assert_eq!(
            plan.request(now, &policy).unwrap_err(),
            CheckoutError::ReconciliationRequired
        );
    }
    policy.stripe_account_id = "acct_other".into();
    assert_eq!(
        plan.request(plan.created_at(), &policy).unwrap_err(),
        CheckoutError::Binding
    );
    let mut input = quote.input().clone();
    input.expires_at = plan.created_at() + 1800;
    let short = ManagedQuote::new(input).unwrap();
    assert_eq!(
        ManagedCheckoutPlan::new(
            &short,
            plan.order_id(),
            Uuid::new_v4(),
            plan.policy().clone(),
            plan.created_at()
        )
        .unwrap_err(),
        CheckoutError::Expired
    );
    let mut changed = serde_json::to_value(&plan).unwrap();
    changed["body"]["line_items"][0]["price_data"]["unit_amount"] = 1.into();
    assert!(ManagedCheckoutPlan::restore(changed, &quote, plan.order_id()).is_err());
}

#[test]
fn provider_session_binding_rejects_substituted_identity_money_mode_or_redirect() {
    let (_, _, plan) = setup();
    let original = session(&plan);
    assert!(plan.session_binding(&original).is_ok());
    for (pointer, value) in [
        ("/id", json!("cs_live_wrong")),
        ("/amount_total", json!(1)),
        ("/currency", json!("usd")),
        ("/livemode", json!(true)),
        ("/metadata/managed_quote_digest", json!("0".repeat(64))),
        ("/client_reference_id", json!(Uuid::new_v4())),
        ("/expires_at", json!(plan.expires_at() + 1)),
        (
            "/url",
            json!("https://checkout.stripe.com.evil.test/c/pay/anything"),
        ),
    ] {
        let mut altered = original.clone();
        *altered.pointer_mut(pointer).unwrap() = value;
        assert!(plan.session_binding(&altered).is_err(), "{pointer}");
    }
}

#[test]
fn raw_signature_account_and_mode_are_authenticated_before_receipt_creation() {
    let (_, _, plan) = setup();
    let event = wire(session(&plan));
    let (body, signature) = signed(&event);
    assert!(verify(&event).is_ok());
    assert_eq!(
        verify_managed_checkout_event(&body, &signature, b"wrong-secret", "acct_fixture", false)
            .unwrap_err(),
        CheckoutError::Authentication
    );
    assert!(verify_managed_checkout_event(
        &body,
        &signature,
        b"fixture-secret",
        "acct_other",
        false
    )
    .is_err());
    assert!(verify_managed_checkout_event(
        &body,
        &signature,
        b"fixture-secret",
        "acct_fixture",
        true
    )
    .is_err());
    let mut altered = body.clone();
    altered.push(b' ');
    assert!(verify_managed_checkout_event(
        &altered,
        &signature,
        b"fixture-secret",
        "acct_fixture",
        false
    )
    .is_err());
    let mut connected = event;
    connected["account"] = "acct_connected".into();
    assert!(verify(&connected).is_err());
}

#[test]
fn only_consistent_native_states_produce_paid_pending_failed_or_expired_observations() {
    let (_, _, plan) = setup();
    let original = wire(session(&plan));
    for (event_type, status, payment_status, outcome) in [
        (
            "checkout.session.completed",
            "complete",
            "paid",
            ManagedPaymentOutcome::Paid,
        ),
        (
            "checkout.session.completed",
            "complete",
            "unpaid",
            ManagedPaymentOutcome::Pending,
        ),
        (
            "checkout.session.async_payment_succeeded",
            "complete",
            "paid",
            ManagedPaymentOutcome::Paid,
        ),
        (
            "checkout.session.async_payment_failed",
            "complete",
            "unpaid",
            ManagedPaymentOutcome::Failed,
        ),
        (
            "checkout.session.expired",
            "expired",
            "unpaid",
            ManagedPaymentOutcome::Expired,
        ),
    ] {
        let mut event = original.clone();
        event["type"] = event_type.into();
        event["data"]["object"]["status"] = status.into();
        event["data"]["object"]["payment_status"] = payment_status.into();
        assert_eq!(verify(&event).unwrap().data().outcome, outcome);
    }
    for (event_type, status, payment_status) in [
        ("checkout.session.completed", "open", "paid"),
        (
            "checkout.session.async_payment_succeeded",
            "complete",
            "unpaid",
        ),
        (
            "checkout.session.completed",
            "complete",
            "no_payment_required",
        ),
        ("checkout.session.async_payment_failed", "complete", "paid"),
    ] {
        let mut event = original.clone();
        event["type"] = event_type.into();
        event["data"]["object"]["status"] = status.into();
        event["data"]["object"]["payment_status"] = payment_status.into();
        assert!(verify(&event).is_err());
    }
}

#[test]
fn signed_events_from_other_products_or_invalid_object_fields_cannot_enter_managed_orders() {
    let (_, _, plan) = setup();
    let original = wire(session(&plan));
    for (pointer, value) in [
        (
            "/data/object/metadata/purpose",
            json!("platform_balance_top_up"),
        ),
        (
            "/data/object/metadata/managed_operation_id",
            json!(Uuid::nil()),
        ),
        ("/data/object/metadata/managed_quote_digest", json!("wrong")),
        ("/data/object/payment_intent", Value::Null),
        ("/data/object/amount_total", json!(-1)),
        ("/data/object/livemode", json!(true)),
        ("/data/object/mode", json!("subscription")),
    ] {
        let mut event = original.clone();
        *event.pointer_mut(pointer).unwrap() = value;
        assert!(verify(&event).is_err(), "{pointer}");
    }
}

fn managed_orders_bounty(funding_deadline: i64) -> crate::BountyCommitment {
    crate::BountyCommitment::test_fixture(false, funding_deadline)
}
