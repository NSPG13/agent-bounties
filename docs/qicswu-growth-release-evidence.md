# QICSWU growth release v1 — evidence and decision record

Status: **release candidate pending final verification and approval; not deployed**.

North star: Policy-Qualified Independently Funded Canonical Settled Root Work Units per Complete UTC Day (QICSWU/day).

This report records why three slices were selected, what the code can establish, and what remains unproven. No live deployment, wallet action, funding, settlement, user contact, or production experiment was performed as part of this release preparation. There is no observed production uplift attributable to these changes.

## Baseline truth boundary

The frozen baseline window is exactly `[2026-08-04T00:00:00Z,2026-09-01T00:00:00Z)`, containing 28 complete UTC days.

The retained sources reconcile an **aggregate-aligned diagnostic set of 26 noncanary settlement rows / 36.465 USDC** associated with that period. This is not a complete exact-window gross observation because historical-factory coverage is retained only for the August 20–22 snapshot epoch:

- All three public protocol responses used by the frozen selection are now retained in full. The baseline verifier checks each response's capture-time SHA-256 and event count, then derives the 21 selected in-window rows from those raw artifacts rather than trusting a hand-selected ledger.
- 21 settlements / 21.265 USDC are selected from the three documented public protocol streams.
- The current Open Competition V2 endpoint filters the current factory. A retained dual-RPC settlement snapshot contains seven historical V2 rows, and the separate [`historical V2 factory-membership artifact`](../ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json) retains matching `eth_getLogs` responses from two providers proving those seven bounty IDs/contracts belong to the declared historical factory. Two declared canary contracts are excluded and the other five are 3.04 USDC each, adding 5 settlements / 15.2 USDC. Neither artifact establishes complete historical-factory coverage outside that snapshot epoch.
- `21 + 5 = 26` settlements and `21.265 + 15.2 = 36.465` USDC.

This arithmetic reconciles count and amount only. It does not qualify any root work unit. The north-star baseline is therefore:

| Field | Frozen result |
|---|---:|
| Status | `unavailable` |
| QICSWU total | `null` |
| QICSWU/day | `null` |
| Qualified GMV/day | `null` |
| Aggregate-aligned diagnostic | 26 settlements / 36.465 USDC |

The point estimate is unavailable because the package lacks complete beneficial-principal ownership, reimbursement, root-work lineage, standalone-value classification, pre-participation funding, safe/final block identity, receipt/log, contract-state, and lifecycle evidence for all candidates. In addition, the policy carries a machine-enforced publication gate for two unresolved production controls: raw evidence reference resolution and an approved independent-RPC provider registry. Two declared synthetic canaries are deterministically excluded; the other 26 candidates remain unknown, not eligible and not zero.

Authoritative artifacts:

- [`qicswu-policy-v1.json`](../ops/metrics/qicswu-policy-v1.json)
- [`qicswu-baseline-source-observations-v1.json`](../ops/metrics/qicswu-baseline-source-observations-v1.json)
- [`qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json`](../ops/metrics/qicswu-baseline-raw-autonomous-events-2026-09-01-v1.json)
- [`qicswu-baseline-raw-open-competition-v1-events-2026-09-01-v1.json`](../ops/metrics/qicswu-baseline-raw-open-competition-v1-events-2026-09-01-v1.json)
- [`qicswu-baseline-raw-open-competition-v2-beta3-events-2026-09-01-v1.json`](../ops/metrics/qicswu-baseline-raw-open-competition-v2-beta3-events-2026-09-01-v1.json)
- [`qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json`](../ops/metrics/qicswu-baseline-historical-v2-factory-membership-2026-09-02-v1.json)
- [`qicswu-baseline-input-2026-08-04-2026-09-01-v1.json`](../ops/metrics/qicswu-baseline-input-2026-08-04-2026-09-01-v1.json)
- [`qicswu-baseline-result-2026-08-04-2026-09-01-v1.json`](../ops/metrics/qicswu-baseline-result-2026-08-04-2026-09-01-v1.json)
- [`qicswu-principal-registry-v1.json`](../ops/metrics/qicswu-principal-registry-v1.json)
- [`qicswu-root-work-map-v1.json`](../ops/metrics/qicswu-root-work-map-v1.json)

## Current product evidence

These are audit snapshots captured on 2026-09-01. They have different windows and denominators and must not be combined into one conversion rate.

