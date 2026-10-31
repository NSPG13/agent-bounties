mod diagnostics_tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        routing::{get, post},
    };
    use tower::ServiceExt;

    fn routes(state: SharedState) -> Router {
        Router::new()
            .route("/v1/opportunities", get(list_opportunities))
            .route(
                "/v1/discovery/subscriptions",
                post(create_discovery_subscription),
            )
            .with_state(state)
    }

    async fn assert_empty_response(app: Router, request: Request<Body>, status: StatusCode) {
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), status);
        assert!(to_bytes(response.into_body(), 4096)
            .await
            .unwrap()
            .is_empty());
    }

    async fn assert_canonical_unavailable(app: Router, request: Request<Body>, protocol: &str) {
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(value["coverage_scope"], "canonical_identity");
        assert_eq!(value["coverage"], "unavailable");
        assert_eq!(value["degraded"], true);
        assert_eq!(value["items"], serde_json::json!([]));
        let sources = value["source_statuses"].as_array().unwrap();
        assert_eq!(sources.len(), 1);
        let protocols = sources[0]["protocol_statuses"].as_array().unwrap();
        assert_eq!(protocols.len(), 1);
        assert_eq!(protocols[0]["protocol"], protocol);
        assert_eq!(protocols[0]["coverage"], "unavailable");
        assert_eq!(protocols[0]["item_count"], serde_json::Value::Null);
        assert_eq!(protocols[0]["error"], "canonical_identity_read_unavailable");
    }

    #[tokio::test]
    async fn diagnostics_canonical_missing_store_keeps_unavailable_projection() {
        let request = Request::builder().uri(
            "/v1/opportunities?opportunity_id=canonical:base-mainnet:0x1111111111111111111111111111111111111111"
        ).body(Body::empty()).unwrap();
        assert_canonical_unavailable(
            routes(test_state(BountyNetwork::default())),
            request,
            "autonomous-v1",
        )
        .await;
    }

    #[tokio::test]
    async fn diagnostics_bad_scope_stays_bad_request() {
        for id in [
            "canonical:base-mainnet:invalid",
            "canonical:base-sepolia:0x1111111111111111111111111111111111111111",
        ] {
            let request = Request::builder()
                .uri(format!("/v1/opportunities?opportunity_id={id}"))
                .body(Body::empty())
                .unwrap();
            assert_empty_response(
                routes(test_state(BountyNetwork::default())),
                request,
                StatusCode::BAD_REQUEST,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn diagnostics_poisoned_legacy_lock_keeps_other_source_reports() {
        let state = test_state(BountyNetwork::default());
        let network = state.network.clone();
        assert!(std::thread::spawn(move || {
            let _guard = network.lock().unwrap();
            panic!("synthetic poisoned state; no user data");
        })
        .join()
        .is_err());
        let response = routes(state)
            .oneshot(
                Request::builder()
                    .uri("/v1/opportunities")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(value["degraded"], true);
        let sources = value["source_statuses"].as_array().unwrap();
        assert_eq!(sources.len(), 3);
        let legacy = sources
            .iter()
            .find(|item| item["source_type"] == "legacy_bounty")
            .unwrap();
        assert_eq!(legacy["available"], false);
        assert_eq!(legacy["coverage"], "unavailable");
        assert_eq!(legacy["error"], "legacy_read_model_unavailable");
        assert!(sources
            .iter()
            .any(|item| item["source_type"] == "unfunded_offchain"));
        assert!(sources
            .iter()
            .any(|item| item["source_type"] == "canonical_base"));
    }

    #[tokio::test]
    async fn diagnostics_missing_signing_keeps_empty_503_without_echoing_endpoint() {
        let request = Request::builder().method("POST").uri("/v1/discovery/subscriptions")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"endpoint_url":"https://private.example.invalid/webhook?token=private-marker"}"#)).unwrap();
        assert_empty_response(
            routes(test_state(BountyNetwork::default())),
            request,
            StatusCode::SERVICE_UNAVAILABLE,
        )
        .await;
    }

    #[tokio::test]
    async fn diagnostics_timeout_lookup_and_missing_config_stay_unavailable_without_signing() {
        let state = test_state(BountyNetwork::default());
        assert!(state.x402_relayer.relayer.is_none());
        assert_eq!(timeout_relayer(&state.x402_relayer).unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(timeout_rpc_config(&state.base_rpc_urls, "base-mainnet").unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        let app = Router::new().route("/v1/base/autonomous-bounties/timeout-relay", post(relay_autonomous_timeout)).with_state(state);
        let request = Request::builder().method("POST").uri("/v1/base/autonomous-bounties/timeout-relay?private=private-marker")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"round":1,"action":"expire_claim","network":"base-mainnet","bounty_contract":"0x1111111111111111111111111111111111111111"}"#)).unwrap();
        assert_empty_response(app, request, StatusCode::SERVICE_UNAVAILABLE).await;
    }

    #[tokio::test]
    async fn diagnostics_timeout_cannot_infer_whether_provider_observed_request() {
        use std::sync::atomic::{AtomicBool, Ordering};
        // No signer or transaction exists. These futures model a timeout before
        // versus after the provider observes a request; both remain ambiguous.
        for provider_observed in [false, true] {
            let observed = Arc::new(AtomicBool::new(false));
            let signal = observed.clone();
            let result = tokio::time::timeout(Duration::from_millis(1), async move {
                signal.store(provider_observed, Ordering::SeqCst);
                std::future::pending::<Result<BaseRelayedTransaction, ChainBaseError>>().await
            }).await;
            assert_eq!(timeout_execution_result(result).unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(observed.load(Ordering::SeqCst), provider_observed);
        }
        for (error, status) in [
            (ChainBaseError::RelayerProvider("https://private-marker/?token=private-query-marker".into()), StatusCode::SERVICE_UNAVAILABLE),
            (ChainBaseError::RelayerProvider("revert private-marker".into()), StatusCode::CONFLICT),
            (ChainBaseError::InvalidRelayIntent("private-marker".into()), StatusCode::UNPROCESSABLE_ENTITY),
        ] {
            assert_eq!(timeout_execution_result::<BaseRelayedTransaction>(Ok(Err(error))).unwrap_err(), status);
        }
    }

    #[tokio::test]
    async fn diagnostics_timeout_receipt_read_failure_keeps_known_hash_and_no_confirmation() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route("/", post(|Json(request): Json<serde_json::Value>| async move {
            assert_eq!(request["method"], "eth_getTransactionReceipt");
            (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "30")], "private-response-marker")
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let transaction = BaseRelayedTransaction {
            relayer: "0x2222222222222222222222222222222222222222".into(),
            tx_hash: format!("0x{}", "ab".repeat(32)), estimated_gas: 1, gas_limit: 1,
            max_fee_per_gas_wei: 1, max_priority_fee_per_gas_wei: 1, estimated_max_cost_wei: 1,
        };
        let saved = timeout_relay::test_saved();
        let confirmation = wait_for_timeout_confirmation(&format!("http://{address}/?private=private-query-marker"),
            &saved.scope, &transaction, 1, 1).await;
        server.abort();
        assert!(confirmation.is_err());
        let response = timeout_response_after_broadcast("base-mainnet", "0x1111111111111111111111111111111111111111".into(),
            AutonomousTimeoutAction::ExpireClaim, &saved.scope, &saved, timeout_confirmation_result(confirmation));
        assert_eq!(response.transaction_hash, transaction.tx_hash);
        assert!(!response.confirmed);
        assert!(response.confirmed_block.is_none() && response.canonical_event_id.is_none());
        assert!(response.evidence_boundary.contains("before any retry"));
    }

    #[tokio::test]
    #[ignore = "isolated diagnostics fixture process only"]
    async fn diagnostics_timeout_lease_busy_fixture() {
        assert_eq!(env::var("AGENT_BOUNTIES_DIAGNOSTIC_FIXTURE").as_deref(), Ok("local-only"));
        let url = env::var("AGENT_BOUNTIES_DIAGNOSTIC_DATABASE_URL").unwrap();
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert_eq!(parsed.path(), "/diagnostics_empty");
        let store = PostgresStore::connect(&url).await.unwrap();
        // Earlier fixtures deliberately use the empty schema. Migrate only in
        // this final fixture; no key, signing, RPC or broadcast is configured.
        store.migrate().await.unwrap();
        let network = "diagnostic-timeout-lease";
        let first = timeout_lease_result(store.acquire_x402_relayer_lease(network, 30).await).unwrap();
        let busy = store.acquire_x402_relayer_lease(network, 30).await;
        assert_eq!(timeout_lease_result(busy).unwrap_err(), StatusCode::SERVICE_UNAVAILABLE);
        store.release_x402_relayer_lease(network, first).await.unwrap();
        let next = timeout_lease_result(store.acquire_x402_relayer_lease(network, 30).await).unwrap();
        store.release_x402_relayer_lease(network, next).await.unwrap();
    }

    // A separate process supplies fixture-only manifests and a synthetic signing key.
    // Keeping environment setup outside the test prevents races with other unit tests.
    #[tokio::test]
    #[ignore = "isolated diagnostics fixture process only"]
    async fn diagnostics_configured_failure_fixtures() {
        assert_eq!(
            env::var("AGENT_BOUNTIES_DIAGNOSTIC_FIXTURE").as_deref(),
            Ok("local-only")
        );
        for (protocol, expected) in [
            ("open-competition", "open-competition-v1"),
            ("open-competition-v2", "open-competition-v2-beta3"),
        ] {
            let request = Request::builder().uri(format!("/v1/opportunities?opportunity_id={protocol}:base-mainnet:0x1111111111111111111111111111111111111111"))
                .body(Body::empty()).unwrap();
            assert_canonical_unavailable(
                routes(test_state(BountyNetwork::default())),
                request,
                expected,
            )
            .await;
        }
        let mut state = test_state(BountyNetwork::default());
        Arc::make_mut(&mut state).discovery_webhooks =
            DiscoveryWebhookConfig::from_env().unwrap().map(Arc::new);
        assert!(state.discovery_webhooks.is_some());
        let request = Request::builder()
            .method("POST")
            .uri("/v1/discovery/subscriptions")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"endpoint_url":"https://8.8.8.8/fixture?private-marker"}"#,
            ))
            .unwrap();
        assert_empty_response(routes(state), request, StatusCode::SERVICE_UNAVAILABLE).await;

        if let Ok(url) = env::var("AGENT_BOUNTIES_DIAGNOSTIC_DATABASE_URL") {
            let parsed = url::Url::parse(&url).unwrap();
            assert_eq!(parsed.host_str(), Some("127.0.0.1"));
            assert_eq!(parsed.path(), "/diagnostics_empty");
            let store = PostgresStore::connect(&url).await.unwrap();
            let mut state = test_state(BountyNetwork::default());
            Arc::make_mut(&mut state).store = Some(store);
            Arc::make_mut(&mut state).discovery_webhooks =
                DiscoveryWebhookConfig::from_env().unwrap().map(Arc::new);
            // Numeric public IP avoids DNS. Validation makes no HTTP call;
            // persistence deliberately fails in the disposable empty database.
            let request = Request::builder()
                .method("POST")
                .uri("/v1/discovery/subscriptions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"endpoint_url":"https://8.8.8.8/fixture?private-marker"}"#,
                ))
                .unwrap();
            assert_empty_response(
                routes(state.clone()),
                request,
                StatusCode::SERVICE_UNAVAILABLE,
            )
            .await;
            let app = open_competition_v2_api::router().with_state(state.clone());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri("/v1/base/open-competition-v2-beta3/inventory")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let problem: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(problem["error_code"], "database_read_failed");
            assert_eq!(problem["failed_transition"], "load_inventory");
            assert_eq!(problem["retryable"], true);
            assert_eq!(
                problem["message"],
                "Canonical V2 inventory is temporarily unavailable. Try again later."
            );
            assert!(!problem.to_string().contains("relation"));
            for (method, path, body, transition) in [
                ("GET", "/v1/base/open-competition-v2-beta3/events", None, "load_events"),
                ("GET", "/v1/base/open-competition-v2-beta3/proof-jobs/00000000-0000-0000-0000-000000000000", None, "get_proof_job"),
                ("POST", "/v1/base/open-competition-v2-beta3/proof-quotes", Some(serde_json::json!({"network":"base-mainnet","competition_contract":"0x1111111111111111111111111111111111111111","solver":"0x2222222222222222222222222222222222222222","solver_nonce":"1","relay":false,"metric":{"mode":"all_equal","threshold":"1","vectors":[]}})), "quote_proof"),
                ("POST", "/v1/base/open-competition-v2-beta3/proof-jobs/00000000-0000-0000-0000-000000000000/relay-authorization", Some(serde_json::json!({"authorization_deadline":1})), "authorize_proof_relay"),
            ] {
                let request = Request::builder().method(method).uri(path).header("content-type", "application/json")
                    .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string()))).unwrap();
                let response = open_competition_v2_api::router().with_state(state.clone()).oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
                let problem: serde_json::Value = serde_json::from_slice(&to_bytes(response.into_body(),4096).await.unwrap()).unwrap();
                assert_eq!(problem["failed_transition"], transition);
                assert_eq!(problem["error_code"], "database_read_failed");
                assert_eq!(problem["retryable"], true);
                assert_eq!(problem["message"], "Canonical V2 data is temporarily unavailable. Try again later.");
                assert!(!problem.to_string().contains("relation"));
            }
            let release =
                open_competition_v2_api::release_from_environment("base-mainnet").unwrap();
            let (status, Json(problem)) = open_competition_v2_api::current_indexer_agreement(
                &state,
                "base-mainnet",
                &release,
            )
            .await
            .unwrap_err();
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(problem["error_code"], "indexer_agreement_read_failed");
            assert_eq!(problem["failed_transition"], "resolve_release");
            assert_eq!(problem["retryable"], true);
            assert_eq!(
                problem["message"],
                "Canonical V2 reader checks are temporarily unavailable. Try again later."
            );
            assert!(!problem.to_string().contains("relation"));
        }
    }
}
