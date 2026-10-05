//! Stripe Invoicing for the seller-of-record business path (ADR 0006).
//!
//! AgentBounties (the US entity) invoices a business buyer for a verified outcome. Invoices accept
//! only US bank transfers (`customer_balance`). Those payments cannot be charged back, so a paid
//! invoice is final when the signature-verified `invoice.paid` webhook arrives. Card and ACH debit
//! stay disabled until dispute reconciliation exists.
//!
//! Planners return `StripeRequestIntent`s with deterministic idempotency keys per order. Parsers
//! accept native Stripe webhook events (`{id, type, livemode, data: {object}}`) and only for objects
//! tagged `metadata.purpose = "invoiced_outcome"`. Callers must verify the webhook signature first.

use super::{StripeIntegrationError, StripeRequestIntent, STRIPE_API_VERSION};
use serde::{Deserialize, Serialize};

pub const CUSTOMERS_ENDPOINT: &str = "/v1/customers";
pub const INVOICES_ENDPOINT: &str = "/v1/invoices";
pub const INVOICE_ITEMS_ENDPOINT: &str = "/v1/invoiceitems";
pub const CREDIT_NOTES_ENDPOINT: &str = "/v1/credit_notes";
pub const INVOICED_OUTCOME_PURPOSE: &str = "invoiced_outcome";

/// One invoice line in US cents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeInvoiceLine {
    pub code: String,
    pub description: String,
    pub amount_cents: u64,
}

/// Everything needed to issue one order's invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeInvoiceOrder {
    pub order_id: String,
    pub buyer_name: String,
    pub buyer_email: String,
    pub lines: Vec<StripeInvoiceLine>,
    pub days_until_due: u32,
    /// Public URL of the business terms the buyer accepts by paying.
    pub terms_url: String,
    pub terms_sha256: String,
}

fn invalid(field: &str) -> StripeIntegrationError {
    StripeIntegrationError::InvalidField(field.to_string())
}

fn post(endpoint: String, idempotency_key: String, body: serde_json::Value) -> StripeRequestIntent {
    StripeRequestIntent {
        method: "POST".to_string(),
        endpoint,
        api_version: STRIPE_API_VERSION.to_string(),
        idempotency_key,
        body,
    }
}

fn stripe_id(value: &str, prefix: &str, field: &str) -> Result<String, StripeIntegrationError> {
    let value = value.trim();
    (value.starts_with(prefix)
        && value.len() > prefix.len()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
    .then(|| value.to_string())
    .ok_or_else(|| invalid(field))
}

fn order_id(value: &str) -> Result<String, StripeIntegrationError> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
    .then(|| value.to_string())
    .ok_or_else(|| invalid("order_id"))
}

impl StripeInvoiceOrder {
    fn validated(&self) -> Result<String, StripeIntegrationError> {
        let order = order_id(&self.order_id)?;
        if self.buyer_name.trim().is_empty() || !self.buyer_email.contains('@') {
            return Err(invalid("buyer"));
        }
        if self.lines.is_empty()
            || self.lines.iter().any(|line| {
                line.amount_cents == 0
                    || line.description.trim().is_empty()
                    || line.code.trim().is_empty()
            })
        {
            return Err(invalid("lines"));
        }
        if !(1..=90).contains(&self.days_until_due) {
            return Err(invalid("days_until_due"));
        }
        if !self.terms_url.starts_with("https://") || !self.terms_sha256.starts_with("sha256:") {
            return Err(invalid("terms"));
        }
        Ok(order)
    }

    fn metadata(&self, order: &str) -> serde_json::Value {
        serde_json::json!({
            "order_id": order,
            "purpose": INVOICED_OUTCOME_PURPOSE,
            "terms_sha256": self.terms_sha256,
        })
    }

    pub fn total_cents(&self) -> u64 {
        self.lines.iter().map(|line| line.amount_cents).sum()
    }

