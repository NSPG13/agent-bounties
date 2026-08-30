#!/usr/bin/env python3
"""Immutable acceptance checks for the verifier settlement watchdog."""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
NOW = "2026-09-01T12:00:00Z"
MAIN_SHA = "a" * 40
SCHEMA = "agent-bounties/regression-verifier-watchdog-plan-v1"
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ADDRESS_ONE = "0x" + "11" * 20
ADDRESS_TWO = "0x" + "22" * 20


def require(path: str) -> Path:
    candidate = ROOT / path
    if not candidate.is_file():
        raise SystemExit(f"missing required file: {path}")
    return candidate


def hash32(value: str) -> str:
    return "0x" + hashlib.sha256(value.encode("utf-8")).hexdigest()


def job(
    name: str,
    expiry: str,
    *,
    status: str = "submitted",
    readiness: str = "ready",
    canonical_event: str | None = None,
) -> dict[str, Any]:
    return {
        "job_id": name,
        "bounty_contract": "0x" + hashlib.sha1(name.encode()).hexdigest()[:40],
        "round": 3,
        "status": status,
        "canonical_job_hash": hash32(f"job:{name}"),
        "submission_hash": hash32(f"submission:{name}"),
        "verification_expires_at": expiry,
        "required_verifiers": [ADDRESS_ONE, ADDRESS_TWO],
        "threshold": 2,
        "input_readiness": readiness,
        "canonical_terminal_event": canonical_event,
    }


def run(
    job_id: str,
    stage: str,
    conclusion: str,
    *,
    attempt: int = 1,
    head_sha: str = MAIN_SHA,
    provider_role: str | None = None,
    artifact_hash: str | None = None,
    retryable: bool = True,
    signer: str | None = None,
    workflow: str | None = None,
    canonical_job_hash: str | None = None,
) -> dict[str, Any]:
    return {
        "job_id": job_id,
        "stage": stage,
        "status": "completed",
        "conclusion": conclusion,
        "attempt": attempt,
        "head_sha": head_sha,
        "workflow": workflow or {
            "runner": "regression-verifier-runner.yml",
            "signer_one": "regression-verifier-signer.yml",
            "signer_two": "regression-verifier-signer.yml",
            "relay": "regression-verifier-signer.yml",
        }[stage],
        "provider_role": provider_role,
        "artifact_hash": artifact_hash,
        "retryable": retryable,
        "signer": signer,
        "canonical_job_hash": canonical_job_hash,
    }


POLICY = {
    "schema": "agent-bounties/regression-verifier-watchdog-policy-v1",
    "network": "base-mainnet",
    "current_main_sha": MAIN_SHA,
    "max_attempts_per_stage": 2,
    "minimum_retry_budget_seconds": 900,
    "backoff_seconds": 300,
    "allowed_workflows": [
        "regression-verifier-runner.yml",
        "regression-verifier-signer.yml",
    ],
    "provider_roles": {
        "signer_one": ["signer_one_primary", "signer_one_secondary"],
        "signer_two": ["signer_two_primary", "signer_two_secondary"],
        "relay": ["relay_primary", "relay_secondary"],
    },
}


