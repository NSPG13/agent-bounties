#!/usr/bin/env python3
"""Immutable acceptance check for the OpenHands earning integration bounty."""

from __future__ import annotations

import json
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


skill = require(".agents/skills/agent-bounties/SKILL.md").read_text(encoding="utf-8")
lower = skill.lower()
for phrase in (
    "canonical",
    "claimable",
    "bountysettled",
    "one exact next action",
    "post your own bounty",
):
    if phrase not in lower:
        raise SystemExit(f"OpenHands skill is missing {phrase}")

hooks = json.loads(require(".openhands/hooks.json").read_text(encoding="utf-8"))
stop_hooks = hooks.get("stop")
if not isinstance(stop_hooks, list) or not stop_hooks:
    raise SystemExit("OpenHands integration requires a stop hook")
commands = json.dumps(stop_hooks, sort_keys=True)
if ".openhands/hooks/agent-bounties-evidence" not in commands:
    raise SystemExit("stop hook does not invoke the Agent Bounties evidence guard")

guard = require(".openhands/hooks/agent-bounties-evidence.py").read_text(
    encoding="utf-8"
)
for phrase in ("submission", "evidence", "test", "decision", "deny"):
    if phrase not in guard.lower():
        raise SystemExit(f"evidence guard is missing {phrase}")
for forbidden in ("private_key", "seed phrase", "eth_sendtransaction"):
    if forbidden in (skill + guard).lower():
        raise SystemExit(
            f"OpenHands integration contains forbidden wallet behavior: {forbidden}"
        )

checker = require("scripts/check-openhands-integration.py")
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
    raise SystemExit(f"OpenHands integration smoke failed:\n{completed.stdout[-4000:]}")
for fixture in ("claimable", "unfunded", "verifier-unready", "submitted-not-paid"):
    require(f"integrations/openhands/fixtures/{fixture}.json")

print("OpenHands integration acceptance checks passed")
