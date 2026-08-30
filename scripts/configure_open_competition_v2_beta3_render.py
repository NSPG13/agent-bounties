#!/usr/bin/env python3
"""Reconcile and deploy the exact Open Competition V2 Beta3 Render runtime."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

from eth_account import Account

import render_deploy_recovery as render


V2_GROUP = "agent-bounties-v2-beta3"
RELAYER_GROUP = "agent-bounties-x402-relayer"
BASE_GROUP = "agent-bounties-base"
RPC_LOG_BATCH_SIZE = 50
V2_SERVICES = (
    render.ServiceSpec("agent-bounties-api", "web_service", "https://api.agentbounties.app/health"),
    render.ServiceSpec("agent-bounties-mcp", "web_service", "https://mcp.agentbounties.app/health"),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta3-indexer", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta3-shadow", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta3-keeper", "background_worker", None),
    render.ServiceSpec("agent-bounties-open-competition-v2-beta3-broker", "background_worker", None),
)
RELAYER_SERVICE_NAMES = {
    "agent-bounties-api",
    "agent-bounties-open-competition-v2-beta3-keeper",
    "agent-bounties-open-competition-v2-beta3-broker",
}
WORKER_ENVIRONMENT = {
    "agent-bounties-open-competition-v2-beta3-indexer": {
        "APP_PACKAGE": "worker",
        "APP_BINARY": "worker",
        "RUST_LOG": "info",
        "BASE_INDEXER_PROTOCOL": "open-competition-v2-beta3",
        "OPEN_COMPETITION_V2_INDEXER_NETWORK": "base-mainnet",
        "OPEN_COMPETITION_V2_INDEXER_POLL_SECONDS": "15",
        "OPEN_COMPETITION_V2_INDEXER_CONFIRMATIONS": "2",
        "OPEN_COMPETITION_V2_INDEXER_MAX_BLOCKS_PER_QUERY": str(RPC_LOG_BATCH_SIZE),
        "BASE_INDEXER_RETRY_INITIAL_SECONDS": "5",
        "BASE_INDEXER_RETRY_MAX_SECONDS": "120",
        "BASE_INDEXER_EXIT_AFTER_FAILURES": "8",
    },
    "agent-bounties-open-competition-v2-beta3-shadow": {
        "APP_PACKAGE": "worker",
        "APP_BINARY": "worker",
        "RUST_LOG": "info",
        "BASE_INDEXER_PROTOCOL": "open-competition-v2-shadow",
        "OPEN_COMPETITION_V2_INDEXER_NETWORK": "base-mainnet",
        "OPEN_COMPETITION_V2_INDEXER_MAX_BLOCKS_PER_QUERY": str(RPC_LOG_BATCH_SIZE),
        "OPEN_COMPETITION_V2_SHADOW_POLL_SECONDS": "30",
        "OPEN_COMPETITION_V2_SHADOW_REQUEST_DELAY_MS": "250",
    },
    "agent-bounties-open-competition-v2-beta3-keeper": {
        "APP_PACKAGE": "worker",
        "APP_BINARY": "worker",
        "RUST_LOG": "info",
        "BASE_INDEXER_PROTOCOL": "open-competition-v2-keeper",
        "OPEN_COMPETITION_V2_KEEPER_NETWORK": "base-mainnet",
        "OPEN_COMPETITION_V2_KEEPER_POLL_SECONDS": "5",
    },
    "agent-bounties-open-competition-v2-beta3-broker": {
        "APP_PACKAGE": "worker",
        "APP_BINARY": "worker",
        "RUST_LOG": "info",
        "BASE_INDEXER_PROTOCOL": "open-competition-v2-broker",
        "OPEN_COMPETITION_V2_BROKER_POLL_SECONDS": "5",
    },
}
ADDRESS = re.compile(r"^0x[0-9a-fA-F]{40}$")
HASH = re.compile(r"^0x[0-9a-fA-F]{64}$")


class Beta3RenderError(RuntimeError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise Beta3RenderError(message)


def rpc_call(
    url: str, method: str, params: list[Any], request_id: int, *, role: str
) -> Any:
    payload = json.dumps(
        {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params},
        separators=(",", ":"),
    ).encode()
    request = urllib.request.Request(
        url,
        data=payload,
        headers={
            "content-type": "application/json",
            "user-agent": "agent-bounties-beta3-rpc-preflight/1",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            value = json.loads(response.read())
    except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
        raise Beta3RenderError(
            f"{role} {method} RPC preflight failed: {type(error).__name__}"
        ) from None
    require(isinstance(value, dict), f"{role} {method} RPC response must be an object")
    error = value.get("error")
    if error is not None:
        code = error.get("code") if isinstance(error, dict) else "unknown"
        raise Beta3RenderError(
            f"{role} {method} RPC preflight returned error code={code}"
        )
    require("result" in value, f"{role} {method} RPC preflight omitted result")
    return value["result"]


def preflight_rpc_pair(
    runtime: dict[str, Any], primary_rpc_url: str, shadow_rpc_url: str
) -> dict[str, Any]:
    deployment_block = runtime["deployment_block"]
    query_end = deployment_block + RPC_LOG_BATCH_SIZE - 1
    factory = runtime["factory_contract"].lower()
    safe_blocks: list[dict[str, Any]] = []
    for offset, (role, url) in enumerate(
        (("primary", primary_rpc_url), ("shadow", shadow_rpc_url))
    ):
        chain_id = rpc_call(
            url, "eth_chainId", [], 910_000 + offset * 10, role=role
        )
        require(chain_id == "0x2105", "RPC preflight returned a non-Base-mainnet chain id")
        logs = rpc_call(
            url,
            "eth_getLogs",
            [
                {
                    "address": factory,
                    "fromBlock": hex(deployment_block),
                    "toBlock": hex(query_end),
                }
            ],
            910_001 + offset * 10,
            role=role,
        )
        require(isinstance(logs, list), "RPC preflight logs result must be a list")
        safe = rpc_call(
            url,
            "eth_getBlockByNumber",
            ["safe", False],
            910_002 + offset * 10,
            role=role,
        )
        require(isinstance(safe, dict), "RPC preflight safe block must be an object")
        try:
            number = int(str(safe["number"]), 16)
        except (KeyError, TypeError, ValueError):
            raise Beta3RenderError("RPC preflight safe block number is invalid") from None
        require(number >= deployment_block, "RPC safe head predates the Beta3 deployment")
        safe_blocks.append({"number": number, "hash": str(safe.get("hash", "")).lower()})

    common = min(item["number"] for item in safe_blocks)
    common_hashes = []
    for offset, (role, url) in enumerate(
        (("primary", primary_rpc_url), ("shadow", shadow_rpc_url))
    ):
        block = rpc_call(
            url,
            "eth_getBlockByNumber",
            [hex(common), False],
            910_003 + offset * 10,
            role=role,
        )
        require(isinstance(block, dict), "RPC preflight common block must be an object")
        common_hashes.append(str(block.get("hash", "")).lower())
    require(
        len(set(common_hashes)) == 1 and HASH.fullmatch(common_hashes[0]) is not None,
        "primary and shadow RPCs disagree on the common safe block",
    )
    max_lag = max(item["number"] - common for item in safe_blocks)
    require(max_lag <= 64, "primary and shadow RPC safe heads differ by more than 64 blocks")
    return {
        "passed": True,
        "chain_id": 8453,
        "factory_contract": factory,
        "archive_query_from_block": deployment_block,
        "archive_query_to_block": query_end,
        "common_safe_block": common,
        "common_safe_block_hash": common_hashes[0],
        "max_safe_head_lag_blocks": max_lag,
        "endpoints_redacted": True,
    }


def validated_runtime(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    require(value.get("protocol_version") == "agent-bounties/open-competition-v2-beta3", "runtime protocol mismatch")
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
    keeper_address: str,
    deployer_address: str,
    refund_reserve_min_base_units: int,
) -> dict[str, str]:
    require(primary_rpc_url.startswith("https://"), "primary RPC must use HTTPS")
    require(shadow_rpc_url.startswith("https://"), "shadow RPC must use HTTPS")
    require(primary_rpc_url.rstrip("/") != shadow_rpc_url.rstrip("/"), "primary and shadow RPCs must differ")
    require(prover_url.startswith("https://"), "production prover must use HTTPS")
    require(len(prover_api_key) >= 32, "prover API key must contain at least 32 characters")
    roles = {broker_address.lower(), keeper_address.lower(), deployer_address.lower()}
    require(
        all(ADDRESS.fullmatch(value) is not None for value in roles),
        "broker, keeper or deployer address is invalid",
    )
    require(len(roles) == 3, "broker, keeper and deployer addresses must be distinct")
    require(refund_reserve_min_base_units > 0, "refund reserve minimum must be positive")
    return {
        "BASE_MAINNET_OPEN_COMPETITION_V2_BETA3_RELEASE_MANIFEST_JSON": json.dumps(runtime, separators=(",", ":"), sort_keys=True),
        "OPEN_COMPETITION_V2_FACTORY_CONTRACT": runtime["factory_contract"].lower(),
        "OPEN_COMPETITION_V2_KEEPER_FACTORY": runtime["factory_contract"].lower(),
        "OPEN_COMPETITION_V2_DEPLOYMENT_BLOCK": str(runtime["deployment_block"]),
        "OPEN_COMPETITION_V2_INDEXER_RPC_URL": primary_rpc_url,
        "OPEN_COMPETITION_V2_SHADOW_RPC_URL": shadow_rpc_url,
        "OPEN_COMPETITION_V2_PROVER_URL": prover_url,
        "OPEN_COMPETITION_V2_PROVER_API_KEY": prover_api_key,
        "OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS": broker_address.lower(),
        "OPEN_COMPETITION_V2_REFUND_RESERVE_MIN_BASE_UNITS": str(refund_reserve_min_base_units),
        "OPEN_COMPETITION_V2_GROTH16_PROOF_FEE_BASE_UNITS": "100000",
        "OPEN_COMPETITION_V2_PLONK_PROOF_FEE_BASE_UNITS": "100000",
        "OPEN_COMPETITION_V2_RELAY_FEE_BASE_UNITS": "10000",
        "OPEN_COMPETITION_V2_GROTH16_PROOF_SLA_SECONDS": "1800",
        "OPEN_COMPETITION_V2_PLONK_PROOF_SLA_SECONDS": "1800",
        "OPEN_COMPETITION_V2_INDEXER_AGREEMENT_MAX_AGE_SECONDS": "300",
        "OPEN_COMPETITION_V2_INDEXER_MAX_LAG_BLOCKS": "64",
        "OPEN_COMPETITION_V2_PROVER_TIMEOUT_SECONDS": "120",
        "OPEN_COMPETITION_V2_BROKER_LEASE_SECONDS": "180",
        "OPEN_COMPETITION_V2_REFUND_WINDOW_SECONDS": "1800",
        "OPEN_COMPETITION_V2_RELAYER_MAX_GAS": "8000000",
        "OPEN_COMPETITION_V2_RELAYER_MAX_FEE_PER_GAS_WEI": "10000000000",
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


def ensure_base_mainnet_transaction_rpc(
    client: render.RenderClient, base_group: dict[str, Any], primary_rpc_url: str
) -> dict[str, Any]:
    require(primary_rpc_url.startswith("https://"), "Base mainnet transaction RPC must use HTTPS")
    record = client.ensure_env_group_env_var(
        base_group, "BASE_MAINNET_RPC_URL", primary_rpc_url
    )
    return {
        "group": BASE_GROUP,
        "key": "BASE_MAINNET_RPC_URL",
        "changed": bool(record["changed"]),
    }


def ensure_v2_group(
    client: render.RenderClient,
    owner_id: str,
    environment_id: str | None,
) -> dict[str, Any]:
    query = urllib.parse.urlencode({"name": V2_GROUP, "ownerId": owner_id, "limit": "20"})
    matches = [
        group
        for group in render.unwrap_env_group_entries(
            client._read_with_retry(f"/env-groups?{query}")
        )
        if group.get("name") == V2_GROUP and group.get("ownerId") == owner_id
    ]
    require(len(matches) <= 1, f"expected at most one Render environment group named {V2_GROUP}")
    if not matches:
        payload: dict[str, Any] = {
            "name": V2_GROUP,
            "ownerId": owner_id,
            "envVars": [],
            "secretFiles": [],
            "serviceIds": [],
        }
        if environment_id is not None:
            payload["environmentId"] = environment_id
        try:
            client._write_with_retry("POST", "/env-groups", payload)
        except render.RenderHttpError as error:
            if error.status != 409:
                raise
    group = named_group(client, owner_id, V2_GROUP)
    require(
        environment_id is None or group.get("environmentId") == environment_id,
        f"{V2_GROUP} is in an unexpected project environment",
    )
    return group


def ensure_group_link(
    client: render.RenderClient,
    group_name: str,
    group: dict[str, Any],
    spec: render.ServiceSpec,
    service: dict[str, Any],
) -> dict[str, Any]:
    current = client.get_env_group(group["id"])
    if not render.env_group_has_service(current, spec, service["id"]):
        try:
            client._write_with_retry(
                "POST", f"/env-groups/{group['id']}/services/{service['id']}", None
            )
        except render.RenderHttpError as error:
            if error.status != 409:
                raise
        current = client.get_env_group(group["id"])
    require(
        render.env_group_has_service(current, spec, service["id"]),
        f"Render did not attach {group_name} to {spec.name}",
    )
    return current


def provision_worker(
    client: render.RenderClient,
    spec: render.ServiceSpec,
    reference: dict[str, Any],
    groups: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    require(spec.name in WORKER_ENVIRONMENT, f"direct provisioning is not authorized for {spec.name}")
    require(
        reference.get("name") == "agent-bounties-api"
        and reference.get("type") == "web_service"
        and reference.get("branch") == "main"
        and render.normalize_repo(reference.get("repo")) == render.REPOSITORY,
        "Render provisioning reference service is invalid",
    )
    owner_id = render.validate_owner_id(reference.get("ownerId"))
    environment_id = reference.get("environmentId")
    if environment_id is not None:
        environment_id = render.validate_environment_id(environment_id)
    for name, group in groups.items():
        require(group.get("ownerId") == owner_id, f"{name} is in an unexpected workspace")
        group_environment = group.get("environmentId")
        if group_environment is not None:
            group_environment = render.validate_environment_id(group_environment)
        if environment_id is None:
            environment_id = group_environment
        require(
            group_environment is None or group_environment == environment_id,
            f"{name} is in an unexpected project environment",
        )
    database_url = render.validate_database_url(
        client.get_env_var(reference, "DATABASE_URL").get("value")
    )
    exact_environment = dict(WORKER_ENVIRONMENT[spec.name])
    exact_environment["DATABASE_URL"] = database_url
    payload: dict[str, Any] = {
        "type": "background_worker",
        "name": spec.name,
        "ownerId": owner_id,
        "repo": "https://github.com/NSPG13/agent-bounties",
        "branch": "main",
        "autoDeploy": "no",
        "envVars": [
            {"key": key, "value": value} for key, value in exact_environment.items()
        ],
        "serviceDetails": {
            "runtime": "docker",
            "envSpecificDetails": {
                "dockerContext": ".",
                "dockerfilePath": "./Dockerfile",
            },
            "plan": "starter",
            "region": "oregon",
            "maxShutdownDelaySeconds": 60,
        },
    }
    if environment_id is not None:
        payload["environmentId"] = environment_id
    try:
        created = render.select_service(
            spec, [client._write_with_retry("POST", "/services", payload)]
        )
    except render.RenderHttpError as error:
        if error.status != 409:
            raise
        created = client.resolve_service(spec)
    require(created.get("ownerId") == owner_id, "new Render worker changed workspaces")
    require(
        environment_id is None or created.get("environmentId") == environment_id,
        "new Render worker changed project environments",
    )
    for name, group in groups.items():
        ensure_group_link(client, name, group, spec, created)
    verified = client.resolve_service(spec)
    require(verified.get("ownerId") == owner_id, "provisioned Render worker changed workspaces")
    require(
        environment_id is None or verified.get("environmentId") == environment_id,
        "provisioned Render worker changed project environments",
    )
    for key, expected in exact_environment.items():
        require(
            client.get_env_var(verified, key) == {"key": key, "value": expected},
            f"provisioned Render worker did not retain {key}",
        )
    return verified


def resolve_services(client: render.RenderClient) -> dict[str, dict[str, Any]]:
    services: dict[str, dict[str, Any]] = {}
    missing: list[render.ServiceSpec] = []
    for spec in V2_SERVICES:
        try:
            services[spec.name] = client.resolve_service(spec)
        except render.RenderServiceMissing:
            missing.append(spec)
    if missing:
        reference = services.get("agent-bounties-api")
        require(reference is not None, "validated API reference service is unavailable")
        owner_id = render.validate_owner_id(reference.get("ownerId"))
        reference_environment = reference.get("environmentId")
        if reference_environment is not None:
            reference_environment = render.validate_environment_id(reference_environment)
        all_groups = {
            BASE_GROUP: named_group(client, owner_id, BASE_GROUP),
            V2_GROUP: ensure_v2_group(client, owner_id, reference_environment),
            RELAYER_GROUP: named_group(client, owner_id, RELAYER_GROUP),
        }
        for spec in missing:
            required = {BASE_GROUP: all_groups[BASE_GROUP], V2_GROUP: all_groups[V2_GROUP]}
            if spec.name in RELAYER_SERVICE_NAMES:
                required[RELAYER_GROUP] = all_groups[RELAYER_GROUP]
            services[spec.name] = provision_worker(client, spec, reference, required)
    if missing:
        services = {spec.name: client.resolve_service(spec) for spec in V2_SERVICES}
    return services


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
    require(
        Account.from_key(relayer_private_key).address.lower()
        == environment["OPEN_COMPETITION_V2_BROKER_PAYMENT_ADDRESS"],
        "relayer private key does not match the isolated broker address",
    )
    services = resolve_services(client)
    owner_ids = {str(service.get("ownerId")) for service in services.values()}
    require(len(owner_ids) == 1, "Beta3 services do not share one Render workspace")
    owner_id = render.validate_owner_id(owner_ids.pop())
    base_group = named_group(client, owner_id, BASE_GROUP)
    v2_group = named_group(client, owner_id, V2_GROUP)
    relayer_group = named_group(client, owner_id, RELAYER_GROUP)

    for spec in V2_SERVICES:
        service = services[spec.name]
        base_group = ensure_group_link(client, BASE_GROUP, base_group, spec, service)
        v2_group = ensure_group_link(client, V2_GROUP, v2_group, spec, service)
        if spec.name in RELAYER_SERVICE_NAMES:
            relayer_group = ensure_group_link(
                client, RELAYER_GROUP, relayer_group, spec, service
            )
        client.disable_native_auto_deploy(service)

    changes = [
        ensure_base_mainnet_transaction_rpc(
            client,
            base_group,
            environment["OPEN_COMPETITION_V2_INDEXER_RPC_URL"],
        )
    ]
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
        "schema_version": "agent-bounties/open-competition-v2-beta3-render-deployment-v1",
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
    parser.add_argument("--relayer-private-key-env", default="OPEN_COMPETITION_V2_BROKER_PRIVATE_KEY")
    parser.add_argument("--broker-address", required=True)
    parser.add_argument("--keeper-address", required=True)
    parser.add_argument("--deployer-address", required=True)
    parser.add_argument("--refund-reserve-min-base-units", type=int, default=110_000)
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
        keeper_address=args.keeper_address,
        deployer_address=args.deployer_address,
        refund_reserve_min_base_units=args.refund_reserve_min_base_units,
    )
    rpc_preflight = preflight_rpc_pair(
        runtime, args.primary_rpc_url, args.shadow_rpc_url
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
    evidence["rpc_preflight"] = rpc_preflight
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"passed": True, "output": str(args.output)}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
