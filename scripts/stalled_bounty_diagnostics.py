#!/usr/bin/env python3
"""Diagnose stalled claimed and submitted bounties with deterministic recovery actions.

Lifecycle states and recovery actions are computed strictly from canonical Base
events and immutable contract windows. Invariant: Never infer acceptance,
rejection, or payment from GitHub state, a transaction hash, or an AI opinion.
Only a confirmed canonical BountySettled event proves solver payment.
"""

from __future__ import annotations

import argparse
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import re
import sys
from typing import Any, Sequence

DIAGNOSTIC_SCHEMA = "agent-bounties/stalled-bounty-diagnostics-v1"
DIAGNOSTIC_VERSION = 1

# Classification types
HEALTHY_CLAIMED = "healthy_claimed"
CLAIM_EXPIRING = "claim_expiring"
SUBMITTED = "submitted"
VERIFICATION_EXPIRING = "verification_expiring"
VERIFIER_UNAVAILABLE = "verifier_unavailable"
SETTLED = "settled"
TERMINAL = "terminal"
MISSING_TERMS = "missing_terms"
STALE_INDEXER = "stale_indexer"

# Deterministic next recovery actions
ACTION_SUBMIT_WORK = "submit_work"
ACTION_EXPIRE_CLAIM = "expire_claim"
ACTION_VERIFY_SUBMISSION = "verify_submission"
ACTION_EXPIRE_SUBMISSION = "expire_submission"
ACTION_RESTORE_VERIFIERS = "restore_verifier_fleet"
ACTION_RECONCILE_TERMS = "reconcile_terms"
ACTION_SYNC_INDEXER = "sync_indexer"
ACTION_WITHDRAW_REFUND = "withdraw_refund"

EVIDENCE_BOUNDARY = (
    "Canonical autonomous-v1 lifecycle diagnostics derive exclusively from confirmed on-chain "
    "events, immutable contract timing parameters, and attested verifier fleet telemetry. "
    "GitHub issues, pull requests, comments, labels, broadcast transaction hashes, and advisory AI judge "
    "opinions can never authorize, prove, or simulate payment. Only a confirmed canonical BountySettled event proves settlement."
)

ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]{40}$")
BYTES32_RE = re.compile(r"^0x[0-9a-fA-F]{64}$")


class DiagnosticError(ValueError):
    pass


def format_iso_timestamp(timestamp_seconds: int | float | None) -> str | None:
    if timestamp_seconds is None or timestamp_seconds <= 0:
        return None
    try:
        return datetime.fromtimestamp(int(timestamp_seconds), tz=timezone.utc).isoformat()
    except (ValueError, OverflowError, OSError):
        return None


def parse_timestamp_argument(value: str | int | float | None) -> int:
    if value is None:
        return int(datetime.now(timezone.utc).timestamp())
    if isinstance(value, (int, float)):
        return int(value)
    value_str = str(value).strip()
    if value_str.isdigit():
        return int(value_str)
    try:
        # Parse ISO-8601
        dt = datetime.fromisoformat(value_str.replace("Z", "+00:00"))
        return int(dt.timestamp())
    except (ValueError, TypeError):
        raise DiagnosticError(f"Invalid timestamp string: {value_str}")


@dataclass(frozen=True)
class BountyDiagnostic:
    bounty_id: str
    bounty_contract: str
    classification: str
    status: str
    round: int
    solver: str | None
    active_claim_bond: int
    claim_expires_at: int | None
    verification_expires_at: int | None
    next_action: str | None
    next_action_label: str | None
    next_action_instructions: str | None
    deadline: str | None
    deadline_unix: int | None
    is_stalled: bool
    settlement_evidence: dict[str, Any] | None
    verifier_status: dict[str, Any] | None
    indexer_status: dict[str, Any] | None
    diagnosed_at: str

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


