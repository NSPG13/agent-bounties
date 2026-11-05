//! Opt-in loopback Anvil + disposable Postgres rehearsal of a card-paid order becoming a funded
//! autonomous-v2 bounty through the real factory, bounded wallet, worker and indexer. No hosted
//! writes. Run `scripts/check-managed-funding.sh`.
use super::rehearsal_support::{
    address, balance, call_data, deploy, isolated_store, mint, pay, rpc, send, TARGET,
};
use super::*;
use alloy::sol_types::SolValue;
use db::managed_service_orders::ManagedOrderStatus;
use serde_json::json;

const BUYER: &str = "account:rehearsal";

async fn status(store: &PostgresStore, order_id: Uuid) -> ManagedOrderStatus {
    store
        .get_managed_service_order(BUYER, order_id)
        .await
        .unwrap()
        .unwrap()
        .status
}

async fn delegate_nonce(url: &str, config: &ManagedFundingConfig) -> serde_json::Value {
    rpc(
        url,
        "eth_getTransactionCount",
        json!([config.delegate.address(), "latest"]),
    )
    .await
}

#[tokio::test]
#[ignore = "run scripts/check-managed-funding.sh with a disposable loopback database"]
async fn paid_card_order_funds_its_exact_bounty_once_through_real_evm_and_indexer() {
    let url = std::env::var("AGENT_BOUNTIES_TEST_RPC_URL").unwrap();
    let store = isolated_store(&std::env::var("AGENT_BOUNTIES_TEST_DATABASE_URL").unwrap()).await;
    let chain = deploy(&url, &store, 50_000_000).await;
    let config = chain.funding_config();
    let (quote, predicted) = chain.bounty_quote(&store, BUYER, "5a").await;
    let order_id = pay(&store, &quote).await;

    let signed = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(
        (signed.reserved, signed.action),
        (1, "signed"),
        "{signed:?}"
    );
    let tx_hash = signed.tx_hash.clone().unwrap();
    assert_eq!(
        status(&store, order_id).await,
        ManagedOrderStatus::FundingBroadcast
    );
    assert!(store.request_managed_refund(BUYER, order_id).await.is_err());
    // Mined, but nothing is funded until the indexer confirms canonical events.
    let mined = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(mined.action, "awaiting_confirmation", "{mined:?}");
    assert_eq!(
        status(&store, order_id).await,
        ManagedOrderStatus::FundingBroadcast
    );

    chain.index(&store).await;
    let funded = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(funded.action, "funded", "{funded:?}");
    assert_eq!(funded.tx_hash.as_deref(), Some(tx_hash.as_str()));
    assert_eq!(status(&store, order_id).await, ManagedOrderStatus::Funded);
    assert_eq!(
        balance(&url, &chain.token, &predicted).await,
        u128::from(TARGET)
    );
    assert_eq!(
        balance(&url, &chain.token, &chain.wallet).await,
        u128::from(50_000_000 - TARGET)
    );

    // Nothing further is signed: one order, one transaction, one nonce.
    let idle = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!((idle.reserved, idle.action), (0, "idle"), "{idle:?}");
    assert_eq!(delegate_nonce(&url, &config).await, "0x1");
    assert!(store.request_managed_refund(BUYER, order_id).await.is_err());

    // An emptied wallet fails simulation: nothing is signed and the order waits, reserved.
    let (second_quote, second_bounty) = chain.bounty_quote(&store, BUYER, "6b").await;
    let second = pay(&store, &second_quote).await;
    let remaining = alloy::primitives::U256::from(balance(&url, &chain.token, &chain.wallet).await);
    send(
        &url,
        &chain.owner,
        Some(&chain.wallet),
        call_data(
            "withdrawToken(address,address,uint256)",
            &(address(&chain.token), address(&chain.owner), remaining).abi_encode_params(),
        ),
    )
    .await;
    let blocked = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(
        (blocked.reserved, blocked.action),
        (1, "not_signed"),
        "{blocked:?}"
    );
    assert!(blocked.tx_hash.is_none());
    assert_eq!(
        status(&store, second).await,
        ManagedOrderStatus::FundingReserved
    );
    assert_eq!(delegate_nonce(&url, &config).await, "0x1");
    mint(&url, &chain.owner, &chain.token, &chain.wallet, TARGET).await;
    let resumed = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(resumed.action, "signed", "{resumed:?}");
    chain.index(&store).await;
    let funded = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(funded.action, "funded", "{funded:?}");
    assert_eq!(status(&store, second).await, ManagedOrderStatus::Funded);
    assert_eq!(
        balance(&url, &chain.token, &second_bounty).await,
        u128::from(TARGET)
    );
    assert_eq!(balance(&url, &chain.token, &chain.wallet).await, 0);

    // A card dispute recorded after reservation still stops funding before anything is signed.
    mint(&url, &chain.owner, &chain.token, &chain.wallet, TARGET).await;
    let (third_quote, _) = chain.bounty_quote(&store, BUYER, "8d").await;
    let third = pay(&store, &third_quote).await;
    store
        .reserve_managed_funding(third, &config.target, 3_600)
        .await
        .unwrap();
    store
        .record_managed_dispute(&managed_orders::stripe_dispute::ManagedDisputeEvent {
            event_id: "evt_rehearsal_dispute".into(),
            event_type: "charge.dispute.created".into(),
            event_created: chrono::Utc::now().timestamp(),
            livemode: false,
            dispute_id: format!("dp_{}", third.simple()),
            payment_intent_id: format!("pi_{}", third.simple()),
            amount_minor: third_quote.total_minor(),
            currency: "mxn".into(),
            status: "needs_response".into(),
            reason: "fraudulent".into(),
        })
        .await
        .unwrap();
    let stopped = poll_managed_funding_once(&store, &config).await.unwrap();
    assert_eq!(stopped.action, "reconciliation_required", "{stopped:?}");
    assert!(stopped.tx_hash.is_none());
    assert_eq!(
        status(&store, third).await,
        ManagedOrderStatus::FundingReconciliationRequired
    );
    assert_eq!(delegate_nonce(&url, &config).await, "0x2");
    assert_eq!(
        balance(&url, &chain.token, &chain.wallet).await,
        u128::from(TARGET)
    );
}
