//! Safe-block admission and reconciliation of pre-checkpoint legacy history.
use super::*;
use chain_base::JsonRpcTransport;
use std::collections::{BTreeMap, BTreeSet};
mod bootstrap_reads;
use bootstrap_reads::{BootstrapReadPolicy, BootstrapReads};

// Only bootstrap uses this wrapper. Normal polling and global RPC parsers keep
// their existing behavior. Every delegated operation remains one wire attempt.
struct BootstrapTransport<T>(T);

#[async_trait::async_trait]
impl<T: JsonRpcTransport> JsonRpcTransport for BootstrapTransport<T> {
    async fn post_json_value(
        &self,
        rpc_url: &str,
        request: &serde_json::Value,
    ) -> Result<serde_json::Value, ChainBaseError> {
        let request_id = request
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                ChainBaseError::InvalidRpcResponse("bootstrap request id missing".into())
            })?;
        let value = self.0.post_json_value(rpc_url, request).await?;
        checked_bootstrap_response(value, request_id)
    }
}

fn checked_bootstrap_response(
    value: serde_json::Value,
    request_id: u64,
) -> Result<serde_json::Value, ChainBaseError> {
    let invalid = || {
        ChainBaseError::InvalidRpcResponse(
            "legacy bootstrap response has invalid version, id or result/error fields".into(),
        )
    };
    let object = value.as_object().ok_or_else(invalid)?;
    // Validate BOTH headers and logs before any typed parser can classify a
    // throttling error. A malformed error never authorizes another request.
    if object.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0")
        || object.get("id").and_then(serde_json::Value::as_u64) != Some(request_id)
        || object.contains_key("result") == object.contains_key("error")
    {
        return Err(invalid());
    }
    if let Some(error) = object.get("error") {
        if !error.is_object()
            || error
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .is_none()
            || error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .is_none()
        {
            return Err(invalid());
        }
    }
    Ok(value)
}

fn same_events(stored: &[AutonomousBountyEvent], canonical: &[AutonomousBountyEvent]) -> bool {
    // Ignore database IDs and observation timestamps, never the actual event payload.
    let signature = |e: &AutonomousBountyEvent| {
        serde_json::to_string(&serde_json::json!([
            e.log_key,
            e.tx_hash,
            e.block_number,
            e.log_index,
            e.contract_address,
            e.bounty_id,
            e.kind,
            e.data
        ]))
        .expect("event is serializable")
    };
    let a = stored.iter().map(signature).collect::<BTreeSet<_>>();
    let b = canonical.iter().map(signature).collect::<BTreeSet<_>>();
    a.len() == stored.len() && b.len() == canonical.len() && a == b
}

pub(super) async fn verify_previous(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
    cursor: Option<u64>,
    safe: &chain_base::BaseBlockIdentity,
) -> anyhow::Result<Option<db::AutonomousCanonicalCheckpoint>> {
    let checkpoint = store
        .autonomous_canonical_checkpoint(&config.network, &config.factory_contract)
        .await?;
    if let Some(c) = &checkpoint {
        if c.invalidated || cursor != Some(c.block_number) {
            return Err(anyhow!(
                "autonomous canonical checkpoint is invalid or detached from cursor"
            ));
        }
        if c.block_number > safe.number {
            store
                .invalidate_autonomous_canonical_checkpoint(
                    &config.network,
                    &config.factory_contract,
                )
                .await?;
            return Err(anyhow!("safe chain regressed below autonomous checkpoint"));
        }
        let now =
            fetch_exact_block_identity(&config.rpc_url, c.block_number, config.request_id + 30_001)
                .await?;
        if now.hash != c.block_hash || now.number != c.block_number {
            store
                .invalidate_autonomous_canonical_checkpoint(
                    &config.network,
                    &config.factory_contract,
                )
                .await?;
            return Err(anyhow!("autonomous chain reorganization detected; history quarantined, explicit reconciliation required"));
        }
    } else if let Some(cursor) = cursor {
        if cursor > safe.number {
            // Legacy indexers used latest-minus-confirmations. Wait for safe to
            // catch up before certifying their cursor, without treating this
            // initial wait as a regression of an already certified checkpoint.
            return Err(anyhow::Error::new(IndexerOperationalError {
                message: "legacy cursor is not yet safe".into(),
                retryable: true,
                provider_delay_seconds: None,
            }));
        }
        let mut reads = BootstrapReads::new(BootstrapReadPolicy::default());
        // One observation deadline includes SQL and all RPC retries. Exhaustion
        // enters the existing non-exiting operator hold, rather than resetting
        // this budget through the polling/supervisor restart loop.
        let result = tokio::time::timeout_at(
            reads.deadline(),
            bootstrap_previous(store, config, cursor, &mut reads),
        )
        .await;
        return match result {
            Ok(Ok(checkpoint)) => Ok(Some(checkpoint)),
            Ok(Err(error)) => Err(bootstrap_reads::stopped(&redact_operational_error(
                &error.to_string(),
            ))),
            Err(_) => Err(bootstrap_reads::stopped(
                "absolute observation deadline exhausted",
            )),
        };
    }
    Ok(checkpoint)
}

