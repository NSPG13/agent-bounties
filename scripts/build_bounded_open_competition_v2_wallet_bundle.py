#!/usr/bin/env python3
"""Build the deterministic Base manifest for the recoverable V2 reserve factory."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "contracts" / "base-escrow"
CREATE2_DEPLOYER = "0x4e59b44847b379578588920ca78fbf26c0b4956c"
CREATE2_DEPLOYER_CODE_HASH = "0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989"
COMPETITION_FACTORY = "0xa45c6636d75fc94eec8cf6f6a34308c687e42ce4"
USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
RELEASE_HASH = "0x0195f28ff1705e7613b55fbe6407092ceaba5c9c6d2b68bbf3f73558192854be"
FACTORY_CONTRACT = "BoundedOpenCompetitionV2WalletFactory"
WALLET_CONTRACT = "BoundedOpenCompetitionV2Wallet"
SALT_LABEL = "agent-bounties/base-mainnet/bounded-open-competition-v2-wallet-factory/v1"
SOURCE_INPUTS = ("contracts/base-escrow",)


def executable(name: str) -> str:
    found = shutil.which(name)
    if found:
        return found
    for candidate in (
        ROOT / ".tools" / "foundry" / f"{name}.exe",
        Path.home() / ".foundry" / "bin" / f"{name}.exe",
        Path.home() / ".foundry" / "bin" / name,
    ):
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


def normalize_address(value: str, field: str) -> str:
    normalized = value.strip().lower()
    if not re.fullmatch(r"0x[0-9a-f]{40}", normalized):
        raise SystemExit(f"{field} must be a 20-byte hex address")
    return normalized


def normalize_hash(value: str, field: str) -> str:
    normalized = value.strip().lower()
    if not re.fullmatch(r"0x[0-9a-f]{64}", normalized) or normalized == "0x" + "00" * 32:
        raise SystemExit(f"{field} must be a nonzero bytes32 hex value")
    return normalized


def build_bundle(
    competition_factory: str = COMPETITION_FACTORY,
    release_hash: str = RELEASE_HASH,
) -> dict:
    competition_factory = normalize_address(competition_factory, "competition factory")
    release_hash = normalize_hash(release_hash, "release hash")
    run([FORGE, "build", "--force", "--ast"], cwd=CONTRACTS)
    init_code = append_constructor(bytecode(FACTORY_CONTRACT), "constructor(address)", competition_factory)
    salt = cast("keccak", SALT_LABEL).lower()
    init_code_hash = keccak(init_code)
    reserve_factory = cast(
        "create2",
        "--deployer",
        CREATE2_DEPLOYER,
        "--salt",
        salt,
        "--init-code-hash",
        init_code_hash,
    ).splitlines()[0].lower()
    implementation = create_address(reserve_factory, 1)
    factory_runtime = exact_runtime(
        FACTORY_CONTRACT,
        {
            "competitionFactory": competition_factory,
            "settlementToken": USDC,
            "implementation": implementation,
        },
    )
    implementation_runtime = exact_runtime(
        WALLET_CONTRACT,
        {
            "competitionFactory": competition_factory,
            "settlementToken": USDC,
            "deploymentFactory": reserve_factory,
        },
    )
    clone_runtime = f"0x363d3d373d3d3d363d73{implementation[2:]}5af43d82803e903d91602b57fd5bf3"
    return {
        "schema": "agent-bounties/bounded-open-competition-v2-wallet-deployment-v1",
        "contract_source_revision": run(["git", "rev-parse", "HEAD:contracts/base-escrow"]),
        "contract_source_revision_kind": "git-tree",
        "contract_source_dirty": bool(run(["git", "status", "--short", "--", *SOURCE_INPUTS])),
        "network": "base-mainnet",
        "chain_id": 8453,
        "canonical": {
            "protocol_version": "agent-bounties/open-competition-v2-beta3",
            "competition_factory": competition_factory,
            "settlement_token": USDC,
            "release_hash": release_hash,
        },
        "deterministic_deployer": {
            "address": CREATE2_DEPLOYER,
            "runtime_code_hash": CREATE2_DEPLOYER_CODE_HASH,
        },
        "reserve_factory": {
            "address": reserve_factory,
            "implementation": implementation,
            "factory_contract": FACTORY_CONTRACT,
            "wallet_contract": WALLET_CONTRACT,
            "salt_label": SALT_LABEL,
            "salt": salt,
            "init_code_hash": init_code_hash,
            "deployment_transaction": f"0x{salt[2:]}{init_code[2:]}",
            "runtime_code_hash": keccak(factory_runtime),
            "implementation_runtime_code_hash": keccak(implementation_runtime),
            "clone_runtime_code_hash": keccak(clone_runtime),
            "runtime_size_bytes": (len(factory_runtime) - 2) // 2,
            "implementation_runtime_size_bytes": (len(implementation_runtime) - 2) // 2,
        },
        "contracts": {
            FACTORY_CONTRACT: {"source_sha256": source_sha256(FACTORY_CONTRACT)},
            WALLET_CONTRACT: {"source_sha256": source_sha256(WALLET_CONTRACT)},
        },
        "recovery_boundary": (
            "The owner can revoke and recover uncommitted or canonically refundable USDC. Healthy active "
            "competition escrow remains committed until settlement, cancellation, verifier unavailability, or expiry."
        ),
        "evidence_boundary": (
            "This manifest derives exact addresses, bytecode, and unsigned deployment calldata. It is not "
            "deployment, delegation, funding, activation, GMV, payout, or settlement evidence."
        ),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--competition-factory",
        default=COMPETITION_FACTORY,
        help="Exact canonical Open Competition V2 Beta3 factory for this release.",
    )
    parser.add_argument(
        "--release-hash",
        default=RELEASE_HASH,
        help="Exact canonical release hash bound to the factory.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "deployments" / "bounded-open-competition-v2-wallet-base-mainnet.json",
    )
    args = parser.parse_args()
    bundle = build_bundle(args.competition_factory, args.release_hash)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
