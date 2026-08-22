#!/usr/bin/env python3
"""Verify an owner signature and build the exact reserve-funding relay call."""

from __future__ import annotations

import argparse
import json
import re
import time
from pathlib import Path
from typing import Any

from eth_abi import encode
from eth_account import Account
from eth_account.messages import encode_typed_data
from eth_utils import keccak, to_checksum_address

from build_open_competition_v2_gmv_activation import POLICY_TYPE


SIGNATURE = re.compile(r"^0x[0-9a-fA-F]{130}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


class RelayError(ValueError):
    pass


def raw_hash(value: object, field: str) -> bytes:
    normalized = str(value or "").lower()
    if not HASH.fullmatch(normalized):
        raise RelayError(f"{field} must be bytes32")
    return bytes.fromhex(normalized[2:])


def build_relay(bundle: dict[str, Any], signature: str, now: int | None = None) -> dict[str, Any]:
    if bundle.get("schema_version") != "agent-bounties/open-competition-v2-gmv-meta-activation-v1":
        raise RelayError("activation bundle schema is invalid")
    if not SIGNATURE.fullmatch(signature):
        raise RelayError("signature must be 65-byte hex")
    typed_data = bundle.get("owner_authorization", {}).get("typed_data")
    if not isinstance(typed_data, dict):
        raise RelayError("owner typed data is missing")
    owner = str(bundle.get("owner") or "").lower()
    recovered = Account.recover_message(
        encode_typed_data(full_message=typed_data), signature=signature
    ).lower()
    if recovered != owner:
        raise RelayError(f"signature recovered {recovered}, expected {owner}")
    message = typed_data.get("message", {})
    valid_before = int(message.get("validBefore", 0))
    valid_after = int(message.get("validAfter", 0))
    current = int(time.time()) if now is None else now
    if valid_after >= valid_before or current <= valid_after or current >= valid_before:
        raise RelayError("owner authorization is not currently valid")
    if str(message.get("from") or "").lower() != owner:
        raise RelayError("owner authorization sender mismatch")
    if str(message.get("to") or "").lower() != str(bundle.get("reserve_wallet") or "").lower():
        raise RelayError("owner authorization destination mismatch")
    if int(message.get("value", 0)) != int(bundle.get("initial_funding_base_units", 0)):
        raise RelayError("owner authorization amount mismatch")

    policy = bundle.get("policy")
    if not isinstance(policy, dict):
        raise RelayError("policy is missing")
    policy_tuple = (
        to_checksum_address(policy["delegate"]),
        int(policy["valid_after"]),
        int(policy["valid_until"]),
        int(policy["period_seconds"]),
        int(policy["solver_reward"]),
        int(policy["keeper_reward"]),
        int(policy["exact_funding_per_competition"]),
        int(policy["max_per_period"]),
        int(policy["max_lifetime_spend"]),
        raw_hash(policy["beta_risk_hash"], "Beta risk hash"),
        raw_hash(policy["gmv_metric_program_hash"], "GMV metric program hash"),
        raw_hash(policy["gmv_journal_schema_hash"], "GMV journal schema hash"),
    )
    commitments = [
        raw_hash(value, "approved creation commitment")
        for value in bundle.get("approved_creation_commitments", [])
    ]
    if len(commitments) != 20 or len(set(commitments)) != 20:
        raise RelayError("exactly twenty unique approved creations are required")
    raw_signature = bytes.fromhex(signature[2:])
    r = raw_signature[:32]
    s = raw_signature[32:64]
    v = raw_signature[64]
    if v in (0, 1):
        v += 27
    if v not in (27, 28):
        raise RelayError("signature recovery id must be 0, 1, 27, or 28")
    function = (
        f"createWalletWithAuthorization(address,{POLICY_TYPE},bytes32[],bytes32,"
        "uint256,uint256,uint256,bytes32,uint8,bytes32,bytes32)"
    )
    arguments = [
        "address",
        POLICY_TYPE,
        "bytes32[]",
        "bytes32",
        "uint256",
        "uint256",
        "uint256",
        "bytes32",
        "uint8",
        "bytes32",
        "bytes32",
    ]
    values: list[object] = [
        to_checksum_address(owner),
        policy_tuple,
        commitments,
        raw_hash(bundle.get("user_salt"), "user salt"),
        int(bundle["initial_funding_base_units"]),
        valid_after,
        valid_before,
        raw_hash(message.get("nonce"), "authorization nonce"),
        v,
        r,
        s,
    ]
    data = "0x" + (keccak(text=function)[:4] + encode(arguments, values)).hex()
    return {
        "schema_version": "agent-bounties/open-competition-v2-gmv-meta-relay-v1",
        "network": "base-mainnet",
        "chain_id": 8453,
        "from": None,
        "to": bundle["reserve_factory"],
        "value_wei": 0,
        "data": data,
        "function": function,
        "expected_owner": owner,
        "expected_reserve_wallet": bundle["reserve_wallet"],
        "expected_funding_base_units": bundle["initial_funding_base_units"],
        "valid_before": valid_before,
        "evidence_boundary": "This verified signature and unsigned relay call are not funding evidence. Require the exact confirmed reserve creation and USDC transfer.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--signature", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        bundle = json.loads(args.bundle.read_text(encoding="utf-8-sig"))
        relay = build_relay(bundle, args.signature)
    except (OSError, json.JSONDecodeError, KeyError, ValueError, RelayError) as error:
        print(f"relay build blocked: {error}")
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(relay, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
