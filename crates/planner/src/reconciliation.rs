use common::{ActivationState, ActivationStatus};
use alloy::primitives::Address;
use std::collections::HashMap;

pub struct ReconciliationEngine {
    factory_state: HashMap<Address, ActivationState>,
    feed_state: HashMap<Address, ActivationState>,
}

impl ReconciliationEngine {
    pub fn new() -> Self {
        Self {
            factory_state: HashMap::new(),
            feed_state: HashMap::new(),
        }
    }

    pub fn register_factory_state(&mut self, state: ActivationState) {
        self.factory_state.insert(state.contract_address, state);
    }

    pub fn register_feed_state(&mut self, state: ActivationState) {
        self.feed_state.insert(state.contract_address, state);
    }

    pub fn reconcile(&mut self, contract: Address) -> ReconciliationResult {
        let factory = self.factory_state.get(&contract);
        let feed = self.feed_state.get(&contract);

        match (factory, feed) {
            (Some(f), Some(feed_state)) if f.canonical => {
                ReconciliationResult::AlreadyCanonical
            }
            (Some(f), Some(feed_state)) if f.is_active() && feed_state.is_active() => {
                ReconciliationResult::Resume(f.status.clone())
            }
            (Some(f), Some(feed_state)) if f.is_terminal() => {
                ReconciliationResult::Terminal(f.status.clone())
            }
            (Some(f), None) if f.is_active() => {
                ReconciliationResult::Create(f.status.clone())
            }
            (None, _) => ReconciliationResult::InvalidTerms,
            _ => ReconciliationResult::Ambiguous,
        }
    }

    pub fn mark_canonical(&mut self, contract: Address) {
        if let Some(state) = self.factory_state.get_mut(&contract) {
            state.mark_canonical();
        }
    }
}

impl Default for ReconciliationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReconciliationResult {
    AlreadyCanonical,
    Resume(ActivationStatus),
    Create(ActivationStatus),
    Terminal(ActivationStatus),
    InvalidTerms,
    Ambiguous,
}
