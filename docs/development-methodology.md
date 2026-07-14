# Eval-Driven Development

The project uses loop-based harness testing wherever outcomes are deterministic
and measurable.

## Deterministic Harnesses

- ledger conservation and no double-pay,
- bounty and settlement state transitions,
- escrow release/refund/split tests,
- webhook replay/idempotency,
- MCP scripted flows,
- read-only production-discovery contract checks for API, MCP, public proof
  surfaces, risk policy, eval history endpoints, and payment rail advertising,
- spawned API/MCP HTTP service smoke, including the production-discovery
  contract and an MCP-only paid bounty lifecycle through route, post, claim,
  submit, verify, and payout-status tools,
- optional Docker-backed Postgres service smoke with restart hydration checks,
- optional live SDK smoke for Python and TypeScript clients against a local API,
- verifier fixture outputs,
- bounded operational recovery decisions, including restart budgets, monotonic
  cursor resume, revision skew, verifier suppression, webhook replay, and
  integrity containment.
- deterministic abuse controls, including claim-owner enforcement and
  low-value payout caps.
- GitHub CI proof guards that route missing PR acceptance metadata, self-merged
  PRs, and PRs without independent approval to review before any automatic
  bounty settlement.
- GitHub claim-reservation fixtures that reject templated no-progress claims,
  allow progress-backed reservations, recommend stale release after the
  reservation window, and prove claim comments never authorize settlement.

## Product Evals

`BountyBench` fixtures score routing, template fit, verifier choice, privacy
classification, and funding mode. AI-judge filters may flag low-quality bounties
or submissions, but they never authorize settlement.

`AbuseBench` fixtures score deterministic risk-policy behavior. These fixtures
cover non-claim-owner submissions, high-value Base USDC automatic-release caps,
unsafe credential-seeking requests, weak GitHub PR proof, stale or low-effort
claim signals, and normal work that must remain allowed.

`JudgeBench` fixtures score product-quality AI-judge filters. The current gate
covers bounty clarity, acceptance-criteria completeness, spam/fraud risk,
proof-page usefulness, submission quality, and template fit. These filters may
request revision, request review, or reject unsafe work, but they do not settle
funds.

`EvalLoops/all-v0` composes the project loops into one report:

- `RouterLoop` scores blocked-goal routing against `BountyBench`.
- `TemplateLoop` scores template-fit judge fixtures.
- `VerifierLoop` scores known-good and known-bad deterministic verifier cases,
  including the rule that AI-judge filters never accept payment settlement.
- `ProofLoop` scores proof-page usefulness fixtures.
- `AbuseLoop` scores risk and payout-safety fixtures.

Each loop records its baseline floor, gate threshold, candidate score, accepted
candidate, score delta, and source suite. A candidate is accepted only when it
improves over the baseline floor, clears the gate threshold, and has no fixture
failures.

`RecoveryLoop` applies the same rule to production failures. Redacted snapshots
in `ops/fixtures/recovery-cases.json` commit the required recovery decision,
automatic actions, manual escalations, and explicitly forbidden actions. The
candidate policy passes only at a score of 1.0; availability improvements may
not trade away any payment, contract, ledger, cursor, or evidence invariant.
AI can propose or rank repairs, but the deterministic policy decides whether an
action is allowlisted and never grants wallet or settlement authority.

When the hosted API or MCP server runs `BountyBench`, `AbuseBench`,
`JudgeBench`, or `EvalLoops/all-v0`, it appends a durable `EvalRun` summary
with suite, score, pass/fail, and timestamp. Agents and dashboards can read
this history from `GET /v1/evals/runs` or MCP `get_eval_runs` to inspect quality
evidence before trusting a network. Eval history remains advisory evidence
only; it cannot release funds without a deterministic verifier result or an
operator decision.

## Loop Pattern

1. Mutate one routing/template/verifier candidate.
2. Run deterministic tests.
3. Run fixture corpus.
4. Score against thresholds.
5. Keep only non-regressing changes.

## CI Gate

`scripts/check.ps1` and `scripts/check.sh` are the source of truth for the
deterministic gate. GitHub Actions runs `scripts/check.sh`, which covers Rust
formatting, clippy, workspace tests, the local demo, `BountyBench`,
`AbuseBench`, `JudgeBench`, `EvalLoops/all-v0`, spawned API/MCP HTTP service smoke, operator CLI
planners, SDK compilation, and Foundry escrow tests. The service smoke proves
that agents can discover the network, inspect payment trust surfaces, use the
hosted MCP surface to route blocked work, create paid work, claim it, submit
proof, verify it, and inspect pending payout state. The CLI planners include
the agent discovery manifest and Base Sepolia Foundry runbook generator so
changes to public machine-readable entrypoints and operator payment commands
are visible in CI. New deterministic product requirements should be added to
these scripts before they are considered enforced.

The same gate validates `ops/self-healing-policy.json`, runs RecoveryBench, and
tests the controller contract. The scheduled `Operational Control Loop`
workflow applies the reviewed policy to read-only production observations and
preserves the snapshot and recovery plan as an artifact.

`scripts/check-postgres.ps1` and `scripts/check-postgres.sh` are the durable
hosted-mode gate. They require Docker, start Postgres, run the spawned
API/MCP service smoke with `DATABASE_URL`, restart both services, and verify
that persisted API-created and MCP-created bounty state hydrates correctly.

`scripts/check-sdk-live.ps1` and `scripts/check-sdk-live.sh` are the SDK
adoption gate. They start the API in in-memory mode and run the Python and
TypeScript SDKs through the same agent bounty lifecycle that external agent
developers are expected to copy. If `OPERATOR_API_TOKEN` is set, the spawned API
requires it for hosted mutation surfaces and both SDK smoke clients send it, so
the same gate covers authenticated operator flows. The separate GitHub Actions
`SDK Live Smoke` workflow runs this gate for SDK and API public-surface changes
and on manual dispatch.

`scripts/check-production-smoke.ps1` and `scripts/check-production-smoke.sh`
are the hosted read-only release gate. They run against public API and MCP URLs
and avoid mutating bounties, ledgers, Stripe state, Base broadcasts, or chain-log
reconciliation. Use the optional eval-history requirement after the environment
has persisted at least one eval run.

`scripts/check-production-compose.ps1` and
`scripts/check-production-compose.sh` are the local production-container gate.
They validate and build the optional Base indexer worker service, build the
production API/MCP/Postgres compose topology, run the read-only production smoke
against high local ports, and tear the stack down. The separate GitHub Actions
`Containers` workflow runs this gate for production packaging changes and on
manual dispatch.
