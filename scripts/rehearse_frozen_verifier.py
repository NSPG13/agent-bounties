#!/usr/bin/env python3
"""Rehearse the unchanged watchdog against its exact historical source fixture.

This is a benchmark regression test, not approval of the current application
for signing. Live runner/signer workflows keep their existing source guards.
"""

from __future__ import annotations

import hashlib
import os
from pathlib import Path, PurePosixPath
import runpy
import shutil
import subprocess
import sys
import tarfile
import tempfile


ROOT = Path(__file__).resolve().parents[1]
REFERENCE_COMMIT = "1998b4ebe57ddbc82ffb72653f0c4b498e74cb42"
REFERENCE_FIXTURE = "scripts/fixtures/regression-verifier-1998b4e.tar.gz"
REFERENCE_FIXTURE_SHA256 = "74162e0c58870d73907360150af37eb8ba6737e41de493e2403dc662793e77ea"
BENCHMARK = "benchmarks/direct-flagship-v1/verifier-settlement-watchdog"
FROZEN_FILES = {
    f"{BENCHMARK}/check.py": "88847fb860825a618827f2116f4b03334bdbb49b26f5dfbcf4a42e09dfc09227",
    f"{BENCHMARK}/rehearse.py": "617b2eff25b1739abbe3fa3df5b8f22815c26c5002a67e893723dbf39290bf64",
    "scripts/regression_verifier_pipeline.py": "3335c8783b2acb42be20b654540d8b63d0073f48ac96774ee57f4d77a29c5328",
    "scripts/test_regression_verifier_pipeline.py": "fe3b7aac688043071c4736b931dcfc16c236a48708a78a149bcaf25775a3b748",
    "scripts/regression_verifier_source_guard.py": "ab8a6491acd5a5b8e93db5ef36d40db6276af8ba658bcd6a52c34d6aeb0be83d",
    "scripts/test_regression_verifier_source_guard.py": "d18ced511f9cd9f266c989dae93ff0088ce38d290f19a6491d6d7b3e6aa108ac",
    ".github/workflows/regression-verifier-runner.yml": "fd76338f5a85968099c184dc30dd8ed8468076522c303a788bbe7173771264e5",
    ".github/workflows/regression-verifier-signer.yml": "6444651670bd84ae700bb339e8ee2a5cece32d8a8841d49731384613100c5fb3",
    ".github/workflows/regression-verifier-signing-reusable.yml": "afe53cdbf70a782033f59a61ec4f12f6d6684fcad0294188b8d1188b91992329",
}
SOURCE_DIGESTS = {
    "worker-build": "f3a66b73cbb03164a4372a13c3b859e0ef8fe8629f7c076be8f016f5950cf9e5",
    "signing-runtime": "6b0c5073097e9612f94e35089285729d0c0f5db580f75b351d19a97f8bffd2f1",
}
MAX_ARCHIVE_BYTES = 64 * 1024 * 1024
MAX_FILES = 4096


def clean_environment() -> dict[str, str]:
    # The historical rehearsal needs Python and loopback HTTP only.
    # Never forward wallet, cloud, database, provider or GitHub credentials.
    allowed = ("PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT", "TEMP", "TMP", "TMPDIR")
    return {
        **{key: os.environ[key] for key in allowed if key in os.environ},
        "PYTHONDONTWRITEBYTECODE": "1",
        "PYTHONNOUSERSITE": "1",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": os.devnull,
        "GIT_TERMINAL_PROMPT": "0",
        "LC_ALL": "C.UTF-8",
    }


def verify_frozen_files(root: Path, *, benchmark_only: bool = False) -> None:
    for relative, expected in FROZEN_FILES.items():
        if benchmark_only and not relative.startswith(BENCHMARK + "/"):
            continue
        path = root / relative
        if path.is_symlink() or not path.is_file() or not path.resolve().is_relative_to(root.resolve()):
            raise ValueError(f"missing or unsafe frozen verifier file: {relative}")
        if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError(f"frozen verifier file changed: {relative}")


def verify_reference(root: Path) -> None:
    verify_frozen_files(root)
    # Verify the guard's bytes before importing it. Use the original guard and
    # original expected hashes, including every resolved Rust compile-time input.
    guard = runpy.run_path(str(root / "scripts/regression_verifier_source_guard.py"))
    for scope, expected in SOURCE_DIGESTS.items():
        if guard["source_digest"](root, scope) != expected:
            raise ValueError(f"frozen reference {scope} source digest mismatch")


def extract_reference(archive: Path, destination: Path) -> None:
    if archive.stat().st_size > MAX_ARCHIVE_BYTES:
        raise ValueError("frozen reference archive exceeds size limit")
    with tarfile.open(archive, mode="r:*") as bundle:
        seen: set[str] = set()
        members = []
        total = 0
        for member in bundle:
            path = PurePosixPath(member.name)
            normalized = member.name.rstrip("/")
            if (
                not normalized or path.is_absolute() or ".." in path.parts
                or "\\" in member.name or ":" in member.name
                or path.as_posix() != normalized
                or not (member.isfile() or member.isdir())
                or normalized.casefold() in seen
            ):
                raise ValueError("unsafe or duplicate path in frozen reference archive")
            seen.add(normalized.casefold())
            total += member.size
            if member.size < 0 or total > MAX_ARCHIVE_BYTES or len(seen) > MAX_FILES:
                raise ValueError("frozen reference expansion exceeds limits")
            members.append(member)
        # Validate the whole archive before writing any member. Links, devices,
        # absolute paths, traversal and case-colliding paths are never extracted.
        for member in members:
            output = destination / member.name
            if member.isdir():
                output.mkdir(parents=True, exist_ok=True)
                continue
            output.parent.mkdir(parents=True, exist_ok=True)
            source = bundle.extractfile(member)
            if source is None:
                raise ValueError("frozen reference member has no bytes")
            with source, output.open("xb") as target:
                shutil.copyfileobj(source, target)


def stage_reference(repository: Path, destination: Path) -> None:
    archive = repository / REFERENCE_FIXTURE
    if archive.is_symlink() or not archive.is_file() or archive.stat().st_size > MAX_ARCHIVE_BYTES:
        raise ValueError("missing or unsafe frozen reference fixture")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != REFERENCE_FIXTURE_SHA256:
        raise ValueError("frozen reference fixture digest mismatch")
    # Bundled exact historical bytes work in shallow and source-only checkouts.
    # No network request, credentials, local branch or checkout filters are used.
    destination.mkdir()
    extract_reference(archive, destination)
    verify_reference(destination)


def main() -> int:
    try:
        # Published benchmark code remains immutable. The historical runtime
        # and its original source guards are verified inside the sealed archive.
        # Current runtime changes use their own reviewed release/source pins;
        # equating them to the old runtime would prevent security repairs.
        verify_frozen_files(ROOT, benchmark_only=True)
        with tempfile.TemporaryDirectory(prefix="frozen-verifier-") as temporary:
            base = Path(temporary)
            reference = base / "reference"
            stage_reference(ROOT, reference)
            print(f"Frozen verifier reference: {REFERENCE_COMMIT}; current app tested separately", flush=True)
            subprocess.run(
                [sys.executable, "-I", "-B", str(reference / BENCHMARK / "rehearse.py")],
                cwd=reference, env=clean_environment(), check=True, timeout=1200,
            )
            verify_reference(reference)
        print("Historical benchmark passed; live signing source restrictions are unchanged.")
    except (OSError, ValueError, subprocess.SubprocessError, tarfile.TarError) as error:
        print(f"Frozen verifier rehearsal failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
