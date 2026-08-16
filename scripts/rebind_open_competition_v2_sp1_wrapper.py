#!/usr/bin/env python3
"""Bind an exact safe-v5 SP1 wrapper to a ceremony-generated verifier key."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import re


CIRCUIT_VERSION = "agent-bounties-sp1-safe-v5"
SYSTEMS = {
    "groth16": ("Groth16Verifier", "SP1VerifierGroth16.sol"),
    "plonk": ("PlonkVerifier", "SP1VerifierPlonk.sol"),
}


def rebind(source: str, vkey: bytes, system: str) -> tuple[str, str]:
    if system not in SYSTEMS:
        raise ValueError("proof system must be groth16 or plonk")
    verifier_contract, _ = SYSTEMS[system]
    required = (
        f'import {{{verifier_contract}}} from "./{verifier_contract}.sol";',
        f'return "{CIRCUIT_VERSION}";',
        "contract SP1Verifier",
    )
    for fragment in required:
        if source.count(fragment) != 1:
            raise ValueError(f"SP1 wrapper is missing exact fragment: {fragment}")
    pattern = re.compile(
        r"(function VERIFIER_HASH\(\) public pure returns \(bytes32\) \{\s*return )"
        r"0x[0-9a-f]{64}(;\s*\})"
    )
    if len(pattern.findall(source)) != 1:
        raise ValueError("SP1 wrapper must contain exactly one verifier hash")
    verifier_hash = "0x" + hashlib.sha256(vkey).hexdigest()
    return pattern.sub(rf"\g<1>{verifier_hash}\g<2>", source), verifier_hash


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--verifying-key", type=Path, required=True)
    parser.add_argument("--proof-system", choices=tuple(SYSTEMS), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit(f"refusing to overwrite {args.output}")
    source = args.reference.read_text(encoding="utf-8")
    output, verifier_hash = rebind(source, args.verifying_key.read_bytes(), args.proof_system)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(output.replace("\r\n", "\n"), encoding="utf-8", newline="\n")
    print(verifier_hash)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
