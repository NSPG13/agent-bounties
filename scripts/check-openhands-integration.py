#!/usr/bin/env python3
"""Smoke test for the OpenHands earning integration.

Validates, without any network access, that the integration satisfies the
bounty's deterministic contract:

* the OpenHands skill is synchronized with canonical guidance,
* the stop hook is wired to the evidence guard and refuses completion
  without focused checks and submission evidence,
* claimable, unfunded, verifier-unready, and submitted-not-paid fixtures
  each carry exactly one deterministic next action.

Exit code 0 means the integration is safe to ship; any other exit code fails
the precommitted sandbox regression.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]

EXPECTED_NEXT_ACTIONS = {
    "claimable.json": "claim",
    "unfunded.json": "wait",
    "verifier-unready.json": "wait",
    "submitted-not-paid.json": "verify",
}
FEED_URL = "https://api.agentbounties.app/v1/base/autonomous-bounties/feed"
FORBIDDEN = ("private_key", "seed phrase", "eth_sendtransaction")


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    skill_path = ROOT / ".agents" / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        fail("missing OpenHands skill .agents/skills/agent-bounties/SKILL.md")
    skill = skill_path.read_text(encoding="utf-8")
    lower = skill.lower()
    for phrase in ("canonical", "claimable", "bountysettled", "one exact next action", "post your own bounty"):
        if phrase not in lower:
            fail(f"OpenHands skill is missing {phrase}")

    hooks_path = ROOT / ".openhands" / "hooks.json"
    if not hooks_path.is_file():
        fail("missing .openhands/hooks.json")
    try:
        hooks = json.loads(hooks_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        fail(f".openhands/hooks.json is not valid JSON: {error}")
    stop_hooks = hooks.get("stop")
    if not isinstance(stop_hooks, list) or not stop_hooks:
        fail("OpenHands integration requires a stop hook")
    if ".openhands/hooks/agent-bounties-evidence" not in json.dumps(stop_hooks, sort_keys=True):
        fail("stop hook does not invoke the Agent Bounties evidence guard")

    guard_path = ROOT / ".openhands" / "hooks" / "agent-bounties-evidence.py"
    if not guard_path.is_file():
        fail("missing .openhands/hooks/agent-bounties-evidence.py")
    guard = guard_path.read_text(encoding="utf-8")
    try:
        compile(guard, str(guard_path), "exec")
    except SyntaxError as error:
        fail(f"evidence guard has a syntax error: {error}")
    guard_lower = guard.lower()
    for phrase in ("submission", "evidence", "test", "decision", "deny"):
        if phrase not in guard_lower:
            fail(f"evidence guard is missing {phrase}")
    combined = (skill + guard).lower()
    for forbidden in FORBIDDEN:
        if forbidden in combined:
            fail(f"OpenHands integration contains forbidden wallet behavior: {forbidden}")

    fixtures_dir = ROOT / "integrations" / "openhands" / "fixtures"
    for fixture, expected_action in EXPECTED_NEXT_ACTIONS.items():
        path = fixtures_dir / fixture
        if not path.is_file():
            fail(f"missing fixture integrations/openhands/fixtures/{fixture}")
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            fail(f"fixture {fixture} is not valid JSON: {error}")
        state = data.get("state")
        if state != fixture.replace(".json", ""):
            fail(f"fixture {fixture} state must be {fixture.replace('.json', '')}, got {state!r}")
        action = data.get("next_action")
        if action != expected_action:
            fail(f"fixture {fixture} next_action must be {expected_action}, got {action!r}")
        if not str(data.get("next_action_detail", "")).strip():
            fail(f"fixture {fixture} must explain its next_action")

    print("OpenHands integration smoke passed")


if __name__ == "__main__":
    main()
