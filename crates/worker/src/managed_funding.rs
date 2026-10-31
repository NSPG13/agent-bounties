//! Card-paid orders to funded autonomous-v2 bounties. One delegate key sends only the exact
//! `createBounty` call each accepted quote committed, through the bounded funding wallet that
//! enforces the same limits on chain. Signed bytes are saved before any send, a retry resends
//! those bytes, and only confirmed canonical events mark an order funded.
use anyhow::Context;
use chain_base::{
    base_network_descriptor, broadcast_signed_transaction, fetch_transaction_receipt,
    BaseTransactionRelayer, EvmTransactionIntent, PreparedBaseRelayTransaction,
};
use db::{managed_service_orders::ManagedFundingOperation, PostgresStore};
use managed_orders::funding::{observe_funding, FundingObservation, FundingTarget};
use serde::{Deserialize, Serialize};
use sha3::Digest as _;
use std::time::Duration;
use uuid::Uuid;

pub const MANAGED_FUNDING_ENV: &str = "MANAGED_BOUNTY_FUNDING";
pub const MANAGED_FUNDING_KEY_ENV: &str = "MANAGED_BOUNTY_FUNDING_DELEGATE_PRIVATE_KEY";
const STAGE_TIMEOUT_SECONDS: u64 = 45;
const EVIDENCE_BOUNDARY: &str = "Only confirmed canonical CanonicalBountyCreated and FundingAdded events for the committed bounty mark an order funded. A signature, broadcast or receipt is not funding evidence.";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFundingSettings {
    pub network: String,
    pub funding_wallet: String,
    /// Card payments younger than this are not funded yet.
    pub min_paid_age_seconds: i64,
    /// Orders whose funding deadline is closer than this are not reserved and stay refundable.
    pub deadline_margin_seconds: i64,
    pub max_gas: u64,
    pub max_fee_per_gas_wei: u64,
    #[serde(default = "default_lease_seconds")]
    pub lease_seconds: i64,
    #[serde(default = "default_poll_seconds")]
    pub poll_seconds: u64,
}

fn default_lease_seconds() -> i64 {
    180
}

fn default_poll_seconds() -> u64 {
    30
}

impl ManagedFundingSettings {
    pub fn validate(&self, delegate: &str) -> anyhow::Result<()> {
        let address = |value: &str| {
            value.len() == 42
                && value.starts_with("0x")
                && value[2..]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        };
        anyhow::ensure!(
            matches!(self.network.as_str(), "base-mainnet" | "base-sepolia"),
            "managed funding network must be base-mainnet or base-sepolia"
        );
        anyhow::ensure!(
            address(&self.funding_wallet) && !self.funding_wallet.eq_ignore_ascii_case(delegate),
            "funding wallet must be a lowercase address distinct from the delegate"
        );
        anyhow::ensure!(
            (0..=30 * 86_400).contains(&self.min_paid_age_seconds)
                && (600..=7 * 86_400).contains(&self.deadline_margin_seconds)
                && (100_000..=3_000_000).contains(&self.max_gas)
                && (1..=1_000_000_000_000).contains(&self.max_fee_per_gas_wei)
                && (STAGE_TIMEOUT_SECONDS as i64 * 2..=600).contains(&self.lease_seconds)
                && (5..=300).contains(&self.poll_seconds),
            "managed funding age, margin, gas, fee, lease or poll setting is out of range"
        );
        Ok(())
    }
}

pub struct ManagedFundingConfig {
    pub settings: ManagedFundingSettings,
    pub target: FundingTarget,
    pub chain_id: u64,
    pub rpc_url: String,
    pub delegate: BaseTransactionRelayer,
}

