# Sandboxed Regression Verifier

`sandboxed_regression_v1` is the deterministic coding-verification runner. It
executes a source snapshot against tests committed before bounty funding and
emits an unsigned verifier candidate. It does not sign, relay, settle, or prove
payment.

## Current Status

Implemented and enabled for the precommitted Base-mainnet verifier set:

- exact autonomous verification-job validation;
- content-addressed source and benchmark staging;
- immutable OCI image and direct-argv validation;
- network-disabled, read-only, non-root Docker execution;
- CPU, memory, process, time, output, and tmpfs bounds;
- scope-bound receipt and `0x` bytes32 response hash;
- pass/fail candidates only for completed ordinary exits;
- no verdict on timeout, output overflow, resource kill, input mismatch, or
  runtime failure;
- an exact pre-submission sandbox preflight, returned by submission
  preparation, that must pass before a cooperative client signs;
- per-job infrastructure-failure records so one invalid input does not prevent
  later jobs in the same scheduled batch from running;
- a scheduled no-secrets GitHub runner that emits candidates only;
- isolated signing jobs that re-fetch current state before signing;
- a separate keeper relay that revalidates the exact committed verifier set
  before broadcast.

Direct coding bounties default to one precommitted automated verifier. A second
verifier is optional for higher-risk work; the two project signer keys are
cryptographically distinct but share project governance, so threshold two is
redundancy rather than organizationally independent review. Workflow success
without a canonical job or a confirmed `BountySettled` event is not completion
or payment evidence.

## Immutable Terms

The default verification policy names one verifier:

```json
{
  "mechanism": "signed_quorum",
  "engine": "sandboxed_regression_v1",
  "verifiers": ["0xVerifierOne"],
  "threshold": 1
}
```

For higher-risk work, commit two distinct verifier wallets and threshold two.
The contract calls both forms `signed_quorum`; product surfaces call threshold
one **single verifier**.

The benchmark commits the full runner manifest:

```json
{
  "engine": "sandboxed_regression_v1",
  "runner_manifest": {
    "schema_version": "agent-bounties/regression-sandbox-v1",
    "image": "registry.example/verifier@sha256:<64-lowercase-hex>",
    "command": ["cargo", "test", "--locked", "--target-dir", "/tmp/target"],
    "workdir": "/workspace",
    "benchmark_digest": "sha256:<64-lowercase-hex>",
    "timeout_seconds": 120,
    "cpu_millis": 1000,
    "memory_bytes": 536870912,
    "pids_limit": 128,
    "max_output_bytes": 1048576,
    "tmpfs_bytes": 268435456,
    "max_source_bytes": 536870912,
    "max_source_files": 50000,
    "max_benchmark_bytes": 67108864,
    "max_benchmark_files": 10000,
    "platform": "linux/amd64",
    "test_seed": 1
  }
}
```

Shell entrypoints and mutable image tags are invalid. Submission evidence must
contain `source_snapshot_digest` using the same directory-digest algorithm as
the worker.

## Before Submission

`prepare_autonomous_bounty_submission` returns a
`verifier_preflight.job` for sandboxed-regression bounties. Save the complete
preparation response and run the exact job through the same download, staging,
digest-validation, image, and worker path used by the hosted verifier:

```powershell
python scripts/regression_verifier_pipeline.py preflight `
  --input path\to\submission-preparation.json `
  --worker target\release\worker.exe `
  --staging target\regression-preflight-staging `
  --output target\regression-preflight-receipt.json
```

Do not sign or relay unless the command succeeds and the receipt contains
`schema=agent-bounties/regression-preflight-v1`, `status=passed`, and
`safe_to_sign=true`. Infrastructure failures and ordinary benchmark failures
both fail closed. The receipt is advisory in autonomous-v1 and cannot be reused
as a verifier attestation, settlement, or payment evidence.

The exact preventive contract boundary for a future protocol version is a
short-lived hosted preflight receipt committed into the submission
authorization. That would make skipping preflight impossible, but it requires
a new signed message/contract version rather than an unsafe reinterpretation
of existing immutable bounties.

## Scheduled Failure Isolation

The no-secrets runner catches input and infrastructure failures per job, writes
one `agent-bounties/regression-infrastructure-failure-v1` record with
`verdict_emitted=false`, adds it to `manifest.json`, and continues with the next
eligible job. GitHub Actions receives a warning and step-summary entry. The
artifact upload runs with `if: always()` so partial diagnostics survive an
unexpected workflow failure. A failed job never becomes a pass/fail candidate
and the signing workflows consume only the manifest's `candidates` list.

## Local Rehearsal

Stage operator-provided directories into the runner-owned store:

```powershell
cargo run -p worker -- --stage-regression-input source `
  path\to\source target\regression-staging 536870912 50000
cargo run -p worker -- --stage-regression-input benchmark `
  path\to\benchmark target\regression-staging 67108864 10000
```

Fetch one canonical job from
`GET /v1/base/autonomous-bounties/verification-jobs`, save only the returned job
inside `{"job": ...}`, and run:

```powershell
$env:REGRESSION_SANDBOX_STAGING_ROOT = "$PWD\target\regression-staging"
$env:REGRESSION_SANDBOX_DOCKER_BINARY = "docker"
cargo run -p worker -- --run-regression path\to\request.json
```

The request cannot choose host paths or override policy. The worker recomputes
terms, policy, benchmark, evidence, artifact, and staging digests before
execution.

Run the deterministic and live-Docker harnesses with:

```powershell
cargo test -p verifier-sdk
cargo test -p worker
cargo test -p worker `
  docker_rehearsal_passes_fails_and_produces_no_infrastructure_verdicts `
  -- --ignored
```

## Deployment Boundary

Do not mount a Docker socket into the Base indexer or any service holding RPC,
database, wallet, Stripe, or operator secrets. A hosted runner must be a
separate no-secrets service with a runner-owned staging volume. Signing must be
a separate capability that verifies a fresh candidate against the current
canonical job and requires exactly the verifier path or paths committed before
funding.

The signer and keeper workflows read their Base endpoint from the dedicated
`REGRESSION_VERIFIER_RPC_URL` Actions variable and otherwise use
`https://mainnet.base.org`. Keep this separate from the application's general
`BASE_MAINNET_RPC_URL`: an unavailable shared provider must not disable the
isolated verification path. Any configured endpoint remains subject to the
same exact current-job revalidation and on-chain signature checks; an RPC
response, signature, or broadcast is never settlement evidence.

Only confirmed canonical `BountySettled` is payout evidence. A runner receipt,
response hash, verifier signature, quorum plan, relay transaction hash, or
hosted database row is not payment evidence.
