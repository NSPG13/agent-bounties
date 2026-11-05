use super::*;
use hmac::{Hmac, Mac};

fn plan() -> ManagedRefundPlan {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let q: ManagedQuote = serde_json::from_value(fixture["quote"].clone()).unwrap();
    ManagedRefundPlan::new(
        &q,
        Uuid::new_v4(),
        Uuid::new_v4(),
        "pi_refund_fixture".into(),
        q.input().expires_at + 1,
    )
    .unwrap()
}
fn object(p: &ManagedRefundPlan, status: &str) -> Value {
    json!({"id":"re_fixture","object":"refund","charge":"ch_fixture","payment_intent":p.payment_intent_id(),
        "amount":p.amount_minor(),"currency":currency(p.currency()),"created":p.created_at(),"status":status,
        "metadata":p.request(p.created_at(),p.merchant()).unwrap().body["metadata"]})
}
fn signed(p: &ManagedRefundPlan, object: Value, kind: &str) -> (Vec<u8>, String) {
    let wire = json!({"id":"evt_refund_fixture","object":"event","type":kind,"livemode":p.merchant().livemode,
        "created":p.created_at()+1,"data":{"object":object}});
    let bytes = serde_json::to_vec(&wire).unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"refund-fixture-secret").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&bytes);
    (
        bytes,
        format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes())),
    )
}

#[test]
fn full_original_payment_refund_includes_fee_and_works_after_offer_expiry() {
    let p = plan();
    let r = p.request(p.created_at(), p.merchant()).unwrap();
    assert_eq!(r.endpoint, "/v1/refunds");
    assert_eq!(r.body["amount"], p.amount_minor());
    assert_eq!(r.body["payment_intent"], "pi_refund_fixture");
    assert_eq!(r.body["reason"], "requested_by_customer");
    assert!(r.body.get("destination").is_none());
    assert!(r.body.get("refund_application_fee").is_none());
    assert_eq!(
        r.body,
        p.request(p.created_at() + 3600, p.merchant()).unwrap().body
    );
    assert_eq!(
        r.idempotency_key,
        p.request(p.created_at() + 3600, p.merchant())
            .unwrap()
            .idempotency_key
    );
}

#[test]
fn expired_retry_and_changed_merchant_do_not_issue_a_new_refund() {
    let p = plan();
    assert!(matches!(
        p.request(p.created_at() - 1, p.merchant()),
        Err(RefundError::ReconciliationRequired)
    ));
    assert!(matches!(
        p.request(p.created_at() + 23 * 3600, p.merchant()),
        Err(RefundError::ReconciliationRequired)
    ));
    let mut wrong = p.merchant().clone();
    wrong.livemode = !wrong.livemode;
    assert!(matches!(
        p.request(p.created_at(), &wrong),
        Err(RefundError::Binding)
    ));
    assert!(p.binding(&object(&p, "succeeded")).is_ok());
}

#[test]
fn restored_plan_recomputes_request_and_original_payment_binding() {
    let p = plan();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let q: ManagedQuote = serde_json::from_value(fixture["quote"].clone()).unwrap();
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        ManagedRefundPlan::restore(v.clone(), &q, p.order_id(), p.payment_intent_id()).unwrap(),
        p
    );
    for pointer in ["/body/amount", "/amount_minor"] {
        let mut bad = v.clone();
        *bad.pointer_mut(pointer).unwrap() = json!(1);
        assert!(ManagedRefundPlan::restore(bad, &q, p.order_id(), p.payment_intent_id()).is_err());
    }
    assert!(ManagedRefundPlan::restore(v, &q, p.order_id(), "pi_different").is_err());
}

