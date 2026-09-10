# Posting journey release evidence

PR #1404 refreshes the reviewed worker-build fingerprint for its posting
journey backend and MCP descriptor changes. The complete guard input set contains
164 files: the Cargo workspace, lockfile, configuration and local crates, plus
literal Rust compile-time includes. The reviewed changed inputs are
`Cargo.lock`, `crates/api/src/site_auth.rs`, `crates/db/Cargo.toml`,
`crates/db/src/lib.rs`, `crates/db/src/site_posting_drafts.rs`,
`crates/mcp-server/src/chatgpt_app.rs`, `crates/mcp-server/src/main.rs`,
`crates/mcp-server/fixtures/public-mcp-contract-v1.json`,
`crates/mcp-server/fixtures/tool-registry.json`,
`migrations/0036_site_posting_drafts.sql`, and
`migrations/0037_site_wallet_provider.sql`. Draft canonicalization adds pinned
`serde_jcs 0.2.0` and its locked `ryu-js 0.2.2` dependency; existing dependency
versions are unchanged. The cached, checksum-locked `serde_jcs 0.2.0` manifest
enables `serde_json`'s `float_roundtrip` feature through its dependency, without
a workspace manifest feature change.

The worker-build digest changes from
`41c015895864373c73125be5ba1fc87576270128ebde84f7a93bf7898754f9d2` to
`46d909540dc46a34d2de4ac4b820683b8854c941ac69e43d3c529e2b2bf80902`.
Only the eight expected worker-build digests in the runner, signer and reusable
signing workflows are refreshed. The signing-runtime digest remains
`469bf155b1bbc5f19ee91ee41172e113cd5baea6f9d1f2d574d88672b1999ddc`;
guard code, source scopes, before/after-build validation, authorization,
signing and secret gates remain unchanged.

Local validation passed all 12 source-guard tests, all 28 pipeline tests, and
both expected-digest checks. Pipeline tests use fixtures; their simulated
relay output is not an on-chain transaction. The full MCP suite passed 85 tests
with one intentional digest-printer test ignored, including both catalog
digest guards and creator-handoff output coverage.

Before merge or deployment, require passing source-guard and pipeline tests,
both expected-digest checks, `cargo test -p verifier-sdk -p worker`, and the
complete required CI suite on the final release revision. A matching digest
does not prove verifier correctness or authorize signing, relay, or payments.
Any subsequent guarded input edit requires review, a fresh digest, synchronized
workflow pins and these checks again. No live signing or relay is part of this
release validation.
