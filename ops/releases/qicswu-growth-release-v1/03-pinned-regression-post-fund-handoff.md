# Slice 3 — pinned-regression human post/fund handoff

## Outcome and scope

Install a shared, immutable pinned-regression handoff contract from AI/MCP drafting through browser review, while keeping the current production catalog explicitly inactive and every such handoff draft-only. Incomplete verifier inputs and crowdfunded drafts remain draft-only; unsupported hosted action routes remain status-only; telemetry begins at real interface boundaries. A separately reviewed future activation release is required before this path may expose wallet continuation or create a live bounty.

Owned surfaces:

- MCP tool schema, inactive catalog, reviewed descriptor locks, and runtime: `crates/mcp-server/Cargo.toml`, `crates/mcp-server/src/main.rs`, `crates/mcp-server/src/chatgpt_app.rs`, `crates/mcp-server/fixtures/pinned-regression-catalog-v1.json`, `crates/mcp-server/fixtures/public-mcp-contract-v1.json`, `crates/mcp-server/fixtures/tool-registry.json`, `Cargo.lock`
- shared browser preflight, inactive catalog mirror, handoff, and funding integrity: `site/pinned-regression-catalog-v1.js`, `site/bounty-verification.js`, `site/bounty-funding-integrity.js`, `site/ai-bounty-handoff.js`, `site/bounty-composer-v2.js`, `site/post.html`
- fail-closed routes: `site/authorize.js`
- bounded telemetry API, SDK, OpenAPI, persistence contract, and navigation cleanup: `crates/api/src/main.rs`, `crates/api/src/opportunities.rs`, `crates/api/fixtures/openapi-contract.json`, `crates/sdk-typescript/src/index.ts`, `crates/sdk-typescript/fixtures/public-api.json`, `crates/db/src/lib.rs`, `migrations/0030_site_analytics_transition_outcomes.sql`, `migrations/checksums.json`, `docs/site-analytics.md`, `site/analytics.js`, analytics-loader HTML pages under `site/`, `site/index.html`, `site/post-a-bounty-with-chatgpt-claude-gemini.html`
- public guidance: `docs/agent-quickstart.md`, `site/agent/index.md`, `site/llms.txt`
- component gates: `.github/workflows/pages.yml`, `scripts/check-chatgpt-app-runtime.py`, `scripts/check-public-handoffs.py`, `scripts/check-site.py`, `scripts/test-ai-bounty-handoff.js`, `scripts/test-bounty-verification.js`, `scripts/test-bounty-reward-continuity.js`, `scripts/test-bounty-funding-integrity.js`, `scripts/test_check_public_handoffs.py`
- composite and opt-in continuous gates: `scripts/check-qicswu-growth-golden-paths.py`, `scripts/qicswu-continuous-golden-path.py`, `scripts/start-qicswu-isolated-docker.sh`, `scripts/test_check_qicswu_growth_golden_paths.py`, `scripts/check.py`, `ops/releases/qicswu-growth-release-v1/04-local-verification-environment.md`
- blocked future-activation authority: `ops/releases/qicswu-growth-release-v1/pinned-regression-catalog-activation-v1.json`, `ops/releases/qicswu-growth-release-v1/manifest.json`

## Prerequisites

