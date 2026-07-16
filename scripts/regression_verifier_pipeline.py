#!/usr/bin/env python3
"""Run, validate, sign, and relay sandboxed-regression verifier jobs."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path, PurePosixPath
from typing import Any


CANDIDATE_SCHEMA = "agent-bounties/regression-candidate-v1"
ATTESTATION_SCHEMA = "agent-bounties/regression-attestation-v1"
MANIFEST_SCHEMA = "agent-bounties/regression-candidate-manifest-v1"
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
SHA = re.compile(r"^[0-9a-f]{40}$")
REPOSITORY = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
DEFAULT_API = "https://agent-bounties-api.onrender.com"


class PipelineError(RuntimeError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def normalize_address(value: object, field: str) -> str:
    normalized = str(value or "").strip().lower()
    if not ADDRESS.fullmatch(normalized):
        raise PipelineError(f"{field} must be a lowercase EVM address")
    return normalized


def run(command: list[str], *, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        command,
        check=False,
        capture_output=True,
        text=True,
        env=env,
        timeout=900,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()[:800]
        raise PipelineError(f"command failed closed: {detail}")
    return completed.stdout.strip()


def fetch_json(url: str, timeout: float = 30) -> Any:
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/json",
            "User-Agent": "agent-bounties-regression-verifier/1",
        },
    )
    with urllib.request.urlopen(request, timeout=timeout) as response:
        if response.status != 200:
            raise PipelineError(f"verification feed returned HTTP {response.status}")
        return json.loads(response.read().decode("utf-8"))


def verification_jobs(api_base: str, network: str, verifier: str) -> list[dict[str, Any]]:
    query = urllib.parse.urlencode({"network": network, "verifier": verifier})
    value = fetch_json(
        f"{api_base.rstrip('/')}/v1/base/autonomous-bounties/verification-jobs?{query}"
    )
    if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
        raise PipelineError("verification feed must be an array of jobs")
    return value


def parse_github_commit_url(value: object) -> tuple[str, str]:
    try:
        parsed = urllib.parse.urlparse(str(value))
    except ValueError as error:
        raise PipelineError("artifact reference is not a valid URL") from error
    parts = [part for part in parsed.path.split("/") if part]
    if (
        parsed.scheme != "https"
        or parsed.hostname != "github.com"
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or len(parts) != 4
        or parts[2] != "commit"
    ):
        raise PipelineError("artifact reference must be an exact public GitHub commit URL")
    repository = f"{parts[0]}/{parts[1]}"
    commit = parts[3].lower()
    if not REPOSITORY.fullmatch(repository) or not SHA.fullmatch(commit):
        raise PipelineError("artifact reference repository or commit is invalid")
    return repository, commit


def benchmark_source(job: dict[str, Any]) -> tuple[str, str, str]:
    source = (
        job.get("terms", {})
        .get("document", {})
        .get("benchmark", {})
        .get("source")
    )
    if not isinstance(source, dict) or set(source) != {
        "kind",
        "repository",
        "commit",
        "subdirectory",
    }:
        raise PipelineError("benchmark source must use the exact github_commit schema")
    repository = str(source.get("repository", ""))
    commit = str(source.get("commit", "")).lower()
    subdirectory = str(source.get("subdirectory", ""))
    if source.get("kind") != "github_commit" or not REPOSITORY.fullmatch(repository):
        raise PipelineError("benchmark repository is invalid")
    if not SHA.fullmatch(commit):
        raise PipelineError("benchmark commit must be a full Git SHA")
    validate_subdirectory(subdirectory)
    return repository, commit, subdirectory


def validate_subdirectory(value: str) -> None:
    path = PurePosixPath(value)
    if (
        not value
        or value.startswith("/")
        or value.endswith("/")
        or "\\" in value
        or any(part in {"", ".", ".."} for part in path.parts)
    ):
        raise PipelineError("snapshot subdirectory must be a normalized relative path")


def download_archive(repository: str, commit: str, compressed_limit: int) -> bytes:
    url = f"https://codeload.github.com/{repository}/tar.gz/{commit}"
    request = urllib.request.Request(url, headers={"User-Agent": "agent-bounties-regression-verifier/1"})
    body = bytearray()
    with urllib.request.urlopen(request, timeout=60) as response:
        if response.status != 200:
            raise PipelineError(f"GitHub archive returned HTTP {response.status}")
        while True:
            chunk = response.read(64 * 1024)
            if not chunk:
                break
            body.extend(chunk)
            if len(body) > compressed_limit:
                raise PipelineError("GitHub archive exceeds the compressed input limit")
    return bytes(body)


def extract_snapshot(
    archive: bytes,
    destination: Path,
    *,
    subdirectory: str | None,
    max_bytes: int,
    max_files: int,
) -> None:
    prefix = None if subdirectory is None else PurePosixPath(subdirectory).parts
    seen: set[str] = set()
    total = 0
    files = 0
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        members = bundle.getmembers()
        if len(members) > max_files * 4 + 100:
            raise PipelineError("GitHub archive has too many entries")
        for member in members:
            path = PurePosixPath(member.name)
            parts = path.parts
            if not parts or path.is_absolute() or any(part in {"", ".", ".."} for part in parts):
                raise PipelineError("GitHub archive contains an unsafe path")
            relative = parts[1:]
            if prefix is not None:
                if relative[: len(prefix)] != prefix:
                    continue
                relative = relative[len(prefix) :]
            if not relative:
                continue
            relative_name = "/".join(relative)
            if relative_name in seen:
                raise PipelineError("GitHub archive contains duplicate paths")
            seen.add(relative_name)
            target = destination.joinpath(*relative)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise PipelineError("GitHub archive contains links or special files")
            files += 1
            total += member.size
            if files > max_files or total > max_bytes:
                raise PipelineError("GitHub snapshot exceeds committed limits")
            target.parent.mkdir(parents=True, exist_ok=True)
            source = bundle.extractfile(member)
            if source is None:
                raise PipelineError("GitHub archive file is unreadable")
            with target.open("wb") as output:
                shutil.copyfileobj(source, output, 64 * 1024)
            target.chmod(0o555 if member.mode & 0o111 else 0o444)
    if files == 0:
        raise PipelineError("GitHub snapshot contains no regular files")


def runner_manifest(job: dict[str, Any]) -> dict[str, Any]:
    value = (
        job.get("terms", {})
        .get("document", {})
        .get("benchmark", {})
        .get("runner_manifest")
    )
    if not isinstance(value, dict):
        raise PipelineError("runner manifest is unavailable")
    for field in (
        "max_source_bytes",
        "max_source_files",
        "max_benchmark_bytes",
        "max_benchmark_files",
        "benchmark_digest",
    ):
        if field not in value:
            raise PipelineError(f"runner manifest is missing {field}")
    return value


def stage(
    worker: Path,
    kind: str,
    source: Path,
    staging: Path,
    max_bytes: int,
    max_files: int,
) -> dict[str, Any]:
    value = json.loads(
        run(
            [
                str(worker),
                "--stage-regression-input",
                kind,
                str(source),
                str(staging),
                str(max_bytes),
                str(max_files),
            ]
        )
    )
    if not isinstance(value, dict) or not str(value.get("snapshot", {}).get("digest", "")).startswith(
        "sha256:"
    ):
        raise PipelineError("worker returned invalid staging evidence")
    return value


def run_job(worker: Path, staging: Path, job: dict[str, Any], scratch: Path) -> dict[str, Any]:
    manifest = runner_manifest(job)
    source_repo, source_commit = parse_github_commit_url(
        job.get("submission_evidence", {}).get("artifact_reference")
    )
    source_subdir = str(
        job.get("submission_evidence", {}).get("evidence", {}).get("source_subdirectory", ".")
    )
    if source_subdir != ".":
        validate_subdirectory(source_subdir)
    benchmark_repo, benchmark_commit, benchmark_subdir = benchmark_source(job)

    source_dir = scratch / "source"
    benchmark_dir = scratch / "benchmark"
    source_dir.mkdir()
    benchmark_dir.mkdir()
    source_bytes = int(manifest["max_source_bytes"])
    source_files = int(manifest["max_source_files"])
    benchmark_bytes = int(manifest["max_benchmark_bytes"])
    benchmark_files = int(manifest["max_benchmark_files"])
    source_archive = download_archive(source_repo, source_commit, min(source_bytes, 256 * 1024 * 1024))
    benchmark_archive = download_archive(
        benchmark_repo, benchmark_commit, min(benchmark_bytes, 128 * 1024 * 1024)
    )
    extract_snapshot(
        source_archive,
        source_dir,
        subdirectory=None if source_subdir == "." else source_subdir,
        max_bytes=source_bytes,
        max_files=source_files,
    )
    extract_snapshot(
        benchmark_archive,
        benchmark_dir,
        subdirectory=benchmark_subdir,
        max_bytes=benchmark_bytes,
        max_files=benchmark_files,
    )
    staged_source = stage(worker, "source", source_dir, staging, source_bytes, source_files)
    staged_benchmark = stage(
        worker, "benchmark", benchmark_dir, staging, benchmark_bytes, benchmark_files
    )
    expected_source = str(
        job.get("submission_evidence", {}).get("evidence", {}).get("source_snapshot_digest", "")
    )
    if staged_source["snapshot"]["digest"] != expected_source:
        raise PipelineError("downloaded source does not match submission evidence")
    if staged_benchmark["snapshot"]["digest"] != manifest["benchmark_digest"]:
        raise PipelineError("downloaded benchmark does not match immutable terms")

    request = scratch / "request.json"
    write_json(request, {"job": job})
    environment = dict(os.environ)
    environment["REGRESSION_SANDBOX_STAGING_ROOT"] = str(staging)
    environment.setdefault("REGRESSION_SANDBOX_DOCKER_BINARY", "docker")
    outcome = json.loads(run([str(worker), "--run-regression", str(request)], env=environment))
    return {
        "schema": CANDIDATE_SCHEMA,
        "job": job,
        "outcome": outcome,
        "runner_revision": os.environ.get("GITHUB_SHA", "local"),
    }


def command_run(args: argparse.Namespace) -> None:
    verifiers = [normalize_address(value, "verifier") for value in args.verifier]
    if len(set(verifiers)) != 2:
        raise PipelineError("runner requires exactly two distinct verifier addresses")
    jobs = verification_jobs(args.api_base, args.network, verifiers[0])
    selected = [
        job
        for job in jobs
        if [str(value).lower() for value in job.get("eligible_verifiers", [])] == verifiers
        and job.get("threshold") == 2
        and job.get("verification_mode") == "signed_quorum"
    ][: args.max_jobs]
    args.output.mkdir(parents=True, exist_ok=True)
    candidates = []
    for job in selected:
        job_id = str(job.get("job_id", ""))
        if not re.fullmatch(r"[A-Za-z0-9_.:-]{1,200}", job_id):
            raise PipelineError("verification job id is invalid")
        with tempfile.TemporaryDirectory(prefix="agent-bounties-regression-") as temporary:
            candidate = run_job(
                args.worker.resolve(), args.staging.resolve(), job, Path(temporary)
            )
        name = f"candidate-{hashlib.sha256(job_id.encode()).hexdigest()}.json"
        write_json(args.output / name, candidate)
        candidates.append({"job_id": job_id, "file": name})
    write_json(
        args.output / "manifest.json",
        {"schema": MANIFEST_SCHEMA, "network": args.network, "candidates": candidates},
    )


def current_job(api_base: str, network: str, verifier: str, job_id: str) -> dict[str, Any]:
    matches = [
        job
        for job in verification_jobs(api_base, network, verifier)
        if job.get("job_id") == job_id
    ]
    if len(matches) != 1:
        raise PipelineError("candidate does not have exactly one current canonical job")
    return matches[0]


def validate_candidate(
    worker: Path,
    candidate: dict[str, Any],
    current: dict[str, Any],
    scratch: Path,
) -> None:
    if candidate.get("schema") != CANDIDATE_SCHEMA:
        raise PipelineError("candidate schema is invalid")
    request = scratch / "validate.json"
    write_json(
        request,
        {"job": candidate.get("job"), "current_job": current, "outcome": candidate.get("outcome")},
    )
    if run([str(worker.resolve()), "--validate-regression-candidate", str(request)]) != "ok":
        raise PipelineError("worker did not validate the regression candidate")


def command_sign(args: argparse.Namespace) -> None:
    key = os.environ.get(args.private_key_env, "").strip()
    if not key:
        raise PipelineError(f"{args.private_key_env} is required")
    signer = normalize_address(
        run([str(args.cast), "wallet", "address", "--private-key", key]), "signer"
    )
    expected = normalize_address(args.expected_signer, "expected signer")
    if signer != expected:
        raise PipelineError("signer private key does not match the expected public address")
    manifest = read_json(args.candidates / "manifest.json")
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise PipelineError("candidate manifest schema is invalid")
    args.output.mkdir(parents=True, exist_ok=True)
    signed = []
    for entry in manifest.get("candidates", []):
        candidate = read_json(args.candidates / entry["file"])
        job = candidate.get("job", {})
        job_id = str(job.get("job_id", ""))
        current = current_job(args.api_base, args.network, signer, job_id)
        with tempfile.TemporaryDirectory(prefix="agent-bounties-sign-") as temporary:
            validate_candidate(args.worker, candidate, current, Path(temporary))
        expiry = int(current["verification_expires_at"])
        now = int(time.time())
        deadline = min(now + 900, expiry)
        if deadline <= now + 120:
            raise PipelineError("verification deadline is too close to sign safely")
        outcome = candidate["outcome"]
        response_hash = str(outcome.get("response_hash", "")).lower()
        if not HASH.fullmatch(response_hash):
            raise PipelineError("candidate response hash is invalid")
        passed = outcome.get("verdict") == "passed"
        bounty = normalize_address(current.get("bounty_contract"), "bounty contract")
        digest = run(
            [
                str(args.cast),
                "call",
                "--rpc-url",
                args.rpc_url,
                bounty,
                "attestationDigest(address,bool,bytes32,uint256)(bytes32)",
                signer,
                str(passed).lower(),
                response_hash,
                str(deadline),
            ]
        ).lower()
        if not HASH.fullmatch(digest):
            raise PipelineError("contract returned an invalid attestation digest")
        signature = run(
            [
                str(args.cast),
                "wallet",
                "sign",
                "--no-hash",
                "--private-key",
                key,
                digest,
            ]
        ).lower()
        if not re.fullmatch(r"0x[0-9a-f]{130}", signature):
            raise PipelineError("signer returned an invalid signature")
        name = f"attestation-{hashlib.sha256(job_id.encode()).hexdigest()}.json"
        write_json(
            args.output / name,
            {
                "schema": ATTESTATION_SCHEMA,
                "job_id": job_id,
                "bounty_contract": bounty,
                "verifier": signer,
                "passed": passed,
                "response_hash": response_hash,
                "deadline": deadline,
                "signature": signature,
                "candidate_file": entry["file"],
            },
        )
        signed.append({"job_id": job_id, "file": name})
    write_json(
        args.output / "manifest.json",
        {"schema": ATTESTATION_SCHEMA, "signer": signer, "attestations": signed},
    )


def command_relay(args: argparse.Namespace) -> None:
    keeper = os.environ.get(args.keeper_key_env, "").strip()
    if not keeper:
        raise PipelineError(f"{args.keeper_key_env} is required")
    expected = {
        normalize_address(args.verifier[0], "verifier"),
        normalize_address(args.verifier[1], "verifier"),
    }
    candidate_manifest = read_json(args.candidates / "manifest.json")
    manifests = [read_json(path / "manifest.json") for path in args.attestations]
    by_signer: dict[str, dict[str, str]] = {}
    for path, manifest in zip(args.attestations, manifests, strict=True):
        signer = normalize_address(manifest.get("signer"), "attestation signer")
        if signer in by_signer:
            raise PipelineError("duplicate attestation signer")
        by_signer[signer] = {entry["job_id"]: str(path / entry["file"]) for entry in manifest["attestations"]}
    if set(by_signer) != expected:
        raise PipelineError("attestation artifacts do not contain the exact verifier set")

    for entry in candidate_manifest.get("candidates", []):
        job_id = entry["job_id"]
        candidate = read_json(args.candidates / entry["file"])
        current = current_job(args.api_base, args.network, sorted(expected)[0], job_id)
        with tempfile.TemporaryDirectory(prefix="agent-bounties-relay-") as temporary:
            validate_candidate(args.worker, candidate, current, Path(temporary))
        attestations = [read_json(Path(by_signer[signer][job_id])) for signer in sorted(expected)]
        first = attestations[0]
        for attestation in attestations:
            if (
                attestation.get("schema") != ATTESTATION_SCHEMA
                or attestation.get("job_id") != job_id
                or attestation.get("bounty_contract") != current.get("bounty_contract").lower()
                or attestation.get("passed") != first.get("passed")
                or attestation.get("response_hash") != first.get("response_hash")
            ):
                raise PipelineError("attestation artifacts disagree on canonical scope or verdict")
        tuple_values = ",".join(
            f"({item['verifier']},{str(item['passed']).lower()},{item['response_hash']},{item['deadline']},{item['signature']})"
            for item in attestations
        )
        transaction = run(
            [
                str(args.cast),
                "send",
                "--json",
                "--rpc-url",
                args.rpc_url,
                "--private-key",
                keeper,
                current["bounty_contract"],
                "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])",
                f"[{tuple_values}]",
            ]
        )
        receipt = json.loads(transaction)
        transaction_hash = str(receipt.get("transactionHash", "")).lower()
        if not HASH.fullmatch(transaction_hash) or str(receipt.get("status", "")) not in {"0x1", "1"}:
            raise PipelineError("attestation relay did not return a successful receipt")
        print(f"relayed {job_id}: {transaction_hash}")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    subcommands = root.add_subparsers(dest="command", required=True)

    run_parser = subcommands.add_parser("run")
    run_parser.add_argument("--api-base", default=DEFAULT_API)
    run_parser.add_argument("--network", default="base-mainnet")
    run_parser.add_argument("--verifier", action="append", required=True)
    run_parser.add_argument("--worker", type=Path, required=True)
    run_parser.add_argument("--staging", type=Path, required=True)
    run_parser.add_argument("--output", type=Path, required=True)
    run_parser.add_argument("--max-jobs", type=int, default=5)
    run_parser.set_defaults(handler=command_run)

    sign_parser = subcommands.add_parser("sign")
    sign_parser.add_argument("--api-base", default=DEFAULT_API)
    sign_parser.add_argument("--network", default="base-mainnet")
    sign_parser.add_argument("--rpc-url", required=True)
    sign_parser.add_argument("--candidates", type=Path, required=True)
    sign_parser.add_argument("--output", type=Path, required=True)
    sign_parser.add_argument("--worker", type=Path, required=True)
    sign_parser.add_argument("--cast", type=Path, default=Path("cast"))
    sign_parser.add_argument("--private-key-env", required=True)
    sign_parser.add_argument("--expected-signer", required=True)
    sign_parser.set_defaults(handler=command_sign)

    relay_parser = subcommands.add_parser("relay")
    relay_parser.add_argument("--api-base", default=DEFAULT_API)
    relay_parser.add_argument("--network", default="base-mainnet")
    relay_parser.add_argument("--rpc-url", required=True)
    relay_parser.add_argument("--candidates", type=Path, required=True)
    relay_parser.add_argument("--attestations", type=Path, action="append", required=True)
    relay_parser.add_argument("--verifier", action="append", required=True)
    relay_parser.add_argument("--worker", type=Path, required=True)
    relay_parser.add_argument("--cast", type=Path, default=Path("cast"))
    relay_parser.add_argument("--keeper-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    relay_parser.set_defaults(handler=command_relay)
    return root


def main() -> int:
    try:
        args = parser().parse_args()
        args.handler(args)
    except (PipelineError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"regression verifier pipeline failed: {error}", file=os.sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
