use crate::{
    analytics_exclusion_is_authorized, build_opportunity_projection, OpportunityQuery, SharedState,
};
use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        DefaultBodyLimit, Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use db::{AttributionReliability, DiscoveryInterface, DiscoveryRouteFamily};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use url::Url;
use utoipa::ToSchema;
use uuid::Uuid;

const A2A_PROTOCOL_VERSION: &str = "1.0";
const A2A_MEDIA_TYPE: &str = "application/a2a+json";
const MAX_TASKS: usize = 500;
const TASK_TTL_HOURS: i64 = 24;
const DEFAULT_PAGE_SIZE: usize = 20;
const MAX_PAGE_SIZE: usize = 100;
const AGENT_CARD_JSON: &str = include_str!("../fixtures/agent-card.json");
const PAYMENT_BOUNDARY: &str = "Only a confirmed canonical BountySettled event proves autonomous-v1 solver payment, and only a confirmed canonical CompetitionSettledV2 event proves Open Competition V2 solver payment.";

type TaskStore = Arc<Mutex<A2aTaskStore>>;
type A2aResult<T> = Result<T, Box<Response>>;

pub(crate) fn router() -> Router<SharedState> {
    Router::new()
        .route("/.well-known/agent-card.json", get(agent_card))
        .route("/a2a/v1/message:send", post(send_message))
        .route("/a2a/v1/tasks", get(list_tasks))
        .route("/a2a/v1/tasks/:task_id", get(get_task).post(cancel_task))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(Extension(Arc::new(Mutex::new(A2aTaskStore::default()))))
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aAgentCard {
    name: String,
    description: String,
    supported_interfaces: Vec<A2aAgentInterface>,
    provider: A2aAgentProvider,
    version: String,
    documentation_url: String,
    capabilities: A2aAgentCapabilities,
    default_input_modes: Vec<String>,
    default_output_modes: Vec<String>,
    skills: Vec<A2aAgentSkill>,
    icon_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct A2aAgentInterface {
    url: String,
    protocol_binding: String,
    protocol_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
struct A2aAgentProvider {
    url: String,
    organization: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct A2aAgentCapabilities {
    streaming: bool,
    push_notifications: bool,
    extended_agent_card: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct A2aAgentSkill {
    id: String,
    name: String,
    description: String,
    tags: Vec<String>,
    examples: Vec<String>,
    input_modes: Vec<String>,
    output_modes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aSendMessageRequest {
    message: A2aMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configuration: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aSendMessageResponse {
    task: A2aTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aMessage {
    message_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    role: String,
    parts: Vec<A2aPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub(crate) enum A2aTaskState {
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
}

impl A2aTaskState {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Canceled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aTaskStatus {
    state: A2aTaskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<A2aMessage>,
    timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aArtifact {
    artifact_id: String,
    name: String,
    parts: Vec<A2aPart>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aTask {
    id: String,
    context_id: String,
    status: A2aTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    artifacts: Option<Vec<A2aArtifact>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    history: Vec<A2aMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aListTasksResponse {
    tasks: Vec<A2aTask>,
    total_size: usize,
    page_size: usize,
    next_page_token: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aProblem {
    r#type: String,
    title: String,
    status: u16,
    detail: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    supported_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct A2aHttpErrorEnvelope {
    error: A2aHttpError,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct A2aHttpError {
    code: u16,
    status: String,
    message: String,
    details: Vec<A2aErrorInfo>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct A2aErrorInfo {
    #[serde(rename = "@type")]
    type_url: String,
    reason: String,
    domain: String,
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aGetTaskQuery {
    history_length: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct A2aListTasksQuery {
    context_id: Option<String>,
    status: Option<A2aTaskState>,
    page_size: Option<usize>,
    page_token: Option<String>,
    history_length: Option<usize>,
    include_artifacts: Option<bool>,
}

#[derive(Debug, Clone)]
struct StoredTask {
    task: A2aTask,
    message_id: String,
    request_fingerprint: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub(crate) struct A2aTaskStore {
    tasks: BTreeMap<String, StoredTask>,
    task_by_message_id: BTreeMap<String, String>,
}

impl A2aTaskStore {
    fn prune(&mut self, now: DateTime<Utc>) {
        let cutoff = now - Duration::hours(TASK_TTL_HOURS);
        let expired = self
            .tasks
            .iter()
            .filter(|(_, stored)| stored.created_at < cutoff)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            self.remove(&id);
        }
        while self.tasks.len() >= MAX_TASKS {
            let oldest = self
                .tasks
                .iter()
                .min_by_key(|(_, stored)| stored.created_at)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.remove(&id),
                None => break,
            }
        }
    }

    fn remove(&mut self, id: &str) {
        if let Some(stored) = self.tasks.remove(id) {
            self.task_by_message_id.remove(&stored.message_id);
        }
    }

    fn insert(&mut self, task: A2aTask, message_id: String, request_fingerprint: String) {
        let now = Utc::now();
        self.prune(now);
        self.task_by_message_id
            .insert(message_id.clone(), task.id.clone());
        self.tasks.insert(
            task.id.clone(),
            StoredTask {
                task,
                message_id,
                request_fingerprint,
                created_at: now,
                updated_at: now,
            },
        );
    }
}

#[derive(Debug)]
struct SkillInvocation {
    skill: String,
    parameters: Value,
}

#[derive(Debug)]
struct ExecutionOutcome {
    state: A2aTaskState,
    summary: String,
    artifact_name: Option<String>,
    data: Option<Value>,
}

#[utoipa::path(
    get,
    path = "/.well-known/agent-card.json",
    responses((status = 200, description = "A2A 1.0 Agent Card", body = A2aAgentCard))
)]
pub(crate) async fn agent_card() -> Response {
    let card: A2aAgentCard =
        serde_json::from_str(AGENT_CARD_JSON).expect("bundled A2A Agent Card must be valid");
    let mut response = Json(card).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300, must-revalidate"),
    );
    response
}

#[utoipa::path(
    post,
    path = "/a2a/v1/message:send",
    request_body(content = A2aSendMessageRequest, content_type = "application/a2a+json"),
    responses(
        (status = 200, description = "Completed or input-required A2A task", body = A2aSendMessageResponse, content_type = "application/a2a+json"),
        (status = 400, description = "Invalid request, unsupported A2A operation, or unsupported content type", body = A2aProblem, content_type = "application/problem+json"),
        (status = 409, description = "Message ID conflicts with an earlier request", body = A2aProblem, content_type = "application/problem+json")
    )
)]
pub(crate) async fn send_message(
    State(state): State<SharedState>,
    Extension(store): Extension<TaskStore>,
    headers: HeaderMap,
    request: Result<Json<A2aSendMessageRequest>, JsonRejection>,
) -> Response {
    if let Err(response) = validate_version(&headers) {
        return *response;
    }
    if let Err(response) = validate_content_type(&headers) {
        return *response;
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => {
            return problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request-too-large",
                "Request too large",
                "A2A request bodies are limited to 64 KiB.",
            )
        }
        Err(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid-parameters",
                "Invalid JSON request",
                "The request body must be valid A2A 1.0 JSON matching SendMessageRequest.",
            )
        }
    };
    if let Err(response) = validate_message(&request) {
        return *response;
    }

    let fingerprint = serde_json::to_string(&request).unwrap_or_default();
    {
        let mut guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "task-store-unavailable",
                    "Task store unavailable",
                    "The bounded A2A task store is temporarily unavailable.",
                )
            }
        };
        guard.prune(Utc::now());
        if let Some(task_id) = guard.task_by_message_id.get(&request.message.message_id) {
            if let Some(stored) = guard.tasks.get(task_id) {
                if stored.request_fingerprint != fingerprint {
                    return problem(StatusCode::CONFLICT, "message-id-conflict", "Message ID conflict", "This messageId was already used for a different request. Generate a new stable messageId.");
                }
                return a2a_json(
                    StatusCode::OK,
                    &A2aSendMessageResponse {
                        task: stored.task.clone(),
                    },
                );
            }
        }
    }

    let context_id = match resolve_context_id(&request.message, &store) {
        Ok(context_id) => context_id,
        Err(response) => return *response,
    };
    let invocation = invocation_from_message(&request.message);
    let analytics_excluded = analytics_exclusion_is_authorized(
        state.analytics_exclusion_token.as_deref(),
        state.operator_api_token.as_deref(),
        &headers,
    );
    let outcome = execute_skill(&state, invocation, analytics_excluded).await;
    let task = task_from_outcome(request.message.clone(), context_id, outcome);
    match store.lock() {
        Ok(mut guard) => guard.insert(
            task.clone(),
            request.message.message_id.clone(),
            fingerprint,
        ),
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "task-store-unavailable",
                "Task store unavailable",
                "The bounded A2A task store is temporarily unavailable.",
            )
        }
    }
    a2a_json(StatusCode::OK, &A2aSendMessageResponse { task })
}

#[utoipa::path(
    get,
    path = "/a2a/v1/tasks/{id}",
    params(("id" = String, Path, description = "A2A task ID"), ("historyLength" = Option<usize>, Query, description = "Most recent task messages to include")),
    responses(
        (status = 200, description = "Latest task state", body = A2aTask, content_type = "application/a2a+json"),
        (status = 404, description = "Task not found", body = A2aHttpErrorEnvelope, content_type = "application/a2a+json")
    )
)]
pub(crate) async fn get_task(
    Extension(store): Extension<TaskStore>,
    headers: HeaderMap,
    Path(id): Path<String>,
    query: Result<Query<A2aGetTaskQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = validate_version(&headers) {
        return *response;
    }
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid-parameters",
                "Invalid query parameters",
                "historyLength must be a non-negative integer.",
            )
        }
    };
    let mut guard = match store.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "task-store-unavailable",
                "Task store unavailable",
                "The bounded A2A task store is temporarily unavailable.",
            )
        }
    };
    guard.prune(Utc::now());
    let Some(stored) = guard.tasks.get(&id) else {
        return task_not_found(&id);
    };
    let task = with_history_length(stored.task.clone(), query.history_length);
    a2a_json(StatusCode::OK, &task)
}

