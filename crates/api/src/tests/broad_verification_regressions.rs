use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use serde_json::{json, Value};
fn routes(state: SharedState) -> Router {
    crate::verification::router()
        .merge(crate::review_reports::router())
        .merge(crate::review_tasks::router())
        .merge(crate::artifact_submission::router())
        .layer(axum::middleware::from_fn(
            crate::privacy_cache::protect_private_reads,
        ))
        .with_state(state)
}
async fn call(
    app: &Router,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .uri(path)
        .method(if body.is_some() { "POST" } else { "GET" })
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            req.body(
                body.map(|v| Body::from(v.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let code = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (code, String::from_utf8_lossy(&bytes).into())
}
fn plan() -> Value {
    json!({"schema_version":"broad-verification/v1","criteria":[{"id":"work","text":"Deliver the work","review":"Does it solve the brief?"}],"checks":[{"id":"files","criteria":["work"],"purpose":"required","rule":{"checker":"contents_v1","paths":["work.txt"],"folders":[]}}]})
}

#[tokio::test]
async fn review_signatures_verify_eoas_and_contract_wallets_at_a_canonical_safe_block() {
    use alloy::{
        primitives::B256,
        signers::{local::PrivateKeySigner, SignerSync},
    };
    use std::sync::{Arc, Mutex};
    let signer: PrivateKeySigner = format!("{:064x}", 7).parse().unwrap();
    let address = format!("{:#x}", signer.address());
    let digest = B256::repeat_byte(0x42);
    let signature = format!(
        "0x{}",
        hex::encode(signer.sign_hash_sync(&digest).unwrap().as_bytes())
    );
    let block = format!("0x{}", "ab".repeat(32));
    for (mode, expected) in [
        ("eoa", Ok(())),
        ("contract", Ok(())),
        ("wrong_chain", Err(StatusCode::SERVICE_UNAVAILABLE)),
        ("missing_safe_block", Err(StatusCode::SERVICE_UNAVAILABLE)),
        ("malformed_code", Err(StatusCode::SERVICE_UNAVAILABLE)),
        ("wrong_magic", Err(StatusCode::FORBIDDEN)),
        ("wrong_signer", Err(StatusCode::FORBIDDEN)),
    ] {
        let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = observed.clone();
        let safe_hash = block.clone();
        let app = Router::new().route(
            "/",
            post(move |Json(body): Json<Value>| {
                captured.lock().unwrap().push(body.clone());
                let safe_hash = safe_hash.clone();
                async move {
                    let result = match body["method"].as_str().unwrap() {
                        "eth_chainId" => json!(if mode == "wrong_chain" {
                            "0x1"
                        } else {
                            "0x14a34"
                        }),
                        "eth_getBlockByNumber" => {
                            if mode == "missing_safe_block" {
                                Value::Null
                            } else {
                                json!({"hash":safe_hash})
                            }
                        }
                        "eth_getCode" => json!(match mode {
                            "eoa" | "wrong_signer" => "0x",
                            "malformed_code" => "broken",
                            _ => "0x6000",
                        }),
                        "eth_call" => json!(if mode == "wrong_magic" {
                            format!("0x{}", "00".repeat(32))
                        } else {
                            format!("0x1626ba7e{}", "00".repeat(28))
                        }),
                        other => panic!("unexpected RPC method {other}"),
                    };
                    Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let state = test_state_with_base_rpc(
            BountyNetwork::default(),
            format!("http://{}", listener.local_addr().unwrap()),
        );
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let actual_digest = if mode == "wrong_signer" {
            B256::repeat_byte(0x43)
        } else {
            digest
        };
        let result = crate::review_reports::verify_signature(
            &state,
            "base-sepolia",
            &address,
            actual_digest,
            &signature,
        )
        .await;
        server.abort();
        assert_eq!(result, expected, "{mode}");
        for call in observed.lock().unwrap().iter() {
            if call["method"] == "eth_getBlockByNumber" {
                assert_eq!(call["params"], json!(["safe", false]));
            }
            if matches!(call["method"].as_str(), Some("eth_getCode" | "eth_call")) {
                assert_eq!(
                    call["params"][1],
                    json!({"blockHash":block,"requireCanonical":true})
                );
            }
            if call["method"] == "eth_call" {
                assert_eq!(call["params"][0]["to"], address);
                assert!(call["params"][0]["data"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("0x1626ba7e{}", hex::encode(digest))));
            }
        }
    }
}
#[tokio::test]
async fn public_discovery_and_plan_validation_do_not_need_operator_or_provider_keys() {
    let app = routes(test_state(BountyNetwork::default()));
    let (code, body) = call(&app, "/v1/verification/checkers", None, None).await;
    assert_eq!(code, StatusCode::OK);
    let catalog: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(catalog["checks"].as_array().unwrap().len(), 7);
    for entry in catalog["checks"].as_array().unwrap() {
        assert!(entry["parameters_schema"].is_object(), "{}", entry["id"]);
        assert_eq!(entry["payment_approval_status"], "advisory_only");
    }
    let (code, body) = call(&app, "/v1/verification/plans/validate", Some(plan()), None).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["valid"], true);
    let mut invalid = plan();
    invalid["criteria"][0]
        .as_object_mut()
        .unwrap()
        .remove("review");
    invalid["checks"][0]["purpose"] = json!("diagnostic");
    let (_, body) = call(&app, "/v1/verification/plans/validate", Some(invalid), None).await;
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["valid"],
        false
    );
}
#[tokio::test]
async fn anonymous_upload_run_import_and_task_access_fail_before_storage() {
    let app = routes(test_state(BountyNetwork::default()));
    for (path, body) in [
        ("/v1/verification/artifacts", json!({"files":[]})),
        (
            "/v1/verification/artifacts/import-github",
            json!({"repository":"NSPG13/agent-bounties","commit":"a".repeat(40),"paths":["README.md"]}),
        ),
        (
            "/v1/verification/runs",
            json!({"artifact_id":Uuid::new_v4(),"plan":plan()}),
        ),
    ] {
        assert_eq!(
            call(&app, path, Some(body), None).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(&app, "/v1/verification/review-tasks", None, None)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL"]
async fn private_artifact_and_cached_report_are_not_visible_to_other_principals() {
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let owner_wallet = format!("0x{}00000000", Uuid::new_v4().simple());
    let participant = format!("0x{}00000000", Uuid::new_v4().simple());
    let owner = format!("wallet:8453:{owner_wallet}");
    let token = super::security_regressions::fixture_wallet_session(&store, &owner_wallet).await;
    let participant_token =
        super::security_regressions::fixture_wallet_session(&store, &participant).await;
    let files = std::collections::BTreeMap::from([("secret.txt".into(), b"private work".to_vec())]);
    let manifest = broad_verification::Manifest::from_files(&files).unwrap();
    let artifact = store
        .reserve_artifact(&owner, "private", &manifest, 10 * 1024 * 1024 * 1024)
        .await
        .unwrap();
    store
        .finish_artifact(artifact.id, &owner, &artifact.manifest_hash)
        .await
        .unwrap();
    let plan: broad_verification::Plan = serde_json::from_value(plan()).unwrap();
    let run = store
        .begin_verification_run(&owner, artifact.id, &plan)
        .await
        .unwrap();
    let mut state = test_state(BountyNetwork::default());
    Arc::make_mut(&mut state).store = Some(store.clone());
    Arc::make_mut(&mut state).operator_api_token = Some("local-operator".into());
    let app = routes(state);
    for path in [
        format!("/v1/verification/artifacts/{}", artifact.id),
        format!("/v1/verification/artifacts/{}/files/0", artifact.id),
        format!("/v1/verification/runs/{}", run.id),
    ] {
        let (code, body) = call(&app, &path, None, Some("local-operator")).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert!(!body.contains("secret.txt"));
    }
    let path = format!("/v1/verification/artifacts/{}", artifact.id);
    assert_eq!(
        call(&app, &path, None, Some(&token)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, &path, None, Some(&participant_token)).await.0,
        StatusCode::NOT_FOUND
    );
    let grant = json!({"principal_key":format!("wallet:8453:{participant}"),"expires_at":Utc::now()+chrono::Duration::hours(1)});
    assert_eq!(
        call(
            &app,
            &format!("{path}/participants"),
            Some(grant.clone()),
            Some(&token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &path, None, Some(&participant_token)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            &format!("/v1/verification/runs/{}", run.id),
            None,
            Some(&participant_token)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            &format!("{path}/participants/revoke"),
            Some(grant),
            Some(&token)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &path, None, Some(&participant_token)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            &app,
            &format!("/v1/verification/runs/{}", run.id),
            None,
            Some(&participant_token)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    store
        .revoke_auth_session(&service_runtime::auth::hash_secret(&token))
        .await
        .unwrap();
    assert_eq!(
        call(&app, &path, None, Some(&token)).await.0,
        StatusCode::UNAUTHORIZED
    );
}
