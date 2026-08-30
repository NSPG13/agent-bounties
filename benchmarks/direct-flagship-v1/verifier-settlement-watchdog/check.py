#!/usr/bin/env python3
"""Immutable acceptance checks for the verifier settlement watchdog."""

from __future__ import annotations

import hashlib
import http.server
import io
import json
import os
import random
import re
import shlex
import socket
import subprocess
import sys
import tempfile
import threading
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
NOW = "2026-09-01T12:00:00Z"
MAIN_SHA = "a" * 40
SCHEMA = "agent-bounties/regression-verifier-watchdog-plan-v1"
HASH_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
ADDRESS_ONE = "0x" + "11" * 20
ADDRESS_TWO = "0x" + "22" * 20
PIPELINE_SHA256 = "6af6dc49cf5b90a314e4f87263abd5d9714cd037513d934604670a72fa33031c"
PIPELINE_TEST_SHA256 = "6edc11c081d6f0592c09c1e9a16f44ed9165f15ce7fd83d222fa8e209b6b3d08"
SOURCE_GUARD_SHA256 = "06185f1a88bc3f8168ce2ed8c6ecec2b4b6d78fa9815a851fc4c21c88225f79c"
SOURCE_GUARD_TEST_SHA256 = "6937f9c76e0b1a526d89fbc16fa4e2ecae1be999b6e208bca7bacbde656a4273"
WORKER_BUILD_SHA256 = "6f9370dfd818959efbda012d22cabb1cb3be485e44d8dad9a183a2e04a1fd7b1"
SIGNING_RUNTIME_SHA256 = "6b8f857539fb5168f2894bf72a32859a85d8edc34e515fa4ee3cc59cff5d26ac"
CANONICAL_WORKFLOW_SHA256 = {
    ".github/workflows/regression-verifier-runner.yml": "e0226724ff53c13637d047d81b40cd16b7290d1259b3602630e842758bd614f4",
    ".github/workflows/regression-verifier-watchdog.yml": "2cc7333b9fa5d613c1f84416bfd5593ef6c7416fc916e5157a3e43eac89b0d68",
    ".github/workflows/regression-verifier-signer.yml": "ae96d5317b33acf0979c0b24c27e9c54715e9ce46cd483a463163bb3247d28df",
    ".github/workflows/regression-verifier-signing-reusable.yml": "b873ce859541e68a96319a67bd95d1eac92e4e6d74307b214297d8e19e5de4f3",
}


def require(path: str) -> Path:
    candidate = ROOT / path
    if not candidate.is_file():
        raise SystemExit(f"missing required file: {path}")
    return candidate


def canonical_workflow_hash(path: str) -> str:
    try:
        document = json.loads(require(path).read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise SystemExit(f"{path} must use strict JSON-syntax YAML") from error
    canonical = json.dumps(document, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


# Reject an effective workflow or trusted-executable mutation before running
# the expensive live state-machine matrix. Whitespace-only JSON formatting
# remains free; executable semantics do not.
for workflow_path, expected_hash in CANONICAL_WORKFLOW_SHA256.items():
    if canonical_workflow_hash(workflow_path) != expected_hash:
        raise SystemExit(f"{workflow_path} differs from the reviewed effective workflow")
pipeline_bytes_early = require("scripts/regression_verifier_pipeline.py").read_bytes().replace(
    b"\r\n", b"\n"
)
pipeline_tests_early = require("scripts/test_regression_verifier_pipeline.py").read_bytes().replace(
    b"\r\n", b"\n"
)
source_guard_early = require("scripts/regression_verifier_source_guard.py").read_bytes().replace(
    b"\r\n", b"\n"
)
source_guard_tests_early = require(
    "scripts/test_regression_verifier_source_guard.py"
).read_bytes().replace(b"\r\n", b"\n")
if hashlib.sha256(pipeline_bytes_early).hexdigest() != PIPELINE_SHA256:
    raise SystemExit("regression verifier pipeline differs from the reviewed executable")
if hashlib.sha256(pipeline_tests_early).hexdigest() != PIPELINE_TEST_SHA256:
    raise SystemExit("regression verifier pipeline tests differ from the reviewed suite")
if hashlib.sha256(source_guard_early).hexdigest() != SOURCE_GUARD_SHA256:
    raise SystemExit("regression verifier source guard differs from the reviewed executable")
if hashlib.sha256(source_guard_tests_early).hexdigest() != SOURCE_GUARD_TEST_SHA256:
    raise SystemExit("regression verifier source guard tests differ from the reviewed suite")
source_guard = require("scripts/regression_verifier_source_guard.py")
for guard_scope, expected_digest in (
    ("worker-build", WORKER_BUILD_SHA256),
    ("signing-runtime", SIGNING_RUNTIME_SHA256),
):
    guarded = subprocess.run(
        [
            sys.executable,
            str(source_guard),
            "--root",
            str(ROOT),
            "--scope",
            guard_scope,
            "--expected-sha256",
            expected_digest,
        ],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=30,
        check=False,
    )
    if guarded.returncode != 0:
        raise SystemExit(
            f"reviewed {guard_scope} source set drifted:\n"
            + guarded.stdout.decode("utf-8", "replace")[-5000:]
        )


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
    conclusion: str | None,
    *,
    run_status: str = "completed",
    attempt: int = 1,
    head_sha: str = MAIN_SHA,
    provider_role: str | None = None,
    artifact_hash: str | None = None,
    retryable: bool = True,
    signer: str | None = None,
    workflow: str | None = None,
    canonical_job_hash: str | None = None,
    workflow_run_id: int | None = None,
    workflow_job_id: int | None = None,
) -> dict[str, Any]:
    identity = int(hashlib.sha256(f"{job_id}:{stage}".encode()).hexdigest()[:12], 16)
    run_identity = int(
        hashlib.sha256(
            f"{job_id}:{'runner' if stage == 'runner' else 'signer-workflow'}".encode()
        ).hexdigest()[:12],
        16,
    )
    return {
        "job_id": job_id,
        "stage": stage,
        "status": run_status,
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
        "workflow_run_id": workflow_run_id
        if workflow_run_id is not None
        else 10_000 + run_identity,
        "workflow_job_id": workflow_job_id
        if workflow_job_id is not None
        else 20_000 + identity,
        "workflow_run_attempt": attempt,
    }


def production_job(name: str, expiry: str) -> dict[str, Any]:
    expires_at = datetime.fromisoformat(expiry.replace("Z", "+00:00"))
    return {
        "job_id": f"base-mainnet:{name}:3",
        "network": "base-mainnet",
        "bounty_id": hash32(f"bounty:{name}"),
        "bounty_contract": "0x" + hashlib.sha1(name.encode()).hexdigest()[:40],
        "round": 3,
        "solver_wallet": "0x" + "33" * 20,
        "verification_mode": "signed_quorum",
        "verifier_module": None,
        "eligible_verifiers": [ADDRESS_ONE, ADDRESS_TWO],
        "threshold": 2,
        "verifier_reward": "10000",
        "current_solver_payout": "100000",
        "verification_expires_at": int(expires_at.timestamp()),
        "terms": {"terms_hash": hash32(f"terms:{name}"), "document": {}},
        "submission_evidence": {
            "evidence_hash": hash32(f"evidence:{name}"),
            "created_at": int(datetime.fromisoformat(NOW.replace("Z", "+00:00")).timestamp()),
        },
        "required_action": "Evaluate the immutable policy and relay a matching quorum.",
        "payout_boundary": "Only confirmed canonical BountySettled proves payment.",
    }


def normalize_production_job(item: dict[str, Any]) -> dict[str, Any]:
    canonical_payload = json.dumps(item, sort_keys=True, separators=(",", ":"))
    return {
        "job_id": item["job_id"],
        "bounty_contract": item["bounty_contract"],
        "round": item["round"],
        "status": "submitted",
        "canonical_job_hash": "0x" + hashlib.sha256(canonical_payload.encode()).hexdigest(),
        "submission_hash": item["submission_evidence"]["evidence_hash"],
        "verification_expires_at": datetime.fromtimestamp(
            item["verification_expires_at"], timezone.utc
        ).isoformat().replace("+00:00", "Z"),
        "required_verifiers": item["eligible_verifiers"],
        "threshold": item["threshold"],
        "input_readiness": "ready",
        "canonical_terminal_event": None,
    }


def candidate_archive(items: list[dict[str, Any]]) -> bytes:
    output = io.BytesIO()
    entries = []
    files: dict[str, object] = {}
    for item in items:
        job_id = item["job_id"]
        filename = f"candidate-{hashlib.sha256(job_id.encode()).hexdigest()}.json"
        entries.append({"job_id": job_id, "file": filename})
        files[filename] = {
            "schema": "agent-bounties/regression-candidate-v1",
            "job": item,
            "outcome": {"verdict": "passed", "response_hash": hash32(f"response:{job_id}")},
            "runner_revision": MAIN_SHA,
        }
    files["manifest.json"] = {
        "schema": "agent-bounties/regression-candidate-manifest-v1",
        "network": "base-mainnet",
        "candidates": entries,
    }
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as bundle:
        for name, value in sorted(files.items()):
            bundle.writestr(name, json.dumps(value, sort_keys=True, separators=(",", ":")))
    return output.getvalue()


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


def validate(
    plan: dict[str, Any],
    expected_jobs: list[dict[str, Any]],
    observed_runs: list[dict[str, Any]],
) -> dict[str, dict[str, Any]]:
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
    expected_by_id = {item["job_id"]: item for item in expected_jobs}
    runs_by_job_stage: dict[tuple[str, str], list[dict[str, Any]]] = {}
    for observed in observed_runs:
        runs_by_job_stage.setdefault((observed["job_id"], observed["stage"]), []).append(observed)
    automated_targets = {
        "retry_runner": "regression-verifier-runner.yml",
        "retry_signer_one": "regression-verifier-signer.yml",
        "retry_signer_two": "regression-verifier-signer.yml",
        "retry_relay": "regression-verifier-signer.yml",
    }
    retry_stages = {
        "retry_runner": "runner",
        "retry_signer_one": "signer_one",
        "retry_signer_two": "signer_two",
        "retry_relay": "relay",
    }
    forbidden_actions = {"accept", "reject", "sign", "settle", "pay", "transfer", "wallet_call"}
    seen_idempotency_keys: set[str] = set()
    seen_automated_run_ids: set[int] = set()
    for record in records:
        job_id = record.get("job_id")
        if not isinstance(job_id, str) or job_id not in expected_by_id or job_id in by_id:
            raise SystemExit("watchdog record job_id is missing or duplicated")
        if record.get("canonical_job_hash") != expected_by_id[job_id]["canonical_job_hash"]:
            raise SystemExit(f"watchdog record {job_id} changed the canonical job hash")
        if record.get("verification_expires_at") != expected_by_id[job_id]["verification_expires_at"]:
            raise SystemExit(f"watchdog record {job_id} changed the canonical verification deadline")
        if not str(record.get("next_owner", "")).strip():
            raise SystemExit(f"watchdog record {job_id} has no next owner")
        if not str(record.get("reason", "")).strip():
            raise SystemExit(f"watchdog record {job_id} has no reason")
        recheck_at = str(record.get("recheck_at", ""))
        try:
            parsed_recheck = datetime.fromisoformat(recheck_at.replace("Z", "+00:00"))
        except ValueError as error:
            raise SystemExit(f"watchdog record {job_id} has no parseable UTC recheck") from error
        if parsed_recheck.tzinfo != timezone.utc or not recheck_at.endswith("Z"):
            raise SystemExit(f"watchdog record {job_id} recheck must be explicit UTC")
        idempotency_key = str(record.get("idempotency_key", ""))
        if not HASH_RE.fullmatch(idempotency_key):
            raise SystemExit(f"watchdog record {job_id} has an invalid idempotency key")
        action = record.get("next_action")
        action_text = str(action).lower()
        if (
            action in forbidden_actions
            or "payment" in action_text
            or ("verdict" in action_text and action != "escalate_no_verdict")
        ):
            raise SystemExit(f"watchdog emitted forbidden authority: {action}")
        target_workflow = record.get("target_workflow")
        workflow_run_id = record.get("workflow_run_id")
        workflow_job_id = record.get("workflow_job_id")
        workflow_run_attempt = record.get("workflow_run_attempt")
        affected_workflow_jobs = record.get("affected_workflow_jobs")
        if record.get("automation_allowed") is True:
            expected_target = automated_targets.get(str(action))
            if target_workflow != expected_target:
                raise SystemExit(f"watchdog record {job_id} does not bind its allowlisted workflow")
            if (
                not isinstance(workflow_run_id, int)
                or workflow_run_id <= 0
                or not isinstance(workflow_job_id, int)
                or workflow_job_id <= 0
                or not isinstance(workflow_run_attempt, int)
                or workflow_run_attempt <= 0
            ):
                raise SystemExit(
                    f"watchdog retry {job_id} must bind positive workflow run, job, and attempt IDs"
                )
            if action in retry_stages:
                matching_runs = runs_by_job_stage.get((job_id, retry_stages[str(action)]), [])
                selected = matching_runs[-1] if matching_runs else {}
                if (
                    workflow_run_id != selected.get("workflow_run_id")
                    or workflow_job_id != selected.get("workflow_job_id")
                    or workflow_run_attempt != selected.get("workflow_run_attempt")
                ):
                    raise SystemExit(
                        f"watchdog retry {job_id} does not bind the selected stage's latest job attempt"
                    )
                affected_stages = [retry_stages[str(action)]]
                if action in {"retry_signer_one", "retry_signer_two"}:
                    affected_stages.append("relay")
                expected_affected = []
                names = {
                    "runner": "run-no-secrets",
                    "signer_one": "sign-one / sign",
                    "signer_two": "sign-two / sign",
                    "relay": "relay",
                }
                for stage in affected_stages:
                    stage_runs = [
                        item
                        for item in runs_by_job_stage.get((job_id, stage), [])
                        if item.get("workflow_run_id") == workflow_run_id
                    ]
                    if not stage_runs:
                        raise SystemExit(
                            f"watchdog retry {job_id} does not model every affected workflow job"
                        )
                    affected = stage_runs[-1]
                    expected_affected.append(
                        {
                            "workflow_job_id": affected["workflow_job_id"],
                            "name": names[stage],
                            "effect": "target" if stage == retry_stages[str(action)] else "dependent",
                        }
                    )
                if affected_workflow_jobs != expected_affected:
                    raise SystemExit(
                        f"watchdog retry {job_id} does not bind the exact target and dependent jobs"
                    )
            if workflow_run_id in seen_automated_run_ids:
                raise SystemExit(
                    f"watchdog retry {job_id} duplicates a workflow-run retry target"
                )
            seen_automated_run_ids.add(workflow_run_id)
        elif affected_workflow_jobs != [] or any(
            value is not None
            for value in (
                target_workflow,
                workflow_run_id,
                workflow_job_id,
                workflow_run_attempt,
            )
        ):
            raise SystemExit(f"non-automated watchdog record {job_id} must not target a workflow")
        key_payload = json.dumps(
            [
                job_id,
                expected_by_id[job_id]["canonical_job_hash"],
                action,
                record.get("provider_role"),
                target_workflow,
                workflow_run_id,
                workflow_job_id,
                workflow_run_attempt,
                affected_workflow_jobs,
                POLICY["current_main_sha"],
            ],
            separators=(",", ":"),
            sort_keys=True,
        )
        expected_key = "sha256:" + hashlib.sha256(key_payload.encode("utf-8")).hexdigest()
        if idempotency_key != expected_key or idempotency_key in seen_idempotency_keys:
            raise SystemExit(f"watchdog record {job_id} has an unbound or duplicate idempotency key")
        seen_idempotency_keys.add(idempotency_key)
        generated = datetime.fromisoformat(NOW.replace("Z", "+00:00"))
        expected_recheck = (
            generated + timedelta(seconds=POLICY["backoff_seconds"])
            if record.get("automation_allowed") is True
            else generated
        )
        if parsed_recheck != expected_recheck:
            raise SystemExit(f"watchdog record {job_id} recheck does not match the policy")
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
isolated = validate(isolated_plan, isolated_jobs, [])
expect(isolated["bad-input"], "escalate_no_verdict", False)
expect(isolated["earlier-ready"], "await_scheduled_runner", False)
expect(isolated["later-ready"], "await_scheduled_runner", False)

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
    run("signer-gap", "relay", "skipped", provider_role="relay_primary"),
]
_, signer_plan = invoke(signer_jobs, signer_runs)
signer = validate(signer_plan, signer_jobs, signer_runs)
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
relay = validate(relay_plan, relay_jobs, relay_runs)
expect(relay["relay-gap"], "retry_relay", True, "relay_secondary")

