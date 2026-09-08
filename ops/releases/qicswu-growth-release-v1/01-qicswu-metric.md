# Slice 1 — QICSWU metric v1

## Outcome and scope

Install a deterministic, versioned, fail-closed evaluator for Policy-Qualified Independently Funded Canonical Settled Root Work Units per Complete UTC Day. This slice changes measurement only; it does not change contracts, API behavior, wallet behavior, funding, or settlement.

Owned files:

- `scripts/qicswu_metric.py`
- `scripts/qicswu_verify_baseline.py`
- `scripts/test_qicswu_metric.py`
- `scripts/qicswu_evidence.py`
- `scripts/test_qicswu_evidence.py`
- `scripts/qicswu_shadow.py`
- `scripts/test_qicswu_shadow.py`
- `.github/workflows/qicswu-production-shadow.yml`
- `docs/qicswu-metric-v1.md`
- `docs/qicswu-slice-1-release.md`
- `ops/metrics/qicswu-policy-v1.json`
- `ops/metrics/qicswu-principal-registry-v1.json`
- `ops/metrics/qicswu-root-work-map-v1.json`
- `ops/metrics/qicswu-baseline-source-observations-v1.json`
- `ops/metrics/qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json`
- `ops/metrics/qicswu-baseline-raw-open-competition-v1-events-2026-09-01-v1.json`
- `ops/metrics/qicswu-baseline-raw-open-competition-v2-beta3-events-2026-09-01-v1.json`
- `ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json`
- `ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json`
- `ops/metrics/qicswu-baseline-result-2026-08-04-2026-09-01-v1.json`

## Prerequisites

- Python 3 supported by repository gates.
- A locked policy id and content hash.
- Complete retained raw event streams for every accepted protocol and factory, not merely current-factory projections.
- At least two independent RPC observations agreeing on block number and hash, successful receipts, matching logs, reconciled contract state, and accepted finality.
- Content-addressed raw event, receipt-transfer, and state observations that an upstream production package verifier resolves before supplying the evaluator's typed payout views; decoded-view hashes alone are not source truth.
- An approved registry that maps every RPC `provider_id` to a normalized, independently controlled provider authority; two labels or URL aliases for one authority never count as independent observations.
- Canonical funding and first-participation events with transaction, receipt/log, non-reorg, raw-evidence, and exact candidate-scope bindings sufficient to prove reward-principal funding occurred strictly before the same solver's participation.
- Evidence-backed wallet-to-beneficial-principal mappings, operator/affiliate classification, and reimbursement review.
- Evidence-backed root-work mappings and standalone end-customer value adjudication.

## Data migrations

None. This slice uses versioned JSON artifacts and a pure evaluator. A semantic change requires a new policy id and a restated series; never mutate released v1 semantics in place.

## Rollout

1. Lock the source commit and run the metric unit tests and deterministic frozen-baseline regeneration.
2. Review the policy, excluded contracts, factories, and wallet exclusions against current authoritative protocol records.
3. Compare generated artifact hashes with the committed result.
4. Install the evaluator in internal reporting with `status=unavailable` and all north-star/secondary values `null` while any completeness or publication gate is open.
5. Only after every candidate and required stream is reconciled, the raw-reference resolver and independent-provider registry are approved, and a new policy version sets `publication_gate.status=approved` with no blockers may an approved reporting job publish a number, including zero.

## Monitoring

- result `status`, policy/input/registry/root/result hashes, and policy id;
- source coverage by protocol and factory;
- candidate count, unique canonical identity count, duplicates, unknown records, and excluded records;
- unavailable and per-candidate reason-code counts;
- beneficial-principal, reimbursement, root-work, standalone-value, funding-cutoff, receipt, finality, and lifecycle coverage;
- qualified units/day, qualified GMV/day, median solver payout, largest funder share, largest solver share, and operator-subsidy guardrail only when available.

Alert on any numeric publication while `status != available`, any hash drift without a version change, a missing required protocol/factory stream, duplicate or cross-candidate-reused chain-log identities, protocol-round mismatch, unresolved typed-view source reference, or a negative/invalid amount.

Open Competition V2 is the primary protocol for new work. The production
shadow must read and retain V2 as primary and V1 for compatibility on every
run, use canonical
`solution_committed` and `entry_qualified` participation semantics, preserve
protocol identity in rollout snapshots, and fail closed if either competition
source disappears or becomes claim-based. Autonomous exclusive-claim work is
the legacy path and remains measured at equal unit weight; product priority
does not rewrite the north star.

