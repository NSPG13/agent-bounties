//! Public Stripe crypto onramp sessions for the non-custodial fiat path (ADR 0006).
//!
//! A first-party page asks for a session that delivers Base USDC to the user's own wallet. The
//! wallet address is locked, and Stripe acts as merchant of record. The route is off unless
//! `ENABLE_STRIPE_CRYPTO_ONRAMP=true`. It only accepts approved origins, binds live sessions to a
//! public customer IP, bounds amounts, and rate-limits per client, per wallet and globally. A
//! session, a purchase, or a wallet balance is never bounty funding; only an indexed
//! `FundingAdded` is.
//!
//! The Origin header is not authentication: scripts can send any value. The client IP is the one
//! the outermost trusted proxy recorded, never the client-supplied left of `X-Forwarded-For`, and
//! the global and per-wallet caps bound Stripe API use however the IP is derived.

use super::*;
use payments_stripe::onramp::{plan_onramp_session, usdc_amount_cents, StripeOnrampSessionRequest};
use service_runtime::client_ip::{forwarded_client_ip, is_public_ip, rate_limit_bucket};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const SCHEMA: &str = "agent-bounties/stripe-crypto-onramp-session-v1";
/// Distinct rate-limit keys held at once. Beyond it new keys are refused, so attacker-chosen keys
/// can never grow memory without bound.
const MAX_RATE_LIMIT_KEYS: usize = 10_000;
const GLOBAL_KEY: &str = "global";
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

/// Sliding one-minute windows. A session is admitted only when every `(key, per_minute)` limit has
/// room, and is then counted against all of them. Empty windows are dropped, and new keys are
/// refused once `MAX_RATE_LIMIT_KEYS` are held.
pub(crate) fn allow_session(limits: &[(String, usize)], now: Instant) -> bool {
    let windows = SESSION_RATE_LIMITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut windows = windows
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    admit(&mut windows, limits, now, MAX_RATE_LIMIT_KEYS)
}

fn admit(
    windows: &mut HashMap<String, VecDeque<Instant>>,
    limits: &[(String, usize)],
    now: Instant,
    max_keys: usize,
) -> bool {
    windows.retain(|_, seen| {
        while seen
            .front()
            .is_some_and(|at| now.duration_since(*at) >= Duration::from_secs(60))
        {
            seen.pop_front();
        }
        !seen.is_empty()
    });
    let new_keys = limits
        .iter()
        .filter(|(key, _)| !windows.contains_key(key))
        .count();
    if windows.len() + new_keys > max_keys
        || limits
            .iter()
            .any(|(key, per_minute)| windows.get(key).map_or(0, VecDeque::len) >= *per_minute)
    {
        return false;
    }
    for (key, _) in limits {
        windows.entry(key.clone()).or_default().push_back(now);
    }
    true
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StripeOnrampSessionBody {
    pub wallet_address: String,
    pub destination_amount: String,
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
    let hops = setting("STRIPE_CRYPTO_ONRAMP_TRUSTED_PROXY_HOPS", "1")
        .parse::<usize>()
        .unwrap_or(1);
    let client_ip = headers
        .get(setting("STRIPE_CRYPTO_ONRAMP_CLIENT_IP_HEADER", "x-forwarded-for").as_str())
        .and_then(|value| value.to_str().ok())
        .and_then(|value| forwarded_client_ip(value, hops));
    if livemode && !client_ip.is_some_and(is_public_ip) {
        return Err(onramp_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "stripe_onramp_client_ip_unavailable",
            "a public customer IP is required for a live onramp session",
        ));
    }
    let network = setting("STRIPE_CRYPTO_ONRAMP_NETWORK", "base");
    // Validate every input before it can reach the rate limiter, so limiter keys are bounded.
    // The idempotency id is always server-generated: a caller-chosen id could replay another
    // caller's session, client secret included.
    let intent = plan_onramp_session(&StripeOnrampSessionRequest {
        wallet_address: body.wallet_address.clone(),
        destination_amount: body.destination_amount.clone(),
        customer_ip_address: client_ip.map(|ip| ip.to_string()),
        destination_network: network.clone(),
        wallet_address_key: setting("STRIPE_CRYPTO_ONRAMP_WALLET_KEY", &network),
        request_id: Uuid::new_v4().simple().to_string(),
    })
    .map_err(|error| {
        onramp_error(
            StatusCode::BAD_REQUEST,
            "stripe_onramp_invalid_request",
            error.to_string(),
        )
    })?;
    let wallet = body.wallet_address.trim().to_ascii_lowercase();
    let limit = |name: &str, default: usize| {
        setting(name, &default.to_string())
            .parse::<usize>()
            .unwrap_or(default)
    };
    let mut limits = vec![
        (
            GLOBAL_KEY.to_string(),
            limit("STRIPE_CRYPTO_ONRAMP_GLOBAL_SESSIONS_PER_MINUTE", 30),
        ),
        (
            format!("wallet:{wallet}"),
            limit("STRIPE_CRYPTO_ONRAMP_WALLET_SESSIONS_PER_MINUTE", 3),
        ),
    ];
    if let Some(ip) = client_ip {
        limits.push((
            format!("ip:{}", rate_limit_bucket(ip)),
            limit("STRIPE_CRYPTO_ONRAMP_SESSIONS_PER_MINUTE", 5),
        ));
    }
    if !allow_session(&limits, Instant::now()) {
        return Err(onramp_error(
            StatusCode::TOO_MANY_REQUESTS,
            "stripe_onramp_rate_limited",
            "too many onramp sessions were requested; retry in a minute",
        ));
    }
    let report = execute_stripe_request(&intent, secret_key, &state.stripe_api_base_url)
        .await
        .map_err(|_| {
            // Stripe's error body carries request-log links and account details; it stays
            // server-side.
            onramp_error(
                StatusCode::BAD_GATEWAY,
                "stripe_onramp_unavailable",
                "Stripe could not create the onramp session",
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
    fn session_limits_slide_and_every_limit_must_have_room() {
        let client = format!("test-ip-{}", Uuid::new_v4());
        let wallet = format!("test-wallet-{}", Uuid::new_v4());
        let start = Instant::now();
        let limits =
            |client_limit: usize| vec![(client.clone(), client_limit), (wallet.clone(), 2)];
        assert!(allow_session(&limits(5), start));
        assert!(allow_session(&limits(5), start));
        assert!(
            !allow_session(&limits(5), start + Duration::from_secs(30)),
            "the wallet cap binds even when the client has room"
        );
        assert!(allow_session(&limits(5), start + Duration::from_secs(61)));
    }

    #[test]
    fn limiter_memory_is_bounded_and_expired_windows_are_dropped() {
        let mut windows = HashMap::new();
        let start = Instant::now();
        let key = |index: usize| vec![(format!("key-{index}"), 5)];
        assert!(admit(&mut windows, &key(1), start, 2));
        assert!(admit(&mut windows, &key(2), start, 2));
        assert!(
            !admit(&mut windows, &key(3), start, 2),
            "a new key is refused once the map is full"
        );
        assert!(
            admit(&mut windows, &key(1), start, 2),
            "known keys still work"
        );
        let later = start + Duration::from_secs(61);
        assert!(admit(&mut windows, &key(3), later, 2));
        assert_eq!(windows.len(), 1, "expired windows are removed");
    }
}
