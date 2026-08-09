from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable


SCHEMA = "agent-bounties/render-deploy-evidence-v1"
REPOSITORY = "github.com/nspg13/agent-bounties"
PROTOCOL = "agent-bounties/autonomous-v1"
BLUEPRINT_PATH = "render.yaml"
BLUEPRINT_RECOVERABLE_SERVICE_NAMES = frozenset(
    {"agent-bounties-open-competition-v1-indexer"}
)
BLUEPRINT_STATUSES = {"created", "paused", "in_sync", "syncing", "error"}
ENV_GROUP_SERVICE_TYPES = {
    "static_site": "static",
    "web_service": "web",
    "private_service": "pserv",
    "background_worker": "worker",
    "cron_job": "cron",
}
OPEN_COMPETITION_WORKER_NAME = "agent-bounties-open-competition-v1-indexer"
OPEN_COMPETITION_REFERENCE_SERVICE_NAME = "agent-bounties-base-indexer"
OPEN_COMPETITION_ENV_GROUP_NAME = "agent-bounties-base"
OPEN_COMPETITION_RELEASE_MANIFEST_PATH = Path(
    "deployments/open-competition-v1-base-mainnet.json"
)
OPEN_COMPETITION_ENTRANT_RELEASE_AUDIT_PATH = Path(
    "deployments/open-competition-entrant-wallet-v1-base-mainnet.json"
)
OPEN_COMPETITION_ENTRANT_FACTORY = "0x9b92a65a42de770157f30dd75f44a3136f2cda79"
OPEN_COMPETITION_ENTRANT_IMPLEMENTATION = (
    "0xd7890aa6c4d4c981c246a05576a6fc689255923c"
)
OPEN_COMPETITION_FACTORY = "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5"
BASE_MAINNET_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
OPEN_COMPETITION_ENTRANT_RUNTIME_HASHES = {
    "factory_runtime_code_hash": (
        "0xa0596a53e2f4685d104c2f24176307edfcb4fe8f0fd86162378347996c8f3c40"
    ),
    "implementation_runtime_code_hash": (
        "0xd1789de47b6c956b090f4fcf693361ef93ad4aeeec74ddc359bdcf73cb1ea998"
    ),
    "clone_runtime_code_hash": (
        "0xe94f67382a2692b2ebe7f71ab4163ae0c9c16bded92d45695454deed927b01d4"
    ),
}
HOSTED_BASE_MAINNET_RPC_URL = "https://base.drpc.org"
OPEN_COMPETITION_WORKER_ENVIRONMENT = {
    "APP_PACKAGE": "worker",
    "APP_BINARY": "worker",
    "RUST_LOG": "info",
    "PUBLIC_BASE_URL": "https://api.agentbounties.app",
    "BASE_INDEXER_PROTOCOL": "open-competition-v1",
    "OPEN_COMPETITION_INDEXER_NETWORK": "base-mainnet",
    "OPEN_COMPETITION_V1_FACTORY_CONTRACT": (
        "0x9e9382beb8b1a45b737d484b5eafa7b8779d4ca5"
    ),
    "OPEN_COMPETITION_V1_DEPLOYMENT_BLOCK": "49663931",
    "OPEN_COMPETITION_INDEXER_RPC_URL": HOSTED_BASE_MAINNET_RPC_URL,
    "OPEN_COMPETITION_INDEXER_POLL_SECONDS": "15",
    "OPEN_COMPETITION_INDEXER_CONFIRMATIONS": "2",
    "OPEN_COMPETITION_INDEXER_MAX_BLOCKS_PER_QUERY": "2000",
    "BASE_INDEXER_RETRY_INITIAL_SECONDS": "5",
    "BASE_INDEXER_RETRY_MAX_SECONDS": "120",
    "BASE_INDEXER_EXIT_AFTER_FAILURES": "8",
}
HEALTH_STABILITY_PROBES = 8
DEPLOY_MODES = {"build_and_deploy", "deploy_only"}
ACTIVE_STATUSES = {
    "created",
    "queued",
    "build_in_progress",
    "pre_deploy_in_progress",
    "update_in_progress",
}
FAILED_STATUSES = {
    "build_failed",
    "pre_deploy_failed",
    "update_failed",
    "canceled",
    "deactivated",
}
TRANSIENT_HTTP_STATUSES = {429, 500, 503}
CUSTOM_DOMAINS = {
    # Render Hobby permits two custom domains per workspace. Keep runtime
    # domains here; marketing and legacy hosts redirect at the DNS edge.
    "agent-bounties-api": ("api.agentbounties.app",),
    "agent-bounties-mcp": ("mcp.agentbounties.app",),
}
PUBLIC_ENV_SERVICE_NAMES = {
    "agent-bounties-api",
    "agent-bounties-mcp",
}
CLOUD_AGENT_API_SERVICE_NAME = "agent-bounties-api"
API_RUNTIME_ENVIRONMENT = {
    "AGENT_BOUNTIES_SOCIAL_MENTION_DRAFTS_ENABLED": "true",
    # The hosted relayer applies its own 120% padding after eth_estimateGas.
    # Relayed Open Competition commits use roughly 420k gas in the frozen
    # mainnet-fork replay, so the former 300k x402-only cap rejected the exact
    # canary action before broadcast.
    "X402_RELAYER_MAX_GAS": "700000",
}
CLOUD_AGENT_RUNTIME_ENVIRONMENT = {
    "CLOUD_AGENT_ENABLED": "true",
    "CLOUD_AGENT_PUBLIC_DRAFTS": "true",
    "CLOUD_AGENT_PROVIDER": "openai",
    "CLOUD_AGENT_PROTOCOL": "openai_responses",
    "CLOUD_AGENT_ENDPOINT": "https://api.openai.com/v1/responses",
    "CLOUD_AGENT_MODEL": "gpt-5.6-luna",
    "CLOUD_AGENT_REASONING_EFFORT": "low",
    "CLOUD_AGENT_MAX_INPUT_CHARS": "12000",
    "CLOUD_AGENT_MAX_OUTPUT_TOKENS": "12000",
    "CLOUD_AGENT_MAX_DAILY_DRAFTS": "50",
    "CLOUD_AGENT_TIMEOUT_SECONDS": "90",
}


class RecoveryError(RuntimeError):
    pass


class RenderServiceMissing(RecoveryError):
    def __init__(self, service_name: str) -> None:
        self.service_name = service_name
        super().__init__(f"expected exactly one Render service named {service_name}; found 0")


class RenderHttpError(RecoveryError):
    def __init__(self, status: int, body: str) -> None:
        self.status = status
        super().__init__(f"Render API returned HTTP {status}: {redact(body)}")


class RenderTransportError(RecoveryError):
    pass


class RenderDeployFailure(RecoveryError):
    def __init__(self, message: str, evidence: dict[str, Any]) -> None:
        self.evidence = evidence
        super().__init__(message)


class NeynarClient:
    def __init__(
        self,
        api_key: str,
        *,
        api_base: str = "https://api.neynar.com",
        timeout_seconds: float = 20,
    ) -> None:
        if not api_key.strip():
            raise RecoveryError("NEYNAR_API_KEY is required")
        self._api_key = api_key.strip()
        self._api_base = api_base.rstrip("/")
        self._timeout_seconds = timeout_seconds

    def request_json(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> Any:
        body = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self._api_base}{path}",
            data=body,
            method=method,
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "x-api-key": self._api_key,
                "User-Agent": "agent-bounties-neynar-provisioner/1",
            },
        )
        try:
            with urllib.request.urlopen(
                request, timeout=self._timeout_seconds
            ) as response:
                response_body = response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            error.read()
            raise RecoveryError(
                f"Neynar API returned HTTP {error.code}"
            ) from None
        except (urllib.error.URLError, TimeoutError) as error:
            raise RecoveryError(
                f"Neynar API transport failed: {redact(str(error))}"
            ) from None
        try:
            return json.loads(response_body) if response_body else {}
        except json.JSONDecodeError:
            raise RecoveryError("Neynar API returned invalid JSON") from None


@dataclass(frozen=True)
class ServiceSpec:
    name: str
    service_type: str
    health_url: str | None


