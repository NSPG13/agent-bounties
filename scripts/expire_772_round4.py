#!/usr/bin/env python3
"""Fail-closed one-shot recovery facade for Agent Bounties #772 round 4.

Dry-run is the default. Execution is possible only through the existing bounded
hosted timeout relay and requires an explicit acknowledgement string. This file
contains no signer material and cannot accept a target, chain, or calldata.
"""

from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import urllib.request
from dataclasses import asdict, dataclass
from typing import Any

CHAIN_ID = 8453
RPC_URL = "https://mainnet.base.org"
API_URL = "https://api.agentbounties.app/v1/base/autonomous-bounties/timeout-relay"
CONTRACT = "0x9baa8a4a2ad3096c6ebfb2c994a93afb7a299274"
BOUNTY_ID = "0x34e8d16cdbfff635e77ce703cc6efea8fc64a3adb1ee2ef293c604b85bb6a8cb"
SOLVER = "0xc49e5374f0072abc0b4c134b2fd413d87aa6354a"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
CLONE_CODEHASH = "0x6e7d6297e170d10e6484c9b72314bb0e2173cd967aa8e05231ee369dbde0c0a1"
SELECTOR = "0xf9251ec7"
EVENT_TOPIC = "0x2d21c86724fb1d7ecb4465174a1fdf4969530254e9351bcb18f680eaa85d75e9"
STATUS_SUBMITTED = 3
ROUND = 4
VERIFICATION_EXPIRES_AT = 1786586903
BOND = 10000
ACK = "authorize-expire-772-round4-once"


class RecoveryError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RecoveryError(message)


