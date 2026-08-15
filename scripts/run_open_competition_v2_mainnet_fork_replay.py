#!/usr/bin/env python3
"""Replay an exact V2 Beta2 mainnet deployment bundle on an isolated Anvil fork."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import socket
import subprocess
import time
from typing import Any

from _shared.evm import keccak256, keccak_bytes
from _shared.rpc import rpc

import open_competition_v2_proof_rehearsal


ROOT = Path(__file__).resolve().parents[1]
EXPECTED_SCHEMA = "agent-bounties/open-competition-v2-beta2-release-bundle-v1"
OUTPUT_SCHEMA = "agent-bounties/open-competition-v2-beta2-mainnet-fork-replay-v1"


def selector(signature: str) -> str:
    return "0x" + keccak_bytes(signature.encode())[:4].hex()


def address_result(value: str) -> str:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 32:
        raise ValueError("address getter returned a non-word value")
    return "0x" + raw[12:].hex()


def bool_result(value: str) -> bool:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 32 or int.from_bytes(raw, "big") not in (0, 1):
        raise ValueError("bool getter returned an invalid ABI word")
    return bool(int.from_bytes(raw, "big"))


def free_port() -> int:
    with socket.socket() as server:
        server.bind(("127.0.0.1", 0))
        return int(server.getsockname()[1])


def anvil_binary() -> str:
    candidate = shutil.which("anvil")
    if candidate:
        return candidate
    local = ROOT / ".tools" / "foundry" / ("anvil.exe" if __import__("os").name == "nt" else "anvil")
    if local.exists():
        return str(local)
    raise RuntimeError("anvil is required for the mainnet-fork replay")


def wait_for_rpc(url: str, process: subprocess.Popen[Any]) -> None:
    for _ in range(120):
        if process.poll() is not None:
            raise RuntimeError("Anvil exited before the fork became ready")
        try:
            if rpc(url, "eth_chainId", []) == "0x2105":
                return
        except RuntimeError:
            pass
        time.sleep(0.25)
    raise RuntimeError("Anvil fork did not become ready")


def wait_receipt(url: str, transaction_hash: str) -> dict[str, Any]:
    for _ in range(120):
        value = rpc(url, "eth_getTransactionReceipt", [transaction_hash])
        if value:
            if int(value["status"], 16) != 1:
                raise RuntimeError("exact deployment transaction reverted")
            return value
        time.sleep(0.25)
    raise RuntimeError("exact deployment receipt timed out")


def runtime(url: str, address: str) -> tuple[str, int]:
    code = rpc(url, "eth_getCode", [address, "latest"])
    raw = bytes.fromhex(code.removeprefix("0x"))
    if not raw:
        raise RuntimeError(f"runtime is missing at {address}")
    return keccak256(raw), len(raw)


def call(url: str, to: str, signature: str) -> str:
    return rpc(url, "eth_call", [{"to": to, "data": selector(signature)}, "latest"])


def validate_bundle(bundle: dict[str, Any]) -> None:
    if bundle.get("schema_version") != EXPECTED_SCHEMA:
        raise ValueError("release bundle schema mismatch")
    if bundle.get("network") != "base-mainnet" or bundle.get("chain_id") != 8453:
        raise ValueError("fork replay requires a Base mainnet release bundle")
    if bundle.get("activation", {}).get("mainnet_signing_allowed") is True:
        raise ValueError("an ungraduated Beta2 bundle must not authorize mainnet signing")
    if bundle.get("deployment_state") != "blocked":
        raise ValueError("current Beta2 replay expects a fail-closed release bundle")


def replay(
    bundle: dict[str, Any], upstream_rpc: str, output: Path, *,
    run_proof_rehearsal: bool = False,
    prepare_proof_fixtures: Path | None = None,
    prepared_proof_dir: Path | None = None,
    proof_evidence_dir: Path | None = None,
) -> dict[str, Any]:
    validate_bundle(bundle)
    port = free_port()
    local_rpc = f"http://127.0.0.1:{port}"
    log_path = output.with_suffix(".anvil.log")
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log = log_path.open("w", encoding="utf-8")
    command = [
        anvil_binary(),
        "--fork-url",
        upstream_rpc,
        "--fork-block-number",
        str(bundle["preflight_safe_block"]["number"]),
        "--chain-id",
        "8453",
        "--port",
        str(port),
        "--silent",
    ]
    process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
    try:
        wait_for_rpc(local_rpc, process)
        deployer = bundle["deployer"]
        for item in bundle["deployment_transactions"]:
            if rpc(local_rpc, "eth_getCode", [item["predicted_address"], "latest"]) != "0x":
                raise RuntimeError(
                    f"predicted {item['component']} address is occupied at the pinned fork block"
                )
        rpc(local_rpc, "anvil_setBalance", [deployer, hex(10**20)])
        rpc(local_rpc, "anvil_impersonateAccount", [deployer])
        receipts: dict[str, dict[str, Any]] = {}
        for item in bundle["deployment_transactions"]:
            transaction = {
                "from": deployer,
                "data": item["data"],
                "value": "0x0",
            }
            gas = int(rpc(local_rpc, "eth_estimateGas", [transaction]), 16)
            transaction["gas"] = hex(gas * 5 // 4 + 50_000)
            receipt = wait_receipt(
                local_rpc, rpc(local_rpc, "eth_sendTransaction", [transaction])
            )
            deployed = str(receipt.get("contractAddress", "")).lower()
            if deployed != item["predicted_address"]:
                raise RuntimeError(
                    f"{item['component']} address differs from the release bundle"
                )
            receipts[item["component"]] = receipt
        rpc(local_rpc, "anvil_stopImpersonatingAccount", [deployer])

        components: dict[str, Any] = {}
        for key in (
            "groth16_verifier",
            "plonk_verifier",
            "factory",
            "groth16_adapter",
            "plonk_adapter",
            "implementation",
        ):
            expected = bundle[key]
            observed_hash, observed_bytes = runtime(local_rpc, expected["address"])
            if observed_hash != expected["runtime_code_hash"] or observed_bytes != expected["runtime_code_bytes"]:
                raise RuntimeError(f"{key} runtime differs from the release bundle")
            components[key] = {
                "address": expected["address"],
                "runtime_code_hash": observed_hash,
                "runtime_code_bytes": observed_bytes,
            }

        factory = bundle["factory"]["address"]
        getters = {
            "settlement_token": ("settlementToken()", bundle["settlement_token"]),
            "groth16_adapter": ("groth16Adapter()", bundle["groth16_adapter"]["address"]),
            "plonk_adapter": ("plonkAdapter()", bundle["plonk_adapter"]["address"]),
            "implementation": ("implementation()", bundle["implementation"]["address"]),
        }
        observed_getters: dict[str, str] = {}
        for name, (signature, expected) in getters.items():
            observed = address_result(call(local_rpc, factory, signature))
            if observed != expected:
                raise RuntimeError(f"factory {name} getter differs from the release bundle")
            observed_getters[name] = observed
        for key in ("groth16_adapter", "plonk_adapter"):
            if not bool_result(call(local_rpc, bundle[key]["address"], "verifierAvailable()")):
                raise RuntimeError(f"{key} rejected its pinned immutable verifier on the fork")

        proof_rehearsal = None
        if prepare_proof_fixtures is not None:
            context = open_competition_v2_proof_rehearsal.prepare_context(
                local_rpc, bundle, prepare_proof_fixtures
            )
            proof_rehearsal = {
                "prepared": True,
                "context_hash": context["context_hash"],
                "proofs": context["proofs"],
            }
        elif run_proof_rehearsal:
            proof_rehearsal = open_competition_v2_proof_rehearsal.run(
                local_rpc,
                bundle,
                output.parent / "open-competition-v2-proof-work",
                prepared_dir=prepared_proof_dir,
                proof_evidence_dir=proof_evidence_dir,
            )

        result = {
            "schema_version": OUTPUT_SCHEMA,
            "passed": True,
            "broadcast": False,
            "source_commit": bundle["source_commit"],
            "source_tree_hash": bundle["source_tree_hash"],
            "fork_block": bundle["preflight_safe_block"],
            "deployment_transactions": {
                name: {
                    "transaction_hash": receipt["transactionHash"],
                    "block_number": int(receipt["blockNumber"], 16),
                }
                for name, receipt in receipts.items()
            },
            "components": components,
            "factory_getters": observed_getters,
            "pinned_sp1_verifiers_available": True,
            "proof_rehearsal": proof_rehearsal,
            "release_remains_blocked": True,
            "evidence_boundary": "This proves exact deployment and immutable configuration on an isolated Base mainnet fork. It is not a live deployment, proof canary, settlement, payment, independent review, or public activation.",
        }
        output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        return result
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
        log.close()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--rpc-url", default="https://mainnet.base.org")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--proof-rehearsal", action="store_true")
    parser.add_argument("--prepare-proof-fixtures", type=Path)
    parser.add_argument("--prepared-proof-dir", type=Path)
    parser.add_argument("--proof-evidence-dir", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.prepare_proof_fixtures and args.proof_rehearsal:
        raise SystemExit("--prepare-proof-fixtures and --proof-rehearsal are mutually exclusive")
    if bool(args.prepared_proof_dir) != bool(args.proof_evidence_dir):
        raise SystemExit("--prepared-proof-dir and --proof-evidence-dir must be supplied together")
    if args.prepared_proof_dir and not args.proof_rehearsal:
        raise SystemExit("external proof artifacts require --proof-rehearsal")
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    result = replay(
        bundle,
        args.rpc_url,
        args.output,
        run_proof_rehearsal=args.proof_rehearsal,
        prepare_proof_fixtures=args.prepare_proof_fixtures,
        prepared_proof_dir=args.prepared_proof_dir,
        proof_evidence_dir=args.proof_evidence_dir,
    )
    print(json.dumps({"output": str(args.output), "passed": result["passed"], "factory": result["components"]["factory"]["address"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
