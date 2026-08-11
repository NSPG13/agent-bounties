#!/usr/bin/env python3
"""Immutable acceptance check for stalled-bounty diagnostics."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(os.environ.get("WORKSPACE_ROOT", "/workspace"))


def read(path: str) -> str:
    candidate = ROOT / path
    if not candidate.is_file():
        raise SystemExit(f"missing required file: {path}")
    return candidate.read_text(encoding="utf-8")


implementation = read("scripts/stalled_bounty_diagnostics.py").lower()
tests = read("scripts/test_stalled_bounty_diagnostics.py").lower()
for phrase in (
    "healthy_claimed",
    "claim_expiring",
    "submitted",
    "verification_expiring",
    "verifier_unavailable",
    "settled",
    "next_action",
    "deadline",
    "bountysettled",
):
    if phrase not in implementation + tests:
        raise SystemExit(f"stalled-work diagnostics lack {phrase}")
for case in ("boundary", "missing terms", "stale", "outage"):
    if case not in tests:
        raise SystemExit(f"stalled-work tests lack {case}")

completed = subprocess.run(
    [sys.executable, "-m", "unittest", "scripts.test_stalled_bounty_diagnostics", "-v"],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=45,
    check=False,
)
if completed.returncode:
    raise SystemExit(f"stalled-work diagnostic tests failed:\n{completed.stdout[-4000:]}")

print("stalled-bounty diagnostic acceptance checks passed")