| Evidence | Observation | Decision implication |
|---|---:|---|
| Rolling platform projection, 28d | 26 settlements; 36.465 USDC; 15 of 62 mature claims settled (24.19%) | Settlement exists, but gross activity is not independent-root value and claim-to-settlement liveness is weak. |
| Browser event totals, 720h | 987 browser ids; 16 post starts / 0 confirmations; 6 claim starts / 0 confirmations | The first canonical transaction is not closing in observed browser traffic. These aggregate event counts are not a joined cohort funnel. |
| Wallet event totals, 720h | 11 wallet-missing detections / 0 wallet connections | Wallet readiness is a visible activation drop; no person-level or wallet-level sequence is established. |
| Market interaction totals, 720h | 9 market-to-funded clicks among 408 measured market events | Demand activation is low; click events are not funding evidence or a market-to-funded conversion rate. |
| Machine inventory snapshot | 22 advertised ready items: 9 closed scoring, 2 future, 4 active platform GMV competitions, 7 platform meta child tasks | Most advertised inventory was not immediately executable; zero items represented ordinary end-customer work. |
| API requests | 2,212 HTTP 2xx of 5,007 requests (~44.2%) | HTTP success is weak and is not an end-to-end SLO; sequential client errors remain a usability bottleneck. |
| Independence | `independent_active_agents=null`; beneficial-principal coverage unavailable | Wallet counts cannot establish independent users or funders. |
| Economic value | The audited projection reports 0 platform revenue and 66.065 USDC lifetime payouts across 52 settlements | A historical gross-payout claim exists, but this release package does not independently reconcile its receipts, state, or finality; sustainable external demand and revenue are not established. |

Feedback evidence is selection-biased: participating agents value canonical event/payment evidence, while reported friction clusters around sequential 422 errors, wallet/readiness ambiguity, stalled settlement signing, and expected payout without canonical escrow. This is directional product input, not representative user-opinion research.

## Evidence-ranked bottleneck decision log

`Observed` statements below are retained facts from the audit snapshots or code review. `Hypothesis` statements are causal explanations to be tested; rank and confidence do not turn them into facts. Implementation cost is relative to this repository (`S`, `M`, `L`), and the risk column names the material boundary even when the selected action avoids crossing it.

