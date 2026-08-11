#!/usr/bin/env python3
"""Build exact deterministic deployment bundles for the Open Competition entrant wallet."""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts" / "base-escrow"
CREATE2_DEPLOYER = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
CREATE2_DEPLOYER_CODE_HASH = "0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989"
FACTORY_CONTRACT = "OpenCompetitionEntrantWalletFactoryV1"
WALLET_CONTRACT = "OpenCompetitionEntrantWalletV1"
NETWORKS = {
    "base-sepolia": {
        "chain_id": 84532,
        "rpc_url": "https://sepolia.base.org",
        "competition_factory": "0x7231f1312448fa60078fb56cdb6e2c392bd1269b",
        "settlement_token": "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
    },
    "base-mainnet": {
        "chain_id": 8453,
        "rpc_url": "https://mainnet.base.org",
        "competition_factory": "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5",
        "settlement_token": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    },
}
SOURCE_INPUTS = ("contracts/base-escrow",)


def executable(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    candidate = ROOT / ".tools" / "foundry" / f"{name}.exe"
    if candidate.exists():
        return str(candidate)
    raise SystemExit(f"{name} is required; install Foundry or use .tools/foundry")


CAST = executable("cast")
FORGE = executable("forge")


def run(command: list[str], cwd: Path = ROOT, input_text: str | None = None) -> str:
    result = subprocess.run(
        command,
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        input=input_text,
    )
    return result.stdout.strip()


def cast(*args: str) -> str:
    return run([CAST, *args])


def keccak(value: str) -> str:
    return run([CAST, "keccak"], input_text=value).lower()


def artifact(contract: str) -> dict:
    path = CONTRACTS / "out" / f"{contract}.sol" / f"{contract}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def bytecode(contract: str) -> str:
    value = artifact(contract)["bytecode"]["object"]
    if not value.startswith("0x") or "__$" in value:
        raise SystemExit(f"{contract} creation bytecode is unavailable or unlinked")
    return value.lower()


def immutable_names(contract: str) -> dict[str, str]:
    value = artifact(contract)
    identifiers = set(value["deployedBytecode"]["immutableReferences"])
    names: dict[str, str] = {}

    def visit(node: object) -> None:
        if isinstance(node, dict):
            identifier = str(node.get("id"))
            if identifier in identifiers and node.get("nodeType") == "VariableDeclaration":
                names[identifier] = str(node["name"])
            for child in node.values():
                visit(child)
        elif isinstance(node, list):
            for child in node:
                visit(child)

    visit(value.get("ast", {}))
    if set(names) != identifiers:
        for path in (CONTRACTS / "out").glob("*/*.json"):
            visit(json.loads(path.read_text(encoding="utf-8")).get("ast", {}))
            if set(names) == identifiers:
                break
    if set(names) != identifiers:
        raise SystemExit(f"{contract} immutable metadata is incomplete")
    return names


def immutable_word(value: str | int) -> str:
    encoded = f"{value:x}" if isinstance(value, int) else value.lower().removeprefix("0x")
    if len(encoded) > 64 or any(character not in "0123456789abcdef" for character in encoded):
        raise SystemExit(f"invalid immutable value: {value}")
    return encoded.rjust(64, "0")


def exact_runtime(contract: str, immutables: dict[str, str | int]) -> str:
    value = artifact(contract)
    runtime = value["deployedBytecode"]["object"].lower().removeprefix("0x")
    references = value["deployedBytecode"]["immutableReferences"]
    names = immutable_names(contract)
    if set(immutables) != set(names.values()):
        raise SystemExit(
            f"{contract} immutable mismatch: expected {sorted(names.values())}, got {sorted(immutables)}"
        )
    for identifier, locations in references.items():
        word = immutable_word(immutables[names[identifier]])
        for location in locations:
            if location["length"] != 32:
                raise SystemExit(f"{contract} has a non-word immutable")
            start = location["start"] * 2
            runtime = f"{runtime[:start]}{word}{runtime[start + 64:]}"
    return f"0x{runtime}"


def append_constructor(code: str, signature: str, *args: str) -> str:
    encoded = cast("abi-encode", signature, *args)
    return f"{code}{encoded[2:]}".lower()


def create_address(deployer: str, nonce: int) -> str:
    return cast("compute-address", "--nonce", str(nonce), deployer).split(":", 1)[-1].strip().lower()


def source_sha256(name: str) -> str:
    return f"0x{hashlib.sha256((CONTRACTS / 'src' / f'{name}.sol').read_bytes()).hexdigest()}"


def build_bundle(network: str, *, compile_contracts: bool = True) -> dict:
    config = NETWORKS[network]
    if compile_contracts:
        run([FORGE, "build", "--force", "--ast"], cwd=CONTRACTS)
    init_code = append_constructor(
        bytecode(FACTORY_CONTRACT), "constructor(address)", config["competition_factory"]
    )
    salt_label = f"agent-bounties/{network}/open-competition-entrant-wallet-factory/v1"
    salt = cast("keccak", salt_label).lower()
    init_code_hash = keccak(init_code)
    wallet_factory = cast(
        "create2",
        "--deployer",
        CREATE2_DEPLOYER,
        "--salt",
        salt,
        "--init-code-hash",
        init_code_hash,
    ).splitlines()[0].lower()
    implementation = create_address(wallet_factory, 1)
    factory_runtime = exact_runtime(
        FACTORY_CONTRACT,
        {
            "competitionFactory": config["competition_factory"],
            "settlementToken": config["settlement_token"],
            "implementation": implementation,
        },
    )
    implementation_runtime = exact_runtime(
        WALLET_CONTRACT,
        {
            "deploymentFactory": wallet_factory,
            "factory": config["competition_factory"],
            "settlementToken": config["settlement_token"],
        },
    )
    clone_runtime = f"0x363d3d373d3d3d363d73{implementation[2:]}5af43d82803e903d91602b57fd5bf3"
    source_revision = run(["git", "rev-parse", "HEAD:contracts/base-escrow"])
    source_dirty = bool(run(["git", "status", "--short", "--", *SOURCE_INPUTS]))
    return {
        "schema_version": "agent-bounties/open-competition-entrant-wallet-deployment-v1",
        "network": network,
        "chain_id": config["chain_id"],
        "deployment_state": "source_only_not_ready_to_earn",
        "contract_source_revision": source_revision,
        "contract_source_revision_kind": "git-tree",
        "contract_source_dirty": source_dirty,
        "compiler": {"solc": "0.8.26", "optimizer": True, "optimizer_runs": 200, "evm": "cancun"},
        "rpc_url": config["rpc_url"],
        "canonical": {
            "competition_factory": config["competition_factory"],
            "settlement_token": config["settlement_token"],
        },
        "deterministic_deployer": {
            "address": CREATE2_DEPLOYER,
            "runtime_code_hash": CREATE2_DEPLOYER_CODE_HASH,
        },
        "entrant_wallet_factory": {
            "address": wallet_factory,
            "implementation": implementation,
            "salt_label": salt_label,
            "salt": salt,
            "init_code_hash": init_code_hash,
            "deployment_transaction": f"0x{salt[2:]}{init_code[2:]}",
            "runtime_code_hash": keccak(factory_runtime),
            "runtime_code_bytes": (len(factory_runtime) - 2) // 2,
            "implementation_runtime_code_hash": keccak(implementation_runtime),
            "implementation_runtime_code_bytes": (len(implementation_runtime) - 2) // 2,
            "clone_runtime_code_hash": keccak(clone_runtime),
        },
        "contracts": {
            FACTORY_CONTRACT: {"source_sha256": source_sha256(FACTORY_CONTRACT)},
            WALLET_CONTRACT: {"source_sha256": source_sha256(WALLET_CONTRACT)},
        },
        "activation_gates": {
            "base_sepolia_rehearsal_passed": False,
            "exact_mainnet_fork_replay_passed": False,
            "static_analysis_passed": False,
            "independent_review_complete": False,
            "keeper_relay_rehearsed": False,
            "keeper_gas_reserve_verified": False,
            "relay_support_available": False,
            "gas_sponsorship_available": False,
        },
        "evidence_boundary": (
            "This bundle derives exact addresses, bytecode, runtime hashes, and unsigned CREATE2 deployment "
            "calldata. It is not deployment, policy approval, relay availability, gas sponsorship, competition "
            "entry, settlement, or payment evidence."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--network", choices=sorted(NETWORKS), required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    output = args.output or ROOT / "deployments" / f"open-competition-entrant-wallet-v1-{args.network}.json"
    bundle = build_bundle(args.network)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
