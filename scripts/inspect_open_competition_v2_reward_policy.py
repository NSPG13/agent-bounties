#!/usr/bin/env python3
"""Read the exact Open Competition V2 reserve state at a Base safe block."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
from typing import Any

from eth_abi import decode, encode
from eth_utils import keccak, to_checksum_address
from web3 import Web3

from build_open_competition_v2_reward_policy import CHAIN_ID, STATE_SCHEMA


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_RESERVE = "0x7b0ae568f76d11aa4025e2aa05865a566bbcfc8d"
POLICY_OUTPUTS = (
    "address",
    "uint64",
    "uint64",
    "uint64",
    "uint256",
    "uint256",
    "uint256",
    "uint256",
    "uint256",
    "bytes32",
    "bytes32",
    "bytes32",
)


class InspectError(ValueError):
    pass


def selector(signature: str) -> bytes:
    return keccak(text=signature)[:4]


def lower_address(value: str) -> str:
    return str(value).lower()


def call_raw(
    w3: Web3, to: str, signature: str, block: int, arguments: bytes = b""
) -> bytes:
    return bytes(
        w3.eth.call(
            {"to": to_checksum_address(to), "data": selector(signature) + arguments},
            block_identifier=block,
        )
    )


def call_one(w3: Web3, to: str, signature: str, output: str, block: int) -> Any:
    return decode([output], call_raw(w3, to, signature, block))[0]


def inspect_state(w3: Web3, reserve_wallet: str) -> dict[str, Any]:
    if not w3.is_connected():
        raise InspectError("Base RPC is unavailable")
    if int(w3.eth.chain_id) != CHAIN_ID:
        raise InspectError("RPC is not Base mainnet")
    safe_response = w3.provider.make_request("eth_getBlockByNumber", ["safe", False])
    safe_value = (
        safe_response.get("result") if isinstance(safe_response, dict) else None
    )
    if (
        not isinstance(safe_value, dict)
        or not safe_value.get("number")
        or not safe_value.get("hash")
    ):
        raise InspectError("RPC did not return an exact safe block")
    safe_block = int(str(safe_value["number"]), 16)
    reserve = lower_address(to_checksum_address(reserve_wallet))
    reserve_code = bytes(
        w3.eth.get_code(to_checksum_address(reserve), block_identifier=safe_block)
    )
    if not reserve_code:
        raise InspectError("reserve has no code at the safe block")

    owner = lower_address(call_one(w3, reserve, "owner()", "address", safe_block))
    settlement_token = lower_address(
        call_one(w3, reserve, "settlementToken()", "address", safe_block)
    )
    competition_factory = lower_address(
        call_one(w3, reserve, "competitionFactory()", "address", safe_block)
    )
    deployment_factory = lower_address(
        call_one(w3, reserve, "deploymentFactory()", "address", safe_block)
    )
    implementation = lower_address(
        call_one(w3, competition_factory, "implementation()", "address", safe_block)
    )
    factory_code = bytes(
        w3.eth.get_code(
            to_checksum_address(competition_factory), block_identifier=safe_block
        )
    )
    implementation_code = bytes(
        w3.eth.get_code(
            to_checksum_address(implementation), block_identifier=safe_block
        )
    )
    token_code = bytes(
        w3.eth.get_code(
            to_checksum_address(settlement_token), block_identifier=safe_block
        )
    )
    if not factory_code or not implementation_code or not token_code:
        raise InspectError("a reserve dependency has no code at the safe block")

    policy_values = decode(
        POLICY_OUTPUTS, call_raw(w3, reserve, "policy()", safe_block)
    )
    policy = {
        "delegate": lower_address(policy_values[0]),
        "valid_after": policy_values[1],
        "valid_until": policy_values[2],
        "period_seconds": policy_values[3],
        "solver_reward": policy_values[4],
        "keeper_reward": policy_values[5],
        "exact_funding_per_competition": policy_values[6],
        "max_per_period": policy_values[7],
        "max_lifetime_spend": policy_values[8],
        "beta_risk_hash": "0x" + policy_values[9].hex(),
        "gmv_metric_program_hash": "0x" + policy_values[10].hex(),
        "gmv_journal_schema_hash": "0x" + policy_values[11].hex(),
    }
    balance_arguments = encode(["address"], [to_checksum_address(reserve)])
    reserve_balance = decode(
        ["uint256"],
        call_raw(
            w3, settlement_token, "balanceOf(address)", safe_block, balance_arguments
        ),
    )[0]
    active_policy_hash = call_one(
        w3, reserve, "activePolicyHash()", "bytes32", safe_block
    )
    generated_at = (
        datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")
    )
    return {
        "schema_version": STATE_SCHEMA,
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "block_tag": "safe",
        "safe_block": safe_block,
        "safe_block_hash": str(safe_value["hash"]).lower(),
        "generated_at": generated_at,
        "reserve_wallet": reserve,
        "reserve_runtime_code_hash": "0x" + keccak(reserve_code).hex(),
        "owner": owner,
        "settlement_token": settlement_token,
        "settlement_token_runtime_code_hash": "0x" + keccak(token_code).hex(),
        "deployment_factory": deployment_factory,
        "competition_factory": competition_factory,
        "competition_factory_runtime_code_hash": "0x" + keccak(factory_code).hex(),
        "competition_implementation": implementation,
        "competition_implementation_runtime_code_hash": "0x"
        + keccak(implementation_code).hex(),
        "policy_version": call_one(
            w3, reserve, "policyVersion()", "uint64", safe_block
        ),
        "active_policy_hash": "0x" + active_policy_hash.hex(),
        "period_bucket": call_one(w3, reserve, "periodBucket()", "uint256", safe_block),
        "period_spent": call_one(w3, reserve, "periodSpent()", "uint256", safe_block),
        "lifetime_spent": call_one(
            w3, reserve, "lifetimeSpent()", "uint256", safe_block
        ),
        "reserve_balance": reserve_balance,
        "revoked": call_one(w3, reserve, "revoked()", "bool", safe_block),
        "policy": policy,
        "evidence_boundary": "This read-only safe-block snapshot is not a policy change, competition activation, GMV event, entry, payout, or settlement.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default="https://mainnet.base.org")
    parser.add_argument("--reserve-wallet", default=DEFAULT_RESERVE)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        value = inspect_state(
            Web3(Web3.HTTPProvider(args.rpc_url)), args.reserve_wallet
        )
    except (InspectError, OSError, TypeError, ValueError) as error:
        print(f"reserve inspection blocked: {error}")
        return 2
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
    print(args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
