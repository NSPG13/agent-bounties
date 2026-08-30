#!/usr/bin/env python3
"""Immutable acceptance checks for the verifier settlement watchdog."""

from __future__ import annotations

import hashlib
import http.server
import json
import os
import random
import re
import subprocess
import sys
import tempfile
import threading
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
    workflow_run_id: int | None = None,
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
        "workflow_run_id": workflow_run_id
        if workflow_run_id is not None
        else 4200 + {"runner": 1, "signer_one": 2, "signer_two": 3, "relay": 4}[stage],
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
    expected_by_id = {item["job_id"]: item for item in expected_jobs}
    automated_targets = {
        "dispatch_runner": "regression-verifier-runner.yml",
        "retry_runner": "regression-verifier-runner.yml",
        "retry_signer_one": "regression-verifier-signer.yml",
        "retry_signer_two": "regression-verifier-signer.yml",
        "retry_relay": "regression-verifier-signer.yml",
    }
    forbidden_actions = {"accept", "reject", "sign", "settle", "pay", "transfer", "wallet_call"}
    for record in records:
        job_id = record.get("job_id")
        if not isinstance(job_id, str) or job_id in by_id:
            raise SystemExit("watchdog record job_id is missing or duplicated")
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
        target_workflow = record.get("target_workflow")
        workflow_run_id = record.get("workflow_run_id")
        if record.get("automation_allowed") is True:
            expected_target = automated_targets.get(str(action))
            if target_workflow != expected_target:
                raise SystemExit(f"watchdog record {job_id} does not bind its allowlisted workflow")
            if action == "dispatch_runner" and workflow_run_id is not None:
                raise SystemExit(f"new runner dispatch {job_id} must not claim an existing workflow run")
            if action != "dispatch_runner" and (
                not isinstance(workflow_run_id, int) or workflow_run_id <= 0
            ):
                raise SystemExit(f"watchdog retry {job_id} must bind a positive workflow run ID")
        elif target_workflow is not None or workflow_run_id is not None:
            raise SystemExit(f"non-automated watchdog record {job_id} must not target a workflow")
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

