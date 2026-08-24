#!/usr/bin/env python3
"""Smoke test for the A2A Agent Card implementation.

Verifies that the Agent Card fixture is valid JSON, matches the canonical
product name, and exposes the required skills and A2A 1.0 interface. This is
invoked by benchmarks/direct-growth-v2/a2a-agent-card/check.py.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    fixture_path = ROOT / "fixtures" / "a2a-agent-card.json"
    if not fixture_path.is_file():
        print(f"missing fixture: {fixture_path}")
        return 1
    card = json.loads(fixture_path.read_text(encoding="utf-8"))

    if card.get("name") != "Agent Bounties":
        print("Agent Card name mismatch")
        return 1
    if not card.get("version"):
        print("Agent Card version required")
        return 1
    if not isinstance(card.get("capabilities"), dict):
        print("Agent Card capabilities required")
        return 1
    if not isinstance(card.get("defaultInputModes"), list) or not card["defaultInputModes"]:
        print("defaultInputModes must be a non-empty array")
        return 1
    if not isinstance(card.get("defaultOutputModes"), list) or not card["defaultOutputModes"]:
        print("defaultOutputModes must be a non-empty array")
        return 1

    interfaces = card.get("supportedInterfaces")
    if not isinstance(interfaces, list) or not interfaces:
        print("supportedInterfaces required")
        return 1
    for iface in interfaces:
        if not str(iface.get("url", "")).startswith("https://api.agentbounties.app/"):
            print("interface must use canonical HTTPS API")
            return 1
        if iface.get("protocolVersion") != "1.0":
            print("interface must declare A2A 1.0")
            return 1
        if iface.get("protocolBinding") != "https://agentbounties.app/docs/a2a-direct-api-binding-v1":
            print("interface must identify the documented binding")
            return 1

    skills = card.get("skills")
    if not isinstance(skills, list):
        print("skills must be an array")
        return 1
    required = {
        "discover-funded-work",
        "plan-bounty-claim",
        "submit-bounty-evidence",
        "check-bounty-settlement",
        "post-bounty",
    }
    have = {str(s.get("id", "")) for s in skills}
    missing = required - have
    if missing:
        print(f"missing skills: {sorted(missing)}")
        return 1

    print("A2A Agent Card smoke passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