    /// `POST /v1/customers` for the buyer.
    pub fn plan_customer(&self) -> Result<StripeRequestIntent, StripeIntegrationError> {
        let order = self.validated()?;
        Ok(post(
            CUSTOMERS_ENDPOINT.to_string(),
            format!("invoice_customer:{order}"),
            serde_json::json!({
                "name": self.buyer_name.trim(),
                "email": self.buyer_email.trim(),
                "metadata": self.metadata(&order),
            }),
        ))
    }

    /// `POST /v1/invoices`: a draft invoice that is sent to the buyer and payable only by US bank
    /// transfer. The footer states the seller-of-record terms.
    pub fn plan_draft_invoice(
        &self,
        customer_id: &str,
    ) -> Result<StripeRequestIntent, StripeIntegrationError> {
        let order = self.validated()?;
        Ok(post(
            INVOICES_ENDPOINT.to_string(),
            format!("invoice_draft:{order}"),
            serde_json::json!({
                "customer": stripe_id(customer_id, "cus_", "customer_id")?,
                "currency": "usd",
                "collection_method": "send_invoice",
                "days_until_due": self.days_until_due,
                "auto_advance": false,
                "payment_settings": {
                    "payment_method_types": ["customer_balance"],
                },
                "description": format!("Order {order}"),
                "footer": format!(
                    "AgentBounties is the seller of record for this verified outcome and delivers it through independent contractors. Paying this invoice accepts the business terms at {} ({}).",
                    self.terms_url, self.terms_sha256
                ),
                "metadata": self.metadata(&order),
            }),
        ))
    }

    /// `POST /v1/invoiceitems`, one per quote line, attached to the draft invoice.
    pub fn plan_invoice_items(
        &self,
        customer_id: &str,
        invoice_id: &str,
    ) -> Result<Vec<StripeRequestIntent>, StripeIntegrationError> {
        let order = self.validated()?;
        let customer = stripe_id(customer_id, "cus_", "customer_id")?;
        let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
        Ok(self
            .lines
            .iter()
            .map(|line| {
                post(
                    INVOICE_ITEMS_ENDPOINT.to_string(),
                    format!("invoice_item:{order}:{}", line.code),
                    serde_json::json!({
                        "customer": customer,
                        "invoice": invoice,
                        "currency": "usd",
                        "amount": line.amount_cents,
                        "description": line.description,
                        "metadata": {
                            "order_id": order,
                            "purpose": INVOICED_OUTCOME_PURPOSE,
                            "line_code": line.code,
                        },
                    }),
                )
            })
            .collect())
    }
}

/// `GET /v1/invoices/{id}`: reads a stored invoice so a retried issue resumes from its real status.
pub fn plan_retrieve_invoice(
    invoice_id: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
    Ok(StripeRequestIntent {
        method: "GET".to_string(),
        endpoint: format!("{INVOICES_ENDPOINT}/{invoice}"),
        api_version: STRIPE_API_VERSION.to_string(),
        idempotency_key: format!("invoice_retrieve:{invoice}"),
        body: serde_json::json!({}),
    })
}

/// `POST /v1/invoices/{id}/finalize`.
pub fn plan_finalize_invoice(
    order: &str,
    invoice_id: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let order = order_id(order)?;
    let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
    Ok(post(
        format!("{INVOICES_ENDPOINT}/{invoice}/finalize"),
        format!("invoice_finalize:{order}"),
        serde_json::json!({ "auto_advance": false }),
    ))
}

/// `POST /v1/invoices/{id}/send`: emails the hosted invoice with bank-transfer instructions.
pub fn plan_send_invoice(
    order: &str,
    invoice_id: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let order = order_id(order)?;
    let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
    Ok(post(
        format!("{INVOICES_ENDPOINT}/{invoice}/send"),
        format!("invoice_send:{order}"),
        serde_json::json!({}),
    ))
}