def invoke(jobs: list[dict[str, Any]], runs: list[dict[str, Any]]) -> tuple[bytes, dict[str, Any]]:
    tool = require("scripts/regression_verifier_watchdog.py")
    with tempfile.TemporaryDirectory(prefix="watchdog-benchmark-") as temporary:
        directory = Path(temporary)
        jobs_path = directory / "jobs.json"
        runs_path = directory / "runs.json"
        policy_path = directory / "policy.json"
        jobs_path.write_text(
            json.dumps(
                {
                    "schema": "agent-bounties/regression-verifier-watchdog-jobs-v1",
                    "network": "base-mainnet",
                    "safe_block": 50_700_000,
                    "jobs": jobs,
                }
            ),
            encoding="utf-8",
        )
        runs_path.write_text(
            json.dumps(
                {
                    "schema": "agent-bounties/regression-verifier-watchdog-runs-v1",
                    "repository": "NSPG13/agent-bounties",
                    "current_main_sha": MAIN_SHA,
                    "runs": runs,
                }
            ),
            encoding="utf-8",
        )
        policy_path.write_text(json.dumps(POLICY), encoding="utf-8")
        command = [
            sys.executable,
            str(tool),
            "plan",
            "--jobs",
            str(jobs_path),
            "--runs",
            str(runs_path),
            "--policy",
            str(policy_path),
            "--now",
            NOW,
        ]
        completed = subprocess.run(
            command,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=30,
            check=False,
        )
        if completed.returncode != 0:
            raise SystemExit(
                "watchdog planner failed:\n" + completed.stdout.decode("utf-8", "replace")[-5000:]
            )
        raw = completed.stdout
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError as error:
            raise SystemExit("watchdog planner did not emit one JSON object") from error
        repeated = subprocess.run(
            command,
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=30,
            check=False,
        )
        if repeated.returncode != 0 or repeated.stdout != raw:
            raise SystemExit("watchdog plan is not byte-for-byte deterministic")
        return raw, parsed


