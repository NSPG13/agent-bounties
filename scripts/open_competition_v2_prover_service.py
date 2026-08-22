#!/usr/bin/env python3
"""Authenticated, durable HTTP wrapper for the pinned Beta3 CPU prover."""

from __future__ import annotations

from concurrent.futures import ThreadPoolExecutor
import hashlib
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hmac
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any


REQUEST_SCHEMA = "agent-bounties/open-competition-v2-prover-request-v1"
MAX_BODY_BYTES = 1_048_576
IDEMPOTENCY = re.compile(r"^[A-Za-z0-9:_-]{1,200}$")
PROFILE_ID = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
HEX_640 = re.compile(r"^0x[0-9a-fA-F]{1280}$")


class QueueFullError(RuntimeError):
    pass


def process_diagnostic(value: str) -> str:
    result = value
    for key, secret in os.environ.items():
        if secret and any(marker in key.upper() for marker in ("KEY", "TOKEN", "SECRET", "PASSWORD")):
            result = result.replace(secret, "[redacted]")
    result = re.sub(r"(?i)(https?://)[^/@\s:]+:[^/@\s]+@", r"\1[redacted]@", result)
    return result[-4_000:]


def canonical_hash(value: Any) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return "0x" + hashlib.sha256(encoded).hexdigest()


def validate_request(value: Any, idempotency_header: str, now: int) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "schema_version",
        "idempotency_key",
        "proof_job_id",
        "proof_system",
        "program_input",
        "expected_public_values",
        "proof_sla_deadline",
    }:
        raise ValueError("request fields do not match the prover schema")
    if value["schema_version"] != REQUEST_SCHEMA:
        raise ValueError("request schema mismatch")
    key = value["idempotency_key"]
    if not isinstance(key, str) or not IDEMPOTENCY.fullmatch(key) or key != idempotency_header:
        raise ValueError("idempotency key is invalid or differs from the header")
    if not isinstance(value["proof_job_id"], str) or not re.fullmatch(
        r"[0-9a-fA-F-]{36}", value["proof_job_id"]
    ):
        raise ValueError("proof_job_id is invalid")
    if value["proof_system"] not in {"groth16", "plonk"}:
        raise ValueError("proof_system must be groth16 or plonk")
    if not isinstance(value["program_input"], dict):
        raise ValueError("program_input must be an object")
    if not isinstance(value["expected_public_values"], str) or not HEX_640.fullmatch(
        value["expected_public_values"]
    ):
        raise ValueError("expected_public_values must be exactly 640 bytes")
    deadline = value["proof_sla_deadline"]
    if not isinstance(deadline, int) or deadline <= now:
        raise ValueError("proof SLA deadline has expired")
    return value


