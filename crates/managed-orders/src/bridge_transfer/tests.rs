use super::*;
use proptest::prelude::*;

fn fixture(payout: bool) -> (BridgeTransferPlan, Value) {
    let v: Value = serde_json::from_str(if payout {
        include_str!("../../tests/fixtures/bridge-payout.json")
    } else {
        include_str!("../../tests/fixtures/bridge-conversion.json")
    })
    .unwrap();
    (
        BridgeTransferPlan::restore(v["plan"].clone()).unwrap(),
        v["response"].clone(),
    )
}
fn completed(plan: &BridgeTransferPlan, mut v: Value) -> Value {
    v["state"] = json!("payment_processed");
    v["updated_at"] = json!("2026-10-22T00:01:00Z");
    match plan.route() {
        BridgeRoute::MxnConversion {
            company_source_clabe,
            ..
        } => {
            v["source"]["clabe"] = json!(company_source_clabe);
            v["receipt"] = json!({"initial_amount":"2250.00","initial_amount_currency":"mxn",
                "final_amount":"112.500000","final_amount_currency":"usdc",
                "destination_tx_hash":format!("0x{}","a".repeat(64))});
        }
        BridgeRoute::SupplierMxnPayout { .. } => {
            v["receipt"] = json!({"initial_amount":"102.000000","initial_amount_currency":"usdc",
                "final_amount":"2000.00","final_amount_currency":"mxn"});
        }
    }
    v
}

#[test]
fn plans_are_immutable_exact_and_add_no_second_platform_fee() {
    for payout in [false, true] {
        let (p, _) = fixture(payout);
        let restored = BridgeTransferPlan::restore(serde_json::to_value(&p).unwrap()).unwrap();
        assert!(p == restored);
        assert_eq!(p.digest(), restored.digest());
        let body = p.request(p.created_at()).unwrap();
        assert_eq!(body, p.request(p.created_at() + 3600).unwrap());
        assert_eq!(body["on_behalf_of"], p.company().customer_id.to_string());
        assert!(body.get("developer_fee").is_none());
        assert!(body.get("features").is_none());
        if payout {
            assert_eq!(body["source"]["amount"], "102.000000");
            assert_eq!(body["destination"]["amount"], "2000.00");
            assert_eq!(
                body["return_instructions"]["address"],
                p.company().base_treasury_address
            );
            assert!(body.get("amount").is_none());
        } else {
            assert_eq!(body["amount"], "2250.00");
            assert_eq!(body["destination"]["payment_rail"], "base");
            assert!(body["source"].get("clabe").is_none()); // returned only after deposit
        }
    }
}

#[test]
fn plans_reject_unknown_fields_nil_ids_corrupt_clabe_and_unsafe_amounts() {
    let (p, _) = fixture(false);
    let original = serde_json::to_value(p).unwrap();
    for (pointer, value) in [
        ("/schema", json!("other")),
        ("/operation_id", json!(Uuid::nil())),
        ("/created_at", json!(0)),
        ("/quote_digest", json!("a")),
        ("/company/customer_id", json!(Uuid::nil())),
        (
            "/company/base_treasury_address",
            json!("0x0000000000000000000000000000000000000000"),
        ),
        ("/route/company_source_clabe", json!("999000000000000000")),
        ("/route/input_minor", json!(MAX_EXACT_INTEGER + 1)),
        ("/route/minimum_output_micro_usdc", json!(0)),
    ] {
        let mut v = original.clone();
        *v.pointer_mut(pointer).unwrap() = value;
        assert!(BridgeTransferPlan::restore(v).is_err(), "{pointer}");
    }
    let mut v = original;
    v["allow_any_from_address"] = json!(true);
    assert!(BridgeTransferPlan::restore(v).is_err());
}

#[test]
fn create_window_expires_without_disabling_readback() {
    let (p, v) = fixture(false);
    assert_eq!(p.request(p.created_at() - 1), Err(BridgeError::Expired));
    assert!(p
        .request(p.created_at() + CREATE_WINDOW_SECONDS - 1)
        .is_ok());
    assert_eq!(
        p.request(p.created_at() + CREATE_WINDOW_SECONDS),
        Err(BridgeError::Expired)
    );
    assert!(p.observe(&v, None).is_ok());
}

