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
PINNED_IMAGE = re.compile(r"^[a-z0-9][a-z0-9._/:@-]{0,446}@sha256:[0-9a-f]{64}$")
CANDIDATE_FILE = re.compile(r"^candidate-[0-9a-f]{64}\.json$")
ATTESTATION_FILE = re.compile(r"^attestation-[0-9a-f]{64}\.json$")
DEFAULT_API = "https://api.agentbounties.app"
MAX_GITHUB_SOURCE_ARCHIVE_BYTES = 256 * 1024 * 1024
MAX_GITHUB_BENCHMARK_ARCHIVE_BYTES = 128 * 1024 * 1024
MAX_GITHUB_SOURCE_ARCHIVE_UNCOMPRESSED_BYTES = 512 * 1024 * 1024
MAX_GITHUB_BENCHMARK_ARCHIVE_UNCOMPRESSED_BYTES = 256 * 1024 * 1024
MAX_GITHUB_ARCHIVE_ENTRIES = 100_000
BASE_MAINNET_CHAIN_ID = 8453
RELAY_GAS_LIMIT = 500_000
RELAY_MAX_FEE_PER_GAS = 500_000_000
RELAY_PRIORITY_FEE_PER_GAS = 1_000_000
CANONICAL_BOUNTY_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
CANONICAL_SETTLEMENT_TOKEN = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
CANONICAL_BOUNTY_RUNTIME = (
    "0x363d3d373d3d3d363d732fa36d2b2327642db3a6cc8cdd91544ad7484eb9"
    "5af43d82803e903d91602b57fd5bf3"
)


class PipelineError(RuntimeError):
    pass


def canonical_json(value: object) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)


def write_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def read_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def manifest_file(root: Path, value: object, pattern: re.Pattern[str], field: str) -> Path:
    name = str(value or "")
    if not pattern.fullmatch(name):
        raise PipelineError(f"{field} is not an exact content-addressed filename")
    return root / name


def content_addressed_name(prefix: str, job_id: str) -> str:
    return f"{prefix}-{hashlib.sha256(job_id.encode()).hexdigest()}.json"


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


def environment_without(*names: str) -> dict[str, str]:
    environment = os.environ.copy()
    for name in names:
        environment.pop(name, None)
    return environment


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


def required_job_signers(job: dict[str, Any]) -> list[str]:
    if job.get("verification_mode") != "signed_quorum":
        raise PipelineError("regression job must use signed verification")
    try:
        threshold = int(job.get("threshold"))
    except (TypeError, ValueError) as error:
        raise PipelineError("regression verifier threshold is invalid") from error
    signers = [
        normalize_address(value, "eligible verifier")
        for value in job.get("eligible_verifiers", [])
    ]
    if threshold not in {1, 2} or len(signers) != threshold:
        raise PipelineError("regression job requires exactly one or two committed verifiers")
    if len(set(signers)) != len(signers):
        raise PipelineError("regression job verifier addresses must be distinct")
    return signers


def selected_jobs(
    jobs: list[dict[str, Any]],
    configured_verifiers: list[str],
    maximum: int,
) -> list[dict[str, Any]]:
    selected = []
    for job in jobs:
        try:
            signers = required_job_signers(job)
        except PipelineError:
            continue
        if signers == configured_verifiers[: len(signers)]:
            selected.append(job)
        if len(selected) == maximum:
            break
    return selected


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
    max_archive_bytes: int = MAX_GITHUB_SOURCE_ARCHIVE_UNCOMPRESSED_BYTES,
) -> None:
    if max_archive_bytes < 1:
        raise PipelineError("GitHub archive uncompressed limit must be positive")
    prefix = None if subdirectory is None else PurePosixPath(subdirectory).parts
    seen: set[str] = set()
    total = 0
    files = 0
    archive_entries = 0
    archive_bytes = 0
    selected_entries = 0
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as bundle:
        for member in bundle:
            archive_entries += 1
            if archive_entries > MAX_GITHUB_ARCHIVE_ENTRIES:
                raise PipelineError("GitHub archive has too many entries")
            if member.size < 0:
                raise PipelineError("GitHub archive contains a negative member size")
            archive_bytes += member.size
            if archive_bytes > max_archive_bytes:
                raise PipelineError("GitHub archive exceeds the uncompressed input limit")
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
            selected_entries += 1
            if selected_entries > max_files * 4 + 100:
                raise PipelineError("GitHub selected snapshot has too many entries")
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


