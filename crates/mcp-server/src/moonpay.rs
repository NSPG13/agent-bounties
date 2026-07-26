use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::Sha256;
use std::{
    collections::{HashMap, VecDeque},
    env,
    net::IpAddr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use url::Url;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const RESPONSE_SCHEMA: &str = "agent-bounties/moonpay-onramp-checkout-v1";
const DEFAULT_ALLOWED_ORIGIN: &str = "https://agentbounties.app";
const DEFAULT_CLIENT_IP_HEADER: &str = "x-forwarded-for";
const DEFAULT_RATE_LIMIT_PER_MINUTE: u32 = 10;
const DEFAULT_MIN_FIAT_MINOR: u64 = 100;
const DEFAULT_MAX_FIAT_MINOR: u64 = 1_000_000;
const MAX_URL_LENGTH: usize = 4_000;
const MAX_CURRENCY_CODE_LENGTH: usize = 48;
const LIVE_CHECKOUT_BASE: &str = "https://buy.moonpay.com/";
const SANDBOX_CHECKOUT_BASE: &str = "https://buy-sandbox.moonpay.com/";

static CHECKOUT_RATE_LIMITS: OnceLock<Mutex<HashMap<String, VecDeque<Instant>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoonpayEnvironment {
    Sandbox,
    Live,
}

impl MoonpayEnvironment {
    fn as_str(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Live => "live",
        }
    }

    fn checkout_base(self) -> &'static str {
        match self {
            Self::Sandbox => SANDBOX_CHECKOUT_BASE,
            Self::Live => LIVE_CHECKOUT_BASE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnrampAsset {
    Usdc,
    Eth,
}

impl OnrampAsset {
    fn parse(value: Option<&str>) -> Result<Self, ApiError> {
        match value.unwrap_or("usdc").trim().to_ascii_lowercase().as_str() {
            "usdc" => Ok(Self::Usdc),
            "eth" => Ok(Self::Eth),
            _ => Err(ApiError::bad_request(
                "unsupported_asset",
                "asset must be usdc or eth",
                "Choose Base USDC for bounty funding or Base ETH for transaction gas.",
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Usdc => "USDC",
            Self::Eth => "ETH",
        }
    }

    fn live_currency_env(self) -> &'static str {
        match self {
            Self::Usdc => "MOONPAY_USDC_BASE_CURRENCY_CODE",
            Self::Eth => "MOONPAY_ETH_BASE_CURRENCY_CODE",
        }
    }

    fn live_currency_default(self) -> &'static str {
        match self {
            Self::Usdc => "usdc_base",
            Self::Eth => "eth_base",
        }
    }

    fn sandbox_currency_env(self) -> &'static str {
        match self {
            Self::Usdc => "MOONPAY_SANDBOX_USDC_CURRENCY_CODE",
            Self::Eth => "MOONPAY_SANDBOX_ETH_CURRENCY_CODE",
        }
    }

    fn sandbox_currency_default(self) -> &'static str {
        match self {
            Self::Usdc => "usdc",
            Self::Eth => "eth",
        }
    }
}

#[derive(Debug, Clone)]
struct MoonpayConfig {
    publishable_key: String,
    secret_key: String,
    environment: MoonpayEnvironment,
    allowed_origins: Vec<String>,
    client_ip_header: String,
    min_fiat_minor: u64,
    max_fiat_minor: u64,
    rate_limit_per_minute: u32,
}