## Rollback

Stop new evaluations and remove the reporting consumer from service. Preserve emitted v1 artifacts for audit. Restore the prior dashboard/reporting behavior without translating `unavailable` to zero. If semantics were wrong, issue a v2 policy and explicit restatement rather than altering v1.

## Required approvals

- metric/data owner for definition and reproducibility;
- protocol/payment reviewer for canonical event and funding semantics;
- security/privacy reviewer for beneficial-principal and reimbursement evidence handling;
- maintainer for reporting exposure.

## Deterministic release verification

Hypothesis: for a locked input bundle, two independent runs produce byte-identical hashes and never emit a numeric north-star value when any required coverage field or candidate qualification fact is unknown.

Pass: all positive, exclusion, deduplication, boundary, operator, reimbursement, root, funding-cutoff, finality, and missing-evidence fixtures pass; the frozen bundle regenerates exactly; the current baseline remains `unavailable` with a `null` value.

Fail/rollback: any unknown becomes eligible, a protocol label creates a second identity for one chain log, one canonical evidence log is reused across conflicting candidate scopes, a wallet exclusion is treated as proof of independence, funding is compared against returned bonds rather than the event-bound promised reward principal, a root is double counted, or regeneration drifts.

This verifies the evaluator and frozen evidence package. It is not a production experiment, a production-like end-to-end golden path, or evidence of north-star uplift.

## Falsifiable production shadow experiment

**Hypothesis.** Against actual production event/RPC sources, the scheduled shadow reporter can close and retain every required protocol/factory stream for a complete UTC day, classify every observed settlement candidate without silent loss, reproduce the locked result byte-for-byte in two isolated evaluations, and fail closed to `status=unavailable` with all numeric outputs `null` whenever any required evidence remains unknown.

**Exposure and unit.** Run an internal, read-only shadow consumer against the production event/RPC sources after the configured safe/final block threshold. It receives no user traffic, writes no contract or wallet transaction, changes no public dashboard, and cannot gate payments. The evaluation unit is one complete UTC day; candidate settlement/root records are the audit units within that day. Keep synthetic local-fork fixtures and events in a separate namespace and exclude them from the production shadow input.

**Primary data-quality success criteria.** For each eligible day:

- 100% of the policy-declared protocol and factory streams close with retained raw bytes, query bounds, counts, and hashes;
- two isolated runs over the same immutable daily bundle produce byte-identical result bytes and hashes;
- every observed candidate is represented exactly once after canonical-identity/root deduplication and is either qualified, deterministically excluded, or unknown with bounded reason codes;
- receipt/log, block-hash/finality, contract-state, lifecycle, funding-cutoff, beneficial-principal, reimbursement, and root-work coverage are explicitly reported; and
- zero numeric north-star or secondary value is emitted whenever any required stream, coverage field, or candidate fact is incomplete.

These are operational and data-quality criteria. A non-null QICSWU/day value is neither required nor assumed; `unavailable` is the correct result while evidence is incomplete.

**Guardrails.** Zero contract writes, signing requests, fund movement, verifier decisions, or payment/custody changes; zero public numeric publication while unavailable; no weakening or runtime override of policy, finality, independence, reimbursement, root, or funding-cutoff rules; no sensitive principal/reimbursement evidence in public artifacts or unrestricted logs; no merging of synthetic canaries with production observations; and no use of this shadow result as release authorization or as proof of a production-like continuous golden path.

**Duration and sample rule.** Start only after the metric/data owner, protocol/payment reviewer, and security/privacy reviewer approve the shadow source and evidence-handling plan. Evaluate 14 consecutive complete UTC days. Each day is eligible only after every declared source passes its finality/safe-block rule and the daily immutable bundle is sealed. A late or incomplete day is a failed day and resets the consecutive-day count; it is never dropped from the audit log or replaced by a more convenient sample.

**Rollback and decision rule.** Immediately disable the shadow reporting consumer on any write attempt, sensitive-data exposure, policy/hash drift, synthetic/production namespace collision, duplicate or omitted candidate, non-deterministic replay, or numeric output from incomplete evidence; preserve sealed inputs and outputs for diagnosis. If all criteria pass for 14 consecutive eligible days, approve only the evaluator's readiness for continued internal production-shadow reporting. If any criterion fails, reject operational readiness, keep QICSWU reporting unavailable, fix the reason-coded failure, and restart the full sample. Passing does not authorize public reporting, catalog activation, live funding, settlement changes, or a claim that QICSWU/day increased.