def pull_pinned_image(manifest: dict[str, Any], docker_binary: str) -> None:
    image = str(manifest.get("image", ""))
    platform = str(manifest.get("platform", ""))
    if not PINNED_IMAGE.fullmatch(image):
        raise PipelineError("runner image must be an immutable lowercase OCI digest")
    if platform not in {"linux/amd64", "linux/arm64"}:
        raise PipelineError("runner platform is unsupported")
    run([docker_binary, "pull", "--platform", platform, image])


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
    docker_binary = os.environ.get("REGRESSION_SANDBOX_DOCKER_BINARY", "docker")
    pull_pinned_image(manifest, docker_binary)
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
    source_archive = download_archive(
        source_repo,
        source_commit,
        MAX_GITHUB_SOURCE_ARCHIVE_BYTES,
    )
    benchmark_archive = download_archive(
        benchmark_repo,
        benchmark_commit,
        MAX_GITHUB_BENCHMARK_ARCHIVE_BYTES,
    )
    extract_snapshot(
        source_archive,
        source_dir,
        subdirectory=None if source_subdir == "." else source_subdir,
        max_bytes=source_bytes,
        max_files=source_files,
        max_archive_bytes=MAX_GITHUB_SOURCE_ARCHIVE_UNCOMPRESSED_BYTES,
    )
    extract_snapshot(
        benchmark_archive,
        benchmark_dir,
        subdirectory=benchmark_subdir,
        max_bytes=benchmark_bytes,
        max_files=benchmark_files,
        max_archive_bytes=MAX_GITHUB_BENCHMARK_ARCHIVE_UNCOMPRESSED_BYTES,
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
    environment.setdefault("REGRESSION_SANDBOX_DOCKER_BINARY", docker_binary)
    outcome = json.loads(run([str(worker), "--run-regression", str(request)], env=environment))
    return {
        "schema": CANDIDATE_SCHEMA,
        "job": job,
        "outcome": outcome,
        "runner_revision": os.environ.get("GITHUB_SHA", "local"),
    }


def command_run(args: argparse.Namespace) -> None:
    verifiers = [normalize_address(value, "verifier") for value in args.verifier]
    if len(verifiers) not in {1, 2} or len(set(verifiers)) != len(verifiers):
        raise PipelineError("runner requires one or two distinct verifier addresses")
    jobs = verification_jobs(args.api_base, args.network, verifiers[0])
    selected = selected_jobs(jobs, verifiers, args.max_jobs)
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
    *,
    secret_names: tuple[str, ...] = (),
) -> None:
    if candidate.get("schema") != CANDIDATE_SCHEMA:
        raise PipelineError("candidate schema is invalid")
    request = scratch / "validate.json"
    write_json(
        request,
        {"job": candidate.get("job"), "current_job": current, "outcome": candidate.get("outcome")},
    )
    if run(
        [str(worker.resolve()), "--validate-regression-candidate", str(request)],
        env=environment_without(*secret_names),
    ) != "ok":
        raise PipelineError("worker did not validate the regression candidate")


def checked_hash(value: object, field: str) -> str:
    normalized = str(value or "").lower()
    if not HASH.fullmatch(normalized):
        raise PipelineError(f"{field} must be a bytes32 value")
    return normalized


def cast_keccak(cast: Path, value: str, environment: dict[str, str]) -> str:
    digest = run([str(cast), "keccak", value], env=environment).lower()
    if not HASH.fullmatch(digest):
        raise PipelineError("local keccak command returned an invalid digest")
    return digest


