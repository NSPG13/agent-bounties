#!/usr/bin/env python3
"""Recover funds left in deterministic Beta3 Base Sepolia rehearsal actors."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import time
from typing import Any

from eth_account import Account

from _shared.evm import address_word, keccak_bytes, uint_word
from _shared.rpc import rpc
from run_open_competition_v2_sepolia_rehearsal import (
    SignedRpc,
    derived_actor,
    normalized_key,
)


CHAIN_ID = 84532
NETWORK = "base-sepolia"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
TRANSFER_SELECTOR = "a9059cbb"
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
ETH_SWEEP_GAS_RESERVE = 150_000
ZERO_ADDRESS = "0x" + "00" * 20


class RecoveryError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RecoveryError(message)


def call_word(url: str, address: str, signature: str) -> int:
    selector = "0x" + keccak_bytes(signature.encode())[:4].hex()
    value = rpc(url, "eth_call", [{"to": address, "data": selector}, "latest"])
    require(isinstance(value, str) and value.startswith("0x"), f"{signature} returned invalid data")
    return int(value, 16)


def call_address(url: str, address: str, signature: str) -> str:
    value = call_word(url, address, signature)
    return "0x" + value.to_bytes(32, "big")[-20:].hex()


def usdc_balance(url: str, address: str, block: str = "latest") -> int:
    data = "0x70a08231" + address_word(address).hex()
    value = rpc(url, "eth_call", [{"to": USDC, "data": data}, block])
    return int(value, 16)


def transfer_data(recipient: str, amount: int) -> str:
    return "0x" + TRANSFER_SELECTOR + address_word(recipient).hex() + uint_word(amount).hex()


def address_call_data(signature: str, address: str) -> str:
    return "0x" + keccak_bytes(signature.encode())[:4].hex() + address_word(address).hex()


def contribution(url: str, competition: str, contributor: str) -> int:
    value = rpc(
        url,
        "eth_call",
        [
            {
                "to": competition,
                "data": address_call_data("contributions(address)", contributor),
            },
            "latest",
        ],
    )
    return int(value, 16)


def actor_set(root_key: bytes, source_commit: str, run_id: str, attempts: list[int]) -> list[Any]:
    actors: list[Any] = []
    seen: set[str] = set()
    for attempt in attempts:
        salt = f"{run_id}:{attempt}:sepolia"
        for label in ("solver-a", "solver-b"):
            actor = derived_actor(root_key, source_commit, label, salt)
            if actor.address.lower() not in seen:
                actors.append(actor)
                seen.add(actor.address.lower())
    return actors


def parse_actor_scope(value: str) -> tuple[str, str, int]:
    parts = value.split(":")
    require(len(parts) == 3, "actor scope must be SOURCE_COMMIT:RUN_ID:ATTEMPT")
    source_commit, run_id, attempt_text = parts
    require(re.fullmatch(r"[0-9a-f]{40}", source_commit) is not None, "actor scope source commit is invalid")
    require(run_id.isdigit() and int(run_id) > 0, "actor scope run ID is invalid")
    require(attempt_text.isdigit() and int(attempt_text) > 0, "actor scope attempt is invalid")
    return source_commit, run_id, int(attempt_text)


def sweepable_eth(balance: int, maximum_fee_per_gas: int) -> int:
    reserve = ETH_SWEEP_GAS_RESERVE * maximum_fee_per_gas
    return max(balance - reserve, 0)


def expired_competition_needs_expiry(
    status: int,
    proof_deadline: int,
    block_timestamp: int,
    leader: str,
) -> bool:
    if status == 1:
        require(block_timestamp > proof_deadline, "competition proof deadline has not passed")
        require(leader == ZERO_ADDRESS, "expired competition has a qualifying leader")
        return True
    require(status == 3, f"expired competition has unrecoverable status {status}")
    return False


def wait_safe(url: str, receipts: list[dict[str, Any]], timeout: int = 1_800) -> dict[str, Any]:
    target = max((int(receipt["blockNumber"], 16) for receipt in receipts), default=0)
    deadline = time.time() + timeout
    while time.time() < deadline:
        safe = rpc(url, "eth_getBlockByNumber", ["safe", False])
        if safe and int(safe["number"], 16) >= target:
            for receipt in receipts:
                canonical = rpc(url, "eth_getBlockByNumber", [receipt["blockNumber"], False])
                require(
                    canonical and canonical["hash"].lower() == receipt["blockHash"].lower(),
                    "recovery receipt was reorged before the safe block",
                )
            return safe
        time.sleep(3)
    raise RecoveryError("recovery transactions did not reach a Base safe block")


def recover(args: argparse.Namespace) -> dict[str, Any]:
    require(args.rpc_url.startswith("https://"), "recovery RPC must use HTTPS")
    require(int(rpc(args.rpc_url, "eth_chainId", []), 16) == CHAIN_ID, "recovery RPC is not Base Sepolia")
    require(ADDRESS.fullmatch(args.deployer) is not None, "expected deployer is invalid")
    require(ADDRESS.fullmatch(args.competition) is not None, "competition address is invalid")
    require(
        all(ADDRESS.fullmatch(address) is not None for address in args.funding_competition),
        "funding competition address is invalid",
    )
    require(
        all(ADDRESS.fullmatch(address) is not None for address in args.expired_competition),
        "expired competition address is invalid",
    )
    require(re.fullmatch(r"[0-9a-f]{40}", args.source_commit) is not None, "source commit is invalid")
    require(args.attempts and all(attempt > 0 for attempt in args.attempts), "attempts must be positive")

    root_key = normalized_key(os.environ.get(args.private_key_env, ""))
    deployer = Account.from_key(root_key)
    require(deployer.address.lower() == args.deployer.lower(), "protected key does not match the expected deployer")
    require(call_address(args.rpc_url, args.competition, "creator()") == deployer.address.lower(), "competition creator mismatch")

    actor_scopes = [(args.source_commit, args.run_id, attempt) for attempt in args.attempts]
    actor_scopes.extend(parse_actor_scope(scope) for scope in args.additional_actor_scope)
    actors: list[Any] = []
    seen_actors: set[str] = set()
    for source_commit, run_id, attempt in actor_scopes:
        for actor in actor_set(root_key, source_commit, run_id, [attempt]):
            if actor.address.lower() not in seen_actors:
                actors.append(actor)
                seen_actors.add(actor.address.lower())
    actor_addresses = {actor.address.lower() for actor in actors}
    missing = sorted(address.lower() for address in args.required_actor if address.lower() not in actor_addresses)
    require(not missing, f"required rehearsal actors were not derived: {missing}")

    client = SignedRpc(args.rpc_url)
    receipts: list[dict[str, Any]] = []
    actions: list[dict[str, Any]] = []
    status_before = call_word(args.rpc_url, args.competition, "status()")
    if status_before == 1:
        deadline = call_word(args.rpc_url, args.competition, "proofDeadline()")
        require(int(rpc(args.rpc_url, "eth_getBlockByNumber", ["latest", False])["timestamp"], 16) > deadline, "competition proof deadline has not passed")
        require(call_address(args.rpc_url, args.competition, "leader()") != "0x" + "00" * 20, "competition has no qualifying leader")
        receipt = client.send(
            deployer,
            to=args.competition,
            data="0x" + keccak_bytes(b"finalizeBestScore()")[:4].hex(),
        )
        receipts.append(receipt)
        actions.append({"kind": "finalize_best_score", "transaction_hash": receipt["transactionHash"]})
    else:
        require(status_before == 2, f"competition has unrecoverable status {status_before}")

    for competition in args.expired_competition:
        require(
            call_address(args.rpc_url, competition, "creator()") == deployer.address.lower(),
            "expired competition creator mismatch",
        )
        expired_status = call_word(args.rpc_url, competition, "status()")
        if expired_status == 1:
            deadline = call_word(args.rpc_url, competition, "proofDeadline()")
            latest_timestamp = int(
                rpc(args.rpc_url, "eth_getBlockByNumber", ["latest", False])["timestamp"],
                16,
            )
            leader = call_address(args.rpc_url, competition, "leader()")
        else:
            deadline = 0
            latest_timestamp = 0
            leader = ZERO_ADDRESS
        if expired_competition_needs_expiry(
            expired_status,
            deadline,
            latest_timestamp,
            leader,
        ):
            receipt = client.send(
                deployer,
                to=competition,
                data="0x" + keccak_bytes(b"expireCompetition()")[:4].hex(),
            )
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "expire_competition_without_leader",
                    "competition": competition.lower(),
                    "transaction_hash": receipt["transactionHash"],
                }
            )
        refundable = contribution(args.rpc_url, competition, deployer.address)
        if refundable:
            receipt = client.send(
                deployer,
                to=competition,
                data=address_call_data("withdrawRefundFor(address)", deployer.address),
            )
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "withdraw_expired_competition_refund",
                    "competition": competition.lower(),
                    "amount_base_units": refundable,
                    "transaction_hash": receipt["transactionHash"],
                }
            )

    for competition in args.funding_competition:
        require(
            call_address(args.rpc_url, competition, "creator()") == deployer.address.lower(),
            "funding competition creator mismatch",
        )
        funding_status = call_word(args.rpc_url, competition, "status()")
        if funding_status == 0:
            receipt = client.send(
                deployer,
                to=competition,
                data="0x" + keccak_bytes(b"cancelFunding()")[:4].hex(),
            )
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "cancel_funding_competition",
                    "competition": competition.lower(),
                    "transaction_hash": receipt["transactionHash"],
                }
            )
        else:
            require(funding_status == 3, f"funding competition has unrecoverable status {funding_status}")
        refundable = contribution(args.rpc_url, competition, deployer.address)
        if refundable:
            receipt = client.send(
                deployer,
                to=competition,
                data=address_call_data("withdrawRefundFor(address)", deployer.address),
            )
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "withdraw_funding_refund",
                    "competition": competition.lower(),
                    "amount_base_units": refundable,
                    "transaction_hash": receipt["transactionHash"],
                }
            )

    for actor in actors:
        token_balance = usdc_balance(args.rpc_url, actor.address)
        if token_balance:
            receipt = client.send(actor, to=USDC, data=transfer_data(deployer.address, token_balance))
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "sweep_actor_usdc",
                    "actor": actor.address.lower(),
                    "amount_base_units": token_balance,
                    "transaction_hash": receipt["transactionHash"],
                }
            )

        balance = int(rpc(args.rpc_url, "eth_getBalance", [actor.address, "latest"]), 16)
        maximum_fee, _ = client.fees()
        value = sweepable_eth(balance, maximum_fee)
        if value:
            receipt = client.send(actor, to=deployer.address, value=value)
            receipts.append(receipt)
            actions.append(
                {
                    "kind": "sweep_actor_eth",
                    "actor": actor.address.lower(),
                    "amount_wei": value,
                    "transaction_hash": receipt["transactionHash"],
                }
            )

    safe = wait_safe(args.rpc_url, receipts)
    require(call_word(args.rpc_url, args.competition, "status()") == 2, "competition did not settle")
    for competition in args.funding_competition:
        require(call_word(args.rpc_url, competition, "status()") == 3, "funding competition did not cancel")
        require(
            contribution(args.rpc_url, competition, deployer.address) == 0,
            "funding competition refund remains unclaimed",
        )
    for competition in args.expired_competition:
        require(call_word(args.rpc_url, competition, "status()") == 3, "expired competition did not cancel")
        require(
            contribution(args.rpc_url, competition, deployer.address) == 0,
            "expired competition refund remains unclaimed",
        )
        require(usdc_balance(args.rpc_url, competition, safe["number"]) == 0, "expired competition still holds USDC")
    deployer_usdc = usdc_balance(args.rpc_url, deployer.address, safe["number"])
    deployer_eth = int(rpc(args.rpc_url, "eth_getBalance", [deployer.address, safe["number"]]), 16)
    require(deployer_usdc >= args.minimum_usdc, "recovered deployer USDC is below the release reserve")
    require(deployer_eth >= args.minimum_eth, "recovered deployer ETH is below the release reserve")

    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-sepolia-rehearsal-recovery-v1",
        "passed": True,
        "network": NETWORK,
        "chain_id": CHAIN_ID,
        "source_commit": args.source_commit,
        "release_run_id": args.run_id,
        "attempts": args.attempts,
        "actor_scopes": [
            {"source_commit": source_commit, "run_id": run_id, "attempt": attempt}
            for source_commit, run_id, attempt in actor_scopes
        ],
        "competition": args.competition.lower(),
        "funding_competitions": [address.lower() for address in args.funding_competition],
        "expired_competitions": [address.lower() for address in args.expired_competition],
        "deployer": deployer.address.lower(),
        "derived_actor_addresses": sorted(actor_addresses),
        "actions": actions,
        "safe_block_number": int(safe["number"], 16),
        "safe_block_hash": safe["hash"].lower(),
        "deployer_usdc_base_units": deployer_usdc,
        "deployer_eth_wei": deployer_eth,
        "evidence_boundary": "Base Sepolia rehearsal recovery only. These test-token sweeps are not mainnet funding, solver payment, or adoption evidence.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--private-key-env", default="BASE_SEPOLIA_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--attempt", dest="attempts", action="append", type=int, required=True)
    parser.add_argument("--additional-actor-scope", action="append", default=[])
    parser.add_argument("--competition", required=True)
    parser.add_argument("--funding-competition", action="append", default=[])
    parser.add_argument("--expired-competition", action="append", default=[])
    parser.add_argument("--required-actor", action="append", default=[])
    parser.add_argument("--minimum-usdc", type=int, default=1_000_000)
    parser.add_argument("--minimum-eth", type=int, default=500_000_000_000_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = recover(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "actions": len(result["actions"]), "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
