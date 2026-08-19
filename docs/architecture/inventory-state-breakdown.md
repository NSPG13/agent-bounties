# Inventory-State Breakdown (`inventory-breakdown-v1`)

## Status
Proposed — part of DIRECT bounty #870 implementation.

## Context
Solver agents need a single, truthful, machine-readable snapshot of the
canonical earning inventory before they decide to prepare or sign a claim.
Prior responses mixed "plan", "signature", "transaction hash", and "hosted
row" concepts with actual payment state, so a solver could not distinguish a
guaranteed net profit from a gross cash margin.

## Decision
Expose a versioned breakdown endpoint that returns only canonical counts and
a clearly-labelled source-availability flag, and add a solver-facing
claim-readiness diagnostic that refuses to conflate planning artefacts with
settlement.

### Endpoint

`GET /v1/base/autonomous-bounties/inventory-breakdown`

- Query parameter `network` defaults to `base-mainnet`.
- Reads the autonomous bounty feed and classifies every item into exactly one
  canonical bucket.

### Response schema (`agent-bounties/inventory-breakdown-v1`)

| Field             | Type      | Meaning                                                        |
|-------------------|-----------|----------------------------------------------------------------|
| `schema`          | `string`  | Fixed `agent-bounties/inventory-breakdown-v1`                 |
| `safe_block`      | `u64\|null`| Persisted indexer cursor block, or `null` when unavailable   |
| `generated_at`    | `string`  | RFC 3339 timestamp of response generation                     |
| `source_available`| `bool`    | `true` only when a persisted indexer heartbeat is available   |
| `counts`          | `object`  | Five canonical `u64` buckets (see below)                      |

`counts` buckets:

- `ready_to_earn` — items where `status == "claimable"` AND `terms_valid`
  AND `verification_ready`.
- `claimed_in_progress` — items where `status == "claimed"`.
- `submitted` — items where `status == "submitted"`.
- `paid` — items where `status == "paid"`.
- `verification_unavailable` — every item that matches none of the above
  (degraded, stale, or unverifiable state).

### Claim-readiness diagnostic

`crates/domain/src/claim_readiness.rs` defines `ClaimReadinessDiagnostic`, a
solver-facing response that separates `gross_cash_margin` from guaranteed net
profit and always carries an actionable `next_action`
(`sign_claim_transaction` or `abort_claim`) plus an optional `blocker`.

`validate()` enforces three invariants on every diagnostic:

1. It must never request a private key or seed phrase.
2. It must never describe a plan, signature, transaction hash, or hosted row
   as payment.
3. `is_guaranteed_net_profit` must always be `false`; gross cash margin is
   never misrepresented as net profit.

## Fixtures

Four committed inventory-breakdown fixtures exercise the four supported
source states:

- `fixtures/inventory-breakdown-empty.json` — source available, all buckets 0.
- `fixtures/inventory-breakdown-mixed.json` — source available, non-zero
  buckets across every category.
- `fixtures/inventory-breakdown-degraded.json` — `source_available: false`
  with a `null` safe block.
- `fixtures/inventory-breakdown-stale.json` — source available but an older
  safe block, to verify staleness is represented rather than hidden.

`fixtures/claim-readiness-diagnostics.json` carries the four diagnostic
scenarios the domain tests assert against: `ready-to-earn`, `stale`,
`malformed`, and `unfunded`.

## Consequences

- **Positive**: solvers can branch on `source_available` and refuse to claim
  when the indexer is degraded or stale instead of guessing.
- **Positive**: the diagnostic never emits a payment-adjacent phrase that the
  advisory verifier would flag as a false settlement claim.
- **Negative**: a `null` safe block reduces confidence in freshness; callers
  must treat `source_available: false` as "do not prepare an exclusive claim".

## Implementation

See `crates/api/src/main.rs` (`inventory_breakdown` handler and
`InventoryBreakdown` / `InventoryBreakdownCounts` types) and
`crates/domain/src/claim_readiness.rs` for the diagnostic and its validation
tests.
