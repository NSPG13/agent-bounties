#!/usr/bin/env python3
"""Refund one exact failed x402 charge with canonical Base evidence."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from types import SimpleNamespace
from typing import Any

from eth_account import Account

import open_competition_v2_proof_rehearsal as rehearsal
import run_open_competition_v2_sepolia_rehearsal as chain
from _shared.evm import keccak256
from _shared.rpc import rpc


TRANSFER_TOPIC = keccak256(b"Transfer(address,address,uint256)")


class RecoveryError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RecoveryError(message)


def address_topic(address: str) -> str:
    value = address.lower().removeprefix("0x")
    require(len(value) == 40, "invalid EVM address")
    return "0x" + "0" * 24 + value


def is_transfer(
    log: dict[str, Any], *, asset: str, sender: str, recipient: str, amount: int
) -> bool:
    topics = [str(value).lower() for value in log.get("topics", [])]
    if len(topics) != 3:
        return False
    return (
        str(log.get("address", "")).lower() == asset.lower()
        and topics[0] == TRANSFER_TOPIC.lower()
        and topics[1] == address_topic(sender)
        and topics[2] == address_topic(recipient)
        and int(str(log.get("data", "0x0")), 16) == amount
    )


def canonical_transfer(
    url: str,
    transaction_hash: str,
    *,
    asset: str,
    sender: str,
    recipient: str,
    amount: int,
) -> dict[str, Any]:
    receipt = chain.SignedRpc(url).wait_receipt(transaction_hash)
    require(
        any(
            is_transfer(
                log,
                asset=asset,
                sender=sender,
                recipient=recipient,
                amount=amount,
            )
            for log in receipt.get("logs", [])
        ),
        "transaction does not contain the exact required USDC transfer",
    )
    return receipt


def existing_refund(
    url: str,
    payment_block: int,
    *,
    asset: str,
    broker: str,
    payer: str,
    amount: int,
) -> dict[str, Any] | None:
    logs = rpc(
        url,
        "eth_getLogs",
        [
            {
                "address": asset,
                "fromBlock": hex(payment_block),
                "toBlock": "latest",
                "topics": [
                    TRANSFER_TOPIC,
                    address_topic(broker),
                    address_topic(payer),
                ],
            }
        ],
    )
    for log in logs:
        if is_transfer(
            log, asset=asset, sender=broker, recipient=payer, amount=amount
        ):
            return log
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=("base-sepolia", "base-mainnet"), required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--payment-transaction", required=True)
    parser.add_argument("--asset", required=True)
    parser.add_argument("--payer", required=True)
    parser.add_argument("--broker", required=True)
    parser.add_argument("--amount", type=int, required=True)
    parser.add_argument("--broker-private-key-env", default="OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    chain.configure_network(SimpleNamespace(network=args.network, rpc_url=args.rpc_url))
    payment = canonical_transfer(
        args.rpc_url,
        args.payment_transaction,
        asset=args.asset,
        sender=args.payer,
        recipient=args.broker,
        amount=args.amount,
    )
    payment_block = int(payment["blockNumber"], 16)
    prior = existing_refund(
        args.rpc_url,
        payment_block,
        asset=args.asset,
        broker=args.broker,
        payer=args.payer,
        amount=args.amount,
    )

    if prior is None:
        key = os.environ.get(args.broker_private_key_env, "")
        require(bool(key), f"{args.broker_private_key_env} is required")
        broker = Account.from_key(key)
        require(broker.address.lower() == args.broker.lower(), "broker signer differs")
        client = chain.SignedRpc(args.rpc_url)
        refund = client.send(
            broker,
            to=args.asset,
            data=rehearsal.function_data(
                "transfer(address,uint256)",
                ["address", "uint256"],
                [args.payer, args.amount],
            ),
        )
        refund_transaction = chain.receipt_hash(refund)
        refund_block = int(refund["blockNumber"], 16)
        recovered_existing = False
    else:
        refund_transaction = str(prior["transactionHash"]).lower()
        canonical_transfer(
            args.rpc_url,
            refund_transaction,
            asset=args.asset,
            sender=args.broker,
            recipient=args.payer,
            amount=args.amount,
        )
        refund_block = int(prior["blockNumber"], 16)
        recovered_existing = True

    evidence = {
        "schema_version": "agent-bounties/open-competition-v2-x402-charge-recovery-v1",
        "passed": True,
        "network": args.network,
        "asset": args.asset.lower(),
        "payer": args.payer.lower(),
        "broker": args.broker.lower(),
        "amount": str(args.amount),
        "payment_transaction": args.payment_transaction.lower(),
        "payment_block": payment_block,
        "refund_transaction": refund_transaction,
        "refund_block": refund_block,
        "recovered_existing": recovered_existing,
        "evidence_boundary": "Exact canonical ERC-20 transfers prove the failed x402 charge and its refund; they do not prove competition settlement.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(evidence, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
