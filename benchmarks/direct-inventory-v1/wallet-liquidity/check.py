#!/usr/bin/env python3
"""Immutable acceptance check for bounded-wallet liquidity reporting."""

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


implementation = read("scripts/bounded_wallet_liquidity.py").lower()
tests = read("scripts/test_bounded_wallet_liquidity.py").lower()
for phrase in (
    "usdc_balance",
    "lifetime_spent",
    "max_lifetime",
    "period_spent",
    "max_per_period",
    "policy_hash",
    "policy_version",
    "observed_block",
):
    if phrase not in implementation:
        raise SystemExit(f"liquidity report lacks {phrase}")
for case in ("empty", "cap", "policy", "wrong chain", "unavailable"):
    if case not in tests:
        raise SystemExit(f"liquidity tests lack {case}")

completed = subprocess.run(
    [sys.executable, "-m", "unittest", "scripts.test_bounded_wallet_liquidity", "-v"],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=45,
    check=False,
)
if completed.returncode:
    raise SystemExit(f"bounded-wallet liquidity tests failed:\n{completed.stdout[-4000:]}")

print("bounded-wallet liquidity acceptance checks passed")
