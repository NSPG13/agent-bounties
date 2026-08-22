#!/usr/bin/env python3
"""Agent Bounties evidence guard for OpenHands — validates submission evidence before allowing task completion."""

import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))
EVIDENCE_DIR = ROOT / "integrations" / "openhands" / "fixtures"
ERRORS = []


def check_evidence():
    """Validate that submission evidence meets canonical requirements."""
    # Check that evidence files exist
    required_fixtures = ["claimable", "unfunded", "verifier-unready", "submitted-not-paid"]
    for name in required_fixtures:
        fixture = EVIDENCE_DIR / f"{name}.json"
        if not fixture.is_file():
            ERRORS.append(f"Missing evidence fixture: {name}.json")
            continue
        try:
            data = json.loads(fixture.read_text(encoding="utf-8"))
            if "state" not in data:
                ERRORS.append(f"Fixture {name}.json missing 'state' field")
            if "next_action" not in data:
                ERRORS.append(f"Fixture {name}.json missing 'next_action' field")
        except json.JSONDecodeError as e:
            ERRORS.append(f"Fixture {name}.json invalid JSON: {e}")

    # Test: verify the skill file exists
    skill_path = ROOT / ".agents" / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        ERRORS.append("Missing OpenHands agent skill SKILL.md")

    # Test: verify skill contains key phrases
    if skill_path.is_file():
        skill_text = skill_path.read_text(encoding="utf-8").lower()
        for phrase in ("canonical", "claimable", "bountysettled", "submission", "evidence"):
            if phrase not in skill_text:
                ERRORS.append(f"Skill missing key phrase: {phrase}")

    # Safety: verify no direct wallet credential exposure patterns
    if skill_path.is_file():
        skill_text = skill_path.read_text(encoding="utf-8")
        unsafe_patterns = ["0x" + "0" * 64, "BEGIN PRIVATE", "mnemonic"]
        for pat in unsafe_patterns:
            if pat.lower() in skill_text.lower():
                ERRORS.append(f"Unsafe wallet pattern detected in skill")


def make_decision():
    """Return one exact next action based on evidence state."""
    if ERRORS:
        return "deny"
    return "pass"


if __name__ == "__main__":
    check_evidence()
    decision = make_decision()

    result = {
        "decision": decision,
        "submission": "agent-bounties-openhands-v1",
        "evidence": f"{len(ERRORS)} errors" if ERRORS else "all checks passed",
        "test": "evidence guard validation",
    }

    if ERRORS:
        result["errors"] = ERRORS
        print(json.dumps(result, indent=2))
        sys.exit(1)

    print(json.dumps(result, indent=2))
    print("Agent Bounties evidence guard: submission ready")
    sys.exit(0)
