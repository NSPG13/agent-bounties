#!/usr/bin/env python3
"""Return the exact Base Sepolia canary deficit from the broker to the deployer."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from eth_account import Account

from _shared.evm import address_word, uint_word
from _shared.rpc import rpc
from fund_open_competition_v2_beta3_broker import SignedRpc, usdc_balance


CHAIN_ID = 84_532
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
BROKER = "0x176f486a724720c4fdfc920d7c17dd1004c2bfb4"
DEPLOYER = "0xfd7be4c69541ab297aece2a674fc1418b898cc0a"
DEPLOYER_TARGET_USDC = 900_000
MAX_TRANSFER_USDC = 22_500
MINIMUM_BROKER_USDC_AFTER = 100_000
MINIMUM_BROKER_ETH_AFTER = 100_000_000_000_000
TRANSFER_SELECTOR = "a9059cbb"


class SepoliaRebalanceError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise SepoliaRebalanceError(message)


def planned_transfer(*, deployer_usdc: int, broker_usdc: int) -> int:
    require(deployer_usdc >= 0 and broker_usdc >= 0, "balances cannot be negative")
    deficit = max(DEPLOYER_TARGET_USDC - deployer_usdc, 0)
    require(deficit <= MAX_TRANSFER_USDC, "Sepolia deployer deficit exceeds the reviewed cap")
    require(
        broker_usdc >= deficit + MINIMUM_BROKER_USDC_AFTER,
        "broker cannot cover the deficit and retain its reviewed test reserve",
    )
    return deficit


def rebalance(*, rpc_url: str, private_key: str) -> dict[str, Any]:
    require(rpc_url.startswith("https://"), "Base Sepolia RPC must use HTTPS")
    signer = Account.from_key(private_key)
    require(signer.address.lower() == BROKER, "protected signer is not the reviewed broker")
    client = SignedRpc(rpc_url, signer, CHAIN_ID)
    safe_before = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False])
    require(safe_before and safe_before.get("hash"), "RPC did not return a safe block")

    deployer_before = usdc_balance(rpc_url, USDC, DEPLOYER, safe_before["number"])
    broker_before = usdc_balance(rpc_url, USDC, BROKER, safe_before["number"])
    broker_eth_before = int(rpc(rpc_url, "eth_getBalance", [BROKER, safe_before["number"]]), 16)
    transfer = planned_transfer(deployer_usdc=deployer_before, broker_usdc=broker_before)
    require(
        broker_eth_before >= MINIMUM_BROKER_ETH_AFTER,
        "broker lacks the reviewed post-transfer relay reserve",
    )

    receipt = None
    safe_after = safe_before
    if transfer:
        calldata = "0x" + TRANSFER_SELECTOR + address_word(DEPLOYER).hex() + uint_word(transfer).hex()
        receipt = client.send(to=USDC, data=calldata)
        safe_after = client.wait_safe(int(receipt["blockNumber"], 16))

    deployer_after = usdc_balance(rpc_url, USDC, DEPLOYER, safe_after["number"])
    broker_after = usdc_balance(rpc_url, USDC, BROKER, safe_after["number"])
    broker_eth_after = int(rpc(rpc_url, "eth_getBalance", [BROKER, safe_after["number"]]), 16)
    require(deployer_after >= DEPLOYER_TARGET_USDC, "deployer test reserve did not reconcile")
    require(broker_after >= MINIMUM_BROKER_USDC_AFTER, "broker test reserve fell below its floor")
    require(broker_eth_after >= MINIMUM_BROKER_ETH_AFTER, "broker relay reserve fell below its floor")

    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-sepolia-rebalance-v1",
        "passed": True,
        "network": "base-sepolia",
        "chain_id": CHAIN_ID,
        "usdc": USDC,
        "broker": BROKER,
        "deployer": DEPLOYER,
        "deployer_usdc_before": deployer_before,
        "deployer_usdc_after": deployer_after,
        "broker_usdc_before": broker_before,
        "broker_usdc_after": broker_after,
        "broker_eth_before": broker_eth_before,
        "broker_eth_after": broker_eth_after,
        "transferred_usdc_base_units": transfer,
        "transaction": receipt["transactionHash"].lower() if receipt else None,
        "safe_block_number": int(safe_after["number"], 16),
        "safe_block_hash": safe_after["hash"].lower(),
        "evidence_boundary": (
            "This is a canonical transfer of valueless Base Sepolia test USDC between the dedicated "
            "Beta3 broker and deployer. It is not mainnet funding, GMV, a bounty settlement, or payment evidence."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--private-key-env", default="OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    private_key = os.environ.get(args.private_key_env, "")
    require(bool(private_key), f"{args.private_key_env} is required")
    result = rebalance(rpc_url=args.rpc_url, private_key=private_key)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output), "transaction": result["transaction"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
