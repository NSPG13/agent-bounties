#!/usr/bin/env python3
"""Build a hash-bound Beta2 trusted-setup manifest from verified setup files."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any


SCHEMA = "agent-bounties/open-competition-v2-beta2-trusted-setup-v1"
CIRCUIT_VERSION = "agent-bounties-sp1-safe-v4"
MODELS = {"groth16": "mpc_phase2", "plonk": "public_mpc_kzg_srs"}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verification_evidence(
    path: Path,
    *,
    system: str,
    constraint_hash: str,
    proving_key_hash: str,
    verifying_key_hash: str,
    transcript_hash: str,
) -> tuple[dict[str, Any], int, str]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema_version") != (
        "agent-bounties/open-competition-v2-beta2-setup-verification-evidence-v1"
    ):
        raise ValueError(f"{system} setup verification evidence schema mismatch")
    if value.get("proof_system") != system:
        raise ValueError(f"{system} setup verification evidence targets another system")
    if value.get("security_model") != MODELS[system]:
        raise ValueError(f"{system} setup verification evidence has an unsafe model")
    if value.get("verification_passed") is not True:
        raise ValueError(f"{system} setup verification did not pass")
    expected_hashes = {
        "constraint_system_sha256": constraint_hash,
        "proving_key_sha256": proving_key_hash,
        "verifying_key_sha256": verifying_key_hash,
        "transcript_sha256": transcript_hash,
    }
    for field, expected in expected_hashes.items():
        if value.get(field) != expected:
            raise ValueError(f"{system} setup verification evidence {field} mismatch")
    count = value.get("contribution_count")
    if not isinstance(count, int) or count < 2:
        raise ValueError(f"{system} setup requires at least two contributions")
    uri = value.get("ceremony_uri")
    if not isinstance(uri, str) or not uri.startswith("https://"):
        raise ValueError(f"{system} setup ceremony URI must use HTTPS")
    return value, count, uri


def system_record(args: argparse.Namespace, system: str) -> dict[str, Any]:
    constraint = getattr(args, f"{system}_constraint_system")
    proving_key = getattr(args, f"{system}_proving_key")
    verifying_key = getattr(args, f"{system}_verifying_key")
    transcript = getattr(args, f"{system}_transcript")
    evidence_path = getattr(args, f"{system}_verification_evidence")
    verifier_hash = getattr(args, f"{system}_verifier_hash")
    if not re.fullmatch(r"0x[0-9a-f]{64}", verifier_hash):
        raise ValueError(f"{system} verifier hash must be lowercase bytes32")
    hashes = {
        "constraint_system_sha256": sha256(constraint),
        "proving_key_sha256": sha256(proving_key),
        "verifying_key_sha256": sha256(verifying_key),
        "transcript_sha256": sha256(transcript),
    }
    evidence, count, uri = verification_evidence(
        evidence_path,
        system=system,
        constraint_hash=hashes["constraint_system_sha256"],
        proving_key_hash=hashes["proving_key_sha256"],
        verifying_key_hash=hashes["verifying_key_sha256"],
        transcript_hash=hashes["transcript_sha256"],
    )
    if evidence.get("verifier_hash") != verifier_hash:
        raise ValueError(f"{system} setup verification evidence verifier hash mismatch")
    return {
        "security_model": MODELS[system],
        "verification_passed": True,
        "verifier_hash": verifier_hash,
        **hashes,
        "verification_evidence_sha256": sha256(evidence_path),
        "ceremony_uri": uri,
        "contribution_count": count,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sp1-source-commit", required=True)
    for system in ("groth16", "plonk"):
        parser.add_argument(f"--{system}-constraint-system", type=Path, required=True)
        parser.add_argument(f"--{system}-proving-key", type=Path, required=True)
        parser.add_argument(f"--{system}-verifying-key", type=Path, required=True)
        parser.add_argument(f"--{system}-transcript", type=Path, required=True)
        parser.add_argument(f"--{system}-verification-evidence", type=Path, required=True)
        parser.add_argument(f"--{system}-verifier-hash", required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", args.sp1_source_commit):
        raise SystemExit("--sp1-source-commit must be a full lowercase Git commit")
    value = {
        "schema_version": SCHEMA,
        "sp1_source_commit": args.sp1_source_commit,
        "circuit_version": CIRCUIT_VERSION,
        "mainnet_eligible": True,
        "proof_systems": {
            system: system_record(args, system) for system in ("groth16", "plonk")
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "mainnet_eligible": True}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
