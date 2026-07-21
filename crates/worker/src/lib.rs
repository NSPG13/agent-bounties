use anyhow::{anyhow, Context};
use app::BountyNetwork;
use chain_base::{
    autonomous_bounty_event_topics, base_network_descriptor, build_autonomous_bounty_feed,
    decode_autonomous_bounty_logs, fetch_base_contract_logs, fetch_base_multi_contract_logs,
    fetch_block_number, fetch_block_timestamp, redact_provider_urls, rpc_logs_to_evm_logs,
    AutonomousBountyEvent, AutonomousBountyEventKind, BaseContractLogQuery,
    BaseMultiContractLogQuery, BaseNetworkDescriptor, ChainBaseError,
};
use chrono::{DateTime, Utc};
use db::{BaseIndexerHeartbeat, DbError, PostgresStore};
use domain::{
    AgentWebhookEventType, DiscoveryOpportunitySnapshot, DiscoveryRewardFilter, Submission,
    VerifierResult,
};
use hmac::{Hmac, Mac};
use reqwest::{redirect::Policy, Url};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::HashSet,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};
use tokio::net::lookup_host;
use verifier_sdk::{VerificationInput, Verifier, VerifierResultType};

mod regression_sandbox;
pub use regression_sandbox::*;

const AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE: usize = 500;
const INDEXER_HEARTBEAT_SUCCESS: &str = "success";
const INDEXER_HEARTBEAT_SKIPPED: &str = "skipped";
const INDEXER_HEARTBEAT_FAILED: &str = "failed";

pub fn redact_operational_error(message: &str) -> String {
    redact_provider_urls(message)
}

#[derive(Debug)]
struct IndexerOperationalError {
    message: String,
    retryable: bool,
}

impl fmt::Display for IndexerOperationalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for IndexerOperationalError {}

pub fn indexer_error_is_retryable(error: &anyhow::Error) -> bool {
    if let Some(error) = error.downcast_ref::<IndexerOperationalError>() {
        return error.retryable;
    }
    if let Some(error) = error.downcast_ref::<ChainBaseError>() {
        return matches!(
            error,
            ChainBaseError::RpcTransport(_)
                | ChainBaseError::RpcHttpStatus(_)
                | ChainBaseError::RpcProviderError { .. }
        );
    }
    matches!(error.downcast_ref::<DbError>(), Some(DbError::Sqlx(_)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexerRecoveryPolicy {
    pub initial_backoff_seconds: u64,
    pub max_backoff_seconds: u64,
    pub exit_after_consecutive_failures: u32,
}

impl Default for IndexerRecoveryPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_seconds: 5,
            max_backoff_seconds: 120,
            exit_after_consecutive_failures: 8,
        }
    }
}

impl IndexerRecoveryPolicy {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> anyhow::Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let defaults = Self::default();
        let initial_backoff_seconds = optional_positive_u64(
            &lookup,
            "BASE_INDEXER_RETRY_INITIAL_SECONDS",
            defaults.initial_backoff_seconds,
        )?;
        let max_backoff_seconds = optional_positive_u64(
            &lookup,
            "BASE_INDEXER_RETRY_MAX_SECONDS",
            defaults.max_backoff_seconds,
        )?;
        let exit_after_consecutive_failures = optional_positive_u64(
            &lookup,
            "BASE_INDEXER_EXIT_AFTER_FAILURES",
            u64::from(defaults.exit_after_consecutive_failures),
        )?
        .try_into()
        .context("BASE_INDEXER_EXIT_AFTER_FAILURES exceeds u32")?;

        if max_backoff_seconds < initial_backoff_seconds {
            return Err(anyhow!(
                "BASE_INDEXER_RETRY_MAX_SECONDS must be greater than or equal to BASE_INDEXER_RETRY_INITIAL_SECONDS"
            ));
        }