class Cast:
    def __init__(self, rpc_url: str = RPC_URL) -> None:
        self.rpc_url = rpc_url

    def run(self, *args: str) -> str:
        result = subprocess.run(
            ["cast", *args, "--rpc-url", self.rpc_url],
            check=False,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode != 0:
            raise RecoveryError(result.stderr.strip() or "cast command failed")
        return result.stdout.strip()

    def call(self, target: str, signature: str, *args: str, block: str) -> str:
        return self.run("call", target, signature, *args, "--block", block)

    def safe_block(self) -> dict[str, Any]:
        return json.loads(self.run("block", "safe", "--json"))

    def receipt(self, transaction_hash: str) -> dict[str, Any]:
        return json.loads(self.run("receipt", transaction_hash, "--json"))


def parse_uint(value: str) -> int:
    return int(value.split()[0], 0)


def block_number(value: object) -> int:
    require(isinstance(value, str), "safe block number missing")
    return int(value, 0)


@dataclass(frozen=True)
class Snapshot:
    block_number: int
    block_hash: str
    block_timestamp: int
    chain_id: int
    codehash: str
    bounty_id: str
    status: int
    round: int
    solver: str
    verification_expires_at: int
    active_claim_bond: int
    solver_usdc_balance: int


def snapshot(cast: Cast) -> Snapshot:
    safe = cast.safe_block()
    number = block_number(safe.get("number"))
    block = str(number)
    snap = Snapshot(
        block_number=number,
        block_hash=str(safe.get("hash", "")).lower(),
        block_timestamp=parse_uint(str(safe.get("timestamp"))),
        chain_id=parse_uint(cast.run("chain-id")),
        codehash=cast.run("codehash", CONTRACT, "--block", block).lower(),
        bounty_id=cast.call(CONTRACT, "bountyId()(bytes32)", block=block).lower(),
        status=parse_uint(cast.call(CONTRACT, "status()(uint8)", block=block)),
        round=parse_uint(cast.call(CONTRACT, "round()(uint64)", block=block)),
        solver=cast.call(CONTRACT, "solver()(address)", block=block).lower(),
        verification_expires_at=parse_uint(
            cast.call(CONTRACT, "verificationExpiresAt()(uint64)", block=block)
        ),
        active_claim_bond=parse_uint(
            cast.call(CONTRACT, "activeClaimBond()(uint256)", block=block)
        ),
        solver_usdc_balance=parse_uint(
            cast.call(USDC, "balanceOf(address)(uint256)", SOLVER, block=block)
        ),
    )
    expected = {
        "chain_id": CHAIN_ID,
        "codehash": CLONE_CODEHASH,
        "bounty_id": BOUNTY_ID,
        "status": STATUS_SUBMITTED,
        "round": ROUND,
        "solver": SOLVER,
        "verification_expires_at": VERIFICATION_EXPIRES_AT,
        "active_claim_bond": BOND,
    }
    for field, wanted in expected.items():
        observed = getattr(snap, field)
        require(observed == wanted, f"tuple mismatch for {field}: expected {wanted}, got {observed}")
    require(snap.block_timestamp > VERIFICATION_EXPIRES_AT, "verification deadline has not passed")
    require(len(snap.block_hash) == 66, "safe block hash malformed")
    return snap


def dry_run(cast: Cast) -> dict[str, Any]:
    before = snapshot(cast)
    block = str(before.block_number)
    simulation = cast.run("call", CONTRACT, SELECTOR, "--block", block)
    gas = parse_uint(cast.run("estimate", CONTRACT, SELECTOR, "--block", block))
    require(simulation in ("", "0x"), "expiry simulation returned unexpected data")
    require(0 < gas <= 250000, f"gas estimate outside recovery cap: {gas}")
    return {
        "schema": "agent-bounties/expiry-772-round4-dry-run-v1",
        "network": "base-mainnet",
        "tuple": asdict(before),
        "intent": {"to": CONTRACT, "value_wei": 0, "function": "expireSubmission()", "data": SELECTOR},
        "simulation": {"success": True, "return_data": simulation or "0x", "gas_estimate": gas, "gas_cap": 250000},
        "execution": {"performed": False, "endpoint": API_URL, "action": "expire_submission"},
    }


def execute_once(cast: Cast, acknowledgement: str) -> dict[str, Any]:
    require(acknowledgement == ACK, f"--acknowledge must be exactly {ACK}")
    before = snapshot(cast)
    body = json.dumps({
        "network": "base-mainnet", "bounty_contract": CONTRACT, "action": "expire_submission"
    }).encode()
    request = urllib.request.Request(API_URL, data=body, headers={"content-type": "application/json"}, method="POST")
    with urllib.request.urlopen(request, timeout=240) as response:
        relay = json.load(response)
    transaction_hash = str(relay.get("transaction_hash", "")).lower()
    require(len(transaction_hash) == 66 and transaction_hash.startswith("0x"), "relay transaction hash missing")
    receipt = cast.receipt(transaction_hash)
    require(parse_uint(str(receipt.get("status"))) == 1, "expiry transaction failed")
    require(str(receipt.get("to", "")).lower() == CONTRACT, "receipt target mismatch")
    logs = receipt.get("logs")
    require(isinstance(logs, list), "receipt logs missing")
    matching = [log for log in logs if str(log.get("address", "")).lower() == CONTRACT and
                [str(topic).lower() for topic in log.get("topics", [])] == [
                    EVENT_TOPIC, BOUNTY_ID, f"0x{ROUND:064x}", f"0x{'0'*24}{SOLVER[2:]}"
                ] and parse_uint(str(log.get("data"))) == BOND]
    require(len(matching) == 1, f"expected one exact SubmissionExpired event, found {len(matching)}")
    after_block = parse_uint(str(receipt.get("blockNumber")))
    status = parse_uint(cast.call(CONTRACT, "status()(uint8)", block=str(after_block)))
    bond = parse_uint(cast.call(CONTRACT, "activeClaimBond()(uint256)", block=str(after_block)))
    balance = parse_uint(cast.call(USDC, "balanceOf(address)(uint256)", SOLVER, block=str(after_block)))
    require(status == 1, f"post-state is not claimable: {status}")
    require(bond == 0, f"post-state bond is not zero: {bond}")
    require(balance - before.solver_usdc_balance == BOND, "solver USDC refund delta is not exactly 10000")
    return {
        "schema": "agent-bounties/expiry-772-round4-receipt-v1",
        "transaction_hash": transaction_hash,
        "block_number": after_block,
        "gas_used": parse_uint(str(receipt.get("gasUsed"))),
        "canonical_event": "SubmissionExpired",
        "round": ROUND,
        "solver": SOLVER,
        "claim_bond_refunded": BOND,
        "solver_usdc_balance_before": before.solver_usdc_balance,
        "solver_usdc_balance_after": balance,
        "status_after": status,
        "active_claim_bond_after": bond,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rpc-url", default=RPC_URL)
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--acknowledge", default="")
    args = parser.parse_args()
    cast = Cast(args.rpc_url)
    result = execute_once(cast, args.acknowledge) if args.execute else dry_run(cast)
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as handle:
            handle.write(encoded)
    print(encoded, end="")


if __name__ == "__main__":
    try:
        main()
    except (RecoveryError, OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"expire_772_round4: {error}")
