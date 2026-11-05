//! Safe-block admission and reconciliation of pre-checkpoint legacy history.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

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
        let before =
            fetch_exact_block_identity(&config.rpc_url, cursor, config.request_id + 30_002).await?;
        let events = store
            .list_autonomous_bounty_events_for_factory(
                &config.network,
                &config.factory_contract,
                false,
            )
            .await?;
        let mut blocks = BTreeMap::<u64, Vec<AutonomousBountyEvent>>::new();
        for event in events {
            if event.block_number > cursor {
                return Err(anyhow!("uncertified events extend beyond legacy cursor"));
            }
            blocks.entry(event.block_number).or_default().push(event);
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
            let logs =
                fetch_base_multi_contract_logs(&config.rpc_url, &query, config.request_id + 30_003)
                    .await?;
            let mut canonical = decode_autonomous_bounty_logs(rpc_logs_to_evm_logs(logs.result)?)?;
            let observed_keys = stored
                .iter()
                .map(|event| event.log_key.as_str())
                .collect::<BTreeSet<_>>();
            canonical.retain(|event| observed_keys.contains(event.log_key.as_str()));
            if !same_events(&stored, &canonical) {
                return Err(anyhow!("legacy event history disagrees with safe canonical logs; reconciliation required"));
            }
        }
        let after =
            fetch_exact_block_identity(&config.rpc_url, cursor, config.request_id + 30_004).await?;
        if before != after || before.number != cursor {
            return Err(anyhow!(
                "canonical chain changed during legacy reconciliation"
            ));
        }
        store
            .commit_autonomous_canonical_checkpoint(
                &config.network,
                &config.factory_contract,
                Some(cursor),
                None,
                &after,
                None,
            )
            .await?;
        return Ok(Some(db::AutonomousCanonicalCheckpoint {
            block_number: after.number,
            block_hash: after.hash,
            invalidated: false,
        }));
    }
    Ok(checkpoint)
}

#[cfg(test)]
mod tests {
    use super::*;

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