#[utoipa::path(
    get,
    path = "/a2a/v1/tasks",
    params(
        ("contextId" = Option<String>, Query, description = "Filter by A2A context ID"),
        ("status" = Option<String>, Query, description = "Filter by A2A task state"),
        ("pageSize" = Option<usize>, Query, description = "Page size from 1 through 100"),
        ("pageToken" = Option<String>, Query, description = "Opaque cursor returned by the previous page"),
        ("historyLength" = Option<usize>, Query, description = "Most recent task messages to include"),
        ("includeArtifacts" = Option<bool>, Query, description = "Include task artifacts; defaults to false")
    ),
    responses(
        (status = 200, description = "Bounded task list", body = A2aListTasksResponse, content_type = "application/a2a+json"),
        (status = 400, description = "Invalid list parameters", body = A2aProblem, content_type = "application/problem+json")
    )
)]
pub(crate) async fn list_tasks(
    Extension(store): Extension<TaskStore>,
    headers: HeaderMap,
    query: Result<Query<A2aListTasksQuery>, QueryRejection>,
) -> Response {
    if let Err(response) = validate_version(&headers) {
        return *response;
    }
    let Query(query) = match query {
        Ok(query) => query,
        Err(_) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid-parameters",
                "Invalid query parameters",
                "Use the documented camelCase list-task query parameters and enum values.",
            )
        }
    };
    let page_size = query.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid parameters",
            "pageSize must be between 1 and 100 inclusive.",
        );
    }
    let mut guard = match store.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "task-store-unavailable",
                "Task store unavailable",
                "The bounded A2A task store is temporarily unavailable.",
            )
        }
    };
    guard.prune(Utc::now());
    let Some(context_id) = query.context_id.as_ref() else {
        if query.page_token.is_some() {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid-parameters",
                "Context required",
                "Anonymous task listing requires the opaque contextId returned by message:send.",
            );
        }
        return a2a_json(
            StatusCode::OK,
            &A2aListTasksResponse {
                tasks: Vec::new(),
                total_size: 0,
                page_size,
                next_page_token: String::new(),
            },
        );
    };
    let mut matches = guard
        .tasks
        .values()
        .filter(|stored| {
            stored.task.context_id == *context_id
                && query
                    .status
                    .as_ref()
                    .is_none_or(|status| stored.task.status.state == *status)
        })
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.task.id.cmp(&left.task.id))
    });
    let total_size = matches.len();
    let start = match query.page_token.as_deref() {
        None => 0,
        Some(token) => match decode_page_token(token) {
            Some((timestamp, id)) => matches
                .iter()
                .position(|stored| {
                    stored.task.status.timestamp == timestamp && stored.task.id == id
                })
                .map(|position| position + 1)
                .unwrap_or_else(|| usize::MAX),
            None => usize::MAX,
        },
    };
    if start == usize::MAX {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid parameters",
            "pageToken must be the opaque cursor returned by a previous response.",
        );
    }
    let include_artifacts = query.include_artifacts.unwrap_or(false);
    let tasks = matches
        .into_iter()
        .skip(start)
        .take(page_size)
        .map(|stored| list_task(stored.task, query.history_length, include_artifacts))
        .collect::<Vec<_>>();
    let next_position = start.saturating_add(tasks.len());
    let next_page_token = if next_position < total_size {
        tasks.last().map(encode_page_token).unwrap_or_default()
    } else {
        String::new()
    };
    a2a_json(
        StatusCode::OK,
        &A2aListTasksResponse {
            tasks,
            total_size,
            page_size,
            next_page_token,
        },
    )
}

