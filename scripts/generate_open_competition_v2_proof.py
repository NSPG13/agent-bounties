#!/usr/bin/env python3
"""Generate and validate one exact Open Competition V2 Beta3 SP1 proof artifact."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from _shared.evm import keccak256

import open_competition_v2_proof_rehearsal as rehearsal


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--prepared-dir", type=Path, required=True)
    parser.add_argument("--proof-name", choices=tuple(rehearsal.PROOF_SPECS), required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--log-dir", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    evidence = rehearsal.generate_proof(
        bundle, args.prepared_dir, args.proof_name, args.output, args.log_dir
    )
    proof = bytes.fromhex(evidence["proof_hex"].removeprefix("0x"))
    journal = bytes.fromhex(evidence["journal_hex"].removeprefix("0x"))
    print(json.dumps({
        "proof_name": args.proof_name,
        "mode": evidence["mode"],
        "proof_hash": keccak256(proof),
        "proof_bytes": len(proof),
        "journal_hash": keccak256(journal),
        "journal_bytes": len(journal),
        "elapsed_seconds": evidence["elapsed_seconds"],
        "output": str(args.output),
    }))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
