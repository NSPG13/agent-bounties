#!/usr/bin/env python3
"""Verify and freeze the Beta2 Aztec Ignition PLONK setup evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re


EVIDENCE_SCHEMA = "agent-bounties/open-competition-v2-beta2-setup-verification-evidence-v1"
TRANSCRIPT_SCHEMA = "agent-bounties/open-competition-v2-beta2-plonk-ignition-transcript-v1"
START_INDEX = 174


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify_source(build_go: Path, trusted_setup_go: Path) -> dict[str, str]:
    build = build_go.read_text(encoding="utf-8")
    setup = trusted_setup_go.read_text(encoding="utf-8")
    required_build = (
        'DownloadAndSaveAztecIgnitionSrs(174, srsFileName)',
        'if !strings.Contains(dataDir, "dev")',
    )
    required_setup = (
        'BaseURL:  "https://aztec-ignition.s3.amazonaws.com/"',
        'Ceremony: "MAIN IGNITION"',
        'if !next.Follows(&current)',
        'sanityCheck(&srs)',
    )
    for fragment in required_build:
        if build.count(fragment) != 1:
            raise ValueError(f"PLONK build source lacks exact fragment: {fragment}")
    for fragment in required_setup:
        if setup.count(fragment) != 1:
            raise ValueError(f"PLONK setup source lacks exact fragment: {fragment}")
    return {"build_go_sha256": sha256(build_go), "trusted_setup_go_sha256": sha256(trusted_setup_go)}


def contribution_count(log: str) -> tuple[int, int]:
    if "success ✅: all contributions are valid" not in log:
        raise ValueError("PLONK setup log lacks contribution-chain success")
    if "success ✅: kzg sanity check with SRS" not in log:
        raise ValueError("PLONK setup log lacks KZG sanity-check success")
    processed = [int(value) for value in re.findall(r"processing contribution\s+(\d+)", log)]
    if not processed:
        raise ValueError("PLONK setup log has no processed contribution inventory")
    final_number = max(processed)
    count = final_number - START_INDEX
    if count < 2:
        raise ValueError("PLONK setup evidence has fewer than two contributions")
    return final_number, count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sp1-build-source", type=Path, required=True)
    parser.add_argument("--sp1-trusted-setup-source", type=Path, required=True)
    parser.add_argument("--build-log", type=Path, required=True)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--transcript-output", type=Path, required=True)
    parser.add_argument("--evidence-output", type=Path, required=True)
    args = parser.parse_args()
    if "dev" in str(args.build_dir).lower():
        raise SystemExit("PLONK build directory selects SP1's unsafe dev setup path")
    source_hashes = verify_source(args.sp1_build_source, args.sp1_trusted_setup_source)
    log = args.build_log.read_text(encoding="utf-8")
    final_number, count = contribution_count(log)
    files = {
        "constraint_system": args.build_dir / "plonk_circuit.bin",
        "proving_key": args.build_dir / "plonk_pk.bin",
        "verifying_key": args.build_dir / "plonk_vk.bin",
        "srs": args.build_dir / "srs.bin",
        "srs_lagrange": args.build_dir / "srs_lagrange.bin",
    }
    hashes = {f"{name}_sha256": sha256(path) for name, path in files.items()}
    transcript = {
        "schema_version": TRANSCRIPT_SCHEMA,
        "ceremony": "MAIN IGNITION",
        "base_url": "https://aztec-ignition.s3.amazonaws.com/",
        "start_index": START_INDEX,
        "final_contribution_number": final_number,
        "contribution_count": count,
        "build_log_sha256": sha256(args.build_log),
        **source_hashes,
        "srs_sha256": hashes["srs_sha256"],
        "srs_lagrange_sha256": hashes["srs_lagrange_sha256"],
    }
    args.transcript_output.parent.mkdir(parents=True, exist_ok=True)
    args.transcript_output.write_text(json.dumps(transcript, indent=2) + "\n", encoding="utf-8")
    verifier_hash = "0x" + hashes["verifying_key_sha256"]
    evidence = {
        "schema_version": EVIDENCE_SCHEMA,
        "proof_system": "plonk",
        "security_model": "public_mpc_kzg_srs",
        "verification_passed": True,
        "constraint_system_sha256": hashes["constraint_system_sha256"],
        "proving_key_sha256": hashes["proving_key_sha256"],
        "verifying_key_sha256": hashes["verifying_key_sha256"],
        "transcript_sha256": sha256(args.transcript_output),
        "verifier_hash": verifier_hash,
        "contribution_count": count,
        "ceremony_uri": "https://aztec-ignition.s3.amazonaws.com/",
    }
    args.evidence_output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verifier_hash": verifier_hash, "contribution_count": count}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
