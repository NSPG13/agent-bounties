#!/usr/bin/env python3
"""Mini-SWE-Agent canonical bounty selector.

Selects one canonically claimable coding bounty with positive cash margin,
no conflicting exclusive claimant, and fresh inventory state.
"""
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional


def load_inventory(path: str = "fixtures/multiple.json") -> list[dict]:
    """Load bounty inventory from a JSON fixture file."""
    fixture_path = Path(__file__).parent / path
    if not fixture_path.exists():
        print(f"ERROR: inventory file not found: {fixture_path}", file=sys.stderr)
        sys.exit(1)
    with open(fixture_path) as f:
        return json.load(f)


def is_fresh(bounty: dict, max_age_hours: int = 24) -> bool:
    """Check if bounty was created within max_age_hours."""
    created = bounty.get("created_at", "")
    if not created:
        return False
    try:
        created_dt = datetime.fromisoformat(created.replace("Z", "+00:00"))
        age = datetime.now(timezone.utc) - created_dt
        return age.total_seconds() < max_age_hours * 3600
    except (ValueError, TypeError):
        return False


def has_positive_margin(bounty: dict) -> bool:
    """Bounty must have positive cash margin (reward > bond)."""
    reward = float(bounty.get("reward_usdc", 0))
    bond = float(bounty.get("bond_usdc", 0))
    return reward > bond and reward > 0


def has_no_exclusive_claimant(bounty: dict) -> bool:
    """No other exclusive claimant should hold the bounty."""
    claimants = bounty.get("claimants", [])
    if not claimants:
        return True
    # Check if any claimant has exclusive rights
    for c in claimants:
        if c.get("exclusive", False):
            return False
    return True


def is_canonical(bounty: dict) -> bool:
    """Only canonical (on-chain verified) bounties are eligible."""
    return bounty.get("canonical", False) and bounty.get("state") == "claimable-live"


def select_bounty(inventory_path: str = "fixtures/multiple.json") -> Optional[dict]:
    """Select the best eligible bounty from inventory.

    Returns None if no eligible bounty found (fail-closed).
    """
    inventory = load_inventory(inventory_path)

    if not inventory:
        print("INFO: empty inventory — no bounties available")
        return None

    eligible = []
    for b in inventory:
        checks = {
            "canonical": is_canonical(b),
            "fresh": is_fresh(b),
            "margin": has_positive_margin(b),
            "no_exclusive": has_no_exclusive_claimant(b),
        }
        if all(checks.values()):
            eligible.append((b, checks))
        else:
            failed = [k for k, v in checks.items() if not v]
            print(f"SKIP #{b.get('id','?')}: failed checks={failed}")

    if not eligible:
        print("INFO: no eligible bounty found — fail-closed")
        return None

    # Sort by reward descending, then by freshness
    eligible.sort(key=lambda x: (
        -float(x[0].get("reward_usdc", 0)),
        x[0].get("created_at", ""),
    ))

    selected = eligible[0][0]
    print(f"SELECTED: #{selected['id']} — {selected.get('title','?')} "
          f"(${selected.get('reward_usdc',0)} USDC)")
    return selected


def emit_evidence(bounty: dict, output_dir: str = "evidence") -> str:
    """Emit verification-ready evidence for the selected bounty."""
    os.makedirs(output_dir, exist_ok=True)
    evidence = {
        "bounty_id": bounty.get("id"),
        "selected_at": datetime.now(timezone.utc).isoformat(),
        "reward_usdc": bounty.get("reward_usdc"),
        "title": bounty.get("title"),
        "state": bounty.get("state"),
        "sandbox_version": "1.0",
        "checks_passed": {
            "canonical": True,
            "fresh_inventory": True,
            "positive_margin": True,
            "no_exclusive_claimant": True,
        },
    }
    path = os.path.join(output_dir, f"evidence_{bounty.get('id')}.json")
    with open(path, "w") as f:
        json.dump(evidence, f, indent=2)
    print(f"EVIDENCE: written to {path}")
    return path


def main():
    import argparse
    parser = argparse.ArgumentParser(description="Mini-SWE-Agent bounty selector")
    parser.add_argument(
        "--inventory", default="fixtures/multiple.json",
        help="Path to inventory fixture (default: fixtures/multiple.json)"
    )
    parser.add_argument(
        "--output-dir", default="evidence",
        help="Evidence output directory (default: evidence)"
    )
    parser.add_argument(
        "--fail-open", action="store_true",
        help="Exit 0 even when no bounty found (default: fail-closed with exit 1)"
    )
    args = parser.parse_args()

    bounty = select_bounty(args.inventory)

    if bounty is None:
        if args.fail_open:
            print("INFO: fail-open mode — exiting 0")
            sys.exit(0)
        else:
            print("ERROR: fail-closed — no eligible bounty", file=sys.stderr)
            sys.exit(1)

    emit_evidence(bounty, args.output_dir)
    print(f"SUCCESS: selected bounty #{bounty['id']}")
    sys.exit(0)


if __name__ == "__main__":
    main()
