use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq)]
pub struct BountyConfig {
    pub total_funding_usdc: f64,
    pub solver_reward_usdc: f64,
    pub claim_bond_usdc: f64,
    pub required_child_spend_usdc: f64,
    pub sponsored_claim_gas: bool,
}

impl Default for BountyConfig {
    fn default() -> Self {
        Self {
            total_funding_usdc: 2.00,
            solver_reward_usdc: 1.99,
            claim_bond_usdc: 0.01,
            required_child_spend_usdc: 0.00,
            sponsored_claim_gas: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bounty {
    pub id: String,
    pub config: BountyConfig,
    pub acceptance_criteria: Vec<String>,
    pub funded: bool,
    pub verifier_ready: bool,
    pub canonical: bool,
    pub indexed: bool,
}

impl Bounty {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            config: BountyConfig::default(),
            acceptance_criteria: Vec::new(),
            funded: false,
            verifier_ready: false,
            canonical: false,
            indexed: false,
        }
    }

    pub fn with_config(mut self, config: BountyConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_acceptance_criteria(mut self, criteria: Vec<String>) -> Self {
        self.acceptance_criteria = criteria;
        self
    }

    pub fn with_funded(mut self, funded: bool) -> Self {
        self.funded = funded;
        self
    }

    pub fn with_verifier_ready(mut self, ready: bool) -> Self {
        self.verifier_ready = ready;
        self
    }

    pub fn with_canonical(mut self, canonical: bool) -> Self {
        self.canonical = canonical;
        self
    }

    pub fn with_indexed(mut self, indexed: bool) -> Self {
        self.indexed = indexed;
        self
    }

    pub fn is_earning_ready(&self) -> bool {
        self.funded && self.verifier_ready && self.canonical && self.indexed
    }
}

pub struct BountyManager {
    bounties: Arc<RwLock<HashMap<String, Bounty>>>,
}

impl Default for BountyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BountyManager {
    pub fn new() -> Self {
        Self {
            bounties: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create_bounty(&self, bounty: Bounty) {
        let mut bounties = self.bounties.write().unwrap();
        bounties.insert(bounty.id.clone(), bounty);
    }

    pub fn get_bounty(&self, id: &str) -> Option<Bounty> {
        let bounties = self.bounties.read().unwrap();
        bounties.get(id).cloned()
    }

    pub fn get_earning_ready_bounties(&self) -> Vec<Bounty> {
        let bounties = self.bounties.read().unwrap();
        bounties
            .values()
            .filter(|b| b.is_earning_ready())
            .cloned()
            .collect()
    }

    pub fn seed_direct_coding_bounties(&self, count: usize) -> Vec<String> {
        let mut ids = Vec::new();
        
        for i in 0..count {
            let id = format!("direct-coding-bounty-{}", i + 1);
            let bounty = Bounty::new(&id)
                .with_config(BountyConfig::default())
                .with_acceptance_criteria(vec![
                    "All tests pass".to_string(),
                    "Code follows style guidelines".to_string(),
                    "No breaking changes".to_string(),
                ]);
            
            self.create_bounty(bounty);
            ids.push(id);
        }
        
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_bounty_config() {
        let config = BountyConfig::default();
        assert_eq!(config.total_funding_usdc, 2.00);
        assert_eq!(config.solver_reward_usdc, 1.99);
        assert_eq!(config.claim_bond_usdc, 0.01);
        assert_eq!(config.required_child_spend_usdc, 0.00);
        assert!(config.sponsored_claim_gas);
    }

    #[test]
    fn test_bounty_earning_ready() {
        let bounty = Bounty::new("test-1")
            .with_funded(true)
            .with_verifier_ready(true)
            .with_canonical(true)
            .with_indexed(true);
        
        assert!(bounty.is_earning_ready());
    }

    #[test]
    fn test_bounty_not_earning_ready() {
        let bounty = Bounty::new("test-1")
            .with_funded(true)
            .with_verifier_ready(false);
        
        assert!(!bounty.is_earning_ready());
    }

    #[test]
    fn test_seed_direct_coding_bounties() {
        let manager = BountyManager::new();
        let ids = manager.seed_direct_coding_bounties(5);
        
        assert_eq!(ids.len(), 5);
        
        for id in &ids {
            let bounty = manager.get_bounty(id).unwrap();
            assert_eq!(bounty.config.total_funding_usdc, 2.00);
            assert_eq!(bounty.config.solver_reward_usdc, 1.99);
            assert_eq!(bounty.acceptance_criteria.len(), 3);
        }
    }

    #[test]
    fn test_earning_ready_filter() {
        let manager = BountyManager::new();
        
        manager.create_bounty(
            Bounty::new("ready")
                .with_funded(true)
                .with_verifier_ready(true)
                .with_canonical(true)
                .with_indexed(true)
        );
        
        manager.create_bounty(
            Bounty::new("not-ready")
                .with_funded(true)
        );
        
        let ready = manager.get_earning_ready_bounties();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "ready");
    }
}
