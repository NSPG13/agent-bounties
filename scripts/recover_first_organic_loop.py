#!/usr/bin/env python3
"""Recover the protected solver bond for the first organic mainnet loop.

This is deliberately not a generic transaction runner. Every on-chain identity,
state value, call selector, and receipt event is pinned to issue #169.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any, Sequence


CHAIN_ID = 8453
RPC_URL = "https://mainnet.base.org"
CONTRACT = "0x680030abf3ffffbc8d0a550b6355a8713c54d3c8"
CONTRACT_CODEHASH = (
    "0x6e7d6297e170d10e6484c9b72314bb0e2173cd967aa8e05231ee369dbde0c0a1"
)
FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
BOUNTY_ID = "0x01f058453a0eabf14ea67205d0e794be022f718bb9c133f8e507269dec9f1f38"
SOLVER = "0x65e3e64935ea709bdf1119e1fc991299c04c0154"
ROUND = 1
SUBMITTED_STATUS = 3
CLAIMABLE_STATUS = 1
VERIFICATION_EXPIRES_AT = 1_784_052_795
CLAIM_BOND = 100_000
EXPIRE_SELECTOR = "0xf9251ec7"
EXPIRED_EVENT_TOPIC = (
    "0x2d21c86724fb1d7ecb4465174a1fdf4969530254e9351bcb18f680eaa85d75e9"
)
ZERO_ADDRESS = "0x0000000000000000000000000000000000000000"
GAS_LIMIT = 200_000


class RecoveryError(RuntimeError):
    pass


def parse_uint(value: str) -> int:
    token = value.strip().split()[0]
    return int(token, 0)


def normalize_address(value: str) -> str:
    normalized = value.strip().lower()
    if len(normalized) != 42 or not normalized.startswith("0x"):
        raise RecoveryError(f"invalid address returned by RPC: {value!r}")
    int(normalized[2:], 16)
    return normalized


def extract_json(value: str) -> dict[str, Any]:
    start = value.find("{")
    if start < 0:
        raise RecoveryError("cast did not return a JSON object")
    parsed = json.loads(value[start:])
    if not isinstance(parsed, dict):
        raise RecoveryError("cast JSON response is not an object")
    return parsed


class CastClient:
    def __init__(self, cast_bin: str, rpc_url: str, block_tag: str = "finalized") -> None:
        self.cast_bin = cast_bin
        self.rpc_url = rpc_url
        self.block_tag = block_tag

    def run(self, *args: str, retry: bool = True) -> str:
        command = [self.cast_bin, *args]
        attempts = 3 if retry else 1
        message = "unknown cast failure"
        for attempt in range(attempts):
            completed = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                encoding="utf-8",
            )
            if completed.returncode == 0:
                return completed.stdout.strip()
            message = completed.stderr.strip() or completed.stdout.strip()
            if attempt + 1 < attempts:
                time.sleep(2)
        raise RecoveryError(f"cast command failed: {message}")

    def chain_id(self) -> int:
        return parse_uint(self.run("chain-id", "--rpc-url", self.rpc_url))

    def block_timestamp(self) -> int:
        return parse_uint(
            self.run(
                "block",
                self.block_tag,
                "--field",
                "timestamp",
                "--rpc-url",
                self.rpc_url,
            )
        )

    def codehash(self) -> str:
        code = self.run(
            "code",
            CONTRACT,
            "--block",
            self.block_tag,
            "--rpc-url",
            self.rpc_url,
        ).lower()
        if code == "0x":
            raise RecoveryError("pinned recovery contract has no bytecode")
        return self.run("keccak", code).lower()

    def expire_selector(self) -> str:
        return self.run("calldata", "expireSubmission()").lower()

    def call(self, signature: str) -> str:
        return self.run(
            "call",
            CONTRACT,
            signature,
            "--block",
            self.block_tag,
            "--rpc-url",
            self.rpc_url,
        )

    def send_expiry(self, private_key: str) -> dict[str, Any]:
        output = self.run(
            "send",
            CONTRACT,
            "expireSubmission()",
            "--rpc-url",
            self.rpc_url,
            "--private-key",
            private_key,
            "--gas-limit",
            str(GAS_LIMIT),
            "--json",
            retry=False,
        )
        return extract_json(output)

    def keeper_address(self, private_key: str) -> str:
        return normalize_address(
            self.run("wallet", "address", "--private-key", private_key)
        )

    def keeper_balance(self, keeper: str) -> int:
        return parse_uint(
            self.run(
                "balance",
                keeper,
                "--block",
                self.block_tag,
                "--rpc-url",
                self.rpc_url,
            )
        )


@dataclass(frozen=True)
class ContractState:
    chain_id: int
    block_timestamp: int
    codehash: str
    expire_selector: str
    factory: str
    settlement_token: str
    bounty_id: str
    round: int
    status: int
    solver: str
    verification_expires_at: int
    active_claim_bond: int


def read_state(client: CastClient) -> ContractState:
    return ContractState(
        chain_id=client.chain_id(),
        block_timestamp=client.block_timestamp(),
        codehash=client.codehash(),
        expire_selector=client.expire_selector(),
        factory=normalize_address(client.call("factory()(address)")),
        settlement_token=normalize_address(client.call("settlementToken()(address)")),
        bounty_id=client.call("bountyId()(bytes32)").strip().lower(),
        round=parse_uint(client.call("round()(uint64)")),
        status=parse_uint(client.call("status()(uint8)")),
        solver=normalize_address(client.call("solver()(address)")),
        verification_expires_at=parse_uint(
            client.call("verificationExpiresAt()(uint64)")
        ),
        active_claim_bond=parse_uint(client.call("activeClaimBond()(uint256)")),
    )


def validate_identity(state: ContractState) -> None:
    expected = {
        "chain_id": CHAIN_ID,
        "codehash": CONTRACT_CODEHASH,
        "expire_selector": EXPIRE_SELECTOR,
        "factory": FACTORY,
        "settlement_token": USDC,
        "bounty_id": BOUNTY_ID,
        "round": ROUND,
    }
    for field, value in expected.items():
        if getattr(state, field) != value:
            raise RecoveryError(
                f"fail-closed identity mismatch for {field}: "
                f"expected {value}, got {getattr(state, field)}"
            )


def validate_submitted(state: ContractState) -> None:
    expected = {
        "status": SUBMITTED_STATUS,
        "solver": SOLVER,
        "verification_expires_at": VERIFICATION_EXPIRES_AT,
        "active_claim_bond": CLAIM_BOND,
    }
    for field, value in expected.items():
        if getattr(state, field) != value:
            raise RecoveryError(
                f"fail-closed submitted-state mismatch for {field}: "
                f"expected {value}, got {getattr(state, field)}"
            )


def validate_recovered(state: ContractState) -> None:
    expected = {
        "status": CLAIMABLE_STATUS,
        "solver": ZERO_ADDRESS,
        "verification_expires_at": 0,
        "active_claim_bond": 0,
    }
    for field, value in expected.items():
        if getattr(state, field) != value:
            raise RecoveryError(
                f"fail-closed recovered-state mismatch for {field}: "
                f"expected {value}, got {getattr(state, field)}"
            )


def padded_uint(value: int) -> str:
    return "0x" + value.to_bytes(32, "big").hex()


def padded_address(value: str) -> str:
    return "0x" + (b"\x00" * 12 + bytes.fromhex(value[2:])).hex()


def validate_receipt(receipt: dict[str, Any]) -> str:
    status = receipt.get("status")
    if isinstance(status, str):
        status = int(status, 0)
    if status != 1:
        raise RecoveryError(f"expiry transaction failed with receipt status {status!r}")

    expected_topics = [
        EXPIRED_EVENT_TOPIC,
        BOUNTY_ID,
        padded_uint(ROUND),
        padded_address(SOLVER),
    ]
    matching_logs = []
    for log in receipt.get("logs", []):
        if normalize_address(log.get("address", "")) != CONTRACT:
            continue
        topics = [str(topic).lower() for topic in log.get("topics", [])]
        if topics == expected_topics and str(log.get("data", "")).lower() == padded_uint(
            CLAIM_BOND
        ):
            matching_logs.append(log)
    if len(matching_logs) != 1:
        raise RecoveryError(
            "receipt must contain exactly one canonical SubmissionExpired event "
            "for the pinned solver and 0.10 USDC bond"
        )

    transaction_hash = receipt.get("transactionHash") or receipt.get("transaction_hash")
    if not isinstance(transaction_hash, str) or len(transaction_hash) != 66:
        raise RecoveryError("receipt is missing a canonical transaction hash")
    return transaction_hash.lower()


def receipt_block_tag(receipt: dict[str, Any]) -> str:
    block_number = receipt.get("blockNumber") or receipt.get("block_number")
    if isinstance(block_number, int) and block_number >= 0:
        return str(block_number)
    if isinstance(block_number, str):
        parse_uint(block_number)
        return block_number
    raise RecoveryError("receipt is missing a canonical block number")


def write_report(path: Path, report: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def recover(
    client: CastClient,
    *,
    execute: bool,
    private_key: str | None,
    report_path: Path,
) -> dict[str, Any]:
    before = read_state(client)
    validate_identity(before)
    report: dict[str, Any] = {
        "schema": "agent-bounties/first-organic-loop-recovery-v1",
        "contract": CONTRACT,
        "solver": SOLVER,
        "expected_bond_minor": CLAIM_BOND,
        "verification_expires_at": VERIFICATION_EXPIRES_AT,
        "execute_requested": execute,
        "before": asdict(before),
    }

    if before.status == CLAIMABLE_STATUS:
        validate_recovered(before)
        report["outcome"] = "already_recovered"
        write_report(report_path, report)
        return report

    validate_submitted(before)
    if before.block_timestamp <= VERIFICATION_EXPIRES_AT:
        report["outcome"] = "not_due"
        report["seconds_until_due"] = VERIFICATION_EXPIRES_AT - before.block_timestamp + 1
        write_report(report_path, report)
        return report

    if not execute:
        report["outcome"] = "ready"
        write_report(report_path, report)
        return report

    if not private_key:
        raise RecoveryError("BASE_KEEPER_PRIVATE_KEY is required for --execute")
    keeper = client.keeper_address(private_key)
    keeper_balance = client.keeper_balance(keeper)
    if keeper_balance <= 0:
        raise RecoveryError(f"keeper {keeper} has no Base ETH for gas")

    receipt = client.send_expiry(private_key)
    transaction_hash = validate_receipt(receipt)
    client.block_tag = receipt_block_tag(receipt)
    after = read_state(client)
    validate_identity(after)
    validate_recovered(after)

    report.update(
        {
            "outcome": "recovered",
            "keeper": keeper,
            "keeper_balance_before_wei": keeper_balance,
            "transaction_hash": transaction_hash,
            "basescan_url": f"https://basescan.org/tx/{transaction_hash}",
            "after": asdict(after),
        }
    )
    write_report(report_path, report)
    return report


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--execute", action="store_true")
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", RPC_URL))
    parser.add_argument("--cast-bin", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument(
        "--block-tag",
        choices=("finalized", "latest"),
        default="finalized",
        help="Use latest only for a disposable local fork; automation uses finalized.",
    )
    parser.add_argument(
        "--report",
        type=Path,
        default=Path("target/first-organic-loop-recovery.json"),
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    client = CastClient(args.cast_bin, args.rpc_url, args.block_tag)
    try:
        report = recover(
            client,
            execute=args.execute,
            private_key=os.environ.get("BASE_KEEPER_PRIVATE_KEY"),
            report_path=args.report,
        )
    except (RecoveryError, json.JSONDecodeError, OSError, ValueError) as error:
        print(f"first_organic_loop_recovery=failed error={error}", file=sys.stderr)
        return 1
    print(
        "first_organic_loop_recovery="
        f"{report['outcome']} contract={CONTRACT} solver={SOLVER}"
    )
    if report.get("transaction_hash"):
        print(f"transaction_hash={report['transaction_hash']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