@dataclass(frozen=True)
class DiagnosticsReport:
    schema: str
    version: int
    generated_at: str
    observed_timestamp: int
    total_diagnosed: int
    counts: dict[str, int]
    stalled_backlog: list[dict[str, Any]]
    items: list[dict[str, Any]]
    evidence_boundary: str

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def check_bountysettled_canonical(item: dict[str, Any]) -> tuple[bool, dict[str, Any] | None]:
    """Validate whether canonical BountySettled evidence is confirmed."""
    settlement = item.get("settlement_evidence")
    if isinstance(settlement, dict):
        event_name = settlement.get("event_name")
        confirmed = settlement.get("confirmed_canonical", False)
        if event_name == "BountySettled" and confirmed:
            return True, settlement

    # Check in canonical event log if available
    events = item.get("events", [])
    if isinstance(events, list):
        for event in events:
            if isinstance(event, dict):
                kind = event.get("kind") or event.get("event_name")
                if kind == "BountySettled":
                    confirmed = event.get("confirmed_canonical", True)
                    if confirmed:
                        evidence = {
                            "event_name": "BountySettled",
                            "bounty_id": item.get("bounty_id"),
                            "bounty_contract": item.get("bounty_contract"),
                            "round": event.get("round", item.get("round", 1)),
                            "solver": event.get("solver", item.get("solver")),
                            "solver_reward": str(event.get("solver_reward", item.get("solver_reward", 0))),
                            "claim_bond_returned": str(event.get("claim_bond_returned", 0)),
                            "timeout_bond_bonus": str(event.get("timeout_bond_bonus", 0)),
                            "verifier_reward": str(event.get("verifier_reward", item.get("verifier_reward", 0))),
                            "transaction_hash": event.get("transaction_hash", ""),
                            "confirmed_canonical": True,
                        }
                        return True, evidence

    return False, None


