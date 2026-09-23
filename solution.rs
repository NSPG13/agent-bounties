//! Core library for the project.
//!
//! This module provides the primary functionality used by the binary
//! and the test suite. The original implementation contained an
//! infinite loop in `process_items`, which caused the test suite to
//! time‑out. The function has been rewritten to correctly iterate over
//! the supplied items and return a result without hanging.

/// Represents an item that can be processed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// A simple identifier for the item.
    pub id: usize,
    /// The payload associated with the item.
    pub payload: String,
}

/// Processes a slice of `Item`s and returns a vector containing the
/// `payload`s of items whose `id` is even. The previous implementation
/// used a `while true` loop with no break condition, which caused an
/// infinite loop and consequently a test timeout.
///
/// # Examples
///
/// 