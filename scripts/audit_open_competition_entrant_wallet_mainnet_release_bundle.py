#!/usr/bin/env python3
"""Fail-closed audit for the unsigned entrant-wallet Base mainnet release bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

from web3 import Web3

import build_open_competition_entrant_wallet_bundle as source_builder
import build_open_competition_entrant_wallet_mainnet_release_bundle as release_builder
import run_open_competition_entrant_wallet_mainnet_fork_replay as fork_replay


SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-release-bundle-audit-v1"


class AuditError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AuditError(message)


def load(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise AuditError(f"{label} is unreadable: {error}") from error
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def sha256(path: Path) -> str:
    return f"0x{hashlib.sha256(path.read_bytes()).hexdigest()}"


def code_hash(w3: Web3, address: str, block: int) -> str:
    return f"0x{Web3.keccak(w3.eth.get_code(Web3.to_checksum_address(address), block)).hex()}"


def forbidden_keys(value: Any, path: str = "$") -> list[str]:
    denied = {"private_key", "mnemonic", "seed_phrase", "signature", "user_salt", "recovery_file"}
    found: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            location = f"{path}.{key}"
            if key.lower() in denied:
                found.append(location)
            found.extend(forbidden_keys(child, location))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.extend(forbidden_keys(child, f"{path}[{index}]"))
    return found


def audit(args: argparse.Namespace) -> dict[str, Any]:
    bundle = load(args.bundle, "release bundle")
    source = load(args.source_bundle, "source deployment bundle")
    sepolia_audit = load(args.sepolia_audit, "Sepolia audit")
    fork_audit = load(args.fork_audit, "mainnet-fork audit")
    competition_release = load(args.competition_release, "competition release")
    require(source == source_builder.build_bundle("base-mainnet", compile_contracts=False), "source bundle does not regenerate")
    require(bundle.get("schema_version") == release_builder.SCHEMA, "release schema mismatch")
    require(bundle.get("network") == "base-mainnet" and bundle.get("chain_id") == 8453, "release chain mismatch")
    require(bundle.get("deployment_state") == "mainnet_canary_not_ready_to_earn", "release state mismatch")
    require(bundle.get("contract_source_revision") == source["contract_source_revision"], "contract tree mismatch")
    require(bundle.get("admin") == release_builder.ADMIN, "admin mismatch")
    require(not forbidden_keys(bundle), "release bundle contains secret-bearing fields")

    action = bundle.get("action", {})
    entrant = source["entrant_wallet_factory"]
    expected_action = {
        "name": "deploy_open_competition_entrant_wallet_factory_v1",
        "from": release_builder.ADMIN,
        "to": fork_replay.CREATE2_DEPLOYER,
        "value_wei": 0,
        "data": entrant["deployment_transaction"],
        "expected_factory": entrant["address"],
        "expected_implementation": entrant["implementation"],
        "factory_runtime_code_hash": entrant["runtime_code_hash"],
        "implementation_runtime_code_hash": entrant["implementation_runtime_code_hash"],
        "clone_runtime_code_hash": entrant["clone_runtime_code_hash"],
    }
    require(action == expected_action, "unsigned deployment action mismatch")

    evidence = bundle.get("release_evidence", {})
    require(sepolia_audit.get("passed") is True and fork_audit.get("passed") is True, "required release audit failed")
    require(evidence.get("base_sepolia_rehearsal_passed") is True, "Sepolia release gate missing")
    require(evidence.get("exact_mainnet_fork_replay_passed") is True, "mainnet-fork release gate missing")
    require(evidence.get("static_analysis_passed") is True, "static-analysis release gate missing")
    require(evidence.get("independent_review") == "timeboxed_and_waived_by_admin", "independent-review disposition mismatch")
    require(evidence.get("frozen_bytecode") is True, "bytecode freeze gate missing")
    require(evidence.get("base_sepolia_audit_sha256") == sha256(args.sepolia_audit), "Sepolia audit hash mismatch")
    require(evidence.get("mainnet_fork_audit_sha256") == sha256(args.fork_audit), "fork audit hash mismatch")
    require(evidence.get("static_analysis_sha256") == sha256(args.static_analysis), "static-analysis hash mismatch")

    preserved = bundle.get("preserved_hidden_canary", {})
    canonical_canary = competition_release.get("hidden_canary", {})
    require(preserved.get("bounty") == canonical_canary.get("bounty_contract"), "preserved canary address mismatch")
    require(preserved.get("settlement_transaction") == canonical_canary.get("settlement_transaction"), "preserved canary settlement mismatch")
    require(preserved.get("solver_reward_usdc_base_units") == 1_000_000, "preserved canary was not 1 USDC")
    require(preserved.get("canonical_bounty_settled_event") is True, "preserved canary lacks settlement event")
    require(preserved.get("escrow_conservation_passed") is True, "preserved canary escrow mismatch")

    activation = bundle.get("activation", {})
    require(
        activation
        == {
            "public_creation_enabled": False,
            "public_commitments_enabled": False,
            "public_inventory_enabled": False,
            "relay_support_available": False,
            "gas_sponsorship_available": False,
            "r4_release_evidence_complete": False,
        },
        "activation gates are not fail-closed",
    )
    signing = bundle.get("signing_constraints", {})
    require(signing.get("explicit_wallet_confirmation_required") is True, "wallet confirmation constraint missing")
    require(signing.get("single_zero_value_create2_call") is True, "bounded deployment constraint missing")
    require(signing.get("existing_bounties_or_contributors_touched") is False, "bundle claims existing state mutation")

    w3 = Web3(Web3.HTTPProvider(args.rpc, request_kwargs={"timeout": 30}))
    require(w3.is_connected() and w3.eth.chain_id == 8453, "Base mainnet RPC unavailable")
    pinned = bundle.get("preflight_safe_block", {})
    block_number = int(pinned.get("number", -1))
    block = w3.eth.get_block(block_number)
    require(f"0x{block.hash.hex()}".lower() == str(pinned.get("hash", "")).lower(), "preflight safe block hash mismatch")
    admin = Web3.to_checksum_address(release_builder.ADMIN)
    require(int(w3.eth.get_balance(admin, block_number)) == int(pinned.get("admin_eth_wei", -1)), "pinned admin ETH mismatch")
    usdc = w3.eth.contract(
        address=Web3.to_checksum_address(release_builder.USDC),
        abi=[
            {
                "name": "balanceOf",
                "type": "function",
                "stateMutability": "view",
                "inputs": [{"name": "account", "type": "address"}],
                "outputs": [{"type": "uint256"}],
            }
        ],
    )
    require(int(usdc.functions.balanceOf(admin).call(block_identifier=block_number)) == int(pinned.get("admin_usdc_base_units", -1)), "pinned admin USDC mismatch")
    require(int(pinned["admin_eth_wei"]) >= int(pinned["minimum_admin_eth_wei"]), "pinned admin ETH is insufficient")
    require(w3.eth.get_code(Web3.to_checksum_address(action["expected_factory"]), block_number) == b"", "predicted factory is occupied")
    require(w3.eth.get_code(Web3.to_checksum_address(action["expected_implementation"]), block_number) == b"", "predicted implementation is occupied")
    dependencies = bundle.get("canonical_dependencies", {})
    checks = (
        (dependencies["competition_factory"], dependencies["competition_factory_runtime_code_hash"]),
        (dependencies["settlement_token"], dependencies["settlement_token_runtime_code_hash"]),
        (dependencies["approved_canary_verifier"], dependencies["approved_canary_verifier_runtime_code_hash"]),
        (dependencies["deterministic_deployer"], dependencies["deterministic_deployer_runtime_code_hash"]),
    )
    require(all(code_hash(w3, address, block_number) == expected for address, expected in checks), "canonical dependency runtime mismatch")

    assertions = {
        "source_bundle_exactly_regenerated": True,
        "unsigned_create2_action_matches_frozen_bytecode": True,
        "live_sepolia_and_exact_fork_audits_match": True,
        "static_analysis_and_review_disposition_match": True,
        "pinned_safe_block_balances_and_dependencies_match": True,
        "predicted_factory_and_implementation_are_vacant": True,
        "settled_hidden_1_usdc_canary_is_preserved": True,
        "existing_bounties_and_contributors_are_not_action_targets": True,
        "hosted_and_public_activation_remain_disabled": True,
        "bundle_contains_no_signing_secret": True,
    }
    return {
        "schema_version": SCHEMA,
        "network": "base-mainnet",
        "chain_id": 8453,
        "contract_source_revision": source["contract_source_revision"],
        "preflight_safe_block": pinned,
        "expected_factory": entrant["address"],
        "expected_implementation": entrant["implementation"],
        "assertions": assertions,
        "passed": all(assertions.values()),
        "public_activation_enabled": False,
        "evidence_boundary": "This audit proves unsigned bundle consistency only, not deployment, settlement, payment, relay availability, gas sponsorship, or activation.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-release-bundle.json"))
    parser.add_argument("--source-bundle", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-deployment-regenerated.json"))
    parser.add_argument("--sepolia-audit", type=Path, default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal-audit.json"))
    parser.add_argument("--fork-audit", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-fork-replay-audit.json"))
    parser.add_argument("--static-analysis", type=Path, default=Path("docs/security/open-competition-entrant-wallet-v1-static-analysis.md"))
    parser.add_argument("--competition-release", type=Path, default=Path("deployments/open-competition-v1-base-mainnet.json"))
    parser.add_argument("--rpc", default="https://mainnet.base.org")
    parser.add_argument("--output", type=Path, default=Path("target/open-competition-entrant-wallet/base-mainnet-release-bundle-audit.json"))
    args = parser.parse_args()
    result = audit(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "audit": str(args.output), "passed": result["passed"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