        Ok(Self {
            initial_backoff_seconds,
            max_backoff_seconds,
            exit_after_consecutive_failures,
        })
    }

    pub fn decision(&self, consecutive_failures: u32, retryable: bool) -> IndexerRecoveryDecision {
        if !retryable {
            return IndexerRecoveryDecision::HaltForOperatorInvestigation {
                consecutive_failures,
            };
        }
        if consecutive_failures >= self.exit_after_consecutive_failures {
            return IndexerRecoveryDecision::ExitForSupervisorRestart {
                consecutive_failures,
            };
        }

        let exponent = consecutive_failures.saturating_sub(1).min(63);
        let multiplier = 1u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let backoff_seconds = self
            .initial_backoff_seconds
            .saturating_mul(multiplier)
            .min(self.max_backoff_seconds);
        IndexerRecoveryDecision::RetryFromPersistedCursor {
            consecutive_failures,
            backoff_seconds,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum IndexerRecoveryDecision {
    RetryFromPersistedCursor {
        consecutive_failures: u32,
        backoff_seconds: u64,
    },
    ExitForSupervisorRestart {
        consecutive_failures: u32,
    },
    HaltForOperatorInvestigation {
        consecutive_failures: u32,
    },
}

pub struct VerificationJob<V: Verifier> {
    pub verifier: V,
    pub input: VerificationInput,
}

impl<V: Verifier> VerificationJob<V> {
    pub async fn run(self) -> VerifierResultType<VerifierResult> {
        self.verifier.verify(self.input).await
    }
}

pub fn submission_summary(submission: &Submission) -> String {
    format!(
        "submission={} bounty={} artifact_digest={}",
        submission.id, submission.bounty_id, submission.artifact_digest
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutonomousIndexerConfig {
    pub network: String,
    pub rpc_url: String,
    pub factory_contract: String,
    pub start_block: Option<u64>,
    pub poll_seconds: u64,
    pub confirmations: u64,
    pub max_blocks_per_query: u64,
    pub request_id: u64,
    pub public_base_url: String,
}

impl AutonomousIndexerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> anyhow::Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let requested_network = lookup("BASE_INDEXER_NETWORK")
            .filter(|value| nonempty(value))
            .unwrap_or_else(|| "base-mainnet".to_string());
        let descriptor = base_network_descriptor(&requested_network)?;
        let network = canonical_base_network(&descriptor);
        let factory_contract_env = factory_contract_env_for_network(&descriptor)?;
        let factory_contract = lookup("BASE_INDEXER_FACTORY_CONTRACT")
            .filter(|value| nonempty(value))
            .or_else(|| lookup(factory_contract_env).filter(|value| nonempty(value)))
            .ok_or_else(|| {
                anyhow!(
                    "set BASE_INDEXER_FACTORY_CONTRACT or {factory_contract_env} before running the autonomous Base indexer"
                )
            })?;
        let rpc_url = lookup("BASE_INDEXER_RPC_URL")
            .filter(|value| nonempty(value))
            .or_else(|| lookup(&descriptor.rpc_url_env).filter(|value| nonempty(value)))
            .ok_or_else(|| {
                anyhow!(
                    "set BASE_INDEXER_RPC_URL or {} before running the Base indexer",
                    descriptor.rpc_url_env
                )
            })?;
        let start_block = lookup("BASE_INDEXER_START_BLOCK")
            .or_else(|| lookup("BASE_INDEXER_FROM_BLOCK"))
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_START_BLOCK", &value))
            .transpose()?;
        let poll_seconds = lookup("BASE_INDEXER_POLL_SECONDS")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_POLL_SECONDS", &value))
            .transpose()?
            .unwrap_or(15);
        let confirmations = lookup("BASE_INDEXER_CONFIRMATIONS")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_CONFIRMATIONS", &value))
            .transpose()?
            .unwrap_or(2);
        let max_blocks_per_query = lookup("BASE_INDEXER_MAX_BLOCKS_PER_QUERY")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_MAX_BLOCKS_PER_QUERY", &value))
            .transpose()?
            .unwrap_or(2_000)
            .max(1);
        let request_id = lookup("BASE_INDEXER_REQUEST_ID")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("BASE_INDEXER_REQUEST_ID", &value))
            .transpose()?
            .unwrap_or(1);
        let public_base_url = lookup("PUBLIC_BASE_URL")
            .filter(|value| nonempty(value))
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_string());
        let factory_contract = BaseContractLogQuery::new(
            factory_contract,
            start_block.unwrap_or(0),
            None,
            autonomous_bounty_event_topics(),
        )?
        .contract;
        Ok(Self {
            network,
            rpc_url,
            factory_contract,
            start_block,
            poll_seconds,
            confirmations,
            max_blocks_per_query,
            request_id,
            public_base_url,
        })
    }

    pub fn network_descriptor(&self) -> anyhow::Result<BaseNetworkDescriptor> {
        Ok(base_network_descriptor(&self.network)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomousIndexerPollReport {
    pub network: BaseNetworkDescriptor,
    pub factory_contract: String,
    pub latest_block: u64,
    pub confirmations: u64,
    pub confirmed_to_block: Option<u64>,
    pub from_block: Option<u64>,
    pub to_block: Option<u64>,
    pub canonical_bounty_contracts: usize,
    pub fetched_logs: usize,
    pub persisted_events: usize,
    pub persisted_cursor_block: Option<u64>,
    pub skipped_reason: Option<String>,
}

pub async fn poll_autonomous_indexer_once_with_heartbeat(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
) -> anyhow::Result<AutonomousIndexerPollReport> {
    let started_at = Utc::now();
    match poll_autonomous_indexer_once(store, config).await {
        Ok(report) => {
            let completed_at = Utc::now();
            let status = if report.skipped_reason.is_some() {
                INDEXER_HEARTBEAT_SKIPPED
            } else {
                INDEXER_HEARTBEAT_SUCCESS
            };
            store
                .upsert_base_indexer_heartbeat(&BaseIndexerHeartbeat {
                    network: config.network.clone(),
                    escrow_contract: config.factory_contract.clone(),
                    status: status.to_string(),
                    started_at,
                    completed_at: Some(completed_at),
                    latest_block: Some(report.latest_block),
                    confirmed_to_block: report.confirmed_to_block,
                    from_block: report.from_block,
                    to_block: report.to_block,
                    fetched_logs: report.fetched_logs as u64,
                    persisted_cursor_block: report.persisted_cursor_block,
                    skipped_reason: report.skipped_reason.clone(),
                    error_message: None,
                    updated_at: completed_at,
                })
                .await?;
            Ok(report)
        }
        Err(error) => {
            let completed_at = Utc::now();
            let retryable = indexer_error_is_retryable(&error);
            let error_message = redact_operational_error(&error.to_string());
            let heartbeat = BaseIndexerHeartbeat {
                network: config.network.clone(),
                escrow_contract: config.factory_contract.clone(),
                status: INDEXER_HEARTBEAT_FAILED.to_string(),
                started_at,
                completed_at: Some(completed_at),
                latest_block: None,
                confirmed_to_block: None,
                from_block: None,
                to_block: None,
                fetched_logs: 0,
                persisted_cursor_block: None,
                skipped_reason: None,
                error_message: Some(error_message.clone()),
                updated_at: completed_at,
            };
            if let Err(heartbeat_error) = store.upsert_base_indexer_heartbeat(&heartbeat).await {
                return Err(anyhow::Error::new(IndexerOperationalError {
                    message: format!(
                        "{error_message}; failed to persist autonomous Base indexer failure heartbeat: {}",
                        redact_operational_error(&heartbeat_error.to_string())
                    ),
                    retryable: retryable && matches!(heartbeat_error, DbError::Sqlx(_)),
                }));
            }
            Err(anyhow::Error::new(IndexerOperationalError {
                message: error_message,
                retryable,
            }))
        }
    }
}

pub async fn poll_autonomous_indexer_once(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
) -> anyhow::Result<AutonomousIndexerPollReport> {
    let descriptor = config.network_descriptor()?;
    backfill_autonomous_event_block_times(store, config).await?;
    let latest_block = fetch_block_number(&config.rpc_url, config.request_id).await?;
    let confirmed_to_block = latest_block.checked_sub(config.confirmations);
    let scan_cursor = store
        .get_base_log_cursor(&config.network, &config.factory_contract)
        .await?;
    let from_block = next_indexer_from_block(
        scan_cursor.as_ref().map(|cursor| cursor.last_scanned_block),
        None,
        config.start_block,
    )?;

    let skipped_report =
        |confirmed_to_block: Option<u64>, reason: &str| AutonomousIndexerPollReport {
            network: descriptor.clone(),
            factory_contract: config.factory_contract.clone(),
            latest_block,
            confirmations: config.confirmations,
            confirmed_to_block,
            from_block: Some(from_block),
            to_block: None,
            canonical_bounty_contracts: 0,
            fetched_logs: 0,
            persisted_events: 0,
            persisted_cursor_block: scan_cursor.as_ref().map(|cursor| cursor.last_scanned_block),
            skipped_reason: Some(reason.to_string()),
        };
    let Some(confirmed_to_block) = confirmed_to_block else {
        return Ok(skipped_report(
            None,
            "latest block is below configured confirmations",
        ));
    };
    if confirmed_to_block < from_block {
        return Ok(skipped_report(
            Some(confirmed_to_block),
            "no confirmed blocks are ready to scan",
        ));
    }

    let to_block = bounded_to_block(from_block, confirmed_to_block, config.max_blocks_per_query);
    let topics = autonomous_bounty_event_topics();
    let factory_query = BaseContractLogQuery::new(
        &config.factory_contract,
        from_block,
        Some(to_block),
        topics.clone(),
    )?;
    let factory_response = fetch_base_contract_logs(
        &config.rpc_url,
        &factory_query,
        config.request_id.saturating_add(1),
    )
    .await?;
    let factory_logs = rpc_logs_to_evm_logs(factory_response.result)?;
    let mut events = decode_autonomous_bounty_logs(factory_logs.clone())?;
    if events.iter().any(|event| {
        !event
            .contract_address
            .eq_ignore_ascii_case(&config.factory_contract)
            || !matches!(
                event.kind,
                AutonomousBountyEventKind::CanonicalBountyCreated
                    | AutonomousBountyEventKind::CanonicalBountyTermsCommitted
                    | AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured
                    | AutonomousBountyEventKind::CanonicalBountyVerificationConfigured
                    | AutonomousBountyEventKind::ExternalBountySubmitted
            )
    }) {
        return Err(anyhow!(
            "factory query returned a non-factory autonomous event"
        ));
    }

    let mut bounty_contracts = store
        .list_canonical_autonomous_bounty_contracts(&config.network, &config.factory_contract)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    for event in &events {
        if event.kind == AutonomousBountyEventKind::CanonicalBountyCreated {
            let address = event.data["bounty_contract"]
                .as_str()
                .ok_or_else(|| anyhow!("canonical creation event is missing bounty_contract"))?;
            let normalized =
                BaseContractLogQuery::new(address, from_block, Some(to_block), topics.clone())?
                    .contract;
            bounty_contracts.insert(normalized);
        }
    }

    let mut fetched_logs = factory_logs.len();
    let mut ordered_contracts = bounty_contracts.iter().cloned().collect::<Vec<_>>();
    ordered_contracts.sort();
    for (index, bounty_contract_batch) in ordered_contracts
        .chunks(AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE)
        .enumerate()
    {
        let query = BaseMultiContractLogQuery::new(
            bounty_contract_batch.iter().cloned(),
            from_block,
            Some(to_block),
            topics.clone(),
        )?;
        let request_id = config
            .request_id
            .checked_add(2 + index as u64)
            .ok_or_else(|| anyhow!("autonomous indexer request id overflowed"))?;
        let response = fetch_base_multi_contract_logs(&config.rpc_url, &query, request_id).await?;
        let logs = rpc_logs_to_evm_logs(response.result)?;
        fetched_logs += logs.len();
        let bounty_events = decode_autonomous_bounty_logs(logs)?;
        let expected_emitters = bounty_contract_batch
            .iter()
            .map(|address| address.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        if bounty_events.iter().any(|event| {
            !expected_emitters.contains(&event.contract_address.to_ascii_lowercase())
                || matches!(
                    event.kind,
                    AutonomousBountyEventKind::CanonicalBountyCreated
                        | AutonomousBountyEventKind::ExternalBountySubmitted
                )
        }) {
            return Err(anyhow!(
                "canonical bounty batch query returned an invalid emitter or factory event"
            ));
        }
        events.extend(bounty_events);
    }

    events.sort_by_key(|event| (event.block_number, event.log_index));
    let mut seen = HashSet::new();
    events.retain(|event| seen.insert(event.log_key.clone()));
    let mut event_blocks = events
        .iter()
        .map(|event| event.block_number)
        .collect::<Vec<_>>();
    event_blocks.sort_unstable();
    event_blocks.dedup();
    let mut block_times = std::collections::HashMap::new();
    for (index, block_number) in event_blocks.iter().copied().enumerate() {
        let timestamp = fetch_block_timestamp(
            &config.rpc_url,
            block_number,
            config
                .request_id
                .saturating_add(20_000)
                .saturating_add(index as u64),
        )
        .await?;
        block_times.insert(block_number, timestamp);
    }
    for event in &mut events {
        event.occurred_at = *block_times
            .get(&event.block_number)
            .ok_or_else(|| anyhow!("canonical event block timestamp is unavailable"))?;
        store
            .upsert_autonomous_bounty_event(&config.network, event)
            .await?;
    }
    for (&block_number, &occurred_at) in &block_times {
        store
            .confirm_autonomous_event_block_time(&config.network, block_number, occurred_at)
            .await?;
    }
    enqueue_canonical_discovery_events(store, &config.network, &config.public_base_url, &events)
        .await?;
    let last_log_key = events.last().map(|event| event.log_key.as_str());
    store
        .upsert_base_log_cursor(
            &config.network,
            &config.factory_contract,
            to_block,
            last_log_key,
        )
        .await?;

    Ok(AutonomousIndexerPollReport {
        network: descriptor,
        factory_contract: config.factory_contract.clone(),
        latest_block,
        confirmations: config.confirmations,
        confirmed_to_block: Some(confirmed_to_block),
        from_block: Some(from_block),
        to_block: Some(to_block),
        canonical_bounty_contracts: bounty_contracts.len(),
        fetched_logs,
        persisted_events: events.len(),
        persisted_cursor_block: Some(to_block),
        skipped_reason: None,
    })
}

async fn backfill_autonomous_event_block_times(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
) -> anyhow::Result<()> {
    let blocks = store
        .list_unverified_autonomous_event_blocks(&config.network, 50)
        .await?;
    for (index, block_number) in blocks.into_iter().enumerate() {
        let occurred_at = fetch_block_timestamp(
            &config.rpc_url,
            block_number,
            config
                .request_id
                .saturating_add(10_000)
                .saturating_add(index as u64),
        )
        .await?;
        store
            .confirm_autonomous_event_block_time(&config.network, block_number, occurred_at)
            .await?;
    }
    Ok(())
}

pub async fn hydrate_bounty_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    service_runtime::hydrate_bounty_network_with_ledger_context(
        store,
        "hydrate ledger from Postgres",
    )
    .await
}

pub fn next_indexer_from_block(
    scan_cursor_block: Option<u64>,
    event_cursor_block: Option<u64>,
    configured_start_block: Option<u64>,
) -> anyhow::Result<u64> {
    if let Some(block) = scan_cursor_block {
        return block
            .checked_add(1)
            .ok_or_else(|| anyhow!("base indexer cursor overflowed"));
    }
    if let Some(block) = event_cursor_block {
        return Ok(block);
    }
    configured_start_block.ok_or_else(|| {
        anyhow!(
            "set BASE_INDEXER_START_BLOCK for the first run; it should be the escrow contract deployment block"
        )
    })
}

pub fn bounded_to_block(
    from_block: u64,
    confirmed_to_block: u64,
    max_blocks_per_query: u64,
) -> u64 {
    let capped_end = from_block.saturating_add(max_blocks_per_query.saturating_sub(1));
    capped_end.min(confirmed_to_block)
}

fn canonical_base_network(descriptor: &BaseNetworkDescriptor) -> String {
    match descriptor.chain_id {
        8_453 => "base-mainnet".to_string(),
        84_532 => "base-sepolia".to_string(),
        other => other.to_string(),
    }
}

fn factory_contract_env_for_network(
    descriptor: &BaseNetworkDescriptor,
) -> anyhow::Result<&'static str> {
    match descriptor.chain_id {
        8_453 => Ok("BASE_MAINNET_BOUNTY_FACTORY"),
        84_532 => Ok("BASE_SEPOLIA_BOUNTY_FACTORY"),
        _ => Err(anyhow!("unsupported Base chain id {}", descriptor.chain_id)),
    }
}

fn parse_u64_env(name: &str, value: &str) -> anyhow::Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .with_context(|| format!("{name} must be a non-negative integer"))
}

fn optional_positive_u64<F>(lookup: &F, name: &str, default: u64) -> anyhow::Result<u64>
where
    F: Fn(&str) -> Option<String>,
{
    let value = lookup(name)
        .filter(|value| nonempty(value))
        .map(|value| parse_u64_env(name, &value))
        .transpose()?
        .unwrap_or(default);
    if value == 0 {
        return Err(anyhow!("{name} must be greater than zero"));
    }
    Ok(value)
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

const DISCOVERY_WEBHOOK_SCHEMA: &str = "agent-bounties/discovery-webhook-v1";
const DISCOVERY_WEBHOOK_MAX_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone)]
pub struct DiscoveryWebhookConfig {
    signing_key: Vec<u8>,
    pub request_timeout_seconds: u64,
    pub lease_seconds: u64,
    pub batch_size: u32,
}

