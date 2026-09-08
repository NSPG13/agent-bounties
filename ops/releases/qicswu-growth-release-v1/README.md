# QICSWU growth release v1

Status: **release candidate; not deployed**.

This package contains exactly three independently deployable product slices with scoped application rollback. Slice 3's additive analytics allowlist migration is intentionally forward-only and remains inert if its producers are rolled back. The slices are ordered by dependency on trustworthy measurement, then supply-side actionability, then preparation for a separately authorized external-demand activation:

1. `qicswu-metric-v1` — a versioned, fail-closed north-star contract, deterministic evaluator, qualification ledger, frozen baseline, and read-only scheduled production shadow. Open Competition is its primary participation path; both competition protocols and the legacy exclusive-claim lane remain measured without reweighting.
2. `truthful-ready-to-earn-v1` — remove work that fails the documented structural, state, time, action-allowlist, or applicable canonical-margin checks from the agent-facing `ready_to_earn` view.
3. `pinned-regression-post-fund-handoff-v1` — install the immutable verifier handoff contract, fail-closed browser/MCP behavior, and boundary telemetry while the production catalog remains explicitly inactive and every such handoff remains draft-only.

The release does not deploy code, publish a metric, contact users, or move funds. It makes no claim of actual production uplift.

## Release invariant

The frozen 28-complete-UTC-day window is `[2026-08-04T00:00:00Z,2026-09-01T00:00:00Z)`. The retained evidence contains an aggregate-aligned diagnostic set of 26 noncanary settlements and 36.465 USDC, but incomplete historical-factory coverage means this is not a complete exact-window gross observation and it is not the north-star value. The policy-qualified value is `null` / `unavailable` until source coverage, beneficial-principal ownership, reimbursement relationships, pre-participation funding, root-work lineage, standalone value, finality, receipts, contract state, and complete lifecycle evidence are reconciled for every candidate.

A missing field is unknown, never zero. A wallet not on the operator exclusion list is not thereby independent. A transaction hash is not settlement evidence. Only the relevant final canonical settlement event can prove payment.

## Global rollout order

1. Lock the candidate commit, run the static gate specification in `docs/qicswu-growth-release-evidence.md`, and retain the machine reports outside the fingerprinted source tree, keyed to the exact commit and source-tree hash.
2. Approve and install slice 1 as an internal measurement artifact. Keep its public value unavailable while the evidence gate is incomplete.
3. Deploy slice 2 to the API independently and monitor the actionability distribution before expanding traffic.
4. Deploy slice 3 site assets first, then the MCP server. The current pinned catalogs remain `inactive_preconditions_unmet`, so both surfaces must keep the handoff draft-only and must not expose wallet continuation.
5. Stop this release at the draft-only boundary. The checked-in [activation contract](pinned-regression-catalog-activation-v1.json) is blocked, has null gate evidence, and authorizes nothing. Run the activation experiment only after a separate catalog-activation release satisfies every catalog gate, including the missing cohort-measurement contract, publishes a new synchronized MCP/browser catalog version and hash, passes its own locked verification, and receives the required live-funds approval.

The slices may stop after any numbered step. None requires the following slice to be safe or useful.

Slice 3 is not an activation release. Neither deployment of these files nor a synthetic local-fork pass authorizes changing the current catalog to `active`, exposing a production wallet path, or counting funding-ready traffic.

## Global stop conditions

Stop rollout and use the affected slice's rollback if any of these occurs:

- an incomplete or crowdfunded draft can expose wallet continuation;
- an unsupported action routes to a generic or missing page;
- `canonical_post_confirmed` is emitted without indexed creation, funding, and claimability evidence;
- `ready_to_earn` contains an action that fails its documented local structural, state, time, action-allowlist, or applicable canonical-margin predicate;
- a metric input is unknown but the evaluator publishes a numeric north-star value;
- policy or artifact hashes drift without an explicit version change;
- error rate, latency, or zero-inventory guardrails breach the approved release threshold.

## Rollback order

Each runbook has a scoped rollback. For a complete release rollback, revert slice 3 MCP first, then slice 3 site assets, then slice 2 API, then stop new slice 1 evaluations. Retain already generated measurement artifacts as immutable audit evidence; do not rewrite a released v1 policy or historical result.

## Approval boundary

At minimum, release requires a maintainer, a protocol/payment-invariant reviewer, and the owner named in each slice runbook. Production deployment and any experiment involving live funds require separate explicit authorization. The existing public notices [#1117](https://github.com/NSPG13/agent-bounties/issues/1117), [#1183](https://github.com/NSPG13/agent-bounties/issues/1183), and [#1122](https://github.com/NSPG13/agent-bounties/issues/1122) provide applicable public context; this package does not create a new notice or change their status.

## Verification boundary

The repository has strong component and composite gates. Even when every listed component passes, that is not a claim that one continuous production path ran from MCP request through browser wallet, Base-fork transaction, indexer ingestion, discovery, verification, settlement, and reconciled metric output. Only the ignored or external direct-run attestation for the exact clean commit can establish the named same-identity synthetic local-fork boundaries; the tracked gate table remains static.

Provision and run the disposable local facilities with [`04-local-verification-environment.md`](04-local-verification-environment.md). The runner accepts explicit binary paths, records them and their versions, and fails unavailable when a prerequisite is missing.
