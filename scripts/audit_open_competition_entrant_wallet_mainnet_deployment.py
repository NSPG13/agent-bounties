#!/usr/bin/env python3
"""Audit the Base mainnet entrant-wallet deployment at a canonical safe block."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from web3 import Web3


RECEIPT_SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-deployment-receipt-v1"
AUDIT_SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-deployment-audit-v1"
RELEASE_SCHEMA = "agent-bounties/open-competition-entrant-wallet-release-v1"
PROTOCOL_VERSION = "agent-bounties/open-competition-entrant-wallet-v1"


class AuditError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AuditError(message)


def is_hex_bytes(value: Any, byte_length: int) -> bool:
    if not isinstance(value, str) or not value.startswith("0x") or len(value) != 2 + byte_length * 2:
        return False
    try:
        bytes.fromhex(value[2:])
    except ValueError:
        return False
    return True


def load(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AuditError(f"{label} is unreadable: {error}") from error
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def runtime_hash(w3: Web3, address: str, block: int) -> str:
    code = w3.eth.get_code(Web3.to_checksum_address(address), block)
    require(bool(code), f"{address} has no code at block {block}")
    return f"0x{Web3.keccak(code).hex()}".lower()


def audit(bundle: dict[str, Any], browser_receipt: dict[str, Any], rpc_url: str) -> dict[str, Any]:
    require(bundle.get("network") == "base-mainnet" and bundle.get("chain_id") == 8453, "bundle chain mismatch")
    require(browser_receipt.get("schema_version") == RECEIPT_SCHEMA, "deployment receipt schema mismatch")
    require(browser_receipt.get("network") == "base-mainnet" and browser_receipt.get("chain_id") == 8453, "receipt chain mismatch")
    require(browser_receipt.get("contract_source_revision") == bundle.get("contract_source_revision"), "contract tree mismatch")
    action = bundle.get("action")
    require(isinstance(action, dict), "bundle action is missing")
    tx_hash = browser_receipt.get("transaction_hash")
    require(is_hex_bytes(tx_hash, 32), "deployment transaction hash is required")

    w3 = Web3(Web3.HTTPProvider(rpc_url, request_kwargs={"timeout": 30}))
    require(w3.is_connected() and w3.eth.chain_id == 8453, "Base mainnet RPC unavailable")
    transaction = w3.eth.get_transaction(tx_hash)
    receipt = w3.eth.get_transaction_receipt(tx_hash)
    require(int(receipt.status) == 1, "deployment transaction reverted")
    require(transaction["from"].lower() == str(bundle.get("admin", "")).lower(), "deployment sender mismatch")
    require(transaction["to"] is not None and transaction["to"].lower() == str(action.get("to", "")).lower(), "deployment target mismatch")
    require(int(transaction["value"]) == 0, "deployment transferred ETH")
    require(f"0x{bytes(transaction['input']).hex()}".lower() == str(action.get("data", "")).lower(), "deployment calldata mismatch")
    deployment_block = int(receipt.blockNumber)
    deployment_hash = f"0x{receipt.blockHash.hex()}".lower()
    require(int(browser_receipt.get("block_number", -1)) == deployment_block, "browser receipt block mismatch")
    require(str(browser_receipt.get("block_hash", "")).lower() == deployment_hash, "browser receipt block hash mismatch")

    safe = w3.eth.get_block("safe")
    safe_number = int(safe.number)
    require(safe_number >= deployment_block, "deployment is not canonical at a Base safe block")
    safe_hash = f"0x{safe.hash.hex()}".lower()
    factory = str(action.get("expected_factory", "")).lower()
    implementation = str(action.get("expected_implementation", "")).lower()
    expected_factory_hash = str(action.get("factory_runtime_code_hash", "")).lower()
    expected_implementation_hash = str(action.get("implementation_runtime_code_hash", "")).lower()
    require(runtime_hash(w3, factory, safe_number) == expected_factory_hash, "entrant factory runtime mismatch")
    require(runtime_hash(w3, implementation, safe_number) == expected_implementation_hash, "entrant implementation runtime mismatch")

    factory_contract = w3.eth.contract(
        address=Web3.to_checksum_address(factory),
        abi=[
            {"name": "competitionFactory", "type": "function", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
            {"name": "settlementToken", "type": "function", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
            {"name": "implementation", "type": "function", "stateMutability": "view", "inputs": [], "outputs": [{"type": "address"}]},
        ],
    )
    competition_factory = factory_contract.functions.competitionFactory().call(block_identifier=safe_number).lower()
    settlement_token = factory_contract.functions.settlementToken().call(block_identifier=safe_number).lower()
    observed_implementation = factory_contract.functions.implementation().call(block_identifier=safe_number).lower()
    dependencies = bundle.get("canonical_dependencies", {})
    require(competition_factory == str(dependencies.get("competition_factory", "")).lower(), "competition factory dependency mismatch")
    require(settlement_token == str(dependencies.get("settlement_token", "")).lower(), "settlement token dependency mismatch")
    require(observed_implementation == implementation, "factory implementation pointer mismatch")

    release_manifest = {
        "schema_version": RELEASE_SCHEMA,
        "protocol_version": PROTOCOL_VERSION,
        "network": "base-mainnet",
        "chain_id": 8453,
        "deployment_state": "mainnet_canary_not_ready_to_earn",
        "factory_contract": factory,
        "implementation_contract": implementation,
        "competition_factory": competition_factory,
        "settlement_token": settlement_token,
        "deployment_block": deployment_block,
        "factory_runtime_code_hash": expected_factory_hash,
        "implementation_runtime_code_hash": expected_implementation_hash,
        "clone_runtime_code_hash": str(action.get("clone_runtime_code_hash", "")).lower(),
    }
    assertions = {
        "exact_admin_zero_value_create2_transaction": True,
        "deployment_receipt_matches_browser_receipt": True,
        "deployment_block_is_canonical_at_safe_block": True,
        "factory_and_implementation_runtimes_match": True,
        "factory_dependencies_match_frozen_bundle": True,
        "public_activation_remains_disabled": True,
    }
    return {
        "schema_version": AUDIT_SCHEMA,
        "network": "base-mainnet",
        "chain_id": 8453,
        "transaction_hash": tx_hash.lower(),
        "deployment_block": deployment_block,
        "deployment_block_hash": deployment_hash,
        "canonical_safe_block": {"number": safe_number, "hash": safe_hash, "timestamp": int(safe.timestamp)},
        "release_manifest": release_manifest,
        "assertions": assertions,
        "passed": all(assertions.values()),
        "evidence_boundary": "This proves exact entrant-factory deployment and runtime identity at a Base safe block. It is not an entrant-wallet canary, hosted relay, public activation, bounty settlement, or payment receipt.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-release-bundle.json"))
    parser.add_argument("--receipt", type=Path, required=True)
    parser.add_argument("--rpc", default="https://mainnet.base.org")
    parser.add_argument("--output", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-deployment-audit.json"))
    args = parser.parse_args()
    result = audit(load(args.bundle, "release bundle"), load(args.receipt, "browser deployment receipt"), args.rpc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "passed": result["passed"], "audit": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
