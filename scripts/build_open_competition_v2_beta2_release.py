#!/usr/bin/env python3
"""Build a pinned, unsigned Open Competition V2 Beta2 deployment bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from typing import Any

from _shared.evm import address_bytes, address_word, artifact_hex, create_address, keccak256
from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
CONTRACT_ROOT = ROOT / "contracts" / "base-escrow"
OUT = CONTRACT_ROOT / "out"
DEFAULT_DEPLOYER = "0x884834e884d6e93462655a2820140ad03e6747bc"
VERIFIER_ASSETS_PATH = ROOT / "deployments/open-competition-v2-beta2-verifier-assets.json"
SP1_SAFE_CIRCUIT_VERSION = "agent-bounties-sp1-safe-v2"
METRIC_IDENTITY_PATH = ROOT / "programs/public-vector-metric-v1/release-identity.json"
METRIC_IDENTITY = json.loads(METRIC_IDENTITY_PATH.read_text(encoding="utf-8"))
METRIC_REVIEW_EVIDENCE_HASH = keccak256(
    METRIC_IDENTITY_PATH.read_bytes().replace(b"\r\n", b"\n")
)
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
SOLC_VERSION = "0.8.26+commit.8a97fa7a"
SOLC_IMAGE = (
    "docker.io/ethereum/solc@"
    "sha256:0158f0b11d4cd88556af7eff7b76e98c1c058d4a3153fae342e3a90b75358be4"
)
MIN_DEPLOYER_ETH_WEI = 100_000_000_000_000
CANARY_BUDGET = 525_000
PRELAUNCH_GATE_NAMES = (
    "repository_gate_complete",
    "patched_sp1_dependency_graph_clean",
    "isolated_sp1_builds_match",
    "advisory_regression_vectors_complete",
    "real_groth16_plonk_proofs_self_verified",
    "static_analysis_triaged",
    "base_sepolia_groth16_first_proven_complete",
    "base_sepolia_plonk_best_score_complete",
    "base_sepolia_pooled_funding_complete",
    "base_sepolia_x402_and_byo_complete",
    "base_sepolia_expiry_refunds_complete",
    "base_sepolia_verifier_failure_refunds_complete",
    "base_mainnet_fork_exact_replay_complete",
    "critical_and_high_findings_resolved",
    "owner_mainnet_deployment_approved",
)
PUBLIC_BETA_GATE_NAMES = PRELAUNCH_GATE_NAMES + (
    "mainnet_verifiers_factory_deployed",
    "mainnet_groth16_canary_complete",
    "mainnet_plonk_canary_complete",
    "mainnet_canary_accounting_reconciled",
    "x402_success_and_refund_complete",
    "production_indexers_agree",
    "fresh_agent_wallet_flow_complete",
    "production_interfaces_agree",
    "owner_public_beta_activation_approved",
)
BROKER_CANARY_GATE_NAMES = PRELAUNCH_GATE_NAMES + (
    "mainnet_verifiers_factory_deployed",
    "production_indexers_agree",
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
GATE_MANIFEST_RELATIVE = "deployments/open-competition-v2-beta2-release-gates.json"
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
    "contracts/base-escrow/src/ISP1Verifier.sol",
    "contracts/base-escrow/src/OpenCompetitionBountyV2Beta2.sol",
    "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta2.sol",
    "contracts/base-escrow/src/Sp1VerifierAdapterV2Beta2.sol",
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


def strict_hex_bytes(value: object, field: str) -> bytes:
    if not isinstance(value, str) or not re.fullmatch(r"0x(?:[0-9a-f]{2})+", value):
        raise ValueError(f"{field} must be nonempty lowercase 0x-prefixed bytes")
    return bytes.fromhex(value[2:])


def load_verifier_assets(
    path: Path = VERIFIER_ASSETS_PATH, *, require_proof_evidence: bool = True
) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema_version") != "agent-bounties/open-competition-v2-beta2-verifier-assets-v1":
        raise ValueError("verifier asset schema mismatch")
    if value.get("circuit_version") != SP1_SAFE_CIRCUIT_VERSION:
        raise ValueError("verifier assets target another circuit version")
    if value.get("gpu_proving_enabled") is not False:
        raise ValueError("Beta2 verifier assets must come from the CPU-only release path")
    source_commit = value.get("sp1_source_commit", "")
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("verifier assets require an exact patched SP1 source commit")
    systems = value.get("proof_systems")
    if not isinstance(systems, dict) or set(systems) != {"groth16", "plonk"}:
        raise ValueError("verifier assets must contain exactly Groth16 and PLONK")
    for name, item in systems.items():
        if not isinstance(item, dict):
            raise ValueError(f"{name} verifier assets must be an object")
        verifier_hash = item.get("verifier_hash", "")
        bytes32(verifier_hash)
        creation = strict_hex_bytes(item.get("creation_code"), f"{name}.creation_code")
        runtime = strict_hex_bytes(item.get("runtime_code"), f"{name}.runtime_code")
        if item.get("creation_code_hash") != keccak256(creation):
            raise ValueError(f"{name} verifier creation code hash mismatch")
        if item.get("runtime_code_hash") != keccak256(runtime):
            raise ValueError(f"{name} verifier runtime code hash mismatch")
        if not verifier_hash.startswith("0x") or creation[:4].hex() == "":
            raise ValueError(f"{name} verifier identity is incomplete")
    proof_names = ("groth16_self_verified", "plonk_self_verified_1", "plonk_self_verified_2")
    proof_evidence = value.get("proof_evidence")
    if not isinstance(proof_evidence, dict) or set(proof_evidence) != set(proof_names):
        raise ValueError("verifier asset proof evidence inventory mismatch")
    complete = all(
        isinstance(proof_evidence[name], str)
        and re.fullmatch(r"0x[0-9a-f]{64}", proof_evidence[name])
        for name in proof_names
    )
    expected_state = "self_verified" if complete else "verifier_bytecode_only"
    if value.get("asset_state") != expected_state:
        raise ValueError("verifier asset state disagrees with proof evidence")
    if not complete and any(proof_evidence[name] is not None for name in proof_names):
        raise ValueError("pending verifier assets must not contain partial proof evidence")
    if require_proof_evidence and not complete:
        raise ValueError("verifier assets lack the three hash-bound self-verified proofs")
    return value


def load_gates(path: Path, expected_subject_hash: str | None = None) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    gates = value.get("gates")
    evidence = value.get("evidence")
    if value.get("schema_version") != "agent-bounties/open-competition-v2-beta2-release-gates-v3":
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
    value["broker_canary_ready"] = all(gates[name] for name in BROKER_CANARY_GATE_NAMES)
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
    dependencies = {"settlement_token": network["usdc"]}
    runtime_hashes: dict[str, str] = {}
    for name, address in dependencies.items():
        code = rpc(rpc_url, "eth_getCode", [address, tag])
        if code == "0x":
            raise RuntimeError(f"{name} bytecode is unavailable at the safe block")
        runtime_hashes[name] = keccak256(bytes.fromhex(code[2:]))
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
    verifier_assets: dict[str, Any],
    allow_pending_metric_identity: bool = False,
) -> dict[str, Any]:
    network = NETWORKS[network_name]
    deployer = deployer.lower()
    address_bytes(deployer)
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    if not re.fullmatch(r"0x[0-9a-f]{64}", repository_subject):
        raise ValueError("repository subject must be a 32-byte hash")
    if METRIC_IDENTITY.get("status") != "reproduced_beta2" and not allow_pending_metric_identity:
        raise RuntimeError("patched Beta2 metric ELF and vkey have not been reproduced")
    if preflight["deployer_eth_wei"] < MIN_DEPLOYER_ETH_WEI:
        raise RuntimeError("deployer ETH is below the bounded deployment reserve")
    groth16_verifier = create_address(deployer, preflight["deployer_nonce"])
    plonk_verifier = create_address(deployer, preflight["deployer_nonce"] + 1)
    factory_address = create_address(deployer, preflight["deployer_nonce"] + 2)
    groth16_adapter = create_address(factory_address, 1)
    plonk_adapter = create_address(factory_address, 2)
    implementation = create_address(factory_address, 3)
    factory_artifact = artifact("OpenCompetitionBountyFactoryV2Beta2", "OpenCompetitionBountyFactoryV2Beta2")
    adapter_artifact = artifact("Sp1VerifierAdapterV2Beta2", "Sp1VerifierAdapterV2Beta2")
    bounty_artifact = artifact("OpenCompetitionBountyV2Beta2", "OpenCompetitionBountyV2Beta2")
    systems = verifier_assets["proof_systems"]
    if verifier_assets["sp1_source_commit"] != SP1_COMMIT:
        raise ValueError("metric identity and verifier assets use different SP1 source commits")
    groth16_verifier_hash = systems["groth16"]["verifier_hash"]
    plonk_verifier_hash = systems["plonk"]["verifier_hash"]
    groth16_verifier_runtime = strict_hex_bytes(
        systems["groth16"]["runtime_code"], "groth16.runtime_code"
    )
    plonk_verifier_runtime = strict_hex_bytes(
        systems["plonk"]["runtime_code"], "plonk.runtime_code"
    )
    groth16_runtime_hash = keccak256(groth16_verifier_runtime)
    plonk_runtime_hash = keccak256(plonk_verifier_runtime)
    factory_data = artifact_hex(factory_artifact.get("bytecode"), "factory.bytecode") + b"".join(
        (
            address_word(network["usdc"]),
            address_word(groth16_verifier),
            bytes32(groth16_verifier_hash),
            bytes32(groth16_runtime_hash),
            address_word(plonk_verifier),
            bytes32(plonk_verifier_hash),
            bytes32(plonk_runtime_hash),
        )
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
    for name, address, proof_system, verifier, verifier_hash, runtime_hash in (
        (
            "groth16",
            groth16_adapter,
            PROOF_SYSTEM_GROTH16,
            groth16_verifier,
            groth16_verifier_hash,
            groth16_runtime_hash,
        ),
        (
            "plonk",
            plonk_adapter,
            PROOF_SYSTEM_PLONK,
            plonk_verifier,
            plonk_verifier_hash,
            plonk_runtime_hash,
        ),
    ):
        init_code = adapter_bytecode + b"".join(
            (
                bytes32(proof_system),
                address_word(verifier),
                bytes32(verifier_hash),
                bytes32(runtime_hash),
            )
        )
        runtime = patch_runtime(
            adapter_artifact,
            {
                "proofSystem": bytes32(proof_system),
                "verifier": address_word(verifier),
                "verifierHash": bytes32(verifier_hash),
                "expectedRuntimeCodeHash": bytes32(runtime_hash),
            },
            f"{name}_adapter",
        )
        adapters[name] = {
            "address": address,
            "proof_system": proof_system,
            "verifier": verifier,
            "verifier_hash": verifier_hash,
            "verifier_runtime_code_hash": runtime_hash,
            "creation_code_hash": keccak256(init_code),
            "runtime_code_hash": keccak256(runtime),
            "runtime_code_bytes": len(runtime),
        }
    source_hashes = {relative: normalized_sha256(ROOT / relative) for relative in SOURCE_FILES}
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta2-release-bundle-v1",
        "protocol_version": "agent-bounties/open-competition-v2-beta2",
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
            "patched_source_commit": verifier_assets["sp1_source_commit"],
            "circuit_version": verifier_assets["circuit_version"],
            "gpu_proving_enabled": False,
            "proof_evidence": verifier_assets["proof_evidence"],
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
            "from_nonce": preflight["deployer_nonce"] + 2,
            "deployment_calldata": "0x" + factory_data.hex(),
            "creation_code_hash": keccak256(factory_data),
            "runtime_code_hash": keccak256(factory_runtime),
            "runtime_code_bytes": len(factory_runtime),
        },
        "groth16_verifier": {
            "address": groth16_verifier,
            "from_nonce": preflight["deployer_nonce"],
            "verifier_hash": groth16_verifier_hash,
            "deployment_calldata": systems["groth16"]["creation_code"],
            "creation_code_hash": systems["groth16"]["creation_code_hash"],
            "runtime_code_hash": groth16_runtime_hash,
            "runtime_code_bytes": len(groth16_verifier_runtime),
        },
        "plonk_verifier": {
            "address": plonk_verifier,
            "from_nonce": preflight["deployer_nonce"] + 1,
            "verifier_hash": plonk_verifier_hash,
            "deployment_calldata": systems["plonk"]["creation_code"],
            "creation_code_hash": systems["plonk"]["creation_code_hash"],
            "runtime_code_hash": plonk_runtime_hash,
            "runtime_code_bytes": len(plonk_verifier_runtime),
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
            "funding_source": "any external Base USDC wallet that acknowledges the exact Beta2 risk hash",
        },
        "activation": {
            "mainnet_signing_allowed": gates["prelaunch_complete"],
            "broker_canary_enabled": gates["broker_canary_ready"],
            "public_creation_enabled": gates["public_beta_launch_complete"],
            "default_protocol_enabled": gates["graduation_complete"],
            "synthetic_canaries_excluded_from_adoption_metrics": True,
        },
        "deployment_transactions": [
            {
                "component": "groth16_verifier",
                "from_nonce": preflight["deployer_nonce"],
                "predicted_address": groth16_verifier,
                "data": systems["groth16"]["creation_code"],
            },
            {
                "component": "plonk_verifier",
                "from_nonce": preflight["deployer_nonce"] + 1,
                "predicted_address": plonk_verifier,
                "data": systems["plonk"]["creation_code"],
            },
            {
                "component": "factory",
                "from_nonce": preflight["deployer_nonce"] + 2,
                "predicted_address": factory_address,
                "data": "0x" + factory_data.hex(),
            },
        ],
        "evidence_boundary": "This is unsigned release planning evidence. It is not deployment, proof acceptance, settlement, refund, payment, review, or public activation evidence.",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--network", choices=tuple(NETWORKS), required=True)
    parser.add_argument("--deployer", default=DEFAULT_DEPLOYER)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--rpc-url")
    parser.add_argument("--gates", type=Path, default=ROOT / "deployments/open-competition-v2-beta2-release-gates.json")
    parser.add_argument("--verifier-assets", type=Path, default=VERIFIER_ASSETS_PATH)
    parser.add_argument("--allow-pending-proof-evidence", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runtime-manifest-output", type=Path)
    return parser.parse_args()


def runtime_manifest(bundle: dict[str, Any], deployment_block: int = 0) -> dict[str, Any]:
    metric_ready = METRIC_IDENTITY.get("status") == "reproduced_beta2"
    public_beta = bool(bundle["activation"]["public_creation_enabled"])
    broker_canary = bool(bundle["activation"]["broker_canary_enabled"])
    return {
        "protocol_version": bundle["protocol_version"],
        "network": bundle["network"],
        "source_commit": bundle["source_commit"],
        "repository_subject_hash": bundle["repository_subject"]["hash"],
        "sp1_source_commit": bundle["sp1"]["patched_source_commit"],
        "sp1_circuit_version": bundle["sp1"]["circuit_version"],
        "factory_contract": bundle["factory"]["address"],
        "factory_runtime_code_hash": bundle["factory"]["runtime_code_hash"],
        "implementation_contract": bundle["implementation"]["address"],
        "implementation_runtime_code_hash": bundle["implementation"]["runtime_code_hash"],
        "settlement_token": bundle["settlement_token"],
        "groth16_verifier": bundle["groth16_verifier"]["address"],
        "groth16_verifier_hash": bundle["groth16_verifier"]["verifier_hash"],
        "groth16_verifier_runtime_code_hash": bundle["groth16_verifier"]["runtime_code_hash"],
        "groth16_adapter": bundle["groth16_adapter"]["address"],
        "groth16_adapter_runtime_code_hash": bundle["groth16_adapter"]["runtime_code_hash"],
        "plonk_verifier": bundle["plonk_verifier"]["address"],
        "plonk_verifier_hash": bundle["plonk_verifier"]["verifier_hash"],
        "plonk_verifier_runtime_code_hash": bundle["plonk_verifier"]["runtime_code_hash"],
        "plonk_adapter": bundle["plonk_adapter"]["address"],
        "plonk_adapter_runtime_code_hash": bundle["plonk_adapter"]["runtime_code_hash"],
        "deployment_block": deployment_block,
        "release_hash": bundle["source_tree_hash"],
        "beta_risk_hash": bundle["risk"]["hash"],
        "public_creation_enabled": public_beta,
        "proof_broker_enabled": broker_canary and metric_ready,
        "metric_programs": [
            {
                "profile_id": bundle["metric_profile"]["profile_id"],
                "classification": "reviewed" if metric_ready else "disabled",
                "program_vkey": bundle["metric_profile"]["program_vkey"],
                "source_hash": bundle["metric_profile"]["source_hash"],
                "elf_hash": bundle["metric_profile"]["elf_hash"],
                "journal_schema_hash": bundle["metric_profile"]["journal_schema_hash"],
                "metric_program_hash": bundle["metric_profile"]["metric_program_hash"],
                "review_evidence_hash": METRIC_REVIEW_EVIDENCE_HASH,
            }
        ],
    }


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
        verifier_assets=load_verifier_assets(
            args.verifier_assets,
            require_proof_evidence=not args.allow_pending_proof_evidence,
        ),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    runtime_manifest_value = runtime_manifest(bundle)
    runtime_output = args.runtime_manifest_output or args.output.with_suffix(".runtime.json")
    runtime_output.write_text(json.dumps(runtime_manifest_value, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "factory": bundle["factory"]["address"], "state": bundle["deployment_state"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
