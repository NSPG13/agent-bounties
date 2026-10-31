#!/usr/bin/env python3
"""Concise direct-bounty evidence checklist and validator (Issue #686).

Binds the repository, exact source commit, mandatory subdirectory, repository-bound
pull request URL, repository-bound check-run URLs, immutable credential-free HTTPS
artifact URL + artifact digest, and the canonical Base Mainnet `BountySettled`
settlement boundary.

Explicitly separates:
1. `submission` evidence (repository, commit_sha, subdirectory, pull_request_url, submission_hash)
2. `verification` evidence (check_run_urls, artifact_url, artifact_digest, evidence_hash)
3. `payment` evidence (confirmed canonical `BountySettled` event verified via RPC at a Base
   safe block; solver-supplied receipt JSON without verified chain RPC proof never becomes
   `paid: true`)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import parse_qs, urlsplit


CHECKLIST_SCHEMA = "agent-bounties/direct-bounty-evidence-checklist-v1"
BASE_MAINNET_CHAIN_ID = 8453
NATIVE_BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
CANONICAL_BOUNTY_FACTORIES = frozenset(
    {
        "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9",
        "0xc33e2ae33bb9580837ea59df18e57fa1039ae58a",
    }
)
# Canonical keccak256 topic0 for BountySettled(bytes32,uint64,address,uint256,uint256,uint256,uint256,bytes32,bytes32,bytes32,bytes32)
BOUNTY_SETTLED_TOPIC0 = (
    "0x2ee7b1323824324868017119c9922527f2e2c8eb20a8044c3dd574bdbfb2dd71"
)
IS_CANONICAL_BOUNTY_SELECTOR = "0xdb021126"

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
    "/current/",
    "/unstable/",
    "/nightly/",
    "/canary/",
    "/refs/heads/",
    "/refs/tags/",
    "/branch/",
    "/branches/",
    "/tag/",
    "/tags/",
    "/releases/download/",
    "/releases/latest",
    "/tree/main",
    "/tree/master",
    "/blob/main/",
    "/blob/master/",
    "/raw/main/",
    "/raw/master/",
)

RpcCallable = Callable[[str, list[Any]], Any]


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
    repository: str,
    commit_sha: str,
    artifact_digest: str,
) -> list[str]:
    """Validate that artifact_url is an immutable, content-addressed HTTPS URI.

    Mere substring occurrence of a digest or commit SHA inside an arbitrary or
    mutable URL (e.g. `/releases/download/v1.0/<digest>.tar.gz` or
    `https://example.com/builds/<digest>.zip`) is NOT proof of immutability and
    is rejected.
    """
    parsed, errors = _validate_https_no_credentials(url, "artifact_url")
    if errors or parsed is None:
        return errors

    host_lower = (parsed.hostname or "").lower()
    path_lower = parsed.path.lower()

    if parsed.query:
        errors.append("mutable_or_query_artifact_url")
    if any(seg in path_lower for seg in MUTABLE_URL_SEGMENTS):
        errors.append("mutable_artifact_url")
        return errors

    digest_hex = artifact_digest.removeprefix("sha256:").lower()
    repo_lower = repository.lower()

    # Strict allowlist of genuinely immutable, content-addressed or repository-pinned HTTPS structures:
    # 1. Exact CAS path `/sha256/<exact-64-hex-digest>` (must match artifact_digest)
    cas_match = re.search(r"/sha256/([0-9a-f]{64})(?:/[a-z0-9_.-]+)?$", path_lower)
    is_exact_cas = bool(cas_match and digest_hex and cas_match.group(1) == digest_hex)

    # 2. GitHub raw/archive URL pinned to the exact repository and exact 40-hex commit_sha
    is_pinned_github_commit = False
    if commit_sha and repo_lower:
        if host_lower == "raw.githubusercontent.com":
            is_pinned_github_commit = path_lower.startswith(f"/{repo_lower}/{commit_sha.lower()}/")
        elif host_lower == "github.com":
            is_pinned_github_commit = (
                path_lower.startswith(f"/{repo_lower}/raw/{commit_sha.lower()}/")
                or path_lower == f"/{repo_lower}/archive/{commit_sha.lower()}.tar.gz"
            )

    # 3. Repository-bound GitHub Actions run artifact ID (`https://github.com/<owner>/<repo>/actions/runs/<run_id>/artifacts/<artifact_id>`)
    is_repo_actions_artifact = bool(
        host_lower == "github.com"
        and repo_lower
        and re.fullmatch(
            rf"/{re.escape(repo_lower)}/actions/runs/[1-9][0-9]*/artifacts/[1-9][0-9]*",
            path_lower,
        )
    )

    if not (is_exact_cas or is_pinned_github_commit or is_repo_actions_artifact):
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


def _decode_settled_log_words(data_hex: Any) -> dict[str, Any] | None:
    """Decode non-indexed ABI words from a canonical BountySettled log entry."""
    if not isinstance(data_hex, str) or not data_hex.startswith("0x"):
        return None
    body = data_hex[2:].lower()
    if len(body) != 64 * 8 or any(c not in "0123456789abcdef" for c in body):
        return None
    words = [body[i * 64 : (i + 1) * 64] for i in range(8)]
    return {
        "solver_reward": int(words[0], 16),
        "claim_bond_returned": int(words[1], 16),
        "timeout_bond_bonus": int(words[2], 16),
        "verifier_reward": int(words[3], 16),
        "submission_hash": "0x" + words[4],
        "evidence_hash": "0x" + words[5],
        "policy_hash": "0x" + words[6],
        "verification_hash": "0x" + words[7],
    }


def _validate_verified_settlement(
    payment: Mapping[str, Any],
    *,
    expected_submission_hash: str,
    expected_evidence_hash: str,
    rpc_call: RpcCallable | None = None,
) -> tuple[dict[str, Any] | None, list[str]]:
    """Validate canonical Base Mainnet BountySettled receipt proof via independent RPC verification.

    Self-reported or fabricated `receipt` JSON inside `payment` is NEVER trusted
    to set `paid: true`. `paid: true` requires `rpc_call` to confirm:
    1. Chain ID == 8453 (Base Mainnet)
    2. `bounty_contract` is registered on a canonical AgentBountyFactory (`isCanonicalBounty`)
    3. `eth_getTransactionReceipt(transaction_hash)` succeeded (`status == 1`) at `blockNumber <= safe_block`
    4. The receipt logs contain the exact `BountySettled` event from `bounty_contract`
       with matching `bounty_id`, `solver`, `submission_hash`, `evidence_hash`, and
       payout conservation math.
    """
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

    factory_addr = str(
        payment.get("bounty_factory", "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9")
    ).strip().lower()
    if factory_addr not in CANONICAL_BOUNTY_FACTORIES:
        errors.append("unverified_receipt_noncanonical_factory")

    # Never trust caller-supplied receipt JSON without independent chain RPC verification
    if rpc_call is None:
        errors.append("unverified_receipt_missing_proof")
        if "receipt" in payment:
            errors.append("unverified_self_reported_receipt_rejected")
        return None, errors

    if errors:
        return None, errors

    try:
        raw_chain = rpc_call("eth_chainId", [])
        chain_id = int(str(raw_chain), 16) if str(raw_chain).startswith("0x") else int(raw_chain)
        if chain_id != BASE_MAINNET_CHAIN_ID:
            return None, ["unverified_receipt_wrong_chain"]

        safe_blk = rpc_call("eth_getBlockByNumber", ["safe", False])
        if not isinstance(safe_blk, Mapping) or safe_blk.get("number") is None:
            return None, ["unverified_receipt_safe_block_unavailable"]
        safe_num_raw = str(safe_blk["number"])
        safe_block = int(safe_num_raw, 16) if safe_num_raw.startswith("0x") else int(safe_num_raw)
        safe_tag = hex(safe_block)

        # Verify bounty_contract is registered on canonical factory at safe_tag
        call_data = IS_CANONICAL_BOUNTY_SELECTOR + ("0" * 24) + bounty_contract[2:]
        is_canon_hex = rpc_call("eth_call", [{"to": factory_addr, "data": call_data}, safe_tag])
        if (
            not isinstance(is_canon_hex, str)
            or len(is_canon_hex) != 66
            or int(is_canon_hex[2:], 16) != 1
        ):
            return None, ["unverified_receipt_noncanonical_bounty_contract"]

        chain_receipt = rpc_call("eth_getTransactionReceipt", [tx_hash])
        if not isinstance(chain_receipt, Mapping):
            return None, ["unverified_receipt_not_found_on_chain"]
    except Exception:
        return None, ["unverified_receipt_rpc_failure"]

    status_raw = chain_receipt.get("status")
    status_int = (
        int(status_raw, 16)
        if isinstance(status_raw, str) and status_raw.startswith("0x")
        else (int(status_raw) if isinstance(status_raw, int) and not isinstance(status_raw, bool) else -1)
    )
    if status_int != 1:
        return None, ["unverified_receipt_failed_status"]

    blk_raw = chain_receipt.get("blockNumber") or chain_receipt.get("block_number")
    block_number = (
        int(blk_raw, 16)
        if isinstance(blk_raw, str) and blk_raw.startswith("0x")
        else (int(blk_raw) if isinstance(blk_raw, int) and not isinstance(blk_raw, bool) else -1)
    )
    if block_number <= 0 or block_number > safe_block:
        return None, ["unverified_receipt_unfinalized_block"]

    logs = chain_receipt.get("logs")
    if not isinstance(logs, list) or not logs:
        return None, ["unverified_receipt_missing_settled_log"]

    expected_solver_topic = "0x" + ("0" * 24) + solver[2:]
    matched_log: Mapping[str, Any] | None = None
    for entry in logs:
        if not isinstance(entry, Mapping):
            continue
        if entry.get("removed") is True:
            continue
        addr = str(entry.get("address", "")).strip().lower()
        topics = entry.get("topics", [])
        if (
            addr == bounty_contract
            and isinstance(topics, list)
            and len(topics) >= 4
            and str(topics[0]).strip().lower() == BOUNTY_SETTLED_TOPIC0
            and str(topics[1]).strip().lower() == bounty_id
            and str(topics[3]).strip().lower() == expected_solver_topic
        ):
            matched_log = entry
            break

    if matched_log is None:
        return None, ["unverified_receipt_canonical_log_mismatch"]

    idx_raw = matched_log.get("logIndex") if "logIndex" in matched_log else matched_log.get("log_index")
    log_index = (
        int(idx_raw, 16)
        if isinstance(idx_raw, str) and idx_raw.startswith("0x")
        else (int(idx_raw) if isinstance(idx_raw, int) and not isinstance(idx_raw, bool) else -1)
    )
    if log_index < 0:
        return None, ["unverified_receipt_canonical_log_mismatch"]

    decoded_data = _decode_settled_log_words(matched_log.get("data"))
    if decoded_data is None:
        return None, ["unverified_receipt_canonical_log_mismatch"]

    if decoded_data["submission_hash"] != expected_submission_hash.lower():
        errors.append("unverified_receipt_submission_hash_mismatch")
    if decoded_data["evidence_hash"] != expected_evidence_hash.lower():
        errors.append("unverified_receipt_evidence_hash_mismatch")

    solver_reward = decoded_data["solver_reward"]
    bond_returned = decoded_data["claim_bond_returned"]
    timeout_bonus = decoded_data["timeout_bond_bonus"]
    solver_payout = solver_reward + bond_returned + timeout_bonus
    if solver_reward <= 0 or decoded_data["verifier_reward"] <= 0:
        errors.append("unverified_receipt_payout_math_mismatch")

    expected_payout = payment.get("solver_payout")
    if expected_payout is not None and (
        isinstance(expected_payout, bool)
        or not isinstance(expected_payout, int)
        or expected_payout != solver_payout
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
        "bounty_factory": factory_addr,
        "bounty_contract": bounty_contract,
        "bounty_id": bounty_id,
        "solver": solver,
        "solver_payout": solver_payout,
        "transaction_hash": tx_hash,
        "block_number": block_number,
        "safe_block": safe_block,
        "log_index": log_index,
    }, []


def evaluate_direct_bounty_evidence(
    payload: Mapping[str, Any],
    *,
    rpc_call: RpcCallable | None = None,
) -> dict[str, Any]:
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
        or "." in subdirectory.split("/")
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
            repository=repository if REPO_RE.match(repository) else "",
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
                rpc_call=rpc_call,
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
    parser.add_argument("--rpc-url", default=None, help="Optional Base HTTPS RPC URL when verifying payment")
    args = parser.parse_args(argv)
    payload = json.loads(args.input.read_text(encoding="utf-8"))

    rpc_fn: RpcCallable | None = None
    if isinstance(payload, Mapping) and payload.get("payment") is not None:
        try:
            from scripts._shared.rpc import rpc as shared_rpc, select_working_base_rpc
        except ImportError:
            from _shared.rpc import rpc as shared_rpc, select_working_base_rpc  # type: ignore
        try:
            endpoint = select_working_base_rpc(preferred=args.rpc_url)
            req_seq = 0

            def _bound_rpc(method: str, params: list[Any]) -> Any:
                nonlocal req_seq
                req_seq += 1
                return shared_rpc(endpoint, method, params, request_id=req_seq)

            rpc_fn = _bound_rpc
        except Exception:
            rpc_fn = None

    result = evaluate_direct_bounty_evidence(payload, rpc_call=rpc_fn)
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
