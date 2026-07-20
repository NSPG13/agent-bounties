#!/usr/bin/env python3
"""Build the unsigned, immutable Base Sepolia sponsor activation bundle."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
from typing import Any
from urllib.error import URLError
from urllib.request import Request, urlopen

from _shared.evm import address_bytes, address_word, artifact_hex, create_address, keccak256, uint_word


CHAIN_ID = 84_532
CHAIN_ID_HEX = "0x14a34"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
DIFFICULTY_BITS = 16
MAX_BOND = 100_000
MAX_NETWORK_PER_DAY = 1_000_000
MAX_LIFETIME_PER_SOLVER = 100_000
SPONSOR_SEED = 100_000
DEFAULT_RPC = "https://sepolia.base.org"


def patched_runtime(artifact: dict[str, Any], values: list[bytes], name: str) -> bytes:
    deployed = artifact.get("deployedBytecode")
    runtime = bytearray(artifact_hex(deployed, f"{name}.deployedBytecode"))
    references = deployed.get("immutableReferences") if isinstance(deployed, dict) else None
    if not isinstance(references, dict) or len(references) != len(values):
        raise ValueError(f"{name} immutable reference count changed")
    for value, (_, locations) in zip(
        values, sorted(references.items(), key=lambda item: int(item[0]))
    ):
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
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    ).encode("utf-8")
    request = Request(
        url,
        data=payload,
        headers={"content-type": "application/json", "user-agent": "agent-bounties-activation/1"},
    )
    try:
        with urlopen(request, timeout=30) as response:
            body = json.load(response)
    except (OSError, URLError) as error:
        raise RuntimeError(f"RPC transport failed for {method}: {error}") from error
    if body.get("error"):
        raise RuntimeError(f"RPC {method} failed: {json.dumps(body['error'], sort_keys=True)}")
    return body.get("result")


def uint_result(value: str) -> int:
    return int(value, 16)


def balance_of_data(address: str) -> str:
    return f"0x70a08231{address_word(address).hex()}"


def deployment_action(
    *,
    name: str,
    nonce: int,
    data: bytes,
    expected_contract: str,
    runtime: bytes,
) -> dict[str, Any]:
    return {
        "name": name,
        "from_nonce": nonce,
        "to": None,
        "value_wei": 0,
        "data": f"0x{data.hex()}",
        "creation_code_hash": keccak256(data),
        "expected_contract": expected_contract.lower(),
        "expected_runtime_code": f"0x{runtime.hex()}",
        "runtime_code_hash": keccak256(runtime),
        "runtime_code_bytes": len(runtime),
    }


def build_bundle(args: argparse.Namespace) -> dict[str, Any]:
    repo = Path(__file__).resolve().parents[1]
    deployer = args.deployer.lower()
    grant_signer = args.grant_signer.lower()
    address_bytes(deployer)
    address_bytes(grant_signer)
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
            raise RuntimeError("pinned Base Sepolia block hash mismatch")
        if rpc(args.rpc_url, "eth_chainId", []) != CHAIN_ID_HEX:
            raise RuntimeError("RPC is not Base Sepolia")
        if rpc(args.rpc_url, "eth_getCode", [USDC, block_tag]) == "0x":
            raise RuntimeError("native Base Sepolia USDC code is unavailable at the pinned block")
        observed_nonce = uint_result(
            rpc(args.rpc_url, "eth_getTransactionCount", [deployer, block_tag])
        )
        if observed_nonce != args.deployer_nonce:
            raise RuntimeError(
                f"deployer nonce mismatch at pinned block: expected {args.deployer_nonce}, got {observed_nonce}"
            )
        deployer_eth = uint_result(rpc(args.rpc_url, "eth_getBalance", [deployer, block_tag]))
        deployer_usdc = uint_result(
            rpc(
                args.rpc_url,
                "eth_call",
                [{"to": USDC, "data": balance_of_data(deployer)}, block_tag],
            )
        )

    factory_address = create_address(deployer, args.deployer_nonce)
    implementation_address = create_address(factory_address, 1)
    verifier_address = create_address(deployer, args.deployer_nonce + 1)
    sponsor_address = create_address(deployer, args.deployer_nonce + 2)
    if not args.offline:
        for name, address in (
            ("factory", factory_address),
            ("verifier", verifier_address),
            ("sponsor", sponsor_address),
        ):
            if rpc(args.rpc_url, "eth_getCode", [address, block_tag]) != "0x":
                raise RuntimeError(f"predicted {name} address is occupied at the pinned block: {address}")
        sponsor_balance = uint_result(
            rpc(
                args.rpc_url,
                "eth_call",
                [{"to": USDC, "data": balance_of_data(sponsor_address)}, block_tag],
            )
        )
        if sponsor_balance != 0:
            raise RuntimeError("predicted sponsor address already holds test USDC")

    out = repo / "contracts" / "base-escrow" / "out"
    factory_artifact = json.loads(
        (out / "AgentBountyFactory.sol" / "AgentBountyFactory.json").read_text(encoding="utf-8")
    )
    implementation_artifact = json.loads(
        (out / "AgentBounty.sol" / "AgentBounty.json").read_text(encoding="utf-8")
    )
    verifier_artifact = json.loads(
        (out / "LeadingZeroWorkVerifier.sol" / "LeadingZeroWorkVerifier.json").read_text(
            encoding="utf-8"
        )
    )
    sponsor_artifact = json.loads(
        (out / "AtomicClaimSponsor.sol" / "AtomicClaimSponsor.json").read_text(encoding="utf-8")
    )

    factory_data = artifact_hex(factory_artifact.get("bytecode"), "factory.bytecode") + address_word(USDC)
    verifier_data = artifact_hex(verifier_artifact.get("bytecode"), "verifier.bytecode") + uint_word(
        DIFFICULTY_BITS
    )
    sponsor_data = b"".join(
        [
            artifact_hex(sponsor_artifact.get("bytecode"), "sponsor.bytecode"),
            address_word(USDC),
            address_word(factory_address),
            address_word(grant_signer),
            uint_word(MAX_BOND),
            uint_word(MAX_NETWORK_PER_DAY),
            uint_word(MAX_LIFETIME_PER_SOLVER),
        ]
    )
    factory_runtime = patched_runtime(
        factory_artifact,
        [address_word(USDC), address_word(implementation_address)],
        "factory",
    )
    implementation_runtime = artifact_hex(
        implementation_artifact.get("deployedBytecode"), "implementation.deployedBytecode"
    )
    verifier_runtime = patched_runtime(
        verifier_artifact, [uint_word(DIFFICULTY_BITS)], "verifier"
    )
    sponsor_runtime = patched_runtime(
        sponsor_artifact,
        [
            address_word(USDC),
            address_word(factory_address),
            uint_word(MAX_BOND),
            uint_word(MAX_NETWORK_PER_DAY),
            uint_word(MAX_LIFETIME_PER_SOLVER),
        ],
        "sponsor",
    )
    transfer_data = bytes.fromhex("a9059cbb") + address_word(sponsor_address) + uint_word(SPONSOR_SEED)

    if deployer_usdc < SPONSOR_SEED:
        raise RuntimeError("deployer lacks the test USDC required to seed the sponsor")

    return {
        "schema_version": "agent-bounties/base-sepolia-sponsor-activation-v1",
        "protocol_version": "agent-bounties/autonomous-v1",
        "network": "base-sepolia",
        "chain_id": CHAIN_ID,
        "source_commit": args.source_commit,
        "deployer": deployer,
        "settlement_token": USDC,
        "grant_signer": grant_signer,
        "preflight_block": {
            "number": args.preflight_block_number,
            "hash": args.preflight_block_hash.lower(),
            "deployer_nonce": args.deployer_nonce,
            "deployer_eth_wei": deployer_eth,
            "deployer_usdc_base_units": deployer_usdc,
        },
        "factory": {
            **deployment_action(
                name="deploy_factory",
                nonce=args.deployer_nonce,
                data=factory_data,
                expected_contract=factory_address,
                runtime=factory_runtime,
            ),
            "expected_implementation": implementation_address.lower(),
            "expected_implementation_runtime_code": f"0x{implementation_runtime.hex()}",
            "implementation_runtime_code_hash": keccak256(implementation_runtime),
        },
        "verifier": {
            **deployment_action(
                name="deploy_leading_zero_verifier",
                nonce=args.deployer_nonce + 1,
                data=verifier_data,
                expected_contract=verifier_address,
                runtime=verifier_runtime,
            ),
            "difficulty_bits": DIFFICULTY_BITS,
        },
        "sponsor": {
            **deployment_action(
                name="deploy_atomic_claim_sponsor",
                nonce=args.deployer_nonce + 2,
                data=sponsor_data,
                expected_contract=sponsor_address,
                runtime=sponsor_runtime,
            ),
            "max_bond_base_units": MAX_BOND,
            "max_network_per_day_base_units": MAX_NETWORK_PER_DAY,
            "max_lifetime_per_solver_base_units": MAX_LIFETIME_PER_SOLVER,
        },
        "sponsor_funding": {
            "name": "seed_sponsor",
            "from": deployer,
            "to": USDC,
            "value_wei": 0,
            "data": f"0x{transfer_data.hex()}",
            "amount_base_units": SPONSOR_SEED,
            "recipient": sponsor_address.lower(),
        },
        "deterministic_gates": [
            "forge test --fuzz-runs 1000",
            (
                "RUN_SEPOLIA_FORK=true forge test --match-contract "
                "AtomicClaimSponsorMainnetForkTest --match-test "
                "testRealUsdcZeroBalanceSolverCompletesSponsoredLoopOnBaseSepolia -vv"
            ),
        ],
        "evidence_boundary": (
            "This unsigned bundle pins live Base Sepolia state and compiler-derived constructor/runtime "
            "artifacts. Foundry tests separately execute the contracts against native Base Sepolia USDC. "
            "Live deployment and sponsor funding still require confirmed receipts plus bytecode/getter "
            "verification; neither proves a bounty claim, completion, settlement, or payout."
        ),
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--deployer", required=True)
    parser.add_argument("--grant-signer", required=True)
    parser.add_argument("--deployer-nonce", type=int, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--preflight-block-number", type=int, required=True)
    parser.add_argument("--preflight-block-hash", required=True)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_SEPOLIA_RPC_URL", DEFAULT_RPC))
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
    print(
        json.dumps(
            {
                "output": str(args.output),
                "factory": bundle["factory"]["expected_contract"],
                "verifier": bundle["verifier"]["expected_contract"],
                "sponsor": bundle["sponsor"]["expected_contract"],
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
