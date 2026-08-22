#!/usr/bin/env python3
"""Deterministic Hermes integration smoke for Agent Bounties."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "integrations" / "hermes" / "fixtures"
REQUIRED_STATES = ("claimable", "unfunded", "stale")
FEED = "https://api.agentbounties.app/v1/base/autonomous-bounties/feed"
INSTALL = (
    "hermes skills install "
    "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
)


def fail(msg: str) -> None:
    raise SystemExit(msg)


def main() -> None:
    skill = (ROOT / "skills" / "agent-bounties" / "SKILL.md").read_text(encoding="utf-8")
    if not skill.startswith("---\n"):
        fail("skill missing YAML frontmatter")
    lower = skill.lower()
    if FEED not in lower:
        fail(f"skill missing canonical feed URL {FEED}")
    if "claimable-live" not in lower:
        fail("skill missing claimable-live guidance")
    if "label:bounty" not in lower or "not" not in lower:
        fail("skill must warn that label:bounty is not claimability evidence")
    if "hermes" not in lower:
        fail("skill frontmatter/body should mention Hermes")

    readme = (ROOT / "integrations" / "hermes" / "README.md").read_text(encoding="utf-8")
    if INSTALL not in readme:
        fail("README missing one-command hermes skills install")
    if "--now" not in readme and "/reset" not in readme:
        fail("README must document --now or /reset for fresh-session activation")

    seen_actions: set[str] = set()
    for state in REQUIRED_STATES:
        path = FIXTURE_DIR / f"{state}.json"
        if not path.is_file():
            fail(f"missing fixture {path}")
        data = json.loads(path.read_text(encoding="utf-8"))
        if data.get("state") != state:
            fail(f"{path} state field must be {state}")
        action = data.get("next_action")
        if not isinstance(action, str) or len(action.strip()) < 20:
            fail(f"{path} needs a concrete next_action string")
        if action in seen_actions:
            fail("each fixture must have a distinct next_action")
        seen_actions.add(action)
        # state-specific invariants
        al = action.lower()
        if state == "claimable" and "claim" not in al:
            fail("claimable next_action must direct a claim")
        if state == "unfunded" and ("do not" not in al and "don't" not in al and "not" not in al):
            fail("unfunded next_action must block premature work")
        if state == "stale" and "feed" not in al:
            fail("stale next_action must redirect to claimable feed")

    print("check-hermes-integration: ok")
    print(f"fixtures={','.join(REQUIRED_STATES)}")
    print(f"install={INSTALL}")


if __name__ == "__main__":
    main()
