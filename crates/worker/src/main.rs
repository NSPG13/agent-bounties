use anyhow::Context;
use db::PostgresStore;
use std::{
    env,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::sleep;
use worker::{
    dispatch_discovery_webhooks_once, indexer_error_is_retryable,
    poll_autonomous_indexer_once_with_heartbeat, poll_open_competition_indexer_once_with_heartbeat,
    poll_open_competition_v2_broker_once, poll_open_competition_v2_indexer_once_with_heartbeat,
    poll_open_competition_v2_keeper_once, poll_open_competition_v2_shadow_once,
    redact_operational_error, run_regression_sandbox_request, snapshot_directory,
    stage_regression_input, validate_regression_candidate, AutonomousIndexerConfig,
    DiscoveryWebhookConfig, IndexerRecoveryDecision, IndexerRecoveryPolicy,
    OpenCompetitionIndexerConfig, OpenCompetitionV2BrokerChainConfig,
    OpenCompetitionV2BrokerConfig, OpenCompetitionV2IndexerConfig, OpenCompetitionV2KeeperConfig,
    OpenCompetitionV2ShadowConfig, RegressionCandidateValidationRequest, RegressionInputKind,
    RegressionSandboxRunRequest, REGRESSION_SANDBOX_DOCKER_BINARY_ENV,
    REGRESSION_SANDBOX_STAGING_ROOT_ENV,
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
    if arguments
        .first()
        .is_some_and(|value| value == "--validate-regression-candidate")
    {
        if arguments.len() != 2 {
            anyhow::bail!("usage: worker --validate-regression-candidate <request.json>");
        }
        let request = std::fs::read_to_string(&arguments[1])
            .context("failed to read regression candidate request")?;
        let request: RegressionCandidateValidationRequest = serde_json::from_str(&request)
            .context("failed to parse regression candidate request")?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_secs();
        validate_regression_candidate(request, now)?;
        println!("ok");
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

    if protocol == "open-competition-v1" {
        return run_open_competition_indexer(&store, once).await;
    }
    if protocol == "open-competition-v2-beta3" {
        return run_open_competition_v2_indexer(&store, once).await;
    }
    if protocol == "open-competition-v2-broker" {
        return run_open_competition_v2_broker(&store, once).await;
    }
    if protocol == "open-competition-v2-keeper" {
        return run_open_competition_v2_keeper(&store, once).await;
    }
    if protocol == "open-competition-v2-shadow" {
        return run_open_competition_v2_shadow(&store, once).await;
    }
    if protocol != "autonomous-v1" {
        anyhow::bail!(
            "BASE_INDEXER_PROTOCOL must be autonomous-v1, open-competition-v1, open-competition-v2-beta3, open-competition-v2-broker, open-competition-v2-keeper, or open-competition-v2-shadow"
        );
    }
    let config = AutonomousIndexerConfig::from_env()?;
    let discovery_webhooks = DiscoveryWebhookConfig::from_env()?;
    let recovery_policy = IndexerRecoveryPolicy::from_env()?;
    let mut consecutive_failures = 0u32;

    loop {
        match poll_autonomous_indexer_once_with_heartbeat(&store, &config).await {
            Ok(report) => {
                consecutive_failures = 0;
                println!("{}", serde_json::to_string(&report)?);
                if let Some(discovery_webhooks) = &discovery_webhooks {
                    let delivery_report =
                        dispatch_discovery_webhooks_once(&store, discovery_webhooks).await?;
                    println!("{}", serde_json::to_string(&delivery_report)?);
                }
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

async fn run_open_competition_v2_shadow(store: &PostgresStore, once: bool) -> anyhow::Result<()> {
    let config = OpenCompetitionV2ShadowConfig::from_env()?;
    let poll_seconds = env_u64("OPEN_COMPETITION_V2_SHADOW_POLL_SECONDS", 30)?.clamp(5, 300);
    loop {
        match poll_open_competition_v2_shadow_once(store, &config).await {
            Ok(report) => println!("{}", serde_json::to_string(&report)?),
            Err(error) => eprintln!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "schema": "agent-bounties/open-competition-v2-shadow-recovery-v1",
                    "error": redact_operational_error(&error.to_string()),
                    "decision": "retry_full_safe_comparison",
                    "evidence_boundary": "A failed or stale comparison disables public Beta3 operations; it cannot create chain or payment evidence."
                }))?
            ),
        }
        if once || wait_or_shutdown(poll_seconds).await? {
            return Ok(());
        }
    }
}

async fn run_open_competition_v2_keeper(store: &PostgresStore, once: bool) -> anyhow::Result<()> {
    let config = OpenCompetitionV2KeeperConfig::from_env()?;
    let chain = OpenCompetitionV2BrokerChainConfig::from_env()?;
    let poll_seconds = env_u64("OPEN_COMPETITION_V2_KEEPER_POLL_SECONDS", 5)?.clamp(1, 60);
    loop {
        match poll_open_competition_v2_keeper_once(store, &config, &chain).await {
            Ok(report) => println!("{}", serde_json::to_string(&report)?),
            Err(error) => eprintln!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "schema": "agent-bounties/open-competition-v2-keeper-recovery-v1",
                    "error": redact_operational_error(&error.to_string()),
                    "decision": "retry_from_safe_projection",
                    "evidence_boundary": "Keeper retries are permissionless and idempotent at the contract state machine; only safe canonical V2 events prove outcomes."
                }))?
            ),
        }
        if once || wait_or_shutdown(poll_seconds).await? {
            return Ok(());
        }
    }
}

