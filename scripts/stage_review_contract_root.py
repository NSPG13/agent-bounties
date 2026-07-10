#!/usr/bin/env python3
"""Stage bounded API/MCP source files for trusted external-PR contract checks."""

from __future__ import annotations

import argparse
import hashlib
import json
import stat
import sys
from pathlib import Path


REQUIRED_SOURCES = (
    Path("crates/api/src/main.rs"),
    Path("crates/mcp-server/src/main.rs"),
)
MAX_SOURCE_BYTES = 5 * 1024 * 1024


class StageError(RuntimeError):
    pass


def require_within(path: Path, root: Path, label: str) -> None:
    try:
        path.relative_to(root)
    except ValueError as error:
        raise StageError(f"{label} escapes its trusted root: {path}") from error


def stage_contract_root(
    worktree: Path,
    output: Path,
    *,
    max_source_bytes: int = MAX_SOURCE_BYTES,
) -> list[dict[str, object]]:
    if max_source_bytes <= 0:
        raise StageError("max_source_bytes must be positive")

    try:
        worktree_root = worktree.resolve(strict=True)
    except OSError as error:
        raise StageError(f"external PR worktree does not exist: {worktree}") from error
    if not worktree_root.is_dir():
        raise StageError(f"external PR worktree is not a directory: {worktree_root}")

    output_root = output.resolve(strict=False)
    if output_root == worktree_root or worktree_root in output_root.parents:
        raise StageError("staged contract root must be outside the external PR worktree")
    if output.exists():
        if output.is_symlink() or not output.is_dir():
            raise StageError(f"staged contract root is not a regular directory: {output}")
        if any(output.iterdir()):
            raise StageError(f"staged contract root must be empty: {output}")
    else:
        output.mkdir(parents=True)
    output_root = output.resolve(strict=True)

    report: list[dict[str, object]] = []
    for relative in REQUIRED_SOURCES:
        source = worktree_root / relative
        try:
            source_stat = source.lstat()
        except OSError as error:
            raise StageError(f"required PR contract source is missing: {relative}") from error
        if stat.S_ISLNK(source_stat.st_mode):
            raise StageError(f"required PR contract source must not be a symlink: {relative}")
        if not stat.S_ISREG(source_stat.st_mode):
            raise StageError(f"required PR contract source must be a regular file: {relative}")

        try:
            resolved_source = source.resolve(strict=True)
        except OSError as error:
            raise StageError(f"unable to resolve PR contract source: {relative}") from error
        require_within(resolved_source, worktree_root, str(relative))

        data = source.read_bytes()
        if len(data) > max_source_bytes:
            raise StageError(
                f"required PR contract source exceeds {max_source_bytes} bytes: {relative}"
            )
        if b"\x00" in data:
            raise StageError(f"required PR contract source contains a NUL byte: {relative}")
        try:
            data.decode("utf-8")
        except UnicodeDecodeError as error:
            raise StageError(
                f"required PR contract source is not valid UTF-8: {relative}"
            ) from error

        destination = output_root / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        resolved_parent = destination.parent.resolve(strict=True)
        require_within(resolved_parent, output_root, str(relative))
        destination.write_bytes(data)
        report.append(
            {
                "path": relative.as_posix(),
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )

    return report


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worktree", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        sources = stage_contract_root(args.worktree, args.output)
    except StageError as error:
        print(f"external PR contract staging failed: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"staged_contract_root": str(args.output), "sources": sources}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