# Existing queued or in-progress work is owned by that active workflow run. A
# later schedule must wait instead of dispatching or rerunning the same stage.
active_jobs = [
    job("active-runner", "2026-09-01T14:20:00Z"),
    job("active-signer", "2026-09-01T14:30:00Z"),
    job("active-relay", "2026-09-01T14:40:00Z"),
]
active_runs = [
    run("active-runner", "runner", None, run_status="queued", workflow_run_id=4401),
    run("active-signer", "runner", "success", artifact_hash=candidate_hash, workflow_run_id=4402),
    run(
        "active-signer",
        "signer_one",
        None,
        run_status="in_progress",
        provider_role="signer_one_primary",
        workflow_run_id=4403,
    ),
    run("active-relay", "runner", "success", artifact_hash=candidate_hash, workflow_run_id=4404),
    run("active-relay", "signer_one", "success", signer=ADDRESS_ONE, workflow_run_id=4405),
    run("active-relay", "signer_two", "success", signer=ADDRESS_TWO, workflow_run_id=4406),
    run(
        "active-relay",
        "relay",
        None,
        run_status="in_progress",
        provider_role="relay_primary",
        workflow_run_id=4407,
    ),
]
_, active_plan = invoke(active_jobs, active_runs)
active = validate(active_plan, active_jobs, active_runs)
for active_job in active_jobs:
    expect(active[active_job["job_id"]], "await_active_run", False)

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
unsafe = validate(unsafe_plan, unsafe_jobs, unsafe_runs)
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
terminal = validate(terminal_plan, terminal_jobs, [])
expect(terminal["settled"], "observe_terminal", False)
expect(terminal["expired"], "expire_submission", False)

