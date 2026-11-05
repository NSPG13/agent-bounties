//! Read-only refund evidence. Credit-note issuance and customer credit balances
//! are accounting operations; neither proves that money was returned.
use super::*;
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceRefundBinding {
    pub refund_id: String,
    pub amount_cents: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvoiceRefundSnapshot {
    pub refund_id: String,
    pub amount_cents: u64,
    pub currency: String,
    pub status: String,
}

fn refund_id(value: &str) -> Result<String, StripeIntegrationError> {
    let prefix = if value.starts_with("pyr_") {
        "pyr_"
    } else {
        "re_"
    };
    stripe_id(value, prefix, "refund_id")
}

fn get(endpoint: String) -> StripeRequestIntent {
    StripeRequestIntent {
        method: "GET".into(),
        api_version: STRIPE_API_VERSION.into(),
        idempotency_key: format!("invoice_refund_read:{endpoint}"),
        endpoint,
        body: serde_json::json!({}),
    }
}

pub fn plan_retrieve_credit_note(id: &str) -> Result<StripeRequestIntent, StripeIntegrationError> {
    Ok(get(format!(
        "{CREDIT_NOTES_ENDPOINT}/{}",
        stripe_id(id, "cn_", "credit_note_id")?
    )))
}

pub fn plan_retrieve_invoice_refund(
    id: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    Ok(get(format!("/v1/refunds/{}", refund_id(id)?)))
}

/// Validate the complete credit-note allocation before following any refund ID.
/// Partial allocations, balance credits and recorded external payments need
/// separate evidence and cannot complete this Stripe repayment flow.
pub fn parse_credit_note_refund_bindings(
    object: &Value,
    credit_note: &str,
    invoice: &str,
    order: &str,
    paid_cents: u64,
    livemode: bool,
) -> Result<Vec<InvoiceRefundBinding>, StripeIntegrationError> {
    if string_field(object, "object")? != "credit_note"
        || string_field(object, "id")? != stripe_id(credit_note, "cn_", "credit_note_id")?
        || string_field(object, "invoice")? != stripe_id(invoice, "in_", "invoice_id")?
        || string_field(object, "status")? != "issued"
        || string_field(object, "currency")? != "usd"
        || object.get("livemode").and_then(Value::as_bool) != Some(livemode)
        || object.pointer("/metadata/purpose").and_then(Value::as_str)
            != Some(INVOICED_OUTCOME_PURPOSE)
        || object.pointer("/metadata/order_id").and_then(Value::as_str) != Some(order)
        || paid_cents == 0
        || cents_field(object, "total")? != paid_cents
        || cents_field(object, "post_payment_amount")? != paid_cents
        || cents_field(object, "pre_payment_amount")? != 0
        || !matches!(object.get("out_of_band_amount"), Some(Value::Null))
            && object.get("out_of_band_amount").and_then(Value::as_u64) != Some(0)
        || !matches!(
            object.get("customer_balance_transaction"),
            Some(Value::Null)
        )
    {
        return Err(invalid("credit_note_refund_binding"));
    }
    let allocations = field(object, "refunds")?
        .as_array()
        .ok_or_else(|| invalid("refunds"))?;
    if allocations.is_empty() || allocations.len() > 100 {
        return Err(invalid("refunds"));
    }
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut result = Vec::new();
    for allocation in allocations {
        if allocation
            .get("type")
            .is_some_and(|value| !value.is_null() && value.as_str() != Some("refund"))
        {
            return Err(invalid("refunds.type"));
        }
        let id = refund_id(&string_field(allocation, "refund")?)?;
        let amount = cents_field(allocation, "amount_refunded")?;
        if amount == 0 || !seen.insert(id.clone()) {
            return Err(invalid("refunds"));
        }
        total = total
            .checked_add(amount)
            .ok_or_else(|| invalid("refunds.amount_refunded"))?;
        result.push(InvoiceRefundBinding {
            refund_id: id,
            amount_cents: amount,
        });
    }
    if total != paid_cents {
        return Err(invalid("refunds.amount_refunded"));
    }
    result.sort_by(|left, right| left.refund_id.cmp(&right.refund_id));
    Ok(result)
}

/// Parse a fresh GET /v1/refunds/{id} from the configured account. The credit
/// note supplies the exact invoice/order association; refund metadata is not
/// assumed to inherit from the invoice or credit note.
pub fn parse_invoice_refund_snapshot(
    object: &Value,
    binding: &InvoiceRefundBinding,
    livemode: bool,
) -> Result<InvoiceRefundSnapshot, StripeIntegrationError> {
    let id = refund_id(&string_field(object, "id")?)?;
    let amount = cents_field(object, "amount")?;
    let currency = string_field(object, "currency")?;
    let status = string_field(object, "status")?;
    let destination = object
        .pointer("/destination_details/type")
        .and_then(Value::as_str);
    if string_field(object, "object")? != "refund"
        || id != binding.refund_id || amount == 0 || amount != binding.amount_cents
        || currency != "usd"
        || !matches!(status.as_str(), "pending" | "requires_action" | "succeeded" | "failed" | "canceled")
        // This adapter serves the US bank-transfer invoice path. Cash-balance
        // refunds also report success, but do not return money to the buyer's bank.
        || destination.is_some_and(|value| value != "us_bank_transfer")
        || (status == "succeeded" && destination != Some("us_bank_transfer"))
        // Refund objects can omit livemode; the authenticated request key fixes
        // their scope. If returned, the field must agree with the credit note.
        || object.get("livemode").is_some_and(|value| value.as_bool() != Some(livemode))
    {
        return Err(invalid("refund_binding"));
    }
    Ok(InvoiceRefundSnapshot {
        refund_id: id,
        amount_cents: amount,
        currency,
        status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn credit() -> Value {
        json!({"id":"cn_1","object":"credit_note","invoice":"in_1","status":"issued",
            "currency":"usd","livemode":false,"total":11750,"pre_payment_amount":0,
            "post_payment_amount":11750,"out_of_band_amount":null,"customer_balance_transaction":null,
            "metadata":{"purpose":"invoiced_outcome","order_id":"ord_1"},
            "refunds":[{"refund":"re_1","amount_refunded":11750}]})
    }
    fn bindings(value: &Value) -> Result<Vec<InvoiceRefundBinding>, StripeIntegrationError> {
        parse_credit_note_refund_bindings(value, "cn_1", "in_1", "ord_1", 11750, false)
    }
    fn refund() -> Value {
        // Native Refund objects need not carry livemode or invoice metadata.
        json!({"id":"re_1","object":"refund","amount":11750,"currency":"usd","status":"pending",
            "destination_details":{"type":"us_bank_transfer","us_bank_transfer":{}}})
    }

    #[test]
    fn refund_reconciliation_plans_are_bodyless_gets_and_reject_path_injection() {
        for plan in [
            plan_retrieve_credit_note("cn_1").unwrap(),
            plan_retrieve_invoice_refund("re_1").unwrap(),
            plan_retrieve_invoice_refund("pyr_1").unwrap(),
        ] {
            let request = crate::build_stripe_http_request(
                &plan,
                "sk_test_fixture",
                "https://api.stripe.com",
            )
            .unwrap();
            assert_eq!(request.method, "GET");
            assert!(request.body.is_empty());
        }
        for id in ["", "re_", "re_1/refund", "re_1?expand[]=charge", "ch_1"] {
            assert!(plan_retrieve_invoice_refund(id).is_err());
        }
        assert!(plan_retrieve_credit_note("cn_1/void").is_err());
    }

    #[test]
    fn credit_note_requires_a_complete_bound_stripe_refund_allocation() {
        assert_eq!(
            bindings(&credit()).unwrap(),
            vec![InvoiceRefundBinding {
                refund_id: "re_1".into(),
                amount_cents: 11750
            }]
        );
        for (pointer, value) in [
            ("/id", json!("cn_other")),
            ("/invoice", json!("in_other")),
            ("/status", json!("void")),
            ("/currency", json!("mxn")),
            ("/livemode", json!(true)),
            ("/metadata/order_id", json!("ord_other")),
            ("/metadata/purpose", json!("other")),
            ("/total", json!(11749)),
            ("/post_payment_amount", json!(0)),
            ("/pre_payment_amount", json!(11750)),
            ("/out_of_band_amount", json!(11750)),
            ("/customer_balance_transaction", json!("cbtxn_1")),
            ("/refunds", json!([])),
            ("/refunds/0/amount_refunded", json!(11749)),
            ("/refunds/0/refund", json!("re_other/path")),
            (
                "/refunds",
                json!([{"refund":"re_1","amount_refunded":5875},{"refund":"re_1","amount_refunded":5875}]),
            ),
            (
                "/refunds",
                json!([{"refund":"re_1","type":"payment_record_refund","amount_refunded":11750}]),
            ),
        ] {
            let mut candidate = credit();
            *candidate.pointer_mut(pointer).unwrap() = value;
            assert!(bindings(&candidate).is_err(), "{pointer}");
        }
        for missing in [
            "refunds",
            "currency",
            "out_of_band_amount",
            "customer_balance_transaction",
        ] {
            let mut candidate = credit();
            candidate.as_object_mut().unwrap().remove(missing);
            assert!(bindings(&candidate).is_err(), "{missing}");
        }
    }

    #[test]
    fn refund_status_and_identity_must_be_explicit_and_match_the_allocation() {
        let binding = bindings(&credit()).unwrap().remove(0);
        for status in [
            "pending",
            "requires_action",
            "succeeded",
            "failed",
            "canceled",
        ] {
            let mut candidate = refund();
            candidate["status"] = json!(status);
            assert_eq!(
                parse_invoice_refund_snapshot(&candidate, &binding, false)
                    .unwrap()
                    .status,
                status
            );
        }
        for (field, value) in [
            ("id", json!("re_other")),
            ("object", json!("charge")),
            ("amount", json!(11749)),
            ("currency", json!("mxn")),
            ("status", json!(null)),
            ("status", json!("unknown")),
            ("livemode", json!(true)),
            (
                "destination_details",
                json!({"type":"customer_cash_balance"}),
            ),
            ("destination_details", json!({"type":"crypto"})),
        ] {
            let mut candidate = refund();
            candidate[field] = value;
            assert!(
                parse_invoice_refund_snapshot(&candidate, &binding, false).is_err(),
                "{field}"
            );
        }
        let mut missing_destination = refund();
        missing_destination["status"] = json!("succeeded");
        missing_destination
            .as_object_mut()
            .unwrap()
            .remove("destination_details");
        assert!(parse_invoice_refund_snapshot(&missing_destination, &binding, false).is_err());
    }
}
