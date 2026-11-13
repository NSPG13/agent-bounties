use alloy::{
    consensus::{Transaction, TxEnvelope},
    eips::eip2718::Decodable2718,
};
use chain_base::{BaseRelayedTransaction, PreparedBaseRelayTransaction};
use db::{
    managed_service_orders::{ManagedFundingOperation, ManagedOrder},
    PostgresStore,
};
use hmac::{Hmac, Mac};
use managed_orders::{
    funding::FundingTarget,
    stripe_checkout::{verify_managed_checkout_event, CheckoutPolicy},
    FiatCurrency, ManagedQuote, PriceBasis, QuoteInput, ServiceTerms,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use uuid::Uuid;

pub const FACTORY: &str = service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_FACTORY;
const IMPLEMENTATION: &str = "0x8420c9bd1ff8a6b1abc4230b349538612c58f3a6";
const WALLET: &str = "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";
const RECIPIENT: &str = "0x884834e884d6e93462655a2820140ad03e6747bc";
const DELEGATE: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";
pub const MUTABLE_OPERATION: &[&str] = &["managed_bounty_funding_operations"];
pub const MUTABLE_SIGNING: &[&str] = &[
    "managed_bounty_funding_operations",
    "relay_transaction_outbox",
    "gas_sponsorship_daily_budget",
];

pub struct Fixture {
    pub store: PostgresStore,
    pub pool: PgPool,
    url: String,
    schema: String,
}

impl Fixture {
    pub async fn new(label: &str, migrate: bool) -> Self {
        let base = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&base);
        let admin = PgPool::connect(&base).await.unwrap();
        assert!([
            "startup",
            "idle",
            "busy",
            "expired",
            "disputed",
            "mined",
            "old",
            "wrong_network",
            "wrong_factory",
            "wrong_wallet",
            "wrong_digest",
            "core",
            "fx_unsigned"
        ]
        .contains(&label));
        let schema = format!("recovery_funding_{label}");
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        admin.close().await;
        let mut parsed = reqwest::Url::parse(&base).unwrap();
        parsed
            .query_pairs_mut()
            .append_pair("options", &format!("-csearch_path={schema}"));
        let url = parsed.to_string();
        let pool = PgPool::connect(&url).await.unwrap();
        let store = PostgresStore::from_pool(pool.clone());
        if migrate {
            store.migrate().await.unwrap();
        }
        Self {
            store,
            pool,
            url,
            schema,
        }
    }

    pub async fn snapshot(&self) -> BTreeMap<String, String> {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT tablename FROM pg_tables WHERE schemaname=$1 ORDER BY tablename",
        )
        .bind(&self.schema)
        .fetch_all(&self.pool)
        .await
        .unwrap();
        let mut result = BTreeMap::new();
        for table in tables {
            assert!(table
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            let digest: String = sqlx::query_scalar(&format!("SELECT md5(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text,'[]')) FROM \"{table}\" t")).fetch_one(&self.pool).await.unwrap();
            result.insert(table, digest);
        }
        result
    }

    pub async fn status(&self, id: Uuid) -> String {
        let order = self
            .store
            .get_managed_service_order("fixture-buyer", id)
            .await
            .unwrap()
            .unwrap();
        serde_json::to_value(order.status)
            .unwrap()
            .as_str()
            .unwrap()
            .into()
    }

    async fn protected(&self) -> Value {
        sqlx::query_scalar(PROTECTED_SQL)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    pub async fn assert_held(&self, orders: i64, journals: i64) {
        let held: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_stripe_bank_actions WHERE state='held' AND funding_operation_id IS NOT NULL").fetch_one(&self.pool).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM relay_transaction_outbox")
            .fetch_one(&self.pool)
            .await
            .unwrap();
        let funded: i64 = sqlx::query_scalar("SELECT count(*) FROM managed_bounty_funding_operations WHERE outcome='funded' OR funded_tx_hash IS NOT NULL OR funded_block IS NOT NULL").fetch_one(&self.pool).await.unwrap();
        assert_eq!((held, count, funded), (orders, journals, 0));
    }

    pub async fn reserved(&self) -> (ManagedOrder, ManagedFundingOperation) {
        let now = chrono::Utc::now().timestamp();
        let nonce = format!("0x{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let terms = fixture_terms(now + 86_400, &nonce);
        let quote = ManagedQuote::new(QuoteInput {
            quote_id: Uuid::new_v4(),
            buyer_principal: "fixture-buyer".into(),
            stripe_account_id: "acct_fixture_only".into(),
            livemode: true,
            issued_at: now - 1,
            expires_at: now + 7200,
            terms: ServiceTerms {
                deliverable: "Test deliverable".into(),
                acceptance_criteria: "Test acceptance criteria".into(),
                delivery_deadline: now + 86_400,
                terms_version: "test-1".into(),
                remedies: "Test remedies".into(),
            },
            currency: FiatCurrency::Mxn,
            price_basis: PriceBasis {
                reference: "synthetic-test-price".into(),
                minor_units_per_usdc_numerator: 2000,
                minor_units_per_usdc_denominator: 1,
                valid_until: now + 7200,
            },
            solver_reward_micro_usdc: 100_000_000,
            verifier_reserve_micro_usdc: 5_000_000,
            additional_costs: vec![],
            bounty: managed_orders_bounty(now + 86_400, &nonce),
        })
        .unwrap();
        self.store
            .upsert_autonomous_bounty_terms(&terms)
            .await
            .unwrap();
        self.store.put_managed_service_quote(&quote).await.unwrap();
        let order = self
            .store
            .accept_managed_service_quote(
                "fixture-buyer",
                quote.input().quote_id,
                &quote.digest().unwrap(),
                &Uuid::new_v4().to_string(),
            )
            .await
            .unwrap();
        let policy = CheckoutPolicy {
            stripe_account_id: "acct_fixture_only".into(),
            livemode: true,
            currency: FiatCurrency::Mxn,
            minimum_minor: 1000,
            maximum_minor: 1_000_000,
            payment_method_configuration: None,
            adaptive_pricing: false,
        };
        let plan = self
            .store
            .prepare_managed_checkout("fixture-buyer", order.order_id, policy)
            .await
            .unwrap();
        let request = plan.request(plan.created_at(), plan.policy()).unwrap();
        let total: u64 = request.body["line_items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| {
                line["price_data"]["unit_amount"].as_u64().unwrap()
                    * line["quantity"].as_u64().unwrap_or(1)
            })
            .sum();
        let id = format!("cs_live_{}", order.order_id.simple());
        let response = json!({"id":id,"object":"checkout.session","mode":"payment","livemode":true,
            "metadata":request.body["metadata"],"client_reference_id":plan.order_id(),"amount_total":total,
            "currency":"mxn","status":"complete","payment_status":"paid","payment_intent":format!("pi_{}",order.order_id.simple()),
            "expires_at":plan.expires_at(),"url":format!("https://checkout.stripe.com/c/pay/{id}")});
        let checkout_lease = self
            .store
            .claim_managed_checkout(order.order_id)
            .await
            .unwrap()
            .expect("fixture checkout lease");
        self.store
            .complete_managed_checkout(&checkout_lease, &plan.session_binding(&response).unwrap())
            .await
            .unwrap();
        let body = serde_json::to_vec(&json!({"id":format!("evt_{}",order.order_id.simple()),"object":"event","type":"checkout.session.completed","livemode":true,"created":plan.created_at(),"data":{"object":response}})).unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(b"managed-fixture-secret").unwrap();
        mac.update(format!("{now}.").as_bytes());
        mac.update(&body);
        let signature = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
        let event = verify_managed_checkout_event(
            &body,
            &signature,
            b"managed-fixture-secret",
            "acct_fixture_only",
            true,
        )
        .unwrap();
        self.store
            .reconcile_managed_checkout_event(&event)
            .await
            .unwrap();
        let op = self
            .store
            .reserve_managed_funding(
                order.order_id,
                &FundingTarget {
                    network: "base-mainnet".into(),
                    factory: FACTORY.into(),
                    funding_wallet: WALLET.into(),
                    fee: fixture_fee(),
                },
                3600,
            )
            .await
            .unwrap();
        (order, op)
    }

    pub async fn replace_plan(&self, op: &ManagedFundingOperation, corrupt: bool) {
        let mut hash = Sha256::new();
        hash.update(b"agent-bounties:managed-funding-plan:v1\0");
        hash.update(serde_json::to_vec(&op.plan).unwrap());
        let digest = if corrupt {
            "0".repeat(64)
        } else {
            hex::encode(hash.finalize())
        };
        sqlx::query(
            "UPDATE managed_bounty_funding_operations SET plan=$1,plan_digest=$2 WHERE order_id=$3",
        )
        .bind(serde_json::to_value(&op.plan).unwrap())
        .bind(digest)
        .bind(op.order_id)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    pub async fn seed_journal(&self, op: &ManagedFundingOperation) {
        // Receipt-only tests use a structurally stored sentinel, not a claimed valid signature.
        let saved = PreparedBaseRelayTransaction {
            nonce: 7,
            signed_transaction: "0x02".into(),
            transaction: BaseRelayedTransaction {
                relayer: DELEGATE.into(),
                tx_hash: format!("0x{}", hex::encode(sha3::Keccak256::digest([2]))),
                estimated_gas: 21000,
                gas_limit: 25200,
                max_fee_per_gas_wei: 3,
                max_priority_fee_per_gas_wei: 1,
                estimated_max_cost_wei: 75640,
            },
        };
        self.store
            .save_relay_outbox(op.operation_id, "base-mainnet", &saved)
            .await
            .unwrap();
    }

    pub async fn run(
        &self,
        name: &str,
        mode: &str,
        action: &str,
        mutable: &[&str],
        op: Option<&ManagedFundingOperation>,
    ) -> Value {
        let before = self.snapshot().await;
        let protected_before = if before.is_empty() {
            json!({})
        } else {
            self.protected().await
        };
        let export = self
            .export(name, mode, action, mutable, op, &before, &protected_before)
            .await;
        let mut rpc = Rpc::start(mode, self.pool.clone(), op.cloned()).await;
        let settings = if mode == "invalid" {
            "{".into()
        } else {
            json!({"network":"base-mainnet","funding_wallet":WALLET,"min_paid_age_seconds":0,"deadline_margin_seconds":3600,"max_gas":1_500_000,"max_fee_per_gas_wei":2_000_000_000u64,"lease_seconds":180,"poll_seconds":300}).to_string()
        };
        let logs = tempfile::tempdir().unwrap();
        let out_path = logs.path().join("stdout");
        let err_path = logs.path().join("stderr");
        let mut command = Command::new(env!("CARGO_BIN_EXE_worker"));
        command
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&out_path).unwrap())
            .stderr(std::fs::File::create(&err_path).unwrap())
            .env("DATABASE_URL", &self.url)
            .env("BASE_MAINNET_RPC_URL", &rpc.url)
            .env("BASE_INDEXER_PROTOCOL", "managed-bounty-funding")
            .env("RUST_LOG", "info")
            .env("RENDER_SERVICE_ID", "srv-aaaaaaaaaaaaaaaaaaaa")
            .env(
                "RENDER_GIT_COMMIT",
                "1111111111111111111111111111111111111111",
            )
            .env("RECOVERY_FUNDING_MODE", "active")
            .env("ENABLE_HOSTED_FINANCIAL_SIGNING", "true")
            .env("BASE_MAINNET_BOUNTY_V2_FACTORY", FACTORY)
            .env("BASE_MAINNET_BOUNTY_V2_IMPLEMENTATION", IMPLEMENTATION)
            .env("BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_BPS", "750")
            .env("BASE_MAINNET_BOUNTY_V2_PLATFORM_FEE_RECIPIENT", RECIPIENT)
            .env("MANAGED_BOUNTY_FUNDING", &settings)
            .env(
                "MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY",
                format!("{:064x}", 1),
            );
        let mut child = command.spawn().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let stdout = bounded_file(&out_path);
                let stderr = bounded_file(&err_path);
                if let Some(status) = child.try_wait().unwrap() {
                    assert_eq!(
                        action, "config_error",
                        "unexpected worker exit: {status}: {stderr}"
                    );
                    assert!(!status.success());
                    assert!(stderr.contains("MANAGED_BOUNTY_FUNDING is invalid"));
                    break json!({"action":"config_error"});
                }
                if let Some(report) = stdout
                    .lines()
                    .chain(stderr.lines())
                    .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                    .find(|v| {
                        v["schema"] == "agent-bounties/managed-funding-v1"
                            || v["schema"] == "agent-bounties/managed-funding-recovery-v1"
                    })
                {
                    if action == "poll_error" {
                        assert_eq!(report["decision"], "retry_from_persisted_operation");
                    } else {
                        assert_eq!(report["action"], action, "{report}");
                    }
                    break report;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        if child.try_wait().unwrap().is_none() {
            child.start_kill().unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .unwrap()
            .unwrap();
        let report = result.expect("worker did not finish one poll within 30 seconds");
        let requests = rpc.requests.lock().unwrap().clone();
        assert!(
            rpc.errors.lock().unwrap().is_empty(),
            "{:?}",
            rpc.errors.lock().unwrap()
        );
        if matches!(mode, "invalid" | "silent") {
            assert!(requests.is_empty());
        }
        if mode != "positive" {
            assert!(!requests
                .iter()
                .any(|v| v["method"] == "eth_sendRawTransaction"));
        }
        if mode == "positive" {
            assert_eq!(
                requests
                    .iter()
                    .filter(|v| v["method"] == "eth_sendRawTransaction")
                    .count(),
                1
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|v| v["method"] == "eth_getTransactionCount")
                    .count(),
                usize::from(action == "signed")
            );
        }
        rpc.finish().await;
        let after = self.snapshot().await;
        let protected_after = self.protected().await;
        if action != "config_error" {
            assert_protected(&protected_before, &protected_after, action, op);
        }
        if action != "config_error" {
            assert_eq!(
                before.keys().collect::<Vec<_>>(),
                after.keys().collect::<Vec<_>>()
            );
            for (table, hash) in &before {
                if !mutable.contains(&table.as_str()) {
                    assert_eq!(Some(hash), after.get(table), "unexpected write: {table}");
                }
            }
        }
        if let Some(path) = export {
            let facts = sqlx::query_scalar::<_,Value>("SELECT jsonb_build_object('held',(SELECT count(*) FROM managed_stripe_bank_actions WHERE state='held' AND funding_operation_id IS NOT NULL),'journals',(SELECT count(*) FROM relay_transaction_outbox),'funded',(SELECT count(*) FROM managed_bounty_funding_operations WHERE outcome='funded'),'orders',(SELECT COALESCE(jsonb_object_agg(order_id::text,outcome),'{}') FROM managed_bounty_funding_operations),'journal',(SELECT COALESCE(jsonb_agg(jsonb_build_object('operation_id',operation_id,'nonce',nonce,'tx_hash',tx_hash,'raw',signed_transaction,'send_attempts',send_attempts) ORDER BY operation_id),'[]') FROM relay_transaction_outbox))").fetch_one(&self.pool).await.unwrap();
            let result = json!({"report":report,"requests":requests,"facts":facts,"before":before,"after":after,"protected_before":protected_before,"protected_after":protected_after});
            std::fs::write(
                path.join("native-result.json"),
                serde_json::to_vec_pretty(&result).unwrap(),
            )
            .unwrap();
        }
        report
    }

    pub async fn core_poll(&self, name: &str, action: &str, op: &ManagedFundingOperation) {
        use worker::managed_funding::{poll_managed_funding_once, ManagedFundingConfig};
        let before = self.snapshot().await;
        let protected_before = self.protected().await;
        let mut rpc = Rpc::start("positive", self.pool.clone(), Some(op.clone())).await;
        let budget = Arc::new(SyntheticBudget {
            durable: self.store.gas_sponsor_budget(),
            reservations: Mutex::new(Vec::new()),
        });
        // Reconstructed each entry: no in-memory transaction/nonce can bridge these calls.
        let config = ManagedFundingConfig {
            settings: serde_json::from_value(json!({"network":"base-mainnet","funding_wallet":WALLET,"min_paid_age_seconds":0,"deadline_margin_seconds":3600,"max_gas":1_500_000,"max_fee_per_gas_wei":2_000_000_000u64,"lease_seconds":180,"poll_seconds":300})).unwrap(),
            target: FundingTarget { network:"base-mainnet".into(),factory:FACTORY.into(),funding_wallet:WALLET.into(),fee:fixture_fee() },
            chain_id:8453, rpc_url:rpc.url.clone(),
            delegate:chain_base::BaseTransactionRelayer::from_private_key(&format!("{:064x}",1)).unwrap().with_gas_budget(budget.clone()),
        };
        let report = tokio::time::timeout(
            Duration::from_secs(15),
            poll_managed_funding_once(&self.store, &config),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(report.action, action);
        assert_eq!(budget.reservations.lock().unwrap().len(), 1);
        let requests = rpc.requests.lock().unwrap().clone();
        assert!(rpc.errors.lock().unwrap().is_empty());
        rpc.finish().await;
        assert_eq!(
            requests
                .iter()
                .filter(|v| v["method"] == "eth_sendRawTransaction")
                .count(),
            1
        );
        assert_eq!(
            requests
                .iter()
                .filter(|v| v["method"] == "eth_getTransactionCount")
                .count(),
            usize::from(action == "signed")
        );
        let after = self.snapshot().await;
        for (table, hash) in before {
            if !MUTABLE_SIGNING.contains(&table.as_str()) {
                assert_eq!(
                    after.get(&table),
                    Some(&hash),
                    "unexpected core write {table}"
                );
            }
        }
        let protected_after = self.protected().await;
        assert_protected(&protected_before, &protected_after, action, Some(op));
        if let Ok(root) = std::env::var("RECOVERY_FUNDING_FIXTURE_EXPORT") {
            let result = json!({"scope":"production_core_poll_with_synthetic_budget_not_shipping_process_or_real_FX_EVM","report":report,"requests":requests,"synthetic_budget_reservations":*budget.reservations.lock().unwrap(),"protected_before":protected_before,"protected_after":protected_after});
            std::fs::write(
                PathBuf::from(root).join(format!("{name}.json")),
                serde_json::to_vec_pretty(&result).unwrap(),
            )
            .unwrap();
        }
    }

    async fn export(
        &self,
        name: &str,
        mode: &str,
        action: &str,
        mutable: &[&str],
        op: Option<&ManagedFundingOperation>,
        before: &BTreeMap<String, String>,
        protected_before: &Value,
    ) -> Option<PathBuf> {
        let Ok(root) = std::env::var("RECOVERY_FUNDING_FIXTURE_EXPORT") else {
            return None;
        };
        let root = PathBuf::from(root);
        assert!(root.is_absolute());
        let path = root.join(name);
        std::fs::create_dir(&path).unwrap();
        let request = json!({"name":name,"database_schema":self.schema});
        std::fs::write(
            path.join("request.tmp"),
            serde_json::to_vec(&request).unwrap(),
        )
        .unwrap();
        std::fs::rename(path.join("request.tmp"), path.join("request.json")).unwrap();
        // The host lifecycle driver alone owns Docker. No socket/CLI is mounted in this
        // no-egress compiler container; it exports only the frozen fixture schema.
        tokio::time::timeout(Duration::from_secs(30), async {
            while !path.join("export-ready.json").is_file() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let receipt: Value =
            serde_json::from_slice(&std::fs::read(path.join("export-ready.json")).unwrap())
                .unwrap();
        let bytes = std::fs::metadata(path.join("before.sql")).unwrap().len();
        assert!(bytes <= 16 * 1024 * 1024);
        let dump = std::fs::read(path.join("before.sql")).unwrap();
        assert_eq!(receipt["sha256"], hex::encode(Sha256::digest(&dump)));
        let metadata = json!({"schema":"agentizable/funding-process-fixture-v1","name":name,"database_schema":self.schema,"rpc_mode":mode,"action":action,"mutable_tables":mutable,"before":before,"protected_before":protected_before,"protected_sql":PROTECTED_SQL,"plan":op.map(|v|&v.plan),"operation_id":op.map(|v|v.operation_id),"sql_sha256":hex::encode(Sha256::digest(&dump)),"valid_until":chrono::Utc::now().timestamp()+20*3600});
        std::fs::write(
            path.join("case.json"),
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        Some(path)
    }
}

const PROTECTED_SQL: &str = "SELECT jsonb_build_object(
'operations', (SELECT COALESCE(jsonb_object_agg(order_id::text, to_jsonb(t)-ARRAY['lease_token','lease_until','outcome','tx_hash']), '{}') FROM managed_bounty_funding_operations t),
'prior_hashes', (SELECT COALESCE(jsonb_object_agg(order_id::text,tx_hash) FILTER (WHERE tx_hash IS NOT NULL), '{}') FROM managed_bounty_funding_operations),
'journals', (SELECT COALESCE(jsonb_object_agg(operation_id::text,to_jsonb(t)-ARRAY['send_attempts','next_send_at']), '{}') FROM relay_transaction_outbox t),
'gas', (SELECT COALESCE(jsonb_object_agg(budget_key||':'||budget_day::text,to_jsonb(t)-ARRAY['reserved_micro_usdc','updated_at']), '{}') FROM gas_sponsorship_daily_budget t))";

fn assert_protected(
    before: &Value,
    after: &Value,
    action: &str,
    op: Option<&ManagedFundingOperation>,
) {
    assert_eq!(
        before["operations"], after["operations"],
        "committed operation identity changed"
    );
    for key in ["prior_hashes", "journals", "gas"] {
        let old = before[key].as_object().unwrap();
        let new = after[key].as_object().unwrap();
        for (id, row) in old {
            assert_eq!(new.get(id), Some(row), "immutable {key} row {id} changed");
        }
        if key == "journals" {
            let added: Vec<_> = new.keys().filter(|id| !old.contains_key(*id)).collect();
            if action == "signed" {
                assert_eq!(added, vec![&op.unwrap().operation_id.to_string()]);
            } else {
                assert!(added.is_empty(), "unexpected new journal");
            }
        }
        if key == "gas" && !matches!(action, "signed" | "rebroadcast") {
            assert_eq!(old, new);
        }
    }
}

fn bounded_file(path: &std::path::Path) -> String {
    assert!(
        std::fs::metadata(path).unwrap().len() < 1024 * 1024,
        "worker output exceeded one MiB"
    );
    std::fs::read_to_string(path).unwrap()
}

pub fn assert_signed(raw: &str, op: &ManagedFundingOperation) {
    let bytes = hex::decode(raw.strip_prefix("0x").unwrap()).unwrap();
    let mut remaining = bytes.as_slice();
    let envelope = TxEnvelope::decode_2718(&mut remaining).unwrap();
    assert!(remaining.is_empty());
    assert!(matches!(&envelope, TxEnvelope::Eip1559(_)));
    assert_eq!(
        envelope
            .signature()
            .recover_address_from_prehash(&envelope.signature_hash())
            .unwrap()
            .to_string()
            .to_ascii_lowercase(),
        DELEGATE
    );
    assert_eq!(envelope.chain_id(), Some(8453));
    assert_eq!(envelope.nonce(), 0);
    assert_eq!(
        envelope.to().unwrap().to_string().to_ascii_lowercase(),
        op.plan.funding_wallet
    );
    assert_eq!(envelope.value(), alloy::primitives::U256::ZERO);
    assert_eq!(
        format!("0x{}", hex::encode(envelope.input())),
        op.plan.calldata
    );
    assert!(envelope.gas_limit() <= 1_500_000 && envelope.max_fee_per_gas() <= 2_000_000_000);
}

struct SyntheticBudget {
    durable: Arc<dyn chain_base::RelayGasBudget>,
    reservations: Mutex<Vec<u128>>,
}
#[async_trait::async_trait]
impl chain_base::RelayGasBudget for SyntheticBudget {
    async fn ensure_sender_nonce_available(
        &self,
        chain: u64,
        sender: &str,
        nonce: u64,
    ) -> Result<(), chain_base::ChainBaseError> {
        self.durable
            .ensure_sender_nonce_available(chain, sender, nonce)
            .await
    }
    async fn reserve(&self, chain: u64, cost: u128) -> Result<(), chain_base::ChainBaseError> {
        assert_eq!(chain, 8453);
        assert!(cost > 0 && cost <= 1_500_000u128 * 2_000_000_000);
        self.reservations.lock().unwrap().push(cost);
        Ok(())
    }
}

struct Rpc {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    errors: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Rpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Rpc {
    async fn finish(&mut self) {
        assert!(
            !self.task.is_finished(),
            "RPC fixture task panicked or exited early"
        );
        self.task.abort();
        let result = (&mut self.task).await;
        assert!(
            result.is_err_and(|error| error.is_cancelled()),
            "RPC task did not cancel cleanly"
        );
    }
    async fn start(mode: &str, pool: PgPool, op: Option<ManagedFundingOperation>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let failures = errors.clone();
        let mode = mode.to_owned();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = tokio::time::timeout(Duration::from_secs(3), async {
                    let mut bytes = Vec::new();
                    loop {
                        let mut chunk = [0u8; 4096];
                        let count = stream.read(&mut chunk).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&chunk[..count]);
                        assert!(bytes.len() <= 65536);
                        let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") else {
                            continue;
                        };
                        let header = std::str::from_utf8(&bytes[..end]).unwrap();
                        let length: usize = header
                            .lines()
                            .filter_map(|v| v.split_once(':'))
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .unwrap()
                            .1
                            .trim()
                            .parse()
                            .unwrap();
                        assert!(length <= 60000);
                        if bytes.len() >= end + 4 + length {
                            break serde_json::from_slice::<Value>(
                                &bytes[end + 4..end + 4 + length],
                            )
                            .unwrap();
                        }
                    }
                })
                .await
                .unwrap();
                observed.lock().unwrap().push(request.clone());
                let method = request["method"].as_str().unwrap();
                let oracle = request["params"][0]["to"].as_str().is_some_and(|v| {
                    v.eq_ignore_ascii_case("0x420000000000000000000000000000000000000f")
                });
                let result = match (mode.as_str(), method) {
                    ("mined", "eth_getTransactionReceipt") => {
                        json!({"transactionHash":request["params"][0],"blockNumber":"0x64","blockHash":format!("0x{}","aa".repeat(32)),"status":"0x1","logs":[]})
                    }
                    ("null" | "positive" | "prepare", "eth_getTransactionReceipt") => Value::Null,
                    ("positive" | "prepare" | "reject", "eth_chainId") => json!("0x2105"),
                    ("positive" | "prepare", "eth_call") if oracle => {
                        json!(format!("0x{:064x}", 10))
                    }
                    ("positive" | "prepare", "eth_call") => json!("0x"),
                    ("positive" | "prepare", "eth_estimateGas") => json!("0x5208"),
                    ("positive" | "prepare", "eth_feeHistory") => {
                        json!({"oldestBlock":"0x1","baseFeePerGas":["0x1","0x1"],"gasUsedRatio":[0.5],"reward":[["0x1"]]})
                    }
                    ("positive" | "prepare", "eth_getTransactionCount") => json!("0x0"),
                    ("positive" | "prepare", "eth_getBalance") => json!("0x1000000000000000000"),
                    ("positive", "eth_sendRawTransaction") => {
                        let raw = request["params"][0].as_str().unwrap();
                        let operation = op.as_ref().unwrap();
                        assert_signed(raw, operation);
                        let saved = PostgresStore::from_pool(pool.clone())
                            .relay_outbox(operation.operation_id, "base-mainnet")
                            .await
                            .unwrap()
                            .unwrap();
                        assert_eq!(
                            saved.signed_transaction, raw,
                            "broadcast before durable exact bytes"
                        );
                        let hash = format!(
                            "0x{}",
                            hex::encode(sha3::Keccak256::digest(hex::decode(&raw[2..]).unwrap()))
                        );
                        assert_eq!(saved.transaction.tx_hash, hash);
                        assert_eq!(saved.nonce, 0);
                        json!(hash)
                    }
                    ("reject", "eth_call") => Value::Null,
                    _ => {
                        failures
                            .lock()
                            .unwrap()
                            .push(format!("unexpected {method} in {mode}"));
                        Value::Null
                    }
                };
                let body=if mode=="reject" && method=="eth_call" {json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"synthetic simulation rejected"}})}
                else {json!({"jsonrpc":"2.0","id":request["id"],"result":result})}.to_string();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            requests,
            errors,
            task,
        }
    }
}

