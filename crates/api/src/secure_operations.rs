use crate::{security_auth, SharedState};
use axum::http::{HeaderMap, StatusCode};
use service_runtime::auth::{self, Principal};

pub async fn authenticated(
    state: &SharedState,
    headers: &HeaderMap,
) -> Result<Principal, StatusCode> {
    let principal = security_auth::principal_for_write(state, headers).await?;
    if principal.is_authenticated() {
        Ok(principal)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}
pub async fn require_resource(
    state: &SharedState,
    principal: &Principal,
    kind: &str,
    id: &str,
    permission: &str,
) -> Result<(), StatusCode> {
    if auth::authorize_resource(state.store.as_ref(), principal, kind, id, permission)
        .await
        .map_err(security_auth::status)?
    {
        Ok(())
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
pub async fn assign_owner(
    state: &SharedState,
    principal: &Principal,
    kind: &str,
    id: &str,
) -> Result<(), StatusCode> {
    // Local operator fixtures need no persistent owner; hosted user writes always require PostgreSQL.
    if state.store.is_none() && matches!(principal, Principal::Operator) {
        return Ok(());
    }
    auth::assign_resource_owner(
        state
            .store
            .as_ref()
            .ok_or(StatusCode::SERVICE_UNAVAILABLE)?,
        principal,
        kind,
        id,
    )
    .await
    .map_err(security_auth::status)
}
pub fn mutation_status(error: db::security_writes::OwnedMutationError) -> StatusCode {
    use db::security_writes::OwnedMutationError;
    match error {
        OwnedMutationError::Auth(error) => security_auth::status(error.into()),
        OwnedMutationError::Conflict => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
pub fn limit(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| *v > 0 && *v <= 10000)
        .unwrap_or(default)
}

pub async fn read_bounty(
    state: &SharedState,
    principal: &Principal,
    id: uuid::Uuid,
) -> Result<(), StatusCode> {
    service_runtime::privacy::bounty_status_for_principal(
        state.store.as_ref(),
        &state.network,
        id,
        principal,
    )
    .await
    .map(|_| ())
    .map_err(|e| {
        if e.retryable() {
            StatusCode::SERVICE_UNAVAILABLE
        } else {
            StatusCode::NOT_FOUND
        }
    })
}
pub async fn solver_agent(
    state: &SharedState,
    principal: &Principal,
    id: uuid::Uuid,
) -> Result<(), StatusCode> {
    require_resource(state, principal, "agent", &id.to_string(), "write").await?;
    let agent = if let Some(store) = state.store.as_ref() {
        store
            .get_agent(id)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::NOT_FOUND)?
    } else {
        state
            .network
            .lock()
            .expect("state poisoned")
            .agents
            .get(&id)
            .cloned()
            .ok_or(StatusCode::NOT_FOUND)?
    };
    if agent.status != domain::AgentStatus::Active {
        return Err(StatusCode::FORBIDDEN);
    }
    let wallet = agent.payout_wallet.clone().ok_or(StatusCode::FORBIDDEN)?;
    state
        .network
        .lock()
        .expect("state poisoned")
        .agents
        .insert(id, agent);
    security_auth::authorize_verified_wallet(state, principal, &wallet, "base-mainnet").await
}

#[derive(serde::Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ParticipantGrantRequest {
    /// Participant's principal from GET /v1/auth/session; no account linking occurs.
    pub principal_key: String,
    /// Required for granting access; ignored when revoking it.
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}
pub(crate) fn participant_key(value: &str) -> Result<(), StatusCode> {
    let valid = if let Some(value) = value.strip_prefix("account:") {
        value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
    } else if let Some(value) = value.strip_prefix("wallet:") {
        value.split_once(':').is_some_and(|(chain, address)| {
            matches!(chain, "8453" | "84532")
                && auth::normalize_wallet(address).is_ok_and(|v| v == address)
        })
    } else {
        false
    };
    if valid {
        Ok(())
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}
#[utoipa::path(post,path="/v1/bounties/{id}/participants",request_body=ParticipantGrantRequest,responses((status=204,description="Read access granted to the named participant"),(status=404,description="Owned bounty not found")))]
pub async fn grant_participant(
    axum::extract::State(state): axum::extract::State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ParticipantGrantRequest>,
) -> Result<StatusCode, StatusCode> {
    let principal = authenticated(&state, &headers).await?;
    let key = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    participant_key(&request.principal_key)?;
    let expires = request.expires_at.ok_or(StatusCode::BAD_REQUEST)?;
    if expires <= chrono::Utc::now() || expires > chrono::Utc::now() + chrono::Duration::days(30) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .grant_resource_access(
            "bounty",
            &id.to_string(),
            &key,
            &request.principal_key,
            "read",
            expires,
        )
        .await
        .map_err(|e| match e {
            db::auth::AuthStoreError::Conflict => StatusCode::NOT_FOUND,
            e => security_auth::status(e.into()),
        })?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/v1/bounties/{id}/participants/revoke",request_body=ParticipantGrantRequest,responses((status=204,description="Participant read access revoked"),(status=404,description="Owned bounty not found")))]
pub async fn revoke_participant(
    axum::extract::State(state): axum::extract::State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<uuid::Uuid>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ParticipantGrantRequest>,
) -> Result<StatusCode, StatusCode> {
    let principal = authenticated(&state, &headers).await?;
    let key = principal.key().ok_or(StatusCode::UNAUTHORIZED)?;
    participant_key(&request.principal_key)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .revoke_resource_access("bounty", &id.to_string(), &key, &request.principal_key)
        .await
        .map_err(|e| match e {
            db::auth::AuthStoreError::Conflict => StatusCode::NOT_FOUND,
            e => security_auth::status(e.into()),
        })?;
    Ok(StatusCode::NO_CONTENT)
}
