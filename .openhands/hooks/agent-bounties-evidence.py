#!/usr/bin/env python3
"""OpenHands stop hook: block completion reporting until focused checks and
submission evidence exist.

The hook refuses to allow a task-completion claim (decision=deny) when the
agent has not run the focused test command and produced a submission evidence
file describing the exact artifact (repository, commit, test command, snapshot
digest, discovery source). It never touches wallet state: no key material and
no transaction broadcasting live in this hook.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE_REQUIRED = (
    "repository",
    "commit",
    "test_command",
    "source_snapshot_digest",
    "discovery_source",
)


def _find_evidence() -> Path | None:
    candidates = (
        ROOT / "evidence" / "submission.json",
        ROOT / ".openhands" / "evidence" / "submission.json",
    )
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def _load_evidence(path: Path) -> dict | None:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return None
    if not isinstance(data, dict):
        return None
    return data


def _run_focused_tests() -> tuple[bool, str]:
    checker = ROOT / "benchmarks" / "direct-growth-v2" / "openhands-integration" / "check.py"
    if not checker.is_file():
        return False, "benchmark check.py not found; cannot prove focused checks passed"
    env = {**os.environ, "WORKSPACE_ROOT": str(ROOT)}
    try:
        completed = subprocess.run(
            [sys.executable, str(checker)],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
    except subprocess.TimeoutExpired:
        return False, "focused checks timed out"
    if completed.returncode != 0:
        tail = (completed.stdout or "")[-1200:]
        return False, f"focused checks failed: {tail}"
    return True, "focused checks passed"


def main() -> int:
    decision = "deny"
    reasons: list[str] = []

    evidence_path = _find_evidence()
    if evidence_path is None:
        reasons.append("missing submission evidence file")
    else:
        evidence = _load_evidence(evidence_path)
        if evidence is None:
            reasons.append("submission evidence is not valid JSON")
        else:
            missing = [key for key in EVIDENCE_REQUIRED if not evidence.get(key)]
            if missing:
                reasons.append(f"submission evidence missing fields: {missing}")

    tests_ok, test_note = _run_focused_tests()
    if not tests_ok:
        reasons.append(test_note)

    if not reasons:
        decision = "allow"

    print(json.dumps({"decision": decision, "reasons": reasons, "tests": test_note}))
    return 0 if decision == "allow" else 1


if __name__ == "__main__":
    sys.exit(main())