async fn run_open_competition_v2_broker(store: &PostgresStore, once: bool) -> anyhow::Result<()> {
    let config = OpenCompetitionV2BrokerConfig::from_env()?;
    let chain = OpenCompetitionV2BrokerChainConfig::from_env()?;
    let poll_seconds = env_u64("OPEN_COMPETITION_V2_BROKER_POLL_SECONDS", 5)?.clamp(1, 60);
    loop {
        match poll_open_competition_v2_broker_once(store, &config, &chain).await {
            Ok(report) => println!("{}", serde_json::to_string(&report)?),
            Err(error) => eprintln!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "schema": "agent-bounties/open-competition-v2-broker-recovery-v1",
                    "error": redact_operational_error(&error.to_string()),
                    "decision": "retry_from_persisted_lease",
                    "evidence_boundary": "A retry cannot create payment evidence. Only safe canonical USDC and CompetitionSettledV2 events change proof-job outcomes."
                }))?
            ),
        }
        if once || wait_or_shutdown(poll_seconds).await? {
            return Ok(());
        }
    }
}

async fn run_open_competition_v2_indexer(store: &PostgresStore, once: bool) -> anyhow::Result<()> {
    let config = OpenCompetitionV2IndexerConfig::from_env()?;
    let recovery_policy = IndexerRecoveryPolicy::from_env()?;
    let mut consecutive_failures = 0_u32;
    loop {
        match poll_open_competition_v2_indexer_once_with_heartbeat(store, &config).await {
            Ok(report) => {
                consecutive_failures = 0;
                println!("{}", serde_json::to_string(&report)?);
                if once || wait_or_shutdown(config.poll_seconds).await? {
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
                        "schema": "agent-bounties/open-competition-v2-beta3-indexer-recovery-v1",
                        "protocol_version": "agent-bounties/open-competition-v2-beta3",
                        "network": config.network,
                        "factory_contract": config.factory_contract,
                        "deployment_block": config.deployment_block,
                        "error": redact_operational_error(&error.to_string()),
                        "decision": decision,
                        "evidence_boundary": "Recovery resumes only from the isolated V2 factory cursor and cannot create settlement evidence."
                    }))?
                );
                if once {
                    anyhow::bail!("Open Competition V2 Base indexer poll failed");
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
                        anyhow::bail!("Open Competition V2 indexer exhausted its recovery budget");
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

async fn run_open_competition_indexer(store: &PostgresStore, once: bool) -> anyhow::Result<()> {
    let config = OpenCompetitionIndexerConfig::from_env()?;
    let recovery_policy = IndexerRecoveryPolicy::from_env()?;
    let mut consecutive_failures = 0_u32;
    loop {
        match poll_open_competition_indexer_once_with_heartbeat(store, &config).await {
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
                        "schema": "agent-bounties/open-competition-v1-indexer-recovery-v1",
                        "protocol_version": "agent-bounties/open-competition-v1",
                        "network": config.network,
                        "factory_contract": config.factory_contract,
                        "deployment_block": config.deployment_block,
                        "error": redact_operational_error(&error.to_string()),
                        "decision": decision,
                        "evidence_boundary": "Recovery resumes only from the versioned factory cursor. It cannot alter historical bounty rows or create payment evidence."
                    }))?
                );
                if once {
                    anyhow::bail!("open-competition Base indexer poll failed");
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
                            "open-competition Base indexer exhausted its recovery budget"
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

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    match env::var(name) {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .with_context(|| format!("{name} must be an unsigned integer")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("failed to read {name}")),
    }
}
