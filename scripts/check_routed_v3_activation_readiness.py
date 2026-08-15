#!/usr/bin/env python3
"""Report whether the exact durable wallet policy is ready for routed-V3 activation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import activate_routed_v3_dynamic as dynamic
import activate_routed_v3_replacements as activation
from _shared.rpc import (
    RpcError,
    is_retryable_transport_output,
    ordered_base_rpc_endpoints,
    redact_rpc_endpoint,
    select_working_base_rpc,
)


class ReadOnlyFailoverCast(activation.Cast):
    """Run each read against a validated Base endpoint with bounded failover."""

    def __init__(self, executable: str, preferred: str) -> None:
        self._endpoints = ordered_base_rpc_endpoints(preferred or None)
        selected = select_working_base_rpc(
            endpoints=self._endpoints,
            max_retries=2,
        )
        super().__init__(executable, selected)

    def rpc(self, *args: str, timeout: int = 300) -> str:
        last_error: BaseException | None = None
        for endpoint in ordered_base_rpc_endpoints(self.rpc_url, self._endpoints):
            try:
                selected = select_working_base_rpc(
                    endpoints=(endpoint,),
                    max_retries=2,
                )
            except RpcError:
                raise
            except RuntimeError as error:
                last_error = error
                continue
            try:
                result = activation.run(
                    [self.executable, *args, "--rpc-url", selected],
                    timeout=timeout,
                )
            except activation.ActivationError as error:
                if not is_retryable_transport_output(error):
                    raise
                last_error = error
                continue
            self.rpc_url = selected
            return result
        raise activation.ActivationError(
            f"read-only Base RPC failover exhausted: {last_error}"
        ) from last_error

    def send_data(
        self, target: str, data: str, private_key: str
    ) -> dict[str, object]:
        raise activation.ActivationError(
            "read-only failover transport cannot broadcast transactions"
        )


def inspect(
    rpc_url: str,
    cast_bin: str,
    selected_rpc_output: Path | None = None,
) -> dict[str, object]:
    if selected_rpc_output is not None:
        selected_rpc_output.unlink(missing_ok=True)
    try:
        cast = ReadOnlyFailoverCast(cast_bin, rpc_url)
        deployment = dynamic.discover_deployment(cast)
        state = activation.policy_state(cast, deployment)
    except Exception as error:  # Readiness is deliberately fail-closed and non-throwing.
        return {
            "schema": "agent-bounties/routed-v3-activation-readiness-v1",
            "ready": False,
            "reason": str(error)[:2000],
            "financial_action_taken": False,
        }
    if selected_rpc_output is not None:
        selected_rpc_output.parent.mkdir(parents=True, exist_ok=True)
        selected_rpc_output.write_text(cast.rpc_url, encoding="utf-8")
    return {
        "schema": "agent-bounties/routed-v3-activation-readiness-v1",
        "ready": True,
        "reason": "durable wallet policy and direct-chain routed verifier policy are active",
        "wallet": activation.WALLET,
        "router": deployment["router_address"],
        "policy_hash": deployment["policy_hash"],
        "adapter": deployment["adapter_address"],
        "bootstrap_transaction": deployment["bootstrap_transaction"],
        "rpc_endpoint": redact_rpc_endpoint(cast.rpc_url),
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
    parser.add_argument(
        "--require-ready",
        action="store_true",
        help="Exit nonzero when canonical readiness cannot be proven",
    )
    parser.add_argument(
        "--selected-rpc-output",
        type=Path,
        help="Write the exact selected endpoint for a same-job follow-up without logging it",
    )
    args = parser.parse_args()
    report = inspect(args.rpc_url, args.cast, args.selected_rpc_output)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, sort_keys=True))
    return 0 if report["ready"] or not args.require_ready else 2


if __name__ == "__main__":
    raise SystemExit(main())
