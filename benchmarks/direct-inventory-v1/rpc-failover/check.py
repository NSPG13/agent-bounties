#!/usr/bin/env python3
"""Immutable acceptance check for retry-safe Base RPC failover."""

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


transport = read("scripts/_shared/rpc.py").lower()
tests = read("scripts/test_shared_rpc.py").lower()
for phrase in ("eth_chainid", "8453", "429", "500", "retry", "https"):
    if phrase not in transport + tests:
        raise SystemExit(f"shared RPC implementation lacks {phrase}")
for phrase in ("wrong chain", "rpc error", "exhaust"):
    if phrase not in tests:
        raise SystemExit(f"shared RPC tests lack {phrase}")

# Run tests as a file so imports resolve without an undeclared PYTHONPATH.
test_file = ROOT / "scripts" / "test_shared_rpc.py"
if not test_file.is_file():
    raise SystemExit("missing required file: scripts/test_shared_rpc.py")
completed = subprocess.run(
    [sys.executable, str(test_file), "-v"],
    cwd=ROOT,
    text=True,
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    timeout=45,
    check=False,
)
if completed.returncode:
    raise SystemExit(f"shared RPC tests failed:\n{completed.stdout[-4000:]}")

print("retry-safe Base RPC acceptance checks passed")
