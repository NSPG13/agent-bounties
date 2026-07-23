#!/usr/bin/env python3
"""Run routed-V3 replacement activation using direct Base router evidence."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any, Mapping

import activate_routed_v3_replacements as activation


ROUTER = "0x380c1af742593dd88b6f20387e9ee693a0536731"
EVENT_SIGNATURE = "PolicyBootstrapped(bytes32,address,bytes32)"
ACTIVATION_DELAY = 604_800
LOOKBACK_BLOCKS = 10_000


def parse_bootstrap_logs(raw: str) -> dict[str, str | int]:
    try:
        logs = json.loads(raw)
    except json.JSONDecodeError as error:
        raise activation.ActivationError("cast logs returned invalid JSON") from error
    if not isinstance(logs, list) or len(logs) != 1:
        count = len(logs) if isinstance(logs, list) else "non-list"
        raise activation.ActivationError(f"expected exactly one router bootstrap event, got {count}")
    log = logs[0]
    if not isinstance(log, dict):
        raise activation.ActivationError("router bootstrap event is not an object")
    topics = [str(value).strip().lower() for value in log.get("topics", [])]
    if len(topics) != 3:
        raise activation.ActivationError("router bootstrap event has the wrong topic shape")
    policy_hash = activation.bytes32(topics[1], "routed policy hash")
    adapter = activation.address("0x" + topics[2][-40:], "routed adapter")
    runtime_hash = activation.bytes32(log.get("data"), "routed adapter runtime hash")
    transaction_hash = activation.bytes32(log.get("transactionHash"), "bootstrap transaction")
    block_number = activation.parse_uint(log.get("blockNumber"), "bootstrap block")
    return {
        "policy_hash": policy_hash,
        "adapter": adapter,
        "adapter_runtime_code_hash": runtime_hash,
        "bootstrap_transaction": transaction_hash,
        "bootstrap_block": block_number,
    }


def code_hash(cast: activation.Cast, code: str, label: str) -> str:
    value = activation.run([cast.executable, "keccak", code])
    return activation.bytes32(value, f"{label} runtime code hash")


def discover_deployment(cast: activation.Cast) -> dict[str, Any]:
    if cast.chain_id() != activation.CHAIN_ID:
        raise activation.ActivationError("durable routed policy is pinned to Base mainnet")
    router_code = cast.code(ROUTER)
    if router_code in {"0x", "0x0"}:
        raise activation.ActivationError("durable verifier router runtime code is missing")
    latest = activation.parse_uint(cast.rpc("block-number"), "latest block")
    from_block = max(0, latest - LOOKBACK_BLOCKS)
    logs_raw = cast.rpc(
        "logs",
        "--address",
        ROUTER,
        "--from-block",
        str(from_block),
        "--to-block",
        "latest",
        EVENT_SIGNATURE,
        "--json",
    )
    event = parse_bootstrap_logs(logs_raw)
    policy_hash = str(event["policy_hash"])
    adapter = str(event["adapter"])
    adapter_runtime_hash = str(event["adapter_runtime_code_hash"])
    adapter_code = cast.code(adapter)
    if adapter_code in {"0x", "0x0"}:
        raise activation.ActivationError("routed V3 adapter runtime code is missing")
    observed_adapter_hash = code_hash(cast, adapter_code, "adapter")
    if observed_adapter_hash != adapter_runtime_hash:
        raise activation.ActivationError("routed adapter code hash differs from bootstrap evidence")

    factory = activation.address(cast.call(ROUTER, "canonicalFactory()(address)"), "router factory")
    registrar = activation.address(cast.call(ROUTER, "registrar()(address)"), "router registrar")
    guardian = activation.address(cast.call(ROUTER, "guardian()(address)"), "router guardian")
    delay = activation.parse_uint(cast.call(ROUTER, "activationDelay()(uint64)"), "router delay")
    bootstrap_used = activation.parse_bool(cast.call(ROUTER, "bootstrapUsed()(bool)"), "router bootstrap")
    active = activation.parse_bool(
        cast.call(ROUTER, "isPolicyActive(bytes32)(bool)", policy_hash), "routed policy active"
    )
    record = activation.lines(
        cast.call(
            ROUTER,
            "policies(bytes32)(address,bytes32,uint64,uint64,uint64,bool)",
            policy_hash,
        ),
        6,
        "router policy record",
    )
    if factory != activation.FACTORY or registrar != activation.KEEPER or guardian != activation.OWNER:
        raise activation.ActivationError("router authority or factory binding mismatch")
    if delay != ACTIVATION_DELAY or not bootstrap_used or not active:
        raise activation.ActivationError("router bootstrap or activation-delay invariant mismatch")
    if activation.address(record[0], "record adapter") != adapter:
        raise activation.ActivationError("router record adapter mismatch")
    if activation.bytes32(record[1], "record runtime hash") != adapter_runtime_hash:
        raise activation.ActivationError("router record runtime hash mismatch")
    if activation.parse_uint(record[4], "record activated at") == 0:
        raise activation.ActivationError("router record is not activated")
    if activation.parse_bool(record[5], "record vetoed"):
        raise activation.ActivationError("router record is vetoed")

    adapter_router = activation.address(cast.call(adapter, "verifierRouter()(address)"), "adapter router")
    adapter_policy = activation.bytes32(
        cast.call(adapter, "committedPolicyHash()(bytes32)"), "adapter policy hash"
    )
    adapter_factory = activation.address(cast.call(adapter, "canonicalFactory()(address)"), "adapter factory")
    acceptance_hash = activation.bytes32(
        cast.call(adapter, "ACCEPTANCE_CRITERIA_HASH()(bytes32)"), "adapter acceptance hash"
    )
    child_floor = activation.parse_uint(
        cast.call(adapter, "MINIMUM_CHILD_TARGET()(uint256)"), "minimum child target"
    )
    margin_floor = activation.parse_uint(
        cast.call(adapter, "MINIMUM_PARENT_GROSS_MARGIN()(uint256)"), "minimum parent margin"
    )
    if adapter_router != ROUTER or adapter_policy != policy_hash or adapter_factory != activation.FACTORY:
        raise activation.ActivationError("routed adapter immutable metadata mismatch")
    if child_floor != 1_000_000 or margin_floor != 1_000_000:
        raise activation.ActivationError("routed adapter economic invariant mismatch")

    return {
        "schema": "agent-bounties/durable-verifier-router-deployment-v1",
        "network": "base-mainnet",
        "chain_id": activation.CHAIN_ID,
        "router": {
            "address": ROUTER,
            "runtime_code_hash": code_hash(cast, router_code, "router"),
        },
        "router_address": ROUTER,
        "policy_hash": policy_hash,
        "adapter": {
            "address": adapter,
            "runtime_code_hash": adapter_runtime_hash,
            "acceptance_criteria_hash": acceptance_hash,
        },
        "adapter_address": adapter,
        "adapter_runtime_code_hash": adapter_runtime_hash,
        "bootstrap_transaction": event["bootstrap_transaction"],
        "bootstrap_block": event["bootstrap_block"],
        "activation_delay_seconds": delay,
        "direct_chain_evidence": True,
    }


def activate(args: argparse.Namespace) -> dict[str, Any]:
    cast = activation.Cast(args.cast, args.rpc_url)
    deployment = discover_deployment(cast)
    original = activation.load_deployment
    activation.load_deployment = lambda: deployment
    try:
        report = activation.activate(args)
    finally:
        activation.load_deployment = original
    report["deployment_evidence"] = deployment
    return report


def markdown(report: Mapping[str, object]) -> str:
    return activation.markdown(report)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc-url", default=os.environ.get("BASE_MAINNET_RPC_URL", activation.RPC_DEFAULT))
    parser.add_argument("--api", default=os.environ.get("AGENT_BOUNTIES_API_URL", activation.API_DEFAULT))
    parser.add_argument("--cast", default=os.environ.get("CAST_BIN", "cast"))
    parser.add_argument("--python", default=os.environ.get("PYTHON_BIN", "python"))
    parser.add_argument("--output-dir", type=Path, default=activation.ROOT / "target" / "routed-v3-activation")
    parser.add_argument("--output", type=Path, default=activation.ROOT / "target" / "routed-v3-activation.json")
    parser.add_argument(
        "--markdown-output", type=Path, default=activation.ROOT / "target" / "routed-v3-activation.md"
    )
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = activate(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.markdown_output.write_text(markdown(report), encoding="utf-8")
    print(
        json.dumps(
            {
                "activated": [item["issue"] for item in report["results"]],
                "wallet_balance_after": report["wallet_balance_after"],
                "policy_hash": report["policy_hash"],
                "output": str(args.output),
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