def local_attestation_digest(
    cast: Path,
    current: dict[str, Any],
    signer: str,
    passed: bool,
    response_hash: str,
    deadline: int,
    environment: dict[str, str],
) -> str:
    bounty = normalize_address(current.get("bounty_contract"), "bounty contract")
    bounty_id = checked_hash(current.get("bounty_id"), "bounty id")
    try:
        round_number = int(current.get("round"))
    except (TypeError, ValueError) as error:
        raise PipelineError("round must be a positive integer") from error
    if round_number <= 0 or deadline <= 0:
        raise PipelineError("round and attestation deadline must be positive")
    evidence = current.get("submission_evidence")
    terms = current.get("terms")
    if not isinstance(evidence, dict) or not isinstance(terms, dict):
        raise PipelineError("current canonical attestation preimages are unavailable")
    submission_hash = checked_hash(evidence.get("artifact_hash"), "submission hash")
    evidence_hash = checked_hash(evidence.get("evidence_hash"), "evidence hash")
    policy_hash = checked_hash(terms.get("policy_hash"), "policy hash")
    response_hash = checked_hash(response_hash, "response hash")

    domain_type_hash = cast_keccak(
        cast,
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        environment,
    )
    name_hash = cast_keccak(cast, "Agent Bounties", environment)
    version_hash = cast_keccak(cast, "1", environment)
    domain_encoding = run(
        [
            str(cast),
            "abi-encode",
            "f(bytes32,bytes32,bytes32,uint256,address)",
            domain_type_hash,
            name_hash,
            version_hash,
            "8453",
            bounty,
        ],
        env=environment,
    ).lower()
    domain_separator = cast_keccak(cast, domain_encoding, environment)

    attestation_type_hash = cast_keccak(
        cast,
        "VerificationAttestation(address bounty,bytes32 bountyId,uint64 round,address verifier,bytes32 submissionHash,bytes32 evidenceHash,bytes32 policyHash,bool passed,bytes32 responseHash,uint256 deadline)",
        environment,
    )
    struct_encoding = run(
        [
            str(cast),
            "abi-encode",
            "f(bytes32,address,bytes32,uint64,address,bytes32,bytes32,bytes32,bool,bytes32,uint256)",
            attestation_type_hash,
            bounty,
            bounty_id,
            str(round_number),
            signer,
            submission_hash,
            evidence_hash,
            policy_hash,
            str(passed).lower(),
            response_hash,
            str(deadline),
        ],
        env=environment,
    ).lower()
    struct_hash = cast_keccak(cast, struct_encoding, environment)
    return cast_keccak(
        cast,
        "0x1901" + domain_separator.removeprefix("0x") + struct_hash.removeprefix("0x"),
        environment,
    )


def attestation_digest(
    cast: Path,
    rpc_url: str,
    current: dict[str, Any],
    signer: str,
    passed: bool,
    response_hash: str,
    deadline: int,
    environment: dict[str, str],
) -> str:
    bounty = normalize_address(current.get("bounty_contract"), "bounty contract")
    chain_id_text = run(
        [str(cast), "chain-id", "--rpc-url", rpc_url], env=environment
    ).strip().lower()
    try:
        chain_id = int(chain_id_text, 0)
    except ValueError as error:
        raise PipelineError("RPC returned an invalid chain id") from error
    if chain_id != 8453:
        raise PipelineError("RPC is not Base mainnet")
    canonical_bounty_rpc_preflight(cast, rpc_url, bounty, environment)
    local_digest = local_attestation_digest(
        cast,
        current,
        signer,
        passed,
        response_hash,
        deadline,
        environment,
    )
    remote_digest = run(
        [
            str(cast),
            "call",
            "--rpc-url",
            rpc_url,
            bounty,
            "attestationDigest(address,bool,bytes32,uint256)(bytes32)",
            signer,
            str(passed).lower(),
            response_hash,
            str(deadline),
        ],
        env=environment,
    ).lower()
    if not HASH.fullmatch(remote_digest) or remote_digest != local_digest:
        raise PipelineError("RPC attestation digest differs from the local EIP-712 digest")
    return local_digest


