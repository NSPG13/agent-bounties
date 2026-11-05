//! Real database and HTTP regressions for independent autonomous readers.
use super::*;
use chain_base::AutonomousBountyEventKind as Kind;

fn events(factory: &str, child: &str, v2: bool) -> Vec<AutonomousBountyEvent> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../chain-base/tests/fixtures/autonomous-v2-loop.json"
    ))
    .unwrap();
    let logs: Vec<chain_base::RpcEvmLog> = serde_json::from_value(fixture["logs"].clone()).unwrap();
    let decoded =
        chain_base::decode_autonomous_bounty_logs(chain_base::rpc_logs_to_evm_logs(logs).unwrap())
            .unwrap();
    let first = decoded
        .iter()
        .find(|e| e.kind == Kind::CanonicalBountyCreated)
        .unwrap();
    let id = first.bounty_id.clone();
    let mut selected: Vec<_> = decoded
        .into_iter()
        .filter(|e| {
            e.bounty_id == id
                && matches!(
                    e.kind,
                    Kind::CanonicalBountyCreated
                        | Kind::CanonicalBountyTermsCommitted
                        | Kind::CanonicalBountyEconomicsConfigured
                        | Kind::CanonicalBountyVerificationConfigured
                        | Kind::CanonicalBountyPlatformFeeConfigured
                )
                && (v2 || e.kind != Kind::CanonicalBountyPlatformFeeConfigured)
        })
        .collect();
    let economics = selected
        .iter()
        .find(|e| e.kind == Kind::CanonicalBountyEconomicsConfigured)
        .unwrap();
    let v1_target = economics.data["solver_reward"].as_u64().unwrap()
        + economics.data["verifier_reward"].as_u64().unwrap();
    for (i, event) in selected.iter_mut().enumerate() {
        event.id = Uuid::new_v4();
        event.log_key = format!("{factory}:creation:{i}");
        event.block_number = 100;
        event.log_index = i as u64;
        event.contract_address = factory.into();
        event.occurred_at = Utc::now();
        if event.kind == Kind::CanonicalBountyCreated {
            event.data["bounty_contract"] = json!(child);
        }
        if event.kind == Kind::CanonicalBountyEconomicsConfigured {
            event.data["initial_funding"] = json!(0);
            if !v2 {
                event.data["target_amount"] = json!(v1_target);
            }
        }
    }
    // Same bounty ID in both factories deliberately exercises cross-factory
    // collisions. One later child update is beyond v1's cursor, but inside v2's.
    for (block, amount) in [(998, 1), (999, 999_999_999)] {
        selected.push(AutonomousBountyEvent {
            id: Uuid::new_v4(),
            log_key: format!("{factory}:funding:{block}"),
            tx_hash: format!("0x{block:064x}"),
            block_number: block,
            log_index: 0,
            contract_address: child.into(),
            bounty_id: id.clone(),
            kind: Kind::FundingAdded,
            data: json!({"funded_amount":amount}),
            occurred_at: Utc::now(),
        });
    }
    selected
}

