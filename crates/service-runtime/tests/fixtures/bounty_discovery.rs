// Shared synthetic on-chain fixture. Only seed an isolated test database.
pub async fn seed(store: &db::PostgresStore, network: &str) -> (String, String) {
    use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind as Kind};
    use serde_json::json;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let id = format!("0x{suffix}{suffix}");
    let contract = format!("0x00000000{suffix}");
    let factory = service_runtime::CANONICAL_BASE_MAINNET_BOUNTY_FACTORY;
    let zero = format!("0x{}", "0".repeat(40));
    let tx = format!(
        "0x{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let entries = [
        (
            Kind::CanonicalBountyCreated,
            json!({"bounty_contract":contract,"creator":zero,"terms_hash":id,"policy_hash":id}),
        ),
        (
            Kind::CanonicalBountyTermsCommitted,
            json!({"acceptance_criteria_hash":id,"benchmark_hash":id,"evidence_schema_hash":id}),
        ),
        (
            Kind::CanonicalBountyEconomicsConfigured,
            json!({"solver_reward":20_000_000,"verifier_reward":1_000_000,"claim_bond":1_000_000,"target_amount":21_000_000,"initial_funding":21_000_000,"funding_deadline":4_000_000_000_u64,"claim_window_seconds":86400,"verification_window_seconds":172800}),
        ),
        (
            Kind::CanonicalBountyVerificationConfigured,
            json!({"verification_mode":1,"verifier_module":zero,"verifier_reward_recipient":zero,"threshold":1,"verifier_set_hash":id}),
        ),
        (
            Kind::FundingAdded,
            json!({"funded_amount":21_000_000,"target_amount":21_000_000,"amount":21_000_000,"contributor":zero}),
        ),
        (
            Kind::BountyBecameClaimable,
            json!({"funded_amount":21_000_000}),
        ),
    ];
    for (index, (kind, data)) in entries.into_iter().enumerate() {
        store
            .upsert_autonomous_bounty_event(
                network,
                &AutonomousBountyEvent {
                    id: uuid::Uuid::new_v4(),
                    log_key: format!("{tx}:{index}"),
                    tx_hash: tx.clone(),
                    block_number: 1,
                    log_index: index as u64,
                    contract_address: if index < 4 {
                        factory.to_string()
                    } else {
                        contract.clone()
                    },
                    bounty_id: id.clone(),
                    kind,
                    data,
                    occurred_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
    }
    let cursor = store.get_base_log_cursor(network, factory).await.unwrap();
    let previous = store.autonomous_canonical_checkpoint(network, factory).await.unwrap();
    store.commit_autonomous_canonical_checkpoint(network, factory,
        cursor.map(|c| c.last_scanned_block), previous.as_ref(),
        &chain_base::BaseBlockIdentity {number:100,hash:format!("0x{}", "11".repeat(32)),timestamp:1},None).await.unwrap();
    (id, contract)
}
