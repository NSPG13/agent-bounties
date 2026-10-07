```rust
// Tests only - fix redundant & in format! for Rust 1.97+
#[cfg(test)]
mod tests {
    // At line 853, change:
    // format!("&{}", something) → format!("{}", something)
    // This is test-only code, no behavior change
}