def validate(plan: dict[str, Any], expected_jobs: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    if plan.get("schema") != SCHEMA:
        raise SystemExit("watchdog plan schema is invalid")
    if plan.get("network") != "base-mainnet" or plan.get("generated_at") != NOW:
        raise SystemExit("watchdog plan network or generated_at drifted")
    if plan.get("fail_closed") is not True:
        raise SystemExit("watchdog plan must explicitly fail closed")
    records = plan.get("jobs")
    if not isinstance(records, list) or len(records) != len(expected_jobs):
        raise SystemExit("watchdog must return exactly one record per input job")
    expected_order = [
        item["job_id"]
        for item in sorted(expected_jobs, key=lambda item: (item["verification_expires_at"], item["job_id"]))
    ]
    if [item.get("job_id") for item in records] != expected_order:
        raise SystemExit("watchdog records must be deadline ordered")
    by_id: dict[str, dict[str, Any]] = {}
    forbidden_actions = {"accept", "reject", "sign", "settle", "pay", "transfer", "wallet_call"}
    for record in records:
        job_id = record.get("job_id")
        if not isinstance(job_id, str) or job_id in by_id:
            raise SystemExit("watchdog record job_id is missing or duplicated")
        if not str(record.get("next_owner", "")).strip():
            raise SystemExit(f"watchdog record {job_id} has no next owner")
        if not str(record.get("reason", "")).strip():
            raise SystemExit(f"watchdog record {job_id} has no reason")
        if not str(record.get("recheck_at", "")).endswith("Z"):
            raise SystemExit(f"watchdog record {job_id} has no exact UTC recheck")
        if not HASH_RE.fullmatch(str(record.get("idempotency_key", ""))):
            raise SystemExit(f"watchdog record {job_id} has an invalid idempotency key")
        action = record.get("next_action")
        action_text = str(action).lower()
        if (
            action in forbidden_actions
            or "payment" in action_text
            or ("verdict" in action_text and action != "escalate_no_verdict")
        ):
            raise SystemExit(f"watchdog emitted forbidden authority: {action}")
        serialized = json.dumps(record, sort_keys=True).lower()
        if any(token in serialized for token in ("private_key", "seed phrase", "mnemonic", "https://", "http://")):
            raise SystemExit(f"watchdog record {job_id} exposed a secret or provider URL")
        by_id[job_id] = record
    return by_id


def expect(record: dict[str, Any], action: str, automated: bool, provider: str | None = None) -> None:
    if record.get("next_action") != action:
        raise SystemExit(
            f"{record.get('job_id')} expected {action}, got {record.get('next_action')}"
        )
    if record.get("automation_allowed") is not automated:
        raise SystemExit(f"{record.get('job_id')} automation boundary is incorrect")
    if provider is not None and record.get("provider_role") != provider:
        raise SystemExit(f"{record.get('job_id')} expected provider role {provider}")


# Earliest deadline must win, and an unavailable input must not abort another job.
isolated_jobs = [
    job("later-ready", "2026-09-01T14:00:00Z"),
    job("earlier-ready", "2026-09-01T13:00:00Z"),
    job("bad-input", "2026-09-01T12:30:00Z", readiness="unavailable"),
]
_, isolated_plan = invoke(isolated_jobs, [])
isolated = validate(isolated_plan, isolated_jobs)
expect(isolated["bad-input"], "escalate_no_verdict", False)
expect(isolated["earlier-ready"], "dispatch_runner", True)
expect(isolated["later-ready"], "dispatch_runner", True)

# Retry only the missing signer, preserving the successful candidate and signer.
candidate_hash = "sha256:" + "c" * 64
signer_jobs = [job("signer-gap", "2026-09-01T14:00:00Z")]
signer_runs = [
    run("signer-gap", "runner", "success", artifact_hash=candidate_hash),
    run(
        "signer-gap",
        "signer_one",
        "success",
        artifact_hash="sha256:" + "d" * 64,
        signer=ADDRESS_ONE,
        provider_role="signer_one_primary",
    ),
    run(
        "signer-gap",
        "signer_two",
        "failure",
        signer=ADDRESS_TWO,
        provider_role="signer_two_primary",
    ),
]
_, signer_plan = invoke(signer_jobs, signer_runs)
signer = validate(signer_plan, signer_jobs)
expect(signer["signer-gap"], "retry_signer_two", True, "signer_two_secondary")

# A retryable relay provider failure uses only the secondary relay provider.
relay_jobs = [job("relay-gap", "2026-09-01T14:00:00Z")]
relay_runs = [
    run("relay-gap", "runner", "success", artifact_hash=candidate_hash),
    run("relay-gap", "signer_one", "success", signer=ADDRESS_ONE),
    run("relay-gap", "signer_two", "success", signer=ADDRESS_TWO),
    run("relay-gap", "relay", "failure", provider_role="relay_primary", retryable=True),
]
_, relay_plan = invoke(relay_jobs, relay_runs)
relay = validate(relay_plan, relay_jobs)
expect(relay["relay-gap"], "retry_relay", True, "relay_secondary")

# Stale artifacts, replay-like duplicate signers, exhausted attempts, and short
# deadline budgets must never be automated.
unsafe_jobs = [
    job("stale-main", "2026-09-01T15:00:00Z"),
    job("duplicate-signer", "2026-09-01T15:10:00Z"),
    job("attempts-exhausted", "2026-09-01T15:20:00Z"),
    job("unknown-workflow", "2026-09-01T15:30:00Z"),
    job("canonical-drift", "2026-09-01T15:40:00Z"),
    job("nonretryable-relay", "2026-09-01T15:50:00Z"),
    job("too-late", "2026-09-01T12:10:00Z"),
]
unsafe_runs = [
    run("stale-main", "runner", "success", head_sha="b" * 40, artifact_hash=candidate_hash),
    run("duplicate-signer", "runner", "success", artifact_hash=candidate_hash),
    run("duplicate-signer", "signer_one", "success", signer=ADDRESS_ONE),
    run("duplicate-signer", "signer_two", "success", signer=ADDRESS_ONE),
    run("attempts-exhausted", "runner", "failure", attempt=2),
    run("unknown-workflow", "runner", "failure", workflow="unreviewed-wallet-job.yml"),
    run(
        "canonical-drift",
        "runner",
        "success",
        artifact_hash=candidate_hash,
        canonical_job_hash=hash32("different-canonical-job"),
    ),
    run("nonretryable-relay", "runner", "success", artifact_hash=candidate_hash),
    run("nonretryable-relay", "signer_one", "success", signer=ADDRESS_ONE),
    run("nonretryable-relay", "signer_two", "success", signer=ADDRESS_TWO),
    run(
        "nonretryable-relay",
        "relay",
        "failure",
        provider_role="relay_primary",
        retryable=False,
    ),
]
_, unsafe_plan = invoke(unsafe_jobs, unsafe_runs)
unsafe = validate(unsafe_plan, unsafe_jobs)
for job_id in (
    "stale-main",
    "duplicate-signer",
    "attempts-exhausted",
    "unknown-workflow",
    "canonical-drift",
    "nonretryable-relay",
    "too-late",
):
    expect(unsafe[job_id], "escalate_no_verdict", False)

# Canonical state is authoritative. Terminal work is only observed; an expired
# submitted round gets a permissionless expiry plan, never a verdict.
terminal_jobs = [
    job(
        "settled",
        "2026-09-01T11:00:00Z",
        status="settled",
        canonical_event="BountySettled",
    ),
    job("expired", "2026-09-01T11:59:59Z"),
]
_, terminal_plan = invoke(terminal_jobs, [])
terminal = validate(terminal_plan, terminal_jobs)
expect(terminal["settled"], "observe_terminal", False)
expect(terminal["expired"], "expire_submission", False)

# The implementation must ship its own deterministic tests and the tightly
# permissioned production workflow required by the paid outcome.
tests = require("scripts/test_regression_verifier_watchdog.py")
completed = subprocess.run(
    [sys.executable, str(tests), "-v"],
    cwd=ROOT,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=120,
    check=False,
)
if completed.returncode != 0:
    raise SystemExit("watchdog implementation tests failed:\n" + completed.stdout.decode()[-5000:])

workflow = require(".github/workflows/regression-verifier-watchdog.yml").read_text(encoding="utf-8")
workflow_lower = workflow.lower()
for phrase in ("schedule:", "workflow_dispatch:", "actions: write", "contents: read", "--execute"):
    if phrase not in workflow_lower:
        raise SystemExit(f"watchdog workflow is missing {phrase}")
for forbidden in ("pull_request_target", "issue_comment:", "id-token: write", "contents: write", "secrets."):
    if forbidden in workflow_lower:
        raise SystemExit(f"watchdog workflow contains forbidden privilege or trigger: {forbidden}")
for allowed in ("regression-verifier-runner.yml", "regression-verifier-signer.yml"):
    if allowed not in workflow:
        raise SystemExit(f"watchdog workflow does not pin allowlisted workflow {allowed}")
permission_block = workflow_lower.split("permissions:", 1)[1].split("jobs:", 1)[0]
permission_lines = {
    line.strip()
    for line in permission_block.splitlines()
    if line.strip() and not line.lstrip().startswith("#")
}
if permission_lines != {"contents: read", "actions: write"}:
    raise SystemExit("watchdog workflow permissions must be exactly contents: read and actions: write")
referenced_workflows = set(re.findall(r"[a-z0-9_-]+\.ya?ml", workflow_lower))
unknown_workflows = referenced_workflows - {
    "regression-verifier-runner.yml",
    "regression-verifier-signer.yml",
}
if unknown_workflows:
    raise SystemExit(f"watchdog workflow references an unknown workflow: {sorted(unknown_workflows)}")

signer_workflow = require(".github/workflows/regression-verifier-signer.yml").read_text(encoding="utf-8")
for variable in (
    "REGRESSION_VERIFIER_ONE_RPC_URL",
    "REGRESSION_VERIFIER_TWO_RPC_URL",
    "REGRESSION_VERIFIER_RELAY_RPC_URL",
):
    if variable not in signer_workflow:
        raise SystemExit(f"signer workflow is missing independent provider variable {variable}")

documentation = require("docs/sandboxed-regression-verifier.md").read_text(encoding="utf-8").lower()
for phrase in ("watchdog", "fail closed", "idempotency", "provider", "bountysettled"):
    if phrase not in documentation:
        raise SystemExit(f"verifier documentation is missing {phrase}")

print("verifier settlement watchdog acceptance checks passed")
