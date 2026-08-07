#!/usr/bin/env python3
"""Hermes integration smoke check for the Agent Bounties skill."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path


ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parent.parent))

EXIT_OK = 0
EXIT_FAIL = 1


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(EXIT_FAIL)


def require_file(path: str) -> Path:
    candidate = ROOT / path
    if not candidate.is_file():
        fail(f"missing required file: {path}")
    return candidate


def check_skill_frontmatter() -> None:
    skill_path = require_file("skills/agent-bounties/SKILL.md")
    skill = skill_path.read_text(encoding="utf-8")
    if not skill.startswith("---\n"):
        fail("canonical skill requires YAML frontmatter")
    closing = skill.find("\n---\n", 4)
    if closing < 0 or closing + 5 >= 2000:
        fail("Hermes-readable skill frontmatter must close before byte 2000")
    lower = skill.lower()
    required_phrases = [
        "https://api.agentbounties.app/v1/base/autonomous-bounties/feed",
        "claimable-live",
        "post your own bounty",
    ]
    for phrase in required_phrases:
        if phrase not in lower:
            fail(f"canonical skill missing required phrase: {phrase}")
    if "label:bounty" not in lower or "not" not in lower:
        fail("skill must explain that broad bounty labels are not claimability evidence")
    print("PASS: skill frontmatter")


def check_readme() -> None:
    readme = require_file("integrations/hermes/README.md").read_text(encoding="utf-8")
    install_cmd = (
        "hermes skills install "
        "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
    )
    if install_cmd not in readme:
        fail("Hermes README is missing the canonical one-command install")
    if "--now" not in readme and "/reset" not in readme:
        fail("Hermes README must explain fresh-session activation")
    print("PASS: Hermes README")


def check_fixtures() -> None:
    required_states = {"claimable", "unfunded", "stale"}
    required_keys = {"fixture", "description", "state", "next_action"}
    required_state_keys = {"status", "network"}

    for fixture_name in sorted(required_states):
        fixture_path = require_file(f"integrations/hermes/fixtures/{fixture_name}.json")
        try:
            data = json.loads(fixture_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as exc:
            fail(f"invalid JSON in fixture {fixture_name}: {exc}")

        missing_top = required_keys - set(data.keys())
        if missing_top:
            fail(f"fixture {fixture_name} missing top-level keys: {sorted(missing_top)}")

        if not isinstance(data.get("state"), dict):
            fail(f"fixture {fixture_name} state must be a JSON object")

        missing_state = required_state_keys - set(data["state"].keys())
        if missing_state:
            fail(f"fixture {fixture_name} state missing keys: {sorted(missing_state)}")

        if not isinstance(data.get("next_action"), str) or not data["next_action"].strip():
            fail(f"fixture {fixture_name} next_action must be a non-empty string")

        if data["fixture"] != fixture_name:
            fail(f"fixture {fixture_name} has mismatched fixture field: {data['fixture']}")

        print(f"PASS: fixture {fixture_name}")


def main() -> None:
    os.chdir(ROOT)
    check_skill_frontmatter()
    check_readme()
    check_fixtures()
    print("Hermes integration smoke check passed")


if __name__ == "__main__":
    main()
