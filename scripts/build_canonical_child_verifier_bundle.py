#!/usr/bin/env python3
"""Build the unsigned, immutable Base deployment bundle for canonical-child-v1."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any

from Crypto.Hash import keccak


CHAIN_ID = 8453
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
PROTOCOL_TAG = "0x437e2138276203e007f58857babacb739e6612192c7d9ce8f41e610236edf382"
ACCEPTANCE_CRITERIA_HASH = (
    "0x005f591a8549549698e7c028b78ddc84076e0996ef07e19dd543ebdb12cb4553"
)
SOURCE = "contracts/base-escrow/src/CanonicalChildBountyVerifier.sol:CanonicalChildBountyVerifier"


def keccak256(data: bytes) -> str:
    digest = keccak.new(digest_bits=256)
    digest.update(data)
    return f"0x{digest.hexdigest()}"


def address_bytes(value: str) -> bytes:
    raw = value.removeprefix("0x")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", raw):
        raise ValueError(f"invalid EVM address: {value}")
    return bytes.fromhex(raw)


def address_word(value: str) -> bytes:
    return address_bytes(value).rjust(32, b"\0")


def rlp_bytes(value: bytes) -> bytes:
    if len(value) == 1 and value[0] < 0x80:
        return value
    if len(value) <= 55:
        return bytes([0x80 + len(value)]) + value
    length = len(value).to_bytes((len(value).bit_length() + 7) // 8, "big")
    return bytes([0xB7 + len(length)]) + length + value


def rlp_list(values: list[bytes]) -> bytes:
    payload = b"".join(values)
    if len(payload) <= 55:
        return bytes([0xC0 + len(payload)]) + payload
    length = len(payload).to_bytes((len(payload).bit_length() + 7) // 8, "big")
    return bytes([0xF7 + len(length)]) + length + payload


def create_address(deployer: str, nonce: int) -> str:
    if nonce < 0:
        raise ValueError("deployer nonce must be nonnegative")
    encoded_nonce = b"" if nonce == 0 else nonce.to_bytes((nonce.bit_length() + 7) // 8, "big")
    payload = rlp_list([rlp_bytes(address_bytes(deployer)), rlp_bytes(encoded_nonce)])
    return f"0x{keccak256(payload)[-40:]}"


def artifact_hex(field: Any, name: str) -> bytes:
    value = field.get("object") if isinstance(field, dict) else None
    if not isinstance(value, str):
        raise ValueError(f"artifact {name} is missing concrete bytecode")
    value = value.removeprefix("0x")
    if not value or not re.fullmatch(r"[0-9a-fA-F]+", value):
        raise ValueError(f"artifact {name} is missing concrete bytecode")
    if len(value) % 2:
        raise ValueError(f"artifact {name} has odd-length bytecode")
    return bytes.fromhex(value)


def patched_runtime(artifact: dict[str, Any], factory: str, token: str) -> bytes:
    deployed = artifact.get("deployedBytecode")
    runtime = bytearray(artifact_hex(deployed, "deployedBytecode"))
    references = deployed.get("immutableReferences") if isinstance(deployed, dict) else None
    if not isinstance(references, dict) or len(references) != 2:
        raise ValueError("expected exactly canonicalFactory and settlementToken immutable references")

    # Solidity AST ids increase in source declaration order: factory, then token.
    values = [address_word(factory), address_word(token)]
    for value, (_, locations) in zip(values, sorted(references.items(), key=lambda item: int(item[0]))):
        if not isinstance(locations, list) or not locations:
            raise ValueError("immutable reference group is empty")
        for location in locations:
            start = int(location["start"])
            length = int(location["length"])
            if length != 32 or start < 0 or start + length > len(runtime):
                raise ValueError("invalid immutable reference")
            runtime[start : start + length] = value
    return bytes(runtime)


def build_bundle(args: argparse.Namespace) -> dict[str, Any]:
    artifact = json.loads(args.artifact.read_text(encoding="utf-8"))
    creation_code = artifact_hex(artifact.get("bytecode"), "bytecode")
    runtime = patched_runtime(artifact, FACTORY, USDC)
    constructor_data = creation_code + address_word(FACTORY)
    expected_contract = create_address(args.deployer, args.deployer_nonce)

    if not re.fullmatch(r"[0-9a-f]{40}", args.source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", args.preflight_block_hash):
        raise ValueError("preflight block hash must be bytes32 hex")

    return {
        "schema_version": "agent-bounties/canonical-child-verifier-deployment-v1",
        "protocol_version": "agent-bounties/canonical-child-v1",
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "source": SOURCE,
        "source_commit": args.source_commit,
        "canonical_factory": FACTORY,
        "settlement_token": USDC,
        "acceptance_criteria_hash": ACCEPTANCE_CRITERIA_HASH,
        "protocol_tag": PROTOCOL_TAG,
        "preflight_block": {
            "number": args.preflight_block_number,
            "hash": args.preflight_block_hash.lower(),
        },
        "deployment": {
            "from": args.deployer.lower(),
            "deployer_nonce": args.deployer_nonce,
            "to": None,
            "value_wei": 0,
            "expected_contract": expected_contract.lower(),
            "data": f"0x{constructor_data.hex()}",
            "creation_code_hash": keccak256(constructor_data),
            "expected_runtime_code": f"0x{runtime.hex()}",
            "runtime_code_hash": keccak256(runtime),
            "runtime_code_bytes": len(runtime),
        },
        "evidence_boundary": (
            "This unsigned bundle fixes one contract-creation transaction. A successful receipt and "
            "matching runtime/getters prove deployment only; they do not prove bounty funding, "
            "completion, or payout."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--artifact",
        type=Path,
        default=Path(
            "contracts/base-escrow/out/CanonicalChildBountyVerifier.sol/CanonicalChildBountyVerifier.json"
        ),
    )
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--deployer-nonce", type=int, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--preflight-block-number", type=int, required=True)
    parser.add_argument("--preflight-block-hash", required=True)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    address_bytes(args.deployer)
    bundle = build_bundle(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "output": str(args.output),
                "expected_contract": bundle["deployment"]["expected_contract"],
                "runtime_code_hash": bundle["deployment"]["runtime_code_hash"],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
