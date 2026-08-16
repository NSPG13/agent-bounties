#!/usr/bin/env python3
"""Verify that a distinct generated agent wallet completed the Beta3 x402 loop."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
from typing import Any
from uuid import UUID


HEX_ADDRESS = re.compile(r"^0x[0-9a-f]{40}$")
HEX_HASH = re.compile(r"^0x[0-9a-f]{64}$")


class FreshAgentFlowError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise FreshAgentFlowError(message)


def verify(rehearsal: dict[str, Any], success: dict[str, Any]) -> dict[str, Any]:
    require(rehearsal.get("passed") is True, "mainnet rehearsal did not pass")
    require(success.get("passed") is True, "x402 solver flow did not pass")
    require(
        rehearsal.get("network") == success.get("network") == "base-mainnet",
        "fresh agent flow must run on Base mainnet",
    )
    require(
        rehearsal.get("source_commit") == success.get("source_commit"),
        "fresh agent source commit differs from the canary",
    )
    derivation_id = rehearsal.get("actor_derivation_id")
    require(
        isinstance(derivation_id, str) and HEX_HASH.fullmatch(derivation_id),
        "fresh agent derivation identity is invalid",
    )
    require(
        success.get("actor_derivation_id") == derivation_id,
        "fresh agent derivation identity changed during x402 settlement",
    )

    actors = rehearsal.get("actors")
    require(isinstance(actors, dict), "rehearsal actor evidence is missing")
    deployer = actors.get("deployer")
    solver = actors.get("solver_a")
    other_solver = actors.get("solver_b")
    require(
        all(isinstance(value, str) and HEX_ADDRESS.fullmatch(value) for value in (deployer, solver, other_solver)),
        "rehearsal actor addresses are invalid",
    )
    require(len({deployer, solver, other_solver}) == 3, "generated solver is not a distinct wallet")
    require(success.get("solver") == solver, "x402 settlement used a different solver wallet")

    canary = rehearsal.get("x402_canary")
    require(isinstance(canary, dict) and canary.get("active") is True, "x402 canary was not active")
    require(canary.get("solver") == solver, "x402 canary was not bound to the generated solver")
    require(
        canary.get("competition") == success.get("competition"),
        "x402 settlement used a different competition",
    )
    require(success.get("generated_agent_wallet") is True, "solver was not marked as generated")
    require(success.get("manual_state_corrections") == 0, "solver flow required manual state correction")
    require(success.get("standard_exact") is True, "solver did not use standard-exact x402")
    require(success.get("eip3009") is True, "solver did not use EIP-3009 USDC authorization")

    for field in ("payment_transaction", "relay_transaction", "proof_hash", "public_values_hash"):
        require(
            isinstance(success.get(field), str) and HEX_HASH.fullmatch(success[field]),
            f"{field} is missing or invalid",
        )
    try:
        settlement_event_id = str(UUID(str(success.get("settlement_event_id"))))
    except (ValueError, TypeError, AttributeError) as error:
        raise FreshAgentFlowError("canonical settlement event id is invalid") from error

    transactions = rehearsal.get("transactions")
    require(isinstance(transactions, dict), "rehearsal transaction evidence is missing")
    require(
        "fund_solver_a_gas" in transactions,
        "fresh solver did not receive an isolated gas grant",
    )
    require(
        "fund_solver_a_usdc" in transactions,
        "fresh solver did not receive an isolated USDC budget",
    )

    return {
        "schema_version": "agent-bounties/open-competition-v2-beta3-fresh-agent-flow-v1",
        "passed": True,
        "network": "base-mainnet",
        "source_commit": success["source_commit"],
        "actor_derivation_id": derivation_id,
        "deployer": deployer,
        "solver": solver,
        "competition": success["competition"],
        "payment_transaction": success["payment_transaction"],
        "relay_transaction": success["relay_transaction"],
        "settlement_event_id": settlement_event_id,
        "manual_state_corrections": 0,
        "evidence_boundary": "This generated, release-attempt-scoped wallet received bounded gas and USDC, paid the x402 proof quote, authorized relay, and reached canonical CompetitionSettledV2 without manual state correction.",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rehearsal", type=Path, required=True)
    parser.add_argument("--x402-success", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = verify(
        json.loads(args.rehearsal.read_text(encoding="utf-8")),
        json.loads(args.x402_success.read_text(encoding="utf-8")),
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "solver": result["solver"], "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
