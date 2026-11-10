//! Real entrypoint + isolated PostgreSQL: public reads without durable writes.
use chrono::Utc;
use db::{BaseIndexerHeartbeat, NewTrialBounty, PostgresStore};
use domain::{Bounty, FundingMode, Money, PrivacyLevel};
use reqwest::{Client, Method, StatusCode};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    net::TcpListener,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn command(database: Option<&str>, trap: &TcpListener) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_api"));
    cmd.env_clear()
        .env("API_BOOT_MODE", "recovery-read-only")
        .env(
            "RENDER_GIT_COMMIT",
            "1111111111111111111111111111111111111111",
        )
        .env("AUTH_SESSION_SECRET", "invalid-short-fixture")
        .env("WORKSPACE_TOKEN_SECRET", "invalid-short-fixture")
        .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
        .env("ENABLE_BASE_TX_BROADCAST", "true")
        .env("ENABLE_X402_HOSTED_RELAY", "true")
        .env("ENABLE_STRIPE_LIVE_EXECUTION", "true")
        .env("ENABLE_MANAGED_SERVICE_REFUND_EXECUTION", "true")
        .env("X402_RELAYER_PRIVATE_KEY", "invalid-fixture-not-a-key")
        .env(
            "BOND_SPONSOR_GRANT_SIGNER_PRIVATE_KEY",
            "invalid-fixture-not-a-key",
        )
        .env(
            "CUSTOMER_REVIEW_SIGNER_PRIVATE_KEY",
            "invalid-fixture-not-a-key",
        )
        .env("STRIPE_SECRET_KEY", "invalid-fixture-not-a-key")
        .env(
            "STRIPE_API_BASE_URL",
            format!("http://{}", trap.local_addr().unwrap()),
        )
        .env("VERIFIER_EMAIL_ENABLED", "true")
        .env("CREATOR_PUSH_ENABLED", "true")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(database) = database {
        cmd.env("DATABASE_URL", database);
    }
    cmd
}

async fn start(
    database: &str,
    trap: &TcpListener,
    client: &Client,
    rpc: &str,
) -> (Process, String) {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut process = Process(
        command(Some(database), trap)
            .env("PORT", port.to_string())
            .env("BASE_MAINNET_RPC_URL", rpc)
            .spawn()
            .unwrap(),
    );
    let origin = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "read-only API exited before health"
        );
        if let Ok(response) = client.get(format!("{origin}/health")).send().await {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["x-agent-bounties-boot-mode"],
                "recovery-read-only"
            );
            assert_eq!(
                response.headers()["x-agent-bounties-x402-relay"],
                "disabled"
            );
            break;
        }
        assert!(Instant::now() < deadline, "health deadline");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    (process, origin)
}

async fn snapshot(pool: &PgPool) -> BTreeMap<String, String> {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT tablename FROM pg_tables WHERE schemaname='public' ORDER BY tablename",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    let mut result = BTreeMap::new();
    for name in names {
        assert!(name.bytes().all(|v| v.is_ascii_alphanumeric() || v == b'_'));
        let digest: String = sqlx::query_scalar(&format!("SELECT md5(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text, '[]')) FROM public.\"{name}\" t"))
            .fetch_one(pool).await.unwrap();
        result.insert(name, digest);
    }
    result
}

