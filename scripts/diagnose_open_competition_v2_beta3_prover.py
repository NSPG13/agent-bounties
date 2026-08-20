#!/usr/bin/env python3
"""Replay one persisted Beta3 prover job and emit redacted host diagnostics."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
from typing import Any


PROVIDER_JOB_ID = re.compile(r"^beta3-[0-9a-f]{64}$")
PROFILE_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")


def redact(value: str) -> str:
    result = value
    for key, secret in os.environ.items():
        if secret and any(marker in key.upper() for marker in ("KEY", "TOKEN", "SECRET", "PASSWORD")):
            result = result.replace(secret, "[redacted]")
    result = re.sub(r"(?i)(https?://)[^/@\s:]+:[^/@\s]+@", r"\1[redacted]@", result)
    return result[-8_000:]


def load_record(root: Path, provider_job_id: str) -> tuple[Path, dict[str, Any]]:
    for path in root.glob("*.json"):
        record = json.loads(path.read_text(encoding="utf-8"))
        if record.get("provider_job_id") == provider_job_id:
            return path, record
    raise RuntimeError("provider job is not present on this prover host")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--provider-job-id", required=True)
    parser.add_argument("--job-dir", type=Path, default=Path("/var/lib/agent-bounties-prover/jobs"))
    parser.add_argument("--timeout-seconds", type=int, default=1_800)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not PROVIDER_JOB_ID.fullmatch(args.provider_job_id):
        raise SystemExit("provider job id is invalid")

    path, record = load_record(args.job_dir, args.provider_job_id)
    request = record.get("request")
    if not isinstance(request, dict) or not isinstance(request.get("program_input"), dict):
        raise RuntimeError("persisted provider request is invalid")
    program_input = dict(request["program_input"])
    profile_id = program_input.pop("_profile_id", "public-vector-metric-v1")
    if not isinstance(profile_id, str) or not PROFILE_ID.fullmatch(profile_id):
        raise RuntimeError("persisted provider profile is invalid")
    binaries = json.loads(os.environ.get("OPEN_COMPETITION_V2_PROVER_BINARIES", "{}"))
    binary = Path(str(binaries.get(profile_id, "")))
    if not binary.is_file():
        raise RuntimeError("the pinned prover binary is unavailable")

    disk = shutil.disk_usage(args.job_dir)
    diagnostics: dict[str, Any] = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-prover-diagnostic-v1",
        "provider_job_id": args.provider_job_id,
        "persisted_status": record.get("status"),
        "persisted_failure_code": record.get("failure_code"),
        "profile_id": profile_id,
        "proof_system": request.get("proof_system"),
        "binary_exists": binary.is_file(),
        "binary_executable": os.access(binary, os.X_OK),
        "job_record_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "disk_free_bytes": disk.free,
        "docker_socket_exists": Path("/var/run/docker.sock").exists(),
        "uid": os.getuid(),
        "gid": os.getgid(),
    }
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "program-input.json"
            fixture.write_text(json.dumps(program_input, separators=(",", ":")), encoding="utf-8")
            completed = subprocess.run(
                [str(binary), str(fixture), str(request["proof_system"])],
                check=False,
                capture_output=True,
                text=True,
                timeout=args.timeout_seconds,
                env={**os.environ, "SP1_PROVER": "cpu"},
            )
        diagnostics["exit_code"] = completed.returncode
        diagnostics["stderr_sha256"] = hashlib.sha256(completed.stderr.encode()).hexdigest()
        diagnostics["stderr_tail"] = redact(completed.stderr)
        output = None
        if completed.stdout.strip():
            try:
                output = json.loads(completed.stdout.strip().splitlines()[-1])
            except json.JSONDecodeError:
                diagnostics["stdout_tail"] = redact(completed.stdout)
        journal = output.get("journal_hex") if isinstance(output, dict) else None
        proof = output.get("proof_hex") if isinstance(output, dict) else None
        diagnostics["journal_matches"] = (
            isinstance(journal, str)
            and journal.lower() == str(request.get("expected_public_values", "")).lower()
        )
        diagnostics["proof_present"] = isinstance(proof, str) and proof.startswith("0x")
        diagnostics["proof_sha256"] = (
            hashlib.sha256(proof.encode()).hexdigest() if diagnostics["proof_present"] else None
        )
        diagnostics["passed"] = (
            completed.returncode == 0
            and diagnostics["journal_matches"]
            and diagnostics["proof_present"]
        )
    except subprocess.TimeoutExpired as error:
        diagnostics.update(
            passed=False,
            timed_out=True,
            stderr_tail=redact((error.stderr or b"").decode() if isinstance(error.stderr, bytes) else (error.stderr or "")),
        )
    diagnostics["elapsed_seconds"] = round(time.monotonic() - started, 3)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(diagnostics, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({key: diagnostics.get(key) for key in ("passed", "exit_code", "timed_out", "elapsed_seconds")}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
