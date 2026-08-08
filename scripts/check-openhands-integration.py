#!/usr/bin/env python3
"""Smoke test for OpenHands Agent Bounties integration."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def check_skill() -> bool:
    skill_path = ROOT / ".agents" / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        print(f"FAIL: SKILL.md not found at {skill_path}")
        return False
    content = skill_path.read_text(encoding="utf-8")
    lower = content.lower()
    for phrase in ("canonical", "claimable", "bountysettled", "one exact next action", "post your own bounty"):
        if phrase not in lower:
            print(f"FAIL: SKILL.md missing '{phrase}'")
            return False
    for forbidden in ("private_key", "seed phrase", "eth_sendtransaction"):
        if forbidden in lower:
            print(f"FAIL: SKILL.md contains forbidden wallet behavior: {forbidden}")
            return False
    return True


def check_hooks() -> bool:
    hooks_path = ROOT / ".openhands" / "hooks.json"
    if not hooks_path.is_file():
        print(f"FAIL: hooks.json not found at {hooks_path}")
        return False
    try:
        hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        print(f"FAIL: invalid hooks.json: {e}")
        return False
    stop_hooks = hooks.get("stop")
    if not isinstance(stop_hooks, list) or not stop_hooks:
        print("FAIL: hooks.json missing stop hooks list")
        return False
    commands = json.dumps(stop_hooks, sort_keys=True)
    if ".openhands/hooks/agent-bounties-evidence" not in commands:
        print("FAIL: stop hook does not invoke agent-bounties-evidence")
        return False
    return True


def check_guard() -> bool:
    guard_path = ROOT / ".openhands" / "hooks" / "agent-bounties-evidence.py"
    if not guard_path.is_file():
        print(f"FAIL: evidence guard not found at {guard_path}")
        return False
    content = guard_path.read_text(encoding="utf-8")
    lower = content.lower()
    for phrase in ("submission", "evidence", "test", "decision", "deny"):
        if phrase not in lower:
            print(f"FAIL: evidence guard missing '{phrase}'")
            return False
    return True


def check_fixtures() -> bool:
    fixtures_dir = ROOT / "integrations" / "openhands" / "fixtures"
    for name in ("claimable.json", "unfunded.json", "verifier-unready.json", "submitted-not-paid.json"):
        path = fixtures_dir / name
        if not path.is_file():
            print(f"FAIL: fixture missing: {name}")
            return False
        try:
            json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            print(f"FAIL: invalid JSON in {name}: {e}")
            return False
    return True


def main() -> int:
    ok = True
    ok &= check_skill()
    ok &= check_hooks()
    ok &= check_guard()
    ok &= check_fixtures()
    if ok:
        print("OpenHands integration smoke test PASSED")
        return 0
    print("OpenHands integration smoke test FAILED")
    return 1


if __name__ == "__main__":
    sys.exit(main())
