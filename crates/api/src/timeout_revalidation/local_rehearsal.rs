//! Opt-in local fork rehearsal. Never sends to an external chain.
//! Both fixture origins map to ONE Anvil node; this is not independent-provider evidence.
use super::*;
use alloy::{
    eips::Encodable2718,
    network::{EthereumWallet, TransactionBuilder},
    primitives::{keccak256, Address, Bytes, U256},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use chain_base::{JsonRpcTransport, ReqwestJsonRpcTransport};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

const BOUNTY: &str = "0xbf99f1c2c690c8bcd45e341006bf7b7dd6c41cdc";
const SOLVER: &str = "0x9f5e0b1df55617456027364cdc21cf86a357ee10";
const USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

struct LocalFork {
    url: String,
    transport: ReqwestJsonRpcTransport,
    checks: AtomicUsize,
    broadcasts: AtomicUsize,
}
impl LocalFork {
    fn new() -> Self {
        let url =
            std::env::var("TIMEOUT_LOCAL_ANVIL_URL").expect("explicit local Anvil URL required");
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.username().is_empty() && parsed.password().is_none());
        Self {
            url,
            transport: ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(20)).unwrap(),
            checks: AtomicUsize::new(0),
            broadcasts: AtomicUsize::new(0),
        }
    }
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let value = self
            .transport
            .post_json_value(
                &self.url,
                &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            )
            .await
            .unwrap();
        assert!(value.get("error").is_none(), "local RPC {method}: {value}");
        value["result"].clone()
    }
    async fn call(&self, to: &str, sig: &str, args: &str) -> Value {
        let data = format!("0x{}{}", hex::encode(&keccak256(sig.as_bytes())[..4]), args);
        self.rpc("eth_call", json!([{"to":to,"data":data},"latest"]))
            .await
    }
    async fn state(&self) -> Value {
        let mut state = serde_json::Map::new();
        for sig in [
            "status()",
            "round()",
            "solver()",
            "activeClaimBond()",
            "timeoutBondPool()",
            "fundedAmount()",
        ] {
            state.insert(sig.into(), self.call(BOUNTY, sig, "").await);
        }
        for (key, address) in [("contractUSDC", BOUNTY), ("solverUSDC", SOLVER)] {
            state.insert(
                key.into(),
                self.call(
                    USDC,
                    "balanceOf(address)",
                    &format!("{:0>64}", &address[2..]),
                )
                .await,
            );
        }
        Value::Object(state)
    }
}
#[axum::async_trait]
impl JsonRpcTransport for LocalFork {
    async fn post_json_value(
        &self,
        rpc_url: &str,
        request: &Value,
    ) -> Result<Value, ChainBaseError> {
        assert!(matches!(
            rpc_url,
            "https://first.fixture.invalid" | "https://second.fixture.invalid"
        ));
        assert!(matches!(
            request["method"].as_str().unwrap(),
            "eth_chainId"
                | "eth_getBlockByNumber"
                | "eth_getCode"
                | "eth_call"
                | "eth_estimateGas"
                | "eth_getBalance"
                | "eth_getTransactionCount"
                | "eth_getTransactionByHash"
                | "eth_getTransactionReceipt"
        ));
        self.transport.post_json_value(&self.url, request).await
    }
}
impl RecoveryIo for LocalFork {
    async fn check(
        &self,
        config: &Config,
        saved: &db::SavedTimeoutRelay,
        _seconds: u64,
    ) -> Result<TimeoutRevalidationObservation, ChainBaseError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        chain_base::timeout_recovery::revalidate_timeout_envelope_with_transport(
            [&config.primary_rpc, &config.shadow_rpc],
            &saved.scope,
            &saved.prepared,
            &saved.prepared.transaction.relayer,
            &config.policy,
            self,
        )
        .await
    }
    async fn quote(&self, chain_id: u64, _maximum_fee_wei: u128) -> Result<u64, ChainBaseError> {
        assert_eq!(chain_id, 8453);
        // Deterministic disposable budget fixture, not a real dollar quote.
        Ok(100)
    }
    async fn broadcast(
        &self,
        _config: &Config,
        saved: &db::SavedTimeoutRelay,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        assert!(saved.send_attempted && !saved.marker_commit_unknown);
        assert_eq!(self.broadcasts.fetch_add(1, Ordering::SeqCst), 0);
        let transport = ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(seconds))?;
        let actual = chain_base::broadcast_signed_transaction_with_transport(
            &self.url,
            &saved.prepared.signed_transaction,
            90,
            &transport,
        )
        .await
        .map_err(|error| {
            eprintln!("local dispatch error: {error:?}");
            error
        })?;
        assert_eq!(actual.result, saved.prepared.transaction.tx_hash);
        // Simulate the critical ambiguous response AFTER local acceptance.
        Err(ChainBaseError::RelayerProvider(
            "local fixture lost accepted response".into(),
        ))
    }
}
fn num(value: &Value) -> u64 {
    u64::from_str_radix(value.as_str().unwrap().strip_prefix("0x").unwrap(), 16).unwrap()
}

