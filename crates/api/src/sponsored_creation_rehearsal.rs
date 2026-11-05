//! Opt-in loopback Anvil + disposable Postgres rehearsal. No hosted writes.
use super::*;
use alloy::{
    primitives::{B256, U256},
    signers::{local::PrivateKeySigner, SignerSync},
    sol_types::SolStruct,
};
use axum::{body::to_bytes, http::Request};
use serde_json::{json, Value};
use tower::ServiceExt;

alloy::sol! {
    struct TransferWithAuthorization {
        address from; address to; uint256 value; uint256 validAfter;
        uint256 validBefore; bytes32 nonce;
    }
}

async fn rpc(url: &str, method: &str, params: serde_json::Value) -> serde_json::Value {
    let result: serde_json::Value = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(result.get("error").is_none(), "{method}: {result}");
    result["result"].clone()
}

fn artifact(name: &str, kind: &str) -> String {
    let path = format!(
        "{}/../../contracts/base-escrow/out/{name}.sol/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    // The oracle and token share one Solidity source file.
    let path = path.replace(
        "/SponsoredCreationFeeOracle.sol/",
        "/SponsoredCreationToken.sol/",
    );
    let path = path.replace(
        "/LegacyCreation1271.sol/",
        "/LegacyCreationContractWallet.fork.t.sol/",
    );
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("run forge build first")).unwrap();
    json[kind]["object"].as_str().unwrap().to_owned()
}

