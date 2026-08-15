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


SCHEMA = "agent-bounties/open-competition-v2-beta2-verifier-assets-v2"
TRUSTED_SETUP_SCHEMA = "agent-bounties/open-competition-v2-beta2-trusted-setup-v1"
CIRCUIT_VERSION = "agent-bounties-sp1-safe-v4"


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


def trusted_setup_provenance(
    path: Path | None,
    *,
    sp1_source_commit: str,
    verifier_hashes: dict[str, str],
    setup_files: dict[str, dict[str, Path]] | None,
    allow_test_only: bool,
) -> dict[str, Any]:
    if path is None:
        if not allow_test_only:
            raise ValueError(
                "trusted setup provenance is required unless test-only setup is explicit"
            )
        return {
            "state": "test_only_unsafe",
            "mainnet_eligible": False,
            "manifest_sha256": None,
            "systems": {
                "groth16": {"security_model": "single_party_local_setup"},
                "plonk": {"security_model": "unverified_setup_provenance"},
            },
        }
    if allow_test_only:
        raise ValueError("trusted setup and test-only setup are mutually exclusive")
    if setup_files is None or set(setup_files) != {"groth16", "plonk"}:
        raise ValueError("trusted setup requires exact files for both proof systems")
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema_version") != TRUSTED_SETUP_SCHEMA:
        raise ValueError("trusted setup manifest schema mismatch")
    if value.get("sp1_source_commit") != sp1_source_commit:
        raise ValueError("trusted setup targets another SP1 source commit")
    if value.get("circuit_version") != CIRCUIT_VERSION:
        raise ValueError("trusted setup targets another circuit version")
    systems = value.get("proof_systems")
    if not isinstance(systems, dict) or set(systems) != {"groth16", "plonk"}:
        raise ValueError("trusted setup must contain exactly Groth16 and PLONK")
    expected_models = {
        "groth16": "mpc_phase2",
        "plonk": "public_mpc_kzg_srs",
    }
    for name, expected_model in expected_models.items():
        item = systems[name]
        if not isinstance(item, dict):
            raise ValueError(f"{name} trusted setup provenance must be an object")
        if item.get("security_model") != expected_model:
            raise ValueError(f"{name} trusted setup security model is unsafe")
        if item.get("verification_passed") is not True:
            raise ValueError(f"{name} trusted setup transcript is unverified")
        if item.get("verifier_hash") != verifier_hashes[name]:
            raise ValueError(f"{name} trusted setup verifier hash mismatch")
        for field in (
            "constraint_system_sha256",
            "proving_key_sha256",
            "verifying_key_sha256",
            "transcript_sha256",
            "verification_evidence_sha256",
        ):
            if not re.fullmatch(r"[0-9a-f]{64}", str(item.get(field, ""))):
                raise ValueError(f"{name} trusted setup {field} is invalid")
        expected_files = {
            "constraint_system_sha256": "constraint_system",
            "proving_key_sha256": "proving_key",
            "verifying_key_sha256": "verifying_key",
            "transcript_sha256": "transcript",
            "verification_evidence_sha256": "verification_evidence",
        }
        files = setup_files.get(name)
        if not isinstance(files, dict) or set(files) != set(expected_files.values()):
            raise ValueError(f"{name} trusted setup file inventory is incomplete")
        for manifest_field, file_field in expected_files.items():
            file_path = files[file_field]
            observed = hashlib.sha256(file_path.read_bytes()).hexdigest()
            if observed != item[manifest_field]:
                raise ValueError(f"{name} trusted setup {manifest_field} does not match the file")
        uri = item.get("ceremony_uri")
        if not isinstance(uri, str) or not uri.startswith("https://"):
            raise ValueError(f"{name} trusted setup ceremony URI must use HTTPS")
    if int(systems["groth16"].get("contribution_count", 0)) < 2:
        raise ValueError("Groth16 phase 2 requires at least two recorded contributions")
    if int(systems["plonk"].get("contribution_count", 0)) < 2:
        raise ValueError("PLONK SRS requires a multi-party ceremony transcript")
    if value.get("mainnet_eligible") is not True:
        raise ValueError("trusted setup manifest is not mainnet eligible")
    return {
        "state": "trusted_mpc",
        "mainnet_eligible": True,
        "manifest_sha256": "0x"
        + hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest(),
        "systems": systems,
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
    parser.add_argument("--trusted-setup-manifest", type=Path)
    for system in ("groth16", "plonk"):
        parser.add_argument(f"--{system}-constraint-system", type=Path)
        parser.add_argument(f"--{system}-proving-key", type=Path)
        parser.add_argument(f"--{system}-verifying-key", type=Path)
        parser.add_argument(f"--{system}-setup-transcript", type=Path)
        parser.add_argument(f"--{system}-setup-verification-evidence", type=Path)
    parser.add_argument("--allow-test-only-setup", action="store_true")
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
    setup_provenance = trusted_setup_provenance(
        args.trusted_setup_manifest,
        sp1_source_commit=args.sp1_source_commit,
        verifier_hashes={
            "groth16": args.groth16_verifier_hash,
            "plonk": args.plonk_verifier_hash,
        },
        setup_files=(
            {
                system: {
                    "constraint_system": getattr(args, f"{system}_constraint_system"),
                    "proving_key": getattr(args, f"{system}_proving_key"),
                    "verifying_key": getattr(args, f"{system}_verifying_key"),
                    "transcript": getattr(args, f"{system}_setup_transcript"),
                    "verification_evidence": getattr(
                        args, f"{system}_setup_verification_evidence"
                    ),
                }
                for system in ("groth16", "plonk")
            }
            if args.trusted_setup_manifest is not None
            else None
        ),
        allow_test_only=args.allow_test_only_setup,
    )
    value = {
        "schema_version": SCHEMA,
        "sp1_source_commit": args.sp1_source_commit,
        "circuit_version": CIRCUIT_VERSION,
        "gpu_proving_enabled": False,
        "asset_state": "self_verified" if all(proof_paths) else "verifier_bytecode_only",
        "setup_provenance": setup_provenance,
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