#[tokio::test]
#[ignore = "requires explicit local Anvil production-bytecode fork and disposable PostgreSQL"]
async fn timeout_revalidation_local_fork_exact_aged_envelope_and_accounting() {
    let io = LocalFork::new();
    assert_eq!(num(&io.rpc("eth_chainId", json!([])).await), 8453);
    let pinned = io
        .rpc("eth_getBlockByNumber", json!(["0x32c03ae", false]))
        .await;
    assert_eq!(
        pinned["hash"],
        "0x9d259419fcb06a5dff6670d2f706b697295146e60b6e49bea222e6f8ac4ae0d1"
    );
    // Refresh ONLY this synthetic fork's clock; never alters remote chain state.
    let now = Utc::now().timestamp();
    assert!(
        now > 1_793_218_835 && now < 1_793_253_540,
        "rehearse within the original published work window"
    );
    io.rpc("evm_setNextBlockTimestamp", json!([now - 64])).await;
    io.rpc("anvil_mine", json!(["0x40", "0x1"])).await;
    let before = io.state().await;
    assert_eq!(num(&before["status()"]), 2);
    assert_eq!(num(&before["round()"]), 3);
    assert_eq!(num(&before["activeClaimBond()"]), 1_000_000);
    assert_eq!(num(&before["timeoutBondPool()"]), 2_000_000);
    assert_eq!(num(&before["fundedAmount()"]), 21_300_000);
    assert_eq!(num(&before["contractUSDC"]), 24_300_000);

    let signer = PrivateKeySigner::random();
    let address = format!("{:#x}", signer.address());
    io.rpc("anvil_setBalance", json!([address, "0xde0b6b3a7640000"]))
        .await;
    let nonce = num(&io
        .rpc("eth_getTransactionCount", json!([address, "pending"]))
        .await);
    let scope = TimeoutRelayScope {
        network: "base-mainnet".into(),
        bounty_contract: BOUNTY.into(),
        action: "expire_claim".into(),
        round: 3,
        factory_contract: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9".into(),
        bounty_id: "0x73fa8fec134f131caa88fe3e0e16006485c098001344857e26640c51744ce234".into(),
        terms_hash: "0x82f436bb4025192f21a42bb4d6b818ace499e8690d24d6d1b11601ad26668bf3".into(),
    };
    let gas = 300_000;
    let fee = 10_000_000_000u128;
    let envelope = TransactionRequest::default()
        .with_to(BOUNTY.parse::<Address>().unwrap())
        .with_value(U256::ZERO)
        .with_input(scope.calldata().unwrap().parse::<Bytes>().unwrap())
        .with_nonce(nonce)
        .with_chain_id(8453)
        .with_gas_limit(gas)
        .with_max_fee_per_gas(fee)
        .with_max_priority_fee_per_gas(1_000_000)
        .build(&EthereumWallet::from(signer))
        .await
        .unwrap();
    let prepared = chain_base::PreparedBaseRelayTransaction {
        nonce,
        signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
        transaction: BaseRelayedTransaction {
            relayer: address.clone(),
            tx_hash: format!("{:#x}", envelope.tx_hash()),
            estimated_gas: gas,
            gas_limit: gas,
            max_fee_per_gas_wei: fee,
            max_priority_fee_per_gas_wei: 1_000_000,
            estimated_max_cost_wei: u128::from(gas) * fee,
        },
    };
    let mut saved = db::SavedTimeoutRelay {
        scope,
        prepared,
        send_attempted: false,
        marker_commit_unknown: false,
    };
    let config = Config {
        primary_rpc: "https://first.fixture.invalid".into(),
        shadow_rpc: "https://second.fixture.invalid".into(),
        policy: TimeoutRevalidationPolicy {
            factory_runtime_code_hash:
                "0x06f810de7b46f854ecc29e9c0c28156edab4b0d3e0bbe2bf5be8876687bebfc6".into(),
            implementation_contract: "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9".into(),
            implementation_runtime_code_hash:
                "0xc36fcba5176b2cd8b57a9fd0cbf931177dc8b36cf8367c1568ccebe5f03be3f6".into(),
            max_gas: gas,
            max_fee_per_gas_wei: fee,
        },
    };
    // Warm the fork's read-only remote storage cache outside the production lease.
    let initial = io.check(&config, &saved, 60).await.unwrap();
    let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&url);
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let schema = format!("timeout_fork_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let lease = store
        .acquire_x402_relayer_lease("base-mainnet", 45)
        .await
        .unwrap()
        .unwrap();
    store
        .save_timeout_relay(
            &saved.scope,
            lease,
            &saved.prepared,
            store.timeout_budget_day().await.unwrap(),
        )
        .await
        .unwrap();
    store
        .upsert_autonomous_bounty_event(
            "base-mainnet",
            &AutonomousBountyEvent {
                id: Uuid::new_v4(),
                log_key: Uuid::new_v4().to_string(),
                tx_hash: format!("0x{}", "bb".repeat(32)),
                block_number: 1,
                log_index: 0,
                contract_address: saved.scope.factory_contract.clone(),
                bounty_id: saved.scope.bounty_id.clone(),
                kind: AutonomousBountyEventKind::CanonicalBountyCreated,
                data: json!({"bounty_contract":BOUNTY,"terms_hash":saved.scope.terms_hash}),
                occurred_at: Utc::now(),
            },
        )
        .await
        .unwrap();
    let op = saved.scope.operation_id().unwrap();
    sqlx::query("UPDATE relay_transaction_outbox SET created_at=clock_timestamp()-interval '1 day' WHERE operation_id=$1").bind(op).execute(&pool).await.unwrap();
    let immutable_before:(DateTime<Utc>,String,Value)=sqlx::query_as("SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1").bind(op).fetch_one(&pool).await.unwrap();
    assert!(
        !store
            .claim_timeout_relay_send(&saved.scope, lease, 1)
            .await
            .unwrap(),
        "ordinary expired send remains blocked"
    );
    let state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "local-fixture",
        store.clone(),
    );
    let stale = saved.clone();
    let started = tokio::time::Instant::now();
    let authorization = first_send(
        &state,
        &config,
        lease,
        TimeoutLeaseBudget::new(started, 45, 15).unwrap(),
        &mut saved,
        &io,
    )
    .await
    .unwrap();
    let elapsed = started.elapsed();
    assert!(saved.send_attempted && !saved.marker_commit_unknown);
    assert_eq!(io.broadcasts.load(Ordering::SeqCst), 1);
    let mut receipt = Value::Null;
    for _ in 0..50 {
        receipt = io
            .rpc(
                "eth_getTransactionReceipt",
                json!([saved.prepared.transaction.tx_hash]),
            )
            .await;
        if !receipt.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(receipt["status"], "0x1");
    assert_eq!(receipt["from"], address);
    assert_eq!(receipt["to"], BOUNTY);
    let tx = io
        .rpc(
            "eth_getTransactionByHash",
            json!([saved.prepared.transaction.tx_hash]),
        )
        .await;
    assert_eq!(num(&tx["nonce"]), nonce);
    assert_eq!(tx["input"], "0x1257d2c8");
    assert_eq!(tx["value"], "0x0");
    assert_eq!(
        num(&io
            .rpc("eth_getTransactionCount", json!([address, "pending"]))
            .await),
        nonce + 1
    );
    let event_sig = format!(
        "{:#x}",
        keccak256(b"ClaimExpired(bytes32,uint64,address,uint256,uint256)")
    );
    let events: Vec<_> = receipt["logs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|log| log["address"] == BOUNTY && log["topics"][0] == event_sig)
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["topics"][1], saved.scope.bounty_id);
    assert_eq!(events[0]["topics"][2], format!("0x{:064x}", 3));
    assert_eq!(events[0]["topics"][3], format!("0x{:0>64}", &SOLVER[2..]));
    assert_eq!(
        events[0]["data"],
        format!("0x{:064x}{:064x}", 1_000_000, 3_000_000)
    );
    assert!(
        !receipt["logs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|log| log["address"] == USDC),
        "expiry must not transfer or refund USDC"
    );
    let after = io.state().await;
    assert_eq!(num(&after["status()"]), 1);
    assert_eq!(num(&after["round()"]), 3);
    assert_eq!(after["solver()"], format!("0x{:064x}", 0));
    assert_eq!(num(&after["activeClaimBond()"]), 0);
    assert_eq!(num(&after["timeoutBondPool()"]), 3_000_000);
    for field in ["fundedAmount()", "contractUSDC", "solverUSDC"] {
        assert_eq!(before[field], after[field]);
    }
    let checks = io.checks.load(Ordering::SeqCst);
    let mut retry = stale;
    assert!(first_send(
        &state,
        &config,
        lease,
        TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
        &mut retry,
        &io
    )
    .await
    .is_err());
    assert!(retry.send_attempted);
    assert_eq!(io.checks.load(Ordering::SeqCst), checks);
    assert_eq!(io.broadcasts.load(Ordering::SeqCst), 1);
    let immutable_after:(DateTime<Utc>,String,Value)=sqlx::query_as("SELECT created_at,signed_transaction,transaction_metadata FROM relay_transaction_outbox WHERE operation_id=$1").bind(op).fetch_one(&pool).await.unwrap();
    assert_eq!(immutable_before, immutable_after);
    let event:(String,i64)=sqlx::query_as("SELECT status,required_budget_micro_usdc FROM timeout_revalidation_events WHERE authorization_id=$1").bind(authorization).fetch_one(&pool).await.unwrap();
    assert_eq!(event, ("consumed".into(), 100));
    io.rpc("anvil_mine", json!(["0x40", "0x1"])).await;
    let safe = io.rpc("eth_getBlockByNumber", json!(["safe", false])).await;
    let canonical = io
        .rpc(
            "eth_getBlockByNumber",
            json!([receipt["blockNumber"], false]),
        )
        .await;
    assert_eq!(canonical["hash"], receipt["blockHash"]);
    assert!(num(&safe["number"]) >= num(&receipt["blockNumber"]));
    let reconciled = chain_base::timeout_recovery::observe_timeout_receipt(
        &io.url,
        &saved.scope,
        &saved.prepared.transaction.tx_hash,
        1,
        &io.transport,
    )
    .await
    .unwrap();
    assert!(matches!(
        reconciled,
        chain_base::timeout_recovery::TimeoutReceiptObservation::Confirmed { .. }
    ));
    let evidence = json!({"kind":"local-anvil-fork-not-live-testnet","fork_block":53216174,"fork_hash":pinned["hash"],"one_node_two_fixture_labels_not_independent_providers":true,"synthetic_relayer":address,"transaction_hash":saved.prepared.transaction.tx_hash,"nonce":nonce,"receipt":receipt,"before":before,"after":after,"check_observation":initial,"safe_block":safe["number"],"original_journal_unchanged":true,"ordinary_stale_send_blocked":true,"retry_broadcasts":0,"total_broadcasts":1,"lost_response_reconciled":true,"authorization_status":event.0,"execution_milliseconds":elapsed.as_millis(),"budget_micro_usdc_is_fixture":100});
    let path = std::env::var("TIMEOUT_LOCAL_EVIDENCE_PATH").unwrap();
    std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    store
        .release_x402_relayer_lease("base-mainnet", lease)
        .await
        .unwrap();
}