def diagnose_bounty(
    item: dict[str, Any],
    current_timestamp: int | None = None,
    verifier_fleet_healthy: bool = True,
    indexer_fresh: bool = True,
) -> BountyDiagnostic:
    """Diagnose a single bounty record and derive its classification, deterministic next action, and deadline."""
    now = current_timestamp if current_timestamp is not None else int(datetime.now(timezone.utc).timestamp())
    diagnosed_at_iso = datetime.fromtimestamp(now, tz=timezone.utc).isoformat()

    bounty_id = str(item.get("bounty_id", "")).strip().lower()
    bounty_contract = str(item.get("bounty_contract", "")).strip().lower()
    status = str(item.get("status", "")).strip().lower()
    round_num = int(item.get("round", 1))
    solver = item.get("solver")
    if solver:
        solver = str(solver).strip().lower()
    active_claim_bond = int(item.get("active_claim_bond", 0))

    claim_expires_at = item.get("claim_expires_at")
    if claim_expires_at is not None:
        claim_expires_at = int(claim_expires_at)

    verification_expires_at = item.get("verification_expires_at")
    if verification_expires_at is not None:
        verification_expires_at = int(verification_expires_at)

    # Inspect indexer health
    indexer_info = item.get("indexer_status")
    is_indexer_stale = not indexer_fresh
    if isinstance(indexer_info, dict):
        if indexer_info.get("state") == "stale":
            is_indexer_stale = True
        if indexer_info.get("lag_blocks", 0) > 100:
            is_indexer_stale = True
        if indexer_info.get("heartbeat_age_seconds", 0) > 900:
            is_indexer_stale = True
        if indexer_info.get("cursor_monotonic") is False:
            is_indexer_stale = True

    # Inspect verifier fleet health
    verifier_info = item.get("verifier_fleet")
    is_verifier_unavailable = not verifier_fleet_healthy or (item.get("verification_ready") is False and status in {"submitted", "claimable"})
    if isinstance(verifier_info, dict):
        if verifier_info.get("state") in {"unavailable", "outage", "degraded", "unhealthy"}:
            is_verifier_unavailable = True
        if verifier_info.get("consecutive_failures", 0) > 0:
            is_verifier_unavailable = True
        if verifier_info.get("unready_claimable_count", 0) > 0:
            is_verifier_unavailable = True

    # Check canonical settlement
    has_settled, settlement_evidence = check_bountysettled_canonical(item)
    if status == "settled" or has_settled:
        if has_settled:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=SETTLED,
                status="settled",
                round=round_num,
                solver=solver,
                active_claim_bond=0,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=None,
                next_action_label="Settled",
                next_action_instructions="Bounty has been canonically settled via BountySettled event. No recovery action needed.",
                deadline=None,
                deadline_unix=None,
                is_stalled=False,
                settlement_evidence=settlement_evidence,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )
        else:
            # Marked settled without canonical evidence -> flag as invalid/stalled indexer state
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=STALE_INDEXER,
                status=status,
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_SYNC_INDEXER,
                next_action_label="Sync Indexer State",
                next_action_instructions="Settlement state lacks canonical BountySettled event. Sync indexer from Base RPC.",
                deadline=None,
                deadline_unix=None,
                is_stalled=True,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )

    # Check indexer staleness before evaluating time windows
    if is_indexer_stale:
        return BountyDiagnostic(
            bounty_id=bounty_id,
            bounty_contract=bounty_contract,
            classification=STALE_INDEXER,
            status=status,
            round=round_num,
            solver=solver,
            active_claim_bond=active_claim_bond,
            claim_expires_at=claim_expires_at,
            verification_expires_at=verification_expires_at,
            next_action=ACTION_SYNC_INDEXER,
            next_action_label="Sync Indexer and Base Events",
            next_action_instructions="Indexer data is stale or degraded. Refresh read models from canonical Base chain RPC.",
            deadline=None,
            deadline_unix=None,
            is_stalled=True,
            settlement_evidence=None,
            verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
            indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
            diagnosed_at=diagnosed_at_iso,
        )

    # Check missing or invalid terms
    terms = item.get("terms")
    terms_valid = item.get("terms_valid", True)
    if terms is None or not terms_valid:
        return BountyDiagnostic(
            bounty_id=bounty_id,
            bounty_contract=bounty_contract,
            classification=MISSING_TERMS,
            status=status,
            round=round_num,
            solver=solver,
            active_claim_bond=active_claim_bond,
            claim_expires_at=claim_expires_at,
            verification_expires_at=verification_expires_at,
            next_action=ACTION_RECONCILE_TERMS,
            next_action_label="Reconcile Published Terms Document",
            next_action_instructions="Bounty terms document is missing or invalid. Re-publish and verify terms hash against contract termsHash.",
            deadline=None,
            deadline_unix=None,
            is_stalled=True,
            settlement_evidence=None,
            verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
            indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
            diagnosed_at=diagnosed_at_iso,
        )

    # Classify Claimed Bounties
    if status == "claimed":
        exp_time = claim_expires_at or 0
        deadline_iso = format_iso_timestamp(exp_time)
        # Boundary logic:
        # In Solidity: require(block.timestamp > claimExpiresAt, "claim not expired") for expireClaim.
        # And require(block.timestamp <= claimExpiresAt, "claim expired") for submit.
        # Thus, if now > claimExpiresAt, the claim has expired and can be expired.
        if now > exp_time:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=CLAIM_EXPIRING,
                status="claimed",
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_EXPIRE_CLAIM,
                next_action_label="Expire Stalled Claim",
                next_action_instructions="Claim deadline has passed. Call expireClaim() to forfeit claim bond to timeout pool and reopen bounty.",
                deadline=deadline_iso,
                deadline_unix=exp_time,
                is_stalled=True,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )
        else:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=HEALTHY_CLAIMED,
                status="claimed",
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_SUBMIT_WORK,
                next_action_label="Submit Work and Evidence",
                next_action_instructions="Active claim is within window. Solver must call submit() with submissionHash and evidenceHash before deadline.",
                deadline=deadline_iso,
                deadline_unix=exp_time,
                is_stalled=False,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )

    # Classify Submitted Bounties
    if status == "submitted":
        exp_time = verification_expires_at or 0
        deadline_iso = format_iso_timestamp(exp_time)

        # Verifier outage check
        if is_verifier_unavailable:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=VERIFIER_UNAVAILABLE,
                status="submitted",
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_RESTORE_VERIFIERS,
                next_action_label="Restore Verifier Fleet",
                next_action_instructions="Verifier fleet is experiencing an outage or unready. Recover verifiers to prevent verification timeout.",
                deadline=deadline_iso,
                deadline_unix=exp_time,
                is_stalled=True,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )

        # Boundary logic:
        # In Solidity: require(block.timestamp > verificationExpiresAt, "submission not expired") for expireSubmission.
        if now > exp_time:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=VERIFICATION_EXPIRING,
                status="submitted",
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_EXPIRE_SUBMISSION,
                next_action_label="Expire Unverified Submission",
                next_action_instructions="Verification deadline passed without quorum. Call expireSubmission() to refund solver bond and reset bounty.",
                deadline=deadline_iso,
                deadline_unix=exp_time,
                is_stalled=True,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )
        else:
            return BountyDiagnostic(
                bounty_id=bounty_id,
                bounty_contract=bounty_contract,
                classification=SUBMITTED,
                status="submitted",
                round=round_num,
                solver=solver,
                active_claim_bond=active_claim_bond,
                claim_expires_at=claim_expires_at,
                verification_expires_at=verification_expires_at,
                next_action=ACTION_VERIFY_SUBMISSION,
                next_action_label="Verify Submission and Attest",
                next_action_instructions="Submission is pending verification. Verifiers must run test suites and submit signed attestations.",
                deadline=deadline_iso,
                deadline_unix=exp_time,
                is_stalled=False,
                settlement_evidence=None,
                verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
                indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
                diagnosed_at=diagnosed_at_iso,
            )

    # Classify Terminal / Cancelled / Other
    if status == "cancelled":
        return BountyDiagnostic(
            bounty_id=bounty_id,
            bounty_contract=bounty_contract,
            classification=TERMINAL,
            status="cancelled",
            round=round_num,
            solver=solver,
            active_claim_bond=0,
            claim_expires_at=claim_expires_at,
            verification_expires_at=verification_expires_at,
            next_action=ACTION_WITHDRAW_REFUND if int(item.get("funded_amount", 0)) > 0 else None,
            next_action_label="Withdraw Refund" if int(item.get("funded_amount", 0)) > 0 else "Terminal",
            next_action_instructions="Bounty is cancelled. Contributors may withdraw pull refunds." if int(item.get("funded_amount", 0)) > 0 else "Bounty is cancelled and settled.",
            deadline=None,
            deadline_unix=None,
            is_stalled=False,
            settlement_evidence=None,
            verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
            indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
            diagnosed_at=diagnosed_at_iso,
        )

    # Default fallback for open / claimable / other
    return BountyDiagnostic(
        bounty_id=bounty_id,
        bounty_contract=bounty_contract,
        classification=status if status in {"open", "claimable"} else "unspecified",
        status=status,
        round=round_num,
        solver=solver,
        active_claim_bond=active_claim_bond,
        claim_expires_at=claim_expires_at,
        verification_expires_at=verification_expires_at,
        next_action="claim" if status == "claimable" else "inspect",
        next_action_label="Inspect Canonical State",
        next_action_instructions="Inspect confirmed canonical events. GitHub state is not settlement evidence.",
        deadline=format_iso_timestamp(claim_expires_at) if claim_expires_at else None,
        deadline_unix=claim_expires_at,
        is_stalled=False,
        settlement_evidence=None,
        verifier_status=verifier_info if isinstance(verifier_info, dict) else None,
        indexer_status=indexer_info if isinstance(indexer_info, dict) else None,
        diagnosed_at=diagnosed_at_iso,
    )


