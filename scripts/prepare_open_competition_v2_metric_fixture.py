#!/usr/bin/env python3
"""Bind a public-vector metric template to one exact V2 competition entry."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from _shared.evm import address_bytes, keccak256, keccak_bytes


POLICY_DOMAIN = bytes.fromhex("f6a226ca20aaca3b9c0b4a609939c334b6c2b03500a5df45188df8bcd7c2b369")
PROOF_SYSTEMS = {
    "groth16": keccak256(b"sp1-groth16"),
    "plonk": keccak256(b"sp1-plonk"),
}
MODE_TAGS = {
    "all_equal": 0,
    "maximize_exact_matches": 1,
    "minimize_absolute_error": 2,
}


def bytes32(value: str, name: str) -> bytes:
    try:
        raw = bytes.fromhex(value.removeprefix("0x"))
    except (AttributeError, ValueError) as error:
        raise ValueError(f"{name} must be bytes32 hex") from error
    if len(raw) != 32:
        raise ValueError(f"{name} must be bytes32 hex")
    return raw


def signed_bytes(value: int, length: int, name: str) -> bytes:
    minimum = -(1 << (length * 8 - 1))
    maximum = (1 << (length * 8 - 1)) - 1
    if value < minimum or value > maximum:
        raise ValueError(f"{name} is outside int{length * 8}")
    return value.to_bytes(length, "big", signed=True)


def verification_policy_hash(mode: str, threshold: int, vectors: list[dict[str, Any]]) -> str:
    if mode not in MODE_TAGS:
        raise ValueError("unsupported public-vector mode")
    if not vectors or len(vectors) > 10_000:
        raise ValueError("vectors must contain 1 through 10000 cases")
    payload = bytearray(POLICY_DOMAIN)
    payload.append(MODE_TAGS[mode])
    payload.extend(signed_bytes(threshold, 16, "threshold"))
    payload.extend(len(vectors).to_bytes(4, "big"))
    for index, vector in enumerate(vectors):
        expected = int(vector["expected"])
        weight = int(vector["weight"])
        if weight <= 0 or weight > 0xFFFFFFFF:
            raise ValueError(f"vectors[{index}].weight must be a positive uint32")
        signed_bytes(int(vector["observed"]), 8, f"vectors[{index}].observed")
        payload.extend(signed_bytes(expected, 8, f"vectors[{index}].expected"))
        payload.extend(weight.to_bytes(4, "big"))
    return "0x" + keccak_bytes(bytes(payload)).hex()


def bind(template: dict[str, Any], scope: dict[str, Any]) -> dict[str, Any]:
    mode = str(template["mode"])
    threshold = int(template["threshold"])
    vectors = template["vectors"]
    if not isinstance(vectors, list):
        raise ValueError("vectors must be a list")
    policy_hash = verification_policy_hash(mode, threshold, vectors)
    proof_system = str(scope["proof_system"])
    if proof_system not in PROOF_SYSTEMS:
        raise ValueError("proof_system must be groth16 or plonk")
    chain_id = int(scope["chain_id"])
    solver_nonce = int(scope["solver_nonce"])
    if chain_id <= 0 or chain_id > 0xFFFFFFFFFFFFFFFF:
        raise ValueError("chain_id must be a positive uint64")
    if solver_nonce < 0 or solver_nonce >= 1 << 128:
        raise ValueError("solver_nonce must be a uint128")
    bound_scope = {
        "chain_id": chain_id,
        "competition": list(address_bytes(scope["competition"])),
        "bounty_id": list(bytes32(scope["bounty_id"], "bounty_id")),
        "solver": list(address_bytes(scope["solver"])),
        "solver_nonce": solver_nonce,
        "proof_system": list(bytes32(PROOF_SYSTEMS[proof_system], "proof_system")),
        "program_vkey": list(bytes32(scope["program_vkey"], "program_vkey")),
        "source_hash": list(bytes32(scope["source_hash"], "source_hash")),
        "elf_hash": list(bytes32(scope["elf_hash"], "elf_hash")),
        "execution_policy_hash": list(bytes32(scope["execution_policy_hash"], "execution_policy_hash")),
        "settlement_policy_hash": list(bytes32(scope["settlement_policy_hash"], "settlement_policy_hash")),
        "beta_risk_hash": list(bytes32(scope["beta_risk_hash"], "beta_risk_hash")),
    }
    return {
        "scope": bound_scope,
        "mode": mode,
        "threshold": threshold,
        "vectors": [dict(vector) for vector in vectors],
        "expected": {"verification_policy_hash": policy_hash},
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--scope", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    template = json.loads(args.template.read_text(encoding="utf-8"))
    scope = json.loads(args.scope.read_text(encoding="utf-8"))
    result = bind(template, scope)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), **result["expected"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
