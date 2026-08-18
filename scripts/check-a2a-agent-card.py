#!/usr/bin/env python3
"""Smoke test for the A2A Agent Card served by this workspace.

Validates the canonical Agent Card fixture and the documented custom binding
without touching the network. Exits non-zero on any failure.
"""
from __future__ import annotations

import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", str(Path.cwd())))


def main() -> int:
    card_path = ROOT / "fixtures" / "a2a-agent-card.json"
    if not card_path.is_file():
        print(f"FAIL: missing {card_path}")
        return 1
    card = json.loads(card_path.read_text(encoding="utf-8"))

    errors = []
    if card.get("name") != "Agent Bounties":
        errors.append("name must be 'Agent Bounties'")
    if not str(card.get("description", "")).strip():
        errors.append("description required")
    if not isinstance(card.get("supportedInterfaces"), list) or not card["supportedInterfaces"]:
        errors.append("supportedInterfaces required")
    for iface in card.get("supportedInterfaces", []):
        url = str(iface.get("url", ""))
        if not url.startswith("https://api.agentbounties.app/"):
            errors.append(f"interface url must use canonical API: {url}")
        if iface.get("protocolVersion") != "1.0":
            errors.append(f"interface must declare A2A 1.0: {url}")
        if iface.get("protocolBinding") != "https://agentbounties.app/docs/a2a-direct-api-binding-v1":
            errors.append(f"interface must declare the documented binding: {url}")
    required_skills = {
        "discover-funded-work", "plan-bounty-claim",
        "submit-bounty-evidence", "check-bounty-settlement", "post-bounty",
    }
    have = {str(s.get("id", "")) for s in card.get("skills", [])}
    missing = required_skills - have
    if missing:
        errors.append(f"missing skills: {sorted(missing)}")

    if errors:
        for e in errors:
            print(f"FAIL: {e}")
        return 1
    print("A2A Agent Card smoke passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
