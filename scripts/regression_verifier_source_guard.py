#!/usr/bin/env python3
"""Fail closed when regression-verifier build or signing sources drift."""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
from pathlib import Path


HEX_DIGEST = re.compile(r"^[0-9a-f]{64}$")
KEEPER_SECRET = re.compile(
    r"\$\{\{\s*secrets(?:\.BASE_KEEPER_PRIVATE_KEY|\[['\"]BASE_KEEPER_PRIVATE_KEY['\"]\])\s*\}\}"
)
SHARED_KEEPER_CONCURRENCY = "agent-bounties-shared-base-keeper"
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


def _contains_keeper_secret(value: object) -> bool:
    if isinstance(value, str):
        return bool(KEEPER_SECRET.search(value))
    if isinstance(value, dict):
        return any(_contains_keeper_secret(item) for item in value.values())
    if isinstance(value, list):
        return any(_contains_keeper_secret(item) for item in value)
    return False


def _exact_shared_lock(value: object) -> bool:
    return value == {
        "group": SHARED_KEEPER_CONCURRENCY,
        "cancel-in-progress": False,
    }


def _yaml_scalar(value: str) -> str:
    value = value.split(" #", 1)[0].strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
        value = value[1:-1]
    return value


def _validate_json_keeper_locks(document: object, workflow_name: str) -> set[str]:
    if not isinstance(document, dict):
        raise GuardError(f"keeper workflow is not a mapping: {workflow_name}")
    if _exact_shared_lock(document.get("concurrency")):
        raise GuardError(f"shared keeper lock must not be workflow-level: {workflow_name}")
    jobs = document.get("jobs")
    if not isinstance(jobs, dict):
        raise GuardError(f"keeper workflow has no effective jobs mapping: {workflow_name}")
    key_jobs: set[str] = set()
    for job_name, job in jobs.items():
        if not isinstance(job_name, str) or not isinstance(job, dict):
            raise GuardError(f"keeper workflow has an invalid job: {workflow_name}")
        receives_key = _contains_keeper_secret(job)
        has_shared_lock = _exact_shared_lock(job.get("concurrency"))
        if receives_key:
            key_jobs.add(job_name)
            if not has_shared_lock:
                raise GuardError(
                    f"key-bearing job lacks the exact shared keeper lock: {workflow_name}:{job_name}"
                )
        elif has_shared_lock:
            raise GuardError(
                f"non-key-bearing job may not hold the shared keeper lock: {workflow_name}:{job_name}"
            )
    if _contains_keeper_secret(document) and not key_jobs:
        raise GuardError(f"keeper secret is outside a recognized job: {workflow_name}")
    return key_jobs


