#!/usr/bin/env python3
"""Idempotently seed the isolated Beta2 broker from the protected deployer."""

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


CHAIN_ID = 8453
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
TRANSFER_SELECTOR = "a9059cbb"
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
MIN_DEPLOYER_USDC_AFTER = 635_000
MIN_DEPLOYER_ETH_AFTER = 100_000_000_000_000


class BrokerFundingError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise BrokerFundingError(message)


def deficits(
    *, current_usdc: int, current_eth: int, target_usdc: int, target_eth: int
) -> tuple[int, int]:
    require(min(current_usdc, current_eth, target_usdc, target_eth) >= 0, "balances cannot be negative")
    require(target_usdc > 0 and target_eth > 0, "broker reserve targets must be positive")
    return max(target_usdc - current_usdc, 0), max(target_eth - current_eth, 0)


def usdc_balance(url: str, address: str, block: str) -> int:
    data = "0x70a08231" + address_word(address).hex()
    return int(rpc(url, "eth_call", [{"to": USDC, "data": data}, block]), 16)


class SignedRpc:
    def __init__(self, url: str, signer: Any) -> None:
        self.url = url
        self.signer = signer
        require(int(rpc(url, "eth_chainId", []), 16) == CHAIN_ID, "RPC is not Base mainnet")

    def send(self, *, to: str, data: str = "0x", value: int = 0) -> dict[str, Any]:
        nonce = int(rpc(self.url, "eth_getTransactionCount", [self.signer.address, "pending"]), 16)
        block = rpc(self.url, "eth_getBlockByNumber", ["latest", False])
        base_fee = int(block.get("baseFeePerGas", "0x0"), 16)
        try:
            priority = int(rpc(self.url, "eth_maxPriorityFeePerGas", []), 16)
        except RuntimeError:
            priority = 1_000_000
        priority = max(priority, 1_000_000)
        maximum = base_fee * 2 + priority
        estimate = {
            "from": self.signer.address,
            "to": to,
            "value": hex(value),
            "data": data,
            "maxFeePerGas": hex(maximum),
            "maxPriorityFeePerGas": hex(priority),
        }
        gas = int(rpc(self.url, "eth_estimateGas", [estimate]), 16)
        require(gas <= 150_000, "broker reserve transfer gas exceeds cap")
        signed = self.signer.sign_transaction(
            {
                "chainId": CHAIN_ID,
                "from": self.signer.address,
                "to": to,
                "nonce": nonce,
                "value": value,
                "data": data,
                "gas": gas * 5 // 4 + 10_000,
                "maxFeePerGas": maximum,
                "maxPriorityFeePerGas": priority,
                "type": 2,
            }
        )
        tx_hash = rpc(
            self.url,
            "eth_sendRawTransaction",
            ["0x" + bytes(signed.raw_transaction).hex()],
        )
        deadline = time.time() + 300
        while time.time() < deadline:
            receipt = rpc(self.url, "eth_getTransactionReceipt", [tx_hash])
            if receipt:
                require(int(receipt["status"], 16) == 1, f"reserve transfer reverted: {tx_hash}")
                return receipt
            time.sleep(2)
        raise BrokerFundingError(f"reserve transfer timed out: {tx_hash}")

    def wait_safe(self, block_number: int) -> dict[str, Any]:
        deadline = time.time() + 1_800
        while time.time() < deadline:
            block = rpc(self.url, "eth_getBlockByNumber", ["safe", False])
            if block and int(block["number"], 16) >= block_number:
                return block
            time.sleep(5)
        raise BrokerFundingError("broker reserve did not reach a Base safe block")


def fund(
    *,
    rpc_url: str,
    private_key: str,
    broker: str,
    target_usdc: int,
    target_eth: int,
) -> dict[str, Any]:
    require(rpc_url.startswith("https://"), "production RPC must use HTTPS")
    require(ADDRESS.fullmatch(broker) is not None, "broker address is invalid")
    signer = Account.from_key(private_key)
    require(signer.address.lower() != broker.lower(), "broker and deployer must be distinct")
    client = SignedRpc(rpc_url, signer)
    safe_before = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False])
    require(safe_before and safe_before.get("hash"), "RPC did not return a safe block")
    current_usdc = usdc_balance(rpc_url, broker, safe_before["number"])
    current_eth = int(rpc(rpc_url, "eth_getBalance", [broker, safe_before["number"]]), 16)
    usdc_deficit, eth_deficit = deficits(
        current_usdc=current_usdc,
        current_eth=current_eth,
        target_usdc=target_usdc,
        target_eth=target_eth,
    )
    deployer_usdc = usdc_balance(rpc_url, signer.address, "latest")
    deployer_eth = int(rpc(rpc_url, "eth_getBalance", [signer.address, "latest"]), 16)
    require(
        deployer_usdc >= usdc_deficit + MIN_DEPLOYER_USDC_AFTER,
        "deployer USDC cannot seed broker and retain the canary budget",
    )
    require(
        deployer_eth >= eth_deficit + MIN_DEPLOYER_ETH_AFTER,
        "deployer ETH cannot seed broker and retain deployment gas",
    )

    receipts: list[dict[str, Any]] = []
    if usdc_deficit:
        calldata = "0x" + TRANSFER_SELECTOR + address_word(broker).hex() + uint_word(usdc_deficit).hex()
        receipts.append(client.send(to=USDC, data=calldata))
    if eth_deficit:
        receipts.append(client.send(to=broker, value=eth_deficit))
    last_block = max(
        [int(receipt["blockNumber"], 16) for receipt in receipts]
        or [int(safe_before["number"], 16)]
    )
    safe_after = client.wait_safe(last_block)
    final_usdc = usdc_balance(rpc_url, broker, safe_after["number"])
    final_eth = int(rpc(rpc_url, "eth_getBalance", [broker, safe_after["number"]]), 16)
    require(final_usdc >= target_usdc, "broker USDC reserve did not reconcile")
    require(final_eth >= target_eth, "broker ETH reserve did not reconcile")
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta2-broker-seed-v1",
        "passed": True,
        "network": "base-mainnet",
        "deployer": signer.address.lower(),
        "broker": broker.lower(),
        "target_usdc_base_units": target_usdc,
        "target_eth_wei": target_eth,
        "funded_usdc_base_units": usdc_deficit,
        "funded_eth_wei": eth_deficit,
        "transactions": [receipt["transactionHash"].lower() for receipt in receipts],
        "safe_block_number": int(safe_after["number"], 16),
        "safe_block_hash": safe_after["hash"].lower(),
        "final_usdc_base_units": final_usdc,
        "final_eth_wei": final_eth,
        "evidence_boundary": "These canonical transfers seed the dedicated broker reserve. They are not proof-job refunds, solver payments, or competition settlements.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--private-key-env", default="BASE_MAINNET_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--broker", required=True)
    parser.add_argument("--target-usdc-base-units", type=int, default=110_000)
    parser.add_argument("--target-eth-wei", type=int, default=100_000_000_000_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    private_key = os.environ.get(args.private_key_env, "")
    require(bool(private_key), f"{args.private_key_env} is required")
    result = fund(
        rpc_url=args.rpc_url,
        private_key=private_key,
        broker=args.broker,
        target_usdc=args.target_usdc_base_units,
        target_eth=args.target_eth_wei,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "broker": result["broker"], "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
