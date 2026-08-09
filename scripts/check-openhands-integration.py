#!/usr/bin/env python3
"""OpenHands integration smoke test — validates OpenHands agent skill, hooks, and fixtures."""

import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parent.parent))
INTEGRATION_DIR = ROOT / "integrations" / "openhands"
FIXTURES_DIR = INTEGRATION_DIR / "fixtures"
SKILL_PATH = ROOT / ".agents" / "skills" / "agent-bounties" / "SKILL.md"
HOOKS_PATH = ROOT / ".openhands" / "hooks.json"
GUARD_PATH = ROOT / ".openhands" / "hooks" / "agent-bounties-evidence.py"

errors = []

# 1. Skill file
if not SKILL_PATH.is_file():
    errors.append(f"Missing OpenHands skill at {SKILL_PATH}")
else:
    skill_text = SKILL_PATH.read_text(encoding="utf-8")
    lower = skill_text.lower()
    required_phrases = [
        "canonical",
        "claimable",
        "bountysettled",
        "one exact next action",
        "post your own bounty",
    ]
    for phrase in required_phrases:
        if phrase not in lower:
            errors.append(f"Skill missing required phrase: {phrase}")

# 2. Hooks
if not HOOKS_PATH.is_file():
    errors.append(f"Missing hooks.json at {HOOKS_PATH}")
else:
    hooks = json.loads(HOOKS_PATH.read_text(encoding="utf-8"))
    stop_hooks = hooks.get("stop")
    if not isinstance(stop_hooks, list) or not stop_hooks:
        errors.append("hooks.json must have a non-empty stop array")
    else:
        commands = json.dumps(stop_hooks, sort_keys=True)
        if ".openhands/hooks/agent-bounties-evidence" not in commands:
            errors.append("stop hook does not invoke the Agent Bounties evidence guard")

# 3. Evidence guard
if not GUARD_PATH.is_file():
    errors.append(f"Missing evidence guard at {GUARD_PATH}")
else:
    guard_text = GUARD_PATH.read_text(encoding="utf-8").lower()
    guard_phrases = ["submission", "evidence", "test", "decision", "deny"]
    for phrase in guard_phrases:
        if phrase not in guard_text:
            errors.append(f"Evidence guard missing required phrase: {phrase}")

# 4. Safety: no forbidden wallet material in skill or guard
combined = ""
if SKILL_PATH.is_file():
    combined += SKILL_PATH.read_text(encoding="utf-8")
if GUARD_PATH.is_file():
    combined += GUARD_PATH.read_text(encoding="utf-8")
combined = combined.lower()

forbidden = ["private_key", "seed phrase", "eth_sendtransaction"]
for item in forbidden:
    if item in combined:
        errors.append(f"Forbidden wallet material found: {item}")

# 5. Fixtures
required_fixtures = ["claimable", "unfunded", "verifier-unready", "submitted-not-paid"]
for name in required_fixtures:
    fixture_path = FIXTURES_DIR / f"{name}.json"
    if not fixture_path.is_file():
        errors.append(f"Missing fixture: {name}.json")
        continue
    try:
        data = json.loads(fixture_path.read_text(encoding="utf-8"))
        if not isinstance(data, dict):
            errors.append(f"Fixture {name}.json is not a JSON object")
            continue
        if "state" not in data:
            errors.append(f"Fixture {name}.json missing 'state' field")
        if "next_action" not in data:
            errors.append(f"Fixture {name}.json missing 'next_action' field")
    except json.JSONDecodeError as e:
        errors.append(f"Fixture {name}.json not valid JSON: {e}")

if errors:
    print("FAILED:")
    for e in errors:
        print(f"  - {e}")
    sys.exit(1)

print("OpenHands integration smoke test passed")
