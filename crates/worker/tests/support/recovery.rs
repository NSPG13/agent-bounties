use chrono::Utc;
use db::PostgresStore;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

pub const FACTORY: &str = service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_V2_FACTORY;
pub const START: u64 = 52_275_492;
pub struct Fixture {
    pub store: PostgresStore,
    pub pool: PgPool,
    pub url: String,
    pub schema: String,
}
impl Fixture {
    pub async fn new(migrate: bool) -> Self {
        let base = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap();
        db::test_support::assert_isolated_security_database(&base);
        let admin = PgPool::connect(&base).await.unwrap();
        let schema = format!("recovery_reader_{}", uuid::Uuid::new_v4().simple());
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
    pub async fn subscription(&self) {
        self.store
            .create_discovery_webhook_subscription(&db::NewDiscoveryWebhookSubscription {
                id: uuid::Uuid::new_v4(),
                endpoint_url: "https://notification.invalid/never-send".into(),
                filters: domain::DiscoverySubscriptionFilters::default(),
                management_token_hash: "fixture".into(),
            })
            .await
            .unwrap();
    }
    pub async fn snapshot(&self) -> BTreeMap<String, String> {
        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT tablename FROM pg_tables WHERE schemaname=$1 ORDER BY tablename",
        )
        .bind(&self.schema)
        .fetch_all(&self.pool)
        .await
        .unwrap();
        let mut out = BTreeMap::new();
        for name in tables {
            assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'));
            let hash: String = sqlx::query_scalar(&format!("SELECT md5(COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text)::text,'[]')) FROM \"{name}\" t"))
                .fetch_one(&self.pool).await.unwrap();
            out.insert(name, hash);
        }
        out
    }
    pub async fn deliveries(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM webhook_deliveries")
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

fn logs() -> Vec<Value> {
    use chain_base::AutonomousBountyEventKind as Kind;
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../chain-base/tests/fixtures/autonomous-v2-loop.json"
    ))
    .unwrap();
    let mut selected = Vec::new();
    let mut id = None;
    for raw in fixture["logs"].as_array().unwrap() {
        let decoded = chain_base::decode_autonomous_bounty_logs(
            chain_base::rpc_logs_to_evm_logs([serde_json::from_value::<chain_base::RpcEvmLog>(
                raw.clone(),
            )
            .unwrap()])
            .unwrap(),
        )
        .unwrap();
        let Some(event) = decoded.first() else {
            continue;
        };
        if event.kind == Kind::CanonicalBountyCreated && id.is_none() {
            id = Some(event.bounty_id.clone());
        }
        if Some(&event.bounty_id) == id.as_ref()
            && matches!(
                event.kind,
                Kind::CanonicalBountyCreated
                    | Kind::CanonicalBountyTermsCommitted
                    | Kind::CanonicalBountyEconomicsConfigured
                    | Kind::CanonicalBountyVerificationConfigured
                    | Kind::CanonicalBountyPlatformFeeConfigured
            )
        {
            let mut value = raw.clone();
            value["address"] = json!(FACTORY);
            value["blockNumber"] = json!(format!("0x{START:x}"));
            selected.push(value);
        }
    }
    assert_eq!(selected.len(), 5);
    selected
}

pub struct Rpc {
    pub url: String,
    pub safe: Arc<AtomicU64>,
    pub requests: Arc<Mutex<Vec<Value>>>,
    pub event_count: usize,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Rpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Rpc {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let safe = Arc::new(AtomicU64::new(START));
        let observed = safe.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let saved = requests.clone();
        let logs = logs();
        let event_count = logs.len();
        let timestamp = Utc::now().timestamp();
        let task = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut input = BufReader::new(socket);
                let mut size = 0;
                loop {
                    let mut line = String::new();
                    input.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        size = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; size];
                input.read_exact(&mut body).await.unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                saved.lock().unwrap().push(request.clone());
                let safe = observed.load(Ordering::SeqCst);
                let result = match request["method"].as_str().unwrap() {
                    "eth_chainId" => json!("0x2105"),
                    "eth_blockNumber" => json!(format!("0x{:x}", safe + 2)),
                    "eth_getBlockByNumber" => {
                        let tag = request["params"][0].as_str().unwrap();
                        let number = if tag == "safe" {
                            safe
                        } else {
                            u64::from_str_radix(tag.strip_prefix("0x").unwrap(), 16).unwrap()
                        };
                        json!({"number":format!("0x{number:x}"),"hash":format!("0x{number:064x}"),"timestamp":format!("0x{:x}",timestamp)})
                    }
                    "eth_getLogs" => {
                        let query = &request["params"][0];
                        let from = u64::from_str_radix(
                            query["fromBlock"]
                                .as_str()
                                .unwrap()
                                .strip_prefix("0x")
                                .unwrap(),
                            16,
                        )
                        .unwrap();
                        let to = u64::from_str_radix(
                            query["toBlock"]
                                .as_str()
                                .unwrap()
                                .strip_prefix("0x")
                                .unwrap(),
                            16,
                        )
                        .unwrap();
                        if query["address"] == FACTORY && from <= START && START <= to {
                            json!(logs)
                        } else {
                            json!([])
                        }
                    }
                    method => panic!("unexpected RPC/write method: {method}"),
                };
                let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
                input.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            safe,
            requests,
            event_count,
            task,
        }
    }
    pub fn config(&self) -> worker::AutonomousIndexerConfig {
        worker::AutonomousIndexerConfig {
            network: "base-mainnet".into(),
            rpc_url: self.url.clone(),
            bootstrap_archive_rpc_url: None,
            factory_contract: FACTORY.into(),
            start_block: Some(START),
            poll_seconds: 1,
            confirmations: 2,
            max_blocks_per_query: 1,
            request_id: 1,
            public_base_url: "https://api.example.invalid".into(),
        }
    }
}
