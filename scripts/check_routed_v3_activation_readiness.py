#!/usr/bin/env python3
"""Report whether the exact durable wallet policy is ready for routed-V3 activation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import activate_routed_v3_dynamic as dynamic
import activate_routed_v3_replacements as activation


def inspect(rpc_url: str, cast_bin: str) -> dict[str, object]:
    try:
        cast = activation.Cast(cast_bin, rpc_url)
        deployment = dynamic.discover_deployment(cast)
        state = activation.policy_state(cast, deployment)
    except Exception as error:  # Readiness is deliberately fail-closed and non-throwing.
        return {
            "schema": "agent-bounties/routed-v3-activation-readiness-v1",
            "ready": False,
            "reason": str(error)[:2000],
            "financial_action_taken": False,
        }
    return {
        "schema": "agent-bounties/routed-v3-activation-readiness-v1",
        "ready": True,
        "reason": "durable wallet policy and direct-chain routed verifier policy are active",
        "wallet": activation.WALLET,
        "router": deployment["router_address"],
        "policy_hash": deployment["policy_hash"],
        "adapter": deployment["adapter_address"],
        "bootstrap_transaction": deployment["bootstrap_transaction"],
        "wallet_balance_base_units": state["wallet_balance"],
        "lifetime_spent_base_units": state["lifetime_spent"],
        "effective_period_spent_base_units": state["effective_period_spent"],
        "policy_version": state["policy_version"],
        "financial_action_taken": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=activation.RPC_DEFAULT)
    parser.add_argument("--cast", default="cast")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = inspect(args.rpc_url, args.cast)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