impl MoonpayConfig {
    fn from_env() -> Result<Self, ApiError> {
        let publishable_key = required_env("MOONPAY_PUBLISHABLE_KEY")?;
        let secret_key = required_env("MOONPAY_SECRET_KEY")?;
        let environment = parse_environment(
            env::var("MOONPAY_ENVIRONMENT").ok().as_deref(),
            &publishable_key,
        )?;
        validate_key_pair(environment, &publishable_key, &secret_key)?;

        let allowed_origins = env::var("MOONPAY_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| DEFAULT_ALLOWED_ORIGIN.to_string())
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(canonical_origin)
            .collect::<Result<Vec<_>, _>>()?;
        if allowed_origins.is_empty() {
            return Err(ApiError::configuration(
                "moonpay_origin_not_configured",
                "MOONPAY_ALLOWED_ORIGINS must contain at least one valid web origin",
            ));
        }

        let client_ip_header = env::var("MOONPAY_CLIENT_IP_HEADER")
            .unwrap_or_else(|_| DEFAULT_CLIENT_IP_HEADER.to_string())
            .trim()
            .to_ascii_lowercase();
        if !valid_header_name(&client_ip_header) {
            return Err(ApiError::configuration(
                "moonpay_client_ip_header_invalid",
                "MOONPAY_CLIENT_IP_HEADER is not a valid HTTP header name",
            ));
        }

        let min_fiat_minor = optional_minor_env("MOONPAY_MIN_FIAT_AMOUNT", DEFAULT_MIN_FIAT_MINOR)?;
        let max_fiat_minor = optional_minor_env("MOONPAY_MAX_FIAT_AMOUNT", DEFAULT_MAX_FIAT_MINOR)?;
        if max_fiat_minor < min_fiat_minor {
            return Err(ApiError::configuration(
                "moonpay_amount_bounds_invalid",
                "MOONPAY_MAX_FIAT_AMOUNT must be at least MOONPAY_MIN_FIAT_AMOUNT",
            ));
        }

        let rate_limit_per_minute = env::var("MOONPAY_CHECKOUTS_PER_MINUTE")
            .ok()
            .map(|value| value.trim().parse::<u32>())
            .transpose()
            .map_err(|_| {
                ApiError::configuration(
                    "moonpay_rate_limit_invalid",
                    "MOONPAY_CHECKOUTS_PER_MINUTE must be a positive integer",
                )
            })?
            .unwrap_or(DEFAULT_RATE_LIMIT_PER_MINUTE);
        if rate_limit_per_minute == 0 || rate_limit_per_minute > 120 {
            return Err(ApiError::configuration(
                "moonpay_rate_limit_invalid",
                "MOONPAY_CHECKOUTS_PER_MINUTE must be between 1 and 120",
            ));
        }

        Ok(Self {
            publishable_key,
            secret_key,
            environment,
            allowed_origins,
            client_ip_header,
            min_fiat_minor,
            max_fiat_minor,
            rate_limit_per_minute,
        })
    }

    fn currency_code(&self, asset: OnrampAsset) -> Result<String, ApiError> {
        let (env_name, default) = match self.environment {
            MoonpayEnvironment::Live => (asset.live_currency_env(), asset.live_currency_default()),
            MoonpayEnvironment::Sandbox => (
                asset.sandbox_currency_env(),
                asset.sandbox_currency_default(),
            ),
        };
        let value = env::var(env_name).unwrap_or_else(|_| default.to_string());
        validate_currency_code(&value).map_err(|message| {
            ApiError::configuration(
                "moonpay_currency_code_invalid",
                format!("{env_name}: {message}"),
            )
        })?;
        Ok(value)
    }
}

#[derive(Debug, Deserialize)]
pub struct PrepareCheckoutRequest {
    wallet_address: String,
    base_currency_amount: String,
    #[serde(default)]
    base_currency_code: Option<String>,
    #[serde(default)]
    asset: Option<String>,
    return_url: String,
    #[serde(default)]
    intent_id: Option<Uuid>,
    #[serde(default)]
    bounty_contract: Option<String>,
}

#[derive(Debug, Serialize)]
struct CheckoutPlan {
    schema_version: &'static str,
    provider: &'static str,
    environment: &'static str,
    state: &'static str,
    asset: &'static str,
    destination_network: &'static str,
    production_destination_network: &'static str,
    destination_wallet: String,
    base_currency_code: String,
    base_currency_amount: String,
    bounty_contract: Option<String>,
    intent_id: Option<Uuid>,
    external_transaction_id: String,
    checkout_url: String,
    protocol_action_completed: bool,
    canonical_event: Option<String>,
    bounty_funded: bool,
    canonical_funding_event: Option<String>,
    next_action: &'static str,
    evidence_boundary: &'static str,
    sandbox_notice: Option<&'static str>,
}

#[derive(Debug)]
struct ValidatedCheckoutRequest {
    wallet_address: String,
    base_currency_amount: String,
    base_currency_code: String,
    amount_minor: u64,
    asset: OnrampAsset,
    return_url: String,
    intent_id: Option<Uuid>,
    bounty_contract: Option<String>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    next_action: String,
}

impl ApiError {
    fn bad_request(
        code: &'static str,
        message: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            next_action: next_action.into(),
        }
    }

    fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code,
            message: message.into(),
            next_action: "Open the on-ramp from an approved Agent Bounties page.".to_string(),
        }
    }

    fn configuration(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: message.into(),
            next_action: "Configure the MoonPay partner credentials and retry without exposing the secret key to the browser.".to_string(),
        }
    }

    fn too_many_requests() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "moonpay_checkout_rate_limited",
            message: "Too many MoonPay checkout links were requested from this device.".to_string(),
            next_action: "Wait briefly, then retry once. Do not open several purchase sessions for the same wallet.".to_string(),
        }
    }

    fn into_response(self) -> Response {
        json_response(
            self.status,
            json!({
                "schema_version": RESPONSE_SCHEMA,
                "code": self.code,
                "error": self.message,
                "next_action": self.next_action,
                "protocol_action_completed": false,
                "canonical_event": null,
                "bounty_funded": false,
                "canonical_funding_event": null,
            }),
        )
    }
}

pub async fn prepare_checkout(
    headers: HeaderMap,
    Json(request): Json<PrepareCheckoutRequest>,
) -> Response {
    match prepare_checkout_inner(&headers, request) {
        Ok(plan) => json_response(StatusCode::OK, json!(plan)),
        Err(error) => error.into_response(),
    }
}

fn prepare_checkout_inner(
    headers: &HeaderMap,
    request: PrepareCheckoutRequest,
) -> Result<CheckoutPlan, ApiError> {
    let config = MoonpayConfig::from_env()?;
    let request_origin = request_origin(headers)?;
    if !config
        .allowed_origins
        .iter()
        .any(|allowed| allowed == &request_origin)
    {
        return Err(ApiError::forbidden(
            "moonpay_origin_not_allowed",
            "The request origin is not approved for MoonPay checkout creation.",
        ));
    }

    let validated = validate_request(&config, &request_origin, request)?;
    let client_ip = client_ip(headers, &config.client_ip_header)?;
    if config.environment == MoonpayEnvironment::Live && client_ip.is_none() {
        return Err(ApiError::configuration(
            "moonpay_client_ip_unavailable",
            "A public customer IP is required to create a live MoonPay checkout URL",
        ));
    }
    if let Some(ip) = client_ip.as_deref() {
        if config.environment == MoonpayEnvironment::Live && !is_public_ip(ip)? {
            return Err(ApiError::configuration(
                "moonpay_client_ip_not_public",
                "The configured proxy header did not contain a public customer IP address",
            ));
        }
    }

    let limiter_key = client_ip
        .clone()
        .unwrap_or_else(|| format!("{}:{}", request_origin, validated.wallet_address));
    enforce_rate_limit(&limiter_key, config.rate_limit_per_minute)?;

    build_checkout_plan(&config, validated, client_ip.as_deref())
}

