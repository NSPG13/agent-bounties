# Sandboxed Regression Verifier

`sandboxed_regression_v1` is the deterministic coding-verification runner. It
executes a source snapshot against tests committed before bounty funding and
emits an unsigned verifier candidate. It does not sign, relay, settle, or prove
payment.

## Current Status

Implemented and tested locally:

- exact autonomous verification-job validation;
- content-addressed source and benchmark staging;
- immutable OCI image and direct-argv validation;
- network-disabled, read-only, non-root Docker execution;
- CPU, memory, process, time, output, and tmpfs bounds;
- scope-bound receipt and `0x` bytes32 response hash;
- pass/fail candidates only for completed ordinary exits;
- no verdict on timeout, output overflow, resource kill, input mismatch, or
  runtime failure.

Not enabled in production:

- a hosted runner service;
- verifier keys or signatures;
- independent quorum operators;
- attestation relay;
- verifier-readiness publication.

Until those pieces pass a separate deployment review, quorum bounties remain
absent from earning-ready inventory.

## Immutable Terms

The verification policy must name the engine and at least two distinct verifier
wallets:

```json
{
  "mechanism": "signed_quorum",
  "engine": "sandboxed_regression_v1",
  "verifiers": ["0xVerifierOne", "0xVerifierTwo"],
  "threshold": 2
}
```

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
canonical job and requires at least two distinct precommitted verifier paths.

Only confirmed canonical `BountySettled` is payout evidence. A runner receipt,
response hash, verifier signature, quorum plan, relay transaction hash, or
hosted database row is not payment evidence.