async fn transaction(url: &str, mut tx: serde_json::Value) -> serde_json::Value {
    // Keep local setup tips small so feeHistory models low-cost Base execution.
    tx["maxFeePerGas"] = "0x3e8".into();
    tx["maxPriorityFeePerGas"] = "0x1".into();
    let hash = rpc(url, "eth_sendTransaction", serde_json::json!([tx])).await;
    let mut receipt = serde_json::Value::Null;
    for _ in 0..100 {
        receipt = rpc(url, "eth_getTransactionReceipt", serde_json::json!([hash])).await;
        if !receipt.is_null() {
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(receipt["status"], "0x1", "{receipt}");
    receipt
}

fn word_call(signature: &str, words: &[String]) -> String {
    format!(
        "0x{}{}",
        hex::encode(&alloy::primitives::keccak256(signature.as_bytes())[..4]),
        words.join("")
    )
}

async fn post(app: axum::Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    post_to(app, "/v1/base/autonomous-bounties/creation-relay", body).await
}
async fn post_to(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    post_authenticated(app, path, body, None).await
}
async fn post_confirmed(app: axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    for _ in 0..20 {
        let result = post_to(app.clone(), path, body.clone()).await;
        if result.0 != StatusCode::ACCEPTED {
            return result;
        }
        sleep(Duration::from_millis(100)).await;
    }
    post_to(app, path, body).await
}
async fn post_authenticated(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
    token: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder();
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .oneshot(
            builder
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

#[tokio::test]
#[ignore = "run scripts/check-sponsored-creation.sh with a disposable loopback database"]
async fn zero_eth_creator_posts_once_through_real_api_database_and_evm() {
    let url = env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed.scheme(), "http");
    assert_eq!(
        rpc(&url, "eth_chainId", serde_json::json!([])).await,
        "0x14a34"
    );
    assert!(rpc(&url, "web3_clientVersion", serde_json::json!([]))
        .await
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("anvil"));
    let database = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&database).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/posting_"));
    let store = PostgresStore::connect(&database).await.unwrap();
    store.migrate().await.unwrap();
    // Public Anvil fixture key. All transactions are guarded to local Anvil.
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("{:064x}", Uuid::new_v4().as_u128()))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    let payer = relayer.address();
    rpc(
        &url,
        "anvil_setBalance",
        serde_json::json!([payer, "0x8ac7230489e80000"]),
    )
    .await;
    rpc(&url, "anvil_impersonateAccount", serde_json::json!([payer])).await;
    let creator: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let owner = format!("{:#x}", creator.address());
    assert_eq!(
        rpc(&url, "eth_getBalance", serde_json::json!([owner, "latest"])).await,
        "0x0"
    );
    let token = chain_base::BASE_SEPOLIA_USDC_TOKEN_ADDRESS;
    rpc(
        &url,
        "anvil_setCode",
        serde_json::json!([
            token,
            artifact("SponsoredCreationToken", "deployedBytecode")
        ]),
    )
    .await;
    rpc(
        &url,
        "anvil_setCode",
        serde_json::json!([
            "0x420000000000000000000000000000000000000F",
            artifact("SponsoredCreationFeeOracle", "deployedBytecode")
        ]),
    )
    .await;
    let receipt = transaction(&url, serde_json::json!({"from":payer,"data":format!("{}{:0>64}",artifact("AgentBountyFactory", "bytecode"),token.trim_start_matches("0x")),"gas":"0x989680"})).await;
    let factory = receipt["contractAddress"].as_str().unwrap();
    let implementation = rpc(
        &url,
        "eth_call",
        serde_json::json!([{"to":factory,"data":word_call("implementation()", &[])},"latest"]),
    )
    .await;
    env::set_var("BASE_SEPOLIA_BOUNTY_FACTORY", factory);
    env::set_var(
        "BASE_SEPOLIA_BOUNTY_IMPLEMENTATION",
        format!("0x{}", &implementation.as_str().unwrap()[26..]),
    );
    let mint = word_call(
        "mint(address,uint256)",
        &[
            format!("{:0>64}", owner.trim_start_matches("0x")),
            format!("{:064x}", 42_600_000),
        ],
    );
    transaction(
        &url,
        serde_json::json!({"from":payer,"to":token,"data":mint}),
    )
    .await;

    let send_mode = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent_bytes = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let proxy=axum::Router::new().route("/",axum::routing::post({
        let upstream=url.clone(); let mode=send_mode.clone();let seen=sent_bytes.clone();
        move |Json(body):Json<Value>| {
            let upstream=upstream.clone();let mode=mode.clone();let seen=seen.clone();
            async move {
                let fault=if body["method"]=="eth_sendRawTransaction" {
                    seen.lock().unwrap().push(body["params"][0].as_str().unwrap().to_owned());
                    mode.swap(0,std::sync::atomic::Ordering::SeqCst)
                } else {0};
                let reply=if fault==1 {Value::Null} else {reqwest::Client::new().post(upstream).json(&body).send().await.unwrap().json::<Value>().await.unwrap()};
                Json(if fault!=0 {json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32000,"message":"fixture lost send reply"}})} else {reply})
            }
        }
    }));
    let proxy_task = tokio::spawn(async move { axum::serve(listener, proxy).await.unwrap() });
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "unused",
        store.clone(),
    );
    let mutable = Arc::get_mut(&mut state).unwrap();
    mutable.base_rpc_urls.base_sepolia = Some(proxy_url);
    mutable.x402_relayer = X402HostedRelayerConfig {
        max_daily_attempts: 100_000, // This disposable DB is reused across independent local runs.
        enabled: true,
        relayer: Some(Arc::new(relayer)),
        confirmations: 1,
        wait_seconds: 2,
        rpc_timeout_seconds: 30,
        ..Default::default()
    };
    assert!(
        enabled(&state, "base-sepolia"),
        "script must enable sponsored creation"
    );
    let now = Utc::now();
    let mut document: AutonomousBountyTermsDocument =
        serde_json::from_str(include_str!("../../../bounties/autonomous-v1/244.json")).unwrap();
    document.contract_terms["creator_wallet"] = owner.clone().into();
    document.contract_terms["network"] = "base-sepolia".into();
    document.contract_terms["settlement_token"] = token.into();
    document.contract_terms["funding_deadline"] = (now.timestamp() + 86_400).into();
    document.contract_terms["creation_nonce"] =
        format!("0x{:064x}", Uuid::new_v4().as_u128()).into();
    document.contract_terms["solver_reward"]["amount"] = 20_300_000.into();
    document.contract_terms["verifier_reward"]["amount"] = 1_000_000.into();
    document.contract_terms["claim_bond"]["amount"] = 1_000_000.into();
    document.contract_terms["initial_funding"]["amount"] = 21_300_000.into();
    document.title = "Local creator review rehearsal".into();
    document.goal = "Exercise signed review and canonical payout with fixture funds".into();
    document.acceptance_criteria =
        vec!["The submitted fixture artifact matches the approved result".into()];
    document.benchmark = serde_json::json!({"engine":"creator_review_v1","reviewer":"creator","acceptance":"all_published_criteria","delivery_deadline":now.timestamp()+86_400});
    document.verification_policy = serde_json::json!({"engine":"creator_review_v1","mechanism":"signed_quorum","threshold":1,"verifiers":[owner],"public_disclosure":"The creator signs the human review verdict."});
    document.evidence_schema = serde_json::json!({"type":"object","required":["artifact_url","artifact_sha256"],"properties":{"artifact_url":{"type":"string","pattern":"^https://"},"artifact_sha256":{"type":"string","pattern":"^sha256:[0-9a-f]{64}$"}}});
    let terms = build_autonomous_bounty_terms_record(&owner, document, now).unwrap();
    store.upsert_autonomous_bounty_terms(&terms).await.unwrap();
    let create = chain_base::autonomous_bounty_create_from_terms(&terms).unwrap();
    let expiry = now.timestamp() as u64 + 7200;
    let plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation_with_expiry("base-sepolia", &create, expiry)
        .unwrap();
    let typed = plan.eip3009_authorization.unwrap();
    let message = TransferWithAuthorization {
        from: creator.address(),
        to: typed.message.to.parse().unwrap(),
        value: U256::from(21_300_000),
        validAfter: U256::ZERO,
        validBefore: U256::from(expiry),
        nonce: typed.message.nonce.parse::<B256>().unwrap(),
    };
    let domain = alloy::sol_types::Eip712Domain {
        name: Some(typed.domain.name.into()),
        version: Some("2".into()),
        chain_id: Some(U256::from(84532)),
        verifying_contract: Some(token.parse().unwrap()),
        salt: None,
    };
    let signed = creator
        .sign_hash_sync(&message.eip712_signing_hash(&domain))
        .unwrap()
        .as_bytes();
    let body = serde_json::json!({"network":"base-sepolia","authorization_valid_before":expiry,"create":create,"signature":{"v":signed[64],"r":format!("0x{}",hex::encode(&signed[..32])),"s":format!("0x{}",hex::encode(&signed[32..64]))}});
    let app = axum::Router::new()
        .route(
            "/v1/x402/base/relays/:relay_id",
            axum::routing::get(crate::get_x402_relay),
        )
        .route(
            "/v1/base/autonomous-bounties/creation-relay",
            axum::routing::post(relay),
        )
        .route(
            "/v1/base/autonomous-bounties/creator-verdict-relay",
            axum::routing::post(crate::sponsored_verdict::relay),
        )
        .route(
            "/v1/base/autonomous-bounties/submission-relay",
            axum::routing::post(crate::sponsored_submission::relay),
        )
        .route(
            "/v1/base/autonomous-bounties/claims",
            axum::routing::post(crate::agent_native_claim),
        )
        .route(
            "/v1/base/autonomous-bounties/contribution-relay",
            axum::routing::post(crate::sponsored_contribution::relay),
        )
        .route(
            "/v1/base/usdc/authorization-revocation-relay",
            axum::routing::post(crate::sponsored_revocation::relay),
        )
        .with_state(state.clone());
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let payer_nonce = rpc(
        &url,
        "eth_getTransactionCount",
        serde_json::json!([payer, "latest"]),
    )
    .await;
    let mut invalid = body.clone();
    invalid["signature"]["r"] = format!("0x{}", "11".repeat(32)).into();
    assert_eq!(post(app.clone(), invalid).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let (status, first) = post(app.clone(), body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["funding_confirmed"], true);
    assert_eq!(first["solver_paid"], false);
    assert_eq!(first["customer_gas_wei"], "0");
    let after = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    assert!(after > before && after <= 1_000_000);
    // Two callers retry the same signature after the first response is lost.
    env::set_var("ENABLE_SPONSORED_BOUNTY_CREATION", "false");
    let (a, b) = tokio::join!(post(app.clone(), body.clone()), post(app.clone(), body));
    env::set_var("ENABLE_SPONSORED_BOUNTY_CREATION", "true");
    assert_eq!(a.0, StatusCode::OK);
    assert_eq!(b.0, StatusCode::OK);
    assert_eq!(a.1["relay"]["id"], first["relay"]["id"]);
    assert!(first["relay"]["transaction"]
        .as_str()
        .is_some_and(|value| value.len() == 66));
    assert_eq!(b.1["relay"]["transaction"], first["relay"]["transaction"]);
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        after
    );
    let nonce_after = rpc(
        &url,
        "eth_getTransactionCount",
        serde_json::json!([payer, "latest"]),
    )
    .await;
    let number = |v: &serde_json::Value| {
        u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    };
    assert_eq!(number(&nonce_after), number(&payer_nonce) + 1);
    assert_eq!(
        rpc(&url, "eth_getBalance", serde_json::json!([owner, "latest"])).await,
        "0x0"
    );
    for address in [&owner, &plan.predicted_bounty_contract] {
        let balance = rpc(&url,"eth_call",serde_json::json!([{"to":token,"data":word_call("balanceOf(address)",&[format!("{:0>64}",address.trim_start_matches("0x"))])},"latest"])).await;
        assert_eq!(number(&balance), 21_300_000);
    }
    let events = store
        .list_autonomous_bounty_events("base-sepolia")
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == AutonomousBountyEventKind::FundingAdded
                && e.contract_address
                    .eq_ignore_ascii_case(&plan.predicted_bounty_contract))
            .count(),
        1
    );
    println!("LOCAL ONLY: exact 21.30 USDC funded once; creator ETH remains zero; gas reservation delta {} micro-USDC; replay sent no second transaction",after-before);
    let solver_key: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let solver = format!("{:#x}", solver_key.address());
    // Real wallet-only principal: authentication cannot substitute for the
    // independent bond authorization or grant a different wallet claim rights.
    let login =
        service_runtime::auth::AuthService::new(store.clone(), "http://127.0.0.1:3000", 84532)
            .unwrap();
    let challenge = login
        .begin_challenge(&solver, &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let login_signature = solver_key
        .sign_message_sync(challenge.message.as_bytes())
        .unwrap();
    let session = login
        .complete_challenge(
            challenge.challenge_id,
            &format!("0x{}", hex::encode(login_signature.as_bytes())),
            &service_runtime::auth::RpcWalletVerifier::new(url.clone(), 84532).unwrap(),
        )
        .await
        .unwrap();
    let bounty = &plan.predicted_bounty_contract;
    for (round, passed) in [(1_u64, false), (2, true)] {
        let mint = word_call(
            "mint(address,uint256)",
            &[
                format!("{:0>64}", solver.trim_start_matches("0x")),
                format!("{:064x}", 1_000_000),
            ],
        );
        transaction(
            &url,
            serde_json::json!({"from":payer,"to":token,"data":mint}),
        )
        .await;
        let claim_path = "/v1/base/autonomous-bounties/claims";
        let mut claim_body = serde_json::json!({"idempotency_key":format!("rehearsal:{bounty}:{round}"),"network":"base-sepolia","bounty_contract":bounty,"solver_wallet":solver,"request_bond_sponsorship":false});
        assert_eq!(
            post_to(app.clone(), claim_path, claim_body.clone()).await.0,
            StatusCode::UNAUTHORIZED
        );
        let mut foreign = claim_body.clone();
        foreign["solver_wallet"] = owner.clone().into();
        assert_eq!(
            post_authenticated(app.clone(), claim_path, foreign, Some(&session.token))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let (status, prepared) = post_authenticated(
            app.clone(),
            claim_path,
            claim_body.clone(),
            Some(&session.token),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{prepared}");
        let auth: Eip3009AuthorizationTypedData =
            serde_json::from_value(prepared["signing_payload"].clone()).unwrap();
        let claim_message = TransferWithAuthorization {
            from: solver_key.address(),
            to: bounty.parse().unwrap(),
            value: U256::from(1_000_000),
            validAfter: auth.message.valid_after.parse().unwrap(),
            validBefore: auth.message.valid_before.parse().unwrap(),
            nonce: auth.message.nonce.parse().unwrap(),
        };
        let claim_signature = solver_key
            .sign_hash_sync(&claim_message.eip712_signing_hash(&domain))
            .unwrap();
        claim_body["wallet_signature"] =
            format!("0x{}", hex::encode(claim_signature.as_bytes())).into();
        let mut claimed = serde_json::Value::Null;
        for _ in 0..10 {
            let (status, response) = post_authenticated(
                app.clone(),
                claim_path,
                claim_body.clone(),
                Some(&session.token),
            )
            .await;
            assert!(
                [StatusCode::OK, StatusCode::ACCEPTED].contains(&status),
                "{status}: {response}"
            );
            if response["candidate"]["status"] == "claimed" {
                claimed = response;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(claimed["candidate"]["status"], "claimed", "{claimed}");
        let submission_hash = format!("0x{:064x}", round + 100);
        let evidence_hash = format!("0x{:064x}", round + 200);
        let submission = AutonomousBountySubmissionAuthorizationRequest {
            bounty_contract: bounty.clone(),
            bounty_id: plan.bounty_id.clone(),
            solver: solver.clone(),
            round,
            submission_hash: submission_hash.clone(),
            evidence_hash: evidence_hash.clone(),
            policy_hash: terms.policy_hash.clone(),
            deadline: Utc::now().timestamp() as u64 + 600,
        };
        let signed = solver_key
            .sign_hash_sync(
                &crate::sponsored_submission::digest("base-sepolia", &submission).unwrap(),
            )
            .unwrap();
        let submission_body = serde_json::json!({"network":"base-sepolia","submission":submission,"signature":format!("0x{}",hex::encode(signed.as_bytes()))});
        let path = "/v1/base/autonomous-bounties/submission-relay";
        let mut tampered = submission_body.clone();
        tampered["submission"]["round"] = (round + 1).into();
        assert_eq!(
            post_to(app.clone(), path, tampered).await.0,
            StatusCode::UNAUTHORIZED
        );
        let nonce = rpc(
            &url,
            "eth_getTransactionCount",
            serde_json::json!([payer, "latest"]),
        )
        .await;
        let mut submitted = serde_json::Value::Null;
        for _ in 0..10 {
            let (status, response) = post_to(app.clone(), path, submission_body.clone()).await;
            assert!(
                [StatusCode::OK, StatusCode::ACCEPTED].contains(&status),
                "{status}: {response}"
            );
            if response["confirmed"] == true {
                submitted = response;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(submitted["confirmed"], true, "{submitted}");
        assert_eq!(submitted["solver_paid"], false);
        let budget = store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc;
        let (retry_a, retry_b) = tokio::join!(
            post_to(app.clone(), path, submission_body.clone()),
            post_to(app.clone(), path, submission_body)
        );
        assert_eq!(retry_a.0, StatusCode::OK);
        assert_eq!(retry_b.0, StatusCode::OK);
        assert_eq!(retry_a.1["transaction_hash"], submitted["transaction_hash"]);
        assert_eq!(
            number(
                &rpc(
                    &url,
                    "eth_getTransactionCount",
                    serde_json::json!([payer, "latest"])
                )
                .await
            ),
            number(&nonce) + 1
        );
        assert_eq!(
            store
                .gas_sponsor_budget_status()
                .await
                .unwrap()
                .reserved_micro_usdc,
            budget
        );
        assert_eq!(
            rpc(
                &url,
                "eth_getBalance",
                serde_json::json!([solver, "latest"])
            )
            .await,
            "0x0"
        );
        println!("LOCAL ONLY: solver claim and submission round {round}; solver ETH remains zero; repeated submission reused exact canonical event");
        let request = AutonomousVerificationAttestationRequest {
            bounty_contract: bounty.clone(),
            bounty_id: plan.bounty_id.clone(),
            round,
            verifier: owner.clone(),
            submission_hash,
            evidence_hash,
            policy_hash: terms.policy_hash.clone(),
            passed,
            response_hash: format!("0x{:064x}", round + 300),
            deadline: Utc::now().timestamp() as u64 + 600,
        };
        let signature = creator
            .sign_hash_sync(&crate::sponsored_verdict::digest("base-sepolia", &request).unwrap())
            .unwrap();
        let body = serde_json::json!({"network":"base-sepolia","attestation":request,"signature":format!("0x{}",hex::encode(signature.as_bytes()))});
        let path = "/v1/base/autonomous-bounties/creator-verdict-relay";
        let mut tampered = body.clone();
        tampered["attestation"]["passed"] = (!passed).into();
        assert_eq!(
            post_to(app.clone(), path, tampered).await.0,
            StatusCode::UNAUTHORIZED
        );
        let nonce = rpc(
            &url,
            "eth_getTransactionCount",
            serde_json::json!([payer, "latest"]),
        )
        .await;
        let mut confirmed = serde_json::Value::Null;
        for _ in 0..10 {
            let (status, response) = post_to(app.clone(), path, body.clone()).await;
            assert!(
                [StatusCode::OK, StatusCode::ACCEPTED].contains(&status),
                "{status}: {response}"
            );
            if status == StatusCode::OK {
                confirmed = response;
                break;
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(confirmed["confirmed"], true, "{confirmed}");
        assert_eq!(confirmed["solver_paid"], passed);
        assert_eq!(confirmed["customer_gas_wei"], "0");
        let budget = store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc;
        let (retry_a, retry_b) = tokio::join!(
            post_to(app.clone(), path, body.clone()),
            post_to(app.clone(), path, body)
        );
        assert_eq!(retry_a.0, StatusCode::OK);
        assert_eq!(retry_b.0, StatusCode::OK);
        assert_eq!(retry_a.1["transaction_hash"], confirmed["transaction_hash"]);
        assert_eq!(
            store
                .gas_sponsor_budget_status()
                .await
                .unwrap()
                .reserved_micro_usdc,
            budget
        );
        assert_eq!(
            number(
                &rpc(
                    &url,
                    "eth_getTransactionCount",
                    serde_json::json!([payer, "latest"])
                )
                .await
            ),
            number(&nonce) + 1
        );
        assert_eq!(
            rpc(&url, "eth_getBalance", serde_json::json!([owner, "latest"])).await,
            "0x0"
        );
        println!("LOCAL ONLY: creator verdict round {round}, passed={passed}; platform paid gas once; concurrent replay reused canonical outcome");
    }
    let paid=rpc(&url,"eth_call",serde_json::json!([{"to":token,"data":word_call("balanceOf(address)",&[format!("{:0>64}",solver.trim_start_matches("0x"))])},"latest"])).await;
    assert_eq!(
        number(&paid),
        21_300_000,
        "solver reward plus returned second-round bond"
    );
    // An independently created, unfunded canonical fixture exercises pooled funding.
    // Its setup creator is the funded Anvil payer; the contributor still has zero ETH.
    let mut funding_document = terms.document.clone();
    funding_document.contract_terms["creator_wallet"] = payer.clone().into();
    funding_document.contract_terms["creation_nonce"] =
        format!("0x{:064x}", Uuid::new_v4().as_u128()).into();
    funding_document.contract_terms["initial_funding"]["amount"] = 0.into();
    funding_document.verification_policy["verifiers"] = serde_json::json!([payer]);
    let funding_terms =
        build_autonomous_bounty_terms_record(&payer, funding_document, Utc::now()).unwrap();
    store
        .upsert_autonomous_bounty_terms(&funding_terms)
        .await
        .unwrap();
    let funding_create = chain_base::autonomous_bounty_create_from_terms(&funding_terms).unwrap();
    let funding_plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation("base-sepolia", &funding_create)
        .unwrap();
    let created = transaction(&url,serde_json::json!({"from":payer,"to":factory,"data":funding_plan.create_bounty.data,"gas":"0x989680"})).await;
    let receipt: RpcTransactionReceipt = serde_json::from_value(created).unwrap();
    for event in decode_autonomous_bounty_logs(receipt.logs_to_evm_logs().unwrap()).unwrap() {
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();
    }
    let expiry = Utc::now().timestamp() as u64 + 600;
    let nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let c = AutonomousBountyContribution {
        bounty_contract: funding_plan.predicted_bounty_contract.clone(),
        contributor: owner.clone(),
        amount: Money::new(21_300_000, "usdc").unwrap(),
        authorization_nonce: Some(nonce.clone()),
        authorization_valid_before: Some(expiry),
    };
    let signed = creator
        .sign_hash_sync(
            &TransferWithAuthorization {
                from: creator.address(),
                to: c.bounty_contract.parse().unwrap(),
                value: U256::from(21_300_000),
                validAfter: U256::ZERO,
                validBefore: U256::from(expiry),
                nonce: nonce.parse().unwrap(),
            }
            .eip712_signing_hash(&domain),
        )
        .unwrap();
    let body = serde_json::json!({"network":"base-sepolia","contribution":c,"signature":format!("0x{}",hex::encode(signed.as_bytes()))});
    let path = "/v1/base/autonomous-bounties/contribution-relay";
    let mut altered = body.clone();
    altered["contribution"]["amount"]["amount"] = 21_300_001.into();
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    assert_eq!(
        post_to(app.clone(), path, altered).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let nonce_before = rpc(
        &url,
        "eth_getTransactionCount",
        serde_json::json!([payer, "latest"]),
    )
    .await;
    send_mode.store(1, std::sync::atomic::Ordering::SeqCst);
    let (pending_code, pending) = post_to(app.clone(), path, body.clone()).await;
    assert_eq!(pending_code, StatusCode::ACCEPTED, "{pending}");
    let attempt = store
        .get_x402_relay_attempt_by_authorization("base-sepolia", &c.bounty_contract, &nonce)
        .await
        .unwrap()
        .unwrap();
    let count = sent_bytes.lock().unwrap().len();
    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/x402/base/relays/{}", attempt.id))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::ACCEPTED);
    assert_eq!(
        sent_bytes.lock().unwrap().len(),
        count,
        "GET status must not resend"
    );
    sleep(Duration::from_secs(11)).await;
    let (mut status, mut reply) = post_to(app.clone(), path, body.clone()).await;
    assert_eq!(
        sent_bytes.lock().unwrap().len(),
        count + 1,
        "explicit retry must resend once"
    );
    for _ in 0..30 {
        if status != StatusCode::ACCEPTED {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        (status, reply) = post_to(app.clone(), path, body.clone()).await;
    }
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert_eq!(reply["funding_confirmed"], true);
    assert_eq!(
        reply["relay"]["transaction"],
        pending["relay"]["transaction"]
    );
    {
        let sent = sent_bytes.lock().unwrap();
        assert_eq!(
            sent[count - 1],
            sent[count],
            "contribution retries use byte-identical transactions"
        );
    }
    assert_eq!(reply["solver_paid"], false);
    let after = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let (retry_a, retry_b) = tokio::join!(
        post_to(app.clone(), path, body.clone()),
        post_to(app.clone(), path, body)
    );
    assert_eq!(retry_a.0, StatusCode::OK);
    assert_eq!(retry_b.0, StatusCode::OK);
    assert_eq!(
        retry_a.1["relay"]["transaction"],
        reply["relay"]["transaction"]
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        after
    );
    assert_eq!(
        number(
            &rpc(
                &url,
                "eth_getTransactionCount",
                serde_json::json!([payer, "latest"])
            )
            .await
        ),
        number(&nonce_before) + 1
    );
    assert_eq!(
        rpc(&url, "eth_getBalance", serde_json::json!([owner, "latest"])).await,
        "0x0"
    );
    println!("LOCAL ONLY: 21.30 USDC pooled contribution relayed once; contributor ETH remains zero; retry succeeds after funding closes");
    let nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let cancel_signature = creator
        .sign_hash_sync(
            &crate::sponsored_revocation::digest("base-sepolia", &owner, &nonce).unwrap(),
        )
        .unwrap();
    let cancel_body = serde_json::json!({"network":"base-sepolia","authorizer":owner,"nonce":nonce,"signature":format!("0x{}",hex::encode(cancel_signature.as_bytes()))});
    let path = "/v1/base/usdc/authorization-revocation-relay";
    let mut altered = cancel_body.clone();
    altered["nonce"] = format!("0x{}", "ff".repeat(32)).into();
    assert_eq!(
        post_to(app.clone(), path, altered).await.0,
        StatusCode::UNAUTHORIZED
    );
    let mut cancelled = serde_json::Value::Null;
    for _ in 0..10 {
        let (status, result) = post_to(app.clone(), path, cancel_body.clone()).await;
        assert!(
            [StatusCode::OK, StatusCode::ACCEPTED].contains(&status),
            "{status}: {result}"
        );
        if result["authorization_cancelled"] == true {
            cancelled = result;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(cancelled["authorization_cancelled"], true, "{cancelled}");
    let budget = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let retry = post_to(app.clone(), path, cancel_body).await;
    assert_eq!(retry.0, StatusCode::OK);
    assert_eq!(retry.1["transaction_hash"], cancelled["transaction_hash"]);
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        budget
    );
    assert_eq!(
        rpc(&url, "eth_getBalance", serde_json::json!([owner, "latest"])).await,
        "0x0"
    );
    println!("LOCAL ONLY: exact USDC nonce revoked with platform gas; original wallet remains at zero ETH; retry sends no second transaction");

    // The same legacy factory supports deployed ERC-1271 wallets whose exact
    // signature fits its fixed v/r/s ABI. No creator-paid fallback is needed.
    let wallet_receipt=transaction(&url,json!({"from":payer,"data":format!("{}{:0>64}",artifact("LegacyCreation1271","bytecode"),owner.trim_start_matches("0x")),"gas":"0x989680"})).await;
    let contract_creator = wallet_receipt["contractAddress"]
        .as_str()
        .unwrap()
        .to_owned();
    transaction(&url,json!({"from":payer,"to":token,"data":word_call("mint(address,uint256)",&[format!("{:0>64}",contract_creator.trim_start_matches("0x")),format!("{:064x}",21_300_000)])})).await;
    let mut contract_document = terms.document.clone();
    contract_document.contract_terms["creator_wallet"] = contract_creator.clone().into();
    contract_document.contract_terms["creation_nonce"] =
        format!("0x{:064x}", Uuid::new_v4().as_u128()).into();
    contract_document.verification_policy["verifiers"] = json!([contract_creator]);
    let contract_terms =
        build_autonomous_bounty_terms_record(&contract_creator, contract_document, Utc::now())
            .unwrap();
    store
        .upsert_autonomous_bounty_terms(&contract_terms)
        .await
        .unwrap();
    let contract_create = chain_base::autonomous_bounty_create_from_terms(&contract_terms).unwrap();
    let expiry = Utc::now().timestamp() as u64 + 3600;
    let contract_plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation_with_expiry("base-sepolia", &contract_create, expiry)
        .unwrap();
    let digest = chain_base::creation_authorization_digest(
        contract_plan.eip3009_authorization.as_ref().unwrap(),
    )
    .unwrap();
    let signature = creator.sign_hash_sync(&digest).unwrap().as_bytes();
    let body = json!({"network":"base-sepolia","create":contract_create,"authorization_valid_before":expiry,"signature":{"r":format!("0x{}",hex::encode(&signature[..32])),"s":format!("0x{}",hex::encode(&signature[32..64])),"v":signature[64]}});
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let mut wrong = body.clone();
    wrong["signature"]["r"] = format!("0x{}", "11".repeat(32)).into();
    assert_eq!(post(app.clone(), wrong).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let (status, created) = post(app.clone(), body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["funding_confirmed"], true);
    assert_eq!(created["solver_paid"], false);
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(post(app.clone(), body).await.0, StatusCode::OK);
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );
    assert_eq!(
        rpc(&url, "eth_getBalance", json!([contract_creator, "latest"])).await,
        "0x0"
    );
    println!("LOCAL ONLY: compatible ERC-1271 creator funded 21.30 USDC with zero ETH; invalid contract signature rejected before gas and exact retry did not duplicate funding");
    // Contract identities keep their authority through the actual authenticated
    // claim route, then use wider ERC-1271 signatures for submission and verdict.
    let wallet_receipt=transaction(&url,json!({"from":payer,"data":format!("{}{:0>64}",artifact("LegacyCreation1271","bytecode"),solver.trim_start_matches("0x")),"gas":"0x989680"})).await;
    let contract_solver = wallet_receipt["contractAddress"]
        .as_str()
        .unwrap()
        .to_owned();
    transaction(&url,json!({"from":payer,"to":token,"data":word_call("mint(address,uint256)",&[format!("{:0>64}",contract_solver.trim_start_matches("0x")),format!("{:064x}",1_000_000)])})).await;
    rpc(&url, "anvil_mine", json!(["0x41", "0x0"])).await;
    let challenge = login
        .begin_challenge(&contract_solver, &Uuid::new_v4().to_string())
        .await
        .unwrap();
    let sig = solver_key
        .sign_message_sync(challenge.message.as_bytes())
        .unwrap();
    let contract_session = login
        .complete_challenge(
            challenge.challenge_id,
            &format!("0x{}", hex::encode(sig.as_bytes())),
            &service_runtime::auth::RpcWalletVerifier::new(url.clone(), 84532).unwrap(),
        )
        .await
        .unwrap();
    let bounty = &contract_plan.predicted_bounty_contract;
    let claim_path = "/v1/base/autonomous-bounties/claims";
    let mut claim = json!({"idempotency_key":Uuid::new_v4().to_string(),"network":"base-sepolia","bounty_contract":bounty,"solver_wallet":contract_solver,"request_bond_sponsorship":false});
    let (status, prepared) = post_authenticated(
        app.clone(),
        claim_path,
        claim.clone(),
        Some(&contract_session.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{prepared}");
    let auth: Eip3009AuthorizationTypedData =
        serde_json::from_value(prepared["signing_payload"].clone()).unwrap();
    let sig = solver_key
        .sign_hash_sync(&chain_base::creation_authorization_digest(&auth).unwrap())
        .unwrap();
    claim["wallet_signature"] = format!("0x{}", hex::encode(sig.as_bytes())).into();
    let (status, claimed) = post_authenticated(
        app.clone(),
        claim_path,
        claim.clone(),
        Some(&contract_session.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    assert_eq!(claimed["candidate"]["status"], "claimed", "{claimed}");
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(
        post_authenticated(
            app.clone(),
            claim_path,
            claim,
            Some(&contract_session.token)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );
    let submission = AutonomousBountySubmissionAuthorizationRequest {
        bounty_contract: bounty.clone(),
        bounty_id: contract_plan.bounty_id.clone(),
        solver: contract_solver.clone(),
        round: 1,
        submission_hash: format!("0x{}", "a1".repeat(32)),
        evidence_hash: format!("0x{}", "a2".repeat(32)),
        policy_hash: contract_terms.policy_hash.clone(),
        deadline: Utc::now().timestamp() as u64 + 600,
    };
    let sig = solver_key
        .sign_hash_sync(&crate::sponsored_submission::digest("base-sepolia", &submission).unwrap())
        .unwrap();
    let submitted = json!({"network":"base-sepolia","submission":submission,"signature":format!("0x7f{}",hex::encode(sig.as_bytes()))});
    let path = "/v1/base/autonomous-bounties/submission-relay";
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let mut bad = submitted.clone();
    bad["submission"]["evidence_hash"] = format!("0x{}", "a3".repeat(32)).into();
    assert_eq!(
        post_to(app.clone(), path, bad).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let (status, result) = post_confirmed(app.clone(), path, submitted.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["solver_paid"], false);
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(
        post_to(app.clone(), path, submitted).await.0,
        StatusCode::OK
    );
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );
    let review = AutonomousVerificationAttestationRequest {
        bounty_contract: bounty.clone(),
        bounty_id: contract_plan.bounty_id.clone(),
        round: 1,
        verifier: contract_creator.clone(),
        submission_hash: submission.submission_hash,
        evidence_hash: submission.evidence_hash,
        policy_hash: contract_terms.policy_hash.clone(),
        passed: true,
        response_hash: format!("0x{}", "a4".repeat(32)),
        deadline: Utc::now().timestamp() as u64 + 600,
    };
    let sig = creator
        .sign_hash_sync(&crate::sponsored_verdict::digest("base-sepolia", &review).unwrap())
        .unwrap();
    let review = json!({"network":"base-sepolia","attestation":review,"signature":format!("0x7f{}",hex::encode(sig.as_bytes()))});
    let path = "/v1/base/autonomous-bounties/creator-verdict-relay";
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let mut bad = review.clone();
    bad["attestation"]["passed"] = false.into();
    assert_eq!(
        post_to(app.clone(), path, bad).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let (status, result) = post_confirmed(app.clone(), path, review.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["solver_paid"], true);
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(post_to(app.clone(), path, review).await.0, StatusCode::OK);
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );
    let paid=rpc(&url,"eth_call",json!([{"to":token,"data":word_call("balanceOf(address)",&[format!("{:0>64}",contract_solver.trim_start_matches("0x"))])},"latest"])).await;
    assert_eq!(number(&paid), 21_300_000);
    for wallet in [&contract_creator, &contract_solver] {
        assert_eq!(
            rpc(&url, "eth_getBalance", json!([wallet, "latest"])).await,
            "0x0"
        );
    }
    println!("LOCAL ONLY: contract-wallet SIWE, sponsored 65-byte claim, 66-byte submission and passing creator verdict; canonical solver payout and exact retries; both wallets retain zero ETH");

    // A separate open bounty accepts an exact 65-byte contract contribution.
    let mut doc = funding_terms.document.clone();
    doc.contract_terms["creation_nonce"] = format!("0x{:064x}", Uuid::new_v4().as_u128()).into();
    let wallet_funding_terms =
        build_autonomous_bounty_terms_record(&payer, doc, Utc::now()).unwrap();
    store
        .upsert_autonomous_bounty_terms(&wallet_funding_terms)
        .await
        .unwrap();
    let wallet_funding_create =
        chain_base::autonomous_bounty_create_from_terms(&wallet_funding_terms).unwrap();
    let wallet_funding_plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation("base-sepolia", &wallet_funding_create)
        .unwrap();
    let created=transaction(&url,json!({"from":payer,"to":factory,"data":wallet_funding_plan.create_bounty.data,"gas":"0x989680"})).await;
    let receipt: RpcTransactionReceipt = serde_json::from_value(created).unwrap();
    for event in decode_autonomous_bounty_logs(receipt.logs_to_evm_logs().unwrap()).unwrap() {
        store
            .upsert_autonomous_bounty_event("base-sepolia", &event)
            .await
            .unwrap();
    }
    transaction(&url,json!({"from":payer,"to":token,"data":word_call("mint(address,uint256)",&[format!("{:0>64}",contract_creator.trim_start_matches("0x")),format!("{:064x}",21_300_000)])})).await;
    let c = AutonomousBountyContribution {
        bounty_contract: wallet_funding_plan.predicted_bounty_contract,
        contributor: contract_creator.clone(),
        amount: Money::new(21_300_000, "usdc").unwrap(),
        authorization_nonce: Some(format!("0x{:064x}", Uuid::new_v4().as_u128())),
        authorization_valid_before: Some(Utc::now().timestamp() as u64 + 600),
    };
    let typed = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_contribution("base-sepolia", &c)
        .unwrap()
        .eip3009_authorization
        .unwrap();
    let sig = creator
        .sign_hash_sync(&chain_base::creation_authorization_digest(&typed).unwrap())
        .unwrap();
    let body = json!({"network":"base-sepolia","contribution":c,"signature":format!("0x{}",hex::encode(sig.as_bytes()))});
    let path = "/v1/base/autonomous-bounties/contribution-relay";
    let (status, result) = post_confirmed(app.clone(), path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["funding_confirmed"], true);
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(post_to(app.clone(), path, body).await.0, StatusCode::OK);
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );

    // Revoke one unused contract creation with a wider signature, then prove the
    // original 65-byte request inert only after canonical finality.
    let mut unused_contract = contract_create.clone();
    unused_contract.creation_nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let deadline = Utc::now().timestamp() as u64 + 3600;
    let unused_contract_plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation_with_expiry("base-sepolia", &unused_contract, deadline)
        .unwrap();
    let sig = creator
        .sign_hash_sync(
            &chain_base::creation_authorization_digest(
                unused_contract_plan.eip3009_authorization.as_ref().unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let saved_contract = json!({"create":unused_contract,"authorization_valid_before":deadline,"signature":format!("0x{}",hex::encode(sig.as_bytes())),"bounty_id":unused_contract_plan.bounty_id,"bounty_contract":unused_contract_plan.predicted_bounty_contract});
    assert_eq!(
        crate::creation_recovery::prove_inert(&saved_contract, &url, "base-sepolia")
            .await
            .unwrap_err(),
        StatusCode::CONFLICT
    );
    let sig = creator
        .sign_hash_sync(
            &crate::sponsored_revocation::digest(
                "base-sepolia",
                &contract_creator,
                &unused_contract.creation_nonce,
            )
            .unwrap(),
        )
        .unwrap();
    let body = json!({"network":"base-sepolia","authorizer":contract_creator,"nonce":unused_contract.creation_nonce,"signature":format!("0x7f{}",hex::encode(sig.as_bytes()))});
    let path = "/v1/base/usdc/authorization-revocation-relay";
    let before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let mut bad = body.clone();
    bad["nonce"] = format!("0x{}", "c1".repeat(32)).into();
    assert_eq!(
        post_to(app.clone(), path, bad).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        before
    );
    let (status, result) = post_confirmed(app.clone(), path, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["authorization_cancelled"], true);
    let nonce = rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await;
    assert_eq!(post_to(app.clone(), path, body).await.0, StatusCode::OK);
    assert_eq!(
        rpc(&url, "eth_getTransactionCount", json!([payer, "latest"])).await,
        nonce
    );
    rpc(&url, "anvil_mine", json!(["0x41", "0x0"])).await;
    let proof = crate::creation_recovery::prove_inert(&saved_contract, &url, "base-sepolia")
        .await
        .unwrap();
    assert_eq!(
        proof["reason"],
        "nonce_disabled_without_deployment_or_funds"
    );
    assert_eq!(
        rpc(&url, "eth_getBalance", json!([contract_creator, "latest"])).await,
        "0x0"
    );
    println!("LOCAL ONLY: contract contribution and 66-byte Circle cancellation each sponsored once; finalized proof permits unused contract creation retirement without customer ETH");
    let mut unused_create = create.clone();
    unused_create.creation_nonce = format!("0x{:064x}", Uuid::new_v4().as_u128());
    let expiry = Utc::now().timestamp() as u64 + 3600;
    let unused_plan = configured_autonomous_planner("base-sepolia")
        .unwrap()
        .plan_creation_with_expiry("base-sepolia", &unused_create, expiry)
        .unwrap();
    let signed = creator
        .sign_hash_sync(
            &TransferWithAuthorization {
                from: creator.address(),
                to: unused_plan.predicted_bounty_contract.parse().unwrap(),
                value: U256::from(21_300_000),
                validAfter: U256::ZERO,
                validBefore: U256::from(expiry),
                nonce: unused_create.creation_nonce.parse().unwrap(),
            }
            .eip712_signing_hash(&domain),
        )
        .unwrap();
    let saved = serde_json::json!({"create":unused_create,"authorization_valid_before":expiry,"signature":format!("0x{}",hex::encode(signed.as_bytes())),"bounty_id":unused_plan.bounty_id,"bounty_contract":unused_plan.predicted_bounty_contract});
    rpc(&url, "anvil_mine", serde_json::json!(["0x41"])).await;
    assert_eq!(
        crate::creation_recovery::prove_inert(&saved, &url, "base-mainnet")
            .await
            .unwrap_err(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        crate::creation_recovery::prove_inert(&saved, &url, "base-sepolia")
            .await
            .unwrap_err(),
        StatusCode::CONFLICT
    );
    rpc(
        &url,
        "evm_setNextBlockTimestamp",
        serde_json::json!([expiry + 1]),
    )
    .await;
    rpc(&url, "anvil_mine", serde_json::json!(["0x41"])).await;
    let proof = crate::creation_recovery::prove_inert(&saved, &url, "base-sepolia")
        .await
        .unwrap();
    assert_eq!(proof["reason"], "expired_unused_authorization");
    let mint = word_call(
        "mint(address,uint256)",
        &[
            format!(
                "{:0>64}",
                unused_plan
                    .predicted_bounty_contract
                    .trim_start_matches("0x")
            ),
            format!("{:064x}", 1),
        ],
    );
    transaction(
        &url,
        serde_json::json!({"from":payer,"to":token,"data":mint}),
    )
    .await;
    rpc(&url, "anvil_mine", serde_json::json!(["0x41"])).await;
    assert_eq!(
        crate::creation_recovery::prove_inert(&saved, &url, "base-sepolia")
            .await
            .unwrap_err(),
        StatusCode::CONFLICT
    );
    println!("LOCAL ONLY: recovery rejected a live authorization and wrong chain, proved expiry at finality, and preserved counterfactual USDC funds");
    proxy_task.abort();
}