# Exercise a larger deterministic matrix with non-semantic job IDs so a solver
# must implement the state machine rather than special-case the named examples.
rng = random.Random(918273)
matrix_jobs: list[dict[str, Any]] = []
matrix_runs: list[dict[str, Any]] = []
matrix_expected: dict[str, tuple[str, bool, str | None]] = {}
cases = (
    "dispatch",
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
)
for index in range(84):
    case = cases[index % len(cases)]
    opaque = hashlib.sha256(f"{rng.getrandbits(128):032x}:{index}".encode()).hexdigest()[:18]
    job_id = f"matrix-{opaque}"
    expiry_minute = 20 + index
    expiry_hour, expiry_minute = divmod(expiry_minute, 60)
    expiry = f"2026-09-01T{12 + expiry_hour:02d}:{expiry_minute:02d}:00Z"
    item = job(job_id, expiry)
    expected: tuple[str, bool, str | None]
    if case == "dispatch":
        expected = ("dispatch_runner", True, None)
    elif case == "runner_retry":
        matrix_runs.append(run(job_id, "runner", "failure"))
        expected = ("retry_runner", True, None)
    elif case == "signer_one_retry":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "failure", signer=ADDRESS_ONE),
            ]
        )
        expected = ("retry_signer_one", True, "signer_one_secondary")
    elif case == "signer_two_retry":
        matrix_runs.extend(
            [
                run(job_id, "runner", "success", artifact_hash=candidate_hash),
                run(job_id, "signer_one", "success", signer=ADDRESS_ONE),
                run(job_id, "signer_two", "failure", signer=ADDRESS_TWO),
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
    else:
        item["status"] = "settled"
        item["canonical_terminal_event"] = "BountySettled"
        expected = ("observe_terminal", False, None)
    matrix_jobs.append(item)
    matrix_expected[job_id] = expected

_, matrix_plan = invoke(matrix_jobs, matrix_runs)
matrix = validate(matrix_plan, matrix_jobs)
for job_id, (action, automated, provider) in matrix_expected.items():
    expect(matrix[job_id], action, automated, provider)


class FakeGitHubHandler(http.server.BaseHTTPRequestHandler):
    requests: list[dict[str, Any]] = []
    signer_run_id = 0
    unsafe_run_metadata = False

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

    def respond(self, status: int, payload: dict[str, Any] | None = None) -> None:
        body = b"" if payload is None else json.dumps(payload).encode("utf-8")
        self.send_response(status)
        if body:
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - standard library handler name
        self.record()
        if self.path == "/repos/NSPG13/agent-bounties/branches/main":
            self.respond(200, {"name": "main", "commit": {"sha": MAIN_SHA}})
            return
        expected = f"/repos/NSPG13/agent-bounties/actions/runs/{self.signer_run_id}"
        if self.path != expected:
            self.respond(404, {"message": "not found"})
            return
        self.respond(
            200,
            {
                "id": self.signer_run_id,
                "path": ".github/workflows/regression-verifier-signer.yml",
                "head_sha": "b" * 40 if self.unsafe_run_metadata else MAIN_SHA,
                "status": "completed",
                "conclusion": "failure",
            },
        )

    def do_POST(self) -> None:  # noqa: N802 - standard library handler name
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        self.record(body)
        allowed = {
            "/repos/NSPG13/agent-bounties/actions/workflows/"
            "regression-verifier-runner.yml/dispatches",
            f"/repos/NSPG13/agent-bounties/actions/runs/{self.signer_run_id}/rerun-failed-jobs",
        }
        self.respond(204 if self.path in allowed else 404, None if self.path in allowed else {"message": "not found"})


def execute(plan: dict[str, Any], api_base: str) -> subprocess.CompletedProcess[bytes]:
    tool = require("scripts/regression_verifier_watchdog.py")
    with tempfile.TemporaryDirectory(prefix="watchdog-execute-") as temporary:
        plan_path = Path(temporary) / "plan.json"
        plan_path.write_text(json.dumps(plan), encoding="utf-8")
        environment = os.environ.copy()
        environment["WATCHDOG_BENCHMARK_TOKEN"] = "benchmark-token"
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
                "--execute",
            ],
            cwd=ROOT,
            env=environment,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=30,
            check=False,
        )