class ProverJobs:
    def __init__(
        self,
        root: Path,
        binaries: dict[str, Path],
        maximum_seconds: int,
        maximum_queued: int,
    ) -> None:
        self.root = root
        self.binaries = binaries
        self.maximum_seconds = maximum_seconds
        self.maximum_queued = maximum_queued
        self.root.mkdir(parents=True, exist_ok=True)
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="beta3-prover")
        self.lock = threading.Lock()
        self.queued: set[str] = set()

    def path_for(self, key: str) -> Path:
        return self.root / f"{hashlib.sha256(key.encode()).hexdigest()}.json"

    def read(self, key: str) -> dict[str, Any] | None:
        path = self.path_for(key)
        return json.loads(path.read_text(encoding="utf-8")) if path.exists() else None

    def write(self, key: str, record: dict[str, Any]) -> None:
        path = self.path_for(key)
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", dir=self.root, delete=False
        ) as handle:
            json.dump(record, handle, separators=(",", ":"))
            handle.write("\n")
            temporary = Path(handle.name)
        temporary.replace(path)

    def submit(self, request: dict[str, Any]) -> dict[str, Any]:
        key = request["idempotency_key"]
        request_hash = canonical_hash(request)
        with self.lock:
            record = self.read(key)
            if record is not None:
                if record["request_hash"] != request_hash:
                    raise RuntimeError("idempotency key was reused for another request")
            else:
                if len(self.queued) >= self.maximum_queued:
                    raise QueueFullError("the proving queue is at its bounded capacity")
                record = {
                    "request_hash": request_hash,
                    "request": request,
                    "status": "pending",
                    "provider_job_id": "beta3-" + hashlib.sha256(key.encode()).hexdigest(),
                    "proof": None,
                    "public_values": None,
                    "failure_code": None,
                    "failure_message": None,
                }
                self.write(key, record)
            if record["status"] == "pending" and key not in self.queued:
                if len(self.queued) >= self.maximum_queued:
                    raise QueueFullError("the proving queue is at its bounded capacity")
                self.queued.add(key)
                self.executor.submit(self._prove, key)
            return response_for(record)

    def resume_pending(self) -> None:
        for path in self.root.glob("*.json"):
            record = json.loads(path.read_text(encoding="utf-8"))
            if record.get("status") == "pending":
                self.submit(record["request"])

    def _prove(self, key: str) -> None:
        record = self.read(key)
        if record is None:
            with self.lock:
                self.queued.discard(key)
            return
        try:
            request = record["request"]
            program_input = dict(request["program_input"])
            profile_id = program_input.pop("_profile_id", "public-vector-metric-v1")
            if not isinstance(profile_id, str) or not PROFILE_ID.fullmatch(profile_id):
                raise RuntimeError("program profile is invalid")
            binary = self.binaries.get(profile_id)
            if binary is None:
                raise RuntimeError("program profile is unavailable")
            remaining = request["proof_sla_deadline"] - int(time.time())
            if remaining <= 0:
                raise TimeoutError("proof SLA expired before execution")
            timeout = min(remaining, self.maximum_seconds)
            with tempfile.TemporaryDirectory(dir=self.root) as directory:
                fixture = Path(directory) / "program-input.json"
                fixture.write_text(
                    json.dumps(program_input, separators=(",", ":")),
                    encoding="utf-8",
                )
                completed = subprocess.run(
                    [str(binary), str(fixture), request["proof_system"]],
                    check=True,
                    capture_output=True,
                    text=True,
                    timeout=timeout,
                    env={**os.environ, "SP1_PROVER": "cpu"},
                )
            output = json.loads(completed.stdout.strip().splitlines()[-1])
            proof = output.get("proof_hex")
            public_values = output.get("journal_hex")
            if (
                not isinstance(proof, str)
                or not re.fullmatch(r"0x[0-9a-f]+", proof)
                or not isinstance(public_values, str)
                or public_values.lower() != request["expected_public_values"].lower()
            ):
                raise RuntimeError("prover output does not match the request-bound journal")
            record.update(
                status="proved",
                proof=proof,
                public_values=public_values,
                failure_code=None,
                failure_message=None,
            )
        except subprocess.TimeoutExpired:
            record.update(
                status="failed",
                failure_code="proof_timeout",
                failure_message="CPU proving exceeded the request-bound SLA.",
            )
        except subprocess.CalledProcessError as error:
            print(
                json.dumps(
                    {
                        "event": "prover_subprocess_failed",
                        "return_code": error.returncode,
                        "stderr_tail": process_diagnostic(error.stderr or ""),
                    },
                    separators=(",", ":"),
                ),
                file=sys.stderr,
                flush=True,
            )
            record.update(
                status="failed",
                failure_code="proof_failed",
                failure_message="The pinned CPU prover failed without producing a valid bound proof.",
            )
        except Exception as error:
            print(
                json.dumps(
                    {
                        "event": "prover_internal_failed",
                        "error_type": type(error).__name__,
                        "message": process_diagnostic(str(error)),
                    },
                    separators=(",", ":"),
                ),
                file=sys.stderr,
                flush=True,
            )
            record.update(
                status="failed",
                failure_code="proof_failed",
                failure_message="The pinned CPU prover failed without producing a valid bound proof.",
            )
        finally:
            self.write(key, record)
            with self.lock:
                self.queued.discard(key)


def response_for(record: dict[str, Any]) -> dict[str, Any]:
    return {
        "status": record["status"],
        "provider_job_id": record["provider_job_id"],
        "proof": record["proof"],
        "public_values": record["public_values"],
        "failure_code": record["failure_code"],
        "failure_message": record["failure_message"],
    }


