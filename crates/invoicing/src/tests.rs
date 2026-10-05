use super::*;
use proptest::prelude::*;

const ORDER: &str = "ord_01";
const TREASURY: &str = "0x7a11000000000000000000000000000000000001";
const BOUNTY: &str = "0xb0b0000000000000000000000000000000000002";
const BOUNTY_ID: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const SOLVER: &str = "0x5010000000000000000000000000000000000003";
const TERMS: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";

fn policy() -> InvoicePolicy {
    InvoicePolicy {
        platform_fee_bps: 750,
        processing_fee_bps: 0,
        processing_fee_fixed_cents: 0,
        max_invoice_cents: 5_000_000,
        days_until_due: 14,
        livemode: false,
    }
}

fn quote() -> InvoiceQuote {
    // $100 solver reward and $10 verifier reward.
    quote_invoice_order(&policy(), "Fix the flaky CI job", 10_000, 1_000).unwrap()
}

fn paid(event_id: &str) -> OrderEvent {
    OrderEvent::InvoicePaid(InvoicePaidEvidence {
        stripe_event_id: event_id.to_string(),
        invoice_id: "in_1".to_string(),
        order_id: ORDER.to_string(),
        status: "paid".to_string(),
        amount_paid_cents: quote().invoice_total_cents,
        amount_remaining_cents: 0,
        currency: "usd".to_string(),
        livemode: false,
    })
}

fn through_funding() -> Vec<OrderEvent> {
    let quote = quote();
    vec![
        OrderEvent::Quoted {
            quote: quote.clone(),
            terms_sha256: TERMS.to_string(),
        },
        OrderEvent::InvoiceIssued {
            invoice_id: "in_1".to_string(),
            total_cents: quote.invoice_total_cents,
        },
        paid("evt_paid"),
        OrderEvent::FundingPlanned {
            treasury: TREASURY.to_string(),
            bounty_id: BOUNTY_ID.to_string(),
            bounty_contract: BOUNTY.to_string(),
            funding_target_usdc: quote.funding_target_usdc,
        },
        OrderEvent::FundingObserved {
            bounty_contract: BOUNTY.to_uppercase().replace("0X", "0x"),
            contributor: TREASURY.to_string(),
            amount_usdc: quote.funding_target_usdc,
            log_key: "84532:0xfund:0".to_string(),
        },
    ]
}

fn project(events: &[OrderEvent]) -> Result<OrderState> {
    project_order(ORDER, &policy(), events)
}

#[test]
fn quote_escrows_the_exact_v2_target_and_invoices_whole_cents() {
    let quote = quote();
    assert_eq!(quote.solver_reward_usdc, 100_000_000);
    assert_eq!(quote.verifier_reward_usdc, 10_000_000);
    assert_eq!(quote.platform_fee_usdc, 7_500_000);
    assert_eq!(quote.funding_target_usdc, 117_500_000);
    assert_eq!(quote.invoice_total_cents, 11_750);
    assert_eq!(
        quote
            .line_items
            .iter()
            .map(|line| (line.code.as_str(), line.amount_cents))
            .collect::<Vec<_>>(),
        [("outcome", 11_000), ("platform_fee", 750)]
    );
    assert_eq!(
        quote.line_items[1].description,
        "Platform fee (7.50% of the solver reward)"
    );

    // A fee that is not whole cents rounds up on the invoice only; escrow stays exact.
    let odd = quote_invoice_order(&policy(), "Odd", 1, 1).unwrap();
    assert_eq!(odd.platform_fee_usdc, 750);
    assert_eq!(odd.funding_target_usdc, 20_750);
    assert_eq!(odd.invoice_total_cents, 3);
}