| Rank | Plausible constraint | Supporting evidence (`Observed`) | Affected funnel transition | North-star mechanism (`Hypothesis`) | Confidence | Cost | Payment / custody / security / rollback risk | Fastest valid test | Decision |
|---:|---|---|---|---|---|---|---|---|---|
| 1 | Metric truth and independence | The frozen artifacts reconcile 26 gross settlements / 36.465 USDC, but beneficial-principal, reimbursement, root-work, finality, and lifecycle evidence is incomplete; QICSWU is therefore `unavailable`, not zero. | Canonical settlement → policy-qualified independent root unit | A fail-closed evaluator prevents synthetic, operator-funded, reimbursed, duplicate-root, or insufficiently evidenced activity from being mistaken for north-star growth and reveals the evidence gaps that block a trustworthy daily series. | High that measurement is presently invalid; medium that measurement alone changes volume. | M | No payment/custody mutation. Privacy/security risk exists in principal and reimbursement evidence; rollback is removal of the reporting consumer while preserving versioned artifacts. | Re-run the frozen bundle twice, exercise positive/exclusion/deduplication/missing-evidence fixtures, and confirm byte-identical output plus `null` whenever a completeness gate is open; then run the documented production shadow experiment. | **Implemented slice 1.** Install measurement first; do not publish a number while unavailable. |
| 2 | External demand / first canonical post | Browser counts show 16 post starts and 0 confirmations; the inventory snapshot contains zero ordinary end-customer work. Code review found that an AI handoff could emit a placeholder verifier rejected by the funding path. Counts are not a joined cohort and do not prove the cause of abandonment. | Human intent → valid canonical post → independently funded, claimable root work | If valid handoffs preserve verifier/reward/evidence invariants and invalid drafts fail before wallet exposure, more genuine funders can reach canonical creation and funding, creating root work that could later settle. | Medium; the drop and contract defect are observed, but external demand and causal uplift are unproven. | M | Funding/custody boundary is high risk if activated. Candidate keeps catalogs inactive and draft-only; invalid routes fail closed. Rollback removes the handoff surface; any live activation requires a separate approval and release. | Component-test exact payload continuity and invalid-draft blocking now. Later, use a separately approved, version/hash-bound activation cohort and require indexed creation/funding plus manual non-operator/non-reimbursement adjudication. | **Implemented slice 3 contract only.** Do not activate the current catalog or authorize live funding. |
| 3 | Agent inventory actionability | In the relevant fixture, 11 of 16 projected V2 opportunities were upcoming rather than scoreable; the live snapshot advertised closed and future work. This proves misleading readiness, not that filtering will increase settlements. | Discovery → executable claim/entry attempt | Removing non-actionable items should reduce wasted attempts and concentrate agents on work capable of progressing toward canonical settlement. | Medium. | S | No custody change. Security risk is accidental exposure of unsupported write routes or concealment of valid inventory; rollback restores the prior projection. | Assert every returned item has a supported executable action and every V2 proof quote/read-only scoring or status `GET` is excluded; shadow-compare ready-set precision before traffic exposure. | **Implemented slice 2.** Limit `ready_to_earn` to executable direct-claim and Open Competition V1 entry actions. |
| 4 | Settlement liveness | The audited projection records 15 settlements versus 53 submissions / 64 claims, including 15 of 62 mature claims settled. Aggregate counts establish attrition, not whether the cause is verifier delay, invalid work, signer availability, or payment failure. | Submitted work → verified → canonical paid settlement | Reducing verifier/signer/payment stalls would directly increase the share of eligible completed work that becomes canonical settlements. | High that attrition exists; low on the causal remedy. | L | High: verifier authority, signer custody, settlement correctness, and live payments. A bad change may mispay or settle invalid work; rollback may not reverse onchain actions. | Add reason-coded timestamps for submission, verification decision, signing, transaction broadcast, receipt, and indexed settlement; observe one complete window before selecting a remedy. | Deferred. Instrument as the next payment-liveness workstream; do not expand this release beyond three slices. |
| 5 | API ergonomics and wallet ambiguity | Approximately 44.2% of audited requests were HTTP 2xx; browser counts show 11 wallet-missing detections, 0 wallet connections, and 0 claim confirmations. Directional feedback reports sequential 422 errors and unclear readiness. These sources do not establish a joined user journey. | Discovery or compose → valid request → wallet readiness → canonical transaction | Stable error taxonomy, preflight guidance, and idempotent recovery may reduce abandonment before canonical post, claim, or funding. | Medium that friction exists; low-to-medium on its contribution to QICSWU/day. | M | Wallet and replay/idempotency changes can create duplicate writes or misleading signing prompts. Favor preflight/telemetry first; rollback disables new recovery behavior. | Instrument bounded reason codes at each failure boundary and compare adjacent event pairs in a versioned shadow cohort; reproduce the top 422 paths with contract tests before changing writes. | Deferred. Monitor the three slices, then prioritize from reason-coded loss rather than aggregate HTTP status. |
| 6 | Complexity, governance, and trust | Repository review found overlapping protocol/surface variants, an incomplete license stub, and concentrated maintenance. No quantitative causal link to settlement volume was observed. | Contributor/operator capacity → safe iteration across the full funnel | Clear ownership, licensing, and fewer overlapping paths may improve contributor trust and change velocity, indirectly raising the rate of safe experiments and fixes. | Low for near-term north-star causality; medium that maintenance risk exists. | L | Protocol consolidation can introduce compatibility and security regressions; licensing changes require authority. Roll back code consolidation by surface, but published licensing decisions may not be practically reversible. | Inventory owners, supported versions, external contributors, and incident/change lead time; resolve the license with authorized review before testing any consolidation. | Deferred. Keep the three slices bounded and reversible; handle governance/licensing as a separate workstream. |

The ranking favors causal proximity to independently funded settled root work, evidence quality, reversibility, and the fastest valid learning that does not require unapproved live money. Exactly three slices are implemented in this candidate; rows 4–6 remain hypotheses or follow-on work, not silently included scope. The log does not claim these are the only possible constraints.

## Slice decision and release status

| Slice | Expected mechanism | Release-candidate status | Production effect proven? |
|---|---|---|---|
| 1. QICSWU metric/evaluator | Prevent false optimization and make qualification gaps observable. | Implemented in candidate artifacts; final locked-commit gates and approvals pending. | No. The current value is correctly unavailable. |
| 2. Truthful `ready_to_earn` | Reduce wasted agent attempts and improve discover-to-action quality. | Implemented across the candidate projection, API runtime, and V2 route-schema surfaces. Every V2 proof quote and read-only scoring/status `GET` is excluded from this release's ready view; final locked-commit gates and rollout approval are pending. | No. No production traffic was exposed. |
| 3. Pinned-regression post/fund handoff | Install an immutable handoff contract while invalid, incomplete, crowdfunded, or currently inactive catalog entries fail closed. | Implemented across candidate MCP/site/gates, but both checked-in catalogs remain `inactive_preconditions_unmet`; every current handoff is draft-only. The blocked [activation contract](../ops/releases/qicswu-growth-release-v1/pinned-regression-catalog-activation-v1.json) has null gate evidence and authorizes nothing. A separate future activation release is required. | No. No production wallet path was exposed and no live bounty was posted or funded. |

