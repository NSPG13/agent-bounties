#[path = "support/recovery.rs"]
mod support;
use serde_json::{json, Value};
use std::{net::TcpListener, process::Output, sync::atomic::Ordering, time::Duration};
use support::{Fixture, Rpc, FACTORY, START};

fn command() -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_recovery-reader"));
    command.env_clear().kill_on_drop(true);
    command
}
async fn output(command: &mut tokio::process::Command) -> Output {
    tokio::time::timeout(Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap()
}
async fn identity() -> Value {
    let response = output(command().arg("--build-identity")).await;
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    serde_json::from_slice(&response.stdout).unwrap()
}
fn runtime(command: &mut tokio::process::Command, identity: &Value) {
    if let Some(value) = identity["binding"].as_object() {
        command
            .env("RENDER_SERVICE_ID", value["service_id"].as_str().unwrap())
            .env(
                "RENDER_GIT_COMMIT",
                value["source_revision"].as_str().unwrap(),
            );
    }
}

#[tokio::test]
async fn recovery_reader_rejects_wrong_admission_before_dependencies() {
    let id = identity().await;
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    for (mode, argument, wrong_id) in [
        ("invalid", None, false),
        ("standby", Some("--broad-verification"), false),
        ("active", None, true),
    ] {
        let mut cmd = command();
        runtime(&mut cmd, &id);
        cmd.env("RECOVERY_READER_MODE", mode)
            .env(
                "DATABASE_URL",
                format!("postgres://fixture@{}/fixture", trap.local_addr().unwrap()),
            )
            .env(
                "RECOVERY_READER_BOUND_SERVICE_ID",
                "srv-aaaaaaaaaaaaaaaaaaaa",
            )
            .env("RECOVERY_READER_BOUND_CAPABILITY", "reader")
            .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
            .env("X402_RELAYER_PRIVATE_KEY", "invalid-fixture");
        if wrong_id {
            cmd.env("RENDER_SERVICE_ID", "srv-cccccccccccccccccccc");
        }
        if let Some(arg) = argument {
            cmd.arg(arg);
        }
        let response = output(&mut cmd).await;
        assert!(!response.status.success());
    }
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    // Standby is live but must have no dependency activity, even with invalid keys.
    let mut cmd = command();
    runtime(&mut cmd, &id);
    cmd.env("RECOVERY_READER_MODE", "standby")
        .env(
            "DATABASE_URL",
            format!("postgres://fixture@{}/fixture", trap.local_addr().unwrap()),
        )
        .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
        .env("REGRESSION_VERIFIER_PRIVATE_KEY", "invalid-fixture");
    let mut child = cmd.spawn().unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(child.try_wait().unwrap().is_none());
    #[cfg(unix)]
    {
        let status = std::process::Command::new("/bin/kill")
            .args(["-TERM", &child.id().unwrap().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        assert!(tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
    }
    #[cfg(not(unix))]
    child.kill().await.unwrap();
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

async fn once(fixture: &Fixture, rpc: &Rpc, id: &Value, trap: &TcpListener) -> Output {
    let mut cmd = command();
    runtime(&mut cmd, id);
    cmd.arg("--once")
        .env("RECOVERY_READER_MODE", "active")
        .env("DATABASE_URL", &fixture.url)
        .env("BASE_INDEXER_PROTOCOL", "autonomous-v2")
        .env("BASE_INDEXER_NETWORK", "base-mainnet")
        .env("BASE_INDEXER_FACTORY_CONTRACT", FACTORY)
        .env("BASE_INDEXER_START_BLOCK", START.to_string())
        .env("BASE_INDEXER_RPC_URL", &rpc.url)
        .env("BASE_INDEXER_MAX_BLOCKS_PER_QUERY", "1")
        .env("DISCOVERY_WEBHOOK_SIGNING_KEY", "invalid-fixture")
        .env("VERIFIER_EMAIL_ENABLED", "true")
        .env("CREATOR_PUSH_ENABLED", "true")
        .env("RESEND_API_KEY", "invalid-fixture")
        .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
        .env("ENABLE_BASE_TX_BROADCAST", "true")
        .env("X402_RELAYER_PRIVATE_KEY", "invalid-fixture")
        .env("STRIPE_SECRET_KEY", "invalid-fixture")
        .env(
            "STRIPE_API_BASE_URL",
            format!("http://{}", trap.local_addr().unwrap()),
        );
    output(&mut cmd).await
}

#[tokio::test]
#[ignore = "requires fresh isolated PostgreSQL and a synthetic reader-bound binary"]
async fn recovery_reader_indexes_without_notifications_and_preserves_hold() {
    let id = identity().await;
    assert_eq!(id["binding"]["service_id"], "srv-aaaaaaaaaaaaaaaaaaaa");
    assert_eq!(
        id["binding"]["source_revision"],
        "1111111111111111111111111111111111111111"
    );
    assert_eq!(id["binding"]["capability"], "reader");
    let rpc = Rpc::start().await;
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    let empty = Fixture::new(false).await;
    assert!(empty.snapshot().await.is_empty());
    assert!(!once(&empty, &rpc, &id, &trap).await.status.success());
    assert!(empty.snapshot().await.is_empty(), "reader ran migrations");

    // An eligible subscription proves ordinary ingestion still enqueues.
    let ordinary = Fixture::new(true).await;
    ordinary.subscription().await;
    let report =
        worker::poll_autonomous_indexer_once_with_heartbeat(&ordinary.store, &rpc.config())
            .await
            .unwrap();
    assert_eq!(report.persisted_events, rpc.event_count);
    assert!(
        ordinary.deliveries().await > 0,
        "subscription must be eligible in the normal caller"
    );

    let recovery = Fixture::new(true).await;
    recovery.subscription().await;
    let before = recovery.snapshot().await;
    let result = once(&recovery, &rpc, &id, &trap).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["persisted_events"], json!(rpc.event_count));
    let checkpoint = recovery
        .store
        .autonomous_canonical_checkpoint("base-mainnet", FACTORY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint.block_number, START);
    assert!(!checkpoint.invalidated);
    assert_eq!(recovery.deliveries().await, 0, "recovery must not enqueue");
    let after = recovery.snapshot().await;
    for (table, hash) in &before {
        if ![
            "autonomous_bounty_events",
            "autonomous_canonical_checkpoints",
            "base_log_cursors",
            "base_indexer_heartbeats",
        ]
        .contains(&table.as_str())
        {
            assert_eq!(
                after.get(table),
                Some(hash),
                "unexpected durable write to {table}"
            );
        }
    }
    rpc.safe.store(START + 1, Ordering::SeqCst);
    let result = once(&recovery, &rpc, &id, &trap).await;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["from_block"], START + 1);
    assert_eq!(report["persisted_events"], 0);
    let restarted = recovery.snapshot().await;
    assert_eq!(
        restarted["autonomous_bounty_events"],
        after["autonomous_bounty_events"]
    );
    assert_eq!(recovery.deliveries().await, 0);

    recovery
        .store
        .invalidate_autonomous_canonical_checkpoint("base-mainnet", FACTORY)
        .await
        .unwrap();
    let held = recovery.snapshot().await;
    let result = once(&recovery, &rpc, &id, &trap).await;
    assert!(!result.status.success());
    let final_state = recovery.snapshot().await;
    for table in [
        "autonomous_bounty_events",
        "autonomous_canonical_checkpoints",
        "base_log_cursors",
        "webhook_deliveries",
    ] {
        assert_eq!(
            final_state[table], held[table],
            "held reader changed {table}"
        );
    }
    assert!(
        recovery
            .store
            .autonomous_canonical_checkpoint("base-mainnet", FACTORY)
            .await
            .unwrap()
            .unwrap()
            .invalidated
    );
    assert!(!rpc.requests.lock().unwrap().is_empty());
    assert_eq!(
        trap.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
