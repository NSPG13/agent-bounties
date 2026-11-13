//! Dedicated recovery process. No general worker dispatcher, migrations,
//! discovery enqueue/dispatch, notification runtime or signing constructors.
use anyhow::{bail, ensure};
use db::PostgresStore;
use serde::Serialize;
use std::{env, time::Duration};
use worker::{
    AutonomousIndexerConfig, IndexerRecoveryDecision, IndexerRecoveryPolicy, IndexerRecoveryState,
};

const DENIED: &str = include_str!("../../../ops/recovery-reader-denied-services.txt");

#[derive(Clone, Debug, Serialize)]
struct Binding<'a> {
    service_id: &'a str,
    source_revision: &'a str,
    capability: &'a str,
}
impl<'a> Binding<'a> {
    fn checked(
        service_id: &'a str,
        source_revision: &'a str,
        capability: &'a str,
    ) -> anyhow::Result<Self> {
        ensure!(
            service_id.len() == 24
                && service_id.starts_with("srv-")
                && service_id[4..]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()),
            "Invalid compiled recovery service identity"
        );
        ensure!(
            !DENIED.lines().any(|id| id == service_id),
            "Existing service cannot host the recovery reader"
        );
        ensure!(
            source_revision.len() == 40
                && source_revision
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid compiled recovery source identity"
        );
        ensure!(
            matches!(capability, "standby" | "reader"),
            "Invalid compiled recovery capability"
        );
        Ok(Self {
            service_id,
            source_revision,
            capability,
        })
    }
    fn runtime(
        &self,
        service: Option<&str>,
        revision: Option<&str>,
        active: bool,
    ) -> anyhow::Result<()> {
        ensure!(
            service == Some(self.service_id) && revision == Some(self.source_revision),
            "Recovery runtime identity does not match its artifact"
        );
        ensure!(
            !active || self.capability == "reader",
            "Standby artifact cannot activate a reader"
        );
        Ok(())
    }
}
fn compiled_binding() -> anyhow::Result<Option<Binding<'static>>> {
    match (
        option_env!("RECOVERY_READER_BOUND_SERVICE_ID"),
        option_env!("RECOVERY_READER_BOUND_REVISION"),
        option_env!("RECOVERY_READER_BOUND_CAPABILITY"),
    ) {
        (None, None, None) => Ok(None), // Ordinary workspace builds are inert only.
        (Some(id), Some(revision), Some(capability)) => {
            Binding::checked(id, revision, capability).map(Some)
        }
        _ => bail!("Incomplete compiled recovery identity"),
    }
}

enum Reader {
    Autonomous(Box<AutonomousIndexerConfig>),
    Creator {
        release: Box<chain_base::creator_open_service::Release>,
        rpc: String,
    },
}
impl Reader {
    fn from_env() -> anyhow::Result<Self> {
        let protocol = env::var("BASE_INDEXER_PROTOCOL").unwrap_or_default();
        match protocol.as_str() {
            "autonomous-v1" | "autonomous-v2" => {
                let config = AutonomousIndexerConfig::from_env_for_protocol(&protocol)
                    .map_err(|_| anyhow::anyhow!("Invalid recovery indexer configuration"))?;
                let factory = if protocol == "autonomous-v1" {
                    service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_FACTORY
                } else {
                    service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_FACTORY
                };
                ensure!(
                    config.network == "base-mainnet"
                        && config.factory_contract.eq_ignore_ascii_case(factory),
                    "Recovery indexer requires the recorded mainnet factory"
                );
                ensure!(
                    protocol != "autonomous-v2" || config.start_block == Some(52_275_492),
                    "Recovery v2 indexer requires its recorded deployment block"
                );
                Ok(Self::Autonomous(Box::new(config)))
            }
            "creator-open-v1" => {
                let raw = env::var("CREATOR_OPEN_RELEASE_JSON")
                    .map_err(|_| anyhow::anyhow!("Recovery creator release missing"))?;
                let release: chain_base::creator_open_service::Release = serde_json::from_str(&raw)
                    .map_err(|_| anyhow::anyhow!("Recovery creator release invalid"))?;
                release
                    .validate()
                    .map_err(|_| anyhow::anyhow!("Recovery creator release invalid"))?;
                ensure!(
                    release.network == "base-mainnet",
                    "Recovery creator release requires mainnet"
                );
                let rpc = env::var("BASE_MAINNET_RPC_URL")
                    .map_err(|_| anyhow::anyhow!("Recovery creator RPC missing"))?;
                ensure!(!rpc.is_empty(), "Recovery creator RPC missing");
                Ok(Self::Creator {
                    release: Box::new(release),
                    rpc,
                })
            }
            _ => bail!("Recovery accepts only autonomous-v1, autonomous-v2 or creator-open-v1"),
        }
    }
    async fn poll(&self, store: &PostgresStore) -> anyhow::Result<()> {
        match self {
            Self::Autonomous(config) => {
                let report =
                    worker::poll_autonomous_recovery_once_with_heartbeat(store, config).await?;
                println!("{}", serde_json::to_string(&report)?);
            }
            Self::Creator { release, rpc } => {
                let count = worker::poll_creator_open_indexer(store, release, rpc).await?;
                println!("creator_open_index_updated={count}");
            }
        }
        Ok(())
    }
    fn delay(&self) -> u64 {
        match self {
            Self::Autonomous(config) => config.poll_seconds,
            Self::Creator { .. } => 15,
        }
    }
    fn report_error(&self, error: &anyhow::Error, decision: IndexerRecoveryDecision) {
        match self {
            Self::Creator { release, .. } => eprintln!(
                "{}",
                worker::creator_open_indexer_recovery_report(&release.network, error, decision)
            ),
            Self::Autonomous(config) => eprintln!(
                "{}",
                serde_json::json!({
                    "schema":"agent-bounties/indexer-recovery-v1", "network":config.network,
                    "factory_contract":config.factory_contract,
                    "error":worker::redact_operational_error(&error.to_string()),
                    "safe_head_observation":worker::indexer_safe_head_observation(error), "decision":decision,
                    "evidence_boundary":"Recovery ingests chain evidence without notification enqueue or signing. Existing quarantine remains authoritative."
                })
            ),
        }
    }
}

