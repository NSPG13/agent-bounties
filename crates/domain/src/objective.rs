
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

// Assuming ObjectiveId and other necessary types are defined elsewhere in the crate
// or directly in this file. For this change, we only modify the Objective struct itself.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Objective {
    pub id: ObjectiveId,
    pub title: String,
    pub description: String,
    // ... (existing fields) ...
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    /// Indicates if this bounty is specifically related to ChatGPT integration.
    /// This can be used for specialized lifecycle tracking, display, and shareable card generation.
    #[serde(default)]
    pub is_chatgpt_bounty: bool,

    /// A notice intended for maintainers, providing context or instructions related
    /// to the bounty's lifecycle, status, or how its shareable card should be handled.
    pub maintainer_notice: Option<String>,
}
    