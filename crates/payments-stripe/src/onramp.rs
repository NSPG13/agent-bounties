//! Stripe fiat-to-crypto onramp sessions (ADR 0006, non-custodial path).
//!
//! The session delivers USDC on Base straight to the user's own wallet, with the address locked.
//! Stripe is the merchant of record for that purchase and handles KYC, fraud and chargebacks.
//! Buying USDC is not bounty funding: the wallet still signs the exact canonical contribution, and
//! only an indexed `FundingAdded` changes a bounty.
//!
//! Stripe's onramp API is in public preview. The network and wallet-address key for Base are
//! settings, to be confirmed in a Stripe sandbox once the onramp application is approved.

use super::{StripeIntegrationError, StripeRequestIntent, STRIPE_API_VERSION};
use serde::{Deserialize, Serialize};

pub const ONRAMP_SESSIONS_ENDPOINT: &str = "/v1/crypto/onramp_sessions";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StripeOnrampSessionRequest {
    /// The user's own wallet. The session locks delivery to it.
    pub wallet_address: String,
    /// USDC to buy, with at most two decimals.
    pub destination_amount: String,
    /// Required in live mode, so Stripe can pre-check supportability.
    pub customer_ip_address: Option<String>,
    /// Stripe's identifier for the Base network.
    pub destination_network: String,
    /// The `wallet_addresses[...]` key Stripe uses for that network.
    pub wallet_address_key: String,
    /// Caller-chosen id that makes retries idempotent.
    pub request_id: String,
}

fn invalid(field: &str) -> StripeIntegrationError {
    StripeIntegrationError::InvalidField(field.to_string())
}

fn identifier(value: &str, field: &str) -> Result<String, StripeIntegrationError> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
    .then(|| value.to_string())
    .ok_or_else(|| invalid(field))
}

/// Parses a positive USDC amount with at most two decimals into cents.
pub fn usdc_amount_cents(value: &str) -> Result<u64, StripeIntegrationError> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || whole.len() > 9
        || fraction.len() > 2
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid("destination_amount"));
    }
    let cents = whole
        .parse::<u64>()
        .map_err(|_| invalid("destination_amount"))?
        * 100
        + format!("{fraction:0<2}")
            .parse::<u64>()
            .map_err(|_| invalid("destination_amount"))?;
    (cents > 0)
        .then_some(cents)
        .ok_or_else(|| invalid("destination_amount"))
}

/// `POST /v1/crypto/onramp_sessions` for a locked, USDC-only purchase on Base.
pub fn plan_onramp_session(
    request: &StripeOnrampSessionRequest,
) -> Result<StripeRequestIntent, StripeIntegrationError> {
    let wallet = request.wallet_address.trim().to_ascii_lowercase();
    if wallet.len() != 42
        || !wallet.starts_with("0x")
        || !wallet[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
        || wallet == format!("0x{}", "0".repeat(40))
    {
        return Err(invalid("wallet_address"));
    }
    let cents = usdc_amount_cents(&request.destination_amount)?;
    let network = identifier(&request.destination_network, "destination_network")?;
    let wallet_key = identifier(&request.wallet_address_key, "wallet_address_key")?;
    let request_id = identifier(&request.request_id.to_ascii_lowercase(), "request_id")?;
    let mut body = serde_json::json!({
        "wallet_addresses": { wallet_key: wallet },
        "lock_wallet_address": true,
        "source_currency": "usd",
        "destination_currency": "usdc",
        "destination_network": network,
        "destination_currencies": ["usdc"],
        "destination_networks": [network],
        "destination_amount": format!("{}.{:02}", cents / 100, cents % 100),
    });
    if let Some(ip) = request.customer_ip_address.as_deref() {
        if ip.parse::<std::net::IpAddr>().is_err() {
            return Err(invalid("customer_ip_address"));
        }
        body["customer_ip_address"] = serde_json::json!(ip);
    }
    Ok(StripeRequestIntent {
        method: "POST".to_string(),
        endpoint: ONRAMP_SESSIONS_ENDPOINT.to_string(),
        api_version: STRIPE_API_VERSION.to_string(),
        idempotency_key: format!("onramp_session:{wallet}:{request_id}"),
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stripe_form_encode;

    fn request() -> StripeOnrampSessionRequest {
        StripeOnrampSessionRequest {
            wallet_address: "0xDE0B295669a9FD93d5F28D9Ec85E40f4cb697BAe".to_string(),
            destination_amount: "25.5".to_string(),
            customer_ip_address: Some("8.8.8.8".to_string()),
            destination_network: "base".to_string(),
            wallet_address_key: "base".to_string(),
            request_id: "req_1".to_string(),
        }
    }

    #[test]
    fn session_locks_usdc_on_base_to_the_users_wallet() {
        let intent = plan_onramp_session(&request()).unwrap();
        assert_eq!(intent.endpoint, "/v1/crypto/onramp_sessions");
        assert_eq!(
            intent.idempotency_key,
            "onramp_session:0xde0b295669a9fd93d5f28d9ec85e40f4cb697bae:req_1"
        );
        let form = stripe_form_encode(&intent.body);
        for expected in [
            "wallet_addresses%5Bbase%5D=0xde0b295669a9fd93d5f28d9ec85e40f4cb697bae",
            "lock_wallet_address=true",
            "destination_currency=usdc",
            "destination_network=base",
            "destination_currencies%5B0%5D=usdc",
            "destination_networks%5B0%5D=base",
            "destination_amount=25.50",
            "customer_ip_address=8.8.8.8",
        ] {
            assert!(form.contains(expected), "{expected} missing from {form}");
        }
    }

    #[test]
    fn session_rejects_bad_wallets_amounts_and_identifiers() {
        let cases: Vec<Box<dyn Fn(&mut StripeOnrampSessionRequest)>> = vec![
            Box::new(|request| request.wallet_address = "0x123".to_string()),
            Box::new(|request| request.wallet_address = format!("0x{}", "0".repeat(40))),
            Box::new(|request| request.destination_amount = "0".to_string()),
            Box::new(|request| request.destination_amount = "1.005".to_string()),
            Box::new(|request| request.destination_amount = "1e3".to_string()),
            Box::new(|request| request.destination_network = "Base Mainnet".to_string()),
            Box::new(|request| request.wallet_address_key = "base]&x=1".to_string()),
            Box::new(|request| request.customer_ip_address = Some("not-an-ip".to_string())),
        ];
        for mutate in cases {
            let mut bad = request();
            mutate(&mut bad);
            assert!(plan_onramp_session(&bad).is_err());
        }
        assert_eq!(usdc_amount_cents("20").unwrap(), 2_000);
        assert_eq!(usdc_amount_cents("0.01").unwrap(), 1);
    }
}