fn validate_request(
    config: &MoonpayConfig,
    request_origin: &str,
    request: PrepareCheckoutRequest,
) -> Result<ValidatedCheckoutRequest, ApiError> {
    let wallet_address = normalize_evm_address(&request.wallet_address, "wallet_address")?;
    let bounty_contract = request
        .bounty_contract
        .as_deref()
        .map(|value| normalize_evm_address(value, "bounty_contract"))
        .transpose()?;
    let (amount_minor, base_currency_amount) = parse_fiat_amount(&request.base_currency_amount)?;
    if amount_minor < config.min_fiat_minor || amount_minor > config.max_fiat_minor {
        return Err(ApiError::bad_request(
            "moonpay_amount_out_of_bounds",
            format!(
                "base_currency_amount must be between {} and {}",
                format_minor(config.min_fiat_minor),
                format_minor(config.max_fiat_minor)
            ),
            "Choose an amount inside the configured safety bounds. MoonPay may apply additional region-specific limits in checkout.",
        ));
    }

    let base_currency_code = request
        .base_currency_code
        .unwrap_or_else(|| "usd".to_string())
        .trim()
        .to_ascii_lowercase();
    if base_currency_code != "usd" {
        return Err(ApiError::bad_request(
            "unsupported_base_currency",
            "This first integration accepts USD-prefilled MoonPay checkouts only.",
            "Continue in USD; MoonPay can show the final local payment methods and conversion before approval.",
        ));
    }

    let asset = OnrampAsset::parse(request.asset.as_deref())?;
    let return_url = validate_return_url(&request.return_url, request_origin)?;

    Ok(ValidatedCheckoutRequest {
        wallet_address,
        base_currency_amount,
        base_currency_code,
        amount_minor,
        asset,
        return_url,
        intent_id: request.intent_id,
        bounty_contract,
    })
}

fn build_checkout_plan(
    config: &MoonpayConfig,
    request: ValidatedCheckoutRequest,
    client_ip: Option<&str>,
) -> Result<CheckoutPlan, ApiError> {
    debug_assert!(request.amount_minor > 0);
    let currency_code = config.currency_code(request.asset)?;
    let external_transaction_id = match request.intent_id {
        Some(intent_id) => format!("ab-intent-{intent_id}-{}", short_nonce()),
        None => format!("ab-web-{}", Uuid::new_v4()),
    };

    let mut checkout = Url::parse(config.environment.checkout_base()).map_err(|_| {
        ApiError::configuration(
            "moonpay_checkout_base_invalid",
            "The configured MoonPay checkout base URL is invalid",
        )
    })?;
    {
        let mut query = checkout.query_pairs_mut();
        query.append_pair("apiKey", &config.publishable_key);
        query.append_pair("currencyCode", &currency_code);
        query.append_pair("walletAddress", &request.wallet_address);
        query.append_pair("baseCurrencyCode", &request.base_currency_code);
        query.append_pair("baseCurrencyAmount", &request.base_currency_amount);
        query.append_pair("externalTransactionId", &external_transaction_id);
        query.append_pair("redirectURL", &request.return_url);
        if config.environment == MoonpayEnvironment::Live {
            let ip = client_ip.ok_or_else(|| {
                ApiError::configuration(
                    "moonpay_client_ip_unavailable",
                    "A public customer IP is required to create a live MoonPay checkout URL",
                )
            })?;
            let allowed_ip = hmac_base64(&config.secret_key, ip)?;
            query.append_pair("allowedIpAddress", &allowed_ip);
        }
    }

    let unsigned_query = checkout
        .query()
        .map(|value| format!("?{value}"))
        .ok_or_else(|| {
            ApiError::configuration(
                "moonpay_checkout_query_missing",
                "The MoonPay checkout URL could not be assembled",
            )
        })?;
    let signature = hmac_base64(&config.secret_key, &unsigned_query)?;
    checkout
        .query_pairs_mut()
        .append_pair("signature", &signature);

    let is_sandbox = config.environment == MoonpayEnvironment::Sandbox;
    Ok(CheckoutPlan {
        schema_version: RESPONSE_SCHEMA,
        provider: "moonpay",
        environment: config.environment.as_str(),
        state: "checkout_ready_wallet_not_yet_topped_up",
        asset: request.asset.label(),
        destination_network: if is_sandbox {
            "moonpay-test-mode"
        } else {
            "base-mainnet"
        },
        production_destination_network: "base-mainnet",
        destination_wallet: request.wallet_address,
        base_currency_code: request.base_currency_code,
        base_currency_amount: request.base_currency_amount,
        bounty_contract: request.bounty_contract,
        intent_id: request.intent_id,
        external_transaction_id,
        checkout_url: checkout.to_string(),
        protocol_action_completed: false,
        canonical_event: None,
        bounty_funded: false,
        canonical_funding_event: None,
        next_action: "Complete the MoonPay checkout, return to Agent Bounties, verify the Base USDC balance, and separately approve the exact original bounty action.",
        evidence_boundary: "MoonPay checkout status and wallet top-up evidence do not complete any Agent Bounties action. Only the matching indexed canonical protocol event changes bounty state.",
        sandbox_notice: is_sandbox.then_some(
            "MoonPay sandbox uses simulated payments and test assets. It validates checkout integration but does not top up Base mainnet.",
        ),
    })
}

