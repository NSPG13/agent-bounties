#!/usr/bin/env python3
"""Hermes integration smoke test — validates skill structure and fixture integrity."""

import json
import os
import sys
from pathlib import Path

ROOT = Path(os.environ.get("WORKSPACE_ROOT", Path(__file__).resolve().parent.parent.parent))
INTEGRATION_DIR = ROOT / "integrations" / "hermes"
SKILL_PATH = ROOT / "skills" / "agent-bounties" / "SKILL.md"
README_PATH = INTEGRATION_DIR / "README.md"
FIXTURES_DIR = INTEGRATION_DIR / "fixtures"

errors = []

# 1. Skill file exists and has valid frontmatter
if not SKILL_PATH.is_file():
    errors.append(f"Missing SKILL.md at {SKILL_PATH}")

skill_text = SKILL_PATH.read_text(encoding="utf-8")
if not skill_text.startswith("---\n"):
    errors.append("SKILL.md must start with YAML frontmatter")

closing = skill_text.find("\n---\n", 4)
if closing < 0:
    errors.append("SKILL.md frontmatter must close with ---")
elif closing + 5 >= 2000:
    errors.append("SKILL.md frontmatter too large (must close before byte 2000)")

# 2. Key phrases present
lower = skill_text.lower()
required_phrases = [
    "https://api.agentbounties.app/v1/base/autonomous-bounties/feed",
    "claimable-live",
    "post your own bounty",
]
for phrase in required_phrases:
    if phrase not in lower:
        errors.append(f"SKILL.md missing required phrase: {phrase}")

# label:bounty must be mentioned with a "not" (disclaimer)
if "label:bounty" not in lower:
    errors.append("SKILL.md must mention label:bounty")
if "not" not in lower:
    errors.append("SKILL.md must explain that broad labels are not claimability evidence")

# 3. README has correct install command
readme_text = README_PATH.read_text(encoding="utf-8")
install_cmd = (
    "hermes skills install "
    "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
)
if install_cmd not in readme_text:
    errors.append("README.md missing canonical one-command install")

has_activation = "--now" in readme_text or "/reset" in readme_text
if not has_activation:
    errors.append("README.md must explain fresh-session activation (--now or /reset)")

# 4. Fixture files
for fixture_name in ("claimable", "unfunded", "stale"):
    fixture_path = FIXTURES_DIR / f"{fixture_name}.json"
    if not fixture_path.is_file():
        errors.append(f"Missing fixture: {fixture_name}.json")
        continue
    try:
        data = json.loads(fixture_path.read_text(encoding="utf-8"))
        if not isinstance(data, dict) or "state" not in data:
            errors.append(f"Fixture {fixture_name}.json must have 'state' field")
        if "next_action" not in data:
            errors.append(f"Fixture {fixture_name}.json must have 'next_action' field")
    except json.JSONDecodeError as e:
        errors.append(f"Fixture {fixture_name}.json is not valid JSON: {e}")

if errors:
    print("FAILED:")
    for e in errors:
        print(f"  - {e}")
    sys.exit(1)

print("Hermes integration smoke test passed")
