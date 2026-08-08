#!/usr/bin/env python3
"""Fail-closed audit for the live Base Sepolia entrant-wallet rehearsal."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from web3 import Web3

import build_open_competition_entrant_wallet_bundle as bundle_builder
import run_open_competition_entrant_wallet_sepolia_rehearsal as rehearsal


SCHEMA = "agent-bounties/open-competition-entrant-wallet-sepolia-rehearsal-audit-v1"


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


def all_true(value: Any) -> bool:
    return isinstance(value, dict) and bool(value) and all(item is True for item in value.values())


def event_count(scenario: dict[str, Any], name: str) -> int:
    return sum(row.get("name") == name for row in scenario.get("events", []))


def find_forbidden_keys(value: Any, path: str = "$") -> list[str]:
    forbidden = {"private_key", "user_salt", "signature", "recovery_salt", "recovery_file"}
    found: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            location = f"{path}.{key}"
            if key.lower() in forbidden:
                found.append(location)
            found.extend(find_forbidden_keys(child, location))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.extend(find_forbidden_keys(child, f"{path}[{index}]"))
    return found


def audit(bundle: dict[str, Any], manifest: dict[str, Any], rpc_url: str) -> dict[str, Any]:
    regenerated = bundle_builder.build_bundle("base-sepolia", compile_contracts=False)
    require(bundle == regenerated, "frozen Sepolia bundle does not exactly regenerate")
    require(manifest.get("schema_version") == rehearsal.SCHEMA, "rehearsal schema mismatch")
    require(manifest.get("network") == "base-sepolia" and manifest.get("chain_id") == 84532, "rehearsal chain mismatch")
    require(manifest.get("deployment_state") == "sepolia_rehearsed_not_ready_to_earn", "rehearsal state mismatch")
    require(manifest.get("contract_source_revision") == bundle.get("contract_source_revision"), "contract tree mismatch")
    require(manifest.get("settlement_token") == rehearsal.USDC, "settlement token mismatch")
    require(manifest.get("canonical_competition_factory") == rehearsal.COMPETITION_FACTORY, "competition factory mismatch")
    entrant = bundle["entrant_wallet_factory"]
    deployment = manifest.get("deployment", {})
    wallet = manifest.get("entrant_wallet", {})
    require(deployment.get("factory") == entrant["address"], "entrant factory address mismatch")
    require(deployment.get("implementation") == entrant["implementation"], "entrant implementation address mismatch")
    require(deployment.get("factory_runtime_hash") == entrant["runtime_code_hash"], "entrant factory runtime mismatch")
    require(deployment.get("implementation_runtime_hash") == entrant["implementation_runtime_code_hash"], "entrant implementation runtime mismatch")
    require(wallet.get("runtime_hash") == entrant["clone_runtime_code_hash"], "entrant clone runtime mismatch")

    funding = manifest.get("actor_funding", {})
    require(funding.get("mode") == "confirmed_atomic_admin_batch" and funding.get("live_evidence") is True, "live funding boundary mismatch")
    require(funding.get("eth_wei") == rehearsal.Runner.ADMIN_FUNDING_ETH_WEI, "keeper ETH funding mismatch")
    require(funding.get("usdc_base_units") == rehearsal.Runner.ADMIN_FUNDING_USDC, "keeper USDC funding mismatch")
    require(funding.get("usdc_token") == rehearsal.USDC, "funding token mismatch")
    provenance = funding.get("provenance", {})
    require(provenance.get("execution_sender") == rehearsal.ADMIN, "funding execution sender mismatch")
    require(provenance.get("admin_authorization") in {"direct_admin_transaction", "successful_eip1271_trace"}, "admin authorization missing")
    require(provenance.get("exact_native_transfer") is True and provenance.get("exact_token_transfer") is True, "funding transfers were not exact")
    require(provenance.get("atomic_transaction") is True, "funding calls were not atomic")

    require(all_true(manifest.get("assertions")), "top-level rehearsal assertion failed")
    scenarios = manifest.get("scenarios", {})
    require(
        set(scenarios) == {"relayed_wallet_settlement", "separate_solver_and_relayed_bond_withdrawal"},
        "scenario set mismatch",
    )
    for name, scenario in scenarios.items():
        require(all_true(scenario.get("assertions")), f"{name} assertion failed")
        require(event_count(scenario, "BountySettled") == 1, f"{name} settlement event mismatch")
        require(scenario.get("balance_deltas", {}).get("bounty") == -110_000, f"{name} escrow delta mismatch")
    require(event_count(scenarios["relayed_wallet_settlement"], "CompetitionSubmissionRejected") == 1, "failed reveal event mismatch")
    require(event_count(scenarios["separate_solver_and_relayed_bond_withdrawal"], "EntryBondWithdrawn") == 1, "bond withdrawal event mismatch")
    actors = manifest.get("actors", {})
    require(len(actors) == 6 and len(set(actors.values())) == 6, "rehearsal actors are not distinct")
    require(wallet.get("owner") == actors.get("owner") and wallet.get("delegate") == actors.get("delegate"), "wallet authority mismatch")
    require(scenarios["relayed_wallet_settlement"].get("winner") == wallet.get("address"), "entrant wallet did not win scenario one")
    require(
        scenarios["separate_solver_and_relayed_bond_withdrawal"].get("winner") == actors.get("passing_competitor"),
        "separate solver did not win scenario two",
    )
    receipts = manifest.get("receipts", {})
    require(isinstance(receipts, dict) and len(receipts) >= 11, "rehearsal receipt set is incomplete")
    require(all(row.get("status") == 1 for row in receipts.values()), "a rehearsal receipt failed")
    require(not find_forbidden_keys(manifest), "rehearsal manifest contains recovery material")

    gates = manifest.get("activation_gates", {})
    require(gates.get("base_sepolia_rehearsal_passed") is True, "Sepolia gate is not true")
    require(gates.get("keeper_relay_rehearsed") is True and gates.get("keeper_gas_reserve_verified") is True, "keeper gates failed")
    require(gates.get("exact_mainnet_fork_replay_passed") is False, "Sepolia evidence falsely claims fork evidence")
    require(gates.get("independent_review_complete") is False, "Sepolia evidence falsely claims review")
    require(gates.get("relay_support_available") is False and gates.get("gas_sponsorship_available") is False, "hosted relay gates were enabled")
    require(gates.get("public_creation_enabled") is False and gates.get("public_inventory_enabled") is False, "public activation was enabled")

    w3 = Web3(Web3.HTTPProvider(rpc_url, request_kwargs={"timeout": 30}))
    require(w3.is_connected() and w3.eth.chain_id == 84532, "Base Sepolia RPC unavailable during audit")
    safe = manifest.get("canonical_safe_block", {})
    observed_safe = w3.eth.get_block("safe")
    require(int(observed_safe.number) >= int(safe.get("number", -1)), "recorded safe block is ahead of Base safe")
    recorded_safe = w3.eth.get_block(int(safe["number"]))
    require(f"0x{recorded_safe.hash.hex()}".lower() == str(safe.get("hash", "")).lower(), "recorded safe block hash mismatch")
    for name, row in receipts.items():
        receipt = w3.eth.get_transaction_receipt(row["transaction_hash"])
        block = w3.eth.get_block(receipt.blockNumber)
        require(receipt.status == 1, f"{name} canonical receipt failed")
        require(f"0x{receipt.blockHash.hex()}".lower() == row["block_hash"].lower(), f"{name} receipt block hash changed")
        require(bytes(block.hash) == bytes(receipt.blockHash), f"{name} receipt is not canonical")
        require(int(receipt.blockNumber) <= int(observed_safe.number), f"{name} is not yet safe")
    require(
        f"0x{Web3.keccak(w3.eth.get_code(Web3.to_checksum_address(rehearsal.VERIFIER))).hex()}"
        == manifest["approved_canary_verifier"]["runtime_hash"],
        "approved verifier runtime changed",
    )

    assertions = {
        "bundle_exactly_regenerated": True,
        "live_atomic_admin_funding_trace_passed": True,
        "frozen_factory_implementation_and_clone_runtimes_match": True,
        "two_live_keeper_relay_scenarios_passed": True,
        "canonical_settlement_and_bond_withdrawal_events_match": True,
        "escrow_deltas_reconcile": True,
        "all_receipts_are_canonical_and_safe": True,
        "recovery_keys_salts_and_signatures_are_absent": True,
        "hosted_and_public_activation_remain_disabled": True,
    }
    return {
        "schema_version": SCHEMA,
        "network": "base-sepolia",
        "chain_id": 84532,
        "contract_source_revision": bundle["contract_source_revision"],
        "canonical_safe_block": safe,
        "entrant_wallet_factory": entrant["address"],
        "entrant_wallet_implementation": entrant["implementation"],
        "entrant_wallet": wallet["address"],
        "funding_transaction": funding["transaction_hash"],
        "assertions": assertions,
        "passed": all(assertions.values()),
        "public_activation_enabled": False,
        "evidence_boundary": (
            "This audit proves the live Base Sepolia entrant-wallet rehearsal only. It is not mainnet deployment, "
            "hosted relay availability, gas sponsorship, public activation, settlement, or payment evidence."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-deployment-regenerated.json"),
    )
    parser.add_argument(
        "--rehearsal",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal.json"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-sepolia-rehearsal-audit.json"),
    )
    parser.add_argument("--rpc", default="https://sepolia.base.org")
    args = parser.parse_args()
    result = audit(load(args.bundle, "bundle"), load(args.rehearsal, "rehearsal"), args.rpc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "audit": str(args.output), "passed": result["passed"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
