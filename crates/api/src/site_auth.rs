use alloy::primitives::Signature;
use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use db::{DbError, PostgresStore, SiteAuthWallet};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env,
    str::FromStr,
    sync::{Arc, Mutex},
};
use tower_http::cors::CorsLayer;
use url::Url;
use uuid::Uuid;

const SESSION_COOKIE: &str = "agent_bounties_session";
const OAUTH_STATE_COOKIE: &str = "agent_bounties_oauth_state";
const SESSION_MAX_AGE_SECONDS: i64 = 8 * 60 * 60;
const OAUTH_STATE_MAX_AGE_SECONDS: i64 = 10 * 60;
const WALLET_CHALLENGE_MAX_AGE_SECONDS: i64 = 5 * 60;
const BASE_CHAIN_ID: i64 = 8453;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct SiteAuthService {
    inner: Arc<SiteAuthInner>,
}

struct SiteAuthInner {
    session_secret: Option<Vec<u8>>,
    wallet_secret: Option<Vec<u8>>,
    web_origin: String,
    api_origin: String,
    allowed_origins: Vec<HeaderValue>,
    providers: BTreeMap<String, OAuthProvider>,
    store: Option<PostgresStore>,
    client: reqwest::Client,
    wallet_challenges: Mutex<HashMap<String, WalletChallenge>>,
}

#[derive(Clone)]
struct OAuthProvider {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    authorize_url: String,
    token_url: String,
    user_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SessionUser {
    provider: String,
    sub: String,
    name: String,
    email: String,
    avatar: String,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Clone)]
struct WalletChallenge {
    account_id: String,
    address: String,
    message: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct OAuthCallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WalletAddressRequest {
    address: String,
}

#[derive(Debug, Deserialize)]
struct WalletVerifyRequest {
    challenge_id: String,
    address: String,
    signature: String,
}

#[derive(Debug, Clone, Serialize)]
struct BrowserWallet {
    address: String,
    label: String,
    chain_id: i64,
    linked_at: DateTime<Utc>,
    proof: String,
}

#[derive(Debug, Clone, Serialize)]
struct AccountActivity {
    title: String,
    status: String,
}

#[derive(Debug, Clone)]
struct TimedAccountActivity {
    title: String,
    status: String,
    occurred_at: String,
}

impl SiteAuthService {
    pub fn from_env(store: Option<PostgresStore>) -> anyhow::Result<Self> {
        let session_secret = env::var("AUTH_SESSION_SECRET")
            .ok()
            .filter(|value| !value.trim().is_empty());
        if session_secret
            .as_ref()
            .is_some_and(|value| value.len() < 32)
        {
            anyhow::bail!("AUTH_SESSION_SECRET must be at least 32 characters");
        }
        let wallet_secret = env::var("AUTH_WALLET_LINK_SECRET")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| session_secret.clone());
        if wallet_secret.as_ref().is_some_and(|value| value.len() < 32) {
            anyhow::bail!("AUTH_WALLET_LINK_SECRET must be at least 32 characters");
        }

        let web_origin = env::var("WEBSITE_BASE_URL")
            .unwrap_or_else(|_| "https://agentbounties.app".to_string())
            .trim_end_matches('/')
            .to_string();
        let api_origin = env::var("PUBLIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.agentbounties.app".to_string())
            .trim_end_matches('/')
            .to_string();
        validate_origin(&web_origin, "WEBSITE_BASE_URL")?;
        validate_origin(&api_origin, "PUBLIC_BASE_URL")?;

        let configured_origins =
            env::var("SITE_AUTH_ALLOWED_ORIGINS").unwrap_or_else(|_| web_origin.clone());
        let mut allowed_origins = Vec::new();
        for value in configured_origins
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            validate_origin(value, "SITE_AUTH_ALLOWED_ORIGINS")?;
            allowed_origins.push(HeaderValue::from_str(value)?);
        }
        if allowed_origins.is_empty() {
            anyhow::bail!("SITE_AUTH_ALLOWED_ORIGINS must contain at least one origin");
        }

        let providers = provider_configs(&api_origin)?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("AgentBounties-SiteAuth/1.0")
            .build()?;
        Ok(Self {
            inner: Arc::new(SiteAuthInner {
                session_secret: session_secret.map(String::into_bytes),
                wallet_secret: wallet_secret.map(String::into_bytes),
                web_origin,
                api_origin,
                allowed_origins,
                providers,
                store,
                client,
                wallet_challenges: Mutex::new(HashMap::new()),
            }),
        })
    }

    fn enabled(&self) -> bool {
        self.inner.session_secret.is_some() && self.inner.store.is_some()
    }

    fn configured_providers(&self) -> BTreeMap<String, bool> {
        ["google", "microsoft", "github", "amazon"]
            .into_iter()
            .map(|provider| {
                (
                    provider.to_string(),
                    self.enabled() && self.inner.providers.contains_key(provider),
                )
            })
            .chain(std::iter::once(("enterprise".to_string(), false)))
            .collect()
    }

    fn account_id(&self, user: &SessionUser) -> Option<String> {
        let secret = self.inner.wallet_secret.as_deref()?;
        Some(hmac_hex(
            secret,
            format!("{}\0{}", user.provider, user.sub).as_bytes(),
        ))
    }

    fn current_user(&self, headers: &HeaderMap) -> Option<SessionUser> {
        let token = cookie_value(headers, SESSION_COOKIE)?;
        verify_session(&token, self.inner.session_secret.as_deref()?, Utc::now())
    }

    fn origin_is_allowed(&self, headers: &HeaderMap) -> bool {
        let Some(origin) = headers.get(header::ORIGIN) else {
            return false;
        };
        self.inner
            .allowed_origins
            .iter()
            .any(|allowed| allowed == origin)
    }
}

