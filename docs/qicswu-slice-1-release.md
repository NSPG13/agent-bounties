# QICSWU measurement foundation — slice 1 release package

## Outcome

This reversible measurement-only slice creates the v1 metric contract, deterministic evaluator, candidate ledger, retained source observations, frozen 28-complete-UTC-day baseline package, and a read-only scheduled production shadow. The shadow receives no user traffic and has no signing, contract-write, payment, or public-publication authority.

The baseline is **verified but unavailable**. Its numeric QICSWU point estimate is deliberately `null` because beneficial-principal, reimbursement, root-work, standalone-value, full lifecycle, and full dual-RPC finality evidence are incomplete. The draft policy also blocks numeric publication until raw evidence references are resolved by a production verifier and independently controlled RPC providers are approved in a registry. The 26 aggregate-aligned settlements are not claimed as independent work.

## Frozen baseline reconciliation

| Evidence set | Settlement rows | GMV base units | Interpretation |
|---|---:|---:|---|
| Current documented public protocol streams | 21 | 21,265,000 | Three full responses retained; exact complete-UTC-window settlement rows are re-derived and hash-verified |
| Historical V2 rows in retained dual-RPC settlement snapshot | 7 | 15,725,000 | Separate dual-provider creation logs prove membership in the historical factory; full-window coverage remains incomplete |
| Declared historical canaries retained and excluded | 2 | 525,000 | Hard policy exclusion; rows remain in raw candidate evidence |
| Historical-factory aggregate gap recovered | 5 | 15,200,000 | Five 3.04-USDC rows explain the aggregate/current-stream gap exactly |
| Aggregate-aligned noncanary settlement set | 26 | 36,465,000 | Matches the observed rolling aggregate count and amount; not an independence metric |

The public aggregate observation used a rolling window beginning at `2026-08-04T19:41:49Z`, not the required complete-day boundary. Matching count and amount is useful reconciliation evidence, but is not sufficient to publish the exact-window north star.

The retained observed candidate set contains 28 rows; it is not claimed to be an exhaustive exact-window universe. Two declared synthetic contracts are conclusively excluded. The remaining 26 are `unknown`, not eligible, until source and qualification evidence are complete.

## Files

- `ops/metrics/qicswu-policy-v1.json` — draft v1.2.0 semantics, protocol/factory scopes, canonical Open Competition participation events, typed payout reconciliation, exclusions, availability rule, and machine-enforced publication gate; immutable after approval.
- `ops/metrics/qicswu-principal-registry-v1.json` — explicit incomplete beneficial-principal and reimbursement registry; no wallet identities are invented.
- `ops/metrics/qicswu-root-work-map-v1.json` — explicit incomplete root map plus two evidenced canary classifications.
- `ops/metrics/qicswu-baseline-source-observations-v1.json` — full-response paths and capture hashes, 21 selected public-stream rows, seven retained historical-factory rows, and discrepancy reconciliation.
- `ops/metrics/qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json` — complete 716-event autonomous response matching its captured canonical hash.
- `ops/metrics/qicswu-baseline-raw-open-competition-v1-events-2026-09-01-v1.json` — complete 36-event V1 response matching its captured canonical hash.
- `ops/metrics/qicswu-baseline-raw-open-competition-v2-beta3-events-2026-09-01-v1.json` — complete 109-event Beta3 response matching its captured canonical hash.
- `ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json` — matching dual-provider factory-creation logs proving that all seven retained historical V2 snapshot rows belong to the declared historical factory; this does not provide full-window lifecycle coverage.
- `ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json` — 28 normalized candidates for the exact half-open window.
- `ops/metrics/qicswu-baseline-result-2026-08-04-2026-09-01-v1.json` — deterministic blocked-publication result: no evaluator-derived ledger, null cardinality diagnostics, and a null north star.
- `scripts/qicswu_metric.py` — stdlib-only evaluator and result checker.
- `scripts/qicswu_verify_baseline.py` — evidence-anchor and frozen-package verifier.
- `scripts/test_qicswu_metric.py` — deterministic contract, adversarial, and baseline tests.
- `scripts/qicswu_evidence.py` — read-only dual-RPC settlement, finality, and transfer evidence capture.
- `scripts/qicswu_shadow.py` — complete-UTC-day production event-stream capture and fail-closed evaluation.
- `.github/workflows/qicswu-production-shadow.yml` — daily internal runner and immutable 30-day evidence retention.
- `docs/qicswu-metric-v1.md` — human-readable normative contract.

## Verification

```bash
python3 scripts/qicswu_verify_baseline.py
python3 -m unittest scripts.test_qicswu_metric -v
```

The verifier must report `verified_unavailable`, 28 raw candidates, two declared canary exclusions, 26 unresolved aggregate-aligned candidates, and a null north star. It recomputes every retained response hash and event count, derives settlement rows using the exact half-open UTC window, and compares them byte-canonically with the frozen selection. The tests cover exact UTC boundaries, per-factory source completeness, available zero, payout reconciliation, reward-principal funding threshold, candidate-scoped receipt-bound funding and participation timing, wallet/principal separation, pre- and post-participation operator funding, nonreward subsidy, reimbursement, canary/maintenance/meta exclusions, retry/round/root deduplication, duplicate conflicts, factory scope, isolated finality rejection, reorgs, hashes, and retained-response tamper detection.

## Decision and public coordination record

No new notice was published and no external PR code was reused. This local slice falls within the already-published scopes of:

- [#1117 — platform metrics, inventory, and external-funder conversion](https://github.com/NSPG13/agent-bounties/issues/1117)
- [#1183 — participation readiness and funnel measurement](https://github.com/NSPG13/agent-bounties/issues/1183)
- [#1122 — public platform statistics and payout proofs](https://github.com/NSPG13/agent-bounties/issues/1122)

The work adds isolated Python/JSON/docs files plus one read-only scheduled workflow. It does not modify contracts, migrations, runtime APIs, payment authorization, user-facing behavior, or existing public metrics. Running the shadow reads public production event/RPC sources and stores private workflow artifacts; it does not publish a metric, contact users, spend funds, or perform live financial activity.

## Release and rollback

Promotion installs a production-shadow measurement consumer, not a public metric. Rollback disables `.github/workflows/qicswu-production-shadow.yml` and stops new evaluations, but preserves every emitted v1 policy, input, registry, result, and hash anchor as immutable audit evidence. Code and documentation may be reverted only without deleting or rewriting retained historical artifacts; no data migration or chain action is involved.

Before a numeric baseline or daily series is production-ready, acquisition must replay all listed factory scopes over exact block-bounded UTC windows with complete raw responses, receipts, safe/final block hashes, and dual-provider agreement; then complete evidenced principal, reimbursement, root lineage, standalone-value, and operator-subsidy adjudication. Any semantic correction must ship as a new policy version with a restatement, never as a silent edit.
