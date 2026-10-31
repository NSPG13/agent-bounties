use super::*;
use alloy::{
    primitives::{Address, B256, U256},
    signers::{local::PrivateKeySigner, SignerSync},
    sol_types::SolStruct,
};
use axum::body::to_bytes;
use serde_json::{json, Value};

alloy::sol! {
    struct TransferWithAuthorization {
        address from;
        address to;
        uint256 value;
        uint256 validAfter;
        uint256 validBefore;
        bytes32 nonce;
    }
}

// Public Foundry fixture key; used only to sign synthetic local authorizations.
const FIXTURE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

fn signed_authorization(
    network: &str,
    bounty: &str,
    bond: u64,
    nonce: &str,
    deadline: u64,
) -> (String, AutonomousBountyAuthorizationSignature) {
    let descriptor = base_network_descriptor(network).unwrap();
    let signer = FIXTURE_KEY.parse::<PrivateKeySigner>().unwrap();
    let solver = format!("{:#x}", signer.address());
    let domain = alloy::sol_types::eip712_domain! {
        name: if descriptor.chain_id == 84532 { "USDC" } else { "USD Coin" },
        version: "2", chain_id: descriptor.chain_id,
        verifying_contract: descriptor.native_usdc_token_address.parse::<Address>().unwrap(),
    };
    let payload = TransferWithAuthorization {
        from: signer.address(),
        to: bounty.parse().unwrap(),
        value: U256::from(bond),
        validAfter: U256::ZERO,
        validBefore: U256::from(deadline),
        nonce: nonce.parse::<B256>().unwrap(),
    };
    let bytes = signer
        .sign_hash_sync(&payload.eip712_signing_hash(&domain))
        .unwrap()
        .as_bytes();
    (
        solver,
        AutonomousBountyAuthorizationSignature {
            v: bytes[64],
            r: format!("0x{}", hex::encode(&bytes[..32])),
            s: format!("0x{}", hex::encode(&bytes[32..64])),
        },
    )
}

#[test]
fn expired_claim_signature_recovers_exact_scope_but_cannot_authorize_broadcast() {
    let bounty = "0x1111111111111111111111111111111111111111";
    let nonce = format!("0x{}", "aa".repeat(32));
    for network in ["base-mainnet", "base-sepolia"] {
        let descriptor = base_network_descriptor(network).unwrap();
        let (solver, signature) = signed_authorization(network, bounty, 150_000, &nonce, 1300);
        assert!(validate_persisted_claim_authorization_signature(
            &descriptor,
            bounty,
            &solver,
            150_000,
            &nonce,
            1300,
            &signature
        )
        .is_ok());
        assert_eq!(
            validate_claim_authorization_signature(
                &descriptor,
                bounty,
                &solver,
                150_000,
                &nonce,
                1300,
                &signature
            )
            .unwrap_err()
            .1
             .0["error"],
            "authorization_expired"
        );
        for (recipient, amount, signed_nonce, expiry) in [
            (
                "0x2222222222222222222222222222222222222222",
                150_000,
                nonce.clone(),
                1300,
            ),
            (bounty, 150_001, nonce.clone(), 1300),
            (bounty, 150_000, format!("0x{}", "bb".repeat(32)), 1300),
            (bounty, 150_000, nonce.clone(), 1301),
        ] {
            assert!(validate_persisted_claim_authorization_signature(
                &descriptor,
                recipient,
                &solver,
                amount,
                &signed_nonce,
                expiry,
                &signature
            )
            .is_err());
        }
        let other_network = base_network_descriptor(if network == "base-mainnet" {
            "base-sepolia"
        } else {
            "base-mainnet"
        })
        .unwrap();
        assert!(validate_persisted_claim_authorization_signature(
            &other_network,
            bounty,
            &solver,
            150_000,
            &nonce,
            1300,
            &signature
        )
        .is_err());
    }
}

