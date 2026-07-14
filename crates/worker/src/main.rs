use anyhow::Context;
use db::PostgresStore;
use std::{env, time::Duration};
use tokio::time::sleep;
use worker::{
    indexer_error_is_retryable, poll_autonomous_indexer_once_with_heartbeat,
    redact_operational_error, AutonomousIndexerConfig, IndexerRecoveryDecision,
    IndexerRecoveryPolicy,
};

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
    let recovery_policy = IndexerRecoveryPolicy::from_env()?;
    let mut consecutive_failures = 0u32;

    loop {
        match poll_autonomous_indexer_once_with_heartbeat(&store, &config).await {
            Ok(report) => {
                consecutive_failures = 0;
                println!("{}", serde_json::to_string(&report)?);
                if once {
                    return Ok(());
                }
                if wait_or_shutdown(config.poll_seconds).await? {
                    return Ok(());
                }
            }
            Err(error) => {
                consecutive_failures = consecutive_failures.saturating_add(1);
                let decision = recovery_policy
                    .decision(consecutive_failures, indexer_error_is_retryable(&error));
                eprintln!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "schema": "agent-bounties/indexer-recovery-v1",
                        "network": config.network,
                        "factory_contract": config.factory_contract,
                        "error": redact_operational_error(&error.to_string()),
                        "decision": decision,
                        "evidence_boundary": "Retry resumes from the persisted monotonic cursor. It cannot create funding, verification, payout, or settlement evidence."
                    }))?
                );

                if once {
                    anyhow::bail!(
                        "autonomous Base indexer poll failed; inspect the redacted failure heartbeat"
                    );
                }

                match decision {
                    IndexerRecoveryDecision::RetryFromPersistedCursor {
                        backoff_seconds, ..
                    } => {
                        if wait_or_shutdown(backoff_seconds).await? {
                            return Ok(());
                        }
                    }
                    IndexerRecoveryDecision::ExitForSupervisorRestart { .. } => {
                        anyhow::bail!(
                            "autonomous Base indexer exhausted its bounded recovery budget; inspect the redacted failure heartbeat"
                        );
                    }
                    IndexerRecoveryDecision::HaltForOperatorInvestigation { .. } => loop {
                        if wait_or_shutdown(86_400).await? {
                            return Ok(());
                        }
                    },
                }
            }
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