#[utoipa::path(
    post,
    path = "/a2a/v1/tasks/{id}:cancel",
    params(("id" = String, Path, description = "A2A task ID")),
    responses(
        (status = 200, description = "Canceled task", body = A2aTask, content_type = "application/a2a+json"),
        (status = 400, description = "Terminal task cannot be canceled", body = A2aHttpErrorEnvelope, content_type = "application/a2a+json"),
        (status = 404, description = "Task not found", body = A2aHttpErrorEnvelope, content_type = "application/a2a+json")
    )
)]
pub(crate) async fn cancel_task(
    Extension(store): Extension<TaskStore>,
    headers: HeaderMap,
    Path(task_action): Path<String>,
) -> Response {
    if let Err(response) = validate_version(&headers) {
        return *response;
    }
    let Some(id) = task_action.strip_suffix(":cancel") else {
        return problem(
            StatusCode::NOT_FOUND,
            "operation-not-found",
            "Operation not found",
            "The only supported task mutation is POST /a2a/v1/tasks/{id}:cancel.",
        );
    };
    let mut guard = match store.lock() {
        Ok(guard) => guard,
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "task-store-unavailable",
                "Task store unavailable",
                "The bounded A2A task store is temporarily unavailable.",
            )
        }
    };
    guard.prune(Utc::now());
    let Some(stored) = guard.tasks.get_mut(id) else {
        return task_not_found(id);
    };
    if stored.task.status.state == A2aTaskState::Canceled {
        return a2a_json(StatusCode::OK, &stored.task);
    }
    if stored.task.status.state.is_terminal() {
        let mut metadata = BTreeMap::new();
        metadata.insert("taskId".to_string(), id.to_string());
        return a2a_protocol_error(
            StatusCode::BAD_REQUEST,
            "FAILED_PRECONDITION",
            "TASK_NOT_CANCELABLE",
            "The task is already terminal and cannot be canceled.",
            metadata,
        );
    }
    let reply = agent_message("The task was canceled before any action was taken.");
    let now = Utc::now();
    stored.task.status = A2aTaskStatus {
        state: A2aTaskState::Canceled,
        message: Some(reply.clone()),
        timestamp: a2a_timestamp(now),
    };
    stored.updated_at = now;
    stored.task.history.push(reply);
    a2a_json(StatusCode::OK, &stored.task)
}

