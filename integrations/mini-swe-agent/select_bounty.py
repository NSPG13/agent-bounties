#!/usr/bin/env python3
"""mini-SWE-agent bounty selector.

Reads a JSON fixture (via --input) that represents a canonical inventory snapshot
and selects one exact next action: claim, wait, refresh, or skip.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def select_action(inventory: dict) -> dict:
    """Return the single next action for the given inventory snapshot."""
    bounties = inventory.get("bounties", [])

    # Empty: nothing to claim
    if not bounties:
        return {
            "action": "wait",
            "next_action": "No claimable bounties found in inventory; retry after next refresh cycle.",
        }

    # Filter out stale entries (older than 24h)
    from datetime import datetime, timezone
    now = datetime.now(timezone.utc)
    fresh = []
    for b in bounties:
        updated = b.get("canonical_updated_at", b.get("updated_at", ""))
        if not updated:
            continue
        try:
            ts = datetime.fromisoformat(updated.replace("Z", "+00:00"))
            if (now - ts).total_seconds() < 86400:
                fresh.append(b)
        except ValueError:
            continue

    if bounties and not fresh:
        return {
            "action": "refresh",
            "next_action": "All inventory entries are stale (>24h); trigger inventory refresh before selecting.",
        }

    # Filter out no-margin and exclusive-claimant entries
    eligible = []
    for b in fresh:
        # Skip if already has an exclusive claimant
        if b.get("exclusive_claimant") or b.get("claimant"):
            continue
        # Skip if no margin (reward <= bond)
        reward = int(b.get("reward", {}).get("amount", 0))
        bond = int(b.get("bond", {}).get("amount", 0))
        if reward <= bond:
            continue
        eligible.append(b)

    if fresh and not eligible:
        # Check if the issue is exclusivity or no-margin
        has_exclusive = any(b.get("exclusive_claimant") or b.get("claimant") for b in fresh)
        if has_exclusive:
            return {
                "action": "skip",
                "next_action": "All available bounties have exclusive claimants; skip this cycle.",
            }
        return {
            "action": "skip",
            "next_action": "No positive-margin bounties available; skip this cycle.",
        }

    if not eligible:
        return {
            "action": "wait",
            "next_action": "No eligible claimable bounties after filtering; retry after refresh.",
        }

    # Select highest-margin bounty
    best = max(eligible, key=lambda b: int(b.get("reward", {}).get("amount", 0)))
    return {
        "action": "claim",
        "next_action": f"Claim bounty #{best.get('number', '?')}: {best.get('title', 'Untitled')} "
                       f"({best.get('reward', {}).get('amount', '0')} {best.get('reward', {}).get('currency', 'USDC')})",
        "selected_bounty": best.get("number"),
        "selected_title": best.get("title"),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="mini-SWE-agent bounty selector")
    parser.add_argument("--input", required=True, type=Path, help="JSON fixture path")
    args = parser.parse_args()

    try:
        inventory = json.loads(args.input.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError) as error:
        print(json.dumps({"action": "wait", "next_action": f"Failed to read inventory: {error}"}))
        sys.exit(1)

    result = select_action(inventory)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
