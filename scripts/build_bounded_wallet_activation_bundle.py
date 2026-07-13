#!/usr/bin/env python3
"""Build the locked Base Sepolia/mainnet bounded-wallet activation bundle."""

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
CREATE2_DEPLOYER_RUNTIME_CODE = "0x7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3"
OWNER = "0x884834e884d6e93462655a2820140ad03e6747bc"
DELEGATES = {
    "creator": "0xae34cc78b8e6b81d5cd7cba109e524065f6c0c13",
    "solver": "0x16a24c545d80f48ea08fa958d29fc095848afef0",
}
RELAYER = "0x43b6a22e6660d3a7e6cbc739c90726b55eabab5e"
POLICY = {
    "valid_after": 0,
    "valid_until": 1_798_761_600,
    "period_seconds": 86_400,
    "max_per_action": 500_000,
    "max_per_period": 1_000_000,
    "max_lifetime_spend": 1_000_000,
    "allowed_actions": 15,
    "allowed_verification_modes": 1,
}
PILOT_FUNDING = {"creator": 300_000, "solver": 100_000}
USDC_EIP712_NAMES = {
    "base-sepolia": "USDC",
    "base-mainnet": "USD Coin",
}
USDC_EIP712_VERSION = "2"
AUTHORIZATION_MAX_VALIDITY_SECONDS = 1_800
MAINNET_FACTORY = "0x082c52131aaf0c56e76b075f895eab6fcab6d2f9"
MAINNET_IMPLEMENTATION = "0x2fa36d2b2327642db3a6cc8cdd91544ad7484eb9"
MAINNET_VERIFIER = "0xcc6059ceeda5bc4ba8a97ecfbffa7488c8fd579e"


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


def immutable_variable_names(contract: str) -> dict[str, str]:
    contract_artifact = artifact(contract)
    identifiers = set(contract_artifact["deployedBytecode"]["immutableReferences"])
    names: dict[str, str] = {}

    def visit(value: object) -> None:
        if isinstance(value, dict):
            identifier = str(value.get("id"))
            if identifier in identifiers and value.get("nodeType") == "VariableDeclaration":
                names[identifier] = str(value["name"])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit(contract_artifact.get("ast", {}))
    if set(names) != identifiers:
        raise SystemExit(f"{contract} immutable metadata is incomplete")
    return names


def immutable_word(value: str | int) -> str:
    if isinstance(value, int):
        encoded = f"{value:x}"
    else:
        encoded = value.lower().removeprefix("0x")
    if len(encoded) > 64 or any(character not in "0123456789abcdef" for character in encoded):
        raise SystemExit(f"invalid immutable value: {value}")
    return encoded.rjust(64, "0")


def exact_runtime_code(contract: str, immutables: dict[str, str | int]) -> str:
    contract_artifact = artifact(contract)
    runtime = contract_artifact["deployedBytecode"]["object"].lower().removeprefix("0x")
    references = contract_artifact["deployedBytecode"]["immutableReferences"]
    names = immutable_variable_names(contract)
    if set(immutables) != set(names.values()):
        raise SystemExit(
            f"{contract} immutable values differ: expected {sorted(names.values())}, "
            f"received {sorted(immutables)}"
        )
    for identifier, locations in references.items():
        word = immutable_word(immutables[names[identifier]])
        for location in locations:
            if location["length"] != 32:
                raise SystemExit(f"{contract} has a non-word immutable")
            start = location["start"] * 2
            runtime = f"{runtime[:start]}{word}{runtime[start + 64:]}"
    return f"0x{runtime}"


def attach_runtime(component: dict, runtime_code: str) -> None:
    component["runtime_code"] = runtime_code
    component["runtime_code_hash"] = keccak(runtime_code)


def create_address(deployer: str, nonce: int) -> str:
    output = cast("compute-address", "--nonce", str(nonce), deployer)
    return output.split(":", 1)[-1].strip().lower()


def source_sha256(name: str) -> str:
    value = (CONTRACTS / "src" / f"{name}.sol").read_bytes()
    return f"0x{hashlib.sha256(value).hexdigest()}"


def append_abi(code: str, signature: str, *args: str) -> str:
    encoded = cast("abi-encode", signature, *args)
    return f"{code}{encoded[2:]}".lower()


