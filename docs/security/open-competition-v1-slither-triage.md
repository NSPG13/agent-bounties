# Open Competition V1 Slither Triage

Date: 2026-08-07

Tool: Slither `0.11.6`, Foundry `1.7.1`, Solidity `0.8.26`.

Targets were analyzed separately with dependencies excluded:

- `OpenCompetitionBountyV1.sol`: 12 findings
- `OpenCompetitionBountyFactoryV1.sol`: 13 findings
- `LeadingZeroWorkVerifier.sol`: 1 Informational finding

There are no High or Medium findings. The 2026-08-07 rerun analyzed each
target separately against frozen source commit
`bc9b3cc9f9f95a87df671be2d13199ac9d06ebcf`; the additional result count is
within the already-triaged detector classes below and does not reflect a
contract-source change.

## Triaged Findings

### Missing arithmetic events

Slither reports the bounty clone's initialization assignments. The canonical
factory emits `CanonicalCompetitionCreated`,
`CanonicalCompetitionTermsCommitted`,
`CanonicalCompetitionEconomicsConfigured`, and
`CanonicalCompetitionVerificationConfigured` after initialization. Those
events contain the assigned rewards, target, deadlines, windows, capacity,
verifier, and hashes. Indexing requires those factory events and rejects a
non-canonical clone, so adding duplicate clone-origin events would weaken the
single canonical event boundary.

Disposition: expected architecture; covered by factory configuration and
version-specific decoder tests.

### Timestamp comparisons

The flagged comparisons are protocol deadlines: funding close, competition
close, reveal expiry, commitment expiry, and cancellation availability. They
do not generate randomness or choose a winner. Winner ordering is the first
confirmed passing reveal transaction; copied reveals additionally require a
commitment from an earlier block.

Disposition: expected deadline semantics; covered by boundary, expiry,
same-block, conservation, and cancellation tests.

### Deterministic-clone assembly

The factory uses the standard minimal-proxy creation code with `CREATE2` and a
memory-safe assembly block. Address prediction hashes the same creation code.
The hosted catalog independently pins the implementation and deployed runtime,
and the deployment bundle predicts the implementation and factory addresses.

Disposition: expected minimal-proxy construction; covered by prediction,
canonical registration, runtime-pin, and deployment-bundle tests.

### Safe token low-level call

`SafeBountyToken` deliberately uses a low-level call to support native USDC's
standard return behavior while rejecting a reverted call, a false return, or a
malformed return. Funding records are written only after exact balance movement
is observed. A token that reports success without transferring is rejected by
the contract test suite.

Disposition: intentional compatibility wrapper with strict semantic checks.

### Refund bonus local

Slither reports the local refund `bonus` as uninitialized. Solidity initializes
local value types to zero; the variable is assigned before use whenever a bonus
exists, and zero is the intended value otherwise. The cancellation/refund tests
cover both paths. The source remains byte-for-byte aligned with the existing
bounded-wallet deployment evidence rather than changing deployed-source hashes
for a cosmetic initialization.

Disposition: false positive under Solidity value-type initialization rules.

This triage is static-analysis evidence, not an independent contract review and
not permission to activate mainnet inventory.