#[test]
fn processing_pass_through_and_caps_are_enforced() {
    let mut with_processing = policy();
    with_processing.processing_fee_bps = 80;
    with_processing.processing_fee_fixed_cents = 30;
    let quote = quote_invoice_order(&with_processing, "Task", 10_000, 1_000).unwrap();
    // ceil(11_750 * 0.8%) = 94, plus 30 fixed.
    assert_eq!(quote.line_items[2].amount_cents, 124);
    assert_eq!(quote.invoice_total_cents, 11_874);
    assert_eq!(
        quote.funding_target_usdc, 117_500_000,
        "processing is never escrowed"
    );

    let mut capped = policy();
    capped.max_invoice_cents = 11_749;
    assert!(quote_invoice_order(&capped, "Task", 10_000, 1_000).is_err());
    for bad in [
        InvoicePolicy {
            platform_fee_bps: 1_001,
            ..policy()
        },
        InvoicePolicy {
            days_until_due: 0,
            ..policy()
        },
        InvoicePolicy {
            max_invoice_cents: 0,
            ..policy()
        },
    ] {
        assert!(quote_invoice_order(&bad, "Task", 1, 1).is_err());
    }
    assert!(quote_invoice_order(&policy(), " ", 1, 1).is_err());
    assert!(quote_invoice_order(&policy(), "Task", 0, 1).is_err());
    assert!(quote_invoice_order(&policy(), "Task", 1, 0).is_err());
    assert!(quote_invoice_order(&policy(), "Task", u64::MAX, 1).is_err());
}

proptest! {
    #[test]
    fn invoice_always_covers_the_escrow(solver in 1u64..10_000_000, verifier in 1u64..10_000_000, bps in 0u16..=1_000) {
        let policy = InvoicePolicy { platform_fee_bps: bps, max_invoice_cents: u64::MAX, ..policy() };
        let quote = quote_invoice_order(&policy, "Task", solver, verifier).unwrap();
        prop_assert_eq!(
            quote.funding_target_usdc,
            quote.solver_reward_usdc + quote.verifier_reward_usdc + quote.platform_fee_usdc
        );
        prop_assert!(quote.invoice_total_cents * USDC_BASE_UNITS_PER_CENT >= quote.funding_target_usdc);
        // The invoice never over-collects by a cent or more for escrowed amounts.
        prop_assert!(quote.invoice_total_cents * USDC_BASE_UNITS_PER_CENT < quote.funding_target_usdc + USDC_BASE_UNITS_PER_CENT);
    }
}

#[test]
fn happy_path_reaches_settlement_with_a_reportable_payment() {
    let mut events = through_funding();
    assert_eq!(project(&events).unwrap().status, OrderStatus::Funded);
    events.push(OrderEvent::SettlementObserved {
        bounty_contract: BOUNTY.to_string(),
        solver: SOLVER.to_string(),
        reportable_usdc: 100_000_000,
        settled_at: 1_780_000_000,
        log_key: "84532:0xsettle:1".to_string(),
    });
    let state = project(&events).unwrap();
    assert_eq!(state.status, OrderStatus::Settled);
    assert_eq!(state.paid_cents, 11_750);
    assert_eq!(state.settlement.unwrap().solver, SOLVER);
    assert_eq!(state.next_action, "record_contractor_payment");
}

#[test]
fn every_step_names_the_next_action() {
    let events = through_funding();
    let actions: Vec<String> = (1..=events.len())
        .map(|count| project(&events[..count]).unwrap().next_action)
        .collect();
    assert_eq!(
        actions,
        [
            "issue_invoice",
            "await_verified_invoice_paid",
            "plan_treasury_funding",
            "sign_treasury_funding_then_await_funding_added",
            "await_bounty_settled"
        ]
    );
}

#[test]
fn payment_evidence_must_match_the_invoice_exactly() {
    let base = &through_funding()[..2];
    let variants: Vec<Box<dyn Fn(&mut InvoicePaidEvidence)>> = vec![
        Box::new(|evidence| evidence.amount_paid_cents -= 1),
        Box::new(|evidence| evidence.amount_remaining_cents = 1),
        Box::new(|evidence| evidence.status = "open".to_string()),
        Box::new(|evidence| evidence.currency = "eur".to_string()),
        Box::new(|evidence| evidence.livemode = true),
        Box::new(|evidence| evidence.invoice_id = "in_other".to_string()),
        Box::new(|evidence| evidence.order_id = "ord_other".to_string()),
    ];
    for mutate in variants {
        let OrderEvent::InvoicePaid(mut evidence) = paid("evt_paid") else {
            unreachable!()
        };
        mutate(&mut evidence);
        let mut events = base.to_vec();
        events.push(OrderEvent::InvoicePaid(evidence));
        assert!(project(&events).is_err());
    }
}