/// `POST /v1/invoices/{id}/void` for an unpaid invoice.
pub fn plan_void_invoice(
    order: &str,
    invoice_id: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let order = order_id(order)?;
    let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
    Ok(post(
        format!("{INVOICES_ENDPOINT}/{invoice}/void"),
        format!("invoice_void:{order}"),
        serde_json::json!({}),
    ))
}

/// How a credit note returns the buyer's money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreditNoteRefund {
    /// `refund_amount`: Stripe refunds the bank-transfer payment.
    StripeRefund,
    /// `out_of_band_amount`: the treasury returns funds outside Stripe and records it here.
    OutOfBand,
}

/// `POST /v1/credit_notes` refunding a paid invoice in full after cancellation.
pub fn plan_credit_note_refund(
    order: &str,
    invoice_id: &str,
    amount_cents: u64,
    refund: CreditNoteRefund,
    memo: &str,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let order = order_id(order)?;
    let invoice = stripe_id(invoice_id, "in_", "invoice_id")?;
    if amount_cents == 0 {
        return Err(invalid("amount_cents"));
    }
    let refund_field = match refund {
        CreditNoteRefund::StripeRefund => "refund_amount",
        CreditNoteRefund::OutOfBand => "out_of_band_amount",
    };
    let mut body = serde_json::json!({
        "invoice": invoice,
        "amount": amount_cents,
        "reason": "order_change",
        "memo": memo.trim(),
        "metadata": {
            "order_id": order,
            "purpose": INVOICED_OUTCOME_PURPOSE,
        },
    });
    body[refund_field] = serde_json::json!(amount_cents);
    Ok(post(
        CREDIT_NOTES_ENDPOINT.to_string(),
        format!("credit_note_refund:{order}"),
        body,
    ))
}

/// A native Stripe webhook event.
#[derive(Debug, Clone, Deserialize)]
pub struct StripeNativeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub livemode: bool,
    pub data: StripeNativeEventData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StripeNativeEventData {
    pub object: serde_json::Value,
}

/// Evidence for the invoiced-order projection, taken from a signature-verified webhook.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InvoiceWebhookEvidence {
    InvoicePaid {
        stripe_event_id: String,
        invoice_id: String,
        order_id: String,
        status: String,
        amount_paid_cents: u64,
        amount_remaining_cents: u64,
        currency: String,
        livemode: bool,
        /// Marked paid in the Dashboard without Stripe receiving funds; never payment evidence.
        paid_out_of_band: bool,
    },
    InvoiceVoided {
        stripe_event_id: String,
        invoice_id: String,
        order_id: String,
        livemode: bool,
    },
    CreditNoteCreated {
        stripe_event_id: String,
        credit_note_id: String,
        invoice_id: String,
        order_id: String,
        status: String,
        total_cents: u64,
        livemode: bool,
    },
}

impl InvoiceWebhookEvidence {
    pub fn order_id(&self) -> &str {
        match self {
            InvoiceWebhookEvidence::InvoicePaid { order_id, .. }
            | InvoiceWebhookEvidence::InvoiceVoided { order_id, .. }
            | InvoiceWebhookEvidence::CreditNoteCreated { order_id, .. } => order_id,
        }
    }

    pub fn stripe_event_id(&self) -> &str {
        match self {
            InvoiceWebhookEvidence::InvoicePaid {
                stripe_event_id, ..
            }
            | InvoiceWebhookEvidence::InvoiceVoided {
                stripe_event_id, ..
            }
            | InvoiceWebhookEvidence::CreditNoteCreated {
                stripe_event_id, ..
            } => stripe_event_id,
        }
    }
}

fn field<'a>(
    object: &'a serde_json::Value,
    key: &str,
) -> Result<&'a serde_json::Value, StripeIntegrationError> {
    object
        .get(key)
        .filter(|value| !value.is_null())
        .ok_or_else(|| StripeIntegrationError::MissingField(key.to_string()))
}