def create2_component(name: str, salt_label: str, init_code: str) -> dict:
    salt = cast("keccak", salt_label).lower()
    init_code_hash = keccak(init_code)
    address = cast(
        "create2",
        "--deployer",
        CREATE2_DEPLOYER,
        "--salt",
        salt,
        "--init-code-hash",
        init_code_hash,
    ).splitlines()[0].lower()
    return {
        "name": name,
        "salt": salt,
        "init_code_hash": init_code_hash,
        "expected_contract": address,
        "deployment_transaction": f"0x{salt[2:]}{init_code[2:]}",
    }


def policy_tuple(delegate: str) -> str:
    return (
        f"({delegate},{POLICY['valid_after']},{POLICY['valid_until']},"
        f"{POLICY['period_seconds']},{POLICY['max_per_action']},"
        f"{POLICY['max_per_period']},{POLICY['max_lifetime_spend']},"
        f"{POLICY['allowed_actions']},{POLICY['allowed_verification_modes']})"
    )


def predicted_wallet(
    wallet_factory: str,
    bounty_factory: str,
    delegate: str,
    user_salt: str,
) -> str:
    constructor = cast(
        "abi-encode",
        "constructor(address,address,(address,uint64,uint64,uint64,uint256,uint256,uint256,uint8,uint8))",
        OWNER,
        bounty_factory,
        policy_tuple(delegate),
    )
    wallet_init_code = f"{bytecode('BoundedAgentWallet')}{constructor[2:]}"
    effective_salt_preimage = cast(
        "abi-encode", "f(address,bytes32)", OWNER, user_salt
    )
    effective_salt = keccak(effective_salt_preimage)
    init_code_hash = keccak(wallet_init_code)
    return cast(
        "create2",
        "--deployer",
        wallet_factory,
        "--salt",
        effective_salt,
        "--init-code-hash",
        init_code_hash,
    ).splitlines()[0].lower()


def wallet_calls(network: str, wallet_factory: str, bounty_factory: str, usdc: str) -> dict:
    wallets = []
    for role in ("creator", "solver"):
        delegate = DELEGATES[role]
        user_salt = cast("keccak", f"agent-bounties/{network}/pilot/{role}/v1").lower()
        predicted = predicted_wallet(wallet_factory, bounty_factory, delegate, user_salt)
        amount = PILOT_FUNDING[role]
        authorization_nonce = cast(
            "keccak", f"agent-bounties/{network}/pilot/{role}/funding-authorization/v1"
        ).lower()
        wallets.append(
            {
                "role": role,
                "owner": OWNER,
                "delegate": delegate,
                "user_salt": user_salt,
                "expected_contract": predicted,
                "initial_funding": str(amount),
                "funding_authorization_nonce": authorization_nonce,
                "policy": POLICY,
            }
        )
    return {
        "wallets": wallets,
        "funding_authorization": {
            "standard": "EIP-3009",
            "primary_type": "TransferWithAuthorization",
            "domain": {
                "name": USDC_EIP712_NAMES[network],
                "version": USDC_EIP712_VERSION,
                "verifying_contract": usdc,
            },
            "valid_after": "0",
            "max_validity_seconds": AUTHORIZATION_MAX_VALIDITY_SECONDS,
        },
    }