fn request_origin(headers: &HeaderMap) -> Result<String, ApiError> {
    let value = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::forbidden(
                "moonpay_origin_required",
                "A browser Origin header is required for checkout creation.",
            )
        })?;
    canonical_origin(value).map_err(|_| {
        ApiError::forbidden(
            "moonpay_origin_invalid",
            "The browser Origin header is invalid.",
        )
    })
}

fn canonical_origin(value: &str) -> Result<String, ApiError> {
    let parsed = Url::parse(value.trim()).map_err(|_| {
        ApiError::configuration(
            "moonpay_origin_invalid",
            format!("invalid web origin: {value}"),
        )
    })?;
    if !matches!(parsed.scheme(), "https" | "http")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.host_str().is_none()
    {
        return Err(ApiError::configuration(
            "moonpay_origin_invalid",
            format!("invalid web origin: {value}"),
        ));
    }
    Ok(parsed.origin().ascii_serialization())
}

fn validate_return_url(value: &str, request_origin: &str) -> Result<String, ApiError> {
    if value.len() > MAX_URL_LENGTH {
        return Err(ApiError::bad_request(
            "moonpay_return_url_too_long",
            "return_url is too long",
            "Return to the bounded Agent Bounties on-ramp page without extra tracking parameters.",
        ));
    }
    let parsed = Url::parse(value).map_err(|_| {
        ApiError::bad_request(
            "moonpay_return_url_invalid",
            "return_url must be an absolute URL",
            "Use the current Agent Bounties on-ramp page as the return URL.",
        )
    })?;
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed.origin().ascii_serialization() != request_origin
        || parsed.path() != "/onramp.html"
    {
        return Err(ApiError::bad_request(
            "moonpay_return_url_not_allowed",
            "return_url must point to /onramp.html on the requesting Agent Bounties origin",
            "Use the on-ramp page's own URL without a fragment.",
        ));
    }
    Ok(parsed.to_string())
}

fn normalize_evm_address(value: &str, field: &'static str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.len() != 42
        || !trimmed.starts_with("0x")
        || !trimmed[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(ApiError::bad_request(
            "invalid_evm_address",
            format!("{field} must be a 20-byte 0x-prefixed EVM address"),
            "Reconnect the intended Base wallet and retry.",
        ));
    }
    Ok(format!("0x{}", trimmed[2..].to_ascii_lowercase()))
}

