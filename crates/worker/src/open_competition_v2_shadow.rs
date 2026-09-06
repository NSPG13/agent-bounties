use super::{
    nonempty, parse_u64_env, OpenCompetitionV2IndexerConfig, AUTONOMOUS_LOG_ADDRESS_BATCH_SIZE,
};
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
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenCompetitionV2ShadowConfig {
    pub indexer: OpenCompetitionV2IndexerConfig,
    pub shadow_rpc_url: String,
    pub request_delay_ms: u64,
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
        let request_delay_ms = lookup("OPEN_COMPETITION_V2_SHADOW_REQUEST_DELAY_MS")
            .filter(|value| nonempty(value))
            .map(|value| parse_u64_env("OPEN_COMPETITION_V2_SHADOW_REQUEST_DELAY_MS", &value))
            .transpose()?
            .unwrap_or(250)
            .min(5_000);
        Ok(Self {
            indexer,
            shadow_rpc_url,
            request_delay_ms,
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

/// A process-local optimization, populated only after independent verification
/// and a successful agreement write. It is never restored from primary events.
#[derive(Default)]
pub struct OpenCompetitionV2ShadowCache {
    verified: Option<VerifiedShadowPrefix>,
}

struct VerifiedShadowPrefix {
    config: OpenCompetitionV2ShadowConfig,
    block: u64,
    block_hash: String,
    events: Vec<OpenCompetitionV2Event>,
}

pub async fn poll_open_competition_v2_shadow_once(
    store: &PostgresStore,
    config: &OpenCompetitionV2ShadowConfig,
) -> anyhow::Result<OpenCompetitionV2ShadowReport> {
    poll_open_competition_v2_shadow_with_cache(
        store,
        config,
        &mut OpenCompetitionV2ShadowCache::default(),
    )
    .await
}

pub async fn poll_open_competition_v2_shadow_with_cache(
    store: &PostgresStore,
    config: &OpenCompetitionV2ShadowConfig,
    cache: &mut OpenCompetitionV2ShadowCache,
) -> anyhow::Result<OpenCompetitionV2ShadowReport> {
    // Taking the prefix makes every early error (including cancellation) leave
    // the next poll on the complete-replay path.
    let previous = cache.verified.take();
    let observed_at = Utc::now();
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
    let previous = reusable_prefix(previous, config, common_safe_block).await?;
    let shadow_events = fetch_shadow_events(config, common_safe_block, previous.as_ref()).await?;
    // A block reorganization during a scan must not certify logs fetched from
    // different histories. The target commits to the entire cached prefix too.
    let primary_after = fetch_exact_block_identity(
        &config.indexer.rpc_url,
        common_safe_block,
        config.indexer.request_id + 100_006,
    )
    .await?;
    let shadow_after = fetch_exact_block_identity(
        &config.shadow_rpc_url,
        common_safe_block,
        config.indexer.request_id + 100_007,
    )
    .await?;
    let primary_events = store
        .list_open_competition_v2_events(&config.indexer.network, &config.indexer.factory_contract)
        .await?
        .into_iter()
        .filter(|event| event.block_number <= common_safe_block)
        .collect::<Vec<_>>();
    let primary_hash = canonical_event_set_hash(&primary_events)?;
    let shadow_hash = canonical_event_set_hash(&shadow_events)?;
    let block_hashes_match = primary_identity
        .hash
        .eq_ignore_ascii_case(&shadow_identity.hash)
        && primary_identity
            .hash
            .eq_ignore_ascii_case(&primary_after.hash)
        && shadow_identity
            .hash
            .eq_ignore_ascii_case(&shadow_after.hash)
        && [
            primary_identity.number,
            shadow_identity.number,
            primary_after.number,
            shadow_after.number,
        ]
        .iter()
        .all(|number| *number == common_safe_block);
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
            observed_at,
        })
        .await?;
    let shadow_event_count = shadow_events.len();
    if agrees {
        cache.verified = Some(VerifiedShadowPrefix {
            config: config.clone(),
            block: common_safe_block,
            block_hash: primary_identity.hash.clone(),
            events: shadow_events,
        });
    }
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
        shadow_event_count,
        canonical_event_set_hash: primary_hash,
        agrees,
        failure_code,
        evidence_boundary: "Agreement means two independent RPCs returned the same safe block and canonical V2 event set. Only CompetitionSettledV2 proves payment.".to_string(),
    })
}