async fn shutdown() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! { value = tokio::signal::ctrl_c() => value?, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
async fn wait(mut seconds: u64) -> anyhow::Result<bool> {
    loop {
        let chunk = seconds.min(86_400);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(chunk)) => {},
            result = shutdown() => { result?; return Ok(true); }
        }
        seconds -= chunk;
        if seconds == 0 {
            return Ok(false);
        }
    }
}

pub async fn run() -> anyhow::Result<()> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let binding = compiled_binding()?;
    if arguments == ["--build-identity"] {
        println!(
            "{}",
            serde_json::json!({"schema":"agentizable/recovery-reader-build-v1", "binding":binding, "unbound_inert_only":binding.is_none()})
        );
        return Ok(());
    }
    ensure!(
        arguments.is_empty() || arguments == ["--once"] || arguments == ["--check-binding"],
        "Recovery reader arguments rejected"
    );
    let once = arguments == ["--once"];
    let mode = env::var("RECOVERY_READER_MODE").unwrap_or_else(|_| "standby".into());
    ensure!(
        matches!(mode.as_str(), "standby" | "active"),
        "Invalid recovery reader mode"
    );
    let active = mode == "active";
    match binding {
        Some(binding) => binding.runtime(
            env::var("RENDER_SERVICE_ID").ok().as_deref(),
            env::var("RENDER_GIT_COMMIT").ok().as_deref(),
            active,
        )?,
        None => ensure!(!active, "Unbound recovery artifact is inert only"),
    }
    if arguments == ["--check-binding"] {
        return Ok(());
    }
    if !active {
        ensure!(!once, "Standby does not accept an indexing request");
        return shutdown().await;
    }
    let reader = Reader::from_env()?;
    let policy = IndexerRecoveryPolicy::from_env()
        .map_err(|_| anyhow::anyhow!("Invalid recovery policy"))?;
    let database =
        env::var("DATABASE_URL").map_err(|_| anyhow::anyhow!("Recovery database missing"))?;
    let store = tokio::time::timeout(Duration::from_secs(10), PostgresStore::connect(&database))
        .await
        .map_err(|_| anyhow::anyhow!("Recovery database unavailable"))?
        .map_err(|_| anyhow::anyhow!("Recovery database unavailable"))?;
    let mut state = IndexerRecoveryState::default();
    loop {
        let result = tokio::select! { result = reader.poll(&store) => result, stopped = shutdown() => { stopped?; return Ok(()); } };
        let delay = match result {
            Ok(()) => {
                state.on_success();
                if once {
                    return Ok(());
                }
                reader.delay()
            }
            Err(error) => {
                let decision = state.on_error(&policy, &error);
                reader.report_error(&error, decision);
                if once {
                    bail!("Recovery index poll failed; inspect redacted report");
                }
                match decision {
                    IndexerRecoveryDecision::RetryFromPersistedCursor {
                        backoff_seconds, ..
                    } => backoff_seconds.max(reader.delay()),
                    IndexerRecoveryDecision::ExitForSupervisorRestart { .. } => {
                        bail!("Recovery indexer exhausted its recovery budget")
                    }
                    IndexerRecoveryDecision::HaltForOperatorInvestigation { .. } => loop {
                        if wait(86_400).await? {
                            return Ok(());
                        }
                    },
                }
            }
        };
        if wait(delay).await? {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: &str = "srv-aaaaaaaaaaaaaaaaaaaa";
    const B: &str = "srv-bbbbbbbbbbbbbbbbbbbb";
    const REV: &str = "1111111111111111111111111111111111111111";
    #[test]
    fn identity_and_capability_bind_before_any_dependency() {
        let active = Binding::checked(A, REV, "reader").unwrap();
        assert!(active.runtime(Some(A), Some(REV), true).is_ok());
        for service in [None, Some(""), Some(B)] {
            assert!(active.runtime(service, Some(REV), true).is_err());
        }
        for revision in [None, Some("wrong")] {
            assert!(active.runtime(Some(A), revision, true).is_err());
        }
        let standby = Binding::checked(A, REV, "standby").unwrap();
        assert!(standby.runtime(Some(A), Some(REV), false).is_ok());
        assert!(standby.runtime(Some(A), Some(REV), true).is_err());
    }
    #[test]
    fn old_services_and_malformed_build_inputs_fail_closed() {
        for id in DENIED.lines().chain([
            "",
            "srv-;touch /tmp/invalid",
            "srv-AAAAAAAAAAAAAAAAAAAA",
            "not-a-service",
        ]) {
            assert!(Binding::checked(id, REV, "reader").is_err());
        }
        for revision in ["", "short", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
            assert!(Binding::checked(A, revision, "reader").is_err());
        }
        for cap in ["", "true", "READER"] {
            assert!(Binding::checked(A, REV, cap).is_err());
        }
    }
}
