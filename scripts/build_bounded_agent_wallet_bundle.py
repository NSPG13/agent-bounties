#!/usr/bin/env python3
"""Build a deterministic Base mainnet bounded-wallet factory manifest."""

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
BOUNTY_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
VERIFIER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
LEGACY_SIGNED_QUORUM_VERIFIER_SET_HASH = "0x2c5a10915ca1fb99d4a11e2222b4f32b986b4e0f5599f55d70e9c8f9725a28cd"
SINGLE_VERIFIER_SET_HASH = "0x0838846e439ed67544d8a06da2a0f344fb25cd44723ad65839da3f242a72b1f2"
SOURCE_INPUTS = ("contracts/base-escrow",)
VERSIONS = {
    "v1": {
        "factory": "BoundedAgentWalletFactory",
        "wallet": "BoundedAgentWallet",
        "salt_label": "agent-bounties/base-mainnet/bounded-agent-wallet-factory/v1",
    },
    "v2": {
        "factory": "BoundedAgentWalletV2Factory",
        "wallet": "BoundedAgentWalletV2",
        "salt_label": "agent-bounties/base-mainnet/bounded-agent-wallet-factory/v2",
    },
}


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
            candidate = json.loads(path.read_text(encoding="utf-8"))
            visit(candidate.get("ast", {}))
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


def build_bundle(version: str = "v1") -> dict:
    config = VERSIONS[version]
    factory_contract = config["factory"]
    wallet_contract = config["wallet"]
    salt_label = config["salt_label"]
    run([FORGE, "build", "--force", "--ast"], cwd=CONTRACTS)
    init_code = append_constructor(bytecode(factory_contract), "constructor(address)", BOUNTY_FACTORY)
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
        factory_contract,
        {"bountyFactory": BOUNTY_FACTORY, "settlementToken": USDC, "implementation": implementation},
    )
    implementation_runtime = exact_runtime(
        wallet_contract,
        {"deploymentFactory": wallet_factory, "factory": BOUNTY_FACTORY, "settlementToken": USDC},
    )
    clone_runtime = f"0x363d3d373d3d3d363d73{implementation[2:]}5af43d82803e903d91602b57fd5bf3"
    return {
        "schema": f"agent-bounties/bounded-agent-wallet-deployment-{version}",
        "contract_source_revision": run(["git", "rev-parse", "HEAD:contracts/base-escrow"]),
        "contract_source_revision_kind": "git-tree",
        "contract_source_dirty": bool(
            run(["git", "status", "--short", "--", *SOURCE_INPUTS])
        ),
        "network": "base-mainnet",
        "chain_id": 8453,
        "rpc_url": "https://mainnet.base.org",
        "canonical": {
            "bounty_factory": BOUNTY_FACTORY,
            "settlement_token": USDC,
            "deterministic_verifier": VERIFIER,
            "signed_quorum_verifier_set_hash": (
                SINGLE_VERIFIER_SET_HASH
                if version == "v2"
                else LEGACY_SIGNED_QUORUM_VERIFIER_SET_HASH
            ),
        },
        "deterministic_deployer": {
            "address": CREATE2_DEPLOYER,
            "runtime_code_hash": CREATE2_DEPLOYER_CODE_HASH,
        },
        "wallet_factory": {
            "address": wallet_factory,
            "implementation": implementation,
            "version": version,
            "factory_contract": factory_contract,
            "wallet_contract": wallet_contract,
            "salt_label": salt_label,
            "salt": salt,
            "init_code_hash": init_code_hash,
            "deployment_transaction": f"0x{salt[2:]}{init_code[2:]}",
            "runtime_code_hash": keccak(factory_runtime),
            "implementation_runtime_code_hash": keccak(implementation_runtime),
            "clone_runtime_code_hash": keccak(clone_runtime),
        },
        "contracts": {
            path.stem: {"source_sha256": source_sha256(path.stem)}
            for path in sorted((CONTRACTS / "src").glob("*.sol"))
        },
        "evidence_boundary": (
            "This manifest derives exact addresses, bytecode, and unsigned deployment calldata. It is not "
            "deployment, delegation, funding, completion, payout, or settlement evidence."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", choices=sorted(VERSIONS), default="v1")
    parser.add_argument(
        "--output",
        type=Path,
    )
    args = parser.parse_args()
    output = args.output or (
        ROOT / "deployments" / (
            "bounded-agent-wallet-base-mainnet.json"
            if args.version == "v1"
            else f"bounded-agent-wallet-{args.version}-base-mainnet.json"
        )
    )
    bundle = build_bundle(args.version)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
