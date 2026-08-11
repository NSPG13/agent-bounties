use common::{ActivationState, ActivationStatus};
use planner::{ReconciliationEngine, ReconciliationResult};
use alloy::primitives::Address;

#[test]
fn test_claimable_state_creates_new_activation() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([1u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Claimable);
    engine.register_factory_state(factory_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Create(ActivationStatus::Claimable));
}

#[test]
fn test_claimed_state_resumes_without_duplicate() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([2u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Claimed);
    let feed_state = ActivationState::new(contract, ActivationStatus::Claimed);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Resume(ActivationStatus::Claimed));
}

#[test]
fn test_submitted_state_resumes_without_duplicate() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([3u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Submitted);
    let feed_state = ActivationState::new(contract, ActivationStatus::Submitted);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Resume(ActivationStatus::Submitted));
}

#[test]
fn test_verifying_state_resumes_without_duplicate() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([4u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Verifying);
    let feed_state = ActivationState::new(contract, ActivationStatus::Verifying);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Resume(ActivationStatus::Verifying));
}

#[test]
fn test_canonical_contract_skips_planner() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([5u8; 20]);
    
    let mut factory_state = ActivationState::new(contract, ActivationStatus::Claimed);
    factory_state.mark_canonical();
    let feed_state = ActivationState::new(contract, ActivationStatus::Claimed);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::AlreadyCanonical);
}

#[test]
fn test_invalid_terms_fails_closed() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([6u8; 20]);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::InvalidTerms);
}

#[test]
fn test_terminal_failed_state_fails_closed() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([7u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Failed);
    let feed_state = ActivationState::new(contract, ActivationStatus::Failed);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Terminal(ActivationStatus::Failed));
}

#[test]
fn test_settled_state_is_terminal() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([8u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Settled);
    let feed_state = ActivationState::new(contract, ActivationStatus::Settled);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let result = engine.reconcile(contract);
    assert_eq!(result, ReconciliationResult::Terminal(ActivationStatus::Settled));
}

#[test]
fn test_mark_canonical_prevents_reprocessing() {
    let mut engine = ReconciliationEngine::new();
    let contract = Address::from([9u8; 20]);
    
    let factory_state = ActivationState::new(contract, ActivationStatus::Claimed);
    let feed_state = ActivationState::new(contract, ActivationStatus::Claimed);
    
    engine.register_factory_state(factory_state);
    engine.register_feed_state(feed_state);
    
    let first_result = engine.reconcile(contract);
    assert_eq!(first_result, ReconciliationResult::Resume(ActivationStatus::Claimed));
    
    engine.mark_canonical(contract);
    
    let second_result = engine.reconcile(contract);
    assert_eq!(second_result, ReconciliationResult::AlreadyCanonical);
}