#[test]
fn api_binding_accepts_refund_objects_without_livemode_but_checks_every_original_binding() {
    let p = plan();
    let v = object(&p, "pending");
    assert!(p.binding(&v).is_ok());
    for (pointer, value) in [
        ("/id", json!("ch_not_refund")),
        ("/charge", json!("pi_wrong_type")),
        ("/payment_intent", json!("pi_another")),
        ("/amount", json!(1)),
        ("/currency", json!("eur")),
        ("/metadata/merchant_account_id", json!("acct_another")),
        ("/metadata/managed_quote_digest", json!("f".repeat(64))),
        ("/created", json!(p.created_at() - 1)),
        ("/status", json!("future_unknown")),
    ] {
        let mut bad = v.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        assert!(p.binding(&bad).is_err(), "{pointer}");
    }
    let mut bad = v;
    bad["livemode"] = json!(!p.merchant().livemode);
    assert!(p.binding(&bad).is_err());
}

#[test]
fn native_signed_events_support_all_documented_statuses_without_making_api_responses_evidence() {
    let p = plan();
    for status in [
        "pending",
        "requires_action",
        "succeeded",
        "failed",
        "canceled",
    ] {
        let (body, sig) = signed(
            &p,
            object(&p, status),
            if status == "failed" {
                "refund.failed"
            } else {
                "refund.updated"
            },
        );
        let e = verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", p.merchant())
            .unwrap();
        assert!(p.matches_event(e.data()));
        assert_eq!(serde_json::to_value(e.data().outcome).unwrap(), status);
    }
}

