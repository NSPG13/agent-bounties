#!/usr/bin/env python3
"""Promote only Beta3 activation gates while preserving deployed release identity."""

from __future__ import annotations

import argparse
from copy import deepcopy
import json
from pathlib import Path
from typing import Any

import build_open_competition_v2_beta3_release as release


class PromotionError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise PromotionError(message)


def promote(
    bundle: dict[str, Any],
    deployment: dict[str, Any],
    gates: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    require(bundle.get("protocol_version") == release.PROTOCOL_VERSION, "bundle protocol mismatch")
    require(bundle.get("network") in release.NETWORKS, "bundle network mismatch")
    require(deployment.get("complete") is True, "deployment evidence is incomplete")
    deployed_runtime = deployment.get("runtime_manifest")
    require(isinstance(deployed_runtime, dict), "deployment runtime manifest is missing")
    require(deployed_runtime.get("factory_contract") == bundle["factory"]["address"], "deployed factory differs from bundle")
    require(deployed_runtime.get("release_hash") == bundle["source_tree_hash"], "deployed release hash differs from bundle")
    require(gates.get("subject_hash") == bundle["repository_subject"]["hash"], "gate subject differs from bundle")
    deployment_block = deployed_runtime.get("deployment_block")
    require(isinstance(deployment_block, int) and deployment_block > 0, "deployment block is invalid")

    promoted = deepcopy(bundle)
    promoted["release_gates"] = gates
    promoted["activation"] = release.activation_state(bundle["network"], gates)
    runtime = release.runtime_manifest(promoted, deployment_block)
    for key in (
        "factory_contract",
        "implementation_contract",
        "groth16_verifier",
        "plonk_verifier",
        "release_hash",
        "beta_risk_hash",
    ):
        require(runtime[key] == deployed_runtime[key], f"promotion changed deployed identity: {key}")
    return promoted, runtime


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--deployment", type=Path, required=True)
    parser.add_argument("--gates", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runtime-output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    bundle = json.loads(args.bundle.read_text(encoding="utf-8"))
    deployment = json.loads(args.deployment.read_text(encoding="utf-8"))
    gates = release.load_gates(args.gates, bundle["repository_subject"]["hash"])
    promoted, runtime = promote(bundle, deployment, gates)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(promoted, indent=2) + "\n", encoding="utf-8")
    args.runtime_output.parent.mkdir(parents=True, exist_ok=True)
    args.runtime_output.write_text(json.dumps(runtime, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "runtime": str(args.runtime_output), "activation": promoted["activation"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
