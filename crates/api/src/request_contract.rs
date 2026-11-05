//! Correctable request errors, without echoing caller credentials or payloads.
use axum::{
    body::{to_bytes, Body},
    extract::{MatchedPath, Request},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct RequestProblem {
    schema_version: &'static str,
    error_code: &'static str,
    message: &'static str,
    retryable: bool,
    field: Option<String>,
    allowed_values: Vec<&'static str>,
    next_action: String,
    request_schema_url: &'static str,
    operation: String,
}

fn field_from_rejection(text: &str) -> Option<String> {
    // Only copy a bounded identifier, never a rejected value or a JSON excerpt.
    ["missing field `", "unknown field `"]
        .iter()
        .find_map(|prefix| {
            let field = text.split_once(prefix)?.1.split_once('`')?.0;
            (!field.is_empty()
                && field.len() <= 80
                && field
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .then(|| field.to_string())
        })
}

fn opportunity_correction(query: &str) -> Option<(String, Vec<&'static str>)> {
    for (field, value) in url::form_urlencoded::parse(query.as_bytes()) {
        let allowed: &[&str] = match field.as_ref() {
            "network" if chain_base::base_network_descriptor(&value).is_err() => {
                &["base-mainnet", "base-sepolia"]
            }
            "view" if crate::opportunities::OpportunityView::parse(Some(&value)).is_err() => &[
                "recent",
                "engineering",
                "creative",
                "urgent",
                "seeking_funding",
                "ready_to_earn",
            ],
            "source_type" => &["unfunded_offchain", "legacy_bounty", "canonical_base"],
            "work_state" => &["open", "claimable", "in_progress", "submitted", "completed"],
            "payment_state" => &["none", "seeking_funding", "escrowed", "paid"],
            "limit" if value.parse::<u32>().is_err() => return Some((field.into_owned(), vec![])),
            _ => continue,
        };
        if !allowed.contains(&value.as_ref()) {
            return Some((field.into_owned(), allowed.to_vec()));
        }
    }
    None
}

pub async fn corrective_errors(request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or(request.uri().path())
        .to_string();
    let operation = format!("{} {route}", request.method());
    let query = request.uri().query().unwrap_or("").to_string();
    let response = next.run(request).await;
    let status = response.status();
    if !route.starts_with("/v1/")
        || !matches!(
            status,
            StatusCode::BAD_REQUEST
                | StatusCode::UNPROCESSABLE_ENTITY
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
        )
        || response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|v| v.to_str().unwrap_or("").starts_with("application/json"))
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = to_bytes(body, 16_384).await.unwrap_or_default();
    let (field, allowed_values) = if route == "/v1/opportunities" {
        opportunity_correction(&query)
            .map(|(f, a)| (Some(f), a))
            .unwrap_or((
                field_from_rejection(&String::from_utf8_lossy(&bytes)),
                vec![],
            ))
    } else {
        (
            field_from_rejection(&String::from_utf8_lossy(&bytes)),
            vec![],
        )
    };
    let problem = RequestProblem {
        schema_version: "agent-bounties/request-problem-v1",
        error_code: "invalid_request",
        message: "The request does not match this operation's input contract.",
        retryable: false,
        next_action: if status == StatusCode::UNSUPPORTED_MEDIA_TYPE {
            "Send the request as application/json using the published request schema.".into()
        } else if let Some(field) = &field {
            format!("Correct the {field} field using allowed_values and the published request schema, then submit the corrected request.")
        } else {
            "Check this operation's path, query parameters, and JSON body against the published request schema, then submit the corrected request.".into()
        },
        field,
        allowed_values,
        request_schema_url: "/api-docs/openapi.json",
        operation,
    };
    let json = Json(problem).into_response();
    // Keep security, CORS, and cache headers from the original response.
    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
    Response::from_parts(parts, Body::new(json.into_body()))
}

pub fn document(mut document: utoipa::openapi::OpenApi) -> utoipa::openapi::OpenApi {
    use utoipa::openapi::{response::Response, Content, Ref, RefOr};
    // domain::Id is the public UUID alias used by shared request/response types.
    document.components.as_mut().unwrap().schemas.insert(
        "Id".into(),
        utoipa::openapi::schema::ObjectBuilder::new()
            .schema_type(utoipa::openapi::SchemaType::String)
            .format(Some(utoipa::openapi::SchemaFormat::KnownFormat(
                utoipa::openapi::KnownFormat::Uuid,
            )))
            .into(),
    );
    for (path, item) in &mut document.paths.paths {
        if !path.starts_with("/v1/") {
            continue;
        }
        for operation in item.operations.values_mut() {
            if [
                "/v1/cloud-agent/objective-plans",
                "/v1/cloud-agent/bounty-drafts",
                "/v1/base/autonomous-bounties/{bounty_contract}/analysis",
            ]
            .contains(&path.as_str())
            {
                operation.deprecated = Some(utoipa::openapi::Deprecated::True);
            }
            for status in ["400", "415", "422"] {
                let response = operation
                    .responses
                    .responses
                    .entry(status.to_string())
                    .or_insert_with(|| {
                        RefOr::T(Response::new(
                            "Correct the indicated request field before retrying.",
                        ))
                    });
                if let RefOr::T(response) = response {
                    if response.content.is_empty() {
                        // Preserve the documented possibility of existing domain-specific
                        // JSON errors; middleware only converts empty/plain rejections.
                        response.content.insert("application/json".into(), Content::new(
                            utoipa::openapi::Schema::AnyOf(utoipa::openapi::schema::AnyOfBuilder::new()
                                .item(Ref::from_schema_name("RequestProblem"))
                                .item(utoipa::openapi::schema::ObjectBuilder::new()
                                    .description(Some("Operation-specific structured error; preserve its error code and next action.")))
                                .build())));
                    }
                }
            }
        }
    }
    document
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        routing::{get, post},
        Router,
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        network: String,
    }

    #[tokio::test]
    async fn invalid_requests_are_correctable_and_existing_json_is_preserved() {
        let app = Router::new()
            .route(
                "/v1/readiness",
                post(|Json(input): Json<Input>| async move { input.network }),
            )
            .route(
                "/v1/opportunities",
                get(|| async { StatusCode::BAD_REQUEST }),
            )
            .route(
                "/v1/existing",
                get(|| async {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error_code":"specific"})),
                    )
                }),
            )
            .layer(axum::middleware::from_fn(corrective_errors));
        for (uri, body, field, code) in [
            ("/v1/readiness", Some("{}"), Some("network"), 422),
            ("/v1/readiness", Some("{"), None, 400),
            ("/v1/opportunities?view=not_a_view", None, Some("view"), 400),
            ("/v1/opportunities?limit=oops", None, Some("limit"), 400),
        ] {
            let request = Request::builder()
                .uri(uri)
                .method(if body.is_some() { "POST" } else { "GET" })
                .header("content-type", "application/json")
                .body(Body::from(body.unwrap_or("")))
                .unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), code);
            let json: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 16_384).await.unwrap())
                    .unwrap();
            assert_eq!(json["field"].as_str(), field);
            assert_eq!(json["retryable"], false);
            assert!(json["next_action"].as_str().unwrap().len() > 30);
            if field == Some("view") {
                assert!(json["allowed_values"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("ready_to_earn")));
            }
        }
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/existing")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let json: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16_384).await.unwrap()).unwrap();
        assert_eq!(json, serde_json::json!({"error_code":"specific"}));
    }

    #[test]
    fn rejection_fields_cannot_echo_secrets_and_supported_aliases_are_accepted() {
        assert_eq!(
            field_from_rejection("missing field `network` at line 1"),
            Some("network".into())
        );
        assert_eq!(
            field_from_rejection("unknown field `Bearer secret: token`"),
            None
        );
        assert_eq!(opportunity_correction("network=8453&view=RECENT"), None);
    }
}
