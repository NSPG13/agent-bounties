#!/usr/bin/env python3
"""Verify the isolated Beta2 broker has bounded Base gas and refund reserves."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any

from _shared.evm import address_word
from _shared.rpc import rpc


PROTOCOL_VERSION = "agent-bounties/open-competition-v2-beta2"
BASE_MAINNET_CHAIN_ID = "0x2105"
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")


class BrokerReserveError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BrokerReserveError(message)


def inspect_reserve(
    runtime: dict[str, Any],
    *,
    rpc_url: str,
    broker: str,
    keeper: str,
    deployer: str,
    minimum_usdc_base_units: int,
    minimum_eth_wei: int,
) -> dict[str, Any]:
    require(runtime.get("protocol_version") == PROTOCOL_VERSION, "runtime protocol mismatch")
    require(runtime.get("network") == "base-mainnet", "reserve gate accepts only Base mainnet")
    require(rpc_url.startswith("https://"), "production RPC must use HTTPS")
    roles = {"broker": broker.lower(), "keeper": keeper.lower(), "deployer": deployer.lower()}
    require(all(ADDRESS.fullmatch(value) for value in roles.values()), "release role address is invalid")
    require(len(set(roles.values())) == len(roles), "broker, keeper and deployer must be distinct")
    require(minimum_usdc_base_units > 0, "minimum USDC reserve must be positive")
    require(minimum_eth_wei > 0, "minimum ETH reserve must be positive")
    require(rpc(rpc_url, "eth_chainId", []) == BASE_MAINNET_CHAIN_ID, "RPC is not Base mainnet")

    block = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False])
    require(isinstance(block, dict) and block.get("hash"), "RPC did not return a safe block")
    block_tag = block["number"]
    token = str(runtime.get("settlement_token", "")).lower()
    require(ADDRESS.fullmatch(token) is not None, "runtime settlement token is invalid")
    require(rpc(rpc_url, "eth_getCode", [token, block_tag]) != "0x", "settlement token has no code")

    balance_data = "0x70a08231" + address_word(roles["broker"]).hex()
    usdc_balance = int(
        rpc(rpc_url, "eth_call", [{"to": token, "data": balance_data}, block_tag]), 16
    )
    eth_balance = int(rpc(rpc_url, "eth_getBalance", [roles["broker"], block_tag]), 16)
    require(usdc_balance >= minimum_usdc_base_units, "broker USDC refund reserve is below minimum")
    require(eth_balance >= minimum_eth_wei, "broker Base ETH gas reserve is below minimum")

    return {
        "schema_version": "agent-bounties/open-competition-v2-beta2-broker-reserve-v1",
        "passed": True,
        "network": "base-mainnet",
        "broker": roles["broker"],
        "keeper": roles["keeper"],
        "deployer": roles["deployer"],
        "settlement_token": token,
        "safe_block_number": int(block_tag, 16),
        "safe_block_hash": str(block["hash"]).lower(),
        "usdc_balance_base_units": usdc_balance,
        "minimum_usdc_base_units": minimum_usdc_base_units,
        "eth_balance_wei": eth_balance,
        "minimum_eth_wei": minimum_eth_wei,
        "roles_are_isolated": True,
        "evidence_boundary": "This safe-block observation proves bounded broker reserves and role separation at one canonical block. It is not proof generation, refund, relay, competition settlement, or future solvency evidence.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--broker", required=True)
    parser.add_argument("--keeper", required=True)
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--minimum-usdc-base-units", type=int, default=110_000)
    parser.add_argument("--minimum-eth-wei", type=int, default=20_000_000_000_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = inspect_reserve(
        json.loads(args.runtime.read_text(encoding="utf-8")),
        rpc_url=args.rpc_url,
        broker=args.broker,
        keeper=args.keeper,
        deployer=args.deployer,
        minimum_usdc_base_units=args.minimum_usdc_base_units,
        minimum_eth_wei=args.minimum_eth_wei,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "broker": result["broker"], "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
