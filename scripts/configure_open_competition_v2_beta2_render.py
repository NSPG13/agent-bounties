#!/usr/bin/env python3
"""Reconcile and deploy the exact Open Competition V2 Beta2 Render runtime."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.parse
from typing import Any

import render_deploy_recovery as render


V2_GROUP = "agent-bounties-v2-beta2"
RELAYER_GROUP = "agent-bounties-x402-relayer"
V2_SERVICES = (
    render.ServiceSpec("agent-bounties-api", "web_service", "https://api.agentbounties.app/health"),
    render.ServiceSpec("agent-bounties-mcp", "web_service", "https://mcp.agentbounties.app/health"),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta2-indexer", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta2-shadow", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta2-keeper", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta2-broker", "background_worker", None),
)
RELAYER_SERVICE_NAMES = {
    "agent-bounties-api",
    "agent-bounties-open-competition-v2-beta2-keeper",
    "agent-bounties-open-competition-v2-beta2-broker",
}
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")


class Beta2RenderError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Beta2RenderError(message)


def validated_runtime(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(value.get("protocol_version") == "agent-bounties/open-competition-v2-beta2", "runtime protocol mismatch")
    require(value.get("network") == "base-mainnet", "Render accepts only the mainnet runtime")
    require(ADDRESS.fullmatch(str(value.get("factory_contract", ""))) is not None, "factory address is invalid")
    require(ADDRESS.fullmatch(str(value.get("settlement_token", ""))) is not None, "settlement token is invalid")
    require(HASH.fullmatch(str(value.get("release_hash", ""))) is not None, "release hash is invalid")
    require(HASH.fullmatch(str(value.get("beta_risk_hash", ""))) is not None, "risk hash is invalid")
    require(isinstance(value.get("deployment_block"), int) and value["deployment_block"] > 0, "deployment block is missing")
    return value


def runtime_environment(
    runtime: dict[str, Any],
    *,
    primary_rpc_url: str,
    shadow_rpc_url: str,
    prover_url: str,
    prover_api_key: str,
    broker_address: str,
) -> dict[str, str]:
    require(primary_rpc_url.startswith("https://"), "primary RPC must use HTTPS")
    require(shadow_rpc_url.startswith("https://"), "shadow RPC must use HTTPS")
    require(primary_rpc_url.rstrip("/") != shadow_rpc_url.rstrip("/"), "primary and shadow RPCs must differ")
    require(prover_url.startswith("https://"), "production prover must use HTTPS")
    require(len(prover_api_key) >= 32, "prover API key must contain at least 32 characters")
    require(ADDRESS.fullmatch(broker_address) is not None, "broker address is invalid")
    return {
        "BASE_MAINNET_OPEN_COMPETITION_V2_BETA2_RELEASE_MANIFEST_JSON": json.dumps(runtime, separators=(",", ":"), sort_keys=True),
        "OPEN_COMPETITION_V2_FACTORY_CONTRACT": runtime["factory_contract"].lower(),
        "OPEN_COMPETITION_V2_KEEPER_FACTORY": runtime["factory_contract"].lower(),
        "OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK": str(runtime["deployment_block"]),
        "OPEN_COMPETITION_V2_INDEXER_RPC_URL": primary_rpc_url,
        "OPEN_COMPETITION_V2_SHADOW_RPC_URL": shadow_rpc_url,
        "OPEN_COMPETITION_V2_PROVER_URL": prover_url,
        "OPEN_COMPETITION_V2_PROVER_API_KEY": prover_api_key,
        "OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS": broker_address.lower(),
        "OPEN_COMPETITION_V2_GROTH16_PROOF_FEE_BASE_UNITS": "100000",
        "OPEN_COMPETITION_V2_PLONK_PROOF_FEE_BASE_UNITS": "100000",
        "OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS": "10000",
        "OPEN_COMPETITION_V2_GROTH16_PROOF_SLA_SECONDS": "1800",
        "OPEN_COMPETITION_V2_PLONK_PROOF_SLA_SECONDS": "1800",
    }


def named_group(client: render.RenderClient, owner_id: str, name: str) -> dict[str, Any]:
    query = urllib.parse.urlencode({"name": name, "ownerId": owner_id, "limit": "20"})
    groups = [
        group
        for group in render.unwrap_env_group_entries(client._read_with_retry(f"/env-groups?{query}"))
        if group.get("name") == name and group.get("ownerId") == owner_id
    ]
    require(len(groups) == 1, f"expected exactly one Render environment group named {name}")
    group = groups[0]
    require(re.fullmatch(r"evg-[0-9a-z]+", str(group.get("id", ""))) is not None, f"{name} has an invalid id")
    return client.get_env_group(group["id"])


def deploy(
    client: render.RenderClient,
    runtime: dict[str, Any],
    *,
    revision: str,
    environment: dict[str, str],
    relayer_private_key: str,
    timeout_seconds: float,
    poll_seconds: float,
) -> dict[str, Any]:
    revision = render.validate_revision(revision)
    require(re.fullmatch(r"0x[0-9a-fA-F]{64}", relayer_private_key) is not None, "relayer private key is invalid")
    services = {spec.name: client.resolve_service(spec) for spec in V2_SERVICES}
    owner_ids = {str(service.get("ownerId")) for service in services.values()}
    require(len(owner_ids) == 1, "Beta2 services do not share one Render workspace")
    owner_id = render.validate_owner_id(owner_ids.pop())
    v2_group = named_group(client, owner_id, V2_GROUP)
    relayer_group = named_group(client, owner_id, RELAYER_GROUP)

    for spec in V2_SERVICES:
        service = services[spec.name]
        require(render.env_group_has_service(v2_group, spec, service["id"]), f"{V2_GROUP} is not linked to {spec.name}")
        if spec.name in RELAYER_SERVICE_NAMES:
            require(render.env_group_has_service(relayer_group, spec, service["id"]), f"{RELAYER_GROUP} is not linked to {spec.name}")
        client.disable_native_auto_deploy(service)

    changes = []
    for key, value in environment.items():
        record = client.ensure_env_group_env_var(v2_group, key, value)
        changes.append({"group": V2_GROUP, "key": key, "changed": bool(record["changed"])})
    record = client.ensure_env_group_env_var(relayer_group, "X402_RELAYER_PRIVATE_KEY", relayer_private_key)
    changes.append({"group": RELAYER_GROUP, "key": "X402_RELAYER_PRIVATE_KEY", "changed": bool(record["changed"])})

    pending: dict[str, tuple[dict[str, Any], str]] = {}
    for name, service in services.items():
        created = client.ensure_deploy(service, revision, force=True)
        deploy_id, status = render.validate_deploy(created, revision, name)
        if status == "live":
            continue
        if status in render.FAILED_STATUSES:
            raise render.deploy_failure(client, name, service, created)
        pending[name] = (service, deploy_id)
    completed = render.poll_deploys(
        client,
        pending,
        revision,
        timeout_seconds=timeout_seconds,
        poll_seconds=poll_seconds,
    )
    return {
        "schema_version": "agent-bounties/open-competition-v2-beta2-render-deployment-v1",
        "passed": True,
        "recorded_at": utc_now(),
        "revision": revision,
        "release_hash": runtime["release_hash"],
        "beta_risk_hash": runtime["beta_risk_hash"],
        "runtime_sha256": hashlib.sha256(json.dumps(runtime, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        "environment_changes": changes,
        "services": [
            {
                "name": name,
                "service_id": service["id"],
                "deploy_id": completed.get(name, {}).get("id"),
                "status": completed.get(name, {}).get("status", "already_live"),
            }
            for name, service in services.items()
        ],
        "secrets_redacted": True,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--primary-rpc-url", required=True)
    parser.add_argument("--shadow-rpc-url", required=True)
    parser.add_argument("--prover-url", required=True)
    parser.add_argument("--prover-api-key-env", default="OPEN_COMPETITION_V2_PROVER_API_KEY")
    parser.add_argument("--relayer-private-key-env", default="BASE_KEEPER_PRIVATE_KEY")
    parser.add_argument("--broker-address", required=True)
    parser.add_argument("--timeout-seconds", type=float, default=2400)
    parser.add_argument("--poll-seconds", type=float, default=10)
    parser.add_argument("--output", type=Path, required=True)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    runtime = validated_runtime(args.runtime)
    prover_api_key = os.environ.get(args.prover_api_key_env, "")
    relayer_private_key = os.environ.get(args.relayer_private_key_env, "")
    environment = runtime_environment(
        runtime,
        primary_rpc_url=args.primary_rpc_url,
        shadow_rpc_url=args.shadow_rpc_url,
        prover_url=args.prover_url,
        prover_api_key=prover_api_key,
        broker_address=args.broker_address,
    )
    client = render.RenderClient(os.environ.get("RENDER_API_KEY", ""))
    evidence = deploy(
        client,
        runtime,
        revision=args.revision,
        environment=environment,
        relayer_private_key=relayer_private_key,
        timeout_seconds=args.timeout_seconds,
        poll_seconds=args.poll_seconds,
    )
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
