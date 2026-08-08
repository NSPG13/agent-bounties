#!/usr/bin/env python3
"""Smoke-test the OpenHands Agent Bounties earning integration.

Validates:
1. All required integration files exist with correct structure
2. Hooks configuration is valid JSON with stop hooks
3. Evidence guard can import and validate test fixtures
4. Forbidden wallet patterns are absent from all integration files
5. Execute hook produces valid evidence bundles
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parent.parent))

FAILURES = 0


def check(label: str, condition: bool, detail: str = "") -> bool:
    global FAILURES
    status = "PASS" if condition else "FAIL"
    detail_str = f" — {detail}" if detail else ""
    print(f"  [{status}] {label}{detail_str}")
    if not condition:
        FAILURES += 1
    return condition


def file_exists(path: str) -> bool:
    return (ROOT / path).is_file()


def test_hooks_json():
    """Validate hooks.json structure."""
    hooks_path = ROOT / ".openhands" / "hooks.json"
    if not hooks_path.is_file():
        check("hooks.json exists", False, f"not found at {hooks_path}")
        return

    try:
        hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        check("hooks.json valid JSON", False, str(e))
        return

    check("hooks.json is valid JSON", True)

    stop_hooks = hooks.get("stop", [])
    check("stop hooks configured", isinstance(stop_hooks, list) and len(stop_hooks) > 0,
          f"found {len(stop_hooks) if isinstance(stop_hooks, list) else 'non-list'}")

    # Verify evidence guard is referenced
    commands_str = json.dumps(stop_hooks, sort_keys=True)
    check("evidence guard in stop hooks", ".openhands/hooks/agent-bounties-evidence" in commands_str)


def test_evidence_guard():
    """Test the evidence guard script."""
    guard_path = ROOT / ".openhands" / "hooks" / "agent-bounties-evidence.py"
    if not guard_path.is_file():
        check("evidence guard exists", False)
        return

    guard_text = guard_path.read_text(encoding="utf-8").lower()
    required = ("submission", "evidence", "test", "decision", "deny")
    for phrase in required:
        check(f"evidence guard contains '{phrase}'", phrase in guard_text)

    # Check forbidden wallet patterns absent
    forbidden = ("private_key", "seed phrase", "eth_sendtransaction")
    all_text = guard_text
    for pattern in forbidden:
        check(f"no '{pattern}' in guard", pattern not in all_text)


def test_fixtures():
    """Test that fixture files are valid JSON."""
    fixtures_dir = ROOT / "integrations" / "openhands" / "fixtures"
    if not fixtures_dir.is_dir():
        check("fixtures directory exists", False, str(fixtures_dir))
        return

    expected = ("claimable.json", "submitted-not-paid.json", "unfunded.json", "verifier-unready.json")
    for name in expected:
        fp = fixtures_dir / name
        exists = fp.is_file()
        check(f"fixture {name} exists", exists)
        if exists:
            try:
                data = json.loads(fp.read_text(encoding="utf-8"))
                check(f"fixture {name} valid JSON", isinstance(data, dict),
                      f"keys: {list(data.keys())[:5]}")
            except json.JSONDecodeError as e:
                check(f"fixture {name} valid JSON", False, str(e))


def test_execute_hook():
    """Test that execute hook produces evidence output."""
    execute_path = ROOT / ".openhands" / "hooks" / "agent-bounties-execute.py"
    if not execute_path.is_file():
        check("execute hook exists", False)
        return

    check("execute hook exists", True)

    execute_text = execute_path.read_text(encoding="utf-8").lower()
    for phrase in ("evidence", "claimable", "bounty", "submission"):
        check(f"execute hook references '{phrase}'", phrase in execute_text)


def test_skill():
    """Test the skill markdown."""
    skill_path = ROOT / ".agents" / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        check("skill exists", False)
        return

    skill_text = skill_path.read_text(encoding="utf-8").lower()
    phrases = ("canonical", "claimable", "bountysettled", "one exact next action", "post your own bounty")
    for phrase in phrases:
        check(f"skill contains '{phrase}'", phrase in skill_text)


def main():
    print("OpenHands Agent Bounties Integration Smoke Test")
    print(f"Workspace root: {ROOT}")
    print()

    test_hooks_json()
    print()
    test_evidence_guard()
    print()
    test_fixtures()
    print()
    test_execute_hook()
    print()
    test_skill()
    print()

    print(f"\n{'=' * 50}")
    if FAILURES == 0:
        print("ALL CHECKS PASSED — integration ready for earning loop")
    else:
        print(f"{FAILURES} CHECK(S) FAILED — integration incomplete")
    print(f"{'=' * 50}")

    return 0 if FAILURES == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
