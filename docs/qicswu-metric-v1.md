# QICSWU metric contract v1.2.0

## Decision

The primary product metric is **Policy-Qualified Independently Funded Canonical Settled Root Work Units per Complete UTC Day** (QICSWU/day).

It is intentionally narrower than settlement count, payout volume, wallet-level non-operator GMV, or funded inventory. A value may be published only when every candidate can be evaluated from reconciled evidence. Missing evidence produces `unavailable` and `null`, never a guessed zero.

The executable policy is [`ops/metrics/qicswu-policy-v1.json`](../ops/metrics/qicswu-policy-v1.json). The deterministic evaluator is [`scripts/qicswu_metric.py`](../scripts/qicswu_metric.py).

This policy is an unapproved release candidate, not a previously published production series. Version `1.2.0` replaces the draft `1.1.0` before first deployment: it preserves the separately anchored event, receipt-transfer, and contract-state payout views, corrects Open Competition V1 participation to the canonical `solution_committed` event, and declares Open Competition as the primary product path without excluding or reweighting other qualifying settlements. The frozen baseline is restated under policy ID `qicswu-base-mainnet-v1.2.0`; no earlier numeric baseline existed to overwrite.

## Formula and unit

For a half-open window of `N` complete UTC days:

```text
QICSWU/day = count(distinct qualifying root_work_id) / N
```

Each root work unit is counted once in its lifetime, on its all-history earliest policy-qualifying settlement. A later evaluation window cannot recount the same root. The root map must retain evidence that settlement history was reconciled and identify that earliest qualifying candidate. Multiple bounties, rounds, contracts, wallets, evaluation windows, or settlement events for the same economic work do not create extra units.

The v1 baseline window is exactly:

```text
[2026-08-04T00:00:00Z, 2026-09-01T00:00:00Z)
```

## Candidate requirements

A settlement counts only when every condition below is determined and true:

1. **Exact supported scope.** Network, chain, protocol version, factory, settlement event kind, and protocol-specific round semantics match the versioned policy. Autonomous settlements require a positive integer round; both competition protocols require `null`. Open Competition V2 includes both the historical `0xa45c...` factory and the current `0x29d0...` factory.
2. **Canonical and final.** At least two independent RPC observations agree on exact block number and block hash; the receipt succeeded; the settlement log matches the receipt and reconciled contract state; and no removal or reorg is observed.
3. **Positive solver payment.** `solver_payout_base_units` is reconciled and positive. It is new value paid to the solver (`solver_reward` plus a completion/timeout bonus where applicable), excluding a returned claim or entry bond. The candidate's `solver_payout_reconciliation` contains three typed views with distinct raw-evidence references and a canonical payload hash for each decoded view. `settlement_event` binds the complete candidate scope and protocol version, event kind, solver, transaction/log, block/hash, base reward, bonus, and event-reported returned bond. Its base reward must equal `promised_reward_principal_base_units`; autonomous and Open Competition V1 settlements reconcile the event-reported returned bond, while Open Competition V2 requires zero because its qualifying-entry flow has no solver bond. `receipt_transfer` binds a successful receipt's exact USDC transfer log, block/hash, bounty-contract sender, solver recipient, and gross amount. `contract_state` binds the complete candidate scope and protocol version plus solver to an exact state read at the settlement block, with the payout-specific minimum number of RPC observations agreeing on that block and hash. Event reward plus bonus, receipt transfer minus the matching event-reported bond, contract-state payout, and `solver_payout_base_units` must all agree. Missing, malformed, payload-hash-drifted, negative-after-bond, identity-mismatched, wrong-asset, overlapping-evidence, or unequal views remain `unknown`.
4. **Reward principal funded before participation.** Canonical funding strictly precedes the first autonomous claim, Open Competition V1 solution commitment, or Open Competition V2 qualified entry. Every funding log and the participation cutoff must carry reconciled transaction/block identity; bind exactly to the candidate network, chain, protocol, factory, bounty contract, bounty ID, and round; and include successful receipt/log binding, explicit non-removal/non-reorg state, and retained raw evidence. The participation event must additionally bind to the solver wallet, and upstream reconciliation must prove it is the earliest qualifying participation event across the covered source. Confirmed independent funding must cover at least `promised_reward_principal_base_units`. A later bonus does not increase this threshold.

Open Competition is the primary product path. The evaluator therefore preserves protocol identity through its ledger and rollout snapshots, and monitoring reports Open Competition separately from the legacy exclusive-claim path. This product priority never changes a unit's qualification or weight in the marketplace-wide north star.
5. **Beneficial principals, not wallet aliases.** Solver and funders have evidenced beneficial-principal mappings. A wallet is not treated as a person, organization, or independent actor by default.
6. **Independent economic source.** The solver and every qualifying funder differ by beneficial controller and neither is an operator principal or affiliate. A qualifying funder has an evidenced `no_reimbursement_found` relationship to the solver for the root work. Funding classified as reimbursed, common-control, or suspected reimbursement is excluded from independent principal; missing review remains unknown. A work unit qualifies only if the remaining independent pre-participation funding still covers the entire promised reward principal.
7. **Root-work lineage.** The candidate maps to an evidenced `root_work_id` so repeated settlements of one economic job can be deduplicated.
8. **Standalone end-customer value.** The root is classified `standalone_end_customer_value`. Synthetic canaries, rehearsals, benchmarks, platform maintenance, and marketplace-generated meta-demand are excluded.

