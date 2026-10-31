use super::*;
use managed_orders::{FiatCurrency, PriceBasis, QuoteInput, ServiceTerms};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Duration;

pub(super) struct Fixture {
    pub(super) store: PostgresStore,
    admin: PgPool,
    url: String,
    schema: String,
    /// Quotes carry the refundable review deposit line every card quote has.
    deposit: bool,
}

async fn scoped_pool(url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .unwrap()
}

impl Fixture {
    pub(super) async fn new() -> Self {
        let url = std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL")
            .expect("isolated test database required");
        crate::test_support::assert_isolated_security_database(&url);
        let admin = PgPool::connect(&url).await.unwrap();
        let schema = format!("managed_orders_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .unwrap();
        let store = PostgresStore::from_pool(scoped_pool(&url, &schema).await);
        store.migrate().await.unwrap();
        Self {
            store,
            admin,
            url,
            schema,
            deposit: false,
        }
    }
    /// A fixture whose quotes carry a review deposit line of 20,000 minor units.
    pub(super) async fn with_deposit() -> Self {
        Self {
            deposit: true,
            ..Self::new().await
        }
    }
    pub(super) async fn reconnect(&self) -> PostgresStore {
        PostgresStore::from_pool(scoped_pool(&self.url, &self.schema).await)
    }
    pub(super) async fn cleanup(self) {
        self.store.pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
    pub(super) async fn now(&self) -> i64 {
        sqlx::query_scalar("SELECT floor(extract(epoch FROM clock_timestamp()))::bigint")
            .fetch_one(&self.store.pool)
            .await
            .unwrap()
    }
    pub(super) async fn quote(&self) -> ManagedQuote {
        let now = self.now().await;
        ManagedQuote::new(QuoteInput {
            quote_id: Uuid::new_v4(),
            buyer_principal: "fixture-buyer".into(),
            stripe_account_id: "acct_fixture_only".into(),
            livemode: false,
            issued_at: now - 1,
            expires_at: now + 600,
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
                minor_units_per_usdc_numerator: 2_000,
                minor_units_per_usdc_denominator: 1,
                valid_until: now + 600,
            },
            solver_reward_micro_usdc: 100_000_000,
            verifier_reserve_micro_usdc: 5_000_000,
            additional_costs: if self.deposit {
                vec![managed_orders::AdditionalCost {
                    code: managed_orders::pricing::REVIEW_DEPOSIT_CODE.into(),
                    label: "Review deposit (refundable, 10% of solver reward)".into(),
                    amount_minor: 20_000,
                }]
            } else {
                vec![]
            },
            // Each fixture order commits to its own bounty through a fresh creation nonce.
            bounty: managed_orders_bounty(
                now + 86_400,
                &format!("0x{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            ),
        })
        .unwrap()
    }
    async fn count(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "managed_service_orders" | "managed_service_actions"
        ));
        sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&self.store.pool)
            .await
            .unwrap()
    }
}

