#!/usr/bin/env python3
"""Rehearse the frozen watchdog benchmark against its matching source fixture.

The benchmark binds the entire historical worker source set. Copying today's
repository into its known-good fixture makes unrelated schema changes fail the
immutable checker. Current verifier inputs are checked separately by the live
workflow source guards; this rehearsal must retain the benchmark's old inputs.
"""
from __future__ import annotations

import io
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
REVISION = "136d23971ba75fa27d93e8eef06ee7fb1d486303"
BENCHMARK = "benchmarks/direct-flagship-v1/verifier-settlement-watchdog"


def main() -> None:
    exists = subprocess.run(
        ["git", "cat-file", "-e", f"{REVISION}^{{commit}}"],
        cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        check=False,
    )
    if exists.returncode:
        # Shallow CI checkouts may not contain the reviewed fixture commit.
        subprocess.run(
            ["git", "fetch", "--no-tags", "--depth=1",
             "https://github.com/NSPG13/agent-bounties.git", REVISION],
            cwd=ROOT, check=True, timeout=120,
        )
    paths = subprocess.check_output(
        ["git", "ls-tree", "-r", "--name-only", REVISION, "--", BENCHMARK],
        cwd=ROOT, text=True,
    ).splitlines()
    if not paths:
        raise RuntimeError("The pinned benchmark fixture is missing")
    for path in paths:
        pinned = subprocess.check_output(["git", "show", f"{REVISION}:{path}"], cwd=ROOT)
        if (ROOT / path).read_bytes() != pinned:
            raise RuntimeError(f"Immutable benchmark changed: {path}; review its fixture revision")

    archive = subprocess.check_output(["git", "archive", "--format=tar", REVISION], cwd=ROOT)
    with tempfile.TemporaryDirectory(prefix="watchdog-reviewed-source-") as directory:
        fixture = Path(directory)
        with tarfile.open(fileobj=io.BytesIO(archive)) as bundle:
            bundle.extractall(fixture, filter="data")
        print(f"Rehearsing unchanged benchmark against reviewed source {REVISION}", flush=True)
        subprocess.run(
            [sys.executable, str(fixture / BENCHMARK / "rehearse.py")],
            cwd=fixture, check=True,
        )


if __name__ == "__main__":
    main()
