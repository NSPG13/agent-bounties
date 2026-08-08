#!/usr/bin/env python3
"""Smoke-check Hermes integration fixtures and install docs."""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    readme = (ROOT / "integrations/hermes/README.md").read_text(encoding="utf-8")
    install = (
        "hermes skills install "
        "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
    )
    if install not in readme:
        print("missing canonical install command", file=sys.stderr)
        return 1
    if "--now" not in readme and "/reset" not in readme:
        print("missing fresh-session activation guidance", file=sys.stderr)
        return 1

    expected_actions = {
        "claimable": "call_agent_native_claim",
        "unfunded": "skip_or_fund_only",
        "stale": "rerun_check_in",
    }
    for name, action in expected_actions.items():
        path = ROOT / f"integrations/hermes/fixtures/{name}.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        if data.get("state") != name:
            print(f"{name}: bad state field", file=sys.stderr)
            return 1
        na = data.get("next_action") or {}
        if na.get("action") != action:
            print(f"{name}: expected next_action.action={action}", file=sys.stderr)
            return 1
        # exactly one next_action object
        if not isinstance(na, dict) or "action" not in na:
            print(f"{name}: next_action must be a single object", file=sys.stderr)
            return 1

    skill = (ROOT / "skills/agent-bounties/SKILL.md").read_text(encoding="utf-8")
    if not skill.startswith("---\n"):
        print("skill missing frontmatter", file=sys.stderr)
        return 1
    if "https://api.agentbounties.app/v1/base/autonomous-bounties/feed" not in skill.lower():
        print("skill missing canonical feed URL", file=sys.stderr)
        return 1

    print("Hermes integration smoke OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
