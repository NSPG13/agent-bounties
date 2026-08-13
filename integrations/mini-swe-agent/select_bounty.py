#!/usr/bin/env python3
"""mini-SWE-agent bounty selector.

Reads a JSON fixture (via --input) that represents a canonical inventory
snapshot and selects one exact next action: claim, wait, refresh, or skip.

Determinism: an optional --now ISO-8601 timestamp pins the reference clock so
tests are reproducible. Fixtures that are meant to be "fresh" use timestamps far
in the future relative to the pinned clock; "stale" fixtures use timestamps far
in the past.
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

STALE_AFTER_SECONDS = 86400  # 24 hours


def _parse_iso(value: str) -> datetime:
    """Parse an ISO-8601 timestamp into an aware UTC datetime."""
    if value.endswith("Z"):
        value = value[:-1] + "+00:00"
    dt = datetime.fromisoformat(value)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def signed_gross_margin(bounty: dict) -> int | None:
    """Signed gross cash margin = reward - bond - external spend.

    Returns None (fail closed) when the economics fields are missing or
    malformed, so such entries are never selected.
    """
    try:
        reward = int(bounty.get("reward", {}).get("amount", 0))
        bond = int(bounty.get("bond", {}).get("amount", 0))
        spend = bounty.get("external_spend")
        if isinstance(spend, dict):
            external = int(spend.get("amount", 0))
        else:
            external = int(spend or 0)
    except (TypeError, ValueError, AttributeError):
        return None
    return reward - bond - external


def select_action(inventory: dict, now: datetime | None = None) -> dict:
    """Return the single next action for the given inventory snapshot."""
    bounties = inventory.get("bounties", [])

    if not bounties:
        return {
            "action": "wait",
            "next_action": "No claimable bounties found in inventory; retry after next refresh cycle.",
        }

    reference = now if now is not None else datetime.now(timezone.utc)

    fresh = []
    for b in bounties:
        updated = b.get("canonical_updated_at", b.get("updated_at", ""))
        if not updated:
            continue
        try:
            ts = _parse_iso(updated)
            if (reference - ts).total_seconds() < STALE_AFTER_SECONDS:
                fresh.append(b)
        except (ValueError, TypeError):
            continue

    if bounties and not fresh:
        return {
            "action": "refresh",
            "next_action": "All inventory entries are stale (>24h); trigger inventory refresh before selecting.",
        }

    eligible = []
    for b in fresh:
        if b.get("exclusive_claimant") or b.get("claimant"):
            continue
        margin = signed_gross_margin(b)
        if margin is None or margin <= 0:
            continue
        eligible.append(b)

    if fresh and not eligible:
        has_exclusive = any(b.get("exclusive_claimant") or b.get("claimant") for b in fresh)
        if has_exclusive:
            return {
                "action": "skip",
                "next_action": "All available bounties have exclusive claimants; skip this cycle.",
            }
        return {
            "action": "skip",
            "next_action": "No positive signed-margin bounties available; skip this cycle.",
        }

    if not eligible:
        return {
            "action": "wait",
            "next_action": "No eligible claimable bounties after filtering; retry after refresh.",
        }

    best = max(eligible, key=lambda b: signed_gross_margin(b))
    margin = signed_gross_margin(best)
    reward = best.get("reward", {})
    return {
        "action": "claim",
        "next_action": (
            f"Claim bounty #{best.get('number', '?')}: {best.get('title', 'Untitled')} "
            f"(signed gross margin {margin} {reward.get('currency', 'USDC')})"
        ),
        "selected_bounty": best.get("number"),
        "selected_title": best.get("title"),
        "signed_gross_margin": margin,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="mini-SWE-agent bounty selector")
    parser.add_argument("--input", required=True, type=Path, help="JSON fixture path")
    parser.add_argument("--now", default=None, help="Optional ISO-8601 reference timestamp (deterministic tests)")
    args = parser.parse_args()

    now = None
    if args.now:
        try:
            now = _parse_iso(args.now)
        except (ValueError, TypeError) as error:
            print(json.dumps({"action": "wait", "next_action": f"Invalid --now timestamp: {error}"}))
            sys.exit(1)

    try:
        inventory = json.loads(args.input.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as error:
        print(json.dumps({"action": "wait", "next_action": f"Failed to read inventory: {error}"}))
        sys.exit(1)

    result = select_action(inventory, now)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