#[test]
fn funding_is_planned_only_after_payment_and_observed_only_from_the_treasury() {
    let events = through_funding();
    let mut unpaid = events[..2].to_vec();
    unpaid.push(events[3].clone());
    assert!(
        project(&unpaid).is_err(),
        "no funding plan before invoice.paid"
    );

    let mut wrong_target = events[..3].to_vec();
    let OrderEvent::FundingPlanned {
        treasury,
        bounty_id,
        bounty_contract,
        funding_target_usdc,
    } = events[3].clone()
    else {
        unreachable!()
    };
    wrong_target.push(OrderEvent::FundingPlanned {
        treasury: treasury.clone(),
        bounty_id: bounty_id.clone(),
        bounty_contract: bounty_contract.clone(),
        funding_target_usdc: funding_target_usdc - 1,
    });
    assert!(project(&wrong_target).is_err());

    for (contributor, amount, contract) in [
        (SOLVER, funding_target_usdc, BOUNTY),
        (TREASURY, funding_target_usdc - 1, BOUNTY),
        (TREASURY, funding_target_usdc, SOLVER),
    ] {
        let mut observed = events[..4].to_vec();
        observed.push(OrderEvent::FundingObserved {
            bounty_contract: contract.to_string(),
            contributor: contributor.to_string(),
            amount_usdc: amount,
            log_key: "84532:0xfund:0".to_string(),
        });
        assert!(project(&observed).is_err());
    }
}

#[test]
fn duplicate_or_replayed_evidence_fails_closed() {
    let mut events = through_funding();
    events.insert(3, paid("evt_paid"));
    assert!(project(&events).is_err(), "the same Stripe event twice");

    let mut second_quote = through_funding();
    second_quote.insert(
        1,
        OrderEvent::Quoted {
            quote: quote(),
            terms_sha256: TERMS.to_string(),
        },
    );
    assert!(project(&second_quote).is_err());
    assert!(project(&[]).is_err());
    assert!(
        project(&through_funding()[1..]).is_err(),
        "log must start with a quote"
    );

    let wrong_fee = InvoicePolicy {
        platform_fee_bps: 500,
        ..policy()
    };
    assert!(project_order(ORDER, &wrong_fee, &through_funding()).is_err());
}

#[test]
fn cancellation_before_funding_refunds_the_buyer_once() {
    let mut events = through_funding()[..3].to_vec();
    events.push(OrderEvent::CancellationRequested {
        reason: "buyer withdrew".to_string(),
    });
    let state = project(&events).unwrap();
    assert_eq!(state.refund_due_cents(), 11_750);
    assert_eq!(state.next_action, "issue_credit_note_refund");
    let credit = |refund_cents: u64, event_id: &str| OrderEvent::CreditNoteIssued {
        credit_note_id: "cn_1".to_string(),
        invoice_id: "in_1".to_string(),
        refund_cents,
        stripe_event_id: event_id.to_string(),
        livemode: false,
    };
    let mut partial = events.clone();
    partial.push(credit(11_749, "evt_cn"));
    assert!(project(&partial).is_err());
    events.push(credit(11_750, "evt_cn"));
    let state = project(&events).unwrap();
    assert_eq!(state.status, OrderStatus::Refunded);
    assert_eq!(state.refund_due_cents(), 0);
    events.push(credit(11_750, "evt_cn_2"));
    assert!(project(&events).is_err(), "no second refund");
}