def canonical_bounty_rpc_preflight(
    cast: Path,
    rpc_url: str,
    bounty: str,
    environment: dict[str, str],
) -> None:
    code = run(
        [str(cast), "code", "--rpc-url", rpc_url, "--block", "safe", bounty],
        env=environment,
    ).strip().lower()
    if code != CANONICAL_BOUNTY_RUNTIME:
        raise PipelineError("RPC bounty runtime is not the precommitted canonical clone")
    factory = normalize_address(
        run(
            [
                str(cast),
                "call",
                "--rpc-url",
                rpc_url,
                "--block",
                "safe",
                bounty,
                "factory()(address)",
            ],
            env=environment,
        ).strip(),
        "RPC bounty factory",
    )
    if factory != CANONICAL_BOUNTY_FACTORY:
        raise PipelineError("RPC bounty factory is not the canonical factory")
    settlement_token = normalize_address(
        run(
            [
                str(cast),
                "call",
                "--rpc-url",
                rpc_url,
                "--block",
                "safe",
                bounty,
                "settlementToken()(address)",
            ],
            env=environment,
        ).strip(),
        "RPC settlement token",
    )
    if settlement_token != CANONICAL_SETTLEMENT_TOKEN:
        raise PipelineError("RPC bounty settlement token is not canonical Base USDC")


def relay_rpc_preflight(
    cast: Path,
    rpc_url: str,
    bounty: str,
    expected_keeper: str,
    attestations: str,
    environment: dict[str, str],
) -> int:
    chain_id_text = run(
        [str(cast), "chain-id", "--rpc-url", rpc_url], env=environment
    ).strip().lower()
    try:
        chain_id = int(chain_id_text, 0)
    except ValueError as error:
        raise PipelineError("relay RPC returned an invalid chain id") from error
    if chain_id != BASE_MAINNET_CHAIN_ID:
        raise PipelineError("relay RPC is not Base mainnet")
    canonical_bounty_rpc_preflight(cast, rpc_url, bounty, environment)

    call = [
        str(cast),
        "call",
        "--rpc-url",
        rpc_url,
        "--from",
        expected_keeper,
        bounty,
        "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])",
        attestations,
    ]
    run(call, env=environment)
    estimate_text = run(
        [
            str(cast),
            "estimate",
            "--rpc-url",
            rpc_url,
            "--from",
            expected_keeper,
            bounty,
            "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])",
            attestations,
        ],
        env=environment,
    ).strip().lower()
    try:
        estimated_gas = int(estimate_text, 0)
    except ValueError as error:
        raise PipelineError("relay RPC returned an invalid gas estimate") from error
    if estimated_gas <= 0 or estimated_gas > RELAY_GAS_LIMIT:
        raise PipelineError("relay gas estimate exceeds the precommitted limit")

    observed_nonces: dict[str, int] = {}
    for block_tag in ("latest", "pending"):
        nonce_text = run(
            [
                str(cast),
                "nonce",
                "--rpc-url",
                rpc_url,
                "--block",
                block_tag,
                expected_keeper,
            ],
            env=environment,
        ).strip().lower()
        try:
            nonce = int(nonce_text, 0)
        except ValueError as error:
            raise PipelineError("relay RPC returned an invalid keeper nonce") from error
        if nonce < 0:
            raise PipelineError("relay RPC returned a negative keeper nonce")
        observed_nonces[block_tag] = nonce
    if observed_nonces["pending"] != observed_nonces["latest"]:
        raise PipelineError(
            "keeper has another pending transaction; retry after it confirms or drops"
        )
    return observed_nonces["latest"]