#[test]
fn decimals_never_round_or_use_floating_point() {
    assert_eq!(parse_decimal("0.000001", 6), Ok(1));
    assert_eq!(parse_decimal("12.340000", 2), Ok(1234));
    assert_eq!(parse_decimal("9007199254.740991", 6), Ok(MAX_EXACT_INTEGER));
    for s in [
        "",
        ".1",
        "1.",
        "+1",
        "-1",
        "1e2",
        " 1",
        "1 ",
        "NaN",
        "1.0000001",
        "9007199254.740992",
        "18446744073709551616",
        "1.2.3",
    ] {
        assert!(parse_decimal(s, 6).is_err(), "{s}");
    }
    assert!(parse_decimal("1.001", 2).is_err());
}
proptest! {
    #[test]
    fn exact_provider_decimal_round_trips(n in 0..=MAX_EXACT_INTEGER, scale in 0..=6u32) {
        let encoded=if scale==0 { n.to_string() } else { decimal(n,scale) };
        prop_assert_eq!(parse_decimal(&encoded,scale),Ok(n));
    }
}

#[test]
fn pending_and_submitted_provider_states_never_report_delivered_money() {
    let (p, mut v) = fixture(false);
    for state in [
        "awaiting_funds",
        "in_review",
        "funds_received",
        "payment_submitted",
        "pending",
    ] {
        v["state"] = json!(state);
        v["source"]["clabe"] = json!(match p.route() {
            BridgeRoute::MxnConversion {
                company_source_clabe,
                ..
            } => company_source_clabe,
            _ => unreachable!(),
        });
        v["receipt"] =
            json!({"destination_tx_hash":format!("0x{}","b".repeat(64)),"final_amount":"112.5"});
        let observation = p.observe(&v, None).unwrap();
        assert_eq!(observation.output(), None);
        assert_eq!(observation.final_tx_hash(), None);
        assert!(!observation.output_matches_commitment());
    }
}

#[test]
fn completed_output_requires_bound_source_amount_currency_and_final_hash() {
    for payout in [false, true] {
        let (p, v) = fixture(payout);
        let v = completed(&p, v);
        let observation = p.observe(&v, None).unwrap();
        assert!(observation.output_matches_commitment());
        assert_eq!(
            observation.output(),
            Some(if payout { 200000 } else { 112500000 })
        );
        for (pointer, value) in [
            ("/client_reference_id", json!(Uuid::new_v4())),
            ("/on_behalf_of", json!(Uuid::new_v4())),
            ("/currency", json!("eur")),
            ("/amount", json!("999999999")),
            ("/developer_fee", json!("0.01")),
            ("/source/payment_rail", json!("ethereum")),
            ("/destination/currency", json!("usdt")),
            ("/receipt/final_amount_currency", json!("usd")),
            ("/receipt/initial_amount_currency", json!("eur")),
            ("/receipt/initial_amount", json!("999999999")),
            ("/state", json!("future_unknown_state")),
            ("/updated_at", json!("2020-01-01T00:00:00Z")),
        ] {
            let mut bad = v.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            assert!(p.observe(&bad, None).is_err(), "{payout} {pointer}");
        }
        assert!(p.observe(&v, Some(Uuid::new_v4())).is_err());
        let mut bad = v;
        if payout {
            bad["destination"]["external_account_id"] = json!(Uuid::new_v4());
        } else {
            bad["receipt"]
                .as_object_mut()
                .unwrap()
                .remove("destination_tx_hash");
        }
        assert!(p.observe(&bad, None).is_err());
    }
}

#[test]
fn shortfall_is_recordable_but_cannot_satisfy_the_obligation() {
    for payout in [false, true] {
        let (p, v) = fixture(payout);
        let mut v = completed(&p, v);
        v["receipt"]["final_amount"] = json!("1.00");
        let short = p.observe(&v, None).unwrap();
        assert!(!short.output_matches_commitment());
        assert_eq!(short.output(), Some(if payout { 100 } else { 1000000 }));
        assert_eq!(short.state(), BridgeTransferState::PaymentProcessed);
        assert_eq!(short.transfer_id().to_string(), v["id"].as_str().unwrap());
    }
}

