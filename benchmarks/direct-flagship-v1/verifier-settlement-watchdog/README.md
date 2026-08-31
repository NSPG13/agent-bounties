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
- `workflow_run_id`, `workflow_job_id`, and `workflow_run_attempt` for one
  bounded retry of an exact failed job, and `null` for every non-writing wait;
- `affected_workflow_jobs`, an exact ordered list containing the target job and
  every dependent job GitHub will also execute. It is empty for non-writing
  records. A signer retry binds its exact signer as `target` and the exact
  same-run relay as `dependent`; runner and relay retries bind only themselves;
- a stable `sha256:<64 lowercase hex>` `idempotency_key` bound to the canonical
  job hash, selected action, provider role, target workflow, exact workflow run
  ID, job ID, run attempt, complete affected-job list, and current
  protected-main SHA;
- a provider **role**, never a provider URL or secret;
- a plain-language `reason` and exact `recheck_at` timestamp.

For an allowlisted automated retry, `recheck_at` is exactly `generated_at` plus
the policy backoff. For a terminal observation, expiry, reconciliation, or
fail-closed escalation, it is exactly `generated_at`. Every timestamp is a
parseable RFC 3339 UTC instant.

Records are ordered by verification deadline and then job ID. Repeating the
same command with the same inputs must produce byte-for-byte identical output.

Allowed automated actions are `retry_runner`, `retry_signer_one`,
`retry_signer_two`, and `retry_relay`. Safe non-automated actions include
`await_scheduled_runner`, `await_shared_retry`, `observe_terminal`,
`expire_submission`, `await_active_run`, `reconcile_canonical_state`, and
`escalate_no_verdict`.
Queued or in-progress runner, signer, or relay runs must produce
`await_active_run` and no automation. The watchdog may never
emit or execute a verdict, attestation, signature, settlement, payment, wallet,
or arbitrary workflow action.
The short visibility gap after a candidate runner succeeds but before its
`workflow_run` signer appears must also produce `await_active_run`, with no
target workflow, run ID, or write. The same rule applies between successful
signer stages and a not-yet-visible downstream stage. A retry is allowed only
after the exact downstream stage has appeared and failed retryably.

The same tool must expose the production execution boundary:

```text
GITHUB_TOKEN=... python scripts/regression_verifier_watchdog.py plan-live \
  --api-base https://api.agentbounties.app \
  --repository NSPG13/agent-bounties \
  --github-api-base https://api.github.com \
  --token-env GITHUB_TOKEN \
  --policy ops/regression-verifier-watchdog-policy.json \
  --output target/watchdog-plan.json \
  --allow-workflow regression-verifier-runner.yml \
  --allow-workflow regression-verifier-signer.yml

WATCHDOG_GITHUB_TOKEN=... python scripts/regression_verifier_watchdog.py execute \
  --plan PLAN.json \
  --repository NSPG13/agent-bounties \
  --github-api-base https://api.github.com \
  --token-env WATCHDOG_GITHUB_TOKEN \
  --state .watchdog/state.json \
  --execute \
  --allow-workflow regression-verifier-runner.yml \
  --allow-workflow regression-verifier-signer.yml
```

`plan-live` must build the executable plan itself from the pinned production
verification-job feed, protected-main metadata, and GitHub Actions run state.
It must work from a clean checkout without a pre-created `target/watchdog-plan.json`,
write that file before execution, and send the repository token only to the
pinned `https://api.github.com` origin. The Agent Bounties feed is pinned to
`https://api.agentbounties.app`. The exact workflow allowlist is required on
both commands and cannot be expanded by a plan or environment value.
The live path must consume the real source schemas: the platform response is a
bare `AutonomousVerificationJob` array with a Unix-seconds deadline, while
GitHub returns `workflow_runs` and per-run `jobs`. It must derive a stable
canonical job hash from each complete production job and normalize only the
effective allowlisted runner/signer job states; benchmark-private `jobs` or
`runs` wrappers are not production inputs.

A GitHub job status applies only to canonical jobs proven present in that exact
runner's bounded `regression-candidates-<run_id>` artifact. `plan-live` must
fetch the run's artifact metadata, inspect the archive without extracting it,
validate the exact manifest/file names and size limits, and hash each complete
embedded production job. Signer runs must expose their upstream candidate run
ID in the exact run name and may inherit only that runner's proven membership.
An old run, a missing/expired artifact, a job-ID-only match, or a run timestamp
is not processing evidence. A newly submitted job must wait for the next fixed
runner schedule rather than inherit status from an older workflow run. The
watchdog never creates a new runner execution. When several current-main
observations exist for one stage, monotonic GitHub run, attempt, and job IDs
select the newest observation regardless of API response order.