#[test]
fn cancellation_after_funding_waits_for_the_treasury_refund() {
    let mut events = through_funding();
    events.push(OrderEvent::CancellationRequested {
        reason: "scope changed".to_string(),
    });
    let state = project(&events).unwrap();
    assert_eq!(state.refund_due_cents(), 0, "escrow is still at risk");
    assert_eq!(
        state.next_action,
        "cancel_bounty_then_withdraw_treasury_refund"
    );
    let mut early_credit = events.clone();
    early_credit.push(OrderEvent::CreditNoteIssued {
        credit_note_id: "cn_1".to_string(),
        invoice_id: "in_1".to_string(),
        refund_cents: 11_750,
        stripe_event_id: "evt_cn".to_string(),
        livemode: false,
    });
    assert!(project(&early_credit).is_err());

    let mut short_refund = events.clone();
    short_refund.push(OrderEvent::RefundObserved {
        bounty_contract: BOUNTY.to_string(),
        contributor: TREASURY.to_string(),
        amount_usdc: quote().funding_target_usdc - 1,
        log_key: "84532:0xrefund:0".to_string(),
    });
    assert!(project(&short_refund).is_err());

    events.push(OrderEvent::RefundObserved {
        bounty_contract: BOUNTY.to_string(),
        contributor: TREASURY.to_string(),
        amount_usdc: quote().funding_target_usdc,
        log_key: "84532:0xrefund:0".to_string(),
    });
    assert_eq!(project(&events).unwrap().refund_due_cents(), 11_750);
}

#[test]
fn a_settlement_that_beats_cancellation_wins() {
    let mut events = through_funding();
    events.push(OrderEvent::CancellationRequested {
        reason: "late cancel".to_string(),
    });
    events.push(OrderEvent::SettlementObserved {
        bounty_contract: BOUNTY.to_string(),
        solver: SOLVER.to_string(),
        reportable_usdc: 100_000_000,
        settled_at: 1_780_000_000,
        log_key: "84532:0xsettle:1".to_string(),
    });
    let state = project(&events).unwrap();
    assert_eq!(state.status, OrderStatus::Settled);
    assert_eq!(state.refund_due_cents(), 0);
}

#[test]
fn unpaid_invoices_are_voided_not_cancelled() {
    let mut events = through_funding()[..2].to_vec();
    let mut cancelled = events.clone();
    cancelled.push(OrderEvent::CancellationRequested {
        reason: "no".to_string(),
    });
    assert!(project(&cancelled).is_err());
    events.push(OrderEvent::InvoiceVoided {
        invoice_id: "in_1".to_string(),
        stripe_event_id: "evt_void".to_string(),
    });
    assert_eq!(project(&events).unwrap().status, OrderStatus::Voided);
    events.push(paid("evt_paid"));
    assert!(
        project(&events).is_err(),
        "a voided invoice never becomes fundable"
    );
}

fn contractor(kind: TaxFormKind, tin_matched: bool) -> ContractorRecord {
    ContractorRecord {
        contractor_id: "ctr_42".to_string(),
        wallet: "0x5010000000000000000000000000000000000003"
            .to_uppercase()
            .replace("0X", "0x"),
        agreement_sha256: TERMS.to_string(),
        agreement_accepted_at: 1_700_000_000,
        tax_form: Some(TaxFormRecord {
            kind,
            provider: "tax-provider".to_string(),
            provider_reference: "form_123".to_string(),
            tin_matched,
            received_at: 1_700_000_000,
        }),
    }
}