#[test]
fn deposit_instructions_bind_asset_rail_amount_and_company_source() {
    for payout in [false, true] {
        let (p, v) = fixture(payout);
        assert!(p.observe(&v, None).unwrap().deposit().is_some());
        for (field, value) in [
            ("payment_rail", json!("ethereum")),
            ("currency", json!("usdt")),
            ("amount", json!("1.00")),
        ] {
            let mut bad = v.clone();
            bad["source_deposit_instructions"][field] = value;
            assert!(p.observe(&bad, None).is_err());
        }
        let mut bad = v;
        if payout {
            bad["source_deposit_instructions"]["from_address"] =
                json!("0x3333333333333333333333333333333333333333");
        } else {
            bad["source_deposit_instructions"]["clabe"] = json!("000000000000000000");
        }
        assert!(p.observe(&bad, None).is_err());
    }
}

#[test]
fn late_return_reopens_provider_delivery_and_stale_data_cannot_erase_it() {
    let (p, v) = fixture(true);
    let mut v = completed(&p, v);
    let delivered = p.observe(&v, None).unwrap();
    v["state"] = json!("returned");
    v["updated_at"] = json!("2026-10-23T00:00:00Z");
    let returned = p.observe(&v, None).unwrap();
    assert!(delivered.validate_successor(&returned).is_ok());
    assert!(!returned.output_matches_commitment());
    assert!(returned.state().exception());
    assert_eq!(
        returned.validate_successor(&delivered),
        Err(BridgeError::History)
    );
    assert!(returned.validate_successor(&returned).is_ok());
    v["state"] = json!("payment_processed");
    v["updated_at"] = json!("2026-10-24T00:00:00Z");
    assert_eq!(
        returned.validate_successor(&p.observe(&v, None).unwrap()),
        Err(BridgeError::History)
    );
}

#[test]
fn changed_deposit_and_same_timestamp_conflicts_fail_closed() {
    let (p, mut v) = fixture(true);
    let first = p.observe(&v, None).unwrap();
    v["source_deposit_instructions"]["to_address"] =
        json!("0x3333333333333333333333333333333333333333");
    v["updated_at"] = json!("2026-10-22T00:01:00Z");
    assert_eq!(
        first.validate_successor(&p.observe(&v, None).unwrap()),
        Err(BridgeError::History)
    );
    let (p, mut v) = fixture(true);
    let first = p.observe(&v, None).unwrap();
    v["state"] = json!("funds_received");
    assert_eq!(
        first.validate_successor(&p.observe(&v, None).unwrap()),
        Err(BridgeError::History)
    );
}

#[test]
fn underfunding_is_an_exception_without_automatic_topup() {
    let (p, mut v) = fixture(true);
    v["state"] = json!("underfunded");
    v["additional_funding_instructions"] = json!({"amount":"10.00","destination":{"payment_rail":"bridge_transfer","bridge_transfer_id":Uuid::new_v4()}});
    let o = p.observe(&v, None).unwrap();
    assert!(o.state().exception());
    assert!(!o.output_matches_commitment());
    assert_eq!(
        p.request(p.created_at()).unwrap()["source"]["amount"],
        "102.000000"
    );
}

#[test]
fn processed_receipt_cannot_change_hash_or_output_in_later_readback() {
    let (p, v) = fixture(false);
    let mut v = completed(&p, v);
    let first = p.observe(&v, None).unwrap();
    v["updated_at"] = json!("2026-10-22T00:02:00Z");
    v["receipt"]["destination_tx_hash"] = json!(format!("0x{}", "c".repeat(64)));
    assert_eq!(
        first.validate_successor(&p.observe(&v, None).unwrap()),
        Err(BridgeError::History)
    );
    let (p, v) = fixture(true);
    let mut v = completed(&p, v);
    let first = p.observe(&v, None).unwrap();
    v["updated_at"] = json!("2026-10-22T00:02:00Z");
    v["receipt"]["final_amount"] = json!("1.00");
    assert_eq!(
        first.validate_successor(&p.observe(&v, None).unwrap()),
        Err(BridgeError::History)
    );
}

#[test]
fn beneficiary_digest_is_stable_for_key_order_and_binds_changed_private_fields() {
    let first: Value = serde_json::from_str(r#"{"id":"fixture","nested":{"z":1,"a":2}}"#).unwrap();
    let second: Value = serde_json::from_str(r#"{"nested":{"a":2,"z":1},"id":"fixture"}"#).unwrap();
    assert_eq!(
        external_account_snapshot_digest(&first),
        external_account_snapshot_digest(&second)
    );
    let mut changed = second;
    changed["nested"]["a"] = json!(3);
    assert_ne!(
        external_account_snapshot_digest(&first),
        external_account_snapshot_digest(&changed)
    );
    assert!(external_account_snapshot_digest(&json!([1])).is_err());
}
