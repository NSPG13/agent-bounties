#!/usr/bin/env python3
"""Fail-closed audit for the Open Competition V1 deployment and rehearsal manifests."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


SCHEMA = "agent-bounties/open-competition-v1-rehearsal-manifest-v1"
PROTOCOL = "agent-bounties/open-competition-v1"
ADMIN = "0x884834e884d6e93462655a2820140ad03e6747bc"
USDC = "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HASH = re.compile(r"^0x[0-9a-f]{64}$")
REQUIRED_ADVERSARIAL = {
    "copied_reveal_rejected",
    "same_block_reveal_rejected",
    "authorization_substitution_rejected",
    "capacity_enforced",
    "verifier_revert_recoverable",
}


class RehearsalAuditError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RehearsalAuditError(message)


def load(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(isinstance(value, dict), f"{path} must contain a JSON object")
    return value


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


def event_names(scenario: dict[str, Any]) -> list[str]:
    events = scenario.get("events")
    require(isinstance(events, list) and events, "scenario events must be non-empty")
    names = []
    for event in events:
        require(isinstance(event, dict), "scenario event must be an object")
        name = event.get("name")
        tx_hash = event.get("transaction_hash")
        block_number = event.get("block_number")
        log_index = event.get("log_index")
        require(isinstance(name, str) and name, "event name is required")
        require(isinstance(tx_hash, str) and HASH.fullmatch(tx_hash), f"{name} transaction hash is invalid")
        require(isinstance(block_number, int) and block_number > 0, f"{name} block number is invalid")
        require(isinstance(log_index, int) and log_index >= 0, f"{name} log index is invalid")
        names.append(name)
    return names


def audit(bundle: dict[str, Any], rehearsal: dict[str, Any]) -> dict[str, Any]:
    reject_secrets(bundle)
    reject_secrets(rehearsal)
    require(bundle.get("schema_version") == "agent-bounties/open-competition-v1-deployment-bundle-v1", "deployment bundle schema mismatch")
    require(rehearsal.get("schema_version") == SCHEMA, "rehearsal schema mismatch")
    for document in (bundle, rehearsal):
        require(document.get("protocol_version") == PROTOCOL, "protocol mismatch")
        require(document.get("network") == "base-sepolia", "network must be base-sepolia")
        require(document.get("chain_id") == 84532, "chain id must be 84532")
        require(document.get("deployer") == ADMIN, "deployer must be the frozen admin")
        require(document.get("settlement_token") == USDC, "settlement token must be native Base Sepolia USDC")
    require(rehearsal.get("deployment_state") == "sepolia_rehearsed_not_ready_to_earn", "rehearsal deployment state is invalid")
    require(rehearsal.get("public_inventory_eligible") is False, "rehearsal must remain outside public inventory")
    require(bundle.get("source_commit") == rehearsal.get("source_commit"), "source commit changed after bytecode freeze")
    require(bundle.get("compiler") == rehearsal.get("compiler"), "compiler settings changed after bytecode freeze")
    require(re.fullmatch(r"[0-9a-f]{40}", str(rehearsal.get("source_commit", ""))) is not None, "source commit must be full lowercase SHA")

    deployments = rehearsal.get("deployments")
    require(isinstance(deployments, dict), "deployments are required")
    action_by_name = {action["name"]: action for action in bundle.get("actions", [])}
    expected = {
        "verifier": action_by_name.get("deploy_leading_zero_work_verifier_16"),
        "factory": action_by_name.get("deploy_open_competition_factory_v1"),
    }
    for name, action in expected.items():
        require(isinstance(action, dict), f"bundle {name} action is missing")
        deployment = deployments.get(name)
        require(isinstance(deployment, dict), f"rehearsal {name} deployment is missing")
        require(deployment.get("address") == action.get("expected_contract"), f"{name} address mismatch")
        require(deployment.get("runtime_code_hash") == action.get("runtime_code_hash"), f"{name} runtime hash mismatch")
        require(deployment.get("runtime_matches") is True, f"{name} runtime was not proven")
        require(HASH.fullmatch(str(deployment.get("transaction_hash", ""))) is not None, f"{name} transaction hash is invalid")
        require(HASH.fullmatch(str(deployment.get("block_hash", ""))) is not None, f"{name} block hash is invalid")
        require(isinstance(deployment.get("block_number"), int) and deployment["block_number"] > 0, f"{name} block is invalid")
    implementation = deployments.get("implementation")
    factory_action = expected["factory"]
    require(isinstance(implementation, dict), "implementation deployment is missing")
    require(implementation.get("address") == factory_action.get("expected_implementation"), "implementation address mismatch")
    require(implementation.get("runtime_code_hash") == factory_action.get("implementation_runtime_code_hash"), "implementation runtime hash mismatch")
    require(implementation.get("runtime_matches") is True, "implementation runtime was not proven")

    actors = rehearsal.get("actors")
    required_actors = {"creator", "failed_competitor", "passing_competitor", "expiring_competitor", "relayer"}
    require(isinstance(actors, dict) and required_actors <= actors.keys(), "ephemeral actor set is incomplete")
    actor_addresses = [actors[name] for name in sorted(required_actors)]
    require(all(isinstance(address, str) and ADDRESS.fullmatch(address) for address in actor_addresses), "actor address is invalid")
    require(len(set(actor_addresses)) == len(actor_addresses), "rehearsal actors must use distinct wallets")
    require(ADMIN not in actor_addresses, "admin deployer cannot be a rehearsal actor")

    scenarios = rehearsal.get("scenarios")
    require(isinstance(scenarios, dict), "scenarios are required")
    settlement = scenarios.get("settlement_and_losing_bond_withdrawal")
    cancellation = scenarios.get("expiry_cancellation_and_refund")
    for scenario in (settlement, cancellation):
        require(isinstance(scenario, dict), "both required scenarios must be present")
        require(ADDRESS.fullmatch(str(scenario.get("bounty_contract", ""))) is not None, "scenario bounty address is invalid")
        require(scenario.get("reconciled") is True, "scenario balances were not reconciled")
        assertions = scenario.get("assertions")
        require(isinstance(assertions, dict) and assertions and all(value is True for value in assertions.values()), "scenario assertions must all pass")
        transactions = scenario.get("transactions")
        require(isinstance(transactions, list) and transactions and all(isinstance(item, str) and HASH.fullmatch(item) for item in transactions), "scenario transactions are invalid")
    settlement_events = event_names(settlement)
    cancellation_events = event_names(cancellation)
    require(settlement_events.count("BountySettled") == 1, "settlement scenario needs exactly one canonical BountySettled")
    require("EntryBondWithdrawn" in settlement_events, "settlement scenario is missing losing-bond withdrawal")
    require("CommitmentExpired" in cancellation_events, "cancellation scenario is missing commitment expiry")
    require("BountyCancelled" in cancellation_events, "cancellation scenario is missing cancellation")
    require("RefundWithdrawn" in cancellation_events, "cancellation scenario is missing contributor refund")
    require("BountySettled" not in cancellation_events, "cancelled scenario cannot contain settlement")

    adversarial = rehearsal.get("adversarial_checks")
    require(isinstance(adversarial, dict), "adversarial checks are required")
    require(REQUIRED_ADVERSARIAL <= adversarial.keys(), "adversarial check set is incomplete")
    require(all(adversarial[name] is True for name in REQUIRED_ADVERSARIAL), "an adversarial check failed")
    frozen = rehearsal.get("bytecode_freeze")
    require(isinstance(frozen, dict), "bytecode freeze is required")
    require(frozen.get("verifier_runtime_code_hash") == expected["verifier"]["runtime_code_hash"], "verifier freeze mismatch")
    require(frozen.get("factory_runtime_code_hash") == expected["factory"]["runtime_code_hash"], "factory freeze mismatch")
    require(frozen.get("implementation_runtime_code_hash") == expected["factory"]["implementation_runtime_code_hash"], "implementation freeze mismatch")
    return {
        "schema_version": "agent-bounties/open-competition-v1-rehearsal-audit-v1",
        "passed": True,
        "source_commit": rehearsal["source_commit"],
        "factory": deployments["factory"]["address"],
        "verifier": deployments["verifier"]["address"],
        "scenario_count": 2,
        "adversarial_check_count": len(REQUIRED_ADVERSARIAL),
        "deployment_state": rehearsal["deployment_state"],
        "public_inventory_eligible": False,
        "evidence_boundary": "The audit proves manifest consistency, not mainnet readiness or payment."
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--rehearsal", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = audit(load(args.bundle), load(args.rehearsal))
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