# Exercise a larger deterministic matrix with non-semantic job IDs so a solver
# must implement the state machine rather than special-case the named examples.
rng = random.Random(918273)
matrix_jobs: list[dict[str, Any]] = []
matrix_runs: list[dict[str, Any]] = []
matrix_expected: dict[str, tuple[str, bool, str | None]] = {}
cases = (
    "await_schedule",
    "runner_retry",
    "signer_one_retry",
    "signer_two_retry",
    "relay_retry",
    "stale",
    "drift",
    "unknown",
    "nonretryable",
    "exhausted",
    "unavailable",
    "too_late",
    "expired",
    "terminal",
    "runner_active",
    "signer_active",
    "relay_active",
)
for index in range(102):
    case = cases[index % len(cases)]
    opaque = hashlib.sha256(f"{rng.getrandbits(128):032x}:{index}".encode()).hexdigest()[:18]
    job_id = f"matrix-{opaque}"
    expiry_minute = 20 + index
    expiry_hour, expiry_minute = divmod(expiry_minute, 60)
    expiry = f"2026-09-01T{12 + expiry_hour:02d}:{expiry_minute:02d}:00Z"
    item = job(job_id, expiry)
    expected: tuple[str, bool, str | None]
    if case == "await_schedule":
        expected = ("await_scheduled_runner", False, None)
    elif case == "runner_retry":
        matrix_runs.append(run(job_id, "runner", "failure"))
        expected = ("retry_runner", True, None)
    elif case == "signer_one_retry":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "failure", signer=ADDRESS_ONE),
                run(job_id, "signer_two", "success", signer=ADDRESS_TWO),
                run(job_id, "relay", "skipped", provider_role="relay_primary"),
            ]
        )
        expected = ("retry_signer_one", True, "signer_one_secondary")
    elif case == "signer_two_retry":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "success", signer=ADDRESS_ONE),
                run(job_id, "signer_two", "failure", signer=ADDRESS_TWO),
                run(job_id, "relay", "skipped", provider_role="relay_primary"),
            ]
        )
        expected = ("retry_signer_two", True, "signer_two_secondary")
    elif case == "relay_retry":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "success", signer=ADDRESS_ONE),
                run(job_id, "signer_two", "success", signer=ADDRESS_TWO),
                run(job_id, "relay", "failure", provider_role="relay_primary"),
            ]
        )
        expected = ("retry_relay", True, "relay_secondary")
    elif case == "stale":
        matrix_runs.append(run(job_id, "runner", "success", head_sha="b" * 40))
        expected = ("escalate_no_verdict", False, None)
    elif case == "drift":
        matrix_runs.append(
            run(job_id, "runner", "success", canonical_job_hash=hash32(job_id + ":drift"))
        )
        expected = ("escalate_no_verdict", False, None)
    elif case == "unknown":
        matrix_runs.append(run(job_id, "runner", "failure", workflow="unknown-release.yml"))
        expected = ("escalate_no_verdict", False, None)
    elif case == "nonretryable":
        matrix_runs.append(run(job_id, "runner", "failure", retryable=False))
        expected = ("escalate_no_verdict", False, None)
    elif case == "exhausted":
        matrix_runs.append(run(job_id, "runner", "failure", attempt=2))
        expected = ("escalate_no_verdict", False, None)
    elif case == "unavailable":
        item["input_readiness"] = "unavailable"
        expected = ("escalate_no_verdict", False, None)
    elif case == "too_late":
        item["verification_expires_at"] = "2026-09-01T12:14:59Z"
        expected = ("escalate_no_verdict", False, None)
    elif case == "expired":
        item["verification_expires_at"] = "2026-09-01T11:59:59Z"
        expected = ("expire_submission", False, None)
    elif case == "terminal":
        item["status"] = "settled"
        item["canonical_terminal_event"] = "BountySettled"
        expected = ("observe_terminal", False, None)
    elif case == "runner_active":
        matrix_runs.append(run(job_id, "runner", None, run_status="queued"))
        expected = ("await_active_run", False, None)
    elif case == "signer_active":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", None, run_status="in_progress"),
            ]
        )
        expected = ("await_active_run", False, None)
    else:
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "success", signer=ADDRESS_ONE),
                run(job_id, "signer_two", "success", signer=ADDRESS_TWO),
                run(job_id, "relay", None, run_status="in_progress"),
            ]
        )
        expected = ("await_active_run", False, None)
    matrix_jobs.append(item)
    matrix_expected[job_id] = expected

_, matrix_plan = invoke(matrix_jobs, matrix_runs)
matrix = validate(matrix_plan, matrix_jobs, matrix_runs)
for job_id, (action, automated, provider) in matrix_expected.items():
    expect(matrix[job_id], action, automated, provider)

# One GitHub workflow run can cover several canonical jobs. Retrying that run
# more than once would race its shared run_attempt and duplicate a write. The
# earliest-deadline job owns the one automated retry; peers wait for its result.
shared_jobs = [
    job("shared-first", "2026-09-01T13:40:00Z"),
    job("shared-second", "2026-09-01T13:50:00Z"),
]
shared_runs = [
    run("shared-first", "runner", "failure", workflow_run_id=4290, workflow_job_id=7290),
    run("shared-second", "runner", "failure", workflow_run_id=4290, workflow_job_id=7290),
]
_, shared_plan = invoke(shared_jobs, shared_runs)
shared = validate(shared_plan, shared_jobs, shared_runs)
expect(shared["shared-first"], "retry_runner", True, "runner")
expect(shared["shared-second"], "await_shared_retry", False, "runner")


class FakeGitHubHandler(http.server.BaseHTTPRequestHandler):
    requests: list[dict[str, Any]] = []
    retry_runs: dict[int, dict[str, Any]] = {}
    retry_jobs: dict[int, dict[str, Any]] = {}
    retry_attempt_jobs: dict[int, list[dict[str, Any]]] = {}
    unsafe_run_metadata = False
    stale_branch_metadata = False
    scheduled_jobs: object = []
    scheduled_runs: dict[str, Any] = {}
    scheduled_run_jobs: dict[int, dict[str, Any]] = {}
    scheduled_artifacts: dict[int, dict[str, Any]] = {}
    scheduled_archives: dict[int, bytes] = {}
    fail_post_path: str | None = None
    accept_then_drop_path: str | None = None

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def record(self, body: bytes = b"") -> None:
        self.__class__.requests.append(
            {
                "method": self.command,
                "path": self.path,
                "authorization": self.headers.get("Authorization"),
                "body": body.decode("utf-8", "replace"),
            }
        )

    def respond(self, status: int, payload: object | None = None) -> None:
        body = (
            b""
            if payload is None
            else payload
            if isinstance(payload, bytes)
            else json.dumps(payload).encode("utf-8")
        )
        self.send_response(status)
        if body:
            self.send_header(
                "Content-Type",
                "application/zip" if isinstance(payload, bytes) else "application/json",
            )
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - standard library handler name
        self.record()
        if self.path == "/v1/base/autonomous-bounties/verification-jobs?network=base-mainnet":
            self.respond(200, self.scheduled_jobs)
            return
        if self.path == "/repos/NSPG13/agent-bounties/actions/runs?per_page=100":
            self.respond(200, self.scheduled_runs)
            return
        jobs_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/runs/(\d+)/jobs\?per_page=100",
            self.path,
        )
        if jobs_match:
            payload = self.scheduled_run_jobs.get(int(jobs_match.group(1)))
            self.respond(200 if payload is not None else 404, payload or {"message": "not found"})
            return
        retry_attempts_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/runs/(\d+)/jobs\?filter=all&per_page=100",
            self.path,
        )
        if retry_attempts_match:
            run_id = int(retry_attempts_match.group(1))
            jobs = self.retry_attempt_jobs.get(run_id)
            self.respond(
                200 if jobs is not None else 404,
                {"total_count": len(jobs or []), "jobs": jobs or []},
            )
            return
        artifacts_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/runs/(\d+)/artifacts\?per_page=100",
            self.path,
        )
        if artifacts_match:
            payload = self.scheduled_artifacts.get(int(artifacts_match.group(1)))
            self.respond(200 if payload is not None else 404, payload or {"message": "not found"})
            return
        archive_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/artifacts/(\d+)/zip",
            self.path,
        )
        if archive_match:
            payload = self.scheduled_archives.get(int(archive_match.group(1)))
            self.respond(200 if payload is not None else 404, payload or {"message": "not found"})
            return
        if self.path == "/repos/NSPG13/agent-bounties/branches/main":
            branch_sha = "b" * 40 if self.stale_branch_metadata else MAIN_SHA
            self.respond(200, {"name": "main", "commit": {"sha": branch_sha}})
            return
        run_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/runs/(\d+)",
            self.path,
        )
        if run_match:
            payload = self.retry_runs.get(int(run_match.group(1)))
            if payload is None:
                self.respond(404, {"message": "not found"})
                return
            payload = dict(payload)
            if self.unsafe_run_metadata:
                payload["head_sha"] = "b" * 40
            self.respond(200, payload)
            return
        job_match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/jobs/(\d+)",
            self.path,
        )
        if job_match:
            payload = self.retry_jobs.get(int(job_match.group(1)))
            self.respond(200 if payload is not None else 404, payload or {"message": "not found"})
            return
        self.respond(404, {"message": "not found"})

    def do_POST(self) -> None:  # noqa: N802 - standard library handler name
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        self.record(body)
        if self.path == self.fail_post_path:
            self.respond(500, {"message": "injected later write failure"})
            return
        match = re.fullmatch(
            r"/repos/NSPG13/agent-bounties/actions/jobs/(\d+)/rerun",
            self.path,
        )
        workflow_job_id = int(match.group(1)) if match else 0
        job = self.retry_jobs.get(workflow_job_id)
        if job is None:
            self.respond(404, {"message": "not found"})
            return
        run_id = int(job["run_id"])
        self.retry_runs[run_id]["run_attempt"] += 1
        run_attempt = self.retry_runs[run_id]["run_attempt"]
        self.retry_attempt_jobs.setdefault(run_id, []).append(
            {
                **job,
                "id": workflow_job_id + run_attempt * 100_000,
                "run_attempt": run_attempt,
                "status": "completed",
                "conclusion": "success",
            }
        )
        if self.path == self.accept_then_drop_path:
            self.close_connection = True
            try:
                self.connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            self.connection.close()
            return
        self.respond(201, None)