The package runbooks are in [`ops/releases/qicswu-growth-release-v1`](../ops/releases/qicswu-growth-release-v1/README.md). Applicable public context already exists in [#1117](https://github.com/NSPG13/agent-bounties/issues/1117), [#1183](https://github.com/NSPG13/agent-bounties/issues/1183), and [#1122](https://github.com/NSPG13/agent-bounties/issues/1122). This preparation neither posts a new notice nor changes those issues.

## Golden-path evidence boundary

Two component composites are required:

- Human post/fund composite: MCP schema and handoff construction preserve a pinned benchmark/evidence schema while the catalog remains inactive; browser preflight blocks invalid or inactive drafts; site checks enforce route and named event boundaries; the contract harness separately proves a fully funded canonical settlement loop.
- Agent discover-to-settle composite: API tests prove ready-view actionability and exclude V2 proof quotes/read-only scoring/status actions; agent-loop fixtures prove selection/action/payment-evidence interpretation; the contract harness separately proves claim, submit, verify, settle, and balances.

Passing these components is useful release evidence, but they do not form one continuous run. The opt-in continuous gate must directly execute the checked-in runner and carry the same exact payload plus its fixed, disclosed synthetic dev identities through MCP, the real browser composer, a hash- and runtime-pinned Anvil Base snapshot, indexer/Postgres, API discovery, verification, settlement, indexed payment proof, and metric reconciliation. An imported report is inspection-only and cannot satisfy the gate. Even a direct pass is a synthetic local-fork wiring canary, not production execution, independent funding, external demand, verifier custody, or a QICSWU unit. The result is claimed only when the ignored or external machine attestation for the exact clean commit reports a direct-run pass; this static table is never edited to make that claim.

## Release verification specification

This tracked table is a static gate specification, not the mutable result record. After the candidate commit is locked, run every required gate without editing tracked files. Retain the final JSON reports under ignored `target/` storage or CI artifact storage, and bind each attestation to the exact commit, clean-tree flag, source-tree SHA-256, command output digest, and report SHA-256. Editing this document after a run changes the tested source and invalidates that run; do not create a self-referential “final result” commit. Do not convert a component pass into continuous evidence.

<!-- ROOT_RELEASE_VERIFICATION_START -->
| Gate | Exact command or evidence | Required interpretation |
|---|---|---|
| Source integrity | `python3 scripts/qicswu-continuous-golden-path.py --check-source-integrity && git diff --check` before and after the full gate | The command emits the exact commit and canonical source-tree SHA-256 and exits nonzero for a dirty tree. |
| Full preflight | `bash scripts/preflight.sh full` | Use the [local verification environment](../ops/releases/qicswu-growth-release-v1/04-local-verification-environment.md); missing infrastructure is unavailable, never passed. |
| Metric evaluator | `python -m unittest scripts.test_qicswu_metric -v` | Nonzero tests and exit zero required. |
| Frozen metric reproduction | `python scripts/qicswu_verify_baseline.py` | Re-evaluate and compare the retained result byte-for-byte. |
| Opportunity actionability | `cargo test --locked -p api opportunities::tests::` | Nonzero tests and exit zero required. |
| MCP handoff/runtime | `cargo test --locked -p mcp-server chatgpt_app::tests:: && python3 scripts/check-chatgpt-app-runtime.py --build` | Both explicit loopback activation and a real no-override draft-only runtime are required. |
| Browser/site handoff | `node scripts/test-bounty-verification.js && node scripts/test-bounty-reward-continuity.js && node scripts/test-bounty-funding-integrity.js && node scripts/test-ai-bounty-handoff.js && python3 scripts/test_check_public_handoffs.py -v && python3 scripts/check-public-handoffs.py && python3 scripts/check-site.py` | Component evidence only. |
| Rust workspace | `cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace` | All exit zero. |
| Contract paid loop | `cd contracts/base-escrow && forge test --fuzz-runs 1000` | Synthetic contract evidence only. |
| Direct synthetic local-fork post/fund/settle/index/rediscover/measure | `DOCKER_HOST=unix:///tmp/qicswu-release-gate-01/docker.sock python3 scripts/check-qicswu-growth-golden-paths.py --run-continuous-local-fork --require-continuous --continuous-run-output target/qicswu-continuous-golden-path.json --output target/qicswu-growth-golden-paths.json` after provisioning [the exact local facilities](../ops/releases/qicswu-growth-release-v1/04-local-verification-environment.md) | Only this direct runner can prove the named same-identity synthetic boundaries; imported JSON cannot. |
| Full repository gate | `python3 scripts/qicswu-continuous-golden-path.py --check-source-integrity && bash scripts/check.sh && python3 scripts/qicswu-continuous-golden-path.py --check-source-integrity` | Exit zero and identical clean source fingerprints are required. |
<!-- ROOT_RELEASE_VERIFICATION_END -->

## First approved experiment

The current inactive v1 catalog is ineligible for this experiment. The current analytics payload and reports do not bind events to an activation id plus catalog version/hash and do not implement the required handoff-view/funding-start cohort query. Run it only through a separate catalog-activation release after slice 1 is installed as the measurement contract, slices 2–3 pass their locked gates, every activation-contract evidence field—including the cohort-measurement contract—is populated, and live-funds approval is explicit.

- Population and traffic denominator: before traffic, lock the experiment id, activated catalog version/SHA-256, and exact UTC `activation_at`. Persist those bindings with `event_id`, session-local `session_id`, and `occurred_at`. Within the half-open activation window, retain the first handoff view per session and freeze the first 30 distinct sessions ordered by `(occurred_at,event_id)`. Repeat views in one session count once; new sessions may count again, so this is explicitly session-level rather than user-level. Do not filter by principal status or later eligibility.
- Immutable cohort evidence: freeze a private, access-controlled canonical-JSON export of the 30 retained event/session/time rows and query bounds; publish only its count and SHA-256. The reporting query, allowlisted activation fields, pair definition, deduplication tests, and export must exist before activation traffic.
- Duration/sample: continue until both 14 complete UTC days have elapsed and the 30-view cohort is complete.
- Observable reporting: report only directly observable adjacent pairs, each with its own event definition, denominator, deduplication rule, bounded window, and measured join coverage. Join `funding_started` to a retained handoff view only in the same session, later than that view and inside the locked window. The handoff-view/funding-start pair uses the fixed denominator of 30 sessions. Do not concatenate browser identifiers, wallets, contracts, claims, or settlements into a full ordered cohort funnel.
- Manual adjudication: investigate beneficial principal, operator/affiliate relationship, reimbursement, canonical funding, claimability, and root-work follow-through only for a bounty with `canonical_post_confirmed` plus matching indexed creation/funding evidence. Do not infer those properties for views, wallet events, or unconfirmed attempts.
- Primary activation test: at least one such confirmed bounty has indexed creation, funding, and claimability evidence and is manually adjudicated to a non-operator, non-reimbursed funder, with zero invalid-draft wallet exposures and zero unsupported-route continuations.
- North-star follow-through: observe each confirmed bounty through its committed task window. Count it only if final settlement independently passes QICSWU policy and root deduplication.
- Guardrails: no premature canonical telemetry, no change to verification/payment invariants, no unsupported route reaching a wallet, and no hidden canonically claimable direct inventory.
- Failure rule: immediately roll back a safety/evidence-boundary breach. If both thresholds complete with no manually eligible confirmed bounty, reject the activation hypothesis and diagnose only the observed pairwise losses.

Even a passing activation test is not proof that QICSWU/day increased. Production uplift requires an eligible final settlement and a complete, available comparison series; causal confidence requires a suitable concurrent control or repeated experiment at enough volume.

## What remains unproven

- The QICSWU baseline point estimate and every qualified secondary metric.
- Beneficial-principal independence, reimbursement absence, and distinct root value for the 26 gross candidates.
- That any currently advertised work is ordinary external end-customer demand.
- That the new handoff causes a human to fund a bounty, an agent to claim it, or a verifier to settle it.
- Any production funding-ready path from the current inactive, draft-only catalog; the blocked activation contract authorizes none.
- That filtered discovery causes more claims or settlements.
- A continuous production-like golden path across all services.
- Production reliability, latency, accessibility, wallet-provider compatibility, and uplift under real traffic.
- Sustainable platform revenue or decentralized maintenance.

These are experiment and operations obligations, not inferred successes.
