use std::sync::Arc;

const CANONICAL_ROUTED_V3_BOOTSTRAP_BLOCK: u64 = 18_500_000;

pub struct BootstrapConfig {
    canonical_block: u64,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            canonical_block: CANONICAL_ROUTED_V3_BOOTSTRAP_BLOCK,
        }
    }
}

impl BootstrapConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn canonical_block(&self) -> u64 {
        self.canonical_block
    }

    pub fn with_canonical_block(mut self, block: u64) -> Self {
        self.canonical_block = block;
        self
    }
}

pub struct BootstrapManager {
    config: Arc<BootstrapConfig>,
}

impl BootstrapManager {
    pub fn new(config: BootstrapConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub fn get_bootstrap_block(&self) -> u64 {
        self.config.canonical_block()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_canonical_block() {
        let config = BootstrapConfig::default();
        assert_eq!(config.canonical_block(), CANONICAL_ROUTED_V3_BOOTSTRAP_BLOCK);
    }

    #[test]
    fn test_custom_canonical_block() {
        let config = BootstrapConfig::new().with_canonical_block(20_000_000);
        assert_eq!(config.canonical_block(), 20_000_000);
    }

    #[test]
    fn test_bootstrap_manager() {
        let config = BootstrapConfig::default();
        let manager = BootstrapManager::new(config);
        assert_eq!(manager.get_bootstrap_block(), CANONICAL_ROUTED_V3_BOOTSTRAP_BLOCK);
    }
}