pub fn router(service: SiteAuthService) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(service.inner.allowed_origins.clone())
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::ACCEPT, header::CONTENT_TYPE])
        .allow_credentials(true);
    Router::new()
        .route("/v1/site-auth/healthz", get(healthz))
        .route("/v1/site-auth/session", get(session))
        .route("/v1/site-auth/account", get(account))
        .route("/v1/site-auth/login/:provider", get(begin_oauth))
        .route("/v1/site-auth/callback/:provider", get(finish_oauth))
        .route("/v1/site-auth/logout", post(logout))
        .route("/v1/site-auth/wallet/challenge", post(begin_wallet_link))
        .route("/v1/site-auth/wallet/verify", post(finish_wallet_link))
        .route("/v1/site-auth/wallet/unlink", post(unlink_wallet))
        .layer(cors)
        .layer(Extension(service))
}

async fn healthz(Extension(service): Extension<SiteAuthService>) -> Response {
    no_store_json(
        StatusCode::OK,
        json!({
            "ok": service.enabled(),
            "providers": service.configured_providers(),
            "storage": if service.inner.store.is_some() { "postgres" } else { "unavailable" },
        }),
    )
}

async fn session(Extension(service): Extension<SiteAuthService>, headers: HeaderMap) -> Response {
    let user = service.current_user(&headers);
    no_store_json(
        StatusCode::OK,
        json!({
            "authenticated": user.is_some(),
            "user": user,
            "providers": service.configured_providers(),
        }),
    )
}

async fn account(Extension(service): Extension<SiteAuthService>, headers: HeaderMap) -> Response {
    let Some(user) = service.current_user(&headers) else {
        return error_json(StatusCode::UNAUTHORIZED, "authentication_required");
    };
    let Some(account_id) = service.account_id(&user) else {
        return no_store_json(
            StatusCode::OK,
            unavailable_account_dashboard("account_service_unavailable", Vec::new()),
        );
    };
    let Some(store) = service.inner.store.as_ref() else {
        return no_store_json(
            StatusCode::OK,
            unavailable_account_dashboard("wallet_link_store_unavailable", Vec::new()),
        );
    };
    let wallets = match store.list_site_auth_wallets(&account_id).await {
        Ok(wallets) => wallets.into_iter().map(browser_wallet).collect::<Vec<_>>(),
        Err(_) => {
            return no_store_json(
                StatusCode::OK,
                unavailable_account_dashboard("wallet_link_store_unavailable", Vec::new()),
            )
        }
    };
    if wallets.is_empty() {
        return no_store_json(
            StatusCode::OK,
            unavailable_account_dashboard("marketplace_identity_unlinked", wallets),
        );
    }
    match load_account_evidence(&service)
        .await
        .and_then(|evidence| build_linked_account_dashboard(wallets.clone(), &evidence))
    {
        Ok(dashboard) => no_store_json(StatusCode::OK, dashboard),
        Err(_) => no_store_json(
            StatusCode::OK,
            unavailable_account_dashboard("marketplace_evidence_unavailable", wallets),
        ),
    }
}

async fn begin_oauth(
    Extension(service): Extension<SiteAuthService>,
    Path(provider): Path<String>,
) -> Response {
    let provider = provider.to_ascii_lowercase();
    let Some(config) = service.inner.providers.get(&provider) else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some(&format!("{provider}_not_configured")),
            None,
        );
    };
    if !service.enabled() {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("provider_not_configured"),
            None,
        );
    }
    let now = Utc::now().timestamp();
    let state_payload = format!("{provider}|{now}|{}", Uuid::new_v4());
    let Some(secret) = service.inner.session_secret.as_deref() else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("provider_not_configured"),
            None,
        );
    };
    let state = sign_bytes(state_payload.as_bytes(), secret);
    let authorization = match authorization_url(&provider, config, &state) {
        Ok(url) => url,
        Err(_) => {
            return auth_redirect(
                &service,
                "error",
                None,
                Some("provider_not_configured"),
                None,
            )
        }
    };
    let state_cookie = format!(
        "{OAUTH_STATE_COOKIE}={state}; Path=/v1/site-auth/callback; HttpOnly; Secure; SameSite=Lax; Max-Age={OAUTH_STATE_MAX_AGE_SECONDS}"
    );
    redirect_response(&authorization, Some(&state_cookie))
}

