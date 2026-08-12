#!/usr/bin/env python3
"""Immutable acceptance check for the inventory-state breakdown."""

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


checker = "scripts/check-inventory-state-breakdown.py"
source = read(checker).lower()
for phrase in (
    "inventory-state-breakdown-v1",
    "ready_to_earn",
    "in_progress",
    "submitted",
    "paid",
    "verification_unavailable",
    "generated_at",
    "source",
):
    if phrase not in source:
        raise SystemExit(f"inventory breakdown checker lacks {phrase}")
for fixture in ("empty", "mixed", "degraded", "stale"):
    read(f"scripts/fixtures/inventory-state-breakdown/{fixture}.json")

completed = subprocess.run(
    [sys.executable, checker],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=45,
    check=False,
)
if completed.returncode:
    raise SystemExit(f"inventory breakdown checks failed:\n{completed.stdout[-4000:]}")

print("inventory-state breakdown acceptance checks passed")
