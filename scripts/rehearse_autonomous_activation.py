#!/usr/bin/env python3
"""Replay an unsigned autonomous-v1 activation bundle on a Base mainnet fork."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from Crypto.Hash import keccak


def rpc(url: str, method: str, params: list[Any], request_id: int = 1) -> Any:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(url, data=payload, headers={"content-type": "application/json"})
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except URLError as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")


def selector(signature: str) -> str:
    digest = keccak.new(digest_bits=256)
    digest.update(signature.encode("ascii"))
    return digest.hexdigest()[:8]


def address_word(address: str) -> str:
    raw = address.removeprefix("0x")
    if len(raw) != 40 or any(character not in "0123456789abcdefABCDEF" for character in raw):
        raise ValueError(f"invalid EVM address: {address}")
    return raw.lower().rjust(64, "0")


def call(url: str, contract: str, signature: str, arguments: str = "") -> str:
    return rpc(
        url,
        "eth_call",
        [{"to": contract, "data": f"0x{selector(signature)}{arguments}"}, "latest"],
    )


def uint_result(value: str) -> int:
    return int(value, 16)


def address_result(value: str) -> str:
    return f"0x{value.removeprefix('0x')[-40:]}".lower()


def wait_receipt(url: str, transaction_hash: str, timeout_seconds: float = 45) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        receipt = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if receipt:
            if receipt.get("status") != "0x1":
                raise RuntimeError(f"fork transaction reverted: {transaction_hash}")
            return receipt
        time.sleep(0.1)
    raise TimeoutError(f"timed out waiting for fork transaction {transaction_hash}")


def free_port() -> int:
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return int(listener.getsockname()[1])


def find_anvil(repo: Path, configured: str | None) -> str:
    candidates = [configured, shutil.which("anvil")]
    if os.name == "nt":
        candidates.append(str(repo / ".tools" / "foundry" / "anvil.exe"))
    else:
        candidates.append(str(repo / ".tools" / "foundry" / "anvil"))
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return candidate
    raise FileNotFoundError("anvil was not found; install Foundry or pass --anvil")


def wait_ready(url: str, process: subprocess.Popen[bytes]) -> None:
    deadline = time.monotonic() + 45
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"anvil exited before readiness with code {process.returncode}")
        try:
            if uint_result(rpc(url, "eth_chainId", [])) == 8453:
                return
        except (RuntimeError, URLError):
            pass
        time.sleep(0.25)
    raise TimeoutError("anvil Base fork did not become ready")


def validate_bundle(bundle: dict[str, Any]) -> None:
    if bundle.get("schema_version") != "agent-bounties/autonomous-activation-bundle-v1":
        raise ValueError("unsupported activation bundle schema")
    if bundle.get("network") != "base-mainnet" or bundle.get("chain_id") != 8453:
        raise ValueError("activation bundle is not for Base mainnet")
    deployment = bundle["deployment"]
    if deployment.get("to") is not None or deployment.get("value_wei") != 0:
        raise ValueError("factory deployment must be a zero-value contract creation")
    calls = bundle["creation_batch"]["wallet_calls"]
    if len(calls) != len(bundle["bounties"]) + 1:
        raise ValueError("activation batch must contain one approval and one call per bounty")
    if bundle["creation_batch"]["total_initial_funding"] != "4000000":
        raise ValueError("activation bundle must remain capped at 4 USDC total")


def rehearse(repo: Path, bundle_path: Path, fork_url: str, anvil_path: str | None) -> dict[str, Any]:
    bundle = json.loads(bundle_path.read_text(encoding="utf-8"))
    validate_bundle(bundle)
    deployment = bundle["deployment"]
    creator = deployment["from"]
    port = free_port()
    local_url = f"http://127.0.0.1:{port}"
    anvil = find_anvil(repo, anvil_path)
    with tempfile.TemporaryDirectory(prefix="agent-bounties-activation-") as temp:
        stdout_path = Path(temp) / "anvil.stdout.log"
        stderr_path = Path(temp) / "anvil.stderr.log"
        creationflags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
        with stdout_path.open("wb") as stdout, stderr_path.open("wb") as stderr:
            process = subprocess.Popen(
                [anvil, "--fork-url", fork_url, "--port", str(port), "--chain-id", "8453", "--silent"],
                cwd=repo,
                stdout=stdout,
                stderr=stderr,
                creationflags=creationflags,
            )
            try:
                wait_ready(local_url, process)
                fork_block = uint_result(rpc(local_url, "eth_blockNumber", []))
                nonce = uint_result(rpc(local_url, "eth_getTransactionCount", [creator, "latest"]))
                if nonce != deployment["deployer_nonce"]:
                    raise RuntimeError(
                        f"deployer nonce drift: bundle={deployment['deployer_nonce']} fork={nonce}"
                    )
                if rpc(local_url, "eth_getCode", [deployment["expected_factory"], "latest"]) != "0x":
                    raise RuntimeError("predicted factory address already contains code")
                wallet_eth = uint_result(rpc(local_url, "eth_getBalance", [creator, "latest"]))
                wallet_usdc = uint_result(
                    call(
                        local_url,
                        deployment["settlement_token"],
                        "balanceOf(address)",
                        address_word(creator),
                    )
                )
                total_funding = int(bundle["creation_batch"]["total_initial_funding"])
                if wallet_usdc < total_funding:
                    raise RuntimeError(
                        f"forked wallet has {wallet_usdc} USDC minor units; {total_funding} required"
                    )

                rpc(local_url, "anvil_impersonateAccount", [creator])
                rpc(local_url, "anvil_setBalance", [creator, "0xde0b6b3a7640000"])
                deploy_hash = rpc(
                    local_url,
                    "eth_sendTransaction",
                    [{"from": creator, "data": deployment["data"], "value": "0x0"}],
                )
                deploy_receipt = wait_receipt(local_url, deploy_hash)
                observed_factory = deploy_receipt.get("contractAddress", "").lower()
                if observed_factory != deployment["expected_factory"].lower():
                    raise RuntimeError(
                        f"factory address mismatch: expected {deployment['expected_factory']} observed {observed_factory}"
                    )
                observed_implementation = address_result(
                    call(local_url, observed_factory, "implementation()")
                )
                if observed_implementation != deployment["expected_implementation"].lower():
                    raise RuntimeError("factory implementation address mismatch")
                observed_token = address_result(call(local_url, observed_factory, "settlementToken()"))
                if observed_token != deployment["settlement_token"].lower():
                    raise RuntimeError("factory settlement token mismatch")

                transaction_summaries = []
                for transaction in bundle["creation_batch"]["wallet_calls"]:
                    transaction_hash = rpc(
                        local_url,
                        "eth_sendTransaction",
                        [
                            {
                                "from": creator,
                                "to": transaction["to"],
                                "data": transaction["data"],
                                "value": "0x0",
                            }
                        ],
                    )
                    receipt = wait_receipt(local_url, transaction_hash)
                    transaction_summaries.append(
                        {
                            "function": transaction["function"],
                            "gas_used": uint_result(receipt["gasUsed"]),
                            "log_count": len(receipt.get("logs", [])),
                        }
                    )

                bounty_summaries = []
                for bounty in bundle["bounties"]:
                    contract = bounty["predicted_bounty_contract"]
                    canonical = uint_result(
                        call(local_url, observed_factory, "isCanonicalBounty(address)", address_word(contract))
                    )
                    bounty_id = call(local_url, contract, "bountyId()").lower()
                    funded = uint_result(call(local_url, contract, "fundedAmount()"))
                    target = uint_result(call(local_url, contract, "targetAmount()"))
                    status = uint_result(call(local_url, contract, "status()"))
                    token_balance = uint_result(
                        call(
                            local_url,
                            deployment["settlement_token"],
                            "balanceOf(address)",
                            address_word(contract),
                        )
                    )
                    if (
                        canonical != 1
                        or bounty_id != bounty["bounty_id"].lower()
                        or funded != 1_000_000
                        or target != 1_000_000
                        or status != 1
                        or token_balance != 1_000_000
                    ):
                        raise RuntimeError(f"issue {bounty['issue']} did not become fully funded and claimable")
                    bounty_summaries.append(
                        {
                            "issue": bounty["issue"],
                            "contract": contract,
                            "bounty_id": bounty_id,
                            "funded": funded,
                            "target": target,
                            "status": "claimable",
                        }
                    )
                final_wallet_usdc = uint_result(
                    call(
                        local_url,
                        deployment["settlement_token"],
                        "balanceOf(address)",
                        address_word(creator),
                    )
                )
                if final_wallet_usdc != wallet_usdc - total_funding:
                    raise RuntimeError("forked creator USDC conservation check failed")
                return {
                    "result": "pass",
                    "network": "base-mainnet-fork",
                    "fork_block": fork_block,
                    "deployer_nonce": nonce,
                    "factory": observed_factory,
                    "implementation": observed_implementation,
                    "deployment_gas_used": uint_result(deploy_receipt["gasUsed"]),
                    "transactions": transaction_summaries,
                    "bounties": bounty_summaries,
                    "creator_usdc_before": wallet_usdc,
                    "creator_usdc_after": final_wallet_usdc,
                    "creator_eth_before_wei": wallet_eth,
                    "evidence_boundary": "Fork transaction hashes are rehearsal evidence only and do not prove mainnet funding or payout.",
                }
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        default="deployments/base-mainnet-activation.json",
        help="unsigned activation bundle generated by the Rust CLI",
    )
    parser.add_argument(
        "--fork-url",
        default=os.environ.get("BASE_MAINNET_RPC_URL", "https://mainnet.base.org"),
    )
    parser.add_argument("--anvil", help="path to the anvil executable")
    parser.add_argument("--output", help="optional JSON evidence output path")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    try:
        result = rehearse(repo, repo / args.bundle, args.fork_url, args.anvil)
    except Exception as error:  # noqa: BLE001 - CLI must emit one actionable failure.
        print(json.dumps({"result": "fail", "error": str(error)}, indent=2), file=sys.stderr)
        return 1
    rendered = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        output = repo / args.output
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(rendered, encoding="utf-8")
        print(f"activation_rehearsal={output}")
    else:
        print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