def execute(
    plan: dict[str, Any],
    api_base: str,
    state_path: Path,
) -> subprocess.CompletedProcess[bytes]:
    tool = require("scripts/regression_verifier_watchdog.py")
    with tempfile.TemporaryDirectory(prefix="watchdog-execute-") as temporary:
        plan_path = Path(temporary) / "plan.json"
        plan_path.write_text(json.dumps(plan), encoding="utf-8")
        environment = os.environ.copy()
        environment["WATCHDOG_BENCHMARK_TOKEN"] = "benchmark-token"
        environment["WATCHDOG_BENCHMARK_LOOPBACK"] = "1"
        return subprocess.run(
            [
                sys.executable,
                str(tool),
                "execute",
                "--plan",
                str(plan_path),
                "--repository",
                "NSPG13/agent-bounties",
                "--github-api-base",
                api_base,
                "--token-env",
                "WATCHDOG_BENCHMARK_TOKEN",
                "--state",
                str(state_path),
                "--execute",
                "--allow-workflow",
                "regression-verifier-runner.yml",
                "--allow-workflow",
                "regression-verifier-signer.yml",
            ],
            cwd=ROOT,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=30,
            check=False,
        )


def plan_live(
    api_base: str,
    output_path: Path,
    policy_path: Path,
) -> subprocess.CompletedProcess[bytes]:
    tool = require("scripts/regression_verifier_watchdog.py")
    environment = os.environ.copy()
    environment["WATCHDOG_BENCHMARK_TOKEN"] = "benchmark-token"
    environment["WATCHDOG_BENCHMARK_LOOPBACK"] = "1"
    environment["WATCHDOG_BENCHMARK_NOW"] = NOW
    return subprocess.run(
        [
            sys.executable,
            str(tool),
            "plan-live",
            "--api-base",
            api_base,
            "--repository",
            "NSPG13/agent-bounties",
            "--github-api-base",
            api_base,
            "--token-env",
            "WATCHDOG_BENCHMARK_TOKEN",
            "--policy",
            str(policy_path),
            "--output",
            str(output_path),
            "--allow-workflow",
            "regression-verifier-runner.yml",
            "--allow-workflow",
            "regression-verifier-signer.yml",
        ],
        cwd=ROOT,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=30,
        check=False,
    )


