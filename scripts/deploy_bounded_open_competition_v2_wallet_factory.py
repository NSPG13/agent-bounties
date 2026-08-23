#!/usr/bin/env python3
"""Deploy and verify the exact recoverable Open Competition V2 reserve factory."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
from typing import Any

from eth_account import Account
from eth_utils import to_checksum_address

from _shared.evm import keccak256
from _shared.rpc import rpc
from deploy_open_competition_v2_beta3 import SignedRpc, require


SCHEMA = "agent-bounties/bounded-open-competition-v2-wallet-deployment-evidence-v1"
MANIFEST_SCHEMA = "agent-bounties/bounded-open-competition-v2-wallet-deployment-v1"
PROTOCOL = "agent-bounties/open-competition-v2-beta3"
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
HEX_DATA = re.compile(r"^0x(?:[0-9a-f]{2})+$")


def normalized_json_hash(value: dict[str, Any]) -> str:
    payload = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return "0x" + hashlib.sha256(payload).hexdigest()


def validate_manifest(manifest: dict[str, Any], release: dict[str, Any]) -> None:
    require(manifest.get("schema") == MANIFEST_SCHEMA, "reserve deployment schema mismatch")
    require(manifest.get("network") == "base-mainnet", "reserve deployment is not Base mainnet")
    require(manifest.get("chain_id") == 8453, "reserve deployment chain ID is not Base mainnet")
    require(manifest.get("contract_source_dirty") is False, "reserve contract source is dirty")
    require(release.get("protocol_version") == PROTOCOL, "release protocol mismatch")
    require(release.get("network") == "base-mainnet", "release is not Base mainnet")

    canonical = manifest.get("canonical", {})
    require(canonical.get("protocol_version") == PROTOCOL, "reserve protocol mismatch")
    require(
        canonical.get("competition_factory") == release.get("factory_contract"),
        "reserve factory is bound to another competition factory",
    )
    require(
        canonical.get("settlement_token") == release.get("settlement_token"),
        "reserve factory is bound to another settlement token",
    )
    require(
        canonical.get("release_hash") == release.get("release_hash"),
        "reserve factory is bound to another release",
    )
    require(HASH.fullmatch(str(canonical.get("release_hash") or "")) is not None, "release hash is invalid")
    require(
        HASH.fullmatch(str(release.get("factory_runtime_code_hash") or "")) is not None,
        "release factory runtime hash is invalid",
    )

    deterministic = manifest.get("deterministic_deployer", {})
    reserve = manifest.get("reserve_factory", {})
    for value, field in (
        (deterministic.get("address"), "deterministic deployer"),
        (reserve.get("address"), "reserve factory"),
        (reserve.get("implementation"), "reserve implementation"),
    ):
        require(ADDRESS.fullmatch(str(value or "")) is not None, f"{field} address is invalid")
    for value, field in (
        (deterministic.get("runtime_code_hash"), "deterministic deployer"),
        (reserve.get("runtime_code_hash"), "reserve factory"),
        (reserve.get("implementation_runtime_code_hash"), "reserve implementation"),
    ):
        require(HASH.fullmatch(str(value or "")) is not None, f"{field} runtime hash is invalid")
    require(
        HEX_DATA.fullmatch(str(reserve.get("deployment_transaction") or "")) is not None,
        "reserve deployment calldata is invalid",
    )
    require(
        str(reserve["deployment_transaction"]).startswith(str(reserve.get("salt") or "")),
        "reserve deployment calldata does not start with its CREATE2 salt",
    )


def initial_evidence(manifest: dict[str, Any], release: dict[str, Any], signer: str) -> dict[str, Any]:
    return {
        "schema_version": SCHEMA,
        "network": "base-mainnet",
        "chain_id": 8453,
        "protocol_version": PROTOCOL,
        "manifest_hash": normalized_json_hash(manifest),
        "release_hash": release["release_hash"],
        "competition_factory": release["factory_contract"],
        "deployer": signer.lower(),
        "reserve_factory": manifest["reserve_factory"]["address"],
        "reserve_implementation": manifest["reserve_factory"]["implementation"],
        "transaction": None,
        "safe_block": None,
        "runtime_hashes": {},
        "complete": False,
        "evidence_boundary": (
            "This proves exact reserve-factory deployment at a canonical safe block. It does not prove "
            "owner authorization, reserve funding, bounty activation, GMV, payout, or settlement."
        ),
    }


def load_evidence(
    output: Path, manifest: dict[str, Any], release: dict[str, Any], signer: str
) -> dict[str, Any]:
    expected = initial_evidence(manifest, release, signer)
    if not output.exists():
        return expected
    evidence = json.loads(output.read_text(encoding="utf-8"))
    for field in (
        "schema_version",
        "network",
        "chain_id",
        "protocol_version",
        "manifest_hash",
        "release_hash",
        "competition_factory",
        "deployer",
        "reserve_factory",
        "reserve_implementation",
    ):
        require(evidence.get(field) == expected[field], f"existing reserve evidence differs: {field}")
    return evidence


def write_evidence(output: Path, evidence: dict[str, Any]) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")


def send_create2(client: SignedRpc, manifest: dict[str, Any]) -> dict[str, Any]:
    deterministic = to_checksum_address(
        manifest["deterministic_deployer"]["address"]
    )
    data = manifest["reserve_factory"]["deployment_transaction"]
    nonce = client.pending_nonce()
    latest = rpc(client.url, "eth_getBlockByNumber", ["latest", False])
    base_fee = int(latest.get("baseFeePerGas", "0x0"), 16)
    try:
        priority = int(rpc(client.url, "eth_maxPriorityFeePerGas", []), 16)
    except RuntimeError:
        priority = 1_000_000
    priority = max(priority, 1_000_000)
    maximum = base_fee * 2 + priority
    estimate = {
        "from": client.signer.address,
        "to": deterministic,
        "data": data,
        "value": "0x0",
        "maxFeePerGas": hex(maximum),
        "maxPriorityFeePerGas": hex(priority),
    }
    gas = int(rpc(client.url, "eth_estimateGas", [estimate]), 16)
    require(gas <= 25_000_000, "reserve factory deployment gas exceeds the release cap")
    signed = client.signer.sign_transaction(
        {
            "chainId": 8453,
            "nonce": nonce,
            "to": deterministic,
            "value": 0,
            "data": data,
            "gas": gas * 6 // 5 + 50_000,
            "maxFeePerGas": maximum,
            "maxPriorityFeePerGas": priority,
            "type": 2,
        }
    )
    transaction_hash = client.broadcast(signed)
    return client.wait_receipt(transaction_hash)


def deploy(
    manifest: dict[str, Any],
    release: dict[str, Any],
    client: SignedRpc,
    output: Path,
) -> dict[str, Any]:
    validate_manifest(manifest, release)
    evidence = load_evidence(output, manifest, release, client.signer.address)
    deterministic = manifest["deterministic_deployer"]
    reserve = manifest["reserve_factory"]
    require(
        client.code_hash(deterministic["address"]) == deterministic["runtime_code_hash"],
        "universal CREATE2 deployer runtime mismatch",
    )
    require(
        client.code_hash(release["factory_contract"]) == release["factory_runtime_code_hash"],
        "canonical competition factory runtime mismatch",
    )

    factory_hash = client.code_hash(reserve["address"])
    implementation_hash = client.code_hash(reserve["implementation"])
    if factory_hash is None:
        require(implementation_hash is None, "reserve implementation address is unexpectedly occupied")
        receipt = send_create2(client, manifest)
        deployment_block = int(receipt["blockNumber"], 16)
        evidence["transaction"] = {
            "transaction_hash": receipt["transactionHash"].lower(),
            "block_number": deployment_block,
            "block_hash": receipt["blockHash"].lower(),
            "gas_used": int(receipt["gasUsed"], 16),
            "to": deterministic["address"],
            "value_wei": 0,
            "recovered_exact_deployment": False,
        }
        write_evidence(output, evidence)
    else:
        require(factory_hash == reserve["runtime_code_hash"], "reserve factory address has the wrong runtime")
        require(
            implementation_hash == reserve["implementation_runtime_code_hash"],
            "reserve implementation address has the wrong runtime",
        )
        latest = rpc(client.url, "eth_getBlockByNumber", ["latest", False])
        deployment_block = int(latest["number"], 16)
        if evidence.get("transaction") is None:
            evidence["transaction"] = {
                "transaction_hash": None,
                "block_number": deployment_block,
                "block_hash": latest["hash"].lower(),
                "gas_used": None,
                "to": deterministic["address"],
                "value_wei": 0,
                "recovered_exact_deployment": True,
            }
            write_evidence(output, evidence)

    safe = client.wait_safe(deployment_block)
    safe_tag = safe["number"]
    expected = {
        deterministic["address"]: deterministic["runtime_code_hash"],
        release["factory_contract"]: release["factory_runtime_code_hash"],
        reserve["address"]: reserve["runtime_code_hash"],
        reserve["implementation"]: reserve["implementation_runtime_code_hash"],
    }
    observed: dict[str, str] = {}
    for address, expected_hash in expected.items():
        actual = client.code_hash(address, safe_tag)
        require(actual == expected_hash, f"safe runtime hash mismatch: {address}")
        observed[address] = actual

    transaction = evidence["transaction"]
    if transaction["transaction_hash"] is not None:
        block = rpc(client.url, "eth_getBlockByNumber", [hex(transaction["block_number"]), False])
        require(block and block["hash"].lower() == transaction["block_hash"], "reserve deployment was reorged")
    evidence["safe_block"] = {
        "number": int(safe["number"], 16),
        "hash": safe["hash"].lower(),
        "timestamp": int(safe["timestamp"], 16),
    }
    evidence["runtime_hashes"] = observed
    evidence["complete"] = True
    write_evidence(output, evidence)
    return evidence


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url")
    parser.add_argument("--private-key-env", default="BASE_MAINNET_DEPLOYER_PRIVATE_KEY")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    private_key = os.environ.get(args.private_key_env, "")
    require(
        re.fullmatch(r"(?:0x)?[0-9a-fA-F]{64}", private_key) is not None,
        "deployment private key is missing or invalid",
    )
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    release = json.loads(args.release.read_text(encoding="utf-8"))
    signer = Account.from_key(private_key)
    evidence = deploy(
        manifest,
        release,
        SignedRpc(args.rpc_url, signer, args.shadow_rpc_url),
        args.output,
    )
    print(
        json.dumps(
            {
                "output": str(args.output),
                "reserve_factory": manifest["reserve_factory"]["address"],
                "safe_block": evidence["safe_block"]["number"],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