Execution must reject a different repository, stale main, a non-allowlisted
workflow, missing/invalid run, job, or attempt IDs, duplicate actions against
one workflow run, or metadata that does not bind the expected exact failed job
before making a write request. A bounded retry may call only
`POST /actions/jobs/{workflow_job_id}/rerun` for the expected job name in a
completed failed run whose workflow and head are exact current main. It may
never use a run-wide rerun endpoint or dispatch a workflow. The token is read
only from the named environment variable, never a command-line value or output
artifact.
The production command must pin the API origin to `https://api.github.com` and
must reject every other non-test origin before sending the repository token.

All actions in one plan are atomic at the write boundary: validate the entire
plan, fetch current main, and fetch/revalidate every referenced workflow run
and exact job before issuing the first POST. If any later action is unsafe,
execute no action.
The state file records successfully executed idempotency keys atomically. An
unchanged plan replay must make no GitHub request and report the already
executed actions as skipped. If a later POST fails after an earlier POST
succeeds, the earlier key must already be durable; a retry may issue only the
remaining POST.
For signer retries, GitHub's exact-job endpoint also reruns dependent jobs. The
plan and idempotency key must therefore bind the exact same-run relay job, and
the executor must validate the signer target and relay dependency before the
single signer POST. An unmodeled dependent job is a hard failure.

If GitHub accepts a job retry but the connection fails before local state is
written, a greater run-wide `run_attempt` is insufficient evidence. The next
execution must query all attempts for that run and find the exact target job
name at a greater job attempt before recording the action without another POST.
An attempt increase caused by another job must fail closed without writing.

## Required behavior

- Prioritize the earliest live verification deadline.
- Isolate one bad job so another valid job still gets a plan.
- Wait for the fixed schedule when a live job has no candidate.
- Retry only the missing or retryable stage, within the attempt and time budget.
- Let only the earliest-deadline canonical job own a retry when several jobs
  share one workflow run; the other jobs wait for that shared retry.
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
checked-in `ops/regression-verifier-watchdog-policy.json`, plus a scheduled
`.github/workflows/regression-verifier-watchdog.yml`. The workflow
may have only `contents: read` and `actions: write`, must run from exact current
`main`, must use the repository token without other secrets, and must allowlist
only the existing regression runner/signer workflows.

The scheduled workflow must use one repository-wide concurrency group with
`cancel-in-progress: false`, preventing overlapping schedules from issuing the
same idempotent action before GitHub run state catches up.
It must be schedule-only. `workflow_dispatch` is forbidden because GitHub lets
manual dispatch select a branch containing a modified workflow; scheduled
workflows run only from the protected default branch.
It must restore and save `.watchdog/state.json` with commit-pinned `actions/cache`
restore/save actions so successfully executed keys survive later schedules.
Cache save must run with `if: ${{ always() }}` so a successful first action is
not forgotten when a later action fails.
The job must inherit the exact top-level permission map; job-level permission
overrides are forbidden. The workflow must use strict JSON-syntax YAML so the
benchmark validates the effective document rather than substring spellings.
Its only job has no `defaults`, custom shell, container, services, or extra
keys. Its only steps are: commit-pinned checkout of `${{ github.repository }}`
at `main` with credentials disabled; commit-pinned cache restore; the exact
`plan-live` argv; the exact `execute` argv; and commit-pinned cache save.
Shell suffixes, shell comments that hide arguments, extra commands, extra jobs,
and unrelated actions are forbidden.

The signer and reusable-signer workflows must also use strict JSON-syntax YAML,
so provider bindings are read from their effective parsed jobs rather than raw
text boundaries or decoy scalars. Their effective sign and relay commands must
equal the complete precommitted argv; extra substitutions, comments, suffixes,
or shell operators are forbidden.
Their complete job settings and ordered step lists must equal the checked-in
allowlist, with every third-party action commit-pinned. Signing and keeper
private keys are forbidden at job scope and are exposed only in the `env` map
of the single exact sign or relay command that consumes them. Extra steps,
including a command before or after the expected pipeline command, are
forbidden. The signer workflow run name must bind the exact upstream candidate
runner ID so live planning can prove job membership from the runner artifact.

The candidate-producing `.github/workflows/regression-verifier-runner.yml`
must also use strict JSON-syntax YAML and equal the complete benchmarked
contract. It may expose only the exact fixed schedule; `workflow_dispatch` is
forbidden. It has `contents: read`, one serialized
`run-no-secrets` job, no secret environment values, and an exact ordered list
of commit-pinned actions and commands. Checkout is bound to
`${{ github.repository }}` at `main` with persisted credentials disabled. The
only candidate-producing command is the exact precommitted
`regression_verifier_pipeline.py run` invocation, and only its exact output is
uploaded under `regression-candidates-${{ github.run_id }}`. Extra jobs, steps,
commands, checkout refs, artifact paths, or unpinned actions are forbidden.
The signer accepts a runner only when its event is `schedule` and its
repository, branch, revision, and artifact all bind to current protected
`main`.

