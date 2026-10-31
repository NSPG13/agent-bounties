//! Shared authenticated evidence surface. No endpoint grants settlement authority.
use crate::{secure_operations, SharedState};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use broad_verification::{Manifest, Plan, MAX_FILES, MAX_UPLOAD};
use db::verification::{Artifact, EvidenceError, VerificationRun};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;
use worker::artifact_store::{validate_archive, ArtifactStore};

fn status(e: EvidenceError) -> StatusCode {
    match e {
        EvidenceError::NotFound => StatusCode::NOT_FOUND,
        EvidenceError::Conflict => StatusCode::CONFLICT,
        EvidenceError::Capacity => StatusCode::TOO_MANY_REQUESTS,
        EvidenceError::Invalid => StatusCode::BAD_REQUEST,
        EvidenceError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    }
}
fn storage() -> Result<ArtifactStore, StatusCode> {
    ArtifactStore::from_env().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
fn store(state: &SharedState) -> Result<&db::PostgresStore, StatusCode> {
    state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)
}
async fn owner(state: &SharedState, headers: &HeaderMap) -> Result<String, StatusCode> {
    secure_operations::authenticated(state, headers)
        .await?
        .key()
        .ok_or(StatusCode::UNAUTHORIZED)
}
pub(crate) struct WriteOwner(String);
#[axum::async_trait]
impl axum::extract::FromRequestParts<SharedState> for WriteOwner {
    type Rejection = StatusCode;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        owner(state, &parts.headers).await.map(Self)
    }
}
fn key(headers: &HeaderMap) -> Result<&str, StatusCode> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 128 && v.bytes().all(|b| b.is_ascii_graphic()))
        .ok_or(StatusCode::BAD_REQUEST)
}