impl ManagedFundingConfig {
    pub fn from_env(store: &PostgresStore) -> anyhow::Result<Self> {
        let settings: ManagedFundingSettings = serde_json::from_str(
            &std::env::var(MANAGED_FUNDING_ENV)
                .with_context(|| format!("{MANAGED_FUNDING_ENV} is required"))?,
        )
        .with_context(|| format!("{MANAGED_FUNDING_ENV} is invalid"))?;
        let delegate = BaseTransactionRelayer::from_private_key(
            &std::env::var(MANAGED_FUNDING_KEY_ENV)
                .map_err(|_| anyhow::anyhow!("managed funding delegate credential unavailable"))?,
        )
        .map_err(|_| anyhow::anyhow!("managed funding delegate credential is invalid"))?
        .with_gas_budget(store.gas_sponsor_budget());
        settings.validate(&delegate.address())?;
        let chain_id = base_network_descriptor(&settings.network)?.chain_id;
        let deployment = service_runtime::autonomous_v2_deployment_for_chain(chain_id)
            .map_err(|error| anyhow::anyhow!("autonomous-v2 settings rejected: {error:?}"))?
            .context("autonomous-v2 factory settings are required for managed funding")?;
        let rpc_url = std::env::var(if settings.network == "base-mainnet" {
            "BASE_MAINNET_RPC_URL"
        } else {
            "BASE_SEPOLIA_RPC_URL"
        })
        .context("managed funding RPC URL is required")?;
        Ok(Self {
            target: FundingTarget {
                network: settings.network.clone(),
                factory: deployment.factory.to_ascii_lowercase(),
                funding_wallet: settings.funding_wallet.clone(),
                fee: deployment.fee,
            },
            settings,
            chain_id,
            rpc_url,
            delegate,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManagedFundingReport {
    pub schema: &'static str,
    pub network: String,
    pub reserved: usize,
    pub not_reserved: usize,
    pub action: &'static str,
    pub order_id: Option<Uuid>,
    pub tx_hash: Option<String>,
    pub detail: Option<String>,
    pub evidence_boundary: &'static str,
}

/// One pass: reserve ready orders, then advance at most one transaction. While any operation
/// has signed bytes and is not terminal, no new transaction is signed.
pub async fn poll_managed_funding_once(
    store: &PostgresStore,
    config: &ManagedFundingConfig,
) -> anyhow::Result<ManagedFundingReport> {
    let settings = &config.settings;
    let (mut reserved, mut not_reserved) = (0, 0);
    for order_id in store
        .list_managed_orders_ready_for_funding(
            &config.target,
            settings.min_paid_age_seconds,
            settings.deadline_margin_seconds,
            10,
        )
        .await?
    {
        match store
            .reserve_managed_funding(order_id, &config.target, settings.deadline_margin_seconds)
            .await
        {
            Ok(_) => reserved += 1,
            // The order stays held and refundable.
            Err(_) => not_reserved += 1,
        }
    }
    let mut report = ManagedFundingReport {
        schema: "agent-bounties/managed-funding-v1",
        network: config.target.network.clone(),
        reserved,
        not_reserved,
        action: "idle",
        order_id: None,
        tx_hash: None,
        detail: None,
        evidence_boundary: EVIDENCE_BOUNDARY,
    };
    let open = store.list_open_managed_funding(100).await?;
    let in_flight = open.iter().find(|op| op.saved_tx_hash.is_some());
    let candidates: Vec<&ManagedFundingOperation> = match in_flight {
        Some(op) => vec![op],
        None => open.iter().take(3).collect(),
    };
    for op in candidates {
        if op.plan.network != config.target.network
            || op.plan.factory != config.target.factory
            || op.plan.funding_wallet != config.target.funding_wallet
        {
            anyhow::bail!("open funding operation belongs to another configuration");
        }
        let Some(lease) = store
            .claim_managed_funding_lease(op.order_id, settings.lease_seconds)
            .await?
        else {
            report.action = "busy";
            return Ok(report);
        };
        let step = advance(store, config, op, lease).await;
        let released = store
            .release_managed_funding_lease(op.order_id, lease)
            .await;
        let (action, tx_hash, detail) = step?;
        released?;
        report.action = action;
        report.order_id = Some(op.order_id);
        report.tx_hash = tx_hash;
        report.detail = detail;
        // An unsigned operation that could not be signed does not block the next one.
        if action != "not_signed" {
            break;
        }
    }
    Ok(report)
}

type Step = (&'static str, Option<String>, Option<String>);

async fn advance(
    store: &PostgresStore,
    config: &ManagedFundingConfig,
    op: &ManagedFundingOperation,
    lease: Uuid,
) -> anyhow::Result<Step> {
    match store
        .relay_outbox(op.operation_id, &config.target.network)
        .await?
    {
        Some(saved) => advance_signed(store, config, op, lease, &saved).await,
        None => sign(store, config, op, lease).await,
    }
}

async fn sign(
    store: &PostgresStore,
    config: &ManagedFundingConfig,
    op: &ManagedFundingOperation,
    lease: Uuid,
) -> anyhow::Result<Step> {
    let plan = &op.plan;
    if plan.funding_deadline <= chrono::Utc::now().timestamp() + STAGE_TIMEOUT_SECONDS as i64 * 4 {
        store
            .record_managed_funding_reconciliation(op.order_id)
            .await?;
        return Ok((
            "reconciliation_required",
            None,
            Some(
                "Funding deadline reached before a transaction was signed; nothing was sent."
                    .into(),
            ),
        ));
    }
    // A dispute recorded after reservation still stops funding while nothing is signed.
    if store.managed_order_disputed(op.order_id).await? {
        store
            .record_managed_funding_reconciliation(op.order_id)
            .await?;
        return Ok((
            "reconciliation_required",
            None,
            Some("A card dispute was opened before signing; nothing was sent.".into()),
        ));
    }
    let intent = EvmTransactionIntent {
        from: Some(config.delegate.address()),
        to: plan.funding_wallet.clone(),
        value_wei: 0,
        data: plan.calldata.clone(),
        function: "createBounty".into(),
    };
    // Simulation runs here: an unfunded wallet, spent policy budget or revoked delegate fails
    // before anything is signed.
    let prepared = tokio::time::timeout(
        Duration::from_secs(STAGE_TIMEOUT_SECONDS),
        config.delegate.prepare_signed_transaction(
            &config.rpc_url,
            config.chain_id,
            &intent,
            config.settings.max_gas,
            u128::from(config.settings.max_fee_per_gas_wei),
        ),
    )
    .await;
    let prepared = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) => {
            return Ok((
                "not_signed",
                None,
                Some(crate::redact_operational_error(&error.to_string())),
            ))
        }
        Err(_) => return Ok(("not_signed", None, Some("Preparation timed out.".into()))),
    };
    // A stale holder discards its signature: unsaved bytes are never sent.
    if !store
        .managed_funding_lease_has_time(op.order_id, lease, 10)
        .await?
    {
        return Ok((
            "not_signed",
            None,
            Some("Lease expired before saving.".into()),
        ));
    }
    store
        .save_relay_outbox(op.operation_id, &config.target.network, &prepared)
        .await?;
    store
        .record_managed_funding_broadcast(op.order_id, lease, &prepared.transaction.tx_hash)
        .await?;
    send(store, config, op, lease, &prepared).await?;
    Ok(("signed", Some(prepared.transaction.tx_hash), None))
}

async fn advance_signed(
    store: &PostgresStore,
    config: &ManagedFundingConfig,
    op: &ManagedFundingOperation,
    lease: Uuid,
    saved: &PreparedBaseRelayTransaction,
) -> anyhow::Result<Step> {
    let tx_hash = saved.transaction.tx_hash.to_ascii_lowercase();
    if op.tx_hash.as_deref() != Some(tx_hash.as_str()) {
        // Repair a crash between saving the bytes and recording their hash.
        store
            .record_managed_funding_broadcast(op.order_id, lease, &tx_hash)
            .await?;
    }
    let events = store.verified_funding_events(&op.plan, &tx_hash).await?;
    match observe_funding(&op.plan, &events) {
        FundingObservation::Funded {
            tx_hash,
            block_number,
        } => {
            store
                .record_managed_funding_funded(op.order_id, &tx_hash, block_number)
                .await?;
            return Ok(("funded", Some(tx_hash), None));
        }
        FundingObservation::Conflict => {
            store
                .record_managed_funding_reconciliation(op.order_id)
                .await?;
            return Ok((
                "reconciliation_required",
                Some(tx_hash),
                Some("Canonical events differ from the committed bounty.".into()),
            ));
        }
        FundingObservation::NotObserved => {}
    }
    let receipt = tokio::time::timeout(
        Duration::from_secs(STAGE_TIMEOUT_SECONDS),
        fetch_transaction_receipt(&config.rpc_url, &tx_hash, 2),
    )
    .await
    .context("receipt lookup timed out")??
    .result;
    if let Some(receipt) = receipt {
        if receipt.status.as_deref() == Some("0x0") {
            store
                .record_managed_funding_reconciliation(op.order_id)
                .await?;
            return Ok((
                "reconciliation_required",
                Some(tx_hash),
                Some("The funding transaction reverted.".into()),
            ));
        }
        // Mined; the indexer confirms its events before this order is funded.
        return Ok(("awaiting_confirmation", Some(tx_hash), None));
    }
    if store
        .managed_funding_send_window_closed(op.operation_id)
        .await?
    {
        store
            .record_managed_funding_reconciliation(op.order_id)
            .await?;
        return Ok((
            "reconciliation_required",
            Some(tx_hash),
            Some("No receipt within the send window.".into()),
        ));
    }
    if store.relay_outbox_send_ready(op.operation_id).await? {
        config
            .delegate
            .reserve_prepared_retransmission(&config.rpc_url, config.chain_id, saved)
            .await?;
        send(store, config, op, lease, saved).await?;
        return Ok(("rebroadcast", Some(tx_hash), None));
    }
    Ok(("awaiting_receipt", Some(tx_hash), None))
}

/// Send the saved bytes at most once per claimed outbox attempt, only while the lease holds.
async fn send(
    store: &PostgresStore,
    config: &ManagedFundingConfig,
    op: &ManagedFundingOperation,
    lease: Uuid,
    saved: &PreparedBaseRelayTransaction,
) -> anyhow::Result<()> {
    let raw = hex::decode(
        saved
            .signed_transaction
            .strip_prefix("0x")
            .context("saved transaction is not hex")?,
    )?;
    anyhow::ensure!(
        format!("0x{}", hex::encode(sha3::Keccak256::digest(raw))) == saved.transaction.tx_hash
            && saved
                .transaction
                .relayer
                .eq_ignore_ascii_case(&config.delegate.address())
            && saved.transaction.max_fee_per_gas_wei
                <= u128::from(config.settings.max_fee_per_gas_wei),
        "saved funding transaction failed integrity checks"
    );
    if !store.claim_relay_outbox_send(op.operation_id).await?
        || !store
            .managed_funding_lease_has_time(op.order_id, lease, STAGE_TIMEOUT_SECONDS as i64 + 2)
            .await?
    {
        return Ok(());
    }
    // The bytes and hash are durable; a lost reply cannot change the nonce or the call.
    let _ = tokio::time::timeout(
        Duration::from_secs(STAGE_TIMEOUT_SECONDS),
        broadcast_signed_transaction(&config.rpc_url, &saved.signed_transaction, 3),
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod rehearsal;
#[cfg(any(test, feature = "rehearsal-support"))]
pub mod rehearsal_support;

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> ManagedFundingSettings {
        serde_json::from_value(serde_json::json!({
            "network": "base-sepolia",
            "funding_wallet": format!("0x{}", "f0".repeat(20)),
            "min_paid_age_seconds": 3_600,
            "deadline_margin_seconds": 21_600,
            "max_gas": 1_500_000,
            "max_fee_per_gas_wei": 2_000_000_000u64
        }))
        .unwrap()
    }

    #[test]
    fn settings_reject_unsafe_values_and_unknown_fields() {
        let delegate = format!("0x{}", "d0".repeat(20));
        settings().validate(&delegate).unwrap();
        let checks: [fn(&mut ManagedFundingSettings); 8] = [
            |s| s.network = "ethereum".into(),
            |s| s.funding_wallet = s.funding_wallet.to_ascii_uppercase(),
            |s| s.funding_wallet = format!("0x{}", "d0".repeat(20)),
            |s| s.min_paid_age_seconds = -1,
            |s| s.deadline_margin_seconds = 60,
            |s| s.max_gas = 10_000_000,
            |s| s.max_fee_per_gas_wei = 0,
            |s| s.lease_seconds = 30,
        ];
        for change in checks {
            let mut changed = settings();
            change(&mut changed);
            assert!(changed.validate(&delegate).is_err());
        }
        let mut value = serde_json::to_value(serde_json::json!({
            "network": "base-sepolia", "funding_wallet": format!("0x{}", "f0".repeat(20)),
            "min_paid_age_seconds": 0, "deadline_margin_seconds": 3_600, "max_gas": 1_000_000,
            "max_fee_per_gas_wei": 1
        }))
        .unwrap();
        value["private_key"] = serde_json::json!("never in settings");
        assert!(serde_json::from_value::<ManagedFundingSettings>(value).is_err());
    }
}
