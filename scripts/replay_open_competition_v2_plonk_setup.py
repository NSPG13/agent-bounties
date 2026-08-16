#!/usr/bin/env python3
"""Replay the pinned Aztec Ignition chain and reproduce the PLONK SRS."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import subprocess
import tempfile


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sp1-root", type=Path, required=True)
    parser.add_argument("--sp1-source-commit", required=True)
    parser.add_argument("--expected-srs", type=Path, required=True)
    parser.add_argument("--replayed-srs", type=Path, required=True)
    parser.add_argument("--log", type=Path, required=True)
    args = parser.parse_args()

    sp1_root = args.sp1_root.resolve()
    module_root = sp1_root / "crates" / "recursion" / "gnark-ffi" / "go"
    actual_commit = subprocess.check_output(
        ["git", "-C", str(sp1_root), "rev-parse", "HEAD"], text=True
    ).strip()
    if actual_commit != args.sp1_source_commit:
        raise SystemExit(
            f"SP1 source commit mismatch: expected {args.sp1_source_commit}, got {actual_commit}"
        )
    if not args.expected_srs.is_file():
        raise SystemExit(f"expected SRS is missing: {args.expected_srs}")

    args.replayed_srs.parent.mkdir(parents=True, exist_ok=True)
    args.log.parent.mkdir(parents=True, exist_ok=True)
    args.replayed_srs.unlink(missing_ok=True)
    helper = """package main

import (
    "os"

    "github.com/succinctlabs/sp1-recursion-gnark/sp1/trusted_setup"
)

func main() {
    if len(os.Args) != 2 {
        panic("expected output path")
    }
    trusted_setup.DownloadAndSaveAztecIgnitionSrs(174, os.Args[1])
}
"""
    with tempfile.TemporaryDirectory(prefix="agent-bounties-plonk-replay-") as directory:
        helper_path = Path(directory) / "main.go"
        helper_path.write_text(helper, encoding="utf-8")
        with args.log.open("w", encoding="utf-8") as log:
            process = subprocess.Popen(
                ["go", "run", str(helper_path), str(args.replayed_srs.resolve())],
                cwd=module_root,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                bufsize=1,
            )
            assert process.stdout is not None
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
                log.flush()
            status = process.wait()
            if status != 0:
                raise SystemExit(f"PLONK setup replay failed with exit code {status}")

    expected_hash = sha256(args.expected_srs)
    replayed_hash = sha256(args.replayed_srs)
    if replayed_hash != expected_hash:
        raise SystemExit(
            f"PLONK SRS mismatch: expected {expected_hash}, replayed {replayed_hash}"
        )
    print(f"replayed_srs_sha256={replayed_hash}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
