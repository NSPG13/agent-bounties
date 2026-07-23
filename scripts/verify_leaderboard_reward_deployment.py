#!/usr/bin/env python3
"""Attest one SolverLeaderboardRewards deployment from receipt and live getters."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")
NETWORKS = {
    "base-mainnet": (8453, "0x833589fCD6eDb6E08f4C7C32D4f71b54bdA02913"),
    "base-sepolia": (84532, "0x036CbD53842c5426634e7929541eC2318f3dCF7e"),
}
CODE_VISIBILITY_ATTEMPTS = 15
CODE_VISIBILITY_DELAY_SECONDS = 2


class VerificationError(RuntimeError):
    pass


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise VerificationError(f"{path} must contain an object")
    return value


def normalize_address(value: Any, field: str) -> str:
    text = str(value or "")
    if not ADDRESS.fullmatch(text):
        raise VerificationError(f"{field} is not an EVM address")
    return text.lower()


def chain_int(value: Any, field: str) -> int:
    text = str(value).strip().split()[0]
    try:
        parsed = int(text, 0)
    except ValueError as error:
        raise VerificationError(f"{field} is not an integer") from error
    if parsed < 0:
        raise VerificationError(f"{field} cannot be negative")
    return parsed


def run_cast(cast: str, rpc_url: str, *arguments: str) -> str:
    result = subprocess.run(
        [cast, *arguments, "--rpc-url", rpc_url],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip() or "cast failed"
        raise VerificationError(message)
    return result.stdout.strip()


def run_local_cast(cast: str, *arguments: str) -> str:
    result = subprocess.run(
        [cast, *arguments],
        check=False,
        capture_output=True,
        text=True,
        timeout=60,
    )
    if result.returncode != 0:
        message = result.stderr.strip() or result.stdout.strip() or "cast failed"
        raise VerificationError(message)
    return result.stdout.strip()


def wait_for_runtime_code(cast: str, rpc_url: str, contract: str) -> str:
    last_error: VerificationError | None = None
    for attempt in range(CODE_VISIBILITY_ATTEMPTS):
        try:
            runtime_code = run_cast(cast, rpc_url, "code", contract)
        except VerificationError as error:
            last_error = error
        else:
            if runtime_code != "0x":
                return runtime_code
        if attempt + 1 < CODE_VISIBILITY_ATTEMPTS:
            time.sleep(CODE_VISIBILITY_DELAY_SECONDS)
    detail = f": {last_error}" if last_error else ""
    raise VerificationError(f"deployed contract code was not visible after bounded retries{detail}")


def transaction_hash(broadcast: dict[str, Any], contract: str) -> str:
    for transaction in broadcast.get("transactions", []):
        if not isinstance(transaction, dict):
            continue
        deployed = str(transaction.get("contractAddress", "")).lower()
        if deployed != contract:
            continue
        value = str(transaction.get("hash", ""))
        if HASH.fullmatch(value):
            return value.lower()
    for receipt in broadcast.get("receipts", []):
        if not isinstance(receipt, dict):
            continue
        deployed = str(receipt.get("contractAddress", "")).lower()
        value = str(receipt.get("transactionHash", ""))
        if deployed == contract and HASH.fullmatch(value):
            return value.lower()
    raise VerificationError("broadcast does not identify the deployed contract transaction")


def verify(args: argparse.Namespace) -> dict[str, Any]:
    if args.network not in NETWORKS:
        raise VerificationError("unsupported network")
    expected_chain, expected_token = NETWORKS[args.network]
    manifest = load_json(args.manifest)
    broadcast = load_json(args.broadcast)
    contract = normalize_address(manifest.get("reward_contract"), "reward contract")
    signer_a = normalize_address(manifest.get("signer_a"), "signer A")
    signer_b = normalize_address(manifest.get("signer_b"), "signer B")
    if signer_a == signer_b:
        raise VerificationError("deployment signers are not distinct")

    chain_id = chain_int(run_cast(args.cast, args.rpc_url, "chain-id"), "chain id")
    if chain_id != expected_chain or chain_int(manifest.get("chain_id"), "manifest chain id") != chain_id:
        raise VerificationError("deployment chain does not match the requested network")
    runtime_code = wait_for_runtime_code(args.cast, args.rpc_url, contract)

    def call(signature: str) -> str:
        return run_cast(args.cast, args.rpc_url, "call", contract, signature)

    token = normalize_address(call("settlementToken()(address)"), "settlement token")
    if token != expected_token.lower():
        raise VerificationError("deployment is not pinned to native USDC")
    if normalize_address(call("signerA()(address)"), "live signer A") != signer_a:
        raise VerificationError("signer A getter drifted")
    if normalize_address(call("signerB()(address)"), "live signer B") != signer_b:
        raise VerificationError("signer B getter drifted")
    if chain_int(call("DAILY_REWARD()(uint256)"), "daily reward") != 3_000_000:
        raise VerificationError("daily reward drifted")
    if chain_int(call("WEEKLY_REWARD()(uint256)"), "weekly reward") != 26_000_000:
        raise VerificationError("weekly reward drifted")
    if chain_int(call("FINALIZATION_DELAY()(uint64)"), "finalization delay") != 3600:
        raise VerificationError("finalization delay drifted")
    daily_start = chain_int(call("firstDailyStart()(uint64)"), "first daily start")
    weekly_start = chain_int(call("firstWeeklyStart()(uint64)"), "first weekly start")
    if daily_start % 86_400 != 0 or weekly_start < 345_600 or (weekly_start - 345_600) % 604_800 != 0:
        raise VerificationError("program calendar alignment drifted")

    tx_hash = transaction_hash(broadcast, contract)
    receipt = json.loads(run_cast(args.cast, args.rpc_url, "receipt", tx_hash, "--json"))
    if chain_int(receipt.get("status"), "receipt status") != 1:
        raise VerificationError("deployment receipt failed")
    if normalize_address(receipt.get("contractAddress"), "receipt contract") != contract:
        raise VerificationError("receipt contract does not match the manifest")
    block_number = chain_int(receipt.get("blockNumber"), "deployment block")
    # `cast codehash` uses eth_getProof, which public Base RPCs do not expose.
    # Hashing standard eth_getCode output locally produces the same Keccak hash.
    code_hash = run_local_cast(args.cast, "keccak", runtime_code).lower()
    if not HASH.fullmatch(code_hash):
        raise VerificationError("runtime code hash is invalid")

    return {
        "schema": "agent-bounties/leaderboard-reward-deployment-v1",
        "network": args.network,
        "chain_id": chain_id,
        "reward_contract": contract,
        "settlement_token": token,
        "signer_a": signer_a,
        "signer_b": signer_b,
        "daily_reward_usdc_base_units": 3_000_000,
        "weekly_reward_usdc_base_units": 26_000_000,
        "first_daily_start": daily_start,
        "first_weekly_start": weekly_start,
        "deployment_transaction": tx_hash,
        "deployment_block": block_number,
        "runtime_code_hash": code_hash,
        "verified_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "evidence_boundary": "Confirmed deployment receipt and exact live immutable getter checks. This is not reward funding or payment evidence.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", required=True, choices=sorted(NETWORKS))
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--broadcast", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cast", default="cast")
    args = parser.parse_args()
    try:
        report = verify(args)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    except (OSError, json.JSONDecodeError, VerificationError) as error:
        print(f"leaderboard_reward_deployment=failed error={error}")
        return 1
    print(
        "leaderboard_reward_deployment=passed "
        f"network={report['network']} contract={report['reward_contract']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
