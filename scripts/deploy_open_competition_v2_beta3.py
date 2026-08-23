#!/usr/bin/env python3
"""Broadcast an exact, prelaunch-approved Open Competition V2 Beta3 bundle."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import time
from typing import Any

from eth_account import Account

import build_open_competition_v2_beta3_release as release
from _shared.evm import keccak256
from _shared.rpc import rpc


SCHEMA = "agent-bounties/open-competition-v2-beta3-deployment-evidence-v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def validate_bundle(bundle: dict[str, Any], signer_address: str) -> None:
    require(
        bundle.get("schema_version")
        == "agent-bounties/open-competition-v2-beta3-release-bundle-v1",
        "release bundle schema mismatch",
    )
    require(bundle.get("network") == "base-mainnet", "bundle is not Base mainnet")
    require(bundle.get("chain_id") == 8453, "bundle chain ID is not Base mainnet")
    require(bundle.get("deployer", "").lower() == signer_address.lower(), "signer differs from bundle deployer")
    require(
        bundle.get("activation", {}).get("mainnet_signing_allowed") is True,
        "prelaunch gates do not authorize mainnet signing",
    )
    gates = bundle.get("release_gates", {})
    require(gates.get("prelaunch_complete") is True, "prelaunch gate summary is false")
    values = gates.get("gates", {})
    evidence = gates.get("evidence", {})
    for name in release.PRELAUNCH_GATE_NAMES:
        require(values.get(name) is True, f"prelaunch gate is false: {name}")
        require(isinstance(evidence.get(name), dict), f"prelaunch evidence is absent: {name}")
        require(
            evidence[name].get("subject_hash") == bundle["repository_subject"]["hash"],
            f"prelaunch evidence targets another release: {name}",
        )
    transactions = bundle.get("deployment_transactions")
    require(isinstance(transactions, list) and len(transactions) == 3, "exactly three deployments are required")
    expected = ("groth16_verifier", "plonk_verifier", "factory")
    for index, (transaction, component) in enumerate(zip(transactions, expected, strict=True)):
        require(transaction.get("component") == component, "deployment order changed")
        require(transaction.get("from_nonce") == transactions[0]["from_nonce"] + index, "deployment nonces are not contiguous")
        require(
            isinstance(transaction.get("data"), str)
            and re.fullmatch(r"0x[0-9a-f]+", transaction["data"]),
            f"invalid deployment calldata: {component}",
        )
        require(
            transaction.get("predicted_address") == bundle[component]["address"],
            f"predicted address mismatch: {component}",
        )


class SignedRpc:
    def __init__(self, url: str, signer: Any, shadow_url: str | None = None) -> None:
        self.url = url
        self.signer = signer
        self.broadcast_urls = tuple(
            dict.fromkeys([url, *([shadow_url] if shadow_url else [])])
        )
        for endpoint in self.broadcast_urls:
            require(
                int(rpc(endpoint, "eth_chainId", []), 16) == 8453,
                "RPC is not Base mainnet",
            )

    def receipt(self, transaction_hash: str) -> dict[str, Any] | None:
        for endpoint in self.broadcast_urls:
            try:
                receipt = rpc(endpoint, "eth_getTransactionReceipt", [transaction_hash])
            except RuntimeError:
                continue
            if receipt:
                require(
                    receipt.get("transactionHash", "").lower()
                    == transaction_hash.lower(),
                    "RPC returned a receipt for an unexpected transaction",
                )
                return receipt
        return None

    def transaction(self, transaction_hash: str) -> dict[str, Any] | None:
        for endpoint in self.broadcast_urls:
            try:
                transaction = rpc(endpoint, "eth_getTransactionByHash", [transaction_hash])
            except RuntimeError:
                continue
            if transaction:
                require(
                    transaction.get("hash", "").lower() == transaction_hash.lower(),
                    "RPC returned an unexpected pending transaction",
                )
                return transaction
        return None

    def broadcast(self, signed: Any) -> str:
        raw_transaction = "0x" + bytes(signed.raw_transaction).hex()
        expected_hash = "0x" + bytes(signed.hash).hex()
        submitted = False
        for endpoint in self.broadcast_urls:
            try:
                transaction_hash = rpc(
                    endpoint,
                    "eth_sendRawTransaction",
                    [raw_transaction],
                )
            except RuntimeError:
                continue
            require(
                transaction_hash.lower() == expected_hash.lower(),
                "RPC returned an unexpected transaction hash",
            )
            submitted = True
            break
        if (
            not submitted
            and self.receipt(expected_hash) is None
            and self.transaction(expected_hash) is None
        ):
            raise RuntimeError("raw deployment submission failed on every approved RPC")
        return expected_hash

    def code_hash(self, address: str, block: str = "latest") -> str | None:
        deadline = time.time() + 120
        last_error: RuntimeError | None = None
        while time.time() < deadline:
            try:
                raw = rpc(self.url, "eth_getCode", [address, block])
                return None if raw == "0x" else keccak256(bytes.fromhex(raw[2:]))
            except RuntimeError as error:
                last_error = error
                time.sleep(2)
        raise RuntimeError(f"runtime bytecode lookup failed: {last_error}") from last_error

    def pending_nonce(self) -> int:
        return int(
            rpc(self.url, "eth_getTransactionCount", [self.signer.address, "pending"]),
            16,
        )

    def send_creation(self, transaction: dict[str, Any]) -> dict[str, Any]:
        nonce = self.pending_nonce()
        require(nonce == transaction["from_nonce"], "deployer nonce moved; rebuild the exact release bundle")
        require(self.code_hash(transaction["predicted_address"]) is None, "predicted deployment address is occupied")
        latest = rpc(self.url, "eth_getBlockByNumber", ["latest", False])
        base_fee = int(latest.get("baseFeePerGas", "0x0"), 16)
        try:
            priority = int(rpc(self.url, "eth_maxPriorityFeePerGas", []), 16)
        except RuntimeError:
            priority = 1_000_000
        priority = max(priority, 1_000_000)
        maximum = base_fee * 2 + priority
        estimate = {
            "from": self.signer.address,
            "data": transaction["data"],
            "value": "0x0",
            "maxFeePerGas": hex(maximum),
            "maxPriorityFeePerGas": hex(priority),
        }
        gas = int(rpc(self.url, "eth_estimateGas", [estimate]), 16)
        require(gas <= 25_000_000, "deployment gas exceeds the release cap")
        signed = self.signer.sign_transaction(
            {
                "chainId": 8453,
                "nonce": nonce,
                "value": 0,
                "data": transaction["data"],
                "gas": gas * 6 // 5 + 50_000,
                "maxFeePerGas": maximum,
                "maxPriorityFeePerGas": priority,
                "type": 2,
            }
        )
        transaction_hash = self.broadcast(signed)
        return self.wait_receipt(transaction_hash)

    def wait_receipt(self, transaction_hash: str, timeout_seconds: int = 600) -> dict[str, Any]:
        deadline = time.time() + timeout_seconds
        while time.time() < deadline:
            receipt = self.receipt(transaction_hash)
            if receipt:
                require(int(receipt["status"], 16) == 1, f"deployment reverted: {transaction_hash}")
                return receipt
            time.sleep(2)
        raise RuntimeError(f"deployment receipt timed out: {transaction_hash}")

    def wait_safe(self, block_number: int, timeout_seconds: int = 1800) -> dict[str, Any]:
        deadline = time.time() + timeout_seconds
        last_error: RuntimeError | None = None
        while time.time() < deadline:
            try:
                safe = rpc(self.url, "eth_getBlockByNumber", ["safe", False])
            except RuntimeError as error:
                last_error = error
                time.sleep(5)
                continue
            if safe and int(safe["number"], 16) >= block_number:
                return safe
            time.sleep(5)
        detail = f": {last_error}" if last_error is not None else ""
        raise RuntimeError(f"deployment did not reach a Base safe block{detail}")

    def require_safe_code_hashes(
        self, expected_hashes: dict[str, str], block_number: int
    ) -> None:
        safe = self.wait_safe(block_number)
        for address, expected_hash in expected_hashes.items():
            actual = self.code_hash(address, safe["number"])
            require(actual == expected_hash, f"safe runtime hash mismatch: {address}")


def expected_runtime_hashes(bundle: dict[str, Any]) -> dict[str, str]:
    return {
        name: bundle[name]["runtime_code_hash"]
        for name in (
            "groth16_verifier",
            "plonk_verifier",
            "factory",
            "groth16_adapter",
            "plonk_adapter",
            "implementation",
        )
    }


def initial_evidence(bundle: dict[str, Any]) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA,
        "protocol_version": bundle["protocol_version"],
        "network": bundle["network"],
        "source_commit": bundle["source_commit"],
        "repository_subject_hash": bundle["repository_subject"]["hash"],
        "deployer": bundle["deployer"],
        "transactions": [],
        "safe_block": None,
        "runtime_hashes": {},
        "runtime_manifest": None,
        "complete": False,
        "evidence_boundary": "This proves exact immutable component deployment. It does not prove a canary, payment, indexer agreement, or public activation.",
    }


def load_evidence(bundle: dict[str, Any], output: Path) -> dict[str, Any]:
    if not output.exists():
        return initial_evidence(bundle)
    evidence = json.loads(output.read_text(encoding="utf-8"))
    require(evidence.get("schema_version") == SCHEMA, "existing deployment evidence schema mismatch")
    require(
        evidence.get("repository_subject_hash") == bundle["repository_subject"]["hash"],
        "existing deployment evidence targets another release",
    )
    require(evidence.get("deployer", "").lower() == bundle["deployer"].lower(), "existing deployment evidence has another deployer")
    return evidence


def write_evidence(output: Path, evidence: dict[str, Any]) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")


def recovered_component_record(
    bundle: dict[str, Any], transaction: dict[str, Any], client: SignedRpc
) -> dict[str, Any]:
    component = transaction["component"]
    expected_hash = bundle[component]["runtime_code_hash"]
    deadline = time.time() + 600
    while time.time() < deadline:
        actual = client.code_hash(transaction["predicted_address"])
        if actual is not None:
            require(actual == expected_hash, f"occupied predicted address has the wrong runtime: {component}")
            latest = rpc(client.url, "eth_getBlockByNumber", ["latest", False])
            return {
                "component": component,
                "transaction_hash": None,
                "block_number": int(latest["number"], 16),
                "block_hash": latest["hash"].lower(),
                "contract_address": transaction["predicted_address"],
                "gas_used": None,
                "recovered_exact_deployment": True,
            }
        require(
            client.pending_nonce() > transaction["from_nonce"],
            f"missing deployment cannot be recovered: {component}",
        )
        time.sleep(2)
    raise RuntimeError(f"timed out recovering exact deployment: {component}")


def reconcile_canonical_receipts(
    evidence: dict[str, Any], client: SignedRpc
) -> int:
    for item in evidence["transactions"]:
        transaction_hash = item["transaction_hash"]
        if transaction_hash is None:
            continue
        receipt = client.receipt(transaction_hash)
        require(receipt is not None, f"canonical deployment receipt is absent: {transaction_hash}")
        require(
            int(receipt["status"], 16) == 1,
            f"canonical deployment reverted: {transaction_hash}",
        )
        observed_address = str(receipt.get("contractAddress", "")).lower()
        require(
            observed_address == item["contract_address"],
            "canonical deployment receipt has another contract address",
        )
        item["block_number"] = int(receipt["blockNumber"], 16)
        item["block_hash"] = receipt["blockHash"].lower()
        item["gas_used"] = int(receipt["gasUsed"], 16)
    return max(item["block_number"] for item in evidence["transactions"])


def deploy(bundle: dict[str, Any], client: SignedRpc, output: Path) -> dict[str, Any]:
    evidence = load_evidence(bundle, output)
    completed = {item["component"]: item for item in evidence["transactions"]}
    for transaction in bundle["deployment_transactions"]:
        component = transaction["component"]
        if component in completed:
            require(
                completed[component]["contract_address"] == transaction["predicted_address"],
                f"existing evidence address differs: {component}",
            )
            continue
        if component == "factory":
            dependency_blocks = [
                int(item["block_number"])
                for item in evidence["transactions"]
                if item["component"] in {"groth16_verifier", "plonk_verifier"}
            ]
            require(len(dependency_blocks) == 2, "factory verifier evidence is incomplete")
            client.require_safe_code_hashes(
                {
                    bundle["groth16_verifier"]["address"]: bundle["groth16_verifier"][
                        "runtime_code_hash"
                    ],
                    bundle["plonk_verifier"]["address"]: bundle["plonk_verifier"][
                        "runtime_code_hash"
                    ],
                },
                max(dependency_blocks),
            )
        if client.code_hash(transaction["predicted_address"]) is not None or client.pending_nonce() > transaction["from_nonce"]:
            record = recovered_component_record(bundle, transaction, client)
        else:
            receipt = client.send_creation(transaction)
            observed_address = str(receipt.get("contractAddress", "")).lower()
            require(observed_address == transaction["predicted_address"], "receipt contract address differs from release bundle")
            record = {
                "component": transaction["component"],
                "transaction_hash": receipt["transactionHash"].lower(),
                "block_number": int(receipt["blockNumber"], 16),
                "block_hash": receipt["blockHash"].lower(),
                "contract_address": observed_address,
                "gas_used": int(receipt["gasUsed"], 16),
            }
        evidence["transactions"].append(record)
        write_evidence(output, evidence)
    deployment_block = reconcile_canonical_receipts(evidence, client)
    client.wait_safe(deployment_block)
    deployment_block = reconcile_canonical_receipts(evidence, client)
    safe = client.wait_safe(deployment_block)
    write_evidence(output, evidence)
    for item in evidence["transactions"]:
        if item["transaction_hash"] is None:
            continue
        block = rpc(client.url, "eth_getBlockByNumber", [hex(item["block_number"]), False])
        require(block and block["hash"].lower() == item["block_hash"], "deployment receipt was reorged")
    observed: dict[str, str] = {}
    for name, expected_hash in expected_runtime_hashes(bundle).items():
        actual = client.code_hash(bundle[name]["address"], "safe")
        require(actual == expected_hash, f"safe runtime hash mismatch: {name}")
        observed[name] = actual
    evidence["safe_block"] = {
        "number": int(safe["number"], 16),
        "hash": safe["hash"].lower(),
        "timestamp": int(safe["timestamp"], 16),
    }
    evidence["runtime_hashes"] = observed
    evidence["runtime_manifest"] = release.runtime_manifest(bundle, deployment_block)
    evidence["complete"] = True
    write_evidence(output, evidence)
    return evidence


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url")
    parser.add_argument("--private-key-env", default="BASE_MAINNET_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    private_key = os.environ.get(args.private_key_env, "")
    require(re.fullmatch(r"(?:0x)?[0-9a-fA-F]{64}", private_key) is not None, "deployment private key is missing or invalid")
    signer = Account.from_key(private_key)
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    validate_bundle(bundle, signer.address)
    evidence = deploy(
        bundle,
        SignedRpc(args.rpc_url, signer, args.shadow_rpc_url),
        args.output,
    )
    print(json.dumps({"output": str(args.output), "factory": bundle["factory"]["address"], "safe_block": evidence["safe_block"]["number"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
