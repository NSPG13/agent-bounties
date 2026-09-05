#!/usr/bin/env python3
"""Known-good and known-bad rehearsal for the Glama onboarding benchmark."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


HERE = Path(__file__).resolve().parent
FIXTURE = HERE / "fixtures" / "passing"
CHECK = HERE / "check.py"


def run(workspace: Path, should_pass: bool, label: str) -> None:
    env = dict(os.environ)
    env["WORKSPACE_ROOT"] = str(workspace)
    completed = subprocess.run(
        [sys.executable, str(CHECK)],
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=10,
        check=False,
    )
    passed = completed.returncode == 0
    if passed != should_pass:
        raise SystemExit(
            f"{label}: expected {'pass' if should_pass else 'failure'}, "
            f"got exit {completed.returncode}\n{completed.stdout}"
        )


with tempfile.TemporaryDirectory(prefix="glama-onboarding-benchmark-") as raw:
    root = Path(raw)
    passing = root / "passing"
    shutil.copytree(FIXTURE, passing)
    run(passing, True, "known-good")

    cases = {
        "wrong-rail": lambda data: data["mcp"].__setitem__("first_touch_rail", "direct"),
        "missing-tool": lambda data: data["mcp"].__setitem__(
            "prepare_bounty_post_discoverable", False
        ),
        "wallet-authority": lambda data: data["wallet_boundary"].__setitem__(
            "agent_received_wallet_signature", True
        ),
        "not-settled": lambda data: data["lifecycle"]["settled"].__setitem__(
            "event", "SubmissionVerified"
        ),
        "not-excluded": lambda data: data.__setitem__("measurement_exclusion", "none"),
    }
    for name, mutate in cases.items():
        candidate = root / name
        shutil.copytree(FIXTURE, candidate)
        evidence_path = candidate / "glama-onboarding-audit.json"
        data = json.loads(evidence_path.read_text(encoding="utf-8"))
        mutate(data)
        evidence_path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        run(candidate, False, name)

print("Glama onboarding audit self-test passed: 1 positive, 5 negative")

