#!/usr/bin/env python3
"""Fail-closed local audit for the unsigned Open Competition V1 mainnet bundle."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
from typing import Any

from _shared.evm import create_address, keccak256
import build_open_competition_v1_mainnet_bundle as builder


ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")


class MainnetBundleAuditError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise MainnetBundleAuditError(message)


def reject_secrets(value: Any, location: str = "$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            lowered = key.lower()
            require(
                lowered not in {"private_key", "mnemonic", "secret", "seed_phrase"},
                f"secret-bearing field is forbidden at {location}.{key}",
            )
            reject_secrets(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_secrets(child, f"{location}[{index}]")


def audit(bundle: dict[str, Any], repo: Path) -> dict[str, Any]:
    reject_secrets(bundle)
    require(bundle.get("schema_version") == "agent-bounties/open-competition-v1-mainnet-bundle-v1", "schema mismatch")
    require(bundle.get("protocol_version") == "agent-bounties/open-competition-v1", "protocol mismatch")
    require(bundle.get("network") == "base-mainnet" and bundle.get("chain_id") == builder.CHAIN_ID, "network mismatch")
    require(bundle.get("deployment_state") == "sepolia_rehearsed_not_ready_to_earn", "predeployment state mismatch")
    require(bundle.get("deployer") == builder.ADMIN, "deployer mismatch")
    require(bundle.get("settlement_token") == builder.USDC, "settlement token mismatch")
    require(re.fullmatch(r"[0-9a-f]{40}", str(bundle.get("source_commit", ""))) is not None, "source commit invalid")

    preflight = bundle.get("preflight_block")
    require(isinstance(preflight, dict), "preflight block missing")
    require(isinstance(preflight.get("number"), int) and preflight["number"] > 0, "preflight block number invalid")
    require(HASH.fullmatch(str(preflight.get("hash", ""))) is not None, "preflight block hash invalid")
    require(isinstance(preflight.get("deployer_nonce"), int) and preflight["deployer_nonce"] >= 0, "deployer nonce invalid")
    require(preflight.get("deployer_eth_wei", 0) >= builder.MIN_DEPLOYER_ETH_WEI, "deployment gas balance is insufficient")
    require(preflight.get("deployer_usdc_base_units", 0) >= builder.MIN_CANARY_USDC, "canary USDC balance is insufficient")

    actions = bundle.get("actions")
    require(isinstance(actions, list) and len(actions) == 1, "bundle must contain exactly one deployment")
    action = actions[0]
    expected_factory = create_address(builder.ADMIN, preflight["deployer_nonce"])
    expected_implementation = create_address(expected_factory, 1)
    require(action.get("name") == "deploy_open_competition_factory_v1", "deployment action mismatch")
    require(action.get("from") == builder.ADMIN and action.get("from_nonce") == preflight["deployer_nonce"], "deployment sender or nonce mismatch")
    require(action.get("to") is None and action.get("value_wei") == 0, "deployment target or value mismatch")
    require(action.get("expected_contract") == expected_factory, "predicted factory mismatch")
    require(action.get("expected_implementation") == expected_implementation, "predicted implementation mismatch")
    require(ADDRESS.fullmatch(str(action.get("expected_contract", ""))) is not None, "factory address invalid")
    require(HASH.fullmatch(str(action.get("runtime_code_hash", ""))) is not None, "factory runtime hash invalid")
    require(HASH.fullmatch(str(action.get("implementation_runtime_code_hash", ""))) is not None, "implementation runtime hash invalid")
    require(keccak256(bytes.fromhex(action["data"][2:])) == action.get("creation_code_hash"), "creation code hash mismatch")
    require(keccak256(bytes.fromhex(action["expected_runtime_code"][2:])) == action.get("runtime_code_hash"), "factory runtime hash mismatch")
    require(
        keccak256(bytes.fromhex(action["expected_implementation_runtime_code"][2:]))
        == action.get("implementation_runtime_code_hash"),
        "implementation runtime hash mismatch",
    )
    require(action["data"].lower().endswith(builder.USDC[2:].rjust(64, "0")), "factory constructor token mismatch")

    sources = bundle.get("source_sha256")
    require(isinstance(sources, dict) and len(sources) == 4, "frozen source hashes incomplete")
    for relative, expected in sources.items():
        path = repo / relative
        require(path.is_file(), f"frozen source missing: {relative}")
        observed = hashlib.sha256(path.read_bytes().replace(b"\r\n", b"\n")).hexdigest()
        require(observed == expected, f"frozen source hash mismatch: {relative}")

    verifier = bundle.get("verifier_profile")
    require(isinstance(verifier, dict), "verifier profile missing")
    require(verifier.get("verifier_address") == builder.VERIFIER, "verifier address mismatch")
    require(verifier.get("runtime_code_hash") == builder.VERIFIER_RUNTIME_HASH, "verifier runtime mismatch")
    require(verifier.get("difficulty_bits") == builder.DIFFICULTY_BITS, "verifier difficulty mismatch")
    require(
        verifier.get("benchmark_preimage") == builder.CANARY_BENCHMARK_PREIMAGE,
        "canary benchmark preimage mismatch",
    )
    require(verifier.get("benchmark_hash") == builder.CANARY_BENCHMARK_HASH, "canary benchmark hash mismatch")
    require(
        verifier.get("evidence_schema_preimage") == builder.CANARY_EVIDENCE_SCHEMA_PREIMAGE,
        "canary evidence schema preimage mismatch",
    )
    require(
        verifier.get("evidence_schema_hash") == builder.CANARY_EVIDENCE_SCHEMA_HASH,
        "canary evidence schema hash mismatch",
    )
    require(verifier.get("usage") == "protocol_canary_only", "verifier usage boundary missing")
    require(verifier.get("public_inventory_eligible") is False, "verifier cannot be public before canary")

    canary = bundle.get("hidden_canary")
    require(isinstance(canary, dict), "hidden canary missing")
    expected_canary = {
        "solver_reward_usdc_base_units": builder.CANARY_SOLVER_REWARD,
        "verifier_reward_usdc_base_units": builder.CANARY_VERIFIER_REWARD,
        "entry_bond_usdc_base_units": builder.CANARY_SOLVER_BOND,
        "initial_funding_usdc_base_units": builder.CANARY_INITIAL_FUNDING,
        "total_admin_usdc_budget_base_units": builder.MIN_CANARY_USDC,
        "max_entries": 4,
        "competition_window_seconds": 86_400,
        "reveal_window_seconds": 3_600,
        "creator_may_compete": False,
        "separate_solver_wallet_required": True,
        "inventory_visibility": "hidden",
    }
    require(canary == expected_canary, "hidden canary bounds mismatch")
    activation = bundle.get("activation")
    require(isinstance(activation, dict), "activation gates missing")
    require(activation.get("public_creation_enabled") is False, "public creation must remain disabled")
    require(activation.get("public_commitments_enabled") is False, "public commitments must remain disabled")
    require(activation.get("public_inventory_eligible") is False, "public inventory must remain disabled")
    return {
        "schema_version": "agent-bounties/open-competition-v1-mainnet-bundle-audit-v1",
        "passed": True,
        "source_commit": bundle["source_commit"],
        "preflight_block": preflight["number"],
        "factory": expected_factory,
        "implementation": expected_implementation,
        "verifier": builder.VERIFIER,
        "hidden_canary_usdc_budget_base_units": builder.MIN_CANARY_USDC,
        "deployment_state": bundle["deployment_state"],
        "public_inventory_eligible": False,
        "evidence_boundary": "This audit proves unsigned bundle consistency, not deployment, settlement, payment, or activation.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    report = audit(json.loads(args.bundle.read_text(encoding="utf-8")), repo)
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
