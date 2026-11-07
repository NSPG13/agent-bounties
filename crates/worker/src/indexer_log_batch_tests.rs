//! Actual poll regressions against a provider that refuses more than ten addresses.
//! These fixtures have no unverified timestamp backlog: a failed partition must
//! leave new-range history, projections, cursor/certification and deliveries intact.
use super::*;
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const NETWORK: &str = "base-sepolia";
const FACTORY: &str = "0x1111111111111111111111111111111111111111";

#[derive(Clone, Copy, Debug)]
enum Protocol {
    Autonomous(&'static str),
    Competition,
    Beta3,
}
#[derive(Clone, Copy, Debug)]
enum Scenario {
    Success,
    RefusedMiddle,
    InvalidLast,
}
fn address(n: usize) -> String {
    format!("0x{n:040x}")
}
fn hash(n: u64) -> String {
    format!("0x{n:064x}")
}
fn block(n: u64) -> chain_base::BaseBlockIdentity {
    chain_base::BaseBlockIdentity {
        number: n,
        hash: hash(n),
        timestamp: n,
    }
}
fn topics(protocol: Protocol) -> Vec<String> {
    match protocol {
        Protocol::Autonomous(_) => autonomous_bounty_event_topics(),
        Protocol::Competition => open_competition_event_topics(),
        Protocol::Beta3 => open_competition_v2_event_topics(),
    }
}
fn creation(protocol: Protocol, n: usize, at: u64) -> Value {
    let words = if matches!(protocol, Protocol::Beta3) {
        2
    } else {
        3
    };
    json!({"address": FACTORY,"topics":[topics(protocol)[0],hash(n as u64),format!("0x{n:064x}"),hash(999)],
        "data":format!("0x{}","00".repeat(32*words)),"transactionHash":hash(n as u64),
        "blockNumber":format!("0x{at:x}"),"logIndex":format!("0x{n:x}")})
}
fn creation_bundle(protocol: Protocol, n: usize, at: u64) -> Vec<Value> {
    let mut created = creation(protocol, n, at);
    if !matches!(protocol, Protocol::Autonomous(_)) {
        return vec![created];
    }
    created["logIndex"] = json!(format!("0x{:x}", n * 4));
    let mut bundle = vec![created.clone()];
    for (offset, words) in [
        (1, vec![7_u64, 8, 9]),
        (2, vec![10, 1, 11, 0, 2_000_000_000, 3600, 1800]),
        (3, vec![2, 444, 555, 2, 10]),
    ] {
        let mut log = created.clone();
        log["topics"] = json!([topics(protocol)[offset], hash(n as u64)]);
        log["data"] = json!(format!(
            "0x{}",
            words
                .iter()
                .map(|word| format!("{word:064x}"))
                .collect::<String>()
        ));
        log["logIndex"] = json!(format!("0x{:x}", n * 4 + offset));
        bundle.push(log);
    }
    bundle
}
fn funding(protocol: Protocol, emitter: &str) -> Value {
    use sha3::Digest;
    let signature = if matches!(protocol, Protocol::Beta3) {
        "FundingAddedV2(bytes32,address,uint256,uint256,uint256)"
    } else {
        "FundingAdded(bytes32,address,uint256,uint256,uint256)"
    };
    json!({"address":emitter,"topics":[format!("0x{}",hex::encode(sha3::Keccak256::digest(signature.as_bytes()))),hash(1),hash(999)],
        "data":format!("0x{:064x}{:064x}{:064x}",1,1,1),"transactionHash":hash(10000),"blockNumber":"0x65","logIndex":"0x0"})
}
fn evm(raw: Value) -> Vec<chain_base::EvmLog> {
    rpc_logs_to_evm_logs([serde_json::from_value::<chain_base::RpcEvmLog>(raw).unwrap()]).unwrap()
}
struct Rpc {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Rpc {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn rpc(protocol: Protocol, count: usize, scenario: Scenario) -> Rpc {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    let task = tokio::spawn(async move {
        let mut batch_index = 0;
        while let Ok((socket, _)) = listener.accept().await {
            let mut reader = BufReader::new(socket);
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    return;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some((key, value)) = line.split_once(':') {
                    if key.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).await.unwrap();
            let q: Value = serde_json::from_slice(&bytes).unwrap();
            seen.lock().unwrap().push(q.clone());
            let mut status = 200;
            let result = match q["method"].as_str().unwrap() {
                "eth_blockNumber" => json!("0x65"),
                "eth_getBlockByNumber" => {
                    let tag = q["params"][0].as_str().unwrap();
                    let n = if tag == "safe" {
                        101
                    } else {
                        u64::from_str_radix(&tag[2..], 16).unwrap()
                    };
                    json!({"number":format!("0x{n:x}"),"hash":hash(n),"timestamp":format!("0x{n:x}")})
                }
                "eth_getLogs" => {
                    let f = &q["params"][0];
                    assert_eq!(f["fromBlock"], "0x65");
                    assert_eq!(f["toBlock"], "0x65");
                    assert_eq!(f["topics"], json!([topics(protocol)]));
                    if f["address"].is_string() {
                        assert_eq!(f["address"], FACTORY);
                        if count == 0 {
                            json!([])
                        } else {
                            json!(creation_bundle(protocol, count + 1, 101))
                        }
                    } else {
                        batch_index += 1;
                        let addresses = f["address"].as_array().unwrap();
                        if addresses.len() > 10
                            || matches!(scenario, Scenario::RefusedMiddle) && batch_index == 2
                        {
                            status = 403;
                            json!([])
                        } else if matches!(scenario, Scenario::InvalidLast)
                            && batch_index == (count + 1).div_ceil(10)
                        {
                            json!([funding(protocol, &address(9999))])
                        } else if batch_index == 1 {
                            json!([funding(protocol, &address(1))])
                        } else {
                            json!([])
                        }
                    }
                }
                other => panic!("unexpected method {other}"),
            };
            let response=if status==200{json!({"jsonrpc":"2.0","id":q["id"],"result":result})}else{json!({"jsonrpc":"2.0","id":q["id"],"error":{"code":-32602,"message":"maximum10addresses"}})}.to_string();
            reader.get_mut().write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        }
    });
    Rpc {
        url,
        requests,
        task,
    }
}
struct Fixture {
    store: PostgresStore,
    pool: PgPool,
    admin: PgPool,
    schema: String,
}
impl Fixture {
    async fn new(protocol: Protocol, count: usize) -> Self {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").expect("isolated database");
        db::test_support::assert_isolated_security_database(&url);
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("log_batches_{}", uuid::Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = url
            .parse::<sqlx::postgres::PgConnectOptions>()
            .unwrap()
            .options([("search_path", schema.as_str())]);
        let pool = PgPool::connect_with(options).await.unwrap();
        let store = PostgresStore::from_pool(pool.clone());
        store.migrate().await.unwrap();
        if matches!(protocol, Protocol::Autonomous(_)) {
            store
                .commit_autonomous_canonical_checkpoint(
                    NETWORK,
                    FACTORY,
                    None,
                    None,
                    &block(100),
                    None,
                )
                .await
                .unwrap();
        } else {
            store
                .upsert_base_log_cursor(NETWORK, FACTORY, 100, None)
                .await
                .unwrap();
        }
        for n in 1..=count {
            let raw = creation(protocol, n, 100);
            match protocol {
                Protocol::Autonomous(_) => {
                    for mut event in decode_autonomous_bounty_logs(
                        creation_bundle(protocol, n, 100)
                            .into_iter()
                            .flat_map(evm)
                            .collect::<Vec<_>>(),
                    )
                    .unwrap()
                    {
                        event.occurred_at = DateTime::from_timestamp(100, 0).unwrap();
                        store
                            .upsert_autonomous_bounty_event(NETWORK, &event)
                            .await
                            .unwrap();
                    }
                }
                Protocol::Competition => {
                    let mut event = decode_open_competition_logs(evm(raw)).unwrap().remove(0);
                    event.occurred_at = DateTime::from_timestamp(100, 0).unwrap();
                    store
                        .upsert_open_competition_event(NETWORK, FACTORY, &event)
                        .await
                        .unwrap();
                }
                Protocol::Beta3 => {
                    let mut event = decode_open_competition_v2_logs(evm(raw)).unwrap().remove(0);
                    event.occurred_at = DateTime::from_timestamp(100, 0).unwrap();
                    store
                        .upsert_open_competition_v2_event(
                            NETWORK,
                            FACTORY,
                            &event,
                            &OpenCompetitionV2SafeContext {
                                block_hash: hash(100),
                                safe_block_number: 100,
                                safe_block_hash: hash(100),
                            },
                        )
                        .await
                        .unwrap();
                }
            }
        }
        match protocol {
            Protocol::Autonomous(_) => {
                store
                    .confirm_autonomous_event_block_time(
                        NETWORK,
                        100,
                        DateTime::from_timestamp(100, 0).unwrap(),
                    )
                    .await
                    .unwrap();
            }
            Protocol::Competition => {
                store
                    .confirm_open_competition_event_block_time(
                        NETWORK,
                        FACTORY,
                        100,
                        DateTime::from_timestamp(100, 0).unwrap(),
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        store
            .create_discovery_webhook_subscription(&db::NewDiscoveryWebhookSubscription {
                id: uuid::Uuid::new_v4(),
                endpoint_url: "https://subscriber.invalid/discovery".into(),
                filters: domain::DiscoverySubscriptionFilters::default(),
                management_token_hash: "synthetic-only".into(),
            })
            .await
            .unwrap();
        Self {
            store,
            pool,
            admin,
            schema,
        }
    }
    async fn snapshot(&self) -> Value {
        let mut states = serde_json::Map::new();
        for table in [
            "autonomous_bounty_events",
            "open_competition_events",
            "open_competition_v2_events",
            "open_competition_v2_projections",
            "open_competition_v2_indexer_agreements",
            "base_log_cursors",
            "autonomous_canonical_checkpoints",
            "webhook_deliveries",
        ] {
            let value:Value=sqlx::query_scalar(&format!("SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text),'[]'::jsonb) FROM {table} t")).fetch_one(&self.pool).await.unwrap();
            states.insert(table.into(), value);
        }
        Value::Object(states)
    }
    async fn close(self) {
        drop(self.store);
        self.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
async fn run_case(protocol: Protocol, count: usize, scenario: Scenario) {
    let f = Fixture::new(protocol, count).await;
    let before = f.snapshot().await;
    let rpc = rpc(protocol, count, scenario).await;
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        match protocol {
            Protocol::Autonomous(protocol) => {
                let c = AutonomousIndexerConfig::from_lookup_for_protocol(
                    |key| match key {
                        "BASE_INDEXER_NETWORK" => Some(NETWORK.into()),
                        "BASE_INDEXER_FACTORY_CONTRACT" => Some(FACTORY.into()),
                        "BASE_INDEXER_RPC_URL" => Some(rpc.url.clone()),
                        "BASE_INDEXER_CONFIRMATIONS" => Some("0".into()),
                        "BASE_INDEXER_MAX_BLOCKS_PER_QUERY" => Some("1".into()),
                        _ => None,
                    },
                    protocol,
                )
                .unwrap();
                poll_autonomous_indexer_once(&f.store, &c)
                    .await
                    .map(|r| r.persisted_cursor_block)
            }
            Protocol::Competition => poll_open_competition_indexer_once(
                &f.store,
                &OpenCompetitionIndexerConfig {
                    network: NETWORK.into(),
                    rpc_url: rpc.url.clone(),
                    factory_contract: FACTORY.into(),
                    deployment_block: 1,
                    poll_seconds: 15,
                    confirmations: 0,
                    max_blocks_per_query: 1,
                    request_id: 10000,
                },
            )
            .await
            .map(|r| r.persisted_cursor_block),
            Protocol::Beta3 => poll_open_competition_v2_indexer_once(
                &f.store,
                &OpenCompetitionV2IndexerConfig {
                    network: NETWORK.into(),
                    rpc_url: rpc.url.clone(),
                    factory_contract: FACTORY.into(),
                    deployment_block: 1,
                    poll_seconds: 15,
                    confirmations: 0,
                    max_blocks_per_query: 1,
                    request_id: 20000,
                },
            )
            .await
            .map(|r| r.persisted_cursor_block),
        }
    })
    .await
    .expect("bounded loopback poll");
    let requests = rpc.requests.lock().unwrap().clone();
    let batches = requests
        .iter()
        .filter(|q| q["method"] == "eth_getLogs" && q["params"][0]["address"].is_array())
        .collect::<Vec<_>>();
    assert!(
        batches
            .iter()
            .all(|q| q["params"][0]["address"].as_array().unwrap().len() <= 10),
        "provider bound applies to every actual request"
    );
    match scenario {
        Scenario::Success => {
            assert_eq!(result.unwrap(), Some(101));
            let total = if count == 0 { 0 } else { count + 1 };
            let actual = batches
                .iter()
                .flat_map(|q| {
                    q["params"][0]["address"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a.as_str().unwrap().to_owned())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                (1..=total).map(address).collect::<Vec<_>>(),
                "every existing plus same-range new contract exactly once"
            );
            assert_eq!(batches.len(), total.div_ceil(10));
            let table = match protocol {
                Protocol::Autonomous(_) => "autonomous_bounty_events",
                Protocol::Competition => "open_competition_events",
                Protocol::Beta3 => "open_competition_v2_events",
            };
            let after = f.snapshot().await;
            assert_eq!(
                after[table].as_array().unwrap().len(),
                if count == 0 {
                    0
                } else if matches!(protocol, Protocol::Autonomous(_)) {
                    4 * (count + 1) + 1
                } else {
                    count + 2
                }
            );
            if count > 0 && matches!(protocol, Protocol::Autonomous(_)) {
                assert!(
                    !after["webhook_deliveries"].as_array().unwrap().is_empty(),
                    "successful poll enqueues real fixture discovery notifications"
                );
            }
            assert_eq!(
                f.store
                    .get_base_log_cursor(NETWORK, FACTORY)
                    .await
                    .unwrap()
                    .unwrap()
                    .last_scanned_block,
                101
            );
        }
        Scenario::RefusedMiddle | Scenario::InvalidLast => {
            let error = result.unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains(if matches!(scenario, Scenario::RefusedMiddle) {
                        "403"
                    } else {
                        "invalid emitter"
                    }),
                "{error}"
            );
            assert_eq!(
                batches.len(),
                if matches!(scenario, Scenario::RefusedMiddle) {
                    2
                } else {
                    (count + 1).div_ceil(10)
                }
            );
            assert_eq!(f.snapshot().await,before,"late partition failure cannot persist any new-range event, projection, checkpoint, cursor or notification");
            assert_eq!(
                requests.last().unwrap()["method"],
                "eth_getLogs",
                "no timestamp, final-header or later batch request follows failure"
            );
        }
    }
    drop(rpc);
    f.close().await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn autonomous_boundaries_cover_complete_union() {
    for protocol in ["autonomous-v1", "autonomous-v2"] {
        for n in [0, 9, 10, 11, 73] {
            run_case(Protocol::Autonomous(protocol), n, Scenario::Success).await;
        }
    }
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn autonomous_middle_refusal_preserves_range() {
    run_case(
        Protocol::Autonomous("autonomous-v1"),
        21,
        Scenario::RefusedMiddle,
    )
    .await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn autonomous_last_invalid_emitter_preserves_range() {
    run_case(
        Protocol::Autonomous("autonomous-v2"),
        21,
        Scenario::InvalidLast,
    )
    .await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn competition_boundaries_cover_complete_union() {
    for n in [0, 9, 10, 11, 73] {
        run_case(Protocol::Competition, n, Scenario::Success).await;
    }
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn competition_middle_refusal_preserves_range() {
    run_case(Protocol::Competition, 21, Scenario::RefusedMiddle).await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn competition_last_invalid_emitter_preserves_range() {
    run_case(Protocol::Competition, 21, Scenario::InvalidLast).await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn beta3_boundaries_cover_complete_union() {
    for n in [0, 9, 10, 11, 73] {
        run_case(Protocol::Beta3, n, Scenario::Success).await;
    }
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn beta3_middle_refusal_preserves_range() {
    run_case(Protocol::Beta3, 21, Scenario::RefusedMiddle).await;
}
#[tokio::test]
#[ignore = "requires isolated local PostgreSQL"]
async fn beta3_last_invalid_emitter_preserves_range() {
    run_case(Protocol::Beta3, 21, Scenario::InvalidLast).await;
}
