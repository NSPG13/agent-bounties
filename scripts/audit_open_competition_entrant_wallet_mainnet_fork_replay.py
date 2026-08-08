#!/usr/bin/env python3
"""Fail-closed audit for the entrant-wallet frozen bundle and mainnet fork replay."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from web3 import Web3

import build_open_competition_entrant_wallet_bundle as bundle_builder
import run_open_competition_entrant_wallet_mainnet_fork_replay as fork_replay


ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "agent-bounties/open-competition-entrant-wallet-mainnet-fork-replay-audit-v1"


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


def forbidden_secret_keys(value: Any, path: str = "$") -> list[str]:
    forbidden = {"private_key", "user_salt", "signature", "recovery_salt", "recovery_file"}
    found: list[str] = []
    if isinstance(value, dict):
        for key, child in value.items():
            next_path = f"{path}.{key}"
            if key.lower() in forbidden:
                found.append(next_path)
            found.extend(forbidden_secret_keys(child, next_path))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            found.extend(forbidden_secret_keys(child, f"{path}[{index}]"))
    return found


def audit(bundle: dict[str, Any], replay: dict[str, Any], rpc_url: str) -> dict[str, Any]:
    fork_replay.validate_bundle(bundle)
    regenerated = bundle_builder.build_bundle("base-mainnet", compile_contracts=False)
    require(bundle == regenerated, "frozen bundle does not exactly match current contract artifacts and source tree")
    require(replay.get("schema_version") == fork_replay.SCHEMA, "replay schema mismatch")
    require(replay.get("network") == fork_replay.FORK_NETWORK and replay.get("chain_id") == 8453, "replay chain mismatch")
    require(replay.get("broadcast") is False, "replay claims a live broadcast")
    require(replay.get("passed") is True, "replay is not marked passed")
    require(replay.get("contract_source_revision") == bundle.get("contract_source_revision"), "contract source tree mismatch")
    entrant = bundle["entrant_wallet_factory"]
    deployment = replay.get("deployment", {})
    wallet = replay.get("entrant_wallet", {})
    require(deployment.get("factory", "").lower() == entrant["address"], "entrant factory address mismatch")
    require(deployment.get("implementation", "").lower() == entrant["implementation"], "entrant implementation address mismatch")
    require(deployment.get("factory_runtime_hash") == entrant["runtime_code_hash"], "entrant factory runtime mismatch")
    require(
        deployment.get("implementation_runtime_hash") == entrant["implementation_runtime_code_hash"],
        "entrant implementation runtime mismatch",
    )
    require(wallet.get("runtime_hash") == entrant["clone_runtime_code_hash"], "entrant clone runtime mismatch")
    require(all_true(replay.get("assertions")), "one or more replay assertions failed")
    scenarios = replay.get("scenarios", {})
    require(
        set(scenarios) == {"relayed_wallet_settlement", "separate_solver_and_relayed_bond_withdrawal"},
        "scenario set mismatch",
    )
    for name, scenario in scenarios.items():
        require(all_true(scenario.get("assertions")), f"{name} assertion failed")
        require(event_count(scenario, "BountySettled") == 1, f"{name} canonical settlement event mismatch")
        require(scenario.get("balance_deltas", {}).get("bounty") == -110_000, f"{name} escrow delta mismatch")
    require(
        event_count(scenarios["relayed_wallet_settlement"], "CompetitionSubmissionRejected") == 1,
        "failed reveal event mismatch",
    )
    require(
        event_count(scenarios["separate_solver_and_relayed_bond_withdrawal"], "EntryBondWithdrawn") == 1,
        "losing-bond withdrawal event mismatch",
    )
    actors = replay.get("actors", {})
    require(len(actors) == 6 and len(set(actors.values())) == 6, "ephemeral fork actors are not distinct")
    require(wallet.get("owner") == actors.get("owner"), "entrant owner mismatch")
    require(wallet.get("delegate") == actors.get("delegate"), "entrant delegate mismatch")
    require(wallet.get("address") not in actors.values(), "entrant wallet unexpectedly equals an EOA actor")
    require(scenarios["relayed_wallet_settlement"].get("winner") == wallet.get("address"), "entrant wallet did not win the relayed scenario")
    require(
        scenarios["separate_solver_and_relayed_bond_withdrawal"].get("winner") == actors.get("passing_competitor"),
        "separate solver did not win the losing-wallet scenario",
    )
    receipts = replay.get("receipts", {})
    require(isinstance(receipts, dict) and len(receipts) == 20, "replay receipt set mismatch")
    require(
        all(row.get("status") == 1 and row.get("transaction_hash") and row.get("block_hash") for row in receipts.values()),
        "a replay receipt is unsuccessful or incomplete",
    )
    setup = replay.get("fork_setup", {})
    require(setup.get("mode") == "local_anvil_impersonation" and setup.get("live_evidence") is False, "fork setup boundary mismatch")
    require(setup.get("admin") == fork_replay.ADMIN, "fork admin mismatch")
    require(setup.get("admin_eth_before_wei", 0) >= 500_000_000_000_000, "pinned admin ETH was insufficient")
    require(setup.get("admin_usdc_before_base_units", 0) >= 400_000, "pinned admin USDC was insufficient")
    require(all(setup[name].get("status") == 1 for name in ("deployment", "keeper_eth_funding", "keeper_usdc_funding")), "fork setup receipt failed")
    runtime = replay.get("canonical_runtime_evidence", {})
    require(
        runtime.get("competition_factory_runtime_before") == runtime.get("competition_factory_runtime_after"),
        "canonical competition factory runtime changed during replay",
    )
    require(runtime.get("approved_verifier_runtime_hash") == fork_replay.VERIFIER_RUNTIME_HASH, "approved verifier pin mismatch")
    require(runtime.get("deterministic_deployer_runtime_hash") == fork_replay.CREATE2_DEPLOYER_RUNTIME_HASH, "CREATE2 pin mismatch")
    gates = replay.get("activation_gates", {})
    require(gates.get("exact_mainnet_fork_replay_passed") is True, "fork replay gate is not true")
    require(gates.get("keeper_relay_rehearsed") is True and gates.get("keeper_gas_reserve_verified") is True, "keeper replay gates failed")
    require(gates.get("base_sepolia_rehearsal_passed") is False, "fork evidence falsely claims live Sepolia evidence")
    require(gates.get("independent_review_complete") is False, "fork evidence falsely claims independent review")
    require(gates.get("relay_support_available") is False and gates.get("gas_sponsorship_available") is False, "hosted relay gates were enabled")
    require(gates.get("public_creation_enabled") is False and gates.get("public_inventory_enabled") is False, "public activation was enabled")
    secrets = forbidden_secret_keys(replay)
    require(not secrets, f"replay contains forbidden recovery material: {secrets}")

    fork_block = replay.get("fork_block", {})
    provider = Web3(Web3.HTTPProvider(rpc_url, request_kwargs={"timeout": 30}))
    require(provider.is_connected() and provider.eth.chain_id == 8453, "Base mainnet RPC unavailable during audit")
    canonical = provider.eth.get_block(int(fork_block.get("number", -1)))
    require(canonical.hash.hex().lower() == str(fork_block.get("hash", "")).removeprefix("0x").lower(), "fork block is no longer canonical")
    require(int(canonical.timestamp) == int(fork_block.get("timestamp", -1)), "fork block timestamp mismatch")

    assertions = {
        "bundle_exactly_regenerated": True,
        "canonical_fork_block_reverified": True,
        "frozen_factory_implementation_and_clone_runtimes_match": True,
        "approved_verifier_and_create2_runtime_pins_match": True,
        "two_keeper_relay_scenarios_passed": True,
        "canonical_settlement_and_bond_withdrawal_events_match": True,
        "escrow_deltas_reconcile": True,
        "all_replay_receipts_succeeded": True,
        "ephemeral_actors_are_distinct": True,
        "recovery_keys_salts_and_signatures_are_absent": True,
        "no_mainnet_broadcast_occurred": True,
        "hosted_and_public_activation_remain_disabled": True,
    }
    return {
        "schema_version": SCHEMA,
        "network": fork_replay.FORK_NETWORK,
        "chain_id": 8453,
        "contract_source_revision": bundle["contract_source_revision"],
        "fork_block": replay["fork_block"],
        "entrant_wallet_factory": entrant["address"],
        "entrant_wallet_implementation": entrant["implementation"],
        "approved_verifier": fork_replay.VERIFIER,
        "assertions": assertions,
        "passed": all(assertions.values()),
        "broadcast": False,
        "evidence_boundary": (
            "This audit proves frozen-bundle and exact local mainnet-fork consistency only. It is not live "
            "deployment, hosted relay availability, gas sponsorship, public activation, settlement, or payment evidence."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--bundle",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-deployment-regenerated.json"),
    )
    parser.add_argument(
        "--replay",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-fork-replay.json"),
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/open-competition-entrant-wallet/base-mainnet-fork-replay-audit.json"),
    )
    parser.add_argument("--rpc", default="https://mainnet.base.org")
    args = parser.parse_args()
    result = audit(load(args.bundle, "bundle"), load(args.replay, "replay"), args.rpc)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"completed": True, "audit": str(args.output), "passed": result["passed"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
