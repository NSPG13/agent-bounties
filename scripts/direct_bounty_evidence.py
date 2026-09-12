#!/usr/bin/env python3
"""Direct coding bounty evidence checklist validator and formatter (Issue #686)."""

from __future__ import annotations

import json
import re
from typing import Any, Dict
from urllib.parse import urlparse


class DirectBountyEvidenceError(ValueError):
    """Raised when direct bounty evidence validation fails."""

    pass


def validate_https_url(url: str, field_name: str) -> None:
    if not isinstance(url, str) or not url.strip():
        raise DirectBountyEvidenceError(f"{field_name} must be a non-empty string")
    parsed = urlparse(url)
    if parsed.scheme != "https":
        raise DirectBountyEvidenceError(f"{field_name} must use HTTPS scheme, got: {url}")
    if not parsed.netloc:
        raise DirectBountyEvidenceError(f"{field_name} must contain a valid domain, got: {url}")


def validate_commit_hash(sha: str, field_name: str) -> None:
    if not isinstance(sha, str) or not re.match(r"^[a-fA-F0-9]{40}$", sha):
        raise DirectBountyEvidenceError(
            f"{field_name} must be a valid 40-character hex SHA-1 commit hash"
        )


def validate_sha256_digest(digest: str, field_name: str) -> None:
    if not isinstance(digest, str) or not re.match(r"^[a-fA-F0-9]{64}$", digest):
        raise DirectBountyEvidenceError(
            f"{field_name} must be a valid 64-character hex SHA-256 digest"
        )


def validate_submission_evidence(sub: Dict[str, Any]) -> None:
    if not isinstance(sub, dict):
        raise DirectBountyEvidenceError("submission evidence must be an object")

    validate_commit_hash(sub.get("source_commit", ""), "submission.source_commit")

    repo = sub.get("repository", "")
    if not isinstance(repo, str) or not re.match(r"^[a-zA-Z0-9_-]+/[a-zA-Z0-9_.-]+$", repo):
        raise DirectBountyEvidenceError("submission.repository must be in owner/repo format")

    sub_dir = sub.get("subdirectory", "")
    if not isinstance(sub_dir, str):
        raise DirectBountyEvidenceError("submission.subdirectory must be a string")

    validate_https_url(sub.get("pull_request_url", ""), "submission.pull_request_url")


def validate_verification_evidence(ver: Dict[str, Any]) -> None:
    if not isinstance(ver, dict):
        raise DirectBountyEvidenceError("verification evidence must be an object")

    check_runs = ver.get("check_run_urls", [])
    if not isinstance(check_runs, list) or len(check_runs) == 0:
        raise DirectBountyEvidenceError(
            "verification.check_run_urls must be a non-empty list of URLs"
        )
    for idx, url in enumerate(check_runs):
        validate_https_url(url, f"verification.check_run_urls[{idx}]")

    validate_sha256_digest(ver.get("artifact_digest", ""), "verification.artifact_digest")
    validate_https_url(ver.get("artifact_url", ""), "verification.artifact_url")


def validate_payment_evidence(pay: Dict[str, Any]) -> None:
    if not isinstance(pay, dict):
        raise DirectBountyEvidenceError("payment evidence must be an object")

    settlement_event = pay.get("settlement_event", "")
    if settlement_event != "BountySettled":
        raise DirectBountyEvidenceError(
            f"payment.settlement_event must be 'BountySettled', got: {settlement_event}"
        )

    tx_hash = pay.get("tx_hash", "")
    if not isinstance(tx_hash, str) or not re.match(r"^0x[a-fA-F0-9]{64}$", tx_hash):
        raise DirectBountyEvidenceError(
            "payment.tx_hash must be a valid 0x-prefixed 64-hex transaction hash"
        )

    amount = pay.get("amount_usdc", 0.0)
    if not isinstance(amount, (int, float)) or amount <= 0:
        raise DirectBountyEvidenceError("payment.amount_usdc must be a positive number")


def validate_direct_bounty_evidence(evidence: Dict[str, Any]) -> Dict[str, Any]:
    """Validate a complete direct bounty evidence object."""
    if not isinstance(evidence, dict):
        raise DirectBountyEvidenceError("evidence checklist must be a JSON object")

    schema_version = evidence.get("schema_version")
    if schema_version != "agent-bounties/direct-evidence-v1":
        raise DirectBountyEvidenceError(f"unsupported schema_version: {schema_version}")

    if "submission" not in evidence:
        raise DirectBountyEvidenceError("missing required section: submission")
    if "verification" not in evidence:
        raise DirectBountyEvidenceError("missing required section: verification")
    if "payment" not in evidence:
        raise DirectBountyEvidenceError("missing required section: payment")

    validate_submission_evidence(evidence["submission"])
    validate_verification_evidence(evidence["verification"])
    validate_payment_evidence(evidence["payment"])

    return evidence


def format_compact_evidence_checklist(evidence: Dict[str, Any]) -> str:
    """Format evidence into a compact human and machine readable summary without secrets."""
    validated = validate_direct_bounty_evidence(evidence)
    sub = validated["submission"]
    ver = validated["verification"]
    pay = validated["payment"]

    pr_url = sub["pull_request_url"]
    pr_short = pr_url.replace("https://github.com/", "")

    return json.dumps(
        {
            "v": "direct-v1",
            "sub": {
                "repo": sub["repository"],
                "commit": sub["source_commit"][:7],
                "pr": pr_short,
            },
            "ver": {
                "digest": ver["artifact_digest"][:12],
                "checks": len(ver["check_run_urls"]),
            },
            "pay": {
                "evt": pay["settlement_event"],
                "usdc": pay["amount_usdc"],
                "tx": pay["tx_hash"][:10],
            },
        },
        separators=(",", ":"),
    )