def _validate_yaml_keeper_locks(workflow_text: str, workflow_name: str) -> set[str]:
    """Parse the effective jobs/concurrency subset of an Actions YAML document.

    Block-scalar contents may expose a secret but can never impersonate mapping
    keys. Unsupported tabs fail closed. This deliberately recognizes only the
    simple mapping structure used by checked-in Actions workflows.
    """

    in_jobs = False
    current_job: str | None = None
    block_parent_indent: int | None = None
    block_job: str | None = None
    top_concurrency = False
    job_concurrency: str | None = None
    top_group: str | None = None
    locks: dict[str, dict[str, str]] = {}
    key_jobs: set[str] = set()
    secret_count = 0

    for line_number, raw in enumerate(workflow_text.splitlines(), 1):
        if raw[: len(raw) - len(raw.lstrip(" \t"))].find("\t") >= 0:
            raise GuardError(f"tabs are unsupported in keeper YAML: {workflow_name}:{line_number}")
        stripped = raw.strip()
        indent = len(raw) - len(raw.lstrip(" "))
        secret_here = bool(KEEPER_SECRET.search(raw))

        if block_parent_indent is not None and (not stripped or indent > block_parent_indent):
            if secret_here:
                if block_job is None:
                    raise GuardError(
                        f"keeper secret is outside a recognized job: {workflow_name}:{line_number}"
                    )
                key_jobs.add(block_job)
                secret_count += 1
            continue
        block_parent_indent = None
        block_job = None

        if not stripped or stripped.startswith("#"):
            continue
        effective = stripped.split(" #", 1)[0].rstrip()

        if indent == 0:
            job_concurrency = None
            current_job = None
            if effective == "jobs:":
                in_jobs = True
                top_concurrency = False
                continue
            in_jobs = False
            top_concurrency = effective == "concurrency:"
            continue

        if top_concurrency:
            if indent == 2 and effective.startswith("group:"):
                top_group = _yaml_scalar(effective.split(":", 1)[1])
            if re.search(r":\s*[|>][+\-]?\d?\s*$", effective):
                block_parent_indent = indent
            continue

        if not in_jobs:
            if secret_here:
                raise GuardError(
                    f"keeper secret is outside a recognized job: {workflow_name}:{line_number}"
                )
            continue

        job_match = re.fullmatch(r"([A-Za-z0-9_-]+):", effective) if indent == 2 else None
        if job_match:
            current_job = job_match.group(1)
            job_concurrency = None
            continue
        if current_job is None:
            if secret_here:
                raise GuardError(
                    f"keeper secret is outside a recognized job: {workflow_name}:{line_number}"
                )
            continue

        if secret_here:
            key_jobs.add(current_job)
            secret_count += 1

        if indent == 4:
            job_concurrency = current_job if effective == "concurrency:" else None
        elif job_concurrency and indent == 6:
            if effective.startswith("group:"):
                locks.setdefault(job_concurrency, {})["group"] = _yaml_scalar(
                    effective.split(":", 1)[1]
                )
            elif effective.startswith("cancel-in-progress:"):
                locks.setdefault(job_concurrency, {})["cancel-in-progress"] = _yaml_scalar(
                    effective.split(":", 1)[1]
                ).lower()

        if re.search(r":\s*[|>][+\-]?\d?\s*$", effective):
            block_parent_indent = indent
            block_job = current_job

    if top_group == SHARED_KEEPER_CONCURRENCY:
        raise GuardError(f"shared keeper lock must not be workflow-level: {workflow_name}")
    for job_name, values in locks.items():
        has_shared_lock = values == {
            "group": SHARED_KEEPER_CONCURRENCY,
            "cancel-in-progress": "false",
        }
        if job_name in key_jobs and not has_shared_lock:
            raise GuardError(
                f"key-bearing job lacks the exact shared keeper lock: {workflow_name}:{job_name}"
            )
        if job_name not in key_jobs and has_shared_lock:
            raise GuardError(
                f"non-key-bearing job may not hold the shared keeper lock: {workflow_name}:{job_name}"
            )
    for job_name in key_jobs:
        if locks.get(job_name) != {
            "group": SHARED_KEEPER_CONCURRENCY,
            "cancel-in-progress": "false",
        }:
            raise GuardError(
                f"key-bearing job lacks the exact shared keeper lock: {workflow_name}:{job_name}"
            )
    if KEEPER_SECRET.search(workflow_text) and secret_count == 0:
        raise GuardError(f"keeper secret was not parsed effectively: {workflow_name}")
    return key_jobs


def validate_keeper_workflow_locks(root: Path) -> list[str]:
    workflow_root = root / ".github" / "workflows"
    paths = sorted({*workflow_root.glob("*.yml"), *workflow_root.glob("*.yaml")})
    validated: list[str] = []
    for workflow_path in paths:
        workflow_text = workflow_path.read_text(encoding="utf-8")
        if not KEEPER_SECRET.search(workflow_text):
            continue
        try:
            document = __import__("json").loads(workflow_text)
        except ValueError:
            key_jobs = _validate_yaml_keeper_locks(workflow_text, workflow_path.name)
        else:
            key_jobs = _validate_json_keeper_locks(document, workflow_path.name)
        if not key_jobs:
            raise GuardError(f"keeper workflow has no key-bearing job: {workflow_path.name}")
        validated.append(workflow_path.name)
    if not validated:
        raise GuardError("no keeper workflow was available for shared-lock validation")
    return validated


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
