//! Opt-in loopback Anvil + disposable Postgres rehearsal. No hosted writes.
use super::*;
use alloy::{
    primitives::{B256, U256},
    signers::{local::PrivateKeySigner, SignerSync},
};
use axum::{body::to_bytes, http::Request};
use serde_json::Value;
use tower::ServiceExt;

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

async fn post_to(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    post_authenticated(app, path, body, None).await
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

fn signed(
    release: &service::Release,
    r: &service::Request,
    signers: &[&PrivateKeySigner],
) -> serde_json::Value {
    let p = service::prepare(release, r, &service::Signatures::default()).unwrap();
    let signature = |req: &Option<service::SignatureRequirement>| -> String {
        req.as_ref()
            .map(|r| {
                let signer = signers
                    .iter()
                    .find(|s| s.address() == r.signer)
                    .expect("fixture signer");
                format!(
                    "0x{}",
                    hex::encode(signer.sign_hash_sync(&r.digest).unwrap().as_bytes())
                )
            })
            .unwrap_or_else(|| "0x".into())
    };
    json!({"request":r,"approval_hash":p.approval_hash,"signatures":{"token":signature(&p.token_authorization),"action":signature(&p.action_authorization)}})
}
async fn send_checked(
    app: &axum::Router,
    release: &service::Release,
    r: &service::Request,
    signers: &[&PrivateKeySigner],
    event: &str,
) -> Value {
    rpc(
        &env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap(),
        "anvil_mine",
        json!(["0x40", "0x0"]),
    )
    .await;
    let body = signed(release, r, signers);
    let (mut code, mut out) = post_to(app.clone(), "/v1/base/creator-open/relay", body).await;
    for _ in 0..30 {
        if code != StatusCode::ACCEPTED {
            break;
        }
        sleep(Duration::from_millis(50)).await;
        (code, out) = post_to(
            app.clone(),
            "/v1/base/creator-open/status",
            json!({"network":release.network,"approval_hash":out["approval_hash"]}),
        )
        .await;
    }
    assert_eq!(code, StatusCode::OK, "{out}");
    assert_eq!(out["confirmed"], true, "{out}");
    assert_eq!(out["evidence"]["event"], event, "{out}");
    assert_eq!(out["solver_paid"], event == "OpenBountySettled");
    out
}
async fn balance(url: &str, token: &str, address: alloy::primitives::Address) -> U256 {
    let value=rpc(url,"eth_call",json!([{"to":token,"data":word_call("balanceOf(address)",&[format!("{:0>64}",hex::encode(address))])},"latest"])).await;
    U256::from_str_radix(value.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}
#[tokio::test]
#[ignore = "run scripts/check-creator-open.sh with disposable loopback Anvil and PostgreSQL"]
async fn sponsored_creator_open_lifecycle_and_recovery() {
    use service::{Action, Request as OpenRequest};
    let url = env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
    let endpoint = url::Url::parse(&url).unwrap();
    assert_eq!(endpoint.host_str(), Some("127.0.0.1"));
    assert_eq!(endpoint.scheme(), "http");
    assert_eq!(rpc(&url, "eth_chainId", json!([])).await, "0x14a34");
    assert!(rpc(&url, "web3_clientVersion", json!([]))
        .await
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("anvil"));
    let db = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    let parsed = url::Url::parse(&db).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/posting_"));
    let store = PostgresStore::connect(&db).await.unwrap();
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    let transaction_relayer =
        BaseTransactionRelayer::from_private_key(&format!("{:064x}", Uuid::new_v4().as_u128()))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    let payer = transaction_relayer.address();
    rpc(
        &url,
        "anvil_setBalance",
        json!([payer, "0x8ac7230489e80000"]),
    )
    .await;
    rpc(&url, "anvil_impersonateAccount", json!([payer])).await;
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
    let deployed=transaction(&url,json!({"from":payer,"data":format!("{}{:0>64}",artifact("CreatorReviewBountyFactoryV1","bytecode"),token.trim_start_matches("0x")),"gas":"0x989680"})).await;
    let factory = deployed["contractAddress"].as_str().unwrap();
    let implementation = rpc(
        &url,
        "eth_call",
        json!([{"to":factory,"data":word_call("implementation()",&[])},"latest"]),
    )
    .await;
    let implementation = format!("0x{}", &implementation.as_str().unwrap()[26..]);
    let factory_code = rpc(&url, "eth_getCode", json!([factory, "latest"])).await;
    let implementation_code = rpc(&url, "eth_getCode", json!([implementation, "latest"])).await;
    let release = service::Release {
        protocol: chain_base::creator_open::PROTOCOL.into(),
        network: "base-sepolia".into(),
        factory: factory.parse().unwrap(),
        implementation: implementation.parse().unwrap(),
        factory_code_hash: alloy::primitives::keccak256(
            hex::decode(factory_code.as_str().unwrap().trim_start_matches("0x")).unwrap(),
        ),
        implementation_code_hash: alloy::primitives::keccak256(
            hex::decode(
                implementation_code
                    .as_str()
                    .unwrap()
                    .trim_start_matches("0x"),
            )
            .unwrap(),
        ),
        deployment_block: u64::from_str_radix(
            deployed["blockNumber"]
                .as_str()
                .unwrap()
                .trim_start_matches("0x"),
            16,
        )
        .unwrap(),
    };
    env::set_var(
        "CREATOR_OPEN_SEPOLIA_RELEASE_JSON",
        serde_json::to_string(&release).unwrap(),
    );
    env::set_var("ENABLE_CREATOR_OPEN_SPONSORSHIP", "true");
    let creator: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let solver: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    let second: PrivateKeySigner = format!("{:064x}", Uuid::new_v4().as_u128())
        .parse()
        .unwrap();
    for wallet in [&creator, &solver, &second] {
        transaction(&url,json!({"from":payer,"to":token,"data":word_call("mint(address,uint256)",&[format!("{:0>64}",hex::encode(wallet.address())),format!("{:064x}",100_000_000)])})).await;
    }
    // Exercise ambiguous send replies and failures before the upstream receives
    // a transaction, while all canonical reads still come from real Anvil.
    let send_mode = Arc::new(std::sync::atomic::AtomicUsize::new(2));
    let sent_bytes = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let log_ranges = Arc::new(std::sync::Mutex::new(Vec::<(u64, u64)>::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let proxy=axum::Router::new().route("/",axum::routing::post({
        let upstream=url.clone(); let mode=send_mode.clone();let seen=sent_bytes.clone();let ranges=log_ranges.clone();
        move |Json(body):Json<Value>| {
            let upstream=upstream.clone();let mode=mode.clone();let seen=seen.clone();let ranges=ranges.clone();
            async move {
                if body["method"]=="eth_getLogs" {
                    let block=|key:&str| u64::from_str_radix(body["params"][0][key].as_str().unwrap().trim_start_matches("0x"),16).unwrap();
                    let (from,to)=(block("fromBlock"),block("toBlock"));
                    ranges.lock().unwrap().push((from,to));
                    if to.saturating_sub(from)>=1000 {
                        return Json(json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32614,"message":"eth_getLogs is limited to a 1,000 range"}}));
                    }
                }
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
    mutable.base_rpc_urls.base_sepolia = Some(proxy_url.clone());
    mutable.x402_relayer = X402HostedRelayerConfig {
        max_daily_attempts: 100_000, // This disposable DB is reused across independent local runs.
        enabled: true,
        relayer: Some(Arc::new(transaction_relayer)),
        confirmations: 1,
        wait_seconds: 2,
        rpc_timeout_seconds: 30,
        ..Default::default()
    };
    let app = axum::Router::new()
        .route("/v1/base/creator-open/bounties", axum::routing::get(feed))
        .route("/v1/base/creator-open/import", axum::routing::post(import))
        .route(
            "/v1/base/creator-open/prepare",
            axum::routing::post(prepare),
        )
        .route("/v1/base/creator-open/relay", axum::routing::post(relay))
        .route("/v1/base/creator-open/status", axum::routing::post(status))
        .with_state(state.clone());
    let now = Utc::now().timestamp() as u64;
    let terms = chain_base::creator_open::Terms {
        schema: chain_base::creator_open::PROTOCOL.into(),
        network: "base-sepolia".into(),
        bounty_id: format!("0x{:064x}", Uuid::new_v4().as_u128()),
        creator: format!("{:#x}", creator.address()),
        title: "Zero ETH full creator lifecycle".into(),
        goal: "Exercise every customer action through the exact signed API".into(),
        acceptance_criteria: vec!["One canonical payout; all remaining bonds returned".into()],
        solver_reward_micro_usdc: 20_000_000,
        review_reward_micro_usdc: 1_000_000,
        submission_deadline: now + 90 * 86400,
        review_window_seconds: 86400,
        max_pending_entries: 4,
        disclosure: chain_base::creator_open::DISCLOSURE.into(),
    };
    let mut request = OpenRequest {
        public_evidence: None,
        original_terms: terms,
        user_salt: B256::repeat_byte(1),
        expected_revision: 1,
        expected_solver_reward_micro_usdc: 20_000_000,
        valid_before: now + 1800,
        action: Action::Create {
            token_nonce: B256::repeat_byte(1),
        },
    };
    rpc(&url, "anvil_mine", json!(["0x40", "0x0"])).await;
    let prepared_check =
        service::prepare(&release, &request, &service::Signatures::default()).unwrap();
    let pinned_block = live_block(&url).await.unwrap();
    service::verify_fresh_state(
        &chain_base::ReqwestJsonRpcTransport::default(),
        &url,
        &release,
        &prepared_check,
        &pinned_block,
        Utc::now().timestamp() as u64,
    )
    .await
    .expect("initial release and state verification");
    let (code, prepared) =
        post_to(app.clone(), "/v1/base/creator-open/prepare", json!(request)).await;
    assert_eq!(code, StatusCode::OK, "{prepared}");
    assert_eq!(prepared["wallet_balance_micro_usdc"], "100000000");
    assert_eq!(prepared["prepared"]["customer_gas_wei"], "0");
    let initial_budget = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    let mut bad = signed(&release, &request, &[&creator]);
    bad["signatures"]["token"] = format!("0x{}", "11".repeat(65)).into();
    assert_eq!(
        post_to(app.clone(), "/v1/base/creator-open/relay", bad)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        initial_budget
    );
    let creation = request.clone();
    let created = send_checked(
        &app,
        &release,
        &creation,
        &[&creator],
        "CreatorOpenBountyCreated",
    )
    .await;
    assert_eq!(
        sent_bytes.lock().unwrap().len(),
        1,
        "lost send reply must preserve the single mined transaction"
    );
    let spent = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    assert!(spent > initial_budget);
    let repeated = send_checked(
        &app,
        &release,
        &creation,
        &[&creator],
        "CreatorOpenBountyCreated",
    )
    .await;
    assert_eq!(
        created["operation"]["transaction_hash"],
        repeated["operation"]["transaction_hash"]
    );
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        spent
    );
    let persisted = store
        .creator_open_request(
            "base-sepolia",
            &key(
                service::prepare(&release, &creation, &service::Signatures::default())
                    .unwrap()
                    .approval_hash,
            ),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(!serde_json::to_string(&persisted)
        .unwrap()
        .contains("signature"));
    request.action = Action::Increase {
        new_reward_micro_usdc: 25_000_000,
        token_nonce: B256::repeat_byte(2),
    };
    send_mode.store(1, std::sync::atomic::Ordering::SeqCst);
    let (pending_code, pending) = post_to(
        app.clone(),
        "/v1/base/creator-open/relay",
        signed(&release, &request, &[&creator]),
    )
    .await;
    assert_eq!(pending_code, StatusCode::ACCEPTED, "{pending}");
    let pending_id: Uuid = pending["operation"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let saved = store
        .relay_outbox(pending_id, "base-sepolia")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pending["operation"]["transaction_hash"],
        saved.transaction.tx_hash
    );
    assert!(store
        .gas_sponsor_budget()
        .ensure_sender_nonce_available(84532, &payer, saved.nonce)
        .await
        .is_err());
    let count = sent_bytes.lock().unwrap().len();
    let (code, read) = post_to(
        app.clone(),
        "/v1/base/creator-open/status",
        json!({"network":"base-sepolia","approval_hash":pending["approval_hash"]}),
    )
    .await;
    assert_eq!(code, StatusCode::ACCEPTED);
    assert_eq!(read["confirmed"], false);
    assert_eq!(
        sent_bytes.lock().unwrap().len(),
        count,
        "status reads must not resend"
    );
    sleep(Duration::from_secs(11)).await;
    let recovered =
        send_checked(&app, &release, &request, &[&creator], "OpenRewardIncreased").await;
    assert_eq!(
        recovered["operation"]["transaction_hash"],
        pending["operation"]["transaction_hash"]
    );
    {
        let sent = sent_bytes.lock().unwrap();
        assert_eq!(
            sent[count - 1],
            sent[count],
            "retry must submit byte-identical transaction"
        );
    }
    request.expected_revision = 2;
    request.expected_solver_reward_micro_usdc = 25_000_000;
    let evidence_document = service::CreatorOpenPublicEvidence {
        artifact_url: "https://example.org/result".into(),
        artifact_sha256: format!("sha256:{}", "ab".repeat(32)),
        notes: "Deterministic local evidence fixture.".into(),
    };
    let evidence_commitment = evidence_document.commitment().unwrap();
    request.public_evidence = Some(evidence_document.clone());
    request.action = Action::Submit {
        solver: solver.address(),
        submission_hash: B256::repeat_byte(3),
        evidence_hash: evidence_commitment,
        token_nonce: B256::repeat_byte(5),
    };
    send_checked(&app, &release, &request, &[&solver], "OpenSubmissionAdded").await;
    let published = store
        .creator_open_public_evidence(
            "base-sepolia",
            &prepared_check.bounty_contract.to_string(),
            &[format!("{evidence_commitment:#x}")],
        )
        .await
        .unwrap();
    assert_eq!(
        published[&format!("{evidence_commitment:#x}")].artifact_url,
        evidence_document.artifact_url
    );
    request.public_evidence = None;
    let late_increase = OpenRequest {
        action: Action::Increase {
            new_reward_micro_usdc: 26_000_000,
            token_nonce: B256::repeat_byte(9),
        },
        ..request.clone()
    };
    assert_eq!(
        post_to(
            app.clone(),
            "/v1/base/creator-open/relay",
            signed(&release, &late_increase, &[&creator])
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    request.action = Action::Review {
        entry: 1,
        solver: solver.address(),
        submission_hash: B256::repeat_byte(3),
        evidence_hash: evidence_commitment,
        passed: false,
        response_hash: B256::repeat_byte(6),
    };
    send_checked(
        &app,
        &release,
        &request,
        &[&creator],
        "OpenSubmissionRejected",
    )
    .await;
    request.action = Action::Submit {
        solver: solver.address(),
        submission_hash: B256::repeat_byte(7),
        evidence_hash: B256::repeat_byte(8),
        token_nonce: B256::repeat_byte(9),
    };
    send_checked(&app, &release, &request, &[&solver], "OpenSubmissionAdded").await;
    request.action = Action::Submit {
        solver: second.address(),
        submission_hash: B256::repeat_byte(10),
        evidence_hash: B256::repeat_byte(11),
        token_nonce: B256::repeat_byte(12),
    };
    send_checked(&app, &release, &request, &[&second], "OpenSubmissionAdded").await;
    request.action = Action::Review {
        entry: 2,
        solver: solver.address(),
        submission_hash: B256::repeat_byte(7),
        evidence_hash: B256::repeat_byte(8),
        passed: true,
        response_hash: B256::repeat_byte(13),
    };
    let winner = send_checked(&app, &release, &request, &[&creator], "OpenBountySettled").await;
    let receipt = fetch_transaction_receipt(
        &url,
        winner["operation"]["transaction_hash"].as_str().unwrap(),
        1,
    )
    .await
    .unwrap()
    .result
    .unwrap();
    let block = fetch_exact_block_identity(&url, receipt.block_number().unwrap().unwrap(), 2)
        .await
        .unwrap();
    let prepared = service::prepare(&release, &request, &service::Signatures::default()).unwrap();
    assert_eq!(
        service::receipt_outcome(&release, &prepared, &receipt, &block)
            .unwrap()
            .unwrap()["solver_paid"],
        true
    );
    let mut forged = receipt.clone();
    forged.block_hash = Some(format!("{:#x}", B256::repeat_byte(88)));
    assert!(service::receipt_outcome(&release, &prepared, &forged, &block).is_err());
    let mut missing = receipt.clone();
    missing.logs.clear();
    assert!(
        service::receipt_outcome(&release, &prepared, &missing, &block)
            .unwrap()
            .is_none()
    );
    env::set_var("ENABLE_CREATOR_OPEN_SPONSORSHIP", "false");
    let (code, recovered) = post_to(
        app.clone(),
        "/v1/base/creator-open/status",
        json!({"network":"base-sepolia","approval_hash":winner["approval_hash"]}),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{recovered}");
    assert_eq!(recovered["solver_paid"], true);
    env::set_var("ENABLE_CREATOR_OPEN_SPONSORSHIP", "true");
    request.action = Action::RefundBond { entry: 3 };
    send_checked(&app, &release, &request, &[], "OpenBondRefunded").await;
    assert_eq!(
        balance(&url, token, solver.address()).await,
        U256::from(124_000_000)
    );
    assert_eq!(
        balance(&url, token, second.address()).await,
        U256::from(100_000_000)
    );
    assert_eq!(
        balance(&url, token, creator.address()).await,
        U256::from(76_000_000)
    );
    let mut cancelled = creation.clone();
    cancelled.user_salt = B256::repeat_byte(14);
    cancelled.original_terms.bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
    cancelled.action = Action::Create {
        token_nonce: B256::repeat_byte(15),
    };
    send_checked(
        &app,
        &release,
        &cancelled,
        &[&creator],
        "CreatorOpenBountyCreated",
    )
    .await;
    cancelled.action = Action::Cancel;
    send_checked(
        &app,
        &release,
        &cancelled,
        &[&creator],
        "OpenBountyCancelled",
    )
    .await;
    cancelled.action = Action::RefundPrincipal;
    send_checked(&app, &release, &cancelled, &[], "OpenPrincipalRefunded").await;
    assert_eq!(
        balance(&url, token, creator.address()).await,
        U256::from(76_000_000)
    );
    rpc(&url, "anvil_mine", json!(["0x40", "0x0"])).await;
    assert!(
        worker::poll_creator_open_indexer(&store, &release, &url)
            .await
            .unwrap()
            >= 2
    );
    let contract = service::prepare(&release, &creation, &service::Signatures::default())
        .unwrap()
        .bounty_contract;
    let indexed = store
        .creator_open_index_batch("base-sepolia", Some(&format!("{contract:#x}")))
        .await
        .unwrap()
        .pop()
        .unwrap();
    let mut incomplete = indexed.projection.as_ref().unwrap().clone();
    incomplete.entries.remove(&1);
    assert!(service::verify_projection(
        &chain_base::ReqwestJsonRpcTransport::default(),
        &url,
        &incomplete,
        indexed.checkpoint.as_ref().unwrap()
    )
    .await
    .is_err());
    assert!(indexed.available);
    assert_eq!(indexed.projection.as_ref().unwrap().status, "settled");
    assert_eq!(indexed.projection.as_ref().unwrap().outstanding_bonds, 0);
    let response=app.clone().oneshot(Request::builder().uri(format!("/v1/base/creator-open/bounties?network=base-sepolia&bounty_contract={contract:#x}")).body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_eq!(body["items"][0]["solver_paid"], true);
    assert_eq!(
        body["items"][0]["terms"]["solver_reward_micro_usdc"],
        25_000_000
    );
    // A sleeping or restarted worker must catch up through the real provider's
    // 1,000-block limit. Incomplete pages remain unavailable until the safe head.
    let reserved_before_catchup = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    rpc(&url, "anvil_mine", json!(["0xbb8", "0x0"])).await;
    let safe_after_sleep = chain_base::fetch_safe_block_identity(&url, 20)
        .await
        .unwrap();
    let mut previous = indexed.checkpoint.as_ref().unwrap().number;
    assert!(safe_after_sleep.number > previous + 2000);
    for page in 0..3 {
        assert!(
            worker::poll_creator_open_indexer(&store, &release, &proxy_url)
                .await
                .unwrap()
                >= 2
        );
        let refreshed = store
            .creator_open_index_batch("base-sepolia", Some(&format!("{contract:#x}")))
            .await
            .unwrap()
            .pop()
            .unwrap();
        let checkpoint = refreshed.checkpoint.as_ref().unwrap().number;
        assert!(checkpoint > previous && checkpoint - previous <= 1000);
        previous = checkpoint;
        let response = app.clone().oneshot(Request::builder().uri(format!("/v1/base/creator-open/bounties?network=base-sepolia&bounty_contract={contract:#x}")).body(axum::body::Body::empty()).unwrap()).await.unwrap();
        if page < 2 {
            assert!(!refreshed.available);
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        } else {
            assert_eq!(checkpoint, safe_after_sleep.number);
            assert!(refreshed.available);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                refreshed
                    .projection
                    .as_ref()
                    .unwrap()
                    .settlement_transaction,
                indexed.projection.as_ref().unwrap().settlement_transaction
            );
        }
    }
    assert!(log_ranges
        .lock()
        .unwrap()
        .iter()
        .all(|(from, to)| to - from < 1000));
    assert_eq!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc,
        reserved_before_catchup
    );
    let mut external = creation.clone();
    external.user_salt = B256::repeat_byte(20);
    external.original_terms.bounty_id = format!("0x{:064x}", Uuid::new_v4().as_u128());
    external.action = Action::Create {
        token_nonce: B256::repeat_byte(21),
    };
    let external_body = signed(&release, &external, &[&creator]);
    let external_signatures: service::Signatures =
        serde_json::from_value(external_body["signatures"].clone()).unwrap();
    let external_plan = service::prepare(&release, &external, &external_signatures).unwrap();
    let external_receipt = transaction(&url, json!({"from":payer,"to":external_plan.transaction.to,"data":external_plan.transaction.data})).await;
    rpc(&url, "anvil_mine", json!(["0x40", "0x0"])).await;
    let import_body = json!({"original_terms":external.original_terms,"user_salt":external.user_salt,"creation_transaction_hash":external_receipt["transactionHash"]});
    let budget_before = store
        .gas_sponsor_budget_status()
        .await
        .unwrap()
        .reserved_micro_usdc;
    env::set_var("ENABLE_CREATOR_OPEN_SPONSORSHIP", "false");
    for field in ["creation_transaction_hash", "user_salt"] {
        let mut bad = import_body.clone();
        bad[field] = json!(format!("{:#x}", B256::repeat_byte(99)));
        assert_eq!(
            post_to(app.clone(), "/v1/base/creator-open/import", bad)
                .await
                .0,
            StatusCode::CONFLICT
        );
    }
    let mut bad_terms = import_body.clone();
    bad_terms["original_terms"]["solver_reward_micro_usdc"] = json!(1_234_567);
    assert_eq!(
        post_to(app.clone(), "/v1/base/creator-open/import", bad_terms)
            .await
            .0,
        StatusCode::CONFLICT
    );
    for _ in 0..2 {
        let (code, imported) = post_to(
            app.clone(),
            "/v1/base/creator-open/import",
            import_body.clone(),
        )
        .await;
        assert_eq!(code, StatusCode::ACCEPTED, "{imported}");
        assert_eq!(
            imported["bounty_contract"],
            json!(external_plan.bounty_contract)
        );
        assert_eq!(imported["index_status"], "pending_canonical_scan");
    }
    assert_eq!(
        budget_before,
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc
    );
    assert!(store
        .find_sponsored_action("base-sepolia", &key(external_plan.approval_hash))
        .await
        .unwrap()
        .is_none());
    env::set_var("ENABLE_CREATOR_OPEN_SPONSORSHIP", "true");
    let snapshot = rpc(&url, "evm_snapshot", json!([])).await;
    external.action = Action::Cancel;
    let body = signed(&release, &external, &[&creator]);
    let signatures: service::Signatures =
        serde_json::from_value(body["signatures"].clone()).unwrap();
    let plan = service::prepare(&release, &external, &signatures).unwrap();
    transaction(
        &url,
        json!({"from":payer,"to":plan.transaction.to,"data":plan.transaction.data}),
    )
    .await;
    rpc(&url, "anvil_mine", json!(["0x40", "0x0"])).await;
    worker::poll_creator_open_indexer(&store, &release, &url)
        .await
        .unwrap();
    let address = format!("{:#x}", plan.bounty_contract);
    let old = store
        .creator_open_index_batch("base-sepolia", Some(&address))
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(old.projection.as_ref().unwrap().status, "cancelled");
    assert_eq!(rpc(&url, "evm_revert", json!([snapshot])).await, true);
    worker::poll_creator_open_indexer(&store, &release, &url)
        .await
        .unwrap();
    let invalidated = store
        .creator_open_index_batch("base-sepolia", Some(&address))
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(!invalidated.available);
    assert!(
        !store
            .save_creator_open_index(
                &old,
                old.checkpoint.as_ref().unwrap(),
                &old.logs,
                old.projection.as_ref().unwrap(),
                true
            )
            .await
            .unwrap(),
        "old worker cannot restore reorged evidence"
    );
    worker::poll_creator_open_indexer(&store, &release, &url)
        .await
        .unwrap();
    let rebuilt = store
        .creator_open_index_batch("base-sepolia", Some(&address))
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert!(rebuilt.available);
    assert_eq!(rebuilt.projection.as_ref().unwrap().status, "open");
    // The real-time API clock must not follow an arbitrary RPC timestamp. Test
    // permissionless expiry with an explicit simulated time at the chain adapter,
    // then use the same durable budget/broadcaster and API receipt reconciliation.
    let expired_at = external.original_terms.submission_deadline
        + external.original_terms.review_window_seconds
        + 1;
    rpc(&url, "evm_setNextBlockTimestamp", json!([expired_at])).await;
    rpc(&url, "evm_mine", json!([])).await;
    external.valid_before = expired_at + 1800;
    for action in [Action::Expire, Action::RefundPrincipal] {
        external.action = action;
        let plan = service::prepare(&release, &external, &service::Signatures::default()).unwrap();
        let block = live_block(&url).await.unwrap();
        let transport = chain_base::ReqwestJsonRpcTransport::default();
        service::verify_fresh_state(&transport, &url, &release, &plan, &block, block.timestamp)
            .await
            .unwrap();
        service::simulate(&transport, &url, &plan, &payer, &block)
            .await
            .unwrap();
        let request_key = key(plan.approval_hash);
        let op = store
            .reserve_sponsored_action(
                "base-sepolia",
                &request_key,
                &format!("{:#x}", creator.address()),
                10000,
                10000,
            )
            .await
            .unwrap();
        store
            .save_creator_open_request(
                "base-sepolia",
                &request_key,
                &format!("{:#x}", plan.bounty_contract),
                op.id,
                &external,
            )
            .await
            .unwrap();
        let op = crate::sponsored_verdict::broadcast(
            &state,
            "base-sepolia",
            op,
            &plan.transaction,
            1_500_000,
        )
        .await
        .unwrap();
        assert!(op.transaction_hash.is_some(), "{op:?}");
        let mut confirmed = false;
        for _ in 0..30 {
            let (code, result) = post_to(
                app.clone(),
                "/v1/base/creator-open/status",
                json!({"network":"base-sepolia","approval_hash":plan.approval_hash}),
            )
            .await;
            if code == StatusCode::OK {
                assert_eq!(result["solver_paid"], false);
                confirmed = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        assert!(
            confirmed,
            "expiry/refund must reconcile through the real API"
        );
    }
    for wallet in [&creator, &solver, &second] {
        assert_eq!(
            rpc(&url, "eth_getBalance", json!([wallet.address(), "latest"])).await,
            "0x0"
        );
    }
    assert!(
        store
            .gas_sponsor_budget_status()
            .await
            .unwrap()
            .reserved_micro_usdc
            < 1_000_000
    );
    proxy_task.abort();
    println!("LOCAL ONLY: native signatures + durable API + PostgreSQL + actual candidate bytecode passed creation/replay, increase, competing submissions, rejection, one payout, bond refund, cancellation/principal refund, disabled-flag recovery, and zero ETH for all customers");
}
