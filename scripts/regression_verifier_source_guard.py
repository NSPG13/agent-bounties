#!/usr/bin/env python3
"""Fail closed when regression-verifier build or signing sources drift."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path


HEX_DIGEST = re.compile(r"^[0-9a-f]{64}$")
BUILD_ROOTS = ("Cargo.toml", "Cargo.lock", ".cargo", "crates")
RUNTIME_FILES = (
    "scripts/regression_verifier_pipeline.py",
    "scripts/test_regression_verifier_pipeline.py",
    "scripts/regression_verifier_source_guard.py",
    "scripts/test_regression_verifier_source_guard.py",
)


class GuardError(RuntimeError):
    """Raised when a guarded source set cannot be trusted."""


def _guarded_files(root: Path, scope: str) -> list[Path]:
    if scope == "worker-build":
        candidates: list[Path] = []
        for relative in BUILD_ROOTS:
            candidate = root / relative
            if not candidate.exists():
                raise GuardError(f"missing guarded build input: {relative}")
            if candidate.is_symlink():
                raise GuardError(f"guarded build input may not be a symlink: {relative}")
            if candidate.is_file():
                candidates.append(candidate)
                continue
            for child in candidate.rglob("*"):
                if child.is_symlink():
                    raise GuardError(
                        f"guarded build input may not be a symlink: {child.relative_to(root).as_posix()}"
                    )
                if child.is_file():
                    candidates.append(child)
        return sorted(candidates, key=lambda path: path.relative_to(root).as_posix())
    if scope == "signing-runtime":
        candidates = []
        for relative in RUNTIME_FILES:
            candidate = root / relative
            if not candidate.is_file() or candidate.is_symlink():
                raise GuardError(f"missing or unsafe guarded signing runtime: {relative}")
            candidates.append(candidate)
        return candidates
    raise GuardError(f"unsupported guard scope: {scope}")


def source_digest(root: Path, scope: str) -> str:
    root = root.resolve()
    digest = hashlib.sha256()
    files = _guarded_files(root, scope)
    if not files:
        raise GuardError(f"guard scope contains no files: {scope}")
    for path in files:
        relative = path.relative_to(root).as_posix().encode("utf-8")
        # GitHub executes the guarded workflows on Linux while maintainers also
        # rehearse them on Windows. Hash canonical LF bytes so checkout newline
        # policy cannot create a false source drift or a Windows-only digest.
        payload = path.read_bytes().replace(b"\r\n", b"\n")
        digest.update(len(relative).to_bytes(8, "big"))
        digest.update(relative)
        digest.update(len(payload).to_bytes(8, "big"))
        digest.update(payload)
    return digest.hexdigest()


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--root", type=Path, default=Path.cwd())
    result.add_argument("--scope", choices=("worker-build", "signing-runtime"), required=True)
    result.add_argument("--expected-sha256")
    result.add_argument("--print-digest", action="store_true")
    return result


def main() -> int:
    args = parser().parse_args()
    try:
        actual = source_digest(args.root, args.scope)
        expected = str(args.expected_sha256 or "").lower()
        if args.print_digest:
            print(actual)
        if expected:
            if not HEX_DIGEST.fullmatch(expected):
                raise GuardError("expected digest must be 64 lowercase hexadecimal characters")
            if actual != expected:
                raise GuardError(
                    f"{args.scope} source digest mismatch: expected {expected}, observed {actual}"
                )
        elif not args.print_digest:
            raise GuardError("provide --expected-sha256 or --print-digest")
    except (GuardError, OSError, ValueError) as error:
        print(f"regression verifier source guard failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
