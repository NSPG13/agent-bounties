#!/usr/bin/env python3
"""Verify the ordered Beta2 Groth16 MPC transcript and emit setup evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any


COMMAND_SCHEMA = "agent-bounties/open-competition-v2-beta2-groth16-mpc-command-v1"
TRANSCRIPT_SCHEMA = "agent-bounties/open-competition-v2-beta2-groth16-mpc-transcript-v1"
EVIDENCE_SCHEMA = "agent-bounties/open-competition-v2-beta2-setup-verification-evidence-v1"
COMMANDS = (
    "init-phase1",
    "contribute-phase1",
    "contribute-phase1",
    "contribute-phase1",
    "verify-phase1",
    "init-phase2",
    "contribute-phase2",
    "contribute-phase2",
    "contribute-phase2",
    "finalize",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(record: dict[str, Any], field: str) -> dict[str, str]:
    values = record.get(field)
    if not isinstance(values, list) or not values:
        raise ValueError(f"ceremony record {field} is empty")
    result = {}
    for value in values:
        if not isinstance(value, dict):
            raise ValueError(f"ceremony record {field} contains a non-object")
        name = value.get("path")
        digest = value.get("sha256")
        if not isinstance(name, str) or name in result or "/" in name or "\\" in name:
            raise ValueError(f"ceremony record {field} path is ambiguous")
        if not re.fullmatch(r"[0-9a-f]{64}", str(digest)):
            raise ValueError(f"ceremony record {field} digest is invalid")
        result[name] = digest
    return result


def verify(transcript: dict[str, Any], root: Path, r1cs: Path) -> tuple[int, str]:
    if transcript.get("schema_version") != TRANSCRIPT_SCHEMA:
        raise ValueError("Groth16 transcript schema mismatch")
    records = transcript.get("records")
    if not isinstance(records, list) or len(records) != len(COMMANDS):
        raise ValueError("Groth16 transcript command inventory mismatch")
    for index, (record, command) in enumerate(zip(records, COMMANDS, strict=True)):
        if record.get("schema_version") != COMMAND_SCHEMA:
            raise ValueError(f"Groth16 command {index} schema mismatch")
        if record.get("command") != command or record.get("verified") is not True:
            raise ValueError(f"Groth16 command {index} is not verified {command}")
    for offset in (1, 2, 3, 6, 7, 8):
        expected_id = offset if offset <= 3 else offset - 5
        if records[offset].get("contribution_id") != expected_id:
            raise ValueError("Groth16 contribution sequence is not contiguous")
    r1cs_hash = sha256(r1cs)
    if inventory(records[0], "inputs").get(r1cs.name) != r1cs_hash:
        raise ValueError("Groth16 Phase 1 targets another R1CS")
    chains = ((0, 1), (1, 2), (2, 3), (4, 5), (5, 6), (6, 7), (7, 8))
    for previous, current in chains:
        outputs = inventory(records[previous], "outputs")
        inputs = inventory(records[current], "inputs")
        if not set(outputs.items()).issubset(set(inputs.items())):
            raise ValueError("Groth16 contribution hash chain is broken")
    for verify_index, contribution_indexes in ((4, (1, 2, 3)), (9, (6, 7, 8))):
        inputs = inventory(records[verify_index], "inputs")
        for contribution_index in contribution_indexes:
            if not set(inventory(records[contribution_index], "outputs").items()).issubset(
                set(inputs.items())
            ):
                raise ValueError("Groth16 verifier did not consume every contribution")
    for phase, verify_index in (("phase1", 4), ("phase2", 9)):
        beacon = transcript.get(f"{phase}_beacon")
        if not isinstance(beacon, dict):
            raise ValueError(f"Groth16 {phase} beacon is missing")
        randomness = beacon.get("randomness")
        if not re.fullmatch(r"[0-9a-f]{64}", str(randomness)):
            raise ValueError(f"Groth16 {phase} beacon randomness is invalid")
        if records[verify_index].get("beacon_hex") != "0x" + randomness:
            raise ValueError(f"Groth16 {phase} verifier used another beacon")
    if int(transcript["phase2_beacon"].get("round", 0)) <= int(
        transcript["phase1_beacon"].get("round", 0)
    ):
        raise ValueError("Groth16 Phase 2 beacon is not newer than Phase 1")
    final_outputs = inventory(records[9], "outputs")
    for name in ("groth16_pk.bin", "groth16_vk.bin", "Groth16Verifier.sol"):
        if final_outputs.get(name) != sha256(root / name):
            raise ValueError(f"Groth16 final output mismatch: {name}")
    return 3, r1cs_hash


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--r1cs", type=Path, required=True)
    parser.add_argument("--ceremony-uri", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.ceremony_uri.startswith("https://"):
        raise SystemExit("--ceremony-uri must use HTTPS")
    transcript_path = args.root / "transcript.json"
    transcript = json.loads(transcript_path.read_text(encoding="utf-8"))
    count, constraint_hash = verify(transcript, args.root, args.r1cs)
    vk_hash = sha256(args.root / "groth16_vk.bin")
    evidence = {
        "schema_version": EVIDENCE_SCHEMA,
        "proof_system": "groth16",
        "security_model": "mpc_phase2",
        "verification_passed": True,
        "constraint_system_sha256": constraint_hash,
        "proving_key_sha256": sha256(args.root / "groth16_pk.bin"),
        "verifying_key_sha256": vk_hash,
        "transcript_sha256": sha256(transcript_path),
        "verifier_hash": "0x" + vk_hash,
        "contribution_count": count,
        "phase1_contribution_count": count,
        "phase2_contribution_count": count,
        "ceremony_uri": args.ceremony_uri,
    }
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verifier_hash": "0x" + vk_hash, "contribution_count": count}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
