#!/usr/bin/env python3
"""Deterministic A2A Agent Card smoke test for Agent Bounties."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BINDING = "https://agentbounties.app/docs/a2a-direct-api-binding-v1"
API_BASE = "https://api.agentbounties.app"

REQUIRED_SKILLS = {
    "discover-funded-work",
    "plan-bounty-claim",
    "submit-bounty-evidence",
    "check-bounty-settlement",
    "post-bounty",
}

FORBIDDEN_TERMS = [
    "private_key", "private key", "seed phrase", "mnemonic",
    "api_key", "secret", "eth_sendtransaction"
]

REQUIRED_PHRASES = ["canonical", "claimable", "bountysettled"]

def load_card(path: Path) -> dict:
    """Load and validate an Agent Card JSON file."""
    if not path.exists():
        print(f"MISSING: {path}", file=sys.stderr)
        sys.exit(1)
    try:
        with open(path, encoding="utf-8") as f:
            card = json.load(f)
    except (json.JSONDecodeError, OSError) as e:
        print(f"PARSE ERROR: {path}: {e}", file=sys.stderr)
        sys.exit(1)
    return card

def check_card(card: dict, label: str) -> int:
    """Run all checks on a card, return error count."""
    errors = 0

    # Required top-level fields
    for field, expected in [
        ("name", "Agent Bounties"),
        ("version", None),
        ("description", None),
    ]:
        value = card.get(field)
        if value is None or (isinstance(value, str) and not value.strip()):
            print(f"FAIL [{label}]: missing or empty '{field}'", file=sys.stderr)
            errors += 1
        elif expected and value != expected:
            print(f"FAIL [{label}]: '{field}' expected '{expected}', got '{value}'",
                  file=sys.stderr)
            errors += 1

    # capabilities must be dict
    caps = card.get("capabilities")
    if not isinstance(caps, dict):
        print(f"FAIL [{label}]: capabilities must be a dict", file=sys.stderr)
        errors += 1

    # defaultInputModes and defaultOutputModes
    for mode_field in ("defaultInputModes", "defaultOutputModes"):
        modes = card.get(mode_field)
        if not isinstance(modes, list) or not modes:
            print(f"FAIL [{label}]: {mode_field} must be a non-empty array",
                  file=sys.stderr)
            errors += 1

    # supportedInterfaces
    interfaces = card.get("supportedInterfaces")
    if not isinstance(interfaces, list) or not interfaces:
        print(f"FAIL [{label}]: supportedInterfaces must be a non-empty array",
              file=sys.stderr)
        errors += 1
    else:
        for i, iface in enumerate(interfaces):
            url = str(iface.get("url", ""))
            if not url.startswith(API_BASE + "/"):
                print(f"FAIL [{label}]: interface[{i}] url must start with {API_BASE}/",
                      file=sys.stderr)
                errors += 1
            if iface.get("protocolVersion") != "1.0":
                print(f"FAIL [{label}]: interface[{i}] protocolVersion must be '1.0'",
                      file=sys.stderr)
                errors += 1
            if iface.get("protocolBinding") != BINDING:
                print(f"FAIL [{label}]: interface[{i}] protocolBinding must be '{BINDING}'",
                      file=sys.stderr)
                errors += 1

    # protocolVersion on root is forbidden
    if "protocolVersion" in card:
        print(f"FAIL [{label}]: protocolVersion belongs on interfaces, not root",
              file=sys.stderr)
        errors += 1

    # skills
    skills = card.get("skills")
    if not isinstance(skills, list):
        print(f"FAIL [{label}]: skills must be an array", file=sys.stderr)
        errors += 1
    else:
        skill_ids = {str(s.get("id", "")) for s in skills}
        missing = REQUIRED_SKILLS - skill_ids
        if missing:
            print(f"FAIL [{label}]: missing skills: {sorted(missing)}",
                  file=sys.stderr)
            errors += 1
        for s in skills:
            for f in ("id", "name", "description", "tags"):
                val = s.get(f)
                if val is None or val == "" or val == []:
                    print(f"FAIL [{label}]: skill '{s.get('id','?')}' missing {f}",
                          file=sys.stderr)
                    errors += 1

    # Forbidden terms
    serialized = json.dumps(card, sort_keys=True).lower()
    for term in FORBIDDEN_TERMS:
        if term in serialized:
            print(f"FAIL [{label}]: exposes forbidden material: '{term}'",
                  file=sys.stderr)
            errors += 1

    # Required phrases
    for phrase in REQUIRED_PHRASES:
        if phrase not in serialized:
            print(f"FAIL [{label}]: missing required phrase: '{phrase}'",
                  file=sys.stderr)
            errors += 1

    return errors


def main():
    paths = [
        (ROOT / "fixtures" / "a2a-agent-card.json", "fixture"),
        (ROOT / "site" / ".well-known" / "agent-card.json", "site"),
    ]

    total_errors = 0
    for path, label in paths:
        card = load_card(path)
        total_errors += check_card(card, label)

    # Cross-verify both cards are identical
    fixture_card = load_card(paths[0][0])
    site_card = load_card(paths[1][0])
    if json.dumps(fixture_card, sort_keys=True) != json.dumps(site_card, sort_keys=True):
        print("FAIL: fixture and site agent cards differ", file=sys.stderr)
        total_errors += 1
    else:
        h = hashlib.sha256(json.dumps(fixture_card, sort_keys=True).encode()).hexdigest()[:12]
        print(f"OK: fixture and site cards match (sha256={h})")

    if total_errors:
        print(f"\n{total_errors} error(s) found", file=sys.stderr)
        sys.exit(1)
    else:
        print("\nAll A2A Agent Card checks passed")


if __name__ == "__main__":
    main()