async fn bootstrap_previous(
    store: &PostgresStore,
    config: &AutonomousIndexerConfig,
    cursor: u64,
    reads: &mut BootstrapReads,
) -> anyhow::Result<db::AutonomousCanonicalCheckpoint> {
    let tag = format!("0x{cursor:x}");
    let transport = BootstrapTransport(
        chain_base::ReqwestJsonRpcTransport::single_attempt_without_redirects(
            Duration::from_secs(30),
        )?
        .with_read_observation_deadline(reads.deadline()),
    );
    let before = reads
        .run(|| {
            chain_base::fetch_block_identity_with_transport(
                &config.rpc_url,
                &tag,
                config.request_id + 30_002,
                &transport,
            )
        })
        .await?;
    let archive_rpc = config.bootstrap_archive_rpc_url.as_deref();
    if let Some(archive) = archive_rpc {
        let anchor = reads
            .run(|| {
                chain_base::fetch_block_identity_with_transport(
                    archive,
                    &tag,
                    config.request_id + 30_005,
                    &transport,
                )
            })
            .await?;
        if anchor != before {
            return Err(anyhow!(
                "bootstrap archive disagrees with primary canonical cursor"
            ));
        }
    }
    let events = store
        .list_autonomous_bounty_events_for_factory(&config.network, &config.factory_contract, false)
        .await?;
    let mut blocks = BTreeMap::<u64, Vec<AutonomousBountyEvent>>::new();
    for event in &events {
        if event.block_number > cursor {
            return Err(anyhow!("uncertified events extend beyond legacy cursor"));
        }
        blocks
            .entry(event.block_number)
            .or_default()
            .push(event.clone());
    }
    for (number, stored) in blocks {
        let emitters = stored
            .iter()
            .map(|e| e.contract_address.clone())
            .collect::<BTreeSet<_>>();
        let query = BaseMultiContractLogQuery::new(
            emitters,
            number,
            Some(number),
            autonomous_bounty_event_topics(),
        )?;
        let logs = reads
            .run(|| {
                chain_base::fetch_base_multi_contract_logs_with_transport(
                    archive_rpc.unwrap_or(&config.rpc_url),
                    &query,
                    config.request_id + 30_003,
                    &transport,
                )
            })
            .await?;
        let mut canonical = decode_autonomous_bounty_logs(rpc_logs_to_evm_logs(logs.result)?)?;
        let observed_keys = stored
            .iter()
            .map(|event| event.log_key.as_str())
            .collect::<BTreeSet<_>>();
        canonical.retain(|event| observed_keys.contains(event.log_key.as_str()));
        if !same_events(&stored, &canonical) {
            return Err(anyhow!(
                "legacy event history disagrees with safe canonical logs; reconciliation required"
            ));
        }
    }
    let after = reads
        .run(|| {
            chain_base::fetch_block_identity_with_transport(
                &config.rpc_url,
                &tag,
                config.request_id + 30_004,
                &transport,
            )
        })
        .await?;
    if before != after || before.number != cursor {
        return Err(anyhow!(
            "canonical chain changed during legacy reconciliation"
        ));
    }
    if let Some(archive) = archive_rpc {
        let anchor = reads
            .run(|| {
                chain_base::fetch_block_identity_with_transport(
                    archive,
                    &tag,
                    config.request_id + 30_006,
                    &transport,
                )
            })
            .await?;
        if anchor != after {
            return Err(anyhow!(
                "bootstrap archive changed during legacy reconciliation"
            ));
        }
    }
    store
        .commit_autonomous_bootstrap_checkpoint(
            &config.network,
            &config.factory_contract,
            cursor,
            &after,
            &events,
        )
        .await?;
    Ok(db::AutonomousCanonicalCheckpoint {
        block_number: after.number,
        block_hash: after.hash,
        invalidated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bootstrap_envelopes_are_validated_before_header_or_log_throttling_retry() {
        use serde_json::json;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct FixedResponse {
            value: serde_json::Value,
            calls: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl JsonRpcTransport for FixedResponse {
            async fn post_json_value(
                &self,
                _: &str,
                _: &serde_json::Value,
            ) -> Result<serde_json::Value, ChainBaseError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(self.value.clone())
            }
        }
        let throttle = json!({"code":-32016,"message":"slow down"});
        let query = BaseMultiContractLogQuery::new(
            [format!("0x{}", "11".repeat(20))],
            100,
            Some(100),
            autonomous_bounty_event_topics(),
        )
        .unwrap();
        for header in [true, false] {
            for value in [
                json!({"jsonrpc":"2.0","id":8,"error":throttle}),
                json!({"jsonrpc":"1.0","id":7,"error":throttle}),
                json!({"jsonrpc":"2.0","error":throttle}),
                json!({"jsonrpc":"2.0","id":7,"result":[],"error":throttle}),
                json!({"jsonrpc":"2.0","id":7}),
                json!({"jsonrpc":"2.0","id":7,"error":{"code":-32016}}),
                json!({"jsonrpc":"2.0","id":7,"error":{"code":-32016,"message":42}}),
                json!({"jsonrpc":"2.0","id":7,"error":{"code":"-32016","message":"slow down"}}),
            ] {
                assert!(matches!(
                    checked_bootstrap_response(value.clone(), 7),
                    Err(ChainBaseError::InvalidRpcResponse(_))
                ));
                let transport = BootstrapTransport(FixedResponse {
                    value,
                    calls: AtomicUsize::new(0),
                });
                let mut reads = BootstrapReads::new(BootstrapReadPolicy::default());
                let error = reads
                    .run(|| async {
                        if header {
                            chain_base::fetch_block_identity_with_transport(
                                "unused", "0x64", 7, &transport,
                            )
                            .await
                            .map(|_| ())
                        } else {
                            chain_base::fetch_base_multi_contract_logs_with_transport(
                                "unused", &query, 7, &transport,
                            )
                            .await
                            .map(|_| ())
                        }
                    })
                    .await
                    .unwrap_err();
                assert_eq!(transport.0.calls.load(Ordering::SeqCst), 1);
                assert!(!indexer_error_is_retryable(&error));
            }
            let transport = BootstrapTransport(FixedResponse {
                value: json!({"jsonrpc":"2.0","id":7,"error":throttle}),
                calls: AtomicUsize::new(0),
            });
            let error = if header {
                chain_base::fetch_block_identity_with_transport("unused", "0x64", 7, &transport)
                    .await
                    .map(|_| ())
            } else {
                chain_base::fetch_base_multi_contract_logs_with_transport(
                    "unused", &query, 7, &transport,
                )
                .await
                .map(|_| ())
            }
            .unwrap_err();
            assert!(matches!(
                error,
                ChainBaseError::RpcProviderError { code: -32016, .. }
            ));
        }
        let transport = BootstrapTransport(FixedResponse {
            value: json!({"jsonrpc":"2.0","id":7,"result":{"number":"0x64","hash":format!("0x{}","11".repeat(32)),"timestamp":"0x1"}}),
            calls: AtomicUsize::new(0),
        });
        assert_eq!(
            chain_base::fetch_block_identity_with_transport("unused", "0x64", 7, &transport)
                .await
                .unwrap()
                .number,
            100
        );
        let transport = BootstrapTransport(FixedResponse {
            value: json!({"jsonrpc":"2.0","id":7,"result":[]}),
            calls: AtomicUsize::new(0),
        });
        assert!(chain_base::fetch_base_multi_contract_logs_with_transport(
            "unused", &query, 7, &transport
        )
        .await
        .unwrap()
        .result
        .is_empty());
    }

    async fn test_rpc(
        reply: impl Fn(&serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let mut response = reply(&request);
                let status = response
                    .get("_http_status")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(200);
                response["jsonrpc"] = response
                    .get("_version")
                    .cloned()
                    .unwrap_or_else(|| "2.0".into());
                response["id"] = response
                    .get("_response_id")
                    .cloned()
                    .unwrap_or_else(|| request["id"].clone());
                let body = response.to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    #[ignore = "requires isolated local PostgreSQL"]
    async fn archive_bootstrap_requires_primary_anchor_and_exact_history_then_uses_only_primary() {
        use serde_json::json;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("archive_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let store = PostgresStore::from_pool(sqlx::PgPool::connect_with(options).await.unwrap());
        store.migrate().await.unwrap();
        let factory = format!("0x{}", "61".repeat(20));
        let log = json!({
            "address": factory,
            "topics": [autonomous_bounty_event_topics()[0], format!("0x{}", "dd".repeat(32)),
                format!("0x{}{}", "00".repeat(12), "62".repeat(20)),
                format!("0x{}{}", "00".repeat(12), "63".repeat(20))],
            "data": format!("0x{}{}{}", "11".repeat(32), "22".repeat(32), "33".repeat(32)),
            "transactionHash": format!("0x{}", "cc".repeat(32)),
            "blockNumber": "0x32", "logIndex": "0x0"
        });
        let event = decode_autonomous_bounty_logs(
            rpc_logs_to_evm_logs([
                serde_json::from_value::<chain_base::RpcEvmLog>(log.clone()).unwrap()
            ])
            .unwrap(),
        )
        .unwrap()
        .remove(0);
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();
        let mut later_log = log.clone();
        later_log["blockNumber"] = "0x33".into();
        later_log["transactionHash"] = format!("0x{}", "ee".repeat(32)).into();
        later_log["topics"][1] = format!("0x{}", "ef".repeat(32)).into();
        let later_event = decode_autonomous_bounty_logs(
            rpc_logs_to_evm_logs([serde_json::from_value::<chain_base::RpcEvmLog>(
                later_log.clone(),
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap()
        .remove(0);
        store
            .upsert_autonomous_bounty_event("base-sepolia", &later_event)
            .await
            .unwrap();
        store
            .upsert_base_log_cursor("base-sepolia", &factory, 100, Some(&event.log_key))
            .await
            .unwrap();
        let original_events = serde_json::to_value(
            store
                .list_autonomous_bounty_events_for_factory("base-sepolia", &factory, false)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(original_events.as_array().unwrap().len(), 2);
        let original_cursor = store
            .get_base_log_cursor("base-sepolia", &factory)
            .await
            .unwrap()
            .unwrap();
        let header =
            json!({"number":"0x64", "hash":format!("0x{}", "aa".repeat(32)), "timestamp":"0x1"});
        let primary_header = header.clone();
        let (primary, primary_task) = test_rpc(move |request| {
            assert_eq!(request["method"], "eth_getBlockByNumber");
            assert_eq!(request["params"][0], "0x64");
            json!({"result": primary_header})
        })
        .await;
        let mode = Arc::new(AtomicUsize::new(0));
        let anchors = Arc::new(AtomicUsize::new(0));
        let log_reads = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let archive_log_reads = log_reads.clone();
        let archive_mode = mode.clone();
        let archive_anchors = anchors.clone();
        let (archive, archive_task) = test_rpc(move |request| {
            let mode = archive_mode.load(Ordering::SeqCst);
            match request["method"].as_str().unwrap() {
                "eth_getBlockByNumber" => {
                    assert_eq!(request["params"][0], "0x64");
                    let index = archive_anchors.fetch_add(1, Ordering::SeqCst);
                    let mut value = header.clone();
                    if mode == 1 || (mode == 2 && index == 1) {
                        value["hash"] = format!("0x{}", "bb".repeat(32)).into();
                    }
                    json!({"result":value})
                }
                "eth_getLogs" => {
                    let block = request["params"][0]["fromBlock"].as_str().unwrap();
                    assert!(matches!(block, "0x32" | "0x33"));
                    assert_eq!(request["params"][0]["toBlock"], block);
                    let mut reads = archive_log_reads.lock().unwrap();
                    reads.push(block.to_string());
                    if mode == 7
                        && block == "0x33"
                        && reads.iter().filter(|b| *b == "0x33").count() == 1
                    {
                        return json!({"_http_status":503});
                    }
                    if mode == 3 {
                        return json!({"result":[]});
                    }
                    if mode == 5 {
                        return json!({"error":{"code":-32000,"message":"archive unavailable"}});
                    }
                    let mut value = if block == "0x32" {
                        log.clone()
                    } else {
                        later_log.clone()
                    };
                    if mode == 4 {
                        value["data"] = format!("0x{}", "44".repeat(96)).into();
                    }
                    if mode == 6 {
                        value["data"] = "0x01".into();
                    }
                    if mode == 8 {
                        return json!({"result":[value], "_response_id":0});
                    }
                    if mode == 9 {
                        return json!({"result":[value], "_version":"1.0"});
                    }
                    if mode == 10 { return json!({"error":{"code":-32016,"message":"slow down"}, "_response_id":0}); }
                    if mode == 11 { return json!({"error":{"code":-32016,"message":"slow down"}, "_version":"1.0"}); }
                    if mode == 12 { return json!({"result":[value],"error":{"code":-32016,"message":"slow down"}}); }
                    json!({"result":[value]})
                }
                other => panic!("archive must never supply live heads: {other}"),
            }
        })
        .await;
        let config = AutonomousIndexerConfig {
            network: "base-sepolia".into(),
            rpc_url: primary,
            bootstrap_archive_rpc_url: Some(archive),
            factory_contract: factory.clone(),
            request_id: 123,
            ..Default::default()
        };
        let safe = chain_base::BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "aa".repeat(32)),
            timestamp: 1,
        };
        // Before/after anchor disagreement, missing/changed payload, provider
        // outage and malformed logs cannot certify or mutate historical rows.
        for failure in [1, 2, 3, 4, 5, 6, 8, 9, 10, 11, 12] {
            mode.store(failure, Ordering::SeqCst);
            anchors.store(0, Ordering::SeqCst);
            log_reads.lock().unwrap().clear();
            let error = verify_previous(&store, &config, Some(100), &safe)
                .await
                .unwrap_err();
            assert!(
                !indexer_error_is_retryable(&error),
                "bootstrap case {failure} must hold without automatic restart"
            );
            if failure >= 8 {
                assert_eq!(
                    log_reads.lock().unwrap().len(),
                    1,
                    "malformed envelope must not retry"
                );
            }
            assert!(store
                .autonomous_canonical_checkpoint(&config.network, &factory)
                .await
                .unwrap()
                .is_none());
            let cursor = store
                .get_base_log_cursor(&config.network, &factory)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(cursor.last_scanned_block, 100);
            assert_eq!(cursor.last_log_key, original_cursor.last_log_key);
            assert_eq!(cursor.updated_at, original_cursor.updated_at);
            assert_eq!(
                serde_json::to_value(
                    store
                        .list_autonomous_bounty_events_for_factory(&config.network, &factory, false)
                        .await
                        .unwrap()
                )
                .unwrap(),
                original_events
            );
        }
        // A temporary middle-block failure must retry that block without
        // re-reading the successful prefix or persisting partial authority.
        mode.store(7, Ordering::SeqCst);
        log_reads.lock().unwrap().clear();
        anchors.store(0, Ordering::SeqCst);
        let checkpoint = verify_previous(&store, &config, Some(100), &safe)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(checkpoint.block_hash, safe.hash);
        assert_eq!(anchors.load(Ordering::SeqCst), 2);
        assert_eq!(*log_reads.lock().unwrap(), vec!["0x32", "0x33", "0x33"]);
        archive_task.abort();
        assert!(verify_previous(&store, &config, Some(100), &safe)
            .await
            .is_ok());
        assert_eq!(
            serde_json::to_value(
                store
                    .list_autonomous_bounty_events_for_factory(&config.network, &factory, false)
                    .await
                    .unwrap()
            )
            .unwrap(),
            original_events
        );
        primary_task.abort();
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated local PostgreSQL"]
    async fn legacy_cursor_waits_for_safe_without_certifying_or_advancing_history() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("legacy_safe_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let store = PostgresStore::from_pool(sqlx::PgPool::connect_with(options).await.unwrap());
        store.migrate().await.unwrap();

        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            // Empty legacy history still requires matching cursor headers before
            // and after reconciliation. No request is allowed during safe lag.
            for _ in 0..2 {
                let (socket, _) = listener.accept().await.unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["method"], "eth_getBlockByNumber");
                assert_eq!(request["params"][0], "0x64");
                let body = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{
                    "number":"0x64", "hash":format!("0x{}", "aa".repeat(32)),"timestamp":"0x1"}})
                .to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        let factory = format!("0x{}", "61".repeat(20));
        let config = AutonomousIndexerConfig {
            network: "base-sepolia".into(),
            rpc_url: format!("http://{address}"),
            bootstrap_archive_rpc_url: None,
            factory_contract: factory.clone(),
            start_block: None,
            poll_seconds: 5,
            confirmations: 2,
            max_blocks_per_query: 100,
            request_id: 123,
            public_base_url: "http://localhost".into(),
        };
        store
            .upsert_base_log_cursor(&config.network, &factory, 100, None)
            .await
            .unwrap();
        let original = store
            .get_base_log_cursor(&config.network, &factory)
            .await
            .unwrap()
            .unwrap();
        let mut safe = chain_base::BaseBlockIdentity {
            number: 99,
            hash: format!("0x{}", "aa".repeat(32)),
            timestamp: 1,
        };
        let error = verify_previous(&store, &config, Some(100), &safe)
            .await
            .unwrap_err();
        let recovery = IndexerRecoveryPolicy {
            initial_backoff_seconds: 5,
            max_backoff_seconds: 120,
            exit_after_consecutive_failures: 8,
        };
        assert_eq!(
            recovery.decision_for_error(1, &error),
            IndexerRecoveryDecision::RetryFromPersistedCursor {
                consecutive_failures: 1,
                backoff_seconds: 5,
            }
        );
        assert_eq!(
            recovery.decision_for_error(8, &error),
            IndexerRecoveryDecision::ExitForSupervisorRestart {
                consecutive_failures: 8,
            }
        );
        assert!(indexer_provider_delay(&error).is_none());
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        assert!(store
            .autonomous_canonical_checkpoint(&config.network, &factory)
            .await
            .unwrap()
            .is_none());
        let waiting = store
            .get_base_log_cursor(&config.network, &factory)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(waiting.last_scanned_block, original.last_scanned_block);
        assert_eq!(waiting.last_log_key, original.last_log_key);
        assert_eq!(waiting.updated_at, original.updated_at);

        safe.number = 100;
        let certified = verify_previous(&store, &config, Some(100), &safe)
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(certified.block_number, 100);
        assert_eq!(certified.block_hash, safe.hash);
        assert!(!certified.invalidated);

        // The identical height lag after certification is an integrity failure,
        // not a bootstrap wait. It must quarantine and remain nonretryable.
        safe.number = 99;
        let error = verify_previous(&store, &config, Some(100), &safe)
            .await
            .unwrap_err();
        assert!(!indexer_error_is_retryable(&error));
        assert!(
            store
                .autonomous_canonical_checkpoint(&config.network, &factory)
                .await
                .unwrap()
                .unwrap()
                .invalidated
        );
        safe.number = 100;
        let error = verify_previous(&store, &config, Some(100), &safe)
            .await
            .unwrap_err();
        assert!(!indexer_error_is_retryable(&error));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(
            store
                .get_base_log_cursor(&config.network, &factory)
                .await
                .unwrap()
                .unwrap()
                .last_scanned_block,
            100
        );
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires isolated local PostgreSQL"]
    async fn changed_rpc_hash_quarantines_paid_history_without_rewriting_it() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&url);
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let schema = format!("reorg_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let store =
            PostgresStore::from_pool(sqlx::PgPool::connect_with(options.clone()).await.unwrap());
        store.migrate().await.unwrap();
        let reorg = Arc::new(AtomicBool::new(false));
        let changed = reorg.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["method"], "eth_getBlockByNumber");
                assert_eq!(request["params"][0], "0x64");
                let hash = if changed.load(Ordering::SeqCst) {
                    "bb"
                } else {
                    "aa"
                };
                let body = serde_json::json!({"jsonrpc":"2.0","id":request["id"],"result":{
                    "number":"0x64", "hash":format!("0x{}",hash.repeat(32)),"timestamp":"0x1"}})
                .to_string();
                reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
            }
        });
        let factory = format!("0x{}", "61".repeat(20));
        let child = format!("0x{}", "62".repeat(20));
        let config = AutonomousIndexerConfig {
            network: "base-sepolia".into(),
            rpc_url: format!("http://{address}"),
            bootstrap_archive_rpc_url: None,
            factory_contract: factory.clone(),
            start_block: None,
            poll_seconds: 5,
            confirmations: 2,
            max_blocks_per_query: 100,
            request_id: 123,
            public_base_url: "http://localhost".into(),
        };
        for (index, kind, emitter, data) in [
            (
                0,
                AutonomousBountyEventKind::CanonicalBountyCreated,
                factory.clone(),
                serde_json::json!({"bounty_contract":child}),
            ),
            (
                1,
                AutonomousBountyEventKind::BountySettled,
                child.clone(),
                serde_json::json!({"solver_reward":7}),
            ),
        ] {
            let event = AutonomousBountyEvent {
                id: uuid::Uuid::new_v4(),
                log_key: format!("reorg-test-{index}"),
                tx_hash: format!("0x{}", "cc".repeat(32)),
                block_number: 100,
                log_index: index,
                contract_address: emitter,
                bounty_id: format!("0x{}", "dd".repeat(32)),
                kind,
                data,
                occurred_at: Utc::now(),
            };
            store
                .upsert_autonomous_bounty_event(&config.network, &event)
                .await
                .unwrap();
        }
        let block = chain_base::BaseBlockIdentity {
            number: 100,
            hash: format!("0x{}", "aa".repeat(32)),
            timestamp: 1,
        };
        store
            .commit_autonomous_canonical_checkpoint(
                &config.network,
                &factory,
                None,
                None,
                &block,
                None,
            )
            .await
            .unwrap();
        assert!(verify_previous(&store, &config, Some(100), &block)
            .await
            .is_ok());
        assert_eq!(
            store
                .list_autonomous_bounty_events(&config.network)
                .await
                .unwrap()
                .len(),
            2
        );
        reorg.store(true, Ordering::SeqCst);
        let error = verify_previous(&store, &config, Some(100), &block)
            .await
            .unwrap_err();
        assert!(!indexer_error_is_retryable(&error));
        let independent =
            PostgresStore::from_pool(sqlx::PgPool::connect_with(options).await.unwrap());
        assert!(independent
            .list_autonomous_bounty_events(&config.network)
            .await
            .is_err());
        let now = Utc::now();
        let start = now - chrono::Duration::days(1);
        assert!(independent
            .platform_payout_proof_rows(&config.network, start, now, &[])
            .await
            .is_err());
        assert!(independent
            .platform_demand_growth_stats(&config.network, now, start, &[], &[], &[])
            .await
            .is_err());
        assert!(independent
            .platform_metrics_stats(
                &config.network,
                start,
                now,
                start,
                start,
                now,
                &[],
                &[],
                &[]
            )
            .await
            .is_err());
        assert_eq!(
            independent
                .list_autonomous_bounty_events_for_factory(&config.network, &factory, false)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            independent
                .get_base_log_cursor(&config.network, &factory)
                .await
                .unwrap()
                .unwrap()
                .last_scanned_block,
            100
        );
        server.abort();
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .unwrap();
    }

    #[test]
    fn reconciliation_compares_payload_and_rejects_duplicate_or_orphaned_logs() {
        let event = AutonomousBountyEvent {
            id: uuid::Uuid::new_v4(),
            log_key: "key".into(),
            tx_hash: "0x01".into(),
            block_number: 3,
            log_index: 0,
            contract_address: "factory".into(),
            bounty_id: "bounty".into(),
            kind: AutonomousBountyEventKind::BountySettled,
            data: serde_json::json!({"paid":7}),
            occurred_at: Utc::now(),
        };
        assert!(same_events(
            std::slice::from_ref(&event),
            std::slice::from_ref(&event)
        ));
        assert!(!same_events(std::slice::from_ref(&event), &[]));
        assert!(!same_events(
            &[event.clone(), event.clone()],
            std::slice::from_ref(&event)
        ));
        let mut changed = event.clone();
        changed.data = serde_json::json!({"paid":8});
        assert!(!same_events(&[event], &[changed]));
    }
}
