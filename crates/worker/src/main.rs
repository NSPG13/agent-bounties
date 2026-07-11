use anyhow::Context;
use db::PostgresStore;
use std::{env, time::Duration};
use tokio::time::sleep;
use worker::{poll_autonomous_indexer_once_with_heartbeat, AutonomousIndexerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let once =
        env_flag("BASE_INDEXER_ONCE") || env::args().skip(1).any(|argument| argument == "--once");
    let database_url = env::var("DATABASE_URL")
        .context("DATABASE_URL is required for the Base USDC indexer worker")?;
    let store = PostgresStore::connect(&database_url).await?;
    store.migrate().await?;
    let protocol = env::var("BASE_INDEXER_PROTOCOL")
        .unwrap_or_else(|_| "autonomous-v1".to_string())
        .trim()
        .to_ascii_lowercase();

    if protocol != "autonomous-v1" {
        anyhow::bail!("BASE_INDEXER_PROTOCOL must be autonomous-v1");
    }
    let config = AutonomousIndexerConfig::from_env()?;

    loop {
        let report = poll_autonomous_indexer_once_with_heartbeat(&store, &config).await?;
        println!("{}", serde_json::to_string(&report)?);
        if once {
            return Ok(());
        }
        if wait_or_shutdown(config.poll_seconds).await? {
            return Ok(());
        }
    }
}

async fn wait_or_shutdown(seconds: u64) -> anyhow::Result<bool> {
    tokio::select! {
        _ = sleep(Duration::from_secs(seconds)) => Ok(false),
        signal = tokio::signal::ctrl_c() => {
            signal.context("failed to listen for shutdown signal")?;
            Ok(true)
        }
    }
}

fn env_flag(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}
