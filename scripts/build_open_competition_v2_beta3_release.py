#!/usr/bin/env python3
"""Build a pinned, unsigned Open Competition V2 Beta3 deployment bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
from typing import Any

from _shared.evm import address_bytes, address_word, artifact_hex, create_address, keccak256
from _shared.rpc import rpc


ROOT = Path(__file__).resolve().parents[1]
CONTRACT_ROOT = ROOT / "contracts" / "base-escrow"
OUT = CONTRACT_ROOT / "out"
DEFAULT_DEPLOYER = "0xfd7be4c69541ab297aece2a674fc1418b898cc0a"
PROTOCOL_VERSION = "agent-bounties/open-competition-v2-beta3"
VERIFIER_ASSETS_PATH = ROOT / "deployments/open-competition-v2-beta3-verifier-assets.json"
SP1_SAFE_CIRCUIT_VERSION = "agent-bounties-sp1-safe-v5"
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
STRUCTURED_ARTIFACT_IDENTITY_PATH = (
    ROOT / "programs/structured-artifact-metric-v1/release-identity.json"
)
STRUCTURED_ARTIFACT_IDENTITY = json.loads(
    STRUCTURED_ARTIFACT_IDENTITY_PATH.read_text(encoding="utf-8")
)
STRUCTURED_ARTIFACT_REVIEW_EVIDENCE_HASH = keccak256(
    STRUCTURED_ARTIFACT_IDENTITY_PATH.read_bytes().replace(b"\r\n", b"\n")
)
CANONICAL_GMV_IDENTITY_PATH = (
    ROOT / "programs/forward-canonical-gmv-attribution-metric-v2/release-identity.json"
)
CANONICAL_GMV_IDENTITY = json.loads(
    CANONICAL_GMV_IDENTITY_PATH.read_text(encoding="utf-8")
)
CANONICAL_GMV_REVIEW_EVIDENCE_HASH = keccak256(
    CANONICAL_GMV_IDENTITY_PATH.read_bytes().replace(b"\r\n", b"\n")
)
METRIC_PROFILES = (
    {
        "identity": METRIC_IDENTITY,
        "identity_path": METRIC_IDENTITY_PATH,
        "journal_schema_hash": JOURNAL_SCHEMA_HASH,
        "metric_program_hash": METRIC_PROGRAM_HASH,
        "review_evidence_hash": METRIC_REVIEW_EVIDENCE_HASH,
    },
    {
        "identity": STRUCTURED_ARTIFACT_IDENTITY,
        "identity_path": STRUCTURED_ARTIFACT_IDENTITY_PATH,
        "journal_schema_hash": "0x63c02a04ca74b569649c9374b088b08d90fb1e85d2be0d1e0ca141307938fb0d",
        "metric_program_hash": "0x760b8c342a91b4c215b8f102c85b696e70073a98c62a87987d2930eadbeb22b9",
        "review_evidence_hash": STRUCTURED_ARTIFACT_REVIEW_EVIDENCE_HASH,
    },
    {
        "identity": CANONICAL_GMV_IDENTITY,
        "identity_path": CANONICAL_GMV_IDENTITY_PATH,
        "journal_schema_hash": "0x660ddc720ea9fc13e7bbdd88839a2ac7b19a124e5daf046518350fa6febe8a40",
        "metric_program_hash": "0xe1b52ffcfff0675b7dacea84dcabdf3fbcf1cde09b3d2fb55aa389acac5c2ff9",
        "review_evidence_hash": CANONICAL_GMV_REVIEW_EVIDENCE_HASH,
    },
)
PROOF_SYSTEM_GROTH16 = keccak256(b"sp1-groth16")
PROOF_SYSTEM_PLONK = keccak256(b"sp1-plonk")
SP1_COMMIT = METRIC_IDENTITY["sp1_commit"]
SP1_RUNTIME_COMMIT = METRIC_IDENTITY["sp1_runtime_commit"]
SP1_VERSION = METRIC_IDENTITY["sp1_version"]
HOST_RUST_VERSION = METRIC_IDENTITY["rust_version"]
SP1_GUEST_RUST_VERSION = METRIC_IDENTITY["sp1_guest_rust_version"]
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
    "trusted_setup_provenance_complete",
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
    "broker_refund_reserve_ready",
    "x402_success_and_refund_complete",
    "production_indexers_agree",
    "fresh_agent_wallet_flow_complete",
    "production_interfaces_agree",
    "owner_public_beta_activation_approved",
)
BROKER_CANARY_GATE_NAMES = PRELAUNCH_GATE_NAMES + (
    "mainnet_verifiers_factory_deployed",
    "production_indexers_agree",
    "broker_refund_reserve_ready",
)
SEPOLIA_BROKER_REHEARSAL_GATE_NAMES = (
    "repository_gate_complete",
    "patched_sp1_dependency_graph_clean",
    "isolated_sp1_builds_match",
    "advisory_regression_vectors_complete",
    "real_groth16_plonk_proofs_self_verified",
    "static_analysis_triaged",
    "base_mainnet_fork_exact_replay_complete",
    "critical_and_high_findings_resolved",
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
GATE_MANIFEST_RELATIVE = "deployments/open-competition-v2-beta3-release-gates.json"
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
    "contracts/base-escrow/src/OpenCompetitionBountyV2Beta3.sol",
    "contracts/base-escrow/src/OpenCompetitionBountyFactoryV2Beta3.sol",
    "contracts/base-escrow/src/Sp1VerifierAdapterV2Beta3.sol",
    "crates/competition-metric-core/src/lib.rs",
    "programs/public-vector-metric-v1/program/src/main.rs",
    "programs/structured-artifact-metric-v1/program/src/main.rs",
    "programs/forward-canonical-gmv-attribution-metric-v2/program/src/main.rs",
)


def metric_profiles_ready() -> bool:
    return all(profile["identity"].get("status") == "reproduced_beta3" for profile in METRIC_PROFILES)


def metric_profile_documents() -> list[dict[str, str]]:
    for profile in METRIC_PROFILES:
        identity = profile["identity"]
        if (
            identity.get("sp1_commit") != SP1_COMMIT
            or identity.get("sp1_version") != SP1_VERSION
            or identity.get("rust_version") != HOST_RUST_VERSION
            or identity.get("sp1_guest_rust_version") != SP1_GUEST_RUST_VERSION
        ):
            raise ValueError("metric profiles must share the exact patched SP1 and Rust toolchains")
    return [
        {
            "profile_id": profile["identity"]["profile_id"],
            "program_vkey": profile["identity"]["program_vkey"],
            "source_hash": profile["identity"]["source_hash"],
            "elf_hash": profile["identity"]["elf_keccak256"],
            "elf_sha256": profile["identity"]["elf_sha256"],
            "journal_schema_hash": profile["journal_schema_hash"],
            "metric_program_hash": profile["metric_program_hash"],
            "review_evidence_hash": profile["review_evidence_hash"],
        }
        for profile in METRIC_PROFILES
    ]


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


def configure_release_root(path: Path) -> None:
    global ROOT, CONTRACT_ROOT, OUT
    ROOT = path.resolve()
    CONTRACT_ROOT = ROOT / "contracts" / "base-escrow"
    OUT = CONTRACT_ROOT / "out"


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
    if value.get("schema_version") != "agent-bounties/open-competition-v2-beta3-verifier-assets-v2":
        raise ValueError("verifier asset schema mismatch")
    if value.get("circuit_version") != SP1_SAFE_CIRCUIT_VERSION:
        raise ValueError("verifier assets target another circuit version")
    if value.get("gpu_proving_enabled") is not False:
        raise ValueError("Beta3 verifier assets must come from the CPU-only release path")
    setup = value.get("setup_provenance")
    if not isinstance(setup, dict):
        raise ValueError("verifier assets lack setup provenance")
    setup_state = setup.get("state")
    if setup_state not in {"trusted_mpc", "test_only_unsafe"}:
        raise ValueError("verifier setup provenance state is invalid")
    if setup.get("mainnet_eligible") is not (setup_state == "trusted_mpc"):
        raise ValueError("verifier setup mainnet eligibility is inconsistent")
    manifest_hash = setup.get("manifest_sha256")
    if setup_state == "trusted_mpc":
        if not re.fullmatch(r"0x[0-9a-f]{64}", str(manifest_hash)):
            raise ValueError("trusted setup manifest hash is invalid")
        setup_systems = setup.get("systems")
        expected_models = {
            "groth16": "mpc_phase2",
            "plonk": "public_mpc_kzg_srs",
        }
        if not isinstance(setup_systems, dict) or set(setup_systems) != set(expected_models):
            raise ValueError("trusted setup proof-system inventory mismatch")
        for name, model in expected_models.items():
            item = setup_systems[name]
            if not isinstance(item, dict) or item.get("security_model") != model:
                raise ValueError(f"{name} trusted setup security model is invalid")
            if item.get("verification_passed") is not True:
                raise ValueError(f"{name} trusted setup verification is incomplete")
            if item.get("verifier_hash") != value.get("proof_systems", {}).get(name, {}).get("verifier_hash"):
                raise ValueError(f"{name} trusted setup verifier binding mismatch")
            for field in (
                "constraint_system_sha256",
                "proving_key_sha256",
                "verifying_key_sha256",
                "transcript_sha256",
                "verification_evidence_sha256",
            ):
                if not re.fullmatch(r"[0-9a-f]{64}", str(item.get(field, ""))):
                    raise ValueError(f"{name} trusted setup {field} is invalid")
            if int(item.get("contribution_count", 0)) < 2:
                raise ValueError(f"{name} trusted setup lacks multi-party contributions")
    elif manifest_hash is not None:
        raise ValueError("test-only setup cannot claim a trusted manifest")
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
    observed_subject_hashes: set[str] = set()
    if value.get("schema_version") != "agent-bounties/open-competition-v2-beta3-release-gates-v5":
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
        observed_subject_hashes.add(subject_hash)
        if not re.fullmatch(r"0x[0-9a-f]{64}", evidence_hash):
            raise ValueError(f"release gate evidence has invalid hash: {name}")
        if not isinstance(uri, str) or not uri.startswith("https://"):
            raise ValueError(f"release gate evidence requires an HTTPS URI: {name}")
    expected_risk = keccak256(value["beta_risk_preimage"].encode())
    if len(observed_subject_hashes) > 1:
        raise ValueError("completed release gates target multiple repository subjects")
    value["subject_hash"] = (
        expected_subject_hash
        if expected_subject_hash is not None
        else next(iter(observed_subject_hashes), None)
    )
    value["beta_risk_hash"] = expected_risk
    value["prelaunch_complete"] = all(gates[name] for name in PRELAUNCH_GATE_NAMES)
    value["public_beta_launch_complete"] = all(
        gates[name] for name in PUBLIC_BETA_GATE_NAMES
    )
    value["broker_canary_ready"] = all(gates[name] for name in BROKER_CANARY_GATE_NAMES)
    value["sepolia_broker_rehearsal_ready"] = all(
        gates[name] for name in SEPOLIA_BROKER_REHEARSAL_GATE_NAMES
    )
    value["graduation_complete"] = all(
        gates[name] for name in GRADUATION_GATE_NAMES
    )
    return value


def activation_state(network_name: str, gates: dict[str, Any]) -> dict[str, bool]:
    return {
        "mainnet_signing_allowed": gates["prelaunch_complete"],
        "broker_canary_enabled": gates["broker_canary_ready"],
        "sepolia_broker_rehearsal_enabled": (
            network_name == "base-sepolia"
            and gates["sepolia_broker_rehearsal_ready"]
        ),
        "public_creation_enabled": gates["public_beta_launch_complete"],
        "default_protocol_enabled": gates["graduation_complete"],
        "synthetic_canaries_excluded_from_adoption_metrics": True,
    }


def online_preflight(network: dict[str, Any], rpc_url: str, deployer: str) -> dict[str, Any]:
    def read(method: str, params: list[Any]) -> Any:
        return rpc(
            rpc_url,
            method,
            params,
            attempts=5,
            retry_delay=1,
        )

    if read("eth_chainId", []) != network["chain_id_hex"]:
        raise RuntimeError("RPC chain ID mismatch")
    block = read("eth_getBlockByNumber", ["safe", False])
    if not block or not block.get("hash"):
        raise RuntimeError("RPC did not return a canonical safe block")
    tag = block["number"]
    dependencies = {"settlement_token": network["usdc"]}
    runtime_hashes: dict[str, str] = {}
    for name, address in dependencies.items():
        code = read("eth_getCode", [address, tag])
        if code == "0x":
            raise RuntimeError(f"{name} bytecode is unavailable at the safe block")
        runtime_hashes[name] = keccak256(bytes.fromhex(code[2:]))
    eth_balance = int(read("eth_getBalance", [deployer, tag]), 16)
    nonce = int(read("eth_getTransactionCount", [deployer, tag]), 16)
    balance_data = "0x70a08231" + address_word(deployer).hex()
    usdc_balance = int(
        read("eth_call", [{"to": network["usdc"], "data": balance_data}, tag]), 16
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


def resume_exact_verifier_pair(
    *,
    deployer: str,
    observed_nonce: int,
    code_hash: Any,
    groth16_runtime_hash: str,
    plonk_runtime_hash: str,
) -> int:
    """Reuse an exact verifier pair when a prior factory deployment was interrupted."""
    if observed_nonce < 2:
        return observed_nonce
    minimum_nonce = max(0, observed_nonce - 16)
    for start_nonce in range(observed_nonce - 2, minimum_nonce - 1, -1):
        groth16 = create_address(deployer, start_nonce)
        plonk = create_address(deployer, start_nonce + 1)
        if (
            code_hash(groth16) == groth16_runtime_hash
            and code_hash(plonk) == plonk_runtime_hash
        ):
            return start_nonce
    return observed_nonce


def apply_partial_deployment_resume(
    *,
    preflight: dict[str, Any],
    rpc_url: str,
    deployer: str,
    verifier_assets: dict[str, Any],
) -> dict[str, Any]:
    observed_nonce = int(preflight["deployer_nonce"])
    safe_block = hex(int(preflight["number"]))

    def code_hash(address: str) -> str | None:
        code = rpc(
            rpc_url,
            "eth_getCode",
            [address, safe_block],
            attempts=5,
            retry_delay=1,
        )
        return None if code == "0x" else keccak256(bytes.fromhex(code[2:]))

    systems = verifier_assets["proof_systems"]
    start_nonce = resume_exact_verifier_pair(
        deployer=deployer,
        observed_nonce=observed_nonce,
        code_hash=code_hash,
        groth16_runtime_hash=systems["groth16"]["runtime_code_hash"],
        plonk_runtime_hash=systems["plonk"]["runtime_code_hash"],
    )
    value = dict(preflight)
    value["observed_deployer_nonce"] = observed_nonce
    value["deployer_nonce"] = start_nonce
    value["resuming_exact_verifiers"] = start_nonce != observed_nonce
    return value


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
    allow_test_only_setup: bool = False,
) -> dict[str, Any]:
    network = NETWORKS[network_name]
    deployer = deployer.lower()
    address_bytes(deployer)
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("source commit must be a full lowercase Git commit")
    if not re.fullmatch(r"0x[0-9a-f]{64}", repository_subject):
        raise ValueError("repository subject must be a 32-byte hash")
    if not metric_profiles_ready() and not allow_pending_metric_identity:
        raise RuntimeError("every patched Beta3 metric ELF and vkey must be reproduced")
    if preflight["deployer_eth_wei"] < MIN_DEPLOYER_ETH_WEI:
        raise RuntimeError("deployer ETH is below the bounded deployment reserve")
    groth16_verifier = create_address(deployer, preflight["deployer_nonce"])
    plonk_verifier = create_address(deployer, preflight["deployer_nonce"] + 1)
    factory_address = create_address(deployer, preflight["deployer_nonce"] + 2)
    groth16_adapter = create_address(factory_address, 1)
    plonk_adapter = create_address(factory_address, 2)
    implementation = create_address(factory_address, 3)
    factory_artifact = artifact("OpenCompetitionBountyFactoryV2Beta3", "OpenCompetitionBountyFactoryV2Beta3")
    adapter_artifact = artifact("Sp1VerifierAdapterV2Beta3", "Sp1VerifierAdapterV2Beta3")
    bounty_artifact = artifact("OpenCompetitionBountyV2Beta3", "OpenCompetitionBountyV2Beta3")
    systems = verifier_assets["proof_systems"]
    trusted_setup = verifier_assets["setup_provenance"]
    if (
        network_name == "base-mainnet"
        and trusted_setup["mainnet_eligible"] is not True
        and not allow_test_only_setup
    ):
        raise RuntimeError("Base mainnet requires hash-bound trusted setup provenance")
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
        "schema_version": "agent-bounties/open-competition-v2-beta3-release-bundle-v1",
        "protocol_version": PROTOCOL_VERSION,
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
            "circuit_commit": SP1_COMMIT,
            "runtime_commit": SP1_RUNTIME_COMMIT,
            "host_rust_version": HOST_RUST_VERSION,
            "guest_rust_version": SP1_GUEST_RUST_VERSION,
            "patched_source_commit": verifier_assets["sp1_source_commit"],
            "circuit_version": verifier_assets["circuit_version"],
            "gpu_proving_enabled": False,
            "proof_evidence": verifier_assets["proof_evidence"],
            "setup_provenance": trusted_setup,
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
        "metric_profiles": metric_profile_documents(),
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
            "funding_source": "any external Base USDC wallet that acknowledges the exact Beta3 risk hash",
        },
        "activation": activation_state(network_name, gates),
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
    parser.add_argument("--gates", type=Path, default=ROOT / "deployments/open-competition-v2-beta3-release-gates.json")
    parser.add_argument("--verifier-assets", type=Path, default=VERIFIER_ASSETS_PATH)
    parser.add_argument("--allow-pending-proof-evidence", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runtime-manifest-output", type=Path)
    parser.add_argument(
        "--release-root",
        type=Path,
        help="Exact frozen release checkout to hash and read compiled artifacts from",
    )
    return parser.parse_args()


def runtime_manifest(bundle: dict[str, Any], deployment_block: int = 0) -> dict[str, Any]:
    metric_ready = metric_profiles_ready()
    public_beta = bool(bundle["activation"]["public_creation_enabled"])
    broker_canary = bool(bundle["activation"]["broker_canary_enabled"])
    sepolia_broker_rehearsal = bool(
        bundle["activation"].get("sepolia_broker_rehearsal_enabled", False)
    )
    return {
        "protocol_version": bundle["protocol_version"],
        "network": bundle["network"],
        "source_commit": bundle["source_commit"],
        "repository_subject_hash": bundle["repository_subject"]["hash"],
        "sp1_source_commit": bundle["sp1"]["patched_source_commit"],
        "sp1_runtime_commit": bundle["sp1"]["runtime_commit"],
        "sp1_circuit_version": bundle["sp1"]["circuit_version"],
        "sp1_host_rust_version": bundle["sp1"]["host_rust_version"],
        "sp1_guest_rust_version": bundle["sp1"]["guest_rust_version"],
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
        "proof_broker_enabled": (
            broker_canary or sepolia_broker_rehearsal
        ) and metric_ready,
        "metric_programs": [
            {
                "profile_id": profile["profile_id"],
                "classification": "reviewed" if metric_ready else "disabled",
                "program_vkey": profile["program_vkey"],
                "source_hash": profile["source_hash"],
                "elf_hash": profile["elf_hash"],
                "journal_schema_hash": profile["journal_schema_hash"],
                "metric_program_hash": profile["metric_program_hash"],
                "review_evidence_hash": profile["review_evidence_hash"],
            }
            for profile in bundle["metric_profiles"]
        ],
    }


def main() -> int:
    args = parse_args()
    if args.release_root is not None:
        configure_release_root(args.release_root)
    network = NETWORKS[args.network]
    deployer = args.deployer.lower()
    verify_exact_checkout(args.source_commit)
    subject_hash = repository_subject_hash(args.source_commit)
    rpc_url = args.rpc_url or network["rpc"]
    verifier_assets = load_verifier_assets(
        args.verifier_assets,
        require_proof_evidence=not args.allow_pending_proof_evidence,
    )
    preflight = apply_partial_deployment_resume(
        preflight=online_preflight(network, rpc_url, deployer),
        rpc_url=rpc_url,
        deployer=deployer,
        verifier_assets=verifier_assets,
    )
    bundle = build_bundle(
        network_name=args.network,
        deployer=deployer,
        source_commit=args.source_commit,
        repository_subject=subject_hash,
        preflight=preflight,
        gates=load_gates(args.gates, subject_hash),
        verifier_assets=verifier_assets,
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
