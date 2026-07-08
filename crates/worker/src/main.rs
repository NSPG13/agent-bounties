use anyhow::Context;
use db::PostgresStore;
use std::{env, time::Duration};
use tokio::time::sleep;
use worker::{poll_base_indexer_once_with_heartbeat, BaseIndexerConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let once =
        env_flag("BASE_INDEXER_ONCE") || env::args().skip(1).any(|argument| argument == "--once");
    let database_url = env::var("DATABASE_URL")
        .context("DATABASE_URL is required for the Base USDC indexer worker")?;
    let config = BaseIndexerConfig::from_env()?;
    let store = PostgresStore::connect(&database_url).await?;
    store.migrate().await?;

    loop {
        let report = poll_base_indexer_once_with_heartbeat(&store, &config).await?;
        println!("{}", serde_json::to_string(&report)?);
        if once {
            return Ok(());
        }

        tokio::select! {
            _ = sleep(Duration::from_secs(config.poll_seconds)) => {}
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for shutdown signal")?;
                return Ok(());
            }
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