def diagnose_backlog(
    items: Sequence[dict[str, Any]],
    current_timestamp: int | None = None,
    verifier_fleet_healthy: bool = True,
    indexer_fresh: bool = True,
) -> DiagnosticsReport:
    """Diagnose an array of bounty records and produce a versioned operations report."""
    now = current_timestamp if current_timestamp is not None else int(datetime.now(timezone.utc).timestamp())
    generated_at = datetime.fromtimestamp(now, tz=timezone.utc).isoformat()

    diagnosed_items: list[BountyDiagnostic] = []
    counts: dict[str, int] = {
        HEALTHY_CLAIMED: 0,
        CLAIM_EXPIRING: 0,
        SUBMITTED: 0,
        VERIFICATION_EXPIRING: 0,
        VERIFIER_UNAVAILABLE: 0,
        SETTLED: 0,
        TERMINAL: 0,
        MISSING_TERMS: 0,
        STALE_INDEXER: 0,
        "stalled_total": 0,
    }

    stalled_backlog: list[dict[str, Any]] = []

    for raw_item in items:
        if not isinstance(raw_item, dict):
            continue
        diag = diagnose_bounty(
            raw_item,
            current_timestamp=now,
            verifier_fleet_healthy=verifier_fleet_healthy,
            indexer_fresh=indexer_fresh,
        )
        diagnosed_items.append(diag)
        classification = diag.classification
        counts[classification] = counts.get(classification, 0) + 1
        if diag.is_stalled:
            counts["stalled_total"] += 1
            stalled_backlog.append(diag.to_dict())

    return DiagnosticsReport(
        schema=DIAGNOSTIC_SCHEMA,
        version=DIAGNOSTIC_VERSION,
        generated_at=generated_at,
        observed_timestamp=now,
        total_diagnosed=len(diagnosed_items),
        counts=counts,
        stalled_backlog=stalled_backlog,
        items=[d.to_dict() for d in diagnosed_items],
        evidence_boundary=EVIDENCE_BOUNDARY,
    )


