//! Exercise the actual entrypoint: containment must precede dependency access.
use reqwest::{Client, Method, StatusCode};
use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const REVISION: &str = "1111111111111111111111111111111111111111";

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn configured(trap: &TcpListener) -> Command {
    let dependency = format!("127.0.0.1:{}", trap.local_addr().unwrap().port());
    let mut command = Command::new(env!("CARGO_BIN_EXE_api"));
    command
        .env_clear()
        .env("API_BOOT_MODE", "recovery-health-only")
        .env("RENDER_GIT_COMMIT", REVISION)
        .env(
            "DATABASE_URL",
            format!("postgres://fixture@{dependency}/fixture"),
        )
        .env("AUTH_SESSION_SECRET", "invalid-short-fixture")
        .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
        .env("ENABLE_BASE_TX_BROADCAST", "true")
        .env("ENABLE_X402_HOSTED_RELAY", "true")
        .env("ENABLE_BOND_SPONSORSHIP", "true")
        .env("ENABLE_STRIPE_LIVE_EXECUTION", "true")
        .env("ENABLE_MANAGED_SERVICE_REFUND_EXECUTION", "true")
        .env("ENABLE_MANAGED_RECOVERED_BOUNTY_REFUNDS", "true")
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
        .env("STRIPE_API_BASE_URL", format!("http://{dependency}"))
        .env("BASE_MAINNET_RPC_URL", format!("http://{dependency}"))
        .env("VERIFIER_EMAIL_ENABLED", "true")
        .env("CREATOR_PUSH_ENABLED", "true")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn dependency_trap() -> TcpListener {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn assert_no_dependency_access(trap: &TcpListener) {
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock,
        "Recovery boot contacted a configured dependency"
    );
}

#[tokio::test]
async fn recovery_boot_and_restart_only_serve_liveness_without_dependencies() {
    let trap = dependency_trap();
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(500))
        .build()
        .unwrap();
    // A restart must retain the same boundary, including mistakenly attached config.
    for _ in 0..2 {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let origin = format!("http://127.0.0.1:{port}");
        let mut process = Process(
            configured(&trap)
                .env("PORT", port.to_string())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let health = loop {
            assert!(
                process.0.try_wait().unwrap().is_none(),
                "API exited before liveness"
            );
            if let Ok(response) = client.get(format!("{origin}/health")).send().await {
                break response;
            }
            assert!(Instant::now() < deadline, "Recovery liveness did not start");
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(health.headers()["x-agent-bounties-revision"], REVISION);
        assert_eq!(
            health.headers()["x-agent-bounties-boot-mode"],
            "recovery-health-only"
        );
        assert_eq!(health.headers()["x-agent-bounties-x402-relay"], "disabled");
        assert_eq!(health.headers()["cache-control"], "no-store");
        assert_eq!(health.text().await.unwrap(), "ok");
        assert_eq!(
            client
                .head(format!("{origin}/health"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        for (method, path) in [
            (Method::GET, "/readiness/marketplace"),
            (Method::GET, "/v1/site-auth/session"),
            (Method::GET, "/v1/base/autonomous-bounties"),
            (Method::POST, "/v1/bounties/fixture/funding-contributions"),
            (Method::POST, "/v1/site-auth/logout"),
            (Method::POST, "/health"),
            (Method::OPTIONS, "/v1/site-auth/session"),
            (Method::GET, "/unknown"),
        ] {
            let response = client
                .request(method, format!("{origin}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(response.headers()["cache-control"], "no-store");
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["ready"], false);
            assert_eq!(body["reason"], "recovery_health_only");
        }
        assert_no_dependency_access(&trap);
        drop(process);
    }
    assert_no_dependency_access(&trap);
}

#[test]
fn malformed_mode_or_diagnostic_arguments_fail_before_database_access() {
    let trap = dependency_trap();
    for (mode, argument) in [
        ("", None),
        ("recovery-health-onli", None),
        ("normal ", None),
        ("recovery-health-only", Some("--notification-readiness")),
        ("recovery-health-only", Some("--unexpected")),
    ] {
        let mut command = configured(&trap);
        command.env("API_BOOT_MODE", mode);
        if let Some(argument) = argument {
            command.arg(argument);
        }
        let mut process = Process(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                assert!(!status.success());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Invalid boot configuration did not fail"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_no_dependency_access(&trap);
    }
}
