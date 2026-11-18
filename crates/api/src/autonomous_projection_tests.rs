//! Real database and HTTP regressions for independent autonomous readers.
use super::*;
use alloy::sol_types::SolCall;
use chain_base::AutonomousBountyEventKind as Kind;

alloy::sol! {
    interface WalletReadinessMulticall {
        struct ReadCall { address target; bool allowFailure; bytes callData; }
        struct ReadResult { bool success; bytes returnData; }
        function aggregate3(ReadCall[] calldata calls) external payable returns (ReadResult[] memory returnData);
    }
}

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

    let wallet = format!("0x{}", "11".repeat(20));
    let usdc = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
    let address_word = |address: &str| format!("{}{}", "0".repeat(24), &address[2..]);
    let protocol_hash = alloy::primitives::keccak256(AUTONOMOUS_V2_PROTOCOL_VERSION.as_bytes());
    let words = [
        format!("{:064x}", 250_000),
        address_word(&factories[1]),
        address_word(usdc),
        address_word(&format!("0x{}", "44".repeat(20))),
        format!("{:064x}", 100_000),
        format!("{:064x}", 1),
        hex::encode(protocol_hash),
    ];
    let returns: Vec<_> = words
        .iter()
        .map(|word| WalletReadinessMulticall::ReadResult {
            success: true,
            returnData: hex::decode(word).unwrap().into(),
        })
        .collect();
    let readiness_multicall = format!(
        "0x{}",
        hex::encode(WalletReadinessMulticall::aggregate3Call::abi_encode_returns(&returns))
    );
    let rpc_requests = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let captured_requests = rpc_requests.clone();
    let rpc_v2_factory = factories[1].clone();
    let rpc_v1_factory = factories[0].clone();
    let canonical_call = format!(
        "0x{}{}",
        hex::encode(&alloy::primitives::keccak256(b"isCanonicalBounty(address)")[..4]),
        address_word(&children[1])
    );
    let rpc = Router::new().route(
        "/",
        post(move |Json(request): Json<Value>| {
            let captured_requests = captured_requests.clone();
            let factory = rpc_v2_factory.clone();
            let legacy_factory = rpc_v1_factory.clone();
            let canonical_call = canonical_call.clone();
            let readiness_multicall = readiness_multicall.clone();
            async move {
                captured_requests.lock().unwrap().push(request.clone());
                let respond = |request: &Value| {
                    let result = match request["method"].as_str().unwrap() {
                        "eth_chainId" => json!("0x2105"),
                        "eth_blockNumber" => json!("0x3e8"),
                        "eth_getBlockByNumber" => json!({
                            "number": if request["params"][0] == "safe" { "0x3e6" } else { "0x3e8" },
                            "hash":format!("0x{}", "ab".repeat(32)),
                            "timestamp":format!("0x{:x}", Utc::now().timestamp())
                        }),
                        "eth_call" => {
                            assert_eq!(request["params"][1], "0x3e8");
                            let target = request["params"][0]["to"].as_str().unwrap();
                            if target == factory || target == legacy_factory {
                                assert_eq!(request["params"][0]["data"], canonical_call);
                                json!(format!("0x{:064x}", u8::from(target == factory)))
                            } else {
                                assert_eq!(target, "0xca11bde05977b3631167028862be2a173976ca11");
                                json!(readiness_multicall)
                            }
                        },
                        method => panic!("unexpected fixture RPC: {method}"),
                    };
                    json!({"jsonrpc":"2.0", "id":request["id"], "result":result})
                };
                Json(if let Some(batch) = request.as_array() {
                    Value::Array(batch.iter().map(respond).collect())
                } else { respond(&request) })
            }
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
    // Wallet readiness chooses identity from canonical factory history, never
    // from the caller or a healthy sibling; fresh chain checks happen afterward.
    for i in 0..2 {
        let (factory, protocol) =
            canonical_wallet_readiness_target(&state, "base-mainnet", &children[i])
                .await
                .unwrap();
        assert_eq!(factory, factories[i]);
        assert_eq!(
            protocol,
            [
                chain_base::AgentWalletReadinessProtocol::AutonomousV1,
                chain_base::AgentWalletReadinessProtocol::AutonomousV2
            ][i]
        );
    }
    assert_eq!(
        canonical_wallet_readiness_target(
            &state,
            "base-mainnet",
            &format!("0x{}", "73".repeat(20))
        )
        .await
        .unwrap_err()
        .0,
        StatusCode::NOT_FOUND
    );
    let v2_factory = env::var("BASE_MAINNET_BOUNTY_V2_FACTORY").unwrap();
    env::set_var(
        "BASE_MAINNET_BOUNTY_V2_FACTORY",
        format!("0x{}", "99".repeat(20)),
    );
    assert!(
        canonical_wallet_readiness_target(&state, "base-mainnet", &children[1])
            .await
            .is_err()
    );
    env::set_var("BASE_MAINNET_BOUNTY_V2_FACTORY", v2_factory);
    let app = Router::new()
        .route("/opportunities", get(list_opportunities))
        .route("/feed", get(autonomous_bounty_feed))
        .route("/readiness", get(marketplace_readiness::readiness))
        .route(
            "/v1/base/agent-wallet/readiness",
            post(prepare_agent_wallet_to_earn),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    // Exercise the actual handler, including its wire contract. The legacy
    // factory explicitly rejects this V2 child, so the old handler fails here.
    reader(&store, &factories[0], false, 997).await;
    reader(&store, &factories[1], true, 998).await;
    let mut wallet_request = json!({
        "network":"base-mainnet", "wallet_address":wallet,
        "bounty_contract":children[1], "claim_bond_base_units":"100000",
        "signing_capabilities":["eip712_typed_data","eip3009_receive_with_authorization"],
        "wallet_profile":"generic-evm",
        "policy":{
            "allowed_chain_ids":[8453], "allowed_contracts":[usdc,children[1]],
            "per_transaction_usdc_base_units":"200000",
            "rolling_24h_usdc_base_units":"500000", "human_approval_policy":"out_of_policy"
        }
    });
    let response = client
        .post(format!("{base}/v1/base/agent-wallet/readiness"))
        .json(&wallet_request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report: Value = response.json().await.unwrap();
    assert_eq!(
        report["schema_version"],
        "agent-bounties/agent-wallet-readiness-v1"
    );
    assert_eq!(report["ready"], true, "{report}");
    assert_eq!(report["canonical_factory"], factories[1]);
    assert_eq!(report["bounty_contract"], children[1]);
    assert_eq!(report["observed_block_number"], 1000);
    assert_eq!(report["recommended_claim_path"], "agent_native_claim");
    assert!(report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| check["name"] == "protocol_version"
            && check["status"] == "pass"
            && check["required"]
                .as_str()
                .unwrap()
                .contains(&format!("{protocol_hash:#x}"))));
    let prior_rpc_count = {
        let requests = rpc_requests.lock().unwrap();
        assert!(requests
            .iter()
            .any(|request| request["params"][0]["to"] == factories[1]));
        assert!(!requests
            .iter()
            .any(|request| request["params"][0]["to"] == factories[0]));
        requests.len()
    };
    wallet_request["bounty_contract"] = json!(format!("0x{}", "73".repeat(20)));
    let response = client
        .post(format!("{base}/v1/base/agent-wallet/readiness"))
        .json(&wallet_request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let problem: Value = response.json().await.unwrap();
    assert_eq!(
        problem["schema_version"],
        "agent-bounties/agent-wallet-readiness-problem-v1"
    );
    assert_eq!(problem["error"], "canonical_identity_unavailable");
    assert_eq!(problem["failed_transition"], "resolve_canonical_factory");
    assert_eq!(problem["retryable"], false);
    assert_eq!(
        rpc_requests.lock().unwrap().len(),
        prior_rpc_count,
        "unknown canonical identity must fail before RPC or factory fallback"
    );
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
    // A transient provider-behind failure retains certification but cannot
    // masquerade as a healthy reader or an empty canonical public feed.
    for factory in &factories {
        reader(&store, factory, true, 999).await;
    }
    let saved = store
        .autonomous_canonical_checkpoint("base-mainnet", &factories[1])
        .await
        .unwrap();
    let mut failed = store
        .get_base_indexer_heartbeat("base-mainnet", &factories[1])
        .await
        .unwrap()
        .unwrap();
    failed.status = "failed".into();
    failed.error_message = Some("autonomous provider safe head behind checkpoint".into());
    store.upsert_base_indexer_heartbeat(&failed).await.unwrap();
    for route in [
        "/feed".to_string(),
        format!("/feed?bounty_contract={}", children[1]),
        format!("/feed?claimable_only=true&bounty_contract={}", children[1]),
    ] {
        let response = client.get(format!("{base}{route}")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(
            response.text().await.unwrap(),
            "[]",
            "unavailable is not empty"
        );
    }
    assert_eq!(
        client
            .get(format!("{base}/feed?bounty_contract={}", children[0]))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
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
    assert_eq!(v2["ready"], false);
    assert_eq!(v2["blocker"], "reader_failed");
    assert_eq!(
        store
            .autonomous_canonical_checkpoint("base-mainnet", &factories[1])
            .await
            .unwrap(),
        saved
    );
    reader(&store, &factories[1], true, 999).await;
    // The unrelated orphan fixture remains a certification error. Only the
    // typed temporary availability condition was changed to HTTP 503.
    assert_eq!(
        client
            .get(format!("{base}/feed"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    // Verified-only metrics retain their v1 scope instead of inheriting v2 rows.
    assert!(store
        .list_autonomous_bounty_events_for_factory("base-mainnet", &factories[0], true)
        .await
        .unwrap()
        .is_empty());
    server.abort();
    rpc_server.abort();
}
