# QICSWU growth rollout control v1

Status: **pre-deployment; blocked on an available baseline and explicit approvals**.

This package controls rollout of the already verified
`qicswu-growth-release-v1` candidate. It does not alter the QICSWU metric or
authorize production. The synthetic release candidate remains commit
`04be4364d6ddf2d38d9c3a142da51b491c31ebb2`, source-tree hash
`sha256:af167a3ec447f4d204cca4ad9f945be7b45e14cdc55fb46fe02500cc4d276628`.
Rollout-control code may advance independently; it must never silently replace
that frozen deployment candidate.

The controller is [`scripts/qicswu_rollout_control.py`](../../../scripts/qicswu_rollout_control.py).
It provides four boundaries:

1. `preregister` binds one future deployment timestamp to its immediately
   preceding 28 complete UTC days, the exact release commit/tree, one slice,
   the fixed slice order, target, qualified-GMV floor, median-payout floor, and
   the final 56-day decision contract.
2. `validate-preregistration` rejects a blocked, changed, late, or incomplete
   registration. The preregistration explicitly cannot authorize itself.
3. `daily-snapshot` accepts a separately sealed deployment authorization, one
   complete UTC-day QICSWU result, and explicit funnel/proof/payment/inventory/
   safety observations. Its output is content-addressed and immutable on disk.
4. `evaluate` accepts exactly the first 56 consecutive complete-exposure days,
   splits them into two non-overlapping 28-day windows, and applies the frozen
   volume, GMV, payout, evidence, dispute, diversity, repeat-funder, and 50%
   concentration rules.

## Current blocker

The retained baseline result is truthfully `unavailable`, with a null QICSWU
count, because the production raw-evidence resolver and independent RPC
provider registry are not approved. Beneficial-principal, reimbursement,
root-work, standalone-value, and complete lifecycle evidence also remain
incomplete. Therefore no target count or production preregistration can yet be
created. The controller emits a sealed `blocked` record and exit code `3`; it
does not translate missing evidence into zero.

An available exact-window baseline uses the provisional ambition:

```text
target_count_per_28_day_window = max(10, 2 * baseline_count)
```

This is explicitly not a statistical significance threshold. By default,
qualified GMV and median solver payout may not regress below the available
baseline. If an available zero-count baseline has no median, a positive payout
floor must be explicitly preregistered.

## Preregister one slice

Choose a future UTC deployment time. If it is not exactly midnight, the
exposure windows begin at the next UTC midnight; the partial deployment day is
never counted. Produce a fresh QICSWU result for the exact preceding 28-day
window, then run:

```bash
python3 scripts/qicswu_rollout_control.py preregister \
  --manifest ops/releases/qicswu-growth-release-v1/manifest.json \
  --baseline /approved/immutable/qicswu-baseline-result.json \
  --deployment-at 2026-09-10T00:00:00Z \
  --registered-at 2026-09-09T20:00:00Z \
  --release-commit 04be4364d6ddf2d38d9c3a142da51b491c31ebb2 \
  --source-tree-sha256 sha256:af167a3ec447f4d204cca4ad9f945be7b45e14cdc55fb46fe02500cc4d276628 \
  --slice-id qicswu-metric-v1 \
  --output /approved/immutable/qicswu-preregistration.json
```

The timestamps above illustrate the contract only; they are not a scheduled or
approved deployment. Never copy them into an authorization without selecting
the real future timestamp and regenerating the exact preceding baseline.
For slice 2 or 3, also pass `--preceding-evaluation` with the sealed, passing
56-day evaluation for the immediately preceding slice. A later slice without
that evidence is blocked, even when its own baseline and target are complete.

## Separate deployment authorization

Production requires a second sealed JSON document with schema
`agent-bounties/qicswu-deployment-authorization-v1`. It must bind the exact
preregistration hash, deployment timestamp, release commit, and slice, use scope
`production_deployment_of_exact_registered_slice_only`, and contain an explicit
`approved` record for every role listed in the preregistration. For slice 1
those roles are metric/data owner, protocol/payment reviewer,
security/privacy reviewer, and maintainer.

Deployment authorization always keeps these fields false unless they receive
their own separate approvals:

```json
{
  "external_outreach_authorized": false,
  "incentive_or_spend_authorized": false
}
```

The authorization document is sealed by removing `artifact_hash`, hashing its
canonical sorted compact JSON with SHA-256, and restoring that value as
`artifact_hash`. The controller validates but does not manufacture approval.

## Daily monitoring input

For each complete UTC day, provide a document with schema
`agent-bounties/qicswu-daily-monitoring-input-v1` and these sections:

- adjacent `funnel_transitions`, each with its own numerator, denominator,
  join-coverage status, and source references;
- `canonical_settlement_proofs` observed/direct counts and exceptions;
- `payment_correctness` reconciled/disputed counts and exceptions;
- `inventory_correctness` status, `ready_to_earn_count`, and exceptions;
- `safety_incidents` with an exact open count and incident records; and
- arrays for unresolved `duplicate`, `identity`, `reimbursement`, `finality`,
  and `payout_reconciliation` disputes.

The daily QICSWU result must use the same policy id and hash as the baseline.
An unavailable day is retained with null units and fails the final decision; it
is never dropped or replaced. The writer permits an identical replay but
refuses to overwrite a different artifact at the same path.

## Final decision

The first two complete post-deployment windows must each meet the registered
count, GMV, and median-payout floors. Across all 56 days there must be at least
five independent funding principals, five solver principals, and two funding
principals that each fund at least two distinct counted roots. Funding-count
and GMV concentration are attributed fractionally by each principal's share of
independent reward-principal funding within a root; neither maximum may exceed
`0.500000`.

Every unit must retain its completed QICSWU eligibility binding and direct
canonical settlement transaction/log/block proof. Duplicate roots/proofs,
unavailable days, monitoring mismatches, unresolved disputes, incorrect
inventory, payment disputes, or open safety incidents fail the result.

Passing one slice does not authorize the next. Freeze a new timestamp,
baseline, preregistration, and explicit authorization before each production
deployment. External outreach, incentives, and spend remain separately gated.

## Verification

```bash
python3 -m unittest scripts.test_qicswu_rollout_control -v
python3 -m py_compile scripts/qicswu_rollout_control.py \
  scripts/test_qicswu_rollout_control.py
```

The deterministic tests cover unavailable and wrong-window baselines, hash
tampering, self-authorization, outreach-scope leakage, immutable writes, a full
passing 56-day fixture, and concentration failure despite meeting the volume
target.