pub(super) async fn accept(
    store: &PostgresStore,
    quote: &ManagedQuote,
    key: &str,
) -> Result<ManagedOrder> {
    store
        .accept_managed_service_quote(
            &quote.input().buyer_principal,
            quote.input().quote_id,
            &quote.digest().unwrap(),
            key,
        )
        .await
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn immutable_quotes_orders_and_held_actions_survive_reconnect_and_migration_replay() {
    let f = Fixture::new().await;
    let quote = f.quote().await;
    f.store.put_managed_service_quote(&quote).await.unwrap();
    f.store.put_managed_service_quote(&quote).await.unwrap();
    let mut altered = quote.input().clone();
    altered.terms.remedies.push_str(" changed");
    assert!(matches!(
        f.store
            .put_managed_service_quote(&ManagedQuote::new(altered).unwrap())
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    let order = accept(&f.store, &quote, "retry-1").await.unwrap();
    assert_eq!(order.status, ManagedOrderStatus::AwaitingPayment);
    let restarted = f.reconnect().await;
    restarted.migrate().await.unwrap();
    assert_eq!(accept(&restarted, &quote, "retry-1").await.unwrap(), order);
    assert_eq!(
        restarted
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await
            .unwrap(),
        Some(order)
    );
    assert_eq!(f.count("managed_service_orders").await, 1);
    assert_eq!(f.count("managed_service_actions").await, 1);
    let mut independent = quote.input().clone();
    independent.quote_id = Uuid::new_v4();
    let independent = ManagedQuote::new(independent).unwrap();
    f.store
        .put_managed_service_quote(&independent)
        .await
        .unwrap();
    accept(&restarted, &independent, "retry-2").await.unwrap();
    assert_eq!(f.count("managed_service_orders").await, 2);
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn wrong_buyer_altered_digest_future_and_expired_acceptance_leave_no_order() {
    let f = Fixture::new().await;
    let quote = f.quote().await;
    f.store.put_managed_service_quote(&quote).await.unwrap();
    let id = quote.input().quote_id;
    assert!(matches!(
        f.store
            .accept_managed_service_quote("wrong-buyer", id, &quote.digest().unwrap(), "k")
            .await,
        Err(ManagedOrderStoreError::NotFound)
    ));
    assert!(f
        .store
        .get_managed_service_quote("wrong-buyer", id)
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        f.store
            .accept_managed_service_quote("fixture-buyer", id, &"0".repeat(64), "k")
            .await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert!(accept(&f.store, &quote, "invalid key").await.is_err());
    for delta in [-1_000, 1_000] {
        let mut changed = quote.input().clone();
        changed.quote_id = Uuid::new_v4();
        changed.issued_at += delta;
        changed.expires_at += delta;
        changed.price_basis.valid_until += delta;
        let changed = ManagedQuote::new(changed).unwrap();
        f.store.put_managed_service_quote(&changed).await.unwrap();
        assert!(matches!(
            accept(&f.store, &changed, "k").await,
            Err(ManagedOrderStoreError::Quote(OrderError::Expired))
        ));
    }
    assert_eq!(f.count("managed_service_orders").await, 0);
    assert_eq!(f.count("managed_service_actions").await, 0);
    let order = accept(&f.store, &quote, "good").await.unwrap();
    assert!(f
        .store
        .get_managed_service_order("wrong-buyer", order.order_id)
        .await
        .unwrap()
        .is_none());
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn concurrent_acceptance_from_independent_pools_creates_one_order_and_action() {
    let f = Fixture::new().await;
    let quote = f.quote().await;
    f.store.put_managed_service_quote(&quote).await.unwrap();
    let second = f.reconnect().await;
    let mut tasks = vec![];
    for i in 0..16 {
        let store = if i % 2 == 0 {
            f.store.clone()
        } else {
            second.clone()
        };
        let quote = quote.clone();
        tasks.push(tokio::spawn(async move {
            accept(&store, &quote, "one-retry-key").await.unwrap()
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for task in tasks {
        ids.insert(task.await.unwrap().order_id);
    }
    assert_eq!(ids.len(), 1);
    assert_eq!(f.count("managed_service_orders").await, 1);
    assert_eq!(f.count("managed_service_actions").await, 1);
    second.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn a_retry_key_cannot_accept_two_quotes_or_replace_a_quotes_first_key() {
    let f = Fixture::new().await;
    let first = f.quote().await;
    let second = f.quote().await;
    f.store.put_managed_service_quote(&first).await.unwrap();
    f.store.put_managed_service_quote(&second).await.unwrap();
    let other = f.reconnect().await;
    let (a, b) = tokio::join!(
        accept(&f.store, &first, "same-key"),
        accept(&other, &second, "same-key")
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let (accepted, rejected) = if a.is_ok() {
        (&first, &second)
    } else {
        (&second, &first)
    };
    assert!(matches!(
        accept(&other, accepted, "new-key").await,
        Err(ManagedOrderStoreError::Quote(OrderError::Conflict))
    ));
    assert_eq!(f.count("managed_service_orders").await, 1);
    assert_eq!(f.count("managed_service_actions").await, 1);
    accept(&other, rejected, "new-key").await.unwrap();
    assert_eq!(f.count("managed_service_orders").await, 2);
    other.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn failure_between_order_and_action_rolls_back_and_retries_after_reconnect() {
    let f = Fixture::new().await;
    let quote = f.quote().await;
    f.store.put_managed_service_quote(&quote).await.unwrap();
    // This schema is private to this test. Force the SECOND durable write to fail.
    sqlx::query(
        "ALTER TABLE managed_service_actions ADD CONSTRAINT injected_failure CHECK (false)",
    )
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(matches!(
        accept(&f.store, &quote, "retry-key").await,
        Err(ManagedOrderStoreError::Database(_))
    ));
    assert_eq!(f.count("managed_service_orders").await, 0);
    assert_eq!(f.count("managed_service_actions").await, 0);
    sqlx::query("ALTER TABLE managed_service_actions DROP CONSTRAINT injected_failure")
        .execute(&f.store.pool)
        .await
        .unwrap();
    let restarted = f.reconnect().await;
    accept(&restarted, &quote, "retry-key").await.unwrap();
    assert_eq!(f.count("managed_service_orders").await, 1);
    assert_eq!(f.count("managed_service_actions").await, 1);
    restarted.pool.close().await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn corrupted_stored_amounts_indexes_and_missing_actions_fail_closed() {
    let f = Fixture::new().await;
    let quote = f.quote().await;
    f.store.put_managed_service_quote(&quote).await.unwrap();
    let original = serde_json::to_value(&quote).unwrap();
    sqlx::query("UPDATE managed_service_quotes SET quote=jsonb_set(quote,'{total_minor}','1') WHERE quote_id=$1")
        .bind(quote.input().quote_id).execute(&f.store.pool).await.unwrap();
    assert!(matches!(
        accept(&f.store, &quote, "k").await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    sqlx::query(
        "UPDATE managed_service_quotes SET quote=$1, expires_at=expires_at+1 WHERE quote_id=$2",
    )
    .bind(original)
    .bind(quote.input().quote_id)
    .execute(&f.store.pool)
    .await
    .unwrap();
    assert!(matches!(
        f.store
            .get_managed_service_quote("fixture-buyer", quote.input().quote_id)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    sqlx::query("UPDATE managed_service_quotes SET expires_at=expires_at-1 WHERE quote_id=$1")
        .bind(quote.input().quote_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    let order = accept(&f.store, &quote, "k").await.unwrap();
    sqlx::query("DELETE FROM managed_service_actions WHERE order_id=$1")
        .bind(order.order_id)
        .execute(&f.store.pool)
        .await
        .unwrap();
    assert!(matches!(
        f.store
            .get_managed_service_order("fixture-buyer", order.order_id)
            .await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    assert!(matches!(
        accept(&f.store, &quote, "k").await,
        Err(ManagedOrderStoreError::Integrity)
    ));
    assert_eq!(f.count("managed_service_orders").await, 1);
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires isolated AGENT_BOUNTIES_TEST_DATABASE_URL"]
async fn expiry_is_checked_after_lock_wait_but_a_committed_retry_survives_expiry() {
    let f = Fixture::new().await;
    let mut input = f.quote().await.input().clone();
    input.expires_at = f.now().await + 2;
    let quote = ManagedQuote::new(input.clone()).unwrap();
    input.quote_id = Uuid::new_v4();
    let replay = ManagedQuote::new(input).unwrap();
    f.store.put_managed_service_quote(&quote).await.unwrap();
    f.store.put_managed_service_quote(&replay).await.unwrap();
    let accepted = accept(&f.store, &replay, "already-accepted").await.unwrap();
    let mut lock = f.store.pool.begin().await.unwrap();
    sqlx::query("SELECT quote_id FROM managed_service_quotes WHERE quote_id=$1 FOR UPDATE")
        .bind(quote.input().quote_id)
        .fetch_one(&mut *lock)
        .await
        .unwrap();
    let other = f.reconnect().await;
    let task = {
        let store = other.clone();
        let quote = quote.clone();
        tokio::spawn(async move { accept(&store, &quote, "blocked-until-expired").await })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished());
    while f.now().await < quote.input().expires_at {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    lock.commit().await.unwrap();
    assert!(matches!(
        task.await.unwrap(),
        Err(ManagedOrderStoreError::Quote(OrderError::Expired))
    ));
    assert_eq!(
        accept(&other, &replay, "already-accepted").await.unwrap(),
        accepted
    );
    assert_eq!(f.count("managed_service_orders").await, 1);
    other.pool.close().await;
    f.cleanup().await;
}

pub(super) const FIXTURE_FACTORY: &str = "0x44de6074440fc006c20087991364ce65c87e6d88";
const FIXTURE_IMPLEMENTATION: &str = "0x9482786192e16ca4eb55f61787af664acd6b8e53";
pub(super) const FIXTURE_WALLET: &str = "0xf0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";

pub(super) fn fixture_fee() -> chain_base::AutonomousV2FactoryFee {
    chain_base::AutonomousV2FactoryFee {
        platform_fee_bps: 750,
        platform_fee_recipient: "0x884834e884d6e93462655a2820140ad03e6747bc".into(),
    }
}

pub(super) fn fixture_target() -> managed_orders::funding::FundingTarget {
    managed_orders::funding::FundingTarget {
        network: "base-sepolia".into(),
        factory: FIXTURE_FACTORY.into(),
        funding_wallet: FIXTURE_WALLET.into(),
        fee: fixture_fee(),
    }
}

/// The public terms the fixture quote commits to. The hash covers only the document, so the
/// same deadline and nonce always rebuild the same record.
pub(super) fn fixture_terms(
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
            "protocol_version": chain_base::AUTONOMOUS_V2_PROTOCOL_VERSION, "network": "base-sepolia",
            "settlement_token": chain_base::base_network_descriptor("base-sepolia").unwrap().native_usdc_token_address,
            "creator_wallet": FIXTURE_WALLET, "solver_reward": usdc(100_000_000),
            "verifier_reward": usdc(5_000_000), "claim_bond": usdc(5_000_000),
            "initial_funding": usdc(112_500_000), "platform_fee_bps": 750,
            "platform_fee": usdc(7_500_000), "platform_fee_recipient": fixture_fee().platform_fee_recipient,
            "funding_deadline": funding_deadline, "claim_window_seconds": 604_800,
            "verification_window_seconds": 259_200, "creation_nonce": creation_nonce
        }
    });
    chain_base::build_autonomous_bounty_terms_record(
        FIXTURE_WALLET,
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
    let plan = chain_base::AutonomousBountyTxPlanner::new(FIXTURE_FACTORY, FIXTURE_IMPLEMENTATION)
        .unwrap()
        .plan_v2_creation("base-sepolia", &create, &fixture_fee())
        .unwrap();
    let calldata = chain_base::encode_bounded_v2_funding_wallet_create_call(&create).unwrap();
    managed_orders::BountyCommitment {
        network: "base-sepolia".into(),
        factory: FIXTURE_FACTORY.into(),
        funding_wallet: FIXTURE_WALLET.into(),
        creation_nonce: create.base.creation_nonce.to_ascii_lowercase(),
        terms_hash: terms.terms_hash.to_ascii_lowercase(),
        verification_setup: "customer_review_v1".into(),
        funding_deadline,
        predicted_bounty: plan.plan.predicted_bounty_contract.to_ascii_lowercase(),
        create_call_keccak256: managed_orders::funding::calldata_keccak256(&calldata).unwrap(),
    }
}