fn validate_version(headers: &HeaderMap) -> A2aResult<()> {
    let Some(version) = headers.get("a2a-version") else {
        return Ok(());
    };
    let version = version.to_str().unwrap_or_default();
    if version == "1.0" {
        Ok(())
    } else {
        Err(Box::new(problem_with_versions(
            StatusCode::BAD_REQUEST,
            "version-not-supported",
            "Protocol version not supported",
            format!("A2A protocol version {version:?} is not supported by this agent."),
        )))
    }
}

fn validate_content_type(headers: &HeaderMap) -> A2aResult<()> {
    let Some(content_type) = headers.get(header::CONTENT_TYPE) else {
        return Ok(());
    };
    let content_type = content_type
        .to_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.starts_with(A2A_MEDIA_TYPE) || content_type.starts_with("application/json") {
        Ok(())
    } else {
        Err(Box::new(a2a_protocol_error(
            StatusCode::BAD_REQUEST,
            "INVALID_ARGUMENT",
            "CONTENT_TYPE_NOT_SUPPORTED",
            "Use application/a2a+json. application/json is also accepted for compatibility.",
            BTreeMap::new(),
        )))
    }
}

fn validate_message(request: &A2aSendMessageRequest) -> A2aResult<()> {
    let message = &request.message;
    if message.message_id.trim().is_empty() || message.message_id.len() > 128 {
        return Err(Box::new(problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid message",
            "message.messageId must contain 1 through 128 characters.",
        )));
    }
    if message.role != "ROLE_USER" {
        return Err(Box::new(problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid message",
            "message.role must be ROLE_USER.",
        )));
    }
    if message.parts.is_empty() || message.parts.len() > 16 {
        return Err(Box::new(problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid message",
            "message.parts must contain 1 through 16 parts.",
        )));
    }
    if message
        .parts
        .iter()
        .any(|part| part.text.as_ref().is_some_and(|text| text.len() > 8_000))
    {
        return Err(Box::new(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid-parameters",
            "Message part too large",
            "Each text part is limited to 8,000 UTF-8 bytes.",
        )));
    }
    if message.parts.iter().any(|part| {
        part.data
            .as_ref()
            .and_then(|data| serde_json::to_vec(data).ok())
            .is_some_and(|data| data.len() > 32_768)
    }) {
        return Err(Box::new(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "invalid-parameters",
            "Data part too large",
            "Each structured data part is limited to 32 KiB.",
        )));
    }
    if message
        .parts
        .iter()
        .any(|part| part.text.is_some() == part.data.is_some())
    {
        return Err(Box::new(problem(
            StatusCode::BAD_REQUEST,
            "invalid-parameters",
            "Invalid message",
            "Each part must contain exactly one supported content field: text or data.",
        )));
    }
    if request
        .configuration
        .as_ref()
        .is_some_and(|configuration| configuration.get("taskPushNotificationConfig").is_some())
    {
        return Err(Box::new(a2a_protocol_error(
            StatusCode::BAD_REQUEST,
            "FAILED_PRECONDITION",
            "PUSH_NOTIFICATION_NOT_SUPPORTED",
            "The public Agent Card declares pushNotifications=false. Use the Agent Bounties signed discovery webhook API separately.",
            BTreeMap::new(),
        )));
    }
    Ok(())
}

fn resolve_context_id(message: &A2aMessage, store: &TaskStore) -> A2aResult<String> {
    if let Some(task_id) = message.task_id.as_deref() {
        let guard = store.lock().map_err(|_| {
            Box::new(problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "task-store-unavailable",
                "Task store unavailable",
                "The bounded A2A task store is temporarily unavailable.",
            ))
        })?;
        let Some(stored) = guard.tasks.get(task_id) else {
            return Err(Box::new(task_not_found(task_id)));
        };
        if stored.task.status.state.is_terminal() {
            let mut metadata = BTreeMap::new();
            metadata.insert("taskId".to_string(), task_id.to_string());
            return Err(Box::new(a2a_protocol_error(
                StatusCode::BAD_REQUEST,
                "FAILED_PRECONDITION",
                "UNSUPPORTED_OPERATION",
                "Messages cannot be sent to a completed, failed, or canceled task. Start a new task and reuse the contextId if the conversation should continue.",
                metadata,
            )));
        }
        return Ok(stored.task.context_id.clone());
    }
    Ok(message
        .context_id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string()))
}

