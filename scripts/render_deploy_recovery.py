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
    "agent-bounties-api": "api.bountyboard.global",
    "agent-bounties-mcp": "mcp.bountyboard.global",
}
PUBLIC_ENV_SERVICE_NAMES = {
    "agent-bounties-api",
    "agent-bounties-mcp",
}
CLOUD_AGENT_API_SERVICE_NAME = "agent-bounties-api"
CLOUD_AGENT_RUNTIME_ENVIRONMENT = {
    "CLOUD_AGENT_ENABLED": "true",
    "CLOUD_AGENT_PUBLIC_DRAFTS": "true",
    "CLOUD_AGENT_PROVIDER": "openai-compatible",
    "CLOUD_AGENT_PROTOCOL": "openai_chat_completions",
    "CLOUD_AGENT_ENDPOINT": "https://api.openai.com/v1/chat/completions",
    "CLOUD_AGENT_MODEL": "gpt-4.1-mini",
    "CLOUD_AGENT_MAX_INPUT_CHARS": "12000",
    "CLOUD_AGENT_MAX_OUTPUT_TOKENS": "2500",
    "CLOUD_AGENT_MAX_DAILY_DRAFTS": "25",
    "CLOUD_AGENT_TIMEOUT_SECONDS": "45",
}


class RecoveryError(RuntimeError):
    pass


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
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def redact(value: str) -> str:
    value = re.sub(r"(?i)(authorization:\s*bearer\s+)[^\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)([?&](?:key|token)=)[^&\s]+", r"\1[redacted]", value)
    value = re.sub(r"(?i)(render_api_key\s*[=:]\s*)[^\s,;]+", r"\1[redacted]", value)
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

    def resolve_service(self, spec: ServiceSpec) -> dict[str, Any]:
        query = urllib.parse.urlencode(
            {"name": spec.name, "includePreviews": "false", "limit": "20"}
        )
        return select_service(spec, self._read_with_retry(f"/services?{query}"))

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
        created = self._request_json(
            "POST", f"/services/{service_id}/custom-domains", {"name": domain}
        )
        custom_domain = created.get("customDomain", created) if isinstance(created, dict) else None
        if not isinstance(custom_domain, dict) or str(custom_domain.get("name", "")).lower() != domain.lower():
            raise RecoveryError(f"Render did not attach {domain} to {service['name']}")
        return custom_domain

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
    if payload.get("authority") != "draft_only":
        raise RecoveryError("cloud-agent readiness exceeds draft-only authority")
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
        "authority": "draft_only",
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


def deploy(
    client: RenderClient,
    revision: str,
    *,
    deploy_mode: str = "build_and_deploy",
    specs: tuple[ServiceSpec, ...] = SERVICE_SPECS,
    deploy_timeout_seconds: float,
    health_timeout_seconds: float,
    poll_seconds: float,
    public_base_url: str = "https://api.bountyboard.global",
    mcp_base_url: str = "https://mcp.bountyboard.global",
    cloud_agent_api_key: str | None = None,
    base_mainnet_leaderboard_reward_contract: str | None = None,
    base_sepolia_leaderboard_reward_contract: str | None = None,
) -> dict[str, Any]:
    deploy_mode = validate_deploy_mode(deploy_mode)
    services: list[tuple[ServiceSpec, dict[str, Any]]] = []
    pending: dict[str, tuple[dict[str, Any], str]] = {}
    initial: dict[str, dict[str, Any]] = {}
    preexisting_live: dict[str, dict[str, Any]] = {}

    for spec in specs:
        services.append((spec, client.resolve_service(spec)))

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

    for spec, service in services:
        client.disable_native_auto_deploy(service)

    public_environment_values = {
        "PUBLIC_BASE_URL": normalize_public_base_url(
            "PUBLIC_BASE_URL", public_base_url
        ),
        "MCP_BASE_URL": normalize_public_base_url("MCP_BASE_URL", mcp_base_url),
    }
    leaderboard_environment = leaderboard_environment_values(
        base_mainnet_leaderboard_reward_contract,
        base_sepolia_leaderboard_reward_contract,
    )
    public_environment = []
    public_environment_changed: dict[str, bool] = {}
    for spec, service in services:
        if spec.name not in PUBLIC_ENV_SERVICE_NAMES:
            continue
        public_environment_changed[spec.name] = False
        for key, value in public_environment_values.items():
            record = client.ensure_env_var(service, key, value)
            public_environment_changed[spec.name] |= record["changed"]
            public_environment.append(
                {
                    "service": spec.name,
                    "key": key,
                    "value": value,
                    "changed": record["changed"],
                }
            )
        if spec.name == CLOUD_AGENT_API_SERVICE_NAME:
            for key, value in leaderboard_environment.items():
                record = client.ensure_env_var(service, key, value)
                public_environment_changed[spec.name] |= record["changed"]
                public_environment.append(
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

    custom_domains = []
    for spec, service in services:
        domain = CUSTOM_DOMAINS.get(spec.name)
        if domain is None:
            continue
        record = client.ensure_custom_domain(service, domain)
        custom_domains.append(
            {
                "service": spec.name,
                "name": domain,
                "status": record.get("verificationStatus", record.get("status", "attached")),
            }
        )

    metadata_revision_exempt = (
        frozenset(spec.name for spec in specs if spec.health_url is not None)
        if deploy_mode == "deploy_only"
        else frozenset()
    )
    for spec, service in services:
        if deploy_mode == "deploy_only" and spec.name != CLOUD_AGENT_API_SERVICE_NAME:
            created = preexisting_live[spec.name]
        else:
            created = client.ensure_deploy(
                service,
                revision,
                force=(
                    True
                    if deploy_mode == "deploy_only"
                    else public_environment_changed.get(spec.name, False)
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
        "public_environment": public_environment,
        "cloud_environment": cloud_environment,
        "secret_environment": secret_environment,
        "cloud_readiness": cloud_readiness,
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
        default="https://api.bountyboard.global",
    )
    parser.add_argument(
        "--mcp-base-url",
        default="https://mcp.bountyboard.global",
    )
    parser.add_argument("--base-mainnet-leaderboard-reward-contract")
    parser.add_argument("--base-sepolia-leaderboard-reward-contract")
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
        "cloud_readiness": {},
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
            cloud_agent_api_key=os.environ.get("CLOUD_AGENT_API_KEY"),
            base_mainnet_leaderboard_reward_contract=(
                args.base_mainnet_leaderboard_reward_contract
            ),
            base_sepolia_leaderboard_reward_contract=(
                args.base_sepolia_leaderboard_reward_contract
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
