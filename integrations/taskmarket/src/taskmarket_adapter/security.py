"""Filesystem and network guards for untrusted tool arguments.

`taskmarket_submit` must never upload arbitrary host files: paths are resolved
canonically, symlinks are rejected anywhere along the path, the target must be
a regular file inside one allowlisted artifact root, and its size is bounded.
"""
from __future__ import annotations

import os
import pathlib
import stat
from typing import List, Optional, Sequence

from .errors import SecurityError

ALLOWED_NETWORKS = frozenset({"base-mainnet", "base-sepolia"})
DEFAULT_NETWORK = "base-mainnet"

ENV_ARTIFACT_ROOTS = "TASKMARKET_ARTIFACT_ROOTS"
ENV_MAX_ARTIFACT_BYTES = "TASKMARKET_MAX_ARTIFACT_BYTES"

DEFAULT_MAX_ARTIFACT_BYTES = 20 * 1024 * 1024


def validate_network(network: Optional[str]) -> str:
    """Validate a declared network against the Base allowlist."""
    if network is None:
        return DEFAULT_NETWORK
    if not isinstance(network, str) or network not in ALLOWED_NETWORKS:
        raise SecurityError(
            "refused: network must be one of " + ", ".join(sorted(ALLOWED_NETWORKS))
        )
    return network


def artifact_roots(env: Optional[dict] = None) -> List[pathlib.Path]:
    """Parse the operator-configured artifact root allowlist (colon-separated)."""
    environment = os.environ if env is None else env
    raw_value = environment.get(ENV_ARTIFACT_ROOTS, "")
    if not raw_value.strip():
        raise SecurityError("refused: no artifact root configured by the operator")
    roots: List[pathlib.Path] = []
    for part in raw_value.split(os.pathsep):
        candidate = pathlib.Path(part.strip())
        if not candidate.is_absolute():
            raise SecurityError("refused: artifact roots must be absolute paths")
        resolved = pathlib.Path(os.path.realpath(candidate))
        if resolved not in roots:
            roots.append(resolved)
    if not roots:
        raise SecurityError("refused: no artifact root configured by the operator")
    return roots


def max_artifact_bytes(env: Optional[dict] = None) -> int:
    environment = os.environ if env is None else env
    raw_value = environment.get(ENV_MAX_ARTIFACT_BYTES)
    if not raw_value:
        return DEFAULT_MAX_ARTIFACT_BYTES
    try:
        limit = int(raw_value)
    except ValueError:
        raise SecurityError("refused: invalid artifact size limit configured") from None
    if limit <= 0:
        raise SecurityError("refused: invalid artifact size limit configured")
    return limit


def resolve_artifact(
    path_str: str,
    roots: Optional[Sequence[pathlib.Path]] = None,
    size_limit: Optional[int] = None,
) -> pathlib.Path:
    """Resolve an artifact path under host policy. Returns the canonical path.

    Refuses (in order): non-absolute or empty paths, any symlink component,
    targets outside every allowed root, non-regular files, and oversized files.
    """
    if not isinstance(path_str, str) or not path_str.strip():
        raise SecurityError("refused: artifact path required")
    if "\x00" in path_str:
        raise SecurityError("refused: invalid artifact path")

    allowed_roots = roots if roots is not None else artifact_roots()
    limit = max_artifact_bytes() if size_limit is None else size_limit

    candidate = pathlib.Path(path_str)
    if not candidate.is_absolute():
        raise SecurityError("refused: artifact path must be absolute")

    # Reject symlinks anywhere along the path, including the final component.
    for prefix in candidate.parents:
        if os.path.islink(prefix):
            raise SecurityError("refused: artifact path contains a symlink")
    if os.path.islink(candidate):
        raise SecurityError("refused: artifact path contains a symlink")

    resolved = pathlib.Path(os.path.realpath(candidate))
    if not any(resolved == root or root in resolved.parents for root in allowed_roots):
        raise SecurityError("refused: artifact path is outside the allowed artifact roots")

    try:
        info = os.lstat(resolved)  # lstat: never follow a last-second symlink swap
    except OSError:
        raise SecurityError("refused: artifact file not accessible") from None
    if not stat.S_ISREG(info.st_mode):
        raise SecurityError("refused: artifact must be a regular file")
    if info.st_size > limit:
        raise SecurityError("refused: artifact exceeds the configured size limit")
    return resolved