def network_bundle(
    name: str,
    chain_id: int,
    rpc_url: str,
    usdc: str,
    bounty_factory: str | None,
    verifier_module: str | None = None,
    verifier_difficulty: int = 8,
) -> dict:
    components = []
    bounty_implementation = MAINNET_IMPLEMENTATION if bounty_factory else None
    if bounty_factory is None:
        factory_init = append_abi(
            bytecode("AgentBountyFactory"), "constructor(address)", usdc
        )
        factory_component = create2_component(
            "AgentBountyFactory",
            f"agent-bounties/{name}/autonomous-v1/factory/v1",
            factory_init,
        )
        components.append(factory_component)
        bounty_factory = factory_component["expected_contract"]
        bounty_implementation = create_address(bounty_factory, 1)
        attach_runtime(
            factory_component,
            exact_runtime_code(
                "AgentBountyFactory",
                {
                    "settlementToken": usdc,
                    "implementation": bounty_implementation,
                },
            ),
        )
    bounty_factory_runtime = exact_runtime_code(
        "AgentBountyFactory",
        {
            "settlementToken": usdc,
            "implementation": bounty_implementation,
        },
    )
    verifier_component = None
    if verifier_module is None:
        verifier_init = append_abi(
            bytecode("LeadingZeroWorkVerifier"),
            "constructor(uint8)",
            str(verifier_difficulty),
        )
        verifier_component = create2_component(
            "LeadingZeroWorkVerifier",
            f"agent-bounties/{name}/leading-zero-verifier/{verifier_difficulty}/v1",
            verifier_init,
        )
        attach_runtime(
            verifier_component,
            exact_runtime_code(
                "LeadingZeroWorkVerifier", {"difficultyBits": verifier_difficulty}
            ),
        )
        components.append(verifier_component)
        verifier_module = verifier_component["expected_contract"]
    verifier_runtime = exact_runtime_code(
        "LeadingZeroWorkVerifier", {"difficultyBits": verifier_difficulty}
    )
    wallet_factory_init = append_abi(
        bytecode("BoundedAgentWalletFactory"),
        "constructor(address)",
        bounty_factory,
    )
    wallet_factory_component = create2_component(
        "BoundedAgentWalletFactory",
        f"agent-bounties/{name}/bounded-wallet-factory/v1",
        wallet_factory_init,
    )
    wallet_factory_runtime = exact_runtime_code(
        "BoundedAgentWalletFactory",
        {"bountyFactory": bounty_factory, "settlementToken": usdc},
    )
    attach_runtime(wallet_factory_component, wallet_factory_runtime)
    components.append(wallet_factory_component)
    setup = wallet_calls(
        name, wallet_factory_component["expected_contract"], bounty_factory, usdc
    )
    return {
        "network": name,
        "chain_id": chain_id,
        "chain_id_hex": hex(chain_id),
        "rpc_url": rpc_url,
        "native_usdc": usdc.lower(),
        "bounty_factory": bounty_factory,
        "bounty_implementation": bounty_implementation,
        "bounty_factory_runtime_code_hash": keccak(bounty_factory_runtime),
        "verifier_module": verifier_module,
        "verifier_difficulty_bits": verifier_difficulty,
        "verifier_runtime_code_hash": keccak(verifier_runtime),
        "wallet_factory": wallet_factory_component["expected_contract"],
        "wallet_factory_runtime_code_hash": keccak(wallet_factory_runtime),
        "wallet_runtime_code_hash": keccak(
            exact_runtime_code(
                "BoundedAgentWallet",
                {"factory": bounty_factory, "settlementToken": usdc},
            )
        ),
        "deployments": components,
        "pilot": {
            **setup,
            "owner": OWNER,
            "relayer": RELAYER,
            "relayer_eth_funding_wei": "100000000000000",
            "total_usdc_funding": "400000",
            "solver_reward": "200000",
            "verifier_reward": "100000",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        default=ROOT / "deployments" / "bounded-wallet-base-activation.json",
    )
    args = parser.parse_args()
    run([FORGE, "build"], cwd=CONTRACTS)
    revision = run(["git", "rev-parse", "HEAD"])
    bundle = {
        "schema_version": "agent-bounties/bounded-wallet-activation-v1",
        "protocol_version": "agent-bounties/bounded-wallet-v1",
        "source_revision": revision,
        "deterministic_deployer": {
            "contract": CREATE2_DEPLOYER,
            "runtime_code_hash": CREATE2_DEPLOYER_CODE_HASH,
            "runtime_code": CREATE2_DEPLOYER_RUNTIME_CODE,
        },
        "contracts": {
            name: {"source_sha256": source_sha256(name)}
            for name in (
                "AgentBounty",
                "AgentBountyFactory",
                "BoundedAgentWallet",
                "BoundedAgentWalletFactory",
                "LeadingZeroWorkVerifier",
            )
        },
        "networks": {
            "base-sepolia": network_bundle(
                "base-sepolia",
                84_532,
                "https://sepolia.base.org",
                "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
                None,
            ),
            "base-mainnet": network_bundle(
                "base-mainnet",
                8_453,
                "https://mainnet.base.org",
                "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
                MAINNET_FACTORY,
                MAINNET_VERIFIER,
                16,
            ),
        },
        "evidence_boundary": "This bundle pins source-derived deployment and onboarding transactions. It is not deployment, funding, authorization, completion, payout, or settlement evidence. Confirm exact safe-block code and canonical events after execution.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
