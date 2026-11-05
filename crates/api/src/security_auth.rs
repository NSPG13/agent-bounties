//! Wallet-only SIWE sessions and shared API caller authorization.
use crate::{site_auth::SiteAuthService, SharedState};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use db::PostgresStore;
use serde::Deserialize;
use serde_json::json;
use service_runtime::auth::{self, AuthError, AuthService, Principal, RpcWalletVerifier};
use std::{collections::BTreeMap, env, net::SocketAddr};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone)]
pub struct SecurityAuthState {
    site_auth: Option<SiteAuthService>,
    services: BTreeMap<u64, AuthService>,
    verifiers: BTreeMap<u64, RpcWalletVerifier>,
    ip_hash_secret: Option<String>,
}
impl SecurityAuthState {
    #[cfg(test)]
    pub(crate) fn for_managed_payment_test(site_auth: SiteAuthService) -> Self {
        let mut state = Self::disabled();
        state.site_auth = Some(site_auth);
        state
    }
    #[cfg(test)]
    pub fn disabled() -> Self {
        Self {
            site_auth: None,
            services: BTreeMap::new(),
            verifiers: BTreeMap::new(),
            ip_hash_secret: None,
        }
    }
    pub fn from_env(
        store: Option<PostgresStore>,
        site_auth: SiteAuthService,
    ) -> anyhow::Result<Self> {
        let origin =
            env::var("PUBLIC_BASE_URL").unwrap_or_else(|_| "https://api.agentizable.ai".into());
        let mut services = BTreeMap::new();
        let mut verifiers = BTreeMap::new();
        for (chain_id, env_name) in [
            (8453, "BASE_MAINNET_RPC_URL"),
            (84532, "BASE_SEPOLIA_RPC_URL"),
        ] {
            if let Some(store) = store.clone() {
                services.insert(chain_id, AuthService::new(store, &origin, chain_id)?);
            }
            if let Some(url) = env::var(env_name).ok().filter(|s| !s.trim().is_empty()) {
                verifiers.insert(chain_id, RpcWalletVerifier::new(url, chain_id)?);
            }
        }
        let secret = env::var("AUTH_IP_RATE_LIMIT_SECRET")
            .or_else(|_| env::var("AUTH_WALLET_LINK_SECRET"))
            .ok()
            .filter(|s| s.len() >= 32);
        Ok(Self {
            site_auth: Some(site_auth),
            services,
            verifiers,
            ip_hash_secret: secret,
        })
    }
}
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/auth/wallet/challenge", post(auth_wallet_challenge))
        .route("/v1/auth/wallet/session", post(auth_wallet_session))
        .route("/v1/auth/session", get(auth_session))
        .route("/v1/auth/session/revoke", post(revoke_auth_session))
        .layer(DefaultBodyLimit::max(20 * 1024))
}
pub fn status(error: AuthError) -> StatusCode {
    match error {
        AuthError::Unauthorized => StatusCode::UNAUTHORIZED,
        AuthError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        AuthError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
        AuthError::Conflict => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    }
}
pub async fn principal(state: &SharedState, headers: &HeaderMap) -> Result<Principal, StatusCode> {
    principal_inner(state, headers, false).await
}
pub async fn principal_for_write(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<Principal, StatusCode> {
    principal_inner(state, headers, true).await
}
async fn principal_inner(
    state: &SharedState,
    headers: &HeaderMap,
    write: bool,
) -> Result<Principal, StatusCode> {
    let account = state
        .security_auth
        .site_auth
        .as_ref()
        .and_then(|site| site.authenticated_account_id(headers, write));
    auth::authenticate_principal(
        state.store.as_ref(),
        state.operator_api_token.as_deref(),
        credential_header(headers, "x-operator-token")?,
        credential_header(headers, "authorization")?,
        account.as_deref(),
    )
    .await
    .map_err(status)
}
fn credential_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a str>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if values.next().is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    value
        .map(|value| value.to_str().map_err(|_| StatusCode::UNAUTHORIZED))
        .transpose()
}
pub async fn authorize_verified_wallet(
    state: &SharedState,
    principal: &Principal,
    solver_wallet: &str,
    network: &str,
) -> Result<(), StatusCode> {
    let chain = match network {
        "base-mainnet" => 8453,
        "base-sepolia" => 84532,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    if auth::authorize_wallet(state.store.as_ref(), principal, chain, solver_wallet)
        .await
        .map_err(status)?
    {
        Ok(())
    } else {
        Err(StatusCode::FORBIDDEN)
    }
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletChallengeRequest {
    pub address: String,
    pub chain_id: Option<u64>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletSessionRequest {
    pub challenge_id: Uuid,
    pub signature: String,
    pub chain_id: Option<u64>,
}
fn no_store(value: impl serde::Serialize) -> Response {
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    response
}
fn failure(error: AuthError) -> Response {
    let code = match &error {
        AuthError::Unauthorized => "authentication_required",
        AuthError::Unavailable => "authentication_unavailable",
        AuthError::InvalidChallenge => "challenge_invalid_or_expired",
        AuthError::InvalidSignature => "wallet_signature_invalid",
        AuthError::InvalidInput => "invalid_authentication_request",
        AuthError::Conflict => "idempotency_conflict",
        AuthError::QuotaExceeded => "authentication_rate_limited",
    };
    let limited = matches!(&error, AuthError::QuotaExceeded);
    let mut response = (status(error), Json(json!({"error":code}))).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
    if limited {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
    }
    response
}

fn client_ip_bucket(state: &SharedState, peer: Option<ConnectInfo<SocketAddr>>) -> String {
    // Forwarded headers are caller-controlled unless the ingress is explicitly trusted. Missing peer/secret uses one conservative shared bucket.
    match (peer, &state.security_auth.ip_hash_secret) {
        (Some(ConnectInfo(peer)), Some(secret)) => {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
                .expect("HMAC accepts arbitrary key length");
            mac.update(peer.ip().to_string().as_bytes());
            hex::encode(mac.finalize().into_bytes())
        }
        _ => "shared-ingress".to_string(),
    }
}

#[derive(Clone, Copy)]
enum AuthEndpoint {
    Exchange,
    SessionRead,
    Revoke,
}
impl AuthEndpoint {
    fn rate(self) -> (&'static str, i64) {
        match self {
            Self::Exchange => ("auth_exchange_ip", 60),
            Self::SessionRead => ("auth_read_ip", 120),
            Self::Revoke => ("auth_revoke_ip", 60),
        }
    }
}
async fn auth_endpoint_quota(
    store: Option<&PostgresStore>,
    bucket: &str,
    endpoint: AuthEndpoint,
) -> Result<(), AuthError> {
    let store = store.ok_or(AuthError::Unavailable)?;
    let (action, limit) = endpoint.rate();
    let request_id = Uuid::new_v4();
    let request_hash = auth::hash_secret(&request_id.to_string());
    store
        .consume_security_quota_once(
            &format!("ip:{}", auth::hash_secret(bucket)),
            action,
            &request_hash,
            &request_hash,
            limit,
            60,
        )
        .await?;
    // Bounded amortized cleanup also removes expired buckets for clients that never return.
    if request_id.as_bytes()[0] & 15 == 0 {
        store.prune_security_auth_state().await?;
    }
    Ok(())
}

#[utoipa::path(post,path="/v1/auth/wallet/challenge",request_body=WalletChallengeRequest,responses((status=200,body=WalletChallengeResponse),(status=400,description="Unsupported chain or invalid wallet"),(status=429,description="Durable challenge quota exceeded"),(status=503,description="Durable auth or chain RPC unavailable")))]
pub async fn auth_wallet_challenge(
    State(state): State<SharedState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    Json(request): Json<WalletChallengeRequest>,
) -> Response {
    let chain_id = request.chain_id.unwrap_or(8453);
    if !matches!(chain_id, 8453 | 84532) {
        return failure(AuthError::InvalidInput);
    }
    let Some(service) = state.security_auth.services.get(&chain_id) else {
        return failure(AuthError::Unavailable);
    };
    if !state.security_auth.verifiers.contains_key(&chain_id) {
        return failure(AuthError::Unavailable);
    }
    let bucket = client_ip_bucket(&state, peer);
    match service.begin_challenge(&request.address, &bucket).await {
        Ok(value) => no_store(value),
        Err(error) => failure(error),
    }
}
#[utoipa::path(post,path="/v1/auth/wallet/session",request_body=WalletSessionRequest,responses((status=200,body=WalletSessionResponse),(status=400,description="Invalid signature or expired, replayed or mismatched challenge"),(status=429,description="Authentication exchange rate limit exceeded"),(status=503,description="Durable auth or chain RPC unavailable")))]
pub async fn auth_wallet_session(
    State(state): State<SharedState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    Json(request): Json<WalletSessionRequest>,
) -> Response {
    let chain_id = request.chain_id.unwrap_or(8453);
    if !matches!(chain_id, 8453 | 84532) {
        return failure(AuthError::InvalidInput);
    }
    let (Some(service), Some(verifier)) = (
        state.security_auth.services.get(&chain_id),
        state.security_auth.verifiers.get(&chain_id),
    ) else {
        return failure(AuthError::Unavailable);
    };
    if let Err(error) = auth_endpoint_quota(
        state.store.as_ref(),
        &client_ip_bucket(&state, peer),
        AuthEndpoint::Exchange,
    )
    .await
    {
        return failure(error);
    }
    match service
        .complete_challenge(request.challenge_id, &request.signature, verifier)
        .await
    {
        Ok(value) => no_store(value),
        Err(error) => failure(error),
    }
}
#[utoipa::path(get,path="/v1/auth/session",responses((status=200,body=CallerSessionResponse),(status=401,description="Missing, expired, revoked or invalid session"),(status=429,description="Session status rate limit exceeded")),security(("wallet_session"=[])))]
pub async fn auth_session(
    State(state): State<SharedState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = auth_endpoint_quota(
        state.store.as_ref(),
        &client_ip_bucket(&state, peer),
        AuthEndpoint::SessionRead,
    )
    .await
    {
        return failure(error);
    }
    match principal(&state, &headers).await {
        Ok(Principal::Anonymous) => failure(AuthError::Unauthorized),
        Ok(value) => no_store(json!({"principal":value})),
        Err(code) => (
            code,
            [(header::CACHE_CONTROL, "no-store, private")],
            Json(json!({"error":"authentication_required"})),
        )
            .into_response(),
    }
}
#[utoipa::path(post,path="/v1/auth/session/revoke",responses((status=200,description="Bearer session revoked"),(status=401,description="Session bearer required"),(status=429,description="Session revocation rate limit exceeded")),security(("wallet_session"=[])))]
pub async fn revoke_auth_session(
    State(state): State<SharedState>,
    peer: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = auth_endpoint_quota(
        state.store.as_ref(),
        &client_ip_bucket(&state, peer),
        AuthEndpoint::Revoke,
    )
    .await
    {
        return failure(error);
    }
    let Some(service) = state.security_auth.services.values().next() else {
        return failure(AuthError::Unavailable);
    };
    let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|s| s.to_str().ok())
    else {
        return failure(AuthError::Unauthorized);
    };
    match service.revoke(auth).await {
        Ok(()) => no_store(json!({"revoked":true})),
        Err(error) => failure(error),
    }
}

#[derive(serde::Serialize, ToSchema)]
pub struct WalletChallengeResponse {
    pub challenge_id: Uuid,
    pub message: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub chain_id: u64,
    pub address: String,
}
#[derive(serde::Serialize, ToSchema)]
pub struct WalletSessionResponse {
    pub token: String,
    pub token_type: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub principal: serde_json::Value,
}
#[derive(serde::Serialize, ToSchema)]
pub struct CallerSessionResponse {
    pub principal: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_and_duplicate_credentials_cannot_fall_back_to_account_cookie() {
        let mut headers = HeaderMap::new();
        assert!(credential_header(&headers, "authorization")
            .unwrap()
            .is_none());
        headers.insert("authorization", HeaderValue::from_bytes(&[0xff]).unwrap());
        assert_eq!(
            credential_header(&headers, "authorization"),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert("authorization", HeaderValue::from_static("Bearer first"));
        headers.append("authorization", HeaderValue::from_static("Bearer second"));
        assert_eq!(
            credential_header(&headers, "authorization"),
            Err(StatusCode::UNAUTHORIZED)
        );
        headers.insert(
            "x-operator-token",
            HeaderValue::from_static("local-operator"),
        );
        assert_eq!(
            credential_header(&headers, "x-operator-token"),
            Ok(Some("local-operator"))
        );
    }

    #[tokio::test]
    #[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
    async fn auth_endpoint_ip_limits_are_durable_separate_and_shared_across_pools() {
        let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let first = PostgresStore::connect(&url).await.unwrap();
        first.migrate().await.unwrap();
        let second = PostgresStore::connect(&url).await.unwrap();
        let bucket = Uuid::new_v4().to_string();
        for endpoint in [
            AuthEndpoint::Exchange,
            AuthEndpoint::SessionRead,
            AuthEndpoint::Revoke,
        ] {
            for index in 0..endpoint.rate().1 {
                let store = if index % 2 == 0 { &first } else { &second };
                auth_endpoint_quota(Some(store), &bucket, endpoint)
                    .await
                    .unwrap();
            }
            assert!(matches!(
                auth_endpoint_quota(Some(&second), &bucket, endpoint).await,
                Err(AuthError::QuotaExceeded)
            ));
        }
        assert!(matches!(
            auth_endpoint_quota(None, &bucket, AuthEndpoint::Exchange).await,
            Err(AuthError::Unavailable)
        ));
    }
}