fn invocation_from_message(message: &A2aMessage) -> SkillInvocation {
    for part in &message.parts {
        if let Some(data) = part.data.as_ref() {
            if let Some(skill) = data.get("skill").and_then(Value::as_str) {
                return SkillInvocation {
                    skill: skill.to_string(),
                    parameters: data.clone(),
                };
            }
        }
    }
    let text = message
        .parts
        .iter()
        .filter_map(|part| part.text.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    let lowered = text.to_ascii_lowercase();
    if lowered.contains("alert")
        || lowered.contains("stay informed")
        || lowered.contains("keep informed")
    {
        return SkillInvocation {
            skill: "explain-bounty-alerts".to_string(),
            parameters: json!({"text": text}),
        };
    }
    if (lowered.contains("find") || lowered.contains("list") || lowered.contains("discover"))
        && (lowered.contains("bount") || lowered.contains("work"))
    {
        return SkillInvocation {
            skill: "discover-ready-to-earn-bounties".to_string(),
            parameters: json!({"text": text}),
        };
    }
    for prefix in ["explain bounty ", "show bounty ", "inspect bounty "] {
        if let Some(position) = lowered.find(prefix) {
            let opportunity_id = text[position + prefix.len()..].trim();
            return SkillInvocation {
                skill: "explain-bounty-opportunity".to_string(),
                parameters: json!({"opportunityId": opportunity_id}),
            };
        }
    }
    if lowered.contains("protocol")
        || lowered.contains("prove payment")
        || lowered.contains("interface")
    {
        return SkillInvocation {
            skill: "explain-agent-bounties-protocol".to_string(),
            parameters: json!({"text": text}),
        };
    }
    SkillInvocation {
        skill: "unsupported".to_string(),
        parameters: json!({"text": text}),
    }
}

async fn execute_skill(
    state: &SharedState,
    invocation: SkillInvocation,
    analytics_excluded: bool,
) -> ExecutionOutcome {
    let route_family = match invocation.skill.as_str() {
        "discover-ready-to-earn-bounties" => Some(DiscoveryRouteFamily::OpportunityList),
        "explain-bounty-opportunity" => Some(DiscoveryRouteFamily::OpportunityDetail),
        "explain-agent-bounties-protocol" => Some(DiscoveryRouteFamily::ProtocolOrientation),
        "explain-bounty-alerts" => Some(DiscoveryRouteFamily::Alerts),
        _ => None,
    };
    let outcome = match invocation.skill.as_str() {
        "discover-ready-to-earn-bounties" => discover_bounties(state, &invocation.parameters).await,
        "explain-bounty-opportunity" => explain_opportunity(state, &invocation.parameters).await,
        "explain-agent-bounties-protocol" => protocol_overview(),
        "explain-bounty-alerts" => alert_overview(),
        _ => ExecutionOutcome {
            state: A2aTaskState::InputRequired,
            summary: "Choose one advertised skill: discover-ready-to-earn-bounties, explain-bounty-opportunity, explain-agent-bounties-protocol, or explain-bounty-alerts. Structured data input is the most reliable option.".to_string(),
            artifact_name: None,
            data: None,
        },
    };
    if !analytics_excluded {
        if let (Some(store), Some(route_family)) = (state.store.clone(), route_family) {
            let succeeded = outcome.state != A2aTaskState::Failed;
            tokio::spawn(async move {
                let _ = store
                    .record_discovery_route_usage(
                        DiscoveryInterface::A2a,
                        route_family,
                        AttributionReliability::Observed,
                        succeeded,
                        Utc::now(),
                    )
                    .await;
            });
        }
    }
    outcome
}

async fn discover_bounties(state: &SharedState, parameters: &Value) -> ExecutionOutcome {
    let network =
        string_parameter(parameters, "network").unwrap_or_else(|| "base-mainnet".to_string());
    let view = string_parameter(parameters, "view").unwrap_or_else(|| "ready_to_earn".to_string());
    let source_type = string_parameter(parameters, "sourceType")
        .or_else(|| string_parameter(parameters, "source_type"))
        .unwrap_or_else(|| "canonical_base".to_string());
    let work_state = string_parameter(parameters, "workState")
        .or_else(|| string_parameter(parameters, "work_state"))
        .unwrap_or_else(|| "claimable".to_string());
    let payment_state = string_parameter(parameters, "paymentState")
        .or_else(|| string_parameter(parameters, "payment_state"))
        .unwrap_or_else(|| "escrowed".to_string());
    let limit = parameters
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let min_reward = parameters
        .get("minRewardBaseUnits")
        .and_then(value_as_u128)
        .unwrap_or(0);
    let category = string_parameter(parameters, "category").map(|value| value.to_ascii_lowercase());
    let skills = parameters
        .get("skills")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_ascii_lowercase)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let projection = build_opportunity_projection(
        state,
        OpportunityQuery {
            network: Some(network.clone()),
            view: Some(view.clone()),
            source_type: Some(source_type.clone()),
            work_state: Some(work_state.clone()),
            payment_state: Some(payment_state.clone()),
            limit: Some(300),
        },
    )
    .await;
    let mut projection = match projection {
        Ok(projection) => projection,
        Err(status) => {
            return ExecutionOutcome {
                state: A2aTaskState::Failed,
                summary: format!("The current opportunity projection could not be loaded (HTTP {}). No inventory was invented.", status.as_u16()),
                artifact_name: None,
                data: None,
            }
        }
    };
    projection.items.retain(|item| {
        let reward_matches = item.reward.currency.eq_ignore_ascii_case("USDC")
            && item.reward.amount.parse::<u128>().unwrap_or_default() >= min_reward;
        let category_matches = category.as_ref().is_none_or(|category| {
            item.categories
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(category))
        });
        let skills_match = skills.is_empty()
            || skills.iter().any(|required| {
                item.skills
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(required))
            });
        reward_matches && category_matches && skills_match
    });
    projection.items.truncate(limit);
    let items = projection
        .items
        .into_iter()
        .map(|item| {
            let canonical_url = item.public_url.clone();
            let opportunity_id = item.opportunity_id.clone();
            let attributed_url =
                attributed_discovery_url(&canonical_url, "a2a-opportunity-list", &opportunity_id);
            let mut value = serde_json::to_value(item).expect("opportunity item serializes");
            value
                .as_object_mut()
                .expect("opportunity item is an object")
                .insert(
                    "discovery_links".to_string(),
                    json!({
                        "canonical_url": canonical_url,
                        "attributed_url": attributed_url,
                        "interface": "a2a",
                        "campaign": "a2a-opportunity-list",
                        "discovery_id": opportunity_id
                    }),
                );
            value
        })
        .collect::<Vec<_>>();
    let count = items.len();
    let degraded = projection.degraded;
    ExecutionOutcome {
        state: A2aTaskState::Completed,
        summary: format!("Found {count} matching public opportunities. degraded={degraded}. Re-fetch the selected item before work because inventory and economics can change."),
        artifact_name: Some("Agent Bounties opportunity projection".to_string()),
        data: Some(json!({
            "schemaVersion": "agent-bounties/a2a-opportunity-result-v1",
            "generatedAt": projection.generated_at,
            "network": projection.network,
            "degraded": projection.degraded,
            "sourceStatuses": projection.source_statuses,
            "items": items,
            "evidenceBoundary": projection.evidence_boundary,
            "appliedFilters": {
                "view": view,
                "sourceType": source_type,
                "workState": work_state,
                "paymentState": payment_state,
                "minRewardBaseUnits": min_reward.to_string(),
                "skills": skills,
                "category": category,
                "limit": limit
            }
        })),
    }
}

