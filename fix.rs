```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Module representing the canonical inventory projection response.
/// Fixes the confusion between active work and claimable work.
pub mod canonical_inventory {

    /// Represents the lifecycle state of the projection (e.g., Empty, Mixed, Degraded, Stale).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ProjectionLifecycle {
        /// The canonical projection is current and fully synchronized.
        Verified,
        /// The projection is mostly good but has some stale data (e.g., one late event).
        Degraded,
        /// The projection contains only a subset of the data (e.g., pre-initialization).
        Stale,
        /// The projection is empty but structurally valid (common for "empty" fixtures).
        Empty,
        /// Generic catch-all for unknown canonical states.
        #[default]
        Canon,
    }

    /// Represents the L2 block or indexed generation timestamp context.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct SafeBlockHeader {
        /// The L2 Safe Block number (e.g., from Safe/Alchemy/Covalent or Base Mainnet)
        pub number: u64,
        /// The timestamp associated with the indexed generation.
        pub timestamp: u64, // in seconds since epoch
    }

    /// Represents where the data is coming from and its availability confidence.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct SourceAvailability {
        /// Is the source currently active/listening?
        pub is_active: bool,
        /// The network identifier (e.g., "base-mainnet").
        pub network: String,
    }

    /// Aggregates the specific breakdown counts required by the spec.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct InventoryCounts {
        /// Strict ready-to-earn filtering. The pool of claimable assets.
        pub ready_to_earn: u64,
        /// Assets currently in progress of being claimed.
        pub claimed: u64,
        /// Assets submitted to a verifier or solver.
        pub submitted: u64,
        /// Assets where payment has fully settled.
        pub paid: u64,
        /// Assets waiting on verification signal availability.
        pub verification_unavailable: u64,
    }

    /// The Versioned Response derived from one accepted canonical inventory snapshot.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct BreakdownResponse {
        /// Deterministic version for cache-busting and versioning.
        #[serde(rename = "version")]
        pub version: u16,
        
        /// The observed safe block or indexed generation timestamp.
        #[serde(flatten)]
        pub header: SafeBlockHeader,
        
        /// The source availability context.
        #[serde(flatten)]
        pub source: SourceAvailability,
        
        /// The core breakdown counts.
        pub counts: InventoryCounts,
        
        /// The lifecycle state (Empty, Mixed, etc) to handle edge cases.
        #[serde(default)]
        pub lifecycle: ProjectionLifecycle,
    }

    /// A trait to allow different storage backends or projections to implement this response.
    pub trait IntoBreakdown: Clone + Send + Sync + 'static {
        /// Converts self into the canonical breakdown response.
        fn into_breakdown(&self, network: &str) -> BreakdownResponse;
    }

    impl IntoBreakdown for BreakdownResponse {
        fn into_breakdown(&self, network: &str) -> BreakdownResponse {
            BreakdownResponse {
                version: self.version,
                header: self.header,
                source: SourceAvailability {
                    is_active: self.header.timestamp > 0,
                    network: network.to_string(),
                },
                counts: self.counts,
                lifecycle: self.lifecycle,
            }
        }
    }

    /// Helper to construct the response from raw state logic.
    /// Handles the "Strict ready-to-earn filtering unchanged" requirement.
    pub fn construct_breakdown(
        ready_to_earn: u64,
        claimed: u64,
        submitted: u64,
        paid: u64,
        verification_unavailable: u64,
        safe_block: u64,
        timestamp: u64,
        network: &str,
        lifecycle: ProjectionLifecycle,
    ) -> BreakdownResponse {
        BreakdownResponse {
            version: 2, // Increment from legacy V1
            header: SafeBlockHeader {
                number: safe_block,
                timestamp,
            },
            source: SourceAvailability {
                is_active: true,
                network: network.to_string(),
            },
            counts: InventoryCounts {
                ready_to_earn,
                claimed,
                submitted,
                paid,
                verification_unavailable,
            },
            lifecycle,
        }
    }

    // --- Deterministic Fixtures ---
    // Used for the "Empty, Mixed, Degraded, and Stale" acceptance criteria.

    impl Default for BreakdownResponse {
        fn default() -> Self {
            BreakdownResponse {
                version: 1,
                header: SafeBlockHeader {
                    number: 85000,
                    timestamp: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_else(|e| e)
                        .as_secs(),
                },
                source: SourceAvailability {
                    is_active: true,
                    network: "base-mainnet".to_string(),
                },
                counts: InventoryCounts {
                    ready_to_earn: 10,
                    claimed: 5,
                    submitted: 2,
                    paid: 1,
                    verification_unavailable: 1,
                },
                lifecycle: ProjectionLifecycle::Canon,
            }
        }
    }

    impl BreakdownResponse {
        /// Creates a fixture for an "Empty" projection (e.g., start of epoch).
        pub fn fixture_empty() -> Self {
            construct_breakdown(
                0, 0, 0, 0, 0, 0,
                "base-mainnet",
                ProjectionLifecycle::Empty,
            )
        }

        /// Creates a fixture for a "Mixed" projection (the usual case).
        pub fn fixture_mixed() -> Self {
            construct_breakdown(
                3, 1, 1, 1, 1,
                85001,
                "base-mainnet",
                ProjectionLifecycle::Mixed,
            )
        }

        /// Creates a fixture for a "Degraded" projection (e.g., L2 sync lag).
        pub fn fixture_degraded() -> Self {
            construct_breakdown(
                2, 1, 1, 1, 1, // One extra verification_unavailable
                85002,
                "base-mainnet",
                ProjectionLifecycle::Degraded,
            )
        }

        /// Creates a fixture for a "Stale" projection (e.g., post-settlement).
        pub fn fixture_stale() -> Self {
            construct_breakdown(
                1, 0, 0, 0, 1, // Verification still pending
                85003,
                "base-mainnet",
                ProjectionLifecycle::Stale,
            )
        }
    }

    // --- Implementation for the "One Current Canonical Projection" ---
    
    /// A struct representing the canonical source of truth (The "Canonical" itself).
    pub struct CanonicalInventorySnapshot {
        /// The raw inventory pool map.
        pub raw_pool: HashMap<String, u64>,
        pub block: u64,
        pub timestamp: u64,
    }

    impl CanonicalInventorySnapshot {
        pub fn new(raw_pool: HashMap<String, u64>, block: u64, timestamp: u64) -> Self {
            Self { raw_pool, block, timestamp }
        }

        /// Projects the raw pool into the specific BreakdownResponse counts.
        pub fn project_breakdown(&self) -> BreakdownResponse {
            let counts = InventoryCounts {
                // Summing up the raw counts
                ready_to_earn: self.raw_pool.get("ready").unwrap_or(&0),
                claimed: self.raw_pool.get("claimed").unwrap_or(&0),
                submitted: self.raw_pool.get("submitted").unwrap_or(&0),
                paid: self.raw_pool.get("paid").unwrap_or(&0),
                verification_unavailable: self.raw_pool.get("verified").unwrap_or(&0),
            };

            BreakdownResponse {
                version: 3,
                header: SafeBlockHeader {
                    number: self.block,
                    timestamp: self.timestamp,
                },
                source: SourceAvailability {
                    is_active: true,
                    network: "base-mainnet".to_string(),
                },
                counts,
                lifecycle: ProjectionLifecycle::Verified,
            }
        }
    }

    // --- Re-export convenience for the Homepage ---
    pub use BreakdownResponse;

}
```