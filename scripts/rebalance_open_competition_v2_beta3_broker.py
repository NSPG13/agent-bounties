#!/usr/bin/env python3
"""Safely return excess Beta3 broker USDC to the protected deployer."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import time
from typing import Any

from eth_account import Account

from _shared.evm import address_word, uint_word
from _shared.rpc import rpc
from fund_open_competition_v2_beta3_broker import SignedRpc, usdc_balance


CHAIN_ID = 8453
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
TRANSFER_SELECTOR = "a9059cbb"
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
MAXIMUM_GAS_COST_WEI = 20_000_000_000_000


class BrokerRebalanceError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BrokerRebalanceError(message)


def rebalance_amount(
    *,
    deployer_usdc: int,
    broker_usdc: int,
    minimum_deployer_usdc: int,
    minimum_broker_usdc: int,
    maximum_transfer: int,
) -> int:
    values = (
        deployer_usdc,
        broker_usdc,
        minimum_deployer_usdc,
        minimum_broker_usdc,
        maximum_transfer,
    )
    require(min(values) >= 0, "rebalance values cannot be negative")
    require(minimum_deployer_usdc > 0, "deployer minimum must be positive")
    require(minimum_broker_usdc > 0, "broker minimum must be positive")
    require(maximum_transfer > 0, "transfer cap must be positive")
    shortfall = max(minimum_deployer_usdc - deployer_usdc, 0)
    if shortfall == 0:
        return 0
    surplus = max(broker_usdc - minimum_broker_usdc, 0)
    require(shortfall <= surplus, "broker surplus cannot cover deployer shortfall")
    require(shortfall <= maximum_transfer, "deployer shortfall exceeds transfer cap")
    return shortfall


def common_safe(primary_url: str, shadow_url: str, minimum_block: int = 0) -> dict[str, Any]:
    deadline = time.time() + 1_800
    while time.time() < deadline:
        primary = rpc(primary_url, "eth_getBlockByNumber", ["safe", False])
        shadow = rpc(shadow_url, "eth_getBlockByNumber", ["safe", False])
        if primary and shadow:
            number = min(int(primary["number"], 16), int(shadow["number"], 16))
            if number >= minimum_block:
                primary_common = rpc(primary_url, "eth_getBlockByNumber", [hex(number), False])
                shadow_common = rpc(shadow_url, "eth_getBlockByNumber", [hex(number), False])
                require(primary_common and shadow_common, "RPC omitted the common safe block")
                require(
                    primary_common["hash"].lower() == shadow_common["hash"].lower(),
                    "primary and shadow RPC safe-block hashes disagree",
                )
                return {
                    "number": number,
                    "hash": primary_common["hash"].lower(),
                }
        time.sleep(5)
    raise BrokerRebalanceError("rebalance transaction did not reach a common Base safe block")


def agreed_usdc_balance(
    primary_url: str, shadow_url: str, address: str, block_number: int
) -> int:
    block = hex(block_number)
    primary = usdc_balance(primary_url, USDC, address, block)
    shadow = usdc_balance(shadow_url, USDC, address, block)
    require(primary == shadow, f"primary and shadow RPC balances disagree for {address}")
    return primary


def rebalance(
    *,
    primary_url: str,
    shadow_url: str,
    private_key: str,
    broker: str,
    deployer: str,
    minimum_deployer_usdc: int,
    minimum_broker_usdc: int,
    maximum_transfer: int,
    minimum_broker_eth_after: int,
) -> dict[str, Any]:
    require(primary_url.startswith("https://"), "primary RPC must use HTTPS")
    require(shadow_url.startswith("https://"), "shadow RPC must use HTTPS")
    require(primary_url != shadow_url, "primary and shadow RPCs must be distinct")
    require(ADDRESS.fullmatch(broker) is not None, "broker address is invalid")
    require(ADDRESS.fullmatch(deployer) is not None, "deployer address is invalid")
    require(broker.lower() != deployer.lower(), "broker and deployer must be distinct")
    require(minimum_broker_eth_after > 0, "broker ETH floor must be positive")
    require(int(rpc(primary_url, "eth_chainId", []), 16) == CHAIN_ID, "primary chain ID mismatch")
    require(int(rpc(shadow_url, "eth_chainId", []), 16) == CHAIN_ID, "shadow chain ID mismatch")

    signer = Account.from_key(private_key)
    require(signer.address.lower() == broker.lower(), "broker key does not match broker address")
    safe_before = common_safe(primary_url, shadow_url)
    deployer_before = agreed_usdc_balance(
        primary_url, shadow_url, deployer, safe_before["number"]
    )
    broker_before = agreed_usdc_balance(
        primary_url, shadow_url, broker, safe_before["number"]
    )
    primary_broker_eth_before = int(
        rpc(primary_url, "eth_getBalance", [broker, hex(safe_before["number"])]), 16
    )
    shadow_broker_eth_before = int(
        rpc(shadow_url, "eth_getBalance", [broker, hex(safe_before["number"])]), 16
    )
    require(
        primary_broker_eth_before == shadow_broker_eth_before,
        "broker ETH balance disagrees before rebalance",
    )
    amount = rebalance_amount(
        deployer_usdc=deployer_before,
        broker_usdc=broker_before,
        minimum_deployer_usdc=minimum_deployer_usdc,
        minimum_broker_usdc=minimum_broker_usdc,
        maximum_transfer=maximum_transfer,
    )
    required_eth_before = minimum_broker_eth_after + (
        MAXIMUM_GAS_COST_WEI if amount else 0
    )
    require(
        primary_broker_eth_before >= required_eth_before,
        "broker cannot preserve its ETH floor within the gas-cost cap",
    )

    transaction_hash = None
    transaction_block = safe_before["number"]
    if amount:
        client = SignedRpc(
            primary_url,
            signer,
            CHAIN_ID,
            broadcast_urls=[shadow_url],
        )
        receipt = client.send(
            to=USDC,
            data="0x"
            + TRANSFER_SELECTOR
            + address_word(deployer).hex()
            + uint_word(amount).hex(),
        )
        transaction_hash = receipt["transactionHash"].lower()
        transaction_block = int(receipt["blockNumber"], 16)

    safe_after = common_safe(primary_url, shadow_url, transaction_block)
    deployer_after = agreed_usdc_balance(
        primary_url, shadow_url, deployer, safe_after["number"]
    )
    broker_after = agreed_usdc_balance(
        primary_url, shadow_url, broker, safe_after["number"]
    )
    primary_broker_eth = int(
        rpc(primary_url, "eth_getBalance", [broker, hex(safe_after["number"])]), 16
    )
    shadow_broker_eth = int(
        rpc(shadow_url, "eth_getBalance", [broker, hex(safe_after["number"])]), 16
    )
    require(primary_broker_eth == shadow_broker_eth, "broker ETH balance disagrees")
    require(deployer_after == deployer_before + amount, "deployer delta does not reconcile")
    require(broker_after == broker_before - amount, "broker delta does not reconcile")
    require(deployer_after >= minimum_deployer_usdc, "deployer minimum was not restored")
    require(broker_after >= minimum_broker_usdc, "broker reserve was not preserved")
    require(primary_broker_eth >= minimum_broker_eth_after, "broker ETH floor was not preserved")

    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-broker-rebalance-v1",
        "passed": True,
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "asset": USDC,
        "broker": broker.lower(),
        "deployer": deployer.lower(),
        "minimum_deployer_usdc_base_units": minimum_deployer_usdc,
        "minimum_broker_usdc_base_units": minimum_broker_usdc,
        "maximum_transfer_base_units": maximum_transfer,
        "transferred_base_units": amount,
        "transaction_hash": transaction_hash,
        "safe_before": safe_before,
        "safe_after": safe_after,
        "deployer_usdc_before": deployer_before,
        "deployer_usdc_after": deployer_after,
        "broker_usdc_before": broker_before,
        "broker_usdc_after": broker_after,
        "maximum_gas_cost_wei": MAXIMUM_GAS_COST_WEI,
        "broker_eth_before_wei": primary_broker_eth_before,
        "broker_eth_after_wei": primary_broker_eth,
        "evidence_boundary": (
            "Canonical internal treasury rebalance only. This is not bounty funding, "
            "solver payment, settlement, GMV, or owner-wallet authorization evidence."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url", required=True)
    parser.add_argument("--broker", required=True)
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--private-key-env", default="OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY")
    parser.add_argument("--minimum-deployer-usdc-base-units", type=int, default=635_000)
    parser.add_argument("--minimum-broker-usdc-base-units", type=int, default=110_000)
    parser.add_argument("--maximum-transfer-base-units", type=int, required=True)
    parser.add_argument("--minimum-broker-eth-after-wei", type=int, default=50_000_000_000_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    private_key = os.environ.get(args.private_key_env, "")
    require(bool(private_key), f"{args.private_key_env} is required")
    result = rebalance(
        primary_url=args.rpc_url,
        shadow_url=args.shadow_rpc_url,
        private_key=private_key,
        broker=args.broker,
        deployer=args.deployer,
        minimum_deployer_usdc=args.minimum_deployer_usdc_base_units,
        minimum_broker_usdc=args.minimum_broker_usdc_base_units,
        maximum_transfer=args.maximum_transfer_base_units,
        minimum_broker_eth_after=args.minimum_broker_eth_after_wei,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
