#!/usr/bin/env python3
"""Smoke test for the Hermes earning integration.

Validates, without any network access, that the integration satisfies the
bounty's deterministic contract:

* the canonical skill is Hermes-readable and directs discovery to the
  canonical claimable feed rather than broad labels,
* the Hermes README exposes the exact one-command install and fresh-session
  activation,
* claimable, unfunded, and stale fixtures each carry exactly one deterministic
  next action.

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
    "stale.json": "refresh",
}
FEED_URL = "https://api.agentbounties.app/v1/base/autonomous-bounties/feed"
INSTALL = (
    "hermes skills install "
    "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
)


def fail(message: str) -> None:
    print(f"FAIL: {message}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    skill_path = ROOT / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        fail("missing canonical skill skills/agent-bounties/SKILL.md")
    skill = skill_path.read_text(encoding="utf-8")
    if not skill.startswith("---\n"):
        fail("canonical skill requires YAML frontmatter")
    if "\n---\n" not in skill[4:2000]:
        fail("Hermes-readable skill frontmatter must close before byte 2000")
    lower = skill.lower()
    for phrase in (FEED_URL, "claimable-live", "post your own bounty"):
        if phrase not in lower:
            fail(f"canonical skill is missing {phrase}")
    if "label:bounty" not in lower or "not" not in lower:
        fail("skill must explain that broad bounty labels are not claimability evidence")

    readme_path = ROOT / "integrations" / "hermes" / "README.md"
    if not readme_path.is_file():
        fail("missing integrations/hermes/README.md")
    readme = readme_path.read_text(encoding="utf-8")
    if INSTALL not in readme:
        fail("Hermes README is missing the canonical one-command install")
    if "--now" not in readme and "/reset" not in readme:
        fail("Hermes README must explain fresh-session activation")

    fixtures_dir = ROOT / "integrations" / "hermes" / "fixtures"
    for fixture, expected_action in EXPECTED_NEXT_ACTIONS.items():
        path = fixtures_dir / fixture
        if not path.is_file():
            fail(f"missing fixture integrations/hermes/fixtures/{fixture}")
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as error:
            fail(f"fixture {fixture} is not valid JSON: {error}")
        if data.get("state") is None:
            fail(f"fixture {fixture} must declare a state")
        action = data.get("next_action")
        if not isinstance(action, str) or not action.strip():
            fail(f"fixture {fixture} must carry exactly one next_action")
        if action != expected_action:
            fail(f"fixture {fixture} next_action must be {expected_action}, got {action}")
        if not str(data.get("next_action_detail", "")).strip():
            fail(f"fixture {fixture} must explain its next_action")

    print("Hermes integration smoke passed")


if __name__ == "__main__":
    main()
