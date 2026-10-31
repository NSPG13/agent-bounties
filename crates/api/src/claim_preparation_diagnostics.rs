//! Exercise real HTTP handler failures in isolated processes without credentials or a database.
#[cfg(test)]
mod tests {
    use crate::*;
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    const TEST: &str = "claim_preparation_diagnostics::tests::actual_handler_lookup_failures_emit_only_fixed_stages";
    const MODE: &str = "AGENT_BOUNTIES_CLAIM_DIAGNOSTIC_TEST_MODE";

    #[test]
    fn actual_handler_lookup_failures_emit_only_fixed_stages() {
        if let Ok(mode) = std::env::var(MODE) {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(exercise_handlers(&mode));
            return;
        }
        // A process per configuration avoids environment races with unrelated tests.
        for mode in ["competition", "indexed", "bad_input"] {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .env_clear()
                .env(MODE, mode)
                .args(["--exact", TEST, "--nocapture"]);
            if mode == "competition" {
                // Synthetic parsing fixture only: no deployed manifest or live RPC is used.
                let manifest = json!({
                    "protocol_version":"agent-bounties/open-competition-v1", "network":"base-mainnet",
                    "chain_id":8453, "deployment_state":"active_ready_to_earn",
                    "factory_contract":"0x2222222222222222222222222222222222222222",
                    "implementation_contract":"0x3333333333333333333333333333333333333333",
                    "settlement_token":"0x4444444444444444444444444444444444444444",
                    "deployment_block":1, "factory_runtime_code_hash":format!("0x{}", "00".repeat(32)),
                    "implementation_runtime_code_hash":format!("0x{}", "00".repeat(32))
                });
                serde_json::from_value::<OpenCompetitionReleaseManifest>(manifest.clone()).unwrap();
                child.env(
                    "BASE_MAINNET_OPEN_COMPETITION_V1_RELEASE_MANIFEST_JSON",
                    manifest.to_string(),
                );
            }
            let output = child.output().unwrap();
            let stderr = String::from_utf8(output.stderr).unwrap();
            assert!(
                output.status.success(),
                "isolated handler test failed: {stderr}"
            );
            assert!(!stderr.contains("private-marker"));
            assert!(!stderr.contains("0x1111111111111111111111111111111111111111"));
            let records: Vec<Value> = stderr
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| event["event"] == "service_failure_stage")
                .collect();
            if mode == "bad_input" {
                assert!(
                    records.is_empty(),
                    "caller errors are not dependency outages"
                );
                continue;
            }
            assert_eq!(records.len(), 2, "{stderr}");
            let suffix = if mode == "competition" {
                "competition_lookup"
            } else {
                "indexed_lookup"
            };
            for (event, action) in records.iter().zip(["claim_plan", "authorized_claim_plan"]) {
                assert_eq!(event["stage"], format!("{action}_{suffix}"));
                assert_eq!(event["action"], action);
                assert_eq!(event["component"], "api");
                assert_eq!(event.as_object().unwrap().len(), 9);
                assert!(event.get("request_id").is_none());
                assert!(event.get("bounty_contract").is_none());
                assert!(event.get("error").is_none());
            }
        }
    }

    async fn exercise_handlers(mode: &str) {
        let state = crate::tests::test_state(BountyNetwork::default());
        assert!(state.store.is_none());
        assert!(state.x402_relayer.relayer.is_none());
        let app = Router::new()
            .route("/claim", post(plan_autonomous_bounty_claim))
            .route("/authorized", post(plan_autonomous_bounty_authorized_claim))
            .with_state(state);
        for path in ["/claim", "/authorized"] {
            let body = json!({
                "network":"base-mainnet",
                "bounty_contract":if mode == "bad_input" { "private-marker" } else { "0x1111111111111111111111111111111111111111" },
                "solver":"private-marker-solver", "authorization_nonce":"private-marker-nonce",
                "authorization_valid_before":1,
                "signature":{"v":27,"r":"private-marker-signature","s":"private-marker-signature"}
            });
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri(format!("{path}?private-marker=query"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let expected = if mode == "bad_input" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            assert_eq!(response.status(), expected);
            let response: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap())
                    .unwrap();
            assert_eq!(response["schema_version"], "agent-bounties/action-error-v1");
            assert_eq!(response["error_code"], "action_preparation_failed");
            assert_eq!(response["retryable"], expected.is_server_error());
            assert!(response.get("calls").is_none());
            assert!(!response.to_string().contains("private-marker"));
            if mode == "competition" {
                assert!(response["next_action"]
                    .as_str()
                    .unwrap()
                    .contains("mode is unknown"));
            } else if mode == "indexed" {
                assert_eq!(
                    response["next_action"],
                    "Refresh ready-to-earn inventory and retry."
                );
            }
        }
    }
}