async fn explain_opportunity(state: &SharedState, parameters: &Value) -> ExecutionOutcome {
    let Some(opportunity_id) = string_parameter(parameters, "opportunityId")
        .or_else(|| string_parameter(parameters, "opportunity_id"))
    else {
        return ExecutionOutcome {
            state: A2aTaskState::InputRequired,
            summary: "Provide a public opportunityId, for example canonical:base-mainnet:0x...."
                .to_string(),
            artifact_name: None,
            data: None,
        };
    };
    let projection = build_opportunity_projection(
        state,
        OpportunityQuery {
            network: Some("base-mainnet".to_string()),
            limit: Some(300),
            ..OpportunityQuery::default()
        },
    )
    .await;
    let projection = match projection {
        Ok(projection) => projection,
        Err(status) => {
            return ExecutionOutcome {
                state: A2aTaskState::Failed,
                summary: format!("The current opportunity projection could not be loaded (HTTP {}). No bounty status was invented.", status.as_u16()),
                artifact_name: None,
                data: None,
            }
        }
    };
    let Some(item) = projection
        .items
        .into_iter()
        .find(|item| item.opportunity_id == opportunity_id)
    else {
        return ExecutionOutcome {
            state: A2aTaskState::Completed,
            summary: format!("No current public opportunity matched {opportunity_id:?}. It may have changed state or expired; refresh discovery before acting."),
            artifact_name: Some("Opportunity lookup result".to_string()),
            data: Some(json!({"opportunityId": opportunity_id, "found": false, "generatedAt": projection.generated_at, "degraded": projection.degraded})),
        };
    };
    let canonical_url = item.public_url.clone();
    let attributed_url = attributed_discovery_url(
        &canonical_url,
        "a2a-opportunity-detail",
        &item.opportunity_id,
    );
    ExecutionOutcome {
        state: A2aTaskState::Completed,
        summary: format!("{} is currently {} with payment state {}. This response is read-only and does not reserve or claim it.", item.title, item.work_state, item.payment_state),
        artifact_name: Some("Agent Bounties opportunity detail".to_string()),
        data: Some(json!({
            "found": true,
            "generatedAt": projection.generated_at,
            "degraded": projection.degraded,
            "opportunity": item,
            "discoveryLinks": {
                "canonicalUrl": canonical_url,
                "attributedUrl": attributed_url,
                "interface": "a2a",
                "campaign": "a2a-opportunity-detail",
                "discoveryId": opportunity_id
            },
            "evidenceBoundary": PAYMENT_BOUNDARY
        })),
    }
}

fn attributed_discovery_url(canonical_url: &str, campaign: &str, discovery_id: &str) -> String {
    let Ok(mut url) = Url::parse(canonical_url) else {
        return canonical_url.to_string();
    };
    url.query_pairs_mut()
        .append_pair("utm_source", "a2a")
        .append_pair("utm_medium", "agent")
        .append_pair("utm_campaign", campaign)
        .append_pair("discovery_id", discovery_id);
    url.to_string()
}

fn protocol_overview() -> ExecutionOutcome {
    ExecutionOutcome {
        state: A2aTaskState::Completed,
        summary: "Agent Bounties exposes separate A2A, MCP, REST, feed, and portable-skill interfaces. Discovery is not funding, a claim, verification, settlement, or payment proof.".to_string(),
        artifact_name: Some("Agent Bounties protocol orientation".to_string()),
        data: Some(json!({
            "a2a": {
                "canonicalUrl": "https://api.agentbounties.app/.well-known/agent-card.json",
                "attributedUrl": attributed_discovery_url("https://api.agentbounties.app/.well-known/agent-card.json", "a2a-protocol-orientation", "agent-card")
            },
            "openapi": "https://api.agentbounties.app/api-docs/openapi.json",
            "mcp": "https://mcp.agentbounties.app/mcp",
            "opportunityFeed": "https://api.agentbounties.app/v1/opportunities/feed.json",
            "agentGuide": "https://agentbounties.app/agent/index.md",
            "source": "https://github.com/NSPG13/agent-bounties",
            "evidenceBoundary": PAYMENT_BOUNDARY,
            "walletSafety": "Never send a private key or recovery phrase. Ask the responsible person before every wallet signature."
        })),
    }
}

