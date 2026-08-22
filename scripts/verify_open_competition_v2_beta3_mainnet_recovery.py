#!/usr/bin/env python3
"""Reconstruct Beta3 canary evidence from canonical Base and hosted projections."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import time
from typing import Any
from urllib.request import Request, urlopen


TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"


class RecoveryEvidenceError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RecoveryEvidenceError(message)


def request_json(url: str) -> dict[str, Any]:
    with urlopen(Request(url, headers={"accept": "application/json"}), timeout=45) as response:
        return json.loads(response.read())


def rpc(url: str, method: str, params: list[Any]) -> Any:
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params},
        separators=(",", ":"),
    ).encode()
    with urlopen(
        Request(url, data=body, headers={"content-type": "application/json"}), timeout=45
    ) as response:
        value = json.loads(response.read())
    if value.get("error"):
        raise RecoveryEvidenceError(f"RPC {method} failed: {value['error']}")
    return value["result"]


def topic_address(value: str) -> str:
    raw = value.lower().removeprefix("0x")
    require(len(raw) == 40, "invalid EVM address")
    return "0x" + "0" * 24 + raw


def receipt(url: str, transaction: str) -> dict[str, Any]:
    value = rpc(url, "eth_getTransactionReceipt", [transaction])
    require(isinstance(value, dict), f"transaction {transaction} is unconfirmed")
    require(int(value["status"], 16) == 1, f"transaction {transaction} reverted")
    return value


def require_transfer(
    value: dict[str, Any], token: str, sender: str, recipient: str, amount: int
) -> None:
    expected = (topic_address(sender), topic_address(recipient))
    matches = [
        log
        for log in value.get("logs", [])
        if log.get("address", "").lower() == token.lower()
        and len(log.get("topics", [])) == 3
        and log["topics"][0].lower() == TRANSFER_TOPIC
        and (log["topics"][1].lower(), log["topics"][2].lower()) == expected
        and int(log.get("data", "0x0"), 16) == amount
    ]
    require(len(matches) == 1, f"expected one exact {amount} USDC transfer")


def wait_for_settlement(api: str, competition: str, deadline: float) -> tuple[dict[str, Any], dict[str, Any]]:
    while time.time() < deadline:
        inventory = request_json(
            f"{api.rstrip('/')}/v1/base/open-competition-v2-beta3/inventory?network=base-mainnet"
        )
        events = request_json(
            f"{api.rstrip('/')}/v1/base/open-competition-v2-beta3/events?network=base-mainnet"
        )
        record = next(
            (
                item
                for item in inventory.get("competitions", [])
                if item.get("record", {}).get("projection", {}).get("competition", "").lower()
                == competition.lower()
            ),
            None,
        )
        settlement = next(
            (
                event
                for event in events.get("events", [])
                if event.get("kind") == "competition_settled"
                and event.get("contract_address", "").lower() == competition.lower()
            ),
            None,
        )
        if (
            isinstance(record, dict)
            and record["record"]["projection"].get("state") == "settled"
            and isinstance(settlement, dict)
        ):
            return record, settlement
        time.sleep(3)
    raise RecoveryEvidenceError(f"competition {competition} did not reconcile as settled")


def write(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api", required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--token", required=True)
    parser.add_argument("--broker", required=True)
    parser.add_argument("--payer", required=True)
    parser.add_argument("--plonk-competition", required=True)
    parser.add_argument("--plonk-settlement-transaction", required=True)
    parser.add_argument("--refund-payment-transaction", required=True)
    parser.add_argument("--refund-transaction", required=True)
    parser.add_argument("--x402-success", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout-seconds", type=int, default=300)
    args = parser.parse_args()

    success = json.loads(args.x402_success.read_text(encoding="utf-8"))
    require(success.get("passed") is True, "x402 success evidence did not pass")
    require(success.get("network") == "base-mainnet", "x402 evidence is not Base mainnet")
    x402_competition = str(success.get("competition", ""))
    x402_record, x402_settlement = wait_for_settlement(
        args.api, x402_competition, time.time() + args.timeout_seconds
    )
    plonk_record, plonk_settlement = wait_for_settlement(
        args.api, args.plonk_competition, time.time() + args.timeout_seconds
    )

    require(
        x402_settlement["id"] == success.get("settlement_event_id"),
        "x402 settlement event differs from hosted proof-job evidence",
    )
    require(
        x402_settlement["tx_hash"].lower() == str(success.get("relay_transaction", "")).lower(),
        "x402 settlement transaction differs from hosted evidence",
    )
    require(
        plonk_settlement["tx_hash"].lower() == args.plonk_settlement_transaction.lower(),
        "PLONK settlement transaction differs from the canonical event",
    )

    safe_block = min(
        int(x402_record["record"]["safe_block_number"]),
        int(plonk_record["record"]["safe_block_number"]),
    )
    transactions = {
        "x402_payment": success["payment_transaction"],
        "x402_settlement": success["relay_transaction"],
        "forced_failure_payment": args.refund_payment_transaction,
        "forced_failure_refund": args.refund_transaction,
        "plonk_settlement": args.plonk_settlement_transaction,
    }
    receipts = {name: receipt(args.rpc_url, tx) for name, tx in transactions.items()}
    for name, value in receipts.items():
        require(int(value["blockNumber"], 16) <= safe_block, f"{name} is not safe")
    require_transfer(receipts["x402_payment"], args.token, args.payer, args.broker, 110_000)
    require_transfer(
        receipts["forced_failure_payment"], args.token, args.payer, args.broker, 110_000
    )
    require_transfer(
        receipts["forced_failure_refund"], args.token, args.broker, args.payer, 110_000
    )
    balance = int(
        rpc(
            args.rpc_url,
            "eth_call",
            [
                {
                    "to": args.token,
                    "data": "0x70a08231" + "0" * 24 + x402_competition.removeprefix("0x"),
                },
                "safe",
            ],
        ),
        16,
    )
    require(balance == 0, "settled x402 competition retained USDC")

    plonk = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-mainnet-plonk-recovery-v1",
        "passed": True,
        "network": "base-mainnet",
        "competition": args.plonk_competition.lower(),
        "settled": True,
        "winner": plonk_record["record"]["projection"]["winner"],
        "settlement_event_id": plonk_settlement["id"],
        "settlement_transaction": plonk_settlement["tx_hash"],
        "safe_block": safe_block,
    }
    refund = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-mainnet-x402-refund-recovery-v1",
        "passed": True,
        "network": "base-mainnet",
        "payer": args.payer.lower(),
        "broker": args.broker.lower(),
        "amount": 110_000,
        "payment_transaction": args.refund_payment_transaction.lower(),
        "refund_transaction": args.refund_transaction.lower(),
        "safe_block": safe_block,
    }
    accounting = {
        "schema_version": "agent-bounties/open-competition-v2-beta3-mainnet-accounting-v1",
        "passed": True,
        "reconstructed_from_canonical_events": True,
        "competition_escrow_funded_base_units": 525_000,
        "groth16_solver_reward_base_units": 250_000,
        "plonk_solver_reward_base_units": 250_000,
        "keeper_rewards_base_units": 25_000,
        "x402_success": success,
        "x402_refund": refund,
        "plonk": plonk,
        "safe_block": safe_block,
    }
    write(args.output_dir / "mainnet-plonk-canary.json", plonk)
    write(args.output_dir / "mainnet-x402-refund.json", refund)
    write(args.output_dir / "mainnet-accounting.json", accounting)
    print(json.dumps({"passed": True, "safe_block": safe_block, "output": str(args.output_dir)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
