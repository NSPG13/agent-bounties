#!/usr/bin/env python3
"""Fail closed when regression-verifier build or signing sources drift."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path


HEX_DIGEST = re.compile(r"^[0-9a-f]{64}$")
INCLUDE_CALL = re.compile(r"include_(?:str|bytes)!\s*\(")
LITERAL_INCLUDE = re.compile(
    r'include_(?:str|bytes)!\s*\(\s*"([^"\\]*)"\s*,?\s*\)',
    re.MULTILINE,
)
BUILD_ROOTS = ("Cargo.toml", "Cargo.lock", ".cargo", "crates")
OPTIONAL_BUILD_ROOTS = ("rust-toolchain", "rust-toolchain.toml")
RUNTIME_FILES = (
    "scripts/regression_verifier_pipeline.py",
    "scripts/test_regression_verifier_pipeline.py",
    "scripts/regression_verifier_source_guard.py",
    "scripts/test_regression_verifier_source_guard.py",
)


class GuardError(RuntimeError):
    """Raised when a guarded source set cannot be trusted."""


def _compile_time_inputs(root: Path, guarded_files: set[Path]) -> set[Path]:
    """Resolve every literal Rust include outside or inside the guarded roots.

    Rust permits compile-time inputs to live outside a crate directory. Hashing
    only Cargo manifests and crate roots therefore does not bind the executable.
    Reject non-literal include paths because their effective files cannot be
    proven from this source-only guard.
    """

    inputs: set[Path] = set()
    for source in sorted(path for path in guarded_files if path.suffix == ".rs"):
        text = source.read_text(encoding="utf-8")
        calls = INCLUDE_CALL.findall(text)
        literals = LITERAL_INCLUDE.findall(text)
        if len(calls) != len(literals):
            relative = source.relative_to(root).as_posix()
            raise GuardError(f"non-literal or unsupported compile-time include: {relative}")
        for literal in literals:
            try:
                included = (source.parent / literal).resolve(strict=True)
                relative = included.relative_to(root)
            except (OSError, ValueError) as error:
                source_name = source.relative_to(root).as_posix()
                raise GuardError(
                    f"compile-time include escapes or is missing: {source_name}: {literal}"
                ) from error
            cursor = root
            for part in relative.parts:
                cursor /= part
                if cursor.is_symlink():
                    raise GuardError(
                        "compile-time include may not traverse a symlink: "
                        f"{relative.as_posix()}"
                    )
            if not included.is_file():
                raise GuardError(
                    f"compile-time include is not a file: {relative.as_posix()}"
                )
            inputs.add(included)
    return inputs


def _guarded_files(root: Path, scope: str) -> list[Path]:
    if scope == "worker-build":
        candidates: set[Path] = set()
        for relative in BUILD_ROOTS + OPTIONAL_BUILD_ROOTS:
            candidate = root / relative
            if not candidate.exists():
                if relative in OPTIONAL_BUILD_ROOTS:
                    continue
                raise GuardError(f"missing guarded build input: {relative}")
            if candidate.is_symlink():
                raise GuardError(f"guarded build input may not be a symlink: {relative}")
            if candidate.is_file():
                candidates.add(candidate)
                continue
            for child in candidate.rglob("*"):
                if child.is_symlink():
                    raise GuardError(
                        f"guarded build input may not be a symlink: {child.relative_to(root).as_posix()}"
                    )
                if child.is_file():
                    candidates.add(child)
        candidates.update(_compile_time_inputs(root, candidates))
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