def command_sign(args: argparse.Namespace) -> None:
    if not os.environ.get(args.private_key_env, "").strip():
        raise PipelineError(f"{args.private_key_env} is required")
    secret_free_environment = environment_without(args.private_key_env)
    expected = normalize_address(args.expected_signer, "expected signer")
    manifest = read_json(args.candidates / "manifest.json")
    if manifest.get("schema") != MANIFEST_SCHEMA:
        raise PipelineError("candidate manifest schema is invalid")
    entries = manifest.get("candidates")
    if not isinstance(entries, list) or not all(isinstance(entry, dict) for entry in entries):
        raise PipelineError("candidate manifest entries are invalid")
    args.output.mkdir(parents=True, exist_ok=True)
    signed = []
    seen_jobs: set[str] = set()
    seen_files: set[str] = set()
    for entry in entries:
        candidate_path = manifest_file(
            args.candidates,
            entry.get("file"),
            CANDIDATE_FILE,
            "candidate manifest file",
        )
        job_id = str(entry.get("job_id", ""))
        filename = str(entry.get("file", ""))
        if not job_id or job_id in seen_jobs or filename in seen_files:
            raise PipelineError("candidate manifest contains duplicate or empty entries")
        if filename != content_addressed_name("candidate", job_id):
            raise PipelineError("candidate filename is not bound to its job")
        seen_jobs.add(job_id)
        seen_files.add(filename)
        candidate = read_json(candidate_path)
        job = candidate.get("job", {})
        if str(job.get("job_id", "")) != job_id:
            raise PipelineError("candidate manifest job does not match its file")
        if expected not in required_job_signers(job):
            continue
        current = current_job(args.api_base, args.network, expected, job_id)
        with tempfile.TemporaryDirectory(prefix="agent-bounties-sign-") as temporary:
            validate_candidate(
                args.worker,
                candidate,
                current,
                Path(temporary),
                secret_names=(args.private_key_env,),
            )
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
        digest = attestation_digest(
            args.cast,
            args.rpc_url,
            current,
            expected,
            passed,
            response_hash,
            deadline,
            secret_free_environment,
        ).lower()
        # No child process receives the private key until the canonical job,
        # candidate, Base chain, contract code, and independently computed
        # EIP-712 digest have all passed validation.
        key = os.environ.get(args.private_key_env, "").strip()
        signer = normalize_address(
            run(
                [str(args.cast), "wallet", "address", "--private-key", key],
                env=secret_free_environment,
            ),
            "signer",
        )
        if signer != expected:
            raise PipelineError("signer private key does not match the expected public address")
        signature = run(
            [
                str(args.cast),
                "wallet",
                "sign",
                "--no-hash",
                "--private-key",
                key,
                digest,
            ],
            env=secret_free_environment,
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
        {"schema": ATTESTATION_SCHEMA, "signer": expected, "attestations": signed},
    )


