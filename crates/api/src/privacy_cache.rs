//! Prevent credentials from turning public API URLs into cacheable private responses.
use axum::{
    extract::{MatchedPath, Request},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};

fn may_include_private_data(path: &str) -> bool {
    if path == "/v1/auth"
        || path.starts_with("/v1/auth/")
        || path.starts_with("/v1/managed-service/")
        || (path.starts_with("/v1/verification/") && path != "/v1/verification/checkers")
    {
        return true;
    }
    matches!(
        path,
        "/v1/risk/events"
            | "/v1/risk/reviews"
            | "/v1/bounties/:id"
            | "/v1/agents/:id/paid-status"
            | "/v1/bounties/{id}"
            | "/v1/agents/{id}/paid-status"
    )
}

pub async fn protect_private_reads(request: Request, next: Next) -> Response {
    // Match the route template so encoded IDs cannot bypass the policy and
    // unrelated routes such as /v1/bounties/feed keep their public cache policy.
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| request.uri().path());
    let private = may_include_private_data(path)
        || ["authorization", "cookie", "x-operator-token"]
            .iter()
            .any(|name| request.headers().contains_key(*name))
        || (request.uri().path().starts_with("/v1/")
            && !matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS"));
    let mut response = next.run(request).await;
    if private {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store, private"),
        );
        response.headers_mut().append(
            header::VARY,
            HeaderValue::from_static("Authorization, Cookie, X-Operator-Token"),
        );
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::StatusCode, middleware, routing::get, Router};
    use tower::ServiceExt;

    #[tokio::test]
    async fn private_route_responses_are_never_cached_and_public_assets_keep_their_policy() {
        let app = Router::new()
            .route(
                "/v1/bounties/:id",
                get(|| async {
                    (
                        [
                            (header::CACHE_CONTROL, "public, max-age=60"),
                            (header::VARY, "Origin"),
                        ],
                        "private bounty",
                    )
                }),
            )
            .route(
                "/v1/agents/:id/paid-status",
                get(|| async { "private wallet" }),
            )
            .route(
                "/v1/auth/session",
                get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route(
                "/v1/risk/events",
                get(|| async { StatusCode::UNAUTHORIZED }),
            )
            .route(
                "/v1/risk/reviews",
                get(|| async { "private operator note" }),
            )
            .route(
                "/assets/site.css",
                get(|| async { ([(header::CACHE_CONTROL, "public, max-age=3600")], "body{}") }),
            )
            .route(
                "/v1/bounties/feed",
                get(|| async {
                    (
                        [(header::CACHE_CONTROL, "public, max-age=30")],
                        "public feed",
                    )
                }),
            )
            .layer(middleware::from_fn(protect_private_reads));
        for path in [
            "/v1/bounties/example",
            "/v1/bounties/%65xample",
            "/v1/agents/example/paid-status",
            "/v1/auth/session",
            "/v1/risk/events",
            "/v1/risk/reviews",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store, private"
            );
            let vary = response
                .headers()
                .get_all(header::VARY)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>()
                .join(", ");
            assert!(
                vary.contains("Authorization")
                    && vary.contains("Cookie")
                    && vary.contains("X-Operator-Token")
            );
            if path.starts_with("/v1/bounties/") {
                assert!(vary.contains("Origin"));
            }
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/assets/site.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=3600"
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/bounties/feed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=30"
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/assets/site.css")
                    .header("cookie", "session=synthetic")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, private"
        );
        assert!(!may_include_private_data("/v1/authentication-info"));
        assert!(!may_include_private_data("/v1/bounties/feed/rss"));
    }
}
