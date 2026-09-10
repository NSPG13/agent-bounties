# Posting journey release evidence

PR #1404 refreshes the reviewed worker-build fingerprint for its posting
journey backend and MCP descriptor changes. The complete guard input set contains
164 files: the Cargo workspace, lockfile, configuration and local crates, plus
literal Rust compile-time includes. The reviewed changed inputs are
`Cargo.lock`, `crates/api/src/site_auth.rs`, `crates/db/Cargo.toml`,
`crates/cli/src/main.rs`,
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

The CLI documentation validator now reads routes from the real `site_auth.rs`
module, including session and posting-draft routes. Bounded external-PR source
staging and its exact source-list fixture include the same module; existing
size, encoding, symlink and root-containment checks remain in force.
All eight staging tests pass. The compiled CLI documentation check passes
against 157 documents, 339 routes and 129 MCP names, and both CLI demos pass.

The worker-build digest changes from
`41c015895864373c73125be5ba1fc87576270128ebde84f7a93bf7898754f9d2` to
`25f338acf74ba2a612cccdf5dc4e167ac3d0e929f2d0ad1d6850ca701f172821`.
Production workflow edits refresh the eight expected worker-build digests in
the runner, signer and reusable signing workflows. The signing-runtime digest remains
`469bf155b1bbc5f19ee91ee41172e113cd5baea6f9d1f2d574d88672b1999ddc`;
guard code, source scopes, before/after-build validation, authorization,
signing and secret gates remain unchanged.

The 1200-second watchdog `unfunded_precommit` also receives the same reviewed
worker-build pin, its three derived fixture workflow hashes, and six rehearsal
pin literals, following the prior refresh in commit `a7c2c8e` (PR #1360).
Its exact benchmark directory digest changes from
`sha256:250a62f546a538350ce6f7d00040286b260e7361270fd5fe3ee35a628997a18f`
to `sha256:b7b7a4850daa40c2537c55a4e80136eb7c63a61444e28d384bb6658e1621cf82`.
The activation record explicitly classifies this older precommit as
non-authoritative for the separate immutable 900-second catalog tuple. That
catalog, all historical benchmark bindings, checker logic, runtime hashes,
economics, verifier identity and funding gates are unchanged. This refresh
does not activate or fund the precommit.

The exact refreshed directory digest and all four precommit tests pass. The
full rehearsal accepts known-good behavior and rejects the adversarial
mutations and known-bad behavior. An independent read-only release check found
no matching watchdog binding in the indexed public inventory, with complete
terms for this creator's 17 entries and no new factory creation in the gap to
Base safe block 51113972. This is not a claim that all historical bindings are
absent; every historical binding remains unchanged.

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
