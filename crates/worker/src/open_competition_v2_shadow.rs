use super::{nonempty, OpenCompetitionV2IndexerConfig, AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE};
use anyhow::{anyhow, Context};
use chain_base::{
    decode_open_competition_v2_logs, fetch_base_contract_logs, fetch_base_multi_contract_logs,
    fetch_exact_block_identity, fetch_safe_block_identity, open_competition_v2_event_topics,
    rpc_logs_to_evm_logs, BaseContractLogQuery, BaseMultiContractLogQuery, OpenCompetitionV2Event,
    OpenCompetitionV2EventKind, OPEN_COMPETITION_V2_PROTOCOL_VERSION,
};
use chrono::Utc;
use db::{OpenCompetitionV2IndexerAgreement, PostgresStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ShadowConfig {
    pub indexer: OpenCompetitionV2IndexerConfig,
    pub shadow_rpc_url: String,
}

impl OpenCompetitionV2ShadowConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup<F>(lookup: F) -> anyhow::Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let indexer = OpenCompetitionV2IndexerConfig::from_lookup(|key| lookup(key))?;
        let shadow_rpc_url = lookup("OPEN_COMPETITION_V2_SHADOW_RPC_URL")
            .filter(|value| nonempty(value))
            .ok_or_else(|| anyhow!("OPEN_COMPETITION_V2_SHADOW_RPC_URL is required"))?;
        if shadow_rpc_url.trim() == indexer.rpc_url.trim() {
            return Err(anyhow!(
                "the V2 shadow RPC must be independent from the primary indexer RPC"
            ));
        }
        Ok(Self {
            indexer,
            shadow_rpc_url,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ShadowReport {
    pub protocol_version: String,
    pub network: String,
    pub factory_contract: String,
    pub common_safe_block: u64,
    pub primary_safe_head: u64,
    pub shadow_safe_head: u64,
    pub primary_block_hash: String,
    pub shadow_block_hash: String,
    pub primary_event_count: usize,
    pub shadow_event_count: usize,
    pub canonical_event_set_hash: String,
    pub agrees: bool,
    pub failure_code: Option<String>,
    pub evidence_boundary: String,
}

#[derive(Serialize)]
struct CanonicalEventIdentity<'a> {
    log_key: &'a str,
    tx_hash: &'a str,
    block_number: u64,
    log_index: u64,
    contract_address: &'a str,
    bounty_id: &'a str,
    kind: OpenCompetitionV2EventKind,
    data: &'a serde_json::Value,
}

pub async fn poll_open_competition_v2_shadow_once(
    store: &PostgresStore,
    config: &OpenCompetitionV2ShadowConfig,
) -> anyhow::Result<OpenCompetitionV2ShadowReport> {
    let primary_cursor = store
        .get_base_log_cursor(&config.indexer.network, &config.indexer.factory_contract)
        .await?
        .ok_or_else(|| anyhow!("the primary V2 indexer has not persisted a cursor"))?;
    let primary_safe =
        fetch_safe_block_identity(&config.indexer.rpc_url, config.indexer.request_id + 100_000)
            .await?;
    let shadow_safe =
        fetch_safe_block_identity(&config.shadow_rpc_url, config.indexer.request_id + 100_001)
            .await?;
    let common_safe_block = primary_cursor
        .last_scanned_block
        .min(primary_safe.number)
        .min(shadow_safe.number);
    if common_safe_block < config.indexer.deployment_block {
        return Err(anyhow!(
            "no common safe V2 block is available at or after deployment"
        ));
    }
    let primary_identity = fetch_exact_block_identity(
        &config.indexer.rpc_url,
        common_safe_block,
        config.indexer.request_id + 100_002,
    )
    .await?;
    let shadow_identity = fetch_exact_block_identity(
        &config.shadow_rpc_url,
        common_safe_block,
        config.indexer.request_id + 100_003,
    )
    .await?;
    let primary_events = store
        .list_open_competition_v2_events(&config.indexer.network, &config.indexer.factory_contract)
        .await?
        .into_iter()
        .filter(|event| event.block_number <= common_safe_block)
        .collect::<Vec<_>>();
    let shadow_events = fetch_shadow_events(config, common_safe_block).await?;
    let primary_hash = canonical_event_set_hash(&primary_events)?;
    let shadow_hash = canonical_event_set_hash(&shadow_events)?;
    let block_hashes_match = primary_identity
        .hash
        .eq_ignore_ascii_case(&shadow_identity.hash);
    let events_match = primary_hash == shadow_hash;
    let agrees = block_hashes_match && events_match;
    let failure_code = if !block_hashes_match {
        Some("safe_block_hash_mismatch".to_string())
    } else if !events_match {
        Some("canonical_event_set_mismatch".to_string())
    } else {
        None
    };
    store
        .upsert_open_competition_v2_indexer_agreement(&OpenCompetitionV2IndexerAgreement {
            network: config.indexer.network.clone(),
            factory_contract: config.indexer.factory_contract.clone(),
            protocol_version: OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
            common_safe_block,
            primary_safe_head: primary_safe.number,
            shadow_safe_head: shadow_safe.number,
            primary_block_hash: primary_identity.hash.clone(),
            shadow_block_hash: shadow_identity.hash.clone(),
            canonical_event_count: primary_events
                .len()
                .try_into()
                .context("canonical event count exceeds u64")?,
            canonical_event_set_hash: primary_hash.clone(),
            agrees,
            failure_code: failure_code.clone(),
            observed_at: Utc::now(),
        })
        .await?;
    Ok(OpenCompetitionV2ShadowReport {
        protocol_version: OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
        network: config.indexer.network.clone(),
        factory_contract: config.indexer.factory_contract.clone(),
        common_safe_block,
        primary_safe_head: primary_safe.number,
        shadow_safe_head: shadow_safe.number,
        primary_block_hash: primary_identity.hash,
        shadow_block_hash: shadow_identity.hash,
        primary_event_count: primary_events.len(),
        shadow_event_count: shadow_events.len(),
        canonical_event_set_hash: primary_hash,
        agrees,
        failure_code,
        evidence_boundary: "Agreement means two independent RPCs returned the same safe block and canonical V2 event set. Only CompetitionSettledV2 proves payment.".to_string(),
    })
}

async fn fetch_shadow_events(
    config: &OpenCompetitionV2ShadowConfig,
    to_block: u64,
) -> anyhow::Result<Vec<OpenCompetitionV2Event>> {
    let topics = open_competition_v2_event_topics();
    let mut events = Vec::new();
    let mut from_block = config.indexer.deployment_block;
    let mut request_id = config.indexer.request_id + 110_000;
    while from_block <= to_block {
        let end = query_end(from_block, to_block, config.indexer.max_blocks_per_query);
        let query = BaseContractLogQuery::new(
            &config.indexer.factory_contract,
            from_block,
            Some(end),
            topics.clone(),
        )?;
        let logs = rpc_logs_to_evm_logs(
            fetch_base_contract_logs(&config.shadow_rpc_url, &query, request_id)
                .await?
                .result,
        )?;
        events.extend(decode_open_competition_v2_logs(logs)?);
        request_id = request_id.saturating_add(1);
        if end == u64::MAX {
            break;
        }
        from_block = end + 1;
    }
    let mut competitions = events
        .iter()
        .filter(|event| event.kind == OpenCompetitionV2EventKind::CanonicalCompetitionCreated)
        .map(|event| {
            event.data["competition"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .ok_or_else(|| anyhow!("V2 creation event is missing competition"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    competitions.sort();
    competitions.dedup();
    for batch in competitions.chunks(AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE) {
        let mut from_block = config.indexer.deployment_block;
        while from_block <= to_block {
            let end = query_end(from_block, to_block, config.indexer.max_blocks_per_query);
            let query = BaseMultiContractLogQuery::new(
                batch.iter().cloned(),
                from_block,
                Some(end),
                topics.clone(),
            )?;
            let logs = rpc_logs_to_evm_logs(
                fetch_base_multi_contract_logs(&config.shadow_rpc_url, &query, request_id)
                    .await?
                    .result,
            )?;
            events.extend(decode_open_competition_v2_logs(logs)?);
            request_id = request_id.saturating_add(1);
            if end == u64::MAX {
                break;
            }
            from_block = end + 1;
        }
    }
    events.sort_by_key(|event| (event.block_number, event.log_index));
    let mut seen = HashSet::new();
    events.retain(|event| seen.insert(event.log_key.clone()));
    Ok(events)
}

fn query_end(from_block: u64, to_block: u64, max_blocks: u64) -> u64 {
    from_block
        .saturating_add(max_blocks.saturating_sub(1))
        .min(to_block)
}

fn canonical_event_set_hash(events: &[OpenCompetitionV2Event]) -> anyhow::Result<String> {
    let identities = events
        .iter()
        .map(|event| CanonicalEventIdentity {
            log_key: &event.log_key,
            tx_hash: &event.tx_hash,
            block_number: event.block_number,
            log_index: event.log_index,
            contract_address: &event.contract_address,
            bounty_id: &event.bounty_id,
            kind: event.kind,
            data: &event.data,
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "0x{}",
        hex::encode(Sha256::digest(serde_json::to_vec(&identities)?))
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shadow_rpc_must_be_independent() {
        let result = OpenCompetitionV2ShadowConfig::from_lookup(|key| match key {
            "OPEN_COMPETITION_V2_INDEXER_NETWORK" => Some("base-sepolia".to_string()),
            "OPEN_COMPETITION_V2_FACTORY_CONTRACT" => {
                Some("0x1111111111111111111111111111111111111111".to_string())
            }
            "OPEN_COMPETITION_V2_INDEXER_RPC_URL" | "OPEN_COMPETITION_V2_SHADOW_RPC_URL" => {
                Some("https://rpc.example".to_string())
            }
            "OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK" => Some("1".to_string()),
            _ => None,
        });
        assert!(result.is_err());
    }

    #[test]
    fn event_hash_ignores_observation_time_and_binds_payload() {
        let event = OpenCompetitionV2Event {
            id: uuid::Uuid::nil(),
            protocol_version: OPEN_COMPETITION_V2_PROTOCOL_VERSION.to_string(),
            log_key: "1:0".to_string(),
            tx_hash: format!("0x{}", "11".repeat(32)),
            block_number: 1,
            log_index: 0,
            contract_address: "0x1111111111111111111111111111111111111111".to_string(),
            bounty_id: format!("0x{}", "22".repeat(32)),
            kind: OpenCompetitionV2EventKind::FundingAdded,
            data: serde_json::json!({"amount": 1}),
            occurred_at: Utc::now(),
        };
        let mut observed_later = event.clone();
        observed_later.occurred_at = Utc::now() + chrono::Duration::seconds(10);
        assert_eq!(
            canonical_event_set_hash(&[event.clone()]).unwrap(),
            canonical_event_set_hash(&[observed_later]).unwrap()
        );
        let mut changed = event;
        changed.data = serde_json::json!({"amount": 2});
        assert_ne!(
            canonical_event_set_hash(&[changed]).unwrap(),
            canonical_event_set_hash(&[]).unwrap()
        );
    }
}