async fn finish_oauth(
    Extension(service): Extension<SiteAuthService>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
    headers: HeaderMap,
) -> Response {
    let provider = provider.to_ascii_lowercase();
    if query.error.is_some() {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("access_denied"),
            Some(clear_state_cookie()),
        );
    }
    let state = query.state.unwrap_or_default();
    let code = query.code.unwrap_or_default();
    let state_cookie = cookie_value(&headers, OAUTH_STATE_COOKIE).unwrap_or_default();
    let valid_state = service
        .inner
        .session_secret
        .as_deref()
        .and_then(|secret| verify_signed_bytes(&state, secret))
        .and_then(|payload| String::from_utf8(payload).ok())
        .and_then(|payload| parse_oauth_state(&payload, &provider, Utc::now()))
        .is_some();
    if state.is_empty() || state != state_cookie || code.is_empty() || !valid_state {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("invalid_state"),
            Some(clear_state_cookie()),
        );
    }
    let Some(config) = service.inner.providers.get(&provider) else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("provider_not_configured"),
            Some(clear_state_cookie()),
        );
    };
    let profile = match exchange_code(&service, &provider, config, &code).await {
        Ok(profile) => profile,
        Err(_) => {
            return auth_redirect(
                &service,
                "error",
                None,
                Some("provider_exchange_failed"),
                Some(clear_state_cookie()),
            )
        }
    };
    let Some(account_id) = service.account_id(&profile) else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("account_service_unavailable"),
            Some(clear_state_cookie()),
        );
    };
    let Some(store) = service.inner.store.as_ref() else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("account_service_unavailable"),
            Some(clear_state_cookie()),
        );
    };
    if store
        .upsert_site_auth_account(
            &account_id,
            &profile.provider,
            &profile.sub,
            &profile.name,
            &profile.email,
            &profile.avatar,
        )
        .await
        .is_err()
    {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("account_service_unavailable"),
            Some(clear_state_cookie()),
        );
    }
    let Some(secret) = service.inner.session_secret.as_deref() else {
        return auth_redirect(
            &service,
            "error",
            None,
            Some("account_service_unavailable"),
            Some(clear_state_cookie()),
        );
    };
    let token = sign_session(profile, secret, Utc::now());
    let session_cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/v1/site-auth; HttpOnly; Secure; SameSite=Lax; Max-Age={SESSION_MAX_AGE_SECONDS}"
    );
    let location = auth_result_url(&service.inner.web_origin, "success", Some(&provider), None);
    let mut response = redirect_response(&location, Some(&session_cookie));
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&clear_state_cookie()).expect("valid cookie"),
    );
    response
}

async fn logout(Extension(service): Extension<SiteAuthService>, headers: HeaderMap) -> Response {
    if !service.origin_is_allowed(&headers) {
        return error_json(StatusCode::FORBIDDEN, "invalid_origin");
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(
            "agent_bounties_session=; Path=/v1/site-auth; HttpOnly; Secure; SameSite=Lax; Max-Age=0",
        ),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn begin_wallet_link(
    Extension(service): Extension<SiteAuthService>,
    headers: HeaderMap,
    Json(request): Json<WalletAddressRequest>,
) -> Response {
    let Some((user, origin)) = wallet_request_context(&service, &headers) else {
        return wallet_context_error(&service, &headers);
    };
    let address = match normalize_wallet_address(&request.address) {
        Ok(address) => address,
        Err(error) => return error_json(StatusCode::BAD_REQUEST, error),
    };
    let Some(account_id) = service.account_id(&user) else {
        return error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "account_service_unavailable",
        );
    };
    let now = Utc::now();
    let expires_at = now + ChronoDuration::seconds(WALLET_CHALLENGE_MAX_AGE_SECONDS);
    let challenge_id = Uuid::new_v4().simple().to_string();
    let nonce = Uuid::new_v4().simple().to_string();
    let message = wallet_link_message(&origin, &account_id, &address, &nonce, now, expires_at);
    let mut challenges = service
        .inner
        .wallet_challenges
        .lock()
        .expect("wallet challenge lock");
    challenges.retain(|_, challenge| challenge.expires_at > now);
    challenges.insert(
        challenge_id.clone(),
        WalletChallenge {
            account_id,
            address: address.clone(),
            message: message.clone(),
            expires_at,
        },
    );
    no_store_json(
        StatusCode::CREATED,
        json!({
            "challenge_id": challenge_id,
            "address": address,
            "chain_id": BASE_CHAIN_ID,
            "message": message,
            "expires_at": expires_at,
            "intent": "Prove wallet ownership only; no transaction, approval, or payment.",
        }),
    )
}

async fn finish_wallet_link(
    Extension(service): Extension<SiteAuthService>,
    headers: HeaderMap,
    Json(request): Json<WalletVerifyRequest>,
) -> Response {
    let Some((user, _origin)) = wallet_request_context(&service, &headers) else {
        return wallet_context_error(&service, &headers);
    };
    let address = match normalize_wallet_address(&request.address) {
        Ok(address) => address,
        Err(error) => return error_json(StatusCode::BAD_REQUEST, error),
    };
    let Some(account_id) = service.account_id(&user) else {
        return error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "account_service_unavailable",
        );
    };
    let challenge = service
        .inner
        .wallet_challenges
        .lock()
        .expect("wallet challenge lock")
        .remove(&request.challenge_id);
    let Some(challenge) = challenge else {
        return error_json(StatusCode::BAD_REQUEST, "wallet_challenge_invalid");
    };
    if challenge.expires_at <= Utc::now()
        || challenge.account_id != account_id
        || challenge.address != address
    {
        return error_json(StatusCode::BAD_REQUEST, "wallet_challenge_invalid");
    }
    if !verify_wallet_signature(&challenge.message, &request.signature, &address) {
        return error_json(StatusCode::BAD_REQUEST, "wallet_signature_invalid");
    }
    let Some(store) = service.inner.store.as_ref() else {
        return error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet_link_store_unavailable",
        );
    };
    match store
        .link_site_auth_wallet(&account_id, &address, BASE_CHAIN_ID)
        .await
    {
        Ok(wallets) => no_store_json(
            StatusCode::OK,
            json!({
                "linked": true,
                "wallets": wallets.into_iter().map(browser_wallet).collect::<Vec<_>>(),
            }),
        ),
        Err(DbError::SiteAuthConflict(reason)) => error_json(StatusCode::CONFLICT, &reason),
        Err(_) => error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet_link_store_unavailable",
        ),
    }
}

