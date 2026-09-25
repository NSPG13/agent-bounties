#!/usr/bin/env python3
"""Smoke test for the A2A Agent Card endpoint."""
import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
CARD = ROOT / "fixtures" / "a2a-agent-card.json"

def main():
    if not CARD.is_file():
        print(f"SKIP: {CARD} not found (tested offline)")
        return 0

    card = json.loads(CARD.read_text(encoding="utf-8"))

    assert card.get("name") == "Agent Bounties", "wrong name"
    assert card.get("description"), "missing description"
    assert card.get("version"), "missing version"
    assert isinstance(card.get("capabilities"), dict), "capabilities must be dict"
    assert isinstance(card.get("defaultInputModes"), list) and card["defaultInputModes"], "input modes required"
    assert isinstance(card.get("defaultOutputModes"), list) and card["defaultOutputModes"], "output modes required"

    interfaces = card.get("supportedInterfaces", [])
    assert interfaces, "no interfaces"
    for iface in interfaces:
        assert iface.get("protocolVersion") == "1.0", f"bad version: {iface}"
        assert iface["url"].startswith("https://api.agentbounties.app/"), f"bad url: {iface}"
        assert iface.get("protocolBinding") == "https://agentbounties.app/docs/a2a-direct-api-binding-v1", f"bad binding: {iface}"

    skills = card.get("skills", [])
    assert skills, "no skills"
    required = {"discover-funded-work", "plan-bounty-claim", "submit-bounty-evidence", "check-bounty-settlement", "post-bounty"}
    found = {s["id"] for s in skills}
    assert required == found, f"missing skills: {required - found}, extra: {found - required}"

    for skill in skills:
        for field in ("id", "name", "description", "tags"):
            assert skill.get(field), f"skill {skill.get('id')} missing {field}"

    serialized = json.dumps(card, sort_keys=True).lower()
    for forbidden in ("private_key", "private key", "seed phrase", "api_key", "secret"):
        assert forbidden not in serialized, f"forbidden: {forbidden}"
    for phrase in ("canonical", "claimable", "bountysettled"):
        assert phrase in serialized, f"missing phrase: {phrase}"

    print("A2A Agent Card smoke passed")
    return 0

if __name__ == "__main__":
    sys.exit(main())