- A complete immutable `sandboxed_regression_v1` benchmark with an exact public `github_commit`, non-root subdirectory, digest-pinned image and benchmark, direct argv, resource limits, platform, and seed.
- The exact supported singleton evidence schema: it requires only `source_snapshot_digest` with the `^sha256:[0-9a-f]{64}$` pattern and sets `additionalProperties=false`. Arbitrary fields cannot be advertised as required because the current worker does not enforce them.
- The title, goal, acceptance criteria, source commit/subdirectory, full runner manifest, and evidence schema must equal the versioned `agent-bounties/pinned-regression-catalog-v1` entry. Generic verifier inputs and unrelated task terms remain draft-only until a server-side fetch/stage/pull/digest/execution preflight exists.
- The existing regression sandbox policy and verifier SDK remain authoritative; this slice does not weaken verification.
- The browser must load the shared preflight asset before the composer and handoff importer.
- First-party HTTPS API, Base network configuration, legal acceptance, wallet discovery, canonical factory, and analytics collector must be healthy before live rollout.
- `crowdfund=false` for this canonical earning path. Wallet review remains separate from terms approval.
- Both checked-in v1 catalogs have `activation.status=inactive_preconditions_unmet`. Their required gates are release obligations, not claims that those gates passed.
- The checked-in [activation contract](pinned-regression-catalog-activation-v1.json) is a blocked future-release record: its required gate evidence is null and `production_activation_authorized=false`.
- Future activation requires a server-side, creator-bound stable handoff idempotency key and atomic bounty-identity reservation. Replays and concurrent browser tabs must resolve to the same reserved creation nonce, bounty id, predicted contract, creator wallet, and immutable terms tuple; client storage is only a fail-closed UX guard and is not activation-grade uniqueness authority.

## Data migrations

Apply immutable migration `0030_site_analytics_transition_outcomes.sql` before any site asset can emit the new names. It replaces only `site_analytics_event_name_check` with the exact `0029` allowlist plus 14 bounded transition-outcome names; it adds no table, column, user identifier, backfill, or destructive data change. The migration is forward-only and remains safe if application assets are rolled back because older producers use a subset of the expanded allowlist.

## Rollout

1. Run MCP schema/runtime, Rust, shared JavaScript preflight, public handoff, site, syntax, and static asset gates at a locked commit.
2. Apply migration `0030`, then deploy the API/OpenAPI contract. Verify all 14 bounded transition outcomes persist and an arbitrary event name is rejected before exposing updated producers.
3. Deploy static site assets. The inactive browser catalog safely renders all pinned-regression handoffs as draft-only and does not expose a wallet.
4. Verify invalid/missing/tampered/crowdfunded handoffs cannot reach wallet discovery and that `authorize.js` exposes only the implemented `post.html` destination.
5. Deploy the MCP server so `prepare_bounty_post` can carry validated benchmark and evidence JSON into the review URL while returning `funding_ready=false` under the inactive catalog.
6. Observe draft parsing, bounded preflight failures, and unsupported routes only. Do not start the funding-ready experiment from this release.
7. Treat activation as a separate future release: satisfy every required catalog gate; issue a new catalog version rather than mutating v1; update the MCP fixture, browser mirror, and browser hash together; rerun the locked component and continuous gates; and obtain explicit wallet/live-funds approval before any production `active` state.

## Monitoring

The current inactive release has only bounded, client-declared interface diagnostics: one invalid-handoff event, four funding-block outcomes, wallet-missing/connection/readiness outcomes, and canonical-post stage outcomes if an unexpected path reaches those stages. It does not separately measure draft-only rate, handoff parse versus size failures, MCP tool errors, unsupported-route attempts, or accidental wallet exposure. Static/component gates remain the evidence for those boundaries, and the unauthenticated event stream cannot prove that an interface transition occurred.

The optional approved-image path has component tests for first-party host/path/hash binding and fetched-byte SHA-256 verification. The continuous local-fork payload intentionally carries no image, so it does not prove the image-bearing MCP-to-browser-to-fork path; that remains required evidence for any activation that admits approved images.

After a separate activation release, report only observable adjacent event pairs, each with its own denominator, deduplication rule, join coverage, and bounded window: handoff-view/funding-start, funding-start/wallet-connect, wallet-connect/funding-observed, and canonical-post-start/canonical-post-confirmation. Do not concatenate those pairwise observations into a person-level or session-level ordered funnel. Browser identifiers, wallets, canonical contracts, claims, and settlements are different identity domains; absent an evidenced join, one pair cannot supply the denominator for another.

The current collector does not persist an activation experiment id or catalog version/hash and the current reporting query does not implement the handoff-view/funding-start cohort. Therefore this inactive release cannot execute the future experiment. Before activation traffic, a separately reviewed release must implement and test an access-controlled, event-level cohort query plus immutable export under the contract below. This is a blocking prerequisite, not a post-launch analytics task.