async fn unlink_wallet(
    Extension(service): Extension<SiteAuthService>,
    headers: HeaderMap,
    Json(request): Json<WalletAddressRequest>,
) -> Response {
    let Some((user, _origin)) = wallet_request_context(&service, &headers) else {
        return wallet_context_error(&service, &headers);
    };
    let address = match normalize_wallet_address(&request.address) {
        Ok(address) => address,
        Err(error) => return error_json(StatusCode::BAD_REQUEST, error),
    };
    let Some(account_id) = service.account_id(&user) else {
        return error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "account_service_unavailable",
        );
    };
    let Some(store) = service.inner.store.as_ref() else {
        return error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet_link_store_unavailable",
        );
    };
    match store.unlink_site_auth_wallet(&account_id, &address).await {
        Ok(wallets) => no_store_json(
            StatusCode::OK,
            json!({
                "unlinked": true,
                "wallets": wallets.into_iter().map(browser_wallet).collect::<Vec<_>>(),
            }),
        ),
        Err(_) => error_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "wallet_link_store_unavailable",
        ),
    }
}

fn wallet_request_context(
    service: &SiteAuthService,
    headers: &HeaderMap,
) -> Option<(SessionUser, String)> {
    if !service.origin_is_allowed(headers) {
        return None;
    }
    let user = service.current_user(headers)?;
    let origin = headers.get(header::ORIGIN)?.to_str().ok()?.to_string();
    Some((user, origin))
}

fn wallet_context_error(service: &SiteAuthService, headers: &HeaderMap) -> Response {
    if !service.origin_is_allowed(headers) {
        error_json(StatusCode::FORBIDDEN, "invalid_origin")
    } else {
        error_json(StatusCode::UNAUTHORIZED, "authentication_required")
    }
}

fn provider_configs(api_origin: &str) -> anyhow::Result<BTreeMap<String, OAuthProvider>> {
    let mut providers = BTreeMap::new();
    let definitions = [
        (
            "google",
            "GOOGLE_OAUTH",
            "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            "https://oauth2.googleapis.com/token".to_string(),
            "https://openidconnect.googleapis.com/v1/userinfo".to_string(),
        ),
        (
            "github",
            "GITHUB_OAUTH",
            "https://github.com/login/oauth/authorize".to_string(),
            "https://github.com/login/oauth/access_token".to_string(),
            "https://api.github.com/user".to_string(),
        ),
        (
            "amazon",
            "AMAZON_OAUTH",
            "https://www.amazon.com/ap/oa".to_string(),
            "https://api.amazon.com/auth/o2/token".to_string(),
            "https://api.amazon.com/user/profile".to_string(),
        ),
    ];
    for (provider, prefix, authorize_url, token_url, user_url) in definitions {
        if let Some(config) = read_provider_config(
            provider,
            prefix,
            api_origin,
            authorize_url,
            token_url,
            user_url,
        )? {
            providers.insert(provider.to_string(), config);
        }
    }
    let tenant = env::var("MICROSOFT_OAUTH_TENANT").unwrap_or_else(|_| "common".to_string());
    if !tenant
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        anyhow::bail!("MICROSOFT_OAUTH_TENANT is invalid");
    }
    let authority = format!("https://login.microsoftonline.com/{tenant}");
    if let Some(config) = read_provider_config(
        "microsoft",
        "MICROSOFT_OAUTH",
        api_origin,
        format!("{authority}/oauth2/v2.0/authorize"),
        format!("{authority}/oauth2/v2.0/token"),
        "https://graph.microsoft.com/oidc/userinfo".to_string(),
    )? {
        providers.insert("microsoft".to_string(), config);
    }
    Ok(providers)
}

fn read_provider_config(
    provider: &str,
    prefix: &str,
    api_origin: &str,
    authorize_url: String,
    token_url: String,
    user_url: String,
) -> anyhow::Result<Option<OAuthProvider>> {
    let client_id = env::var(format!("{prefix}_CLIENT_ID"))
        .ok()
        .filter(|value| !value.trim().is_empty());
    let client_secret = env::var(format!("{prefix}_CLIENT_SECRET"))
        .ok()
        .filter(|value| !value.trim().is_empty());
    if client_id.is_some() != client_secret.is_some() {
        anyhow::bail!("{provider} OAuth requires its client id and secret together");
    }
    let (Some(client_id), Some(client_secret)) = (client_id, client_secret) else {
        return Ok(None);
    };
    let redirect_uri = env::var(format!("{prefix}_REDIRECT_URI"))
        .unwrap_or_else(|_| format!("{api_origin}/v1/site-auth/callback/{provider}"));
    validate_origin_url(&redirect_uri, &format!("{prefix}_REDIRECT_URI"))?;
    Ok(Some(OAuthProvider {
        client_id,
        client_secret,
        redirect_uri,
        authorize_url,
        token_url,
        user_url,
    }))
}

fn authorization_url(
    provider: &str,
    config: &OAuthProvider,
    state: &str,
) -> anyhow::Result<String> {
    let mut url = Url::parse(&config.authorize_url)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &config.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("state", state);
        match provider {
            "google" => {
                query
                    .append_pair("scope", "openid email profile")
                    .append_pair("include_granted_scopes", "true")
                    .append_pair("prompt", "select_account");
            }
            "github" => {
                query
                    .append_pair("scope", "read:user user:email")
                    .append_pair("allow_signup", "true");
            }
            "microsoft" => {
                query
                    .append_pair("scope", "openid profile email")
                    .append_pair("response_mode", "query")
                    .append_pair("prompt", "select_account");
            }
            "amazon" => {
                query.append_pair("scope", "profile");
            }
            _ => anyhow::bail!("unsupported provider"),
        }
    }
    Ok(url.into())
}

