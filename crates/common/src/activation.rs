use alloy::primitives::Address;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ActivationStatus {
    Claimable,
    Claimed,
    Submitted,
    Verifying,
    Settled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivationState {
    pub contract_address: Address,
    pub status: ActivationStatus,
    pub canonical: bool,
}

impl ActivationState {
    pub fn new(contract_address: Address, status: ActivationStatus) -> Self {
        Self {
            contract_address,
            status,
            canonical: false,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.status,
            ActivationStatus::Claimable
                | ActivationStatus::Claimed
                | ActivationStatus::Submitted
                | ActivationStatus::Verifying
        )
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            ActivationStatus::Settled | ActivationStatus::Failed
        )
    }

    pub fn mark_canonical(&mut self) {
        self.canonical = true;
    }

    pub fn should_reconcile(&self) -> bool {
        self.is_active() && !self.canonical
    }
}
