use anyhow::Context;
use db::PostgresStore;
use std::{env, time::Duration};
use tokio::time::sleep;
use worker::{
    indexer_error_is_retryable, poll_autonomous_indexer_once_with_heartbeat,
    redact_operational_error, run_regression_sandbox_request, snapshot_directory,
    stage_regression_input, AutonomousIndexerConfig, IndexerRecoveryDecision,
    IndexerRecoveryPolicy, RegressionInputKind, RegressionSandboxRunRequest,
    REGRESSION_SANDBOX_DOCKER_BINARY_ENV, REGRESSION_SANDBOX_STAGING_ROOT_ENV,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|value| value == "--snapshot-directory")
    {
        if arguments.len() != 4 {
            anyhow::bail!("usage: worker --snapshot-directory <path> <max-bytes> <max-files>");
        }
        let max_bytes = arguments[2]
            .parse::<u64>()
            .context("max-bytes must be a positive integer")?;
        let max_files = arguments[3]
            .parse::<u32>()
            .context("max-files must be a positive integer")?;
        let snapshot = snapshot_directory(arguments[1].as_ref(), max_bytes, max_files)?;
        println!("{}", serde_json::to_string_pretty(&snapshot)?);
        return Ok(());
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--run-regression")
    {
        if arguments.len() != 2 {
            anyhow::bail!("usage: worker --run-regression <request.json>");
        }
        let request = std::fs::read_to_string(&arguments[1])
            .context("failed to read regression sandbox request")?;
        let request: RegressionSandboxRunRequest =
            serde_json::from_str(&request).context("failed to parse regression sandbox request")?;
        let staging_root = env::var(REGRESSION_SANDBOX_STAGING_ROOT_ENV)
            .with_context(|| format!("{REGRESSION_SANDBOX_STAGING_ROOT_ENV} is required"))?;
        let docker_binary =
            env::var(REGRESSION_SANDBOX_DOCKER_BINARY_ENV).unwrap_or_else(|_| "docker".to_string());
        let outcome =
            run_regression_sandbox_request(request, staging_root.as_ref(), docker_binary).await?;
        println!("{}", serde_json::to_string_pretty(&outcome)?);
        return Ok(());
    }
    if arguments
        .first()
        .is_some_and(|value| value == "--stage-regression-input")
    {
        if arguments.len() != 6 {
            anyhow::bail!(
                "usage: worker --stage-regression-input <source|benchmark> <input-dir> <staging-root> <max-bytes> <max-files>"
            );
        }
        let kind = RegressionInputKind::parse(&arguments[1])?;
        let max_bytes = arguments[4]
            .parse::<u64>()
            .context("max-bytes must be a positive integer")?;
        let max_files = arguments[5]
            .parse::<u32>()
            .context("max-files must be a positive integer")?;
        let staged = stage_regression_input(
            arguments[2].as_ref(),
            arguments[3].as_ref(),
            kind,
            max_bytes,
            max_files,
        )?;
        println!("{}", serde_json::to_string_pretty(&staged)?);
        return Ok(());
    }
    let once =
        env_flag("BASE_INDEXER_ONCE") || arguments.iter().any(|argument| argument == "--once");
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
