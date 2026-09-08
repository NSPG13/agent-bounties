# Slice 2 — truthful `ready_to_earn`

## Outcome and scope

Make the API's agent-facing `ready_to_earn` view mean that the projected next-action contract passes the repository's current structural, state, and time checks. The view requires claimable work, committed escrow, verification readiness, and an allowlisted earning action; canonical Base items additionally require positive modeled gross cash margin, while legacy sources do not yet expose comparable cash-economics data. This predicate does not attest remote runtime health, worker capacity, or that an attempted action will succeed. This slice intentionally excludes every Open Competition V2 proof-quote action and every read-only scoring/status `GET` from `ready_to_earn`; V2 inventory remains available in its ordinary catalog.

Owned files and coordinated agent-contract fixtures:

- `crates/api/src/opportunities.rs`
- `crates/api/src/main.rs`
- `crates/api/src/open_competition_v2_api.rs`
- `benchmarks/direct-v1/agent-loop/test.mjs`
- `scripts/select-funded-bounty.mjs`
- `scripts/verify-settlement-evidence.mjs`

This is an API projection change. It does not alter contracts, escrow, claims, verifiers, settlements, or source records.

## Prerequisites

- The authoritative opportunity projectors must continue to separate canonical work/payment state from display metadata.
- Evaluation must use a supplied UTC time, not browser time.
- Direct autonomous `claim_bounty` / `prepare_agent_to_earn` remains governed by canonical claimability; its display deadline is the original funding deadline and must not incorrectly hide a still-claimable bounty.
- Open Competition V1 entry requires a live competition deadline and the implemented commit-preparation `POST` contract.
- Open Competition V2 proof quotes and all read-only scoring, snapshot, readiness, and status actions are not earning actions in this release and must fail closed out of `ready_to_earn`, regardless of phase or deadline.
- Configured broker URLs and nominal SLAs are not live worker/prover capacity. The ordinary V2 catalog may advertise only a readiness inspection until a durable, measured capacity heartbeat exists; it must not present a payable quote body or quote action as executable-now.
- Unknown actions and malformed or missing time data fail closed.

## Data migrations

None. No schema or state mutation is required.

## Rollout

1. Run focused opportunity projection/view tests and the API regression suite at a locked commit.
2. In shadow evaluation, compare old and new result sets and review every removed item by reason.
3. Deploy one API instance or bounded traffic cohort.
4. Confirm current claimable direct work and executable V1 entry work remain present, while every V2 proof quote, read-only scoring/status action, and upcoming/expired/non-generative competition item is absent.
5. Expand only if availability and error guardrails hold.

## Monitoring

- `ready_to_earn` item count by source type and next action, with an alert if any Open Competition V2 or `GET` action appears;
- aggregate predicate outcome counts using the bounded primary reasons `work_not_claimable`, `payment_not_escrowed`, `payment_not_committed`, `verification_not_ready`, `action_not_executable_now`, and `cash_margin_not_positive`;
- feed response latency/error rate and zero-inventory alert;
- separately observed adjacent action-start/confirmation and canonical claim/submission/settlement pairs, with explicit denominators and join coverage rather than an inferred full ordered funnel;
- sequential 4xx/422 rate after a ready item is selected;
- QICSWU/day and qualified settled GMV/day when the north-star becomes available.

## Rollback

Redeploy the prior API binary or revert the coordinated projection, runtime configuration, strict route-schema compatibility, predicate, and the agent-contract fixtures listed above. Do not revert only `apply_view` while leaving mismatched runtime inputs or action contracts. No database rollback is needed. Preserve shadow/result-count observations to diagnose whether the rollback restored false-positive inventory.

## Required approvals

- API owner for response-contract compatibility;
- protocol owner for action/window semantics;
- agent-experience owner for the machine-facing contract;
- maintainer for production rollout.

## Falsifiable release experiment

Hypothesis: the filtered view contains zero upcoming, expired, malformed, or non-executable actions while preserving all canonically claimable direct bounties, and reduces failed first action attempts without reducing confirmed claims per eligible item.

Evaluation: shadow the predicate before rollout, then observe seven complete UTC days after bounded rollout. Audit every surfaced action against its canonical source and clock boundary. Compare action-start failures and confirmed claims with the immediately preceding complete-day window, reporting sample size rather than treating a low-volume percentage as proof.

Pass: zero audited false-positive ready items; no canonically claimable direct item is hidden; action-start 4xx/422 rate decreases or is unchanged; confirmed claims per genuinely eligible item do not decline beyond the approved low-volume guardrail.

Fail/rollback: any non-executable item is advertised as ready, any valid direct claim disappears due solely to the old funding deadline, or the feed becomes empty because of a projection bug.
