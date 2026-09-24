//! Core library for the project.
//!
//! This module provides the public API used by the unit tests. The previous
//! implementation contained an infinite loop which caused `cargo test` to
//! time‑out. The functions below are now implemented correctly, are fully
//! documented, and include basic input validation.

/// Returns the sum of all integers from `1` up to and including `n`.
///
/// # Arguments
///
/// * `n` – The upper bound of the range (must be greater than or equal to `1`).
///
/// # Examples
///
/// 