Each signer and relay path has an exact primary and a distinct exact secondary
RPC. Signer one uses `https://mainnet.base.org` then
`https://developer-access-mainnet.base.org`; signer two uses
`https://base-rpc.publicnode.com` then
`https://base-mainnet.public.blastapi.io`; relay uses `https://1rpc.io/base`
then `https://base.meowrpc.com`. GitHub increments `github.run_attempt` when the
watchdog reruns a failed job: attempt one must select the primary and the only
allowed retry must select the secondary. Keeping the primary on retry, using
identical endpoints, or accepting an operator-selected URL is forbidden. The
effective signer and relay command argv must consume the selected
`BASE_MAINNET_RPC_URL`; text hidden after a shell comment does not qualify.
Provider URLs must not appear in watchdog plan or state artifacts.

The immutable checker binds the exact checkout bytes of the signing runtime and
both security test suites; line-ending-only changes therefore fail closed. It
also binds every Cargo workspace, lockfile, configuration, local
crate input, and any root `rust-toolchain` or `rust-toolchain.toml` override
used to build the regression worker. It resolves every literal Rust
`include_str!` and `include_bytes!` input, including files outside crate roots,
with a token scanner that ignores legal line and nested block comments, and
rejects dynamic include paths it cannot bind. Contract tests also require
both PR workflows to watch every resolved external input. The runner, both
signers, and the relay verify that complete
build-input digest before `cargo build`, then recheck the build inputs and
signing runtime after the build and before any private key is exposed. Signing
and keeper keys exist only in the final exact step that consumes them. Adding a
`build.rs` or toolchain override, changing a local dependency, or mutating the
Python runtime therefore fails closed. Workflow command
allowlisting alone is insufficient because a changed executable behind the
same command would otherwise inherit signing authority. Candidate
revalidation runs with signing and keeper secrets removed from its environment.
Before a private key is passed to any child process, the signer verifies Base
chain ID 8453 and, at the RPC's `safe` block, requires the exact precommitted
canonical EIP-1167 clone runtime, factory
`0x082c52131aaf0c56e76b075f895eab6fcab6d2f9`, and Base USDC settlement token
`0x833589fcd6edb6e08f4c7c32d4f71b54bda02913` at the committed bounty
contract. It also computes the EIP-712 attestation digest locally from
canonical fields. The RPC contract digest must equal that independent local
digest; disagreement fails closed.

Before the keeper key is passed to any child process, the relay validates every
candidate and attestation, requires Base chain ID 8453 and the same safe-block
canonical runtime, factory, and token provenance at each exact bounty,
simulates each exact settlement call from the checked-in keeper address, and
enforces a 500,000 gas ceiling. It reads both the latest and pending keeper
nonces through two independent Base RPCs and refuses to send unless every
latest/pending value agrees, so a pending or stale nonce view fails closed. It
repeats that independent check immediately before every send. Every checked-in
job that can receive `BASE_KEEPER_PRIVATE_KEY` is serialized under the
repository-wide `agent-bounties-shared-base-keeper` concurrency group; the
lock is forbidden at workflow scope and on validation-only jobs so public event
churn cannot occupy or replace a pending key-bearing transaction job. Both
`.yml` and `.yaml` files are parsed structurally, and block-scalar text cannot
impersonate concurrency keys. YAML anchors and aliases are forbidden because
they can move a secret into a different effective job without a visible scalar
there. Every workflow is inspected before deciding whether it carries the key;
escaped or multiline quoted YAML scalars are rejected, and JSON escapes are
resolved before secret detection. Dot and literal-bracket keeper access allow
normal expression whitespace; any keeper-key name marks its job as
key-bearing, and dynamic secret indexing is forbidden. Bare, serialized, or
wrapped access to the complete `secrets` context is also forbidden; only direct
named access is allowed. JSON keeper references outside `jobs` fail closed so
workflow-level inheritance cannot bypass job attribution. The
secret-bearing send explicitly sets chain 8453, the preflighted nonce, a
500,000 gas limit, a 0.5 gwei maximum fee, and a 0.001 gwei priority fee. An
unbounded or RPC-selected transaction parameter is forbidden.

## Evidence boundary

A watchdog plan, workflow rerun, benchmark pass, signature, or transaction hash
is not a verdict or payment. Only the precommitted verifier policy can accept or
reject work, and only a confirmed canonical `BountySettled` event proves solver
payment.
