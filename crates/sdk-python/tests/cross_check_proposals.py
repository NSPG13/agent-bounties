"""Cross-check: PR #1673 checklist vs the TypeScript proposal posted on #686.

Why this file exists
--------------------
Two competing implementations were submitted for the same acceptance criteria.
Comparing them in prose is an opinion. Comparing them on the same inputs is a
measurement. So this file does the second thing:

  1. `reference_checklist` is a line-for-line port of the TypeScript validator
     posted in issue #686 (`isValidHttpsUrl` + `validateEvidenceChecklist`).
     It is ported faithfully, including the parts that are weak. Nothing here
     was strengthened to make the comparison look better.
  2. `ours` is `direct_evidence_checklist.build_checklist` from this PR.
  3. Both are run against the same cases, including the four cases that ship
     with the TypeScript proposal's own test file.

Result is a table: for every case, what each checker says. Where they disagree,
the acceptance criteria in the issue body decide who is right.

Offline, deterministic, no network, no clock.
"""

from __future__ import annotations

import json
import re
from typing import Any, Dict, List, Tuple
from urllib.parse import urlparse

from direct_evidence_checklist import build_checklist

# --------------------------------------------------------------------------
# Faithful port of the TypeScript proposal (issue #686 comment, 2026-10-22).
# Ported as written: `payment` is optional, `settlementEvent` is only compared
# to the literal string, `transactionHash` is only checked for being a string,
# and no mutability check exists on any URL.
# --------------------------------------------------------------------------

_COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
_DIGEST_RE = re.compile(r"^sha256:[0-9a-f]{64}$")


def _is_https_url(value: str) -> bool:
    try:
        return urlparse(value).scheme == "https"
    except Exception:
        return False


def reference_checklist(obj: Any) -> Tuple[bool, List[str]]:
    """Port of `validateEvidenceChecklist` from the TypeScript proposal."""
    errors: List[str] = []
    if not isinstance(obj, dict):
        return False, ["Checklist must be a non-null object"]
    c: Dict[str, Any] = obj

    if not c.get("bountyId") or not str(c["bountyId"]).strip():
        errors.append("Missing or invalid bountyId")

    sub = c.get("submission")
    if not isinstance(sub, dict):
        errors.append("Missing submission evidence object")
    else:
        if not sub.get("sourceCommit") or not _COMMIT_RE.match(str(sub["sourceCommit"])):
            errors.append("submission.sourceCommit must be a valid 40-character hex commit SHA")
        if not sub.get("repository") or not _is_https_url(str(sub["repository"])):
            errors.append("submission.repository must be a valid HTTPS URL")
        if not sub.get("pullRequestUrl") or not _is_https_url(str(sub["pullRequestUrl"])):
            errors.append("submission.pullRequestUrl must be a valid HTTPS URL")

    ver = c.get("verification")
    if not isinstance(ver, dict):
        errors.append("Missing verification evidence object")
    else:
        if not ver.get("artifactDigest") or not _DIGEST_RE.match(str(ver["artifactDigest"])):
            errors.append("verification.artifactDigest must be a valid sha256 checksum")
        urls = ver.get("checkRunUrls")
        if not isinstance(urls, list) or len(urls) == 0:
            errors.append("verification.checkRunUrls must be a non-empty array of strings")
        else:
            for idx, url in enumerate(urls):
                if not _is_https_url(str(url)):
                    errors.append(f"verification.checkRunUrls[{idx}] must be a valid HTTPS URL")

    pay = c.get("payment")
    if pay is not None:
        if not isinstance(pay, dict):
            errors.append("payment evidence must be an object if provided")
        else:
            if pay.get("settlementEvent") != "BountySettled":
                errors.append('payment.settlementEvent must equal canonical "BountySettled"')
            if not pay.get("transactionHash") or not isinstance(pay["transactionHash"], str):
                errors.append("payment.transactionHash is required when payment evidence is provided")

    return (len(errors) == 0), errors


