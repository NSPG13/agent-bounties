# ADR 0001: Bounded Self-Healing Control Loop

- Status: accepted for implementation
- Date: 2026-07-13
- Change class: R2, with R3-R4 exclusions
- Notice: <https://github.com/NSPG13/agent-bounties/issues/228>

## Context

Agent Bounties has deterministic payment invariants, deployment attestations,
production smoke tests, and indexer heartbeats, but runtime recovery was not one
coherent contract. API and MCP relied on platform restarts, the indexer exited
on its first transient failure, and incidents did not feed a dedicated recovery
corpus. Calling broad autonomous mutation "self-healing" would be especially
unsafe because hosted coordination is adjacent to real USDC and immutable
contracts.

## Decision

Adopt one bounded loop:

`observe -> diagnose -> plan -> contain/apply -> verify -> learn`

- Encode automatic actions and prerequisites in a source-controlled JSON
  policy.
- Permit only R0-R2 read-only, ephemeral, or idempotent/rebuildable actions.
- Use Render health supervision for API/MCP process replacement.
- Retry transient indexer failures from the persisted monotonic cursor with
  capped exponential backoff; exit after a bounded budget for supervisor
  replacement.
- Run scheduled read-only production probes and retain snapshots/plans.
- Fail closed on revision skew, cursor/event/hash/ledger/payment inconsistency,
  database restore, secrets, access, wallet, verifier, settlement, and contract
  operations.
- Convert every incident into a deterministic RecoveryBench fixture. Require a
  perfect corpus score.

## Alternatives Rejected

### Let an AI choose and execute any repair

Rejected because model output is nondeterministic, may lack telemetry, and has
no legitimate custody or protocol authority.

### Restart every failing component indefinitely

Rejected because persistent dependency or integrity faults become restart
storms and hide the root cause.

### Put all recovery logic in the hosting provider

Rejected because provider health checks cannot validate canonical event,
contract, verifier, ledger, or payment evidence.

### Require humans for every transient failure

Rejected because read retries, stateless restarts, and monotonic idempotent
index replay are measurable and safe to automate.

## Consequences

- Availability recovery becomes faster and testable.
- Automatic action scope remains intentionally smaller than the full incident
  runbook.
- Trusted internal observations are required before any recovery that depends
  on canonical integrity; public HTTP probes cannot assume those facts.
- New incident classes must update policy, fixtures, docs, and gates together.
- "Healthy" still does not mean funded, verified, paid, secure, or compliant.