impl DiscoveryWebhookConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(signing_key) = std::env::var("DISCOVERY_WEBHOOK_SIGNING_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
        else {
            return Ok(None);
        };
        if signing_key.len() < 32 {
            return Err(anyhow!(
                "DISCOVERY_WEBHOOK_SIGNING_KEY must contain at least 32 bytes"
            ));
        }
        Ok(Some(Self {
            signing_key: signing_key.into_bytes(),
            request_timeout_seconds: 10,
            lease_seconds: 30,
            batch_size: 25,
        }))
    }

    pub fn signing_key(&self) -> &[u8] {
        &self.signing_key
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryWebhookDispatchReport {
    pub leased: usize,
    pub delivered: usize,
    pub retried: usize,
    pub dead: usize,
}

pub fn derive_discovery_webhook_secret(
    master_key: &[u8],
    subscription_id: uuid::Uuid,
    secret_version: u32,
) -> anyhow::Result<String> {
    if master_key.len() < 32 {
        return Err(anyhow!("discovery webhook master key is too short"));
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(master_key)
        .map_err(|_| anyhow!("invalid discovery webhook master key"))?;
    mac.update(b"bountyboard.discovery.webhook.secret.v1\0");
    mac.update(subscription_id.as_bytes());
    mac.update(&secret_version.to_be_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

pub async fn validate_public_https_endpoint(endpoint: &str) -> anyhow::Result<Url> {
    let url = Url::parse(endpoint).context("webhook endpoint is not a valid URL")?;
    if url.scheme() != "https" {
        return Err(anyhow!("webhook endpoint must use https"));
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(anyhow!(
            "webhook endpoint cannot contain credentials or a fragment"
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("webhook endpoint must include a host"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = lookup_host((host, port))
        .await
        .context("webhook endpoint host could not be resolved")?
        .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
        return Err(anyhow!(
            "webhook endpoint must resolve only to public routable addresses"
        ));
    }
    Ok(url)
}

pub async fn enqueue_discovery_event(
    store: &PostgresStore,
    event_id: uuid::Uuid,
    event_type: AgentWebhookEventType,
    occurred_at: DateTime<Utc>,
    opportunity: &DiscoveryOpportunitySnapshot,
    data: serde_json::Value,
) -> anyhow::Result<usize> {
    let subscriptions = store.list_enabled_discovery_webhook_subscriptions().await?;
    let payload = serde_json::json!({
        "schema_version": DISCOVERY_WEBHOOK_SCHEMA,
        "event_id": event_id,
        "event_type": event_type,
        "occurred_at": occurred_at,
        "opportunity": opportunity,
        "data": data,
        "evidence_boundary": "This notification mirrors public discovery state. It does not prove funding, verification, settlement, payment, or agent independence. Confirm canonical claims against the authoritative source endpoint."
    });
    let mut enqueued = 0usize;
    for subscription in subscriptions {
        if subscription.filters.matches(opportunity, Utc::now())
            && store
                .enqueue_webhook_delivery(subscription.id, event_id, event_type, &payload)
                .await?
        {
            enqueued += 1;
        }
    }
    Ok(enqueued)
}

async fn enqueue_canonical_discovery_events(
    store: &PostgresStore,
    network: &str,
    public_base_url: &str,
    new_events: &[AutonomousBountyEvent],
) -> anyhow::Result<usize> {
    if new_events.is_empty() {
        return Ok(0);
    }
    let events = store.list_autonomous_bounty_events(network).await?;
    let terms = store.list_autonomous_bounty_terms().await?;
    let feed = build_autonomous_bounty_feed(events, terms, false)?;
    let mut enqueued = 0usize;
    for event in new_events {
        let Some(item) = feed
            .iter()
            .find(|item| item.bounty_id.eq_ignore_ascii_case(&event.bounty_id))
        else {
            continue;
        };
        let state = web_public::canonical_opportunity_state(item);
        let evidence = item
            .terms
            .as_ref()
            .map(|terms| terms.document.evidence_schema.clone())
            .unwrap_or(serde_json::Value::Null);
        let title = item
            .terms
            .as_ref()
            .map(|terms| terms.document.title.as_str())
            .unwrap_or(&item.bounty_id);
        let goal = item
            .terms
            .as_ref()
            .map(|terms| terms.document.goal.as_str());
        let (categories, skills) = web_public::discovery_taxonomy(title, goal, &evidence);
        let opportunity = DiscoveryOpportunitySnapshot {
            opportunity_id: format!("canonical:{network}:{}", item.bounty_contract),
            source_type: "canonical_base".to_string(),
            categories,
            skills,
            work_state: state.work_state,
            payment_state: state.payment_state,
            payment_committed: state.payment_committed,
            reward: DiscoveryRewardFilter {
                amount: item.solver_reward.clone(),
                currency: "USDC".to_string(),
                unit: "base_units".to_string(),
                decimals: 6,
            },
            deadline: state.deadline.as_deref().and_then(|deadline| {
                DateTime::parse_from_rfc3339(deadline)
                    .ok()
                    .map(|deadline| deadline.with_timezone(&Utc))
            }),
            verification_method: item.verification_mode.clone(),
            public_url: format!(
                "{}/v1/base/autonomous-bounties/events?network={network}&bounty_id={}",
                public_base_url.trim_end_matches('/'),
                item.bounty_id
            ),
        };
        let event_type = if event.kind == AutonomousBountyEventKind::CanonicalBountyCreated {
            AgentWebhookEventType::OpportunityPublished
        } else {
            AgentWebhookEventType::OpportunityStateChanged
        };
        enqueued += enqueue_discovery_event(
            store,
            event.id,
            event_type,
            event.occurred_at,
            &opportunity,
            serde_json::json!({
                "network": network,
                "canonical_event": event,
            }),
        )
        .await?;
    }
    Ok(enqueued)
}

pub async fn dispatch_discovery_webhooks_once(
    store: &PostgresStore,
    config: &DiscoveryWebhookConfig,
) -> anyhow::Result<DiscoveryWebhookDispatchReport> {
    let lease_token = uuid::Uuid::new_v4();
    let deliveries = store
        .lease_webhook_deliveries(config.batch_size, lease_token, config.lease_seconds)
        .await?;
    let mut report = DiscoveryWebhookDispatchReport {
        leased: deliveries.len(),
        ..DiscoveryWebhookDispatchReport::default()
    };
    for delivery in deliveries {
        let Some(subscription) = store
            .get_webhook_subscription(delivery.subscription_id)
            .await?
        else {
            continue;
        };
        let outcome = deliver_discovery_webhook(config, &subscription, &delivery).await;
        match outcome {
            Ok(status) => {
                store
                    .mark_webhook_delivery_delivered(delivery.id, lease_token, status)
                    .await?;
                report.delivered += 1;
            }
            Err((status, error)) => {
                let dead = delivery.attempt_count >= DISCOVERY_WEBHOOK_MAX_ATTEMPTS;
                let backoff =
                    60u64.saturating_mul(1u64 << delivery.attempt_count.saturating_sub(1).min(6));
                store
                    .reschedule_webhook_delivery(
                        delivery.id,
                        lease_token,
                        dead,
                        backoff,
                        status,
                        &redact_operational_error(&error),
                    )
                    .await?;
                if dead {
                    report.dead += 1;
                } else {
                    report.retried += 1;
                }
            }
        }
    }
    Ok(report)
}

async fn deliver_discovery_webhook(
    config: &DiscoveryWebhookConfig,
    subscription: &db::WebhookSubscription,
    delivery: &db::WebhookDelivery,
) -> Result<u16, (Option<u16>, String)> {
    let url = validate_public_https_endpoint(&subscription.endpoint_url)
        .await
        .map_err(|error| (None, error.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| (None, "webhook endpoint has no host".to_string()))?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = lookup_host((host.as_str(), port))
        .await
        .map_err(|error| (None, format!("webhook DNS resolution failed: {error}")))?
        .filter(|address| public_ip(address.ip()))
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err((
            None,
            "webhook endpoint has no public routable address".to_string(),
        ));
    }
    let secret = derive_discovery_webhook_secret(
        config.signing_key(),
        subscription.id,
        subscription.secret_version,
    )
    .map_err(|error| (None, error.to_string()))?;
    let body = serde_json::to_vec(&delivery.payload).map_err(|error| {
        (
            None,
            format!("webhook payload serialization failed: {error}"),
        )
    })?;
    let timestamp = Utc::now().timestamp().to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| (None, "invalid webhook signing secret".to_string()))?;
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(&body);
    let signature = format!("v1={}", hex::encode(mac.finalize().into_bytes()));
    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(config.request_timeout_seconds))
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|error| (None, format!("webhook client setup failed: {error}")))?;
    let response = client
        .post(url)
        .header("content-type", "application/json")
        .header("user-agent", "Agent Bounties-Discovery-Webhook/1.0")
        .header("x-bountyboard-timestamp", &timestamp)
        .header("x-bountyboard-signature", signature)
        .header("x-bountyboard-event-id", delivery.event_id.to_string())
        .header("idempotency-key", delivery.event_id.to_string())
        .body(body)
        .send()
        .await
        .map_err(|error| (None, format!("webhook request failed: {error}")))?;
    let status = response.status().as_u16();
    if response.status().is_success() {
        Ok(status)
    } else {
        Err((Some(status), format!("webhook returned HTTP {status}")))
    }
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_ipv4(ip),
        IpAddr::V6(ip) => public_ipv6(ip),
    }
}

fn public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 198 && (18..=19).contains(&b)))
}