class Handler(BaseHTTPRequestHandler):
    server_version = "AgentBountiesBeta3Prover/1"

    def send_json(self, status: int, value: Any) -> None:
        body = json.dumps(value, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.send_header("cache-control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path != "/health":
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not_found"})
            return
        self.send_json(HTTPStatus.OK, {"status": "ok", "backend": "cpu"})

    def do_POST(self) -> None:
        if self.path != "/v1/prove":
            self.send_json(HTTPStatus.NOT_FOUND, {"error": "not_found"})
            return
        expected = "Bearer " + self.server.api_key
        if not hmac.compare_digest(self.headers.get("authorization", ""), expected):
            self.send_json(HTTPStatus.UNAUTHORIZED, {"error": "unauthorized"})
            return
        try:
            length = int(self.headers.get("content-length", "0"))
            if not 0 < length <= MAX_BODY_BYTES:
                raise ValueError("request body size is invalid")
            value = json.loads(self.rfile.read(length))
            request = validate_request(
                value, self.headers.get("idempotency-key", ""), int(time.time())
            )
            self.send_json(HTTPStatus.OK, self.server.jobs.submit(request))
        except QueueFullError as error:
            self.send_json(HTTPStatus.TOO_MANY_REQUESTS, {"error": str(error)})
        except RuntimeError as error:
            self.send_json(HTTPStatus.CONFLICT, {"error": str(error)})
        except (ValueError, json.JSONDecodeError) as error:
            self.send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})

    def log_message(self, format: str, *args: Any) -> None:
        return


class ProverServer(ThreadingHTTPServer):
    def __init__(self, address: tuple[str, int], api_key: str, jobs: ProverJobs) -> None:
        super().__init__(address, Handler)
        self.api_key = api_key
        self.jobs = jobs


def main() -> int:
    api_key = os.environ.get("OPEN_COMPETITION_V2_PROVER_API_KEY", "")
    configured = os.environ.get("OPEN_COMPETITION_V2_PROVER_BINARIES", "").strip()
    if configured:
        value = json.loads(configured)
        if not isinstance(value, dict) or not value:
            raise SystemExit("OPEN_COMPETITION_V2_PROVER_BINARIES must be a nonempty JSON object")
        binaries = {str(profile): Path(str(path)) for profile, path in value.items()}
    else:
        binaries = {
            "public-vector-metric-v1": Path(
                os.environ.get("OPEN_COMPETITION_V2_PROVER_BINARY", "")
            )
        }
    root = Path(os.environ.get("OPEN_COMPETITION_V2_PROVER_JOB_DIR", "prover-jobs"))
    if len(api_key) < 32:
        raise SystemExit("OPEN_COMPETITION_V2_PROVER_API_KEY must contain at least 32 characters")
    if any(not PROFILE_ID.fullmatch(profile) or not binary.is_file() for profile, binary in binaries.items()):
        raise SystemExit("an OPEN_COMPETITION_V2_PROVER_BINARIES profile is invalid or unavailable")
    maximum_seconds = int(os.environ.get("OPEN_COMPETITION_V2_PROVER_MAX_SECONDS", "600"))
    maximum_queued = int(os.environ.get("OPEN_COMPETITION_V2_PROVER_MAX_QUEUED", "2"))
    if not 30 <= maximum_seconds <= 3600:
        raise SystemExit("OPEN_COMPETITION_V2_PROVER_MAX_SECONDS must be between 30 and 3600")
    if not 1 <= maximum_queued <= 16:
        raise SystemExit("OPEN_COMPETITION_V2_PROVER_MAX_QUEUED must be between 1 and 16")
    jobs = ProverJobs(root, binaries, maximum_seconds, maximum_queued)
    jobs.resume_pending()
    server = ProverServer(
        (
            os.environ.get("OPEN_COMPETITION_V2_PROVER_BIND", "127.0.0.1"),
            int(os.environ.get("PORT", "9070")),
        ),
        api_key,
        jobs,
    )
    server.serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
