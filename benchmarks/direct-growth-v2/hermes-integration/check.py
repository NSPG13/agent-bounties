#!/usr/bin/env python3
"""Immutable acceptance check for the Hermes earning integration bounty."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))


def require(path: str) -> Path:
    candidate = ROOT / path
    if not candidate.is_file():
        raise SystemExit(f"missing required file: {path}")
    return candidate


skill_path = require("skills/agent-bounties/SKILL.md")
skill = skill_path.read_text(encoding="utf-8")
if not skill.startswith("---\n"):
    raise SystemExit("canonical skill requires YAML frontmatter")
closing = skill.find("\n---\n", 4)
if closing < 0 or closing + 5 >= 2000:
    raise SystemExit("Hermes-readable skill frontmatter must close before byte 2000")
lower = skill.lower()
for phrase in (
    "https://api.agentbounties.app/v1/base/autonomous-bounties/feed",
    "claimable-live",
    "post your own bounty",
):
    if phrase not in lower:
        raise SystemExit(f"canonical skill is missing {phrase}")
if "label:bounty" not in lower or "not" not in lower:
    raise SystemExit(
        "skill must explain that broad bounty labels are not claimability evidence"
    )

readme = require("integrations/hermes/README.md").read_text(encoding="utf-8")
install = (
    "hermes skills install "
    "https://raw.githubusercontent.com/NSPG13/agent-bounties/main/skills/agent-bounties/SKILL.md"
)
if install not in readme:
    raise SystemExit("Hermes README is missing the canonical one-command install")
if "--now" not in readme and "/reset" not in readme:
    raise SystemExit("Hermes README must explain fresh-session activation")

checker = require("scripts/check-hermes-integration.py")
completed = subprocess.run(
    [sys.executable, str(checker)],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=30,
    check=False,
)
if completed.returncode != 0:
    raise SystemExit(f"Hermes integration smoke failed:\n{completed.stdout[-4000:]}")
for fixture in ("claimable", "unfunded", "stale"):
    require(f"integrations/hermes/fixtures/{fixture}.json")

print("Hermes integration acceptance checks passed")
