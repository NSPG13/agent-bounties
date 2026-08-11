#!/usr/bin/env python3
"""Build a pinned, unsigned Open Competition V2 Beta1 deployment bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from typing import Any

from _shared.evm import address_bytes, address_word, artifact_hex, create_address, keccak256, keccak_bytes
from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
CONTRACT_ROOT = ROOT / "contracts" / "base-escrow"
OUT = CONTRACT_ROOT / "out"
DEFAULT_DEPLOYER = "0x884834e884d6e93462655a2820140ad03e6747bc"
GROTH16_GATEWAY = "0x397a5f7f3dbd538f23de225b51f532c34448da9b"
PLONK_GATEWAY = "0x3b6041173b80e77f038f3f2c0f9744f04837185e"
GROTH16_VERIFIER = "0xb69f2584cbcff99a58c4e7002e8b89af54a6f4e2"
PLONK_VERIFIER = "0xc3c6dddac8829b233dc6536ec024775a57b0af2a"
GROTH16_SELECTOR = "0x4388a21c"
PLONK_SELECTOR = "0x5a093a2f"
METRIC_IDENTITY_PATH = ROOT / "programs/public-vector-metric-v1/release-identity.json"
METRIC_IDENTITY = json.loads(METRIC_IDENTITY_PATH.read_text(encoding="utf-8"))
PROGRAM_VKEY = METRIC_IDENTITY["program_vkey"]
SOURCE_HASH = METRIC_IDENTITY["source_hash"]
ELF_HASH = METRIC_IDENTITY["elf_keccak256"]
ELF_SHA256 = METRIC_IDENTITY["elf_sha256"]
JOURNAL_SCHEMA_HASH = "0xd9c492538aa0822e8a1d651886e79a2b8ddfc2c3428b3ed92e19d337eefe77d4"
METRIC_PROGRAM_HASH = "0x1c27fc20ab65264c7db2997c8b76f78d7291cdb91243481bcae1e88f77beb88a"
PROOF_SYSTEM_GROTH16 = keccak256(b"sp1-groth16")
PROOF_SYSTEM_PLONK = keccak256(b"sp1-plonk")
SP1_COMMIT = METRIC_IDENTITY["sp1_commit"]
SP1_VERSION = METRIC_IDENTITY["sp1_version"]
SP1_INSTALLER_URL = "https://sp1.succinct.xyz"
SP1_INSTALLER_SHA256 = "5f2b976287501d3f5feb62a2a96bbdfd1f5232c9badaf7547ed837c0366f3a7b"
SOLC_VERSION = "0.8.26+commit.8a97fa7a"
SOLC_IMAGE = (
    "docker.io/ethereum/solc@"
    "sha256:0158f0b11d4cd88556af7eff7b76e98c1c058d4a3153fae342e3a90b75358be4"
)
MIN_DEPLOYER_ETH_WEI = 100_000_000_000_000
CANARY_BUDGET = 525_000
PRELAUNCH_GATE_NAMES = (
    "repository_gate_complete",
    "isolated_sp1_builds_match",
    "static_analysis_triaged",
    "base_sepolia_groth16_first_proven_complete",
    "base_sepolia_plonk_best_score_complete",
    "base_sepolia_pooled_funding_complete",
    "base_sepolia_expiry_refunds_complete",
    "base_mainnet_fork_exact_replay_complete",
    "critical_and_high_findings_resolved",
    "mainnet_deployment_review_approved",
)
PUBLIC_BETA_GATE_NAMES = PRELAUNCH_GATE_NAMES + (
    "mainnet_groth16_canary_complete",
    "mainnet_plonk_canary_complete",
    "mainnet_canary_accounting_reconciled",
    "production_indexers_agree",
    "public_beta_activation_review_approved",
)
GRADUATION_GATE_NAMES = PUBLIC_BETA_GATE_NAMES + (
    "independent_reviews_complete",
    "independent_bytecode_and_vkey_reproduction_complete",
    "adversarial_review_regression_merged",
    "external_first_proven_paid_loop_complete",
    "external_best_score_paid_loop_complete",
    "external_poster_paid_loop_complete",
    "proof_job_accounting_reconciled",
    "external_positive_net_winner_complete",
    "unassisted_agent_instructions_complete",
    "graduation_review_approved",
)
REQUIRED_GATE_NAMES = GRADUATION_GATE_NAMES
GATE_MANIFEST_RELATIVE = "deployments/open-competition-v2-beta1-release-gates.json"
NETWORKS = {
    "base-mainnet": {
        "chain_id": 8453,
        "chain_id_hex": "0x2105",
        "rpc": "https://mainnet.base.org",
        "usdc": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
    },
    "base-sepolia": {
        "chain_id": 84532,
        "chain_id_hex": "0x14a34",
        "rpc": "https://sepolia.base.org",
        "usdc": "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
    },
}
SOURCE_FILES = (
    "contracts/base-escrow/src/IAgentBounty.sol",
    "contracts/base-escrow/src/OpenCompetitionBountyV2Beta1.sol",
    "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta1.sol",
    "contracts/base-escrow/src/Sp1VerifierAdapterV2Beta1.sol",
    "crates/competition-metric-core/src/lib.rs",
    "programs/public-vector-metric-v1/program/src/main.rs",
)


def normalized_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()


def source_tree_hash() -> str:
    digest = hashlib.sha256()
    for relative in SOURCE_FILES:
        data = (ROOT / relative).read_bytes().replace(b"\r\n", b"\n")
        digest.update(relative.encode())
        digest.update(b"\0")
        digest.update(str(len(data)).encode())
        digest.update(b"\0")
        digest.update(data)
    return "0x" + digest.hexdigest()


def verify_exact_checkout(source_commit: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    if head != source_commit:
        raise ValueError("source commit does not match the checked-out Git HEAD")
    diff_commands = (
        ["git", "diff", "--quiet", "--"],
        ["git", "diff", "--cached", "--quiet", "--"],
    )
    for args in diff_commands:
        result = subprocess.run(args, cwd=ROOT, check=False)
        if result.returncode == 1:
            raise ValueError("tracked worktree changes make the release source inexact")
        if result.returncode != 0:
            raise RuntimeError("Git could not verify the release worktree")


def repository_subject_hash(source_commit: str) -> str:
    """Hash the exact tracked tree while excluding only its mutable gate evidence."""
    raw = subprocess.check_output(
        ["git", "ls-tree", "-rz", "--full-tree", source_commit], cwd=ROOT
    )
    digest = hashlib.sha256()
    found_gate_manifest = False
    for entry in raw.split(b"\0"):
        if not entry:
            continue
        metadata, path = entry.split(b"\t", 1)
        if path.decode("utf-8") == GATE_MANIFEST_RELATIVE:
            found_gate_manifest = True
            continue
        digest.update(metadata)
        digest.update(b"\t")
        digest.update(path)
        digest.update(b"\0")
    if not found_gate_manifest:
        raise ValueError("release gate manifest is absent from the source commit")
    return "0x" + digest.hexdigest()


def artifact(source: str, contract: str) -> dict[str, Any]:
    path = OUT / f"{source}.sol" / f"{contract}.json"
    return json.loads(path.read_text(encoding="utf-8"))


def immutable_names(value: dict[str, Any]) -> dict[str, str]:
    references = value.get("deployedBytecode", {}).get("immutableReferences", {})
    wanted = set(references)
    result: dict[str, str] = {}

    def visit(node: Any) -> None:
        if isinstance(node, dict):
            node_id = str(node.get("id"))
            if node_id in wanted and node.get("nodeType") == "VariableDeclaration":
                result[node_id] = str(node.get("name"))
            for child in node.values():
                visit(child)
        elif isinstance(node, list):
            for child in node:
                visit(child)

    visit(value.get("ast"))
    if set(result) != wanted:
        raise ValueError("could not bind every immutable reference to a declaration")
    return result


def patch_runtime(value: dict[str, Any], replacements: dict[str, bytes], name: str) -> bytes:
    deployed = value.get("deployedBytecode")
    runtime = bytearray(artifact_hex(deployed, f"{name}.deployedBytecode"))
    references = deployed.get("immutableReferences") if isinstance(deployed, dict) else None
    if not isinstance(references, dict):
        raise ValueError(f"{name} immutable references are missing")
    names = immutable_names(value)
    if set(replacements) != set(names.values()):
        raise ValueError(f"{name} immutable replacement names changed")
    for ast_id, locations in references.items():
        replacement = replacements[names[ast_id]]
        if len(replacement) != 32:
            raise ValueError(f"{name}.{names[ast_id]} must be one ABI word")
        for location in locations:
            start = int(location["start"])
            length = int(location["length"])
            if length != 32 or start < 0 or start + length > len(runtime):
                raise ValueError(f"{name}.{names[ast_id]} immutable location is invalid")
            runtime[start : start + length] = replacement
    return bytes(runtime)


def bytes32(value: str) -> bytes:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 32:
        raise ValueError("bytes32 value required")
    return raw


def bytes4_word(value: str) -> bytes:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 4:
        raise ValueError("bytes4 value required")
    return raw.ljust(32, b"\0")


def selector(signature: str) -> bytes:
    return keccak_bytes(signature.encode())[:4]


def route_data(verifier_selector: str) -> str:
    return "0x" + (selector("routes(bytes4)") + bytes4_word(verifier_selector)).hex()


def decode_route(value: str) -> tuple[str, bool]:
    raw = bytes.fromhex(value.removeprefix("0x"))
    if len(raw) != 64:
        raise ValueError("SP1 route response must contain two ABI words")
    return "0x" + raw[12:32].hex(), bool(int.from_bytes(raw[32:], "big"))


def load_gates(path: Path, expected_subject_hash: str | None = None) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    gates = value.get("gates")
    evidence = value.get("evidence")
    if value.get("schema_version") != "agent-bounties/open-competition-v2-beta1-release-gates-v2":
        raise ValueError("release gate schema mismatch")
    if not isinstance(gates, dict) or set(gates) != set(REQUIRED_GATE_NAMES):
        raise ValueError("release gate inventory mismatch")
    if any(not isinstance(gates[name], bool) for name in REQUIRED_GATE_NAMES):
        raise ValueError("release gates must be booleans")
    if not isinstance(evidence, dict) or set(evidence) != set(REQUIRED_GATE_NAMES):
        raise ValueError("release gate evidence inventory mismatch")
    for name in REQUIRED_GATE_NAMES:
        item = evidence[name]
        if item is not None and not isinstance(item, dict):
            raise ValueError(f"release gate evidence must be an object or null: {name}")
        if not gates[name]:
            continue
        if not isinstance(item, dict):
            raise ValueError(f"completed release gate lacks evidence: {name}")
        source_commit = item.get("source_commit", "")
        subject_hash = item.get("subject_hash", "")
        evidence_hash = item.get("evidence_hash", "")
        uri = item.get("uri", "")
        if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
            raise ValueError(f"release gate evidence has invalid source commit: {name}")
        if not re.fullmatch(r"0x[0-9a-f]{64}", subject_hash):
            raise ValueError(f"release gate evidence has invalid subject hash: {name}")
        if expected_subject_hash is not None and subject_hash != expected_subject_hash:
            raise ValueError(f"release gate evidence targets another repository subject: {name}")
        if not re.fullmatch(r"0x[0-9a-f]{64}", evidence_hash):
            raise ValueError(f"release gate evidence has invalid hash: {name}")
        if not isinstance(uri, str) or not uri.startswith("https://"):
            raise ValueError(f"release gate evidence requires an HTTPS URI: {name}")
    expected_risk = keccak256(value["beta_risk_preimage"].encode())
    value["beta_risk_hash"] = expected_risk
    value["prelaunch_complete"] = all(gates[name] for name in PRELAUNCH_GATE_NAMES)
    value["public_beta_launch_complete"] = all(
        gates[name] for name in PUBLIC_BETA_GATE_NAMES
    )
    value["graduation_complete"] = all(
        gates[name] for name in GRADUATION_GATE_NAMES
    )
    return value


def online_preflight(network: dict[str, Any], rpc_url: str, deployer: str) -> dict[str, Any]:
    if rpc(rpc_url, "eth_chainId", []) != network["chain_id_hex"]:
        raise RuntimeError("RPC chain ID mismatch")
    block = rpc(rpc_url, "eth_getBlockByNumber", ["safe", False])
    if not block or not block.get("hash"):
        raise RuntimeError("RPC did not return a canonical safe block")
    tag = block["number"]
    dependencies = {
        "settlement_token": network["usdc"],
        "groth16_gateway": GROTH16_GATEWAY,
        "plonk_gateway": PLONK_GATEWAY,
        "groth16_verifier": GROTH16_VERIFIER,
        "plonk_verifier": PLONK_VERIFIER,
    }
    runtime_hashes: dict[str, str] = {}
    for name, address in dependencies.items():
        code = rpc(rpc_url, "eth_getCode", [address, tag])
        if code == "0x":
            raise RuntimeError(f"{name} bytecode is unavailable at the safe block")
        runtime_hashes[name] = keccak256(bytes.fromhex(code[2:]))
    for gateway, verifier_selector, expected in (
        (GROTH16_GATEWAY, GROTH16_SELECTOR, GROTH16_VERIFIER),
        (PLONK_GATEWAY, PLONK_SELECTOR, PLONK_VERIFIER),
    ):
        observed, frozen = decode_route(
            rpc(rpc_url, "eth_call", [{"to": gateway, "data": route_data(verifier_selector)}, tag])
        )
        if observed != expected or frozen:
            raise RuntimeError("canonical SP1 v6.1 route is unavailable")
    eth_balance = int(rpc(rpc_url, "eth_getBalance", [deployer, tag]), 16)
    nonce = int(rpc(rpc_url, "eth_getTransactionCount", [deployer, tag]), 16)
    balance_data = "0x70a08231" + address_word(deployer).hex()
    usdc_balance = int(
        rpc(rpc_url, "eth_call", [{"to": network["usdc"], "data": balance_data}, tag]), 16
    )
    return {
        "number": int(tag, 16),
        "hash": block["hash"].lower(),
        "timestamp": int(block["timestamp"], 16),
        "deployer_nonce": nonce,
        "deployer_eth_wei": eth_balance,
        "deployer_usdc_base_units": usdc_balance,
        "dependency_runtime_hashes": runtime_hashes,
    }


def build_bundle(
    *,
    network_name: str,
    deployer: str,
    source_commit: str,
    repository_subject: str,
    preflight: dict[str, Any],
    gates: dict[str, Any],
) -> dict[str, Any]:
    network = NETWORKS[network_name]
    deployer = deployer.lower()
    address_bytes(deployer)
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    if not re.fullmatch(r"0x[0-9a-f]{64}", repository_subject):
        raise ValueError("repository subject must be a 32-byte hash")
    if preflight["deployer_eth_wei"] < MIN_DEPLOYER_ETH_WEI:
        raise RuntimeError("deployer ETH is below the bounded deployment reserve")
    factory_address = create_address(deployer, preflight["deployer_nonce"])
    groth16_adapter = create_address(factory_address, 1)
    plonk_adapter = create_address(factory_address, 2)
    implementation = create_address(factory_address, 3)
    factory_artifact = artifact("OpenCompetitionBountyFactoryV2Beta1", "OpenCompetitionBountyFactoryV2Beta1")
    adapter_artifact = artifact("Sp1VerifierAdapterV2Beta1", "Sp1VerifierAdapterV2Beta1")
    bounty_artifact = artifact("OpenCompetitionBountyV2Beta1", "OpenCompetitionBountyV2Beta1")
    factory_data = artifact_hex(factory_artifact.get("bytecode"), "factory.bytecode") + b"".join(
        (address_word(network["usdc"]), address_word(GROTH16_GATEWAY), address_word(PLONK_GATEWAY))
    )
    factory_runtime = patch_runtime(
        factory_artifact,
        {
            "settlementToken": address_word(network["usdc"]),
            "groth16Adapter": address_word(groth16_adapter),
            "plonkAdapter": address_word(plonk_adapter),
            "implementation": address_word(implementation),
        },
        "factory",
    )
    adapter_bytecode = artifact_hex(adapter_artifact.get("bytecode"), "adapter.bytecode")
    implementation_runtime = artifact_hex(bounty_artifact.get("deployedBytecode"), "implementation.deployedBytecode")
    adapters: dict[str, Any] = {}
    for name, address, proof_system, gateway, route_selector, verifier in (
        ("groth16", groth16_adapter, PROOF_SYSTEM_GROTH16, GROTH16_GATEWAY, GROTH16_SELECTOR, GROTH16_VERIFIER),
        ("plonk", plonk_adapter, PROOF_SYSTEM_PLONK, PLONK_GATEWAY, PLONK_SELECTOR, PLONK_VERIFIER),
    ):
        init_code = adapter_bytecode + b"".join(
            (bytes32(proof_system), address_word(gateway), bytes4_word(route_selector), address_word(verifier))
        )
        runtime = patch_runtime(
            adapter_artifact,
            {
                "proofSystem": bytes32(proof_system),
                "gateway": address_word(gateway),
                "verifierSelector": bytes4_word(route_selector),
                "expectedVerifier": address_word(verifier),
            },
            f"{name}_adapter",
        )
        adapters[name] = {
            "address": address,
            "proof_system": proof_system,
            "gateway": gateway,
            "verifier_selector": route_selector,
            "expected_verifier": verifier,
            "creation_code_hash": keccak256(init_code),
            "runtime_code_hash": keccak256(runtime),
            "runtime_code_bytes": len(runtime),
        }
    source_hashes = {relative: normalized_sha256(ROOT / relative) for relative in SOURCE_FILES}
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta1-release-bundle-v1",
        "protocol_version": "agent-bounties/open-competition-v2-beta1",
        "network": network_name,
        "chain_id": network["chain_id"],
        "source_commit": source_commit,
        "repository_subject": {
            "algorithm": "sha256-git-ls-tree-v1",
            "excluded_paths": [GATE_MANIFEST_RELATIVE],
            "hash": repository_subject,
        },
        "source_tree_hash": source_tree_hash(),
        "source_sha256": source_hashes,
        "compiler": {
            "solc": SOLC_VERSION,
            "image": SOLC_IMAGE,
            "optimizer": True,
            "optimizer_runs": 200,
            "evm": "cancun",
        },
        "sp1": {
            "version": SP1_VERSION,
            "commit": SP1_COMMIT,
            "installer_url": SP1_INSTALLER_URL,
            "installer_sha256": SP1_INSTALLER_SHA256,
        },
        "metric_profile": {
            "profile_id": "public-vector-metric-v1",
            "program_vkey": PROGRAM_VKEY,
            "source_hash": SOURCE_HASH,
            "elf_hash": ELF_HASH,
            "elf_sha256": ELF_SHA256,
            "journal_schema_hash": JOURNAL_SCHEMA_HASH,
            "metric_program_hash": METRIC_PROGRAM_HASH,
        },
        "risk": {
            "preimage": gates["beta_risk_preimage"],
            "hash": gates["beta_risk_hash"],
            "acknowledgement_required": True,
        },
        "release_gates": gates,
        "deployment_state": (
            "graduated_default_ready"
            if gates["graduation_complete"]
            else "public_beta_ready"
            if gates["public_beta_launch_complete"]
            else "reviewed_ready_to_sign"
            if gates["prelaunch_complete"]
            else "blocked"
        ),
        "deployer": deployer,
        "preflight_safe_block": preflight,
        "settlement_token": network["usdc"],
        "factory": {
            "address": factory_address,
            "from_nonce": preflight["deployer_nonce"],
            "deployment_calldata": "0x" + factory_data.hex(),
            "creation_code_hash": keccak256(factory_data),
            "runtime_code_hash": keccak256(factory_runtime),
            "runtime_code_bytes": len(factory_runtime),
        },
        "groth16_adapter": adapters["groth16"],
        "plonk_adapter": adapters["plonk"],
        "implementation": {
            "address": implementation,
            "runtime_code_hash": keccak256(implementation_runtime),
            "runtime_code_bytes": len(implementation_runtime),
        },
        "mainnet_beta_canaries": [
            {"proof_system": "groth16", "winner_mode": "first_proven", "solver_reward": 250_000, "keeper_reward": 12_500, "funding": "pooled", "synthetic": True},
            {"proof_system": "plonk", "winner_mode": "best_score", "solver_reward": 250_000, "keeper_reward": 12_500, "entries": 2, "includes_byo_proof": True, "synthetic": True},
        ],
        "canary_budget": {
            "required_usdc_base_units": CANARY_BUDGET,
            "deployer_is_not_required_to_fund": True,
            "funding_source": "any external Base USDC wallet that acknowledges the exact Beta1 risk hash",
        },
        "activation": {
            "mainnet_signing_allowed": gates["prelaunch_complete"],
            "public_creation_enabled": gates["public_beta_launch_complete"],
            "default_protocol_enabled": gates["graduation_complete"],
            "synthetic_canaries_excluded_from_adoption_metrics": True,
        },
        "evidence_boundary": "This is unsigned release planning evidence. It is not deployment, proof acceptance, settlement, refund, payment, review, or public activation evidence.",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=tuple(NETWORKS), required=True)
    parser.add_argument("--deployer", default=DEFAULT_DEPLOYER)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--gates", type=Path, default=ROOT / "deployments/open-competition-v2-beta1-release-gates.json")
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    network = NETWORKS[args.network]
    deployer = args.deployer.lower()
    verify_exact_checkout(args.source_commit)
    subject_hash = repository_subject_hash(args.source_commit)
    preflight = online_preflight(network, args.rpc_url or network["rpc"], deployer)
    bundle = build_bundle(
        network_name=args.network,
        deployer=deployer,
        source_commit=args.source_commit,
        repository_subject=subject_hash,
        preflight=preflight,
        gates=load_gates(args.gates, subject_hash),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "factory": bundle["factory"]["address"], "state": bundle["deployment_state"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