# Exercise the production executor against a local fake GitHub boundary. It may
# dispatch the runner and rerun only a current-main signer workflow; changing
# the target workflow must fail before any write request.
dispatch_jobs = [job("execute-dispatch", "2026-09-01T14:00:00Z")]
_, dispatch_plan = invoke(dispatch_jobs, [])
relay_execute_jobs = [job("execute-relay", "2026-09-01T14:10:00Z")]
relay_execute_runs = [
    run("execute-relay", "runner", "success", artifact_hash=candidate_hash, workflow_run_id=4301),
    run("execute-relay", "signer_one", "success", signer=ADDRESS_ONE, workflow_run_id=4302),
    run("execute-relay", "signer_two", "success", signer=ADDRESS_TWO, workflow_run_id=4303),
    run(
        "execute-relay",
        "relay",
        "failure",
        provider_role="relay_primary",
        workflow_run_id=4304,
    ),
]
_, relay_execute_plan = invoke(relay_execute_jobs, relay_execute_runs)
execution_plan = {
    "schema": SCHEMA,
    "network": "base-mainnet",
    "generated_at": NOW,
    "fail_closed": True,
    "repository": "NSPG13/agent-bounties",
    "current_main_sha": MAIN_SHA,
    "jobs": dispatch_plan["jobs"] + relay_execute_plan["jobs"],
}
FakeGitHubHandler.requests = []
FakeGitHubHandler.signer_run_id = 4304
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FakeGitHubHandler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
try:
    api_base = f"http://127.0.0.1:{server.server_port}"
    completed = execute(execution_plan, api_base)
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
        "/repos/NSPG13/agent-bounties/actions/workflows/"
        "regression-verifier-runner.yml/dispatches",
        "/repos/NSPG13/agent-bounties/actions/runs/4304/rerun-failed-jobs",
    }
    if {item["path"] for item in writes} != expected_writes or len(writes) != 2:
        raise SystemExit("watchdog execute wrote outside the exact allowlisted GitHub actions")
    if any(item["authorization"] != "Bearer benchmark-token" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute did not use the environment-scoped token")
    dispatch = next(item for item in writes if item["path"].endswith("/dispatches"))
    if json.loads(dispatch["body"]) != {"ref": "main"}:
        raise SystemExit("watchdog runner dispatch must bind the protected main ref")

    unsafe_plan = json.loads(json.dumps(execution_plan))
    unsafe_plan["jobs"][0]["target_workflow"] = "unreviewed-wallet-job.yml"
    request_count = len(FakeGitHubHandler.requests)
    rejected = execute(unsafe_plan, api_base)
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted a non-allowlisted workflow")
    if len(FakeGitHubHandler.requests) != request_count:
        raise SystemExit("watchdog execute contacted GitHub before rejecting an unknown workflow")

    FakeGitHubHandler.requests = []
    FakeGitHubHandler.unsafe_run_metadata = True
    rejected = execute(execution_plan, api_base)
    if rejected.returncode == 0:
        raise SystemExit("watchdog execute accepted stale metadata in a later action")
    if any(item["method"] == "POST" for item in FakeGitHubHandler.requests):
        raise SystemExit("watchdog execute wrote an earlier action before all metadata passed")
finally:
    FakeGitHubHandler.unsafe_run_metadata = False
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

workflow = require(".github/workflows/regression-verifier-watchdog.yml").read_text(encoding="utf-8")
workflow_lower = workflow.lower()
for forbidden in ("pull_request_target", "issue_comment:", "id-token: write", "contents: write", "secrets."):
    if forbidden in workflow_lower:
        raise SystemExit(f"watchdog workflow contains forbidden privilege or trigger: {forbidden}")


def top_level_block(source: str, key: str) -> str:
    lines = source.splitlines()
    try:
        start = next(index for index, line in enumerate(lines) if line.strip() == key and not line[:1].isspace())
    except StopIteration as error:
        raise SystemExit(f"watchdog workflow has no effective top-level {key}") from error
    collected = []
    for line in lines[start + 1 :]:
        stripped = line.strip()
        if stripped and not stripped.startswith("#") and not line[:1].isspace():
            break
        collected.append(line)
    return "\n".join(collected)


trigger_block = top_level_block(workflow, "on:")
trigger_keys = {
    match.group(1)
    for line in trigger_block.splitlines()
    if (match := re.match(r"^  ([a-zA-Z0-9_-]+):\s*(?:#.*)?$", line))
}
if trigger_keys != {"schedule", "workflow_dispatch"}:
    raise SystemExit("watchdog workflow triggers must be exactly schedule and workflow_dispatch")
if not re.search(r'(?m)^\s{4,}-\s+cron:\s*["\'][^"\']+["\']\s*$', trigger_block):
    raise SystemExit("watchdog schedule has no effective cron entry")

jobs_block = top_level_block(workflow, "jobs:")
job_keys = {
    match.group(1)
    for line in jobs_block.splitlines()
    if (match := re.match(r"^  ([a-zA-Z0-9_-]+):\s*(?:#.*)?$", line))
}
if job_keys != {"watchdog"}:
    raise SystemExit("watchdog workflow must contain exactly one effective watchdog job")
command_matches = re.findall(r"(?m)^\s{6,}-\s+run:\s*(\S.*)$", jobs_block)
execute_commands = [
    command
    for command in command_matches
    if re.search(r"(?:^|\s)python\s+scripts/regression_verifier_watchdog\.py\s+execute(?:\s|$)", command)
]
if len(execute_commands) != 1:
    raise SystemExit("scheduled watchdog job must invoke exactly one effective execute command")
execute_command = execute_commands[0]
for flag in ("--plan", "--repository", "--github-api-base", "--token-env", "--execute"):
    if not re.search(rf"(?:^|\s){re.escape(flag)}(?:\s|$)", execute_command):
        raise SystemExit(f"effective watchdog execute command is missing {flag}")
if not re.search(
    r"(?m)^\s{6,}GITHUB_TOKEN:\s*\$\{\{\s*github\.token\s*\}\}\s*$",
    jobs_block,
):
    raise SystemExit("watchdog execute job does not bind the repository token")
for allowed in ("regression-verifier-runner.yml", "regression-verifier-signer.yml"):
    if allowed not in execute_command:
        raise SystemExit(f"watchdog execute command does not pin allowlisted workflow {allowed}")

permission_block = top_level_block(workflow_lower, "permissions:")
permission_lines = {
    line.strip()
    for line in permission_block.splitlines()
    if line.strip() and not line.lstrip().startswith("#")
}
if permission_lines != {"contents: read", "actions: write"}:
    raise SystemExit("watchdog workflow permissions must be exactly contents: read and actions: write")
referenced_workflows = set(re.findall(r"[a-z0-9_-]+\.ya?ml", execute_command.lower()))
unknown_workflows = referenced_workflows - {
    "regression-verifier-runner.yml",
    "regression-verifier-signer.yml",
}
if unknown_workflows:
    raise SystemExit(f"watchdog workflow references an unknown workflow: {sorted(unknown_workflows)}")

signer_workflow = require(".github/workflows/regression-verifier-signer.yml").read_text(encoding="utf-8")
reusable_workflow = require(
    ".github/workflows/regression-verifier-signing-reusable.yml"
).read_text(encoding="utf-8")
if "REGRESSION_VERIFIER_RPC_URL" in signer_workflow or "REGRESSION_VERIFIER_RPC_URL" in reusable_workflow:
    raise SystemExit("signer workflows still consume the shared regression verifier RPC variable")


def provider_binding(block: str, key: str, variable: str) -> str:
    match = re.search(
        rf"(?m)^\s*{re.escape(key)}:\s*\$\{{\{{\s*vars\.{variable}\s*\|\|\s*'([^']+)'\s*\}}\}}\s*$",
        block,
    )
    if not match:
        raise SystemExit(f"effective provider binding is missing for {variable}")
    fallback = match.group(1)
    if not fallback.startswith("https://"):
        raise SystemExit(f"provider fallback for {variable} must be public HTTPS")
    return fallback


try:
    sign_one_block = signer_workflow.split("\n  sign-one:", 1)[1].split("\n  sign-two:", 1)[0]
    sign_two_block = signer_workflow.split("\n  sign-two:", 1)[1].split("\n  relay:", 1)[0]
    relay_block = signer_workflow.split("\n  relay:", 1)[1]
except IndexError as error:
    raise SystemExit("signer workflow job boundaries are unavailable") from error
fallbacks = {
    provider_binding(sign_one_block, "rpc_url", "REGRESSION_VERIFIER_ONE_RPC_URL"),
    provider_binding(sign_two_block, "rpc_url", "REGRESSION_VERIFIER_TWO_RPC_URL"),
    provider_binding(relay_block, "BASE_MAINNET_RPC_URL", "REGRESSION_VERIFIER_RELAY_RPC_URL"),
}
if len(fallbacks) != 3:
    raise SystemExit("signer one, signer two, and relay must have distinct public fallbacks")

workflow_call = reusable_workflow.split("workflow_call:", 1)[1].split("secrets:", 1)[0]
rpc_input = workflow_call.split("rpc_url:", 1)[1]
if not re.search(r"required:\s*true", rpc_input) or not re.search(r"type:\s*string", rpc_input):
    raise SystemExit("reusable signer rpc_url input must be a required string")
if not re.search(
    r"(?m)^\s*BASE_MAINNET_RPC_URL:\s*\$\{\{\s*inputs\.rpc_url\s*\}\}\s*$",
    reusable_workflow,
):
    raise SystemExit("reusable signer does not consume its exact rpc_url input")
if '--rpc-url "$BASE_MAINNET_RPC_URL"' not in reusable_workflow:
    raise SystemExit("reusable signer does not pass its bound provider to the signing command")
if '--rpc-url "$BASE_MAINNET_RPC_URL"' not in relay_block:
    raise SystemExit("relay does not pass its independent provider to the relay command")

documentation = require("docs/sandboxed-regression-verifier.md").read_text(encoding="utf-8").lower()
for phrase in ("watchdog", "fail closed", "idempotency", "provider", "bountysettled"):
    if phrase not in documentation:
        raise SystemExit(f"verifier documentation is missing {phrase}")

print("verifier settlement watchdog acceptance checks passed")
