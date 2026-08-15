#!/usr/bin/env python3
"""Replay the exact canonical-child-v1 module bundle on a pinned Base fork."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
from typing import Any

from _shared.rpc import rpc


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def find_anvil(repo: Path, configured: str | None) -> str:
    executable = "anvil.exe" if os.name == "nt" else "anvil"
    candidates = [configured, shutil.which("anvil"), str(repo / ".tools" / "foundry" / executable)]
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    raise FileNotFoundError("anvil was not found; install Foundry or pass --anvil")


def wait_ready(url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"anvil exited before becoming ready: {process.returncode}")
        try:
            if rpc(url, "eth_chainId", []) == "0x2105":
                return
        except RuntimeError:
            pass
        time.sleep(0.2)
    raise TimeoutError("anvil did not become ready")


def wait_receipt(url: str, transaction_hash: str) -> dict[str, Any]:
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if receipt:
            if receipt.get("status") != "0x1":
                raise RuntimeError(f"fork deployment reverted: {transaction_hash}")
            return receipt
        time.sleep(0.1)
    raise TimeoutError(f"timed out waiting for fork receipt {transaction_hash}")


def address_result(value: str) -> str:
    return f"0x{value.removeprefix('0x')[-40:]}".lower()


def validate_bundle(bundle: dict[str, Any]) -> None:
    deployment = bundle.get("deployment", {})
    if (
        bundle.get("schema_version")
        != "agent-bounties/canonical-child-verifier-deployment-v1"
        or bundle.get("chain_id") != 8453
        or deployment.get("to") is not None
        or deployment.get("value_wei") != 0
    ):
        raise ValueError("invalid canonical child verifier deployment bundle")


def rehearse(repo: Path, bundle_path: Path, upstream_rpc: str, anvil: str | None) -> dict[str, Any]:
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    validate_bundle(bundle)
    deployment = bundle["deployment"]
    block = bundle["preflight_block"]
    port = free_port()
    local_url = f"http://127.0.0.1:{port}"
    command = [
        find_anvil(repo, anvil),
        "--fork-url",
        upstream_rpc,
        "--fork-block-number",
        str(block["number"]),
        "--chain-id",
        "8453",
        "--port",
        str(port),
        "--silent",
    ]
    creation_flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, creationflags=creation_flags)
    try:
        wait_ready(local_url, process)
        observed_block = rpc(local_url, "eth_getBlockByNumber", [hex(block["number"]), False])
        if not observed_block or observed_block.get("hash", "").lower() != block["hash"]:
            raise RuntimeError("pinned Base block hash mismatch")
        deployer = deployment["from"]
        nonce = int(rpc(local_url, "eth_getTransactionCount", [deployer, "latest"]), 16)
        if nonce != deployment["deployer_nonce"]:
            raise RuntimeError(f"deployer nonce mismatch: expected {deployment['deployer_nonce']} got {nonce}")
        if rpc(local_url, "eth_getCode", [deployment["expected_contract"], "latest"]) != "0x":
            raise RuntimeError("predicted deployment address is occupied at the pinned block")

        rpc(local_url, "anvil_setBalance", [deployer, hex(10**18)])
        rpc(local_url, "anvil_impersonateAccount", [deployer])
        transaction = {"from": deployer, "data": deployment["data"], "value": "0x0"}
        estimated_gas = int(rpc(local_url, "eth_estimateGas", [transaction]), 16)
        transaction_hash = rpc(local_url, "eth_sendTransaction", [transaction])
        receipt = wait_receipt(local_url, transaction_hash)
        if receipt.get("contractAddress", "").lower() != deployment["expected_contract"]:
            raise RuntimeError(f"fork contract address mismatch: {receipt.get('contractAddress')}")
        code = rpc(local_url, "eth_getCode", [deployment["expected_contract"], "latest"]).lower()
        if code != deployment["expected_runtime_code"]:
            raise RuntimeError("fork runtime bytecode mismatch")

        module = deployment["expected_contract"]
        factory = address_result(rpc(local_url, "eth_call", [{"to": module, "data": "0x044f3e72"}, "latest"]))
        token = address_result(rpc(local_url, "eth_call", [{"to": module, "data": "0x7b9e618d"}, "latest"]))
        criteria = rpc(local_url, "eth_call", [{"to": module, "data": "0x77de6ca7"}, "latest"]).lower()
        if factory != bundle["canonical_factory"] or token != bundle["settlement_token"]:
            raise RuntimeError("fork immutable address mismatch")
        if criteria != bundle["acceptance_criteria_hash"]:
            raise RuntimeError("fork acceptance criteria commitment mismatch")

        return {
            "status": "passed",
            "network": "base-mainnet-fork",
            "fork_block": block,
            "expected_contract": module,
            "estimated_gas": estimated_gas,
            "transaction_hash": transaction_hash,
            "runtime_code_hash": deployment["runtime_code_hash"],
            "source_commit": bundle["source_commit"],
            "evidence_boundary": "Fork rehearsal only; no Base mainnet transaction or payment occurred.",
        }
    finally:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--bundle",
        type=Path,
        default=Path("deployments/canonical-child-verifier-base-mainnet-deployment.json"),
    )
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", "https://base-rpc.publicnode.com"))
    parser.add_argument("--anvil")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[1]
    bundle_path = args.bundle if args.bundle.is_absolute() else repo / args.bundle
    print(json.dumps(rehearse(repo, bundle_path, args.rpc_url, args.anvil), indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