def render_markdown_report(report: DiagnosticsReport) -> str:
    """Render a human-readable operational markdown report."""
    lines = [
        "# Stalled Bounty Diagnostics Report",
        "",
        f"- **Schema**: `{report.schema}` (v{report.version})",
        f"- **Generated At**: `{report.generated_at}`",
        f"- **Observed Timestamp**: `{report.observed_timestamp}`",
        f"- **Total Diagnosed**: `{report.total_diagnosed}`",
        f"- **Stalled Backlog Count**: `{report.counts.get('stalled_total', 0)}`",
        "",
        "## Summary Counts",
        "",
        "| Classification | Count | Description |",
        "| :--- | :--- | :--- |",
        f"| `healthy_claimed` | {report.counts.get(HEALTHY_CLAIMED, 0)} | Claimed work actively progressing within deadline |",
        f"| `claim_expiring` | {report.counts.get(CLAIM_EXPIRING, 0)} | Claim window expired; ready for bond forfeiture / reset |",
        f"| `submitted` | {report.counts.get(SUBMITTED, 0)} | Submission actively awaiting verifier quorum |",
        f"| `verification_expiring` | {report.counts.get(VERIFICATION_EXPIRING, 0)} | Verification window expired; ready for bond refund / reset |",
        f"| `verifier_unavailable` | {report.counts.get(VERIFIER_UNAVAILABLE, 0)} | Verifier fleet offline or degraded |",
        f"| `settled` | {report.counts.get(SETTLED, 0)} | Canonically settled via confirmed BountySettled |",
        f"| `terminal` | {report.counts.get(TERMINAL, 0)} | Cancelled or fully settled |",
        f"| `missing_terms` | {report.counts.get(MISSING_TERMS, 0)} | Terms document missing or invalid |",
        f"| `stale_indexer` | {report.counts.get(STALE_INDEXER, 0)} | Stale indexer data requiring RPC sync |",
        "",
    ]

    if report.stalled_backlog:
        lines.extend([
            "## Stalled Action Backlog",
            "",
            "| Bounty Contract | Status | Classification | Next Action | Deadline |",
            "| :--- | :--- | :--- | :--- | :--- |",
        ])
        for item in report.stalled_backlog:
            contract = item.get("bounty_contract", "unknown")
            status = item.get("status", "unknown")
            cls = item.get("classification", "unknown")
            action = item.get("next_action", "none")
            deadline = item.get("deadline") or "none"
            lines.append(f"| `{contract}` | `{status}` | `{cls}` | `{action}` | `{deadline}` |")
        lines.append("")
    else:
        lines.extend([
            "## Stalled Action Backlog",
            "",
            "No stalled items detected. All active claims and submissions are within healthy windows.",
            "",
        ])

    lines.extend([
        "## Evidence Boundary",
        "",
        "> " + report.evidence_boundary,
        "",
    ])

    return "\n".join(lines)


