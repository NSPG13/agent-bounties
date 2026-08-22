#!/usr/bin/env python3
"""Versioned stalled-bounty diagnostics from canonical lifecycle events (#873).

Gives each claimed or submitted bounty exactly one deterministic recovery
action before claim or verification deadlines strand solver work and escrow.

Source-of-truth contract: classification, next actions, and deadlines are
derived exclusively from canonical lifecycle events (BountyClaimed,
BountySubmitted, BountySettled -- the canonical settlement event, sometimes
written lowercased as bountysettled -- and terminal events) plus the
immutable claim/verification windows from the bounty terms. The diagnostic
never infers acceptance, rejection, or payment from GitHub issue state, a
transaction hash, or an AI opinion.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

DIAGNOSTIC_VERSION = "stalled-bounty-diagnostics-v1"

# Canonical lifecycle events that may drive a diagnostic.
EVENT_CLAIMED = "BountyClaimed"
EVENT_SUBMITTED = "BountySubmitted"
EVENT_SETTLED = "BountySettled"
EVENT_EXPIRED = "BountyExpired"
EVENT_REFUNDED = "BountyRefunded"
TERMINAL_EVENTS = frozenset({EVENT_EXPIRED, EVENT_REFUNDED})

# Grace applied before a deadline is reported as expiring.
EXPIRY_GRACE_SECONDS = 3600
# Stale-index threshold for the machine-readable discovery feed.
INDEX_MAX_AGE_SECONDS = 6 * 3600


@dataclass(frozen=True)
class LifecycleTerms:
    """Immutable windows from the canonical bounty terms."""

    claim_window_seconds: int | None
    verification_window_seconds: int | None


def _parse_ts(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def _event_at(events: list[dict[str, Any]], name: str) -> datetime | None:
    for event in events:
        if event.get("name") == name:
            return _parse_ts(str(event["at"]))
    return None


def diagnose(
    item: dict[str, Any],
    *,
    now: str | None = None,
    index_max_age_seconds: int = INDEX_MAX_AGE_SECONDS,
) -> dict[str, Any]:
    """Classify one bounty and return its single deterministic next action.

    ``now`` is injectable for determinism; it defaults to current UTC time.
    """

    now_dt = _parse_ts(now) if now else datetime.now(timezone.utc)
    bounty_id = str(item.get("bounty_id", "unknown"))
    events: list[dict[str, Any]] = item.get("canonical_events", [])
    terms_raw = item.get("terms", {})
    terms = LifecycleTerms(
        claim_window_seconds=terms_raw.get("claim_window_seconds"),
        verification_window_seconds=terms_raw.get("verification_window_seconds"),
    )
    verifier_available = bool(item.get("verifier_available", True))
    index_updated_at = item.get("index_updated_at")

    claimed_at = _event_at(events, EVENT_CLAIMED)
    submitted_at = _event_at(events, EVENT_SUBMITTED)
    settled_at = _event_at(events, EVENT_SETTLED)
    terminal_at = _event_at(events, EVENT_EXPIRED) or _event_at(events, EVENT_REFUNDED)

    blockers: list[str] = []

    # Data-quality blockers never override canonical classification, but they
    # are surfaced so operators can fix the feed.
    if index_updated_at is not None:
        index_age = now_dt - _parse_ts(index_updated_at)
        if index_age.total_seconds() > index_max_age_seconds:
            blockers.append("stale index data: refresh the discovery feed")
    if terms.claim_window_seconds is None and terms.verification_window_seconds is None:
        blockers.append("missing terms: restore immutable claim/verification windows")

    # --- Terminal outcomes first ----------------------------------------
    if settled_at is not None:
        return _result(bounty_id, "settled", "none", None, events, terms, blockers, now_dt)
    if terminal_at is not None:
        return _result(bounty_id, "terminal", "none", None, events, terms, blockers, now_dt)

    # --- Claimed-but-not-submitted --------------------------------------
    if claimed_at is not None and submitted_at is None:
        window = terms.claim_window_seconds
        if window is None:
            return _result(
                bounty_id,
                "claim_expiring",
                "restore_terms",
                None,
                events,
                terms,
                blockers + ["claim window missing from terms"],
                now_dt,
            )
        deadline = claimed_at + timedelta(seconds=window)
        remaining = (deadline - now_dt).total_seconds()
        if remaining <= EXPIRY_GRACE_SECONDS:
            return _result(
                bounty_id,
                "claim_expiring",
                "escalate_claim",
                deadline.isoformat(),
                events,
                terms,
                blockers,
                now_dt,
            )
        return _result(
            bounty_id,
            "healthy_claimed",
            "wait_for_submission",
            deadline.isoformat(),
            events,
            terms,
            blockers,
            now_dt,
        )

    # --- Submitted, verification pending --------------------------------
    if submitted_at is not None and settled_at is None:
        window = terms.verification_window_seconds
        if not verifier_available:
            return _result(
                bounty_id,
                "verifier_unavailable",
                "schedule_verifier",
                None,
                events,
                terms,
                blockers,
                now_dt,
            )
        if window is None:
            return _result(
                bounty_id,
                "verification_expiring",
                "restore_terms",
                None,
                events,
                terms,
                blockers + ["verification window missing from terms"],
                now_dt,
            )
        deadline = submitted_at + timedelta(seconds=window)
        remaining = (deadline - now_dt).total_seconds()
        if remaining <= EXPIRY_GRACE_SECONDS:
            return _result(
                bounty_id,
                "verification_expiring",
                "escalate_verification",
                deadline.isoformat(),
                events,
                terms,
                blockers,
                now_dt,
            )
        return _result(
            bounty_id,
            "submitted",
            "wait_for_verification",
            deadline.isoformat(),
            events,
            terms,
            blockers,
            now_dt,
        )

    return _result(
        bounty_id,
        "terminal",
        "investigate_missing_lifecycle",
        None,
        events,
        terms,
        blockers + ["no canonical lifecycle events for this bounty"],
        now_dt,
    )


def _result(
    bounty_id: str,
    classification: str,
    next_action: str,
    deadline: str | None,
    events: list[dict[str, Any]],
    terms: LifecycleTerms,
    blockers: list[str],
    now_dt: datetime,
) -> dict[str, Any]:
    return {
        "diagnostic_version": DIAGNOSTIC_VERSION,
        "generated_at": now_dt.isoformat(),
        "bounty_id": bounty_id,
        "classification": classification,
        "next_action": next_action,
        "deadline": deadline,
        "blockers": blockers,
        "evidence": {
            "canonical_events": [e["name"] for e in events],
            "windows": {
                "claim_window_seconds": terms.claim_window_seconds,
                "verification_window_seconds": terms.verification_window_seconds,
            },
        },
        "disclaimer": (
            "classification derives only from canonical lifecycle events and "
            "immutable windows; no acceptance, rejection, or payment is "
            "inferred from GitHub state, transaction hashes, or AI opinion"
        ),
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--item", required=True, help="bounty item JSON (see scripts/fixtures/stalled-bounty-diagnostics/)")
    parser.add_argument("--now", default=None, help="ISO timestamp override for deterministic runs")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    item = json.loads(Path(args.item).read_text(encoding="utf-8"))
    result = diagnose(item, now=args.now)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
