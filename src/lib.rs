pub mod bootstrap;
pub mod inventory;
pub mod bounty;

pub use bootstrap::{BootstrapConfig, BootstrapManager};
pub use inventory::{InventoryManager, Replacement};
pub use bounty::{Bounty, BountyConfig, BountyManager};

pub struct MaintenanceTask {
    bootstrap: BootstrapManager,
    inventory: InventoryManager,
    bounty: BountyManager,
}

impl MaintenanceTask {
    pub fn new() -> Self {
        let bootstrap_config = BootstrapConfig::default();
        Self {
            bootstrap: BootstrapManager::new(bootstrap_config),
            inventory: InventoryManager::new(),
            bounty: BountyManager::new(),
        }
    }

    pub fn execute_issue_629(&self) -> MaintenanceResult {
        let canonical_block = self.bootstrap.get_bootstrap_block();
        
        let funded_replacements = self.inventory.reconcile_funded_replacements();
        let claimable_feed = self.inventory.get_claimable_feed();
        
        let bounty_ids = self.bounty.seed_direct_coding_bounties(5);
        
        let earning_ready = self.bounty.get_earning_ready_bounties();
        
        MaintenanceResult {
            canonical_block,
            funded_replacements_count: funded_replacements.len(),
            claimable_feed_count: claimable_feed.len(),
            created_bounties: bounty_ids,
            earning_ready_count: earning_ready.len(),
        }
    }
}

impl Default for MaintenanceTask {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceResult {
    pub canonical_block: u64,
    pub funded_replacements_count: usize,
    pub claimable_feed_count: usize,
    pub created_bounties: Vec<String>,
    pub earning_ready_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maintenance_task_execution() {
        let task = MaintenanceTask::new();
        let result = task.execute_issue_629();
        
        assert_eq!(result.canonical_block, 18_500_000);
        assert_eq!(result.created_bounties.len(), 5);
    }

    #[test]
    fn test_maintenance_task_default() {
        let task = MaintenanceTask::default();
        let result = task.execute_issue_629();
        
        assert_eq!(result.created_bounties.len(), 5);
    }
}