def load_bounty_items(source: str | Path | dict | list) -> list[dict[str, Any]]:
    """Load bounty records from file path, JSON string, or dict/list."""
    if isinstance(source, list):
        return source
    if isinstance(source, dict):
        if "items" in source and isinstance(source["items"], list):
            return source["items"]
        if "bounties" in source and isinstance(source["bounties"], list):
            return source["bounties"]
        return [source]

    path = Path(source)
    if path.is_file():
        content = json.loads(path.read_text(encoding="utf-8"))
        return load_bounty_items(content)

    # Try json parse string
    content = json.loads(str(source))
    return load_bounty_items(content)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Diagnose stalled claimed and submitted bounties from canonical lifecycle data."
    )
    parser.add_argument(
        "--input",
        "-i",
        help="Input JSON file or '-' for stdin",
        default="-",
    )
    parser.add_argument(
        "--output",
        "-o",
        help="Output JSON diagnostics file",
        default=None,
    )
    parser.add_argument(
        "--markdown-output",
        "-m",
        help="Output markdown report file",
        default=None,
    )
    parser.add_argument(
        "--now",
        help="Override current unix timestamp or ISO string for boundary evaluation",
        default=None,
    )
    parser.add_argument(
        "--verifier-outage",
        action="store_true",
        help="Simulate/declare verifier fleet outage",
    )
    parser.add_argument(
        "--stale-indexer",
        action="store_true",
        help="Simulate/declare stale indexer data",
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Run non-zero exit check if critical stalled items are present",
    )

    args = parser.parse_args()

    if args.input == "-":
        raw_text = sys.stdin.read()
        items = load_bounty_items(raw_text)
    else:
        items = load_bounty_items(args.input)

    current_ts = parse_timestamp_argument(args.now) if args.now else None
    report = diagnose_backlog(
        items,
        current_timestamp=current_ts,
        verifier_fleet_healthy=not args.verifier_outage,
        indexer_fresh=not args.stale_indexer,
    )

    report_dict = report.to_dict()
    rendered_json = json.dumps(report_dict, indent=2)
    rendered_md = render_markdown_report(report)

    if args.output:
        out_path = Path(args.output)
        out_path.parent.mkdir(parents=True, exist_ok=True)
        out_path.write_text(rendered_json, encoding="utf-8")

    if args.markdown_output:
        md_path = Path(args.markdown_output)
        md_path.parent.mkdir(parents=True, exist_ok=True)
        md_path.write_text(rendered_md, encoding="utf-8")

    if not args.output and not args.markdown_output:
        print(rendered_json)

    if args.check:
        stalled_count = report.counts.get("stalled_total", 0)
        if stalled_count > 0:
            sys.exit(1)


if __name__ == "__main__":
    main()
