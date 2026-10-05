//! Public Stripe crypto onramp sessions for the non-custodial fiat path (ADR 0006).
//!
//! A first-party page asks for a session that delivers Base USDC to the user's own wallet. The
//! wallet address is locked, and Stripe acts as merchant of record. The route is off unless
//! `ENABLE_STRIPE_CRYPTO_ONRAMP=true`. It only accepts approved origins, binds live sessions to a
//! public customer IP, bounds amounts, and rate-limits per client. A session, a purchase, or a
//! wallet balance is never bounty funding; only an indexed `FundingAdded` is.

use super::*;
use payments_stripe::onramp::{plan_onramp_session, usdc_amount_cents, StripeOnrampSessionRequest};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const SCHEMA: &str = "agent-bounties/stripe-crypto-onramp-session-v1";
static SESSION_RATE_LIMITS: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();

fn onramp_error(status: StatusCode, code: &str, message: impl Into<String>) -> AgentActionApiError {
    agent_action_error(
        status,
        code,
        message,
        status == StatusCode::TOO_MANY_REQUESTS,
        "Fund the wallet another way, or retry later. Buying USDC never funds a bounty by itself.",
    )
}

fn setting(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub(crate) fn is_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, ..] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || (first == 100 && (64..128).contains(&second)))
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            !(ip.is_loopback()
                || ip.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Sliding one-minute window per key.
pub(crate) fn allow_session(key: &str, per_minute: usize, now: Instant) -> bool {
    let limits = SESSION_RATE_LIMITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut limits = limits
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let window = limits.entry(key.to_string()).or_default();
    while window
        .front()
        .is_some_and(|seen| now.duration_since(*seen) >= Duration::from_secs(60))
    {
        window.pop_front();
    }
    if window.len() >= per_minute {
        return false;
    }
    window.push_back(now);
    true
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StripeOnrampSessionBody {
    pub wallet_address: String,
    pub destination_amount: String,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct StripeOnrampSession {
    pub schema_version: &'static str,
    pub provider: &'static str,
    pub livemode: bool,
    pub session_id: String,
    /// For mounting Stripe's onramp element on this page only. Do not log or share it.
    pub client_secret: String,
    pub publishable_key: String,
    pub destination_network: String,
    pub wallet_address: String,
    pub destination_amount: String,
    pub state: &'static str,
    pub bounty_funded: bool,
    pub next_action: &'static str,
    pub evidence_boundary: &'static str,
}

pub(crate) async fn create_stripe_onramp_session(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Json(body): Json<StripeOnrampSessionBody>,
) -> Result<Json<StripeOnrampSession>, AgentActionApiError> {
    if env::var("ENABLE_STRIPE_CRYPTO_ONRAMP").ok().as_deref() != Some("true") {
        return Err(onramp_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "stripe_onramp_disabled",
            "the Stripe crypto onramp is not enabled",
        ));
    }
    let secret_key = state.stripe_secret_key.as_deref().unwrap_or_default();
    let publishable_key = setting("STRIPE_CRYPTO_ONRAMP_PUBLISHABLE_KEY", "");
    let livemode = match (
        secret_key.starts_with("sk_live_") || secret_key.starts_with("rk_live_"),
        secret_key.starts_with("sk_test_") || secret_key.starts_with("rk_test_"),
        publishable_key.starts_with("pk_live_"),
        publishable_key.starts_with("pk_test_"),
    ) {
        (true, false, true, false) => true,
        (false, true, false, true) => false,
        _ => {
            return Err(onramp_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "stripe_onramp_misconfigured",
                "STRIPE_SECRET_KEY and STRIPE_CRYPTO_ONRAMP_PUBLISHABLE_KEY must be the same Stripe mode",
            ))
        }
    };
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .trim_end_matches('/')
        .to_string();
    let allowed = setting(
        "STRIPE_CRYPTO_ONRAMP_ALLOWED_ORIGINS",
        "https://agentbounties.app",
    );
    if origin.is_empty()
        || !allowed
            .split(',')
            .any(|candidate| candidate.trim().trim_end_matches('/') == origin)
    {
        return Err(onramp_error(
            StatusCode::FORBIDDEN,
            "stripe_onramp_origin_not_allowed",
            "this origin is not approved for onramp sessions",
        ));
    }
    let cents = usdc_amount_cents(&body.destination_amount).map_err(|_| {
        onramp_error(
            StatusCode::BAD_REQUEST,
            "stripe_onramp_invalid_amount",
            "destination_amount must be a positive USDC amount with at most two decimals",
        )
    })?;
    let parse_cents =
        |name: &str, default: u64| usdc_amount_cents(&setting(name, "")).unwrap_or(default);
    let (minimum, maximum) = (
        parse_cents("STRIPE_CRYPTO_ONRAMP_MIN_USDC", 100),
        parse_cents("STRIPE_CRYPTO_ONRAMP_MAX_USDC", 250_000),
    );
    if cents < minimum || cents > maximum {
        return Err(onramp_error(
            StatusCode::BAD_REQUEST,
            "stripe_onramp_amount_out_of_bounds",
            format!(
                "destination_amount must be between {}.{:02} and {}.{:02} USDC",
                minimum / 100,
                minimum % 100,
                maximum / 100,
                maximum % 100
            ),
        ));
    }
    let client_ip = headers
        .get(setting("STRIPE_CRYPTO_ONRAMP_CLIENT_IP_HEADER", "x-forwarded-for").as_str())
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .and_then(|value| value.trim().parse::<IpAddr>().ok());
    if livemode && !client_ip.as_ref().is_some_and(is_public_ip) {
        return Err(onramp_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "stripe_onramp_client_ip_unavailable",
            "a public customer IP is required for a live onramp session",
        ));
    }
    let wallet = body.wallet_address.trim().to_ascii_lowercase();
    let limiter_key = client_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| format!("{origin}:{wallet}"));
    let per_minute = setting("STRIPE_CRYPTO_ONRAMP_SESSIONS_PER_MINUTE", "5")
        .parse()
        .unwrap_or(5);
    if !allow_session(&limiter_key, per_minute, Instant::now()) {
        return Err(onramp_error(
            StatusCode::TOO_MANY_REQUESTS,
            "stripe_onramp_rate_limited",
            "too many onramp sessions were requested from this client",
        ));
    }
    let network = setting("STRIPE_CRYPTO_ONRAMP_NETWORK", "base");
    let intent = plan_onramp_session(&StripeOnrampSessionRequest {
        wallet_address: wallet.clone(),
        destination_amount: body.destination_amount.clone(),
        customer_ip_address: client_ip.map(|ip| ip.to_string()),
        destination_network: network.clone(),
        wallet_address_key: setting("STRIPE_CRYPTO_ONRAMP_WALLET_KEY", &network),
        request_id: body
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
    })
    .map_err(|error| {
        onramp_error(
            StatusCode::BAD_REQUEST,
            "stripe_onramp_invalid_request",
            error.to_string(),
        )
    })?;
    let report = execute_stripe_request(&intent, secret_key, &state.stripe_api_base_url)
        .await
        .map_err(|error| {
            onramp_error(
                StatusCode::BAD_GATEWAY,
                "stripe_onramp_unavailable",
                error.to_string(),
            )
        })?;
    let session_id = report.stripe_id.clone().filter(|id| id.starts_with("cos_"));
    let client_secret = report
        .response
        .get("client_secret")
        .and_then(|value| value.as_str())
        .map(ToString::to_string);
    let (Some(session_id), Some(client_secret)) = (session_id, client_secret) else {
        return Err(onramp_error(
            StatusCode::BAD_GATEWAY,
            "stripe_onramp_unavailable",
            "Stripe did not return an onramp session",
        ));
    };
    if report.livemode != Some(livemode) {
        return Err(onramp_error(
            StatusCode::BAD_GATEWAY,
            "stripe_onramp_mode_mismatch",
            "Stripe returned a session in the wrong mode",
        ));
    }
    Ok(Json(StripeOnrampSession {
        schema_version: SCHEMA,
        provider: "stripe",
        livemode,
        session_id,
        client_secret,
        publishable_key,
        destination_network: network,
        wallet_address: wallet,
        destination_amount: format!("{}.{:02}", cents / 100, cents % 100),
        state: "onramp_session_ready_wallet_not_yet_topped_up",
        bounty_funded: false,
        next_action: "Complete the Stripe onramp, wait for the USDC to reach your wallet, then sign the exact bounty contribution.",
        evidence_boundary: "Stripe is the merchant of record for the USDC purchase. A session, purchase, or wallet balance is not bounty funding; only the canonical FundingAdded event is.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_qualify_for_live_sessions() {
        for (ip, public) in [
            ("8.8.8.8", true),
            ("2001:4860:4860::8888", true),
            ("10.1.2.3", false),
            ("192.168.1.1", false),
            ("100.64.0.1", false),
            ("127.0.0.1", false),
            ("::1", false),
            ("fd00::1", false),
            ("fe80::1", false),
        ] {
            assert_eq!(is_public_ip(&ip.parse().unwrap()), public, "{ip}");
        }
    }

    #[test]
    fn session_rate_limit_slides_per_client() {
        let key = format!("test-{}", Uuid::new_v4());
        let start = Instant::now();
        assert!(allow_session(&key, 2, start));
        assert!(allow_session(&key, 2, start));
        assert!(!allow_session(&key, 2, start + Duration::from_secs(30)));
        assert!(allow_session(&key, 2, start + Duration::from_secs(61)));
    }
}
