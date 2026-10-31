//! Paid order to its exact canonical bounty. The funding worker may submit only the call the
//! customer's accepted quote committed, and only canonical events prove the bounty was funded.
use crate::ManagedQuote;
use chain_base::{
    autonomous_v2_create_from_terms, encode_bounded_v2_funding_wallet_create_call,
    AutonomousBountyEvent, AutonomousBountyEventKind, AutonomousV2FactoryFee,
};
use domain::AutonomousBountyTermsRecord;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FundingError {
    #[error("the funding call does not match the accepted quote")]
    Mismatch,
    #[error("the accepted quote or its public terms are invalid")]
    Invalid,
    #[error("the order commits to another network, factory or funding wallet")]
    OtherTarget,
}

/// The one network, v2 factory, funding wallet and fee a funding worker serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingTarget {
    pub network: String,
    pub factory: String,
    pub funding_wallet: String,
    pub fee: AutonomousV2FactoryFee,
}

/// The one transaction an order may fund: `funding_wallet.createBounty(...)` with these bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FundingPlan {
    pub order_id: Uuid,
    pub network: String,
    pub factory: String,
    pub funding_wallet: String,
    pub predicted_bounty: String,
    pub creation_nonce: String,
    pub funding_deadline: i64,
    pub target_micro_usdc: u64,
    pub calldata: String,
    pub create_call_keccak256: String,
}

impl FundingPlan {
    /// Rebuilds the exact call from the stored public terms and refuses anything whose bytes,
    /// creator, nonce, rewards or target differ from what the customer accepted and paid for.
    pub fn new(
        order_id: Uuid,
        quote: &ManagedQuote,
        terms: &AutonomousBountyTermsRecord,
        target: &FundingTarget,
    ) -> Result<Self, FundingError> {
        quote.validate().map_err(|_| FundingError::Invalid)?;
        let input = quote.input();
        let bounty = &input.bounty;
        if bounty.network != target.network
            || !bounty.factory.eq_ignore_ascii_case(&target.factory)
            || !bounty
                .funding_wallet
                .eq_ignore_ascii_case(&target.funding_wallet)
        {
            return Err(FundingError::OtherTarget);
        }
        if !terms.terms_hash.eq_ignore_ascii_case(&bounty.terms_hash)
            || !terms
                .creator_wallet
                .eq_ignore_ascii_case(&bounty.funding_wallet)
        {
            return Err(FundingError::Mismatch);
        }
        let create = autonomous_v2_create_from_terms(terms, &target.fee)
            .map_err(|_| FundingError::Invalid)?;
        let amounts_match = u64::try_from(create.base.solver_reward.amount)
            == Ok(input.solver_reward_micro_usdc)
            && u64::try_from(create.base.verifier_reward.amount)
                == Ok(input.verifier_reserve_micro_usdc)
            && u64::try_from(create.base.initial_funding.amount)
                == Ok(quote.bounty_target_micro_usdc());
        if !amounts_match
            || !create
                .base
                .creation_nonce
                .eq_ignore_ascii_case(&bounty.creation_nonce)
            || i64::try_from(create.base.funding_deadline) != Ok(bounty.funding_deadline)
        {
            return Err(FundingError::Mismatch);
        }
        let calldata = encode_bounded_v2_funding_wallet_create_call(&create)
            .map_err(|_| FundingError::Invalid)?;
        let digest = calldata_keccak256(&calldata).ok_or(FundingError::Invalid)?;
        if digest != bounty.create_call_keccak256 {
            return Err(FundingError::Mismatch);
        }
        Ok(Self {
            order_id,
            network: bounty.network.clone(),
            factory: bounty.factory.clone(),
            funding_wallet: bounty.funding_wallet.clone(),
            predicted_bounty: bounty.predicted_bounty.clone(),
            creation_nonce: bounty.creation_nonce.to_ascii_lowercase(),
            funding_deadline: bounty.funding_deadline,
            target_micro_usdc: quote.bounty_target_micro_usdc(),
            calldata,
            create_call_keccak256: digest,
        })
    }
}

pub fn calldata_keccak256(calldata: &str) -> Option<String> {
    let bytes = hex::decode(calldata.strip_prefix("0x")?).ok()?;
    Some(format!("0x{}", hex::encode(Keccak256::digest(bytes))))
}

/// What canonical, confirmed events show for a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FundingObservation {
    NotObserved,
    Funded {
        tx_hash: String,
        block_number: u64,
    },
    /// The predicted bounty exists with other creator, nonce or funding; needs an operator.
    Conflict,
}

/// Funded only when the committed factory emitted `CanonicalBountyCreated` for the predicted
/// bounty with the wallet as creator and the committed nonce, and that bounty emitted
/// `FundingAdded` crediting the wallet with the full target in the same transaction. Callers
/// must pass only events the indexer has confirmed (block time verified).
pub fn observe_funding(plan: &FundingPlan, events: &[AutonomousBountyEvent]) -> FundingObservation {
    let field = |event: &AutonomousBountyEvent, key: &str| {
        event.data[key]
            .as_str()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default()
    };
    let created: Vec<_> = events
        .iter()
        .filter(|event| {
            event.kind == AutonomousBountyEventKind::CanonicalBountyCreated
                && event.contract_address.eq_ignore_ascii_case(&plan.factory)
                && field(event, "bounty_contract") == plan.predicted_bounty
        })
        .collect();
    let [creation] = created.as_slice() else {
        return if created.is_empty() {
            FundingObservation::NotObserved
        } else {
            FundingObservation::Conflict
        };
    };
    if field(creation, "creator") != plan.funding_wallet
        || field(creation, "creation_nonce") != plan.creation_nonce
    {
        return FundingObservation::Conflict;
    }
    let funding: Vec<_> = events
        .iter()
        .filter(|event| {
            event.kind == AutonomousBountyEventKind::FundingAdded
                && event.bounty_id == creation.bounty_id
                && event
                    .contract_address
                    .eq_ignore_ascii_case(&plan.predicted_bounty)
        })
        .collect();
    let target = u128::from(plan.target_micro_usdc);
    let exact = funding.iter().find(|event| {
        event.tx_hash.eq_ignore_ascii_case(&creation.tx_hash)
            && field(event, "contributor") == plan.funding_wallet
            && event.data["amount"].as_u64().map(u128::from) == Some(target)
            && event.data["funded_amount"].as_u64().map(u128::from) == Some(target)
            && event.data["target_amount"].as_u64().map(u128::from) == Some(target)
    });
    match exact {
        Some(event) => FundingObservation::Funded {
            tx_hash: event.tx_hash.to_ascii_lowercase(),
            block_number: event.block_number,
        },
        None if funding.is_empty() => FundingObservation::NotObserved,
        None => FundingObservation::Conflict,
    }
}

#[cfg(test)]
mod tests;
