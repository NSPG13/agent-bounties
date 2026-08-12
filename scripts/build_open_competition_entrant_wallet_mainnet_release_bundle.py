#!/usr/bin/env python3
"""Build the audited unsigned Base mainnet entrant-wallet deployment bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
from typing import Any

from web3 import Web3

import build_open_competition_entrant_wallet_bundle as entrant_builder
import run_open_competition_entrant_wallet_mainnet_fork_replay as fork_replay
import run_open_competition_entrant_wallet_sepolia_rehearsal as sepolia_rehearsal


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-release-bundle-v1"
ADMIN = fork_replay.ADMIN
CHAIN_ID = 8453
USDC = fork_replay.USDC
MIN_ADMIN_ETH_WEI = 100_000_000_000_000
MIN_ADMIN_USDC_BASE_UNITS = 0


class ReleaseBundleError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseBundleError(message)


def load(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ReleaseBundleError(f"{label} is unreadable: {error}") from error
    require(isinstance(value, dict), f"{label} must be a JSON object")
    return value


def sha256(path: Path) -> str:
    return f"0x{hashlib.sha256(path.read_bytes()).hexdigest()}"


def code_hash(w3: Web3, address: str, block: int | str) -> str:
    return f"0x{Web3.keccak(w3.eth.get_code(Web3.to_checksum_address(address), block)).hex()}"


def git(*args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8"
    ).stdout.strip()


def build(args: argparse.Namespace) -> dict[str, Any]:
    source_bundle = load(args.source_bundle, "source deployment bundle")
    regenerated = entrant_builder.build_bundle("base-mainnet", compile_contracts=False)
    require(source_bundle == regenerated, "source deployment bundle does not exactly regenerate")
    sepolia_audit = load(args.sepolia_audit, "Sepolia audit")
    fork_audit = load(args.fork_audit, "mainnet-fork audit")
    require(sepolia_audit.get("passed") is True, "live Sepolia audit is not passed")
    require(fork_audit.get("passed") is True, "exact mainnet-fork audit is not passed")
    require(sepolia_audit.get("contract_source_revision") == source_bundle["contract_source_revision"], "Sepolia contract tree mismatch")
    require(fork_audit.get("contract_source_revision") == source_bundle["contract_source_revision"], "fork contract tree mismatch")
    require(args.waive_independent_review, "independent review is incomplete; pass the explicit admin waiver only if timeboxed")
    static_analysis = args.static_analysis
    require(static_analysis.is_file(), "entrant-wallet static-analysis triage is missing")
    static_text = static_analysis.read_text(encoding="utf-8")
    require("No untriaged finding" in static_text and "13 findings" in static_text, "static-analysis triage is incomplete")

    release = load(args.competition_release, "Open Competition mainnet release")
    require(release.get("network") == "base-mainnet" and release.get("chain_id") == CHAIN_ID, "competition release chain mismatch")
    canary = release.get("hidden_canary", {})
    require(canary.get("canonical_bounty_settled_event") is True, "existing hidden canary lacks canonical settlement")
    require(canary.get("escrow_conservation_passed") is True, "existing hidden canary escrow did not reconcile")
    require(canary.get("solver_reward_usdc_base_units") == 1_000_000, "existing hidden canary was not 1 USDC")

    w3 = Web3(Web3.HTTPProvider(args.rpc, request_kwargs={"timeout": 30}))
    require(w3.is_connected() and w3.eth.chain_id == CHAIN_ID, "Base mainnet RPC unavailable")
    safe = w3.eth.get_block("safe")
    safe_number = int(safe.number)
    safe_hash = f"0x{safe.hash.hex()}"
    admin = Web3.to_checksum_address(ADMIN)
    admin_eth = int(w3.eth.get_balance(admin, safe_number))
    usdc = w3.eth.contract(
        address=Web3.to_checksum_address(USDC),
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
    admin_usdc = int(usdc.functions.balanceOf(admin).call(block_identifier=safe_number))
    require(admin_eth >= MIN_ADMIN_ETH_WEI, "admin Base ETH is below the bounded deployment minimum")
    require(admin_usdc >= MIN_ADMIN_USDC_BASE_UNITS, "admin native Base USDC balance is invalid")
    entrant = source_bundle["entrant_wallet_factory"]
    factory_code = w3.eth.get_code(Web3.to_checksum_address(entrant["address"]), safe_number)
    implementation_code = w3.eth.get_code(Web3.to_checksum_address(entrant["implementation"]), safe_number)
    require(factory_code == b"" and implementation_code == b"", "predicted entrant deployment address is occupied")

    existing_factory_hash = code_hash(w3, fork_replay.COMPETITION_FACTORY, safe_number)
    expected_existing_factory_hash = release["release_manifest"]["factory_runtime_code_hash"]
    require(existing_factory_hash == expected_existing_factory_hash, "canonical competition factory runtime changed")
    verifier_hash = code_hash(w3, fork_replay.VERIFIER, safe_number)
    require(verifier_hash == fork_replay.VERIFIER_RUNTIME_HASH, "approved verifier runtime changed")
    create2_hash = code_hash(w3, fork_replay.CREATE2_DEPLOYER, safe_number)
    require(create2_hash == fork_replay.CREATE2_DEPLOYER_RUNTIME_HASH, "deterministic deployer runtime changed")
    token_hash = code_hash(w3, USDC, safe_number)
    require(token_hash != f"0x{Web3.keccak(b'').hex()}", "native Base USDC has no code")

    action = {
        "name": "deploy_open_competition_entrant_wallet_factory_v1",
        "from": ADMIN,
        "to": fork_replay.CREATE2_DEPLOYER,
        "value_wei": 0,
        "data": entrant["deployment_transaction"],
        "expected_factory": entrant["address"],
        "expected_implementation": entrant["implementation"],
        "factory_runtime_code_hash": entrant["runtime_code_hash"],
        "implementation_runtime_code_hash": entrant["implementation_runtime_code_hash"],
        "clone_runtime_code_hash": entrant["clone_runtime_code_hash"],
    }
    return {
        "schema_version": SCHEMA,
        "protocol_version": "agent-bounties/open-competition-v1",
        "network": "base-mainnet",
        "chain_id": CHAIN_ID,
        "deployment_state": "mainnet_canary_not_ready_to_earn",
        "source_commit": git("rev-parse", "HEAD"),
        "contract_source_revision": source_bundle["contract_source_revision"],
        "contract_source_revision_kind": "git-tree",
        "compiler": source_bundle["compiler"],
        "admin": ADMIN,
        "preflight_safe_block": {
            "number": safe_number,
            "hash": safe_hash,
            "timestamp": int(safe.timestamp),
            "admin_eth_wei": admin_eth,
            "admin_usdc_base_units": admin_usdc,
            "minimum_admin_eth_wei": MIN_ADMIN_ETH_WEI,
            "minimum_admin_usdc_base_units": MIN_ADMIN_USDC_BASE_UNITS,
        },
        "canonical_dependencies": {
            "competition_factory": fork_replay.COMPETITION_FACTORY,
            "competition_factory_runtime_code_hash": existing_factory_hash,
            "settlement_token": USDC,
            "settlement_token_runtime_code_hash": token_hash,
            "approved_canary_verifier": fork_replay.VERIFIER,
            "approved_canary_verifier_runtime_code_hash": verifier_hash,
            "deterministic_deployer": fork_replay.CREATE2_DEPLOYER,
            "deterministic_deployer_runtime_code_hash": create2_hash,
        },
        "action": action,
        "preserved_hidden_canary": {
            "bounty": canary["bounty_contract"],
            "settlement_transaction": canary["settlement_transaction"],
            "settlement_block": canary["settlement_block"],
            "solver": canary["winner"],
            "solver_reward_usdc_base_units": canary["solver_reward_usdc_base_units"],
            "canonical_bounty_settled_event": True,
            "escrow_conservation_passed": True,
            "safe_block": canary["safe_block"],
        },
        "release_evidence": {
            "base_sepolia_rehearsal_passed": True,
            "base_sepolia_audit_sha256": sha256(args.sepolia_audit),
            "exact_mainnet_fork_replay_passed": True,
            "mainnet_fork_audit_sha256": sha256(args.fork_audit),
            "static_analysis_passed": True,
            "static_analysis_sha256": sha256(static_analysis),
            "independent_review": "timeboxed_and_waived_by_admin",
            "frozen_bytecode": True,
        },
        "activation": {
            "public_creation_enabled": False,
            "public_commitments_enabled": False,
            "public_inventory_enabled": False,
            "relay_support_available": False,
            "gas_sponsorship_available": False,
            "r4_release_evidence_complete": False,
        },
        "signing_constraints": {
            "explicit_wallet_confirmation_required": True,
            "signing_time_balance_recheck_required": True,
            "single_zero_value_create2_call": True,
            "existing_bounties_or_contributors_touched": False,
        },
        "evidence_boundary": (
            "This is an unsigned one-action deployment bundle. It preserves the already-settled hidden 1 USDC "
            "canary and keeps hosted relay, gas sponsorship, public creation, commitments, and inventory disabled. "
            "It is not deployment, settlement, payment, or activation evidence."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--source-bundle",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-deployment-regenerated.json"),
    )
    parser.add_argument(
        "--sepolia-audit",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal-audit.json"),
    )
    parser.add_argument(
        "--fork-audit",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-fork-replay-audit.json"),
    )
    parser.add_argument(
        "--static-analysis",
        type=Path,
        default=Path("docs/security/open-competition-entrant-wallet-v1-static-analysis.md"),
    )
    parser.add_argument(
        "--competition-release",
        type=Path,
        default=Path("deployments/open-competition-v1-base-mainnet.json"),
    )
    parser.add_argument("--rpc", default="https://mainnet.base.org")
    parser.add_argument("--waive-independent-review", action="store_true")
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-release-bundle.json"),
    )
    args = parser.parse_args()
    bundle = build(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "completed": True,
                "bundle": str(args.output),
                "safe_block": bundle["preflight_safe_block"],
                "expected_factory": bundle["action"]["expected_factory"],
                "public_activation_enabled": False,
            }
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
