#!/usr/bin/env python3
"""Select exactly one next action for a mini-SWE-agent inventory snapshot.

Usage:
    python select_bounty.py --input <fixture.json>

Reads a canonical inventory fixture and prints a single JSON object:

    {"action": "claim|wait|refresh|skip", "next_action": "..."}

Decision order (deterministic):
    1. empty inventory          -> wait
    2. all items stale          -> refresh
    3. any exclusive claimant   -> skip (respect exclusive claimants)
    4. no positive margin       -> skip
    5. otherwise                -> claim (highest-margin eligible item)
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


STALE_WINDOW_SECONDS = 3600


def _parse_iso(value: str) -> datetime:
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def _margin_of(item: dict) -> float:
    raw = item.get("margin_usdc", 0)
    try:
        return float(raw)
    except (TypeError, ValueError):
        return 0.0


def decide(fixture: dict) -> dict:
    observed_at = _parse_iso(str(fixture.get("observed_at", "1970-01-01T00:00:00Z")))
    items = fixture.get("inventory", [])
    if not isinstance(items, list) or not items:
        return {
            "action": "wait",
            "next_action": (
                "Inventory is empty. Poll the canonical hosted inventory at the "
                "next interval and wait for newly funded claimable work before "
                "attempting a claim."
            ),
        }

    # Stale detection: every item's funding update older than the window.
    all_stale = all(
        (observed_at - _parse_iso(str(item.get("funding_updated_at", "")))).total_seconds()
        > STALE_WINDOW_SECONDS
        for item in items
        if item.get("funding_updated_at")
    )
    if all_stale:
        return {
            "action": "refresh",
            "next_action": (
                "Inventory is stale. Refresh the canonical inventory snapshot "
                "and re-evaluate before claiming so the selected bounty is "
                "still funded and claimable."
            ),
        }

    # Respect exclusive claimants: if any item is exclusively claimed by a
    # wallet that is not the operator, do not race it.
    if any(item.get("exclusive_claimant") for item in items):
        return {
            "action": "skip",
            "next_action": (
                "Inventory contains work with an exclusive claimant. Skip it "
                "and re-poll for openly claimable bounties with positive margin."
            ),
        }

    # Margin: only claim work whose solver reward exceeds its cost.
    eligible = [
        item for item in items
        if bool(item.get("claimable", False))
        and bool(item.get("verification_ready", False))
        and _margin_of(item) > 0
    ]
    if not eligible:
        return {
            "action": "skip",
            "next_action": (
                "No claimable work has positive margin. Skip this inventory "
                "and wait for a funded bounty whose reward exceeds its cost."
            ),
        }

    target = max(eligible, key=_margin_of)
    return {
        "action": "claim",
        "next_action": (
            f"Claim bounty {target.get('bounty_id', 'unknown')} via "
            "agent_native_claim with the operator wallet, wait for the "
            "canonical claim state, then implement only that issue and submit "
            "verification-ready evidence."
        ),
        "selected_bounty_id": target.get("bounty_id", "unknown"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="path to inventory fixture JSON")
    args = parser.parse_args()

    fixture = json.loads(Path(args.input).read_text(encoding="utf-8"))
    result = decide(fixture)
    json.dump(result, sys.stdout)
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