SERVICE_SPECS = (
    ServiceSpec(
        "agent-bounties-api",
        "web_service",
        "https://agent-bounties-api.onrender.com/health",
    ),
    ServiceSpec(
        "agent-bounties-mcp",
        "web_service",
        "https://agent-bounties-mcp.onrender.com/health",
    ),
    ServiceSpec("agent-bounties-base-indexer", "background_worker", None),
    ServiceSpec(
        "agent-bounties-open-competition-v1-indexer",
        "background_worker",
        None,
    ),
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def redact(value: str) -> str:
    value = re.sub(r"(?i)(authorization:\s*bearer\s+)[^\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)([?&](?:key|token)=)[^&\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)(render_api_key\s*[=:]\s*)[^\s,;]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)postgres(?:ql)?://[^\s\"']+", "[database-url-redacted]", value)
    return value[:1000]


BUILD_FAILURE_PATTERNS = {
    "cargo_lock": ("cargo.lock needs to be updated", "lock file needs to be updated"),
    "checkout": ("fatal:", "remote ref", "reference is not a tree", "repository not found"),
    "compile": ("could not compile", "error[e", "compilation failed"),
    "configuration": ("docker context", "dockerfile", "invalid", "root directory"),
    "docker": ("failed to solve", "did not complete successfully"),
    "missing_file": ("does not exist", "is missing", "no such file or directory", "not found"),
    "network": ("connection reset", "connection timed out", "temporary failure"),
    "pipeline_quota": ("build pipeline minutes", "build spend limit"),
    "resource_limit": ("no space left", "out of memory", "signal: 9", "killed"),
    "rust_toolchain": ("requires rustc", "rustc version", "rust version"),
}


def summarize_build_logs(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict) or not isinstance(payload.get("logs"), list):
        raise RecoveryError("Render build-log response is invalid")
    messages = [
        item.get("message", "")
        for item in payload["logs"]
        if isinstance(item, dict) and isinstance(item.get("message"), str)
    ]
    normalized = "\n".join(messages).lower()
    codes = sorted(
        code
        for code, patterns in BUILD_FAILURE_PATTERNS.items()
        if any(pattern in normalized for pattern in patterns)
    )
    excerpts: list[str] = []
    for message in reversed(messages):
        for raw_line in reversed(message.splitlines()):
            line = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", raw_line).strip()
            if not line:
                continue
            if re.search(
                r"(?i)(authorization|bearer|password|private|secret|token|api[_ -]?key|database_url)",
                line,
            ):
                line = "[sensitive build diagnostic redacted]"
            else:
                line = re.sub(r"https?://\S+", "[url]", line)
                line = re.sub(r"\b0x[0-9a-fA-F]{32,}\b", "[hex-redacted]", line)
                line = re.sub(r"\b[A-Za-z0-9_+/=-]{40,}\b", "[value-redacted]", line)
                line = redact(line).replace("`", "'")
            if line not in excerpts:
                excerpts.append(line[:300])
            if len(excerpts) == 8:
                break
        if len(excerpts) == 8:
            break
    return {
        "available": True,
        "classifications": codes or ["unclassified"],
        "excerpts": excerpts,
        "log_count": len(messages),
        "content_sha256": hashlib.sha256(
            "\n".join(messages).encode("utf-8")
        ).hexdigest(),
    }


def validate_revision(revision: str) -> str:
    normalized = revision.strip().lower()
    if not re.fullmatch(r"[0-9a-f]{40}", normalized):
        raise RecoveryError("revision must be an exact 40-character Git SHA")
    return normalized


def validate_deploy_mode(value: str) -> str:
    mode = value.strip().lower()
    if mode not in DEPLOY_MODES:
        raise RecoveryError("deploy mode must be build_and_deploy or deploy_only")
    return mode


def normalize_repo(value: object) -> str:
    if not isinstance(value, str):
        return ""
    normalized = value.strip().lower().removesuffix(".git")
    normalized = re.sub(r"^(?:https?://|git@)", "", normalized)
    return normalized.replace(":", "/", 1) if normalized.startswith("github.com:") else normalized


def unwrap_service_entries(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, list):
        raise RecoveryError("Render service-list response must be an array")
    services: list[dict[str, Any]] = []
    for entry in payload:
        if not isinstance(entry, dict):
            continue
        service = entry.get("service", entry)
        if isinstance(service, dict):
            services.append(service)
    return services


def select_service(spec: ServiceSpec, payload: object) -> dict[str, Any]:
    matches = [
        service
        for service in unwrap_service_entries(payload)
        if service.get("name") == spec.name
    ]
    if not matches:
        raise RenderServiceMissing(spec.name)
    if len(matches) != 1:
        raise RecoveryError(
            f"expected exactly one Render service named {spec.name}; found {len(matches)}"
        )
    service = matches[0]
    if service.get("type") != spec.service_type:
        raise RecoveryError(f"{spec.name} has unexpected Render service type")
    if service.get("branch") != "main":
        raise RecoveryError(f"{spec.name} is not connected to the main branch")
    if normalize_repo(service.get("repo")) != REPOSITORY:
        raise RecoveryError(f"{spec.name} is connected to an unexpected repository")
    service_id = service.get("id")
    if not isinstance(service_id, str) or not re.fullmatch(r"srv-[0-9a-z]+", service_id):
        raise RecoveryError(f"{spec.name} has an invalid Render service id")
    return service


def unwrap_blueprint_entries(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, list):
        raise RecoveryError("Render Blueprint-list response must be an array")
    blueprints: list[dict[str, Any]] = []
    for entry in payload:
        if not isinstance(entry, dict):
            continue
        blueprint = entry.get("blueprint", entry)
        if isinstance(blueprint, dict):
            blueprints.append(blueprint)
    return blueprints


def validate_blueprint(blueprint: object) -> dict[str, Any]:
    if not isinstance(blueprint, dict):
        raise RecoveryError("Render Blueprint response must be an object")
    blueprint = blueprint.get("blueprint", blueprint)
    if not isinstance(blueprint, dict):
        raise RecoveryError("Render Blueprint response is missing metadata")
    blueprint_id = blueprint.get("id")
    if not isinstance(blueprint_id, str) or not re.fullmatch(
        r"exs-[0-9a-z]{20}", blueprint_id
    ):
        raise RecoveryError("Render Blueprint has an invalid id")
    if normalize_repo(blueprint.get("repo")) != REPOSITORY:
        raise RecoveryError("Render Blueprint is connected to an unexpected repository")
    if blueprint.get("branch") != "main":
        raise RecoveryError("Render Blueprint is not connected to the main branch")
    if blueprint.get("path") != BLUEPRINT_PATH:
        raise RecoveryError("Render Blueprint uses an unexpected file path")
    status = blueprint.get("status")
    if status not in BLUEPRINT_STATUSES:
        raise RecoveryError("Render Blueprint has an unknown status")
    if not isinstance(blueprint.get("autoSync"), bool):
        raise RecoveryError("Render Blueprint is missing its Auto Sync state")
    resources = blueprint.get("resources", [])
    if not isinstance(resources, list):
        raise RecoveryError("Render Blueprint resources must be an array")
    return blueprint


def select_blueprint(payload: object) -> dict[str, Any]:
    candidates = [
        blueprint
        for blueprint in unwrap_blueprint_entries(payload)
        if normalize_repo(blueprint.get("repo")) == REPOSITORY
        and blueprint.get("branch") == "main"
        and blueprint.get("path") == BLUEPRINT_PATH
    ]
    if len(candidates) != 1:
        raise RecoveryError(
            "expected exactly one Render Blueprint for the repository main branch and "
            f"{BLUEPRINT_PATH}; found {len(candidates)}"
        )
    return validate_blueprint(candidates[0])


def blueprint_has_service(
    blueprint: dict[str, Any], spec: ServiceSpec
) -> bool:
    matches = [
        resource
        for resource in blueprint.get("resources", [])
        if isinstance(resource, dict) and resource.get("name") == spec.name
    ]
    if len(matches) > 1:
        raise RecoveryError(f"Render Blueprint has duplicate resources named {spec.name}")
    if not matches:
        return False
    if matches[0].get("type") != spec.service_type:
        raise RecoveryError(f"Render Blueprint has an unexpected type for {spec.name}")
    return True


def unwrap_deploy(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise RecoveryError("Render deploy response must be an object")
    deploy = payload.get("deploy", payload)
    if not isinstance(deploy, dict):
        raise RecoveryError("Render deploy response is missing deploy metadata")
    return deploy


def unwrap_custom_domains(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, list):
        raise RecoveryError("Render custom-domain response must be an array")
    domains: list[dict[str, Any]] = []
    for entry in payload:
        if not isinstance(entry, dict):
            continue
        domain = entry.get("customDomain", entry)
        if isinstance(domain, dict):
            domains.append(domain)
    return domains


def unwrap_env_var(payload: object) -> dict[str, str]:
    if not isinstance(payload, dict):
        raise RecoveryError("Render environment-variable response must be an object")
    env_var = payload.get("envVar", payload)
    if not isinstance(env_var, dict):
        raise RecoveryError("Render environment-variable response is missing metadata")
    key = env_var.get("key")
    value = env_var.get("value")
    if not isinstance(key, str) or not isinstance(value, str):
        raise RecoveryError("Render environment-variable response is incomplete")
    return {"key": key, "value": value}


def validate_owner_id(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-z]+-[0-9a-z]+", value):
        raise RecoveryError("Render service is missing its workspace id")
    return value


def validate_environment_id(value: object) -> str:
    if not isinstance(value, str) or not re.fullmatch(r"[a-z]+-[0-9a-z]+", value):
        raise RecoveryError("Render service has an invalid project environment id")
    return value


def validate_database_url(value: object) -> str:
    if not isinstance(value, str) or not value:
        raise RecoveryError("reference Render worker has no DATABASE_URL")
    try:
        parsed = urllib.parse.urlsplit(value)
    except ValueError:
        raise RecoveryError("reference Render worker has an invalid DATABASE_URL") from None
    if parsed.scheme not in {"postgres", "postgresql"} or not parsed.hostname:
        raise RecoveryError("reference Render worker has an invalid DATABASE_URL")
    return value


def unwrap_env_group_entries(payload: object) -> list[dict[str, Any]]:
    if not isinstance(payload, list):
        raise RecoveryError("Render environment-group list response must be an array")
    groups: list[dict[str, Any]] = []
    for entry in payload:
        if not isinstance(entry, dict):
            continue
        group = entry.get("envGroup", entry)
        if isinstance(group, dict):
            groups.append(group)
    return groups


def select_env_group(payload: object, owner_id: str) -> dict[str, Any]:
    matches = [
        group
        for group in unwrap_env_group_entries(payload)
        if group.get("name") == OPEN_COMPETITION_ENV_GROUP_NAME
        and group.get("ownerId") == owner_id
    ]
    if len(matches) != 1:
        raise RecoveryError(
            "expected exactly one Render environment group named "
            f"{OPEN_COMPETITION_ENV_GROUP_NAME} in the reference workspace; "
            f"found {len(matches)}"
        )
    group = matches[0]
    group_id = group.get("id")
    if not isinstance(group_id, str) or not re.fullmatch(r"evg-[0-9a-z]+", group_id):
        raise RecoveryError("Render environment group has an invalid id")
    return group


def unwrap_env_group(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise RecoveryError("Render environment-group response must be an object")
    group = payload.get("envGroup", payload)
    if not isinstance(group, dict):
        raise RecoveryError("Render environment-group response is missing metadata")
    return group


def env_group_env_var(payload: object, key: str) -> dict[str, str]:
    group = unwrap_env_group(payload)
    env_vars = group.get("envVars")
    if not isinstance(env_vars, list):
        raise RecoveryError("Render environment group is missing environment variables")
    matches = [
        item
        for item in env_vars
        if isinstance(item, dict) and item.get("key") == key
    ]
    if len(matches) != 1:
        raise RecoveryError(
            f"Render environment group returned {len(matches)} values for {key}"
        )
    value = matches[0].get("value")
    if not isinstance(value, str):
        raise RecoveryError(f"Render environment group returned an invalid value for {key}")
    return {"key": key, "value": value}


def env_group_has_service(
    payload: object,
    spec: ServiceSpec,
    service_id: str,
) -> bool:
    group = unwrap_env_group(payload)
    links = group.get("serviceLinks")
    if not isinstance(links, list):
        raise RecoveryError("Render environment group is missing service links")
    matches: list[dict[str, Any]] = []
    for link in links:
        if not isinstance(link, dict):
            continue
        service = link.get("service", link)
        if isinstance(service, dict) and service.get("id") == service_id:
            matches.append(service)
    if len(matches) > 1:
        raise RecoveryError("Render environment group has duplicate worker links")
    if not matches:
        return False
    service = matches[0]
    if (
        service.get("name") != spec.name
        or service.get("type") != ENV_GROUP_SERVICE_TYPES.get(spec.service_type)
    ):
        raise RecoveryError("Render environment group linked an unexpected service")
    return True


def normalize_public_base_url(name: str, value: str) -> str:
    candidate = value.strip().rstrip("/")
    try:
        parsed = urllib.parse.urlsplit(candidate)
        port = parsed.port
    except ValueError as error:
        raise RecoveryError(f"{name} is not a valid URL: {error}") from None
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or port is not None
        or parsed.path
        or parsed.query
        or parsed.fragment
    ):
        raise RecoveryError(f"{name} must be an HTTPS origin without credentials or a path")
    return candidate


def public_environment_values(
    public_base_url: str,
    mcp_base_url: str,
    website_base_url: str,
) -> dict[str, str]:
    return {
        "PUBLIC_BASE_URL": normalize_public_base_url(
            "PUBLIC_BASE_URL", public_base_url
        ),
        "MCP_BASE_URL": normalize_public_base_url("MCP_BASE_URL", mcp_base_url),
        "WEBSITE_BASE_URL": normalize_public_base_url(
            "WEBSITE_BASE_URL", website_base_url
        ),
    }


def open_competition_shared_environment(
    manifest_path: Path = OPEN_COMPETITION_RELEASE_MANIFEST_PATH,
    entrant_release_audit_path: Path = OPEN_COMPETITION_ENTRANT_RELEASE_AUDIT_PATH,
    *,
    entrant_relay_canary_enabled: bool = False,
) -> dict[str, str]:
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RecoveryError(
            f"Open Competition release manifest is unavailable: {redact(str(error))}"
        ) from None
    if not isinstance(manifest, dict):
        raise RecoveryError("Open Competition release manifest must be an object")
    if (
        manifest.get("schema_version")
        != "agent-bounties/open-competition-v1-base-mainnet-release-v1"
        or manifest.get("protocol_version")
        != "agent-bounties/open-competition-v1"
        or manifest.get("network") != "base-mainnet"
        or manifest.get("chain_id") != 8453
        or manifest.get("deployment_state")
        != "mainnet_canary_not_ready_to_earn"
    ):
        raise RecoveryError("Open Competition release manifest identity is invalid")

    release = manifest.get("release_manifest")
    catalog = manifest.get("verifier_catalog")
    activation = manifest.get("hosted_activation")
    if not isinstance(release, dict) or not isinstance(catalog, dict):
        raise RecoveryError("Open Competition release manifest is incomplete")
    if not isinstance(activation, dict):
        raise RecoveryError("Open Competition hosted activation evidence is missing")
    if (
        release.get("protocol_version") != "agent-bounties/open-competition-v1"
        or release.get("network") != "base-mainnet"
        or release.get("chain_id") != 8453
        or release.get("deployment_state")
        != "mainnet_canary_not_ready_to_earn"
    ):
        raise RecoveryError("Open Competition hosted release identity is invalid")
    profiles = catalog.get("profiles")
    if (
        catalog.get("schema_version")
        != "agent-bounties/open-competition-v1-verifier-catalog-v1"
        or catalog.get("protocol_version")
        != "agent-bounties/open-competition-v1"
        or catalog.get("network") != "base-mainnet"
        or not isinstance(profiles, list)
        or not profiles
    ):
        raise RecoveryError("Open Competition verifier catalog identity is invalid")
    for profile in profiles:
        if (
            not isinstance(profile, dict)
            or profile.get("deployment_state")
            != "mainnet_canary_not_ready_to_earn"
            or profile.get("public_inventory_eligible") is not False
        ):
            raise RecoveryError("Open Competition verifier profile is not hidden-canary safe")

    activation_fields = {
        "BASE_MAINNET_OPEN_COMPETITION_V1_GAS_SPONSORSHIP_AVAILABLE": (
            "gas_sponsorship_available"
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_RELAY_SUPPORT_AVAILABLE": (
            "relay_support_available"
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_R4_EVIDENCE_COMPLETE": (
            "r4_release_evidence_complete"
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_CREATION_ENABLED": (
            "public_creation_enabled"
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_COMMITMENTS_ENABLED": (
            "public_commitments_enabled"
        ),
    }
    if activation.get("public_inventory_eligible") is not False:
        raise RecoveryError("Open Competition public inventory must remain disabled")
    if activation.get("monitoring_gate_configured") is not True:
        raise RecoveryError("Open Competition monitoring gate must be configured")
    if activation.get("monitoring_active") is not False:
        raise RecoveryError("Open Competition runtime monitoring cannot be pre-attested")
    for field in activation_fields.values():
        if activation.get(field) is not False:
            raise RecoveryError(f"Open Competition hosted activation requires {field}=false")

    values = {
        "BASE_MAINNET_RPC_URL": HOSTED_BASE_MAINNET_RPC_URL,
        "BASE_MAINNET_OPEN_COMPETITION_V1_RELEASE_MANIFEST_JSON": json.dumps(
            release, ensure_ascii=False, separators=(",", ":")
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_VERIFIER_CATALOG_JSON": json.dumps(
            catalog, ensure_ascii=False, separators=(",", ":")
        ),
    }
    values.update({key: "false" for key in activation_fields})
    values["BASE_MAINNET_OPEN_COMPETITION_V1_MONITORING_ACTIVE"] = "true"
    values.update(
        open_competition_entrant_environment(
            entrant_release_audit_path,
            canary_enabled=entrant_relay_canary_enabled,
        )
    )
    return values


def open_competition_entrant_environment(
    audit_path: Path,
    *,
    canary_enabled: bool,
) -> dict[str, str]:
    values = {
        "BASE_MAINNET_OPEN_COMPETITION_V1_ENTRANT_RELAY_CANARY_ENABLED": (
            "true" if canary_enabled else "false"
        ),
        "BASE_MAINNET_OPEN_COMPETITION_V1_ENTRANT_RECOVERY_RELAY_ENABLED": "false",
    }
    if not audit_path.exists():
        if canary_enabled:
            raise RecoveryError(
                "Open Competition entrant relay canary requires a deployment audit"
            )
        return values
    try:
        audit = json.loads(audit_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise RecoveryError(
            f"Open Competition entrant deployment audit is unavailable: {redact(str(error))}"
        ) from None
    if not isinstance(audit, dict):
        raise RecoveryError("Open Competition entrant deployment audit must be an object")
    if (
        audit.get("schema_version")
        != "agent-bounties/open-competition-entrant-wallet-mainnet-deployment-audit-v1"
        or audit.get("network") != "base-mainnet"
        or audit.get("chain_id") != 8453
        or audit.get("passed") is not True
    ):
        raise RecoveryError("Open Competition entrant deployment audit identity is invalid")
    assertions = audit.get("assertions")
    if (
        not isinstance(assertions, dict)
        or not assertions
        or not all(value is True for value in assertions.values())
        or assertions.get("public_activation_remains_disabled") is not True
    ):
        raise RecoveryError("Open Competition entrant deployment assertions are incomplete")
    release = audit.get("release_manifest")
    if not isinstance(release, dict):
        raise RecoveryError("Open Competition entrant release manifest is missing")
    expected_identity = {
        "schema_version": "agent-bounties/open-competition-entrant-wallet-release-v1",
        "protocol_version": "agent-bounties/open-competition-entrant-wallet-v1",
        "network": "base-mainnet",
        "chain_id": 8453,
        "deployment_state": "mainnet_canary_not_ready_to_earn",
        "factory_contract": OPEN_COMPETITION_ENTRANT_FACTORY,
        "implementation_contract": OPEN_COMPETITION_ENTRANT_IMPLEMENTATION,
        "competition_factory": OPEN_COMPETITION_FACTORY,
        "settlement_token": BASE_MAINNET_USDC,
        **OPEN_COMPETITION_ENTRANT_RUNTIME_HASHES,
    }
    if any(release.get(key) != value for key, value in expected_identity.items()):
        raise RecoveryError("Open Competition entrant release identity is invalid")
    if not isinstance(release.get("deployment_block"), int) or release["deployment_block"] <= 0:
        raise RecoveryError("Open Competition entrant deployment block is invalid")
    values[
        "BASE_MAINNET_OPEN_COMPETITION_V1_ENTRANT_WALLET_RELEASE_MANIFEST_JSON"
    ] = json.dumps(release, ensure_ascii=False, separators=(",", ":"))
    return values


def normalize_evm_address(name: str, value: str) -> str:
    normalized = value.strip().lower()
    if not re.fullmatch(r"0x[0-9a-f]{40}", normalized):
        raise RecoveryError(f"{name} must be an exact EVM address")
    return normalized


def leaderboard_environment_values(
    mainnet_contract: str | None,
    sepolia_contract: str | None,
) -> dict[str, str]:
    values: dict[str, str] = {}
    for key, value in (
        ("BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT", mainnet_contract),
        ("BASE_SEPOLIA_LEADERBOARD_REWARD_CONTRACT", sepolia_contract),
    ):
        if value is not None:
            values[key] = normalize_evm_address(key, value)
    return values


def deploy_commit(deploy: dict[str, Any]) -> str | None:
    commit = deploy.get("commit")
    if not isinstance(commit, dict):
        return None
    commit_id = commit.get("id")
    return commit_id.lower() if isinstance(commit_id, str) else None


def auto_deploy_disabled(value: object) -> bool:
    return value is False or value == "no"


def existing_deploy(payload: object, revision: str) -> dict[str, Any] | None:
    if not isinstance(payload, list):
        raise RecoveryError("Render deploy-list response must be an array")
    deploys: list[dict[str, Any]] = []
    for entry in payload:
        try:
            deploy = unwrap_deploy(entry)
        except RecoveryError:
            continue
        deploys.append(deploy)
        if deploy_commit(deploy) == revision and deploy.get("status") in ACTIVE_STATUSES:
            return deploy
    if deploys and deploy_commit(deploys[0]) == revision and deploys[0].get("status") == "live":
        return deploys[0]
    return None


def new_active_deploy(
    payload: object,
    baseline_ids: set[str],
) -> dict[str, Any] | None:
    if not isinstance(payload, list):
        raise RecoveryError("Render deploy-list response must be an array")
    for entry in payload:
        try:
            deploy = unwrap_deploy(entry)
        except RecoveryError:
            continue
        deploy_id = deploy.get("id")
        if (
            isinstance(deploy_id, str)
            and deploy_id not in baseline_ids
            and deploy.get("status") in ACTIVE_STATUSES | {"live"}
        ):
            return deploy
    return None


def current_live_deploy_record(payload: object) -> dict[str, Any]:
    if not isinstance(payload, list):
        raise RecoveryError("Render deploy-list response must be an array")
    for entry in payload:
        try:
            deploy = unwrap_deploy(entry)
        except RecoveryError:
            continue
        if deploy.get("status") != "live":
            continue
        return deploy
    raise RecoveryError("deploy_only requires a current live Render artifact")


def current_live_deploy(payload: object, revision: str) -> dict[str, Any]:
    deploy = current_live_deploy_record(payload)
    if deploy_commit(deploy) != revision:
        raise RecoveryError(
            "deploy_only requires the current live artifact to match the requested revision"
        )
    return deploy


class RenderClient:
    def __init__(
        self,
        token: str,
        *,
        api_base: str = "https://api.render.com/v1",
        timeout_seconds: float = 20,
        sleeper: Callable[[float], None] = time.sleep,
    ) -> None:
        if not token.strip():
            raise RecoveryError("RENDER_API_KEY is required")
        self._token = token.strip()
        self._api_base = api_base.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._sleep = sleeper

    def _request_json(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None = None,
    ) -> Any:
        body = None if payload is None else json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            f"{self._api_base}{path}",
            data=body,
            method=method,
            headers={
                "Accept": "application/json",
                "Authorization": f"Bearer {self._token}",
                "Content-Type": "application/json",
                "User-Agent": "agent-bounties-render-recovery/1",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=self._timeout_seconds) as response:
                response_body = response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            error_body = error.read().decode("utf-8", errors="replace")
            raise RenderHttpError(error.code, error_body) from None
        except (urllib.error.URLError, TimeoutError) as error:
            raise RenderTransportError(
                f"Render API transport failed: {redact(str(error))}"
            ) from None
        try:
            return json.loads(response_body) if response_body else {}
        except json.JSONDecodeError as error:
            raise RecoveryError(f"Render API returned invalid JSON: {error}") from None

    def _read_with_retry(self, path: str, attempts: int = 3) -> Any:
        for attempt in range(1, attempts + 1):
            try:
                return self._request_json("GET", path)
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == attempts:
                    raise
            except RenderTransportError:
                if attempt == attempts:
                    raise
            self._sleep(float(attempt * 2))
        raise AssertionError("unreachable")

    def _write_with_retry(
        self,
        method: str,
        path: str,
        payload: dict[str, Any] | None,
        attempts: int = 3,
    ) -> Any:
        for attempt in range(1, attempts + 1):
            try:
                return self._request_json(method, path, payload)
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == attempts:
                    raise
            except RenderTransportError:
                if attempt == attempts:
                    raise
            self._sleep(float(attempt * 2))
        raise AssertionError("unreachable")

    def resolve_service(self, spec: ServiceSpec) -> dict[str, Any]:
        query = urllib.parse.urlencode(
            {"name": spec.name, "includePreviews": "false", "limit": "20"}
        )
        return select_service(spec, self._read_with_retry(f"/services?{query}"))

    def resolve_env_group(self, owner_id: str) -> dict[str, Any]:
        owner_id = validate_owner_id(owner_id)
        query = urllib.parse.urlencode(
            {
                "name": OPEN_COMPETITION_ENV_GROUP_NAME,
                "ownerId": owner_id,
                "limit": "20",
            }
        )
        return select_env_group(
            self._read_with_retry(f"/env-groups?{query}"), owner_id
        )

    def get_env_group(self, group_id: str) -> dict[str, Any]:
        if not re.fullmatch(r"evg-[0-9a-z]+", group_id):
            raise RecoveryError("Render environment group has an invalid id")
        return unwrap_env_group(self._read_with_retry(f"/env-groups/{group_id}"))

    def get_env_var(self, service: dict[str, Any], key: str) -> dict[str, str]:
        service_id = service.get("id")
        if not isinstance(service_id, str) or not service_id.startswith("srv-"):
            raise RecoveryError("Render service is missing its id")
        encoded_key = urllib.parse.quote(key, safe="")
        return unwrap_env_var(
            self._read_with_retry(f"/services/{service_id}/env-vars/{encoded_key}")
        )

    def ensure_env_group_env_var(
        self,
        group: dict[str, Any],
        key: str,
        value: str,
    ) -> dict[str, Any]:
        group_id = group.get("id")
        if not isinstance(group_id, str) or not re.fullmatch(
            r"evg-[0-9a-z]+", group_id
        ):
            raise RecoveryError("Render environment group has an invalid id")
        encoded_key = urllib.parse.quote(key, safe="")
        path = f"/env-groups/{group_id}/env-vars/{encoded_key}"
        changed = True
        try:
            current = unwrap_env_var(self._read_with_retry(path))
            if current == {"key": key, "value": value}:
                changed = False
        except RenderHttpError as error:
            if error.status != 404:
                raise
        if changed:
            updated = env_group_env_var(
                self._write_with_retry("PUT", path, {"value": value}), key
            )
            if updated != {"key": key, "value": value}:
                raise RecoveryError(
                    f"Render did not update shared Open Competition variable {key}"
                )
        verified = unwrap_env_var(self._read_with_retry(path))
        if verified != {"key": key, "value": value}:
            raise RecoveryError(
                f"Render did not retain shared Open Competition variable {key}"
            )
        return {"key": key, "value": value, "changed": changed}

    def provision_open_competition_service(
        self,
        spec: ServiceSpec,
        reference_service: dict[str, Any],
    ) -> dict[str, Any]:
        if spec.name != OPEN_COMPETITION_WORKER_NAME:
            raise RecoveryError(f"direct Render provisioning is not authorized for {spec.name}")
        if (
            reference_service.get("name") != OPEN_COMPETITION_REFERENCE_SERVICE_NAME
            or reference_service.get("type") != "background_worker"
            or reference_service.get("branch") != "main"
            or normalize_repo(reference_service.get("repo")) != REPOSITORY
        ):
            raise RecoveryError("direct Render provisioning reference service is invalid")

        owner_id = validate_owner_id(reference_service.get("ownerId"))
        environment_id = reference_service.get("environmentId")
        if environment_id is not None:
            environment_id = validate_environment_id(environment_id)
        env_group = self.resolve_env_group(owner_id)
        group_environment_id = env_group.get("environmentId")
        if group_environment_id is not None:
            group_environment_id = validate_environment_id(group_environment_id)
        if (
            environment_id is not None
            and group_environment_id is not None
            and group_environment_id != environment_id
        ):
            raise RecoveryError(
                "reference Render worker and environment group are in different projects"
            )
        if environment_id is None:
            environment_id = group_environment_id
        database_url = validate_database_url(
            self.get_env_var(reference_service, "DATABASE_URL").get("value")
        )

        env_vars = [
            {"key": key, "value": value}
            for key, value in OPEN_COMPETITION_WORKER_ENVIRONMENT.items()
        ]
        env_vars.append({"key": "DATABASE_URL", "value": database_url})
        payload: dict[str, Any] = {
            "type": "background_worker",
            "name": OPEN_COMPETITION_WORKER_NAME,
            "ownerId": owner_id,
            "repo": "https://github.com/NSPG13/agent-bounties",
            "branch": "main",
            "autoDeploy": "no",
            "envVars": env_vars,
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
            created = select_service(
                spec,
                [self._write_with_retry("POST", "/services", payload)],
            )
        except RenderHttpError as error:
            if error.status != 409:
                raise
            # A concurrent Blueprint sync is acceptable only if it created the
            # exact service identity that this controller was about to create.
            created = self.resolve_service(spec)

        if created.get("ownerId") != owner_id:
            raise RecoveryError("new Render worker is in an unexpected workspace")
        if environment_id is not None and created.get("environmentId") != environment_id:
            raise RecoveryError("new Render worker is in an unexpected project environment")

        group_id = env_group["id"]
        linked_group = self.get_env_group(group_id)
        if not env_group_has_service(linked_group, spec, created["id"]):
            try:
                self._write_with_retry(
                    "POST",
                    f"/env-groups/{group_id}/services/{created['id']}",
                    None,
                )
            except RenderHttpError as error:
                if error.status != 409:
                    raise
            linked_group = self.get_env_group(group_id)
        if not env_group_has_service(linked_group, spec, created["id"]):
            raise RecoveryError("Render did not attach the required environment group")

        verified = self.resolve_service(spec)
        if verified.get("ownerId") != owner_id:
            raise RecoveryError("provisioned Render worker changed workspaces")
        if environment_id is not None and verified.get("environmentId") != environment_id:
            raise RecoveryError("provisioned Render worker changed project environments")
        for key, expected in OPEN_COMPETITION_WORKER_ENVIRONMENT.items():
            actual = self.get_env_var(verified, key)
            if actual != {"key": key, "value": expected}:
                raise RecoveryError(
                    f"provisioned Render worker did not retain required {key}"
                )
        retained_database_url = validate_database_url(
            self.get_env_var(verified, "DATABASE_URL").get("value")
        )
        if retained_database_url != database_url:
            raise RecoveryError("provisioned Render worker changed its DATABASE_URL")
        return verified

    def ensure_blueprint_service(
        self,
        spec: ServiceSpec,
        *,
        reference_service: dict[str, Any] | None = None,
        attempts: int = 12,
        poll_seconds: float = 5,
    ) -> dict[str, Any]:
        if spec.name not in BLUEPRINT_RECOVERABLE_SERVICE_NAMES:
            raise RecoveryError(f"Blueprint recovery is not authorized for {spec.name}")
        if attempts < 1 or poll_seconds < 0:
            raise RecoveryError("Blueprint recovery polling configuration is invalid")

        blueprint = select_blueprint(self._read_with_retry("/blueprints?limit=100"))
        blueprint_id = blueprint["id"]
        if not blueprint_has_service(blueprint, spec):
            disabled_for_recovery = False
            try:
                if blueprint["autoSync"]:
                    disabled = validate_blueprint(
                        self._write_with_retry(
                            "PATCH",
                            f"/blueprints/{blueprint_id}",
                            {"autoSync": False},
                        )
                    )
                    if disabled["autoSync"]:
                        raise RecoveryError("Render did not disable Blueprint Auto Sync")
                    disabled_for_recovery = True
                enabled = validate_blueprint(
                    self._write_with_retry(
                        "PATCH",
                        f"/blueprints/{blueprint_id}",
                        {"autoSync": True, "path": BLUEPRINT_PATH},
                    )
                )
                if not enabled["autoSync"]:
                    raise RecoveryError("Render did not enable Blueprint Auto Sync")
            except RecoveryError as error:
                if disabled_for_recovery:
                    try:
                        restored = validate_blueprint(
                            self._write_with_retry(
                                "PATCH",
                                f"/blueprints/{blueprint_id}",
                                {"autoSync": True, "path": BLUEPRINT_PATH},
                            )
                        )
                    except RecoveryError as restore_error:
                        raise RecoveryError(
                            "Blueprint recovery failed and Render Auto Sync could not be "
                            "restored"
                        ) from restore_error
                    if not restored["autoSync"]:
                        raise RecoveryError(
                            "Blueprint recovery failed and Render Auto Sync remains disabled"
                        ) from error
                raise

        last_status = blueprint.get("status")
        for attempt in range(1, attempts + 1):
            current = validate_blueprint(
                self._read_with_retry(f"/blueprints/{blueprint_id}")
            )
            last_status = current["status"]
            if blueprint_has_service(current, spec):
                try:
                    return self.resolve_service(spec)
                except RenderServiceMissing:
                    pass
            if current["status"] == "error":
                raise RecoveryError(
                    f"Render Blueprint entered error state before creating {spec.name}"
                )
            if attempt < attempts:
                self._sleep(poll_seconds)
        if reference_service is not None:
            return self.provision_open_competition_service(spec, reference_service)
        raise RecoveryError(
            f"Render Blueprint did not create {spec.name}; last status {last_status}"
        )

    def disable_native_auto_deploy(self, service: dict[str, Any]) -> None:
        if auto_deploy_disabled(service.get("autoDeploy")):
            return
        service_id = service["id"]
        updated = None
        for attempt in range(1, 4):
            try:
                updated = self._request_json(
                    "PATCH", f"/services/{service_id}", {"autoDeploy": "no"}
                )
                break
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == 3:
                    raise
            except RenderTransportError:
                if attempt == 3:
                    raise
            self._sleep(float(attempt * 2))
        updated_service = updated.get("service", updated) if isinstance(updated, dict) else None
        if not isinstance(updated_service, dict) or not auto_deploy_disabled(
            updated_service.get("autoDeploy")
        ):
            raise RecoveryError(f"Render did not disable native auto-deploy for {service['name']}")

    def list_deploys(self, service_id: str) -> Any:
        return self._read_with_retry(f"/services/{service_id}/deploys?limit=20")

    def ensure_custom_domain(self, service: dict[str, Any], domain: str) -> dict[str, Any]:
        service_id = service["id"]
        existing = [
            item
            for item in unwrap_custom_domains(
                self._read_with_retry(f"/services/{service_id}/custom-domains?limit=100")
            )
            if str(item.get("name", "")).lower() == domain.lower()
        ]
        if len(existing) > 1:
            raise RecoveryError(f"{service['name']} has duplicate Render custom domains for {domain}")
        if existing:
            return existing[0]
        self._request_json(
            "POST", f"/services/{service_id}/custom-domains", {"name": domain}
        )
        for attempt in range(1, 6):
            attached = [
                item
                for item in unwrap_custom_domains(
                    self._read_with_retry(
                        f"/services/{service_id}/custom-domains?limit=100"
                    )
                )
                if str(item.get("name", "")).lower() == domain.lower()
            ]
            if len(attached) > 1:
                raise RecoveryError(
                    f"{service['name']} has duplicate Render custom domains for {domain}"
                )
            if attached:
                return attached[0]
            if attempt < 5:
                self._sleep(float(attempt * 2))
        raise RecoveryError(f"Render did not attach {domain} to {service['name']}")

    def reconcile_custom_domains(
        self, service: dict[str, Any], domains: tuple[str, ...]
    ) -> list[dict[str, Any]]:
        service_id = service["id"]
        desired = {domain.lower() for domain in domains}
        existing = unwrap_custom_domains(
            self._read_with_retry(f"/services/{service_id}/custom-domains?limit=100")
        )
        names = [str(item.get("name", "")) for item in existing]
        if len({name.lower() for name in names}) != len(names):
            raise RecoveryError(f"{service['name']} has duplicate Render custom domains")

        # Delete obsolete aliases first so a plan-level domain cap cannot block
        # attaching the canonical replacement. The onrender.com origin remains
        # available throughout this control-plane migration.
        for name in names:
            if name.lower() not in desired:
                encoded_name = urllib.parse.quote(name, safe="")
                self._request_json(
                    "DELETE",
                    f"/services/{service_id}/custom-domains/{encoded_name}",
                )

        return [self.ensure_custom_domain(service, domain) for domain in domains]

    def ensure_env_var(
        self,
        service: dict[str, Any],
        key: str,
        value: str,
    ) -> dict[str, Any]:
        service_id = service["id"]
        encoded_key = urllib.parse.quote(key, safe="")
        path = f"/services/{service_id}/env-vars/{encoded_key}"
        changed = True
        try:
            current = unwrap_env_var(self._read_with_retry(path))
            if current == {"key": key, "value": value}:
                changed = False
        except RenderHttpError as error:
            if error.status != 404:
                raise

        if changed:
            updated = unwrap_env_var(
                self._request_json("PUT", path, {"value": value})
            )
            if updated != {"key": key, "value": value}:
                raise RecoveryError(
                    f"Render did not update {key} for {service['name']}"
                )

        verified = unwrap_env_var(self._read_with_retry(path))
        if verified != {"key": key, "value": value}:
            raise RecoveryError(
                f"Render did not retain {key} for {service['name']}"
            )
        return {"key": key, "value": value, "changed": changed}

    def get_deploy(self, service_id: str, deploy_id: str) -> dict[str, Any]:
        return unwrap_deploy(
            self._read_with_retry(f"/services/{service_id}/deploys/{deploy_id}")
        )

    def get_build_log_summary(
        self,
        service: dict[str, Any],
        deploy: dict[str, Any],
    ) -> dict[str, Any]:
        service_id = service.get("id")
        owner_id = service.get("ownerId")
        if not isinstance(service_id, str) or not service_id.startswith("srv-"):
            raise RecoveryError("Render service is missing its id")
        if not isinstance(owner_id, str) or not re.fullmatch(r"[a-z]+-[0-9a-z]+", owner_id):
            raise RecoveryError("Render service is missing its workspace id")
        parameters: dict[str, object] = {
            "ownerId": owner_id,
            "resource": [service_id],
            "type": ["build"],
            "direction": "backward",
            "limit": "100",
        }
        created_at = deploy.get("createdAt")
        if isinstance(created_at, str) and created_at:
            parameters["startTime"] = created_at
        query = urllib.parse.urlencode(parameters, doseq=True)
        path = f"/logs?{query}"
        summary = summarize_build_logs(self._read_with_retry(path))
        if summary["log_count"] < 5:
            self._sleep(2)
            later = summarize_build_logs(self._read_with_retry(path))
            if later["log_count"] >= summary["log_count"]:
                summary = later
        return summary

    def ensure_deploy(
        self,
        service: dict[str, Any],
        revision: str,
        *,
        force: bool = False,
        deploy_mode: str = "build_and_deploy",
    ) -> dict[str, Any]:
        deploy_mode = validate_deploy_mode(deploy_mode)
        service_id = service["id"]
        listed_deploys = self.list_deploys(service_id)
        matched = existing_deploy(listed_deploys, revision)
        baseline_ids = {
            deploy["id"]
            for entry in listed_deploys
            if isinstance(entry, dict)
            for deploy in [entry.get("deploy", entry)]
            if isinstance(deploy, dict) and isinstance(deploy.get("id"), str)
        }
        if matched is not None and not (force and matched.get("status") == "live"):
            return matched
        replaced_live_id = (
            matched.get("id")
            if force and matched is not None and matched.get("status") == "live"
            else None
        )

        payload = (
            {"deployMode": "deploy_only"}
            if deploy_mode == "deploy_only"
            else {"clearCache": "do_not_clear", "commitId": revision}
        )

        for attempt in range(1, 4):
            try:
                return unwrap_deploy(
                    self._request_json(
                        "POST",
                        f"/services/{service_id}/deploys",
                        payload,
                    )
                )
            except RenderHttpError as error:
                if error.status not in TRANSIENT_HTTP_STATUSES or attempt == 3:
                    raise
            except RenderTransportError:
                if attempt == 3:
                    raise
            self._sleep(float(attempt * 2))
            latest_deploys = self.list_deploys(service_id)
            matched = (
                new_active_deploy(latest_deploys, baseline_ids)
                if deploy_mode == "deploy_only"
                else existing_deploy(latest_deploys, revision)
            )
            if matched is not None and matched.get("id") != replaced_live_id:
                return matched
        raise AssertionError("unreachable")


def validate_deploy(
    deploy: dict[str, Any],
    revision: str,
    service_name: str,
    *,
    require_revision: bool = True,
) -> tuple[str, str]:
    deploy_id = deploy.get("id")
    status = deploy.get("status")
    if not isinstance(deploy_id, str) or not deploy_id.startswith("dep-"):
        raise RecoveryError(f"{service_name} deploy is missing an id")
    if require_revision and deploy_commit(deploy) != revision:
        raise RecoveryError(f"{service_name} deploy does not attest the requested revision")
    if not isinstance(status, str):
        raise RecoveryError(f"{service_name} deploy is missing status")
    return deploy_id, status


def deploy_failure(
    client: RenderClient,
    service_name: str,
    service: dict[str, Any],
    deploy: dict[str, Any],
) -> RenderDeployFailure:
    deploy_id = str(deploy.get("id", "unknown"))
    status = str(deploy.get("status", "unknown"))
    evidence: dict[str, Any] = {
        "service": service_name,
        "service_id": service.get("id"),
        "deploy_id": deploy_id,
        "status": status,
        "build_logs": {"available": False},
    }
    if status == "build_failed":
        try:
            evidence["build_logs"] = client.get_build_log_summary(service, deploy)
        except RecoveryError as error:
            evidence["build_logs"] = {
                "available": False,
                "error": redact(str(error)),
            }
    return RenderDeployFailure(
        f"{service_name} deploy {deploy_id} ended with status {status}",
        evidence,
    )


def poll_deploys(
    client: RenderClient,
    pending: dict[str, tuple[dict[str, Any], str]],
    revision: str,
    *,
    timeout_seconds: float,
    poll_seconds: float,
    metadata_revision_exempt: frozenset[str] = frozenset(),
    clock: Callable[[], float] = time.monotonic,
    sleeper: Callable[[float], None] = time.sleep,
) -> dict[str, dict[str, Any]]:
    deadline = clock() + timeout_seconds
    completed: dict[str, dict[str, Any]] = {}
    while pending:
        for service_name, (service, deploy_id) in list(pending.items()):
            service_id = service["id"]
            deploy = client.get_deploy(service_id, deploy_id)
            _, status = validate_deploy(
                deploy,
                revision,
                service_name,
                require_revision=service_name not in metadata_revision_exempt,
            )
            if status == "live":
                completed[service_name] = deploy
                pending.pop(service_name)
            elif status in FAILED_STATUSES:
                raise deploy_failure(client, service_name, service, deploy)
            elif status not in ACTIVE_STATUSES:
                raise RecoveryError(f"{service_name} deploy has unknown status {status}")
        if not pending:
            break
        if clock() >= deadline:
            names = ", ".join(sorted(pending))
            raise RecoveryError(f"timed out waiting for Render deploys: {names}")
        sleeper(poll_seconds)
    return completed


def fetch_health(url: str, timeout_seconds: float) -> tuple[int, str, dict[str, str]]:
    separator = "&" if "?" in url else "?"
    probe_url = f"{url}{separator}_agent_bounties_probe={time.time_ns()}"
    request = urllib.request.Request(
        probe_url,
        method="GET",
        headers={
            "Accept": "text/plain",
            "Cache-Control": "no-cache, no-store",
            "Connection": "close",
            "Pragma": "no-cache",
            "User-Agent": "agent-bounties-render-recovery/1",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            return (
                response.status,
                response.read().decode("utf-8", errors="replace"),
                {key.lower(): value for key, value in response.headers.items()},
            )
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode("utf-8", errors="replace"), {}
    except (urllib.error.URLError, TimeoutError) as error:
        raise RecoveryError(f"health probe transport failed: {redact(str(error))}") from None


def fetch_json(url: str, timeout_seconds: float) -> dict[str, Any]:
    separator = "&" if "?" in url else "?"
    request = urllib.request.Request(
        f"{url}{separator}_agent_bounties_probe={time.time_ns()}",
        method="GET",
        headers={
            "Accept": "application/json",
            "Cache-Control": "no-cache, no-store",
            "Connection": "close",
            "Pragma": "no-cache",
            "User-Agent": "agent-bounties-render-recovery/1",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status = response.status
            body = response.read().decode("utf-8", errors="replace")
    except urllib.error.HTTPError as error:
        raise RecoveryError(
            f"JSON readiness probe returned HTTP {error.code}"
        ) from None
    except (urllib.error.URLError, TimeoutError) as error:
        raise RecoveryError(
            f"JSON readiness probe transport failed: {redact(str(error))}"
        ) from None
    if status != 200:
        raise RecoveryError(f"JSON readiness probe returned HTTP {status}")
    try:
        payload = json.loads(body)
    except json.JSONDecodeError as error:
        raise RecoveryError(f"JSON readiness probe returned invalid JSON: {error}") from None
    if not isinstance(payload, dict):
        raise RecoveryError("JSON readiness probe must return an object")
    return payload


def validate_cloud_agent_readiness(
    payload: dict[str, Any],
    *,
    credential_supplied: bool,
) -> dict[str, Any]:
    if payload.get("schema_version") != "agent-bounties/cloud-agent-readiness-v1":
        raise RecoveryError("cloud-agent readiness schema is invalid")
    if payload.get("execution") != "hosted_cloud_api":
        raise RecoveryError("cloud-agent readiness does not attest hosted execution")
    if payload.get("local_fallback") is not False:
        raise RecoveryError("cloud-agent readiness must prohibit a local fallback")
    if payload.get("authority") != "advisory_only":
        raise RecoveryError("cloud-agent readiness exceeds advisory-only authority")
    capabilities = payload.get("capabilities")
    if not isinstance(capabilities, list) or not {
        "bounty_drafting",
        "published_terms_analysis",
    }.issubset(capabilities):
        raise RecoveryError("cloud-agent readiness capabilities are incomplete")
    available = payload.get("available")
    missing = payload.get("missing_configuration")
    if not isinstance(available, bool) or not isinstance(missing, list):
        raise RecoveryError("cloud-agent readiness is incomplete")
    if credential_supplied and (not available or missing):
        raise RecoveryError("supplied cloud-agent credential did not become ready")
    return {
        "available": available,
        "credential_supplied": credential_supplied,
        "provider": payload.get("provider"),
        "model": payload.get("model"),
        "public_drafts": payload.get("public_drafts"),
        "local_fallback": False,
        "authority": "advisory_only",
        "capabilities": capabilities,
        "missing_configuration": missing,
    }


def validate_leaderboard_readiness(
    payload: dict[str, Any],
    *,
    network: str,
    expected_contract: str,
) -> dict[str, Any]:
    if payload.get("schema_version") != "agent-bounties/solver-leaderboard-v1":
        raise RecoveryError("leaderboard readiness schema is invalid")
    if payload.get("network") != network:
        raise RecoveryError("leaderboard readiness reports a different network")
    reward_pool = payload.get("reward_pool")
    if not isinstance(reward_pool, dict):
        raise RecoveryError("leaderboard readiness is missing its reward pool")
    observed_contract = reward_pool.get("contract")
    if (
        not isinstance(observed_contract, str)
        or observed_contract.lower() != expected_contract
    ):
        raise RecoveryError("leaderboard readiness reports a different reward contract")
    return {
        "network": network,
        "contract": observed_contract.lower(),
        "funding_status": reward_pool.get("funding_status"),
        "balance_usdc": reward_pool.get("balance_usdc"),
        "observed_safe_block": reward_pool.get("observed_safe_block"),
    }


def validate_health(
    service_name: str,
    revision: str,
    response: tuple[int, str, dict[str, str]],
) -> dict[str, Any]:
    status, body, headers = response
    observed_revision = headers.get("x-agent-bounties-revision")
    observed_protocol = headers.get("x-agent-bounties-protocol")
    if status != 200 or body.strip() != "ok":
        raise RecoveryError(f"{service_name} health contract is not ready")
    if observed_revision != revision:
        raise RecoveryError(f"{service_name} health reports a different revision")
    if observed_protocol != PROTOCOL:
        raise RecoveryError(f"{service_name} health reports a different protocol")
    return {
        "service": service_name,
        "status": status,
        "body": "ok",
        "revision": observed_revision,
        "protocol": observed_protocol,
    }


def wait_for_health(
    spec: ServiceSpec,
    revision: str,
    *,
    timeout_seconds: float,
    poll_seconds: float,
    probe: Callable[[str, float], tuple[int, str, dict[str, str]]] = fetch_health,
    clock: Callable[[], float] = time.monotonic,
    sleeper: Callable[[float], None] = time.sleep,
    required_consecutive: int = HEALTH_STABILITY_PROBES,
) -> dict[str, Any]:
    if spec.health_url is None:
        raise RecoveryError(f"{spec.name} has no public health contract")
    if required_consecutive < 1:
        raise RecoveryError("health stability probe count must be positive")
    deadline = clock() + timeout_seconds
    last_error = "health did not become ready"
    consecutive = 0
    while True:
        try:
            evidence = validate_health(spec.name, revision, probe(spec.health_url, 10))
            consecutive += 1
            if consecutive >= required_consecutive:
                evidence["consecutive_exact_probes"] = consecutive
                evidence["stability_window_seconds"] = (consecutive - 1) * poll_seconds
                return evidence
        except RecoveryError as error:
            last_error = str(error)
            consecutive = 0
        if clock() >= deadline:
            raise RecoveryError(f"{spec.name} health verification timed out: {last_error}")
        sleeper(poll_seconds)


def write_evidence(path: Path, evidence: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def reconcile_cloud_agent_environment(
    client: RenderClient,
    service: dict[str, Any],
    api_key: str | None,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], bool]:
    runtime_environment = []
    changed = False
    for key, value in CLOUD_AGENT_RUNTIME_ENVIRONMENT.items():
        record = client.ensure_env_var(service, key, value)
        changed |= record["changed"]
        runtime_environment.append(
            {
                "service": service["name"],
                "key": key,
                "value": value,
                "changed": record["changed"],
            }
        )

    secret_environment = []
    normalized_key = api_key.strip() if isinstance(api_key, str) else ""
    if normalized_key:
        record = client.ensure_env_var(service, "CLOUD_AGENT_API_KEY", normalized_key)
        changed |= record["changed"]
        secret_environment.append(
            {
                "service": service["name"],
                "key": "CLOUD_AGENT_API_KEY",
                "configured": True,
                "changed": record["changed"],
            }
        )
    return runtime_environment, secret_environment, changed


def reconcile_neynar_social_environment(
    client: RenderClient,
    service: dict[str, Any],
    *,
    api_key: str | None,
    webhook_secret: str | None,
    signer_uuid: str | None,
    bot_fid: str | None,
    bot_username: str | None,
) -> tuple[list[dict[str, Any]], bool]:
    supplied = {
        "NEYNAR_API_KEY": api_key.strip() if isinstance(api_key, str) else "",
        "NEYNAR_WEBHOOK_SECRET": (
            webhook_secret.strip() if isinstance(webhook_secret, str) else ""
        ),
        "NEYNAR_SIGNER_UUID": (
            signer_uuid.strip() if isinstance(signer_uuid, str) else ""
        ),
        "NEYNAR_BOT_FID": bot_fid.strip() if isinstance(bot_fid, str) else "",
        "NEYNAR_BOT_USERNAME": (
            bot_username.strip().lstrip("@").lower()
            if isinstance(bot_username, str)
            else ""
        ),
    }
    configured = [key for key, value in supplied.items() if value]
    if not configured:
        return [], False
    if len(configured) != len(supplied):
        missing = sorted(key for key, value in supplied.items() if not value)
        raise RecoveryError(
            "Neynar social ingestion requires all provider values together; missing "
            + ", ".join(missing)
        )
    if not re.fullmatch(r"[1-9][0-9]*", supplied["NEYNAR_BOT_FID"]):
        raise RecoveryError("NEYNAR_BOT_FID must be a positive integer")
    if not re.fullmatch(r"[a-z0-9_-]{1,64}", supplied["NEYNAR_BOT_USERNAME"]):
        raise RecoveryError("NEYNAR_BOT_USERNAME is invalid")
    if not re.fullmatch(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}",
        supplied["NEYNAR_SIGNER_UUID"],
    ):
        raise RecoveryError("NEYNAR_SIGNER_UUID must be a UUID")

    evidence = []
    changed = False
    for key, value in supplied.items():
        record = client.ensure_env_var(service, key, value)
        changed |= record["changed"]
        evidence.append(
            {
                "service": service["name"],
                "key": key,
                "configured": True,
                "changed": record["changed"],
            }
        )
    return evidence, changed


def normalize_neynar_social_inputs(
    *,
    api_key: str | None,
    signer_uuid: str | None,
    bot_fid: str | None,
    bot_username: str | None,
) -> dict[str, str]:
    supplied = {
        "NEYNAR_API_KEY": api_key.strip() if isinstance(api_key, str) else "",
        "NEYNAR_SIGNER_UUID": (
            signer_uuid.strip() if isinstance(signer_uuid, str) else ""
        ),
        "NEYNAR_BOT_FID": bot_fid.strip() if isinstance(bot_fid, str) else "",
        "NEYNAR_BOT_USERNAME": (
            bot_username.strip().lstrip("@").lower()
            if isinstance(bot_username, str)
            else ""
        ),
    }
    configured = [key for key, value in supplied.items() if value]
    if not configured:
        return {}
    if len(configured) != len(supplied):
        missing = sorted(key for key, value in supplied.items() if not value)
        raise RecoveryError(
            "Neynar social ingestion requires all account values together; missing "
            + ", ".join(missing)
        )
    if not re.fullmatch(r"[1-9][0-9]*", supplied["NEYNAR_BOT_FID"]):
        raise RecoveryError("NEYNAR_BOT_FID must be a positive integer")
    if not re.fullmatch(r"[a-z0-9_-]{1,64}", supplied["NEYNAR_BOT_USERNAME"]):
        raise RecoveryError("NEYNAR_BOT_USERNAME is invalid")
    if not re.fullmatch(
        r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}",
        supplied["NEYNAR_SIGNER_UUID"],
    ):
        raise RecoveryError("NEYNAR_SIGNER_UUID must be a UUID")
    return supplied


def _neynar_webhook_from_response(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise RecoveryError("Neynar webhook response is invalid")
    webhook = payload.get("webhook", payload)
    if not isinstance(webhook, dict):
        raise RecoveryError("Neynar webhook response is missing webhook data")
    return webhook


def _active_neynar_webhook_secret(webhook: dict[str, Any]) -> str:
    secrets = webhook.get("secrets")
    if not isinstance(secrets, list):
        raise RecoveryError("Neynar webhook response is missing signing secrets")
    candidates = [
        item.get("value", "").strip()
        for item in secrets
        if isinstance(item, dict)
        and not item.get("deleted_at")
        and isinstance(item.get("value"), str)
        and item.get("value", "").strip()
    ]
    if not candidates:
        raise RecoveryError("Neynar webhook has no active signing secret")
    return candidates[-1]


def ensure_neynar_social_webhook(
    client: NeynarClient,
    *,
    bot_fid: str,
    target_url: str,
) -> tuple[dict[str, Any], str]:
    name = "Agent Bounties social mention drafts"
    fid = int(bot_fid)
    desired_subscription = {"cast.created": {"mentioned_fids": [fid]}}
    listed = client.request_json("GET", "/v2/farcaster/webhook/list/")
    if not isinstance(listed, dict) or not isinstance(listed.get("webhooks"), list):
        raise RecoveryError("Neynar webhook list response is invalid")
    webhooks = [item for item in listed["webhooks"] if isinstance(item, dict)]
    matches = [
        item
        for item in webhooks
        if str(item.get("title", "")).casefold() == name.casefold()
        or item.get("target_url") == target_url
    ]
    if len(matches) > 1:
        raise RecoveryError("Neynar has duplicate Agent Bounties webhook registrations")

    changed = False
    webhook = matches[0] if matches else None
    current_filters = (
        webhook.get("subscription", {}).get("filters")
        if isinstance(webhook, dict)
        and isinstance(webhook.get("subscription"), dict)
        else None
    )
    exact = (
        isinstance(webhook, dict)
        and webhook.get("active") is True
        and webhook.get("target_url") == target_url
        and str(webhook.get("title", "")).casefold() == name.casefold()
        and current_filters == desired_subscription
    )
    if not exact:
        body: dict[str, Any] = {
            "name": name,
            "url": target_url,
            "subscription": desired_subscription,
        }
        if webhook is None:
            webhook = _neynar_webhook_from_response(
                client.request_json("POST", "/v2/farcaster/webhook/", body)
            )
        else:
            webhook_id = webhook.get("webhook_id")
            if not isinstance(webhook_id, str) or not webhook_id:
                raise RecoveryError("existing Neynar webhook is missing its id")
            body["webhook_id"] = webhook_id
            webhook = _neynar_webhook_from_response(
                client.request_json("PUT", "/v2/farcaster/webhook/", body)
            )
        changed = True

    webhook_id = webhook.get("webhook_id") if isinstance(webhook, dict) else None
    if not isinstance(webhook_id, str) or not webhook_id:
        raise RecoveryError("Neynar webhook is missing its id")
    if webhook.get("target_url") != target_url or webhook.get("active") is not True:
        raise RecoveryError("Neynar webhook did not retain the active target URL")
    secret = _active_neynar_webhook_secret(webhook)
    return (
        {
            "provider": "neynar",
            "webhook_id": webhook_id,
            "target_url": target_url,
            "active": True,
            "mentioned_fid": fid,
            "changed": changed,
        },
        secret,
    )


def validate_social_mention_readiness(payload: object) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise RecoveryError("social mention readiness response is invalid")
    if payload.get("schema_version") != "agent-bounties/social-mention-ingestion-readiness-v1":
        raise RecoveryError("social mention readiness schema is invalid")
    for field in (
        "enabled",
        "operator_enabled",
        "database_configured",
        "webhook_configured",
        "reply_configured",
        "gate_passed",
    ):
        if payload.get(field) is not True:
            raise RecoveryError(f"social mention readiness reports {field}=false")
    return {
        "enabled": True,
        "provider": payload.get("provider"),
        "source_network": payload.get("source_network"),
        "bot_fid": payload.get("bot_fid"),
        "bot_username": payload.get("bot_username"),
        "github_originated_canonical_funded": payload.get(
            "github_originated_canonical_funded"
        ),
        "github_originated_canonical_settled": payload.get(
            "github_originated_canonical_settled"
        ),
    }


def deploy(
    client: RenderClient,
    revision: str,
    *,
    deploy_mode: str = "build_and_deploy",
    specs: tuple[ServiceSpec, ...] = SERVICE_SPECS,
    deploy_timeout_seconds: float,
    health_timeout_seconds: float,
    poll_seconds: float,
    public_base_url: str = "https://api.agentbounties.app",
    mcp_base_url: str = "https://mcp.agentbounties.app",
    website_base_url: str = "https://agentbounties.app",
    cloud_agent_api_key: str | None = None,
    neynar_api_key: str | None = None,
    neynar_signer_uuid: str | None = None,
    neynar_bot_fid: str | None = None,
    neynar_bot_username: str | None = None,
    base_mainnet_leaderboard_reward_contract: str | None = None,
    base_sepolia_leaderboard_reward_contract: str | None = None,
    open_competition_entrant_relay_canary_enabled: bool = False,
) -> dict[str, Any]:
    deploy_mode = validate_deploy_mode(deploy_mode)
    services: list[tuple[ServiceSpec, dict[str, Any]]] = []
    pending: dict[str, tuple[dict[str, Any], str]] = {}
    initial: dict[str, dict[str, Any]] = {}
    preexisting_live: dict[str, dict[str, Any]] = {}

    resolved: dict[str, dict[str, Any]] = {}
    missing: list[ServiceSpec] = []
    for spec in specs:
        try:
            resolved[spec.name] = client.resolve_service(spec)
        except RenderServiceMissing:
            if spec.name not in BLUEPRINT_RECOVERABLE_SERVICE_NAMES:
                raise
            missing.append(spec)

    for spec in missing:
        resolved[spec.name] = client.ensure_blueprint_service(
            spec,
            reference_service=resolved.get(OPEN_COMPETITION_REFERENCE_SERVICE_NAME),
        )

    # Revalidate every binding after a Blueprint sync and before changing any
    # service configuration or triggering a deployment.
    if missing:
        resolved = {spec.name: client.resolve_service(spec) for spec in specs}
    services = [(spec, resolved[spec.name]) for spec in specs]

    if deploy_mode == "deploy_only":
        for spec, service in services:
            deploys = client.list_deploys(service["id"])
            if spec.health_url is None:
                current = current_live_deploy(deploys, revision)
            else:
                current = current_live_deploy_record(deploys)
                validate_health(
                    spec.name,
                    revision,
                    fetch_health(spec.health_url, 10),
                )
            preexisting_live[spec.name] = current

    reference_service = resolved.get(OPEN_COMPETITION_REFERENCE_SERVICE_NAME)
    if reference_service is None:
        raise RecoveryError("validated reference Render worker is unavailable")
    shared_group = client.resolve_env_group(
        validate_owner_id(reference_service.get("ownerId"))
    )
    shared_group = client.get_env_group(shared_group["id"])
    for spec, service in services:
        if not env_group_has_service(shared_group, spec, service["id"]):
            raise RecoveryError(
                f"required Render environment group is not linked to {spec.name}"
            )
    desired_open_competition_environment = open_competition_shared_environment(
        entrant_relay_canary_enabled=open_competition_entrant_relay_canary_enabled
    )
    reconciled_open_competition_environment = []
    open_competition_environment_changed = False
    for key, value in desired_open_competition_environment.items():
        record = client.ensure_env_group_env_var(shared_group, key, value)
        open_competition_environment_changed |= record["changed"]
        reconciled_open_competition_environment.append(record)

    neynar_inputs = normalize_neynar_social_inputs(
        api_key=neynar_api_key,
        signer_uuid=neynar_signer_uuid,
        bot_fid=neynar_bot_fid,
        bot_username=neynar_bot_username,
    )

    for spec, service in services:
        client.disable_native_auto_deploy(service)
    social_webhook: dict[str, Any] = {}
    neynar_webhook_secret: str | None = None
    if neynar_inputs:
        social_webhook, neynar_webhook_secret = ensure_neynar_social_webhook(
            NeynarClient(neynar_inputs["NEYNAR_API_KEY"]),
            bot_fid=neynar_inputs["NEYNAR_BOT_FID"],
            target_url=f"{public_base_url.rstrip('/')}/v1/social/webhooks/neynar",
        )

    desired_public_environment = public_environment_values(
        public_base_url,
        mcp_base_url,
        website_base_url,
    )
    leaderboard_environment = leaderboard_environment_values(
        base_mainnet_leaderboard_reward_contract,
        base_sepolia_leaderboard_reward_contract,
    )
    reconciled_public_environment = []
    public_environment_changed: dict[str, bool] = {}
    for spec, service in services:
        if spec.name not in PUBLIC_ENV_SERVICE_NAMES:
            continue
        public_environment_changed[spec.name] = False
        for key, value in desired_public_environment.items():
            record = client.ensure_env_var(service, key, value)
            public_environment_changed[spec.name] |= record["changed"]
            reconciled_public_environment.append(
                {
                    "service": spec.name,
                    "key": key,
                    "value": value,
                    "changed": record["changed"],
                }
            )
        if spec.name == CLOUD_AGENT_API_SERVICE_NAME:
            for key, value in API_RUNTIME_ENVIRONMENT.items():
                record = client.ensure_env_var(service, key, value)
                public_environment_changed[spec.name] |= record["changed"]
                reconciled_public_environment.append(
                    {
                        "service": spec.name,
                        "key": key,
                        "value": value,
                        "changed": record["changed"],
                    }
                )
            for key, value in leaderboard_environment.items():
                record = client.ensure_env_var(service, key, value)
                public_environment_changed[spec.name] |= record["changed"]
                reconciled_public_environment.append(
                    {
                        "service": spec.name,
                        "key": key,
                        "value": value,
                        "changed": record["changed"],
                    }
                )

    api_service = next(
        service for spec, service in services if spec.name == CLOUD_AGENT_API_SERVICE_NAME
    )
    cloud_environment, secret_environment, cloud_environment_changed = (
        reconcile_cloud_agent_environment(client, api_service, cloud_agent_api_key)
    )
    public_environment_changed[CLOUD_AGENT_API_SERVICE_NAME] |= cloud_environment_changed
    social_environment, social_environment_changed = reconcile_neynar_social_environment(
        client,
        api_service,
        api_key=neynar_api_key,
        webhook_secret=neynar_webhook_secret,
        signer_uuid=neynar_signer_uuid,
        bot_fid=neynar_bot_fid,
        bot_username=neynar_bot_username,
    )
    public_environment_changed[CLOUD_AGENT_API_SERVICE_NAME] |= social_environment_changed

    custom_domains = []
    for spec, service in services:
        domains = CUSTOM_DOMAINS.get(spec.name, ())
        for domain, record in zip(
            domains, client.reconcile_custom_domains(service, domains), strict=True
        ):
            custom_domains.append(
                {
                    "service": spec.name,
                    "name": domain,
                    "status": record.get(
                        "verificationStatus", record.get("status", "attached")
                    ),
                }
            )

    metadata_revision_exempt = (
        frozenset(spec.name for spec in specs if spec.health_url is not None)
        if deploy_mode == "deploy_only"
        else frozenset()
    )
    for spec, service in services:
        if (
            deploy_mode == "deploy_only"
            and spec.name != CLOUD_AGENT_API_SERVICE_NAME
            and not open_competition_environment_changed
        ):
            created = preexisting_live[spec.name]
        else:
            created = client.ensure_deploy(
                service,
                revision,
                force=(
                    True
                    if deploy_mode == "deploy_only"
                    else (
                        public_environment_changed.get(spec.name, False)
                        or open_competition_environment_changed
                    )
                ),
                deploy_mode=deploy_mode,
            )
        deploy_id, status = validate_deploy(
            created,
            revision,
            spec.name,
            require_revision=spec.name not in metadata_revision_exempt,
        )
        initial[spec.name] = created
        if status == "live":
            continue
        if status in FAILED_STATUSES:
            raise deploy_failure(client, spec.name, service, created)
        if status not in ACTIVE_STATUSES:
            raise RecoveryError(f"{spec.name} deploy has unknown status {status}")
        pending[spec.name] = (service, deploy_id)

    completed = {
        name: deploy_record
        for name, deploy_record in initial.items()
        if deploy_record.get("status") == "live"
    }
    completed.update(
        poll_deploys(
            client,
            pending,
            revision,
            timeout_seconds=deploy_timeout_seconds,
            poll_seconds=poll_seconds,
            metadata_revision_exempt=metadata_revision_exempt,
        )
    )

    health = [
        wait_for_health(
            spec,
            revision,
            timeout_seconds=health_timeout_seconds,
            poll_seconds=poll_seconds,
        )
        for spec, _ in services
        if spec.health_url is not None
    ]
    cloud_readiness = validate_cloud_agent_readiness(
        fetch_json(
            f"{public_base_url.rstrip('/')}/v1/cloud-agent/readiness",
            20,
        ),
        credential_supplied=bool(
            cloud_agent_api_key and cloud_agent_api_key.strip()
        ),
    )
    leaderboard_readiness = []
    for network, key in (
        ("base-mainnet", "BASE_MAINNET_LEADERBOARD_REWARD_CONTRACT"),
        ("base-sepolia", "BASE_SEPOLIA_LEADERBOARD_REWARD_CONTRACT"),
    ):
        expected_contract = leaderboard_environment.get(key)
        if expected_contract is None:
            continue
        leaderboard_readiness.append(
            validate_leaderboard_readiness(
                fetch_json(
                    f"{public_base_url.rstrip('/')}/v1/base/autonomous-bounties/leaderboard?network={network}",
                    20,
                ),
                network=network,
                expected_contract=expected_contract,
            )
        )
    social_readiness = {}
    if social_environment:
        social_readiness = validate_social_mention_readiness(
            fetch_json(
                f"{public_base_url.rstrip('/')}/v1/social/mention-ingestion/readiness",
                20,
            )
        )
    service_evidence = []
    for spec, service in services:
        deployed = completed[spec.name]
        deploy_id, status = validate_deploy(
            deployed,
            revision,
            spec.name,
            require_revision=spec.name not in metadata_revision_exempt,
        )
        service_evidence.append(
            {
                "name": spec.name,
                "service_id": service["id"],
                "service_type": spec.service_type,
                "deploy_id": deploy_id,
                "status": status,
                "commit": revision,
                "metadata_commit": deploy_commit(deployed),
                "runtime_revision": revision,
                "runtime_revision_evidence": (
                    "health_and_readiness"
                    if spec.name in metadata_revision_exempt
                    else "render_deploy_metadata"
                ),
                "trigger": deployed.get("trigger"),
                "native_auto_deploy": "disabled",
            }
        )
    return {
        "deploy_mode": deploy_mode,
        "services": service_evidence,
        "health": health,
        "custom_domains": custom_domains,
        "public_environment": reconciled_public_environment,
        "open_competition_environment": reconciled_open_competition_environment,
        "cloud_environment": cloud_environment,
        "secret_environment": secret_environment,
        "social_environment": social_environment,
        "social_webhook": social_webhook,
        "cloud_readiness": cloud_readiness,
        "social_readiness": social_readiness,
        "leaderboard_readiness": leaderboard_readiness,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Deploy and attest an exact reviewed main revision on Render."
    )
    parser.add_argument("--revision", required=True)
    parser.add_argument(
        "--deploy-mode",
        choices=sorted(DEPLOY_MODES),
        default="build_and_deploy",
    )
    parser.add_argument(
        "--api-url",
        default="https://agent-bounties-api.onrender.com/health",
    )
    parser.add_argument(
        "--mcp-url",
        default="https://agent-bounties-mcp.onrender.com/health",
    )
    parser.add_argument(
        "--public-base-url",
        default="https://api.agentbounties.app",
    )
    parser.add_argument(
        "--mcp-base-url",
        default="https://mcp.agentbounties.app",
    )
    parser.add_argument(
        "--website-base-url",
        default="https://agentbounties.app",
    )
    parser.add_argument("--base-mainnet-leaderboard-reward-contract")
    parser.add_argument("--base-sepolia-leaderboard-reward-contract")
    parser.add_argument(
        "--enable-open-competition-entrant-relay-canary",
        action="store_true",
        help="Enable only the operator-authenticated entrant relay canary.",
    )
    parser.add_argument("--deploy-timeout-seconds", type=float, default=2400)
    parser.add_argument("--health-timeout-seconds", type=float, default=300)
    parser.add_argument("--poll-seconds", type=float, default=10)
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("target/operations/render-deploy.json"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    evidence: dict[str, Any] = {
        "schema": SCHEMA,
        "started_at": utc_now(),
        "completed_at": None,
        "revision": args.revision,
        "deploy_mode": args.deploy_mode,
        "success": False,
        "services": [],
        "health": [],
        "custom_domains": [],
        "public_environment": [],
        "cloud_environment": [],
        "secret_environment": [],
        "social_environment": [],
        "social_webhook": {},
        "cloud_readiness": {},
        "social_readiness": {},
        "leaderboard_readiness": [],
        "failure": None,
        "error": None,
    }
    try:
        revision = validate_revision(args.revision)
        specs = list(SERVICE_SPECS)
        specs[0] = ServiceSpec(specs[0].name, specs[0].service_type, args.api_url)
        specs[1] = ServiceSpec(specs[1].name, specs[1].service_type, args.mcp_url)
        client = RenderClient(os.environ.get("RENDER_API_KEY", ""))
        result = deploy(
            client,
            revision,
            deploy_mode=args.deploy_mode,
            specs=tuple(specs),
            deploy_timeout_seconds=args.deploy_timeout_seconds,
            health_timeout_seconds=args.health_timeout_seconds,
            poll_seconds=args.poll_seconds,
            public_base_url=args.public_base_url,
            mcp_base_url=args.mcp_base_url,
            website_base_url=args.website_base_url,
            cloud_agent_api_key=os.environ.get("CLOUD_AGENT_API_KEY"),
            neynar_api_key=os.environ.get("NEYNAR_API_KEY"),
            neynar_signer_uuid=os.environ.get("NEYNAR_SIGNER_UUID"),
            neynar_bot_fid=os.environ.get("NEYNAR_BOT_FID"),
            neynar_bot_username=os.environ.get("NEYNAR_BOT_USERNAME"),
            base_mainnet_leaderboard_reward_contract=(
                args.base_mainnet_leaderboard_reward_contract
            ),
            base_sepolia_leaderboard_reward_contract=(
                args.base_sepolia_leaderboard_reward_contract
            ),
            open_competition_entrant_relay_canary_enabled=(
                args.enable_open_competition_entrant_relay_canary
            ),
        )
        evidence.update(result)
        evidence["revision"] = revision
        evidence["success"] = True
    except (RecoveryError, ValueError) as error:
        evidence["error"] = redact(str(error))
        if isinstance(error, RenderDeployFailure):
            evidence["failure"] = error.evidence
    finally:
        evidence["completed_at"] = utc_now()
        write_evidence(args.output, evidence)

    if not evidence["success"]:
        print(f"render deploy recovery failed: {evidence['error']}", file=sys.stderr)
        return 1
    print(f"render deploy recovery verified {evidence['revision']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
