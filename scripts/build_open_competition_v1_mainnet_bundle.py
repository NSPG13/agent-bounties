#!/usr/bin/env python3
"""Build the unsigned, bytecode-frozen Base mainnet Open Competition V1 bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from _shared.evm import address_bytes, address_word, artifact_hex, create_address, keccak256


CHAIN_ID = 8_453
CHAIN_ID_HEX = "0x2105"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"
VERIFIER_RUNTIME_HASH = "0xbaa3a8305c4b65d0dc20131d0ef207fdaf4763f345393a831370cd04077df9b3"
DIFFICULTY_BITS = 16
DEFAULT_RPC = "https://mainnet.base.org"
MIN_DEPLOYER_ETH_WEI = 100_000_000_000_000
CANARY_SOLVER_REWARD = 1_000_000
CANARY_VERIFIER_REWARD = 100_000
CANARY_INITIAL_FUNDING = CANARY_SOLVER_REWARD + CANARY_VERIFIER_REWARD
CANARY_SOLVER_BOND = CANARY_VERIFIER_REWARD
MIN_CANARY_USDC = CANARY_INITIAL_FUNDING + CANARY_SOLVER_BOND
CANARY_BENCHMARK_PREIMAGE = "leading-zero-work-v1/difficulty-16/canary-benchmark-v1"
CANARY_EVIDENCE_SCHEMA_PREIMAGE = "agent-bounties/leading-zero-work-evidence-v1"
CANARY_BENCHMARK_HASH = keccak256(CANARY_BENCHMARK_PREIMAGE.encode())
CANARY_EVIDENCE_SCHEMA_HASH = keccak256(CANARY_EVIDENCE_SCHEMA_PREIMAGE.encode())


def verifier_profile() -> dict[str, Any]:
    return {
        "profile_id": "leading-zero-work-v1-difficulty-16-mainnet-canary",
        "verifier_address": VERIFIER,
        "difficulty_bits": DIFFICULTY_BITS,
        "runtime_code_hash": VERIFIER_RUNTIME_HASH,
        "benchmark_preimage": CANARY_BENCHMARK_PREIMAGE,
        "benchmark_hash": CANARY_BENCHMARK_HASH,
        "evidence_schema_preimage": CANARY_EVIDENCE_SCHEMA_PREIMAGE,
        "evidence_schema_hash": CANARY_EVIDENCE_SCHEMA_HASH,
        "usage": "protocol_canary_only",
        "public_inventory_eligible": False,
    }


def normalized_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def patched_runtime(artifact: dict[str, Any], values: list[bytes], name: str) -> bytes:
    deployed = artifact.get("deployedBytecode")
    runtime = bytearray(artifact_hex(deployed, f"{name}.deployedBytecode"))
    references = deployed.get("immutableReferences") if isinstance(deployed, dict) else None
    if not isinstance(references, dict) or len(references) != len(values):
        raise ValueError(f"{name} immutable reference count changed")
    for value, (_, locations) in zip(values, sorted(references.items(), key=lambda item: int(item[0]))):
        if len(value) != 32 or not isinstance(locations, list) or not locations:
            raise ValueError(f"{name} immutable reference is invalid")
        for location in locations:
            start = int(location["start"])
            length = int(location["length"])
            if length != 32 or start < 0 or start + length > len(runtime):
                raise ValueError(f"{name} immutable reference is out of bounds")
            runtime[start : start + length] = value
    return bytes(runtime)


def rpc(url: str, method: str, params: list[Any]) -> Any:
    payload = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = Request(
        url,
        data=payload,
        headers={"content-type": "application/json", "user-agent": "agent-bounties-open-competition/1"},
    )
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except (OSError, URLError) as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")


def balance_of_data(address: str) -> str:
    return f"0x70a08231{address_word(address).hex()}"


def difficulty_bits_data() -> str:
    return "0x249379ad"


def deployment_action(
    *, nonce: int, data: bytes, expected_contract: str, runtime: bytes, implementation: str,
    implementation_runtime: bytes
) -> dict[str, Any]:
    return {
        "name": "deploy_open_competition_factory_v1",
        "from": ADMIN,
        "from_nonce": nonce,
        "to": None,
        "value_wei": 0,
        "data": f"0x{data.hex()}",
        "creation_code_hash": keccak256(data),
        "expected_contract": expected_contract.lower(),
        "expected_runtime_code": f"0x{runtime.hex()}",
        "runtime_code_hash": keccak256(runtime),
        "runtime_code_bytes": len(runtime),
        "expected_implementation": implementation.lower(),
        "expected_implementation_runtime_code": f"0x{implementation_runtime.hex()}",
        "implementation_runtime_code_hash": keccak256(implementation_runtime),
        "implementation_runtime_code_bytes": len(implementation_runtime),
    }


def build_bundle(args: argparse.Namespace) -> dict[str, Any]:
    repo = Path(__file__).resolve().parents[1]
    deployer = args.deployer.lower()
    address_bytes(deployer)
    if deployer != ADMIN:
        raise ValueError(f"deployer must be the frozen admin wallet {ADMIN}")
    if not re.fullmatch(r"[0-9a-f]{40}", args.source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", args.preflight_block_hash):
        raise ValueError("preflight block hash must be bytes32 hex")

    block_tag = hex(args.preflight_block_number)
    if args.offline:
        if args.preflight_deployer_eth_wei is None or args.preflight_deployer_usdc_base_units is None:
            raise ValueError("offline generation requires pinned deployer ETH and USDC balances")
        deployer_eth = args.preflight_deployer_eth_wei
        deployer_usdc = args.preflight_deployer_usdc_base_units
    else:
        block = rpc(args.rpc_url, "eth_getBlockByNumber", [block_tag, False])
        if not block or block.get("hash", "").lower() != args.preflight_block_hash.lower():
            raise RuntimeError("pinned Base mainnet block hash mismatch")
        if rpc(args.rpc_url, "eth_chainId", []) != CHAIN_ID_HEX:
            raise RuntimeError("RPC is not Base mainnet")
        if rpc(args.rpc_url, "eth_getCode", [USDC, block_tag]) == "0x":
            raise RuntimeError("native Base mainnet USDC code is unavailable")
        verifier_runtime = bytes.fromhex(rpc(args.rpc_url, "eth_getCode", [VERIFIER, block_tag])[2:])
        if keccak256(verifier_runtime) != VERIFIER_RUNTIME_HASH:
            raise RuntimeError("approved mainnet verifier runtime hash mismatch")
        observed_difficulty = int(
            rpc(args.rpc_url, "eth_call", [{"to": VERIFIER, "data": difficulty_bits_data()}, block_tag]), 16
        )
        if observed_difficulty != DIFFICULTY_BITS:
            raise RuntimeError("approved mainnet verifier difficulty mismatch")
        observed_nonce = int(rpc(args.rpc_url, "eth_getTransactionCount", [deployer, block_tag]), 16)
        if observed_nonce != args.deployer_nonce:
            raise RuntimeError(
                f"deployer nonce mismatch at pinned block: expected {args.deployer_nonce}, got {observed_nonce}"
            )
        deployer_eth = int(rpc(args.rpc_url, "eth_getBalance", [deployer, block_tag]), 16)
        deployer_usdc = int(
            rpc(args.rpc_url, "eth_call", [{"to": USDC, "data": balance_of_data(deployer)}, block_tag]), 16
        )

    if deployer_eth < MIN_DEPLOYER_ETH_WEI:
        raise RuntimeError(f"admin wallet needs at least {MIN_DEPLOYER_ETH_WEI} wei for bounded deployment gas")
    if deployer_usdc < MIN_CANARY_USDC:
        raise RuntimeError(f"admin wallet needs at least {MIN_CANARY_USDC} USDC base units for the bounded canary")

    factory_address = create_address(deployer, args.deployer_nonce)
    implementation_address = create_address(factory_address, 1)
    if not args.offline:
        for name, address in (("factory", factory_address), ("implementation", implementation_address)):
            if rpc(args.rpc_url, "eth_getCode", [address, block_tag]) != "0x":
                raise RuntimeError(f"predicted {name} address is occupied: {address}")

    out = repo / "contracts" / "base-escrow" / "out"
    factory_artifact = json.loads(
        (out / "OpenCompetitionBountyFactoryV1.sol" / "OpenCompetitionBountyFactoryV1.json").read_text(
            encoding="utf-8"
        )
    )
    implementation_artifact = json.loads(
        (out / "OpenCompetitionBountyV1.sol" / "OpenCompetitionBountyV1.json").read_text(encoding="utf-8")
    )
    factory_data = artifact_hex(factory_artifact.get("bytecode"), "factory.bytecode") + address_word(USDC)
    factory_runtime = patched_runtime(
        factory_artifact,
        [address_word(USDC), address_word(implementation_address)],
        "factory",
    )
    implementation_runtime = artifact_hex(
        implementation_artifact.get("deployedBytecode"), "implementation.deployedBytecode"
    )
    source_files = [
        repo / "contracts/base-escrow/src/OpenCompetitionBountyV1.sol",
        repo / "contracts/base-escrow/src/OpenCompetitionBountyFactoryV1.sol",
        repo / "contracts/base-escrow/src/LeadingZeroWorkVerifier.sol",
        repo / "contracts/base-escrow/src/IAgentBounty.sol",
    ]
    action = deployment_action(
        nonce=args.deployer_nonce,
        data=factory_data,
        expected_contract=factory_address,
        runtime=factory_runtime,
        implementation=implementation_address,
        implementation_runtime=implementation_runtime,
    )
    return {
        "schema_version": "agent-bounties/open-competition-v1-mainnet-bundle-v1",
        "protocol_version": "agent-bounties/open-competition-v1",
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "deployment_state": "sepolia_rehearsed_not_ready_to_earn",
        "source_commit": args.source_commit,
        "compiler": {"solc": "0.8.26", "optimizer": True, "optimizer_runs": 200, "evm": "cancun"},
        "source_sha256": {
            str(path.relative_to(repo)).replace("\\", "/"): normalized_sha256(path) for path in source_files
        },
        "deployer": deployer,
        "settlement_token": USDC,
        "preflight_block": {
            "number": args.preflight_block_number,
            "hash": args.preflight_block_hash.lower(),
            "deployer_nonce": args.deployer_nonce,
            "deployer_eth_wei": deployer_eth,
            "deployer_usdc_base_units": deployer_usdc,
        },
        "actions": [action],
        "verifier_profile": verifier_profile(),
        "factory": factory_address.lower(),
        "implementation": implementation_address.lower(),
        "hidden_canary": {
            "solver_reward_usdc_base_units": CANARY_SOLVER_REWARD,
            "verifier_reward_usdc_base_units": CANARY_VERIFIER_REWARD,
            "entry_bond_usdc_base_units": CANARY_SOLVER_BOND,
            "initial_funding_usdc_base_units": CANARY_INITIAL_FUNDING,
            "total_admin_usdc_budget_base_units": MIN_CANARY_USDC,
            "max_entries": 4,
            "competition_window_seconds": 86_400,
            "reveal_window_seconds": 3_600,
            "creator_may_compete": False,
            "separate_solver_wallet_required": True,
            "inventory_visibility": "hidden",
        },
        "activation": {
            "public_creation_enabled": False,
            "public_commitments_enabled": False,
            "public_inventory_eligible": False,
            "required_before_activation": [
                "canonical safe-block BountySettled event",
                "exact USDC balance reconciliation",
                "healthy indexer heartbeat",
                "verified API, MCP, CLI, Python SDK, and TypeScript SDK behavior",
            ],
        },
        "evidence_boundary": (
            "This bundle contains unsigned exact factory deployment calldata, a pinned approved verifier, "
            "and bounded hidden-canary parameters. It is not deployment, settlement, payment, or activation evidence."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deployer", default=ADMIN)
    parser.add_argument("--deployer-nonce", type=int, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--preflight-block-number", type=int, required=True)
    parser.add_argument("--preflight-block-hash", required=True)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", DEFAULT_RPC))
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--preflight-deployer-eth-wei", type=int)
    parser.add_argument("--preflight-deployer-usdc-base-units", type=int)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    bundle = build_bundle(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "factory": bundle["factory"], "implementation": bundle["implementation"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