fn string_field(object: &serde_json::Value, key: &str) -> Result<String, StripeIntegrationError> {
    field(object, key)?
        .as_str()
        .map(ToString::to_string)
        .ok_or_else(|| invalid(key))
}

fn cents_field(object: &serde_json::Value, key: &str) -> Result<u64, StripeIntegrationError> {
    field(object, key)?.as_u64().ok_or_else(|| invalid(key))
}

/// Parses a verified webhook body. Returns `Ok(None)` for events that are not about an invoiced
/// order, or are tagged but not one of the three applied types (for example `invoice.finalized`),
/// which the endpoint should acknowledge without applying so Stripe does not retry them.
pub fn parse_invoice_webhook(
    payload: &[u8],
) -> Result<Option<InvoiceWebhookEvidence>, StripeIntegrationError> {
    let event: StripeNativeEvent = serde_json::from_slice(payload).map_err(|_| invalid("event"))?;
    let object = &event.data.object;
    let tagged = object
        .get("metadata")
        .and_then(|metadata| metadata.get("purpose"))
        .and_then(serde_json::Value::as_str)
        == Some(INVOICED_OUTCOME_PURPOSE);
    if !tagged {
        return Ok(None);
    }
    let order = object
        .get("metadata")
        .and_then(|metadata| metadata.get("order_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| StripeIntegrationError::MissingField("metadata.order_id".to_string()))
        .and_then(order_id)?;
    let object_kind = string_field(object, "object")?;
    if object.get("livemode").and_then(serde_json::Value::as_bool) != Some(event.livemode) {
        return Err(invalid("livemode"));
    }
    let evidence = match (event.event_type.as_str(), object_kind.as_str()) {
        ("invoice.paid", "invoice") => InvoiceWebhookEvidence::InvoicePaid {
            stripe_event_id: event.id,
            invoice_id: stripe_id(&string_field(object, "id")?, "in_", "id")?,
            order_id: order,
            status: string_field(object, "status")?,
            amount_paid_cents: cents_field(object, "amount_paid")?,
            amount_remaining_cents: cents_field(object, "amount_remaining")?,
            currency: string_field(object, "currency")?.to_ascii_lowercase(),
            livemode: event.livemode,
            paid_out_of_band: object
                .get("paid_out_of_band")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        },
        ("invoice.voided", "invoice") => InvoiceWebhookEvidence::InvoiceVoided {
            stripe_event_id: event.id,
            invoice_id: stripe_id(&string_field(object, "id")?, "in_", "id")?,
            order_id: order,
            livemode: event.livemode,
        },
        ("credit_note.created", "credit_note") => InvoiceWebhookEvidence::CreditNoteCreated {
            stripe_event_id: event.id,
            credit_note_id: stripe_id(&string_field(object, "id")?, "cn_", "id")?,
            invoice_id: stripe_id(&string_field(object, "invoice")?, "in_", "invoice")?,
            order_id: order,
            status: string_field(object, "status")?,
            total_cents: cents_field(object, "total")?,
            livemode: event.livemode,
        },
        _ => return Ok(None),
    };
    if !evidence.stripe_event_id().starts_with("evt_") {
        return Err(invalid("id"));
    }
    Ok(Some(evidence))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stripe_form_encode;

    fn order() -> StripeInvoiceOrder {
        StripeInvoiceOrder {
            order_id: "ord_01".to_string(),
            buyer_name: "Acme Corp".to_string(),
            buyer_email: "ap@acme.example".to_string(),
            lines: vec![
                StripeInvoiceLine {
                    code: "outcome".to_string(),
                    description: "Verified outcome: Fix CI".to_string(),
                    amount_cents: 11_000,
                },
                StripeInvoiceLine {
                    code: "platform_fee".to_string(),
                    description: "Platform fee (7.50% of the solver reward)".to_string(),
                    amount_cents: 750,
                },
            ],
            days_until_due: 14,
            terms_url: "https://agentbounties.app/terms/business".to_string(),
            terms_sha256: format!("sha256:{}", "2".repeat(64)),
        }
    }

    #[test]
    fn invoice_plans_are_bank_transfer_only_and_idempotent_per_order() {
        let order = order();
        assert_eq!(order.total_cents(), 11_750);
        let customer = order.plan_customer().unwrap();
        assert_eq!(customer.endpoint, "/v1/customers");
        assert_eq!(customer.idempotency_key, "invoice_customer:ord_01");

        let draft = order.plan_draft_invoice("cus_123").unwrap();
        let form = stripe_form_encode(&draft.body);
        for expected in [
            "collection_method=send_invoice",
            "days_until_due=14",
            "auto_advance=false",
            "payment_settings%5Bpayment_method_types%5D%5B0%5D=customer_balance",
            "metadata%5Bpurpose%5D=invoiced_outcome",
            "metadata%5Border_id%5D=ord_01",
        ] {
            assert!(form.contains(expected), "{expected} missing from {form}");
        }
        assert!(draft.body["footer"]
            .as_str()
            .unwrap()
            .contains("seller of record"));

        let items = order.plan_invoice_items("cus_123", "in_456").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].idempotency_key, "invoice_item:ord_01:platform_fee");
        assert_eq!(items[1].body["amount"], 750);
        assert_eq!(items[1].body["invoice"], "in_456");

        assert_eq!(
            plan_finalize_invoice("ord_01", "in_456").unwrap().endpoint,
            "/v1/invoices/in_456/finalize"
        );
        assert_eq!(
            plan_send_invoice("ord_01", "in_456").unwrap().endpoint,
            "/v1/invoices/in_456/send"
        );
        assert_eq!(
            plan_void_invoice("ord_01", "in_456").unwrap().endpoint,
            "/v1/invoices/in_456/void"
        );
    }

    #[test]
    fn invoice_plans_reject_injection_and_bad_inputs() {
        let order = order();
        assert!(order.plan_draft_invoice("cus_1/../../v1/refunds").is_err());
        assert!(order.plan_invoice_items("cus_1", "pi_1").is_err());
        assert!(plan_finalize_invoice("ord_01", "in_1/void").is_err());
        assert!(plan_finalize_invoice("ord 01", "in_1").is_err());
        let mut bad = order.clone();
        bad.lines[0].amount_cents = 0;
        assert!(bad.plan_customer().is_err());
        let mut insecure = order.clone();
        insecure.terms_url = "http://agentbounties.app/terms".to_string();
        assert!(insecure.plan_draft_invoice("cus_1").is_err());
    }

    #[test]
    fn credit_note_refunds_the_full_amount_through_one_channel() {
        let refund = plan_credit_note_refund(
            "ord_01",
            "in_456",
            11_750,
            CreditNoteRefund::StripeRefund,
            "cancelled",
        )
        .unwrap();
        assert_eq!(refund.endpoint, "/v1/credit_notes");
        assert_eq!(refund.body["amount"], 11_750);
        assert_eq!(refund.body["refund_amount"], 11_750);
        assert!(refund.body.get("out_of_band_amount").is_none());
        assert_eq!(refund.body["reason"], "order_change");
        let out_of_band =
            plan_credit_note_refund("ord_01", "in_456", 11_750, CreditNoteRefund::OutOfBand, "")
                .unwrap();
        assert_eq!(out_of_band.body["out_of_band_amount"], 11_750);
        assert!(out_of_band.body.get("refund_amount").is_none());
        assert!(
            plan_credit_note_refund("ord_01", "in_456", 0, CreditNoteRefund::OutOfBand, "")
                .is_err()
        );
    }

    fn event(event_type: &str, object: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "id": "evt_1",
            "object": "event",
            "type": event_type,
            "livemode": false,
            "created": 1_780_000_000,
            "data": {"object": object},
        }))
        .unwrap()
    }

    fn paid_invoice() -> serde_json::Value {
        serde_json::json!({
            "id": "in_456",
            "object": "invoice",
            "status": "paid",
            "amount_due": 11_750,
            "amount_paid": 11_750,
            "amount_remaining": 0,
            "currency": "usd",
            "livemode": false,
            "metadata": {"order_id": "ord_01", "purpose": "invoiced_outcome"},
        })
    }

    #[test]
    fn parses_native_invoice_and_credit_note_events() {
        let evidence = parse_invoice_webhook(&event("invoice.paid", paid_invoice()))
            .unwrap()
            .unwrap();
        assert_eq!(
            evidence,
            InvoiceWebhookEvidence::InvoicePaid {
                stripe_event_id: "evt_1".to_string(),
                invoice_id: "in_456".to_string(),
                order_id: "ord_01".to_string(),
                status: "paid".to_string(),
                amount_paid_cents: 11_750,
                amount_remaining_cents: 0,
                currency: "usd".to_string(),
                livemode: false,
                paid_out_of_band: false,
            }
        );
        let mut marked_paid = paid_invoice();
        marked_paid["paid_out_of_band"] = serde_json::json!(true);
        assert!(matches!(
            parse_invoice_webhook(&event("invoice.paid", marked_paid)).unwrap(),
            Some(InvoiceWebhookEvidence::InvoicePaid {
                paid_out_of_band: true,
                ..
            })
        ));
        let credit = parse_invoice_webhook(&event(
            "credit_note.created",
            serde_json::json!({
                "id": "cn_9",
                "object": "credit_note",
                "invoice": "in_456",
                "status": "issued",
                "total": 11_750,
                "livemode": false,
                "metadata": {"order_id": "ord_01", "purpose": "invoiced_outcome"},
            }),
        ))
        .unwrap()
        .unwrap();
        assert_eq!(credit.order_id(), "ord_01");
        assert!(matches!(
            credit,
            InvoiceWebhookEvidence::CreditNoteCreated {
                total_cents: 11_750,
                ..
            }
        ));
    }

    #[test]
    fn retrieving_an_invoice_is_a_bodyless_get() {
        let intent = plan_retrieve_invoice("in_456").unwrap();
        let request =
            crate::build_stripe_http_request(&intent, "sk_test_123", "https://api.stripe.com/")
                .unwrap();
        assert_eq!(request.method, "GET");
        assert_eq!(request.url, "https://api.stripe.com/v1/invoices/in_456");
        assert!(request.body.is_empty());
        assert!(plan_retrieve_invoice("in_456/void").is_err());
        let mut delete = intent;
        delete.method = "DELETE".to_string();
        assert!(
            crate::build_stripe_http_request(&delete, "sk_test_123", "https://api.stripe.com")
                .is_err()
        );
    }

    #[test]
    fn ignores_untagged_events_and_rejects_inconsistent_ones() {
        let mut untagged = paid_invoice();
        untagged["metadata"] = serde_json::json!({"order_id": "ord_01"});
        assert_eq!(
            parse_invoice_webhook(&event("invoice.paid", untagged)).unwrap(),
            None
        );
        let mut mode_mismatch = paid_invoice();
        mode_mismatch["livemode"] = serde_json::json!(true);
        assert!(parse_invoice_webhook(&event("invoice.paid", mode_mismatch)).is_err());
        let mut missing_amount = paid_invoice();
        missing_amount["amount_paid"] = serde_json::Value::Null;
        assert!(parse_invoice_webhook(&event("invoice.paid", missing_amount)).is_err());
        assert_eq!(
            parse_invoice_webhook(&event("invoice.finalized", paid_invoice())).unwrap(),
            None,
            "tagged lifecycle events are acknowledged, not retried"
        );
        assert!(parse_invoice_webhook(b"not json").is_err());
    }
}
