
use serde::{Deserialize, Serialize};
use uuid::Uuid; // Assuming 'uuid' crate is a dependency for unique identifiers

// Existing imports and types...

/// Represents the unique identifier for a user.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

/// Represents the unique identifier for an AI account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AiAccountId(pub Uuid);

/// Defines the entity responsible for maintaining an objective (bounty).
/// This enum is extended to support user-owned AI accounts as a maintainer.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Maintainer {
    /// A GitHub user account.
    GitHubUser(String),
    /// A GitHub organization account.
    Organization(String),
    // Add other existing maintainer types here if they exist
    // Example: DirectWallet(String),

    /// A maintainer interface controlled by a user-owned AI account.
    /// This variant represents the new default bounty interface.
    UserOwnedAiAccount {
        /// The ID of the user who owns the AI account.
        user_id: UserId,
        /// The ID of the AI account acting as the maintainer.
        ai_account_id: AiAccountId,
    },
}

// Example Objective struct (assuming it exists and references Maintainer)
// If the Objective struct or similar bounty definition does not exist
// or does not have a 'maintainer' field, it would need to be added or
// adjusted to include the Maintainer enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectiveId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective {
    pub id: ObjectiveId,
    pub title: String,
    pub description: String,
    pub maintainer: Maintainer, // Ensure this field exists or is added
    // Other fields related to an objective...
}

// Other structs and enums related to Objective...
