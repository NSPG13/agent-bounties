mod diagnostics_tests {
    use super::*;
    use axum::routing::post;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn raw_fixture(response: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let received = stream.read(&mut request).await.unwrap();
            assert!(received > 0, "fixture client sent no request bytes");
            stream.write_all(response).await.unwrap();
            stream.shutdown().await.unwrap();
        });
        (
            format!("http://{address}/private-marker?token=private-query-marker"),
            task,
        )
    }

    fn assert_private(value: &serde_json::Value) {
        let text = value.to_string();
        for secret in [
            "private-marker",
            "private-query-marker",
            "127.0.0.1",
            "http://",
            "private-response-marker",
        ] {
            assert!(!text.contains(secret), "unexpected private value: {text}");
        }
    }

    #[tokio::test]
    async fn diagnostics_hosted_transport_redacts_request_url_and_query() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let response = proxy_hosted_json(
            reqwest::Client::new()
                .get(format!(
                    "http://{address}/private-marker?token=private-query-marker"
                ))
                .timeout(std::time::Duration::from_secs(2)),
        )
        .await
        .0;
        assert_eq!(
            response["error"],
            "hosted API request failed: request_transport_unavailable"
        );
        assert_private(&response);
    }

    #[tokio::test]
    async fn diagnostics_hosted_timeout_is_a_redacted_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        });
        let response = proxy_hosted_json(
            reqwest::Client::new()
                .get(format!(
                    "http://{address}/private-marker?token=private-query-marker"
                ))
                .timeout(std::time::Duration::from_millis(100)),
        )
        .await
        .0;
        task.abort();
        assert_eq!(
            response["error"],
            "hosted API request failed: request_timeout"
        );
        assert_private(&response);
    }

    #[test]
    fn diagnostics_upstream_status_does_not_reclassify_client_errors() {
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::NOT_FOUND,
            StatusCode::CONFLICT,
            StatusCode::GONE,
        ] {
            let response = hosted_response_error(status, "safe API problem".to_string()).0;
            assert_eq!(response["error"], "safe API problem");
            assert!(response.get("error_code").is_none());
        }
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::OK,
        ] {
            let response = hosted_response_error(status, "safe API problem".to_string()).0;
            assert_eq!(response["error"], "safe API problem");
            assert_eq!(response["error_code"], "upstream_unavailable");
        }
    }

    #[tokio::test]
    async fn diagnostics_hosted_body_and_parse_failures_are_redacted() {
        for (raw, expected) in [
            (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 1000\r\nConnection: close\r\n\r\nprivate-response-marker".as_slice(), "response_body_unavailable"),
            (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 23\r\nConnection: close\r\n\r\nprivate-response-marker".as_slice(), "response_json_invalid"),
            (b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(), "response_json_invalid"),
        ] {
            let (url, task) = raw_fixture(raw).await;
            let response = proxy_hosted_json(reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(2))).await.0;
            task.await.unwrap();
            assert_eq!(response["error"], format!("hosted API returned 503 Service Unavailable: {expected}"));
            assert_private(&response);
        }
    }

    #[tokio::test]
    async fn diagnostics_hosted_preserves_supported_api_problem_and_success() {
        let problem = json!({"schema_version":"agent-bounties/open-competition-v2-problem-v1", "state":"failed", "failed_transition":"load_canonical_state", "error_code":"database_unavailable", "retryable":true, "message":"Canonical V2 persistence is not configured."});
        let fixture = problem.clone();
        let router = axum::Router::new()
            .route(
                "/",
                axum::routing::get(move || {
                    let fixture = fixture.clone();
                    async move { (StatusCode::SERVICE_UNAVAILABLE, Json(fixture)) }
                }),
            )
            .route(
                "/ok",
                axum::routing::get(|| async { Json(json!({"items":[]})) }),
            )
            .route(
                "/empty",
                axum::routing::get(|| async { StatusCode::NO_CONTENT }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let client = reqwest::Client::new();
        let response = proxy_hosted_json(client.get(format!("http://{address}/")))
            .await
            .0;
        assert_eq!(response["error_code"], "upstream_unavailable");
        let encoded = response["error"]
            .as_str()
            .unwrap()
            .strip_prefix("hosted API returned 503 Service Unavailable: ")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(encoded).unwrap(),
            problem
        );
        let success = proxy_hosted_json(client.get(format!("http://{address}/ok")))
            .await
            .0;
        assert_eq!(success.pointer("/content/0/json/items"), Some(&json!([])));
        let empty = proxy_hosted_json(client.get(format!("http://{address}/empty")))
            .await
            .0;
        assert_eq!(
            empty.pointer("/content/0/json/http_status"),
            Some(&json!(204))
        );
        task.abort();
    }

    // Spawned alone with API_UPSTREAM_BASE_URL aimed at a closed loopback port.
    #[tokio::test]
    #[ignore = "isolated diagnostics fixture process only"]
    async fn diagnostics_direct_and_remote_failure_fixture() {
        assert_eq!(
            env::var("AGENT_BOUNTIES_DIAGNOSTIC_FIXTURE").as_deref(),
            Ok("local-only")
        );
        let expected = env::var("AGENT_BOUNTIES_DIAGNOSTIC_EXPECTED_FAILURE")
            .unwrap_or_else(|_| "request_transport_unavailable".to_string());
        let app = axum::Router::new()
            .route("/tools/list_opportunities", post(list_opportunities))
            .route(
                "/tools/create_discovery_subscription",
                post(create_discovery_subscription),
            )
            .route("/mcp", post(chatgpt_app::mcp_post))
            .with_state(test_state());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();
        for (path, body) in [
            ("/tools/list_opportunities", json!({})),
            (
                "/tools/create_discovery_subscription",
                json!({"endpoint_url":"https://example.invalid/private-marker?token=private-query-marker"}),
            ),
        ] {
            let response = client
                .post(format!("http://{address}{path}"))
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let response: serde_json::Value = response.json().await.unwrap();
            assert!(response["error"].as_str().unwrap().contains(&expected));
            assert_eq!(response["error_code"], "upstream_unavailable");
            assert_private(&response);
        }
        let response=client.post(format!("http://{address}/mcp")).json(&json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"get_bounty_feed","arguments":{}}})).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32603);
        assert_eq!(body["error"]["data"]["error_code"], "upstream_unavailable");
        assert!(body.to_string().contains(&expected), "{body}");
        assert_private(&body);
        let response = client.post(format!("http://{address}/mcp"))
            .header("mcp-protocol-version", "2026-07-28")
            .header("mcp-method", "tools/call")
            .header("mcp-name", "get_bounty_feed")
            .json(&json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"get_bounty_feed","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}))
            .send().await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32603);
        assert_eq!(body["error"]["data"]["error_code"], "upstream_unavailable");
        assert!(body.to_string().contains(&expected), "{body}");
        assert_private(&body);
        // A real argument error must keep the protocol's invalid-params code.
        let response = client.post(format!("http://{address}/mcp"))
            .json(&json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"get_bounty_feed","arguments":{"limit":"invalid"}}}))
            .send().await.unwrap();
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], -32602);
        assert!(body["error"].get("data").is_none());
        task.abort();
    }
}
