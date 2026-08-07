#!/usr/bin/env python3
"""Fail-closed audit for an Open Competition V1 exact Base mainnet-fork replay."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any


HASH = re.compile(r"^0x[0-9a-f]{64}$")
ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")


class ForkReplayAuditError(ValueError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ForkReplayAuditError(message)


def reject_secrets(value: Any, location: str = "$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            require(key.lower() not in {"private_key", "mnemonic", "secret", "seed_phrase"}, f"secret at {location}.{key}")
            reject_secrets(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            reject_secrets(child, f"{location}[{index}]")


def audit(bundle: dict[str, Any], replay: dict[str, Any]) -> dict[str, Any]:
    reject_secrets(bundle)
    reject_secrets(replay)
    require(bundle.get("schema_version") == "agent-bounties/open-competition-v1-mainnet-bundle-v1", "bundle schema mismatch")
    require(replay.get("schema_version") == "agent-bounties/open-competition-v1-mainnet-fork-replay-v1", "replay schema mismatch")
    require(replay.get("protocol_version") == bundle.get("protocol_version"), "protocol mismatch")
    require(replay.get("network") == "base-mainnet-fork" and replay.get("chain_id") == 8453, "fork network mismatch")
    require(replay.get("source_commit") == bundle.get("source_commit"), "source commit mismatch")
    require(replay.get("fork_block") == bundle.get("preflight_block"), "fork block mismatch")
    require(replay.get("factory") == bundle.get("factory"), "factory mismatch")
    require(replay.get("implementation") == bundle.get("implementation"), "implementation mismatch")
    require(replay.get("verifier") == bundle["verifier_profile"]["verifier_address"], "verifier mismatch")
    require(replay.get("deployer") != replay.get("solver"), "creator/deployer cannot be solver")
    for field in ("factory", "implementation", "verifier", "solver", "bounty"):
        require(ADDRESS.fullmatch(str(replay.get(field, ""))) is not None, f"{field} address invalid")
    require(HASH.fullmatch(str(replay.get("bounty_id", ""))) is not None, "bounty id invalid")
    require(replay.get("passed") is True and replay.get("broadcast") is False, "replay boundary invalid")
    assertions = replay.get("assertions")
    require(isinstance(assertions, dict) and assertions and all(value is True for value in assertions.values()), "replay assertion failed")
    reconciliation = replay.get("usdc_reconciliation")
    require(isinstance(reconciliation, dict), "USDC reconciliation missing")
    require(reconciliation.get("admin_delta") == -1_100_000, "admin USDC delta mismatch")
    require(reconciliation.get("solver_delta") == 1_100_000, "solver USDC delta mismatch")
    require(reconciliation.get("bounty_after") == 0, "bounty retained USDC")
    settlement = replay.get("settlement_event")
    require(isinstance(settlement, dict) and settlement.get("name") == "BountySettled", "settlement event missing")
    require(HASH.fullmatch(str(settlement.get("transaction_hash", ""))) is not None, "settlement transaction invalid")
    require(HASH.fullmatch(str(settlement.get("block_hash", ""))) is not None, "settlement block hash invalid")
    require(isinstance(settlement.get("block_number"), int) and settlement["block_number"] > 0, "settlement block invalid")
    transactions = replay.get("transactions")
    required = {
        "factory_deployment",
        "solver_gas_funding",
        "solver_usdc_funding",
        "factory_approval",
        "competition_creation",
        "solver_bond_approval",
        "solution_commitment",
        "solution_reveal_and_settlement",
    }
    require(isinstance(transactions, dict) and required == transactions.keys(), "fork transaction set mismatch")
    for name, transaction in transactions.items():
        require(HASH.fullmatch(str(transaction.get("transaction_hash", ""))) is not None, f"{name} hash invalid")
        require(HASH.fullmatch(str(transaction.get("block_hash", ""))) is not None, f"{name} block hash invalid")
        require(isinstance(transaction.get("gas_used"), int) and transaction["gas_used"] > 0, f"{name} gas invalid")
    return {
        "schema_version": "agent-bounties/open-competition-v1-mainnet-fork-replay-audit-v1",
        "passed": True,
        "source_commit": replay["source_commit"],
        "fork_block": replay["fork_block"],
        "factory": replay["factory"],
        "bounty": replay["bounty"],
        "settlement_transaction": settlement["transaction_hash"],
        "public_inventory_eligible": False,
        "evidence_boundary": "This audit proves exact local fork-replay consistency. It is not a mainnet settlement or payment receipt.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--replay", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    report = audit(
        json.loads(args.bundle.read_text(encoding="utf-8")),
        json.loads(args.replay.read_text(encoding="utf-8")),
    )
    rendered = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