async fn exchange_code(
    service: &SiteAuthService,
    provider: &str,
    config: &OAuthProvider,
    code: &str,
) -> anyhow::Result<SessionUser> {
    let token: Value = service
        .inner
        .client
        .post(&config.token_url)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", config.client_id.as_str()),
            ("client_secret", config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", config.redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let access_token = token
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("provider access token missing"))?;
    let profile: Value = service
        .inner
        .client
        .get(&config.user_url)
        .bearer_auth(access_token)
        .header(header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    provider_profile(service, provider, access_token, profile).await
}

async fn provider_profile(
    service: &SiteAuthService,
    provider: &str,
    access_token: &str,
    profile: Value,
) -> anyhow::Result<SessionUser> {
    let (sub, name, email, avatar) = match provider {
        "google" => {
            let sub = required_string(&profile, "sub")?;
            let email = value_string(&profile, "email");
            if !email.is_empty()
                && profile.get("email_verified").and_then(Value::as_bool) != Some(true)
            {
                anyhow::bail!("Google email is not verified");
            }
            (
                sub,
                first_non_empty(
                    &[value_string(&profile, "name"), email.clone()],
                    "Google user",
                ),
                email,
                value_string(&profile, "picture"),
            )
        }
        "github" => {
            let sub = profile
                .get("id")
                .and_then(|value| {
                    value
                        .as_u64()
                        .map(|id| id.to_string())
                        .or_else(|| value.as_str().map(str::to_string))
                })
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow::anyhow!("GitHub id missing"))?;
            let mut email = value_string(&profile, "email");
            if email.is_empty() {
                let emails: Value = service
                    .inner
                    .client
                    .get("https://api.github.com/user/emails")
                    .bearer_auth(access_token)
                    .header(header::ACCEPT, "application/json")
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await?;
                if let Some(entries) = emails.as_array() {
                    email = entries
                        .iter()
                        .find(|entry| {
                            entry.get("verified").and_then(Value::as_bool) == Some(true)
                                && entry.get("primary").and_then(Value::as_bool) == Some(true)
                        })
                        .or_else(|| {
                            entries.iter().find(|entry| {
                                entry.get("verified").and_then(Value::as_bool) == Some(true)
                            })
                        })
                        .map(|entry| value_string(entry, "email"))
                        .unwrap_or_default();
                }
            }
            (
                sub,
                first_non_empty(
                    &[
                        value_string(&profile, "name"),
                        value_string(&profile, "login"),
                    ],
                    "GitHub user",
                ),
                email,
                value_string(&profile, "avatar_url"),
            )
        }
        "microsoft" => {
            let sub = required_string(&profile, "sub")?;
            let email = first_non_empty(
                &[
                    value_string(&profile, "email"),
                    value_string(&profile, "preferred_username"),
                ],
                "",
            );
            (
                sub,
                first_non_empty(
                    &[value_string(&profile, "name"), email.clone()],
                    "Microsoft user",
                ),
                email,
                String::new(),
            )
        }
        "amazon" => {
            let sub = required_string(&profile, "user_id")?;
            let email = value_string(&profile, "email");
            (
                sub,
                first_non_empty(
                    &[value_string(&profile, "name"), email.clone()],
                    "Amazon user",
                ),
                email,
                String::new(),
            )
        }
        _ => anyhow::bail!("unsupported provider"),
    };
    let now = Utc::now().timestamp();
    Ok(SessionUser {
        provider: provider.to_string(),
        sub: truncate(&sub, 512),
        name: truncate(&name, 160),
        email: truncate(&email, 320),
        avatar: truncate(&avatar, 2048),
        iat: now,
        exp: now + SESSION_MAX_AGE_SECONDS,
    })
}