fn parse_fiat_amount(value: &str) -> Result<(u64, String), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > 32
        || trimmed.starts_with('+')
        || trimmed.starts_with('-')
    {
        return Err(invalid_amount_error());
    }
    let mut parts = trimmed.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.chars().all(|character| character.is_ascii_digit())
        || fraction.is_some_and(|value| {
            value.len() > 2 || !value.chars().all(|character| character.is_ascii_digit())
        })
    {
        return Err(invalid_amount_error());
    }
    let whole_minor = whole
        .parse::<u64>()
        .ok()
        .and_then(|amount| amount.checked_mul(100))
        .ok_or_else(invalid_amount_error)?;
    let fraction_minor = match fraction.unwrap_or("") {
        "" => 0,
        one if one.len() == 1 => one.parse::<u64>().map_err(|_| invalid_amount_error())? * 10,
        two => two.parse::<u64>().map_err(|_| invalid_amount_error())?,
    };
    let amount_minor = whole_minor
        .checked_add(fraction_minor)
        .ok_or_else(invalid_amount_error)?;
    if amount_minor == 0 {
        return Err(invalid_amount_error());
    }
    Ok((amount_minor, format_minor(amount_minor)))
}

fn invalid_amount_error() -> ApiError {
    ApiError::bad_request(
        "invalid_fiat_amount",
        "base_currency_amount must be a positive decimal with at most two fractional digits",
        "Enter the fiat amount you want MoonPay to quote, then review MoonPay's final fees and received amount before approval.",
    )
}

fn format_minor(value: u64) -> String {
    format!("{}.{:02}", value / 100, value % 100)
}

fn optional_minor_env(name: &'static str, default: u64) -> Result<u64, ApiError> {
    match env::var(name).ok() {
        Some(value) => parse_fiat_amount(&value).map(|(minor, _)| minor),
        None => Ok(default),
    }
}

fn required_env(name: &'static str) -> Result<String, ApiError> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ApiError::configuration(
                "moonpay_not_configured",
                format!("{name} is required before MoonPay checkout creation is enabled"),
            )
        })
}

fn parse_environment(
    configured: Option<&str>,
    publishable_key: &str,
) -> Result<MoonpayEnvironment, ApiError> {
    let inferred = if publishable_key.starts_with("pk_live_") {
        "live"
    } else {
        "sandbox"
    };
    match configured
        .unwrap_or(inferred)
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "sandbox" | "test" => Ok(MoonpayEnvironment::Sandbox),
        "live" | "production" => Ok(MoonpayEnvironment::Live),
        _ => Err(ApiError::configuration(
            "moonpay_environment_invalid",
            "MOONPAY_ENVIRONMENT must be sandbox or live",
        )),
    }
}

fn validate_key_pair(
    environment: MoonpayEnvironment,
    publishable_key: &str,
    secret_key: &str,
) -> Result<(), ApiError> {
    let valid = match environment {
        MoonpayEnvironment::Sandbox => {
            publishable_key.starts_with("pk_test_") && secret_key.starts_with("sk_test_")
        }
        MoonpayEnvironment::Live => {
            publishable_key.starts_with("pk_live_") && secret_key.starts_with("sk_live_")
        }
    };
    if valid {
        Ok(())
    } else {
        Err(ApiError::configuration(
            "moonpay_key_environment_mismatch",
            "MoonPay publishable and secret keys must both match MOONPAY_ENVIRONMENT",
        ))
    }
}

fn validate_currency_code(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_CURRENCY_CODE_LENGTH
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        Err("currency code must contain only letters, numbers, underscores, or hyphens".to_string())
    } else {
        Ok(())
    }
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn client_ip(headers: &HeaderMap, configured_header: &str) -> Result<Option<String>, ApiError> {
    let candidates = [
        configured_header,
        "true-client-ip",
        "x-forwarded-for",
        "x-real-ip",
    ];
    for name in candidates {
        let Some(raw) = headers.get(name).and_then(|value| value.to_str().ok()) else {
            continue;
        };
        let Some(first) = raw
            .split(',')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let parsed = first.parse::<IpAddr>().map_err(|_| {
            ApiError::configuration(
                "moonpay_client_ip_invalid",
                format!("{name} did not contain a valid client IP address"),
            )
        })?;
        return Ok(Some(parsed.to_string()));
    }
    Ok(None)
}