`funding_started` must occur only after local verifier preflight and before wallet continuation. `canonical_post_started` must occur only after the returned creation plan passes local validation and immediately before wallet submission can begin. Preparation, wallet-submission, and reconciliation failures are assigned by an explicit stage marker. `canonical_post_confirmed` requires indexed creation, funding, and claimability evidence. Claim and settlement follow-through must be joined manually from confirmed canonical bounty identities; interface events alone prove neither settlement nor beneficial-principal independence.

## Rollback

For this inactive release, rollback MCP first so it stops emitting the v2 draft handoff, then restore the prior static site and API as needed. If only a route or telemetry fault exists, revert that producer while leaving the stricter shared preflight in place. Do not reverse or edit migration `0030`; its additive allowlist superset is inert for older producers and preserves migration history. A future activation rollback must first return the newly versioned catalog to a fail-closed inactive state across MCP and browser surfaces, then roll back MCP and site assets. A full rollback restores the previous draft experience but must not describe a `creator_review` placeholder as executable verification.

## Required approvals

- MCP/API owner for tool contract and URL payload boundary;
- verifier/security owner for pinned source, sandbox manifest, schema, and unsafe-key validation;
- frontend owner for wallet gating, canonical event semantics, and accessibility;
- payment/protocol reviewer for creation-plan and evidence boundaries;
- maintainer for production deployment and experiment traffic.

## Falsifiable release experiment

This experiment is specified for a separate, approved catalog-activation release; it must not run against the inactive v1 catalog.

Hypothesis: after activation, complete pinned inputs can produce at least one externally funded, canonically created and claimable bounty without exposing wallet continuation to any invalid draft. “Independent” is reserved for a later settled work unit that passes the complete QICSWU beneficial-principal, timing, reimbursement, and root-work policy.

Before traffic, lock an experiment id, exact activated catalog version and SHA-256, and an RFC3339 UTC `activation_at`. Every eligible `canonical_post_handoff_viewed` and `funding_started` event must persist those three bindings alongside the existing client-generated `event_id`, random session-local `session_id`, and `occurred_at`. Replayed `event_id` values remain idempotent.

Run until 14 complete UTC days have elapsed and at least 30 distinct eligible sessions have been observed. The cohort unit is a `session_id`, not a person, principal, wallet, or browser: keep only the first qualifying handoff view per session, order those rows by `(occurred_at, event_id)`, and freeze the first 30. Multiple views in one session count once; a new session may count again and that limitation must be disclosed. The exact activation window is half-open from `activation_at`; lock its end in the cohort export. The traffic denominator is exactly 30 regardless of principal status or later settlement eligibility. Later ordered rows are outside this cohort.

Freeze an access-controlled canonical-JSON export containing the experiment/catalog bindings and the 30 retained `event_id`, `session_id`, and `occurred_at` rows, plus the bounded query parameters. Publish only the row count and SHA-256, not the identifiers. Join `funding_started` only within the same retained session, after its handoff view and inside the locked window. Report this adjacent pair separately; never claim a full ordered cohort funnel.

Manually adjudicate principal ownership, operator/affiliate status, reimbursement, canonical funding, claimability, and root-work follow-through only for bounties with `canonical_post_confirmed` and matching indexed creation/funding evidence. Do not infer those properties for views, wallet events, or unconfirmed attempts.

Pass for activation: at least one confirmed bounty has matching indexed creation, funding, and claimability evidence and, after manual review, a non-operator, non-reimbursed funder; zero invalid/crowdfunded drafts expose wallet continuation; and zero unsupported routes open. Report the pairwise telemetry and join coverage alongside the result. This is an activation result, not yet north-star uplift.

Pass for north-star: an activated bounty later produces a final canonical settlement that independently passes the QICSWU policy. Only then may it contribute one root work unit.

Fail/rollback immediately: any incomplete draft reaches wallet discovery, any unsupported action gains continuation, any confirmation lacks canonical evidence, or any telemetry event is emitted at navigation rather than its named boundary. If no manually eligible activation occurs after both the time and traffic thresholds, reject the activation hypothesis. Diagnose only observed pairwise losses; do not invent an end-to-end funnel drop.