#[test]
fn forged_wrong_mode_and_inconsistent_failed_events_are_rejected() {
    let p = plan();
    let (body, sig) = signed(&p, object(&p, "succeeded"), "refund.updated");
    assert!(matches!(
        verify_managed_refund_event(&body, &sig, b"wrong-secret", p.merchant()),
        Err(RefundError::Authentication)
    ));
    let mut merchant = p.merchant().clone();
    merchant.livemode = !merchant.livemode;
    assert!(verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", &merchant).is_err());
    let (body, sig) = signed(&p, object(&p, "pending"), "refund.failed");
    assert!(matches!(
        verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", p.merchant()),
        Err(RefundError::Event)
    ));
    let (body, sig) = signed(&p, object(&p, "pending"), "charge.refunded");
    assert!(
        verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", p.merchant()).is_err()
    );
}

#[test]
fn authenticated_but_unrelated_refund_cannot_match_order() {
    let p = plan();
    let mut v = object(&p, "succeeded");
    v["amount"] = json!(1);
    let (body, sig) = signed(&p, v, "refund.updated");
    let e =
        verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", p.merchant()).unwrap();
    assert!(!p.matches_event(e.data()));
}

#[test]
fn deposit_refund_returns_only_the_deposit_line_under_its_own_purpose_and_key() {
    let full = plan();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let fixture_quote: ManagedQuote = serde_json::from_value(fixture["quote"].clone()).unwrap();
    assert_eq!(
        ManagedRefundPlan::review_deposit(
            &fixture_quote,
            Uuid::new_v4(),
            Uuid::new_v4(),
            "pi_refund_fixture".into(),
            fixture_quote.input().expires_at + 1,
        ),
        Err(RefundError::Binding),
        "a quote without a deposit line has no deposit to refund"
    );
    let mut input = fixture_quote.input().clone();
    input.additional_costs.push(crate::AdditionalCost {
        code: crate::pricing::REVIEW_DEPOSIT_CODE.into(),
        label: "Review deposit (refundable, 10% of solver reward)".into(),
        amount_minor: 1_870,
    });
    let q = ManagedQuote::new(input).unwrap();
    let id = Uuid::new_v4();
    let deposit = ManagedRefundPlan::review_deposit(
        &q,
        id,
        Uuid::new_v4(),
        "pi_refund_fixture".into(),
        q.input().expires_at + 1,
    );
    let deposit = deposit.unwrap();
    let r = deposit
        .request(deposit.created_at(), deposit.merchant())
        .unwrap();
    assert_eq!(deposit.kind(), RefundKind::ReviewDeposit);
    assert_eq!(deposit.amount_minor(), 1_870);
    assert_eq!(r.body["amount"], deposit.amount_minor());
    assert_eq!(
        r.body["metadata"]["purpose"],
        MANAGED_DEPOSIT_REFUND_PURPOSE
    );
    assert!(r.idempotency_key.contains(":deposit-refund:"));
    // The stored document restores only as a deposit refund.
    let stored = serde_json::to_value(&deposit).unwrap();
    assert_eq!(stored["kind"], "review_deposit");
    assert_eq!(
        ManagedRefundPlan::restore(stored, &q, id, "pi_refund_fixture").unwrap(),
        deposit
    );
    // A full refund's document is unchanged by the new kind.
    assert!(serde_json::to_value(&full).unwrap().get("kind").is_none());
    // Its signed events carry the kind and never match the full refund.
    let (body, sig) = signed(&deposit, object(&deposit, "succeeded"), "refund.updated");
    let event =
        verify_managed_refund_event(&body, &sig, b"refund-fixture-secret", deposit.merchant())
            .unwrap();
    assert_eq!(event.data().kind, RefundKind::ReviewDeposit);
    assert!(!full.matches_event(event.data()));
}

#[test]
fn recovered_bounty_refund_preserves_original_rounding_and_excludes_every_fee() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../site/fixtures/managed-service-quote.json"
    ))
    .unwrap();
    let original: ManagedQuote = serde_json::from_value(fixture["quote"].clone()).unwrap();
    for (currency, numerator) in [(FiatCurrency::Usd, 101), (FiatCurrency::Mxn, 1937)] {
        let mut input = original.input().clone();
        input.currency = currency;
        input.price_basis.minor_units_per_usdc_numerator = numerator;
        input.solver_reward_micro_usdc = 10_000_001;
        input.verifier_reserve_micro_usdc = 500_001;
        input.additional_costs = vec![
            crate::AdditionalCost {
                code: "review_deposit".into(),
                label: "Review deposit".into(),
                amount_minor: 97,
            },
            crate::AdditionalCost {
                code: "currency_conversion".into(),
                label: "Conversion".into(),
                amount_minor: 11,
            },
            crate::AdditionalCost {
                code: "payment_processing".into(),
                label: "Processing".into(),
                amount_minor: 29,
            },
            crate::AdditionalCost {
                code: "service_tax".into(),
                label: "Tax".into(),
                amount_minor: 13,
            },
        ];
        let q = ManagedQuote::new(input).unwrap();
        let id = Uuid::new_v4();
        let op = Uuid::new_v4();
        let p = ManagedRefundPlan::recovered_bounty(
            &q,
            id,
            op,
            "pi_recovered".into(),
            q.input().expires_at + 1,
        )
        .unwrap();
        let expected = q.lines()[0].amount_minor + q.lines()[1].amount_minor;
        assert_eq!(p.amount_minor(), expected);
        assert_eq!(
            q.total_minor() - expected,
            q.lines()[2].amount_minor + 97 + 11 + 29 + 13
        );
        assert_eq!(p.currency(), currency);
        assert_eq!(p.kind(), RefundKind::RecoveredBounty);
        let request = p.request(p.created_at(), p.merchant()).unwrap();
        assert!(request
            .idempotency_key
            .contains(":recovered-bounty-refund:"));
        assert_eq!(
            request.body["metadata"]["purpose"],
            MANAGED_RECOVERED_REFUND_PURPOSE
        );
        let encoded = serde_json::to_value(&p).unwrap();
        assert_eq!(
            ManagedRefundPlan::restore(encoded.clone(), &q, id, "pi_recovered").unwrap(),
            p
        );
        let mut corrupt = encoded;
        corrupt["amount_minor"] = json!(q.total_minor());
        corrupt["body"]["amount"] = json!(q.total_minor());
        assert!(ManagedRefundPlan::restore(corrupt, &q, id, "pi_recovered").is_err());
        let (bytes, header) = signed(&p, object(&p, "succeeded"), "refund.updated");
        let authenticated =
            verify_managed_refund_event(&bytes, &header, b"refund-fixture-secret", p.merchant())
                .unwrap();
        assert!(p.matches_event(authenticated.data()));
    }
}
