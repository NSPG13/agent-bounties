#!/usr/bin/env python3
"""Concise direct-bounty evidence checklist and validator (Issue #686).

Binds the repository, exact source commit, mandatory subdirectory, repository-bound
pull request URL, repository-bound check-run URLs, immutable credential-free HTTPS
artifact URL + artifact digest, and the canonical Base Mainnet `BountySettled`
settlement boundary.

Explicitly separates:
1. `submission` evidence (repository, commit_sha, subdirectory, pull_request_url, submission_hash)
2. `verification` evidence (check_run_urls, artifact_url, artifact_digest, evidence_hash)
3. `payment` evidence (verified canonical `BountySettled` receipt log on Base Mainnet)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any, Mapping, Sequence
from urllib.parse import parse_qs, urlsplit


CHECKLIST_SCHEMA = "agent-bounties/direct-bounty-evidence-checklist-v1"
BASE_MAINNET_CHAIN_ID = 8453
NATIVE_BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
# Canonical keccak256 topic0 for BountySettled in AgentBounty / AgentBountyV2
BOUNTY_SETTLED_TOPIC0 = (
    "0x2ee7b1323824324868017119c9922527f2e2c8eb20a8044c3dd574bdbfb2dd71"
)

REPO_RE = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
COMMIT_SHA_RE = re.compile(r"^[0-9a-f]{40}$")
SHA256_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")
HEX32_RE = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
SAFE_SUBDIR_RE = re.compile(r"^[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*$")

SENSITIVE_QUERY_KEYS = frozenset(
    {
        "token",
        "access_token",
        "auth",
        "key",
        "api_key",
        "apikey",
        "secret",
        "password",
        "sig",
        "signature",
        "x-amz-signature",
        "x-amz-credential",
    }
)
MUTABLE_URL_SEGMENTS = (
    "/latest",
    "/refs/heads/",
    "/branch/",
    "/branches/",
    "/tree/main",
    "/tree/master",
    "/blob/main/",
    "/blob/master/",
    "/raw/main/",
    "/raw/master/",
)


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha256_hex32(value: Any) -> str:
    text = value if isinstance(value, str) else canonical_json(value)
    return "0x" + hashlib.sha256(text.encode("utf-8")).hexdigest()


def _validate_https_no_credentials(url: Any, field_name: str) -> tuple[Any, list[str]]:
    errors: list[str] = []
    if not isinstance(url, str) or not url.strip():
        return None, [f"missing_{field_name}"]
    raw = url.strip()
    try:
        parsed = urlsplit(raw)
    except ValueError:
        return None, [f"malformed_{field_name}"]

    if parsed.scheme.lower() != "https" or not parsed.hostname:
        errors.append(f"non_https_{field_name}")
    if parsed.username or parsed.password or "@" in parsed.netloc:
        errors.append(f"credentials_in_{field_name}")
    if parsed.fragment:
        errors.append(f"fragment_forbidden_in_{field_name}")
    if parsed.query:
        q_keys = {k.lower() for k in parse_qs(parsed.query, keep_blank_values=True).keys()}
        if q_keys & SENSITIVE_QUERY_KEYS:
            errors.append(f"credentials_in_{field_name}")
    return parsed, errors


def _validate_immutable_artifact_url(
    url: Any,
    *,
    commit_sha: str,
    artifact_digest: str,
) -> list[str]:
    parsed, errors = _validate_https_no_credentials(url, "artifact_url")
    if errors or parsed is None:
        return errors

    raw_lower = str(url).strip().lower()
    path_lower = parsed.path.lower()

    if parsed.query:
        errors.append("mutable_or_query_artifact_url")
    if any(seg in path_lower for seg in MUTABLE_URL_SEGMENTS):
        errors.append("mutable_artifact_url")

    digest_hex = artifact_digest.removeprefix("sha256:").lower()
    has_immutable_anchor = (
        (commit_sha and commit_sha.lower() in raw_lower)
        or (digest_hex and digest_hex in raw_lower)
        or bool(re.search(r"/actions/runs/\d+/artifacts/\d+$", path_lower))
        or bool(re.search(r"/sha256/[0-9a-f]{64}(?:/|$)", path_lower))
    )
    if not has_immutable_anchor:
        errors.append("mutable_artifact_url")

    return errors


def _validate_repo_bound_pr_url(url: Any, repository: str) -> list[str]:
    parsed, errors = _validate_https_no_credentials(url, "pull_request_url")
    if errors or parsed is None:
        return errors
    if parsed.hostname.lower() != "github.com" or parsed.query:
        return ["invalid_pull_request_url"]
    m = re.match(r"^/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/([1-9][0-9]*)$", parsed.path)
    if not m:
        return ["invalid_pull_request_url"]
    if m.group(1).lower() != repository.lower():
        return ["cross_repository_pull_request_url"]
    return []


def _validate_repo_bound_check_run_urls(urls: Any, repository: str) -> list[str]:
    if not isinstance(urls, list) or len(urls) == 0:
        return ["missing_check_run_urls"]
    errors: list[str] = []
    seen: set[str] = set()
    pattern = re.compile(
        r"^/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/(?:actions/runs/[1-9][0-9]*(?:/job/[1-9][0-9]*)?|runs/[1-9][0-9]*|checks/[1-9][0-9]*)$"
    )
    for item in urls:
        parsed, item_errs = _validate_https_no_credentials(item, "check_run_url")
        if item_errs or parsed is None:
            errors.extend(item_errs)
            continue
        if parsed.hostname.lower() != "github.com" or parsed.query:
            errors.append("invalid_check_run_url")
            continue
        m = pattern.match(parsed.path)
        if not m:
            errors.append("invalid_check_run_url")
            continue
        if m.group(1).lower() != repository.lower():
            errors.append("cross_repository_check_run_url")
            continue
        norm = str(item).strip()
        if norm in seen:
            errors.append("duplicate_check_run_url")
        seen.add(norm)
    return errors


def _validate_verified_settlement(
    payment: Mapping[str, Any],
    *,
    expected_submission_hash: str,
    expected_evidence_hash: str,
) -> tuple[dict[str, Any] | None, list[str]]:
    """Validate canonical Base Mainnet BountySettled receipt proof (not mere formatting)."""
    errors: list[str] = []

    if int(payment.get("chain_id", -1)) != BASE_MAINNET_CHAIN_ID:
        errors.append("unverified_receipt_wrong_chain")
    if str(payment.get("settlement_token", "")).strip().lower() != NATIVE_BASE_USDC:
        errors.append("unverified_receipt_wrong_token")
    if payment.get("event") != "BountySettled":
        errors.append("unverified_receipt_event_name")

    tx_hash = str(payment.get("transaction_hash", "")).strip().lower()
    if not HEX32_RE.match(tx_hash):
        errors.append("unverified_receipt_tx_hash")

    bounty_contract = str(payment.get("bounty_contract", "")).strip().lower()
    if not ADDRESS_RE.match(bounty_contract):
        errors.append("unverified_receipt_bounty_contract")

    solver = str(payment.get("solver", "")).strip().lower()
    if not ADDRESS_RE.match(solver):
        errors.append("unverified_receipt_solver")

    bounty_id = str(payment.get("bounty_id", "")).strip().lower()
    if not HEX32_RE.match(bounty_id):
        errors.append("unverified_receipt_bounty_id")

    # Verify confirmed transaction receipt & canonical BountySettled log
    receipt = payment.get("receipt")
    if not isinstance(receipt, Mapping):
        errors.append("unverified_receipt_missing_proof")
        return None, errors

    if receipt.get("status") not in (1, "0x1"):
        errors.append("unverified_receipt_failed_status")

    block_number = receipt.get("block_number")
    safe_block = receipt.get("safe_block")
    if (
        isinstance(block_number, bool)
        or not isinstance(block_number, int)
        or block_number <= 0
        or isinstance(safe_block, bool)
        or not isinstance(safe_block, int)
        or safe_block < block_number
    ):
        errors.append("unverified_receipt_unfinalized_block")

    log_entry = receipt.get("bounty_settled_log")
    if not isinstance(log_entry, Mapping):
        errors.append("unverified_receipt_missing_settled_log")
        return None, errors

    log_address = str(log_entry.get("address", "")).strip().lower()
    topics = log_entry.get("topics", [])
    log_index = log_entry.get("log_index")
    if (
        log_address != bounty_contract
        or not isinstance(topics, list)
        or len(topics) < 2
        or str(topics[0]).strip().lower() != BOUNTY_SETTLED_TOPIC0
        or str(topics[1]).strip().lower() != bounty_id
        or isinstance(log_index, bool)
        or not isinstance(log_index, int)
        or log_index < 0
    ):
        errors.append("unverified_receipt_canonical_log_mismatch")

    # Verify commitment binding to submission_hash and evidence_hash
    settled_sub_hash = str(log_entry.get("submission_hash", "")).strip().lower()
    settled_ev_hash = str(log_entry.get("evidence_hash", "")).strip().lower()
    if settled_sub_hash != expected_submission_hash.lower():
        errors.append("unverified_receipt_submission_hash_mismatch")
    if settled_ev_hash != expected_evidence_hash.lower():
        errors.append("unverified_receipt_evidence_hash_mismatch")

    # Verify exact payout conservation math
    solver_reward = log_entry.get("solver_reward")
    bond_returned = log_entry.get("claim_bond_returned", 0)
    timeout_bonus = log_entry.get("timeout_bond_bonus", 0)
    solver_payout = log_entry.get("solver_payout")
    if (
        not isinstance(solver_reward, int)
        or solver_reward <= 0
        or not isinstance(bond_returned, int)
        or bond_returned < 0
        or not isinstance(timeout_bonus, int)
        or timeout_bonus < 0
        or not isinstance(solver_payout, int)
        or solver_payout != solver_reward + bond_returned + timeout_bonus
    ):
        errors.append("unverified_receipt_payout_math_mismatch")

    if errors:
        return None, errors

    return {
        "settled": True,
        "paid": True,
        "event": "BountySettled",
        "chain_id": BASE_MAINNET_CHAIN_ID,
        "settlement_token": NATIVE_BASE_USDC,
        "bounty_contract": bounty_contract,
        "bounty_id": bounty_id,
        "solver": solver,
        "solver_payout": solver_payout,
        "transaction_hash": tx_hash,
        "block_number": block_number,
        "safe_block": safe_block,
        "log_index": log_index,
    }, []


def evaluate_direct_bounty_evidence(payload: Mapping[str, Any]) -> dict[str, Any]:
    """Validate direct-bounty evidence and return machine-readable + human-readable checklist."""
    if not isinstance(payload, Mapping):
        return {
            "schema": CHECKLIST_SCHEMA,
            "ok": False,
            "errors": ["evidence_payload_must_be_object"],
        }

    errors: list[str] = []

    repository = str(payload.get("repository", "")).strip()
    if not REPO_RE.match(repository):
        errors.append("invalid_repository")

    commit_sha = str(payload.get("commit_sha", "")).strip().lower()
    if not COMMIT_SHA_RE.match(commit_sha):
        errors.append("invalid_commit_sha")

    subdirectory = str(payload.get("subdirectory", "")).strip().strip("/")
    if (
        not subdirectory
        or ".." in subdirectory.split("/")
        or not SAFE_SUBDIR_RE.match(subdirectory)
    ):
        errors.append("invalid_or_missing_subdirectory")

    pull_request_url = str(payload.get("pull_request_url", "")).strip()
    if REPO_RE.match(repository):
        errors.extend(_validate_repo_bound_pr_url(pull_request_url, repository))
    else:
        errors.append("invalid_pull_request_url")

    check_run_urls = payload.get("check_run_urls")
    if REPO_RE.match(repository):
        errors.extend(_validate_repo_bound_check_run_urls(check_run_urls, repository))
    else:
        errors.append("missing_check_run_urls")

    artifact_digest = str(payload.get("artifact_digest", "")).strip().lower()
    if not SHA256_DIGEST_RE.match(artifact_digest):
        errors.append("invalid_or_missing_artifact_digest")

    artifact_url = str(payload.get("artifact_url", "")).strip()
    errors.extend(
        _validate_immutable_artifact_url(
            artifact_url,
            commit_sha=commit_sha if COMMIT_SHA_RE.match(commit_sha) else "",
            artifact_digest=artifact_digest if SHA256_DIGEST_RE.match(artifact_digest) else "",
        )
    )

    submission_section = {
        "repository": repository,
        "commit_sha": commit_sha,
        "subdirectory": subdirectory,
        "pull_request_url": pull_request_url,
    }
    submission_hash = sha256_hex32(submission_section)

    verification_section = {
        "check_run_urls": list(check_run_urls) if isinstance(check_run_urls, list) else [],
        "artifact_url": artifact_url,
        "artifact_digest": artifact_digest,
    }
    evidence_hash = sha256_hex32(
        {
            "submission_hash": submission_hash,
            "verification": verification_section,
        }
    )

    # Payment evidence is strictly separated from submission and verification
    raw_payment = payload.get("payment")
    payment_section: dict[str, Any] = {
        "settled": False,
        "paid": False,
        "boundary": "Only a confirmed canonical BountySettled event on Base Mainnet proves payment.",
    }
    if raw_payment is not None:
        if not isinstance(raw_payment, Mapping):
            errors.append("invalid_payment_object")
        else:
            verified_payment, pay_errors = _validate_verified_settlement(
                raw_payment,
                expected_submission_hash=submission_hash,
                expected_evidence_hash=evidence_hash,
            )
            if pay_errors:
                errors.extend(pay_errors)
            elif verified_payment is not None:
                payment_section = verified_payment

    deduped_errors = list(dict.fromkeys(errors))
    ok = len(deduped_errors) == 0

    markdown_lines = [
        f"### Direct Bounty Evidence Checklist ({'PASS' if ok else 'FAIL'})",
        f"- [x] **Submission** (`{submission_hash[:14]}...`): `{repository}@{commit_sha[:12]}` (`{subdirectory}`) — PR: {pull_request_url}"
        if ok
        else f"- [ ] **Submission**: validation errors present",
        f"- [x] **Verification** (`{evidence_hash[:14]}...`): {len(verification_section['check_run_urls'])} check run(s), artifact `{artifact_digest[:23]}...`"
        if ok
        else f"- [ ] **Verification**: validation errors present",
        f"- [{'x' if payment_section.get('paid') else ' '}] **Payment (Canonical BountySettled)**: "
        + (
            f"Settled in tx `{payment_section['transaction_hash'][:14]}...` (log `{payment_section['log_index']}`)"
            if payment_section.get("paid")
            else "Unsettled (PR/check-run/verifier output is NOT payment evidence)"
        ),
    ]

    return {
        "schema": CHECKLIST_SCHEMA,
        "ok": ok,
        "submission": {
            **submission_section,
            "submission_hash": submission_hash,
        },
        "verification": {
            **verification_section,
            "evidence_hash": evidence_hash,
        },
        "payment": payment_section,
        "markdown": "\n".join(markdown_lines),
        "errors": deduped_errors,
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path, help="Path to evidence JSON")
    args = parser.parse_args(argv)
    payload = json.loads(args.input.read_text(encoding="utf-8"))
    result = evaluate_direct_bounty_evidence(payload)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
