//! Real API + PostgreSQL + local Anvil. All writes stay on explicitly checked loopback fixtures.
use super::*;
use alloy::{
    primitives::{keccak256, Address, B256, U256},
    signers::{local::PrivateKeySigner, SignerSync},
    sol_types::SolStruct,
};
use axum::{body::to_bytes, http::Request};
use tower::ServiceExt;
async fn rpc(url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let r: serde_json::Value = reqwest::Client::new()
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(r.get("error").is_none(), "{method}: {r}");
    r["result"].clone()
}
fn artifact(name: &str, kind: &str) -> String {
    let path = format!(
        "{}/../../contracts/base-escrow/out/{name}.sol/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    )
    .replace(
        "/SponsoredCreationFeeOracle.sol/",
        "/SponsoredCreationToken.sol/",
    );
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v[kind]["object"].as_str().unwrap().to_owned()
}
async fn deploy(url: &str, payer: &str, name: &str, argument: &str) -> Address {
    let hash=rpc(url,"eth_sendTransaction",json!([{"from":payer,"data":format!("{}{:0>64}",artifact(name,"bytecode"),argument.trim_start_matches("0x")),"gas":"0x989680","maxFeePerGas":"0x3e8","maxPriorityFeePerGas":"0x1"}])).await;
    for _ in 0..100 {
        let r = rpc(url, "eth_getTransactionReceipt", json!([hash])).await;
        if !r.is_null() {
            assert_eq!(r["status"], "0x1", "{r}");
            return r["contractAddress"].as_str().unwrap().parse().unwrap();
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("fixture deployment receipt missing")
}
async fn code_hash(url: &str, a: Address) -> B256 {
    let c = rpc(url, "eth_getCode", json!([a, "latest"])).await;
    keccak256(hex::decode(c.as_str().unwrap().trim_start_matches("0x")).unwrap())
}
async fn post(app: Router, path: &str, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    let r = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/base/sponsored-setup/{path}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = r.status();
    let bytes = to_bytes(r.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
#[tokio::test]
#[ignore = "requires scripts/check-sponsored-setup.sh with disposable loopback fixtures"]
async fn zero_eth_setup_checks_authority_recovers_and_shares_budget() {
    let url = env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(rpc(&url, "eth_chainId", json!([])).await, "0x14a34");
    assert!(rpc(&url, "web3_clientVersion", json!([]))
        .await
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("anvil"));
    let database = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let dburl = url::Url::parse(&database).unwrap();
    assert_eq!(dburl.host_str(), Some("127.0.0.1"));
    assert!(dburl.path().starts_with("/posting_"));
    let store = PostgresStore::connect(&database).await.unwrap();
    store.migrate().await.unwrap();
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("{:064x}", Uuid::new_v4().as_u128()))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    let payer = relayer.address();
    rpc(
        &url,
        "anvil_setBalance",
        json!([payer, "0x8ac7230489e80000"]),
    )
    .await;
    rpc(&url, "anvil_impersonateAccount", json!([payer])).await;
    let owner: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let attester: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let token = chain_base::BASE_SEPOLIA_USDC_TOKEN_ADDRESS;
    rpc(
        &url,
        "anvil_setCode",
        json!([
            token,
            artifact("SponsoredCreationToken", "deployedBytecode")
        ]),
    )
    .await;
    rpc(
        &url,
        "anvil_setCode",
        json!([
            "0x420000000000000000000000000000000000000F",
            artifact("SponsoredCreationFeeOracle", "deployedBytecode")
        ]),
    )
    .await;
    let factory = deploy(&url, &payer, "AgentBountyFactory", token).await;
    let wallet_factory = deploy(
        &url,
        &payer,
        "BoundedAgentWalletV2Factory",
        &format!("{factory:#x}"),
    )
    .await;
    let implementation=rpc(&url,"eth_call",json!([{"to":wallet_factory,"data":format!("0x{}",hex::encode(&keccak256("implementation()".as_bytes())[..4]))},"latest"])).await;
    let wallet_implementation: Address = format!("0x{}", &implementation.as_str().unwrap()[26..])
        .parse()
        .unwrap();
    let registry = deploy(
        &url,
        &payer,
        "ParticipantEligibilityRegistry",
        &format!("{:#x}", attester.address()),
    )
    .await;
    let release = service::Release {
        network: "base-sepolia".into(),
        bounty_factory: factory,
        bounty_factory_code_hash: code_hash(&url, factory).await,
        wallet_factory,
        wallet_factory_code_hash: code_hash(&url, wallet_factory).await,
        wallet_implementation,
        wallet_implementation_code_hash: code_hash(&url, wallet_implementation).await,
        participant_registry: registry,
        participant_registry_code_hash: code_hash(&url, registry).await,
        deployment_block: 1,
    };
    env::set_var(
        "SPONSORED_SETUP_SEPOLIA_RELEASE_JSON",
        serde_json::to_string(&release).unwrap(),
    );
    env::set_var("ENABLE_SPONSORED_SETUP", "true");
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused",
        store.clone(),
    );
    let mutable = Arc::get_mut(&mut state).unwrap();
    mutable.base_rpc_urls.base_sepolia = Some(url.clone());
    mutable.x402_relayer = X402HostedRelayerConfig {
        max_daily_attempts: 100_000,
        enabled: true,
        relayer: Some(Arc::new(relayer)),
        confirmations: 1,
        wait_seconds: 2,
        rpc_timeout_seconds: 30,
        ..Default::default()
    };
    let app = Router::new()
        .route(
            "/v1/base/sponsored-setup/prepare",
            axum::routing::post(prepare),
        )
        .route("/v1/base/sponsored-setup/relay", axum::routing::post(relay))
        .route(
            "/v1/base/sponsored-setup/status",
            axum::routing::post(status),
        )
        .with_state(state);
    let now = Utc::now().timestamp() as u64;
    let mut request = service::Request {
        network: "base-sepolia".into(),
        owner: format!("{:#x}", owner.address()),
        valid_before: now + 3600,
        action: service::Action::WalletDeploy {
            user_salt: format!("{:#x}", B256::repeat_byte(7)),
            policy: service::Policy {
                delegate: format!("{:#x}", Address::repeat_byte(9)),
                valid_after: now,
                valid_until: now + 86400,
                period_seconds: 86400,
                max_per_action_micro_usdc: 5_000_000,
                max_per_period_micro_usdc: 10_000_000,
                max_lifetime_micro_usdc: 20_000_000,
                max_bounty_target_micro_usdc: 100_000_000,
                allowed_actions: 15,
                allowed_verification_modes: 2,
                deterministic_verifier_module: format!("{:#x}", Address::ZERO),
                signed_quorum_verifier_set_hash: format!("{:#x}", B256::repeat_byte(6)),
                ai_judge_verifier_set_hash: format!("{:#x}", B256::ZERO),
            },
        },
    };
    let budget_before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .remaining_micro_usdc;
    for action in ["wallet", "registration"] {
        if action == "registration" {
            let message = service::ParticipantAttestation {
                wallet: owner.address(),
                participantId: B256::repeat_byte(1),
                sourceHash: B256::repeat_byte(2),
                validUntil: now + 86400,
                nonce: U256::ZERO,
            };
            let digest = message.eip712_signing_hash(&alloy::sol_types::Eip712Domain {
                name: Some("Agent Bounties Participant Registry".into()),
                version: Some("1".into()),
                chain_id: Some(U256::from(84532)),
                verifying_contract: Some(registry),
                salt: None,
            });
            request.action = service::Action::ParticipantRegister {
                participant_id: format!("{:#x}", message.participantId),
                source_hash: format!("{:#x}", message.sourceHash),
                valid_until: message.validUntil,
                registry_nonce: 0,
                attester_signature: format!(
                    "0x{}",
                    hex::encode(attester.sign_hash_sync(&digest).unwrap().as_bytes())
                ),
            };
        }
        let (s, v) = post(app.clone(), "prepare", json!(request)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(v["setup_complete"], false);
        let p = service::prepare(&release, &request).unwrap();
        assert_eq!(v["prepared"]["digest"], json!(p.digest));
        let signed = json!({"request":request,"owner_signature":format!("0x{}",hex::encode(owner.sign_hash_sync(&p.digest).unwrap().as_bytes()))});
        let nonce_before = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
        let mut tampered = signed.clone();
        tampered["request"]["valid_before"] = json!(now + 3599);
        assert_eq!(
            post(app.clone(), "relay", tampered).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
            nonce_before
        );
        let (s, v) = post(app.clone(), "relay", signed.clone()).await;
        assert!(
            [StatusCode::OK, StatusCode::ACCEPTED].contains(&s),
            "{s} {v}"
        );
        let mut complete = false;
        for _ in 0..40 {
            let (s, v) = post(app.clone(), "status", json!(request)).await;
            if s == StatusCode::OK && v["setup_complete"] == true {
                complete = true;
                assert_eq!(v["solver_paid"], false);
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        assert!(complete, "{v}");
        let nonce_after = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
        let (s, v) = post(app.clone(), "relay", signed).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert_eq!(
            rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
            nonce_after
        );
        env::set_var("ENABLE_SPONSORED_SETUP", "false");
        assert_eq!(
            post(app.clone(), "status", json!(request)).await.0,
            StatusCode::OK
        );
        env::set_var("ENABLE_SPONSORED_SETUP", "true");
    }
    assert!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .remaining_micro_usdc
            < budget_before
    );
    assert_eq!(
        rpc(&url, "eth_getBalance", json!([owner.address(), "latest"])).await,
        "0x0"
    );
    assert_eq!(
        rpc(
            &url,
            "eth_getTransactionCount",
            json!([owner.address(), "latest"])
        )
        .await,
        "0x0"
    );
    println!("sponsored_setup: zero-ETH wallet deployment and attested registration, tamper rejection, canonical status, exact retry, flag-off recovery and shared spend passed");
}