fn alert_overview() -> ExecutionOutcome {
    ExecutionOutcome {
        state: A2aTaskState::Completed,
        summary: "Use a signed discovery webhook for low-latency alerts or conditionally poll the JSON Feed with ETag/Last-Modified. Deduplicate IDs and recheck economics and canonical state before work.".to_string(),
        artifact_name: Some("Agent Bounties alert options".to_string()),
        data: Some(json!({
            "signedWebhooks": {
                "documentation": "https://github.com/NSPG13/agent-bounties/blob/main/docs/discovery-subscriptions.md",
                "createEndpoint": "https://api.agentbounties.app/v1/discovery/subscriptions",
                "verify": ["HMAC signature", "timestamp tolerance", "event ID deduplication"]
            },
            "conditionalFeed": {
                "canonicalUrl": "https://api.agentbounties.app/v1/opportunities/feed.json?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&work_state=claimable&payment_state=escrowed",
                "attributedUrl": attributed_discovery_url("https://api.agentbounties.app/v1/opportunities/feed.json?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&work_state=claimable&payment_state=escrowed", "a2a-alerts", "ready-to-earn-feed"),
                "validators": ["ETag", "Last-Modified"],
                "suggestedIntervalSeconds": [300, 900]
            },
            "preWorkGate": ["refetch opportunity", "require claimable work", "require committed payment", "subtract required spend, gas, proof fees, and compute", "obtain human approval before wallet signatures"]
        })),
    }
}

fn task_from_outcome(
    user_message: A2aMessage,
    context_id: String,
    outcome: ExecutionOutcome,
) -> A2aTask {
    let reply = agent_message(&outcome.summary);
    let artifacts =
        outcome
            .artifact_name
            .zip(outcome.data)
            .map_or_else(Vec::new, |(name, data)| {
                vec![A2aArtifact {
                    artifact_id: Uuid::new_v4().to_string(),
                    name,
                    parts: vec![
                        A2aPart {
                            text: Some(outcome.summary.clone()),
                            data: None,
                            metadata: None,
                        },
                        A2aPart {
                            text: None,
                            data: Some(data),
                            metadata: Some(json!({"mediaType": "application/json"})),
                        },
                    ],
                    metadata: Some(json!({"readOnly": true})),
                }]
            });
    A2aTask {
        id: Uuid::new_v4().to_string(),
        context_id,
        status: A2aTaskStatus {
            state: outcome.state,
            message: Some(reply.clone()),
            timestamp: a2a_timestamp(Utc::now()),
        },
        artifacts: Some(artifacts),
        history: vec![user_message, reply],
        metadata: Some(json!({"readOnly": true, "evidenceBoundary": PAYMENT_BOUNDARY})),
    }
}

fn agent_message(text: &str) -> A2aMessage {
    A2aMessage {
        message_id: Uuid::new_v4().to_string(),
        context_id: None,
        task_id: None,
        role: "ROLE_AGENT".to_string(),
        parts: vec![A2aPart {
            text: Some(text.to_string()),
            data: None,
            metadata: None,
        }],
        metadata: None,
    }
}

fn with_history_length(mut task: A2aTask, history_length: Option<usize>) -> A2aTask {
    if let Some(length) = history_length {
        let keep_from = task.history.len().saturating_sub(length);
        task.history = task.history.split_off(keep_from);
    }
    task
}

fn list_task(mut task: A2aTask, history_length: Option<usize>, include_artifacts: bool) -> A2aTask {
    task = with_history_length(task, history_length);
    if !include_artifacts {
        task.artifacts = None;
    }
    task
}

fn encode_page_token(task: &A2aTask) -> String {
    format!(
        "{}.{}",
        hex::encode(task.status.timestamp.as_bytes()),
        task.id
    )
}

fn decode_page_token(token: &str) -> Option<(String, String)> {
    let (encoded_timestamp, id) = token.split_once('.')?;
    if id.is_empty() {
        return None;
    }
    let timestamp = String::from_utf8(hex::decode(encoded_timestamp).ok()?).ok()?;
    Some((timestamp, id.to_string()))
}

fn string_parameter(parameters: &Value, key: &str) -> Option<String> {
    parameters
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn value_as_u128(value: &Value) -> Option<u128> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str()?.parse().ok())
}

