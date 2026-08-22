#!/usr/bin/env python3
"""Smoke test for OpenHands integration."""

import json, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
errors = []

# Check skill
skill_path = ROOT / ".agents/skills/agent-bounties/SKILL.md"
if not skill_path.exists():
    errors.append("SKILL.md missing")
else:
    s = skill_path.read_text().lower()
    for p in ("canonical", "claimable", "bountysettled", "one exact next action", "post your own bounty"):
        if p not in s:
            errors.append(f"skill missing: {p}")

# Check hooks
hooks_path = ROOT / ".openhands/hooks.json"
if not hooks_path.exists():
    errors.append("hooks.json missing")
else:
    hooks = json.loads(hooks_path.read_text())
    if not hooks.get("stop"):
        errors.append("stop hook missing")

# Check guard
guard_path = ROOT / ".openhands/hooks/agent-bounties-evidence.py"
if not guard_path.exists():
    errors.append("evidence guard missing")
else:
    g = guard_path.read_text().lower()
    for p in ("submission", "evidence", "test", "decision", "deny"):
        if p not in g:
            errors.append(f"guard missing: {p}")

# Check fixtures
for name in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    p = ROOT / f"integrations/openhands/fixtures/{name}.json"
    if not p.exists():
        errors.append(f"fixture {name}.json missing")

if errors:
    print("FAILED:"); [print(f"  - {e}") for e in errors]
    sys.exit(1)
print("OpenHands integration smoke test PASSED")
