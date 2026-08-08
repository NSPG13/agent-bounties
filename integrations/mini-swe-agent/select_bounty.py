#!/usr/bin/env python3
"""Select one canonically claimable coding bounty from a fixture inventory."""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def select_bounty(inventory: list[dict]) -> dict:
    """Select best bounty from inventory using claim planning rules."""
    if not inventory:
        return {"action": "wait", "next_action": "No bounties available — wait and retry"}

    # Multiple bounties: pick the highest-margin one
    claimable = [
        b for b in inventory
        if b.get("status") in ("claimable-live", "funded-live")
        and b.get("margin_usdc", 0) > 0
        and not b.get("exclusive_claimant")
    ]

    if len(claimable) > 1:
        best = max(claimable, key=lambda b: b.get("margin_usdc", 0))
        return {
            "action": "claim",
            "bounty_id": best.get("id", ""),
            "margin_usdc": best.get("margin_usdc", 0),
            "next_action": f"Claim bounty {best.get('id')} with {best.get('margin_usdc')} USDC margin and implement"
        }

    # Single claimable
    if len(claimable) == 1:
        b = claimable[0]
        return {
            "action": "claim",
            "bounty_id": b.get("id", ""),
            "margin_usdc": b.get("margin_usdc", 0),
            "next_action": f"Claim bounty {b.get('id')} with {b.get('margin_usdc')} USDC margin and implement"
        }

    # Check for blocked bounties
    stale = [b for b in inventory if b.get("status") == "stale"]
    if stale:
        return {"action": "refresh", "next_action": "Stale bounties detected — refresh inventory"}

    no_margin = [b for b in inventory if b.get("margin_usdc", 0) <= 0]
    if no_margin and not claimable:
        return {"action": "skip", "next_action": "No positive-margin bounties — skip cycle"}

    exclusive = [b for b in inventory if b.get("exclusive_claimant")]
    if exclusive and not claimable:
        return {"action": "skip", "next_action": "All bounties have exclusive claimants — skip"}

    return {"action": "wait", "next_action": "No actionable bounties — wait for next cycle"}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--input", required=True, help="Path to fixture JSON")
    args = parser.parse_args()

    fixture_path = Path(args.input)
    if not fixture_path.is_file():
        print(json.dumps({"action": "wait", "next_action": f"Fixture not found: {args.input}"}))
        sys.exit(0)

    with open(fixture_path, encoding="utf-8") as f:
        inventory = json.load(f)

    if not isinstance(inventory, list):
        print(json.dumps({"action": "wait", "next_action": "Invalid inventory format"}))
        sys.exit(0)

    result = select_bounty(inventory)
    print(json.dumps(result))
    sys.exit(0)


if __name__ == "__main__":
    main()
