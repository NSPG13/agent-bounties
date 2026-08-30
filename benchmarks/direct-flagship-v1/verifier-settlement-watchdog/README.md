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

- `job_id` and `verification_expires_at` copied from canonical input;
- one `next_action` and one `next_owner`;
- `automation_allowed`, which is true only for a bounded, allowlisted retry;
- a stable `sha256:<64 lowercase hex>` `idempotency_key`;
- a provider **role**, never a provider URL or secret;
- a plain-language `reason` and exact `recheck_at` timestamp.

Records are ordered by verification deadline and then job ID. Repeating the
same command with the same inputs must produce byte-for-byte identical output.

Allowed automated actions are `dispatch_runner`, `retry_runner`,
`retry_signer_one`, `retry_signer_two`, and `retry_relay`. Safe non-automated
actions include `observe_terminal`, `expire_submission`,
`reconcile_canonical_state`, and `escalate_no_verdict`. The watchdog may never
emit or execute a verdict, attestation, signature, settlement, payment, wallet,
or arbitrary workflow action.

## Required behavior

- Prioritize the earliest live verification deadline.
- Isolate one bad job so another valid job still gets a plan.
- Dispatch a runner for a live job with no candidate.
- Retry only the missing or retryable stage, within the attempt and time budget.
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

The signer and relay paths must accept distinct provider variables for signer
one, signer two, and relay, each with a safe public fallback. Provider URLs must
not appear in watchdog artifacts.

## Evidence boundary

A watchdog plan, workflow rerun, benchmark pass, signature, or transaction hash
is not a verdict or payment. Only the precommitted verifier policy can accept or
reject work, and only a confirmed canonical `BountySettled` event proves solver
payment.