fn is_public_ip(value: &str) -> Result<bool, ApiError> {
    let ip = value.parse::<IpAddr>().map_err(|_| {
        ApiError::configuration(
            "moonpay_client_ip_invalid",
            "The resolved client IP is invalid",
        )
    })?;
    Ok(match ip {
        IpAddr::V4(value) => {
            let octets = value.octets();
            let shared = octets[0] == 100 && (64..=127).contains(&octets[1]);
            !(value.is_private()
                || value.is_loopback()
                || value.is_link_local()
                || value.is_unspecified()
                || value.is_multicast()
                || shared
                || octets[0] == 0
                || octets[0] >= 240)
        }
        IpAddr::V6(value) => {
            !(value.is_loopback()
                || value.is_unspecified()
                || value.is_unique_local()
                || value.is_unicast_link_local()
                || value.is_multicast())
        }
    })
}

fn enforce_rate_limit(key: &str, limit: u32) -> Result<(), ApiError> {
    let now = Instant::now();
    let window = Duration::from_secs(60);
    let limits = CHECKOUT_RATE_LIMITS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = limits.lock().map_err(|_| {
        ApiError::configuration(
            "moonpay_rate_limiter_unavailable",
            "The MoonPay checkout rate limiter is unavailable",
        )
    })?;
    guard.retain(|_, attempts| {
        while attempts
            .front()
            .is_some_and(|started| now.duration_since(*started) >= window)
        {
            attempts.pop_front();
        }
        !attempts.is_empty()
    });
    let attempts = guard.entry(key.to_string()).or_default();
    if attempts.len() >= limit as usize {
        return Err(ApiError::too_many_requests());
    }
    attempts.push_back(now);
    Ok(())
}

fn hmac_base64(secret: &str, message: &str) -> Result<String, ApiError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|_| {
        ApiError::configuration(
            "moonpay_secret_invalid",
            "The MoonPay secret key could not initialize URL signing",
        )
    })?;
    mac.update(message.as_bytes());
    Ok(STANDARD.encode(mac.finalize().into_bytes()))
}

