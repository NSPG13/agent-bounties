//! Exact recovered-principal evidence. Callers must obtain the history and state from an
//! attested RPC at one safe block. A timestamp-verified indexed row alone is insufficient.
use crate::{
    funding::{observe_funding, FundingObservation, FundingPlan},
    ManagedQuote,
};
use chain_base::{AutonomousBountyEvent, AutonomousBountyEventKind as Kind};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("recovered bounty evidence is incomplete, conflicting or not final")]
pub struct RecoveryError;

/// Canonical block identity retained for revalidation before an irreversible refund request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryBlock {
    pub number: u64,
    pub hash: String,
}

/// No Deserialize implementation: stored JSON does not become fresh chain authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveredPrincipal {
    order_id: uuid::Uuid,
    quote_digest: String,
    funding_tx: String,
    cancellation_tx: String,
    withdrawal_tx: String,
    withdrawal_log_index: u64,
    principal_micro_usdc: u64,
    amount_micro_usdc: u64,
    withdrawal_block: RecoveryBlock,
    safe_block: RecoveryBlock,
}
impl RecoveredPrincipal {
    pub fn order_id(&self) -> uuid::Uuid {
        self.order_id
    }
    pub fn quote_digest(&self) -> &str {
        &self.quote_digest
    }
    pub fn withdrawal_tx(&self) -> &str {
        &self.withdrawal_tx
    }
    pub fn withdrawal_block(&self) -> &RecoveryBlock {
        &self.withdrawal_block
    }
    pub fn identity_digest(&self) -> String {
        // The safe checkpoint advances on revalidation; recovery identity must not.
        let identity = serde_json::json!([
            self.order_id,
            self.quote_digest,
            self.funding_tx,
            self.cancellation_tx,
            self.withdrawal_tx,
            self.withdrawal_log_index,
            self.principal_micro_usdc,
            self.amount_micro_usdc,
            self.withdrawal_block
        ]);
        hex::encode(Sha256::digest(
            serde_json::to_vec(&identity).expect("serializable identity"),
        ))
    }
}
fn hash(s: &str) -> bool {
    s.len() == 66
        && s.starts_with("0x")
        && s[2..].bytes().all(|b| b.is_ascii_hexdigit())
        && s[2..].bytes().any(|b| b != b'0')
}
fn field(event: &AutonomousBountyEvent, key: &str) -> String {
    event.data[key]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase()
}
/// Typed reads made at the same safe block as the receipt checks. These inputs are deliberately
/// not deserializable from a customer's request. Factory/runtime/token attestation belongs at
/// the RPC boundary, before this function is called.
pub struct RecoveryState {
    pub cancelled: bool,
    pub active_claim_bond: u64,
    pub pending_bond_refund_total: u64,
    pub funding_wallet_contribution: u64,
}

/// Verify the entire original funding target, not just the smaller refundable card allocation.
/// `events` must contain the creation/funding, cancellation and withdrawal receipts from the
/// pinned factory and bounty. Each receipt/block identity must be freshly checked by the caller;
/// the pinned contract's terminal cancellation state excludes any prior settlement.
/// The unique withdrawal must also be corroborated by native-USDC Transfer in that receipt.
pub fn verify_recovered_principal(
    quote: &ManagedQuote,
    plan: &FundingPlan,
    events: &[AutonomousBountyEvent],
    safe: RecoveryBlock,
    withdrawal_block: RecoveryBlock,
    state: RecoveryState,
    exact_native_transfer: bool,
) -> Result<RecoveredPrincipal, RecoveryError> {
    let quote_digest = quote.digest().map_err(|_| RecoveryError)?;
    let b = &quote.input().bounty;
    if plan.order_id.is_nil()
        || plan.network != b.network
        || plan.factory != b.factory
        || plan.predicted_bounty != b.predicted_bounty
        || plan.funding_wallet != b.funding_wallet
        || plan.creation_nonce != b.creation_nonce
        || plan.target_micro_usdc != quote.bounty_target_micro_usdc()
        || !state.cancelled
        || state.active_claim_bond != 0
        || state.pending_bond_refund_total != 0
        || state.funding_wallet_contribution != 0
        || !exact_native_transfer
        || !hash(&safe.hash)
        || !hash(&withdrawal_block.hash)
        || safe.number < withdrawal_block.number.saturating_add(2)
    {
        return Err(RecoveryError);
    }
    let mut seen = std::collections::HashSet::new();
    for e in events {
        if !hash(&e.tx_hash)
            || !hash(&e.bounty_id)
            || e.block_number > safe.number
            || !seen.insert((e.tx_hash.to_ascii_lowercase(), e.log_index))
            || !(e.contract_address.eq_ignore_ascii_case(&plan.factory)
                || e.contract_address
                    .eq_ignore_ascii_case(&plan.predicted_bounty))
        {
            return Err(RecoveryError);
        }
    }
    let FundingObservation::Funded {
        tx_hash: funding_tx,
        block_number: funding_block,
    } = observe_funding(plan, events)
    else {
        return Err(RecoveryError);
    };
    let created = events
        .iter()
        .find(|e| {
            e.kind == Kind::CanonicalBountyCreated
                && field(e, "bounty_contract") == plan.predicted_bounty
        })
        .ok_or(RecoveryError)?;
    let bounty_events: Vec<_> = events
        .iter()
        .filter(|e| {
            e.contract_address
                .eq_ignore_ascii_case(&plan.predicted_bounty)
        })
        .collect();
    if bounty_events
        .iter()
        .any(|e| e.bounty_id != created.bounty_id || e.kind == Kind::BountySettled)
    {
        return Err(RecoveryError);
    }
    let cancellations: Vec<_> = bounty_events
        .iter()
        .filter(|e| e.kind == Kind::BountyCancelled)
        .collect();
    let [cancel] = cancellations.as_slice() else {
        return Err(RecoveryError);
    };
    let withdrawals: Vec<_> = bounty_events
        .iter()
        .filter(|e| {
            e.kind == Kind::RefundWithdrawn && field(e, "contributor") == plan.funding_wallet
        })
        .collect();
    let [withdraw] = withdrawals.as_slice() else {
        return Err(RecoveryError);
    };
    let principal = withdraw.data["principal"].as_u64().ok_or(RecoveryError)?;
    let bonus = withdraw.data["timeout_bond_bonus"]
        .as_u64()
        .ok_or(RecoveryError)?;
    let amount = withdraw.data["amount"].as_u64().ok_or(RecoveryError)?;
    if principal != plan.target_micro_usdc
        || principal.checked_add(bonus) != Some(amount)
        || withdraw.block_number != withdrawal_block.number
        || (cancel.block_number, cancel.log_index) >= (withdraw.block_number, withdraw.log_index)
        || cancel.block_number < funding_block
    {
        return Err(RecoveryError);
    }
    Ok(RecoveredPrincipal {
        order_id: plan.order_id,
        quote_digest,
        funding_tx,
        cancellation_tx: cancel.tx_hash.to_ascii_lowercase(),
        withdrawal_tx: withdraw.tx_hash.to_ascii_lowercase(),
        withdrawal_log_index: withdraw.log_index,
        principal_micro_usdc: principal,
        amount_micro_usdc: amount,
        withdrawal_block,
        safe_block: safe,
    })
}

#[cfg(test)]
mod tests;
