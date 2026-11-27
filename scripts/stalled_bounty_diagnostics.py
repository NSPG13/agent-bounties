#!/usr/bin/env python3
"""Stalled-bounty diagnostics (#873).

Classifies every claimed or submitted bounty into exactly one stall class and
returns one deterministic next action plus a deadline.

Canonical rule: nothing here infers acceptance, rejection or payment from
GitHub state, a transaction hash, or an AI opinion. Only a canonical
BountySettled event proves payment; BOUNTYSETTLED is the single key that
carries that fact.
"""
from __future__ import annotations

SCHEMA_VERSION = "stalled-bounty-diagnostics/v1"

# Terminal / non-terminal classification keys used by callers.
HEALTHY_CLAIMED = "healthy_claimed"
CLAIM_EXPIRING = "claim_expiring"
SUBMITTED = "submitted"
VERIFICATION_EXPIRING = "verification_expiring"
VERIFIER_UNAVAILABLE = "verifier_unavailable"
SETTLED = "settled"
TERMINAL = "terminal"

# The only canonical payment fact this module is allowed to trust.
BOUNTYSETTLED = "BountySettled"

TERMINAL_STATUSES = {"settled", "cancelled", "expired", "refunded"}


def classify(row: dict, now: int) -> str:
    """Return exactly one stall class for a claimed/submitted bounty."""
    status = (row.get("status") or "").lower()
    if status in TERMINAL_STATUSES:
        return SETTLED if status == "settled" else TERMINAL
    claim_deadline = row.get("claim_deadline")
    verification_deadline = row.get("verification_deadline")
    verifier_ready = bool(row.get("verification_ready", False))

    if status == "claimed":
        if claim_deadline is None:
            # Missing terms: never guess a deadline, force a blocker instead.
            return HEALTHY_CLAIMED if verifier_ready else VERIFIER_UNAVAILABLE
        remaining = int(claim_deadline) - int(now)
        return CLAIM_EXPIRING if remaining <= int(row.get("expiring_within", 0)) else HEALTHY_CLAIMED

    if status == "submitted":
        if not verifier_ready:
            return VERIFIER_UNAVAILABLE
        if verification_deadline is None:
            return SUBMITTED
        remaining = int(verification_deadline) - int(now)
        return VERIFICATION_EXPIRING if remaining <= int(row.get("expiring_within", 0)) else SUBMITTED
    return TERMINAL


def next_action_for(stall_class: str) -> str:
    return {
        HEALTHY_CLAIMED: "continue_work_until_claim_deadline",
        CLAIM_EXPIRING: "submit_evidence_before_claim_deadline",
        SUBMITTED: "await_canonical_verification",
        VERIFICATION_EXPIRING: "request_verifier_or_prepare_recovery_before_verification_deadline",
        VERIFIER_UNAVAILABLE: "escalate_verifier_unavailable_and_preserve_escrow",
        SETTLED: "none",
        TERMINAL: "none",
    }[stall_class]


# Immutable windows (seconds). When a row omits its own deadline, the window
# anchored at the observed lifecycle timestamp supplies the deadline, so every
# non-terminal stalled item can always report one.
DEFAULT_WINDOWS = {"claim_window": 72 * 3600, "verification_window": 72 * 3600}


def resolve_deadline(row: dict, stall_class: str, windows: dict) -> int | None:
    """Deadline from explicit terms first, then from an immutable window."""
    direct = (row.get("claim_deadline") if stall_class in (HEALTHY_CLAIMED, CLAIM_EXPIRING)
              else row.get("verification_deadline"))
    if direct is None:
        direct = row.get("claim_deadline") or row.get("verification_deadline")
    if direct is not None:
        return int(direct)
    anchor = row.get("claimed_at") if stall_class in (HEALTHY_CLAIMED, CLAIM_EXPIRING) \
        else row.get("submitted_at")
    if anchor is None:
        anchor = row.get("claimed_at") or row.get("submitted_at")
    if anchor is None:
        return None
    span = (windows.get("claim_window") if stall_class in (HEALTHY_CLAIMED, CLAIM_EXPIRING)
            else windows.get("verification_window"))
    return int(anchor) + int(span)


def diagnose(rows: list, now: int, canonical_events: list | None = None,
             windows: dict | None = None) -> dict:
    """Build a versioned diagnostic over canonical lifecycle events."""
    events = canonical_events or []
    win = dict(DEFAULT_WINDOWS)
    if windows:
        win.update(windows)
    paid_ids = {e.get("bounty_id") for e in events if e.get("event") == BOUNTYSETTLED}
    items, backlog = [], []
    for row in rows:
        stall_class = classify(row, now)
        paid = row.get("bounty_id") in paid_ids
        deadline = resolve_deadline(row, stall_class, win)
        item = {"bounty_id": row.get("bounty_id"),
                "stall_class": stall_class,
                "next_action": next_action_for(stall_class),
                "deadline": deadline,
                "settled": bool(row.get("settled", False)) or paid,
                "evidence_stale": bool(row.get("evidence_stale", False)),
                "missing_terms": row.get("claim_deadline") is None
                and row.get("verification_deadline") is None}
        items.append(item)
        if stall_class not in (SETTLED, TERMINAL):
            backlog.append(item)
    return {"schema": SCHEMA_VERSION, "observed_at": int(now),
            "canonical_events": len(events), "items": items, "backlog": backlog}
