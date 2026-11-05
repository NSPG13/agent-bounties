//! Direct MCP writes use the same protected API boundary and only caller credentials.
use axum::{http::HeaderMap, Json};
use serde_json::{json, Value};

pub async fn proxy(headers: HeaderMap, path: &str, body: Value) -> Json<Value> {
    proxy_to(
        headers,
        path,
        body,
        &super::api_upstream_base_url_from_env(),
    )
    .await
}

async fn proxy_to(headers: HeaderMap, path: &str, body: Value, base: &str) -> Json<Value> {
    if let Err(error) = service_runtime::terms_publication::validate_headers(&headers) {
        let (code, status) = if matches!(error, service_runtime::auth::AuthError::Unauthorized) {
            ("publication_wallet_authority_required", 403)
        } else {
            ("invalid_publication_request", 400)
        };
        return Json(
            json!({"error":"Use one caller credential and key; operator credentials cannot publish for a wallet.","error_code":code,"http_status":status}),
        );
    }
    if !headers.contains_key("authorization") && !headers.contains_key("cookie") {
        return Json(
            json!({"error":"Sign in with a verified creator wallet before publishing terms.","error_code":"publication_authentication_required","http_status":401}),
        );
    }
    if service_runtime::terms_publication::idempotency_key(&headers).is_err() {
        return Json(
            json!({"error":"Supply one stable Idempotency-Key of at most 200 characters.","error_code":"invalid_publication_request","http_status":400}),
        );
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("HTTP client");
    let mut request = client
        .post(format!("{}{path}", base.trim_end_matches('/')))
        .json(&body);
    for name in service_runtime::terms_publication::SINGULAR_HEADERS {
        if *name != "x-operator-token" {
            if let Some(value) = headers.get(*name) {
                request = request.header(*name, value);
            }
        }
    }
    let unavailable = || {
        Json(
            json!({"error":"Publication could not be confirmed. Retry the same key and content before any wallet action.","error_code":"publication_upstream_unavailable","http_status":503,"retryable":true}),
        )
    };
    let response = match request.send().await {
        Ok(response) => response,
        Err(_) => return unavailable(),
    };
    let status = response.status();
    // Redirects must never send caller credentials to another origin.
    if status.is_redirection() {
        return unavailable();
    }
    let body: Value = match response.json().await {
        Ok(body) => body,
        Err(_) => return unavailable(),
    };
    if status.is_success() {
        return super::mcp_json(body);
    }
    if body["schema_version"] != "agent-bounties/action-error-v1" {
        return unavailable();
    }
    let mut output = json!({"error":body["message"],"http_status":status.as_u16()});
    for name in [
        "schema_version",
        "error_code",
        "message",
        "retryable",
        "next_action",
        "next_action_url",
    ] {
        if let Some(value) = body.get(name) {
            output[name] = value.clone();
        }
    }
    Json(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Router};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    #[tokio::test]
    async fn direct_publication_proxy_preserves_caller_and_rejects_ambiguity() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let router=Router::new()
            .route("/exists",post(|| async { (axum::http::StatusCode::CONFLICT,Json(json!({"schema_version":"agent-bounties/action-error-v1","error_code":"publication_already_exists","message":"Already published","next_action_url":"/v1/base/autonomous-bounties/terms/0x123","retryable":false,"secret_debug":"must not propagate"}))) }))
            .route("/redirect",post(|| async { axum::response::Redirect::temporary("/terms") }))
            .route("/terms",post(move |headers:HeaderMap, Json(body):Json<Value>| {
            observed.fetch_add(1,Ordering::SeqCst);
            async move { Json(json!({"authorization":headers.get("authorization").unwrap().to_str().unwrap(),"key":headers.get("idempotency-key").unwrap().to_str().unwrap(),"operator":headers.contains_key("x-operator-token"),"body":body})) }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer caller-only".parse().unwrap());
        headers.insert("idempotency-key", "stable".parse().unwrap());
        let result = proxy_to(headers.clone(), "/terms", json!({"test":true}), &base)
            .await
            .0;
        assert_eq!(
            result["content"][0]["json"]["authorization"],
            "Bearer caller-only"
        );
        assert_eq!(result["content"][0]["json"]["key"], "stable");
        assert_eq!(result["content"][0]["json"]["operator"], false);
        let conflict = proxy_to(headers.clone(), "/exists", json!({}), &base)
            .await
            .0;
        assert_eq!(conflict["error_code"], "publication_already_exists");
        assert_eq!(conflict["http_status"], 409);
        assert_eq!(
            conflict["next_action_url"],
            "/v1/base/autonomous-bounties/terms/0x123"
        );
        assert!(conflict.get("secret_debug").is_none());
        assert_eq!(
            proxy_to(headers.clone(), "/redirect", json!({}), &base)
                .await
                .0["http_status"],
            503
        );
        for name in service_runtime::terms_publication::SINGULAR_HEADERS {
            let mut duplicate = headers.clone();
            duplicate.append(*name, "one".parse().unwrap());
            duplicate.append(*name, "two".parse().unwrap());
            assert!(proxy_to(duplicate, "/terms", json!({}), &base).await.0["error"].is_string());
        }
        assert!(proxy_to(HeaderMap::new(), "/terms", json!({}), &base)
            .await
            .0["error"]
            .is_string());
        headers.insert("x-operator-token", "operator-only".parse().unwrap());
        assert!(proxy_to(headers, "/terms", json!({}), &base).await.0["error"].is_string());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
    }
}
