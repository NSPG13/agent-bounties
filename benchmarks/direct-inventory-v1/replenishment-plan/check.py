#!/usr/bin/env python3
"""Immutable acceptance check for deterministic replenishment planning."""

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


implementation = read("scripts/plan_inventory_replenishment.py").lower()
tests = read("scripts/test_plan_inventory_replenishment.py").lower()
for phrase in (
    "idempotency",
    "deficit",
    "wallet_balance",
    "period",
    "lifetime",
    "solver_margin",
    "blocker",
    "financial_action_taken",
):
    if phrase not in implementation:
        raise SystemExit(f"replenishment planner lacks {phrase}")
for case in ("stale", "duplicate", "insufficient", "five"):
    if case not in tests:
        raise SystemExit(f"replenishment tests lack {case}")

completed = subprocess.run(
    [sys.executable, "-m", "unittest", "scripts.test_plan_inventory_replenishment", "-v"],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=45,
    check=False,
)
if completed.returncode:
    raise SystemExit(f"replenishment planner tests failed:\n{completed.stdout[-4000:]}")

print("inventory replenishment-plan acceptance checks passed")
