// Synthetic PostgreSQL integration fixtures only. No chain or payment provider is contacted.
// Included by the API and direct MCP tests so they exercise the same persisted records.
const WRITER_IDS: &str = "AGENT_BOUNTIES_PG_STATUS_WRITER_IDS";

fn validate_test_database_url(raw: &str) -> Result<(), &'static str> {
    let url = reqwest::Url::parse(raw).map_err(|_| "malformed PostgreSQL test URL")?;
    if !matches!(url.scheme(), "postgres" | "postgresql")
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("PostgreSQL tests require a loopback URL without connection overrides");
    }
    let database = url.path().strip_prefix('/').unwrap_or_default();
    if database.is_empty()
        || !database
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("PostgreSQL tests require an explicit local test database name");
    }
    Ok(())
}

#[test]
fn database_url_guard_rejects_remote_or_malformed_targets_before_connecting() {
    for url in [
        "postgres://test:test@127.0.0.1:5432/agent_bounties",
        "postgresql://localhost/agent_bounties_test",
        "postgres://[::1]:5432/agent_bounties",
    ] {
        assert_eq!(validate_test_database_url(url), Ok(()));
    }
    for (case, url) in [
        "not a URL",
        "postgres://test:test@database.example/agent_bounties",
        "postgres://127.0.0.1.database.example/agent_bounties",
        "postgres://test:test@10.0.0.1/agent_bounties",
        "postgres://test:test@127.0.0.1@database.example/agent_bounties",
        "https://127.0.0.1/agent_bounties",
        "postgres://127.0.0.1/",
        "postgres://127.0.0.1/agent_bounties/other",
        "postgres://127.0.0.1/agent_bounties?host=database.example",
        "postgres://127.0.0.1/agent_bounties#other",
    ]
    .iter()
    .enumerate()
    {
        assert!(validate_test_database_url(url).is_err(), "case {case}");
    }
}

pub async fn store() -> db::PostgresStore {
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
        .expect("set AGENT_BOUNTIES_TEST_DATABASE_URL to a disposable test database");
    validate_test_database_url(&url).expect("refusing unsafe PostgreSQL test target");
    db::PostgresStore::connect(&url).await.unwrap()
}

pub async fn seed(store: &db::PostgresStore) -> (app::BountyNetwork, Vec<domain::Bounty>) {
    store.migrate().await.unwrap();
    let mut network = app::BountyNetwork::default();
    let mut bounties = Vec::new();
    for title in ["Selected status fixture", "Unrelated status fixture"] {
        let bounty = network
            .open_pooled_bounty(app::OpenPooledBountyRequest {
                bounty_id: None,
                idempotency_key: None,
                title: title.to_string(),
                template_slug: "fix-ci-failure".to_string(),
                target_amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                funding_mode: domain::FundingMode::Simulated,
                privacy: domain::PrivacyLevel::Public,
                funding_targets: Vec::new(),
            })
            .unwrap();
        store.upsert_bounty(&bounty).await.unwrap();
        bounties.push(bounty);
    }
    (network, bounties)
}

pub async fn run_writer_if_requested() -> bool {
    let Ok(raw) = std::env::var(WRITER_IDS) else {
        return false;
    };
    let ids: Vec<uuid::Uuid> = serde_json::from_str(&raw).unwrap();
    assert_eq!(ids.len(), 2);
    let store = store().await;
    for id in ids {
        let scope = store.load_bounty_status_scope(id).await.unwrap().unwrap();
        let mut network = app::BountyNetwork::default();
        network.bounties.insert(id, scope.bounty);
        let report = network
            .add_funding_contribution(app::AddFundingContributionRequest {
                bounty_id: id,
                contributor_agent_id: None,
                source_organization_id: None,
                amount_minor: 1_000_000,
                currency: "usdc".to_string(),
                rail: domain::PaymentRail::Simulated,
                external_reference: Some(format!("pg-status-fixture:{id}")),
            })
            .unwrap();
        service_runtime::persist_pooled_funding_report(Some(&store), &report)
            .await
            .unwrap();
    }
    println!("independent PostgreSQL writer pid={}", std::process::id());
    true
}

pub async fn write_in_separate_process(test: &str, ids: &[uuid::Uuid]) {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([test, "--ignored", "--exact", "--nocapture"])
        .env(WRITER_IDS, serde_json::to_string(ids).unwrap())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    assert_ne!(child.id(), std::process::id());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("independent PostgreSQL writer exceeded 30 seconds");
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "writer failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("1 passed; 0 failed"),
        "writer test did not execute: {stdout}"
    );
    assert!(stdout.contains("independent PostgreSQL writer pid="));
    println!(
        "writer receipt: {}",
        stdout
            .lines()
            .find(|line| line.starts_with("independent PostgreSQL writer pid="))
            .unwrap()
    );
}

pub fn assert_funded(value: &serde_json::Value, selected: uuid::Uuid, other: uuid::Uuid) {
    assert_eq!(value["bounty"]["id"], selected.to_string());
    assert_eq!(value["bounty"]["status"], "Claimable");
    assert_eq!(value["funding_summary"]["claimable"], true);
    assert_eq!(value["funding_summary"]["applied"]["amount"], 1_000_000);
    let contributions = value["funding_contributions"].as_array().unwrap();
    assert_eq!(contributions.len(), 1);
    assert_eq!(contributions[0]["bounty_id"], selected.to_string());
    // Anonymous status keeps the durable funding amount but hides provider references.
    assert!(contributions[0]["external_reference"].is_null());
    assert_eq!(contributions[0]["amount"]["amount"], 1_000_000);
    assert!(!serde_json::to_string(value)
        .unwrap()
        .contains(&other.to_string()));
    assert!(
        value["settlements"].as_array().unwrap().is_empty(),
        "funding is not settlement"
    );
}