#[test]
fn contractors_need_the_agreement_and_a_usable_tax_form() {
    let now = 1_700_000_100;
    let attestation =
        contractor_attestation(&contractor(TaxFormKind::W9, true), TERMS, now, 30 * 86_400)
            .unwrap();
    assert_eq!(attestation.wallet, SOLVER);
    assert_eq!(attestation.valid_until, now + 30 * 86_400);
    assert_eq!(attestation.source_hash, invoice_contractor_source_hash());
    assert_eq!(
        attestation.source_hash,
        keccak_hex(b"agent-bounties/invoice-contractor-v1")
    );
    assert_eq!(
        attestation.participant_id,
        contractor_participant_id("ctr_42")
    );
    assert!(
        contractor_attestation(&contractor(TaxFormKind::W8Ben, false), TERMS, now, 86_400).is_ok(),
        "foreign payees certify on W-8BEN without a US TIN"
    );

    let mut no_form = contractor(TaxFormKind::W9, true);
    no_form.tax_form = None;
    let mut stale_agreement = contractor(TaxFormKind::W9, true);
    stale_agreement.agreement_sha256 = format!("sha256:{}", "3".repeat(64));
    let mut future_acceptance = contractor(TaxFormKind::W9, true);
    future_acceptance.agreement_accepted_at = now + 1;
    let mut bad_wallet = contractor(TaxFormKind::W9, true);
    bad_wallet.wallet = "0x0000000000000000000000000000000000000000".to_string();
    for (record, validity) in [
        (contractor(TaxFormKind::W9, false), 86_400),
        (no_form, 86_400),
        (stale_agreement, 86_400),
        (future_acceptance, 86_400),
        (bad_wallet, 86_400),
        (
            contractor(TaxFormKind::W9, true),
            MAX_ATTESTATION_SECONDS + 1,
        ),
        (contractor(TaxFormKind::W9, true), 0),
    ] {
        assert!(contractor_attestation(&record, TERMS, now, validity).is_err());
    }
}

#[test]
fn utc_year_handles_boundaries() {
    assert_eq!(utc_year(0), 1970);
    assert_eq!(utc_year(1_767_225_599), 2025); // 2025-12-31T23:59:59Z
    assert_eq!(utc_year(1_767_225_600), 2026); // 2026-01-01T00:00:00Z
    assert_eq!(utc_year(951_782_400), 2000); // 2000-02-29T00:00:00Z
}

#[test]
fn tax_summary_applies_the_year_threshold_and_separates_foreign_payees() {
    let payment = |id: &str, kind, usdc, at, key: &str| ContractorPayment {
        contractor_id: id.to_string(),
        tax_form_kind: kind,
        order_id: "ord".to_string(),
        reportable_usdc: usdc,
        paid_at: at,
        log_key: key.to_string(),
    };
    let in_2026 = 1_780_000_000;
    let payments = vec![
        payment("us_a", TaxFormKind::W9, 1_500_000_000, in_2026, "a1"),
        payment("us_a", TaxFormKind::W9, 500_000_000, in_2026, "a2"),
        payment("us_b", TaxFormKind::W9, 1_999_994_999, in_2026, "b1"),
        payment("intl", TaxFormKind::W8Ben, 9_000_000_000, in_2026, "c1"),
        payment("us_a", TaxFormKind::W9, 700_000_000, 1_750_000_000, "old"),
    ];
    let summary = summarize_contractor_payments(&payments, 2026, None).unwrap();
    assert_eq!(summary.threshold_cents, 200_000);
    assert_eq!(summary.form_1099_nec.len(), 2);
    assert_eq!(summary.form_1099_nec[0].contractor_id, "us_a");
    assert_eq!(summary.form_1099_nec[0].total_cents, 200_000);
    assert!(summary.form_1099_nec[0].requires_1099_nec);
    assert_eq!(summary.form_1099_nec[1].total_cents, 199_999);
    assert!(!summary.form_1099_nec[1].requires_1099_nec);
    assert_eq!(summary.foreign_payee_totals.len(), 1);
    assert!(!summary.foreign_payee_totals[0].requires_1099_nec);

    let earlier = summarize_contractor_payments(&payments, 2025, None).unwrap();
    assert_eq!(earlier.threshold_cents, 60_000);
    assert!(earlier.form_1099_nec[0].requires_1099_nec);
    assert_eq!(form_1099_nec_threshold_cents(2028, Some(210_000)), 210_000);

    let mut duplicated = payments.clone();
    duplicated.push(payments[0].clone());
    assert!(summarize_contractor_payments(&duplicated, 2026, None).is_err());
}