pub fn router() -> Router<SharedState> {
    Router::new()
        .route("/v1/verification/checkers", get(find_checks))
        .route("/v1/verification/plans/validate", post(validate_plan))
        .route(
            "/v1/verification/artifacts",
            post(upload).layer(DefaultBodyLimit::max(
                (MAX_UPLOAD * 4 / 3) + 2 * 1024 * 1024,
            )),
        )
        .route(
            "/v1/verification/artifacts/import-github",
            post(import_github),
        )
        .route("/v1/verification/artifacts/:id", get(get_artifact))
        .route("/v1/verification/artifacts/:id/files/:index", get(download))
        .route("/v1/verification/runs", post(run_checks))
        .route("/v1/verification/runs/:id", get(get_run))
        .layer(DefaultBodyLimit::max(512 * 1024))
}
#[utoipa::path(get,path="/v1/verification/checkers",responses((status=200,description="Versioned broad checker catalog with explicit limitations")))]
pub async fn find_checks() -> Json<Value> {
    Json(broad_verification::catalog())
}
#[utoipa::path(post,path="/v1/verification/plans/validate",request_body=Value,responses((status=200,description="Structural plan validation; no funding or settlement authority"),(status=400,description="Invalid checking plan")))]
pub async fn validate_plan(body: Bytes) -> Result<Json<Value>, StatusCode> {
    let plan: Plan = serde_json::from_value(
        broad_verification::parse_json(&body).map_err(|_| StatusCode::BAD_REQUEST)?,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    match broad_verification::validate_plan(&plan) {
        Ok(()) => Ok(Json(
            json!({"valid":true,"plan_hash":broad_verification::value_digest(&plan).map_err(|_|StatusCode::BAD_REQUEST)?,"payment_authorized":false,"notice":"The creator must accept the meaning of these checks before funding. Structural coverage is not a guarantee of complete verification."}),
        )),
        Err(e) => Ok(Json(
            json!({"valid":false,"error":e.to_string(),"payment_authorized":false}),
        )),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    files: Vec<UploadedFile>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadedFile {
    path: String,
    base64: String,
}
async fn persist(
    state: &SharedState,
    owner: &str,
    key: &str,
    files: BTreeMap<String, Vec<u8>>,
    source: Option<&Value>,
) -> Result<Artifact, StatusCode> {
    let manifest = Manifest::from_files(&files).map_err(|_| StatusCode::BAD_REQUEST)?;
    // Check configuration before reserving capacity; failed storage keeps a retryable reservation.
    let objects = storage()?;
    let db = store(state)?;
    let files = tokio::task::spawn_blocking(move || {
        let mut expanded = 0u64;
        for (path, bytes) in &files {
            expanded = expanded
                .checked_add(validate_archive(path, bytes).map_err(|_| StatusCode::BAD_REQUEST)?)
                .ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
            if expanded > broad_verification::MAX_EXPANDED as u64 {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
        }
        Ok::<_, StatusCode>(files)
    })
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)??;
    let capacity = std::env::var("ARTIFACT_STORAGE_CAPACITY_BYTES")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n >= MAX_UPLOAD as i64)
        .unwrap_or(10 * 1024 * 1024 * 1024);
    let artifact = db
        .reserve_artifact_from_source(owner, key, &manifest, capacity, source)
        .await
        .map_err(status)?;
    if artifact.status == "ready" {
        return Ok(artifact);
    }
    objects
        .put(artifact.id, &manifest, &files)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    db.finish_artifact(artifact.id, owner, &artifact.manifest_hash)
        .await
        .map_err(status)
}
#[utoipa::path(post,path="/v1/verification/artifacts",request_body=Value,responses((status=200,description="Durably stored exact artifact; canonical submission is a separate step"),(status=401,description="Authentication required"),(status=409,description="Idempotency conflict"),(status=503,description="Storage unavailable")))]
pub async fn upload(
    State(state): State<SharedState>,
    WriteOwner(owner): WriteOwner,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Json<Artifact>, StatusCode> {
    let key = key(&headers)?;
    let upload: Upload = serde_json::from_value(value).map_err(|_| StatusCode::BAD_REQUEST)?;
    if upload.files.is_empty() || upload.files.len() > MAX_FILES {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut files = BTreeMap::new();
    let mut size = 0usize;
    for file in upload.files {
        if !broad_verification::safe_path(&file.path)
            || files.contains_key(&file.path)
            || file.base64.len() > MAX_UPLOAD * 4 / 3 + 4
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let bytes = STANDARD
            .decode(file.base64)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        size = size
            .checked_add(bytes.len())
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
        if size > MAX_UPLOAD {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        files.insert(file.path, bytes);
    }
    persist(&state, &owner, key, files, None).await.map(Json)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GithubImport {
    repository: String,
    commit: String,
    paths: Vec<String>,
}
#[utoipa::path(post,path="/v1/verification/artifacts/import-github",request_body=Value,responses((status=200,description="Stored selected files from an exact public GitHub commit"),(status=400,description="Invalid commit or paths")))]
pub async fn import_github(
    State(state): State<SharedState>,
    WriteOwner(owner): WriteOwner,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Result<Json<Artifact>, StatusCode> {
    let key = key(&headers)?;
    let mut request: GithubImport =
        serde_json::from_value(value).map_err(|_| StatusCode::BAD_REQUEST)?;
    let parts: Vec<_> = request.repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || s.len() > 100
                || *s == "."
                || *s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
        || request.commit.len() != 40
        || !request.commit.bytes().all(|b| b.is_ascii_hexdigit())
        || request.paths.is_empty()
        || request.paths.len() > 64
        || request
            .paths
            .iter()
            .any(|p| !broad_verification::safe_path(p))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    request.commit.make_ascii_lowercase();
    request.paths.sort();
    if request.paths.windows(2).any(|v| v[0] == v[1]) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let source = json!({"kind":"github_commit","repository":request.repository,"commit":request.commit,"paths":request.paths});
    let db = store(&state)?;
    if let Some(existing) = db
        .artifact_by_request(&owner, key, &source)
        .await
        .map_err(status)?
    {
        if existing.status == "ready" {
            return Ok(Json(existing));
        }
    }
    let payload_hash =
        broad_verification::value_digest(&source).map_err(|_| StatusCode::BAD_REQUEST)?;
    let attempt_hash = broad_verification::digest(Uuid::new_v4().as_bytes());
    db.consume_security_quota_once(
        &owner,
        "artifact_github_fetch",
        &attempt_hash[7..],
        &payload_hash[7..],
        10,
        60,
    )
    .await
    .map_err(|e| match e {
        db::auth::AuthStoreError::QuotaExceeded => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    })?;
    storage()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut files = BTreeMap::new();
    let mut size = 0usize;
    for path in request.paths {
        if files.contains_key(&path) {
            return Err(StatusCode::BAD_REQUEST);
        }
        let mut url = url::Url::parse("https://raw.githubusercontent.com/").expect("fixed URL");
        url.path_segments_mut().expect("fixed origin").extend(
            parts
                .iter()
                .copied()
                .chain(std::iter::once(request.commit.as_str()))
                .chain(path.split('/')),
        );
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?;
        if !response.status().is_success() {
            return Err(StatusCode::BAD_GATEWAY);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_UPLOAD as u64)
        {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| StatusCode::BAD_GATEWAY)?
        {
            size = size
                .checked_add(chunk.len())
                .ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
            if size > MAX_UPLOAD {
                return Err(StatusCode::PAYLOAD_TOO_LARGE);
            }
            bytes.extend_from_slice(&chunk);
        }
        files.insert(path, bytes);
    }
    persist(&state, &owner, key, files, Some(&source))
        .await
        .map(Json)
}
#[utoipa::path(get,path="/v1/verification/artifacts/{id}",params(("id"=Uuid,Path,description="Evidence record ID")),responses((status=200,description="Owned artifact metadata"),(status=404,description="Artifact unavailable to this caller")))]
pub async fn get_artifact(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Artifact>, StatusCode> {
    crate::artifact_submission::readable(&state, id, &headers)
        .await
        .map(Json)
}
#[utoipa::path(get,path="/v1/verification/artifacts/{id}/files/{index}",params(("id"=Uuid,Path,description="Evidence record ID"),("index"=usize,Path,description="Manifest file index")),responses((status=200,description="Authorized exact-byte download"),(status=404,description="File unavailable")))]
pub async fn download(
    State(state): State<SharedState>,
    Path((id, index)): Path<(Uuid, usize)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let artifact = crate::artifact_submission::readable(&state, id, &headers).await?;
    if artifact.status != "ready" {
        return Err(StatusCode::CONFLICT);
    }
    let file = artifact
        .manifest
        .files
        .get(index)
        .ok_or(StatusCode::NOT_FOUND)?;
    let bytes = storage()?
        .get_file(id, &file.sha256, file.size)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment"),
            (header::CACHE_CONTROL, "private, no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRequest {
    artifact_id: Uuid,
    plan: Plan,
}
#[utoipa::path(post,path="/v1/verification/runs",request_body=Value,responses((status=200,description="Principal-scoped repeatable verification report, not settlement"),(status=401,description="Authentication required")))]
pub async fn run_checks(
    State(state): State<SharedState>,
    WriteOwner(owner): WriteOwner,
    body: Bytes,
) -> Result<Json<VerificationRun>, StatusCode> {
    let request: RunRequest = serde_json::from_value(
        broad_verification::parse_json(&body).map_err(|_| StatusCode::BAD_REQUEST)?,
    )
    .map_err(|_| StatusCode::BAD_REQUEST)?;
    let db = store(&state)?;
    storage()?;
    db.begin_verification_run(&owner, request.artifact_id, &request.plan)
        .await
        .map(Json)
        .map_err(status)
}
#[utoipa::path(get,path="/v1/verification/runs/{id}",params(("id"=Uuid,Path,description="Evidence record ID")),responses((status=200,description="Owned verification report"),(status=404,description="Run unavailable to this caller")))]
pub async fn get_run(
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<VerificationRun>, StatusCode> {
    let owner = crate::security_auth::principal(&state, &headers)
        .await
        .map_err(|status| match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => StatusCode::NOT_FOUND,
            other => other,
        })?
        .key()
        .ok_or(StatusCode::NOT_FOUND)?;
    store(&state)?
        .verification_run(id, &owner)
        .await
        .map(Json)
        .map_err(status)
}

pub fn start_worker(state: SharedState) {
    if let (Some(db), Ok(objects)) = (state.store.clone(), storage()) {
        tokio::spawn(worker::broad_checks::run(db, objects.clone(), None));
        tokio::spawn(crate::artifact_submission::cleanup_loop(state, objects));
    }
}