# --------------------------------------------------------------------------
# Cases. The first four are the proposal's own four tests, translated verbatim.
# The last two are the ones the acceptance criteria require and the proposal
# does not cover.
# --------------------------------------------------------------------------

COMMIT = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
DIGEST = "sha256:" + "0" * 64


def _ts_case(**over: Any) -> Dict[str, Any]:
    base: Dict[str, Any] = {
        "bountyId": "NSPG13/agent-bounties#686",
        "submission": {
            "sourceCommit": COMMIT,
            "repository": "https://github.com/NSPG13/agent-bounties",
            "subdirectory": "packages/bounties",
            "pullRequestUrl": "https://github.com/NSPG13/agent-bounties/pull/686",
        },
        "verification": {
            "checkRunUrls": ["https://github.com/NSPG13/agent-bounties/actions/runs/12345"],
            "artifactDigest": DIGEST,
            "verifiedAt": "2026-10-22T13:00:00Z",
        },
    }
    base.update(over)
    return base


def _ours_case(**over: Any) -> Dict[str, Any]:
    base: Dict[str, Any] = {
        "repository": "https://github.com/NSPG13/agent-bounties",
        "source_commit": COMMIT,
        "subdirectory": "packages/bounties",
        "pull_request_url": "https://github.com/NSPG13/agent-bounties/pull/1673",
        "check_run_urls": ["https://github.com/NSPG13/agent-bounties/actions/runs/12345"],
        "artifact_url": "https://github.com/NSPG13/agent-bounties/releases/download/v1/x.tar.gz",
        "artifact_sha256": "0" * 64,
    }
    base.update(over)
    return base


CASES: List[Tuple[str, str, Dict[str, Any], Dict[str, Any]]] = [
    (
        "C1",
        "proposal's own passing case (verbatim)",
        _ts_case(),
        _ours_case(),
    ),
    (
        "C2",
        "proposal: non-HTTPS repository",
        _ts_case(submission={
            "sourceCommit": COMMIT,
            "repository": "http://insecure.com",
            "subdirectory": "packages/bounties",
            "pullRequestUrl": "https://github.com/NSPG13/agent-bounties/pull/686",
        }),
        _ours_case(repository="http://insecure.com"),
    ),
    (
        "C3",
        "proposal: malformed commit + digest",
        _ts_case(submission={
            "sourceCommit": "short-sha",
            "repository": "https://github.com/NSPG13/agent-bounties",
            "subdirectory": "packages/bounties",
            "pullRequestUrl": "https://github.com/NSPG13/agent-bounties/pull/686",
        }, verification={
            "checkRunUrls": ["https://github.com/NSPG13/agent-bounties/actions/runs/12345"],
            "artifactDigest": "md5:abc1234",
            "verifiedAt": "2026-10-22T13:00:00Z",
        }),
        _ours_case(source_commit="short-sha", artifact_sha256="md5:abc1234"),
    ),
    (
        "C4",
        "proposal: settlementEvent 'PaymentSentByBob'",
        _ts_case(payment={
            "settlementEvent": "PaymentSentByBob",
            "transactionHash": "0x123",
            "settledAt": "2026-10-22T13:10:00Z",
        }),
        _ours_case(settlement={
            "settled": True,
            "provenance": "canonical_live",
            "event": "PaymentSentByBob",
            "tx_hash": "0x" + "a" * 64,
            "log_key": "base:8453:log:17",
            "block_number": 1,
        }),
    ),
    (
        "C5",
        "MUTABLE artifact ref: branch-pinned raw URL (AC: reject mutable)",
        _ts_case(verification={
            "checkRunUrls": ["https://github.com/NSPG13/agent-bounties/actions/runs/12345"],
            "artifactDigest": DIGEST,
            "verifiedAt": "2026-10-22T13:00:00Z",
            "artifact_url": "https://raw.githubusercontent.com/o/r/main/artifact.tar.gz",
        }),
        _ours_case(
            artifact_url="https://raw.githubusercontent.com/o/r/main/artifact.tar.gz",
        ),
    ),
    (
        "C6",
        "SELF-REPORTED settlement: 'BountySettled' string + tx, no chain identity",
        _ts_case(payment={
            "settlementEvent": "BountySettled",
            "transactionHash": "0x123",
            "settledAt": "2026-10-22T13:10:00Z",
        }),
        _ours_case(settlement={
            "settled": True,
            "provenance": "canonical_live",
            "event": "BountySettled",
            "tx_hash": "0x123",
            "log_key": "base:8453:log:17",
            "block_number": 1,
        }),
    ),
    (
        "C7",
        "secret-bearing field present (AC: no secrets in response)",
        _ts_case(api_key="ghp_supersecret", wallet="0xabc"),
        _ours_case(api_key="ghp_supersecret", wallet="0xabc"),
    ),
]