async fn reusable_prefix(
    previous: Option<VerifiedShadowPrefix>,
    config: &OpenCompetitionV2ShadowConfig,
    common_safe_block: u64,
) -> anyhow::Result<Option<VerifiedShadowPrefix>> {
    let Some(previous) = previous.filter(|prefix| {
        prefix.config == *config
            && prefix.block >= config.indexer.deployment_block
            && prefix.block <= common_safe_block
    }) else {
        return Ok(None);
    };
    let primary = fetch_exact_block_identity(
        &config.indexer.rpc_url,
        previous.block,
        config.indexer.request_id + 100_004,
    )
    .await?;
    let shadow = fetch_exact_block_identity(
        &config.shadow_rpc_url,
        previous.block,
        config.indexer.request_id + 100_005,
    )
    .await?;
    if primary.number != previous.block || shadow.number != previous.block {
        return Err(anyhow!(
            "shadow prefix anchor response has another block number"
        ));
    }
    if primary.hash.eq_ignore_ascii_case(&previous.block_hash)
        && shadow.hash.eq_ignore_ascii_case(&previous.block_hash)
    {
        Ok(Some(previous))
    } else {
        Ok(None)
    }
}

async fn fetch_shadow_events(
    config: &OpenCompetitionV2ShadowConfig,
    to_block: u64,
    previous: Option<&VerifiedShadowPrefix>,
) -> anyhow::Result<Vec<OpenCompetitionV2Event>> {
    let topics = open_competition_v2_event_topics();
    let mut events = previous
        .map(|prefix| prefix.events.clone())
        .unwrap_or_default();
    let first_block = match previous {
        Some(prefix) if prefix.block == to_block => return Ok(events),
        Some(prefix) => prefix
            .block
            .checked_add(1)
            .context("shadow prefix block overflow")?,
        None => config.indexer.deployment_block,
    };
    let mut from_block = first_block;
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
        events.extend(decode_shadow_range(
            logs,
            from_block,
            end,
            &[config.indexer.factory_contract.clone()],
        )?);
        pace_shadow_requests(config).await;
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
        let mut from_block = first_block;
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
            events.extend(decode_shadow_range(logs, from_block, end, batch)?);
            pace_shadow_requests(config).await;
            request_id = request_id.saturating_add(1);
            if end == u64::MAX {
                break;
            }
            from_block = end + 1;
        }
    }
    events.sort_by_key(|event| (event.block_number, event.log_index));
    let mut seen = HashMap::new();
    let mut unique = Vec::new();
    for event in events {
        let hash = canonical_event_set_hash(std::slice::from_ref(&event))?;
        if let Some(previous) = seen.insert(event.log_key.clone(), hash.clone()) {
            if previous != hash {
                return Err(anyhow!(
                    "shadow RPC returned conflicting canonical log identities"
                ));
            }
        } else {
            unique.push(event);
        }
    }
    Ok(unique)
}

fn decode_shadow_range(
    logs: Vec<chain_base::EvmLog>,
    from_block: u64,
    to_block: u64,
    addresses: &[String],
) -> anyhow::Result<Vec<OpenCompetitionV2Event>> {
    if logs.iter().any(|log| {
        !(from_block..=to_block).contains(&log.block_number)
            || !addresses
                .iter()
                .any(|address| address.eq_ignore_ascii_case(&log.address))
    }) {
        return Err(anyhow!(
            "shadow RPC returned a log outside the requested range or emitters"
        ));
    }
    Ok(decode_open_competition_v2_logs(logs)?)
}