def command_relay(args: argparse.Namespace) -> None:
    if not os.environ.get(args.keeper_key_env, "").strip():
        raise PipelineError(f"{args.keeper_key_env} is required")
    secret_free_environment = environment_without(args.keeper_key_env)
    expected_keeper = normalize_address(args.expected_keeper, "expected keeper")
    configured = [normalize_address(value, "verifier") for value in args.verifier]
    if len(configured) not in {1, 2} or len(set(configured)) != len(configured):
        raise PipelineError("relay requires one or two distinct configured verifiers")
    candidate_manifest = read_json(args.candidates / "manifest.json")
    manifests = [read_json(path / "manifest.json") for path in args.attestations]
    by_signer: dict[str, dict[str, str]] = {}
    for path, manifest in zip(args.attestations, manifests, strict=True):
        signer = normalize_address(manifest.get("signer"), "attestation signer")
        if signer in by_signer:
            raise PipelineError("duplicate attestation signer")
        entries = manifest.get("attestations")
        if not isinstance(entries, list) or not all(isinstance(entry, dict) for entry in entries):
            raise PipelineError("attestation manifest entries are invalid")
        signer_entries: dict[str, str] = {}
        for entry in entries:
            job_id = str(entry.get("job_id", ""))
            attestation_path = manifest_file(
                path,
                entry.get("file"),
                ATTESTATION_FILE,
                "attestation manifest file",
            )
            if not job_id or job_id in signer_entries:
                raise PipelineError("attestation manifest contains duplicate or empty jobs")
            if str(entry.get("file", "")) != content_addressed_name("attestation", job_id):
                raise PipelineError("attestation filename is not bound to its job")
            signer_entries[job_id] = str(attestation_path)
        by_signer[signer] = signer_entries
    if set(by_signer) != set(configured):
        raise PipelineError("attestation artifacts do not contain every configured verifier")

    candidate_entries = candidate_manifest.get("candidates")
    if not isinstance(candidate_entries, list) or not all(
        isinstance(entry, dict) for entry in candidate_entries
    ):
        raise PipelineError("candidate manifest entries are invalid")
    seen_candidate_jobs: set[str] = set()
    seen_candidate_files: set[str] = set()
    relay_plans: list[dict[str, str]] = []
    for entry in candidate_entries:
        job_id = str(entry.get("job_id", ""))
        candidate_path = manifest_file(
            args.candidates,
            entry.get("file"),
            CANDIDATE_FILE,
            "candidate manifest file",
        )
        filename = str(entry.get("file", ""))
        if not job_id or job_id in seen_candidate_jobs or filename in seen_candidate_files:
            raise PipelineError("candidate manifest contains duplicate or empty entries")
        if filename != content_addressed_name("candidate", job_id):
            raise PipelineError("candidate filename is not bound to its job")
        seen_candidate_jobs.add(job_id)
        seen_candidate_files.add(filename)
        candidate = read_json(candidate_path)
        if str(candidate.get("job", {}).get("job_id", "")) != job_id:
            raise PipelineError("candidate manifest job does not match its file")
        expected = required_job_signers(candidate.get("job", {}))
        if expected != configured[: len(expected)]:
            raise PipelineError("candidate verifier set is not supported by this relay")
        current = current_job(args.api_base, args.network, expected[0], job_id)
        if required_job_signers(current) != expected:
            raise PipelineError("current canonical verifier set differs from the candidate")
        with tempfile.TemporaryDirectory(prefix="agent-bounties-relay-") as temporary:
            validate_candidate(
                args.worker,
                candidate,
                current,
                Path(temporary),
                secret_names=(args.keeper_key_env,),
            )
        try:
            attestations = [
                read_json(Path(by_signer[signer][job_id])) for signer in sorted(expected)
            ]
        except KeyError as error:
            raise PipelineError("required verifier attestation is missing") from error
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
        relay_plans.append(
            {
                "job_id": job_id,
                "bounty": normalize_address(current.get("bounty_contract"), "bounty contract"),
                "attestations": f"[{tuple_values}]",
            }
        )

    if not relay_plans:
        return

    observed_nonce: int | None = None
    for plan in relay_plans:
        nonce = relay_rpc_preflight(
            args.cast,
            args.rpc_url,
            plan["bounty"],
            expected_keeper,
            plan["attestations"],
            secret_free_environment,
        )
        if observed_nonce is None:
            observed_nonce = nonce
        elif nonce != observed_nonce:
            raise PipelineError("relay RPC keeper nonce changed during the pre-write preflight")

    # No child process receives the keeper key until every candidate, current
    # canonical job, exact call, chain, contract, gas bound, and starting nonce
    # has passed validation.
    keeper = os.environ.get(args.keeper_key_env, "").strip()
    keeper_address = normalize_address(
        run(
            [
                str(args.cast),
                "wallet",
                "address",
                "--private-key",
                keeper,
            ],
            env=secret_free_environment,
        ),
        "keeper",
    )
    if keeper_address != expected_keeper:
        raise PipelineError("keeper private key does not match the expected public address")

    for offset, plan in enumerate(relay_plans):
        transaction = run(
            [
                str(args.cast),
                "send",
                "--json",
                "--rpc-url",
                args.rpc_url,
                "--chain",
                str(BASE_MAINNET_CHAIN_ID),
                "--nonce",
                str((observed_nonce or 0) + offset),
                "--gas-limit",
                str(RELAY_GAS_LIMIT),
                "--gas-price",
                str(RELAY_MAX_FEE_PER_GAS),
                "--priority-gas-price",
                str(RELAY_PRIORITY_FEE_PER_GAS),
                "--private-key",
                keeper,
                plan["bounty"],
                "settleWithAttestations((address,bool,bytes32,uint256,bytes)[])",
                plan["attestations"],
            ],
            env=secret_free_environment,
        )
        receipt = json.loads(transaction)
        transaction_hash = str(receipt.get("transactionHash", "")).lower()
        if not HASH.fullmatch(transaction_hash) or str(receipt.get("status", "")) not in {"0x1", "1"}:
            raise PipelineError("attestation relay did not return a successful receipt")
        print(f"relayed {plan['job_id']}: {transaction_hash}")


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
    relay_parser.add_argument("--expected-keeper", required=True)
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
