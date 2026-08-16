#!/usr/bin/env python3
"""Verify and freeze the Beta3 Aztec Ignition PLONK setup evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re


EVIDENCE_SCHEMA = "agent-bounties/open-competition-v2-beta3-setup-verification-evidence-v1"
TRANSCRIPT_SCHEMA = "agent-bounties/open-competition-v2-beta3-plonk-ignition-transcript-v1"
START_INDEX = 174


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(8 * 1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_matching_srs(expected: Path, replayed: Path) -> str:
    expected_hash = sha256(expected)
    replayed_hash = sha256(replayed)
    if replayed_hash != expected_hash:
        raise ValueError("independently replayed PLONK SRS differs from the build SRS")
    return replayed_hash


def verify_source(build_go: Path, trusted_setup_go: Path) -> dict[str, str]:
    build = build_go.read_text(encoding="utf-8")
    setup = trusted_setup_go.read_text(encoding="utf-8")
    required_build = (
        'DownloadAndSaveAztecIgnitionSrs(174, srsFileName)',
        'if !strings.Contains(dataDir, "dev")',
    )
    required_setup = {
        'BaseURL:  "https://aztec-ignition.s3.amazonaws.com/"': 1,
        'Ceremony: "MAIN IGNITION"': 1,
        'if !next.Follows(&current)': 2,
        'sanityCheck(&srs)': 1,
    }
    for fragment in required_build:
        if build.count(fragment) != 1:
            raise ValueError(f"PLONK build source lacks exact fragment: {fragment}")
    for fragment, expected_count in required_setup.items():
        if setup.count(fragment) != expected_count:
            raise ValueError(
                f"PLONK setup source requires {expected_count} exact occurrences: {fragment}"
            )
    return {"build_go_sha256": sha256(build_go), "trusted_setup_go_sha256": sha256(trusted_setup_go)}


def contribution_count(log: str, manifest: dict) -> tuple[int, int]:
    if "success ✅: all contributions are valid" not in log:
        raise ValueError("PLONK setup log lacks contribution-chain success")
    if "success ✅: kzg sanity check with SRS" not in log:
        raise ValueError("PLONK setup log lacks KZG sanity-check success")
    if manifest.get("name") != "MAIN IGNITION":
        raise ValueError("PLONK setup manifest is not MAIN IGNITION")
    participants = manifest.get("participants")
    if not isinstance(participants, list):
        raise ValueError("PLONK setup manifest lacks participant inventory")
    participant_count = len(participants)
    count = participant_count - START_INDEX
    if count < 2:
        raise ValueError("PLONK setup evidence has fewer than two contributions")
    processed = [int(value) for value in re.findall(r"processing contribution\s+(\d+)", log)]
    expected_processed = list(range(START_INDEX + 3, participant_count + 1))
    if processed != expected_processed:
        raise ValueError("PLONK setup log does not match manifest contribution inventory")
    return participant_count, count


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sp1-build-source", type=Path, required=True)
    parser.add_argument("--sp1-trusted-setup-source", type=Path, required=True)
    parser.add_argument("--build-log", type=Path, required=True)
    parser.add_argument("--setup-replay-log", type=Path, required=True)
    parser.add_argument("--setup-manifest", type=Path, required=True)
    parser.add_argument("--replayed-srs", type=Path, required=True)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--transcript-output", type=Path, required=True)
    parser.add_argument("--evidence-output", type=Path, required=True)
    args = parser.parse_args()
    if "dev" in str(args.build_dir).lower():
        raise SystemExit("PLONK build directory selects SP1's unsafe dev setup path")
    source_hashes = verify_source(args.sp1_build_source, args.sp1_trusted_setup_source)
    setup_log = args.setup_replay_log.read_text(encoding="utf-8")
    setup_manifest = json.loads(args.setup_manifest.read_text(encoding="utf-8"))
    final_number, count = contribution_count(setup_log, setup_manifest)
    files = {
        "constraint_system": args.build_dir / "plonk_circuit.bin",
        "proving_key": args.build_dir / "plonk_pk.bin",
        "verifying_key": args.build_dir / "plonk_vk.bin",
        "srs": args.build_dir / "srs.bin",
        "srs_lagrange": args.build_dir / "srs_lagrange.bin",
    }
    hashes = {f"{name}_sha256": sha256(path) for name, path in files.items()}
    replayed_srs_sha256 = require_matching_srs(files["srs"], args.replayed_srs)
    transcript = {
        "schema_version": TRANSCRIPT_SCHEMA,
        "ceremony": "MAIN IGNITION",
        "base_url": "https://aztec-ignition.s3.amazonaws.com/",
        "start_index": START_INDEX,
        "final_contribution_number": final_number,
        "contribution_count": count,
        "build_log_sha256": sha256(args.build_log),
        "setup_replay_log_sha256": sha256(args.setup_replay_log),
        "setup_manifest_sha256": sha256(args.setup_manifest),
        "replayed_srs_sha256": replayed_srs_sha256,
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