async fn claimable_fixture(store: &PostgresStore) -> String {
    let now = Utc::now();
    let mut document: AutonomousBountyTermsDocument =
        serde_json::from_str(include_str!("../../../../bounties/autonomous-v1/244.json")).unwrap();
    document.contract_terms["funding_deadline"] = json!(now.timestamp() + 86_400);
    document.source_url = Some(format!(
        "https://example.com/local-fixture/{}",
        Uuid::new_v4()
    ));
    let record = build_autonomous_bounty_terms_record(
        "0x884834E884d6E93462655A2820140aD03E6747bC",
        document,
        now,
    )
    .unwrap();
    store.upsert_autonomous_bounty_terms(&record).await.unwrap();
    let factory = configured_autonomous_planner("base-mainnet")
        .unwrap()
        .factory_contract;
    let bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let bounty = format!("0x{:040x}", Uuid::new_v4().as_u128());
    let ct = &record.document.contract_terms;
    let events = [
        (
            AutonomousBountyEventKind::CanonicalBountyCreated,
            json!({"bounty_contract":bounty,"creator":record.creator_wallet,"terms_hash":record.terms_hash,"policy_hash":record.policy_hash,"creation_nonce":ct["creation_nonce"]}),
        ),
        (
            AutonomousBountyEventKind::CanonicalBountyTermsCommitted,
            json!({"acceptance_criteria_hash":record.acceptance_criteria_hash,"benchmark_hash":record.benchmark_hash,"evidence_schema_hash":record.evidence_schema_hash}),
        ),
        (
            AutonomousBountyEventKind::CanonicalBountyEconomicsConfigured,
            json!({"solver_reward":ct["solver_reward"]["amount"],"verifier_reward":ct["verifier_reward"]["amount"],"claim_bond":ct["claim_bond"]["amount"],"target_amount":ct["initial_funding"]["amount"],"initial_funding":ct["initial_funding"]["amount"],"funding_deadline":ct["funding_deadline"],"claim_window_seconds":ct["claim_window_seconds"],"verification_window_seconds":ct["verification_window_seconds"]}),
        ),
        (
            AutonomousBountyEventKind::CanonicalBountyVerificationConfigured,
            json!({"verification_mode":0,"verifier_module":record.document.verification_policy["verifier_module"],"verifier_reward_recipient":record.document.verification_policy["verifier_reward_recipient"],"threshold":1,"verifier_set_hash":format!("0x{}", "00".repeat(32))}),
        ),
        (
            AutonomousBountyEventKind::BountyBecameClaimable,
            json!({"funded_amount":ct["initial_funding"]["amount"]}),
        ),
    ];
    for (index, (kind, data)) in events.into_iter().enumerate() {
        store
            .upsert_autonomous_bounty_event(
                "base-mainnet",
                &AutonomousBountyEvent {
                    id: Uuid::new_v4(),
                    log_key: format!("claim-recovery-fixture:{}:{index}", Uuid::new_v4()),
                    tx_hash: format!("0x{:064x}", Uuid::new_v4().as_u128()),
                    block_number: 1_000_000,
                    log_index: index as u64,
                    contract_address: if index < 4 {
                        factory.clone()
                    } else {
                        bounty.clone()
                    },
                    bounty_id: bounty_id.clone(),
                    kind,
                    data,
                    occurred_at: now,
                },
            )
            .await
            .unwrap();
    }
    bounty
}

async fn call_claim(
    state: &SharedState,
    token: &str,
    request: AgentNativeClaimRequest,
) -> (StatusCode, Value) {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    match agent_native_claim(State(state.clone()), headers, Json(request)).await {
        Ok(response) => {
            let status = response.status();
            let body = to_bytes(response.into_body(), 1_000_000).await.unwrap();
            (status, serde_json::from_slice(&body).unwrap())
        }
        Err((status, Json(body))) => (status, body),
    }
}