async fn load_account_evidence(service: &SiteAuthService) -> anyhow::Result<Value> {
    let base = service.inner.api_origin.trim_end_matches('/');
    let urls = [
        (
            "autonomous",
            format!("{base}/v1/base/autonomous-bounties/events?network=base-mainnet"),
        ),
        (
            "competition_v1",
            format!("{base}/v1/base/open-competition-v1/events?network=base-mainnet"),
        ),
        (
            "competition_v2",
            format!("{base}/v1/base/open-competition-v2-beta3/events?network=base-mainnet"),
        ),
        (
            "leaderboard",
            format!("{base}/v1/base/autonomous-bounties/leaderboard?network=base-mainnet"),
        ),
    ];
    let mut evidence = serde_json::Map::new();
    for (key, url) in urls {
        let payload: Value = service
            .inner
            .client
            .get(url)
            .header(header::ACCEPT, "application/json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        evidence.insert(key.to_string(), payload);
    }
    Ok(Value::Object(evidence))
}

fn build_linked_account_dashboard(
    wallets: Vec<BrowserWallet>,
    evidence: &Value,
) -> anyhow::Result<Value> {
    let addresses = wallets
        .iter()
        .map(|wallet| wallet.address.clone())
        .collect::<BTreeSet<_>>();
    let autonomous = event_list(evidence.get("autonomous"))?;
    let competition_v1 = event_list(evidence.get("competition_v1"))?;
    let competition_v2 = event_list(evidence.get("competition_v2"))?;
    let leaderboard = evidence
        .get("leaderboard")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("leaderboard evidence invalid"))?;

    let streams = [
        ("autonomous", autonomous, "bounty_settled"),
        ("competition_v1", competition_v1, "bounty_settled"),
        ("competition_v2", competition_v2, "competition_settled"),
    ];
    let mut earned = 0_u64;
    let mut spent = 0_u64;
    for (_, events, settled_kind) in streams {
        for event in events {
            let data = event.get("data").and_then(Value::as_object);
            if event_kind(event) == "funding_added"
                && address_matches(data, "contributor", &addresses)
            {
                spent = spent
                    .checked_add(required_u64(data, "amount")?)
                    .ok_or_else(|| anyhow::anyhow!("spending overflow"))?;
            }
            if event_kind(event) == settled_kind && address_matches(data, "solver", &addresses) {
                let reward = required_u64(data, "solver_reward")?;
                let bonus = optional_u64(data, "timeout_bond_bonus")?;
                earned = earned
                    .checked_add(reward)
                    .and_then(|value| value.checked_add(bonus))
                    .ok_or_else(|| anyhow::anyhow!("earnings overflow"))?;
            }
        }
    }

    let mut terminal_rounds = BTreeSet::new();
    for event in autonomous {
        if !matches!(
            event_kind(event),
            "bounty_settled" | "claim_expired" | "submission_expired" | "submission_rejected"
        ) {
            continue;
        }
        if let Some(round) = event
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("round"))
            .and_then(Value::as_u64)
        {
            terminal_rounds.insert((bounty_id(event), round));
        }
    }
    let mut participating: BTreeMap<(String, String), TimedAccountActivity> = BTreeMap::new();
    for event in autonomous {
        let data = event.get("data").and_then(Value::as_object);
        let round = data
            .and_then(|value| value.get("round"))
            .and_then(Value::as_u64);
        let id = bounty_id(event);
        if event_kind(event) == "bounty_claimed"
            && address_matches(data, "solver", &addresses)
            && round.is_some_and(|value| !terminal_rounds.contains(&(id.clone(), value)))
        {
            participating.insert(
                ("autonomous".to_string(), id.clone()),
                timed_activity(event, "Claim active"),
            );
        }
    }
    for (source, events, entry_kind, settled_kind, status) in [
        (
            "competition_v1",
            competition_v1,
            "solution_committed",
            "bounty_settled",
            "Entry committed",
        ),
        (
            "competition_v2",
            competition_v2,
            "entry_qualified",
            "competition_settled",
            "Qualified entry",
        ),
    ] {
        let settled = events
            .iter()
            .filter(|event| event_kind(event) == settled_kind)
            .map(bounty_id)
            .collect::<BTreeSet<_>>();
        for event in events {
            let id = bounty_id(event);
            if event_kind(event) == entry_kind
                && address_matches(
                    event.get("data").and_then(Value::as_object),
                    "solver",
                    &addresses,
                )
                && !settled.contains(&id)
            {
                participating.insert((source.to_string(), id), timed_activity(event, status));
            }
        }
    }

    let mut completed_posts: BTreeMap<(String, String), TimedAccountActivity> = BTreeMap::new();
    for (source, events, settled_kind) in streams {
        let created = events
            .iter()
            .filter(|event| {
                matches!(
                    event_kind(event),
                    "canonical_bounty_created" | "canonical_competition_created"
                )
            })
            .filter(|event| {
                address_matches(
                    event.get("data").and_then(Value::as_object),
                    "creator",
                    &addresses,
                )
            })
            .map(bounty_id)
            .collect::<BTreeSet<_>>();
        for event in events
            .iter()
            .filter(|event| event_kind(event) == settled_kind)
        {
            let id = bounty_id(event);
            if created.contains(&id) {
                let occurred_at = event
                    .get("occurred_at")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                completed_posts.insert(
                    (source.to_string(), id),
                    TimedAccountActivity {
                        title: bounty_label(event.get("bounty_id")),
                        status: if occurred_at.is_empty() {
                            "Settled".to_string()
                        } else {
                            format!("Settled {}", &occurred_at[..occurred_at.len().min(10)])
                        },
                        occurred_at: occurred_at.to_string(),
                    },
                );
            }
        }
    }

    let entries = leaderboard
        .get("weekly")
        .and_then(Value::as_object)
        .and_then(|weekly| weekly.get("ranking"))
        .and_then(Value::as_object)
        .and_then(|ranking| ranking.get("entries"))
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("leaderboard evidence invalid"))?;
    let rank = entries
        .iter()
        .filter(|entry| {
            entry
                .get("solver_wallet")
                .and_then(Value::as_str)
                .is_some_and(|wallet| addresses.contains(&wallet.to_ascii_lowercase()))
        })
        .filter_map(|entry| entry.get("rank").and_then(Value::as_u64))
        .filter(|rank| *rank > 0)
        .min();

    let participating_count = participating.len();
    let completed_count = completed_posts.len();
    let participating_items = sorted_activities(participating.into_values());
    let completed_items = sorted_activities(completed_posts.into_values());
    Ok(json!({
        "schema_version": "agent-bounties/account-dashboard-v1",
        "data_status": "available",
        "reason": null,
        "identity_link_status": "verified",
        "wallets": wallets,
        "stats": {
            "participating_bounties": participating_count,
            "completed_posted_bounties": completed_count,
            "earned_usdc": usdc_from_base_units(earned),
            "spent_usdc": usdc_from_base_units(spent),
            "leaderboard_rank": rank,
        },
        "activities": {
            "participating": participating_items,
            "completed_posts": completed_items,
        },
        "evidence_boundary": "Wallet ownership was verified with a one-time EIP-191 signature. Earnings count canonical solver rewards and timeout bonuses; spending counts gross canonical FundingAdded contributions. Rank is the best current weekly rank among linked wallets.",
    }))
}

fn unavailable_account_dashboard(reason: &str, wallets: Vec<BrowserWallet>) -> Value {
    json!({
        "schema_version": "agent-bounties/account-dashboard-v1",
        "data_status": "unavailable",
        "reason": reason,
        "identity_link_status": if wallets.is_empty() { "unlinked" } else { "verified" },
        "wallets": wallets,
        "stats": {
            "participating_bounties": null,
            "completed_posted_bounties": null,
            "earned_usdc": null,
            "spent_usdc": null,
            "leaderboard_rank": null,
        },
        "activities": { "participating": [], "completed_posts": [] },
        "evidence_boundary": "OAuth authentication alone does not prove ownership of a marketplace wallet. Personal values are shown only after address control is verified and every required canonical evidence source loads.",
    })
}

