//! Assignment grants preparation access only; the committed reviewer still signs.
use crate::{secure_operations, security_auth, SharedState};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;
pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/verification/review-tasks", get(list))
        .route("/v1/verification/review-tasks/:id/assign", post(assign))
        .route("/v1/verification/review-tasks/:id/accept", post(accept))
}
fn db(state: &SharedState) -> Result<&db::PostgresStore, StatusCode> {
    state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
#[utoipa::path(get,path="/v1/verification/review-tasks",responses((status=200,description="List authorized pending review tasks"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn list(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let key = security_auth::principal(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let tasks = db(&state)?
        .review_tasks(&key)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(
        json!({"tasks":tasks,"recommendation":"Choose an agent that has accepted the review task. You may review it yourself, but a person may miss a deadline. Assignment does not grant signing authority."}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assignment {
    assignee_principal: String,
}
fn valid_principal(value: &str) -> bool {
    if let Some(id) = value.strip_prefix("account:") {
        return id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(value) = value.strip_prefix("wallet:") {
        return value.split_once(':').is_some_and(|(chain, address)| {
            matches!(chain, "8453" | "84532")
                && service_runtime::auth::normalize_wallet(address).is_ok_and(|v| v == address)
        });
    }
    false
}
#[utoipa::path(post,path="/v1/verification/review-tasks/{id}/assign",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Assign review preparation without signing authority"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn assign(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(input): Json<Assignment>,
) -> Result<Json<Value>, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !valid_principal(&input.assignee_principal) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !db(&state)?
        .assign_review_task(id, &key, &input.assignee_principal)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(
        json!({"assigned":true,"agent_ready":false,"signing_authority_granted":false}),
    ))
}
#[utoipa::path(post,path="/v1/verification/review-tasks/{id}/accept",request_body=Value,params(("id"=Uuid,Path,description="Record ID")),responses((status=200,description="Accept an assigned review task"),(status=404,description="Unavailable to this caller")))]
pub(crate) async fn accept(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let key = secure_operations::authenticated(&state, &headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if !db(&state)?
        .accept_review_task(id, &key)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(
        json!({"accepted":true,"signing_authority_granted":false}),
    ))
}