Policy-declared synthetic contracts and known operator wallets are one-way exclusion controls. They can prove that a row is not independent, but an unlisted wallet or contract is never presumed independent.

## Money fields and guardrails

- `promised_reward_principal_base_units` is the precommitted solver reward principal that independent pre-participation funding must cover.
- `solver_payout_base_units` is the positive, reconciled new value paid to the solver, excluding returned bonds.
- `settled_gmv_base_units` is qualified settlement GMV, including policy-defined solver, verifier/keeper, and completion-bonus value.
- Funding-principal concentration uses actual qualifying funding amounts by beneficial principal. It does not split solver payout evenly among funder IDs.
- Solver-principal concentration uses qualified solver payout by beneficial principal.
- Operator reward funding is excluded from independent principal and reported separately, including canonical operator top-ups after the participation cutoff. A known operator or affiliate solver makes the whole work unit ineligible under v1, even when its funder is external.
- Gas, relayer, onboarding, and other operator support is recorded in `operator_subsidies` by asset and kind. Assets are never added across denominations. Missing subsidy evidence withholds the subsidy guardrail but does not manufacture or erase a primary unit.

The result reports QICSWU/day, qualified settled GMV/day, median solver payout, funding-principal concentration, solver-principal concentration, and operator subsidy. All monetary values remain integer base units until presentation.

## Availability states

Candidate ledger states are:

- `eligible`: every required fact is reconciled and the row qualifies before root deduplication;
- `excluded`: a conclusive policy fact makes the row non-qualifying;
- `unknown`: evidence needed to determine qualification is absent, conflicting, or unreconciled.

When the policy publication gate is blocked, the result withholds the evaluator-derived ledger entirely and sets every candidate/cardinality diagnostic to `null`, including for an empty window. This prevents an unavailable official artifact from presenting a reconstructable north-star value through row cardinality. The retained raw input package remains separately auditable, and approved-policy unit fixtures retain the full ledger so qualification branches remain testable.

The overall primary metric is `available` only when required source coverage and registries are complete and no candidate remains `unknown`. Source completeness is evaluated for every `(protocol, accepted_factory_contract)` pair; each pair needs full-window coverage, retained raw responses, the required independent RPC count, and a safe block hash. A protocol-name-only or current-factory-only stream cannot make the metric available. An empty, fully reconciled window is an available zero. An incomplete window is unavailable with null totals and null daily values.

Exact duplicate settlement rows collapse by `(network, chain_id, transaction_hash, log_index)` and retain `source_record_count`; a protocol label is decoded data, not identity. Contradictory reuse of any settlement, participation, funding, or payout-transfer log across candidate scopes makes the result unavailable. Conflicting duplicates, block-hash disagreement, reorg evidence, or incomplete protocol streams likewise fail closed.

## Integrity and versioning

Policy, input, principal registry, root-work map, decoded payout views, and result use canonical sorted compact JSON for SHA-256 hashes. The result embeds each input hash and a hash of the result body. An input freeze cannot predate either the metric window end or its applied policy. [`scripts/qicswu_verify_baseline.py`](../scripts/qicswu_verify_baseline.py) additionally verifies retained full-response canonical hashes and event counts, re-derives the selected settlement rows from the exact half-open window, checks retained file hashes, chain-log candidate identities, the exact canary/recovery partition, both V2 factory scopes, distinct provider authorities, exact factory-creation log provenance and chronology, the source reconciliation, and the frozen result.

The pure evaluator validates adjudicated typed views; a view hash is not independent proof that its source is truthful. A production shadow package must also resolve every content-addressed reference against retained raw event, receipt, and state observations and validate observations against a registry of independently controlled RPC providers. The checked-in continuous canary constructs its typed views directly from the indexed event, receipt logs, and state reads, but remains synthetic and single-provider. The draft policy's machine-enforced `publication_gate` therefore keeps the metric unavailable with `raw_evidence_resolution_not_implemented` and `rpc_provider_registry_not_approved`; unit fixtures explicitly set a synthetic approved gate only to exercise downstream qualification branches. No numeric production series may be published until both controls are implemented and a new approved policy version sets the gate to `approved` with no blocker reason codes.

Semantic changes require a new `policy_id` and restated historical series. A released policy or frozen baseline must not be silently rewritten.

## Current evidence boundary

The frozen v1 package retains all three complete public responses—716 autonomous events, 36 Open Competition V1 events, and 109 Open Competition V2 Beta3 events—and reproduces their original canonical-JSON hashes. It deterministically selects 21 in-window public settlements and combines them with seven rows from the retained dual-RPC settlement snapshot to construct the 28-candidate ledger. A separate [`historical V2 factory-membership artifact`](../ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json) retains matching factory-creation logs from two providers for all seven bounty IDs/contracts. These historical artifacts cover only the August 20–22 snapshot epoch, not the full baseline window. Complete lifecycle and dual-provider finality evidence is also missing for many candidates; beneficial-principal, reimbursement, root-work, and standalone-value coverage is incomplete. Therefore the baseline truthfully evaluates to `unavailable`, not `0`, `26`, or `0.928571/day`.