fn short_nonce() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(12)
        .collect()
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(environment: MoonpayEnvironment) -> MoonpayConfig {
        MoonpayConfig {
            publishable_key: match environment {
                MoonpayEnvironment::Sandbox => "pk_test_DocsVector00".to_string(),
                MoonpayEnvironment::Live => "pk_live_example".to_string(),
            },
            secret_key: match environment {
                MoonpayEnvironment::Sandbox => "sk_test_DocsVector00".to_string(),
                MoonpayEnvironment::Live => "sk_live_example".to_string(),
            },
            environment,
            allowed_origins: vec!["https://agentbounties.app".to_string()],
            client_ip_header: DEFAULT_CLIENT_IP_HEADER.to_string(),
            min_fiat_minor: 100,
            max_fiat_minor: 1_000_000,
            rate_limit_per_minute: 10,
        }
    }

    fn request(asset: &str) -> ValidatedCheckoutRequest {
        ValidatedCheckoutRequest {
            wallet_address: "0xde0b295669a9fd93d5f28d9ec85e40f4cb697bae".to_string(),
            base_currency_amount: "25.00".to_string(),
            base_currency_code: "usd".to_string(),
            amount_minor: 2_500,
            asset: OnrampAsset::parse(Some(asset)).unwrap(),
            return_url: "https://agentbounties.app/onramp.html?bountyContract=0x1111111111111111111111111111111111111111".to_string(),
            intent_id: Some(Uuid::parse_str("9e5c6d19-ae7a-4b4c-a49f-36f322fd4532").unwrap()),
            bounty_contract: Some("0x1111111111111111111111111111111111111111".to_string()),
        }
    }

    #[test]
    fn url_signing_matches_moonpay_documentation_vector() {
        let query = "?apiKey=pk_test_DocsVector00&currencyCode=eth&walletAddress=0xde0B295669a9FD93d5F28D9Ec85E40f4cb697BAe";
        assert_eq!(
            hmac_base64("sk_test_DocsVector00", query).unwrap(),
            "oIJxSghyzll/BLhUFdQZhkxf7DAS8REFaWr/ibO+K8Q="
        );
    }

    #[test]
    fn live_checkout_is_ip_bound_signed_last_and_never_claims_bounty_funding() {
        let plan = build_checkout_plan(
            &config(MoonpayEnvironment::Live),
            request("usdc"),
            Some("8.8.8.8"),
        )
        .unwrap();
        let url = Url::parse(&plan.checkout_url).unwrap();
        let query = url.query().unwrap();
        assert_eq!(url.host_str(), Some("buy.moonpay.com"));
        assert!(query.contains("currencyCode=usdc_base"));
        assert!(query.contains("walletAddress=0xde0b295669a9fd93d5f28d9ec85e40f4cb697bae"));
        assert!(query.contains("allowedIpAddress="));
        assert!(query.split('&').last().unwrap().starts_with("signature="));
        assert!(!plan.checkout_url.contains("sk_live_example"));
        assert!(!plan.protocol_action_completed);
        assert!(plan.canonical_event.is_none());
        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
        assert_eq!(plan.state, "checkout_ready_wallet_not_yet_topped_up");
    }

    #[test]
    fn wallet_onboarding_checkout_does_not_require_an_existing_bounty() {
        let mut onboarding = request("usdc");
        onboarding.bounty_contract = None;
        onboarding.intent_id = None;
        let plan = build_checkout_plan(&config(MoonpayEnvironment::Sandbox), onboarding, None)
            .unwrap();
        assert!(plan.bounty_contract.is_none());
        assert!(!plan.protocol_action_completed);
        assert!(plan.canonical_event.is_none());
        assert!(!plan.bounty_funded);
        assert!(plan.canonical_funding_event.is_none());
    }

    #[test]
    fn sandbox_checkout_is_test_only_and_supports_gas_asset() {
        let plan = build_checkout_plan(&config(MoonpayEnvironment::Sandbox), request("eth"), None)
            .unwrap();
        let url = Url::parse(&plan.checkout_url).unwrap();
        assert_eq!(url.host_str(), Some("buy-sandbox.moonpay.com"));
        assert_eq!(plan.asset, "ETH");
        assert_eq!(plan.destination_network, "moonpay-test-mode");
        assert!(plan.sandbox_notice.is_some());
        assert!(!url.query().unwrap().contains("allowedIpAddress="));
    }

    #[test]
    fn validation_rejects_wrong_origin_return_path_and_unknown_asset() {
        assert!(validate_return_url(
            "https://evil.example/onramp.html",
            "https://agentbounties.app"
        )
        .is_err());
        assert!(validate_return_url(
            "https://agentbounties.app/earn.html",
            "https://agentbounties.app"
        )
        .is_err());
        assert!(OnrampAsset::parse(Some("btc")).is_err());
    }

    #[test]
    fn amount_parser_is_exact_and_bounded_to_two_decimals() {
        assert_eq!(
            parse_fiat_amount("20").unwrap(),
            (2_000, "20.00".to_string())
        );
        assert_eq!(
            parse_fiat_amount("20.5").unwrap(),
            (2_050, "20.50".to_string())
        );
        assert_eq!(
            parse_fiat_amount("20.05").unwrap(),
            (2_005, "20.05".to_string())
        );
        assert!(parse_fiat_amount("0").is_err());
        assert!(parse_fiat_amount("20.005").is_err());
        assert!(parse_fiat_amount("1e3").is_err());
    }

    #[test]
    fn live_client_ip_must_be_public() {
        assert!(is_public_ip("8.8.8.8").unwrap());
        assert!(!is_public_ip("127.0.0.1").unwrap());
        assert!(!is_public_ip("10.0.0.1").unwrap());
        assert!(!is_public_ip("100.64.0.1").unwrap());
    }
}
