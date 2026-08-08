#!/usr/bin/env python3
"""Smoke test for Hermes Agent Bounties integration."""
from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def check_skill() -> bool:
    skill_path = ROOT / "skills" / "agent-bounties" / "SKILL.md"
    if not skill_path.is_file():
        print(f"FAIL: SKILL.md not found at {skill_path}")
        return False
    content = skill_path.read_text(encoding="utf-8")
    if not content.startswith("---\n"):
        print("FAIL: SKILL.md missing YAML frontmatter")
        return False
    lower = content.lower()
    checks = [
        ("https://api.agentbounties.app/v1/base/autonomous-bounties/feed", "feed URL"),
        ("claimable-live", "claimable-live reference"),
        ("post your own bounty", "post your own bounty"),
        ("label:bounty", "label:bounty explanation"),
    ]
    for phrase, desc in checks:
        if phrase not in lower:
            print(f"FAIL: SKILL.md missing {desc}")
            return False
    if "label:bounty" in lower and "not" not in lower:
        print("FAIL: SKILL.md must explain that label:bounty is not claimability evidence")
        return False
    return True

def check_readme() -> bool:
    readme_path = ROOT / "integrations" / "hermes" / "README.md"
    if not readme_path.is_file():
        print(f"FAIL: README.md not found at {readme_path}")
        return False
    content = readme_path.read_text(encoding="utf-8")
    install_cmd = (
        "hermes skills install "
        "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
    )
    if install_cmd not in content:
        print("FAIL: README missing canonical one-command install")
        return False
    if "--now" not in content and "/reset" not in content:
        print("FAIL: README must explain fresh-session activation")
        return False
    return True

def check_fixtures() -> bool:
    fixtures_dir = ROOT / "integrations" / "hermes" / "fixtures"
    for name in ("claimable.json", "unfunded.json", "stale.json"):
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
    ok &= check_readme()
    ok &= check_fixtures()
    if ok:
        print("Hermes integration smoke test PASSED")
        return 0
    print("Hermes integration smoke test FAILED")
    return 1

if __name__ == "__main__":
    sys.exit(main())
