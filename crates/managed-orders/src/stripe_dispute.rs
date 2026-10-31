//! Card disputes on managed payments. A signed native `charge.dispute.*` event stops funding
//! of an order that is not yet funded and flags a funded order for an operator. It never moves
//! money and never releases or reverses a refund or funding reservation.
use payments_stripe::verify_platform_webhook_event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DISPUTE_EVENTS: [&str; 5] = [
    "charge.dispute.created",
    "charge.dispute.updated",
    "charge.dispute.closed",
    "charge.dispute.funds_withdrawn",
    "charge.dispute.funds_reinstated",
];
const STATUSES: [&str; 8] = [
    "warning_needs_response",
    "warning_under_review",
    "warning_closed",
    "needs_response",
    "under_review",
    "won",
    "lost",
    "prevented",
];

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DisputeError {
    #[error("managed dispute webhook authentication failed")]
    Authentication,
    #[error("managed dispute event is unsupported or inconsistent")]
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedDisputeEvent {
    pub event_id: String,
    pub event_type: String,
    pub event_created: i64,
    pub livemode: bool,
    pub dispute_id: String,
    pub payment_intent_id: String,
    pub amount_minor: u64,
    pub currency: String,
    pub status: String,
    pub reason: String,
}

pub fn is_dispute_event(event_type: &str) -> bool {
    DISPUTE_EVENTS.contains(&event_type)
}

fn id(value: &Value, prefix: &str) -> Option<String> {
    let s = value.as_str()?;
    (s.starts_with(prefix)
        && s.len() > prefix.len()
        && s.len() <= 255
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then(|| s.to_owned())
}

/// Verify the platform signature and mode, then bind the dispute's PaymentIntent, amount and
/// currency. Matching it to a managed order is the store's job.
pub fn verify_managed_dispute_event(
    body: &[u8],
    signature: &str,
    secret: &[u8],
    livemode: bool,
) -> Result<ManagedDisputeEvent, DisputeError> {
    let event = verify_platform_webhook_event(body, signature, secret, livemode)
        .map_err(|_| DisputeError::Authentication)?;
    let raw: Value = serde_json::from_slice(body).map_err(|_| DisputeError::Event)?;
    let d = &event.payload;
    let currency = d["currency"].as_str().unwrap_or_default();
    let reason = d["reason"].as_str().unwrap_or_default();
    let status = d["status"].as_str().unwrap_or_default();
    let parsed = (|| {
        Some(ManagedDisputeEvent {
            event_id: id(&Value::String(event.id.clone()), "evt_")?,
            event_type: event.event_type.clone(),
            event_created: raw["created"].as_i64().filter(|t| *t > 0)?,
            livemode,
            dispute_id: id(&d["id"], "dp_").or_else(|| id(&d["id"], "du_"))?,
            payment_intent_id: id(&d["payment_intent"], "pi_")?,
            amount_minor: d["amount"].as_u64().filter(|a| *a > 0)?,
            currency: currency.to_owned(),
            status: status.to_owned(),
            reason: reason.to_owned(),
        })
    })()
    .ok_or(DisputeError::Event)?;
    let lower = |s: &str| s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    if !is_dispute_event(&parsed.event_type)
        || d["object"] != "dispute"
        || d["livemode"].as_bool() != Some(livemode)
        || currency.len() != 3
        || !lower(currency)
        || !STATUSES.contains(&status)
        || reason.is_empty()
        || reason.len() > 64
        || !lower(reason)
    {
        return Err(DisputeError::Event);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use serde_json::json;
    use sha2::Sha256;

    fn signed(body: &Value) -> (Vec<u8>, String) {
        let bytes = serde_json::to_vec(body).unwrap();
        let now = chrono::Utc::now().timestamp();
        let mut mac = Hmac::<Sha256>::new_from_slice(b"dispute-secret").unwrap();
        mac.update(format!("{now}.").as_bytes());
        mac.update(&bytes);
        let signature = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
        (bytes, signature)
    }
    fn event(kind: &str, dispute: Value) -> Value {
        json!({"id":"evt_dispute_1","object":"event","type":kind,"livemode":false,
            "created":1_800_000_000,"data":{"object":dispute}})
    }
    fn dispute() -> Value {
        json!({"id":"dp_fixture","object":"dispute","amount":225000,"charge":"ch_fixture",
            "currency":"mxn","payment_intent":"pi_fixture","reason":"fraudulent",
            "status":"needs_response","livemode":false})
    }

    #[test]
    fn signed_dispute_binds_payment_intent_amount_and_currency() {
        let (body, signature) = signed(&event("charge.dispute.created", dispute()));
        let parsed =
            verify_managed_dispute_event(&body, &signature, b"dispute-secret", false).unwrap();
        assert_eq!(
            (
                parsed.payment_intent_id.as_str(),
                parsed.amount_minor,
                parsed.currency.as_str()
            ),
            ("pi_fixture", 225_000, "mxn")
        );
        assert_eq!(parsed.event_created, 1_800_000_000);
        assert_eq!(
            verify_managed_dispute_event(&body, &signature, b"other-secret", false),
            Err(DisputeError::Authentication)
        );
        assert_eq!(
            verify_managed_dispute_event(&body, &signature, b"dispute-secret", true),
            Err(DisputeError::Authentication)
        );
    }

    #[test]
    fn unsupported_or_inconsistent_disputes_fail_closed() {
        let cases = [
            event("charge.refunded", dispute()),
            event("charge.dispute.created", {
                let mut d = dispute();
                d["payment_intent"] = Value::Null;
                d
            }),
            event("charge.dispute.created", {
                let mut d = dispute();
                d["livemode"] = true.into();
                d
            }),
            event("charge.dispute.created", {
                let mut d = dispute();
                d["status"] = "paid".into();
                d
            }),
            event("charge.dispute.created", {
                let mut d = dispute();
                d["currency"] = "MXN".into();
                d
            }),
            event("charge.dispute.created", {
                let mut d = dispute();
                d["object"] = "charge".into();
                d
            }),
        ];
        for case in cases {
            let (body, signature) = signed(&case);
            assert!(
                verify_managed_dispute_event(&body, &signature, b"dispute-secret", false).is_err(),
                "{case}"
            );
        }
    }
}
