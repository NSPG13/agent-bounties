//! Opt-in disposable Base-ID EVM with explicitly seeded fixture contracts.
//! Both named origins map to ONE local Anvil;
//! this tests execution/custody boundaries, not independent-provider consensus.
use super::retained_dispatch::{initial_dispatch, DispatchIo};
use super::*;
use alloy::{
    eips::Encodable2718,
    network::{EthereumWallet, TransactionBuilder},
    primitives::{Address, Bytes, U256},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use chain_base::nonce_retirement::retained_dispatch::{
    check_retained_dispatch_with_transport, RetainedNonceDispatch,
};
use chain_base::{timeout_recovery::TimeoutRelayScope, JsonRpcTransport, ReqwestJsonRpcTransport};
use serde_json::{json, Value};
use sha2::Digest;
use std::sync::atomic::{AtomicUsize, Ordering};
const BOUNTY: &str = "0xbf99f1c2c690c8bcd45e341006bf7b7dd6c41cdc";
struct Fork {
    url: String,
    t: ReqwestJsonRpcTransport,
    sends: AtomicUsize,
}
impl Fork {
    fn new() -> Self {
        let url = env::var("RETIREMENT_LOCAL_ANVIL_URL").expect("explicit loopback Anvil required");
        let parsed = url::Url::parse(&url).unwrap();
        assert_eq!(parsed.scheme(), "http");
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert!(parsed.username().is_empty() && parsed.password().is_none());
        Self {
            url,
            t: ReqwestJsonRpcTransport::without_redirects(Duration::from_secs(10)).unwrap(),
            sends: AtomicUsize::new(0),
        }
    }
    async fn response(&self, method: &str, params: Value) -> Value {
        self.t
            .post_json_value(
                &self.url,
                &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
            )
            .await
            .unwrap()
    }
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let r = self.response(method, params).await;
        assert!(r.get("error").is_none(), "local {method}: {r}");
        r["result"].clone()
    }
    async fn bounty(&self) -> Value {
        let mut state = serde_json::Map::new();
        for sig in ["status()", "round()", "fundedAmount()", "timeoutBondPool()"] {
            let data = format!(
                "0x{}",
                hex::encode(&alloy::primitives::keccak256(sig.as_bytes())[..4])
            );
            state.insert(
                sig.into(),
                self.rpc("eth_call", json!([{"to":BOUNTY,"data":data},"latest"]))
                    .await,
            );
        }
        Value::Object(state)
    }
}
#[axum::async_trait]
impl JsonRpcTransport for Fork {
    async fn post_json_value(&self, url: &str, request: &Value) -> Result<Value, ChainBaseError> {
        assert!(matches!(
            url,
            "https://primary.fixture.invalid" | "https://shadow.fixture.invalid"
        ));
        let check = |v: &Value| {
            assert!(matches!(
                v["method"].as_str().unwrap(),
                "eth_chainId"
                    | "eth_getBlockByNumber"
                    | "eth_getTransactionCount"
                    | "eth_getTransactionByHash"
                    | "eth_getTransactionReceipt"
                    | "eth_getCode"
                    | "eth_getBalance"
                    | "eth_call"
                    | "eth_estimateGas"
                    | "eth_gasPrice"
            ))
        };
        if let Some(batch) = request.as_array() {
            for v in batch {
                check(v);
            }
        } else {
            check(request);
        }
        self.t.post_json_value(&self.url, request).await
    }
}
impl RetirementIo for Fork {
    async fn check(
        &self,
        c: &Config,
        p: &NonceRetirementPlan,
        s: &db::SavedTimeoutRelay,
        _: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        check_with_transport(
            [&c.primary, &c.shadow],
            p,
            &s.prepared,
            &s.scope,
            &c.policy,
            Utc::now(),
            self,
        )
        .await
    }
    async fn quote(&self, _: u128) -> Result<u64, ChainBaseError> {
        Ok(100)
    } // disposable accounting fixture
    async fn absent(
        &self,
        _: &Config,
        s: &PreparedBaseRelayTransaction,
        _: u64,
    ) -> Result<(), ChainBaseError> {
        for method in ["eth_getTransactionByHash", "eth_getTransactionReceipt"] {
            if !self
                .rpc(method, json!([s.transaction.tx_hash]))
                .await
                .is_null()
            {
                return Err(ChainBaseError::InvalidRelayIntent(
                    "local replacement already visible".into(),
                ));
            }
        }
        Ok(())
    }
    async fn send(
        &self,
        _: &Config,
        s: &PreparedBaseRelayTransaction,
        _: u64,
    ) -> Result<(), ChainBaseError> {
        assert_eq!(self.sends.fetch_add(1, Ordering::SeqCst), 0);
        let r = self
            .rpc("eth_sendRawTransaction", json!([s.signed_transaction]))
            .await;
        assert_eq!(r, s.transaction.tx_hash);
        Err(ChainBaseError::RelayerProvider(
            "local fixture lost accepted reply".into(),
        ))
    }
}
impl DispatchIo for Fork {
    async fn check(
        &self,
        c: &Config,
        p: &RetainedNonceDispatch,
        saved: &db::SavedNonceRetirement,
        old: &db::SavedTimeoutRelay,
        _: u64,
    ) -> Result<NonceRetirementObservation, ChainBaseError> {
        check_retained_dispatch_with_transport(
            [&c.primary, &c.shadow],
            p,
            saved.plan.id,
            &saved.plan.plan,
            &old.prepared,
            &saved.prepared,
            &old.scope,
            &c.policy,
            Utc::now(),
            self,
        )
        .await
    }
    async fn quote(&self, _: u128) -> Result<u64, ChainBaseError> {
        Ok(100)
    }
    async fn send(
        &self,
        c: &Config,
        saved: &PreparedBaseRelayTransaction,
        seconds: u64,
    ) -> Result<(), ChainBaseError> {
        RetirementIo::send(self, c, saved, seconds).await
    }
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL and explicitly configured loopback Base-ID Anvil with fixture contracts"]
async fn nonce77_retirement_local_chain_rehearsal() {
    run_rehearsal(false).await;
}
#[tokio::test]
#[ignore = "requires isolated PostgreSQL and explicitly configured loopback Base-ID Anvil with fixture contracts"]
async fn nonce77_retained_dispatch_local_chain_rehearsal() {
    run_rehearsal(true).await;
}
async fn run_rehearsal(retained: bool) {
    let io = Fork::new();
    assert_eq!(io.rpc("eth_chainId", json!([])).await, "0x2105");
    let db_url = env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
    db::test_support::assert_isolated_security_database(&db_url);
    let admin = sqlx::PgPool::connect(&db_url).await.unwrap();
    let schema = format!("retirement_evm_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = db_url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .options([("search_path", schema.as_str())]);
    let pool = sqlx::PgPool::connect_with(options).await.unwrap();
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.unwrap();
    let scope = TimeoutRelayScope {
        network: "base-mainnet".into(),
        bounty_contract: BOUNTY.into(),
        action: "expire_claim".into(),
        round: 3,
        factory_contract: "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9".into(),
        bounty_id: "0x73fa8fec134f131caa88fe3e0e16006485c098001344857e26640c51744ce234".into(),
        terms_hash: "0x82f436bb4025192f21a42bb4d6b818ace499e8690d24d6d1b11601ad26668bf3".into(),
    };
    let signer = PrivateKeySigner::random();
    let relayer =
        BaseTransactionRelayer::from_private_key(&format!("0x{}", hex::encode(signer.to_bytes())))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    let sender = relayer.address();
    io.rpc("anvil_setBalance", json!([sender, "0x8ac7230489e80000"]))
        .await;
    io.rpc("anvil_setNonce", json!([sender, "0x4d"])).await;
    io.rpc("evm_setNextBlockTimestamp", json!([Utc::now().timestamp()]))
        .await;
    io.rpc("anvil_mine", json!(["0x41", "0x0"])).await;
    let envelope = TransactionRequest::default()
        .with_to(BOUNTY.parse::<Address>().unwrap())
        .with_value(U256::ZERO)
        .with_input(scope.calldata().unwrap().parse::<Bytes>().unwrap())
        .with_nonce(77)
        .with_chain_id(8453)
        .with_gas_limit(25_000)
        .with_max_fee_per_gas(1_000_000)
        .with_max_priority_fee_per_gas(100_000)
        .build(&EthereumWallet::from(signer))
        .await
        .unwrap();
    let old = db::SavedTimeoutRelay {
        scope: scope.clone(),
        prepared: PreparedBaseRelayTransaction {
            nonce: 77,
            signed_transaction: format!("0x{}", hex::encode(envelope.encoded_2718())),
            transaction: BaseRelayedTransaction {
                relayer: sender.clone(),
                tx_hash: format!("{:#x}", envelope.tx_hash()),
                estimated_gas: 21_000,
                gas_limit: 25_000,
                max_fee_per_gas_wei: 1_000_000,
                max_priority_fee_per_gas_wei: 100_000,
                estimated_max_cost_wei: 25_000_000_000,
            },
        },
        send_attempted: false,
        marker_commit_unknown: false,
    };
    let mut state = crate::tests::test_state_with_operator_token_and_store(
        BountyNetwork::default(),
        "fixture-operator",
        store.clone(),
    );
    let implementation = "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9";
    let factory_code =
        fixture_code(&[("implementation()", format!("{:0>64}", &implementation[2..]))]);
    let implementation_code = fixture_code(&[
        ("status()", format!("{:064x}", 1)),
        ("round()", format!("{:064x}", 3)),
        (
            "factory()",
            format!("{:0>64}", &scope.factory_contract[2..]),
        ),
        ("bountyId()", scope.bounty_id[2..].into()),
        ("termsHash()", scope.terms_hash[2..].into()),
        ("fundedAmount()", format!("{:064x}", 21_300_000)),
        ("timeoutBondPool()", format!("{:064x}", 3_000_000)),
    ]);
    let oracle_code = fixture_code(&[
        ("getL1FeeUpperBound(uint256)", format!("{:064x}", 1000)),
        ("getOperatorFee(uint256)", format!("{:064x}", 10)),
    ]);
    for (addr, code) in [
        (scope.factory_contract.as_str(), factory_code.clone()),
        (implementation, implementation_code.clone()),
        (
            BOUNTY,
            format!(
                "0x363d3d373d3d3d363d73{}5af43d82803e903d91602b57fd5bf3",
                &implementation[2..]
            ),
        ),
        ("0x420000000000000000000000000000000000000f", oracle_code),
    ] {
        io.rpc("anvil_setCode", json!([addr, code])).await;
    }
    io.rpc("evm_setNextBlockTimestamp", json!([Utc::now().timestamp()]))
        .await;
    io.rpc("anvil_mine", json!(["0x41", "0x0"])).await;
    let before = io.bounty().await;
    let policy = chain_base::timeout_recovery::TimeoutRevalidationPolicy {
        factory_runtime_code_hash: format!(
            "{:#x}",
            alloy::primitives::keccak256(hex::decode(&factory_code[2..]).unwrap())
        ),
        implementation_contract: implementation.into(),
        implementation_runtime_code_hash: format!(
            "{:#x}",
            alloy::primitives::keccak256(hex::decode(&implementation_code[2..]).unwrap())
        ),
        max_gas: 300_000,
        max_fee_per_gas_wei: 10_000_000_000,
    };
    Arc::make_mut(&mut state).x402_relayer.relayer = Some(Arc::new(relayer));
    let c = Config {
        primary: "https://primary.fixture.invalid".into(),
        shadow: "https://shadow.fixture.invalid".into(),
        policy,
    };
    // Warm fixture reads only. No journal, signing or send authority yet.
    let _ = quote_with_transport(
        [&c.primary, &c.shadow],
        &old.prepared,
        &scope,
        &c.policy,
        Utc::now(),
        &io,
    )
    .await
    .unwrap();
    let lease = store
        .acquire_x402_relayer_lease("base-mainnet", 45)
        .await
        .unwrap()
        .unwrap();
    store
        .save_timeout_relay(
            &scope,
            lease,
            &old.prepared,
            store.timeout_budget_day().await.unwrap(),
        )
        .await
        .unwrap();
    store.upsert_autonomous_bounty_event("base-mainnet",&AutonomousBountyEvent{id:Uuid::new_v4(),log_key:Uuid::new_v4().to_string(),tx_hash:format!("0x{}","bb".repeat(32)),block_number:1,log_index:0,contract_address:scope.factory_contract.clone(),bounty_id:scope.bounty_id.clone(),kind:AutonomousBountyEventKind::CanonicalBountyCreated,data:json!({"bounty_contract":scope.bounty_contract,"terms_hash":scope.terms_hash}),occurred_at:Utc::now()}).await.unwrap();
    let (plan, o) = quote_with_transport(
        [&c.primary, &c.shadow],
        &old.prepared,
        &scope,
        &c.policy,
        Utc::now(),
        &io,
    )
    .await
    .unwrap();
    let plan = store
        .save_nonce_retirement_plan(&scope, lease, &plan, &o)
        .await
        .unwrap();
    let hash_before = hex::encode(sha2::Sha256::digest(
        old.prepared.signed_transaction.as_bytes(),
    ));
    let dispatch = if retained {
        // Isolated historical post-save fixture. No production journal is rewritten.
        let replacement = state
            .x402_relayer
            .relayer
            .as_ref()
            .unwrap()
            .sign_nonce_retirement(&plan.plan)
            .await
            .unwrap();
        let at = Utc::now() - chrono::Duration::minutes(5);
        let day = store.timeout_budget_day().await.unwrap();
        sqlx::query("INSERT INTO nonce_retirement_approvals(plan_id,operation_id,approved_digest,operator_identity,lease_token,observation,budget_day,reserved_micro_usdc,created_at,expires_at) VALUES($1,$2,$3,'isolated-local-chain-fixture',$4,$5,$6,100,$7,$8)")
            .bind(plan.id).bind(plan.plan.operation_id).bind(&plan.digest).bind(lease)
            .bind(serde_json::to_value(&o).unwrap()).bind(day).bind(at)
            .bind(at+chrono::Duration::seconds(120)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO gas_sponsorship_daily_budget(budget_key,budget_day,reserved_micro_usdc,limit_micro_usdc) VALUES('platform',$1,100,1000000)")
            .bind(day).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO nonce_retirement_envelopes(operation_id,plan_id,tx_hash,signed_transaction,transaction_metadata) VALUES($1,$2,$3,$4,$5)")
            .bind(plan.plan.operation_id).bind(plan.id).bind(&replacement.transaction.tx_hash)
            .bind(&replacement.signed_transaction).bind(serde_json::to_value(&replacement.transaction).unwrap())
            .execute(&pool).await.unwrap();
        Arc::make_mut(&mut state).x402_relayer.relayer = None;
        let saved = store
            .nonce_retirement(plan.plan.operation_id)
            .await
            .unwrap()
            .unwrap();
        let proposal = RetainedNonceDispatch::propose(
            [&c.primary, &c.shadow],
            plan.id,
            &plan.plan,
            &saved.prepared,
            Utc::now(),
        )
        .unwrap();
        let observation = DispatchIo::check(&io, &c, &proposal, &saved, &old, 15)
            .await
            .unwrap();
        let dispatch = store
            .save_retained_nonce_dispatch(&scope, &saved, lease, &proposal, &observation)
            .await
            .unwrap();
        initial_dispatch(
            &state,
            &c,
            &old,
            &saved,
            &dispatch,
            &dispatch.digest,
            lease,
            TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
            &io,
        )
        .await
        .unwrap();
        assert_eq!(
            store
                .nonce_retirement(plan.plan.operation_id)
                .await
                .unwrap()
                .unwrap()
                .prepared
                .signed_transaction,
            replacement.signed_transaction
        );
        Some(dispatch)
    } else {
        first_send(
            &state,
            &c,
            &old,
            &plan,
            &plan.digest,
            lease,
            TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
            &io,
        )
        .await
        .unwrap();
        None
    };
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    let saved = store
        .nonce_retirement(plan.plan.operation_id)
        .await
        .unwrap()
        .unwrap();
    assert!(saved.send_attempted);
    assert_eq!(
        io.rpc("eth_getTransactionCount", json!([sender, "pending"]))
            .await,
        "0x4e"
    );
    assert!(store
        .gas_sponsor_budget()
        .ensure_sender_nonce_available(8453, &sender, 78)
        .await
        .is_err());
    assert!(confirm_with_transport(
        [&c.primary, &c.shadow],
        &plan.plan,
        &saved.prepared,
        Utc::now(),
        &io
    )
    .await
    .is_err());
    io.rpc("anvil_mine", json!(["0x1", "0x0"])).await;
    assert!(
        confirm_with_transport(
            [&c.primary, &c.shadow],
            &plan.plan,
            &saved.prepared,
            Utc::now(),
            &io
        )
        .await
        .is_err(),
        "included but not finalized"
    );
    io.rpc("anvil_mine", json!(["0x40", "0x0"])).await;
    let proof = confirm_with_transport(
        [&c.primary, &c.shadow],
        &plan.plan,
        &saved.prepared,
        Utc::now(),
        &io,
    )
    .await
    .unwrap();
    store
        .confirm_nonce_retirement(&saved, &proof)
        .await
        .unwrap();
    assert!(store
        .gas_sponsor_budget()
        .ensure_sender_nonce_available(8453, &sender, 78)
        .await
        .is_ok());
    let old_attempt = io
        .response(
            "eth_sendRawTransaction",
            json!([old.prepared.signed_transaction]),
        )
        .await;
    assert!(old_attempt["error"]["message"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("nonce"));
    assert_eq!(io.bounty().await, before);
    let original = store
        .timeout_relay(plan.plan.operation_id, "base-mainnet")
        .await
        .unwrap()
        .unwrap();
    assert!(!original.send_attempted);
    assert_eq!(
        hex::encode(sha2::Sha256::digest(
            original.prepared.signed_transaction.as_bytes()
        )),
        hash_before
    );
    assert!(first_send(
        &state,
        &c,
        &old,
        &plan,
        &plan.digest,
        lease,
        TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
        &io
    )
    .await
    .is_err());
    if let Some(dispatch) = dispatch {
        assert!(initial_dispatch(
            &state,
            &c,
            &old,
            &saved,
            &dispatch,
            &dispatch.digest,
            lease,
            TimeoutLeaseBudget::new(tokio::time::Instant::now(), 45, 15).unwrap(),
            &io
        )
        .await
        .is_err());
        let authorizations: i64 =
            sqlx::query_scalar("SELECT count(*) FROM retained_nonce_dispatch_authorizations")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(authorizations, 1);
        assert!(state.x402_relayer.relayer.is_none());
    }
    assert_eq!(io.sends.load(Ordering::SeqCst), 1);
    eprintln!("local_retirement=passed retained_dispatch={retained} original_nonce=77 finalized_nonce={} sends=1 original_send_attempts=0 bounty_unchanged=true two_origins_one_local_chain=true fixture_contracts=true",proof.finalized_nonce);
}

// Test-only EVM selector dispatcher. Unknown methods revert; values are explicit
// fixture state, not a claim about the deployed contracts or Base fee oracle.
fn fixture_code(methods: &[(&str, String)]) -> String {
    let mut code = vec![0x60, 0x00, 0x35, 0x60, 0xe0, 0x1c];
    let mut destinations = Vec::new();
    for (sig, _) in methods {
        code.push(0x80);
        code.push(0x63);
        code.extend_from_slice(&alloy::primitives::keccak256(sig.as_bytes())[..4]);
        code.extend_from_slice(&[0x14, 0x61]);
        destinations.push(code.len());
        code.extend_from_slice(&[0, 0, 0x57]);
    }
    code.extend_from_slice(&[0x60, 0, 0x60, 0, 0xfd]);
    for ((_, word), offset) in methods.iter().zip(destinations) {
        let target = u16::try_from(code.len()).unwrap().to_be_bytes();
        code[offset..offset + 2].copy_from_slice(&target);
        code.extend_from_slice(&[0x5b, 0x50, 0x7f]);
        code.extend(hex::decode(word).unwrap());
        code.extend_from_slice(&[0x60, 0, 0x52, 0x60, 0x20, 0x60, 0, 0xf3]);
    }
    format!("0x{}", hex::encode(code))
}