#[tokio::test]
#[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
async fn expired_signed_retry_recovers_pending_claim_and_stale_direct_relay_never_sends() {
    let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let bounty = claimable_fixture(&store).await;
    let methods = Arc::new(Mutex::new(Vec::<String>::new()));
    let observed = methods.clone();
    let rpc = Router::new().route(
        "/",
        post(move |Json(body): Json<Value>| {
            let observed = observed.clone();
            async move {
                observed
                    .lock()
                    .unwrap()
                    .push(body["method"].as_str().unwrap().to_string());
                let result = match body["method"].as_str().unwrap() {
                    "eth_blockNumber" => json!("0x64"),
                    "eth_chainId" => json!("0x2105"),
                    "eth_getBlockByNumber" => json!({"number":"0x64","hash":format!("0x{}", "11".repeat(32)),"timestamp":format!("0x{:x}", Utc::now().timestamp())}),
                    "eth_getCode" => json!("0x"),
                    _ => Value::Null,
                };
                Json(json!({"jsonrpc":"2.0","id":body["id"],"result":result}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let rpc_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, rpc).await.unwrap();
    });
    let mut state = test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused-local-operator",
        store.clone(),
    );
    let configured = Arc::get_mut(&mut state).unwrap();
    configured.base_rpc_urls.base_mainnet = Some(rpc_url);
    configured.x402_relayer.wait_seconds = 0;
    configured.x402_relayer.enabled = true;
    configured.x402_relayer.relayer = Some(Arc::new(
        BaseTransactionRelayer::from_private_key(FIXTURE_KEY).unwrap(),
    ));
    let item = indexed_autonomous_bounty(&state, "base-mainnet", &bounty)
        .await
        .unwrap();
    assert_eq!(item.status, "claimable");
    assert!(item.terms_valid);
    let bond = item.claim_bond.parse::<u64>().unwrap();
    let nonce = format!("0x{}", "aa".repeat(32));
    let (solver, signature) = signed_authorization("base-mainnet", &bounty, bond, &nonce, 1300);
    let owner = format!("wallet:8453:{solver}");
    let key = format!("expired-retry-{}", Uuid::new_v4());
    let input = NewClaimCandidate {
        id: Uuid::new_v4(),
        idempotency_key: service_runtime::auth::hash_secret(&format!("claim:{owner}:{key}")),
        network: "base-mainnet".into(),
        bounty_contract: bounty.clone(),
        solver_wallet: solver.clone(),
        agent_id: None,
        eligibility_evidence: AgentEligibilityEvidence {
            agent_id: None,
            solver_wallet: solver.clone(),
            capabilities: vec![],
            paid_completions: 0,
            paid_usdc_base_units: 0,
        },
        eligibility_decision: AgentEligibilityDecision {
            eligible: true,
            reasons: vec![],
        },
    };
    let fp = service_runtime::auth::hash_secret(
        &json!(["base-mainnet", bounty, solver, Option::<Uuid>::None]).to_string(),
    );
    store
        .reserve_owned_claim_candidate(&input, 900, 20, &owner, &fp, 10)
        .await
        .unwrap();
    let stale = store
        .set_claim_candidate_authorization(input.id, &nonce, 1300)
        .await
        .unwrap();
    let hash = format!("0x{}", "cc".repeat(32));
    store
        .mark_claim_candidate_relaying(input.id, &hash)
        .await
        .unwrap();
    let now = Utc::now();
    let challenge = db::auth::WalletChallenge {
        id: Uuid::new_v4(),
        domain: "localhost".into(),
        chain_id: 8453,
        address: solver.clone(),
        message: "local fixture; SIWE cryptography tested separately".into(),
        created_at: now,
        expires_at: now + ChronoDuration::minutes(5),
    };
    store.create_wallet_challenge(&challenge).await.unwrap();
    let token = format!(
        "abws_{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    store
        .consume_wallet_challenge_and_create_session(
            challenge.id,
            &service_runtime::auth::hash_secret(&token),
            None,
        )
        .await
        .unwrap();
    let request = AgentNativeClaimRequest {
        idempotency_key: key,
        network: Some("base-mainnet".into()),
        bounty_contract: bounty,
        solver_wallet: solver,
        agent_id: None,
        request_bond_sponsorship: false,
        signature: Some(signature.clone()),
        wallet_signature: None,
        source: Some("local-regression".into()),
    };
    let mut invalid = request.clone();
    invalid.signature.as_mut().unwrap().r = format!("0x{}", "00".repeat(32));
    let before = store.get_claim_candidate(input.id).await.unwrap().unwrap();
    let (status, _) = call_claim(&state, &token, invalid).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        methods.lock().unwrap().iter().all(|method| matches!(method.as_str(), "eth_blockNumber" | "eth_getBlockByNumber" | "eth_chainId" | "eth_getCode")),
        "bad signature may inspect a contract wallet but cannot broadcast or reconcile a transaction"
    );
    assert!(methods
        .lock()
        .unwrap()
        .iter()
        .any(|method| method == "eth_getCode"));
    methods.lock().unwrap().clear();
    assert_eq!(
        store.get_claim_candidate(input.id).await.unwrap().unwrap(),
        before
    );
    let (status, pending) = call_claim(&state, &token, request.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    assert_eq!(pending["candidate"]["claim_transaction_hash"], hash);
    assert_eq!(pending["candidate"]["authorization_valid_before"], 1300);
    let recovered = relay_agent_native_claim(&state, &stale, bond, &nonce, 1300, &signature, None)
        .await
        .unwrap();
    assert_eq!(
        recovered.claim_transaction_hash.as_deref(),
        Some(hash.as_str())
    );
    assert!(
        methods
            .lock()
            .unwrap()
            .iter()
            .all(|method| method == "eth_getTransactionReceipt"),
        "stale state must never enter simulation or broadcast"
    );
    store
        .mark_claim_candidate_claimed(input.id, Uuid::new_v4())
        .await
        .unwrap();
    let (status, claimed) = call_claim(&state, &token, request).await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    assert_eq!(claimed["candidate"]["status"], "claimed");
    assert_eq!(claimed["candidate"]["authorization_valid_before"], 1300);
    server.abort();
}

#[tokio::test]
#[ignore = "requires isolated SECURITY_AUTH_TEST_DATABASE_URL on localhost"]
async fn relay_outcomes_keep_unknown_sends_and_release_only_proven_prebroadcast_failures() {
    let url = std::env::var("SECURITY_AUTH_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let store = PostgresStore::connect(&url).await.unwrap();
    store.migrate().await.unwrap();
    let network = format!("local-relay-outcomes-{}", Uuid::new_v4());
    for scenario in ["simulation", "provider", "timeout", "recorded"] {
        let solver = format!("0x{:040x}", Uuid::new_v4().as_u128());
        let owner = format!("wallet:8453:{solver}");
        let input = NewClaimCandidate {
            id: Uuid::new_v4(),
            idempotency_key: service_runtime::auth::hash_secret(&Uuid::new_v4().to_string()),
            network: network.clone(),
            bounty_contract: format!("0x{:040x}", Uuid::new_v4().as_u128()),
            solver_wallet: solver.clone(),
            agent_id: None,
            eligibility_evidence: AgentEligibilityEvidence {
                agent_id: None,
                solver_wallet: solver,
                capabilities: vec![],
                paid_completions: 0,
                paid_usdc_base_units: 0,
            },
            eligibility_decision: AgentEligibilityDecision {
                eligible: true,
                reasons: vec![],
            },
        };
        let fp = service_runtime::auth::hash_secret("local outcome fixture");
        store
            .reserve_owned_claim_candidate(&input, 900, 20, &owner, &fp, 10)
            .await
            .unwrap();
        let deadline = (Utc::now() + ChronoDuration::minutes(10)).timestamp() as u64;
        let nonce = format!("0x{}", "dd".repeat(32));
        store
            .set_claim_candidate_authorization(input.id, &nonce, deadline)
            .await
            .unwrap();
        store
            .begin_owned_claim_relay(input.id, &owner)
            .await
            .unwrap();
        let sponsorship = store
            .reserve_bond_sponsorship(
                &NewBondSponsorship {
                    id: Uuid::new_v4(),
                    claim_candidate_id: input.id,
                    network: network.clone(),
                    bounty_contract: input.bounty_contract.clone(),
                    solver_wallet: input.solver_wallet.clone(),
                    sponsor_wallet: "0x1111111111111111111111111111111111111111".into(),
                    amount: 100,
                },
                10_000,
                1_000,
            )
            .await
            .unwrap();
        let lease = store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        let (candidate, may_broadcast) = store
            .admit_owned_claim_broadcast(input.id, &nonce, deadline, lease)
            .await
            .unwrap();
        assert!(may_broadcast);
        let hash = format!("0x{}", "cc".repeat(32));
        let result = match scenario {
            "simulation" => Ok(Err(ChainBaseError::RelayerSimulation(
                "synthetic local revert".into(),
            ))),
            "provider" => Ok(Err(ChainBaseError::RelayerProvider(
                "synthetic connection lost after send".into(),
            ))),
            "timeout" => Err(tokio::time::timeout(
                Duration::from_millis(1),
                std::future::pending::<()>(),
            )
            .await
            .unwrap_err()),
            "recorded" => Ok(Ok(BaseRelayedTransaction {
                relayer: "0x1111111111111111111111111111111111111111".into(),
                tx_hash: hash.clone(),
                estimated_gas: 1,
                gas_limit: 1,
                max_fee_per_gas_wei: 1,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 1,
            })),
            _ => unreachable!(),
        };
        let outcome = claim_queue_security::finish_broadcast(
            &store,
            candidate,
            Some(sponsorship.id),
            lease,
            result,
        )
        .await;
        let saved = store.get_claim_candidate(input.id).await.unwrap().unwrap();
        let sponsor = store
            .get_bond_sponsorship_for_candidate(input.id)
            .await
            .unwrap()
            .unwrap();
        match scenario {
            "simulation" => {
                assert_eq!(
                    outcome.unwrap_err().1 .0["error"],
                    "claim_prebroadcast_failed"
                );
                assert_eq!(saved.status, ClaimCandidateStatus::Failed);
                assert_eq!(sponsor.status, BondSponsorshipStatus::Failed);
            }
            "recorded" => {
                assert_eq!(
                    outcome.unwrap().claim_transaction_hash.as_deref(),
                    Some(hash.as_str())
                );
                assert_eq!(saved.failure_code, None);
                assert_eq!(sponsor.status, BondSponsorshipStatus::Broadcast);
            }
            _ => {
                assert_eq!(outcome.unwrap().status, ClaimCandidateStatus::Relaying);
                assert!(saved.claim_transaction_hash.is_none());
                assert_eq!(sponsor.status, BondSponsorshipStatus::Reserved);
                assert!(claim_queue_security::pending_next_action(&saved)
                    .contains("operator reconciliation"));
                assert!(store
                    .withdraw_owned_claim_candidate(input.id, &owner)
                    .await
                    .is_err());
            }
        }
        let reacquired = store
            .acquire_x402_relayer_lease(&network, 45)
            .await
            .unwrap()
            .unwrap();
        store
            .release_x402_relayer_lease(&network, reacquired)
            .await
            .unwrap();
    }
}
