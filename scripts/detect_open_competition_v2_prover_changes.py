#!/usr/bin/env python3
"""Skip costly PR prover builds only when their inputs are known unchanged."""

from __future__ import annotations

import fnmatch
import json
import os
from pathlib import Path
import re
import subprocess


# Keep this conservative: manual releases always rebuild every profile. Service
# changes still run the normal repository/API/payment gates and release tests.
PROVER_INPUTS = (
    ".github/workflows/open-competition-v2-beta3-release.yml",
    ".github/actions/setup-patched-sp1/**",
    ".github/actions/setup-protoc/**",
    ".github/actions/setup-rust/**",
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/**",
    "rust-toolchain*",
    "crates/competition-metric-*/**",
    "programs/public-vector-metric-v1/**",
    "programs/structured-artifact-metric-v1/**",
    "programs/forward-canonical-gmv-attribution-metric-v2/**",
    "deployments/open-competition-v2-beta3-*",
    "ops/open-competition-v2-gnark-safe.Dockerfile",
    "ops/open-competition-v2-ceremony.Dockerfile",
    "ops/open-competition-v2-prover*",
    "tools/open-competition-v2-ceremony/**",
    "scripts/*sp1*",
    "scripts/build_open_competition_v2_metric_elf.sh",
    "scripts/build_open_competition_v2_circuits.sh",
    "scripts/verify-open-competition-v2-metric-release.py",
    "scripts/verify_open_competition_v2_wrap_template.py",
    "scripts/verify_open_competition_v2_gnark_image.py",
    "scripts/detect_open_competition_v2_prover_changes.py",
)


def decide(event_name: str, base_sha: str, root: Path) -> dict:
    if event_name != "pull_request":
        return {"run": True, "reason": "full_non_pr_validation"}
    if not re.fullmatch(r"[0-9a-fA-F]{40}", base_sha):
        return {"run": True, "reason": "base_revision_unavailable"}
    try:
        # Checkout is the PR merge commit; compare its tree with that PR's base.
        # No rename detection means moving an input away still lists its deletion.
        result = subprocess.run(
            ["git", "diff", "--name-only", "--no-renames", "-z", base_sha, "HEAD", "--"],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=30,
        )
        paths = [path for path in result.stdout.decode("utf-8").split("\0") if path]
    except (OSError, subprocess.SubprocessError, UnicodeError):
        return {"run": True, "reason": "change_detection_failed"}
    changed = sorted(
        path for path in paths if any(fnmatch.fnmatchcase(path, pattern) for pattern in PROVER_INPUTS)
    )
    return {
        "run": bool(changed),
        "reason": "prover_inputs_changed" if changed else "prover_inputs_unchanged",
        "changed_inputs": changed,
    }


def main() -> int:
    decision = decide(
        os.environ.get("GITHUB_EVENT_NAME", ""),
        os.environ.get("PR_BASE_SHA", ""),
        Path(__file__).resolve().parents[1],
    )
    print(json.dumps(decision, sort_keys=True))
    if output := os.environ.get("GITHUB_OUTPUT"):
        with Path(output).open("a", encoding="utf-8") as target:
            target.write(f"run={str(decision['run']).lower()}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
