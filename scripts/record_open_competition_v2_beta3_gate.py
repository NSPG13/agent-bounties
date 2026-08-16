#!/usr/bin/env python3
"""Record one Beta3 release gate against exact immutable evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
from typing import Any

import build_open_competition_v2_beta3_release as release


OWNER_GATES = {
    "owner_mainnet_deployment_approved",
    "owner_public_beta_activation_approved",
    "graduation_review_approved",
}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def exact_checkout_allowing_gate_manifest(source_commit: str) -> None:
    require(re.fullmatch(r"[0-9a-f]{40}", source_commit) is not None, "source commit is invalid")
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=release.ROOT, text=True
    ).strip()
    require(head == source_commit, "source commit differs from Git HEAD")
    for cached in (False, True):
        command = ["git", "diff"]
        if cached:
            command.append("--cached")
        command.extend(["--name-only", "--"])
        changed = {
            line.strip()
            for line in subprocess.check_output(
                command, cwd=release.ROOT, text=True
            ).splitlines()
            if line.strip()
        }
        require(
            changed <= {release.GATE_MANIFEST_RELATIVE},
            "tracked changes outside the mutable gate manifest make the release inexact",
        )


def record_gate(
    manifest: dict[str, Any],
    *,
    gate: str,
    source_commit: str,
    subject_hash: str,
    evidence_bytes: bytes,
    uri: str,
    owner_risk_hash: str | None,
) -> dict[str, Any]:
    gates = manifest.get("gates", {})
    evidence = manifest.get("evidence", {})
    require(gate in gates and gate in evidence, "unknown release gate")
    require(uri.startswith("https://"), "evidence URI must use HTTPS")
    expected_risk_hash = release.keccak256(manifest["beta_risk_preimage"].encode())
    if gate in OWNER_GATES:
        require(
            owner_risk_hash == expected_risk_hash,
            "owner approval must acknowledge the exact Beta3 risk hash",
        )
    elif owner_risk_hash is not None:
        raise RuntimeError("owner risk acknowledgement is valid only for owner approval gates")
    gates[gate] = True
    evidence[gate] = {
        "source_commit": source_commit,
        "subject_hash": subject_hash,
        "evidence_hash": "0x" + hashlib.sha256(evidence_bytes).hexdigest(),
        "uri": uri,
    }
    return manifest


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--gate", required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--uri", required=True)
    parser.add_argument("--source-commit")
    parser.add_argument("--owner-risk-hash")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=release.ROOT / release.GATE_MANIFEST_RELATIVE,
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    source_commit = args.source_commit or subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=release.ROOT, text=True
    ).strip()
    exact_checkout_allowing_gate_manifest(source_commit)
    require(args.evidence.is_file(), "evidence file is unavailable")
    subject_hash = release.repository_subject_hash(source_commit)
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    updated = record_gate(
        manifest,
        gate=args.gate,
        source_commit=source_commit,
        subject_hash=subject_hash,
        evidence_bytes=args.evidence.read_bytes(),
        uri=args.uri,
        owner_risk_hash=args.owner_risk_hash,
    )
    args.manifest.write_text(json.dumps(updated, indent=2) + "\n", encoding="utf-8")
    release.load_gates(args.manifest, subject_hash)
    print(
        json.dumps(
            {
                "gate": args.gate,
                "source_commit": source_commit,
                "subject_hash": subject_hash,
                "evidence_hash": updated["evidence"][args.gate]["evidence_hash"],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