async fn reader(store: &PostgresStore, factory: &str, current: bool, cursor: u64) {
    let now = Utc::now();
    let old_cursor = store
        .get_base_log_cursor("base-mainnet", factory)
        .await
        .unwrap();
    let previous = store
        .autonomous_canonical_checkpoint("base-mainnet", factory)
        .await
        .unwrap();
    store
        .commit_autonomous_canonical_checkpoint(
            "base-mainnet",
            factory,
            old_cursor.map(|c| c.last_scanned_block),
            previous.as_ref(),
            &chain_base::BaseBlockIdentity {
                number: cursor,
                hash: format!("0x{}", "ab".repeat(32)),
                timestamp: 1,
            },
            None,
        )
        .await
        .unwrap();
    store
        .upsert_base_indexer_heartbeat(&BaseIndexerHeartbeat {
            network: "base-mainnet".into(),
            escrow_contract: factory.into(),
            status: "success".into(),
            started_at: now,
            completed_at: Some(now - chrono::Duration::seconds(if current { 0 } else { 120 })),
            latest_block: Some(1000),
            confirmed_to_block: Some(cursor),
            from_block: Some(100),
            to_block: Some(cursor),
            fetched_logs: 1,
            persisted_cursor_block: Some(cursor),
            skipped_reason: None,
            error_message: None,
            updated_at: now,
        })
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL; run this test alone"]
async fn autonomous_readers_keep_factory_health_identity_and_boundaries_separate_postgres() {
    assert_eq!(
        env::var("AGENT_BOUNTIES_LOCAL_TEST_DATABASE").as_deref(),
        Ok("1")
    );
    assert!(env::var("AGENT_BOUNTIES_TEST_POSTGRES_CONTAINER")
        .unwrap()
        .starts_with("agent-bounties-test-"));
    let database = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&database).unwrap();
    assert!(matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")));
    assert_eq!(parsed.username(), "agent_bounties_test");
    assert_eq!(parsed.path(), "/agent_bounties_test");
    // The launcher provides a separate process and disposable database. No
    // concurrent tests share these deployment settings or canonical factory rows.
    for (name, value) in [
        (
            "FACTORY",
            service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_FACTORY,
        ),
        (
            "IMPLEMENTATION",
            service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION,
        ),
        ("PLATFORM_FEE_BPS", "750"),
        (
            "PLATFORM_FEE_RECIPIENT",
            service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT,
        ),
    ] {
        env::set_var(format!("BASE_MAINNET_BOUNTY_V2_{name}"), value);
    }
    let factories = [
        autonomous_projection::factory("base-mainnet", AUTONOMOUS_PROTOCOL_VERSION).unwrap(),
        autonomous_projection::factory("base-mainnet", AUTONOMOUS_V2_PROTOCOL_VERSION).unwrap(),
    ];
    let children = [
        format!("0x{}", "71".repeat(20)),
        format!("0x{}", "72".repeat(20)),
    ];
    let store = PostgresStore::connect(&database).await.unwrap();
    store.migrate().await.unwrap();
    for i in 0..2 {
        for event in events(&factories[i], &children[i], i == 1) {
            store
                .upsert_autonomous_bounty_event("base-mainnet", &event)
                .await
                .unwrap();
        }
    }
    // An unrelated emitter copying the bounty ID cannot contaminate either feed.
    let mut poison = events(&factories[0], &children[0], false).pop().unwrap();
    poison.id = Uuid::new_v4();
    poison.log_key = "unrelated-child-with-same-bounty-id".into();
    poison.contract_address = format!("0x{}", "99".repeat(20));
    poison.block_number = 101;
    store
        .upsert_autonomous_bounty_event("base-mainnet", &poison)
        .await
        .unwrap();

    let rpc = Router::new().route(
        "/",
        post(|Json(request): Json<Value>| async move {
            let result = match request["method"].as_str().unwrap() {
                "eth_chainId" => json!("0x2105"),
                "eth_blockNumber" => json!("0x3e8"),
                "eth_getBlockByNumber" => json!({
                    "number": if request["params"][0] == "safe" { "0x3e6" } else { "0x3e8" },
                    "hash":format!("0x{}", "ab".repeat(32)),
                    "timestamp":format!("0x{:x}", Utc::now().timestamp())
                }),
                method => panic!("unexpected fixture RPC: {method}"),
            };
            Json(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let rpc_server = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused",
        store.clone(),
    );
    Arc::get_mut(&mut state).unwrap().base_rpc_urls.base_mainnet = Some(rpc_url);
    let app = Router::new()
        .route("/opportunities", get(list_opportunities))
        .route("/feed", get(autonomous_bounty_feed))
        .route("/readiness", get(marketplace_readiness::readiness))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    for healthy in [[true, false], [false, true], [true, true], [false, false]] {
        for i in 0..2 {
            reader(&store, &factories[i], healthy[i], 997 + i as u64).await;
        }
        let projection: Value = client
            .get(format!("{base}/opportunities?source_type=canonical_base"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let items = projection["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            healthy.iter().filter(|v| **v).count(),
            "{projection}"
        );
        let statuses = projection["source_statuses"][0]["protocol_statuses"]
            .as_array()
            .unwrap();
        for i in 0..2 {
            let label = ["autonomous-v1", "autonomous-v2"][i];
            let status = statuses.iter().find(|s| s["protocol"] == label).unwrap();
            assert_eq!(
                status["coverage"],
                if healthy[i] {
                    "complete"
                } else {
                    "unavailable"
                }
            );
            assert_eq!(
                status["item_count"],
                if healthy[i] { json!(1) } else { Value::Null }
            );
            assert_eq!(
                items.iter().any(|item| item["source_id"] == children[i]),
                healthy[i]
            );
            let identity = format!("canonical:base-mainnet:{}", children[i]);
            let scoped: Value = client
                .get(format!("{base}/opportunities?opportunity_id={identity}"))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            assert_eq!(
                scoped["coverage"],
                if healthy[i] {
                    "complete"
                } else {
                    "unavailable"
                }
            );
            assert_eq!(
                scoped["items"].as_array().unwrap().len(),
                usize::from(healthy[i])
            );
            assert_eq!(
                scoped["source_statuses"][0]["protocol_statuses"][0]["protocol"],
                label
            );
            // The earning-only HTTP path cannot reuse the sibling's healthy reader.
            let response = client
                .get(format!(
                    "{base}/feed?claimable_only=true&bounty_contract={}",
                    children[i]
                ))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if healthy[i] {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            );
            let protocol = [AUTONOMOUS_PROTOCOL_VERSION, AUTONOMOUS_V2_PROTOCOL_VERSION][i];
            let loaded =
                autonomous_projection::load(&state, "base-mainnet", protocol, None, None).await;
            if healthy[i] {
                let feed = loaded.unwrap().into_current().unwrap();
                assert_eq!(feed.len(), 1);
                assert_eq!(feed[0].bounty_contract, children[i]);
                assert_eq!(
                    feed[0].funded_amount,
                    i.to_string(),
                    "each factory uses its own cursor and the safe head"
                );
                assert!(feed[0]
                    .events
                    .iter()
                    .all(|e| e.block_number <= 997 + i as u64));
            } else {
                assert!(matches!(loaded, Err(StatusCode::SERVICE_UNAVAILABLE)));
            }
        }
        let aggregate = autonomous_projection::load_current(&state, "base-mainnet", None).await;
        if healthy.iter().any(|v| *v) {
            assert_eq!(
                aggregate.unwrap().into_current().unwrap().len(),
                items.len()
            );
        } else {
            assert!(matches!(aggregate, Err(StatusCode::SERVICE_UNAVAILABLE)));
        }
        let readiness: Value = client
            .get(format!("{base}/readiness"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let v2 = readiness["readers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["protocol"] == AUTONOMOUS_V2_PROTOCOL_VERSION)
            .unwrap();
        assert_eq!(v2["ready"], healthy[1]);
    }
    // Verified-only metrics retain their v1 scope instead of inheriting v2 rows.
    assert!(store
        .list_autonomous_bounty_events_for_factory("base-mainnet", &factories[0], true)
        .await
        .unwrap()
        .is_empty());
    server.abort();
    rpc_server.abort();
}