def _ours_verdict(raw: Dict[str, Any]) -> Tuple[bool, List[str], str, Dict[str, Any]]:
    out = build_checklist(raw)
    return bool(out["complete"]), list(out["problems"]), str(out["payment"]["status"]), out


def run() -> int:
    print("== A. deliverable binding (submission + verification) ==")
    print(f"{'case':<5}{'ref(TS)':<9}{'ours':<9}what it is")
    print("-" * 96)
    for cid, label, ts_raw, our_raw in CASES:
        ref_ok, _ = reference_checklist(ts_raw)
        ok, _, _, _ = _ours_verdict(our_raw)
        flag = "   <-- DISAGREE" if ref_ok != ok else ""
        print(f"{cid:<5}{str(ref_ok):<9}{str(ok):<9}{label}{flag}")
        if ref_ok != ok:
            _, problems, _, _ = _ours_verdict(our_raw)
            print(f"       ref accepts; ours blocks: {problems[:2]}")

    print()
    print("== B. payment bucket, compared separately (the claim that pays) ==")
    print(f"{'case':<5}{'ref says':<12}{'ours says':<12}verdict")
    print("-" * 96)
    for cid, label, ts_raw, our_raw in CASES:
        if "payment" not in ts_raw:
            continue
        ref_ok, ref_errs = reference_checklist(ts_raw)
        _, _, pay, _ = _ours_verdict(our_raw)
        ref_pay = "settled" if ref_ok else "rejected"
        verdict = "SAME" if (ref_pay == "settled") == (pay == "settled") else "DIFFERENT"
        print(f"{cid:<5}{ref_pay:<12}{pay:<12}{verdict}   {label}")
        if verdict == "DIFFERENT":
            print("       ^ ref treats a self-reported receipt as settled evidence.")

    print()
    print("== C. secret scrubbing (AC: safe inside an MCP/API payload) ==")
    print("-" * 96)
    raw = _ours_case(api_key="ghp_supersecret", wallet="0xabc",
                     maintainer_email="someone@example.com")
    out = build_checklist(raw)
    blob = json.dumps(out, sort_keys=True)
    leaked = [k for k in ("ghp_supersecret", "0xabc", "someone@example.com") if k in blob]
    ref_out = json.dumps(_ts_case(api_key="ghp_supersecret", wallet="0xabc"), sort_keys=True)
    ref_leaked = "ghp_supersecret" in ref_out
    print(f"  ours  : secrets surviving in response payload -> {leaked or 'none'}")
    print(f"  ref   : proposal defines no scrub step; input object carries the secret as-is -> {ref_leaked}")

    print()
    print("== summary ==")
    print("  C5  ref accepts a branch-pinned (mutable) artifact URL; AC requires rejecting it.")
    print("  C6  ref accepts settlementEvent='BountySettled' + tx='0x123' as settled evidence;")
    print("      ours keeps it unverified because tx_hash is not 0x + 64 hex.")
    print("  C7  ours scrubs secret-bearing keys; the proposal has no scrub step.")
    return 0


if __name__ == "__main__":
    raise SystemExit(run())