fn fixture_fee() -> chain_base::AutonomousV2FactoryFee {
    chain_base::AutonomousV2FactoryFee {
        platform_fee_bps: 750,
        platform_fee_recipient: RECIPIENT.into(),
    }
}

fn fixture_terms(
    funding_deadline: i64,
    creation_nonce: &str,
) -> domain::AutonomousBountyTermsRecord {
    let usdc = |amount: u64| serde_json::json!({"amount": amount, "currency": "usdc"});
    let document = serde_json::json!({
        "schema_version": "agent-bounties/terms-v1",
        "title": "Test deliverable", "goal": "Deliver the test deliverable.",
        "acceptance_criteria": ["Test acceptance criteria"],
        "benchmark": {"engine": "customer_review_v1", "reviewer": "paying_customer",
            "acceptance": "all_published_criteria", "delivery_deadline": funding_deadline},
        "verification_policy": {"engine": "customer_review_v1", "mechanism": "signed_quorum",
            "threshold": 1, "verifiers": [chain_base::TEST_CUSTOMER_REVIEW_VERIFIER],
            "public_disclosure": "Customer review."},
        "evidence_schema": {"type": "object", "required": ["artifact_url", "artifact_sha256"],
            "properties": {"artifact_url": {"type": "string", "pattern": "^https://"},
                "artifact_sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}}},
        "source_url": null, "discovery_source": "managed_card_quote",
        "contract_terms": {
            "protocol_version": chain_base::AUTONOMOUS_V2_PROTOCOL_VERSION, "network": "base-mainnet",
            "settlement_token": chain_base::base_network_descriptor("base-mainnet").unwrap().native_usdc_token_address,
            "creator_wallet": WALLET, "solver_reward": usdc(100_000_000),
            "verifier_reward": usdc(5_000_000), "claim_bond": usdc(5_000_000),
            "initial_funding": usdc(112_500_000), "platform_fee_bps": 750,
            "platform_fee": usdc(7_500_000), "platform_fee_recipient": fixture_fee().platform_fee_recipient,
            "funding_deadline": funding_deadline, "claim_window_seconds": 604_800,
            "verification_window_seconds": 259_200, "creation_nonce": creation_nonce
        }
    });
    chain_base::build_autonomous_bounty_terms_record(
        WALLET,
        serde_json::from_value(document).unwrap(),
        chrono::Utc::now(),
    )
    .unwrap()
}

fn managed_orders_bounty(
    funding_deadline: i64,
    creation_nonce: &str,
) -> managed_orders::BountyCommitment {
    let terms = fixture_terms(funding_deadline, creation_nonce);
    let create = chain_base::autonomous_v2_create_from_terms(&terms, &fixture_fee()).unwrap();
    let plan = chain_base::AutonomousBountyTxPlanner::new(FACTORY, IMPLEMENTATION)
        .unwrap()
        .plan_v2_creation("base-mainnet", &create, &fixture_fee())
        .unwrap();
    let calldata = chain_base::encode_bounded_v2_funding_wallet_create_call(&create).unwrap();
    managed_orders::BountyCommitment {
        network: "base-mainnet".into(),
        factory: FACTORY.into(),
        funding_wallet: WALLET.into(),
        creation_nonce: create.base.creation_nonce.to_ascii_lowercase(),
        terms_hash: terms.terms_hash.to_ascii_lowercase(),
        verification_setup: "customer_review_v1".into(),
        funding_deadline,
        predicted_bounty: plan.plan.predicted_bounty_contract.to_ascii_lowercase(),
        create_call_keccak256: managed_orders::funding::calldata_keccak256(&calldata).unwrap(),
    }
}
