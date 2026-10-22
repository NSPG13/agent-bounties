"""Direct-bounty evidence checklist.

Binds the four things a direct coding bounty must be able to point at, and
keeps the three kinds of evidence in three separate buckets:

    submission    what was delivered  (commit, repo, subdirectory, PR, artifact)
    verification  what checked it     (check-run / benchmark URLs)
    payment       what paid           (canonical settlement receipt ONLY)

Why the split exists: a pull request, a green test run, and a transaction hash
are three different claims. Merging them is how "a PR exists" turns into "I got
paid". This module refuses to let a local input assert the payment bucket.

Fail-closed rules:
  * A missing, empty, mutable, or non-HTTPS artifact reference is a hard fail,
    never a warning. `main`, `latest`, `HEAD` and branch-referencing URLs are
    mutable and are rejected.
  * Local state can carry `settled: true` and the payment bucket will still read
    `unverified`. Payment requires a LIVE-CANONICAL settlement receipt carrying
    real chain identity (`tx_hash`, `log_key`, positive `block_number`).
    This mirrors `.openhands/hooks/agent-bounties-evidence.py` on purpose.
  * Secret-bearing and personal fields are scrubbed from the response so the
    output is safe to drop into an MCP or API payload.

Offline and replayable: no network, no clock, no randomness. Same input always
produces the same output.
"""

from __future__ import annotations

import hashlib
import re
from typing import Any, Dict, List, Tuple

__all__ = [
    "CHECKLIST_VERSION",
    "REQUIRED_FIELDS",
    "build_checklist",
    "ChecklistError",
]

CHECKLIST_VERSION = "1.0.0"

REQUIRED_FIELDS: Tuple[str, ...] = (
    "repository",
    "source_commit",
    "subdirectory",
    "pull_request_url",
    "check_run_urls",
    "artifact_url",
    "artifact_sha256",
)

# Fields whose values must never reach a response payload.
_SECRET_PATTERN = re.compile(
    r"(token|secret|password|passwd|api[_-]?key|private[_-]?key|seed|mnemonic"
    r"|email|author|maintainer|wallet|address|cookie|session|credential)",
    re.IGNORECASE,
)

_HEX40 = re.compile(r"^[0-9a-f]{40}$", re.IGNORECASE)
_HEX64 = re.compile(r"^[0-9a-f]{64}$", re.IGNORECASE)
_SHA256 = re.compile(r"^[0-9a-f]{64}$", re.IGNORECASE)
_GITHUB_REPO = re.compile(r"^https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/?$")
_GITHUB_PR = re.compile(
    r"^https://github\.com/[A-Za-z0-9._-]+/[A-Za-z0-9._-]+/pull/\d+$"
)
_TX_HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")

# Any of these in an artifact URL means it can change under you.
_MUTABLE_MARKERS = (
    "/latest",
    "/main/",
    "/master/",
    "/HEAD/",
    "/head/",
    "ref=main",
    "ref=master",
    "ref=HEAD",
    "branch=",
    "@main",
    "@master",
    "@HEAD",
)

_MUTABLE_VALUES = frozenset({"", "main", "master", "head", "latest", "tip", "trunk"})


class ChecklistError(Exception):
    """Raised when the checklist is asked to validate something it cannot."""


def _is_https_url(value: Any) -> bool:
    return isinstance(value, str) and value.startswith("https://")


def _clean(value: Any) -> str:
    """Trim; non-strings become '' so callers never have to type-check."""
    if not isinstance(value, str):
        return ""
    return value.strip()


def _reject_mutable(url: str) -> bool:
    lowered = url.lower()
    return any(marker in lowered for marker in _MUTABLE_MARKERS)


def _scrub(mapping: Dict[str, Any]) -> Dict[str, Any]:
    """Drop secret-bearing / personal keys so the payload is safe to return."""
    return {k: v for k, v in mapping.items() if not _SECRET_PATTERN.search(str(k))}


def _validate_submission(raw: Dict[str, Any]) -> Tuple[Dict[str, Any], List[str]]:
    problems: List[str] = []

    repo = _clean(raw.get("repository"))
    if not _GITHUB_REPO.match(repo):
        problems.append(
            "repository must be an HTTPS github.com/<owner>/<repo> URL, "
            "not a branch or a bare slug"
        )

    commit = _clean(raw.get("source_commit"))
    if not (_HEX40.match(commit) or _HEX64.match(commit)):
        if commit.lower() in _MUTABLE_VALUES:
            problems.append("source_commit must be a pinned commit, not a branch")
        else:
            problems.append("source_commit must be a 40- or 64-character hex digest")

    subdir = _clean(raw.get("subdirectory"))
    if not subdir:
        problems.append("subdirectory is required (use '.' at repository root)")
    elif subdir.startswith("/") or ".." in subdir.split("/"):
        problems.append("subdirectory must be a relative path without '..'")

    pr = _clean(raw.get("pull_request_url"))
    if not _GITHUB_PR.match(pr):
        problems.append("pull_request_url must be https://github.com/<o>/<r>/pull/<n>")

    artifact_url = _clean(raw.get("artifact_url"))
    if not artifact_url:
        problems.append("artifact_url is empty")
    elif not _is_https_url(artifact_url):
        problems.append("artifact_url must use https:// (rejects http, file, git)")
    elif _reject_mutable(artifact_url):
        problems.append(
            "artifact_url is mutable (points at a branch, 'latest', or HEAD)"
        )

    digest = _clean(raw.get("artifact_sha256")).lower()
    if not _SHA256.match(digest):
        problems.append("artifact_sha256 must be a 64-character hex sha256")

    return (
        {
            "repository": repo,
            "source_commit": commit.lower(),
            "subdirectory": subdir,
            "pull_request_url": pr,
            "artifact_url": artifact_url,
            "artifact_sha256": digest,
        },
        problems,
    )


