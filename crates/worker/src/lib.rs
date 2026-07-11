use anyhow::{anyhow, Context};
use app::BountyNetwork;
use chain_base::{
    autonomous_bounty_event_topics, base_network_descriptor, decode_autonomous_bounty_logs,
    fetch_base_contract_logs, fetch_base_multi_contract_logs, fetch_block_number,
    rpc_logs_to_evm_logs, AutonomousBountyEventKind, BaseContractLogQuery,
    BaseMultiContractLogQuery, BaseNetworkDescriptor,
};
use chrono::Utc;
use db::{BaseIndexerHeartbeat, PostgresStore};
use domain::{Submission, VerifierResult};
use ledger::Ledger;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use verifier_sdk::{VerificationInput, Verifier, VerifierResultType};

const AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE: usize = 500;
const INDEXER_HEARTBEAT_SUCCESS: &str = "success";
const INDEXER_HEARTBEAT_SKIPPED: &str = "skipped";
const INDEXER_HEARTBEAT_FAILED: &str = "failed";

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
            let error_message = error.to_string();
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
                error_message: Some(error_message),
                updated_at: completed_at,
            };
            if let Err(heartbeat_error) = store.upsert_base_indexer_heartbeat(&heartbeat).await {
                return Err(error).context(format!(
                    "failed to persist autonomous Base indexer failure heartbeat: {heartbeat_error}"
                ));
            }
            Err(error)
        }
    }
}

pub async fn poll_autonomous_indexer_once(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
) -> anyhow::Result<AutonomousIndexerPollReport> {
    let descriptor = config.network_descriptor()?;
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
    for event in &events {
        store
            .upsert_autonomous_bounty_event(&config.network, event)
            .await?;
    }
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

pub async fn hydrate_bounty_network(store: &PostgresStore) -> anyhow::Result<BountyNetwork> {
    Ok(BountyNetwork {
        agents: store
            .list_agents()
            .await?
            .into_iter()
            .map(|agent| (agent.id, agent))
            .collect(),
        contributor_contacts: store
            .list_contributor_contacts()
            .await?
            .into_iter()
            .map(|contact| (contact.id, contact))
            .collect(),
        audience_members: store
            .list_audience_members()
            .await?
            .into_iter()
            .map(|member| (member.id, member))
            .collect(),
        audience_interactions: store
            .list_audience_interactions()
            .await?
            .into_iter()
            .map(|interaction| (interaction.id, interaction))
            .collect(),
        discovery_responses: store
            .list_discovery_responses()
            .await?
            .into_iter()
            .map(|response| (response.id, response))
            .collect(),
        outreach_attempts: store
            .list_outreach_attempts()
            .await?
            .into_iter()
            .map(|attempt| (attempt.id, attempt))
            .collect(),
        capabilities: store
            .list_capabilities()
            .await?
            .into_iter()
            .map(|capability| (capability.id, capability))
            .collect(),
        help_requests: store
            .list_help_requests()
            .await?
            .into_iter()
            .map(|request| (request.id, request))
            .collect(),
        quotes: store
            .list_quotes()
            .await?
            .into_iter()
            .map(|quote| (quote.id, quote))
            .collect(),
        bounties: store
            .list_bounties()
            .await?
            .into_iter()
            .map(|bounty| (bounty.id, bounty))
            .collect(),
        funding_intents: store
            .list_funding_intents()
            .await?
            .into_iter()
            .map(|intent| (intent.id, intent))
            .collect(),
        funding_contributions: store
            .list_funding_contributions()
            .await?
            .into_iter()
            .map(|contribution| (contribution.id, contribution))
            .collect(),
        escrows: store
            .list_escrows()
            .await?
            .into_iter()
            .map(|escrow| (escrow.id, escrow))
            .collect(),
        claims: store
            .list_claims()
            .await?
            .into_iter()
            .map(|claim| (claim.id, claim))
            .collect(),
        submissions: store
            .list_submissions()
            .await?
            .into_iter()
            .map(|submission| (submission.id, submission))
            .collect(),
        verifier_results: store
            .list_verifier_results()
            .await?
            .into_iter()
            .map(|result| (result.id, result))
            .collect(),
        proofs: store
            .list_proof_records()
            .await?
            .into_iter()
            .map(|proof| (proof.id, proof))
            .collect(),
        settlements: store
            .list_settlements()
            .await?
            .into_iter()
            .map(|settlement| (settlement.id, settlement))
            .collect(),
        reputation_events: store
            .list_reputation_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        template_signals: store
            .list_template_signals()
            .await?
            .into_iter()
            .map(|signal| (signal.id, signal))
            .collect(),
        risk_events: store
            .list_risk_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        risk_reviews: store
            .list_risk_reviews()
            .await?
            .into_iter()
            .map(|review| (review.id, review))
            .collect(),
        payment_events: store
            .list_payment_events()
            .await?
            .into_iter()
            .map(|event| (event.id, event))
            .collect(),
        ledger: Ledger::from_entries(store.list_ledger_entries().await?)
            .context("hydrate ledger from Postgres")?,
        ..BountyNetwork::default()
    })
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

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

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
}
