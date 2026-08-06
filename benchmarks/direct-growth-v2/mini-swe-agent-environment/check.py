#!/usr/bin/env python3
"""Immutable acceptance check for the mini-SWE-agent paid-work environment."""

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


config = require("integrations/mini-swe-agent/config.yaml").read_text(encoding="utf-8")
lower = config.lower()
for phrase in ("inventory", "claim", "evidence", "settlement", "direct argv"):
    if phrase not in lower:
        raise SystemExit(f"mini-SWE-agent config is missing {phrase}")
for forbidden in ("private_key", "seed phrase", "mnemonic", "eth_sendtransaction"):
    if forbidden in lower:
        raise SystemExit(
            f"mini-SWE-agent config contains forbidden wallet behavior: {forbidden}"
        )

selector = require("integrations/mini-swe-agent/select_bounty.py")
expectations = {
    "multiple.json": "claim",
    "empty.json": "wait",
    "stale.json": "refresh",
    "no-margin.json": "skip",
    "exclusive-claimant.json": "skip",
}
for fixture, action in expectations.items():
    path = require(f"integrations/mini-swe-agent/fixtures/{fixture}")
    completed = subprocess.run(
        [sys.executable, str(selector), "--input", str(path)],
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=20,
        check=False,
    )
    if completed.returncode != 0:
        raise SystemExit(f"selector failed for {fixture}:\n{completed.stdout[-3000:]}")
    try:
        result = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"selector returned invalid JSON for {fixture}") from error
    if result.get("action") != action:
        raise SystemExit(f"selector action for {fixture} must be {action}")
    if not str(result.get("next_action", "")).strip():
        raise SystemExit(f"selector must return one exact next action for {fixture}")

readme = (
    require("integrations/mini-swe-agent/README.md").read_text(encoding="utf-8").lower()
)
for phrase in ("source_snapshot_digest", "discovery_source", "bountysettled"):
    if phrase not in readme:
        raise SystemExit(f"mini-SWE-agent README is missing {phrase}")

print("mini-SWE-agent environment acceptance checks passed")
