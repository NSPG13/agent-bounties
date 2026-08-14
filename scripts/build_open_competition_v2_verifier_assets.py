#!/usr/bin/env python3
"""Freeze patched-SP1 verifier bytecode and self-verified proof evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any

from _shared.evm import artifact_hex, keccak256


SCHEMA = "agent-bounties/open-competition-v2-beta2-verifier-assets-v1"
CIRCUIT_VERSION = "agent-bounties-sp1-safe-v1"


def proof_evidence_hash(path: Path) -> str:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("self_verified") is not True:
        raise ValueError(f"{path} is not self-verified proof evidence")
    if value.get("gpu_proving_enabled") is not False:
        raise ValueError(f"{path} does not prove CPU-only generation")
    return "0x" + hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def verifier(name: str, path: Path, verifier_hash: str) -> dict[str, Any]:
    if not re.fullmatch(r"0x[0-9a-f]{64}", verifier_hash):
        raise ValueError(f"{name} verifier hash must be lowercase bytes32")
    value = json.loads(path.read_text(encoding="utf-8"))
    creation = artifact_hex(value.get("bytecode"), f"{name}.bytecode")
    runtime = artifact_hex(value.get("deployedBytecode"), f"{name}.deployedBytecode")
    return {
        "verifier_hash": verifier_hash,
        "creation_code": "0x" + creation.hex(),
        "creation_code_hash": keccak256(creation),
        "runtime_code": "0x" + runtime.hex(),
        "runtime_code_hash": keccak256(runtime),
        "artifact_sha256": "0x" + hashlib.sha256(path.read_bytes()).hexdigest(),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sp1-source-commit", required=True)
    parser.add_argument("--groth16-artifact", type=Path, required=True)
    parser.add_argument("--groth16-verifier-hash", required=True)
    parser.add_argument("--plonk-artifact", type=Path, required=True)
    parser.add_argument("--plonk-verifier-hash", required=True)
    parser.add_argument("--groth16-proof-evidence", type=Path)
    parser.add_argument("--plonk-proof-evidence-1", type=Path)
    parser.add_argument("--plonk-proof-evidence-2", type=Path)
    parser.add_argument("--allow-pending-proof-evidence", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.sp1_source_commit):
        raise SystemExit("--sp1-source-commit must be a full lowercase Git commit")
    proof_paths = (
        args.groth16_proof_evidence,
        args.plonk_proof_evidence_1,
        args.plonk_proof_evidence_2,
    )
    if any(proof_paths) and not all(proof_paths):
        raise SystemExit("all three proof evidence files are required together")
    if not all(proof_paths) and not args.allow_pending_proof_evidence:
        raise SystemExit("proof evidence is required unless planning is explicitly pending")
    proof_evidence = (
        {
            "groth16_self_verified": proof_evidence_hash(args.groth16_proof_evidence),
            "plonk_self_verified_1": proof_evidence_hash(args.plonk_proof_evidence_1),
            "plonk_self_verified_2": proof_evidence_hash(args.plonk_proof_evidence_2),
        }
        if all(proof_paths)
        else {
            "groth16_self_verified": None,
            "plonk_self_verified_1": None,
            "plonk_self_verified_2": None,
        }
    )
    value = {
        "schema_version": SCHEMA,
        "sp1_source_commit": args.sp1_source_commit,
        "circuit_version": CIRCUIT_VERSION,
        "gpu_proving_enabled": False,
        "asset_state": "self_verified" if all(proof_paths) else "verifier_bytecode_only",
        "proof_systems": {
            "groth16": verifier(
                "groth16", args.groth16_artifact, args.groth16_verifier_hash
            ),
            "plonk": verifier("plonk", args.plonk_artifact, args.plonk_verifier_hash),
        },
        "proof_evidence": proof_evidence,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "schema_version": SCHEMA}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
