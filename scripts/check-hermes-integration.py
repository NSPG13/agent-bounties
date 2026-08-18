#!/usr/bin/env python3
"""Smoke test for Hermes Agent Bounties integration."""

import json, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

errors = []

# Check README
readme_path = ROOT / "integrations/hermes/README.md"
if not readme_path.exists():
    errors.append("integrations/hermes/README.md missing")
else:
    content = readme_path.read_text(encoding="utf-8")
    install_cmd = "hermes skills install"
    if install_cmd not in content:
        errors.append(f"README missing '{install_cmd}'")
    if "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md" not in content:
        errors.append("README missing canonical skill URL")
    if "--now" not in content and "/reset" not in content:
        errors.append("README missing fresh-session activation hint")

# Check fixtures
for name in ("claimable", "unfunded", "stale"):
    fixture_path = ROOT / f"integrations/hermes/fixtures/{name}.json"
    if not fixture_path.exists():
        errors.append(f"fixture {name}.json missing")
    else:
        try:
            data = json.loads(fixture_path.read_text(encoding="utf-8"))
            if "fixture" not in data or "action" not in data:
                errors.append(f"fixture {name}.json missing required fields")
        except json.JSONDecodeError:
            errors.append(f"fixture {name}.json is not valid JSON")

# Check SKILL.md
skill_path = ROOT / "skills/agent-bounties/SKILL.md"
if not skill_path.exists():
    errors.append("skills/agent-bounties/SKILL.md missing")
else:
    skill = skill_path.read_text(encoding="utf-8")
    if not skill.startswith("---\n"):
        errors.append("SKILL.md missing YAML frontmatter")

if errors:
    print("FAILED:")
    for e in errors:
        print(f"  - {e}")
    sys.exit(1)
else:
    print("Hermes integration smoke test PASSED")
    print(f"  README: OK")
    print(f"  Fixtures: claimable.json, unfunded.json, stale.json OK")
    print(f"  SKILL.md: OK")
