#!/usr/bin/env python3
"""Verify a pinned Groth16 Phase 1 checkpoint before resuming Phase 2."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any


COMMAND_SCHEMA = "agent-bounties/open-competition-v2-beta3-groth16-mpc-command-v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def inventory(record: dict[str, Any], field: str) -> dict[str, str]:
    return {str(item["path"]): str(item["sha256"]) for item in record.get(field, [])}


def verify_checkpoint(
    root: Path,
    r1cs: Path,
    expected_r1cs: str,
    expected_phase1_1: str,
    expected_phase1_2: str,
    expected_commons: str,
    expected_beacon: str,
) -> None:
    expected_digests = {
        r1cs.name: expected_r1cs,
        "phase1-1.bin": expected_phase1_1,
        "phase1-2.bin": expected_phase1_2,
    }
    for digest in (*expected_digests.values(), expected_commons):
        if not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ValueError("expected Phase 1 digest is invalid")
    if not re.fullmatch(r"0x[0-9a-f]{64}", expected_beacon):
        raise ValueError("expected Phase 1 beacon is invalid")

    actual_digests = {
        r1cs.name: sha256(r1cs),
        "phase1-1.bin": sha256(root / "phase1-1.bin"),
        "phase1-2.bin": sha256(root / "phase1-2.bin"),
    }
    if actual_digests != expected_digests:
        raise ValueError("Phase 1 checkpoint input hash mismatch")
    if sha256(root / "phase1-commons.bin") != expected_commons:
        raise ValueError("Phase 1 commons hash mismatch")

    record = json.loads((root / "05-phase1-verify.json").read_text(encoding="utf-8"))
    if record.get("schema_version") != COMMAND_SCHEMA:
        raise ValueError("Phase 1 checkpoint schema mismatch")
    if record.get("command") != "verify-phase1" or record.get("verified") is not True:
        raise ValueError("Phase 1 checkpoint is not a verified Phase 1 record")
    if inventory(record, "inputs") != expected_digests:
        raise ValueError("Phase 1 checkpoint record inputs mismatch")
    if inventory(record, "outputs") != {"phase1-commons.bin": expected_commons}:
        raise ValueError("Phase 1 checkpoint record output mismatch")
    if record.get("beacon_hex") != expected_beacon:
        raise ValueError("Phase 1 checkpoint record beacon mismatch")

    beacon = json.loads((root / "phase1-beacon.json").read_text(encoding="utf-8"))
    if beacon.get("randomness") != expected_beacon[2:] or int(beacon.get("round", 0)) <= 0:
        raise ValueError("Phase 1 checkpoint beacon evidence mismatch")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--r1cs", type=Path, required=True)
    parser.add_argument("--r1cs-sha256", required=True)
    parser.add_argument("--phase1-1-sha256", required=True)
    parser.add_argument("--phase1-2-sha256", required=True)
    parser.add_argument("--commons-sha256", required=True)
    parser.add_argument("--beacon", required=True)
    args = parser.parse_args()
    verify_checkpoint(
        args.root,
        args.r1cs,
        args.r1cs_sha256,
        args.phase1_1_sha256,
        args.phase1_2_sha256,
        args.commons_sha256,
        args.beacon,
    )
    print(json.dumps({"verified": True, "commons_sha256": args.commons_sha256}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
