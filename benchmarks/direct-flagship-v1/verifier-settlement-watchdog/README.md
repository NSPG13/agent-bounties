# Verifier settlement watchdog benchmark

This immutable benchmark defines the paid outcome for the flagship verifier
settlement watchdog bounty. The solver must deliver a production implementation
that keeps canonically submitted regression bounties moving through the existing
runner, signer, and relay workflows without ever making a verification or
payment decision itself.

Run the benchmark from a submitted source snapshot with:

```text
python /benchmark/check.py
```

The sandbox sets `WORKSPACE_ROOT=/workspace`. The command is network-free,
deterministic, and must finish in 20 minutes with the pinned runner limits.

## Required interface

Add `scripts/regression_verifier_watchdog.py` with this command:

```text
python scripts/regression_verifier_watchdog.py plan \
  --jobs JOBS.json \
  --runs RUNS.json \
  --policy POLICY.json \
  --now 2026-09-01T12:00:00Z
```

It must print one JSON object using schema
`agent-bounties/regression-verifier-watchdog-plan-v1`. The object must contain
`network`, `generated_at`, `fail_closed: true`, and a `jobs` array. Each input
job must produce exactly one record with:

- `job_id`, `canonical_job_hash`, and `verification_expires_at` copied from
  canonical input;
- one `next_action` and one `next_owner`;
- `automation_allowed`, which is true only for a bounded, allowlisted retry;
- `target_workflow`, restricted to the precommitted runner or signer workflow;
- `workflow_run_id` for a bounded retry of an existing signer/relay run, and
  `null` for a new runner dispatch;
- a stable `sha256:<64 lowercase hex>` `idempotency_key` bound to the canonical
  job hash, selected action, provider role, target workflow, exact workflow run
  ID (or null for a dispatch), and current protected-main SHA;
- a provider **role**, never a provider URL or secret;
- a plain-language `reason` and exact `recheck_at` timestamp.

For an allowlisted automated retry, `recheck_at` is exactly `generated_at` plus
the policy backoff. For a terminal observation, expiry, reconciliation, or
fail-closed escalation, it is exactly `generated_at`. Every timestamp is a
parseable RFC 3339 UTC instant.

Records are ordered by verification deadline and then job ID. Repeating the
same command with the same inputs must produce byte-for-byte identical output.

Allowed automated actions are `dispatch_runner`, `retry_runner`,
`retry_signer_one`, `retry_signer_two`, and `retry_relay`. Safe non-automated
actions include `observe_terminal`, `expire_submission`,
`await_active_run`, `reconcile_canonical_state`, and `escalate_no_verdict`.
Queued or in-progress runner, signer, or relay runs must produce
`await_active_run` and no automation. The watchdog may never
emit or execute a verdict, attestation, signature, settlement, payment, wallet,
or arbitrary workflow action.

The same tool must expose the production execution boundary:

```text
WATCHDOG_GITHUB_TOKEN=... python scripts/regression_verifier_watchdog.py execute \
  --plan PLAN.json \
  --repository NSPG13/agent-bounties \
  --github-api-base https://api.github.com \
  --token-env WATCHDOG_GITHUB_TOKEN \
  --state .watchdog/state.json \
  --execute
```

Execution must reject a different repository, stale main, a non-allowlisted
workflow, a missing/invalid run ID, or an action whose workflow does not match
the current GitHub run metadata before making a write request. A new runner
action may dispatch only `regression-verifier-runner.yml` on `main`. A bounded
signer or relay retry may call only `rerun-failed-jobs` on a run whose effective
path is `.github/workflows/regression-verifier-signer.yml` and whose head is
exact current main. The token is read only from the named environment variable,
never a command-line value or output artifact.
The production command must pin the API origin to `https://api.github.com` and
must reject every other non-test origin before sending the repository token.

All actions in one plan are atomic at the write boundary: validate the entire
plan, fetch current main, and fetch/revalidate every referenced workflow run
before issuing the first POST. If any later action is unsafe, execute no action.
The state file records successfully executed idempotency keys atomically. An
unchanged plan replay must make no GitHub request and report the already
executed actions as skipped.

## Required behavior

- Prioritize the earliest live verification deadline.
- Isolate one bad job so another valid job still gets a plan.
- Dispatch a runner for a live job with no candidate.
- Retry only the missing or retryable stage, within the attempt and time budget.
- Wait without automation when the selected stage already has a queued or
  in-progress workflow run.
- Use the secondary provider role after a retryable primary-provider failure.
- Refuse automation for stale-main artifacts, canonical drift, replay,
  duplicate signer evidence, unknown workflows, exhausted retries, and too
  little remaining time.
- Treat canonical terminal events as final and never infer payment from a
  workflow result or transaction hash.
- Plan permissionless submission expiry only after the immutable deadline.

Add deterministic implementation tests at
`scripts/test_regression_verifier_watchdog.py`, incident fixtures under
`scripts/fixtures/regression_verifier_watchdog/`, concise documentation, and a
scheduled `.github/workflows/regression-verifier-watchdog.yml`. The workflow
may have only `contents: read` and `actions: write`, must run from exact current
`main`, must use the repository token without other secrets, and must allowlist
only the existing regression runner/signer workflows.

The scheduled workflow must use one repository-wide concurrency group with
`cancel-in-progress: false`, preventing overlapping schedules from issuing the
same idempotent action before GitHub run state catches up.
It must restore and save `.watchdog/state.json` with pinned `actions/cache`
restore/save actions so successfully executed keys survive later schedules.
The job must inherit the exact top-level permission map; job-level permission
overrides are forbidden. Its only steps are a commit-pinned checkout, a
commit-pinned cache restore, the exact single watchdog execute argv, and a
commit-pinned cache save. Shell suffixes, extra commands, and unrelated actions
are forbidden.

The signer and relay paths must accept distinct provider variables for signer
one, signer two, and relay, each with a safe public fallback. Provider URLs must
not appear in watchdog artifacts.

## Evidence boundary

A watchdog plan, workflow rerun, benchmark pass, signature, or transaction hash
is not a verdict or payment. Only the precommitted verifier policy can accept or
reject work, and only a confirmed canonical `BountySettled` event proves solver
payment.
