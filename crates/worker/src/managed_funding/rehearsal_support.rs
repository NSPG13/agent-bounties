//! Loopback Anvil + disposable Postgres fixtures for card-bounty rehearsals: the real v2
//! factory, bounded funding wallet, signed Stripe webhook path and indexer. Compiled only for
//! tests or the `rehearsal-support` feature; every call asserts a loopback Anvil chain.
#![allow(clippy::missing_panics_doc)]
use super::{ManagedFundingConfig, ManagedFundingSettings};
use crate::AutonomousIndexerConfig;
use alloy::sol_types::SolValue;
use chain_base::{
    autonomous_v2_create_from_terms, build_autonomous_bounty_terms_record,
    encode_bounded_v2_funding_wallet_create_call, AutonomousBountyTxPlanner,
    AutonomousV2FactoryFee, BaseTransactionRelayer, AUTONOMOUS_V2_PROTOCOL_VERSION,
    BASE_SEPOLIA_USDC_TOKEN_ADDRESS, TEST_CUSTOMER_REVIEW_VERIFIER,
};
use db::PostgresStore;
use hmac::{Hmac, Mac};
use managed_orders::{
    funding::FundingTarget,
    stripe_checkout::{verify_managed_checkout_event, CheckoutPolicy},
    BountyCommitment, FiatCurrency, ManagedQuote, PriceBasis, QuoteInput, ServiceTerms,
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

alloy::sol! {
    struct Policy {
        address delegate; uint64 validAfter; uint64 validUntil; uint64 periodSeconds;
        uint64 maxFundingWindowSeconds; uint64 maxClaimWindowSeconds;
        uint64 maxVerificationWindowSeconds; bool delegateMayCancel; uint256 maxPerBounty;
        uint256 maxPerPeriod; uint256 maxLifetimeSpend; address claimEligibilityRegistry;
        bytes32 claimEligibilitySource;
    }
    struct VerificationConfig {
        uint8 mode; address verifierModule; address verifierRewardRecipient;
        bytes32 verifierSetHash; uint8 threshold;
    }
}

pub const FEE_RECIPIENT: &str = "0x884834e884d6e93462655a2820140ad03e6747bc";
pub const SOLVER_REWARD: u64 = 10_000_000;
pub const VERIFIER_REWARD: u64 = 500_000;
pub const TARGET: u64 = 11_250_000;

pub async fn rpc(url: &str, method: &str, params: Value) -> Value {
    let result: Value = reqwest::Client::new()
        .post(url)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(result.get("error").is_none(), "{method}: {result}");
    result["result"].clone()
}

fn artifact(source: &str, name: &str, kind: &str) -> String {
    let path = format!(
        "{}/../../contracts/base-escrow/out/{source}.sol/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let json: Value =
        serde_json::from_slice(&std::fs::read(path).expect("run forge build first")).unwrap();
    json[kind]["object"].as_str().unwrap().to_owned()
}

/// Send from an unlocked or impersonated account and require success.
pub async fn send(url: &str, from: &str, to: Option<&str>, data: String) -> Value {
    let mut tx = json!({"from":from,"data":data,"gas":"0x1c9c380",
        "maxFeePerGas":"0x3b9aca00","maxPriorityFeePerGas":"0x1"});
    if let Some(to) = to {
        tx["to"] = to.into();
    }
    let hash = rpc(url, "eth_sendTransaction", json!([tx])).await;
    let receipt = rpc(url, "eth_getTransactionReceipt", json!([hash])).await;
    assert_eq!(receipt["status"], "0x1", "{receipt}");
    receipt
}

pub fn call_data(signature: &str, args: &[u8]) -> String {
    format!(
        "0x{}{}",
        hex::encode(&alloy::primitives::keccak256(signature.as_bytes())[..4]),
        hex::encode(args)
    )
}

pub fn address(value: &str) -> alloy::primitives::Address {
    value.parse().unwrap()
}

pub async fn balance(url: &str, token: &str, owner: &str) -> u128 {
    let word = rpc(
        url,
        "eth_call",
        json!([{"to":token,"data":call_data("balanceOf(address)", &address(owner).abi_encode())},"latest"]),
    )
    .await;
    u128::from_str_radix(word.as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
}

/// Requires the isolated local test database and creates a fresh schema in it.
pub async fn isolated_store(database: &str) -> PostgresStore {
    db::test_support::assert_isolated_security_database(database);
    let schema = format!("managed_rehearsal_{}", Uuid::new_v4().simple());
    let admin = sqlx::PgPool::connect(database).await.unwrap();
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(move |connection, _| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(database)
        .await
        .unwrap();
    let store = PostgresStore::from_pool(pool);
    store.migrate().await.unwrap();
    store
}

/// One loopback Base Sepolia stand-in with the real contracts deployed.
pub struct Chain {
    pub url: String,
    pub owner: String,
    pub token: String,
    pub factory: String,
    pub implementation: String,
    pub wallet: String,
    pub fee: AutonomousV2FactoryFee,
    pub delegate: BaseTransactionRelayer,
}

/// Deploy the v2 factory and a bounded wallet whose policy allows only the customer-review
/// signer set, fund the wallet, and point the v2 settings at them.
pub async fn deploy(url: &str, store: &PostgresStore, float: u64) -> Chain {
    assert!(url.starts_with("http://127.0.0.1:"));
    assert_eq!(rpc(url, "eth_chainId", json!([])).await, "0x14a34");
    assert!(rpc(url, "web3_clientVersion", json!([]))
        .await
        .as_str()
        .unwrap()
        .to_ascii_lowercase()
        .contains("anvil"));
    let owner = rpc(url, "eth_accounts", json!([])).await[0]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    // Throwaway key; every transaction is guarded to loopback Anvil above.
    let delegate =
        BaseTransactionRelayer::from_private_key(&format!("{:064x}", Uuid::new_v4().as_u128()))
            .unwrap()
            .with_gas_budget(store.gas_sponsor_budget());
    rpc(
        url,
        "anvil_setBalance",
        json!([delegate.address(), "0x8ac7230489e80000"]),
    )
    .await;
    let token = BASE_SEPOLIA_USDC_TOKEN_ADDRESS.to_ascii_lowercase();
    for (target, name) in [
        (token.as_str(), "SponsoredCreationToken"),
        (
            "0x420000000000000000000000000000000000000f",
            "SponsoredCreationFeeOracle",
        ),
    ] {
        rpc(
            url,
            "anvil_setCode",
            json!([
                target,
                artifact("SponsoredCreationToken", name, "deployedBytecode")
            ]),
        )
        .await;
    }
    let receipt = send(
        url,
        &owner,
        None,
        format!(
            "{}{}",
            artifact("AgentBountyFactoryV2", "AgentBountyFactoryV2", "bytecode"),
            hex::encode((address(&token), 750u16, address(FEE_RECIPIENT)).abi_encode_params())
        ),
    )
    .await;
    let factory = receipt["contractAddress"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    let implementation = rpc(
        url,
        "eth_call",
        json!([{"to":factory,"data":call_data("implementation()", &[])},"latest"]),
    )
    .await;
    let implementation = format!("0x{}", &implementation.as_str().unwrap()[26..]);
    for (key, value) in [
        ("BASE_SEPOLIA_BOUNTY_V2_FACTORY", factory.as_str()),
        (
            "BASE_SEPOLIA_BOUNTY_V2_IMPLEMENTATION",
            implementation.as_str(),
        ),
        ("BASE_SEPOLIA_BOUNTY_V2_PLATFORM_FEE_BPS", "750"),
        (
            "BASE_SEPOLIA_BOUNTY_V2_PLATFORM_FEE_RECIPIENT",
            FEE_RECIPIENT,
        ),
    ] {
        std::env::set_var(key, value);
    }
    let fee = service_runtime::autonomous_v2_deployment_for_chain(84_532)
        .unwrap()
        .unwrap()
        .fee;
    let now = chrono::Utc::now().timestamp();
    let policy = Policy {
        delegate: address(&delegate.address()),
        validAfter: 0,
        validUntil: (now + 365 * 86_400) as u64,
        periodSeconds: 86_400,
        maxFundingWindowSeconds: 30 * 86_400,
        maxClaimWindowSeconds: 604_800,
        maxVerificationWindowSeconds: 604_800,
        delegateMayCancel: false,
        maxPerBounty: alloy::primitives::U256::from(20_000_000u64),
        maxPerPeriod: alloy::primitives::U256::from(40_000_000u64),
        maxLifetimeSpend: alloy::primitives::U256::from(100_000_000u64),
        claimEligibilityRegistry: alloy::primitives::Address::ZERO,
        claimEligibilitySource: alloy::primitives::B256::ZERO,
    };
    let config = VerificationConfig {
        mode: 1,
        verifierModule: alloy::primitives::Address::ZERO,
        verifierRewardRecipient: alloy::primitives::Address::ZERO,
        verifierSetHash: alloy::primitives::keccak256(
            vec![address(TEST_CUSTOMER_REVIEW_VERIFIER)].abi_encode(),
        ),
        threshold: 1,
    };
    let receipt = send(
        url,
        &owner,
        None,
        format!(
            "{}{}",
            artifact(
                "BoundedV2FundingWallet",
                "BoundedV2FundingWallet",
                "bytecode"
            ),
            hex::encode(
                (address(&factory), address(&owner), policy, vec![config]).abi_encode_params()
            )
        ),
    )
    .await;
    let wallet = receipt["contractAddress"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    mint(url, &owner, &token, &wallet, float).await;
    Chain {
        url: url.to_owned(),
        owner,
        token,
        factory,
        implementation,
        wallet,
        fee,
        delegate,
    }
}

pub async fn mint(url: &str, from: &str, token: &str, to: &str, amount: u64) {
    send(
        url,
        from,
        Some(token),
        call_data(
            "mint(address,uint256)",
            &(address(to), alloy::primitives::U256::from(amount)).abi_encode_params(),
        ),
    )
    .await;
}

impl Chain {
    pub fn funding_config(&self) -> ManagedFundingConfig {
        let settings: ManagedFundingSettings = serde_json::from_value(json!({
            "network": "base-sepolia", "funding_wallet": self.wallet,
            "min_paid_age_seconds": 0, "deadline_margin_seconds": 3_600,
            "max_gas": 3_000_000, "max_fee_per_gas_wei": 1_000_000_000u64
        }))
        .unwrap();
        settings.validate(&self.delegate.address()).unwrap();
        ManagedFundingConfig {
            settings,
            target: FundingTarget {
                network: "base-sepolia".into(),
                factory: self.factory.clone(),
                funding_wallet: self.wallet.clone(),
                fee: self.fee.clone(),
            },
            chain_id: 84_532,
            rpc_url: self.url.clone(),
            delegate: self.delegate.clone(),
        }
    }

    /// Mine past the confirmation depth and let the real autonomous indexer catch up.
    pub async fn index(&self, store: &PostgresStore) {
        rpc(&self.url, "anvil_mine", json!(["0x3"])).await;
        let config = AutonomousIndexerConfig {
            network: "base-sepolia".into(),
            rpc_url: self.url.clone(),
            factory_contract: self.factory.clone(),
            start_block: Some(0),
            poll_seconds: 1,
            confirmations: 1,
            max_blocks_per_query: 2_000,
            request_id: 1,
            public_base_url: "http://127.0.0.1:8080".into(),
        };
        for _ in 0..3 {
            crate::poll_autonomous_indexer_once_with_heartbeat(store, &config)
                .await
                .unwrap();
        }
    }

    /// Public terms and the accepted quote exactly as the card-quote route builds them.
    pub async fn bounty_quote(
        &self,
        store: &PostgresStore,
        buyer: &str,
        nonce_byte: &str,
    ) -> (ManagedQuote, String) {
        let now = chrono::Utc::now().timestamp();
        let deadline = now + 7 * 86_400;
        let usdc = |amount: u64| json!({"amount": amount, "currency": "usdc"});
        let document = json!({
            "schema_version": "agent-bounties/terms-v1",
            "title": "Rehearsal task", "goal": "Deliver the rehearsal artifact.",
            "acceptance_criteria": ["It is delivered"],
            "benchmark": {"engine": "customer_review_v1", "reviewer": "paying_customer",
                "acceptance": "all_published_criteria", "delivery_deadline": deadline},
            "verification_policy": {"engine": "customer_review_v1", "mechanism": "signed_quorum",
                "threshold": 1, "verifiers": [TEST_CUSTOMER_REVIEW_VERIFIER],
                "public_disclosure": "Customer review."},
            "evidence_schema": {"type": "object", "required": ["artifact_url", "artifact_sha256"],
                "properties": {"artifact_url": {"type": "string", "pattern": "^https://"},
                    "artifact_sha256": {"type": "string", "pattern": "^sha256:[0-9a-f]{64}$"}}},
            "source_url": null, "discovery_source": "managed_card_quote",
            "contract_terms": {
                "protocol_version": AUTONOMOUS_V2_PROTOCOL_VERSION, "network": "base-sepolia",
                "settlement_token": self.token, "creator_wallet": self.wallet,
                "solver_reward": usdc(SOLVER_REWARD), "verifier_reward": usdc(VERIFIER_REWARD),
                "claim_bond": usdc(VERIFIER_REWARD), "initial_funding": usdc(TARGET),
                "platform_fee_bps": 750, "platform_fee": usdc(TARGET - SOLVER_REWARD - VERIFIER_REWARD),
                "platform_fee_recipient": FEE_RECIPIENT, "funding_deadline": deadline,
                "claim_window_seconds": 604_800, "verification_window_seconds": 604_800,
                "creation_nonce": format!("0x{}", nonce_byte.repeat(32))
            }
        });
        let terms = build_autonomous_bounty_terms_record(
            &self.wallet,
            serde_json::from_value(document).unwrap(),
            chrono::Utc::now(),
        )
        .unwrap();
        store.upsert_autonomous_bounty_terms(&terms).await.unwrap();
        let create = autonomous_v2_create_from_terms(&terms, &self.fee).unwrap();
        let predicted = AutonomousBountyTxPlanner::new(&self.factory, &self.implementation)
            .unwrap()
            .plan_v2_creation("base-sepolia", &create, &self.fee)
            .unwrap()
            .plan
            .predicted_bounty_contract
            .to_ascii_lowercase();
        let calldata = encode_bounded_v2_funding_wallet_create_call(&create).unwrap();
        let price_basis = PriceBasis {
            reference: "rehearsal".into(),
            minor_units_per_usdc_numerator: 1_870,
            minor_units_per_usdc_denominator: 1,
            valid_until: now + 7_200,
        };
        let quote = ManagedQuote::new(QuoteInput {
            quote_id: Uuid::new_v4(),
            buyer_principal: buyer.into(),
            stripe_account_id: "acct_rehearsal".into(),
            livemode: false,
            issued_at: now - 1,
            expires_at: now + 7_200,
            terms: ServiceTerms {
                deliverable: "Rehearsal task".into(),
                acceptance_criteria: "- It is delivered".into(),
                delivery_deadline: deadline,
                terms_version: "rehearsal".into(),
                remedies: "Rehearsal remedies".into(),
            },
            currency: FiatCurrency::Mxn,
            price_basis: price_basis.clone(),
            solver_reward_micro_usdc: SOLVER_REWARD,
            verifier_reserve_micro_usdc: VERIFIER_REWARD,
            // The refundable review deposit every card quote carries.
            additional_costs: vec![managed_orders::AdditionalCost {
                code: managed_orders::pricing::REVIEW_DEPOSIT_CODE.into(),
                label: "Review deposit (refundable, 10% of solver reward)".into(),
                amount_minor: managed_orders::fiat_amount(
                    managed_orders::review::review_deposit_micro_usdc(SOLVER_REWARD),
                    &price_basis,
                )
                .unwrap(),
            }],
            bounty: BountyCommitment {
                network: "base-sepolia".into(),
                factory: self.factory.clone(),
                funding_wallet: self.wallet.clone(),
                creation_nonce: format!("0x{}", nonce_byte.repeat(32)),
                terms_hash: terms.terms_hash.to_ascii_lowercase(),
                verification_setup: "customer_review_v1".into(),
                funding_deadline: deadline,
                predicted_bounty: predicted.clone(),
                create_call_keccak256: managed_orders::funding::calldata_keccak256(&calldata)
                    .unwrap(),
            },
        })
        .unwrap();
        (quote, predicted)
    }
}

/// Accept, check out and settle one order through the leased Checkout and signed webhook path.
pub async fn pay(store: &PostgresStore, quote: &ManagedQuote) -> Uuid {
    let buyer = quote.input().buyer_principal.as_str();
    store.put_managed_service_quote(quote).await.unwrap();
    let order = store
        .accept_managed_service_quote(
            buyer,
            quote.input().quote_id,
            &quote.digest().unwrap(),
            &quote.input().quote_id.to_string(),
        )
        .await
        .unwrap();
    let policy = CheckoutPolicy {
        stripe_account_id: quote.input().stripe_account_id.clone(),
        livemode: false,
        currency: FiatCurrency::Mxn,
        minimum_minor: 1_000,
        maximum_minor: 10_000_000,
        payment_method_configuration: None,
        adaptive_pricing: false,
    };
    let plan = store
        .prepare_managed_checkout(buyer, order.order_id, policy)
        .await
        .unwrap();
    let id = format!("cs_test_{}", order.order_id.simple());
    let request = plan.request(plan.created_at(), plan.policy()).unwrap();
    let session = json!({"id":id,"object":"checkout.session","mode":"payment","livemode":false,
        "metadata":request.body["metadata"],"client_reference_id":plan.order_id(),
        "amount_total":quote.total_minor(),"currency":"mxn","status":"complete",
        "payment_status":"paid","payment_intent":format!("pi_{}", order.order_id.simple()),
        "expires_at":plan.expires_at(),"url":format!("https://checkout.stripe.com/c/pay/{id}")});
    let lease = store
        .claim_managed_checkout(order.order_id)
        .await
        .unwrap()
        .unwrap();
    store
        .complete_managed_checkout(&lease, &plan.session_binding(&session).unwrap())
        .await
        .unwrap();
    let body = serde_json::to_vec(&json!({"id":format!("evt_{}", order.order_id.simple()),
        "object":"event","type":"checkout.session.completed","livemode":false,
        "created":plan.created_at(),"data":{"object":session}}))
    .unwrap();
    let now = chrono::Utc::now().timestamp();
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(b"rehearsal-secret").unwrap();
    mac.update(format!("{now}.").as_bytes());
    mac.update(&body);
    let signature = format!("t={now},v1={}", hex::encode(mac.finalize().into_bytes()));
    let event = verify_managed_checkout_event(
        &body,
        &signature,
        b"rehearsal-secret",
        &plan.policy().stripe_account_id,
        false,
    )
    .unwrap();
    store
        .reconcile_managed_checkout_event(&event)
        .await
        .unwrap();
    order.order_id
}