fn sign_session(mut user: SessionUser, secret: &[u8], now: DateTime<Utc>) -> String {
    user.iat = now.timestamp();
    user.exp = now.timestamp() + SESSION_MAX_AGE_SECONDS;
    let payload = serde_json::to_vec(&user).expect("session user serializes");
    sign_bytes(&payload, secret)
}

fn verify_session(token: &str, secret: &[u8], now: DateTime<Utc>) -> Option<SessionUser> {
    let payload = verify_signed_bytes(token, secret)?;
    let user: SessionUser = serde_json::from_slice(&payload).ok()?;
    (user.exp > now.timestamp() && !user.provider.is_empty() && !user.sub.is_empty())
        .then_some(user)
}

fn sign_bytes(payload: &[u8], secret: &[u8]) -> String {
    let encoded = hex::encode(payload);
    format!("{encoded}.{}", hmac_hex(secret, encoded.as_bytes()))
}

fn verify_signed_bytes(token: &str, secret: &[u8]) -> Option<Vec<u8>> {
    let (payload, supplied) = token.split_once('.')?;
    let expected = hmac_hex(secret, payload.as_bytes());
    constant_time_eq(supplied.as_bytes(), expected.as_bytes())
        .then(|| hex::decode(payload).ok())
        .flatten()
}

fn hmac_hex(secret: &[u8], message: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn parse_oauth_state(payload: &str, provider: &str, now: DateTime<Utc>) -> Option<()> {
    let mut parts = payload.split('|');
    let state_provider = parts.next()?;
    let issued_at = parts.next()?.parse::<i64>().ok()?;
    let nonce = parts.next()?;
    if parts.next().is_some()
        || state_provider != provider
        || Uuid::parse_str(nonce).is_err()
        || issued_at > now.timestamp() + 30
        || now.timestamp() - issued_at > OAUTH_STATE_MAX_AGE_SECONDS
    {
        return None;
    }
    Some(())
}

fn normalize_wallet_address(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err("invalid_wallet_address");
    }
    Ok(value.to_ascii_lowercase())
}

fn verify_wallet_signature(message: &str, signature: &str, expected_address: &str) -> bool {
    Signature::from_str(signature)
        .ok()
        .and_then(|signature| signature.recover_address_from_msg(message.as_bytes()).ok())
        .map(|address| format!("{address:#x}") == expected_address)
        .unwrap_or(false)
}

fn wallet_link_message(
    origin: &str,
    account_id: &str,
    address: &str,
    nonce: &str,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> String {
    format!(
        "Agent Bounties wallet ownership verification\n\nSign this message to link the wallet to your signed-in Agent Bounties account.\nThis proves address control only. It does not authorize a transaction, token approval, or payment.\n\nOrigin: {origin}\nAccount: {account_id}\nWallet: {address}\nChain ID: {BASE_CHAIN_ID}\nNonce: {nonce}\nIssued At: {}\nExpiration Time: {}",
        issued_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn browser_wallet(wallet: SiteAuthWallet) -> BrowserWallet {
    BrowserWallet {
        label: short_wallet_address(&wallet.address),
        address: wallet.address,
        chain_id: wallet.chain_id,
        linked_at: wallet.linked_at,
        proof: wallet.proof,
    }
}

fn short_wallet_address(address: &str) -> String {
    if address.len() >= 12 {
        format!("{}…{}", &address[..8], &address[address.len() - 4..])
    } else {
        address.to_string()
    }
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|entry| entry.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.to_string()))
}

fn auth_redirect(
    service: &SiteAuthService,
    result: &str,
    provider: Option<&str>,
    reason: Option<&str>,
    cookie: Option<String>,
) -> Response {
    let location = auth_result_url(&service.inner.web_origin, result, provider, reason);
    redirect_response(&location, cookie.as_deref())
}

fn auth_result_url(
    origin: &str,
    result: &str,
    provider: Option<&str>,
    reason: Option<&str>,
) -> String {
    let mut url =
        Url::parse(&format!("{}/", origin.trim_end_matches('/'))).expect("validated origin");
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("auth", result);
        if let Some(provider) = provider {
            query.append_pair("provider", provider);
        }
        if let Some(reason) = reason {
            query.append_pair("reason", reason);
        }
    }
    url.into()
}

fn redirect_response(location: &str, cookie: Option<&str>) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(location).expect("validated redirect location"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(cookie) = cookie {
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_str(cookie).expect("valid cookie"),
        );
    }
    response
}

fn clear_state_cookie() -> String {
    "agent_bounties_oauth_state=; Path=/v1/site-auth/callback; HttpOnly; Secure; SameSite=Lax; Max-Age=0".to_string()
}

fn no_store_json(status: StatusCode, payload: Value) -> Response {
    let mut response = (status, Json(payload)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn error_json(status: StatusCode, error: &str) -> Response {
    no_store_json(status, json!({ "error": error }))
}

fn validate_origin(value: &str, name: &str) -> anyhow::Result<()> {
    let url = Url::parse(value)?;
    let local_http = url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        );
    if (url.scheme() != "https" && !local_http)
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        anyhow::bail!("{name} must be an HTTPS origin or an HTTP loopback origin");
    }
    Ok(())
}