async fn pace_shadow_requests(config: &OpenCompetitionV2ShadowConfig) {
    if config.request_delay_ms > 0 {
        sleep(Duration::from_millis(config.request_delay_ms)).await;
    }
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
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct RpcFixture {
        url: String,
        requests: Arc<Mutex<Vec<Value>>>,
        observed_at: Arc<Mutex<Vec<chrono::DateTime<Utc>>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for RpcFixture {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn rpc_fixture(respond: impl Fn(&Value) -> Value + Send + Sync + 'static) -> RpcFixture {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed_at = Arc::new(Mutex::new(Vec::new()));
        let timestamps = observed_at.clone();
        let recorded = requests.clone();
        let respond = Arc::new(respond);
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorded = recorded.clone();
                let timestamps = timestamps.clone();
                let respond = respond.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut buffer = [0; 4096];
                    let request = loop {
                        let read = stream.read(&mut buffer).await.unwrap();
                        assert!(read > 0);
                        bytes.extend_from_slice(&buffer[..read]);
                        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&bytes[..end]);
                            let length: usize = headers
                                .lines()
                                .find_map(|line| {
                                    let (key, value) = line.split_once(':')?;
                                    key.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse().unwrap())
                                })
                                .unwrap();
                            if bytes.len() >= end + 4 + length {
                                break serde_json::from_slice::<Value>(
                                    &bytes[end + 4..end + 4 + length],
                                )
                                .unwrap();
                            }
                        }
                    };
                    recorded.lock().unwrap().push(request.clone());
                    timestamps.lock().unwrap().push(Utc::now());
                    let body =
                        json!({"jsonrpc":"2.0","id":request["id"],"result":respond(&request)})
                            .to_string();
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    stream.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        RpcFixture {
            url,
            requests,
            observed_at,
            task,
        }
    }

    fn config(primary: &str, shadow: &str) -> OpenCompetitionV2ShadowConfig {
        OpenCompetitionV2ShadowConfig::from_lookup(|key| match key {
            "OPEN_COMPETITION_V2_INDEXER_NETWORK" => Some("base-sepolia".into()),
            "OPEN_COMPETITION_V2_FACTORY_CONTRACT" => Some(format!("0x{}", "11".repeat(20))),
            "OPEN_COMPETITION_V2_INDEXER_RPC_URL" => Some(primary.into()),
            "OPEN_COMPETITION_V2_SHADOW_RPC_URL" => Some(shadow.into()),
            "OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK" => Some("1".into()),
            "OPEN_COMPETITION_V2_SHADOW_REQUEST_DELAY_MS" => Some("0".into()),
            _ => None,
        })
        .unwrap()
    }

    fn hash() -> String {
        format!("0x{}", "aa".repeat(32))
    }

    fn block_response(request: &Value) -> Value {
        json!({"number":request["params"][0],"hash":hash(),"timestamp":"0x1"})
    }

    fn prefix(config: &OpenCompetitionV2ShadowConfig) -> VerifiedShadowPrefix {
        VerifiedShadowPrefix {
            config: config.clone(),
            block: 10,
            block_hash: hash(),
            events: vec![],
        }
    }

    fn creation_log(block: u64, competition_byte: &str) -> Value {
        let topic = hex::encode(sha3::Keccak256::digest(
            b"CanonicalCompetitionCreatedV2(bytes32,address,address,bytes32,bytes32)",
        ));
        json!({"address":format!("0x{}","11".repeat(20)),
            "topics":[format!("0x{topic}"),format!("0x{block:064x}"),
                format!("0x{}{}","00".repeat(12),competition_byte.repeat(20)),format!("0x{}{}","00".repeat(12),"33".repeat(20))],
            "data":format!("0x{}","00".repeat(64)),"transactionHash":format!("0x{block:064x}"),
            "blockNumber":format!("0x{block:x}"),"logIndex":"0x0"})
    }

    fn decoded_creation(block: u64, competition_byte: &str) -> OpenCompetitionV2Event {
        let log: chain_base::RpcEvmLog =
            serde_json::from_value(creation_log(block, competition_byte)).unwrap();
        decode_open_competition_v2_logs(rpc_logs_to_evm_logs([log]).unwrap())
            .unwrap()
            .remove(0)
    }

    #[tokio::test]
    async fn cold_replay_reads_from_deployment_and_warm_replay_includes_new_competitions() {
        let rpc = rpc_fixture(|request| {
            let query = &request["params"][0];
            if query["address"].is_string() {
                json!([creation_log(11, "44")])
            } else {
                json!([])
            }
        })
        .await;
        let config = config("http://primary.invalid", &rpc.url);
        let cold = fetch_shadow_events(&config, 12, None).await.unwrap();
        assert_eq!(cold.len(), 1);
        assert!(rpc
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r["params"][0]["fromBlock"] == "0x1"));
        rpc.requests.lock().unwrap().clear();
        let mut old = prefix(&config);
        old.events.push(decoded_creation(2, "22"));
        let warm = fetch_shadow_events(&config, 12, Some(&old)).await.unwrap();
        assert_eq!(warm.len(), 2);
        let queries = rpc.requests.lock().unwrap();
        assert_eq!(queries.len(), 2);
        assert!(queries.iter().all(|r| r["params"][0]["fromBlock"] == "0xb"));
        let addresses = queries[1]["params"][0]["address"].as_array().unwrap();
        assert!(addresses.contains(&json!(format!("0x{}", "22".repeat(20)))));
        assert!(addresses.contains(&json!(format!("0x{}", "44".repeat(20)))));
        assert_eq!(
            canonical_event_set_hash(&warm).unwrap(),
            canonical_event_set_hash(&[decoded_creation(2, "22"), decoded_creation(11, "44")])
                .unwrap()
        );
    }

    #[tokio::test]
    async fn verified_anchor_is_rechecked_on_both_providers_even_without_new_blocks() {
        let primary = rpc_fixture(block_response).await;
        let shadow = rpc_fixture(block_response).await;
        let config = config(&primary.url, &shadow.url);
        let old = reusable_prefix(Some(prefix(&config)), &config, 10)
            .await
            .unwrap()
            .unwrap();
        let events = fetch_shadow_events(&config, 10, Some(&old)).await.unwrap();
        assert!(events.is_empty());
        assert_eq!(primary.requests.lock().unwrap().len(), 1);
        assert_eq!(shadow.requests.lock().unwrap().len(), 1);
        assert_eq!(
            shadow.requests.lock().unwrap()[0]["method"],
            "eth_getBlockByNumber"
        );
    }

    #[tokio::test]
    async fn reorg_and_rpc_identity_changes_invalidate_the_verified_prefix() {
        let primary = rpc_fixture(block_response).await;
        let shadow = rpc_fixture(|request| {
            let mut block = block_response(request);
            block["hash"] = json!(format!("0x{}", "bb".repeat(32)));
            block
        })
        .await;
        let config = config(&primary.url, &shadow.url);
        assert!(reusable_prefix(Some(prefix(&config)), &config, 12)
            .await
            .unwrap()
            .is_none());
        let mut changed = config.clone();
        changed.shadow_rpc_url = "http://different.invalid".into();
        assert!(reusable_prefix(Some(prefix(&config)), &changed, 12)
            .await
            .unwrap()
            .is_none());
        assert!(reusable_prefix(Some(prefix(&config)), &config, 9)
            .await
            .unwrap()
            .is_none());
        assert_eq!(primary.requests.lock().unwrap().len(), 1);
        assert_eq!(shadow.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn malformed_anchor_response_is_an_error_instead_of_reusing_the_cache() {
        let primary = rpc_fixture(|_| Value::Null).await;
        let config = config(&primary.url, "http://shadow.invalid");
        assert!(reusable_prefix(Some(prefix(&config)), &config, 12)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn failed_poll_discards_the_prefix_before_any_database_or_rpc_work() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@localhost/unused")
            .unwrap();
        pool.close().await;
        let store = PostgresStore::from_pool(pool);
        let config = config("http://primary.invalid", "http://shadow.invalid");
        let mut cache = OpenCompetitionV2ShadowCache {
            verified: Some(prefix(&config)),
        };
        assert!(
            poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
                .await
                .is_err()
        );
        assert!(cache.verified.is_none());
    }

    #[tokio::test]
    async fn out_of_range_logs_cannot_be_appended_to_a_verified_prefix() {
        let rpc = rpc_fixture(|_| json!([creation_log(2, "22")])).await;
        let config = config("http://primary.invalid", &rpc.url);
        let result = fetch_shadow_events(&config, 12, Some(&prefix(&config))).await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("outside the requested range"));
    }

    #[tokio::test]
    async fn conflicting_duplicate_logs_are_rejected() {
        let rpc = rpc_fixture(|request| {
            if request["params"][0]["address"].is_string() {
                json!([creation_log(2, "22"), creation_log(2, "44")])
            } else {
                json!([])
            }
        })
        .await;
        let config = config("http://primary.invalid", &rpc.url);
        assert!(fetch_shadow_events(&config, 12, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("conflicting canonical log identities"));
    }

    #[tokio::test]
    #[ignore = "requires AGENT_BOUNTIES_TEST_DATABASE_URL"]
    async fn postgres_shadow_cache_certifies_full_sets_and_recovers_from_disagreement() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        let database_url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        let store = PostgresStore::connect(&database_url).await.unwrap();
        store.migrate().await.unwrap();
        let head = Arc::new(AtomicU64::new(10));
        let reorg = Arc::new(AtomicBool::new(false));
        let reorg_on_logs = Arc::new(AtomicBool::new(false));
        let logs = Arc::new(Mutex::new(Vec::<Value>::new()));
        let primary = {
            let head = head.clone();
            let reorg = reorg.clone();
            rpc_fixture(move |request| {
                let number = if request["params"][0] == "safe" { json!(format!("0x{:x}", head.load(Ordering::SeqCst))) } else { request["params"][0].clone() };
                json!({"number":number,"hash":if reorg.load(Ordering::SeqCst) {format!("0x{}","bb".repeat(32))} else {hash()},"timestamp":"0x1"})
            }).await
        };
        let shadow = {
            let head = head.clone();
            let logs = logs.clone();
            let reorg = reorg.clone();
            let reorg_on_logs = reorg_on_logs.clone();
            rpc_fixture(move |request| {
                if request["method"] == "eth_getLogs" {
                    if reorg_on_logs.load(Ordering::SeqCst) { reorg.store(true, Ordering::SeqCst); }
                    let query = &request["params"][0];
                    let from = u64::from_str_radix(query["fromBlock"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
                    let to = u64::from_str_radix(query["toBlock"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
                    return json!(logs.lock().unwrap().iter().filter(|log| {
                        let block = u64::from_str_radix(log["blockNumber"].as_str().unwrap().trim_start_matches("0x"),16).unwrap();
                        (from..=to).contains(&block) && (query["address"] == log["address"] || query["address"].as_array().is_some_and(|addresses| addresses.contains(&log["address"])))
                    }).cloned().collect::<Vec<_>>());
                }
                let number = if request["params"][0] == "safe" { json!(format!("0x{:x}", head.load(Ordering::SeqCst))) } else { request["params"][0].clone() };
                json!({"number":number,"hash":if reorg.load(Ordering::SeqCst) {format!("0x{}","bb".repeat(32))} else {hash()},"timestamp":"0x1"})
            }).await
        };
        let mut config = config(&primary.url, &shadow.url);
        config.indexer.factory_contract = format!("0x00000000{}", uuid::Uuid::new_v4().simple());
        async fn add(
            store: &PostgresStore,
            config: &OpenCompetitionV2ShadowConfig,
            block: u64,
        ) -> Value {
            let mut raw = creation_log(block, &format!("{block:02x}"));
            raw["address"] = json!(config.indexer.factory_contract);
            let log: chain_base::RpcEvmLog = serde_json::from_value(raw.clone()).unwrap();
            let event = decode_open_competition_v2_logs(rpc_logs_to_evm_logs([log]).unwrap())
                .unwrap()
                .remove(0);
            store
                .upsert_open_competition_v2_event(
                    &config.indexer.network,
                    &config.indexer.factory_contract,
                    &event,
                    &db::OpenCompetitionV2SafeContext {
                        block_hash: hash(),
                        safe_block_number: 20,
                        safe_block_hash: hash(),
                    },
                )
                .await
                .unwrap();
            raw
        }
        let first_log = add(&store, &config, 2).await;
        logs.lock().unwrap().push(first_log);
        store
            .upsert_base_log_cursor(
                &config.indexer.network,
                &config.indexer.factory_contract,
                10,
                None,
            )
            .await
            .unwrap();
        let mut cache = OpenCompetitionV2ShadowCache::default();
        let start = Utc::now();
        let cold = poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
            .await
            .unwrap();
        assert!(cold.agrees && cache.verified.is_some());
        let agreement = store
            .get_open_competition_v2_indexer_agreement(
                &config.indexer.network,
                &config.indexer.factory_contract,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(agreement.observed_at >= start && agreement.observed_at <= Utc::now());
        assert!(agreement.observed_at <= primary.observed_at.lock().unwrap()[0]);
        let next_log = add(&store, &config, 11).await;
        logs.lock().unwrap().push(next_log);
        head.store(12, Ordering::SeqCst);
        store
            .upsert_base_log_cursor(
                &config.indexer.network,
                &config.indexer.factory_contract,
                12,
                None,
            )
            .await
            .unwrap();
        shadow.requests.lock().unwrap().clear();
        let warm = poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
            .await
            .unwrap();
        assert!(warm.agrees);
        assert_eq!(
            warm.canonical_event_set_hash,
            canonical_event_set_hash(&cache.verified.as_ref().unwrap().events).unwrap()
        );
        assert!(shadow
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "eth_getLogs")
            .all(|r| r["params"][0]["fromBlock"] == "0xb"));
        let absent_from_shadow = add(&store, &config, 12).await;
        let mismatch = poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
            .await
            .unwrap();
        assert!(!mismatch.agrees && cache.verified.is_none());
        logs.lock().unwrap().push(absent_from_shadow);
        shadow.requests.lock().unwrap().clear();
        assert!(
            poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
                .await
                .unwrap()
                .agrees
        );
        assert!(shadow
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["method"] == "eth_getLogs")
            .all(|r| r["params"][0]["fromBlock"] == "0x1"));
        head.store(14, Ordering::SeqCst);
        store
            .upsert_base_log_cursor(
                &config.indexer.network,
                &config.indexer.factory_contract,
                14,
                None,
            )
            .await
            .unwrap();
        reorg_on_logs.store(true, Ordering::SeqCst);
        let changed_during_scan =
            poll_open_competition_v2_shadow_with_cache(&store, &config, &mut cache)
                .await
                .unwrap();
        assert!(!changed_during_scan.agrees && cache.verified.is_none());
        assert_eq!(
            changed_during_scan.failure_code.as_deref(),
            Some("safe_block_hash_mismatch")
        );
    }

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
    fn shadow_request_delay_is_bounded() {
        let result = OpenCompetitionV2ShadowConfig::from_lookup(|key| match key {
            "OPEN_COMPETITION_V2_INDEXER_NETWORK" => Some("base-sepolia".to_string()),
            "OPEN_COMPETITION_V2_FACTORY_CONTRACT" => {
                Some("0x1111111111111111111111111111111111111111".to_string())
            }
            "OPEN_COMPETITION_V2_INDEXER_RPC_URL" => Some("https://primary.example".to_string()),
            "OPEN_COMPETITION_V2_SHADOW_RPC_URL" => Some("https://shadow.example".to_string()),
            "OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK" => Some("1".to_string()),
            "OPEN_COMPETITION_V2_SHADOW_REQUEST_DELAY_MS" => Some("9999".to_string()),
            _ => None,
        })
        .unwrap();
        assert_eq!(result.request_delay_ms, 5_000);
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