fn a2a_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn a2a_json<T: Serialize>(status: StatusCode, value: &T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(A2A_MEDIA_TYPE),
    );
    response.headers_mut().insert(
        "a2a-version",
        HeaderValue::from_static(A2A_PROTOCOL_VERSION),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn task_not_found(id: &str) -> Response {
    let mut metadata = BTreeMap::new();
    metadata.insert("taskId".to_string(), id.to_string());
    metadata.insert("timestamp".to_string(), a2a_timestamp(Utc::now()));
    a2a_protocol_error(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        "TASK_NOT_FOUND",
        &format!(
            "No retained A2A task matched {id:?}. Tasks are bounded and expire after 24 hours."
        ),
        metadata,
    )
}

fn a2a_protocol_error(
    status: StatusCode,
    status_name: &str,
    reason: &str,
    message: &str,
    metadata: BTreeMap<String, String>,
) -> Response {
    let body = A2aHttpErrorEnvelope {
        error: A2aHttpError {
            code: status.as_u16(),
            status: status_name.to_string(),
            message: message.to_string(),
            details: vec![A2aErrorInfo {
                type_url: "type.googleapis.com/google.rpc.ErrorInfo".to_string(),
                reason: reason.to_string(),
                domain: "a2a-protocol.org".to_string(),
                metadata,
            }],
        },
    };
    a2a_json(status, &body)
}

fn problem(status: StatusCode, kind: &str, title: &str, detail: &str) -> Response {
    problem_value(status, kind, title, detail.to_string(), Vec::new())
}

fn problem_with_versions(status: StatusCode, kind: &str, title: &str, detail: String) -> Response {
    problem_value(
        status,
        kind,
        title,
        detail,
        vec![A2A_PROTOCOL_VERSION.to_string()],
    )
}

fn problem_value(
    status: StatusCode,
    kind: &str,
    title: &str,
    detail: String,
    supported_versions: Vec<String>,
) -> Response {
    let body = A2aProblem {
        r#type: format!("https://a2a-protocol.org/errors/{kind}"),
        title: title.to_string(),
        status: status.as_u16(),
        detail,
        supported_versions,
    };
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response.headers_mut().insert(
        "a2a-version",
        HeaderValue::from_static(A2A_PROTOCOL_VERSION),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_message(text: &str) -> A2aMessage {
        A2aMessage {
            message_id: "message-1".to_string(),
            context_id: None,
            task_id: None,
            role: "ROLE_USER".to_string(),
            parts: vec![A2aPart {
                text: Some(text.to_string()),
                data: None,
                metadata: None,
            }],
            metadata: None,
        }
    }

    #[test]
    fn bundled_agent_card_declares_only_implemented_capabilities() {
        let card: A2aAgentCard = serde_json::from_str(AGENT_CARD_JSON).unwrap();
        assert_eq!(card.supported_interfaces.len(), 1);
        assert_eq!(card.supported_interfaces[0].protocol_binding, "HTTP+JSON");
        assert_eq!(card.supported_interfaces[0].protocol_version, "1.0");
        assert!(!card.capabilities.streaming);
        assert!(!card.capabilities.push_notifications);
        assert!(!card.capabilities.extended_agent_card);
        assert_eq!(card.skills.len(), 4);
    }

    #[test]
    fn text_routing_is_deterministic_and_alert_aware() {
        assert_eq!(
            invocation_from_message(&text_message("Find funded bounties")).skill,
            "discover-ready-to-earn-bounties"
        );
        assert_eq!(
            invocation_from_message(&text_message("How do I stay informed about new work?")).skill,
            "explain-bounty-alerts"
        );
        assert_eq!(
            invocation_from_message(&text_message("Explain the protocol")).skill,
            "explain-agent-bounties-protocol"
        );
    }

    #[test]
    fn attributed_links_keep_the_canonical_target_and_exact_discovery_id() {
        let canonical = "https://agentbounties.app/?bounty=0xabc";
        let discovery_id = "eip155:8453:agent-bounties/autonomous-v1:0xabc";
        let attributed =
            attributed_discovery_url(canonical, "a2a-opportunity-detail", discovery_id);
        let parsed = Url::parse(&attributed).unwrap();
        assert_eq!(parsed.scheme(), "https");
        assert_eq!(parsed.host_str(), Some("agentbounties.app"));
        let query = parsed.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query.get("bounty").map(|value| value.as_ref()),
            Some("0xabc")
        );
        assert_eq!(
            query.get("utm_source").map(|value| value.as_ref()),
            Some("a2a")
        );
        assert_eq!(
            query.get("utm_campaign").map(|value| value.as_ref()),
            Some("a2a-opportunity-detail")
        );
        assert_eq!(
            query.get("discovery_id").map(|value| value.as_ref()),
            Some(discovery_id)
        );
    }

    #[test]
    fn task_store_indexes_messages_idempotently() {
        let mut store = A2aTaskStore::default();
        let outcome = protocol_overview();
        let task = task_from_outcome(
            text_message("Explain the protocol"),
            "context-1".to_string(),
            outcome,
        );
        let task_id = task.id.clone();
        store.insert(task, "message-1".to_string(), "fingerprint".to_string());
        assert_eq!(store.task_by_message_id["message-1"], task_id);
        assert_eq!(store.tasks.len(), 1);
    }

    #[test]
    fn history_length_keeps_the_most_recent_messages() {
        let task = task_from_outcome(
            text_message("Explain the protocol"),
            "context-1".to_string(),
            protocol_overview(),
        );
        let trimmed = with_history_length(task, Some(1));
        assert_eq!(trimmed.history.len(), 1);
        assert_eq!(trimmed.history[0].role, "ROLE_AGENT");
    }

    #[test]
    fn list_projection_uses_cursor_and_hides_artifacts_by_default() {
        let task = task_from_outcome(
            text_message("Explain the protocol"),
            "context-1".to_string(),
            protocol_overview(),
        );
        assert!(!task.artifacts.as_ref().unwrap().is_empty());
        let token = encode_page_token(&task);
        assert_eq!(
            decode_page_token(&token),
            Some((task.status.timestamp.clone(), task.id.clone()))
        );
        assert!(list_task(task.clone(), None, false).artifacts.is_none());
        assert_eq!(
            list_task(task, None, true)
                .artifacts
                .as_ref()
                .unwrap()
                .len(),
            1
        );
    }
}