fn validate_origin_url(value: &str, name: &str) -> anyhow::Result<()> {
    let url = Url::parse(value)?;
    let local_http = url.scheme() == "http"
        && matches!(
            url.host_str(),
            Some("127.0.0.1") | Some("localhost") | Some("::1")
        );
    if url.scheme() != "https" && !local_http {
        anyhow::bail!("{name} must use HTTPS or HTTP loopback");
    }
    Ok(())
}

fn required_string(value: &Value, key: &str) -> anyhow::Result<String> {
    let value = value_string(value, key);
    if value.is_empty() {
        anyhow::bail!("provider profile is missing {key}");
    }
    Ok(value)
}

fn value_string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn first_non_empty(values: &[String], fallback: &str) -> String {
    values
        .iter()
        .find(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn event_list(value: Option<&Value>) -> anyhow::Result<&Vec<Value>> {
    let value = value.ok_or_else(|| anyhow::anyhow!("canonical event stream missing"))?;
    value
        .as_array()
        .or_else(|| value.get("events").and_then(Value::as_array))
        .ok_or_else(|| anyhow::anyhow!("canonical event stream invalid"))
}

fn event_kind(event: &Value) -> &str {
    event
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

fn bounty_id(event: &Value) -> String {
    event
        .get("bounty_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn bounty_label(value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_str).unwrap_or_default();
    if value.starts_with("0x") && value.len() >= 14 {
        format!("Bounty {}…{}", &value[..8], &value[value.len() - 4..])
    } else {
        "Canonical bounty".to_string()
    }
}

fn timed_activity(event: &Value, status: &str) -> TimedAccountActivity {
    TimedAccountActivity {
        title: bounty_label(event.get("bounty_id")),
        status: status.to_string(),
        occurred_at: event
            .get("occurred_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

fn sorted_activities(
    activities: impl Iterator<Item = TimedAccountActivity>,
) -> Vec<AccountActivity> {
    let mut activities = activities.collect::<Vec<_>>();
    activities.sort_by(|left, right| right.occurred_at.cmp(&left.occurred_at));
    activities
        .into_iter()
        .take(6)
        .map(|activity| AccountActivity {
            title: activity.title,
            status: activity.status,
        })
        .collect()
}

fn address_matches(
    data: Option<&serde_json::Map<String, Value>>,
    key: &str,
    addresses: &BTreeSet<String>,
) -> bool {
    data.and_then(|data| data.get(key))
        .and_then(Value::as_str)
        .is_some_and(|address| addresses.contains(&address.to_ascii_lowercase()))
}

fn required_u64(data: Option<&serde_json::Map<String, Value>>, key: &str) -> anyhow::Result<u64> {
    data.and_then(|data| data.get(key))
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("canonical numeric evidence invalid"))
}

fn optional_u64(data: Option<&serde_json::Map<String, Value>>, key: &str) -> anyhow::Result<u64> {
    match data.and_then(|data| data.get(key)) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("canonical numeric evidence invalid")),
    }
}

fn usdc_from_base_units(value: u64) -> String {
    let whole = value / 1_000_000;
    let fractional = value % 1_000_000;
    if fractional == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{fractional:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{local::PrivateKeySigner, SignerSync};
    use chrono::TimeZone;

    fn test_user() -> SessionUser {
        SessionUser {
            provider: "google".to_string(),
            sub: "provider-subject".to_string(),
            name: "Test User".to_string(),
            email: "test@example.com".to_string(),
            avatar: String::new(),
            iat: 0,
            exp: 0,
        }
    }

    #[test]
    fn signed_session_round_trip_and_expiry_are_deterministic() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let token = sign_session(test_user(), secret, now);
        let decoded = verify_session(&token, secret, now).unwrap();
        assert_eq!(decoded.provider, "google");
        assert!(verify_session(
            &token,
            secret,
            now + ChronoDuration::seconds(SESSION_MAX_AGE_SECONDS)
        )
        .is_none());
        assert!(verify_session(&format!("{token}x"), secret, now).is_none());
    }

    #[test]
    fn oauth_state_is_signed_provider_bound_and_expires() {
        let secret = b"0123456789abcdef0123456789abcdef";
        let now = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let payload = format!("google|{}|{}", now.timestamp(), Uuid::new_v4());
        let state = sign_bytes(payload.as_bytes(), secret);
        let decoded = String::from_utf8(verify_signed_bytes(&state, secret).unwrap()).unwrap();
        assert_eq!(parse_oauth_state(&decoded, "google", now), Some(()));
        assert_eq!(parse_oauth_state(&decoded, "github", now), None);
        assert_eq!(
            parse_oauth_state(
                &decoded,
                "google",
                now + ChronoDuration::seconds(OAUTH_STATE_MAX_AGE_SECONDS + 1)
            ),
            None
        );
    }

    #[tokio::test]
    async fn wallet_signature_proves_only_the_exact_challenge() {
        let signer = PrivateKeySigner::random();
        let address = format!("{:#x}", signer.address());
        let issued = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
        let message = wallet_link_message(
            "https://agentbounties.app",
            &"a".repeat(64),
            &address,
            "nonce",
            issued,
            issued + ChronoDuration::minutes(5),
        );
        let signature = signer
            .sign_message_sync(message.as_bytes())
            .unwrap()
            .to_string();
        assert!(verify_wallet_signature(&message, &signature, &address));
        assert!(!verify_wallet_signature(
            &(message + " changed"),
            &signature,
            &address
        ));
    }

    #[test]
    fn money_format_keeps_exact_usdc_precision() {
        assert_eq!(usdc_from_base_units(0), "0");
        assert_eq!(usdc_from_base_units(1), "0.000001");
        assert_eq!(usdc_from_base_units(2_200_000), "2.2");
    }
}
