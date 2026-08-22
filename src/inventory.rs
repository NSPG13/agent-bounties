use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    pub id: String,
    pub funded: bool,
    pub claimable: bool,
}

impl Replacement {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            funded: false,
            claimable: false,
        }
    }

    pub fn with_funded(mut self, funded: bool) -> Self {
        self.funded = funded;
        self
    }

    pub fn with_claimable(mut self, claimable: bool) -> Self {
        self.claimable = claimable;
        self
    }
}

pub struct InventoryManager {
    replacements: Arc<RwLock<HashMap<String, Replacement>>>,
}

impl Default for InventoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl InventoryManager {
    pub fn new() -> Self {
        Self {
            replacements: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_replacement(&self, replacement: Replacement) {
        let mut replacements = self.replacements.write().unwrap();
        replacements.insert(replacement.id.clone(), replacement);
    }

    pub fn reconcile_funded_replacements(&self) -> Vec<Replacement> {
        let replacements = self.replacements.read().unwrap();
        replacements
            .values()
            .filter(|r| r.funded)
            .cloned()
            .collect()
    }

    pub fn get_claimable_feed(&self) -> Vec<Replacement> {
        let replacements = self.replacements.read().unwrap();
        replacements
            .values()
            .filter(|r| r.funded && r.claimable)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replacement_builder() {
        let replacement = Replacement::new("test-1")
            .with_funded(true)
            .with_claimable(true);
        
        assert_eq!(replacement.id, "test-1");
        assert!(replacement.funded);
        assert!(replacement.claimable);
    }

    #[test]
    fn test_inventory_manager_add() {
        let manager = InventoryManager::new();
        let replacement = Replacement::new("test-1").with_funded(true);
        
        manager.add_replacement(replacement);
        let funded = manager.reconcile_funded_replacements();
        
        assert_eq!(funded.len(), 1);
        assert_eq!(funded[0].id, "test-1");
    }

    #[test]
    fn test_claimable_feed_filter() {
        let manager = InventoryManager::new();
        
        manager.add_replacement(
            Replacement::new("funded-only").with_funded(true)
        );
        manager.add_replacement(
            Replacement::new("funded-claimable")
                .with_funded(true)
                .with_claimable(true)
        );
        
        let claimable = manager.get_claimable_feed();
        
        assert_eq!(claimable.len(), 1);
        assert_eq!(claimable[0].id, "funded-claimable");
    }
}