fn public_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn discovery_webhook_secret_is_scoped_and_deterministic() {
        let master = [7u8; 32];
        let first_id = uuid::Uuid::new_v4();
        let second_id = uuid::Uuid::new_v4();
        let first = derive_discovery_webhook_secret(&master, first_id, 1).unwrap();
        assert_eq!(
            first,
            derive_discovery_webhook_secret(&master, first_id, 1).unwrap()
        );
        assert_ne!(
            first,
            derive_discovery_webhook_secret(&master, second_id, 1).unwrap()
        );
        assert_ne!(
            first,
            derive_discovery_webhook_secret(&master, first_id, 2).unwrap()
        );
        assert!(derive_discovery_webhook_secret(b"short", first_id, 1).is_err());
    }

    #[test]
    fn discovery_webhooks_reject_non_public_address_ranges() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip} must be rejected");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
        assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn discovery_webhooks_require_public_https_without_credentials() {
        assert!(validate_public_https_endpoint("http://example.com/hook")
            .await
            .is_err());
        assert!(
            validate_public_https_endpoint("https://user:pass@example.com/hook")
                .await
                .is_err()
        );
        assert!(validate_public_https_endpoint("https://127.0.0.1/hook")
            .await
            .is_err());
    }

    #[test]
    fn autonomous_indexer_requires_factory_and_defaults_to_mainnet() {
        let values = HashMap::from([
            ("BASE_MAINNET_RPC_URL", "https://base.example"),
            (
                "BASE_INDEXER_FACTORY_CONTRACT",
                "0x1111111111111111111111111111111111111111",
            ),
            ("BASE_INDEXER_START_BLOCK", "456"),
        ]);

        let config = AutonomousIndexerConfig::from_lookup(|key| {
            values.get(key).map(|value| value.to_string())
        })
        .unwrap();

        assert_eq!(config.network, "base-mainnet");
        assert_eq!(config.rpc_url, "https://base.example");
        assert_eq!(
            config.factory_contract,
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(config.start_block, Some(456));
    }

    #[test]
    fn base_indexer_requires_start_block_without_existing_cursors() {
        let err = next_indexer_from_block(None, None, None).unwrap_err();

        assert!(err.to_string().contains("BASE_INDEXER_START_BLOCK"));
        assert_eq!(next_indexer_from_block(Some(10), None, None).unwrap(), 11);
        assert_eq!(next_indexer_from_block(None, Some(10), None).unwrap(), 10);
        assert_eq!(next_indexer_from_block(None, None, Some(5)).unwrap(), 5);
    }

    #[test]
    fn base_indexer_caps_scan_ranges() {
        assert_eq!(bounded_to_block(100, 500, 50), 149);
        assert_eq!(bounded_to_block(100, 120, 50), 120);
        assert_eq!(bounded_to_block(100, 120, 0), 100);
    }

    #[test]
    fn recovery_policy_retries_with_capped_exponential_backoff() {
        let policy = IndexerRecoveryPolicy {
            initial_backoff_seconds: 5,
            max_backoff_seconds: 30,
            exit_after_consecutive_failures: 8,
        };

        assert_eq!(
            policy.decision(1, true),
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                consecutive_failures: 1,
                backoff_seconds: 5,
            }
        );
        assert_eq!(
            policy.decision(4, true),
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                consecutive_failures: 4,
                backoff_seconds: 30,
            }
        );
        assert_eq!(
            policy.decision(7, true),
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                consecutive_failures: 7,
                backoff_seconds: 30,
            }
        );
    }

    #[test]
    fn recovery_policy_exits_after_bounded_failures() {
        let policy = IndexerRecoveryPolicy::default();

        assert_eq!(
            policy.decision(8, true),
            IndexerRecoveryDecision::ExitForSupervisorRestart {
                consecutive_failures: 8,
            }
        );
        assert_eq!(
            policy.decision(u32::MAX, true),
            IndexerRecoveryDecision::ExitForSupervisorRestart {
                consecutive_failures: u32::MAX,
            }
        );
    }

    #[test]
    fn recovery_policy_halts_non_retryable_failures_immediately() {
        let policy = IndexerRecoveryPolicy::default();

        assert_eq!(
            policy.decision(1, false),
            IndexerRecoveryDecision::HaltForOperatorInvestigation {
                consecutive_failures: 1,
            }
        );
    }

    #[test]
    fn indexer_retries_only_typed_external_failures() {
        let transport = anyhow::Error::new(ChainBaseError::RpcHttpStatus(503));
        let malformed = anyhow::Error::new(ChainBaseError::InvalidRpcResponse(
            "missing result".to_string(),
        ));
        let integrity = anyhow!("factory query returned a non-factory autonomous event");

        assert!(indexer_error_is_retryable(&transport));
        assert!(!indexer_error_is_retryable(&malformed));
        assert!(!indexer_error_is_retryable(&integrity));
    }

    #[test]
    fn recovery_policy_rejects_zero_and_inverted_limits() {
        let zero = HashMap::from([("BASE_INDEXER_RETRY_INITIAL_SECONDS", "0")]);
        let error =
            IndexerRecoveryPolicy::from_lookup(|key| zero.get(key).map(|value| value.to_string()))
                .unwrap_err();
        assert!(error.to_string().contains("must be greater than zero"));

        let inverted = HashMap::from([
            ("BASE_INDEXER_RETRY_INITIAL_SECONDS", "60"),
            ("BASE_INDEXER_RETRY_MAX_SECONDS", "30"),
        ]);
        let error = IndexerRecoveryPolicy::from_lookup(|key| {
            inverted.get(key).map(|value| value.to_string())
        })
        .unwrap_err();
        assert!(error.to_string().contains("must be greater than or equal"));
    }

    #[test]
    fn operational_errors_redact_provider_urls() {
        let message = "RPC https://user:secret@rpc.example/v2/API_KEY?token=SECRET failed; fallback http://localhost:8545 and wss://rpc.example/SECRET unavailable";

        let redacted = redact_operational_error(message);

        assert_eq!(
            redacted,
            "RPC [redacted-url] failed; fallback [redacted-url] and [redacted-url] unavailable"
        );
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("API_KEY"));
    }
}