# Exercise the production executor against a local fake GitHub boundary. It may
# rerun only one exact failed job from a reviewed current-main run; changing the
# target workflow must fail before any write request.
runner_execute_jobs = [job("execute-runner", "2026-09-01T14:00:00Z")]
runner_execute_runs = [
    run(
        "execute-runner",
        "runner",
        "failure",
        workflow_run_id=4300,
        workflow_job_id=7300,
    )
]
_, runner_execute_plan = invoke(runner_execute_jobs, runner_execute_runs)
relay_execute_jobs = [job("execute-relay", "2026-09-01T14:10:00Z")]
relay_execute_runs = [
    run("execute-relay", "runner", "success", artifact_hash=candidate_hash, workflow_run_id=4301, workflow_job_id=7301),
    run("execute-relay", "signer_one", "success", signer=ADDRESS_ONE, workflow_run_id=4302, workflow_job_id=7302),
    run("execute-relay", "signer_two", "success", signer=ADDRESS_TWO, workflow_run_id=4303, workflow_job_id=7303),
    run(
        "execute-relay",
        "relay",
        "failure",
        provider_role="relay_primary",
        workflow_run_id=4304,
        workflow_job_id=7304,
    ),
]
_, relay_execute_plan = invoke(relay_execute_jobs, relay_execute_runs)
signer_execute_jobs = [job("execute-signer", "2026-09-01T14:20:00Z")]
signer_execute_runs = [
    run(
        "execute-signer",
        "runner",
        "success",
        artifact_hash=candidate_hash,
        workflow_run_id=4306,
        workflow_job_id=7308,
    ),
    run(
        "execute-signer",
        "signer_one",
        "success",
        signer=ADDRESS_ONE,
        workflow_run_id=4305,
        workflow_job_id=7305,
    ),
    run(
        "execute-signer",
        "signer_two",
        "failure",
        signer=ADDRESS_TWO,
        workflow_run_id=4305,
        workflow_job_id=7306,
    ),
    run(
        "execute-signer",
        "relay",
        "skipped",
        workflow_run_id=4305,
        workflow_job_id=7307,
    ),
]
_, signer_execute_plan = invoke(signer_execute_jobs, signer_execute_runs)
execution_plan = {
    "schema": SCHEMA,
    "network": "base-mainnet",
    "generated_at": NOW,
    "fail_closed": True,
    "repository": "NSPG13/agent-bounties",
    "current_main_sha": MAIN_SHA,
    "jobs": runner_execute_plan["jobs"] + relay_execute_plan["jobs"],
}
FakeGitHubHandler.requests = []
FakeGitHubHandler.retry_runs = {
    4300: {
        "id": 4300,
        "path": ".github/workflows/regression-verifier-runner.yml",
        "head_sha": MAIN_SHA,
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
    4304: {
        "id": 4304,
        "path": ".github/workflows/regression-verifier-signer.yml",
        "head_sha": MAIN_SHA,
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
    4305: {
        "id": 4305,
        "path": ".github/workflows/regression-verifier-signer.yml",
        "head_sha": MAIN_SHA,
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
}
FakeGitHubHandler.retry_jobs = {
    7300: {
        "id": 7300,
        "run_id": 4300,
        "name": "run-no-secrets",
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
    7304: {
        "id": 7304,
        "run_id": 4304,
        "name": "relay",
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
    7305: {
        "id": 7305,
        "run_id": 4305,
        "name": "sign-one / sign",
        "status": "completed",
        "conclusion": "success",
        "run_attempt": 1,
    },
    7306: {
        "id": 7306,
        "run_id": 4305,
        "name": "sign-two / sign",
        "status": "completed",
        "conclusion": "failure",
        "run_attempt": 1,
    },
    7307: {
        "id": 7307,
        "run_id": 4305,
        "name": "relay",
        "status": "completed",
        "conclusion": "skipped",
        "run_attempt": 1,
    },
}
FakeGitHubHandler.retry_attempt_jobs = {
    run_id: [dict(job) for job in FakeGitHubHandler.retry_jobs.values() if job["run_id"] == run_id]
    for run_id in FakeGitHubHandler.retry_runs
}
live_production_jobs = [
    production_job("live-signer-gap", "2026-09-01T14:30:00Z"),
    production_job("new-after-run", "2026-09-01T14:45:00Z"),
]
live_production_jobs[1]["submission_evidence"]["created_at"] = int(
    datetime.fromisoformat("2026-09-01T12:30:00+00:00").timestamp()
)
live_jobs = [normalize_production_job(item) for item in live_production_jobs]
live_job = live_jobs[0]


def live_artifact(stage: str, run_id: int) -> str:
    payload = f"{stage}:{live_job['canonical_job_hash']}:{run_id}"
    return "sha256:" + hashlib.sha256(payload.encode()).hexdigest()


live_runs = [
    run(
        live_job["job_id"],
        "runner",
        "success",
        artifact_hash=live_artifact("runner", 5301),
        workflow_run_id=5301,
        workflow_job_id=7301,
        canonical_job_hash=live_job["canonical_job_hash"],
    ),
    run(
        live_job["job_id"],
        "signer_one",
        "success",
        provider_role="signer_one_primary",
        artifact_hash=live_artifact("signer_one", 5302),
        signer=ADDRESS_ONE,
        workflow_run_id=5302,
        workflow_job_id=7302,
        canonical_job_hash=live_job["canonical_job_hash"],
    ),
    run(
        live_job["job_id"],
        "signer_two",
        "failure",
        provider_role="signer_two_primary",
        workflow_run_id=5302,
        workflow_job_id=7303,
        canonical_job_hash=live_job["canonical_job_hash"],
    ),
    run(
        live_job["job_id"],
        "relay",
        "skipped",
        provider_role="relay_primary",
        workflow_run_id=5302,
        workflow_job_id=7304,
        canonical_job_hash=live_job["canonical_job_hash"],
    ),
]
_, live_plan = invoke(live_jobs, live_runs)
live_execution_plan = {
    **live_plan,
    "repository": "NSPG13/agent-bounties",
    "current_main_sha": MAIN_SHA,
}
FakeGitHubHandler.scheduled_jobs = live_production_jobs
FakeGitHubHandler.scheduled_runs = {
    "total_count": 2,
    "workflow_runs": [
        {
            "id": 5301,
            "path": ".github/workflows/regression-verifier-runner.yml",
            "status": "completed",
            "conclusion": "success",
            "run_attempt": 1,
            "head_sha": MAIN_SHA,
            "event": "schedule",
            "created_at": "2026-09-01T12:05:00Z",
        },
        {
            "id": 5302,
            "path": ".github/workflows/regression-verifier-signer.yml",
            "display_title": "Regression Verifier Signer / candidate run 5301",
            "status": "completed",
            "conclusion": "failure",
            "run_attempt": 1,
            "head_sha": MAIN_SHA,
            "event": "workflow_run",
            "created_at": "2026-09-01T12:20:00Z",
        },
    ],
}
FakeGitHubHandler.scheduled_run_jobs = {
    5301: {
        "total_count": 1,
        "jobs": [{"id": 7301, "name": "run-no-secrets", "status": "completed", "conclusion": "success", "run_attempt": 1}],
    },
    5302: {
        "total_count": 3,
        "jobs": [
            {"id": 7302, "name": "sign-one / sign", "status": "completed", "conclusion": "success", "run_attempt": 1},
            {"id": 7303, "name": "sign-two / sign", "status": "completed", "conclusion": "failure", "run_attempt": 1},
            {"id": 7304, "name": "relay", "status": "completed", "conclusion": "skipped", "run_attempt": 1},
        ],
    },
}
live_candidate_archive = candidate_archive([live_production_jobs[0]])
FakeGitHubHandler.scheduled_artifacts = {
    5301: {
        "total_count": 1,
        "artifacts": [
            {
                "id": 6301,
                "name": "regression-candidates-5301",
                "expired": False,
                "size_in_bytes": len(live_candidate_archive),
            }
        ],
    }
}
FakeGitHubHandler.scheduled_archives = {6301: live_candidate_archive}
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeGitHubHandler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
execution_state = tempfile.TemporaryDirectory(prefix="watchdog-state-")
try:
    api_base = f"http://127.0.0.1:{server.server_port}"
    state_root = Path(execution_state.name)
    live_plan_path = state_root / "watchdog-plan.json"
    live_policy_path = require("ops/regression-verifier-watchdog-policy.json")
    checked_policy = json.loads(live_policy_path.read_text(encoding="utf-8"))
    expected_checked_policy = {
        key: value for key, value in POLICY.items() if key != "current_main_sha"
    }
    if checked_policy != expected_checked_policy:
        raise SystemExit("checked-in watchdog policy does not match the precommitted bounds")
    completed = plan_live(api_base, live_plan_path, live_policy_path)
    if completed.returncode != 0 or not live_plan_path.is_file():
        raise SystemExit(
            "watchdog live planning fixture failed:\n"
            + completed.stdout.decode("utf-8", "replace")[-5000:]
        )
    generated_execution_plan = json.loads(live_plan_path.read_text(encoding="utf-8"))
    if generated_execution_plan != live_execution_plan:
        raise SystemExit("watchdog live planner did not produce the exact executable plan")
    live_by_id = {item["job_id"]: item for item in generated_execution_plan["jobs"]}
    if (
        live_by_id[live_jobs[0]["job_id"]]["next_action"] != "retry_signer_two"
        or live_by_id[live_jobs[1]["job_id"]]["next_action"] != "await_scheduled_runner"
        or live_by_id[live_jobs[1]["job_id"]]["workflow_run_id"] is not None
    ):
        raise SystemExit("an older candidate artifact was attached to a newly submitted job")
    requested_paths = {item["path"] for item in FakeGitHubHandler.requests}
    required_live_paths = {
        "/v1/base/autonomous-bounties/verification-jobs?network=base-mainnet",
        "/repos/NSPG13/agent-bounties/actions/runs?per_page=100",
        "/repos/NSPG13/agent-bounties/branches/main",
        "/repos/NSPG13/agent-bounties/actions/runs/5301/artifacts?per_page=100",
        "/repos/NSPG13/agent-bounties/actions/artifacts/6301/zip",
        "/repos/NSPG13/agent-bounties/actions/runs/5301/jobs?per_page=100",
        "/repos/NSPG13/agent-bounties/actions/runs/5302/jobs?per_page=100",
    }
    if not required_live_paths.issubset(requested_paths):
        raise SystemExit("watchdog live planner did not acquire every required production input")
    github_requests = [
        item for item in FakeGitHubHandler.requests if item["path"].startswith("/repos/")
    ]
    if any(item["authorization"] != "Bearer benchmark-token" for item in github_requests):
        raise SystemExit("watchdog live planner did not scope its token to GitHub requests")
    platform_requests = [
        item for item in FakeGitHubHandler.requests if item["path"].startswith("/v1/")
    ]
    if any(item["authorization"] is not None for item in platform_requests):
        raise SystemExit("watchdog live planner leaked the GitHub token to the platform API")

    # GitHub can expose the completed runner and its artifact before the
    # workflow_run-triggered signer is visible. This gap must be a bounded,
    # non-writing wait, never a retry lookup for a signer run that does not yet
    # exist.
    saved_scheduled_jobs = FakeGitHubHandler.scheduled_jobs
    saved_scheduled_runs = FakeGitHubHandler.scheduled_runs
    saved_scheduled_run_jobs = FakeGitHubHandler.scheduled_run_jobs
    saved_scheduled_artifacts = FakeGitHubHandler.scheduled_artifacts
    saved_scheduled_archives = FakeGitHubHandler.scheduled_archives
    FakeGitHubHandler.requests = []
    FakeGitHubHandler.scheduled_jobs = [live_production_jobs[0]]
    FakeGitHubHandler.scheduled_runs = {
        "total_count": 1,
        "workflow_runs": [saved_scheduled_runs["workflow_runs"][0]],
    }
    FakeGitHubHandler.scheduled_run_jobs = {5301: saved_scheduled_run_jobs[5301]}
    FakeGitHubHandler.scheduled_artifacts = {5301: saved_scheduled_artifacts[5301]}
    FakeGitHubHandler.scheduled_archives = {6301: saved_scheduled_archives[6301]}
    runner_only_path = state_root / "runner-only-plan.json"
    completed = plan_live(api_base, runner_only_path, live_policy_path)
    if completed.returncode != 0 or not runner_only_path.is_file():
        raise SystemExit(
            "watchdog failed during the runner-to-signer visibility gap:\n"
            + completed.stdout.decode("utf-8", "replace")[-5000:]
        )
    runner_only_plan = json.loads(runner_only_path.read_text(encoding="utf-8"))
    runner_only_record = runner_only_plan["jobs"][0]
    if (
        runner_only_record["next_action"] != "await_active_run"
        or runner_only_record["automation_allowed"] is not False
        or runner_only_record["provider_role"] != "signer_one"
        or runner_only_record["target_workflow"] is not None
        or runner_only_record["workflow_run_id"] is not None
        or runner_only_record["workflow_job_id"] is not None
        or runner_only_record["workflow_run_attempt"] is not None
    ):
        raise SystemExit("runner-to-signer visibility gap did not fail closed as a bounded wait")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("runner-to-signer visibility gap attempted a write")
    FakeGitHubHandler.scheduled_jobs = saved_scheduled_jobs
    FakeGitHubHandler.scheduled_runs = saved_scheduled_runs
    FakeGitHubHandler.scheduled_run_jobs = saved_scheduled_run_jobs
    FakeGitHubHandler.scheduled_artifacts = saved_scheduled_artifacts
    FakeGitHubHandler.scheduled_archives = saved_scheduled_archives
    FakeGitHubHandler.requests = []
    successful_state = state_root / "successful.json"
    completed = execute(execution_plan, api_base, successful_state)
    if completed.returncode != 0:
        raise SystemExit(
            "watchdog execute fixture failed:\n" + completed.stdout.decode("utf-8", "replace")[-5000:]
        )
    try:
        report = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit("watchdog execute did not emit one JSON report") from error
    if report.get("schema") != "agent-bounties/regression-verifier-watchdog-execution-v1":
        raise SystemExit("watchdog execute report schema is invalid")
    if report.get("executed_count") != 2 or report.get("fail_closed") is not True:
        raise SystemExit("watchdog execute report does not reconcile both bounded actions")
    writes = [item for item in FakeGitHubHandler.requests if item["method"] == "POST"]
    expected_writes = {
        "/repos/NSPG13/agent-bounties/actions/jobs/7300/rerun",
        "/repos/NSPG13/agent-bounties/actions/jobs/7304/rerun",
    }
    if {item["path"] for item in writes} != expected_writes or len(writes) != 2:
        raise SystemExit("watchdog execute wrote outside the exact allowlisted GitHub actions")
    if any(item["authorization"] != "Bearer benchmark-token" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute did not use the environment-scoped token")
    request_count = len(FakeGitHubHandler.requests)
    replayed = execute(execution_plan, api_base, successful_state)
    if replayed.returncode != 0:
        raise SystemExit("watchdog execute could not reconcile an unchanged replay")
    replay_report = json.loads(replayed.stdout)
    if replay_report.get("executed_count") != 0 or replay_report.get("skipped_count") != 2:
        raise SystemExit("watchdog replay did not report both idempotent actions as skipped")
    if len(FakeGitHubHandler.requests) != request_count:
        raise SystemExit("watchdog replay contacted GitHub after both actions were recorded")

    # A signer retry is allowed to invoke GitHub's exact-job endpoint only when
    # the plan also binds the relay job that GitHub will rerun as a dependent.
    signer_record = signer_execute_plan["jobs"][0]
    if signer_record.get("affected_workflow_jobs") != [
        {"workflow_job_id": 7306, "name": "sign-two / sign", "effect": "target"},
        {"workflow_job_id": 7307, "name": "relay", "effect": "dependent"},
    ]:
        raise SystemExit("signer retry plan does not disclose its dependent relay execution")
    FakeGitHubHandler.requests = []
    signer_state = state_root / "signer-dependent.json"
    signer_execution = execute(
        {
            **signer_execute_plan,
            "repository": "NSPG13/agent-bounties",
            "current_main_sha": MAIN_SHA,
        },
        api_base,
        signer_state,
    )
    if signer_execution.returncode != 0:
        raise SystemExit(
            "watchdog rejected the fully modeled signer retry:\n"
            + signer_execution.stdout.decode("utf-8", "replace")[-5000:]
        )
    signer_writes = [item for item in FakeGitHubHandler.requests if item["method"] == "POST"]
    if [item["path"] for item in signer_writes] != [
        "/repos/NSPG13/agent-bounties/actions/jobs/7306/rerun"
    ]:
        raise SystemExit("signer retry wrote outside its exact target-job endpoint")

    FakeGitHubHandler.requests = []
    FakeGitHubHandler.retry_runs[4300]["run_attempt"] = 1
    FakeGitHubHandler.retry_runs[4304]["run_attempt"] = 1
    partial_state = state_root / "partial.json"
    FakeGitHubHandler.fail_post_path = "/repos/NSPG13/agent-bounties/actions/jobs/7304/rerun"
    partial = execute(execution_plan, api_base, partial_state)
    if partial.returncode == 0:
        raise SystemExit("watchdog partial-write fixture did not inject the later failure")
    partial_writes = [item for item in FakeGitHubHandler.requests if item["method"] == "POST"]
    if [item["path"] for item in partial_writes] != [
        "/repos/NSPG13/agent-bounties/actions/jobs/7300/rerun",
        "/repos/NSPG13/agent-bounties/actions/jobs/7304/rerun",
    ]:
        raise SystemExit("watchdog partial-write fixture did not fail after the first action")
    partial_document = json.loads(partial_state.read_text(encoding="utf-8"))
    if partial_document != {
        "schema": "agent-bounties/regression-verifier-watchdog-state-v1",
        "executed_idempotency_keys": [execution_plan["jobs"][0]["idempotency_key"]],
    }:
        raise SystemExit("watchdog did not durably record the first action before later failure")
    FakeGitHubHandler.requests = []
    FakeGitHubHandler.fail_post_path = None
    resumed = execute(execution_plan, api_base, partial_state)
    if resumed.returncode != 0:
        raise SystemExit("watchdog could not resume after a later action failed")
    resumed_writes = [item for item in FakeGitHubHandler.requests if item["method"] == "POST"]
    if [item["path"] for item in resumed_writes] != [
        "/repos/NSPG13/agent-bounties/actions/jobs/7304/rerun"
    ]:
        raise SystemExit("watchdog repeated an earlier successful action after partial failure")

    # The POST can be accepted by GitHub even when the client connection dies
    # before a response or local state write. A later run-attempt proves the
    # remote write happened and must reconcile without replaying it.
    FakeGitHubHandler.requests = []
    FakeGitHubHandler.retry_runs[4300]["run_attempt"] = 1
    crash_state = state_root / "accepted-then-dropped.json"
    crash_plan = {**execution_plan, "jobs": [execution_plan["jobs"][0]]}
    FakeGitHubHandler.accept_then_drop_path = (
        "/repos/NSPG13/agent-bounties/actions/jobs/7300/rerun"
    )
    crashed = execute(crash_plan, api_base, crash_state)
    if crashed.returncode == 0 or crash_state.exists():
        raise SystemExit("watchdog accepted-then-dropped fixture did not lose the local receipt")
    if FakeGitHubHandler.retry_runs[4300]["run_attempt"] != 2:
        raise SystemExit("watchdog crash fixture was not accepted by the remote boundary")
    FakeGitHubHandler.requests = []
    FakeGitHubHandler.accept_then_drop_path = None
    reconciled = execute(crash_plan, api_base, crash_state)
    if reconciled.returncode != 0:
        raise SystemExit("watchdog could not reconcile an accepted write after client failure")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog replayed a remotely accepted job retry")
    reconciled_report = json.loads(reconciled.stdout)
    if reconciled_report.get("executed_count") != 0 or reconciled_report.get("skipped_count") != 1:
        raise SystemExit("watchdog crash reconciliation report is inaccurate")

    # A run-wide attempt increase caused by a different job is not a receipt
    # for the planned target. The stale plan must fail closed without replaying
    # or recording the target idempotency key.
    FakeGitHubHandler.requests = []
    FakeGitHubHandler.retry_runs[4300]["run_attempt"] = 2
    FakeGitHubHandler.retry_attempt_jobs[4300] = [
        {**FakeGitHubHandler.retry_jobs[7300], "run_attempt": 1},
        {
            "id": 999_002,
            "run_id": 4300,
            "name": "unrelated-maintenance-job",
            "status": "completed",
            "conclusion": "success",
            "run_attempt": 2,
        },
    ]
    unrelated_state = state_root / "unrelated-attempt.json"
    unrelated = execute(crash_plan, api_base, unrelated_state)
    if unrelated.returncode == 0 or unrelated_state.exists():
        raise SystemExit("watchdog treated an unrelated job retry as the target crash receipt")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog replayed after an unrelated run-attempt increase")
    FakeGitHubHandler.retry_runs[4300]["run_attempt"] = 1
    FakeGitHubHandler.retry_attempt_jobs[4300] = [
        {**FakeGitHubHandler.retry_jobs[7300], "run_attempt": 1}
    ]

    unsafe_plan = json.loads(json.dumps(execution_plan))
    unsafe_plan["jobs"][0]["target_workflow"] = "unreviewed-wallet-job.yml"
    request_count = len(FakeGitHubHandler.requests)
    rejected = execute(unsafe_plan, api_base, state_root / "unsafe.json")
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted a non-allowlisted workflow")
    if len(FakeGitHubHandler.requests) != request_count:
        raise SystemExit("watchdog execute contacted GitHub before rejecting an unknown workflow")

    FakeGitHubHandler.requests = []
    FakeGitHubHandler.retry_runs[4300]["run_attempt"] = 1
    FakeGitHubHandler.retry_runs[4304]["run_attempt"] = 1
    FakeGitHubHandler.unsafe_run_metadata = True
    rejected = execute(execution_plan, api_base, state_root / "stale.json")
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted stale metadata in a later action")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute wrote an earlier action before all metadata passed")

    FakeGitHubHandler.requests = []
    FakeGitHubHandler.unsafe_run_metadata = False
    FakeGitHubHandler.stale_branch_metadata = True
    rejected = execute(execution_plan, api_base, state_root / "stale-main.json")
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted a plan after protected main advanced")
    if not any(item["path"].endswith("/branches/main") for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute did not fetch protected main before execution")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute wrote from a stale protected-main plan")

    FakeGitHubHandler.requests = []
    FakeGitHubHandler.stale_branch_metadata = False
    rejected = execute(
        execution_plan,
        f"http://localhost:{server.server_port}",
        state_root / "attacker-origin.json",
    )
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted a non-pinned GitHub API origin")
    if FakeGitHubHandler.requests:
        raise SystemExit("watchdog execute sent its token to a non-pinned API origin")
finally:
    FakeGitHubHandler.unsafe_run_metadata = False
    FakeGitHubHandler.stale_branch_metadata = False
    FakeGitHubHandler.fail_post_path = None
    FakeGitHubHandler.accept_then_drop_path = None
    execution_state.cleanup()
    server.shutdown()
    server.server_close()
    thread.join(timeout=5)

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

# The workflows execute this Python module while signing and relaying. Binding
# only their command lines would leave that executable mutable, so both the
# reviewed runtime and its deterministic security tests are byte-for-byte
# precommitted by the immutable checker.
pipeline = require("scripts/regression_verifier_pipeline.py")
pipeline_tests = require("scripts/test_regression_verifier_pipeline.py")
source_guard_tests = require("scripts/test_regression_verifier_source_guard.py")
pipeline_bytes = pipeline.read_bytes().replace(b"\r\n", b"\n")
pipeline_test_bytes = pipeline_tests.read_bytes().replace(b"\r\n", b"\n")
if b"\r" in pipeline_bytes or hashlib.sha256(pipeline_bytes).hexdigest() != PIPELINE_SHA256:
    raise SystemExit("regression verifier pipeline differs from the reviewed executable")
if b"\r" in pipeline_test_bytes or hashlib.sha256(pipeline_test_bytes).hexdigest() != PIPELINE_TEST_SHA256:
    raise SystemExit("regression verifier pipeline tests differ from the reviewed suite")
completed = subprocess.run(
    [sys.executable, str(source_guard_tests), "-v"],
    cwd=ROOT,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=120,
    check=False,
)
if completed.returncode != 0:
    raise SystemExit("source guard tests failed:\n" + completed.stdout.decode()[-5000:])

workflow = require(".github/workflows/regression-verifier-watchdog.yml").read_text(encoding="utf-8")
try:
    workflow_document = json.loads(workflow)
except json.JSONDecodeError as error:
    raise SystemExit("watchdog workflow must use strict JSON-syntax YAML") from error
if set(workflow_document) != {"name", "on", "permissions", "concurrency", "jobs"}:
    raise SystemExit("watchdog workflow has unknown effective top-level keys")
if workflow_document["name"] != "Regression Verifier Watchdog":
    raise SystemExit("watchdog workflow name is not exact")
triggers = workflow_document["on"]
if not isinstance(triggers, dict) or set(triggers) != {"schedule"}:
    raise SystemExit("watchdog workflow must be schedule-only on the protected default branch")
schedule = triggers["schedule"]
if schedule != [{"cron": "3,8,13,18,23,28,33,38,43,48,53,58 * * * *"}]:
    raise SystemExit("watchdog workflow has no single effective cron schedule")
if workflow_document["permissions"] != {"contents": "read", "actions": "write"}:
    raise SystemExit("watchdog workflow permissions must be exactly contents: read and actions: write")
if workflow_document["concurrency"] != {
    "group": "regression-verifier-watchdog-mainnet",
    "cancel-in-progress": False,
}:
    raise SystemExit("watchdog workflow must serialize the exact mainnet concurrency group")
jobs = workflow_document["jobs"]
if not isinstance(jobs, dict) or set(jobs) != {"watchdog"}:
    raise SystemExit("watchdog workflow must contain exactly one effective watchdog job")
watchdog_job = jobs["watchdog"]
if set(watchdog_job) != {"runs-on", "steps"} or watchdog_job["runs-on"] != "ubuntu-latest":
    raise SystemExit("watchdog job may not override permissions, defaults, shell, or container")
steps = watchdog_job["steps"]
if not isinstance(steps, list) or len(steps) != 5 or not all(isinstance(step, dict) for step in steps):
    raise SystemExit("watchdog job must contain exactly five bounded steps")
checkout, restore, live_step, execute_step, save = steps
if set(checkout) != {"uses", "with"} or not re.fullmatch(
    r"actions/checkout@[0-9a-f]{40}", str(checkout["uses"])
):
    raise SystemExit("watchdog checkout action must be commit-pinned")
if checkout["with"] != {
    "repository": "${{ github.repository }}",
    "ref": "main",
    "persist-credentials": False,
}:
    raise SystemExit("watchdog checkout must bind protected main without persisted credentials")
cache_key = "${{ runner.os }}-regression-verifier-watchdog-${{ github.run_id }}"
cache_prefix = "${{ runner.os }}-regression-verifier-watchdog-"
if set(restore) != {"uses", "with"} or not re.fullmatch(
    r"actions/cache/restore@[0-9a-f]{40}", str(restore["uses"])
):
    raise SystemExit("watchdog cache restore must be commit-pinned")
if restore["with"] != {
    "path": ".watchdog/state.json",
    "key": cache_key,
    "restore-keys": cache_prefix,
}:
    raise SystemExit("watchdog cache restore does not bind the exact state file")
token_environment = {"GITHUB_TOKEN": "${{ github.token }}"}
if set(live_step) != {"run", "env"} or live_step["env"] != token_environment:
    raise SystemExit("watchdog live planner step is not tightly scoped")
if set(execute_step) != {"run", "env"} or execute_step["env"] != token_environment:
    raise SystemExit("watchdog execute step is not tightly scoped")
expected_live_tokens = [
    "python", "scripts/regression_verifier_watchdog.py", "plan-live",
    "--api-base", "https://api.agentbounties.app",
    "--repository", "$GITHUB_REPOSITORY",
    "--github-api-base", "https://api.github.com",
    "--token-env", "GITHUB_TOKEN",
    "--policy", "ops/regression-verifier-watchdog-policy.json",
    "--output", "target/watchdog-plan.json",
    "--allow-workflow", "regression-verifier-runner.yml",
    "--allow-workflow", "regression-verifier-signer.yml",
]
expected_execute_tokens = [
    "python", "scripts/regression_verifier_watchdog.py", "execute",
    "--plan", "target/watchdog-plan.json",
    "--repository", "$GITHUB_REPOSITORY",
    "--github-api-base", "https://api.github.com",
    "--token-env", "GITHUB_TOKEN",
    "--state", ".watchdog/state.json", "--execute",
    "--allow-workflow", "regression-verifier-runner.yml",
    "--allow-workflow", "regression-verifier-signer.yml",
]
try:
    live_tokens = shlex.split(str(live_step["run"]), posix=True)
    execute_tokens = shlex.split(str(execute_step["run"]), posix=True)
except ValueError as error:
    raise SystemExit("watchdog production commands are not safely parseable") from error
if live_tokens != expected_live_tokens or execute_tokens != expected_execute_tokens:
    raise SystemExit("watchdog production commands must contain only the exact pinned argv")
if set(save) != {"if", "uses", "with"} or save["if"] != "${{ always() }}" or not re.fullmatch(
    r"actions/cache/save@[0-9a-f]{40}", str(save["uses"])
):
    raise SystemExit("watchdog cache save must be pinned and run after partial failure")
if save["with"] != {"path": ".watchdog/state.json", "key": cache_key}:
    raise SystemExit("watchdog cache save does not persist the exact execution state")

runner_workflow = require(".github/workflows/regression-verifier-runner.yml").read_text(
    encoding="utf-8"
)
try:
    runner_document = json.loads(runner_workflow)
except json.JSONDecodeError as error:
    raise SystemExit("candidate runner workflow must use strict JSON-syntax YAML") from error
if set(runner_document) != {"name", "on", "permissions", "concurrency", "jobs"}:
    raise SystemExit("candidate runner workflow has unknown effective top-level keys")
if runner_document["name"] != "Regression Verifier Runner":
    raise SystemExit("candidate runner workflow name is not exact")
if runner_document["on"] != {
    "schedule": [{"cron": "7,22,37,52 * * * *"}],
}:
    raise SystemExit("candidate runner must expose only the fixed protected-branch schedule")
if runner_document["permissions"] != {"contents": "read"}:
    raise SystemExit("candidate runner permissions must be exactly contents: read")
if runner_document["concurrency"] != {
    "group": "regression-verifier-runner-mainnet",
    "cancel-in-progress": False,
}:
    raise SystemExit("candidate runner must serialize the exact mainnet concurrency group")
runner_jobs = runner_document["jobs"]
if not isinstance(runner_jobs, dict) or set(runner_jobs) != {"run-no-secrets"}:
    raise SystemExit("candidate runner must contain exactly one no-secrets job")
runner_job = runner_jobs["run-no-secrets"]
if set(runner_job) != {"runs-on", "timeout-minutes", "env", "steps"}:
    raise SystemExit("candidate runner job has an unreviewed execution setting")
if (
    runner_job["runs-on"] != "ubuntu-latest"
    or runner_job["timeout-minutes"] != 90
    or runner_job["env"]
    != {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "VERIFIER_ONE": "${{ vars.REGRESSION_VERIFIER_ONE_ADDRESS }}",
        "VERIFIER_TWO": "${{ vars.REGRESSION_VERIFIER_TWO_ADDRESS }}",
    }
):
    raise SystemExit("candidate runner environment is not the exact no-secrets contract")
checkout_action = "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5a"
python_action = "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97"
rust_action = "dtolnay/rust-toolchain@39b0b3842c7e8bbf6904c0bfc3d9006fdd4dc4e0"
cache_action = "Swatinem/rust-cache@42dc69e1aa15d09112580998cf2ef0119e2e91ae"
upload_action = "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"
runner_command = (
    "python scripts/regression_verifier_pipeline.py run --api-base $API_BASE_URL "
    "--network base-mainnet --verifier $VERIFIER_ONE --verifier $VERIFIER_TWO "
    "--worker target/release/worker --staging $RUNNER_TEMP/regression-staging "
    "--output target/regression-candidates --max-jobs 5"
)
build_guard_command = (
    "python scripts/regression_verifier_source_guard.py --scope worker-build "
    f"--expected-sha256 {WORKER_BUILD_SHA256}"
)
runtime_guard_command = (
    "python scripts/regression_verifier_source_guard.py --scope signing-runtime "
    f"--expected-sha256 {SIGNING_RUNTIME_SHA256}"
)
expected_runner_steps = [
    {
        "uses": checkout_action,
        "with": {
            "repository": "${{ github.repository }}",
            "ref": "main",
            "persist-credentials": False,
        },
    },
    {"uses": python_action, "with": {"python-version": "3.12"}},
    {"uses": rust_action},
    {"uses": cache_action},
    {"name": "Verify reviewed worker build inputs", "run": build_guard_command},
    {"name": "Build isolated regression worker", "run": "cargo build --release -p worker"},
    {"name": "Revalidate reviewed sources after build", "run": build_guard_command},
    {"name": "Revalidate reviewed signing runtime", "run": runtime_guard_command},
    {"name": "Run canonical jobs without signing secrets", "run": runner_command},
    {
        "uses": upload_action,
        "with": {
            "name": "regression-candidates-${{ github.run_id }}",
            "path": "target/regression-candidates",
            "if-no-files-found": "error",
            "retention-days": 7,
        },
    },
]
if runner_job["steps"] != expected_runner_steps:
    raise SystemExit("candidate runner steps are not the complete reviewed allowlist")

signer_workflow = require(".github/workflows/regression-verifier-signer.yml").read_text(encoding="utf-8")
reusable_workflow = require(
    ".github/workflows/regression-verifier-signing-reusable.yml"
).read_text(encoding="utf-8")
try:
    signer_document = json.loads(signer_workflow)
    reusable_document = json.loads(reusable_workflow)
except json.JSONDecodeError as error:
    raise SystemExit("signer workflows must use strict JSON-syntax YAML") from error
if set(signer_document) != {"name", "run-name", "on", "permissions", "jobs"}:
    raise SystemExit("signer workflow has unknown effective top-level keys")
if signer_document["name"] != "Regression Verifier Signer":
    raise SystemExit("signer workflow name is not exact")
if signer_document["run-name"] != (
    "Regression Verifier Signer / candidate run ${{ github.event.workflow_run.id }}"
):
    raise SystemExit("signer workflow run name must bind the candidate runner ID")
if signer_document["on"] != {
    "workflow_run": {
        "workflows": ["Regression Verifier Runner"],
        "types": ["completed"],
        "branches": ["main"],
    }
}:
    raise SystemExit("signer workflow trigger is not the exact protected runner completion")
if signer_document["permissions"] != {"actions": "read", "contents": "read"}:
    raise SystemExit("signer workflow permissions are not read-only")
signer_jobs = signer_document["jobs"]
if not isinstance(signer_jobs, dict) or set(signer_jobs) != {"sign-one", "sign-two", "relay"}:
    raise SystemExit("signer workflow must contain exactly two signers and one relay")
signer_guard = (
    "github.event.workflow_run.conclusion == 'success' && "
    "github.event.workflow_run.event == 'schedule' && "
    "github.event.workflow_run.head_branch == 'main' && "
    "github.event.workflow_run.head_repository.full_name == github.repository && "
    "github.event.workflow_run.head_sha == github.sha"
)
expected_provider_bindings = {
    "sign-one": ("https://mainnet.base.org", "https://developer-access-mainnet.base.org"),
    "sign-two": ("https://base-rpc.publicnode.com", "https://base-mainnet.public.blastapi.io"),
}
for job_name, (primary_rpc_url, secondary_rpc_url) in expected_provider_bindings.items():
    selected = signer_jobs[job_name]
    slot = "one" if job_name == "sign-one" else "two"
    address_variable = "ONE" if slot == "one" else "TWO"
    private_key = "ONE" if slot == "one" else "TWO"
    expected_job = {
        "if": signer_guard,
        "uses": "./.github/workflows/regression-verifier-signing-reusable.yml",
        "with": {
            "revision": "${{ github.event.workflow_run.head_sha }}",
            "candidate_run_id": "${{ github.event.workflow_run.id }}",
            "signer_slot": slot,
            "expected_signer": f"${{{{ vars.REGRESSION_VERIFIER_{address_variable}_ADDRESS }}}}",
            "primary_rpc_url": primary_rpc_url,
            "secondary_rpc_url": secondary_rpc_url,
        },
        "secrets": {
            "verifier_private_key": f"${{{{ secrets.REGRESSION_VERIFIER_{private_key}_PRIVATE_KEY }}}}"
        },
    }
    if selected != expected_job:
        raise SystemExit(f"{job_name} is not the exact reviewed reusable-signer call")
relay_job = signer_jobs["relay"]
if set(relay_job) != {
    "if", "needs", "runs-on", "timeout-minutes", "concurrency", "env", "steps"
}:
    raise SystemExit("relay job has an unreviewed execution setting")
if (
    relay_job["if"] != signer_guard
    or relay_job["needs"] != ["sign-one", "sign-two"]
    or relay_job["runs-on"] != "ubuntu-latest"
    or relay_job["timeout-minutes"] != 20
    or relay_job["concurrency"] != {
        "group": "regression-verifier-relay-mainnet",
        "cancel-in-progress": False,
    }
    or relay_job["env"] != {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "BASE_MAINNET_RPC_URL": "${{ github.run_attempt == 1 && 'https://1rpc.io/base' || 'https://base.meowrpc.com' }}",
        "KEEPER_ADDRESS": "0xc26a630e85134ed30968735c8e7de4576cfa5dbc",
        "VERIFIER_ONE": "${{ vars.REGRESSION_VERIFIER_ONE_ADDRESS }}",
        "VERIFIER_TWO": "${{ vars.REGRESSION_VERIFIER_TWO_ADDRESS }}",
    }
):
    raise SystemExit("relay job does not match its exact provider and execution contract")

try:
    workflow_call = reusable_document["on"]["workflow_call"]
    reusable_job = reusable_document["jobs"]["sign"]
except (KeyError, TypeError) as error:
    raise SystemExit("reusable signer effective input or job is missing") from error
required_string = {"required": True, "type": "string"}
if (
    set(reusable_document) != {"name", "on", "permissions", "jobs"}
    or reusable_document["name"] != "Regression Verifier Signing Slot"
    or reusable_document["permissions"] != {"actions": "read", "contents": "read"}
    or set(reusable_document["jobs"]) != {"sign"}
    or workflow_call != {
        "inputs": {
            "revision": required_string,
            "candidate_run_id": required_string,
            "signer_slot": required_string,
            "expected_signer": required_string,
            "primary_rpc_url": required_string,
            "secondary_rpc_url": required_string,
        },
        "secrets": {"verifier_private_key": {"required": True}},
    }
):
    raise SystemExit("reusable signer workflow contract is not exact")
if set(reusable_job) != {"runs-on", "timeout-minutes", "env", "steps"}:
    raise SystemExit("reusable signer has an unreviewed execution setting")
if (
    reusable_job["runs-on"] != "ubuntu-latest"
    or reusable_job["timeout-minutes"] != 20
    or reusable_job["env"] != {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "BASE_MAINNET_RPC_URL": "${{ github.run_attempt == 1 && inputs.primary_rpc_url || inputs.secondary_rpc_url }}",
        "ATTESTATION_OUTPUT": "target/attestations-${{ inputs.signer_slot }}",
        "EXPECTED_SIGNER": "${{ inputs.expected_signer }}",
    }
):
    raise SystemExit("reusable signer environment is not exact or exposes a signing key job-wide")

foundry_action = "foundry-rs/foundry-toolchain@b00af27efadbc7b4ca8b82abbd903b17cc874d2a"
download_action = "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c"
sign_command = (
    "python scripts/regression_verifier_pipeline.py sign --api-base $API_BASE_URL "
    "--network base-mainnet --rpc-url $BASE_MAINNET_RPC_URL "
    "--candidates target/regression-candidates --output $ATTESTATION_OUTPUT "
    "--worker target/release/worker --private-key-env REGRESSION_VERIFIER_PRIVATE_KEY "
    "--expected-signer $EXPECTED_SIGNER"
)
relay_command = (
    "python scripts/regression_verifier_pipeline.py relay --api-base $API_BASE_URL "
    "--network base-mainnet --rpc-url $BASE_MAINNET_RPC_URL "
    "--candidates target/regression-candidates --attestations target/attestations-one "
    "--attestations target/attestations-two --verifier $VERIFIER_ONE "
    "--verifier $VERIFIER_TWO --worker target/release/worker "
    "--expected-keeper $KEEPER_ADDRESS"
)
expected_signer_steps = [
    {
        "uses": checkout_action,
        "with": {"ref": "${{ inputs.revision }}", "persist-credentials": False},
    },
    {"uses": python_action, "with": {"python-version": "3.12"}},
    {"uses": rust_action},
    {"uses": cache_action},
    {"uses": foundry_action},
    {
        "uses": download_action,
        "with": {
            "name": "regression-candidates-${{ inputs.candidate_run_id }}",
            "path": "target/regression-candidates",
            "github-token": "${{ github.token }}",
            "run-id": "${{ inputs.candidate_run_id }}",
        },
    },
    {"name": "Verify reviewed worker build inputs", "run": build_guard_command},
    {"run": "cargo build --release -p worker"},
    {"name": "Revalidate reviewed sources after build", "run": build_guard_command},
    {"name": "Revalidate reviewed signing runtime", "run": runtime_guard_command},
    {
        "name": "Re-fetch state and sign one exact candidate set",
        "run": sign_command,
        "env": {
            "REGRESSION_VERIFIER_PRIVATE_KEY": "${{ secrets.verifier_private_key }}"
        },
    },
    {
        "uses": upload_action,
        "with": {
            "name": "regression-attestations-${{ inputs.signer_slot }}-${{ inputs.candidate_run_id }}",
            "path": "target/attestations-${{ inputs.signer_slot }}",
            "if-no-files-found": "error",
            "retention-days": 7,
        },
    },
]
if reusable_job["steps"] != expected_signer_steps:
    raise SystemExit("reusable signer steps are not the complete reviewed allowlist")

expected_relay_steps = [
    {
        "uses": checkout_action,
        "with": {
            "ref": "${{ github.event.workflow_run.head_sha }}",
            "persist-credentials": False,
        },
    },
    {"uses": python_action, "with": {"python-version": "3.12"}},
    {"uses": rust_action},
    {"uses": cache_action},
    {"uses": foundry_action},
    {
        "uses": download_action,
        "with": {
            "name": "regression-candidates-${{ github.event.workflow_run.id }}",
            "path": "target/regression-candidates",
            "github-token": "${{ github.token }}",
            "run-id": "${{ github.event.workflow_run.id }}",
        },
    },
    {
        "uses": download_action,
        "with": {
            "name": "regression-attestations-one-${{ github.event.workflow_run.id }}",
            "path": "target/attestations-one",
        },
    },
    {
        "uses": download_action,
        "with": {
            "name": "regression-attestations-two-${{ github.event.workflow_run.id }}",
            "path": "target/attestations-two",
        },
    },
    {"name": "Verify reviewed worker build inputs", "run": build_guard_command},
    {"run": "cargo build --release -p worker"},
    {"name": "Revalidate reviewed sources after build", "run": build_guard_command},
    {"name": "Revalidate reviewed signing runtime", "run": runtime_guard_command},
    {
        "name": "Revalidate and relay exact quorum",
        "run": relay_command,
        "env": {"BASE_KEEPER_PRIVATE_KEY": "${{ secrets.BASE_KEEPER_PRIVATE_KEY }}"},
    },
]
if relay_job["steps"] != expected_relay_steps:
    raise SystemExit("relay steps are not the complete reviewed allowlist")


def exact_pipeline_tokens(steps: object, subcommand: str) -> list[str]:
    if not isinstance(steps, list) or not all(isinstance(step, dict) for step in steps):
        raise SystemExit(f"{subcommand} workflow steps are invalid")
    commands = [str(step["run"]) for step in steps if "run" in step]
    matches: list[list[str]] = []
    for command in commands:
        lexer = shlex.shlex(command, posix=True, punctuation_chars=True)
        lexer.whitespace_split = True
        lexer.commenters = "#"
        try:
            tokens = list(lexer)
        except ValueError as error:
            raise SystemExit(f"{subcommand} workflow command is not safely parseable") from error
        if tokens[:3] == ["python", "scripts/regression_verifier_pipeline.py", subcommand]:
            matches.append(tokens)
    if len(matches) != 1:
        raise SystemExit(f"workflow must contain exactly one effective {subcommand} pipeline command")
    return matches[0]


expected_sign_tokens = [
    "python", "scripts/regression_verifier_pipeline.py", "sign",
    "--api-base", "$API_BASE_URL",
    "--network", "base-mainnet",
    "--rpc-url", "$BASE_MAINNET_RPC_URL",
    "--candidates", "target/regression-candidates",
    "--output", "$ATTESTATION_OUTPUT",
    "--worker", "target/release/worker",
    "--private-key-env", "REGRESSION_VERIFIER_PRIVATE_KEY",
    "--expected-signer", "$EXPECTED_SIGNER",
]
expected_relay_tokens = [
    "python", "scripts/regression_verifier_pipeline.py", "relay",
    "--api-base", "$API_BASE_URL",
    "--network", "base-mainnet",
    "--rpc-url", "$BASE_MAINNET_RPC_URL",
    "--candidates", "target/regression-candidates",
    "--attestations", "target/attestations-one",
    "--attestations", "target/attestations-two",
    "--verifier", "$VERIFIER_ONE",
    "--verifier", "$VERIFIER_TWO",
    "--worker", "target/release/worker",
    "--expected-keeper", "$KEEPER_ADDRESS",
]
if exact_pipeline_tokens(reusable_job.get("steps"), "sign") != expected_sign_tokens:
    raise SystemExit("effective signer command must equal the complete precommitted argv")
if exact_pipeline_tokens(relay_job.get("steps"), "relay") != expected_relay_tokens:
    raise SystemExit("effective relay command must equal the complete precommitted argv")

documentation = require("docs/sandboxed-regression-verifier.md").read_text(encoding="utf-8").lower()
for phrase in ("watchdog", "fail closed", "idempotency", "provider", "bountysettled"):
    if phrase not in documentation:
        raise SystemExit(f"verifier documentation is missing {phrase}")

print("verifier settlement watchdog acceptance checks passed")
