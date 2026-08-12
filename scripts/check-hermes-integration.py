#!/usr/bin/env python3
"""Deterministic smoke check for the Hermes Agent Bounties integration."""

from __future__ import annotations

import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "integrations" / "hermes" / "fixtures"
EXPECTED = {
    "claimable": "prepare_canonical_claim",
    "unfunded": "wait_for_canonical_funding",
    "stale": "refresh_canonical_inventory",
}


def load_fixture(name: str) -> dict:
    path = FIXTURES / f"{name}.json"
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema_version") != "agent-bounties/hermes-discovery-fixture-v1":
        raise SystemExit(f"{name}: wrong schema_version")
    if value.get("state") != name:
        raise SystemExit(f"{name}: state mismatch")
    actions = value.get("next_actions")
    if actions != [EXPECTED[name]]:
        raise SystemExit(f"{name}: expected exactly one deterministic next action")
    if value.get("canonical_state_required") is not True:
        raise SystemExit(f"{name}: canonical state boundary missing")
    return value


def main() -> None:
    fixtures = {name: load_fixture(name) for name in EXPECTED}
    if fixtures["claimable"].get("work_authorized") is not True:
        raise SystemExit("claimable: work authorization missing")
    for name in ("unfunded", "stale"):
        if fixtures[name].get("work_authorized") is not False:
            raise SystemExit(f"{name}: unsafe work authorization")
    print("Hermes Agent Bounties integration smoke passed")


if __name__ == "__main__":
    main()