// Reuse the decoded contract fixture and the same reader evidence as the normal
// autonomous projection regressions. The actual API child consumes only RPC/DB.
async fn canonical_fixture(store: &PostgresStore, factory: &str, contract: &str) {
    use chain_base::AutonomousBountyEventKind as Kind;
    let fixture: Value = serde_json::from_str(include_str!(
        "../../chain-base/tests/fixtures/autonomous-v2-loop.json"
    ))
    .unwrap();
    let logs: Vec<chain_base::RpcEvmLog> = serde_json::from_value(fixture["logs"].clone()).unwrap();
    let decoded =
        chain_base::decode_autonomous_bounty_logs(chain_base::rpc_logs_to_evm_logs(logs).unwrap())
            .unwrap();
    let id = decoded
        .iter()
        .find(|e| e.kind == Kind::CanonicalBountyCreated)
        .unwrap()
        .bounty_id
        .clone();
    let events = decoded.into_iter().filter(|e| {
        e.bounty_id == id
            && matches!(
                e.kind,
                Kind::CanonicalBountyCreated
                    | Kind::CanonicalBountyTermsCommitted
                    | Kind::CanonicalBountyEconomicsConfigured
                    | Kind::CanonicalBountyVerificationConfigured
            )
    });
    for (i, mut event) in events.enumerate() {
        event.id = Uuid::new_v4();
        event.log_key = format!("{factory}:recovery:{i}");
        event.block_number = 100;
        event.log_index = i as u64;
        event.contract_address = factory.into();
        event.occurred_at = Utc::now();
        if event.kind == Kind::CanonicalBountyCreated {
            event.data["bounty_contract"] = json!(contract);
        }
        if event.kind == Kind::CanonicalBountyEconomicsConfigured {
            event.data["initial_funding"] = json!(0);
            event.data["target_amount"] = json!(
                event.data["solver_reward"].as_u64().unwrap()
                    + event.data["verifier_reward"].as_u64().unwrap()
            );
        }
        store
            .upsert_autonomous_bounty_event("base-mainnet", &event)
            .await
            .unwrap();
    }
    store
        .commit_autonomous_canonical_checkpoint(
            "base-mainnet",
            factory,
            None,
            None,
            &chain_base::BaseBlockIdentity {
                number: 998,
                hash: format!("0x{}", "ab".repeat(32)),
                timestamp: 1,
            },
            None,
        )
        .await
        .unwrap();
    let now = Utc::now();
    store
        .upsert_base_indexer_heartbeat(&BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: factory.into(),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now),
            latest_block: Some(1000),
            confirmed_to_block: Some(998),
            from_block: Some(100),
            to_block: Some(998),
            fetched_logs: 4,
            persisted_cursor_block: Some(998),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated local AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn real_process_reads_preserve_data_privacy_quarantine_and_restart_boundaries() {
    let database = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&database);
    let pool = PgPool::connect(&database).await.unwrap();
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(18))
        .build()
        .unwrap();
    let rpc_calls = Arc::new(AtomicUsize::new(0));
    let observed = rpc_calls.clone();
    let rpc = axum::Router::new().route(
        "/",
        axum::routing::post(move |axum::Json(request): axum::Json<Value>| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!("0x3e8"),
                    "eth_getBlockByNumber" => json!({
                        "number": if request["params"][0] == "safe" { "0x3e6" } else { "0x3e8" },
                        "hash": format!("0x{}", "ab".repeat(32)),
                        "timestamp": format!("0x{:x}", Utc::now().timestamp()),
                    }),
                    method => panic!("unexpected fixture RPC: {method}"),
                };
                axum::Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let rpc_server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
    // This test requires a fresh fixture; no migration may happen in the API.
    assert!(
        snapshot(&pool).await.is_empty(),
        "use a fresh isolated fixture"
    );
    let (process, origin) = start(&database, &trap, &client, &rpc_url).await;
    assert_eq!(
        client
            .get(format!(
                "{origin}/v1/opportunities?source_type=legacy_bounty"
            ))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(process);
    assert!(snapshot(&pool).await.is_empty(), "API ran migrations");

    let store = PostgresStore::connect(&database).await.unwrap();
    store.migrate().await.unwrap(); // fixture setup only
    let mut public = Bounty::new(
        "Public durable read fixture",
        "fix-ci-failure",
        Money::new(100, "usdc").unwrap(),
        FundingMode::Simulated,
        PrivacyLevel::Public,
    );
    let private = Bounty::new(
        "PRIVATE_MUST_NOT_LEAK",
        "fix-ci-failure",
        Money::new(999, "usdc").unwrap(),
        FundingMode::Simulated,
        PrivacyLevel::Private,
    );
    store.upsert_bounty(&public).await.unwrap();
    store.upsert_bounty(&private).await.unwrap();
    let trial_id = Uuid::new_v4();
    store
        .create_or_get_trial_bounty(&NewTrialBounty {
            id: trial_id,
            idempotency_key: trial_id.to_string(),
            request_fingerprint: trial_id.to_string(),
            title: "Real offchain fixture".into(),
            goal: "Read an existing durable record".into(),
            acceptance_criteria: vec!["No writes".into()],
            source_url: None,
            discovery_source: "test".into(),
            status: "open".into(),
            demo_agent_solution: json!({}),
            expires_at: Utc::now() + chrono::Duration::days(1),
        })
        .await
        .unwrap();
    let factory = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9";
    let contract = "0x1111111111111111111111111111111111111111";
    canonical_fixture(&store, factory, contract).await;
    let before_positive = snapshot(&pool).await;
    let calls_before = rpc_calls.load(Ordering::SeqCst);
    let (process, origin) = start(&database, &trap, &client, &rpc_url).await;
    let identity = format!("canonical:base-mainnet:{contract}");
    let canonical_url = format!("{origin}/v1/opportunities?opportunity_id={identity}");
    let positive: Value = client
        .get(&canonical_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(positive["coverage"], "complete", "{positive}");
    assert_eq!(positive["degraded"], false, "{positive}");
    assert_eq!(positive["items"].as_array().unwrap().len(), 1, "{positive}");
    assert_eq!(positive["items"][0]["source_id"], contract);
    assert!(
        rpc_calls.load(Ordering::SeqCst) > calls_before,
        "API skipped configured RPC"
    );
    let feed_url = format!(
        "{origin}/v1/base/autonomous-bounties/feed?claimable_only=true&bounty_contract={contract}"
    );
    assert_eq!(
        client.get(&feed_url).send().await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        snapshot(&pool).await,
        before_positive,
        "positive reads wrote data"
    );

    // Change only the canonical hold, using fixture administration. The same
    // running API and otherwise healthy evidence must stop exposing this row.
    store
        .invalidate_autonomous_canonical_checkpoint("base-mainnet", factory)
        .await
        .unwrap();
    let held = snapshot(&pool).await;
    let negative: Value = client
        .get(&canonical_url)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(negative["coverage"], "unavailable", "{negative}");
    assert_eq!(negative["degraded"], true);
    assert!(negative["items"].as_array().unwrap().is_empty());
    assert_eq!(
        client.get(&feed_url).send().await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    drop(process);
    assert_eq!(snapshot(&pool).await, held, "quarantined reads wrote data");

    // A real, migrated DB cannot mask argument admission failure. Capture only
    // the child error text and assert the specific early recovery-mode error.
    let rejected = command(Some(&database), &trap)
        .arg("--notification-readiness")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut rejected = Process(rejected);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = rejected.0.try_wait().unwrap() {
            assert!(!status.success());
            break;
        }
        assert!(Instant::now() < deadline, "argument rejection deadline");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut error = String::new();
    std::io::Read::read_to_string(&mut rejected.0.stderr.take().unwrap(), &mut error).unwrap();
    assert!(
        error.contains("Recovery read-only mode does not accept command arguments"),
        "{error}"
    );
    drop(rejected);
    assert_eq!(snapshot(&pool).await, held, "rejected arguments wrote data");

    for pass in 0..2 {
        let before = snapshot(&pool).await;
        let (process, origin) = start(&database, &trap, &client, &rpc_url).await;
        for (source, identity) in [
            ("legacy_bounty", public.id),
            ("unfunded_offchain", trial_id),
        ] {
            let response = client
                .get(format!(
                    "{origin}/v1/opportunities?source_type={source}&view=recent"
                ))
                .header("Origin", "https://agentizable.ai")
                .header("cache-control", "no-cache")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["access-control-allow-origin"],
                "https://agentizable.ai"
            );
            assert!(!response
                .headers()
                .contains_key("access-control-allow-credentials"));
            let value: Value = response.json().await.unwrap();
            assert_eq!(value["degraded"], false);
            assert!(value["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["source_id"] == identity.to_string()));
            assert!(!value.to_string().contains("PRIVATE_MUST_NOT_LEAK"));
            if pass == 1 && source == "legacy_bounty" {
                assert!(value.to_string().contains("Updated durable fixture"));
            }
        }
        let canonical: Value = client
            .get(format!(
                "{origin}/v1/opportunities?source_type=canonical_base"
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(canonical["degraded"], true);
        assert_eq!(canonical["coverage"], "unavailable");
        assert!(canonical["items"].as_array().unwrap().is_empty());
        let private_exact: Value = client.get(format!("{origin}/v1/opportunities?network=base-mainnet&view=recent&source_type=legacy_bounty&opportunity_id=legacy:{}&limit=1", private.id)).send().await.unwrap().json().await.unwrap();
        assert!(private_exact["items"].as_array().unwrap().is_empty());
        let ready: Value = client.get(format!("{origin}/v1/opportunities?network=base-mainnet&view=ready_to_earn&source_type=canonical_base&limit=300")).send().await.unwrap().json().await.unwrap();
        assert_eq!(ready["degraded"], true);
        assert!(ready["items"].as_array().unwrap().is_empty());
        assert_eq!(
            client
                .get(format!("{origin}/readiness/marketplace"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(!client.get(format!("{origin}/v1/base/autonomous-bounties/feed?claimable_only=false&bounty_contract={contract}")).send().await.unwrap().status().is_success());
        assert_eq!(
            client
                .head(format!(
                    "{origin}/v1/opportunities?source_type=unfunded_offchain"
                ))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let hostile = client
            .get(format!("{origin}/health"))
            .header("Origin", "https://attacker.invalid")
            .send()
            .await
            .unwrap();
        assert!(!hostile
            .headers()
            .contains_key("access-control-allow-origin"));
        let head = client
            .head(format!(
                "{origin}/v1/opportunities?source_type=unfunded_offchain"
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.bytes().await.unwrap().is_empty());
        for path in [
            "/health",
            "/readiness/marketplace",
            "/v1/opportunities",
            "/v1/base/autonomous-bounties/feed",
        ] {
            for method in ["GET", "HEAD"] {
                let preflight = client
                    .request(Method::OPTIONS, format!("{origin}{path}"))
                    .header("Origin", "https://agentizable.ai")
                    .header("Access-Control-Request-Method", method)
                    .header("Access-Control-Request-Headers", "accept, cache-control")
                    .send()
                    .await
                    .unwrap();
                assert!(preflight.status().is_success());
                assert_eq!(
                    preflight.headers()["access-control-allow-origin"],
                    "https://agentizable.ai"
                );
                assert!(!preflight
                    .headers()
                    .contains_key("access-control-allow-credentials"));
            }
        }
        for (path, origin_header, method, headers) in [
            (
                "/v1/site-auth/session",
                "https://agentizable.ai",
                "GET",
                "accept",
            ),
            (
                "/v1/opportunities",
                "https://attacker.invalid",
                "GET",
                "cache-control",
            ),
            (
                "/v1/opportunities",
                "https://agentizable.ai",
                "POST",
                "accept",
            ),
            (
                "/v1/opportunities",
                "https://agentizable.ai",
                "GET",
                "authorization",
            ),
            (
                "/v1/opportunities",
                "https://agentizable.ai",
                "GET",
                "content-type",
            ),
        ] {
            assert_eq!(
                client
                    .request(Method::OPTIONS, format!("{origin}{path}"))
                    .header("Origin", origin_header)
                    .header("Access-Control-Request-Method", method)
                    .header("Access-Control-Request-Headers", headers)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        for path in [
            "/health",
            "/readiness/marketplace",
            "/v1/opportunities",
            "/v1/base/autonomous-bounties/feed",
            "/v1/bounties/fixture/funding-contributions",
            "/internal/verifier-checkpoints",
        ] {
            for method in [
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ] {
                assert_eq!(
                    client
                        .request(method, format!("{origin}{path}"))
                        .body("{}")
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::SERVICE_UNAVAILABLE
                );
            }
        }
        for path in [
            "/v1/site-auth/session",
            "/v1/site-auth/login/google",
            "/v1/site-auth/callback/github",
            "/v1/evals/bountybench",
            "/unknown",
            "/v1/opportunities/",
            "/v1/%6fpportunities",
        ] {
            assert_eq!(
                client
                    .get(format!("{origin}{path}"))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::SERVICE_UNAVAILABLE
            );
        }
        drop(process);
        assert_eq!(snapshot(&pool).await, before, "API changed durable data");
        assert_eq!(
            trap.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "API contacted a write provider"
        );
        if pass == 0 {
            public.title = "Updated durable fixture".into();
            store.upsert_bounty(&public).await.unwrap();
        }
    }
    rpc_server.abort();
}

#[test]
fn missing_database_and_arguments_fail_before_dependencies() {
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    for argument in [None, Some("--notification-readiness")] {
        let mut cmd = command(None, &trap);
        if let Some(argument) = argument {
            cmd.arg(argument);
        }
        let mut process = Process(cmd.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