def _validate_verification(raw: Dict[str, Any]) -> Tuple[Dict[str, Any], List[str]]:
    problems: List[str] = []
    raw_runs = raw.get("check_run_urls")

    if not isinstance(raw_runs, (list, tuple)) or len(raw_runs) == 0:
        problems.append("check_run_urls must be a non-empty list")
        return {"check_run_urls": []}, problems

    runs = [str(u).strip() for u in raw_runs]
    for u in runs:
        if not _is_https_url(u):
            problems.append(f"check_run_urls entry is not https:// : {u!r}")
        elif _reject_mutable(u):
            problems.append(f"check_run_urls entry is mutable: {u!r}")
    if len(set(runs)) != len(runs):
        problems.append("check_run_urls contains duplicates")

    return {"check_run_urls": runs}, problems


def _validate_payment(raw: Dict[str, Any]) -> Tuple[Dict[str, Any], List[str]]:
    """Payment is the one bucket local input can never satisfy.

    A `settled: true` in the caller's own state is treated as UNVERIFIED. Only a
    LIVE-CANONICAL receipt with real chain identity upgrades the bucket.
    """
    receipt = raw.get("settlement")
    if not isinstance(receipt, dict):
        return (
            {
                "status": "unverified",
                "reason": (
                    "no canonical settlement receipt; a PR, a green run, or a tx "
                    "hash alone is not payment evidence"
                ),
            },
            [],
        )

    provenance = _clean(receipt.get("provenance"))
    tx_hash = _clean(receipt.get("tx_hash"))
    log_key = _clean(receipt.get("log_key"))
    block = receipt.get("block_number")

    if provenance != "canonical_live":
        return (
            {
                "status": "unverified",
                "reason": f"provenance {provenance!r} is not 'canonical_live'",
            },
            [],
        )
    if not _TX_HASH.match(tx_hash):
        return {"status": "unverified", "reason": "tx_hash is not 0x + 64 hex"}, []
    if not log_key:
        return {"status": "unverified", "reason": "log_key is missing"}, []
    if not isinstance(block, int) or isinstance(block, bool) or block <= 0:
        return {"status": "unverified", "reason": "block_number must be positive"}, []

    return (
        {
            "status": "settled",
            "tx_hash": tx_hash,
            "log_key": log_key,
            "block_number": block,
        },
        [],
    )


def build_checklist(raw: Dict[str, Any]) -> Dict[str, Any]:
    """Validate one direct-bounty evidence object.

    Returns a compact, deterministic, secret-free payload with three separate
    buckets and a boolean `complete`. Never raises on bad input: malformed
    evidence is reported, not thrown away.
    """
    if not isinstance(raw, dict):
        raise ChecklistError("evidence must be a mapping")

    safe = _scrub(raw)

    missing = [f for f in REQUIRED_FIELDS if f not in raw]
    for field in REQUIRED_FIELDS:
        value = raw.get(field)
        if isinstance(value, str) and not value.strip():
            missing.append(field)
        if isinstance(value, (list, tuple)) and len(value) == 0:
            missing.append(field)
    missing = sorted(set(missing))

    submission, sub_problems = _validate_submission(safe)
    verification, ver_problems = _validate_verification(safe)
    payment, pay_problems = _validate_payment(safe)

    problems = missing + sub_problems + ver_problems + pay_problems

    # The digest binds the artifact, not this response. It is echoed so a caller
    # can re-verify the binding without recomputing anything downstream.
    binding = "|".join(
        [
            submission["repository"],
            submission["source_commit"],
            submission["subdirectory"],
            submission["pull_request_url"],
            submission["artifact_url"],
            submission["artifact_sha256"],
        ]
    )

    return {
        "checklist_version": CHECKLIST_VERSION,
        # `complete` means the deliverable is fully bound: submission and
        # verification both pass. Payment is deliberately NOT part of this flag —
        # settling is a separate claim with a separate boundary.
        "complete": not problems,
        "submission": submission,
        "verification": verification,
        "payment": payment,
        "problems": problems,
        "binding_digest": hashlib.sha256(binding.encode("utf-8")).hexdigest(),
    }
